use std::collections::BTreeMap;
use std::path::Path;

use serde_json::json;
use tempfile::{NamedTempFile, TempDir};

use super::{
    AgentCalledWith, AgentEntityRank, AgentExpected, AgentOrder, AssertionResult, EvalCaseReport,
    EvalDefaults, EvalRunArgs, EvalSuite, EvalValidateArgs, FinalAnswerExpected,
    apply_telemetry_retention, build_eval_gate_report, contains_rank_assertion,
    context_contains_assertion, context_field_equals_assertion, eval_case_telemetry_from_trace,
    format_utc_timestamp_millis, json_has_field_path, read_tool_trace, recipe_rank_assertion,
    run_eval_command, run_validate_command, safe_path_segment, score_agent_expectations,
    selected_agent_cases, selected_bridge_cases, suite_file_hash, telemetry_path_for_suite,
    telemetry_row_matches_since, tool_response_budget_assertion, tool_success_assertion,
    validate_since_date, validate_suite, validate_telemetry_suite_name,
};

#[test]
fn field_path_checks_nested_response() {
    let response =
        json!({"data": {"name": "orders", "nested": {"value": 1}}, "rows": [{"name": "first"}]});
    assert!(json_has_field_path(&response, "data.name"));
    assert!(json_has_field_path(&response, "data.nested.value"));
    assert!(json_has_field_path(&response, "rows.0.name"));
    assert!(!json_has_field_path(&response, "data.missing"));
    assert!(!json_has_field_path(&response, "rows.1.name"));
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
fn recipe_rank_assertion_accepts_search_recipes_id_field() {
    let response = json!({
        "data": [
            {"id": "reference/members", "query_count": 3}
        ]
    });
    let result = recipe_rank_assertion(&response, "reference/members", Some(1));
    assert_eq!(result.status, "pass");
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
        ..AgentExpected::default()
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
fn response_byte_budgets_require_trace_telemetry() {
    let expected = AgentExpected {
        max_total_response_bytes: Some(1024),
        max_response_bytes_by_tool: BTreeMap::from([(String::from("search"), 1024)]),
        ..AgentExpected::default()
    };
    let trace = vec![json!({"tool": "search"})];
    let results = score_agent_expectations(&expected, &trace, "");

    assert_eq!(results.len(), 2);
    assert!(results.iter().all(|result| result.status == "fail"));
    assert!(
        results
            .iter()
            .all(|result| result.message.contains("missing response byte telemetry"))
    );
}

#[test]
fn response_byte_budgets_score_observed_bytes() {
    let expected = AgentExpected {
        max_total_response_bytes: Some(100),
        max_response_bytes_by_tool: BTreeMap::from([(String::from("search"), 40)]),
        ..AgentExpected::default()
    };
    let trace = vec![
        json!({"tool": "search", "response_bytes": 41}),
        json!({"tool": "execute_sql", "response_bytes": 20}),
    ];
    let results = score_agent_expectations(&expected, &trace, "");

    assert_eq!(results.len(), 2);
    assert_eq!(results[0].status, "pass");
    assert_eq!(results[1].status, "fail");
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
fn tool_response_budget_checks_bytes_and_shape() {
    let response = json!({
        "data": [{"parent_unique_id": "model.pkg.orders", "expression": "count(*)"}],
        "parent_groups": [{"parent_unique_id": "model.pkg.orders"}]
    });

    let result = tool_response_budget_assertion(
        "search_indicator",
        &response,
        512,
        &[
            "data.0.parent_unique_id".to_string(),
            "data.0.expression".to_string(),
        ],
        &["parent_groups.1".to_string()],
    );

    assert_eq!(result.status, "pass");
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
        "{\"tool\":\"search\",\"tool_call_index\":0}\nnot-json\n{\"tool\":\"get_context\",\"tool_call_index\":0}\n",
    )
    .expect("write trace");
    let trace = read_tool_trace(file.path());
    assert_eq!(trace.rows.len(), 2);
    assert_eq!(trace.errors.len(), 1);
    assert!(!trace.missing);
    assert_eq!(trace.rows[0]["tool_call_index"], json!(0));
    assert_eq!(trace.rows[1]["tool_call_index"], json!(1));
}

#[test]
fn telemetry_stats_summarize_trace_without_params() {
    let trace = vec![
        json!({
            "tool": "search",
            "response_bytes": 40,
            "params_summary": {"query": "revenue"},
            "usage": {"prompt_tokens": 10, "completion_tokens": 5}
        }),
        json!({
            "tool": "get_context",
            "response_bytes": 60,
            "usage": {"input_tokens": 4, "output_tokens": 3, "total_tokens": 7}
        }),
    ];

    let telemetry = eval_case_telemetry_from_trace(&trace);

    assert_eq!(telemetry.tool_call_count, 2);
    assert_eq!(telemetry.distinct_tool_count, 2);
    assert_eq!(telemetry.total_response_bytes, Some(100));
    assert_eq!(telemetry.input_tokens, Some(14));
    assert_eq!(telemetry.output_tokens, Some(8));
    assert_eq!(telemetry.total_tokens, Some(7));
}

#[test]
fn telemetry_history_since_filters_iso_timestamps() {
    let since = validate_since_date("2026-06-01").expect("valid date");
    assert!(telemetry_row_matches_since(
        &json!({"timestamp": "2026-06-01T00:00:00.000Z"}),
        &since
    ));
    assert!(telemetry_row_matches_since(
        &json!({"timestamp": "2026-06-02T12:00:00.000Z"}),
        &since
    ));
    assert!(!telemetry_row_matches_since(
        &json!({"timestamp": "2026-05-31T23:59:59.999Z"}),
        &since
    ));
    assert!(validate_since_date("2026-02-29").is_err());
}

#[test]
fn telemetry_retention_keeps_newest_valid_jsonl_rows() {
    let file = NamedTempFile::new().expect("temp file");
    std::fs::write(
        file.path(),
        "{\"case_id\":\"one\"}\n{\"case_id\":\"two\"}\n{\"case_id\":\"three\"}\n",
    )
    .expect("write telemetry");

    let result = apply_telemetry_retention(file.path(), 2);
    assert!(
        result.is_ok(),
        "retention failed: {}",
        result
            .err()
            .map_or_else(String::new, |error| error.error.to_string())
    );

    let raw = std::fs::read_to_string(file.path()).expect("read telemetry");
    let lines: Vec<&str> = raw.lines().collect();
    assert_eq!(lines.len(), 2);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(lines[0]).unwrap()["case_id"],
        "two"
    );
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(lines[1]).unwrap()["case_id"],
        "three"
    );
}

