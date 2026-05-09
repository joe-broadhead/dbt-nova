use std::collections::BTreeMap;
use std::path::Path;

use serde_json::json;
use tempfile::{NamedTempFile, TempDir};

use super::{
    AgentCalledWith, AgentEntityRank, AgentExpected, AgentOrder, AssertionResult, EvalCaseReport,
    EvalDefaults, EvalRunArgs, EvalSuite, EvalValidateArgs, FinalAnswerExpected,
    contains_rank_assertion, context_contains_assertion, context_field_equals_assertion,
    json_has_field_path, read_tool_trace, run_eval_command, run_validate_command,
    safe_path_segment, score_agent_expectations, selected_agent_cases, selected_bridge_cases,
    tool_success_assertion, validate_suite,
};

#[test]
fn field_path_checks_nested_response() {
    let response = json!({"data": {"name": "orders", "nested": {"value": 1}}});
    assert!(json_has_field_path(&response, "data.name"));
    assert!(json_has_field_path(&response, "data.nested.value"));
    assert!(!json_has_field_path(&response, "data.missing"));
}

#[test]
fn contains_rank_assertion_respects_max_rank() {
    let response = json!({
        "data": [
            {"unique_id": "model.pkg.other"},
            {"unique_id": "model.pkg.orders"}
        ]
    });
    let result = contains_rank_assertion("search_indicator_rank", &response, "orders", Some(1));
    assert_eq!(result.status, "fail");
}

#[test]
fn agent_expectations_score_tool_trace() {
    let expected = AgentExpected {
        must_call: vec!["search_indicator".to_string(), "get_context".to_string()],
        must_not_call: vec!["execute_sql".to_string()],
        ordered: vec![AgentOrder {
            before: "get_context".to_string(),
            must_have_called: vec!["search_indicator".to_string()],
        }],
        selected_entities: vec!["model.pkg.orders".to_string()],
        selected_entity_ranks: vec![AgentEntityRank {
            unique_id: "model.pkg.orders".to_string(),
            tool: Some("search_indicator".to_string()),
            max_rank: Some(1),
        }],
        called_with: vec![AgentCalledWith {
            tool: "search_indicator".to_string(),
            params: BTreeMap::new(),
            contains: BTreeMap::from([(String::from("query"), String::from("gmv"))]),
        }],
        final_answer: Some(FinalAnswerExpected {
            must_contain: vec!["gmv".to_string()],
            must_not_contain: vec!["secret".to_string()],
        }),
    };
    let trace = vec![
        json!({
            "tool": "search_indicator",
            "params_summary": {"query": "gmv"},
            "selected_unique_ids": ["model.pkg.orders"],
            "top_unique_ids": ["model.pkg.orders"]
        }),
        json!({
            "tool": "get_context",
            "params_summary": {"id_or_name": "model.pkg.orders"},
            "selected_unique_ids": ["model.pkg.orders"],
            "top_unique_ids": ["model.pkg.orders"]
        }),
    ];
    let results = score_agent_expectations(&expected, &trace, "GMV uses model.pkg.orders");
    assert!(results.iter().all(|result| result.status == "pass"));
}

#[test]
fn selected_entity_rank_fails_when_entity_is_below_max_rank() {
    let expected = AgentExpected {
        selected_entity_ranks: vec![AgentEntityRank {
            unique_id: "model.pkg.orders".to_string(),
            tool: Some("search".to_string()),
            max_rank: Some(1),
        }],
        ..AgentExpected::default()
    };
    let trace = vec![json!({
        "tool": "search",
        "top_unique_ids": ["model.pkg.other", "model.pkg.orders"]
    })];
    let results = score_agent_expectations(&expected, &trace, "");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].status, "fail");
}

#[test]
fn selected_entity_rank_can_scope_to_tool() {
    let expected = AgentExpected {
        selected_entity_ranks: vec![AgentEntityRank {
            unique_id: "model.pkg.orders".to_string(),
            tool: Some("search".to_string()),
            max_rank: Some(1),
        }],
        ..AgentExpected::default()
    };
    let trace = vec![
        json!({
            "tool": "search",
            "top_unique_ids": ["model.pkg.other", "model.pkg.orders"]
        }),
        json!({
            "tool": "get_context",
            "top_unique_ids": ["model.pkg.orders"]
        }),
    ];
    let results = score_agent_expectations(&expected, &trace, "");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].status, "fail");
}

