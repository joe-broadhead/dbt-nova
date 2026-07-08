use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write as IoWrite};
use std::path::{Component, Path, PathBuf};
use std::process::Command as StdCommand;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{Value as JsonValue, json};

use crate::cli::args::{
    EvalAgentRunArgs, EvalCompareArgs, EvalGateArgs, EvalHistoryArgs, EvalInitArgs, EvalRunArgs,
    EvalValidateArgs, ManifestLoadArgs,
};
use crate::cli::manifest::{build_manifest_load_config, execute_manifest_load};
use crate::cli::output::{CliEnvelope, error_envelope};
use crate::cli::{DispatchError, DispatchResult};
use crate::error::DbtNovaError;
use crate::manifest::ManifestSearch;
use crate::params::{
    CompareEvalRunsParams, GetEvalGateParams, GetEvalHistoryParams, InitEvalSuiteParams,
    RunAgentEvalParams, RunEvalParams, ValidateEvalSuiteParams,
};
use crate::responses::SuccessResponse;
use crate::utils::redact_sensitive_text;
use crate::utils::sql_structure::{
    SqlStructureSignature, compare_sql_structure, compare_sql_structure_signatures,
    sql_structure_signature,
};
use crate::utils::tool_trace::{normalize_tool_trace_indices, read_tool_trace_file};

const DEFAULT_TOP_K: usize = 5;
const DEFAULT_FAIL_UNDER: f64 = 1.0;
const MAX_SAFE_PATH_SEGMENT_CHARS: usize = 120;
const DEFAULT_TELEMETRY_DIR: &str = ".nova/eval-runs/telemetry";
const RAW_PROVIDER_LOGS_ENV: &str = "DBT_NOVA_EVAL_UNSAFE_WRITE_RAW_PROVIDER_LOGS";

mod assertions;
mod compare;
mod gate;
mod provider;
mod render;
mod reporting;
mod telemetry;
mod types;
mod validation;

use assertions::{
    contains_rank_assertion, contains_string_assertion, context_contains_assertion,
    context_field_equals_assertion, effective_limit, fields_assertion,
    metadata_score_max_assertion, metadata_score_min_assertion, rank_assertion, read_tool_trace,
    recipe_queries_assertion, recipe_rank_assertion, reset_trace_file, score_agent_expectations,
    search_columns_assertion, sql_structure_assertion, tool_field_equals_assertion,
    tool_response_budget_assertion, tool_success_assertion,
};
use compare::{
    build_eval_comparison_report, resolve_eval_results_path, resolve_mcp_eval_results_path,
};
use gate::{
    build_eval_gate_report, latest_telemetry_rows, print_gate_report, read_telemetry_rows_for_suite,
};
use render::{finish_report, render_eval_card_markdown, render_markdown, render_tsv};
use reporting::{agent_manifest_source, build_report, refresh_eval_card, write_report_artifacts};
use telemetry::{
    agent_provider_command_preset, assertion_type, days_in_month, eval_case_telemetry_from_trace,
    telemetry_path_for_suite, telemetry_row_matches_since, validate_since_date,
    validate_telemetry_retention, validate_telemetry_suite_name, write_eval_telemetry,
};
use types::{
    AgentArtifacts, AgentCalledWith, AgentCase, AgentEntityRank, AgentExpected, AgentOrder,
    AgentSqlStructureExpected, AgentTelemetryContext, AssertionResult, DateAnchor, EvalAssertion,
    EvalCard, EvalCardGate, EvalCardManifestScope, EvalCardProvider, EvalCardRunContext,
    EvalCardTelemetry, EvalCase, EvalCaseReport, EvalCaseStatusDelta, EvalCaseTelemetry,
    EvalComparisonDelta, EvalComparisonMetricDeltas, EvalComparisonMetrics, EvalComparisonReport,
    EvalComparisonRunSummary, EvalGateReport, EvalHistoryPayload, EvalMcpSafetyPolicy, EvalReport,
    EvalSuite, EvalTelemetryRunContext, FinalAnswerExpected, GateConfigStatus,
};
use validation::{
    agent_prompt, build_eval_gate_report_for_suite, build_eval_validate_payload,
    default_sql_structure_tool, default_top_k, effective_date_anchor, elapsed_ms_to_u64,
    empty_object, ensure_mcp_eval_path_under_root,
    ensure_mcp_latest_telemetry_suite_paths_under_root,
    ensure_mcp_telemetry_suite_paths_under_root, eval_history_rows, eval_mcp_safety_policy,
    load_suite_with_hash, mcp_eval_candidate_path, mcp_eval_filesystem_root, mcp_eval_flag_enabled,
    normalized_string, provider_failure_evidence, provider_invocation_evidence,
    provider_output_for_artifact, ratio, redact_provider_output_text, require_mcp_eval_flag,
    resolve_mcp_existing_file, resolve_mcp_writable_path, resolve_output_dir, safe_path_segment,
    selected_agent_cases, selected_bridge_cases, server_error, starter_suite, success_value,
    truncate, validate_fail_under, with_eval_safety_policy,
};

#[derive(Debug, Clone)]
struct EvalCardTelemetryEvidence {
    telemetry: EvalCardTelemetry,
    gate: EvalCardGate,
}

const MCP_ENABLE_EVAL_RUN_ENV: &str = "DBT_NOVA_MCP_ENABLE_EVAL_RUN";
const MCP_ENABLE_EVAL_WRITES_ENV: &str = "DBT_NOVA_MCP_ENABLE_EVAL_WRITES";
const MCP_ENABLE_AGENT_EVAL_ENV: &str = "DBT_NOVA_MCP_ENABLE_AGENT_EVAL";
const MCP_ENABLE_CUSTOM_AGENT_PROVIDER_ENV: &str = "DBT_NOVA_MCP_ENABLE_CUSTOM_AGENT_PROVIDER";

