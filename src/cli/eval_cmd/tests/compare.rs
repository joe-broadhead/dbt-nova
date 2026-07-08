use super::*;

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