#[test]
fn called_with_matches_safe_params() {
    let expected = AgentExpected {
        called_with: vec![AgentCalledWith {
            tool: "search".to_string(),
            params: BTreeMap::from([(String::from("resource_types"), json!(["model"]))]),
            contains: BTreeMap::from([(String::from("query"), String::from("orders"))]),
        }],
        ..AgentExpected::default()
    };
    let trace = vec![json!({
        "tool": "search",
        "params_summary": {"query": "canonical orders", "resource_types": ["model", "seed"]}
    })];
    let results = score_agent_expectations(&expected, &trace, "");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].status, "pass");
}

#[test]
fn context_value_assertions_check_equals_and_contains() {
    let response = json!({
        "data": {
            "entity": {
                "name": "orders",
                "description": "Canonical orders model"
            }
        }
    });
    assert_eq!(
        context_field_equals_assertion(&response, "data.entity.name", &json!("orders")).status,
        "pass"
    );
    assert_eq!(
        context_contains_assertion(&response, Some("data.entity.description"), "canonical").status,
        "pass"
    );
    assert_eq!(
        context_field_equals_assertion(&response, "data.entity.name", &json!("customers")).status,
        "fail"
    );
}

#[test]
fn tool_success_fails_explicit_false_response() {
    let result = tool_success_assertion(
        "search",
        &json!({
            "success": false,
            "error": {"error_code": "invalid_params", "message": "bad request"},
            "data": [{"unique_id": "model.pkg.should_not_be_embedded"}]
        }),
    );
    assert_eq!(result.status, "fail");
    assert_eq!(result.evidence["success"], false);
    assert_eq!(result.evidence["error.error_code"], "invalid_params");
    assert!(result.evidence.get("data").is_none());
}

#[test]
fn case_report_counts_statuses() {
    let report = EvalCaseReport::new(
        "case".to_string(),
        None,
        vec![
            AssertionResult::pass("a", "ok", json!({})),
            AssertionResult::fail("b", "bad", json!({})),
            AssertionResult::error("c", "err"),
        ],
        None,
    );
    assert_eq!(report.pass_count, 1);
    assert_eq!(report.fail_count, 1);
    assert_eq!(report.error_count, 1);
}

#[test]
fn read_tool_trace_reports_parse_errors() {
    let file = NamedTempFile::new().expect("temp file");
    std::fs::write(
        file.path(),
        "{\"tool\":\"search\"}\nnot-json\n{\"tool\":\"get_context\"}\n",
    )
    .expect("write trace");
    let trace = read_tool_trace(file.path());
    assert_eq!(trace.rows.len(), 2);
    assert_eq!(trace.errors.len(), 1);
    assert!(!trace.missing);
}

#[test]
fn validate_suite_rejects_duplicate_agent_case_ids() {
    let suite = EvalSuite {
        version: 1,
        name: None,
        defaults: EvalDefaults::default(),
        cases: Vec::new(),
        agent_cases: vec![
            super::AgentCase {
                id: "same".to_string(),
                task: "one".to_string(),
                expected: AgentExpected::default(),
            },
            super::AgentCase {
                id: "same".to_string(),
                task: "two".to_string(),
                expected: AgentExpected::default(),
            },
        ],
    };
    let error = validate_suite(&suite).expect_err("duplicate id should fail");
    assert!(error.to_string().contains("duplicate eval case id"));
}

#[test]
fn validate_suite_rejects_duplicate_artifact_segments() {
    let suite = EvalSuite {
        version: 1,
        name: None,
        defaults: EvalDefaults::default(),
        cases: Vec::new(),
        agent_cases: vec![
            super::AgentCase {
                id: "a/b".to_string(),
                task: "one".to_string(),
                expected: AgentExpected::default(),
            },
            super::AgentCase {
                id: "a b".to_string(),
                task: "two".to_string(),
                expected: AgentExpected::default(),
            },
        ],
    };
    let error = validate_suite(&suite).expect_err("artifact path collision should fail");
    assert!(error.to_string().contains("artifact paths"));
}

#[test]
fn validate_suite_rejects_case_insensitive_artifact_segment_collisions() {
    let suite = EvalSuite {
        version: 1,
        name: None,
        defaults: EvalDefaults::default(),
        cases: Vec::new(),
        agent_cases: vec![
            super::AgentCase {
                id: "RevenueFlow".to_string(),
                task: "one".to_string(),
                expected: AgentExpected::default(),
            },
            super::AgentCase {
                id: "revenueflow".to_string(),
                task: "two".to_string(),
                expected: AgentExpected::default(),
            },
        ],
    };
    let error = validate_suite(&suite).expect_err("case-insensitive collision should fail");
    assert!(error.to_string().contains("case-insensitively"));
}