/// Writes a starter eval suite.
///
/// # Errors
/// Returns an error when the target exists without `--force` or cannot be written.
pub fn run_init_command(args: &EvalInitArgs) -> DispatchResult {
    let started = Instant::now();
    let path = PathBuf::from(&args.out);
    if path.exists() && !args.force {
        return Err(DbtNovaError::InvalidParams(format!(
            "eval suite '{}' already exists; pass --force to overwrite",
            path.display()
        ))
        .into());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| server_error(error.to_string()))?;
    }

    fs::write(&path, starter_suite(&args.persona))
        .map_err(|error| server_error(error.to_string()))?;
    let payload = json!({
        "path": path.display().to_string(),
        "persona": args.persona,
    });
    let envelope = CliEnvelope::success("eval init", payload, started.elapsed().as_millis());
    let out =
        serde_json::to_string_pretty(&envelope).map_err(|error| server_error(error.to_string()))?;
    println!("{out}");
    Ok(())
}

/// Validates an eval suite without loading a manifest or running a provider.
///
/// # Errors
/// Returns an error when the suite cannot be read or fails schema validation.
pub fn run_validate_command(args: &EvalValidateArgs) -> DispatchResult {
    let started = Instant::now();
    let payload = build_eval_validate_payload(&args.suite).map_err(|error| {
        if args.json {
            let envelope = error_envelope("eval validate", &error, started.elapsed().as_millis());
            if let Ok(out) = serde_json::to_string_pretty(&envelope) {
                println!("{out}");
                return DispatchError {
                    error,
                    rendered: true,
                };
            }
        }
        DispatchError {
            error,
            rendered: false,
        }
    })?;
    if args.json {
        let envelope =
            CliEnvelope::success("eval validate", payload, started.elapsed().as_millis());
        let out = serde_json::to_string_pretty(&envelope)
            .map_err(|error| server_error(error.to_string()))?;
        println!("{out}");
    } else {
        println!("eval suite is valid");
        println!(
            "  suite: {}",
            payload
                .get("suite_name")
                .and_then(JsonValue::as_str)
                .unwrap_or("suite")
        );
        println!(
            "  version: {}",
            payload
                .get("version")
                .and_then(JsonValue::as_u64)
                .unwrap_or(0)
        );
        println!(
            "  bridge cases: {}",
            payload
                .get("bridge_case_count")
                .and_then(JsonValue::as_u64)
                .unwrap_or(0)
        );
        println!(
            "  agent cases: {}",
            payload
                .get("agent_case_count")
                .and_then(JsonValue::as_u64)
                .unwrap_or(0)
        );
    }
    Ok(())
}

/// Reports readiness from the latest eval telemetry for a suite.
///
/// # Errors
/// Returns an error when existing telemetry or suite configuration cannot be parsed.
pub fn run_gate_command(args: &EvalGateArgs) -> DispatchResult {
    let started = Instant::now();
    let report = build_eval_gate_report_for_suite(&args.suite)?;
    if args.json {
        let envelope = CliEnvelope::success("eval gate", &report, started.elapsed().as_millis());
        let out = serde_json::to_string_pretty(&envelope)
            .map_err(|error| server_error(error.to_string()))?;
        println!("{out}");
    } else {
        print_gate_report(&report);
    }
    Ok(())
}

/// Prints filtered JSONL eval telemetry history.
///
/// # Errors
/// Returns an error when `--since` is invalid or existing telemetry cannot be read.
pub fn run_history_command(args: &EvalHistoryArgs) -> DispatchResult {
    let (_since_boundary, rows) = eval_history_rows(&args.suite, &args.since)?;
    for row in rows {
        let out = serde_json::to_string(&row).map_err(|error| server_error(error.to_string()))?;
        println!("{out}");
    }
    Ok(())
}

/// Compares two eval result directories or results.json files.
///
/// # Errors
/// Returns an error when either input cannot be resolved, read, or parsed.
pub fn run_compare_command(args: &EvalCompareArgs) -> DispatchResult {
    let started = Instant::now();
    let before = resolve_eval_results_path(&args.before, "before")?;
    let after = resolve_eval_results_path(&args.after, "after")?;
    let report = build_eval_comparison_report(&before, &after, None)?;
    if args.json {
        let envelope = CliEnvelope::success("eval compare", &report, started.elapsed().as_millis());
        let out = serde_json::to_string_pretty(&envelope)
            .map_err(|error| server_error(error.to_string()))?;
        println!("{out}");
    } else {
        print!("{}", report.markdown);
    }
    Ok(())
}

/// Builds the MCP/CLI-tool response for eval suite validation.
///
/// # Errors
/// Returns an error when the suite path is unsafe, unreadable, or invalid.
pub fn build_eval_validate_tool_response(
    params: &ValidateEvalSuiteParams,
) -> crate::error::Result<JsonValue> {
    let suite_path = resolve_mcp_existing_file(&params.suite, "suite")?;
    let payload = build_eval_validate_payload(&suite_path.display().to_string())?;
    success_value(with_eval_safety_policy(payload)?, 1)
}

/// Builds the MCP/CLI-tool response for eval gate status.
///
/// # Errors
/// Returns an error when telemetry or referenced suite configuration cannot be parsed.
pub fn build_eval_gate_tool_response(
    params: &GetEvalGateParams,
) -> crate::error::Result<JsonValue> {
    let rows = read_telemetry_rows_for_suite(&params.suite)?;
    ensure_mcp_latest_telemetry_suite_paths_under_root(&rows)?;
    let report = build_eval_gate_report(&params.suite, &rows)?;
    let payload = with_eval_safety_policy(serde_json::to_value(report).map_err(|error| {
        DbtNovaError::ServerError(format!("failed to serialize eval gate report: {error}"))
    })?)?;
    success_value(payload, 1)
}

