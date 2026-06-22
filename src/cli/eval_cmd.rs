#![allow(clippy::too_many_lines)]

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
    EvalAgentRunArgs, EvalGateArgs, EvalHistoryArgs, EvalInitArgs, EvalRunArgs, EvalValidateArgs,
    ManifestLoadArgs,
};
use crate::cli::manifest::{build_manifest_load_config, execute_manifest_load};
use crate::cli::output::{CliEnvelope, error_envelope};
use crate::cli::{DispatchError, DispatchResult};
use crate::error::DbtNovaError;
use crate::manifest::ManifestSearch;
use crate::params::{
    GetEvalGateParams, GetEvalHistoryParams, InitEvalSuiteParams, RunAgentEvalParams,
    RunEvalParams, ValidateEvalSuiteParams,
};
use crate::responses::SuccessResponse;
use crate::utils::tool_trace::{normalize_tool_trace_indices, read_tool_trace_file};

const DEFAULT_TOP_K: usize = 5;
const DEFAULT_FAIL_UNDER: f64 = 1.0;
const MAX_SAFE_PATH_SEGMENT_CHARS: usize = 120;
const DEFAULT_TELEMETRY_DIR: &str = ".nova/eval-runs/telemetry";

mod provider;

#[derive(Debug, Deserialize)]
struct EvalSuite {
    version: u32,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    purpose: Option<String>,
    #[serde(default)]
    manifest_scope: Option<String>,
    #[serde(default)]
    known_gaps: Vec<String>,
    #[serde(default)]
    gate: Option<EvalGateConfig>,
    #[serde(flatten)]
    date_anchor: DateAnchor,
    #[serde(default)]
    defaults: EvalDefaults,
    #[serde(default)]
    cases: Vec<EvalCase>,
    #[serde(default)]
    agent_cases: Vec<AgentCase>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
struct EvalGateConfig {
    threshold: f64,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
struct DateAnchor {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    snapshot_date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    date_range_start: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    date_range_end: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    date_field: Option<String>,
}

impl DateAnchor {
    fn normalized(&self) -> Option<Self> {
        let anchor = Self {
            snapshot_date: normalized_string(self.snapshot_date.as_ref()),
            date_range_start: normalized_string(self.date_range_start.as_ref()),
            date_range_end: normalized_string(self.date_range_end.as_ref()),
            date_field: normalized_string(self.date_field.as_ref()),
        };
        (!anchor.is_empty()).then_some(anchor)
    }

    fn is_empty(&self) -> bool {
        self.snapshot_date
            .as_deref()
            .is_none_or(|value| value.trim().is_empty())
            && self
                .date_range_start
                .as_deref()
                .is_none_or(|value| value.trim().is_empty())
            && self
                .date_range_end
                .as_deref()
                .is_none_or(|value| value.trim().is_empty())
            && self
                .date_field
                .as_deref()
                .is_none_or(|value| value.trim().is_empty())
    }

    fn prompt_lines(&self) -> Vec<String> {
        self.markdown_lines()
            .into_iter()
            .map(|line| line.replace('`', ""))
            .collect()
    }