#[test]
fn telemetry_timestamp_format_is_utc_rfc3339_millis() {
    assert_eq!(
        format_utc_timestamp_millis(1_767_225_600_123),
        "2026-01-01T00:00:00.123Z"
    );
}

#[test]
fn telemetry_paths_include_hash_to_avoid_sanitized_name_collisions() {
    let spaced = telemetry_path_for_suite("sales smoke");
    let slashed = telemetry_path_for_suite("sales/smoke");
    let dashed = telemetry_path_for_suite("sales-smoke");

    assert_ne!(spaced, slashed);
    assert_ne!(spaced, dashed);
    assert_ne!(slashed, dashed);
    let file_name = dashed
        .file_name()
        .and_then(|name| name.to_str())
        .expect("telemetry file name");
    assert!(file_name.starts_with("sales-smoke-"));
    assert!(
        dashed
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("jsonl"))
    );
}

#[test]
fn telemetry_requires_named_suite() {
    let suite = EvalSuite {
        version: 1,
        name: None,
        gate: None,
        defaults: EvalDefaults::default(),
        cases: Vec::new(),
        agent_cases: Vec::new(),
    };

    assert!(validate_telemetry_suite_name(&suite, false).is_ok());
    let error = validate_telemetry_suite_name(&suite, true)
        .expect_err("telemetry should require suite name");
    assert!(error.to_string().contains("non-empty name"));
}

#[test]
fn eval_gate_allows_latest_run_above_threshold() {
    let suite = gate_suite_file(Some(0.5));
    let suite_path = suite.path().display().to_string();
    let rows = vec![
        telemetry_row(
            "gated",
            &suite_path,
            "old",
            1,
            "case_old",
            "assertion",
            "fail",
        ),
        telemetry_row_with_assertion_count(
            "gated",
            &suite_path,
            "new",
            2,
            "case_a",
            "assertion_a",
            "pass",
            2,
        ),
        telemetry_row_with_assertion_count(
            "gated",
            &suite_path,
            "new",
            2,
            "case_b",
            "assertion_b",
            "pass",
            2,
        ),
    ];

    let report = build_eval_gate_report("gated", &rows).expect("gate report");

    assert!(report.allowed);
    assert!(!report.blocked);
    assert!(report.gate_configured);
    assert_eq!(report.threshold, Some(0.5));
    assert_eq!(report.total_evals, 2);
    assert_eq!(report.failed_evals, 0);
    assert!((report.pass_rate - 1.0).abs() < f64::EPSILON);
}