/// Builds the MCP/CLI-tool response for eval telemetry history.
///
/// # Errors
/// Returns an error when the since date is invalid or telemetry cannot be read.
pub fn build_eval_history_tool_response(
    params: &GetEvalHistoryParams,
) -> crate::error::Result<JsonValue> {
    let (since_boundary, rows) = eval_history_rows(&params.suite, &params.since)?;
    ensure_mcp_telemetry_suite_paths_under_root(rows.iter())?;
    let row_count = rows.len();
    let payload = EvalHistoryPayload {
        suite_name: params.suite.clone(),
        since: since_boundary,
        row_count,
        rows,
        safety_policy: eval_mcp_safety_policy()?,
    };
    success_value(payload, row_count)
}

/// Builds the MCP/CLI-tool response for eval result comparison.
///
/// # Errors
/// Returns an error when either results path is unsafe, unreadable, or invalid.
pub fn build_eval_compare_tool_response(
    params: &CompareEvalRunsParams,
) -> crate::error::Result<JsonValue> {
    let root = mcp_eval_filesystem_root()?;
    let before = resolve_mcp_eval_results_path(&params.before, "before")?;
    let after = resolve_mcp_eval_results_path(&params.after, "after")?;
    let report = build_eval_comparison_report(&before, &after, Some(&root))?;
    let payload = with_eval_safety_policy(serde_json::to_value(report).map_err(|error| {
        DbtNovaError::ServerError(format!("failed to serialize eval comparison: {error}"))
    })?)?;
    success_value(payload, 1)
}

/// Builds the MCP/CLI-tool response for deterministic bridge eval execution.
///
/// # Errors
/// Returns an error when MCP eval execution is not explicitly enabled, paths are
/// unsafe, suite validation fails, artifact writes fail, or eval execution fails.
pub async fn build_eval_run_tool_response(
    search: &ManifestSearch,
    params: &RunEvalParams,
) -> crate::error::Result<JsonValue> {
    require_mcp_eval_flag(MCP_ENABLE_EVAL_RUN_ENV, "run_eval")?;
    let suite_path = resolve_mcp_existing_file(&params.suite, "suite")?;
    let output_dir = params
        .output_dir
        .as_deref()
        .map(|path| resolve_mcp_writable_path(path, "output_dir"))
        .transpose()?;
    let output_dir_string = output_dir.as_ref().map(|path| path.display().to_string());
    let report = execute_bridge_eval(
        search,
        &suite_path.display().to_string(),
        output_dir_string.as_deref(),
        params.telemetry,
        params.telemetry_retention,
        &params.case_ids,
        params.fail_under,
        Some(search.manifest_hash.as_str()),
    )
    .await?;
    let payload = with_eval_safety_policy(serde_json::to_value(report).map_err(|error| {
        DbtNovaError::ServerError(format!("failed to serialize eval report: {error}"))
    })?)?;
    success_value(payload, 1)
}

/// Builds the MCP/CLI-tool response for starter eval suite creation.
///
/// # Errors
/// Returns an error when MCP eval writes are not explicitly enabled, the output
/// path is unsafe, the target exists without force, or writing fails.
pub fn build_eval_init_tool_response(
    params: &InitEvalSuiteParams,
) -> crate::error::Result<JsonValue> {
    require_mcp_eval_flag(MCP_ENABLE_EVAL_WRITES_ENV, "init_eval_suite")?;
    let out = resolve_mcp_writable_path(&params.out, "out")?;
    if out.exists() && !params.force {
        return Err(DbtNovaError::InvalidParams(format!(
            "eval suite '{}' already exists; set force=true to overwrite",
            out.display()
        )));
    }
    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent).map_err(|error| server_error(error.to_string()))?;
    }
    let persona = params
        .persona
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("analyst");
    fs::write(&out, starter_suite(persona)).map_err(|error| server_error(error.to_string()))?;
    let payload = with_eval_safety_policy(json!({
        "path": out.display().to_string(),
        "persona": persona,
    }))?;
    success_value(payload, 1)
}

/// Builds the MCP/CLI-tool response for provider-backed agent eval execution.
///
/// # Errors
/// Returns an error when MCP agent eval execution is not explicitly enabled,
/// custom provider execution is requested without its opt-in, paths are unsafe,
/// or the provider-backed eval fails to run.
pub async fn build_agent_eval_tool_response(
    params: &RunAgentEvalParams,
) -> crate::error::Result<JsonValue> {
    require_mcp_eval_flag(MCP_ENABLE_AGENT_EVAL_ENV, "run_agent_eval")?;
    if (params.provider_command.is_some() || params.provider_args_json.is_some())
        && !mcp_eval_flag_enabled(MCP_ENABLE_CUSTOM_AGENT_PROVIDER_ENV)
    {
        return Err(DbtNovaError::InvalidParams(format!(
            "run_agent_eval custom provider commands require {MCP_ENABLE_CUSTOM_AGENT_PROVIDER_ENV}=1"
        )));
    }

    let suite_path = resolve_mcp_existing_file(&params.suite, "suite")?;
    let output_dir = params
        .output_dir
        .as_deref()
        .map(|path| resolve_mcp_writable_path(path, "output_dir"))
        .transpose()?;
    let manifest_path = params
        .manifest_path
        .as_deref()
        .map(|path| resolve_mcp_existing_file(path, "manifest_path"))
        .transpose()?;
    let args = EvalAgentRunArgs {
        suite: suite_path.display().to_string(),
        provider: params
            .provider
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("opencode")
            .to_string(),
        provider_model: params.provider_model.clone(),
        provider_command: params.provider_command.clone(),
        provider_args_json: params.provider_args_json.clone(),
        manifest_path: manifest_path.map(|path| path.display().to_string()),
        manifest_uri: params.manifest_uri.clone(),
        storage_instance_id: params.storage_instance_id.clone(),
        output_dir: output_dir.map(|path| path.display().to_string()),
        telemetry: params.telemetry,
        telemetry_retention: params.telemetry_retention,
        case_ids: params.case_ids.clone(),
        timeout_secs: params.timeout_secs.unwrap_or(600),
        fail_under: params.fail_under,
        cleanup_storage_on_start: params.cleanup_storage_on_start,
        read_only: params.read_only,
        json: false,
    };
    let report = execute_agent_eval_from_args(&args).await?;
    let payload = with_eval_safety_policy(serde_json::to_value(report).map_err(|error| {
        DbtNovaError::ServerError(format!("failed to serialize agent eval report: {error}"))
    })?)?;
    success_value(payload, 1)
}