    fn markdown_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        if let Some(value) = self.snapshot_date.as_deref() {
            lines.push(format!("snapshot_date: `{value}`"));
        }
        match (
            self.date_range_start.as_deref(),
            self.date_range_end.as_deref(),
        ) {
            (Some(start), Some(end)) => lines.push(format!("date_range: `{start}` to `{end}`")),
            (Some(start), None) => lines.push(format!("date_range_start: `{start}`")),
            (None, Some(end)) => lines.push(format!("date_range_end: `{end}`")),
            (None, None) => {}
        }
        if let Some(value) = self.date_field.as_deref() {
            lines.push(format!("date_field: `{value}`"));
        }
        lines
    }
}

#[derive(Debug, Default, Deserialize)]
struct EvalDefaults {
    #[serde(default)]
    persona: Option<String>,
    #[serde(default = "default_top_k")]
    top_k: usize,
}

#[derive(Debug, Deserialize)]
struct EvalCase {
    id: String,
    #[serde(default)]
    question: Option<String>,
    #[serde(default)]
    persona: Option<String>,
    #[serde(flatten)]
    date_anchor: DateAnchor,
    assertions: Vec<EvalAssertion>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum EvalAssertion {
    SearchRank {
        query: String,
        expected_unique_id: String,
        #[serde(default)]
        max_rank: Option<usize>,
        #[serde(default)]
        resource_types: Vec<String>,
        #[serde(default)]
        persona: Option<String>,
    },
    SearchIndicatorRank {
        query: String,
        expected: String,
        #[serde(default)]
        max_rank: Option<usize>,
        #[serde(default)]
        resource_types: Vec<String>,
        #[serde(default)]
        indicator_types: Vec<String>,
        #[serde(default)]
        persona: Option<String>,
    },
    SearchColumnsRank {
        query: String,
        #[serde(default)]
        expected_column: Option<String>,
        #[serde(default)]
        expected_parent_unique_id: Option<String>,
        #[serde(default)]
        max_rank: Option<usize>,
    },
    ContextHas {
        id_or_name: String,
        fields: Vec<String>,
    },
    ContextFieldEquals {
        id_or_name: String,
        field: String,
        expected: JsonValue,
    },
    ContextContains {
        id_or_name: String,
        expected: String,
        #[serde(default)]
        field: Option<String>,
    },
    MetadataScoreMin {
        #[serde(default)]
        id_or_name: Option<String>,
        threshold: f64,
        #[serde(default)]
        persona: Option<String>,
    },
    MetadataScoreMax {
        #[serde(default)]
        id_or_name: Option<String>,
        threshold: f64,
        #[serde(default)]
        persona: Option<String>,
    },
    RecipeRank {
        query: String,
        expected_recipe_id: String,
        #[serde(default)]
        max_rank: Option<usize>,
    },
    RecipeHasQueries {
        recipe_id: String,
        #[serde(default)]
        min_queries: Option<usize>,
    },
    LineageContains {
        id_or_name: String,
        direction: String,
        expected_unique_id: String,
        #[serde(default)]
        depth: Option<usize>,
    },
    ToolSuccess {
        tool: String,
        #[serde(default = "empty_object")]
        params: JsonValue,
    },
    ToolResponseBudget {
        tool: String,
        #[serde(default = "empty_object")]
        params: JsonValue,
        max_response_bytes: usize,
        #[serde(default)]
        must_contain_paths: Vec<String>,
        #[serde(default)]
        must_not_contain_paths: Vec<String>,
    },
}

#[derive(Debug, Deserialize)]
struct AgentCase {
    id: String,
    task: String,
    #[serde(flatten)]
    date_anchor: DateAnchor,
    #[serde(default)]
    expected: AgentExpected,
}

#[derive(Debug, Default, Deserialize)]
struct AgentExpected {
    #[serde(default)]
    must_call: Vec<String>,
    #[serde(default)]
    must_not_call: Vec<String>,
    #[serde(default)]
    ordered: Vec<AgentOrder>,
    #[serde(default)]
    selected_entities: Vec<String>,
    #[serde(default)]
    selected_entity_ranks: Vec<AgentEntityRank>,
    #[serde(default)]
    called_with: Vec<AgentCalledWith>,
    #[serde(default)]
    final_answer: Option<FinalAnswerExpected>,
    #[serde(default)]
    max_tool_calls: Option<usize>,
    #[serde(default)]
    max_distinct_tools: Option<usize>,
    #[serde(default)]
    max_total_response_bytes: Option<usize>,
    #[serde(default)]
    max_response_bytes_by_tool: BTreeMap<String, usize>,
}

#[derive(Debug, Deserialize)]
struct AgentOrder {
    before: String,
    #[serde(default)]
    must_have_called: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct AgentEntityRank {
    unique_id: String,
    #[serde(default)]
    tool: Option<String>,
    #[serde(default)]
    max_rank: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct AgentCalledWith {
    tool: String,
    #[serde(default)]
    params: BTreeMap<String, JsonValue>,
    #[serde(default)]
    contains: BTreeMap<String, String>,
}

#[derive(Debug, Default, Deserialize)]
struct FinalAnswerExpected {
    #[serde(default)]
    must_contain: Vec<String>,
    #[serde(default)]
    must_not_contain: Vec<String>,
}

#[derive(Debug, Serialize)]
struct EvalReport {
    suite_name: String,
    version: u32,
    mode: &'static str,
    output_dir: String,
    eval_card: EvalCard,
    assertion_count: usize,
    pass_count: usize,
    fail_count: usize,
    error_count: usize,
    pass_rate: f64,
    fail_under: f64,
    gate_status: &'static str,
    cases: Vec<EvalCaseReport>,
}

#[derive(Debug, Clone, Serialize)]
struct EvalCard {
    schema_version: &'static str,
    suite_name: String,
    version: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    suite_path: Option<String>,
    purpose: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    persona: Option<String>,
    manifest_scope: EvalCardManifestScope,
    mode: &'static str,
    bridge_case_count: usize,
    agent_case_count: usize,
    run_case_count: usize,
    output_dir: String,
    assertion_count: usize,
    pass_count: usize,
    fail_count: usize,
    error_count: usize,
    pass_rate: f64,
    fail_under: f64,
    run_status: &'static str,
    gate: EvalCardGate,
    telemetry: EvalCardTelemetry,
    #[serde(skip_serializing_if = "Option::is_none")]
    date_anchor: Option<DateAnchor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    provider: Option<EvalCardProvider>,
    known_gaps: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
struct EvalCardManifestScope {
    declared: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    manifest_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    manifest_source: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct EvalCardGate {
    status: String,
    source: &'static str,
    configured: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    threshold: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pass_rate: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    total_evals: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    failed_evals: Option<usize>,
    message: String,
}

#[derive(Debug, Clone, Serialize)]
struct EvalCardTelemetry {
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    timestamp: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    run_id: Option<String>,
    row_count: usize,
    message: String,
}

#[derive(Debug, Clone, Serialize)]
struct EvalCardProvider {
    provider: String,
    command_preset: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
}

#[derive(Debug, Serialize)]
struct EvalCaseReport {
    id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    question: Option<String>,
    pass_count: usize,
    fail_count: usize,
    error_count: usize,
    assertions: Vec<AssertionResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    date_anchor: Option<DateAnchor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    artifacts: Option<AgentArtifacts>,
    #[serde(skip)]
    telemetry: Option<EvalCaseTelemetry>,
}

#[derive(Debug, Serialize)]
struct AssertionResult {
    name: String,
    status: &'static str,
    message: String,
    #[serde(skip_serializing_if = "JsonValue::is_null")]
    evidence: JsonValue,
}

#[derive(Debug, Serialize)]
struct AgentArtifacts {
    stdout: String,
    stderr: String,
    tool_trace: String,
}

#[derive(Debug, Clone)]
struct EvalCaseTelemetry {
    tool_call_count: usize,
    distinct_tool_count: usize,
    total_response_bytes: Option<u64>,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    total_tokens: Option<u64>,
}

#[derive(Debug, Clone, Copy)]
struct AgentTelemetryContext<'a> {
    provider: &'a str,
    provider_command_preset: &'a str,
}

#[derive(Debug, Clone, Copy)]
struct EvalTelemetryRunContext<'a> {
    suite_path: &'a str,
    suite_hash: &'a str,
    suite_case_count: usize,
    manifest_hash: Option<&'a str>,
    duration_ms: u64,
    retention: Option<usize>,
    agent: Option<AgentTelemetryContext<'a>>,
}

#[derive(Debug, Serialize)]
struct EvalGateReport {
    suite_name: String,
    allowed: bool,
    blocked: bool,
    gate_configured: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    threshold: Option<f64>,
    pass_rate: f64,
    total_evals: usize,
    failed_evals: usize,
    failed_eval_ids: Vec<String>,
    failed_case_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    telemetry_timestamp: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output_dir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    suite_path: Option<String>,
    message: String,
}

enum GateConfigStatus {
    Configured {
        gate: EvalGateConfig,
        suite_hash: String,
    },
    Unconfigured,
    Unavailable(String),
}

#[derive(Debug, Serialize)]
struct EvalMcpSafetyPolicy {
    filesystem_root: String,
    eval_run_enabled_env: &'static str,
    eval_writes_enabled_env: &'static str,
    agent_eval_enabled_env: &'static str,
    custom_agent_provider_enabled_env: &'static str,
    local_paths_must_stay_under_filesystem_root: bool,
}

#[derive(Debug, Serialize)]
struct EvalHistoryPayload {
    suite_name: String,
    since: String,
    row_count: usize,
    rows: Vec<JsonValue>,
    safety_policy: EvalMcpSafetyPolicy,
}

#[derive(Debug, Clone, Default)]
struct EvalCardRunContext {
    suite_path: Option<String>,
    manifest_hash: Option<String>,
    manifest_source: Option<String>,
    telemetry_requested: bool,
    provider: Option<EvalCardProvider>,
}

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
    ensure_mcp_telemetry_suite_paths_under_root(&rows)?;
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
    match assertion {
        EvalAssertion::SearchRank {
            query,
            expected_unique_id,
            max_rank,
            resource_types,
            persona,
        } => {
            let limit = effective_limit(*max_rank, suite.defaults.top_k);
            let params = json!({
                "query": query,
                "resource_types": resource_types,
                "persona": effective_persona(persona.as_ref(), case, suite),
                "limit": limit,
            });
            match call_tool(search, "search", params).await {
                Ok(response) => rank_assertion(
                    "search_rank",
                    &response,
                    expected_unique_id,
                    *max_rank,
                    "unique_id",
                ),
                Err(error) => AssertionResult::error("search_rank", error.to_string()),
            }
        }
        EvalAssertion::SearchIndicatorRank {
            query,
            expected,
            max_rank,
            resource_types,
            indicator_types,
            persona,
        } => {
            let limit = effective_limit(*max_rank, suite.defaults.top_k);
            let params = json!({
                "query": query,
                "resource_types": resource_types,
                "indicator_types": indicator_types,
                "persona": effective_persona(persona.as_ref(), case, suite),
                "limit": limit,
            });
            match call_tool(search, "search_indicator", params).await {
                Ok(response) => {
                    contains_rank_assertion("search_indicator_rank", &response, expected, *max_rank)
                }
                Err(error) => AssertionResult::error("search_indicator_rank", error.to_string()),
            }
        }
        EvalAssertion::SearchColumnsRank {
            query,
            expected_column,
            expected_parent_unique_id,
            max_rank,
        } => {
            let limit = effective_limit(*max_rank, suite.defaults.top_k);
            let params = json!({"query": query, "limit": limit});
            match call_tool(search, "search_columns", params).await {
                Ok(response) => search_columns_assertion(
                    &response,
                    expected_column.as_deref(),
                    expected_parent_unique_id.as_deref(),
                    *max_rank,
                ),
                Err(error) => AssertionResult::error("search_columns_rank", error.to_string()),
            }
        }
        EvalAssertion::ContextHas { id_or_name, fields } => {
            let params = json!({"id_or_name": id_or_name});
            match call_tool(search, "get_context", params).await {
                Ok(response) => fields_assertion("context_has", &response, fields),
                Err(error) => AssertionResult::error("context_has", error.to_string()),
            }
        }
        EvalAssertion::ContextFieldEquals {
            id_or_name,
            field,
            expected,
        } => {
            let params = json!({"id_or_name": id_or_name});
            match call_tool(search, "get_context", params).await {
                Ok(response) => context_field_equals_assertion(&response, field, expected),
                Err(error) => AssertionResult::error("context_field_equals", error.to_string()),
            }
        }
        EvalAssertion::ContextContains {
            id_or_name,
            expected,
            field,
        } => {
            let params = json!({"id_or_name": id_or_name});
            match call_tool(search, "get_context", params).await {
                Ok(response) => context_contains_assertion(&response, field.as_deref(), expected),
                Err(error) => AssertionResult::error("context_contains", error.to_string()),
            }
        }
        EvalAssertion::MetadataScoreMin {
            id_or_name,
            threshold,
            persona,
        }
        | EvalAssertion::MetadataScoreMax {
            id_or_name,
            threshold,
            persona,
        } => {
            let params = json!({
                "id_or_name": id_or_name,
                "persona": effective_persona(persona.as_ref(), case, suite),
                "limit": 20,
            });
            match call_tool(search, "get_metadata_score", params).await {
                Ok(response) => match assertion {
                    EvalAssertion::MetadataScoreMin { .. } => {
                        metadata_score_min_assertion(&response, *threshold)
                    }
                    EvalAssertion::MetadataScoreMax { .. } => {
                        metadata_score_max_assertion(&response, *threshold)
                    }
                    _ => unreachable!("matched metadata score assertion"),
                },
                Err(error) => AssertionResult::error(
                    match assertion {
                        EvalAssertion::MetadataScoreMin { .. } => "metadata_score_min",
                        EvalAssertion::MetadataScoreMax { .. } => "metadata_score_max",
                        _ => unreachable!("matched metadata score assertion"),
                    },
                    error.to_string(),
                ),
            }
        }
        EvalAssertion::RecipeRank {
            query,
            expected_recipe_id,
            max_rank,
        } => {
            let limit = effective_limit(*max_rank, suite.defaults.top_k);
            let params = json!({"query": query, "include_queries": false, "limit": limit});
            match call_tool(search, "search_recipes", params).await {
                Ok(response) => recipe_rank_assertion(&response, expected_recipe_id, *max_rank),
                Err(error) => AssertionResult::error("recipe_rank", error.to_string()),
            }
        }
        EvalAssertion::RecipeHasQueries {
            recipe_id,
            min_queries,
        } => {
            let params = json!({"recipe_id": recipe_id, "include_queries": true});
            match call_tool(search, "get_recipe", params).await {
                Ok(response) => recipe_queries_assertion(&response, min_queries.unwrap_or(1)),
                Err(error) => AssertionResult::error("recipe_has_queries", error.to_string()),
            }
        }
        EvalAssertion::LineageContains {
            id_or_name,
            direction,
            expected_unique_id,
            depth,
        } => {
            let params = json!({
                "id_or_name": id_or_name,
                "direction": direction,
                "depth": depth.unwrap_or(2),
            });
            match call_tool(search, "get_lineage", params).await {
                Ok(response) => {
                    contains_string_assertion("lineage_contains", &response, expected_unique_id)
                }
                Err(error) => AssertionResult::error("lineage_contains", error.to_string()),
            }
        }
        EvalAssertion::ToolSuccess { tool, params } => {
            match call_tool(search, tool, params.clone()).await {
                Ok(response) => tool_success_assertion(tool, &response),
                Err(error) => {
                    AssertionResult::error(format!("tool_success:{tool}"), error.to_string())
                }
            }
        }
        EvalAssertion::ToolResponseBudget {
            tool,
            params,
            max_response_bytes,
            must_contain_paths,
            must_not_contain_paths,
        } => match call_tool(search, tool, params.clone()).await {
            Ok(response) => tool_response_budget_assertion(
                tool,
                &response,
                *max_response_bytes,
                must_contain_paths,
                must_not_contain_paths,
            ),
            Err(error) => {
                AssertionResult::error(format!("tool_response_budget:{tool}"), error.to_string())
            }
        },
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
    let case_dir = output_dir.join(safe_path_segment(&case.id));
    if let Err(error) = fs::create_dir_all(&case_dir) {
        return EvalCaseReport::new(
            case.id.clone(),
            Some(case.task.clone()),
            vec![AssertionResult::error(
                "provider_exit_success",
                format!("failed to create case artifact directory: {error}"),
            )],
            None,
        )
        .with_date_anchor(date_anchor);
    }
    let trace_path = output_dir
        .join("tool-calls")
        .join(format!("{}.jsonl", safe_path_segment(&case.id)));
    if let Err(error) = reset_trace_file(&trace_path) {
        return EvalCaseReport::new(
            case.id.clone(),
            Some(case.task.clone()),
            vec![AssertionResult::error(
                "tool_trace_reset",
                format!("failed to reset tool trace file: {error}"),
            )],
            None,
        )
        .with_date_anchor(date_anchor);
    }
    let prompt = agent_prompt(case, date_anchor.as_ref());
    let invocation = match provider::provider_invocation(args, &prompt, &trace_path) {
        Ok(invocation) => invocation,
        Err(error) => {
            return EvalCaseReport::new(
                case.id.clone(),
                Some(case.task.clone()),
                vec![AssertionResult::error(
                    "provider_exit_success",
                    error.to_string(),
                )],
                None,
            )
            .with_date_anchor(date_anchor);
        }
    };

    let output = provider::run_provider_command(&invocation, args, &trace_path, &case_dir).await;
    let (stdout, _stderr, assertions) = match output {
        Ok(output) => {
            if let Err(error) = fs::write(case_dir.join("stdout.log"), &output.stdout) {
                tracing::warn!(error = %error, case_id = %case.id, "failed to write eval stdout");
            }
            if let Err(error) = fs::write(case_dir.join("stderr.log"), &output.stderr) {
                tracing::warn!(error = %error, case_id = %case.id, "failed to write eval stderr");
            }
            let mut assertions = Vec::new();
            if output.status_success {
                assertions.push(AssertionResult::pass(
                    "provider_exit_success",
                    "provider command exited successfully",
                    json!({"command": invocation.command, "args": invocation.args}),
                ));
            } else {
                assertions.push(AssertionResult::fail(
                    "provider_exit_success",
                    "provider command exited with a non-zero status",
                    json!({
                        "command": invocation.command,
                        "args": invocation.args,
                        "stderr": truncate(&output.stderr, 4000),
                    }),
                ));
            }
            (output.stdout, output.stderr, assertions)
        }
        Err(error) => (
            String::new(),
            String::new(),
            vec![AssertionResult::error(
                "provider_exit_success",
                error.to_string(),
            )],
        ),
    };

    let mut assertions = assertions;
    let mut trace = read_tool_trace(&trace_path);
    if trace.rows.is_empty() {
        let provider_trace = provider::read_provider_tool_trace(&stdout);
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
        provider::read_provider_final_answer(&stdout).unwrap_or_else(|| stdout.clone());
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

fn score_agent_expectations(
    expected: &AgentExpected,
    trace: &[JsonValue],
    final_answer_text: &str,
) -> Vec<AssertionResult> {
    let mut assertions = Vec::new();
    for tool in &expected.must_call {
        if first_tool_index(trace, tool).is_some() {
            assertions.push(AssertionResult::pass(
                format!("must_call:{tool}"),
                "required tool was called",
                JsonValue::Null,
            ));
        } else {
            assertions.push(AssertionResult::fail(
                format!("must_call:{tool}"),
                "required tool was not called",
                json!({"observed_tools": observed_tools(trace)}),
            ));
        }
    }
    for tool in &expected.must_not_call {
        if first_tool_index(trace, tool).is_some() {
            assertions.push(AssertionResult::fail(
                format!("must_not_call:{tool}"),
                "forbidden tool was called",
                json!({"observed_tools": observed_tools(trace)}),
            ));
        } else {
            assertions.push(AssertionResult::pass(
                format!("must_not_call:{tool}"),
                "forbidden tool was not called",
                JsonValue::Null,
            ));
        }
    }
    for order in &expected.ordered {
        assertions.push(order_assertion(trace, order));
    }
    for entity in &expected.selected_entities {
        if trace_selected_entity(trace, entity) {
            assertions.push(AssertionResult::pass(
                format!("selected_entity:{entity}"),
                "expected entity appeared in tool evidence",
                JsonValue::Null,
            ));
        } else {
            assertions.push(AssertionResult::fail(
                format!("selected_entity:{entity}"),
                "expected entity did not appear in tool evidence",
                json!({"selected_unique_ids": selected_entities(trace)}),
            ));
        }
    }
    for entity_rank in &expected.selected_entity_ranks {
        assertions.push(selected_entity_rank_assertion(trace, entity_rank));
    }
    for called_with in &expected.called_with {
        assertions.push(called_with_assertion(trace, called_with));
    }
    if let Some(max_tool_calls) = expected.max_tool_calls {
        assertions.push(max_tool_calls_assertion(trace, max_tool_calls));
    }
    if let Some(max_distinct_tools) = expected.max_distinct_tools {
        assertions.push(max_distinct_tools_assertion(trace, max_distinct_tools));
    }
    if let Some(max_total_response_bytes) = expected.max_total_response_bytes {
        assertions.push(max_total_response_bytes_assertion(
            trace,
            max_total_response_bytes,
        ));
    }
    for (tool, max_bytes) in &expected.max_response_bytes_by_tool {
        assertions.push(max_response_bytes_by_tool_assertion(
            trace, tool, *max_bytes,
        ));
    }
    if let Some(final_answer) = expected.final_answer.as_ref() {
        assertions.extend(score_final_answer(final_answer, final_answer_text));
    }
    assertions
}

fn max_tool_calls_assertion(trace: &[JsonValue], max_tool_calls: usize) -> AssertionResult {
    let observed = trace.len();
    if observed <= max_tool_calls {
        AssertionResult::pass(
            "max_tool_calls",
            "observed tool call count stayed within budget",
            json!({"observed": observed, "max": max_tool_calls}),
        )
    } else {
        AssertionResult::fail(
            "max_tool_calls",
            "observed tool call count exceeded budget",
            json!({"observed": observed, "max": max_tool_calls, "tools": observed_tools(trace)}),
        )
    }
}

fn max_distinct_tools_assertion(trace: &[JsonValue], max_distinct_tools: usize) -> AssertionResult {
    let distinct: BTreeSet<String> = trace
        .iter()
        .filter_map(|row| row.get("tool").and_then(JsonValue::as_str))
        .map(ToString::to_string)
        .collect();
    let observed = distinct.len();
    if observed <= max_distinct_tools {
        AssertionResult::pass(
            "max_distinct_tools",
            "observed distinct tool count stayed within budget",
            json!({"observed": observed, "max": max_distinct_tools}),
        )
    } else {
        AssertionResult::fail(
            "max_distinct_tools",
            "observed distinct tool count exceeded budget",
            json!({"observed": observed, "max": max_distinct_tools, "tools": distinct}),
        )
    }
}

fn max_total_response_bytes_assertion(
    trace: &[JsonValue],
    max_total_response_bytes: usize,
) -> AssertionResult {
    let missing_response_bytes = trace_rows_missing_response_bytes(trace);
    if missing_response_bytes > 0 {
        return AssertionResult::fail(
            "max_total_response_bytes",
            "tool trace rows were missing response byte telemetry",
            json!({"missing_response_bytes": missing_response_bytes, "trace_rows": trace.len()}),
        );
    }
    let observed = total_response_bytes(trace);
    if observed <= max_total_response_bytes {
        AssertionResult::pass(
            "max_total_response_bytes",
            "observed total response bytes stayed within budget",
            json!({"observed": observed, "max": max_total_response_bytes}),
        )
    } else {
        AssertionResult::fail(
            "max_total_response_bytes",
            "observed total response bytes exceeded budget",
            json!({"observed": observed, "max": max_total_response_bytes}),
        )
    }
}

fn max_response_bytes_by_tool_assertion(
    trace: &[JsonValue],
    tool: &str,
    max_response_bytes: usize,
) -> AssertionResult {
    let matching_rows: Vec<&JsonValue> = trace
        .iter()
        .filter(|row| row.get("tool").and_then(JsonValue::as_str) == Some(tool))
        .collect();
    let missing_response_bytes = matching_rows
        .iter()
        .filter(|row| response_bytes_from_trace_row(row).is_none())
        .count();
    if missing_response_bytes > 0 {
        return AssertionResult::fail(
            format!("max_response_bytes_by_tool:{tool}"),
            "tool trace rows were missing response byte telemetry",
            json!({"missing_response_bytes": missing_response_bytes, "tool": tool}),
        );
    }
    let observed = matching_rows
        .iter()
        .filter_map(|row| response_bytes_from_trace_row(row))
        .max()
        .unwrap_or(0);
    if observed <= max_response_bytes {
        AssertionResult::pass(
            format!("max_response_bytes_by_tool:{tool}"),
            "observed per-tool response bytes stayed within budget",
            json!({"observed": observed, "max": max_response_bytes}),
        )
    } else {
        AssertionResult::fail(
            format!("max_response_bytes_by_tool:{tool}"),
            "observed per-tool response bytes exceeded budget",
            json!({"observed": observed, "max": max_response_bytes}),
        )
    }
}

fn total_response_bytes(trace: &[JsonValue]) -> usize {
    trace.iter().filter_map(response_bytes_from_trace_row).sum()
}

fn trace_rows_missing_response_bytes(trace: &[JsonValue]) -> usize {
    trace
        .iter()
        .filter(|row| response_bytes_from_trace_row(row).is_none())
        .count()
}

fn response_bytes_from_trace_row(row: &JsonValue) -> Option<usize> {
    row.get("response_bytes")
        .and_then(JsonValue::as_u64)
        .and_then(|value| usize::try_from(value).ok())
}

fn order_assertion(trace: &[JsonValue], order: &AgentOrder) -> AssertionResult {
    let before_index = first_tool_index(trace, &order.before);
    let missing: Vec<&str> = order
        .must_have_called
        .iter()
        .filter_map(|tool| {
            let index = first_tool_index(trace, tool);
            if index.is_none()
                || before_index.is_none_or(|before| index.unwrap_or(usize::MAX) >= before)
            {
                Some(tool.as_str())
            } else {
                None
            }
        })
        .collect();

    if before_index.is_some() && missing.is_empty() {
        AssertionResult::pass(
            format!("order:{}", order.before),
            "tool order matched",
            JsonValue::Null,
        )
    } else {
        AssertionResult::fail(
            format!("order:{}", order.before),
            "tool order did not match",
            json!({
                "before": order.before,
                "must_have_called": order.must_have_called,
                "observed_tools": observed_tools(trace),
            }),
        )
    }
}

fn score_final_answer(
    expected: &FinalAnswerExpected,
    final_answer_text: &str,
) -> Vec<AssertionResult> {
    let haystack = final_answer_text.to_lowercase();
    let mut assertions = Vec::new();
    for needle in &expected.must_contain {
        if haystack.contains(&needle.to_lowercase()) {
            assertions.push(AssertionResult::pass(
                format!("final_answer_contains:{needle}"),
                "final answer contained expected text",
                JsonValue::Null,
            ));
        } else {
            assertions.push(AssertionResult::fail(
                format!("final_answer_contains:{needle}"),
                "final answer did not contain expected text",
                json!({"final_answer": truncate(final_answer_text, 4000)}),
            ));
        }
    }
    for needle in &expected.must_not_contain {
        if haystack.contains(&needle.to_lowercase()) {
            assertions.push(AssertionResult::fail(
                format!("final_answer_excludes:{needle}"),
                "final answer contained forbidden text",
                json!({"final_answer": truncate(final_answer_text, 4000)}),
            ));
        } else {
            assertions.push(AssertionResult::pass(
                format!("final_answer_excludes:{needle}"),
                "final answer excluded forbidden text",
                JsonValue::Null,
            ));
        }
    }
    assertions
}

fn first_tool_index(trace: &[JsonValue], tool: &str) -> Option<usize> {
    trace
        .iter()
        .position(|row| row.get("tool").and_then(JsonValue::as_str) == Some(tool))
}

fn observed_tools(trace: &[JsonValue]) -> Vec<String> {
    trace
        .iter()
        .filter_map(|row| row.get("tool").and_then(JsonValue::as_str))
        .map(ToString::to_string)
        .collect()
}

fn trace_selected_entity(trace: &[JsonValue], entity: &str) -> bool {
    trace.iter().any(|row| {
        row.get("selected_unique_ids")
            .and_then(JsonValue::as_array)
            .is_some_and(|ids| ids.iter().any(|id| id.as_str() == Some(entity)))
    })
}

fn selected_entities(trace: &[JsonValue]) -> Vec<String> {
    let mut out = BTreeSet::new();
    for row in trace {
        if let Some(ids) = row.get("selected_unique_ids").and_then(JsonValue::as_array) {
            for id in ids {
                if let Some(id) = id.as_str() {
                    out.insert(id.to_string());
                }
            }
        }
    }
    out.into_iter().collect()
}

fn selected_entity_rank_assertion(
    trace: &[JsonValue],
    expected: &AgentEntityRank,
) -> AssertionResult {
    let rank = trace_entity_rank(trace, &expected.unique_id, expected.tool.as_deref());
    match (rank, expected.max_rank) {
        (Some(rank), Some(max_rank)) if rank <= max_rank => AssertionResult::pass(
            format!("selected_entity_rank:{}", expected.unique_id),
            format!("expected entity appeared at rank {rank}"),
            json!({"rank": rank, "max_rank": max_rank, "tool": expected.tool}),
        ),
        (Some(rank), Some(max_rank)) => AssertionResult::fail(
            format!("selected_entity_rank:{}", expected.unique_id),
            format!("expected entity appeared at rank {rank}, above max rank {max_rank}"),
            json!({
                "rank": rank,
                "max_rank": max_rank,
                "tool": expected.tool,
                "top_unique_ids": top_unique_ids(trace, expected.tool.as_deref()),
            }),
        ),
        (Some(rank), None) => AssertionResult::pass(
            format!("selected_entity_rank:{}", expected.unique_id),
            format!("expected entity appeared at rank {rank}"),
            json!({"rank": rank, "tool": expected.tool}),
        ),
        (None, _) => AssertionResult::fail(
            format!("selected_entity_rank:{}", expected.unique_id),
            "expected entity did not appear in ranked tool evidence",
            json!({
                "tool": expected.tool,
                "top_unique_ids": top_unique_ids(trace, expected.tool.as_deref()),
            }),
        ),
    }
}

fn trace_entity_rank(trace: &[JsonValue], entity: &str, tool: Option<&str>) -> Option<usize> {
    trace
        .iter()
        .filter(|row| {
            tool.is_none_or(|tool| row.get("tool").and_then(JsonValue::as_str) == Some(tool))
        })
        .filter_map(|row| {
            row.get("top_unique_ids")
                .and_then(JsonValue::as_array)
                .and_then(|ids| ids.iter().position(|id| id.as_str() == Some(entity)))
        })
        .map(|index| index + 1)
        .min()
}

fn top_unique_ids(trace: &[JsonValue], tool: Option<&str>) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    for row in trace.iter().filter(|row| {
        tool.is_none_or(|tool| row.get("tool").and_then(JsonValue::as_str) == Some(tool))
    }) {
        if let Some(ids) = row.get("top_unique_ids").and_then(JsonValue::as_array) {
            for id in ids {
                if let Some(id) = id.as_str() {
                    let id = id.to_string();
                    if seen.insert(id.clone()) {
                        out.push(id);
                    }
                }
            }
        }
    }
    out
}

fn called_with_assertion(trace: &[JsonValue], expected: &AgentCalledWith) -> AssertionResult {
    if trace.iter().any(|row| called_with_matches(row, expected)) {
        return AssertionResult::pass(
            format!("called_with:{}", expected.tool),
            "tool call parameters matched",
            json!({"tool": expected.tool}),
        );
    }
    AssertionResult::fail(
        format!("called_with:{}", expected.tool),
        "no observed tool call matched the expected safe parameters",
        json!({
            "expected": {
                "params": &expected.params,
                "contains": &expected.contains,
            },
            "observed": observed_params_for_tool(trace, &expected.tool),
        }),
    )
}

fn called_with_matches(row: &JsonValue, expected: &AgentCalledWith) -> bool {
    if row.get("tool").and_then(JsonValue::as_str) != Some(expected.tool.as_str()) {
        return false;
    }
    let Some(summary) = row.get("params_summary").and_then(JsonValue::as_object) else {
        return expected.params.is_empty() && expected.contains.is_empty();
    };
    expected.params.iter().all(|(key, value)| {
        summary
            .get(key)
            .is_some_and(|actual| param_value_matches(actual, value))
    }) && expected.contains.iter().all(|(key, value)| {
        summary
            .get(key)
            .is_some_and(|actual| json_contains_string(actual, value))
    })
}

fn param_value_matches(actual: &JsonValue, expected: &JsonValue) -> bool {
    match (actual, expected) {
        (JsonValue::String(actual), JsonValue::String(expected)) => {
            actual.eq_ignore_ascii_case(expected)
        }
        (JsonValue::Array(actual_items), JsonValue::Array(expected_items)) => {
            expected_items.iter().all(|expected| {
                actual_items
                    .iter()
                    .any(|actual| param_value_matches(actual, expected))
            })
        }
        (JsonValue::Array(actual_items), expected) => actual_items
            .iter()
            .any(|actual| param_value_matches(actual, expected)),
        _ => actual == expected,
    }
}

fn observed_params_for_tool(trace: &[JsonValue], tool: &str) -> JsonValue {
    JsonValue::Array(
        trace
            .iter()
            .filter(|row| row.get("tool").and_then(JsonValue::as_str) == Some(tool))
            .filter_map(|row| row.get("params_summary").cloned())
            .collect(),
    )
}

struct ToolTraceRead {
    rows: Vec<JsonValue>,
    errors: Vec<String>,
    missing: bool,
}

fn reset_trace_file(path: &Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, "")
}

fn read_tool_trace(path: &Path) -> ToolTraceRead {
    let read = read_tool_trace_file(path);
    let mut errors = Vec::new();
    if let Some(error) = read.read_error {
        errors.push(error);
    }
    errors.extend(read.parse_warnings.into_iter().map(|warning| {
        format!(
            "failed to parse tool trace line {} in '{}': {}",
            warning.line,
            path.display(),
            warning.message
        )
    }));
    ToolTraceRead {
        rows: read.rows,
        errors,
        missing: read.missing,
    }
}

fn rank_assertion(
    name: &str,
    response: &JsonValue,
    expected: &str,
    max_rank: Option<usize>,
    field: &str,
) -> AssertionResult {
    let rows = data_rows(response);
    if let Some(index) = rows
        .iter()
        .position(|row| string_field_equals(row, field, expected))
    {
        let rank = index + 1;
        if max_rank.is_none_or(|max| rank <= max) {
            AssertionResult::pass(
                name,
                format!("expected item ranked {rank}"),
                json!({"rank": rank, "expected": expected}),
            )
        } else {
            AssertionResult::fail(
                name,
                format!(
                    "expected item ranked {rank}, above max rank {}",
                    max_rank.unwrap_or(0)
                ),
                top_evidence(rows, field),
            )
        }
    } else {
        AssertionResult::fail(
            name,
            "expected item was not returned",
            json!({"expected": expected, "top": top_evidence(rows, field)}),
        )
    }
}

fn contains_rank_assertion(
    name: &str,
    response: &JsonValue,
    expected: &str,
    max_rank: Option<usize>,
) -> AssertionResult {
    let rows = data_rows(response);
    if let Some(index) = rows
        .iter()
        .position(|row| json_contains_string(row, expected))
    {
        let rank = index + 1;
        if max_rank.is_none_or(|max| rank <= max) {
            AssertionResult::pass(
                name,
                format!("expected value ranked {rank}"),
                json!({"rank": rank, "expected": expected}),
            )
        } else {
            AssertionResult::fail(
                name,
                format!(
                    "expected value ranked {rank}, above max rank {}",
                    max_rank.unwrap_or(0)
                ),
                top_evidence(rows, "unique_id"),
            )
        }
    } else {
        AssertionResult::fail(
            name,
            "expected value was not returned",
            json!({"expected": expected, "top": top_evidence(rows, "unique_id")}),
        )
    }
}

fn recipe_rank_assertion(
    response: &JsonValue,
    expected_recipe_id: &str,
    max_rank: Option<usize>,
) -> AssertionResult {
    let rows = data_rows(response);
    if let Some(index) = rows.iter().position(|row| {
        string_field_equals(row, "recipe_id", expected_recipe_id)
            || string_field_equals(row, "id", expected_recipe_id)
    }) {
        let rank = index + 1;
        if max_rank.is_none_or(|max| rank <= max) {
            AssertionResult::pass(
                "recipe_rank",
                format!("expected recipe ranked {rank}"),
                json!({"rank": rank, "expected": expected_recipe_id}),
            )
        } else {
            AssertionResult::fail(
                "recipe_rank",
                format!(
                    "expected recipe ranked {rank}, above max rank {}",
                    max_rank.unwrap_or(0)
                ),
                recipe_top_evidence(rows),
            )
        }
    } else {
        AssertionResult::fail(
            "recipe_rank",
            "expected recipe was not returned",
            json!({"expected": expected_recipe_id, "top": recipe_top_evidence(rows)}),
        )
    }
}

fn search_columns_assertion(
    response: &JsonValue,
    expected_column: Option<&str>,
    expected_parent_unique_id: Option<&str>,
    max_rank: Option<usize>,
) -> AssertionResult {
    let rows = data_rows(response);
    let position = rows.iter().position(|row| {
        expected_column.is_none_or(|column| json_contains_string(row, column))
            && expected_parent_unique_id
                .is_none_or(|parent| string_field_equals(row, "parent_unique_id", parent))
    });
    if let Some(index) = position {
        let rank = index + 1;
        if max_rank.is_none_or(|max| rank <= max) {
            AssertionResult::pass(
                "search_columns_rank",
                format!("expected column result ranked {rank}"),
                json!({"rank": rank}),
            )
        } else {
            AssertionResult::fail(
                "search_columns_rank",
                format!(
                    "expected column result ranked {rank}, above max rank {}",
                    max_rank.unwrap_or(0)
                ),
                top_evidence(rows, "parent_unique_id"),
            )
        }
    } else {
        AssertionResult::fail(
            "search_columns_rank",
            "expected column result was not returned",
            top_evidence(rows, "parent_unique_id"),
        )
    }
}

fn fields_assertion(name: &str, response: &JsonValue, fields: &[String]) -> AssertionResult {
    let missing: Vec<&str> = fields
        .iter()
        .map(String::as_str)
        .filter(|field| !json_has_field_path(response, field))
        .collect();
    if missing.is_empty() {
        AssertionResult::pass(name, "required fields were present", JsonValue::Null)
    } else {
        AssertionResult::fail(
            name,
            "required fields were missing",
            json!({"missing": missing}),
        )
    }
}

fn context_field_equals_assertion(
    response: &JsonValue,
    field: &str,
    expected: &JsonValue,
) -> AssertionResult {
    match json_value_at_path(response, field) {
        Some(actual) if actual == expected => AssertionResult::pass(
            "context_field_equals",
            "context field matched expected value",
            json!({"field": field, "expected": expected}),
        ),
        Some(actual) => AssertionResult::fail(
            "context_field_equals",
            "context field did not match expected value",
            json!({"field": field, "expected": expected, "actual": actual}),
        ),
        None => AssertionResult::fail(
            "context_field_equals",
            "context field was missing",
            json!({"field": field, "expected": expected}),
        ),
    }
}

fn context_contains_assertion(
    response: &JsonValue,
    field: Option<&str>,
    expected: &str,
) -> AssertionResult {
    let target = field.and_then(|field| json_value_at_path(response, field));
    let contains = if let Some(value) = target {
        json_contains_string(value, expected)
    } else if field.is_some() {
        false
    } else {
        json_contains_string(response, expected)
    };
    if contains {
        AssertionResult::pass(
            "context_contains",
            "expected value appeared in context",
            json!({"field": field, "expected": expected}),
        )
    } else {
        AssertionResult::fail(
            "context_contains",
            "expected value did not appear in context",
            json!({"field": field, "expected": expected}),
        )
    }
}

fn metadata_score_min_assertion(response: &JsonValue, threshold: f64) -> AssertionResult {
    let score = find_score(response);
    match score {
        Some(score) if score >= threshold => AssertionResult::pass(
            "metadata_score_min",
            format!("metadata score {score:.3} met threshold {threshold:.3}"),
            json!({"score": score, "threshold": threshold}),
        ),
        Some(score) => AssertionResult::fail(
            "metadata_score_min",
            format!("metadata score {score:.3} was below threshold {threshold:.3}"),
            json!({"score": score, "threshold": threshold}),
        ),
        None => AssertionResult::fail(
            "metadata_score_min",
            "metadata score response did not contain a numeric score",
            JsonValue::Null,
        ),
    }
}

fn metadata_score_max_assertion(response: &JsonValue, threshold: f64) -> AssertionResult {
    let score = find_score(response);
    match score {
        Some(score) if score <= threshold => AssertionResult::pass(
            "metadata_score_max",
            format!("metadata score {score:.3} did not exceed threshold {threshold:.3}"),
            json!({"score": score, "threshold": threshold}),
        ),
        Some(score) => AssertionResult::fail(
            "metadata_score_max",
            format!("metadata score {score:.3} exceeded threshold {threshold:.3}"),
            json!({"score": score, "threshold": threshold}),
        ),
        None => AssertionResult::fail(
            "metadata_score_max",
            "metadata score response did not contain a numeric score",
            JsonValue::Null,
        ),
    }
}

fn recipe_queries_assertion(response: &JsonValue, min_queries: usize) -> AssertionResult {
    let query_count = count_recipe_queries(response);
    if query_count >= min_queries {
        AssertionResult::pass(
            "recipe_has_queries",
            format!("recipe contained {query_count} queries"),
            json!({"query_count": query_count, "min_queries": min_queries}),
        )
    } else {
        AssertionResult::fail(
            "recipe_has_queries",
            format!("recipe contained {query_count} queries, below minimum {min_queries}"),
            json!({"query_count": query_count, "min_queries": min_queries}),
        )
    }
}

fn contains_string_assertion(name: &str, response: &JsonValue, expected: &str) -> AssertionResult {
    if json_contains_string(response, expected) {
        AssertionResult::pass(
            name,
            "expected value appeared in response",
            json!({"expected": expected}),
        )
    } else {
        AssertionResult::fail(
            name,
            "expected value did not appear in response",
            json!({"expected": expected}),
        )
    }
}

fn tool_success_assertion(tool: &str, response: &JsonValue) -> AssertionResult {
    if response.get("success").and_then(JsonValue::as_bool) == Some(false) {
        return AssertionResult::fail(
            format!("tool_success:{tool}"),
            "tool returned an explicit success=false response",
            tool_failure_evidence(response),
        );
    }
    AssertionResult::pass(
        format!("tool_success:{tool}"),
        "tool returned success",
        json!({"count": response.get("count").cloned().unwrap_or(JsonValue::Null)}),
    )
}

fn tool_response_budget_assertion(
    tool: &str,
    response: &JsonValue,
    max_response_bytes: usize,
    must_contain_paths: &[String],
    must_not_contain_paths: &[String],
) -> AssertionResult {
    let response_bytes = serde_json::to_string(response).map_or(usize::MAX, |value| value.len());
    let missing_paths: Vec<&str> = must_contain_paths
        .iter()
        .map(String::as_str)
        .filter(|path| !json_has_field_path(response, path))
        .collect();
    let present_forbidden_paths: Vec<&str> = must_not_contain_paths
        .iter()
        .map(String::as_str)
        .filter(|path| json_has_field_path(response, path))
        .collect();
    if response_bytes <= max_response_bytes
        && missing_paths.is_empty()
        && present_forbidden_paths.is_empty()
    {
        AssertionResult::pass(
            format!("tool_response_budget:{tool}"),
            "tool response stayed within budget and shape constraints",
            json!({"response_bytes": response_bytes, "max_response_bytes": max_response_bytes}),
        )
    } else {
        AssertionResult::fail(
            format!("tool_response_budget:{tool}"),
            "tool response exceeded budget or shape constraints",
            json!({
                "response_bytes": response_bytes,
                "max_response_bytes": max_response_bytes,
                "missing_paths": missing_paths,
                "present_forbidden_paths": present_forbidden_paths,
            }),
        )
    }
}

fn tool_failure_evidence(response: &JsonValue) -> JsonValue {
    let mut evidence = serde_json::Map::new();
    for key in ["success", "error_code", "code", "message", "count"] {
        if let Some(value) = response.get(key).and_then(safe_evidence_scalar) {
            evidence.insert(key.to_string(), value);
        }
    }
    if let Some(error) = response.get("error").and_then(JsonValue::as_object) {
        for key in ["error_code", "code", "message"] {
            if let Some(value) = error.get(key).and_then(safe_evidence_scalar) {
                evidence.insert(format!("error.{key}"), value);
            }
        }
    }
    if evidence.is_empty()
        && let Some(map) = response.as_object()
    {
        evidence.insert(
            "keys".to_string(),
            JsonValue::Array(
                map.keys()
                    .take(20)
                    .cloned()
                    .map(JsonValue::String)
                    .collect(),
            ),
        );
    }
    JsonValue::Object(evidence)
}

fn safe_evidence_scalar(value: &JsonValue) -> Option<JsonValue> {
    match value {
        JsonValue::Null | JsonValue::Bool(_) | JsonValue::Number(_) => Some(value.clone()),
        JsonValue::String(value) => Some(JsonValue::String(truncate(value, 1000))),
        JsonValue::Array(_) | JsonValue::Object(_) => None,
    }
}

fn data_rows(response: &JsonValue) -> &[JsonValue] {
    response
        .get("data")
        .and_then(JsonValue::as_array)
        .map_or(&[], Vec::as_slice)
}

fn string_field_equals(row: &JsonValue, field: &str, expected: &str) -> bool {
    row.get(field)
        .and_then(JsonValue::as_str)
        .is_some_and(|value| value.eq_ignore_ascii_case(expected))
}

fn json_contains_string(value: &JsonValue, expected: &str) -> bool {
    let expected = expected.to_lowercase();
    json_contains_string_lower(value, &expected)
}

fn json_contains_string_lower(value: &JsonValue, expected: &str) -> bool {
    match value {
        JsonValue::String(value) => value.to_lowercase().contains(expected),
        JsonValue::Array(items) => items
            .iter()
            .any(|item| json_contains_string_lower(item, expected)),
        JsonValue::Object(map) => map
            .values()
            .any(|child| json_contains_string_lower(child, expected)),
        JsonValue::Null | JsonValue::Bool(_) | JsonValue::Number(_) => false,
    }
}

fn json_has_field_path(value: &JsonValue, field_path: &str) -> bool {
    json_value_at_path(value, field_path).is_some_and(|value| !value.is_null())
}

fn json_value_at_path<'a>(value: &'a JsonValue, field_path: &str) -> Option<&'a JsonValue> {
    let mut current = value;
    for part in field_path.split('.') {
        current = match current {
            JsonValue::Array(items) => items.get(part.parse::<usize>().ok()?)?,
            JsonValue::Object(_) => current.get(part)?,
            _ => return None,
        };
    }
    Some(current)
}

fn find_score(value: &JsonValue) -> Option<f64> {
    for key in ["overall_score", "score", "metadata_score"] {
        if let Some(score) = find_number_by_key(value, key) {
            return Some(score);
        }
    }
    None
}

fn find_number_by_key(value: &JsonValue, key: &str) -> Option<f64> {
    match value {
        JsonValue::Object(map) => {
            if let Some(score) = map.get(key).and_then(JsonValue::as_f64) {
                return Some(score);
            }
            map.values()
                .find_map(|child| find_number_by_key(child, key))
        }
        JsonValue::Array(items) => items.iter().find_map(|item| find_number_by_key(item, key)),
        JsonValue::Null | JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::String(_) => None,
    }
}

fn count_recipe_queries(value: &JsonValue) -> usize {
    match value {
        JsonValue::Object(map) => {
            for key in ["queries", "query_names"] {
                if let Some(count) = map.get(key).and_then(JsonValue::as_array).map(Vec::len) {
                    return count;
                }
            }
            map.values().map(count_recipe_queries).max().unwrap_or(0)
        }
        JsonValue::Array(items) => items.iter().map(count_recipe_queries).max().unwrap_or(0),
        JsonValue::Null | JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::String(_) => 0,
    }
}

fn top_evidence(rows: &[JsonValue], field: &str) -> JsonValue {
    JsonValue::Array(
        rows.iter()
            .take(10)
            .map(|row| {
                json!({
                    field: row.get(field).cloned().unwrap_or(JsonValue::Null),
                    "name": row.get("name").cloned().unwrap_or(JsonValue::Null),
                    "unique_id": row.get("unique_id").cloned().unwrap_or(JsonValue::Null),
                })
            })
            .collect(),
    )
}

fn recipe_top_evidence(rows: &[JsonValue]) -> JsonValue {
    JsonValue::Array(
        rows.iter()
            .take(10)
            .map(|row| {
                json!({
                    "id": row.get("id").cloned().unwrap_or(JsonValue::Null),
                    "recipe_id": row.get("recipe_id").cloned().unwrap_or(JsonValue::Null),
                    "topic": row.get("topic").cloned().unwrap_or(JsonValue::Null),
                })
            })
            .collect(),
    )
}

fn effective_limit(max_rank: Option<usize>, default_top_k: usize) -> usize {
    max_rank.unwrap_or(default_top_k).max(1)
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

fn build_report(
    suite: &EvalSuite,
    mode: &'static str,
    output_dir: String,
    fail_under: f64,
    cases: Vec<EvalCaseReport>,
) -> EvalReport {
    let pass_count = cases.iter().map(|case| case.pass_count).sum();
    let fail_count = cases.iter().map(|case| case.fail_count).sum();
    let error_count = cases.iter().map(|case| case.error_count).sum();
    let assertion_count = pass_count + fail_count + error_count;
    let pass_rate = if assertion_count == 0 {
        0.0
    } else {
        ratio(pass_count, assertion_count)
    };
    let gate_status = if pass_rate >= fail_under {
        "pass"
    } else {
        "fail"
    };
    let suite_name = suite.name.clone().unwrap_or_else(|| "unnamed".to_string());
    let summary = EvalReportCardSummary {
        suite_name: suite_name.clone(),
        version: suite.version,
        mode,
        output_dir: output_dir.clone(),
        assertion_count,
        pass_count,
        fail_count,
        error_count,
        pass_rate,
        fail_under,
        gate_status,
        run_case_count: cases.len(),
    };
    let eval_card = build_eval_card(
        suite,
        &summary,
        &EvalCardRunContext::default(),
        eval_card_telemetry_evidence(&summary.suite_name, false),
    );
    EvalReport {
        suite_name,
        version: suite.version,
        mode,
        output_dir,
        eval_card,
        assertion_count,
        pass_count,
        fail_count,
        error_count,
        pass_rate,
        fail_under,
        gate_status,
        cases,
    }
}

#[derive(Debug)]
struct EvalReportCardSummary {
    suite_name: String,
    version: u32,
    mode: &'static str,
    output_dir: String,
    assertion_count: usize,
    pass_count: usize,
    fail_count: usize,
    error_count: usize,
    pass_rate: f64,
    fail_under: f64,
    gate_status: &'static str,
    run_case_count: usize,
}

fn refresh_eval_card(report: &mut EvalReport, suite: &EvalSuite, context: &EvalCardRunContext) {
    let summary = EvalReportCardSummary::from_report(report);
    report.eval_card = build_eval_card(
        suite,
        &summary,
        context,
        eval_card_telemetry_evidence(&report.suite_name, context.telemetry_requested),
    );
}

fn build_eval_card(
    suite: &EvalSuite,
    summary: &EvalReportCardSummary,
    context: &EvalCardRunContext,
    evidence: EvalCardTelemetryEvidence,
) -> EvalCard {
    EvalCard {
        schema_version: "eval_card.v1",
        suite_name: summary.suite_name.clone(),
        version: summary.version,
        suite_path: context.suite_path.clone(),
        purpose: suite
            .purpose
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map_or_else(|| default_eval_card_purpose(summary.mode), str::to_string),
        persona: suite
            .defaults
            .persona
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        manifest_scope: EvalCardManifestScope {
            declared: suite
                .manifest_scope
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or("not declared")
                .to_string(),
            manifest_hash: context.manifest_hash.clone(),
            manifest_source: context.manifest_source.clone(),
        },
        mode: summary.mode,
        bridge_case_count: suite.cases.len(),
        agent_case_count: suite.agent_cases.len(),
        run_case_count: summary.run_case_count,
        output_dir: summary.output_dir.clone(),
        assertion_count: summary.assertion_count,
        pass_count: summary.pass_count,
        fail_count: summary.fail_count,
        error_count: summary.error_count,
        pass_rate: summary.pass_rate,
        fail_under: summary.fail_under,
        run_status: summary.gate_status,
        gate: evidence.gate,
        telemetry: evidence.telemetry,
        date_anchor: suite.date_anchor.normalized(),
        provider: context.provider.clone(),
        known_gaps: suite
            .known_gaps
            .iter()
            .map(|gap| gap.trim())
            .filter(|gap| !gap.is_empty())
            .map(str::to_string)
            .collect(),
    }
}

fn eval_card_telemetry_evidence(
    suite_name: &str,
    telemetry_requested: bool,
) -> EvalCardTelemetryEvidence {
    match read_telemetry_rows_for_suite(suite_name) {
        Ok(rows) if rows.is_empty() => missing_eval_card_telemetry(telemetry_requested),
        Ok(rows) => {
            let latest = latest_telemetry_rows(&rows);
            let first = latest.first().copied();
            let timestamp = first
                .and_then(|row| row.get("timestamp"))
                .and_then(JsonValue::as_str)
                .map(str::to_string);
            let run_id = first
                .and_then(|row| row.get("run_id"))
                .and_then(JsonValue::as_str)
                .map(str::to_string);
            let telemetry = EvalCardTelemetry {
                status: "latest".to_string(),
                timestamp,
                run_id,
                row_count: latest.len(),
                message: format!(
                    "latest telemetry includes {} assertion row(s) for suite '{suite_name}'",
                    latest.len()
                ),
            };
            let gate = build_eval_gate_report(suite_name, &rows).map_or_else(
                |error| EvalCardGate {
                    status: "unavailable".to_string(),
                    source: "telemetry",
                    configured: false,
                    threshold: None,
                    pass_rate: None,
                    total_evals: Some(latest.len()),
                    failed_evals: None,
                    message: format!("eval gate could not be derived from telemetry: {error}"),
                },
                |report| EvalCardGate::from_report(&report),
            );
            EvalCardTelemetryEvidence { telemetry, gate }
        }
        Err(error) => EvalCardTelemetryEvidence {
            telemetry: EvalCardTelemetry {
                status: "unavailable".to_string(),
                timestamp: None,
                run_id: None,
                row_count: 0,
                message: format!("eval telemetry could not be read: {error}"),
            },
            gate: EvalCardGate {
                status: "unavailable".to_string(),
                source: "telemetry",
                configured: false,
                threshold: None,
                pass_rate: None,
                total_evals: None,
                failed_evals: None,
                message: format!(
                    "eval gate could not be derived because telemetry is unreadable: {error}"
                ),
            },
        },
    }
}

fn missing_eval_card_telemetry(telemetry_requested: bool) -> EvalCardTelemetryEvidence {
    let message = if telemetry_requested {
        "telemetry was requested, but no telemetry rows were found for this suite"
    } else {
        "no telemetry found for this suite; run with --telemetry to populate latest gate evidence"
    };
    EvalCardTelemetryEvidence {
        telemetry: EvalCardTelemetry {
            status: "missing".to_string(),
            timestamp: None,
            run_id: None,
            row_count: 0,
            message: message.to_string(),
        },
        gate: EvalCardGate {
            status: "missing_telemetry".to_string(),
            source: "telemetry",
            configured: false,
            threshold: None,
            pass_rate: None,
            total_evals: Some(0),
            failed_evals: None,
            message: "gate status unavailable until telemetry exists for the suite".to_string(),
        },
    }
}

fn default_eval_card_purpose(mode: &str) -> String {
    match mode {
        "agent" => "Summarizes provider-backed agent tool-use evidence for this eval suite.",
        _ => "Summarizes deterministic Nova bridge eval evidence for this eval suite.",
    }
    .to_string()
}

impl EvalReportCardSummary {
    fn from_report(report: &EvalReport) -> Self {
        Self {
            suite_name: report.suite_name.clone(),
            version: report.version,
            mode: report.mode,
            output_dir: report.output_dir.clone(),
            assertion_count: report.assertion_count,
            pass_count: report.pass_count,
            fail_count: report.fail_count,
            error_count: report.error_count,
            pass_rate: report.pass_rate,
            fail_under: report.fail_under,
            gate_status: report.gate_status,
            run_case_count: report.cases.len(),
        }
    }
}

impl EvalCardGate {
    fn from_report(report: &EvalGateReport) -> Self {
        let status = if report.gate_configured {
            if report.allowed { "pass" } else { "fail" }
        } else {
            "not_configured"
        };
        Self {
            status: status.to_string(),
            source: "telemetry",
            configured: report.gate_configured,
            threshold: report.threshold,
            pass_rate: Some(report.pass_rate),
            total_evals: Some(report.total_evals),
            failed_evals: Some(report.failed_evals),
            message: report.message.clone(),
        }
    }
}

fn agent_manifest_source(args: &EvalAgentRunArgs) -> Option<String> {
    args.manifest_path
        .as_deref()
        .or(args.manifest_uri.as_deref())
        .map(crate::utils::sanitize_uri)
}

fn write_report_artifacts(
    output_dir: &Path,
    report: &EvalReport,
    suite_path: &str,
) -> DispatchResult {
    fs::create_dir_all(output_dir).map_err(|error| server_error(error.to_string()))?;
    let report_json =
        serde_json::to_string_pretty(report).map_err(|error| server_error(error.to_string()))?;
    fs::write(output_dir.join("results.json"), report_json)
        .map_err(|error| server_error(error.to_string()))?;
    fs::write(output_dir.join("results.tsv"), render_tsv(report))
        .map_err(|error| server_error(error.to_string()))?;
    fs::write(
        output_dir.join("card.md"),
        render_eval_card_markdown(&report.eval_card),
    )
    .map_err(|error| server_error(error.to_string()))?;
    fs::write(output_dir.join("report.md"), render_markdown(report))
        .map_err(|error| server_error(error.to_string()))?;
    if let Err(error) = fs::copy(suite_path, output_dir.join("suite.yml")) {
        tracing::warn!(error = %error, suite_path, "failed to copy eval suite");
    }
    Ok(())
}

fn read_telemetry_rows_for_suite(suite_name: &str) -> crate::error::Result<Vec<JsonValue>> {
    let path = telemetry_path_for_suite(suite_name);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let file = fs::File::open(&path).map_err(|error| server_error(error.to_string()))?;
    let reader = BufReader::new(file);
    let mut rows = Vec::new();
    for (index, line) in reader.lines().enumerate() {
        let line = line.map_err(|error| server_error(error.to_string()))?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let row = serde_json::from_str::<JsonValue>(trimmed).map_err(|error| {
            DbtNovaError::InvalidParams(format!(
                "failed to parse telemetry line {} in '{}': {error}",
                index + 1,
                path.display()
            ))
        })?;
        if row
            .get("suite_name")
            .and_then(JsonValue::as_str)
            .is_some_and(|value| value == suite_name)
        {
            rows.push(row);
        }
    }
    Ok(rows)
}

fn build_eval_gate_report(
    suite_name: &str,
    rows: &[JsonValue],
) -> crate::error::Result<EvalGateReport> {
    let latest_rows = latest_telemetry_rows(rows);
    if latest_rows.is_empty() {
        return Ok(EvalGateReport {
            suite_name: suite_name.to_string(),
            allowed: false,
            blocked: true,
            gate_configured: false,
            threshold: None,
            pass_rate: 0.0,
            total_evals: 0,
            failed_evals: 0,
            failed_eval_ids: Vec::new(),
            failed_case_ids: Vec::new(),
            telemetry_timestamp: None,
            output_dir: None,
            suite_path: None,
            message: format!(
                "no eval telemetry found for suite '{suite_name}'; run the suite with --telemetry first"
            ),
        });
    }

    let total_evals = latest_rows.len();
    let pass_count = latest_rows
        .iter()
        .filter(|row| row.get("status").and_then(JsonValue::as_str) == Some("pass"))
        .count();
    let failed_rows: Vec<&JsonValue> = latest_rows
        .iter()
        .copied()
        .filter(|row| row.get("status").and_then(JsonValue::as_str) != Some("pass"))
        .collect();
    let failed_eval_ids: Vec<String> = failed_rows
        .iter()
        .map(|row| telemetry_eval_id(row))
        .collect();
    let failed_case_ids: Vec<String> = failed_rows
        .iter()
        .filter_map(|row| row.get("case_id").and_then(JsonValue::as_str))
        .map(str::to_string)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let pass_rate = ratio(pass_count, total_evals);
    let first = latest_rows[0];
    let suite_path = first
        .get("suite_path")
        .and_then(JsonValue::as_str)
        .map(str::to_string);
    let gate = gate_config_status_from_suite_path(suite_path.as_deref())?;
    let telemetry_timestamp = first
        .get("timestamp")
        .and_then(JsonValue::as_str)
        .map(str::to_string);
    let output_dir = first
        .get("output_dir")
        .and_then(JsonValue::as_str)
        .map(str::to_string);

    let (gate_configured, threshold, current_suite_hash, unavailable_message) = match gate {
        GateConfigStatus::Configured { gate, suite_hash } => {
            (true, Some(gate.threshold), Some(suite_hash), None)
        }
        GateConfigStatus::Unconfigured => (false, None, None, None),
        GateConfigStatus::Unavailable(message) => (false, None, None, Some(message)),
    };
    let incomplete_message = threshold
        .is_some()
        .then(|| {
            latest_run_incomplete_message(
                &latest_rows,
                current_suite_hash.as_deref().unwrap_or_default(),
            )
        })
        .flatten();
    if let Some(message) = unavailable_message.or(incomplete_message) {
        return Ok(EvalGateReport {
            suite_name: suite_name.to_string(),
            allowed: false,
            blocked: true,
            gate_configured,
            threshold,
            pass_rate,
            total_evals,
            failed_evals: failed_eval_ids.len(),
            failed_eval_ids,
            failed_case_ids,
            telemetry_timestamp,
            output_dir,
            suite_path,
            message,
        });
    }

    let (allowed, message) = if let Some(threshold) = threshold {
        let allowed = pass_rate >= threshold;
        let message = if allowed {
            format!("latest eval telemetry passed gate threshold {threshold:.3}")
        } else {
            format!(
                "latest eval telemetry below gate threshold {threshold:.3}; inspect failed_eval_ids before relying on this suite"
            )
        };
        (allowed, message)
    } else {
        (
            true,
            "no gate threshold configured; advisory gate allowed by default".to_string(),
        )
    };

    Ok(EvalGateReport {
        suite_name: suite_name.to_string(),
        allowed,
        blocked: !allowed,
        gate_configured,
        threshold,
        pass_rate,
        total_evals,
        failed_evals: failed_eval_ids.len(),
        failed_eval_ids,
        failed_case_ids,
        telemetry_timestamp,
        output_dir,
        suite_path,
        message,
    })
}

fn latest_telemetry_rows(rows: &[JsonValue]) -> Vec<&JsonValue> {
    let Some(latest) = rows.iter().max_by_key(|row| {
        row.get("timestamp_ms")
            .and_then(JsonValue::as_u64)
            .unwrap_or(0)
    }) else {
        return Vec::new();
    };
    if let Some(run_id) = latest
        .get("run_id")
        .and_then(JsonValue::as_str)
        .filter(|value| !value.trim().is_empty())
    {
        return rows
            .iter()
            .filter(|row| row.get("run_id").and_then(JsonValue::as_str) == Some(run_id))
            .collect();
    }
    let latest_timestamp = latest
        .get("timestamp_ms")
        .and_then(JsonValue::as_u64)
        .unwrap_or(0);
    if let Some(output_dir) = latest
        .get("output_dir")
        .and_then(JsonValue::as_str)
        .filter(|value| !value.trim().is_empty())
    {
        rows.iter()
            .filter(|row| {
                row.get("timestamp_ms").and_then(JsonValue::as_u64) == Some(latest_timestamp)
                    && row.get("output_dir").and_then(JsonValue::as_str) == Some(output_dir)
            })
            .collect()
    } else {
        rows.iter()
            .filter(|row| {
                row.get("timestamp_ms").and_then(JsonValue::as_u64) == Some(latest_timestamp)
            })
            .collect()
    }
}

fn latest_run_incomplete_message(rows: &[&JsonValue], current_suite_hash: &str) -> Option<String> {
    let Some(recorded_suite_hash) = rows
        .first()
        .and_then(|row| row.get("suite_hash"))
        .and_then(JsonValue::as_str)
        .filter(|value| !value.trim().is_empty())
    else {
        return Some(
            "latest eval telemetry does not include suite_hash; rerun the full suite with --telemetry before checking the gate"
                .to_string(),
        );
    };
    if recorded_suite_hash != current_suite_hash {
        return Some(
            "latest eval telemetry was produced from a different suite file version; rerun the full suite with --telemetry before checking the gate"
                .to_string(),
        );
    }
    let Some(run_case_count) = rows
        .first()
        .and_then(|row| row.get("run_case_count"))
        .and_then(JsonValue::as_u64)
    else {
        return Some(
            "latest eval telemetry does not include run_case_count; rerun the full suite with --telemetry before checking the gate"
                .to_string(),
        );
    };
    let Some(suite_case_count) = rows
        .first()
        .and_then(|row| row.get("suite_case_count"))
        .and_then(JsonValue::as_u64)
    else {
        return Some(
            "latest eval telemetry does not include suite_case_count; rerun the full suite with --telemetry before checking the gate"
                .to_string(),
        );
    };
    if run_case_count != suite_case_count {
        return Some(format!(
            "latest eval telemetry covers {run_case_count} of {suite_case_count} suite cases; rerun the full suite with --telemetry before checking the gate"
        ));
    }
    let Some(expected) = rows
        .first()
        .and_then(|row| row.get("run_assertion_count"))
        .and_then(JsonValue::as_u64)
    else {
        return Some(
            "latest eval telemetry does not include run_assertion_count; rerun the suite with --telemetry before checking the gate"
                .to_string(),
        );
    };
    let observed = u64::try_from(rows.len()).unwrap_or(u64::MAX);
    if expected == observed {
        return None;
    }
    Some(format!(
        "latest eval telemetry is incomplete: found {observed} of {expected} assertion rows; rerun the suite with --telemetry or increase --telemetry-retention"
    ))
}

fn gate_config_status_from_suite_path(
    path: Option<&str>,
) -> crate::error::Result<GateConfigStatus> {
    let Some(path) = path.filter(|value| !value.trim().is_empty()) else {
        return Ok(GateConfigStatus::Unavailable(
            "latest telemetry did not include suite_path; rerun the suite with --telemetry"
                .to_string(),
        ));
    };
    if !Path::new(path).exists() {
        return Ok(GateConfigStatus::Unavailable(format!(
            "suite config '{path}' could not be read; rerun the suite with --telemetry from the current checkout"
        )));
    }
    let (suite, suite_hash) = load_suite_with_hash(path)?;
    Ok(match suite.gate {
        Some(gate) => GateConfigStatus::Configured { gate, suite_hash },
        None => GateConfigStatus::Unconfigured,
    })
}

#[cfg(test)]
fn suite_file_hash(path: &str) -> crate::error::Result<String> {
    let raw = fs::read(path).map_err(|error| {
        DbtNovaError::InvalidParams(format!(
            "failed to read eval suite '{path}' for hash: {error}"
        ))
    })?;
    Ok(blake3::hash(&raw).to_hex().to_string())
}

fn telemetry_eval_id(row: &JsonValue) -> String {
    let case_id = row
        .get("case_id")
        .and_then(JsonValue::as_str)
        .unwrap_or("unknown_case");
    let assertion_name = row
        .get("assertion_name")
        .and_then(JsonValue::as_str)
        .unwrap_or("unknown_assertion");
    format!("{case_id}::{assertion_name}")
}

fn print_gate_report(report: &EvalGateReport) {
    let status = if report.allowed { "allowed" } else { "blocked" };
    println!("Nova eval gate {}: {status}", report.suite_name);
    println!("  gate_configured: {}", report.gate_configured);
    if let Some(threshold) = report.threshold {
        println!("  threshold: {threshold:.3}");
    }
    println!("  pass_rate: {:.3}", report.pass_rate);
    println!("  total_evals: {}", report.total_evals);
    println!("  failed_evals: {}", report.failed_evals);
    if let Some(timestamp) = report.telemetry_timestamp.as_ref() {
        println!("  telemetry_timestamp: {timestamp}");
    }
    println!("  message: {}", report.message);
    if !report.failed_eval_ids.is_empty() {
        println!("  failed_eval_ids: {}", report.failed_eval_ids.join(", "));
    }
}

fn write_eval_telemetry(
    report: &EvalReport,
    context: EvalTelemetryRunContext<'_>,
) -> DispatchResult {
    let telemetry_path = telemetry_path_for_suite(&report.suite_name);
    if let Some(parent) = telemetry_path.parent() {
        fs::create_dir_all(parent).map_err(|error| server_error(error.to_string()))?;
    }
    let timestamp_ms = timestamp_millis();
    let timestamp = format_utc_timestamp_millis(timestamp_ms);
    let run_id = format!(
        "{}-{}-{timestamp_ms}",
        report.mode,
        safe_path_segment(&report.suite_name)
    );
    let git_sha = current_git_sha();
    let manifest_hash = context
        .manifest_hash
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&telemetry_path)
        .map_err(|error| server_error(error.to_string()))?;
    for case in &report.cases {
        for assertion in &case.assertions {
            let mut row = serde_json::Map::new();
            row.insert("timestamp".to_string(), json!(&timestamp));
            row.insert("timestamp_ms".to_string(), json!(timestamp_ms));
            row.insert("run_id".to_string(), json!(&run_id));
            row.insert("run_case_count".to_string(), json!(report.cases.len()));
            row.insert(
                "suite_case_count".to_string(),
                json!(context.suite_case_count),
            );
            row.insert(
                "run_assertion_count".to_string(),
                json!(report.assertion_count),
            );
            row.insert("suite_name".to_string(), json!(&report.suite_name));
            row.insert("suite_path".to_string(), json!(context.suite_path));
            row.insert("suite_hash".to_string(), json!(context.suite_hash));
            row.insert("mode".to_string(), json!(report.mode));
            row.insert("case_id".to_string(), json!(&case.id));
            row.insert("assertion_name".to_string(), json!(&assertion.name));
            row.insert(
                "assertion_type".to_string(),
                json!(assertion_type(&assertion.name)),
            );
            row.insert("status".to_string(), json!(assertion.status));
            row.insert(
                "grade_mode".to_string(),
                json!(telemetry_grade_mode(report.mode)),
            );
            row.insert("duration_ms".to_string(), json!(context.duration_ms));
            row.insert("output_dir".to_string(), json!(&report.output_dir));
            if let Some(manifest_hash) = manifest_hash.as_ref() {
                row.insert("manifest_hash".to_string(), json!(manifest_hash));
            }
            if let Some(git_sha) = git_sha.as_ref() {
                row.insert("git_sha".to_string(), json!(git_sha));
            }
            if let Some(agent) = context.agent {
                row.insert("provider".to_string(), json!(agent.provider));
                row.insert(
                    "provider_command_preset".to_string(),
                    json!(agent.provider_command_preset),
                );
                if let Some(telemetry) = case.telemetry.as_ref() {
                    row.insert(
                        "tool_call_count".to_string(),
                        json!(telemetry.tool_call_count),
                    );
                    row.insert(
                        "distinct_tool_count".to_string(),
                        json!(telemetry.distinct_tool_count),
                    );
                    if let Some(value) = telemetry.total_response_bytes {
                        row.insert("total_response_bytes".to_string(), json!(value));
                    }
                    if let Some(value) = telemetry.input_tokens {
                        row.insert("input_tokens".to_string(), json!(value));
                    }
                    if let Some(value) = telemetry.output_tokens {
                        row.insert("output_tokens".to_string(), json!(value));
                    }
                    if let Some(value) = telemetry.total_tokens {
                        row.insert("total_tokens".to_string(), json!(value));
                    }
                }
            }
            if let Some(date_anchor) = case.date_anchor.as_ref() {
                insert_date_anchor_telemetry(&mut row, date_anchor);
            }
            let line = serde_json::to_string(&JsonValue::Object(row))
                .map_err(|error| server_error(error.to_string()))?;
            file.write_all(line.as_bytes())
                .map_err(|error| server_error(error.to_string()))?;
            file.write_all(b"\n")
                .map_err(|error| server_error(error.to_string()))?;
        }
    }
    drop(file);

    if let Some(max_rows) = context.retention {
        apply_telemetry_retention(&telemetry_path, max_rows)?;
    }
    Ok(())
}

fn apply_telemetry_retention(path: &Path, max_rows: usize) -> DispatchResult {
    let raw = fs::read_to_string(path).map_err(|error| server_error(error.to_string()))?;
    let rows: Vec<&str> = raw.lines().filter(|line| !line.trim().is_empty()).collect();
    if rows.len() <= max_rows {
        return Ok(());
    }
    let keep_from = rows.len().saturating_sub(max_rows);
    let mut out = rows[keep_from..].join("\n");
    out.push('\n');
    fs::write(path, out).map_err(|error| server_error(error.to_string()))?;
    Ok(())
}

fn telemetry_path_for_suite(suite_name: &str) -> PathBuf {
    PathBuf::from(DEFAULT_TELEMETRY_DIR).join(format!(
        "{}-{:016x}.jsonl",
        safe_path_segment(suite_name),
        stable_telemetry_hash(suite_name)
    ))
}

fn stable_telemetry_hash(value: &str) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0001_0000_01b3);
    }
    hash
}

