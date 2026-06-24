use std::collections::BTreeMap;
use std::path::Path;

use serde_json::json;
use tempfile::{NamedTempFile, TempDir};

use super::{
    AgentCalledWith, AgentEntityRank, AgentExpected, AgentOrder, AgentSqlStructureExpected,
    AssertionResult, DateAnchor, EvalCardProvider, EvalCardRunContext, EvalCaseReport,
    EvalDefaults, EvalRunArgs, EvalSuite, EvalValidateArgs, FinalAnswerExpected, agent_prompt,
    apply_telemetry_retention, build_agent_eval_tool_response, build_eval_compare_tool_response,
    build_eval_comparison_report, build_eval_gate_report, build_eval_gate_tool_response,
    build_eval_history_tool_response, build_eval_init_tool_response, build_eval_validate_payload,
    build_eval_validate_tool_response, build_report, contains_rank_assertion,
    context_contains_assertion, context_field_equals_assertion, eval_case_telemetry_from_trace,
    format_utc_timestamp_millis, json_has_field_path, metadata_score_max_assertion,
    metadata_score_min_assertion, provider_invocation_evidence, read_tool_trace,
    recipe_rank_assertion, redact_provider_output_text, refresh_eval_card,
    render_eval_card_markdown, resolve_mcp_writable_path, run_eval_command, run_validate_command,
    safe_path_segment, score_agent_expectations, score_final_answer, selected_agent_cases,
    selected_bridge_cases, sql_structure_assertion, suite_file_hash, telemetry_grade_mode,
    telemetry_path_for_suite, telemetry_row_matches_since, tool_response_budget_assertion,
    tool_success_assertion, validate_since_date, validate_suite, validate_telemetry_suite_name,
};
use crate::params::{
    CompareEvalRunsParams, GetEvalGateParams, GetEvalHistoryParams, InitEvalSuiteParams,
    RunAgentEvalParams, ValidateEvalSuiteParams,
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
fn eval_provider_evidence_redacts_secret_bearing_text() {
    let raw = "\
stderr api_token=raw-token
Authorization: Bearer raw-auth
failed https://user:pass@example.com/manifest.json?token=raw-query
stored s3://bucket/secret/raw-s3/artifact.tar.gz?X-Amz-Signature=raw-signature";

    let redacted = redact_provider_output_text(raw);

    assert!(!redacted.contains("raw-token"));
    assert!(!redacted.contains("raw-auth"));
    assert!(!redacted.contains("raw-query"));
    assert!(!redacted.contains("raw-s3"));
    assert!(!redacted.contains("raw-signature"));
    assert!(!redacted.contains("user:pass"));
    assert!(redacted.contains("[REDACTED]"));
}

#[test]
fn final_answer_failure_evidence_redacts_secret_bearing_text() {
    let expected = FinalAnswerExpected {
        must_contain: vec!["missing phrase".to_string()],
        must_not_contain: Vec::new(),
    };

    let assertions = score_final_answer(
        &expected,
        "answer leaked https://user:pass@example.com/path?token=raw-query and api_token=raw-token",
    );
    let evidence = assertions[0].evidence.to_string();

    assert_eq!(assertions[0].status, "fail");
    assert!(!evidence.contains("raw-query"));
    assert!(!evidence.contains("raw-token"));
    assert!(!evidence.contains("user:pass"));
    assert!(evidence.contains("[REDACTED]"));
}

#[test]
fn provider_invocation_evidence_redacts_secret_bearing_args() {
    let invocation = super::provider::ProviderInvocation {
        command: "/tmp/token/raw-provider/provider".to_string(),
        args: vec![
            "--api-token=raw-token".to_string(),
            "--api-token".to_string(),
            "raw-split-token".to_string(),
            "--client_secret".to_string(),
            "raw-split-secret".to_string(),
            "https://user:pass@example.com/path?token=raw-query".to_string(),
            "s3://bucket/secret/raw-artifact/output.json?X-Amz-Signature=raw-signature".to_string(),
        ],
    };

    let evidence = provider_invocation_evidence(&invocation).to_string();

    assert!(!evidence.contains("raw-provider"));
    assert!(!evidence.contains("raw-token"));
    assert!(!evidence.contains("raw-split-token"));
    assert!(!evidence.contains("raw-split-secret"));
    assert!(!evidence.contains("raw-query"));
    assert!(!evidence.contains("raw-artifact"));
    assert!(!evidence.contains("raw-signature"));
    assert!(!evidence.contains("user:pass"));
    assert!(evidence.contains("[REDACTED]"));
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
fn agent_order_fails_when_sql_precedes_semantic_discovery() {
    let expected = AgentExpected {
        must_call: vec!["search_indicator".to_string(), "execute_sql".to_string()],
        ordered: vec![AgentOrder {
            before: "execute_sql".to_string(),
            must_have_called: vec!["search_indicator".to_string()],
        }],
        ..AgentExpected::default()
    };
    let trace = vec![
        json!({"tool": "execute_sql"}),
        json!({"tool": "search_indicator"}),
    ];

    let results = score_agent_expectations(&expected, &trace, "");

    assert_eq!(
        results
            .iter()
            .filter(|result| result.name.starts_with("must_call:"))
            .filter(|result| result.status == "pass")
            .count(),
        2
    );
    let order_result = results
        .iter()
        .find(|result| result.name == "order:execute_sql")
        .expect("order assertion");
    assert_eq!(order_result.status, "fail");
    assert_eq!(
        order_result.evidence["observed_tools"],
        json!(["execute_sql", "search_indicator"])
    );
}

#[test]
fn sql_structure_assertion_passes_when_only_literals_differ() {
    let result = sql_structure_assertion(
        "sql_structure",
        "
            select o.country, sum(o.amount) as revenue
            from analytics.orders o
            where o.order_date between '2026-03-01' and '2026-03-31'
              and o.country = 'US'
              and o.amount > 100
            group by o.country
        ",
        "
            select orders.country, sum(orders.amount) as revenue
            from analytics.orders
            where orders.order_date between '2024-01-01' and '2024-01-31'
              and orders.country = 'GB'
              and orders.amount > 250
            group by orders.country
        ",
    );

    assert_eq!(result.status, "pass", "{result:#?}");
    assert_eq!(result.evidence["grade_mode"], "query_structure");
}

#[test]
fn sql_structure_assertion_fails_with_missing_filter_diff() {
    let result = sql_structure_assertion(
        "sql_structure",
        "select country, sum(amount) from analytics.orders group by country",
        "
            select country, sum(amount)
            from analytics.orders
            where country = 'US'
            group by country
        ",
    );

    assert_eq!(result.status, "fail");
    assert!(result.message.contains("WHERE"));
    assert_eq!(
        result.evidence["diff"]["missing_filters"],
        json!(["country = ?"])
    );
}

#[test]
fn sql_structure_assertion_fails_with_wrong_table_diff() {
    let result = sql_structure_assertion(
        "sql_structure",
        "select country, sum(amount) from analytics.customers group by country",
        "select country, sum(amount) from analytics.orders group by country",
    );

    assert_eq!(result.status, "fail");
    assert!(result.message.contains("FROM"));
    assert_eq!(
        result.evidence["diff"]["missing_tables"],
        json!(["analytics.orders"])
    );
    assert_eq!(
        result.evidence["diff"]["unexpected_tables"],
        json!(["analytics.customers"])
    );
}

#[test]
fn agent_sql_structure_scores_execute_sql_trace_without_raw_sql() {
    let expected = AgentExpected {
        sql_structures: vec![AgentSqlStructureExpected {
            tool: "execute_sql".to_string(),
            expected_sql: "
                select country, sum(amount) as revenue
                from analytics.orders
                where order_date between '2024-01-01' and '2024-01-31'
                  and country = 'GB'
                group by country
            "
            .to_string(),
        }],
        ..AgentExpected::default()
    };
    let actual_sql = "
        select o.country, sum(o.amount) as revenue
        from analytics.orders o
        where o.order_date between '2026-03-01' and '2026-03-31'
          and o.country = 'US'
        group by o.country
    ";
    let trace = vec![json!({
        "tool": "execute_sql",
        "params_summary": {
            "keys": ["statement"],
            "statement_structure": crate::utils::sql_structure::sql_structure_summary_json(actual_sql)
                .expect("structure")
        }
    })];

    let results = score_agent_expectations(&expected, &trace, "");

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].status, "pass", "{results:#?}");
    assert!(!results[0].evidence.to_string().contains("2026-03-01"));
    assert!(!trace[0].to_string().contains("select o.country"));
}

