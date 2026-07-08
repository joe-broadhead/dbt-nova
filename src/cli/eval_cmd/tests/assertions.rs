use super::*;

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
fn tool_field_equals_checks_json_path_value() {
    let response = json!({
        "data": [
            {
                "execution_surface": "relation",
                "direct_sql_queryable": true
            }
        ]
    });

    let result = tool_field_equals_assertion(
        "search_indicator",
        &response,
        "data.0.direct_sql_queryable",
        &json!(true),
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