fn insert_date_anchor_telemetry(row: &mut serde_json::Map<String, JsonValue>, anchor: &DateAnchor) {
    if let Some(value) = anchor.snapshot_date.as_ref() {
        row.insert("snapshot_date".to_string(), json!(value));
    }
    if let Some(value) = anchor.date_range_start.as_ref() {
        row.insert("date_range_start".to_string(), json!(value));
    }
    if let Some(value) = anchor.date_range_end.as_ref() {
        row.insert("date_range_end".to_string(), json!(value));
    }
    if let Some(value) = anchor.date_field.as_ref() {
        row.insert("date_field".to_string(), json!(value));
    }
    row.insert("date_anchor".to_string(), json!(anchor));
}

fn telemetry_row_matches_since(row: &JsonValue, since_boundary: &str) -> bool {
    row.get("timestamp")
        .and_then(JsonValue::as_str)
        .is_some_and(|timestamp| timestamp >= since_boundary)
}

fn validate_since_date(value: &str) -> crate::error::Result<String> {
    let valid_shape = value.len() == 10
        && value.as_bytes()[4] == b'-'
        && value.as_bytes()[7] == b'-'
        && value
            .chars()
            .enumerate()
            .all(|(index, ch)| matches!(index, 4 | 7) || ch.is_ascii_digit());
    if !valid_shape {
        return Err(DbtNovaError::InvalidParams(
            "--since must use YYYY-MM-DD".to_string(),
        ));
    }
    let year = value[0..4]
        .parse::<i32>()
        .map_err(|_| DbtNovaError::InvalidParams("--since must use YYYY-MM-DD".to_string()))?;
    let month = value[5..7]
        .parse::<u32>()
        .map_err(|_| DbtNovaError::InvalidParams("--since must use YYYY-MM-DD".to_string()))?;
    let day = value[8..10]
        .parse::<u32>()
        .map_err(|_| DbtNovaError::InvalidParams("--since must use YYYY-MM-DD".to_string()))?;
    if !(1..=12).contains(&month) || day == 0 || day > days_in_month(year, month) {
        return Err(DbtNovaError::InvalidParams(
            "--since must use a valid YYYY-MM-DD date".to_string(),
        ));
    }
    Ok(format!("{value}T00:00:00.000Z"))
}