#[test]
fn query_structure_telemetry_grade_mode_is_explicit() {
    assert_eq!(
        telemetry_grade_mode("agent", "sql_structure:execute_sql"),
        "query_structure"
    );
    assert_eq!(
        telemetry_grade_mode("agent", "must_call:search"),
        "provider_trace"
    );
    assert_eq!(
        telemetry_grade_mode("bridge", "search_rank"),
        "deterministic"
    );
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
fn metadata_score_bounds_check_min_and_max() {
    let response = json!({
        "data": {
            "overall_score": 12.0
        }
    });

    assert_eq!(metadata_score_min_assertion(&response, 10.0).status, "pass");
    assert_eq!(metadata_score_min_assertion(&response, 20.0).status, "fail");
    assert_eq!(metadata_score_max_assertion(&response, 20.0).status, "pass");
    assert_eq!(metadata_score_max_assertion(&response, 10.0).status, "fail");
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
fn eval_card_summarizes_bridge_report_with_missing_telemetry() {
    let suite = eval_card_suite("card-bridge", Some(0.9), false);
    let report = build_report(
        &suite,
        "bridge",
        "out/eval-card".to_string(),
        0.9,
        vec![EvalCaseReport::new(
            "bridge_case".to_string(),
            Some("Find canonical orders".to_string()),
            vec![AssertionResult::pass(
                "search_rank",
                "ranked first",
                json!({}),
            )],
            None,
        )],
    );

    assert_eq!(report.eval_card.schema_version, "eval_card.v1");
    assert_eq!(report.eval_card.suite_name, "card-bridge");
    assert_eq!(report.eval_card.mode, "bridge");
    assert_eq!(report.eval_card.bridge_case_count, 1);
    assert_eq!(report.eval_card.agent_case_count, 0);
    assert_eq!(report.eval_card.run_status, "pass");
    assert!((report.eval_card.pass_rate - 1.0).abs() < f64::EPSILON);
    assert_eq!(report.eval_card.telemetry.status, "missing");
    assert_eq!(report.eval_card.gate.status, "missing_telemetry");
    assert_eq!(
        report.eval_card.manifest_scope.declared,
        "synthetic starter manifest"
    );
    assert_eq!(
        report.eval_card.known_gaps,
        vec!["does not cover live warehouse freshness".to_string()]
    );
}

#[test]
fn eval_card_includes_agent_provider_metadata() {
    let suite = eval_card_suite("card-agent", None, true);
    let mut report = build_report(
        &suite,
        "agent",
        "out/agent-card".to_string(),
        1.0,
        vec![EvalCaseReport::new(
            "agent_case".to_string(),
            Some("Use Nova to answer the task".to_string()),
            vec![AssertionResult::pass(
                "must_call:search_indicator",
                "required tool was called",
                json!({}),
            )],
            None,
        )],
    );
    refresh_eval_card(
        &mut report,
        &suite,
        &EvalCardRunContext {
            manifest_source: Some("tests/fixtures/starter_eval_manifest.json".to_string()),
            provider: Some(EvalCardProvider {
                provider: "opencode".to_string(),
                command_preset: "opencode".to_string(),
                model: Some("opencode/deepseek-v4-flash-free".to_string()),
            }),
            ..EvalCardRunContext::default()
        },
    );

    let provider = report
        .eval_card
        .provider
        .as_ref()
        .expect("provider metadata");
    assert_eq!(provider.provider, "opencode");
    assert_eq!(provider.command_preset, "opencode");
    assert_eq!(
        provider.model.as_deref(),
        Some("opencode/deepseek-v4-flash-free")
    );
    assert_eq!(
        report.eval_card.manifest_scope.manifest_source.as_deref(),
        Some("tests/fixtures/starter_eval_manifest.json")
    );
}

#[test]
fn eval_card_uses_latest_telemetry_gate_when_available() {
    let suite_name = "card-gated";
    let suite = gate_suite_file(Some(1.0));
    let suite_hash = suite_file_hash(&suite.path().display().to_string()).expect("suite hash");
    let telemetry_path = telemetry_path_for_suite(suite_name);
    if let Some(parent) = telemetry_path.parent() {
        std::fs::create_dir_all(parent).expect("telemetry dir");
    }
    let rows = [
        telemetry_row_with_suite_hash(
            suite_name,
            &suite.path().display().to_string(),
            &suite_hash,
            "latest",
            2,
            "case_a",
            "assertion_a",
            "pass",
        ),
        telemetry_row_with_suite_hash(
            suite_name,
            &suite.path().display().to_string(),
            &suite_hash,
            "latest",
            2,
            "case_b",
            "assertion_b",
            "fail",
        ),
    ];
    let mut body = rows
        .iter()
        .map(serde_json::to_string)
        .collect::<Result<Vec<_>, _>>()
        .expect("serialize telemetry")
        .join("\n");
    body.push('\n');
    std::fs::write(&telemetry_path, body).expect("write telemetry");

    let suite = eval_card_suite(suite_name, Some(1.0), false);
    let mut report = build_report(
        &suite,
        "bridge",
        "out/card-gated".to_string(),
        1.0,
        vec![EvalCaseReport::new(
            "case_a".to_string(),
            None,
            vec![AssertionResult::pass("assertion_a", "ok", json!({}))],
            None,
        )],
    );
    refresh_eval_card(
        &mut report,
        &suite,
        &EvalCardRunContext {
            telemetry_requested: true,
            ..EvalCardRunContext::default()
        },
    );

    assert_eq!(report.eval_card.telemetry.status, "latest");
    assert_eq!(report.eval_card.telemetry.row_count, 2);
    assert_eq!(report.eval_card.gate.status, "fail");
    assert!(report.eval_card.gate.configured);
    assert_eq!(report.eval_card.gate.threshold, Some(1.0));
    assert_eq!(report.eval_card.gate.total_evals, Some(2));
    assert_eq!(report.eval_card.gate.failed_evals, Some(1));

    std::fs::remove_file(telemetry_path).expect("remove telemetry");
}

#[test]
fn eval_card_represents_no_gate_with_latest_telemetry() {
    let suite = gate_suite_file(None);
    let suite_name = "card-no-gate";
    let suite_hash = suite_file_hash(&suite.path().display().to_string()).expect("suite hash");
    let telemetry_path = telemetry_path_for_suite(suite_name);
    if let Some(parent) = telemetry_path.parent() {
        std::fs::create_dir_all(parent).expect("telemetry dir");
    }
    let row = telemetry_row_with_suite_hash(
        suite_name,
        &suite.path().display().to_string(),
        &suite_hash,
        "latest",
        1,
        "case_a",
        "assertion_a",
        "pass",
    );
    std::fs::write(
        &telemetry_path,
        format!("{}\n", serde_json::to_string(&row).expect("row JSON")),
    )
    .expect("write telemetry");

    let suite = eval_card_suite(suite_name, None, false);
    let report = build_report(
        &suite,
        "bridge",
        "out/card-no-gate".to_string(),
        1.0,
        vec![EvalCaseReport::new(
            "case_a".to_string(),
            None,
            vec![AssertionResult::pass("assertion_a", "ok", json!({}))],
            None,
        )],
    );

    assert_eq!(report.eval_card.telemetry.status, "latest");
    assert_eq!(report.eval_card.gate.status, "not_configured");
    assert!(!report.eval_card.gate.configured);
    assert!(report.eval_card.gate.message.contains("allowed by default"));

    std::fs::remove_file(telemetry_path).expect("remove telemetry");
}

#[test]
fn eval_card_markdown_is_pr_ready() {
    let suite = eval_card_suite("card-markdown", None, false);
    let report = build_report(
        &suite,
        "bridge",
        "out/card-markdown".to_string(),
        1.0,
        vec![EvalCaseReport::new(
            "case".to_string(),
            None,
            vec![AssertionResult::pass(
                "tool_success:search",
                "ok",
                json!({}),
            )],
            None,
        )],
    );

    let markdown = render_eval_card_markdown(&report.eval_card);

    assert!(markdown.starts_with("# Nova Eval Card"));
    assert!(markdown.contains("Pass rate: `100.0%`"));
    assert!(markdown.contains("Gate status: `missing_telemetry`"));
    assert!(markdown.contains("Known gaps:"));
    assert!(markdown.contains("does not cover live warehouse freshness"));
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
        purpose: None,
        manifest_scope: None,
        known_gaps: Vec::new(),
        gate: None,
        date_anchor: DateAnchor::default(),
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
        purpose: None,
        manifest_scope: None,
        known_gaps: Vec::new(),
        gate: Some(super::EvalGateConfig { threshold: 1.1 }),
        date_anchor: DateAnchor::default(),
        defaults: EvalDefaults::default(),
        cases: Vec::new(),
        agent_cases: Vec::new(),
    };
    let error = validate_suite(&suite).expect_err("invalid gate threshold should fail");
    assert!(error.to_string().contains("gate.threshold"));
}

#[test]
fn eval_validate_accepts_snapshot_date_anchor_fields() {
    let suite = NamedTempFile::new().expect("suite file");
    std::fs::write(
        suite.path(),
        r#"
version: 1
name: date-anchor-smoke
snapshot_date: "2026-03-31"
date_field: order_date
cases:
  - id: anchored-bridge
    date_range_start: "2026-03-01"
    date_range_end: "2026-03-31"
    assertions:
      - type: tool_success
        tool: search
        params: {}
agent_cases:
  - id: anchored-agent
    task: Compare revenue last month.
    expected: {}
"#,
    )
    .expect("write suite");

    let payload =
        build_eval_validate_payload(&suite.path().display().to_string()).expect("valid suite");
    assert_eq!(payload["date_anchor"]["snapshot_date"], json!("2026-03-31"));
    assert_eq!(payload["date_anchor"]["date_field"], json!("order_date"));
    assert_eq!(payload["date_anchor_case_count"], json!(2));
}

#[test]
fn eval_validate_accepts_inherited_date_anchor_fields() {
    let suite = NamedTempFile::new().expect("suite file");
    std::fs::write(
        suite.path(),
        r#"
version: 1
name: inherited-date-anchor-smoke
date_range_start: "2026-03-01"
date_range_end: "2026-03-31"
date_field: order_date
cases:
  - id: inherited-range-end
    date_range_start: "2026-03-15"
    assertions:
      - type: tool_success
        tool: search
        params: {}
agent_cases:
  - id: inherited-field
    task: Compare revenue last month.
    snapshot_date: "2026-03-31"
    expected: {}
"#,
    )
    .expect("write suite");

    let payload =
        build_eval_validate_payload(&suite.path().display().to_string()).expect("valid suite");
    assert_eq!(payload["date_anchor_case_count"], json!(2));
}

#[test]
fn eval_validate_rejects_invalid_snapshot_date_anchor() {
    let suite = NamedTempFile::new().expect("suite file");
    std::fs::write(
        suite.path(),
        r#"
version: 1
name: bad-date-anchor
snapshot_date: "2026-02-30"
cases: []
agent_cases: []
"#,
    )
    .expect("write suite");

    let error = build_eval_validate_payload(&suite.path().display().to_string())
        .expect_err("invalid date should fail");
    let error = error.to_string();
    assert!(error.contains("snapshot_date"));
    assert!(error.contains("YYYY-MM-DD"));
}

#[test]
fn eval_validate_rejects_incomplete_date_range_anchor() {
    let suite = EvalSuite {
        version: 1,
        name: None,
        purpose: None,
        manifest_scope: None,
        known_gaps: Vec::new(),
        gate: None,
        date_anchor: DateAnchor::default(),
        defaults: EvalDefaults::default(),
        cases: vec![super::EvalCase {
            id: "range".to_string(),
            question: None,
            persona: None,
            date_anchor: DateAnchor {
                date_range_start: Some("2026-03-01".to_string()),
                ..DateAnchor::default()
            },
            assertions: vec![super::EvalAssertion::ToolSuccess {
                tool: "search".to_string(),
                params: json!({}),
            }],
        }],
        agent_cases: Vec::new(),
    };

    let error = validate_suite(&suite).expect_err("incomplete date range should fail");
    assert!(error.to_string().contains("date_range_end"));
}

#[test]
fn agent_prompt_includes_date_anchor_section() {
    let case = super::AgentCase {
        id: "agent".to_string(),
        task: "Compare gross revenue last month.".to_string(),
        date_anchor: DateAnchor::default(),
        expected: AgentExpected::default(),
    };
    let anchor = DateAnchor {
        snapshot_date: Some("2026-03-31".to_string()),
        date_range_start: Some("2026-03-01".to_string()),
        date_range_end: Some("2026-03-31".to_string()),
        date_field: Some("order_date".to_string()),
    };

    let prompt = agent_prompt(&case, Some(&anchor), None);
    assert!(prompt.contains("Date anchor:"));
    assert!(prompt.contains("snapshot_date: 2026-03-31"));
    assert!(prompt.contains("date_range: 2026-03-01 to 2026-03-31"));
    assert!(prompt.contains("date_field: order_date"));
    assert!(prompt.contains("Do not reinterpret them using today's date"));
}

#[test]
fn reviewer_agent_prompt_uses_review_contract() {
    let case = super::AgentCase {
        id: "reviewer".to_string(),
        task: "Review this draft for semantic bypass.".to_string(),
        date_anchor: DateAnchor::default(),
        expected: AgentExpected::default(),
    };

    let prompt = agent_prompt(&case, None, Some("reviewer"));

    assert!(prompt.contains("dbt-nova reviewer-agent eval"));
    assert!(prompt.contains("Do not execute SQL"));
    assert!(prompt.contains("semantic-layer bypass"));
    assert!(prompt.contains("stale or unknown freshness"));
    assert!(prompt.contains("verdict"));
    assert!(!prompt.contains("Use Nova discovery and execution tools directly"));
}

#[test]
fn validate_suite_rejects_duplicate_agent_case_ids() {
    let suite = EvalSuite {
        version: 1,
        name: None,
        purpose: None,
        manifest_scope: None,
        known_gaps: Vec::new(),
        gate: None,
        date_anchor: DateAnchor::default(),
        defaults: EvalDefaults::default(),
        cases: Vec::new(),
        agent_cases: vec![
            super::AgentCase {
                id: "same".to_string(),
                task: "one".to_string(),
                date_anchor: DateAnchor::default(),
                expected: AgentExpected::default(),
            },
            super::AgentCase {
                id: "same".to_string(),
                task: "two".to_string(),
                date_anchor: DateAnchor::default(),
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
        purpose: None,
        manifest_scope: None,
        known_gaps: Vec::new(),
        gate: None,
        date_anchor: DateAnchor::default(),
        defaults: EvalDefaults::default(),
        cases: Vec::new(),
        agent_cases: vec![
            super::AgentCase {
                id: "a/b".to_string(),
                task: "one".to_string(),
                date_anchor: DateAnchor::default(),
                expected: AgentExpected::default(),
            },
            super::AgentCase {
                id: "a b".to_string(),
                task: "two".to_string(),
                date_anchor: DateAnchor::default(),
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
        purpose: None,
        manifest_scope: None,
        known_gaps: Vec::new(),
        gate: None,
        date_anchor: DateAnchor::default(),
        defaults: EvalDefaults::default(),
        cases: Vec::new(),
        agent_cases: vec![
            super::AgentCase {
                id: "RevenueFlow".to_string(),
                task: "one".to_string(),
                date_anchor: DateAnchor::default(),
                expected: AgentExpected::default(),
            },
            super::AgentCase {
                id: "revenueflow".to_string(),
                task: "two".to_string(),
                date_anchor: DateAnchor::default(),
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
        purpose: None,
        manifest_scope: None,
        known_gaps: Vec::new(),
        gate: None,
        date_anchor: DateAnchor::default(),
        defaults: EvalDefaults::default(),
        cases: vec![super::EvalCase {
            id: "columns".to_string(),
            question: None,
            persona: None,
            date_anchor: DateAnchor::default(),
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
        purpose: None,
        manifest_scope: None,
        known_gaps: Vec::new(),
        gate: None,
        date_anchor: DateAnchor::default(),
        defaults: EvalDefaults::default(),
        cases: Vec::new(),
        agent_cases: vec![super::AgentCase {
            id: "agent".to_string(),
            task: "use nova".to_string(),
            date_anchor: DateAnchor::default(),
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
        date_anchor: DateAnchor::default(),
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
            date_anchor: DateAnchor::default(),
            expected: AgentExpected::default(),
        },
        super::AgentCase {
            id: "two".to_string(),
            task: "task two".to_string(),
            date_anchor: DateAnchor::default(),
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

#[test]
fn eval_validate_tool_response_returns_report_contract_and_policy() {
    let root = std::env::current_dir()
        .expect("cwd")
        .canonicalize()
        .expect("canonical cwd");
    let temp_dir = TempDir::new_in(&root).expect("temp suite dir");
    let suite_path = temp_dir.path().join("suite.yml");
    std::fs::write(
        &suite_path,
        r"
version: 1
name: validate-tool-smoke
cases:
  - id: one
    assertions:
      - type: tool_success
        tool: search
        params: {}
",
    )
    .expect("write suite");

    let response = build_eval_validate_tool_response(&ValidateEvalSuiteParams {
        suite: suite_path.display().to_string(),
    })
    .expect("validate tool response");

    assert_eq!(response["success"], json!(true));
    assert_eq!(response["count"], json!(1));
    assert_eq!(response["data"]["valid"], json!(true));
    assert_eq!(response["data"]["suite_name"], json!("validate-tool-smoke"));
    assert_eq!(response["data"]["bridge_case_count"], json!(1));
    assert_eq!(
        response["data"]["safety_policy"]["local_paths_must_stay_under_filesystem_root"],
        json!(true)
    );
}

#[test]
fn eval_gate_and_history_tool_responses_return_cli_report_data() {
    let suite_name = "mcp-history-smoke";
    let root = std::env::current_dir()
        .expect("cwd")
        .canonicalize()
        .expect("canonical cwd");
    let suite_dir = TempDir::new_in(&root).expect("temp suite dir");
    let suite_path = suite_dir.path().join("suite.yml");
    std::fs::write(
        &suite_path,
        format!(
            "version: 1\nname: {suite_name}\ngate:\n  threshold: 1.0\ncases: []\nagent_cases: []\n"
        ),
    )
    .expect("write suite");
    let suite_hash = suite_file_hash(&suite_path.display().to_string()).expect("suite hash");
    let telemetry_path = telemetry_path_for_suite(suite_name);
    if let Some(parent) = telemetry_path.parent() {
        std::fs::create_dir_all(parent).expect("telemetry dir");
    }
    let rows = [
        json!({
            "timestamp": "2026-05-31T23:59:59.000Z",
            "timestamp_ms": 1,
            "suite_name": suite_name,
            "suite_path": suite_path.display().to_string(),
            "suite_hash": suite_hash,
            "run_id": "run-old",
            "case_id": "old",
            "assertion_id": "old::assertion",
            "status": "pass",
            "run_case_count": 1,
            "suite_case_count": 1,
            "run_assertion_count": 1,
            "gate_threshold": 1.0
        }),
        json!({
            "timestamp": "2026-06-02T00:00:00.000Z",
            "timestamp_ms": 2,
            "suite_name": suite_name,
            "suite_path": suite_path.display().to_string(),
            "suite_hash": suite_hash,
            "run_id": "run-new",
            "case_id": "new",
            "assertion_id": "new::assertion",
            "status": "pass",
            "run_case_count": 1,
            "suite_case_count": 1,
            "run_assertion_count": 1,
            "gate_threshold": 1.0
        }),
    ];
    let mut body = rows
        .iter()
        .map(serde_json::to_string)
        .collect::<Result<Vec<_>, _>>()
        .expect("serialize telemetry")
        .join("\n");
    body.push('\n');
    std::fs::write(&telemetry_path, body).expect("write telemetry");

    let gate = build_eval_gate_tool_response(&GetEvalGateParams {
        suite: suite_name.to_string(),
    })
    .expect("gate tool response");
    assert_eq!(gate["success"], json!(true));
    assert_eq!(gate["data"]["suite_name"], json!(suite_name));
    assert_eq!(gate["data"]["allowed"], json!(true));
    assert_eq!(gate["data"]["total_evals"], json!(1));

    let history = build_eval_history_tool_response(&GetEvalHistoryParams {
        suite: suite_name.to_string(),
        since: "2026-06-01".to_string(),
    })
    .expect("history tool response");
    assert_eq!(history["success"], json!(true));
    assert_eq!(history["count"], json!(1));
    assert_eq!(history["data"]["row_count"], json!(1));
    assert_eq!(history["data"]["rows"][0]["case_id"], json!("new"));
    assert_eq!(
        history["data"]["safety_policy"]["eval_run_enabled_env"],
        json!("DBT_NOVA_MCP_ENABLE_EVAL_RUN")
    );
    assert_eq!(
        history["data"]["safety_policy"]["raw_provider_logs_enabled_env"],
        json!("DBT_NOVA_EVAL_UNSAFE_WRITE_RAW_PROVIDER_LOGS")
    );
    assert_eq!(
        history["data"]["safety_policy"]["provider_logs_redacted_by_default"],
        json!(true)
    );

    std::fs::remove_file(telemetry_path).expect("remove telemetry");
}

#[test]
fn eval_gate_tool_rejects_telemetry_suite_paths_outside_root() {
    let suite_name = "mcp-outside-suite-path";
    let root = std::env::current_dir()
        .expect("cwd")
        .canonicalize()
        .expect("canonical cwd");
    let outside_path = root
        .parent()
        .expect("repo parent")
        .join("outside-mcp-suite.yml");
    let telemetry_path = telemetry_path_for_suite(suite_name);
    if let Some(parent) = telemetry_path.parent() {
        std::fs::create_dir_all(parent).expect("telemetry dir");
    }
    let row = json!({
        "timestamp": "2026-06-02T00:00:00.000Z",
        "timestamp_ms": 1,
        "suite_name": suite_name,
        "suite_path": outside_path.display().to_string(),
        "suite_hash": "hash",
        "run_id": "run-outside",
        "case_id": "case",
        "assertion_id": "case::assertion",
        "status": "pass",
        "run_case_count": 1,
        "suite_case_count": 1,
        "run_assertion_count": 1,
        "gate_threshold": 1.0
    });
    std::fs::write(
        &telemetry_path,
        format!("{}\n", serde_json::to_string(&row).expect("row JSON")),
    )
    .expect("write telemetry");

    let error = build_eval_gate_tool_response(&GetEvalGateParams {
        suite: suite_name.to_string(),
    })
    .expect_err("outside suite path should fail");
    assert!(
        error
            .to_string()
            .contains("outside server working directory")
    );

    std::fs::remove_file(telemetry_path).expect("remove telemetry");
}

#[test]
fn eval_history_tool_rejects_telemetry_suite_paths_outside_root() {
    let suite_name = "mcp-history-outside-suite-path";
    let root = std::env::current_dir()
        .expect("cwd")
        .canonicalize()
        .expect("canonical cwd");
    let outside_path = root
        .parent()
        .expect("repo parent")
        .join("outside-mcp-history-suite.yml");
    let telemetry_path = telemetry_path_for_suite(suite_name);
    if let Some(parent) = telemetry_path.parent() {
        std::fs::create_dir_all(parent).expect("telemetry dir");
    }
    let row = json!({
        "timestamp": "2026-06-02T00:00:00.000Z",
        "timestamp_ms": 1,
        "suite_name": suite_name,
        "suite_path": outside_path.display().to_string(),
        "suite_hash": "hash",
        "run_id": "run-outside",
        "case_id": "case",
        "assertion_id": "case::assertion",
        "status": "pass",
        "run_case_count": 1,
        "suite_case_count": 1,
        "run_assertion_count": 1,
        "gate_threshold": 1.0
    });
    std::fs::write(
        &telemetry_path,
        format!("{}\n", serde_json::to_string(&row).expect("row JSON")),
    )
    .expect("write telemetry");

    let error = build_eval_history_tool_response(&GetEvalHistoryParams {
        suite: suite_name.to_string(),
        since: "2026-06-01".to_string(),
    })
    .expect_err("outside suite path should fail");
    assert!(
        error
            .to_string()
            .contains("outside server working directory")
    );

    std::fs::remove_file(telemetry_path).expect("remove telemetry");
}

#[tokio::test]
async fn eval_write_and_agent_execution_tools_reject_without_mcp_opt_in() {
    let init_error = build_eval_init_tool_response(&InitEvalSuiteParams {
        persona: Some("analyst".to_string()),
        out: "evals/mcp-disabled.yml".to_string(),
        force: false,
    })
    .expect_err("init should require opt-in");
    assert!(
        init_error
            .to_string()
            .contains("DBT_NOVA_MCP_ENABLE_EVAL_WRITES=1")
    );

    let agent_error = build_agent_eval_tool_response(&RunAgentEvalParams {
        suite: "evals/starter.yml".to_string(),
        ..RunAgentEvalParams::default()
    })
    .await
    .expect_err("agent eval should require opt-in");
    assert!(
        agent_error
            .to_string()
            .contains("DBT_NOVA_MCP_ENABLE_AGENT_EVAL=1")
    );
}

#[test]
fn eval_mcp_writable_paths_reject_absolute_parent_traversal() {
    let root = std::env::current_dir()
        .expect("cwd")
        .canonicalize()
        .expect("canonical cwd");
    let unsafe_path = root.join("evals").join("..").join("mcp-disabled.yml");
    let error =
        resolve_mcp_writable_path(&unsafe_path.display().to_string(), "out").expect_err("unsafe");

    assert!(
        error
            .to_string()
            .contains("must stay under the server working directory")
    );
}

#[tokio::test]
async fn bridge_eval_writes_result_artifacts() {
    let temp_dir = TempDir::new().expect("output dir");
    let suite_path = temp_dir.path().join("suite.yml");
    std::fs::write(
        &suite_path,
        r#"
version: 1
name: bridge-date-anchor-smoke
snapshot_date: "2026-03-31"
date_field: order_date
cases:
  - id: orders-search
    date_range_start: "2026-03-01"
    date_range_end: "2026-03-31"
    assertions:
      - type: search_rank
        query: orders
        expected_unique_id: model.nova_test.fct__orders
        max_rank: 5
"#,
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
        telemetry: true,
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
    assert!(output_dir.join("card.md").exists());
    assert!(output_dir.join("report.md").exists());
    assert!(output_dir.join("suite.yml").exists());
    let results = std::fs::read_to_string(output_dir.join("results.json")).expect("results json");
    let results: serde_json::Value = serde_json::from_str(&results).expect("parse results");
    assert_eq!(
        results["eval_card"]["schema_version"],
        json!("eval_card.v1")
    );
    assert_eq!(results["eval_card"]["mode"], json!("bridge"));
    assert_eq!(
        results["eval_card"]["date_anchor"]["snapshot_date"],
        json!("2026-03-31")
    );
    assert_eq!(
        results["cases"][0]["date_anchor"]["snapshot_date"],
        json!("2026-03-31")
    );
    assert_eq!(
        results["cases"][0]["date_anchor"]["date_range_start"],
        json!("2026-03-01")
    );
    assert_eq!(
        results["cases"][0]["date_anchor"]["date_range_end"],
        json!("2026-03-31")
    );
    assert_eq!(
        results["cases"][0]["date_anchor"]["date_field"],
        json!("order_date")
    );
    let report_md = std::fs::read_to_string(output_dir.join("report.md")).expect("report md");
    assert!(report_md.contains("Suite date anchor"));
    assert!(report_md.contains("date_range: `2026-03-01` to `2026-03-31`"));

    let telemetry_path = telemetry_path_for_suite("bridge-date-anchor-smoke");
    let telemetry = std::fs::read_to_string(&telemetry_path).expect("telemetry jsonl");
    let latest = telemetry.lines().last().expect("telemetry line");
    let latest: serde_json::Value = serde_json::from_str(latest).expect("telemetry json");
    assert_eq!(latest["snapshot_date"], json!("2026-03-31"));
    assert_eq!(latest["date_range_start"], json!("2026-03-01"));
    assert_eq!(latest["date_range_end"], json!("2026-03-31"));
    assert_eq!(latest["date_field"], json!("order_date"));
    std::fs::remove_file(telemetry_path).expect("remove telemetry");
}

#[test]
fn eval_comparison_reports_before_after_case_deltas() {
    let temp_dir = TempDir::new().expect("comparison dir");
    let before_dir = temp_dir.path().join("before");
    let after_dir = temp_dir.path().join("after");
    write_eval_results(
        &before_dir,
        &json!({
            "suite_name": "ablation-smoke",
            "version": 1,
            "mode": "bridge",
            "output_dir": before_dir.display().to_string(),
            "assertion_count": 2,
            "pass_count": 1,
            "fail_count": 1,
            "error_count": 0,
            "pass_rate": 0.5,
            "gate_status": "fail",
            "cases": [
                {"id": "kept_pass", "pass_count": 1, "fail_count": 0, "error_count": 0, "assertions": [{"name": "search_rank", "status": "pass"}]},
                {"id": "fixed_case", "pass_count": 0, "fail_count": 1, "error_count": 0, "assertions": [{"name": "context_contains", "status": "fail"}]}
            ]
        }),
    );
    write_eval_results(
        &after_dir,
        &json!({
            "suite_name": "ablation-smoke",
            "version": 1,
            "mode": "bridge",
            "output_dir": after_dir.display().to_string(),
            "assertion_count": 3,
            "pass_count": 2,
            "fail_count": 1,
            "error_count": 0,
            "pass_rate": 0.666_666_666_7,
            "gate_status": "fail",
            "cases": [
                {"id": "kept_pass", "pass_count": 1, "fail_count": 0, "error_count": 0, "assertions": [{"name": "search_rank", "status": "pass"}]},
                {"id": "fixed_case", "pass_count": 1, "fail_count": 0, "error_count": 0, "assertions": [{"name": "context_contains", "status": "pass"}]},
                {"id": "new_failure", "pass_count": 0, "fail_count": 1, "error_count": 0, "assertions": [{"name": "tool_success", "status": "fail"}]}
            ]
        }),
    );

    let report = build_eval_comparison_report(
        &before_dir.join("results.json"),
        &after_dir.join("results.json"),
        None,
    )
    .expect("comparison report");

    assert_eq!(
        report.delta.newly_passing_cases,
        vec!["fixed_case".to_string()]
    );
    assert_eq!(
        report.delta.newly_failing_cases,
        vec!["new_failure".to_string()]
    );
    assert_eq!(report.delta.assertion_count, 1);
    assert!(
        report
            .markdown
            .contains("| Pass rate | 50.0% | 66.7% | +16.7 pp |")
    );
    assert!(report.markdown.contains("`fixed_case`"));
    assert!(report.markdown.contains("`new_failure`"));
}

#[test]
fn eval_comparison_includes_agent_trace_metric_deltas_when_available() {
    let temp_dir = TempDir::new().expect("comparison dir");
    let before_dir = temp_dir.path().join("before-agent");
    let after_dir = temp_dir.path().join("after-agent");
    write_agent_results_with_trace(&before_dir, 2);
    write_agent_results_with_trace(&after_dir, 3);

    let report = build_eval_comparison_report(
        &before_dir.join("results.json"),
        &after_dir.join("results.json"),
        None,
    )
    .expect("comparison report");

    assert_eq!(report.before.metrics.tool_call_count, Some(2));
    assert_eq!(report.after.metrics.tool_call_count, Some(3));
    assert_eq!(report.delta.metrics.tool_call_count, Some(1));
    assert_eq!(report.before.metrics.duration_ms, Some(30));
    assert_eq!(report.after.metrics.duration_ms, Some(35));
    assert_eq!(report.delta.metrics.total_tokens, Some(55));
    assert!(report.markdown.contains("| Tool calls | 2 | 3 | +1 |"));
    assert!(
        report
            .markdown
            .contains("| Total tokens | 180 | 235 | +55 |")
    );
}

#[test]
fn eval_compare_tool_response_returns_markdown_and_safety_policy() {
    let root = std::env::current_dir().expect("cwd");
    let temp_dir = TempDir::new_in(&root).expect("comparison dir under root");
    let before_dir = temp_dir.path().join("before");
    let after_dir = temp_dir.path().join("after");
    write_eval_results(
        &before_dir,
        &json!({
            "suite_name": "mcp-compare",
            "version": 1,
            "mode": "bridge",
            "output_dir": before_dir.display().to_string(),
            "assertion_count": 1,
            "pass_count": 1,
            "fail_count": 0,
            "error_count": 0,
            "pass_rate": 1.0,
            "gate_status": "pass",
            "cases": [{"id": "case", "pass_count": 1, "fail_count": 0, "error_count": 0}]
        }),
    );
    write_eval_results(
        &after_dir,
        &json!({
            "suite_name": "mcp-compare",
            "version": 1,
            "mode": "bridge",
            "output_dir": after_dir.display().to_string(),
            "assertion_count": 1,
            "pass_count": 1,
            "fail_count": 0,
            "error_count": 0,
            "pass_rate": 1.0,
            "gate_status": "pass",
            "cases": [{"id": "case", "pass_count": 1, "fail_count": 0, "error_count": 0}]
        }),
    );

    let response = build_eval_compare_tool_response(&CompareEvalRunsParams {
        before: before_dir.display().to_string(),
        after: after_dir.display().to_string(),
    })
    .expect("tool response");

    assert_eq!(response["success"], json!(true));
    assert_eq!(
        response["data"]["schema_version"],
        json!("eval_comparison.v1")
    );
    assert!(
        response["data"]["markdown"]
            .as_str()
            .expect("markdown")
            .contains("No pass-rate or case-status change was observed")
    );
    assert_eq!(
        response["data"]["safety_policy"]["local_paths_must_stay_under_filesystem_root"],
        json!(true)
    );
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

fn write_eval_results(dir: &Path, value: &serde_json::Value) {
    std::fs::create_dir_all(dir).expect("create result dir");
    std::fs::write(
        dir.join("results.json"),
        serde_json::to_string_pretty(&value).expect("serialize results"),
    )
    .expect("write results");
}

fn write_agent_results_with_trace(dir: &Path, tool_calls: usize) {
    std::fs::create_dir_all(dir.join("tool-calls")).expect("create trace dir");
    let rows = match tool_calls {
        2 => vec![
            json!({
                "tool": "search_indicator",
                "duration_ms": 10,
                "input_tokens": 100,
                "output_tokens": 20,
                "total_tokens": 120,
                "response_bytes": 500
            }),
            json!({
                "tool": "get_context",
                "duration_ms": 20,
                "usage": {"input_tokens": 50, "output_tokens": 10, "total_tokens": 60},
                "response_bytes": 250
            }),
        ],
        3 => vec![
            json!({
                "tool": "search_indicator",
                "duration_ms": 10,
                "input_tokens": 100,
                "output_tokens": 20,
                "total_tokens": 120,
                "response_bytes": 500
            }),
            json!({
                "tool": "get_context",
                "duration_ms": 20,
                "usage": {"input_tokens": 50, "output_tokens": 10, "total_tokens": 60},
                "response_bytes": 250
            }),
            json!({
                "tool": "execute_sql",
                "duration_ms": 5,
                "usage": {"input_tokens": 40, "output_tokens": 15, "total_tokens": 55},
                "response_bytes": 100
            }),
        ],
        _ => panic!("unexpected trace size"),
    };
    let trace = rows
        .into_iter()
        .map(|row| row.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(
        dir.join("tool-calls/agent_case.jsonl"),
        format!("{trace}\n"),
    )
    .expect("write trace");
    write_eval_results(
        dir,
        &json!({
            "suite_name": "agent-ablation",
            "version": 1,
            "mode": "agent",
            "output_dir": dir.display().to_string(),
            "assertion_count": 1,
            "pass_count": 1,
            "fail_count": 0,
            "error_count": 0,
            "pass_rate": 1.0,
            "gate_status": "pass",
            "cases": [{
                "id": "agent_case",
                "question": "Find governed revenue",
                "pass_count": 1,
                "fail_count": 0,
                "error_count": 0,
                "assertions": [{"name": "must_call:search_indicator", "status": "pass"}],
                "artifacts": {
                    "stdout": "stdout.log",
                    "stderr": "stderr.log",
                    "tool_trace": "tool-calls/agent_case.jsonl"
                }
            }]
        }),
    );
}

fn eval_card_suite(name: &str, threshold: Option<f64>, include_agent_case: bool) -> EvalSuite {
    EvalSuite {
        version: 1,
        name: Some(name.to_string()),
        purpose: Some("Proves that Nova can answer the core eval question.".to_string()),
        manifest_scope: Some("synthetic starter manifest".to_string()),
        known_gaps: vec!["does not cover live warehouse freshness".to_string()],
        gate: threshold.map(|threshold| super::EvalGateConfig { threshold }),
        date_anchor: DateAnchor::default(),
        defaults: EvalDefaults {
            persona: Some("analyst".to_string()),
            top_k: 5,
        },
        cases: if include_agent_case {
            Vec::new()
        } else {
            vec![super::EvalCase {
                id: "bridge_case".to_string(),
                question: Some("Find canonical orders".to_string()),
                persona: None,
                date_anchor: DateAnchor::default(),
                assertions: vec![super::EvalAssertion::ToolSuccess {
                    tool: "search".to_string(),
                    params: json!({}),
                }],
            }]
        },
        agent_cases: if include_agent_case {
            vec![super::AgentCase {
                id: "agent_case".to_string(),
                task: "Use Nova to answer the task".to_string(),
                date_anchor: DateAnchor::default(),
                expected: AgentExpected::default(),
            }]
        } else {
            Vec::new()
        },
    }
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

#[allow(clippy::too_many_arguments)]
fn telemetry_row_with_suite_hash(
    suite_name: &str,
    suite_path: &str,
    suite_hash: &str,
    output_dir: &str,
    timestamp_ms: u64,
    case_id: &str,
    assertion_name: &str,
    status: &str,
) -> serde_json::Value {
    let mut row = telemetry_row_with_assertion_count(
        suite_name,
        suite_path,
        output_dir,
        timestamp_ms,
        case_id,
        assertion_name,
        status,
        2,
    );
    if let Some(object) = row.as_object_mut() {
        object.insert("suite_hash".to_string(), json!(suite_hash));
        object.insert("suite_case_count".to_string(), json!(2));
        object.insert("run_case_count".to_string(), json!(2));
    }
    row
}
