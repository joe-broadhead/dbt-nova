use super::*;

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
        gate: Some(EvalGateConfig { threshold: 1.1 }),
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