fn validate_telemetry_retention(value: Option<usize>) -> crate::error::Result<()> {
    if value == Some(0) {
        return Err(DbtNovaError::InvalidParams(
            "--telemetry-retention must be greater than zero".to_string(),
        ));
    }
    Ok(())
}

fn validate_telemetry_suite_name(
    suite: &EvalSuite,
    telemetry_enabled: bool,
) -> crate::error::Result<()> {
    if telemetry_enabled
        && suite
            .name
            .as_deref()
            .is_none_or(|name| name.trim().is_empty())
    {
        return Err(DbtNovaError::InvalidParams(
            "--telemetry requires the eval suite to define a non-empty name".to_string(),
        ));
    }
    Ok(())
}

fn telemetry_grade_mode(mode: &str) -> &'static str {
    match mode {
        "agent" => "provider_trace",
        _ => "deterministic",
    }
}

fn assertion_type(name: &str) -> &str {
    name.split_once(':').map_or(name, |(prefix, _)| prefix)
}

fn agent_provider_command_preset(args: &EvalAgentRunArgs) -> &str {
    if args.provider_command.is_some() || args.provider_args_json.is_some() {
        "custom"
    } else {
        args.provider.as_str()
    }
}

fn eval_case_telemetry_from_trace(trace: &[JsonValue]) -> EvalCaseTelemetry {
    let mut distinct_tools = BTreeSet::new();
    let mut response_bytes_seen = false;
    let mut total_response_bytes = 0_u64;
    for row in trace {
        if let Some(tool) = row.get("tool").and_then(JsonValue::as_str) {
            distinct_tools.insert(tool.to_string());
        }
        if let Some(bytes) = row.get("response_bytes").and_then(JsonValue::as_u64) {
            response_bytes_seen = true;
            total_response_bytes = total_response_bytes.saturating_add(bytes);
        }
    }
    EvalCaseTelemetry {
        tool_call_count: trace.len(),
        distinct_tool_count: distinct_tools.len(),
        total_response_bytes: response_bytes_seen.then_some(total_response_bytes),
        input_tokens: sum_first_available_u64(
            trace,
            &[
                &["input_tokens"],
                &["usage", "input_tokens"],
                &["usage", "prompt_tokens"],
            ],
        ),
        output_tokens: sum_first_available_u64(
            trace,
            &[
                &["output_tokens"],
                &["usage", "output_tokens"],
                &["usage", "completion_tokens"],
            ],
        ),
        total_tokens: sum_first_available_u64(
            trace,
            &[&["total_tokens"], &["usage", "total_tokens"]],
        ),
    }
}

