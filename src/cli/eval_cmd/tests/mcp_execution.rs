use super::*;

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
    let manifest_path = temp_dir.path().join("manifest.json");
    std::fs::copy(fixture_manifest_path("nova_manifest.json"), &manifest_path)
        .expect("copy manifest fixture");
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
        manifest_path: Some(manifest_path.display().to_string()),
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