#[test]
fn validate_suite_rejects_vacuous_search_columns_rank_assertion() {
    let suite = EvalSuite {
        version: 1,
        name: None,
        defaults: EvalDefaults::default(),
        cases: vec![super::EvalCase {
            id: "columns".to_string(),
            question: None,
            persona: None,
            assertions: vec![super::EvalAssertion::SearchColumnsRank {
                query: "revenue".to_string(),
                expected_column: None,
                expected_parent_unique_id: None,
                max_rank: None,
            }],
        }],
        agent_cases: Vec::new(),
    };
    let error = validate_suite(&suite).expect_err("vacuous column rank should fail");
    assert!(error.to_string().contains("expected_column"));
}

#[test]
fn validate_suite_rejects_unmatchable_called_with_param_values() {
    let suite = EvalSuite {
        version: 1,
        name: None,
        defaults: EvalDefaults::default(),
        cases: Vec::new(),
        agent_cases: vec![super::AgentCase {
            id: "agent".to_string(),
            task: "use nova".to_string(),
            expected: AgentExpected {
                called_with: vec![AgentCalledWith {
                    tool: "search".to_string(),
                    params: BTreeMap::from([(String::from("query"), json!({"nested": true}))]),
                    contains: BTreeMap::new(),
                }],
                ..AgentExpected::default()
            },
        }],
    };
    let error = validate_suite(&suite).expect_err("nested param expectation should fail");
    assert!(error.to_string().contains("scalar values"));
}

#[test]
fn selected_case_filters_reject_missing_ids() {
    let cases = vec![super::EvalCase {
        id: "one".to_string(),
        question: None,
        persona: None,
        assertions: vec![super::EvalAssertion::ToolSuccess {
            tool: "search".to_string(),
            params: json!({}),
        }],
    }];
    let error = selected_bridge_cases(&cases, &[String::from("missing")])
        .expect_err("missing case id should fail");
    assert!(error.to_string().contains("not found"));
}

#[test]
fn selected_agent_case_filters_return_requested_cases() {
    let cases = vec![
        super::AgentCase {
            id: "one".to_string(),
            task: "task one".to_string(),
            expected: AgentExpected::default(),
        },
        super::AgentCase {
            id: "two".to_string(),
            task: "task two".to_string(),
            expected: AgentExpected::default(),
        },
    ];
    let selected = selected_agent_cases(&cases, &[String::from("two")]).expect("filter");
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].id, "two");
}

#[test]
fn validate_command_accepts_valid_suite_without_manifest() {
    let suite = NamedTempFile::new().expect("suite file");
    std::fs::write(
        suite.path(),
        r"
version: 1
name: validate-smoke
cases:
  - id: one
    assertions:
      - type: tool_success
        tool: search
        params: {}
",
    )
    .expect("write suite");
    let result = run_validate_command(&EvalValidateArgs {
        suite: suite.path().display().to_string(),
        json: true,
    });
    assert!(
        result.is_ok(),
        "valid suite failed: {}",
        result
            .err()
            .map_or_else(String::new, |error| error.error.to_string())
    );
}

#[tokio::test]
async fn bridge_eval_writes_result_artifacts() {
    let temp_dir = TempDir::new().expect("output dir");
    let suite_path = temp_dir.path().join("suite.yml");
    std::fs::write(
        &suite_path,
        r"
version: 1
name: bridge-smoke
cases:
  - id: orders-search
    assertions:
      - type: search_rank
        query: orders
        expected_unique_id: model.nova_test.fct__orders
        max_rank: 5
",
    )
    .expect("write suite");
    let output_dir = temp_dir.path().join("out");
    let result = run_eval_command(&EvalRunArgs {
        suite: suite_path.display().to_string(),
        manifest_path: Some(
            fixture_manifest_path("nova_manifest.json")
                .display()
                .to_string(),
        ),
        output_dir: Some(output_dir.display().to_string()),
        fail_under: Some(1.0),
        json: true,
        ..EvalRunArgs::default()
    })
    .await;
    assert!(
        result.is_ok(),
        "bridge eval failed: {}",
        result
            .err()
            .map_or_else(String::new, |error| error.error.to_string())
    );
    assert!(output_dir.join("results.json").exists());
    assert!(output_dir.join("results.tsv").exists());
    assert!(output_dir.join("report.md").exists());
    assert!(output_dir.join("suite.yml").exists());
}

#[test]
fn safe_path_segment_blocks_dot_segments_and_caps_length() {
    assert_eq!(safe_path_segment("."), "eval");
    assert_eq!(safe_path_segment(".."), "eval");
    assert_eq!(safe_path_segment("../secret"), "secret");
    assert_eq!(safe_path_segment("a/b"), "a-b");
    assert!(safe_path_segment(&"x".repeat(200)).len() <= 120);
}

fn fixture_manifest_path(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}