fn sum_first_available_u64(trace: &[JsonValue], paths: &[&[&str]]) -> Option<u64> {
    let mut seen = false;
    let mut total = 0_u64;
    for row in trace {
        for path in paths {
            if let Some(value) = json_path_u64(row, path) {
                seen = true;
                total = total.saturating_add(value);
                break;
            }
        }
    }
    seen.then_some(total)
}

fn json_path_u64(value: &JsonValue, path: &[&str]) -> Option<u64> {
    let mut cursor = value;
    for part in path {
        cursor = cursor.get(*part)?;
    }
    cursor.as_u64()
}

fn current_git_sha() -> Option<String> {
    let output = StdCommand::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let sha = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!sha.is_empty()).then_some(sha)
}

fn timestamp_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        })
}

fn format_utc_timestamp_millis(timestamp_ms: u64) -> String {
    let secs = timestamp_ms / 1000;
    let millis = timestamp_ms % 1000;
    let days = i64::try_from(secs / 86_400).unwrap_or(i64::MAX);
    let seconds_of_day = secs % 86_400;
    let (year, month, day) = civil_from_days(days);
    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;
    let second = seconds_of_day % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z")
}

fn civil_from_days(days_since_epoch: i64) -> (i32, u32, u32) {
    let z = days_since_epoch.saturating_add(719_468);
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    if month <= 2 {
        year += 1;
    }
    (
        i32::try_from(year).unwrap_or(i32::MAX),
        u32::try_from(month).unwrap_or(12),
        u32::try_from(day).unwrap_or(31),
    )
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn render_tsv(report: &EvalReport) -> String {
    let mut out = String::from("case_id\tassertion\tstatus\tmessage\n");
    for case in &report.cases {
        for assertion in &case.assertions {
            let _ = writeln!(
                out,
                "{}\t{}\t{}\t{}",
                tsv_escape(&case.id),
                tsv_escape(&assertion.name),
                assertion.status,
                tsv_escape(&assertion.message)
            );
        }
    }
    out
}

fn render_markdown(report: &EvalReport) -> String {
    let mut out = render_eval_card_markdown(&report.eval_card);
    out.push_str("\n## Assertion Details\n\n");
    for case in &report.cases {
        let _ = writeln!(out, "### {}\n", case.id);
        if let Some(date_anchor) = case.date_anchor.as_ref() {
            out.push_str("Date anchor:\n");
            for line in date_anchor.markdown_lines() {
                let _ = writeln!(out, "- {line}");
            }
            out.push('\n');
        }
        for assertion in &case.assertions {
            let _ = writeln!(
                out,
                "- `{}` `{}`: {}",
                assertion.status, assertion.name, assertion.message
            );
        }
        out.push('\n');
    }
    out
}

fn render_eval_card_markdown(card: &EvalCard) -> String {
    let mut out = format!(
        "# Nova Eval Card\n\n- Suite: `{}`\n- Version: `{}`\n- Mode: `{}`\n- Purpose: {}\n- Run status: `{}`\n- Pass rate: `{:.1}%` ({} pass, {} fail, {} error / {} assertions)\n- Gate status: `{}`\n- Telemetry: `{}`\n- Output: `{}`\n",
        card.suite_name,
        card.version,
        card.mode,
        card.purpose,
        card.run_status,
        card.pass_rate * 100.0,
        card.pass_count,
        card.fail_count,
        card.error_count,
        card.assertion_count,
        card.gate.status,
        card.telemetry.status,
        card.output_dir
    );
    if let Some(persona) = card.persona.as_ref() {
        let _ = writeln!(out, "- Persona: `{persona}`");
    }
    if let Some(path) = card.suite_path.as_ref() {
        let _ = writeln!(out, "- Suite path: `{path}`");
    }
    let _ = writeln!(
        out,
        "- Cases: {} bridge, {} agent, {} run",
        card.bridge_case_count, card.agent_case_count, card.run_case_count
    );
    let _ = writeln!(out, "- Manifest scope: {}", card.manifest_scope.declared);
    if let Some(source) = card.manifest_scope.manifest_source.as_ref() {
        let _ = writeln!(out, "- Manifest source: `{source}`");
    }
    if let Some(hash) = card.manifest_scope.manifest_hash.as_ref() {
        let _ = writeln!(out, "- Manifest hash: `{hash}`");
    }
    if let Some(threshold) = card.gate.threshold {
        let _ = writeln!(out, "- Gate threshold: `{threshold:.3}`");
    }
    let _ = writeln!(out, "- Gate message: {}", card.gate.message);
    if let Some(timestamp) = card.telemetry.timestamp.as_ref() {
        let _ = writeln!(out, "- Telemetry timestamp: `{timestamp}`");
    }
    if let Some(run_id) = card.telemetry.run_id.as_ref() {
        let _ = writeln!(out, "- Telemetry run: `{run_id}`");
    }
    let _ = writeln!(out, "- Telemetry rows: {}", card.telemetry.row_count);
    let _ = writeln!(out, "- Telemetry message: {}", card.telemetry.message);
    if let Some(date_anchor) = card.date_anchor.as_ref() {
        out.push_str("- Suite date anchor:\n");
        for line in date_anchor.markdown_lines() {
            let _ = writeln!(out, "  - {line}");
        }
    }
    if let Some(provider) = card.provider.as_ref() {
        let _ = writeln!(out, "- Provider: `{}`", provider.provider);
        let _ = writeln!(
            out,
            "- Provider command preset: `{}`",
            provider.command_preset
        );
        if let Some(model) = provider.model.as_ref() {
            let _ = writeln!(out, "- Provider model: `{model}`");
        }
    }
    out.push_str("- Known gaps:\n");
    if card.known_gaps.is_empty() {
        out.push_str("  - None declared.\n");
    } else {
        for gap in &card.known_gaps {
            let _ = writeln!(out, "  - {gap}");
        }
    }
    out
}

fn tsv_escape(value: &str) -> String {
    value.replace(['\t', '\n', '\r'], " ")
}

fn finish_report(
    command: &str,
    report: &EvalReport,
    json_output: bool,
    elapsed_ms: u128,
) -> DispatchResult {
    let gate_failed = report.gate_status != "pass";
    if json_output {
        let envelope = CliEnvelope::success(command, &report, elapsed_ms);
        let out = serde_json::to_string_pretty(&envelope)
            .map_err(|error| server_error(error.to_string()))?;
        println!("{out}");
        if gate_failed {
            return Err(DispatchError {
                error: DbtNovaError::ServerError(format!(
                    "eval gate failed: pass rate {:.3} below threshold {:.3}",
                    report.pass_rate, report.fail_under
                )),
                rendered: true,
            });
        }
        return Ok(());
    }

    println!(
        "Nova eval {}: {} ({}/{} passed, {:.1}%). Artifacts: {}",
        report.mode,
        report.gate_status,
        report.pass_count,
        report.assertion_count,
        report.pass_rate * 100.0,
        report.output_dir
    );
    if gate_failed {
        return Err(DbtNovaError::ServerError(format!(
            "eval gate failed: pass rate {:.3} below threshold {:.3}",
            report.pass_rate, report.fail_under
        ))
        .into());
    }
    Ok(())
}

impl EvalCaseReport {
    fn new(
        id: String,
        question: Option<String>,
        assertions: Vec<AssertionResult>,
        artifacts: Option<AgentArtifacts>,
    ) -> Self {
        let pass_count = assertions
            .iter()
            .filter(|assertion| assertion.status == "pass")
            .count();
        let fail_count = assertions
            .iter()
            .filter(|assertion| assertion.status == "fail")
            .count();
        let error_count = assertions
            .iter()
            .filter(|assertion| assertion.status == "error")
            .count();
        Self {
            id,
            question,
            pass_count,
            fail_count,
            error_count,
            assertions,
            date_anchor: None,
            artifacts,
            telemetry: None,
        }
    }

    fn with_date_anchor(mut self, date_anchor: Option<DateAnchor>) -> Self {
        self.date_anchor = date_anchor;
        self
    }

    fn with_telemetry(mut self, telemetry: EvalCaseTelemetry) -> Self {
        self.telemetry = Some(telemetry);
        self
    }
}

impl AssertionResult {
    fn pass(name: impl Into<String>, message: impl Into<String>, evidence: JsonValue) -> Self {
        Self {
            name: name.into(),
            status: "pass",
            message: message.into(),
            evidence,
        }
    }

    fn fail(name: impl Into<String>, message: impl Into<String>, evidence: JsonValue) -> Self {
        Self {
            name: name.into(),
            status: "fail",
            message: message.into(),
            evidence,
        }
    }

    fn error(name: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            status: "error",
            message: message.into(),
            evidence: JsonValue::Null,
        }
    }
}