#[test]
fn eval_gate_blocks_latest_run_below_threshold_with_failed_ids() {
    let suite = gate_suite_file(Some(1.0));
    let suite_path = suite.path().display().to_string();
    let rows = vec![
        telemetry_row(
            "gated",
            &suite_path,
            "old",
            1,
            "case_old",
            "assertion",
            "pass",
        ),
        telemetry_row_with_assertion_count(
            "gated",
            &suite_path,
            "new",
            2,
            "case_a",
            "assertion_a",
            "pass",
            2,
        ),
        telemetry_row_with_assertion_count(
            "gated",
            &suite_path,
            "new",
            2,
            "case_b",
            "assertion_b",
            "fail",
            2,
        ),
    ];

    let report = build_eval_gate_report("gated", &rows).expect("gate report");

    assert!(!report.allowed);
    assert!(report.blocked);
    assert!((report.pass_rate - 0.5).abs() < f64::EPSILON);
    assert_eq!(report.failed_evals, 1);
    assert_eq!(report.failed_eval_ids, vec!["case_b::assertion_b"]);
    assert_eq!(report.failed_case_ids, vec!["case_b"]);
}

#[test]
fn eval_gate_uses_run_id_not_reused_output_dir() {
    let suite = gate_suite_file(Some(1.0));
    let suite_path = suite.path().display().to_string();
    let rows = vec![
        telemetry_row_with_run_id(
            "gated",
            &suite_path,
            "stable-output",
            1,
            "case_old",
            "assertion_old",
            "pass",
            "run-old",
        ),
        telemetry_row_with_run_id(
            "gated",
            &suite_path,
            "stable-output",
            2,
            "case_new",
            "assertion_new",
            "fail",
            "run-new",
        ),
    ];

    let report = build_eval_gate_report("gated", &rows).expect("gate report");

    assert!(!report.allowed);
    assert_eq!(report.total_evals, 1);
    assert_eq!(report.failed_eval_ids, vec!["case_new::assertion_new"]);
}

#[test]
fn eval_gate_blocks_partial_latest_run_after_retention() {
    let suite = gate_suite_file(Some(1.0));
    let suite_path = suite.path().display().to_string();
    let rows = vec![
        telemetry_row(
            "gated",
            &suite_path,
            "old",
            1,
            "case_old",
            "assertion_old",
            "fail",
        ),
        telemetry_row_with_assertion_count(
            "gated",
            &suite_path,
            "new",
            2,
            "case_new",
            "assertion_new",
            "pass",
            2,
        ),
    ];

    let report = build_eval_gate_report("gated", &rows).expect("gate report");

    assert!(!report.allowed);
    assert!(report.blocked);
    assert_eq!(report.total_evals, 1);
    assert!(report.message.contains("found 1 of 2"));
}

#[test]
fn eval_gate_blocks_filtered_latest_run_even_when_selected_case_passed() {
    let suite = gate_suite_file(Some(1.0));
    let suite_path = suite.path().display().to_string();
    let rows = vec![telemetry_row_with_case_counts(
        "gated",
        &suite_path,
        "latest",
        1,
        "case_a",
        "assertion_a",
        "pass",
        1,
        1,
        2,
    )];

    let report = build_eval_gate_report("gated", &rows).expect("gate report");

    assert!(!report.allowed);
    assert!(report.blocked);
    assert_eq!(report.total_evals, 1);
    assert!((report.pass_rate - 1.0).abs() < f64::EPSILON);
    assert!(report.message.contains("covers 1 of 2 suite cases"));
}

#[test]
fn eval_gate_blocks_latest_run_from_changed_suite_file() {
    let suite = gate_suite_file(Some(1.0));
    let suite_path = suite.path().display().to_string();
    let rows = vec![telemetry_row(
        "gated",
        &suite_path,
        "latest",
        1,
        "case_a",
        "assertion_a",
        "pass",
    )];
    std::fs::write(
        suite.path(),
        "version: 1\nname: gated\ngate:\n  threshold: 1.0\ncases: []\nagent_cases: []\n# changed\n",
    )
    .expect("change suite");

    let report = build_eval_gate_report("gated", &rows).expect("gate report");

    assert!(!report.allowed);
    assert!(report.blocked);
    assert!(report.message.contains("different suite file version"));
}