/// Runs deterministic Nova bridge assertions against a manifest.
///
/// # Errors
/// Returns an error when the suite is invalid, manifest loading fails, assertions error, or the score gate fails.
pub async fn run_eval_command(args: &EvalRunArgs) -> DispatchResult {
    let started = Instant::now();
    let report = execute_bridge_eval_from_args(args).await?;
    let elapsed_ms = started.elapsed().as_millis();
    finish_report("eval run", &report, args.json, elapsed_ms)
}

/// Runs agent-provider evals and scores the tool-use trace.
///
/// # Errors
/// Returns an error when the suite is invalid, a provider command cannot be run, or the score gate fails.
pub async fn run_agent_eval_command(args: &EvalAgentRunArgs) -> DispatchResult {
    let started = Instant::now();
    let report = execute_agent_eval_from_args(args).await?;
    let elapsed_ms = started.elapsed().as_millis();
    finish_report("eval agent run", &report, args.json, elapsed_ms)
}

async fn execute_bridge_eval_from_args(args: &EvalRunArgs) -> crate::error::Result<EvalReport> {
    let config = build_manifest_load_config(&ManifestLoadArgs {
        manifest_path: args.manifest_path.clone(),
        manifest_uri: args.manifest_uri.clone(),
        storage_instance_id: args.storage_instance_id.clone(),
        cleanup_storage_on_start: args.cleanup_storage_on_start,
        read_only: args.read_only,
        json: false,
    })?;
    let load_result = execute_manifest_load(config).await?;
    execute_bridge_eval(
        &load_result.search,
        &args.suite,
        args.output_dir.as_deref(),
        args.telemetry,
        args.telemetry_retention,
        &args.case_ids,
        args.fail_under,
        Some(load_result.search.manifest_hash.as_str()),
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn execute_bridge_eval(
    search: &ManifestSearch,
    suite_path: &str,
    output_dir: Option<&str>,
    telemetry: bool,
    telemetry_retention: Option<usize>,
    case_ids: &[String],
    fail_under: Option<f64>,
    manifest_hash: Option<&str>,
) -> crate::error::Result<EvalReport> {
    let started = Instant::now();
    let (suite, suite_hash) = load_suite_with_hash(suite_path)?;
    validate_fail_under(fail_under)?;
    validate_telemetry_retention(telemetry_retention)?;
    validate_telemetry_suite_name(&suite, telemetry)?;
    if suite.cases.is_empty() {
        return Err(DbtNovaError::InvalidParams(
            "eval suite contains no bridge cases".to_string(),
        ));
    }
    let selected_cases = selected_bridge_cases(&suite.cases, case_ids)?;
    let output_dir = resolve_output_dir(output_dir, &suite, "bridge");
    fs::create_dir_all(&output_dir).map_err(|error| server_error(error.to_string()))?;

    let mut cases = Vec::with_capacity(selected_cases.len());
    for case in selected_cases {
        cases.push(evaluate_bridge_case(case, &suite, search).await);
    }
    let mut report = build_report(
        &suite,
        "bridge",
        output_dir.display().to_string(),
        fail_under.unwrap_or(DEFAULT_FAIL_UNDER),
        cases,
    );
    if telemetry {
        write_eval_telemetry(
            &report,
            EvalTelemetryRunContext {
                suite_path,
                suite_hash: &suite_hash,
                suite_case_count: suite.cases.len(),
                manifest_hash,
                duration_ms: elapsed_ms_to_u64(started.elapsed()),
                retention: telemetry_retention,
                agent: None,
            },
        )
        .map_err(|error| error.error)?;
    }
    refresh_eval_card(
        &mut report,
        &suite,
        &EvalCardRunContext {
            suite_path: Some(suite_path.to_string()),
            manifest_hash: manifest_hash.map(str::to_string),
            telemetry_requested: telemetry,
            ..EvalCardRunContext::default()
        },
    );
    write_report_artifacts(&output_dir, &report, suite_path).map_err(|error| error.error)?;
    Ok(report)
}

async fn execute_agent_eval_from_args(args: &EvalAgentRunArgs) -> crate::error::Result<EvalReport> {
    let started = Instant::now();
    let (suite, suite_hash) = load_suite_with_hash(&args.suite)?;
    validate_fail_under(args.fail_under)?;
    validate_telemetry_retention(args.telemetry_retention)?;
    validate_telemetry_suite_name(&suite, args.telemetry)?;
    if suite.agent_cases.is_empty() {
        return Err(DbtNovaError::InvalidParams(
            "eval suite contains no agent_cases".to_string(),
        ));
    }
    let selected_cases = selected_agent_cases(&suite.agent_cases, &args.case_ids)?;
    let output_dir = resolve_output_dir(args.output_dir.as_deref(), &suite, "agent");
    fs::create_dir_all(output_dir.join("tool-calls"))
        .map_err(|error| server_error(error.to_string()))?;

    let mut cases = Vec::with_capacity(selected_cases.len());
    for case in selected_cases {
        cases.push(run_agent_case(case, &suite, args, &output_dir).await);
    }
    let mut report = build_report(
        &suite,
        "agent",
        output_dir.display().to_string(),
        args.fail_under.unwrap_or(DEFAULT_FAIL_UNDER),
        cases,
    );
    let elapsed = started.elapsed();
    if args.telemetry {
        write_eval_telemetry(
            &report,
            EvalTelemetryRunContext {
                suite_path: &args.suite,
                suite_hash: &suite_hash,
                suite_case_count: suite.agent_cases.len(),
                manifest_hash: None,
                duration_ms: elapsed_ms_to_u64(elapsed),
                retention: args.telemetry_retention,
                agent: Some(AgentTelemetryContext {
                    provider: &args.provider,
                    provider_command_preset: agent_provider_command_preset(args),
                }),
            },
        )
        .map_err(|error| error.error)?;
    }
    refresh_eval_card(
        &mut report,
        &suite,
        &EvalCardRunContext {
            suite_path: Some(args.suite.clone()),
            manifest_source: agent_manifest_source(args),
            telemetry_requested: args.telemetry,
            provider: Some(EvalCardProvider {
                provider: args.provider.clone(),
                command_preset: agent_provider_command_preset(args).to_string(),
                model: args.provider_model.clone(),
            }),
            ..EvalCardRunContext::default()
        },
    );
    write_report_artifacts(&output_dir, &report, &args.suite).map_err(|error| error.error)?;
    Ok(report)
}

async fn evaluate_bridge_case(
    case: &EvalCase,
    suite: &EvalSuite,
    search: &ManifestSearch,
) -> EvalCaseReport {
    let mut assertions = Vec::with_capacity(case.assertions.len());
    for assertion in &case.assertions {
        assertions.push(evaluate_bridge_assertion(assertion, case, suite, search).await);
    }
    EvalCaseReport::new(case.id.clone(), case.question.clone(), assertions, None)
        .with_date_anchor(effective_date_anchor(&suite.date_anchor, &case.date_anchor))
}

async fn evaluate_bridge_assertion(
    assertion: &EvalAssertion,
    case: &EvalCase,
    suite: &EvalSuite,
    search: &ManifestSearch,
) -> AssertionResult {
    let ctx = BridgeAssertionContext {
        case,
        suite,
        search,
    };
    match assertion {
        EvalAssertion::SearchRank { .. }
        | EvalAssertion::SearchIndicatorRank { .. }
        | EvalAssertion::SearchColumnsRank { .. } => {
            evaluate_search_bridge_assertion(ctx, assertion).await
        }
        EvalAssertion::ContextHas { .. }
        | EvalAssertion::ContextFieldEquals { .. }
        | EvalAssertion::ContextContains { .. } => {
            evaluate_context_bridge_assertion(ctx, assertion).await
        }
        EvalAssertion::MetadataScoreMin { .. } | EvalAssertion::MetadataScoreMax { .. } => {
            evaluate_metadata_bridge_assertion(ctx, assertion).await
        }
        EvalAssertion::RecipeRank { .. } | EvalAssertion::RecipeHasQueries { .. } => {
            evaluate_recipe_bridge_assertion(ctx, assertion).await
        }
        EvalAssertion::LineageContains { .. } => {
            evaluate_lineage_bridge_assertion(ctx, assertion).await
        }
        EvalAssertion::ToolSuccess { tool, params } => {
            evaluate_tool_success_assertion(ctx, tool, params).await
        }
        EvalAssertion::ToolResponseBudget {
            tool,
            params,
            max_response_bytes,
            must_contain_paths,
            must_not_contain_paths,
        } => {
            evaluate_tool_response_budget_assertion(
                ctx,
                tool,
                params,
                *max_response_bytes,
                must_contain_paths,
                must_not_contain_paths,
            )
            .await
        }
        EvalAssertion::ToolFieldEquals {
            tool,
            params,
            field,
            expected,
        } => evaluate_tool_field_equals_assertion(ctx, tool, params, field, expected).await,
        EvalAssertion::SqlStructure {
            actual_sql,
            expected_sql,
        } => sql_structure_assertion("sql_structure", actual_sql, expected_sql),
    }
}

#[derive(Clone, Copy)]
struct BridgeAssertionContext<'a> {
    case: &'a EvalCase,
    suite: &'a EvalSuite,
    search: &'a ManifestSearch,
}

async fn evaluate_search_bridge_assertion(
    ctx: BridgeAssertionContext<'_>,
    assertion: &EvalAssertion,
) -> AssertionResult {
    match assertion {
        EvalAssertion::SearchRank {
            query,
            expected_unique_id,
            max_rank,
            resource_types,
            persona,
        } => {
            evaluate_search_rank_assertion(
                ctx,
                query,
                expected_unique_id,
                *max_rank,
                resource_types,
                persona.as_ref(),
            )
            .await
        }
        EvalAssertion::SearchIndicatorRank {
            query,
            expected,
            max_rank,
            resource_types,
            indicator_types,
            persona,
        } => {
            evaluate_search_indicator_rank_assertion(
                ctx,
                query,
                expected,
                *max_rank,
                resource_types,
                indicator_types,
                persona.as_ref(),
            )
            .await
        }
        EvalAssertion::SearchColumnsRank {
            query,
            expected_column,
            expected_parent_unique_id,
            max_rank,
        } => {
            evaluate_search_columns_rank_assertion(
                ctx,
                query,
                expected_column.as_deref(),
                expected_parent_unique_id.as_deref(),
                *max_rank,
            )
            .await
        }
        _ => unreachable!("search bridge assertion category"),
    }
}

async fn evaluate_context_bridge_assertion(
    ctx: BridgeAssertionContext<'_>,
    assertion: &EvalAssertion,
) -> AssertionResult {
    match assertion {
        EvalAssertion::ContextHas { id_or_name, fields } => {
            evaluate_context_has_assertion(ctx, id_or_name, fields).await
        }
        EvalAssertion::ContextFieldEquals {
            id_or_name,
            field,
            expected,
        } => evaluate_context_field_equals_assertion(ctx, id_or_name, field, expected).await,
        EvalAssertion::ContextContains {
            id_or_name,
            expected,
            field,
        } => evaluate_context_contains_assertion(ctx, id_or_name, expected, field.as_deref()).await,
        _ => unreachable!("context bridge assertion category"),
    }
}

async fn evaluate_metadata_bridge_assertion(
    ctx: BridgeAssertionContext<'_>,
    assertion: &EvalAssertion,
) -> AssertionResult {
    match assertion {
        EvalAssertion::MetadataScoreMin {
            id_or_name,
            threshold,
            persona,
        } => {
            evaluate_metadata_score_assertion(
                ctx,
                "metadata_score_min",
                id_or_name.as_ref(),
                *threshold,
                persona.as_ref(),
                metadata_score_min_assertion,
            )
            .await
        }
        EvalAssertion::MetadataScoreMax {
            id_or_name,
            threshold,
            persona,
        } => {
            evaluate_metadata_score_assertion(
                ctx,
                "metadata_score_max",
                id_or_name.as_ref(),
                *threshold,
                persona.as_ref(),
                metadata_score_max_assertion,
            )
            .await
        }
        _ => unreachable!("metadata bridge assertion category"),
    }
}

async fn evaluate_recipe_bridge_assertion(
    ctx: BridgeAssertionContext<'_>,
    assertion: &EvalAssertion,
) -> AssertionResult {
    match assertion {
        EvalAssertion::RecipeRank {
            query,
            expected_recipe_id,
            max_rank,
        } => evaluate_recipe_rank_assertion(ctx, query, expected_recipe_id, *max_rank).await,
        EvalAssertion::RecipeHasQueries {
            recipe_id,
            min_queries,
        } => evaluate_recipe_queries_assertion(ctx, recipe_id, *min_queries).await,
        _ => unreachable!("recipe bridge assertion category"),
    }
}

async fn evaluate_lineage_bridge_assertion(
    ctx: BridgeAssertionContext<'_>,
    assertion: &EvalAssertion,
) -> AssertionResult {
    match assertion {
        EvalAssertion::LineageContains {
            id_or_name,
            direction,
            expected_unique_id,
            depth,
        } => {
            evaluate_lineage_contains_assertion(
                ctx,
                id_or_name,
                direction,
                expected_unique_id,
                *depth,
            )
            .await
        }
        _ => unreachable!("lineage bridge assertion category"),
    }
}

async fn evaluate_search_rank_assertion(
    ctx: BridgeAssertionContext<'_>,
    query: &str,
    expected_unique_id: &str,
    max_rank: Option<usize>,
    resource_types: &[String],
    persona: Option<&String>,
) -> AssertionResult {
    let params = json!({
        "query": query,
        "resource_types": resource_types,
        "persona": effective_persona(persona, ctx.case, ctx.suite),
        "limit": effective_limit(max_rank, ctx.suite.defaults.top_k),
    });
    match call_tool(ctx.search, "search", params).await {
        Ok(response) => rank_assertion(
            "search_rank",
            &response,
            expected_unique_id,
            max_rank,
            "unique_id",
        ),
        Err(error) => AssertionResult::error("search_rank", error.to_string()),
    }
}

async fn evaluate_search_indicator_rank_assertion(
    ctx: BridgeAssertionContext<'_>,
    query: &str,
    expected: &str,
    max_rank: Option<usize>,
    resource_types: &[String],
    indicator_types: &[String],
    persona: Option<&String>,
) -> AssertionResult {
    let params = json!({
        "query": query,
        "resource_types": resource_types,
        "indicator_types": indicator_types,
        "persona": effective_persona(persona, ctx.case, ctx.suite),
        "limit": effective_limit(max_rank, ctx.suite.defaults.top_k),
    });
    match call_tool(ctx.search, "search_indicator", params).await {
        Ok(response) => {
            contains_rank_assertion("search_indicator_rank", &response, expected, max_rank)
        }
        Err(error) => AssertionResult::error("search_indicator_rank", error.to_string()),
    }
}

async fn evaluate_search_columns_rank_assertion(
    ctx: BridgeAssertionContext<'_>,
    query: &str,
    expected_column: Option<&str>,
    expected_parent_unique_id: Option<&str>,
    max_rank: Option<usize>,
) -> AssertionResult {
    let params = json!({
        "query": query,
        "limit": effective_limit(max_rank, ctx.suite.defaults.top_k)
    });
    match call_tool(ctx.search, "search_columns", params).await {
        Ok(response) => search_columns_assertion(
            &response,
            expected_column,
            expected_parent_unique_id,
            max_rank,
        ),
        Err(error) => AssertionResult::error("search_columns_rank", error.to_string()),
    }
}

async fn evaluate_context_has_assertion(
    ctx: BridgeAssertionContext<'_>,
    id_or_name: &str,
    fields: &[String],
) -> AssertionResult {
    let params = json!({"id_or_name": id_or_name});
    match call_tool(ctx.search, "get_context", params).await {
        Ok(response) => fields_assertion("context_has", &response, fields),
        Err(error) => AssertionResult::error("context_has", error.to_string()),
    }
}

async fn evaluate_context_field_equals_assertion(
    ctx: BridgeAssertionContext<'_>,
    id_or_name: &str,
    field: &str,
    expected: &JsonValue,
) -> AssertionResult {
    let params = json!({"id_or_name": id_or_name});
    match call_tool(ctx.search, "get_context", params).await {
        Ok(response) => context_field_equals_assertion(&response, field, expected),
        Err(error) => AssertionResult::error("context_field_equals", error.to_string()),
    }
}

async fn evaluate_context_contains_assertion(
    ctx: BridgeAssertionContext<'_>,
    id_or_name: &str,
    expected: &str,
    field: Option<&str>,
) -> AssertionResult {
    let params = json!({"id_or_name": id_or_name});
    match call_tool(ctx.search, "get_context", params).await {
        Ok(response) => context_contains_assertion(&response, field, expected),
        Err(error) => AssertionResult::error("context_contains", error.to_string()),
    }
}

async fn evaluate_metadata_score_assertion(
    ctx: BridgeAssertionContext<'_>,
    name: &str,
    id_or_name: Option<&String>,
    threshold: f64,
    persona: Option<&String>,
    scorer: fn(&JsonValue, f64) -> AssertionResult,
) -> AssertionResult {
    let params = json!({
        "id_or_name": id_or_name,
        "persona": effective_persona(persona, ctx.case, ctx.suite),
        "limit": 20,
    });
    match call_tool(ctx.search, "get_metadata_score", params).await {
        Ok(response) => scorer(&response, threshold),
        Err(error) => AssertionResult::error(name, error.to_string()),
    }
}

async fn evaluate_recipe_rank_assertion(
    ctx: BridgeAssertionContext<'_>,
    query: &str,
    expected_recipe_id: &str,
    max_rank: Option<usize>,
) -> AssertionResult {
    let params = json!({
        "query": query,
        "include_queries": false,
        "limit": effective_limit(max_rank, ctx.suite.defaults.top_k)
    });
    match call_tool(ctx.search, "search_recipes", params).await {
        Ok(response) => recipe_rank_assertion(&response, expected_recipe_id, max_rank),
        Err(error) => AssertionResult::error("recipe_rank", error.to_string()),
    }
}

async fn evaluate_recipe_queries_assertion(
    ctx: BridgeAssertionContext<'_>,
    recipe_id: &str,
    min_queries: Option<usize>,
) -> AssertionResult {
    let params = json!({"recipe_id": recipe_id, "include_queries": true});
    match call_tool(ctx.search, "get_recipe", params).await {
        Ok(response) => recipe_queries_assertion(&response, min_queries.unwrap_or(1)),
        Err(error) => AssertionResult::error("recipe_has_queries", error.to_string()),
    }
}

async fn evaluate_lineage_contains_assertion(
    ctx: BridgeAssertionContext<'_>,
    id_or_name: &str,
    direction: &str,
    expected_unique_id: &str,
    depth: Option<usize>,
) -> AssertionResult {
    let params = json!({
        "id_or_name": id_or_name,
        "direction": direction,
        "depth": depth.unwrap_or(2),
    });
    match call_tool(ctx.search, "get_lineage", params).await {
        Ok(response) => {
            contains_string_assertion("lineage_contains", &response, expected_unique_id)
        }
        Err(error) => AssertionResult::error("lineage_contains", error.to_string()),
    }
}

async fn evaluate_tool_success_assertion(
    ctx: BridgeAssertionContext<'_>,
    tool: &str,
    params: &JsonValue,
) -> AssertionResult {
    match call_tool(ctx.search, tool, params.clone()).await {
        Ok(response) => tool_success_assertion(tool, &response),
        Err(error) => AssertionResult::error(format!("tool_success:{tool}"), error.to_string()),
    }
}

async fn evaluate_tool_response_budget_assertion(
    ctx: BridgeAssertionContext<'_>,
    tool: &str,
    params: &JsonValue,
    max_response_bytes: usize,
    must_contain_paths: &[String],
    must_not_contain_paths: &[String],
) -> AssertionResult {
    match call_tool(ctx.search, tool, params.clone()).await {
        Ok(response) => tool_response_budget_assertion(
            tool,
            &response,
            max_response_bytes,
            must_contain_paths,
            must_not_contain_paths,
        ),
        Err(error) => {
            AssertionResult::error(format!("tool_response_budget:{tool}"), error.to_string())
        }
    }
}

async fn evaluate_tool_field_equals_assertion(
    ctx: BridgeAssertionContext<'_>,
    tool: &str,
    params: &JsonValue,
    field: &str,
    expected: &JsonValue,
) -> AssertionResult {
    match call_tool(ctx.search, tool, params.clone()).await {
        Ok(response) => tool_field_equals_assertion(tool, &response, field, expected),
        Err(error) => AssertionResult::error(
            format!("tool_field_equals:{tool}:{field}"),
            error.to_string(),
        ),
    }
}

async fn call_tool(
    search: &ManifestSearch,
    tool: &str,
    params: JsonValue,
) -> crate::error::Result<JsonValue> {
    let started = Instant::now();
    let result = crate::cli::tool::dispatch_tool(search, tool, params.clone()).await;
    match &result {
        Ok(response) => crate::utils::tool_trace::record_tool_call(
            "eval",
            tool,
            Some(&params),
            Some(response),
            true,
            elapsed_ms_to_u64(started.elapsed()),
        ),
        Err(error) => crate::utils::tool_trace::record_tool_call(
            "eval",
            tool,
            Some(&params),
            Some(&json!({"error_code": error.error_code(), "message": error.to_string()})),
            false,
            elapsed_ms_to_u64(started.elapsed()),
        ),
    }
    result
}

async fn run_agent_case(
    case: &AgentCase,
    suite: &EvalSuite,
    args: &EvalAgentRunArgs,
    output_dir: &Path,
) -> EvalCaseReport {
    let date_anchor = effective_date_anchor(&suite.date_anchor, &case.date_anchor);
    let (case_dir, trace_path) =
        match prepare_agent_case_paths(case, output_dir, date_anchor.clone()) {
            Ok(paths) => paths,
            Err(report) => return *report,
        };
    let prompt = agent_prompt(
        case,
        date_anchor.as_ref(),
        suite.defaults.persona.as_deref(),
    );
    let invocation = match provider::provider_invocation(args, &prompt, &trace_path) {
        Ok(invocation) => invocation,
        Err(error) => {
            return agent_case_error_report(
                case,
                date_anchor,
                "provider_exit_success",
                error.to_string(),
            );
        }
    };

    let (stdout, assertions) =
        run_agent_provider(case, args, &case_dir, &trace_path, &invocation).await;
    finish_agent_case_report(
        case,
        date_anchor,
        &case_dir,
        &trace_path,
        &stdout,
        assertions,
    )
}

fn prepare_agent_case_paths(
    case: &AgentCase,
    output_dir: &Path,
    date_anchor: Option<DateAnchor>,
) -> std::result::Result<(PathBuf, PathBuf), Box<EvalCaseReport>> {
    let case_dir = output_dir.join(safe_path_segment(&case.id));
    if let Err(error) = fs::create_dir_all(&case_dir) {
        return Err(Box::new(agent_case_error_report(
            case,
            date_anchor,
            "provider_exit_success",
            format!("failed to create case artifact directory: {error}"),
        )));
    }
    let trace_path = output_dir
        .join("tool-calls")
        .join(format!("{}.jsonl", safe_path_segment(&case.id)));
    if let Err(error) = reset_trace_file(&trace_path) {
        return Err(Box::new(agent_case_error_report(
            case,
            date_anchor,
            "tool_trace_reset",
            format!("failed to reset tool trace file: {error}"),
        )));
    }
    Ok((case_dir, trace_path))
}

async fn run_agent_provider(
    case: &AgentCase,
    args: &EvalAgentRunArgs,
    case_dir: &Path,
    trace_path: &Path,
    invocation: &provider::ProviderInvocation,
) -> (String, Vec<AssertionResult>) {
    let output = provider::run_provider_command(invocation, args, trace_path, case_dir).await;
    match output {
        Ok(output) => {
            write_provider_artifact(case, case_dir, "stdout.log", &output.stdout, "stdout");
            write_provider_artifact(case, case_dir, "stderr.log", &output.stderr, "stderr");
            let assertion = if output.status_success {
                AssertionResult::pass(
                    "provider_exit_success",
                    "provider command exited successfully",
                    provider_invocation_evidence(invocation),
                )
            } else {
                AssertionResult::fail(
                    "provider_exit_success",
                    "provider command exited with a non-zero status",
                    provider_failure_evidence(invocation, &output.stderr),
                )
            };
            (output.stdout, vec![assertion])
        }
        Err(error) => (
            String::new(),
            vec![AssertionResult::error(
                "provider_exit_success",
                error.to_string(),
            )],
        ),
    }
}

fn write_provider_artifact(
    case: &AgentCase,
    case_dir: &Path,
    file_name: &str,
    content: &str,
    label: &str,
) {
    if let Err(error) = fs::write(
        case_dir.join(file_name),
        provider_output_for_artifact(content),
    ) {
        tracing::warn!(error = %error, case_id = %case.id, "failed to write eval {label}");
    }
}

fn finish_agent_case_report(
    case: &AgentCase,
    date_anchor: Option<DateAnchor>,
    case_dir: &Path,
    trace_path: &Path,
    stdout: &str,
    mut assertions: Vec<AssertionResult>,
) -> EvalCaseReport {
    let mut trace = read_tool_trace(trace_path);
    if trace.rows.is_empty() {
        let provider_trace = provider::read_provider_tool_trace(stdout);
        if !provider_trace.rows.is_empty() {
            trace.rows = provider_trace.rows;
            trace.errors.extend(provider_trace.errors);
            trace.missing = false;
        }
    }
    normalize_tool_trace_indices(&mut trace.rows);
    if trace.rows.is_empty() && case.expected.requires_trace() {
        assertions.push(AssertionResult::error(
            "tool_trace_missing",
            "tool trace rows were not observed; verify the provider launches a local dbt-nova process, emits MCP tool events, or writes trace rows",
        ));
    }
    for error in &trace.errors {
        assertions.push(AssertionResult::error("tool_trace_parse", error.clone()));
    }
    let final_answer_text =
        provider::read_provider_final_answer(stdout).unwrap_or_else(|| stdout.to_string());
    assertions.extend(score_agent_expectations(
        &case.expected,
        &trace.rows,
        &final_answer_text,
    ));
    let telemetry = eval_case_telemetry_from_trace(&trace.rows);
    EvalCaseReport::new(
        case.id.clone(),
        Some(case.task.clone()),
        assertions,
        Some(AgentArtifacts {
            stdout: case_dir.join("stdout.log").display().to_string(),
            stderr: case_dir.join("stderr.log").display().to_string(),
            tool_trace: trace_path.display().to_string(),
        }),
    )
    .with_date_anchor(date_anchor)
    .with_telemetry(telemetry)
}

fn agent_case_error_report(
    case: &AgentCase,
    date_anchor: Option<DateAnchor>,
    assertion_name: &str,
    message: impl Into<String>,
) -> EvalCaseReport {
    EvalCaseReport::new(
        case.id.clone(),
        Some(case.task.clone()),
        vec![AssertionResult::error(assertion_name, message)],
        None,
    )
    .with_date_anchor(date_anchor)
}

fn effective_persona(
    assertion_persona: Option<&String>,
    case: &EvalCase,
    suite: &EvalSuite,
) -> Option<String> {
    assertion_persona
        .cloned()
        .or_else(|| case.persona.clone())
        .or_else(|| suite.defaults.persona.clone())
}

#[cfg(test)]
mod tests;