impl AgentExpected {
    fn requires_trace(&self) -> bool {
        !self.must_call.is_empty()
            || !self.must_not_call.is_empty()
            || !self.ordered.is_empty()
            || !self.selected_entities.is_empty()
            || !self.selected_entity_ranks.is_empty()
            || !self.called_with.is_empty()
            || self.max_tool_calls.is_some()
            || self.max_distinct_tools.is_some()
            || self.max_total_response_bytes.is_some()
            || !self.max_response_bytes_by_tool.is_empty()
    }
}

fn build_eval_validate_payload(path: &str) -> crate::error::Result<JsonValue> {
    let suite = load_suite(path)?;
    let date_anchor_case_count = suite
        .cases
        .iter()
        .filter(|case| effective_date_anchor(&suite.date_anchor, &case.date_anchor).is_some())
        .count()
        + suite
            .agent_cases
            .iter()
            .filter(|case| effective_date_anchor(&suite.date_anchor, &case.date_anchor).is_some())
            .count();
    Ok(json!({
        "valid": true,
        "path": path,
        "suite_name": suite.name.as_deref().unwrap_or("suite"),
        "version": suite.version,
        "date_anchor": suite.date_anchor.normalized(),
        "date_anchor_case_count": date_anchor_case_count,
        "bridge_case_count": suite.cases.len(),
        "agent_case_count": suite.agent_cases.len(),
    }))
}