#[test]
fn eval_gate_blocks_legacy_telemetry_without_suite_hash() {
    let suite = gate_suite_file(Some(1.0));
    let suite_path = suite.path().display().to_string();
    let mut row = telemetry_row(
        "gated",
        &suite_path,
        "latest",
        1,
        "case_a",
        "assertion_a",
        "pass",
    );
    row.as_object_mut()
        .expect("telemetry object")
        .remove("suite_hash");
    let rows = vec![row];

    let report = build_eval_gate_report("gated", &rows).expect("gate report");

    assert!(!report.allowed);
    assert!(report.blocked);
    assert!(report.message.contains("suite_hash"));
}

#[test]
fn eval_gate_blocks_legacy_telemetry_without_run_assertion_count() {
    let suite = gate_suite_file(Some(1.0));
    let suite_path = suite.path().display().to_string();
    let mut row = telemetry_row(
        "gated",
        &suite_path,
        "latest",
        1,
        "case_a",
        "assertion_a",
        "pass",
    );
    row.as_object_mut()
        .expect("telemetry object")
        .remove("run_assertion_count");
    let rows = vec![row];

    let report = build_eval_gate_report("gated", &rows).expect("gate report");

    assert!(!report.allowed);
    assert!(report.blocked);
    assert!(report.message.contains("run_assertion_count"));
}

#[test]
fn eval_gate_missing_config_allows_with_explicit_signal() {
    let suite = gate_suite_file(None);
    let suite_path = suite.path().display().to_string();
    let rows = vec![telemetry_row(
        "ungated",
        &suite_path,
        "latest",
        1,
        "case_a",
        "assertion_a",
        "fail",
    )];

    let report = build_eval_gate_report("ungated", &rows).expect("gate report");

    assert!(report.allowed);
    assert!(!report.blocked);
    assert!(!report.gate_configured);
    assert_eq!(report.threshold, None);
    assert!(report.message.contains("allowed by default"));
}

#[test]
fn eval_gate_missing_config_allows_legacy_telemetry_without_run_assertion_count() {
    let suite = gate_suite_file(None);
    let suite_path = suite.path().display().to_string();
    let mut row = telemetry_row(
        "ungated",
        &suite_path,
        "latest",
        1,
        "case_a",
        "assertion_a",
        "pass",
    );
    row.as_object_mut()
        .expect("telemetry object")
        .remove("run_assertion_count");
    let rows = vec![row];

    let report = build_eval_gate_report("ungated", &rows).expect("gate report");

    assert!(report.allowed);
    assert!(!report.blocked);
    assert!(!report.gate_configured);
    assert!(report.message.contains("allowed by default"));
}

#[test]
fn eval_gate_missing_suite_config_blocks_with_actionable_message() {
    let rows = vec![telemetry_row(
        "gated",
        "target/does-not-exist/gated.yml",
        "latest",
        1,
        "case_a",
        "assertion_a",
        "pass",
    )];

    let report = build_eval_gate_report("gated", &rows).expect("gate report");

    assert!(!report.allowed);
    assert!(report.blocked);
    assert!(!report.gate_configured);
    assert!(report.message.contains("could not be read"));
}

#[test]
fn eval_gate_missing_telemetry_returns_actionable_message() {
    let report = build_eval_gate_report("missing", &[]).expect("gate report");

    assert!(!report.allowed);
    assert!(report.blocked);
    assert!(!report.gate_configured);
    assert_eq!(report.total_evals, 0);
    assert!(report.message.contains("--telemetry"));
}

#[test]
fn validate_suite_rejects_invalid_gate_threshold() {
    let suite = EvalSuite {
        version: 1,
        name: None,
        gate: Some(super::EvalGateConfig { threshold: 1.1 }),
        defaults: EvalDefaults::default(),
        cases: Vec::new(),
        agent_cases: Vec::new(),
    };
    let error = validate_suite(&suite).expect_err("invalid gate threshold should fail");
    assert!(error.to_string().contains("gate.threshold"));
}

