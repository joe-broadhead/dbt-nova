use super::*;

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