fn build_eval_gate_report_for_suite(suite_name: &str) -> crate::error::Result<EvalGateReport> {
    let rows = read_telemetry_rows_for_suite(suite_name)?;
    build_eval_gate_report(suite_name, &rows)
}

fn normalized_string(value: Option<&String>) -> Option<String> {
    value
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn effective_date_anchor(suite: &DateAnchor, case: &DateAnchor) -> Option<DateAnchor> {
    let anchor = DateAnchor {
        snapshot_date: normalized_string(case.snapshot_date.as_ref())
            .or_else(|| normalized_string(suite.snapshot_date.as_ref())),
        date_range_start: normalized_string(case.date_range_start.as_ref())
            .or_else(|| normalized_string(suite.date_range_start.as_ref())),
        date_range_end: normalized_string(case.date_range_end.as_ref())
            .or_else(|| normalized_string(suite.date_range_end.as_ref())),
        date_field: normalized_string(case.date_field.as_ref())
            .or_else(|| normalized_string(suite.date_field.as_ref())),
    };
    (!anchor.is_empty()).then_some(anchor)
}

fn validate_date_anchor(anchor: &DateAnchor, location: &str) -> crate::error::Result<()> {
    validate_optional_date(anchor.snapshot_date.as_deref(), location, "snapshot_date")?;
    validate_optional_date(
        anchor.date_range_start.as_deref(),
        location,
        "date_range_start",
    )?;
    validate_optional_date(anchor.date_range_end.as_deref(), location, "date_range_end")?;
    if let Some(field) = anchor.date_field.as_deref()
        && field.trim().is_empty()
    {
        return Err(DbtNovaError::InvalidParams(format!(
            "{location} date_field must be non-empty when set"
        )));
    }
    Ok(())
}

fn validate_complete_date_anchor(anchor: &DateAnchor, location: &str) -> crate::error::Result<()> {
    let snapshot_date =
        validate_optional_date(anchor.snapshot_date.as_deref(), location, "snapshot_date")?;
    let date_range_start = validate_optional_date(
        anchor.date_range_start.as_deref(),
        location,
        "date_range_start",
    )?;
    let date_range_end =
        validate_optional_date(anchor.date_range_end.as_deref(), location, "date_range_end")?;
    if date_range_start.is_some() != date_range_end.is_some() {
        return Err(DbtNovaError::InvalidParams(format!(
            "{location} must include both date_range_start and date_range_end when either date range field is set"
        )));
    }
    if let (Some(start), Some(end)) = (date_range_start, date_range_end)
        && start > end
    {
        return Err(DbtNovaError::InvalidParams(format!(
            "{location} date_range_start must be on or before date_range_end"
        )));
    }
    if snapshot_date.is_none()
        && date_range_start.is_none()
        && anchor
            .date_field
            .as_deref()
            .is_some_and(|field| !field.trim().is_empty())
    {
        return Err(DbtNovaError::InvalidParams(format!(
            "{location} date_field requires snapshot_date or date_range_start/date_range_end"
        )));
    }
    Ok(())
}

fn validate_optional_date(
    value: Option<&str>,
    location: &str,
    field: &str,
) -> crate::error::Result<Option<(i32, u32, u32)>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(DbtNovaError::InvalidParams(format!(
            "{location} {field} must be a non-empty YYYY-MM-DD date"
        )));
    }
    parse_iso_date(trimmed).map(Some).ok_or_else(|| {
        DbtNovaError::InvalidParams(format!(
            "{location} {field} must use YYYY-MM-DD with a valid calendar date"
        ))
    })
}

fn parse_iso_date(value: &str) -> Option<(i32, u32, u32)> {
    let bytes = value.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return None;
    }
    let year = i32::try_from(parse_date_part(&value[0..4])?).ok()?;
    let month = parse_date_part(&value[5..7])?;
    let day = parse_date_part(&value[8..10])?;
    let max_day = days_in_month(year, month);
    if max_day == 0 || day == 0 || day > max_day {
        return None;
    }
    Some((year, month, day))
}

fn parse_date_part(value: &str) -> Option<u32> {
    value
        .as_bytes()
        .iter()
        .all(u8::is_ascii_digit)
        .then(|| value.parse::<u32>().ok())
        .flatten()
}

fn eval_history_rows(
    suite_name: &str,
    since: &str,
) -> crate::error::Result<(String, Vec<JsonValue>)> {
    let since_boundary = validate_since_date(since)?;
    let rows = read_telemetry_rows_for_suite(suite_name)?
        .into_iter()
        .filter(|row| telemetry_row_matches_since(row, &since_boundary))
        .collect();
    Ok((since_boundary, rows))
}

fn ensure_mcp_telemetry_suite_paths_under_root(rows: &[JsonValue]) -> crate::error::Result<()> {
    for row in latest_telemetry_rows(rows) {
        let Some(path) = row
            .get("suite_path")
            .and_then(JsonValue::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        let (_root, candidate) = mcp_eval_candidate_path(path, "suite_path")?;
        if candidate.exists() {
            let canonical = candidate.canonicalize().map_err(|error| {
                DbtNovaError::InvalidParams(format!(
                    "failed to resolve suite_path '{}': {error}",
                    candidate.display()
                ))
            })?;
            let root = mcp_eval_filesystem_root()?;
            ensure_mcp_eval_path_under_root(&canonical, &root, "suite_path")?;
            if !canonical.is_file() {
                return Err(DbtNovaError::InvalidParams(format!(
                    "suite_path '{}' is not a file",
                    canonical.display()
                )));
            }
        }
    }
    Ok(())
}

fn success_value<T: Serialize>(payload: T, count: usize) -> crate::error::Result<JsonValue> {
    serde_json::to_value(SuccessResponse::new(payload, count))
        .map_err(|error| DbtNovaError::ServerError(error.to_string()))
}

fn with_eval_safety_policy(mut payload: JsonValue) -> crate::error::Result<JsonValue> {
    let policy = serde_json::to_value(eval_mcp_safety_policy()?)
        .map_err(|error| DbtNovaError::ServerError(error.to_string()))?;
    let Some(object) = payload.as_object_mut() else {
        return Err(DbtNovaError::ServerError(
            "failed to serialize eval response payload as object".to_string(),
        ));
    };
    object.insert("safety_policy".to_string(), policy);
    Ok(payload)
}

fn eval_mcp_safety_policy() -> crate::error::Result<EvalMcpSafetyPolicy> {
    Ok(EvalMcpSafetyPolicy {
        filesystem_root: mcp_eval_filesystem_root()?.display().to_string(),
        eval_run_enabled_env: MCP_ENABLE_EVAL_RUN_ENV,
        eval_writes_enabled_env: MCP_ENABLE_EVAL_WRITES_ENV,
        agent_eval_enabled_env: MCP_ENABLE_AGENT_EVAL_ENV,
        custom_agent_provider_enabled_env: MCP_ENABLE_CUSTOM_AGENT_PROVIDER_ENV,
        local_paths_must_stay_under_filesystem_root: true,
    })
}

fn require_mcp_eval_flag(env_name: &'static str, tool_name: &str) -> crate::error::Result<()> {
    if mcp_eval_flag_enabled(env_name) {
        return Ok(());
    }
    Err(DbtNovaError::InvalidParams(format!(
        "{tool_name} is disabled for MCP/tool-call use; set {env_name}=1 to enable this local execution capability"
    )))
}

fn mcp_eval_flag_enabled(env_name: &str) -> bool {
    std::env::var(env_name)
        .ok()
        .map(|value| value.trim().to_ascii_lowercase())
        .is_some_and(|value| matches!(value.as_str(), "1" | "true" | "yes" | "on"))
}

fn resolve_mcp_existing_file(raw_path: &str, label: &str) -> crate::error::Result<PathBuf> {
    let (_root, candidate) = mcp_eval_candidate_path(raw_path, label)?;
    let canonical = candidate.canonicalize().map_err(|error| {
        DbtNovaError::InvalidParams(format!(
            "failed to resolve {label} '{}': {error}",
            candidate.display()
        ))
    })?;
    let root = mcp_eval_filesystem_root()?;
    ensure_mcp_eval_path_under_root(&canonical, &root, label)?;
    if !canonical.is_file() {
        return Err(DbtNovaError::InvalidParams(format!(
            "{label} '{}' is not a file",
            canonical.display()
        )));
    }
    Ok(canonical)
}

fn resolve_mcp_writable_path(raw_path: &str, label: &str) -> crate::error::Result<PathBuf> {
    let (root, candidate) = mcp_eval_candidate_path(raw_path, label)?;
    if candidate.exists() {
        let canonical = candidate.canonicalize().map_err(|error| {
            DbtNovaError::InvalidParams(format!(
                "failed to resolve {label} '{}': {error}",
                candidate.display()
            ))
        })?;
        ensure_mcp_eval_path_under_root(&canonical, &root, label)?;
        return Ok(canonical);
    }
    ensure_existing_ancestor_under_root(&candidate, &root, label)?;
    Ok(candidate)
}

fn mcp_eval_candidate_path(
    raw_path: &str,
    label: &str,
) -> crate::error::Result<(PathBuf, PathBuf)> {
    let trimmed = raw_path.trim();
    if trimmed.is_empty() {
        return Err(DbtNovaError::InvalidParams(format!(
            "{label} must not be empty"
        )));
    }
    let root = mcp_eval_filesystem_root()?;
    let path = PathBuf::from(trimmed);
    reject_mcp_eval_parent_dirs(&path, label)?;
    if !path.is_absolute() {
        reject_mcp_eval_relative_traversal(&path, label)?;
    }
    let candidate = if path.is_absolute() {
        path
    } else {
        root.join(path)
    };
    ensure_mcp_eval_path_under_root(&candidate, &root, label)?;
    Ok((root, candidate))
}

fn reject_mcp_eval_parent_dirs(path: &Path, label: &str) -> crate::error::Result<()> {
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(DbtNovaError::InvalidParams(format!(
            "{label} must stay under the server working directory"
        )));
    }
    Ok(())
}

fn reject_mcp_eval_relative_traversal(path: &Path, label: &str) -> crate::error::Result<()> {
    for component in path.components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            Component::ParentDir => {
                return Err(DbtNovaError::InvalidParams(format!(
                    "{label} must stay under the server working directory"
                )));
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(DbtNovaError::InvalidParams(format!(
                    "{label} must be relative or resolve under the server working directory"
                )));
            }
        }
    }
    Ok(())
}

fn ensure_existing_ancestor_under_root(
    candidate: &Path,
    root: &Path,
    label: &str,
) -> crate::error::Result<()> {
    let mut ancestor = candidate.parent();
    while let Some(path) = ancestor {
        if path.exists() {
            let canonical = path.canonicalize().map_err(|error| {
                DbtNovaError::InvalidParams(format!(
                    "failed to resolve parent for {label} '{}': {error}",
                    path.display()
                ))
            })?;
            return ensure_mcp_eval_path_under_root(&canonical, root, label);
        }
        ancestor = path.parent();
    }
    Err(DbtNovaError::InvalidParams(format!(
        "{label} has no existing parent under '{}'",
        root.display()
    )))
}

fn ensure_mcp_eval_path_under_root(
    path: &Path,
    root: &Path,
    label: &str,
) -> crate::error::Result<()> {
    if path.starts_with(root) {
        return Ok(());
    }
    Err(DbtNovaError::InvalidParams(format!(
        "{label} '{}' is outside server working directory '{}'",
        path.display(),
        root.display()
    )))
}

fn mcp_eval_filesystem_root() -> crate::error::Result<PathBuf> {
    std::env::current_dir()
        .and_then(|path| path.canonicalize())
        .map_err(|error| {
            DbtNovaError::InvalidParams(format!(
                "failed to resolve server working directory: {error}"
            ))
        })
}

fn load_suite(path: &str) -> crate::error::Result<EvalSuite> {
    load_suite_with_hash(path).map(|(suite, _hash)| suite)
}

fn load_suite_with_hash(path: &str) -> crate::error::Result<(EvalSuite, String)> {
    let raw = fs::read(path).map_err(|error| {
        DbtNovaError::InvalidParams(format!("failed to read eval suite '{path}': {error}"))
    })?;
    let suite_hash = blake3::hash(&raw).to_hex().to_string();
    let raw = std::str::from_utf8(&raw).map_err(|error| {
        DbtNovaError::InvalidParams(format!(
            "failed to read eval suite '{path}' as UTF-8: {error}"
        ))
    })?;
    let suite: EvalSuite = if Path::new(path)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
    {
        serde_json::from_str(raw).map_err(|error| {
            DbtNovaError::InvalidParams(format!("failed to parse eval suite JSON: {error}"))
        })?
    } else {
        serde_yaml::from_str(raw).map_err(|error| {
            DbtNovaError::InvalidParams(format!("failed to parse eval suite YAML: {error}"))
        })?
    };
    validate_suite(&suite)?;
    Ok((suite, suite_hash))
}

fn validate_suite(suite: &EvalSuite) -> crate::error::Result<()> {
    if suite.version == 0 {
        return Err(DbtNovaError::InvalidParams(
            "eval suite version must be greater than zero".to_string(),
        ));
    }
    if let Some(gate) = suite.gate
        && !(0.0..=1.0).contains(&gate.threshold)
    {
        return Err(DbtNovaError::InvalidParams(
            "eval suite gate.threshold must be between 0.0 and 1.0".to_string(),
        ));
    }
    validate_date_anchor(&suite.date_anchor, "eval suite")?;
    if suite.cases.is_empty()
        && suite.agent_cases.is_empty()
        && let Some(date_anchor) = suite.date_anchor.normalized()
    {
        validate_complete_date_anchor(&date_anchor, "eval suite")?;
    }
    validate_case_ids(suite.cases.iter().map(|case| case.id.as_str()), "cases")?;
    validate_case_ids(
        suite.agent_cases.iter().map(|case| case.id.as_str()),
        "agent_cases",
    )?;
    for case in &suite.cases {
        validate_date_anchor(&case.date_anchor, &format!("eval case '{}'", case.id))?;
        if let Some(date_anchor) = effective_date_anchor(&suite.date_anchor, &case.date_anchor) {
            validate_complete_date_anchor(
                &date_anchor,
                &format!("effective date anchor for eval case '{}'", case.id),
            )?;
        }
        if case.assertions.is_empty() {
            return Err(DbtNovaError::InvalidParams(format!(
                "eval case '{}' must include at least one assertion",
                case.id
            )));
        }
        for assertion in &case.assertions {
            validate_assertion(assertion, &case.id)?;
        }
    }
    for case in &suite.agent_cases {
        validate_date_anchor(&case.date_anchor, &format!("agent case '{}'", case.id))?;
        if let Some(date_anchor) = effective_date_anchor(&suite.date_anchor, &case.date_anchor) {
            validate_complete_date_anchor(
                &date_anchor,
                &format!("effective date anchor for agent case '{}'", case.id),
            )?;
        }
        if case.task.trim().is_empty() {
            return Err(DbtNovaError::InvalidParams(format!(
                "agent case '{}' must include a non-empty task",
                case.id
            )));
        }
        validate_agent_expected(&case.expected, &case.id)?;
    }
    Ok(())
}