#[test]
fn validate_suite_rejects_duplicate_agent_case_ids() {
    let suite = EvalSuite {
        version: 1,
        name: None,
        gate: None,
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
        gate: None,
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
        gate: None,
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
        gate: None,
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
        gate: None,
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

fn gate_suite_file(threshold: Option<f64>) -> NamedTempFile {
    let suite = NamedTempFile::new().expect("suite file");
    let gate = threshold.map_or_else(String::new, |threshold| {
        format!("gate:\n  threshold: {threshold}\n")
    });
    std::fs::write(
        suite.path(),
        format!("version: 1\nname: gated\n{gate}cases: []\nagent_cases: []\n"),
    )
    .expect("write suite");
    suite
}

fn telemetry_row(
    suite_name: &str,
    suite_path: &str,
    output_dir: &str,
    timestamp_ms: u64,
    case_id: &str,
    assertion_name: &str,
    status: &str,
) -> serde_json::Value {
    let run_id = format!("run-{timestamp_ms}");
    telemetry_row_with_run_id_and_count(
        suite_name,
        suite_path,
        output_dir,
        timestamp_ms,
        case_id,
        assertion_name,
        status,
        &run_id,
        1,
    )
}

#[allow(clippy::too_many_arguments)]
fn telemetry_row_with_run_id(
    suite_name: &str,
    suite_path: &str,
    output_dir: &str,
    timestamp_ms: u64,
    case_id: &str,
    assertion_name: &str,
    status: &str,
    run_id: &str,
) -> serde_json::Value {
    telemetry_row_with_run_id_and_count(
        suite_name,
        suite_path,
        output_dir,
        timestamp_ms,
        case_id,
        assertion_name,
        status,
        run_id,
        1,
    )
}

#[allow(clippy::too_many_arguments)]
fn telemetry_row_with_assertion_count(
    suite_name: &str,
    suite_path: &str,
    output_dir: &str,
    timestamp_ms: u64,
    case_id: &str,
    assertion_name: &str,
    status: &str,
    run_assertion_count: u64,
) -> serde_json::Value {
    let run_id = format!("run-{timestamp_ms}");
    telemetry_row_with_run_id_and_count(
        suite_name,
        suite_path,
        output_dir,
        timestamp_ms,
        case_id,
        assertion_name,
        status,
        &run_id,
        run_assertion_count,
    )
}

#[allow(clippy::too_many_arguments)]
fn telemetry_row_with_run_id_and_count(
    suite_name: &str,
    suite_path: &str,
    output_dir: &str,
    timestamp_ms: u64,
    case_id: &str,
    assertion_name: &str,
    status: &str,
    run_id: &str,
    run_assertion_count: u64,
) -> serde_json::Value {
    telemetry_row_with_run_id_and_counts(
        suite_name,
        suite_path,
        output_dir,
        timestamp_ms,
        case_id,
        assertion_name,
        status,
        run_id,
        run_assertion_count,
        1,
        1,
    )
}

#[allow(clippy::too_many_arguments)]
fn telemetry_row_with_case_counts(
    suite_name: &str,
    suite_path: &str,
    output_dir: &str,
    timestamp_ms: u64,
    case_id: &str,
    assertion_name: &str,
    status: &str,
    run_assertion_count: u64,
    run_case_count: u64,
    suite_case_count: u64,
) -> serde_json::Value {
    let run_id = format!("run-{timestamp_ms}");
    telemetry_row_with_run_id_and_counts(
        suite_name,
        suite_path,
        output_dir,
        timestamp_ms,
        case_id,
        assertion_name,
        status,
        &run_id,
        run_assertion_count,
        run_case_count,
        suite_case_count,
    )
}

#[allow(clippy::too_many_arguments)]
fn telemetry_row_with_run_id_and_counts(
    suite_name: &str,
    suite_path: &str,
    output_dir: &str,
    timestamp_ms: u64,
    case_id: &str,
    assertion_name: &str,
    status: &str,
    run_id: &str,
    run_assertion_count: u64,
    run_case_count: u64,
    suite_case_count: u64,
) -> serde_json::Value {
    let mut row = json!({
        "timestamp": format_utc_timestamp_millis(timestamp_ms),
        "timestamp_ms": timestamp_ms,
        "run_id": run_id,
        "run_case_count": run_case_count,
        "suite_case_count": suite_case_count,
        "run_assertion_count": run_assertion_count,
        "suite_name": suite_name,
        "suite_path": suite_path,
        "mode": "bridge",
        "case_id": case_id,
        "assertion_name": assertion_name,
        "status": status,
        "output_dir": output_dir
    });
    if let Ok(hash) = suite_file_hash(suite_path)
        && let Some(object) = row.as_object_mut()
    {
        object.insert("suite_hash".to_string(), json!(hash));
    }
    row
}
