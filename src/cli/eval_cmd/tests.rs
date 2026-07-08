use std::collections::BTreeMap;
use std::path::Path;

use serde_json::json;
use tempfile::{NamedTempFile, TempDir};

use super::assertions::{json_has_field_path, score_final_answer};
use super::gate::suite_file_hash;
use super::telemetry::{
    apply_telemetry_retention, format_utc_timestamp_millis, telemetry_grade_mode,
};
use super::types::{EvalDefaults, EvalGateConfig};
use super::validation::validate_suite;
use super::{
    AgentCalledWith, AgentCase, AgentEntityRank, AgentExpected, AgentOrder,
    AgentSqlStructureExpected, AssertionResult, DateAnchor, EvalAssertion, EvalCardProvider,
    EvalCardRunContext, EvalCase, EvalCaseReport, EvalRunArgs, EvalSuite, EvalValidateArgs,
    FinalAnswerExpected, agent_prompt, build_agent_eval_tool_response,
    build_eval_compare_tool_response, build_eval_comparison_report, build_eval_gate_report,
    build_eval_gate_tool_response, build_eval_history_tool_response, build_eval_init_tool_response,
    build_eval_validate_payload, build_eval_validate_tool_response, build_report,
    contains_rank_assertion, context_contains_assertion, context_field_equals_assertion,
    eval_case_telemetry_from_trace, metadata_score_max_assertion, metadata_score_min_assertion,
    provider, provider_invocation_evidence, read_tool_trace, recipe_rank_assertion,
    redact_provider_output_text, refresh_eval_card, render_eval_card_markdown,
    resolve_mcp_writable_path, run_eval_command, run_validate_command, safe_path_segment,
    score_agent_expectations, selected_agent_cases, selected_bridge_cases, sql_structure_assertion,
    telemetry_path_for_suite, telemetry_row_matches_since, tool_field_equals_assertion,
    tool_response_budget_assertion, tool_success_assertion, validate_since_date,
    validate_telemetry_suite_name,
};
use crate::params::{
    CompareEvalRunsParams, GetEvalGateParams, GetEvalHistoryParams, InitEvalSuiteParams,
    RunAgentEvalParams, ValidateEvalSuiteParams,
};

mod assertions;
mod cards_telemetry;
mod compare;
mod gate_validation;
mod mcp_execution;

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
        gate: threshold.map(|threshold| EvalGateConfig { threshold }),
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