fn validate_assertion(assertion: &EvalAssertion, case_id: &str) -> crate::error::Result<()> {
    match assertion {
        EvalAssertion::SearchColumnsRank {
            expected_column,
            expected_parent_unique_id,
            ..
        } => {
            let has_expected_column = expected_column
                .as_ref()
                .is_some_and(|value| !value.trim().is_empty());
            let has_expected_parent = expected_parent_unique_id
                .as_ref()
                .is_some_and(|value| !value.trim().is_empty());
            if !has_expected_column && !has_expected_parent {
                return Err(DbtNovaError::InvalidParams(format!(
                    "search_columns_rank assertion in case '{case_id}' must include expected_column or expected_parent_unique_id"
                )));
            }
        }
        EvalAssertion::ContextFieldEquals { field, .. } if field.trim().is_empty() => {
            return Err(DbtNovaError::InvalidParams(format!(
                "context_field_equals assertion in case '{case_id}' must include a non-empty field"
            )));
        }
        EvalAssertion::ContextContains {
            expected, field, ..
        } => {
            if expected.trim().is_empty() {
                return Err(DbtNovaError::InvalidParams(format!(
                    "context_contains assertion in case '{case_id}' must include non-empty expected text"
                )));
            }
            if field.as_ref().is_some_and(|field| field.trim().is_empty()) {
                return Err(DbtNovaError::InvalidParams(format!(
                    "context_contains assertion in case '{case_id}' must include a non-empty field when field is set"
                )));
            }
        }
        EvalAssertion::ToolResponseBudget {
            tool,
            max_response_bytes,
            must_contain_paths,
            must_not_contain_paths,
            ..
        } => {
            if tool.trim().is_empty() {
                return Err(DbtNovaError::InvalidParams(format!(
                    "tool_response_budget assertion in case '{case_id}' must include a non-empty tool"
                )));
            }
            if *max_response_bytes == 0 {
                return Err(DbtNovaError::InvalidParams(format!(
                    "tool_response_budget assertion in case '{case_id}' must use max_response_bytes greater than zero"
                )));
            }
            if must_contain_paths
                .iter()
                .chain(must_not_contain_paths)
                .any(|path| path.trim().is_empty())
            {
                return Err(DbtNovaError::InvalidParams(format!(
                    "tool_response_budget assertion in case '{case_id}' must use non-empty field paths"
                )));
            }
        }
        _ => {}
    }
    Ok(())
}

fn validate_agent_expected(expected: &AgentExpected, case_id: &str) -> crate::error::Result<()> {
    for rank in &expected.selected_entity_ranks {
        if rank.unique_id.trim().is_empty() {
            return Err(DbtNovaError::InvalidParams(format!(
                "selected_entity_ranks in agent case '{case_id}' must include non-empty unique_id values"
            )));
        }
        if rank
            .tool
            .as_ref()
            .is_some_and(|tool| tool.trim().is_empty())
        {
            return Err(DbtNovaError::InvalidParams(format!(
                "selected_entity_ranks in agent case '{case_id}' must include non-empty tool values when tool is set"
            )));
        }
        if rank.max_rank == Some(0) {
            return Err(DbtNovaError::InvalidParams(format!(
                "selected_entity_ranks in agent case '{case_id}' must use max_rank greater than zero"
            )));
        }
    }
    for called_with in &expected.called_with {
        if called_with.tool.trim().is_empty() {
            return Err(DbtNovaError::InvalidParams(format!(
                "called_with expectations in agent case '{case_id}' must include non-empty tool values"
            )));
        }
        if called_with.params.is_empty() && called_with.contains.is_empty() {
            return Err(DbtNovaError::InvalidParams(format!(
                "called_with expectations in agent case '{case_id}' must include params or contains constraints"
            )));
        }
        if called_with
            .contains
            .iter()
            .any(|(key, value)| key.trim().is_empty() || value.trim().is_empty())
        {
            return Err(DbtNovaError::InvalidParams(format!(
                "called_with contains expectations in agent case '{case_id}' must include non-empty keys and values"
            )));
        }
        if called_with.params.keys().any(|key| key.trim().is_empty()) {
            return Err(DbtNovaError::InvalidParams(format!(
                "called_with params expectations in agent case '{case_id}' must include non-empty keys"
            )));
        }
        if called_with
            .params
            .values()
            .any(|value| !is_safe_expected_param_value(value))
        {
            return Err(DbtNovaError::InvalidParams(format!(
                "called_with params expectations in agent case '{case_id}' must use scalar values or arrays of scalar values"
            )));
        }
    }
    if expected.max_tool_calls == Some(0)
        || expected.max_distinct_tools == Some(0)
        || expected.max_total_response_bytes == Some(0)
        || expected
            .max_response_bytes_by_tool
            .values()
            .any(|max| *max == 0)
    {
        return Err(DbtNovaError::InvalidParams(format!(
            "agent case '{case_id}' budget expectations must use positive thresholds"
        )));
    }
    if expected
        .max_response_bytes_by_tool
        .keys()
        .any(|tool| tool.trim().is_empty())
    {
        return Err(DbtNovaError::InvalidParams(format!(
            "agent case '{case_id}' max_response_bytes_by_tool keys must be non-empty tool names"
        )));
    }
    Ok(())
}

fn is_safe_expected_param_value(value: &JsonValue) -> bool {
    match value {
        JsonValue::Null | JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::String(_) => true,
        JsonValue::Array(items) => items.iter().all(|item| {
            matches!(
                item,
                JsonValue::Null | JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::String(_)
            )
        }),
        JsonValue::Object(_) => false,
    }
}

fn validate_case_ids<'a>(
    ids: impl Iterator<Item = &'a str>,
    section: &str,
) -> crate::error::Result<()> {
    let mut seen = BTreeSet::new();
    let mut seen_artifact_segments = BTreeSet::new();
    for id in ids {
        let trimmed = id.trim();
        if trimmed.is_empty() {
            return Err(DbtNovaError::InvalidParams(format!(
                "{section} entries must include non-empty ids"
            )));
        }
        if !seen.insert(trimmed.to_string()) {
            return Err(DbtNovaError::InvalidParams(format!(
                "duplicate eval case id '{trimmed}' in {section}"
            )));
        }
        let artifact_segment = safe_path_segment(trimmed);
        let artifact_segment_key = artifact_segment.to_ascii_lowercase();
        if !seen_artifact_segments.insert(artifact_segment_key) {
            return Err(DbtNovaError::InvalidParams(format!(
                "eval case ids in {section} must map to unique artifact paths case-insensitively; duplicate segment '{artifact_segment}'"
            )));
        }
    }
    Ok(())
}

fn resolve_output_dir(explicit: Option<&str>, suite: &EvalSuite, mode: &str) -> PathBuf {
    if let Some(path) = explicit {
        return PathBuf::from(path);
    }
    let suite_name = suite.name.as_deref().unwrap_or("suite");
    PathBuf::from(".nova").join("eval-runs").join(format!(
        "{}-{}-{}",
        timestamp_secs(),
        safe_path_segment(suite_name),
        mode
    ))
}

fn validate_fail_under(value: Option<f64>) -> crate::error::Result<()> {
    let Some(value) = value else {
        return Ok(());
    };
    if !(0.0..=1.0).contains(&value) {
        return Err(DbtNovaError::InvalidParams(
            "--fail-under must be between 0.0 and 1.0".to_string(),
        ));
    }
    Ok(())
}

fn selected_bridge_cases<'a>(
    cases: &'a [EvalCase],
    case_ids: &[String],
) -> crate::error::Result<Vec<&'a EvalCase>> {
    if case_ids.is_empty() {
        return Ok(cases.iter().collect());
    }
    let wanted = normalized_case_filter(case_ids)?;
    let selected: Vec<&EvalCase> = cases
        .iter()
        .filter(|case| wanted.contains(case.id.as_str()))
        .collect();
    validate_selected_cases(
        &wanted,
        selected.iter().map(|case| case.id.as_str()),
        "bridge",
    )?;
    Ok(selected)
}

fn selected_agent_cases<'a>(
    cases: &'a [AgentCase],
    case_ids: &[String],
) -> crate::error::Result<Vec<&'a AgentCase>> {
    if case_ids.is_empty() {
        return Ok(cases.iter().collect());
    }
    let wanted = normalized_case_filter(case_ids)?;
    let selected: Vec<&AgentCase> = cases
        .iter()
        .filter(|case| wanted.contains(case.id.as_str()))
        .collect();
    validate_selected_cases(
        &wanted,
        selected.iter().map(|case| case.id.as_str()),
        "agent",
    )?;
    Ok(selected)
}

fn normalized_case_filter(case_ids: &[String]) -> crate::error::Result<BTreeSet<&str>> {
    let mut wanted = BTreeSet::new();
    for id in case_ids {
        let trimmed = id.trim();
        if trimmed.is_empty() {
            return Err(DbtNovaError::InvalidParams(
                "--case-id values must be non-empty".to_string(),
            ));
        }
        wanted.insert(trimmed);
    }
    Ok(wanted)
}

fn validate_selected_cases<'a>(
    wanted: &BTreeSet<&'a str>,
    selected: impl Iterator<Item = &'a str>,
    mode: &str,
) -> crate::error::Result<()> {
    let found: BTreeSet<&str> = selected.collect();
    let missing: Vec<&str> = wanted.difference(&found).copied().collect();
    if missing.is_empty() {
        return Ok(());
    }
    Err(DbtNovaError::InvalidParams(format!(
        "requested {mode} eval case id(s) not found: {}",
        missing.join(", ")
    )))
}

fn agent_prompt(case: &AgentCase, date_anchor: Option<&DateAnchor>) -> String {
    let date_anchor_section = date_anchor.map_or_else(String::new, |anchor| {
        let mut section = String::from("\nDate anchor:\n");
        for line in anchor.prompt_lines() {
            let _ = writeln!(section, "- {line}");
        }
        section.push_str("- Treat these dates as ground truth for relative time phrases in the task. Do not reinterpret them using today's date.\n");
        section
    });
    format!(
        "You are running a dbt-nova analytics-agent eval.\n\nTask:\n{}\n{}\nRules:\n- Use Nova discovery and execution tools directly. Do not inspect repository files, source code, fixtures, or Rust params unless a Nova command fails and you cannot recover from the error message.\n- For KPI, metric, conversion, funnel, checkout, or business-concept questions, start with search_indicator using compact results: detail=\"compact\", group_mode=\"top\", include_support_signals=true, limit=3, persona=\"analyst\".\n- For rate, conversion, or funnel questions, include the requested metric names literally in the query and set indicator_types=[\"metric\"] unless you are explicitly searching for raw measures.\n- When a metric row returns an expression, copy that expression exactly into SQL; do not substitute similarly named measures or invent a numerator/denominator.\n- Use support_signals, grain dimensions, and relation_name from search_indicator to apply every requested filter before SQL. Do not aggregate across a grain dimension named in the task, such as country, market, channel, segment, or device.\n- Treat relation_name, grain, and expression fields returned by search_indicator as the execution contract. Do not run schema inspection SQL such as DESCRIBE or information_schema when those fields are present.\n- Use execute_sql only after Nova discovery identifies the canonical execution entity or relation. Use one aggregate SQL statement for current and comparison periods when possible. Skip get_entity when search_indicator already returns the relation, grain, measures, and metric expressions you need; otherwise use get_entity with id_or_name and detail=\"compact\".\n- Keep Nova calls to the minimum needed: usually search_indicator plus one execute_sql for calculations, and only search_indicator for model or metric lookup tasks. Avoid get_context, get_lineage, get_sql, and full-detail responses unless blocked.\n- If using the CLI, assume $DBT_NOVA_EVAL_BIN is set. For search/get calls, use --params-json. For execute_sql with quotes or newlines, write a JSON params file like {{\"statement\":\"select ...\",\"row_limit\":50}} and call $DBT_NOVA_EVAL_BIN tool call execute_sql --params-file <file> --json; do not inline multiline SQL in --params-json. Parameter reminders: get_entity uses id_or_name; execute_sql uses statement. Do not run echo, grep, read, or source inspection for normal tool usage.\n\nFinish with a concise answer that cites the Nova evidence, the SQL result, and the explicit filter values used.",
        case.task, date_anchor_section
    )
}

fn starter_suite(persona: &str) -> String {
    let safe_persona = safe_path_segment(persona);
    let persona_yaml = serde_json::to_string(persona).unwrap_or_else(|_| "\"analyst\"".to_string());
    format!(
        r"version: 1
name: nova-{safe_persona}-smoke
defaults:
  persona: {persona_yaml}
  top_k: 5
cases:
  - id: canonical_entity_search
    question: Find the canonical entity for a business concept.
    assertions:
      - type: search_rank
        query: orders
        expected_unique_id: model.pkg.orders
        max_rank: 5
      - type: context_has
        id_or_name: model.pkg.orders
        fields:
          - data.unique_id
          - data.entity.name
agent_cases:
  - id: analyst_metric_lookup
    task: Which canonical model and indicator should be used to analyze gross merchandise value?
    expected:
      must_call:
        - search_indicator
        - get_context
      selected_entities:
        - model.pkg.orders
      final_answer:
        must_contain:
          - gross merchandise value
"
    )
}

fn empty_object() -> JsonValue {
    json!({})
}

fn default_top_k() -> usize {
    DEFAULT_TOP_K
}

fn safe_path_segment(value: &str) -> String {
    let sanitized: String = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = sanitized.trim_matches(['-', '.']).trim();
    let capped: String = trimmed.chars().take(MAX_SAFE_PATH_SEGMENT_CHARS).collect();
    if capped.is_empty() || matches!(capped.as_str(), "." | "..") {
        "eval".to_string()
    } else {
        capped
    }
}

fn truncate(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

fn timestamp_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

fn elapsed_ms_to_u64(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn ratio(numerator: usize, denominator: usize) -> f64 {
    let numerator = u32::try_from(numerator).unwrap_or(u32::MAX);
    let denominator = u32::try_from(denominator).unwrap_or(u32::MAX);
    f64::from(numerator) / f64::from(denominator)
}

fn server_error(message: String) -> DbtNovaError {
    DbtNovaError::ServerError(message)
}

#[cfg(test)]
mod tests;
