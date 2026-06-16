use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde_json::{Value as JsonValue, json};
use tokio::process::Command;

use crate::cli::args::EvalAgentRunArgs;
use crate::error::DbtNovaError;
use crate::tools::catalog::MCP_TOOL_NAMES;
use crate::utils::tool_trace::TRACE_ENV;

use super::server_error;

const PROVIDER_MCP_SERVER_ALIASES_ENV: &str = "DBT_NOVA_EVAL_MCP_SERVER_ALIASES";
const DEFAULT_NOVA_MCP_SERVER_ALIASES: [&str; 5] =
    ["nova", "dbt-nova", "dbt_nova", "dbtnova", "dbt-nova-mcp"];
const MAX_PROVIDER_TOP_UNIQUE_IDS: usize = 20;

#[derive(Debug)]
pub(super) struct ProviderInvocation {
    pub(super) command: String,
    pub(super) args: Vec<String>,
}

pub(super) struct ProviderOutput {
    pub(super) status_success: bool,
    pub(super) stdout: String,
    pub(super) stderr: String,
}

pub(super) async fn run_provider_command(
    invocation: &ProviderInvocation,
    args: &EvalAgentRunArgs,
    trace_path: &Path,
    case_dir: &Path,
) -> crate::error::Result<ProviderOutput> {
    let current_exe = current_eval_executable()?;
    let provider_path = provider_path_with_current_exe(&current_exe)?;
    let mut command = Command::new(&invocation.command);
    command
        .args(&invocation.args)
        .current_dir(std::env::current_dir().map_err(|error| server_error(error.to_string()))?)
        .env(TRACE_ENV, trace_path)
        .env("DBT_NOVA_EVAL_BIN", &current_exe)
        .env("DBT_NOVA_EVAL_CASE_DIR", case_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if let Some(path) = provider_path {
        command.env("PATH", path);
    }
    if let Some(path) = args.manifest_path.as_ref() {
        command.env("DBT_MANIFEST_PATH", path);
    }
    if let Some(uri) = args.manifest_uri.as_ref() {
        command.env("DBT_NOVA_MANIFEST_URI", uri);
    }
    if let Some(instance_id) = args.storage_instance_id.as_ref() {
        command.env("DBT_NOVA_STORAGE_INSTANCE_ID", instance_id);
    }
    if args.cleanup_storage_on_start {
        command.env("DBT_NOVA_CLEANUP_STORAGE_ON_START", "true");
    }
    if args.read_only {
        command.env("DBT_NOVA_STORAGE_READ_ONLY", "true");
    }

    let timeout = Duration::from_secs(args.timeout_secs.max(1));
    let output = tokio::time::timeout(timeout, command.output())
        .await
        .map_err(|_| {
            DbtNovaError::ServerError(format!(
                "provider command timed out after {} seconds",
                timeout.as_secs()
            ))
        })?
        .map_err(|error| {
            DbtNovaError::ServerError(format!(
                "failed to run provider command '{}': {error}",
                invocation.command
            ))
        })?;

    Ok(ProviderOutput {
        status_success: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    })
}

fn current_eval_executable() -> crate::error::Result<PathBuf> {
    std::env::current_exe().map_err(|error| {
        server_error(format!(
            "failed to resolve current eval executable: {error}"
        ))
    })
}

fn provider_path_with_current_exe(current_exe: &Path) -> crate::error::Result<Option<OsString>> {
    let Some(exe_dir) = current_exe.parent() else {
        return Ok(None);
    };
    let mut paths = Vec::from([exe_dir.to_path_buf()]);
    if let Some(existing) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&existing));
    }
    std::env::join_paths(paths)
        .map(Some)
        .map_err(|error| server_error(format!("failed to prepare provider PATH: {error}")))
}

pub(super) fn provider_invocation(
    args: &EvalAgentRunArgs,
    prompt: &str,
    trace_path: &Path,
) -> crate::error::Result<ProviderInvocation> {
    let command = args.provider_command.clone().map_or_else(
        || default_provider_command(&args.provider),
        validate_custom_command,
    )?;
    let mut raw_args = if let Some(json) = args.provider_args_json.as_ref() {
        let values: Vec<String> = serde_json::from_str(json).map_err(|error| {
            DbtNovaError::InvalidParams(format!("invalid --provider-args-json: {error}"))
        })?;
        substitute_provider_args(values, prompt, trace_path, args)
    } else {
        default_provider_args(&args.provider, prompt)?
    };
    apply_provider_model_arg(args, &mut raw_args)?;
    Ok(ProviderInvocation {
        command,
        args: raw_args,
    })
}

fn validate_custom_command(command: String) -> crate::error::Result<String> {
    if command.trim().is_empty() {
        return Err(DbtNovaError::InvalidParams(
            "--provider-command cannot be empty".to_string(),
        ));
    }
    Ok(command)
}

fn default_provider_command(provider: &str) -> crate::error::Result<String> {
    match provider {
        "opencode" => Ok("opencode".to_string()),
        "codex" => Ok("codex".to_string()),
        "claude" => Ok("claude".to_string()),
        "goose" => Ok("goose".to_string()),
        other => Err(DbtNovaError::InvalidParams(format!(
            "unsupported provider '{other}'; use opencode, codex, claude, goose, or pass --provider-command"
        ))),
    }
}

fn default_provider_args(provider: &str, prompt: &str) -> crate::error::Result<Vec<String>> {
    match provider {
        "opencode" => Ok(vec![
            "run".to_string(),
            "--format".to_string(),
            "json".to_string(),
            prompt.to_string(),
        ]),
        "codex" => Ok(vec![
            "exec".to_string(),
            "--json".to_string(),
            "--cd".to_string(),
            std::env::current_dir()
                .map_err(|error| server_error(error.to_string()))?
                .display()
                .to_string(),
            prompt.to_string(),
        ]),
        "claude" => Ok(vec![
            "-p".to_string(),
            "--verbose".to_string(),
            "--output-format".to_string(),
            "stream-json".to_string(),
            prompt.to_string(),
        ]),
        "goose" => Ok(vec![
            "run".to_string(),
            "--text".to_string(),
            prompt.to_string(),
            "--output-format".to_string(),
            "stream-json".to_string(),
            "--no-session".to_string(),
        ]),
        other => Err(DbtNovaError::InvalidParams(format!(
            "unsupported provider '{other}'; use opencode, codex, claude, goose, or pass --provider-command with --provider-args-json"
        ))),
    }
}

fn apply_provider_model_arg(
    args: &EvalAgentRunArgs,
    raw_args: &mut Vec<String>,
) -> crate::error::Result<()> {
    let Some(model) = args
        .provider_model
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(());
    };
    if args.provider != "opencode" || args.provider_args_json.is_some() {
        return Err(DbtNovaError::InvalidParams(
            "--provider-model is only supported by the opencode provider preset; use --provider-args-json for custom commands".to_string(),
        ));
    }
    let insert_at = raw_args.len().saturating_sub(1);
    raw_args.insert(insert_at, "--model".to_string());
    raw_args.insert(insert_at + 1, model.to_string());
    Ok(())
}

fn substitute_provider_args(
    args: Vec<String>,
    prompt: &str,
    trace_path: &Path,
    run_args: &EvalAgentRunArgs,
) -> Vec<String> {
    let workdir = std::env::current_dir()
        .map(|path| path.display().to_string())
        .unwrap_or_default();
    let manifest_path = run_args.manifest_path.as_deref().unwrap_or_default();
    args.into_iter()
        .map(|arg| {
            arg.replace("{prompt}", prompt)
                .replace("{workdir}", &workdir)
                .replace("{trace_path}", &trace_path.display().to_string())
                .replace("{manifest_path}", manifest_path)
        })
        .collect()
}

pub(super) struct ToolTraceRead {
    pub(super) rows: Vec<JsonValue>,
    pub(super) errors: Vec<String>,
}

pub(super) fn read_provider_tool_trace(stdout: &str) -> ToolTraceRead {
    let mut trace = ProviderTraceState::default();
    let mut errors = Vec::new();
    for (index, line) in stdout.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let event = match serde_json::from_str::<JsonValue>(trimmed) {
            Ok(event) => event,
            Err(error) => {
                if trimmed.starts_with('{') {
                    errors.push(format!(
                        "failed to parse provider stdout JSON line {}: {error}",
                        index + 1
                    ));
                }
                continue;
            }
        };
        trace.ingest_event(&event);
    }
    let mut rows = trace.rows;
    normalize_provider_tool_trace_indices(&mut rows);
    ToolTraceRead { rows, errors }
}

pub(super) fn read_provider_final_answer(stdout: &str) -> Option<String> {
    let mut assistant_parts = Vec::new();
    let mut result_parts = Vec::new();
    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || !trimmed.starts_with('{') {
            continue;
        }
        let Ok(event) = serde_json::from_str::<JsonValue>(trimmed) else {
            continue;
        };
        collect_provider_final_answer_event(&event, &mut assistant_parts, &mut result_parts);
    }
    join_provider_text_parts(&result_parts).or_else(|| join_provider_text_parts(&assistant_parts))
}

fn collect_provider_final_answer_event(
    event: &JsonValue,
    assistant_parts: &mut Vec<String>,
    result_parts: &mut Vec<String>,
) {
    if event.get("type").and_then(JsonValue::as_str) == Some("text")
        && let Some(part) = event.get("part")
    {
        collect_provider_text_content(part, assistant_parts);
        return;
    }
    if event.get("type").and_then(JsonValue::as_str) == Some("result") {
        collect_provider_text_content(event.get("result").unwrap_or(event), result_parts);
        return;
    }
    if event.get("type").and_then(JsonValue::as_str) == Some("assistant")
        && let Some(message) = event.get("message")
    {
        collect_provider_text_content(message, assistant_parts);
        return;
    }
    if provider_role(event) == Some("assistant") {
        collect_provider_text_content(event, assistant_parts);
        return;
    }
    if let Some(message) = event.get("message")
        && provider_role(message) == Some("assistant")
    {
        collect_provider_text_content(message, assistant_parts);
        return;
    }
    if let Some(item) = event.get("item")
        && provider_role(item) == Some("assistant")
    {
        collect_provider_text_content(item, assistant_parts);
        return;
    }
    if let Some("assistant_message" | "agent_message") =
        event.get("type").and_then(JsonValue::as_str)
    {
        collect_provider_text_content(event, assistant_parts);
    }
}

fn provider_role(value: &JsonValue) -> Option<&str> {
    value.get("role").and_then(JsonValue::as_str)
}

fn collect_provider_text_content(value: &JsonValue, out: &mut Vec<String>) {
    match value {
        JsonValue::Object(map) => {
            if matches!(
                map.get("type").and_then(JsonValue::as_str),
                Some("tool_use" | "tool_result" | "mcp_tool_call")
            ) {
                return;
            }
            for key in ["text", "output_text"] {
                if let Some(text) = map.get(key).and_then(JsonValue::as_str)
                    && !text.trim().is_empty()
                {
                    out.push(text.to_owned());
                }
            }
            for key in ["content", "parts", "message", "messages"] {
                if let Some(child) = map.get(key) {
                    collect_provider_text_content(child, out);
                }
            }
        }
        JsonValue::Array(items) => {
            for item in items {
                collect_provider_text_content(item, out);
            }
        }
        JsonValue::String(text) => {
            if !text.trim().is_empty() {
                out.push(text.clone());
            }
        }
        JsonValue::Null | JsonValue::Bool(_) | JsonValue::Number(_) => {}
    }
}

fn join_provider_text_parts(parts: &[String]) -> Option<String> {
    let joined = parts
        .iter()
        .map(|part| part.trim())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    (!joined.is_empty()).then_some(joined)
}

#[derive(Default)]
struct ProviderTraceState {
    rows: Vec<JsonValue>,
    claude_tool_row_by_id: BTreeMap<String, usize>,
}

impl ProviderTraceState {
    fn ingest_event(&mut self, event: &JsonValue) {
        if let Some(row) = codex_mcp_tool_row(event) {
            self.rows.push(row);
        }
        self.ingest_claude_event(event);
        if let Some(row) = opencode_mcp_tool_row(event) {
            self.rows.push(row);
        }
    }

    fn ingest_claude_event(&mut self, event: &JsonValue) {
        let Some(content) = claude_message_content(event) else {
            return;
        };
        for part in content {
            match part.get("type").and_then(JsonValue::as_str) {
                Some("tool_use") => self.ingest_claude_tool_use(part),
                Some("tool_result") => self.ingest_claude_tool_result(part),
                _ => {}
            }
        }
    }

    fn ingest_claude_tool_use(&mut self, part: &JsonValue) {
        let Some(name) = part.get("name").and_then(JsonValue::as_str) else {
            return;
        };
        let Some(tool) = normalize_provider_nova_tool_name(name) else {
            return;
        };
        let row_index = self.rows.len();
        self.rows.push(provider_tool_row(
            "provider_stdout_claude",
            &tool,
            true,
            part.get("input"),
            None,
        ));
        if let Some(id) = part.get("id").and_then(JsonValue::as_str)
            && !id.trim().is_empty()
        {
            self.claude_tool_row_by_id.insert(id.to_string(), row_index);
        }
    }

    fn ingest_claude_tool_result(&mut self, part: &JsonValue) {
        let Some(tool_use_id) = part.get("tool_use_id").and_then(JsonValue::as_str) else {
            return;
        };
        let Some(row_index) = self.claude_tool_row_by_id.get(tool_use_id).copied() else {
            return;
        };
        let success = !part
            .get("is_error")
            .and_then(JsonValue::as_bool)
            .unwrap_or(false);
        let response = part.get("content").unwrap_or(part);
        let telemetry = provider_response_telemetry(Some(response));
        let selected_unique_ids = provider_selected_unique_ids(Some(response));
        let top_unique_ids = provider_top_unique_ids(Some(response));
        if let Some(row) = self
            .rows
            .get_mut(row_index)
            .and_then(JsonValue::as_object_mut)
        {
            row.insert("success".to_string(), JsonValue::Bool(success));
            row.insert(
                "response_bytes".to_string(),
                JsonValue::from(telemetry.response_bytes),
            );
            row.insert(
                "response_truncated".to_string(),
                JsonValue::Bool(telemetry.response_truncated),
            );
            if let Some(result_count) = telemetry.result_count {
                row.insert("result_count".to_string(), JsonValue::from(result_count));
            }
            if let Some(total_available) = telemetry.total_available {
                row.insert(
                    "total_available".to_string(),
                    JsonValue::from(total_available),
                );
            }
            row.insert(
                "selected_unique_ids".to_string(),
                json!(selected_unique_ids),
            );
            row.insert("top_unique_ids".to_string(), json!(top_unique_ids));
        }
    }
}

fn codex_mcp_tool_row(event: &JsonValue) -> Option<JsonValue> {
    if event.get("type").and_then(JsonValue::as_str) != Some("item.completed") {
        return None;
    }
    let item = event.get("item")?;
    if item.get("type").and_then(JsonValue::as_str) != Some("mcp_tool_call") {
        return None;
    }
    if !is_nova_mcp_server_alias(item.get("server").and_then(JsonValue::as_str)) {
        return None;
    }
    let tool = item.get("tool").and_then(JsonValue::as_str)?;
    if !is_nova_tool_name(tool) {
        return None;
    }
    let success = item.get("error").is_none_or(JsonValue::is_null);
    Some(provider_tool_row(
        "provider_stdout_codex",
        tool,
        success,
        item.get("arguments"),
        item.get("result"),
    ))
}

fn claude_message_content(event: &JsonValue) -> Option<&Vec<JsonValue>> {
    event
        .get("message")
        .and_then(|message| message.get("content"))
        .and_then(JsonValue::as_array)
        .or_else(|| event.get("content").and_then(JsonValue::as_array))
}

fn opencode_mcp_tool_row(event: &JsonValue) -> Option<JsonValue> {
    if event.get("type").and_then(JsonValue::as_str) != Some("tool_use") {
        return None;
    }
    let part = event.get("part")?;
    let name = part.get("tool").and_then(JsonValue::as_str)?;
    let tool = normalize_provider_nova_tool_name(name)?;
    let state = part.get("state");
    let success = state
        .and_then(|state| state.get("status"))
        .and_then(JsonValue::as_str)
        .is_none_or(|status| status == "completed");
    Some(provider_tool_row(
        "provider_stdout_opencode",
        &tool,
        success,
        state.and_then(|state| state.get("input")),
        state.and_then(|state| state.get("output")),
    ))
}

fn normalize_provider_nova_tool_name(name: &str) -> Option<String> {
    let name = name.trim();
    let (server_alias, tool) = if let Some(suffix) = name.strip_prefix("mcp__") {
        suffix.rsplit_once("__")?
    } else if let Some(parts) = name.rsplit_once("__") {
        parts
    } else {
        name.rsplit_once('.')?
    };
    let tool = tool.trim();
    (is_nova_mcp_server_alias(Some(server_alias)) && is_nova_tool_name(tool))
        .then(|| tool.to_string())
}

fn is_nova_tool_name(name: &str) -> bool {
    MCP_TOOL_NAMES.contains(&name)
}

fn is_nova_mcp_server_alias(alias: Option<&str>) -> bool {
    let Some(alias) = alias else {
        return false;
    };
    let normalized = normalize_mcp_server_alias(alias);
    if normalized.is_empty() {
        return false;
    }
    DEFAULT_NOVA_MCP_SERVER_ALIASES
        .iter()
        .any(|default_alias| normalize_mcp_server_alias(default_alias) == normalized)
        || std::env::var(PROVIDER_MCP_SERVER_ALIASES_ENV)
            .ok()
            .is_some_and(|configured| {
                configured
                    .split([',', ';', ' '])
                    .any(|entry| normalize_mcp_server_alias(entry) == normalized)
            })
}

fn normalize_mcp_server_alias(alias: &str) -> String {
    alias
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|ch| ch.to_ascii_lowercase())
        .collect()
}

fn provider_tool_row(
    transport: &str,
    tool: &str,
    success: bool,
    params: Option<&JsonValue>,
    response: Option<&JsonValue>,
) -> JsonValue {
    let telemetry = provider_response_telemetry(response);
    json!({
        "transport": transport,
        "tool": tool,
        "success": success,
        "duration_ms": 0,
        "params_summary": provider_params_summary(params),
        "response_bytes": telemetry.response_bytes,
        "response_truncated": telemetry.response_truncated,
        "result_count": telemetry.result_count,
        "total_available": telemetry.total_available,
        "selected_unique_ids": provider_selected_unique_ids(response),
        "top_unique_ids": provider_top_unique_ids(response),
    })
}

fn normalize_provider_tool_trace_indices(rows: &mut [JsonValue]) {
    for (index, row) in rows.iter_mut().enumerate() {
        if let Some(obj) = row.as_object_mut() {
            obj.insert("tool_call_index".to_string(), JsonValue::from(index as u64));
        }
    }
}

struct ProviderResponseTelemetry {
    response_bytes: usize,
    response_truncated: bool,
    result_count: Option<u64>,
    total_available: Option<u64>,
}

fn provider_response_telemetry(response: Option<&JsonValue>) -> ProviderResponseTelemetry {
    let normalized = response.and_then(provider_measurement_response);
    let metric_source = normalized.as_ref().or(response);
    ProviderResponseTelemetry {
        response_bytes: response.map_or(0, provider_serialized_len),
        response_truncated: response.is_some_and(provider_response_truncated),
        result_count: metric_source.and_then(|value| provider_response_metric(value, "count")),
        total_available: metric_source.and_then(|value| {
            provider_response_metric(value, "total_available").or_else(|| {
                value
                    .get("_nova_result_meta")
                    .and_then(|meta| meta.get("original_count"))
                    .and_then(JsonValue::as_u64)
            })
        }),
    }
}

fn provider_serialized_len(value: &JsonValue) -> usize {
    if let Some(normalized) = provider_measurement_response(value) {
        return serde_json::to_string(&normalized).map_or(0, |serialized| serialized.len());
    }
    serde_json::to_string(value).map_or(0, |serialized| serialized.len())
}

fn provider_measurement_response(value: &JsonValue) -> Option<JsonValue> {
    match value {
        JsonValue::Object(map) => {
            if let Some(text) = map.get("text").and_then(JsonValue::as_str)
                && let Some(parsed) = parse_provider_json_text(text)
            {
                return Some(parsed);
            }
            if let Some(content) = map.get("content")
                && let Some(parsed) = provider_measurement_response(content)
            {
                return Some(parsed);
            }
            if let Some(result) = map.get("result")
                && let Some(parsed) = provider_measurement_response(result)
            {
                return Some(parsed);
            }
            None
        }
        JsonValue::Array(items) => items.iter().find_map(provider_measurement_response),
        JsonValue::String(raw) => parse_provider_json_text(raw),
        JsonValue::Null | JsonValue::Bool(_) | JsonValue::Number(_) => None,
    }
}

fn parse_provider_json_text(raw: &str) -> Option<JsonValue> {
    let trimmed = raw.trim();
    if !(trimmed.starts_with('{') || trimmed.starts_with('[')) {
        return None;
    }
    serde_json::from_str::<JsonValue>(trimmed).ok()
}

fn provider_response_metric(value: &JsonValue, key: &str) -> Option<u64> {
    match value {
        JsonValue::Object(map) => {
            if let Some(metric) = map.get(key).and_then(JsonValue::as_u64) {
                return Some(metric);
            }
            map.values()
                .find_map(|child| provider_response_metric(child, key))
        }
        JsonValue::Array(items) => items
            .iter()
            .find_map(|item| provider_response_metric(item, key)),
        JsonValue::String(raw) => {
            let trimmed = raw.trim();
            if trimmed.starts_with('{')
                && let Ok(parsed) = serde_json::from_str::<JsonValue>(trimmed)
            {
                return provider_response_metric(&parsed, key);
            }
            None
        }
        JsonValue::Null | JsonValue::Bool(_) | JsonValue::Number(_) => None,
    }
}

fn provider_response_truncated(value: &JsonValue) -> bool {
    match value {
        JsonValue::Object(map) => {
            map.get("truncated")
                .and_then(JsonValue::as_bool)
                .unwrap_or(false)
                || map
                    .get("_nova_result_meta")
                    .and_then(|meta| meta.get("truncated"))
                    .and_then(JsonValue::as_bool)
                    .unwrap_or(false)
                || map.values().any(provider_response_truncated)
        }
        JsonValue::Array(items) => items.iter().any(provider_response_truncated),
        JsonValue::String(raw) => {
            let trimmed = raw.trim();
            trimmed.starts_with('{')
                && serde_json::from_str::<JsonValue>(trimmed)
                    .is_ok_and(|parsed| provider_response_truncated(&parsed))
        }
        JsonValue::Null | JsonValue::Bool(_) | JsonValue::Number(_) => false,
    }
}

fn provider_params_summary(params: Option<&JsonValue>) -> JsonValue {
    let Some(JsonValue::Object(map)) = params else {
        return JsonValue::Null;
    };
    let keys: Vec<JsonValue> = map
        .keys()
        .take(50)
        .cloned()
        .map(JsonValue::String)
        .collect();
    let mut summary = serde_json::Map::from_iter([(String::from("keys"), JsonValue::Array(keys))]);
    for key in [
        "query",
        "persona",
        "id_or_name",
        "resource_type",
        "resource_types",
        "recipe_id",
        "direction",
        "detail",
        "group_mode",
        "indicator_types",
        "include_support_signals",
        "max_parent_groups",
        "preflight_only",
        "row_limit",
        "byte_limit",
        "max_poll_seconds",
        "limit",
        "offset",
    ] {
        if let Some(value) = map.get(key).and_then(provider_safe_value) {
            summary.insert(key.to_string(), value);
        }
    }
    JsonValue::Object(summary)
}

fn provider_safe_value(value: &JsonValue) -> Option<JsonValue> {
    match value {
        JsonValue::Null | JsonValue::Bool(_) | JsonValue::Number(_) => Some(value.clone()),
        JsonValue::String(value) => Some(JsonValue::String(value.chars().take(256).collect())),
        JsonValue::Array(items) => Some(JsonValue::Array(
            items
                .iter()
                .take(20)
                .filter_map(provider_safe_array_value)
                .collect(),
        )),
        JsonValue::Object(_) => None,
    }
}

fn provider_safe_array_value(value: &JsonValue) -> Option<JsonValue> {
    match value {
        JsonValue::Null | JsonValue::Bool(_) | JsonValue::Number(_) => Some(value.clone()),
        JsonValue::String(value) => Some(JsonValue::String(value.chars().take(256).collect())),
        JsonValue::Array(_) | JsonValue::Object(_) => None,
    }
}

fn provider_selected_unique_ids(response: Option<&JsonValue>) -> Vec<String> {
    let mut out = BTreeSet::new();
    if let Some(response) = response {
        collect_provider_unique_ids(response, &mut out);
    }
    out.into_iter().collect()
}

fn provider_top_unique_ids(response: Option<&JsonValue>) -> Vec<String> {
    let Some(response) = response else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    if let Some(rows) = response.get("data").and_then(JsonValue::as_array) {
        for row in rows {
            push_provider_first_unique_id(row, &mut out, &mut seen);
            if out.len() >= MAX_PROVIDER_TOP_UNIQUE_IDS {
                return out;
            }
        }
    }
    if out.is_empty() {
        collect_provider_top_unique_ids(response, &mut out, &mut seen);
    }
    out
}

fn push_provider_first_unique_id(
    value: &JsonValue,
    out: &mut Vec<String>,
    seen: &mut BTreeSet<String>,
) {
    if let Some(id) = provider_first_unique_id(value) {
        push_provider_ordered_unique_id(&id, out, seen);
    }
}

fn collect_provider_top_unique_ids(
    value: &JsonValue,
    out: &mut Vec<String>,
    seen: &mut BTreeSet<String>,
) {
    if out.len() >= MAX_PROVIDER_TOP_UNIQUE_IDS {
        return;
    }
    match value {
        JsonValue::Object(map) => {
            if let Some(id) = provider_first_unique_id(value) {
                push_provider_ordered_unique_id(&id, out, seen);
                if out.len() >= MAX_PROVIDER_TOP_UNIQUE_IDS {
                    return;
                }
            }
            for child in map.values() {
                collect_provider_top_unique_ids(child, out, seen);
                if out.len() >= MAX_PROVIDER_TOP_UNIQUE_IDS {
                    return;
                }
            }
        }
        JsonValue::Array(items) => {
            for item in items {
                collect_provider_top_unique_ids(item, out, seen);
                if out.len() >= MAX_PROVIDER_TOP_UNIQUE_IDS {
                    return;
                }
            }
        }
        JsonValue::String(raw) => {
            let trimmed = raw.trim();
            if trimmed.starts_with('{')
                && let Ok(parsed) = serde_json::from_str::<JsonValue>(trimmed)
            {
                collect_provider_top_unique_ids(&parsed, out, seen);
            }
        }
        JsonValue::Null | JsonValue::Bool(_) | JsonValue::Number(_) => {}
    }
}

fn provider_first_unique_id(value: &JsonValue) -> Option<String> {
    let JsonValue::Object(map) = value else {
        return None;
    };
    for key in ["unique_id", "parent_unique_id", "root_id"] {
        if let Some(id) = map.get(key).and_then(JsonValue::as_str)
            && !id.trim().is_empty()
        {
            return Some(id.chars().take(512).collect());
        }
    }
    None
}

fn push_provider_ordered_unique_id(id: &str, out: &mut Vec<String>, seen: &mut BTreeSet<String>) {
    if out.len() >= MAX_PROVIDER_TOP_UNIQUE_IDS {
        return;
    }
    if seen.insert(id.to_string()) {
        out.push(id.to_string());
    }
}

fn collect_provider_unique_ids(value: &JsonValue, out: &mut BTreeSet<String>) {
    if out.len() >= 200 {
        return;
    }
    match value {
        JsonValue::Object(map) => {
            for key in ["unique_id", "parent_unique_id", "root_id"] {
                if let Some(id) = map.get(key).and_then(JsonValue::as_str)
                    && !id.trim().is_empty()
                {
                    out.insert(id.chars().take(512).collect());
                }
            }
            for child in map.values() {
                collect_provider_unique_ids(child, out);
                if out.len() >= 200 {
                    return;
                }
            }
        }
        JsonValue::Array(items) => {
            for item in items {
                collect_provider_unique_ids(item, out);
                if out.len() >= 200 {
                    return;
                }
            }
        }
        JsonValue::String(raw) => {
            let trimmed = raw.trim();
            if trimmed.starts_with('{')
                && let Ok(parsed) = serde_json::from_str::<JsonValue>(trimmed)
            {
                collect_provider_unique_ids(&parsed, out);
            }
        }
        JsonValue::Null | JsonValue::Bool(_) | JsonValue::Number(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use serde_json::json;

    use super::{
        provider_invocation, provider_path_with_current_exe, read_provider_final_answer,
        read_provider_tool_trace,
    };
    use crate::cli::args::EvalAgentRunArgs;

    #[test]
    fn provider_invocation_rejects_unknown_provider_without_command() {
        let args = EvalAgentRunArgs {
            provider: "custom".to_string(),
            ..EvalAgentRunArgs::default()
        };
        let error = provider_invocation(&args, "prompt", Path::new("trace.jsonl"))
            .expect_err("unknown provider should fail");
        assert!(error.to_string().contains("unsupported provider"));
    }

    #[test]
    fn provider_invocation_rejects_empty_custom_command() {
        let args = EvalAgentRunArgs {
            provider: "custom".to_string(),
            provider_command: Some(" ".to_string()),
            ..EvalAgentRunArgs::default()
        };
        let error = provider_invocation(&args, "prompt", Path::new("trace.jsonl"))
            .expect_err("empty provider command should fail");
        assert!(error.to_string().contains("cannot be empty"));
    }

    #[test]
    fn provider_invocation_substitutes_custom_args() {
        let args = EvalAgentRunArgs {
            provider: "custom".to_string(),
            provider_command: Some("runner".to_string()),
            provider_args_json: Some(
                r#"["--prompt","{prompt}","--trace","{trace_path}"]"#.to_string(),
            ),
            ..EvalAgentRunArgs::default()
        };
        let invocation = provider_invocation(&args, "hello", Path::new("/tmp/trace.jsonl"))
            .expect("custom provider");
        assert_eq!(invocation.command, "runner");
        assert_eq!(
            invocation.args,
            vec!["--prompt", "hello", "--trace", "/tmp/trace.jsonl"]
        );
    }

    #[test]
    fn provider_invocation_uses_opencode_without_dir_flag() {
        let args = EvalAgentRunArgs {
            provider: "opencode".to_string(),
            ..EvalAgentRunArgs::default()
        };
        let invocation = provider_invocation(&args, "hello", Path::new("/tmp/trace.jsonl"))
            .expect("opencode provider");
        assert_eq!(invocation.command, "opencode");
        assert_eq!(invocation.args, vec!["run", "--format", "json", "hello"]);
    }

    #[test]
    fn provider_invocation_inserts_opencode_model_before_prompt() {
        let args = EvalAgentRunArgs {
            provider: "opencode".to_string(),
            provider_model: Some("opencode/deepseek-v4-flash-free".to_string()),
            ..EvalAgentRunArgs::default()
        };
        let invocation = provider_invocation(&args, "hello", Path::new("/tmp/trace.jsonl"))
            .expect("opencode provider");
        assert_eq!(invocation.command, "opencode");
        assert_eq!(
            invocation.args,
            vec![
                "run",
                "--format",
                "json",
                "--model",
                "opencode/deepseek-v4-flash-free",
                "hello"
            ]
        );
    }

    #[test]
    fn provider_path_prefers_current_executable_directory() {
        let path = provider_path_with_current_exe(Path::new("/tmp/dbt-nova/bin/dbt-nova"))
            .expect("path")
            .expect("has parent");
        let first = std::env::split_paths(&path).next();
        assert_eq!(first, Some(PathBuf::from("/tmp/dbt-nova/bin")));
    }

    #[test]
    fn provider_invocation_uses_claude_verbose_stream_json() {
        let args = EvalAgentRunArgs {
            provider: "claude".to_string(),
            ..EvalAgentRunArgs::default()
        };
        let invocation = provider_invocation(&args, "hello", Path::new("/tmp/trace.jsonl"))
            .expect("claude provider");
        assert_eq!(invocation.command, "claude");
        assert_eq!(
            invocation.args,
            vec!["-p", "--verbose", "--output-format", "stream-json", "hello"]
        );
    }

    #[test]
    fn provider_invocation_uses_goose_stream_json() {
        let args = EvalAgentRunArgs {
            provider: "goose".to_string(),
            ..EvalAgentRunArgs::default()
        };
        let invocation = provider_invocation(&args, "hello", Path::new("/tmp/trace.jsonl"))
            .expect("goose provider");
        assert_eq!(invocation.command, "goose");
        assert_eq!(
            invocation.args,
            vec![
                "run",
                "--text",
                "hello",
                "--output-format",
                "stream-json",
                "--no-session"
            ]
        );
    }

    #[test]
    fn provider_trace_reads_codex_mcp_tool_events() {
        let stdout = r#"{"type":"item.completed","item":{"type":"mcp_tool_call","server":"nova","tool":"search_indicator","arguments":{"query":"gmv"},"result":{"content":[{"type":"text","text":"{\"data\":[{\"parent_unique_id\":\"model.pkg.orders\"}]}"}]},"error":null,"status":"completed"}}"#;
        let trace = read_provider_tool_trace(stdout);
        assert_eq!(trace.errors, Vec::<String>::new());
        assert_eq!(trace.rows.len(), 1);
        assert_eq!(trace.rows[0]["tool"], "search_indicator");
        assert_eq!(trace.rows[0]["params_summary"]["query"], "gmv");
        assert_eq!(
            trace.rows[0]["selected_unique_ids"],
            json!(["model.pkg.orders"])
        );
        assert_eq!(trace.rows[0]["top_unique_ids"], json!(["model.pkg.orders"]));
        assert_eq!(
            trace.rows[0]["response_bytes"],
            json!(
                serde_json::to_string(&json!({"data":[{"parent_unique_id":"model.pkg.orders"}]}))
                    .expect("serialized")
                    .len()
            )
        );
    }

    #[test]
    fn provider_trace_reads_codex_events_with_custom_server_alias() {
        let stdout = r#"{"type":"item.completed","item":{"type":"mcp_tool_call","server":"dbt-nova","tool":"get_context","arguments":{"id_or_name":"model.pkg.orders"},"result":{"content":[{"type":"text","text":"{}"}]},"error":null,"status":"completed"}}"#;
        let trace = read_provider_tool_trace(stdout);
        assert_eq!(trace.errors, Vec::<String>::new());
        assert_eq!(trace.rows.len(), 1);
        assert_eq!(trace.rows[0]["tool"], "get_context");
        assert_eq!(
            trace.rows[0]["params_summary"]["id_or_name"],
            "model.pkg.orders"
        );
    }

    #[test]
    fn provider_trace_rejects_codex_events_from_other_servers() {
        let stdout = r#"{"type":"item.completed","item":{"type":"mcp_tool_call","server":"other-server","tool":"search","arguments":{"query":"gmv"},"result":{"content":[{"type":"text","text":"{\"data\":[{\"unique_id\":\"model.pkg.orders\"}]}"}]},"error":null,"status":"completed"}}"#;
        let trace = read_provider_tool_trace(stdout);
        assert_eq!(trace.errors, Vec::<String>::new());
        assert!(trace.rows.is_empty());
    }

    #[test]
    fn provider_trace_reads_claude_mcp_tool_use_events() {
        let stdout = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"mcp__nova__search_indicator","input":{"query":"gmv"}},{"type":"tool_use","name":"mcp__nova__get_context","input":{"id_or_name":"model.pkg.orders"}}]}}"#;
        let trace = read_provider_tool_trace(stdout);
        assert_eq!(trace.errors, Vec::<String>::new());
        assert_eq!(trace.rows.len(), 2);
        assert_eq!(trace.rows[0]["tool_call_index"], json!(0));
        assert_eq!(trace.rows[1]["tool_call_index"], json!(1));
        assert_eq!(trace.rows[0]["tool"], "search_indicator");
        assert_eq!(trace.rows[1]["tool"], "get_context");
        assert_eq!(
            trace.rows[1]["params_summary"]["id_or_name"],
            "model.pkg.orders"
        );
    }

    #[test]
    fn provider_trace_attaches_claude_tool_results() {
        let stdout = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_1","name":"mcp__dbt_nova__search_indicator","input":{"query":"gmv"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_1","content":[{"type":"text","text":"{\"count\":1,\"total_available\":7,\"data\":[{\"parent_unique_id\":\"model.pkg.orders\"}],\"_nova_result_meta\":{\"truncated\":true}}"}]}]}}"#;
        let trace = read_provider_tool_trace(stdout);
        assert_eq!(trace.errors, Vec::<String>::new());
        assert_eq!(trace.rows.len(), 1);
        assert_eq!(trace.rows[0]["tool"], "search_indicator");
        assert!(trace.rows[0]["response_bytes"].as_u64().unwrap_or(0) > 0);
        assert_eq!(trace.rows[0]["response_truncated"], json!(true));
        assert_eq!(trace.rows[0]["result_count"], json!(1));
        assert_eq!(trace.rows[0]["total_available"], json!(7));
        assert_eq!(
            trace.rows[0]["selected_unique_ids"],
            json!(["model.pkg.orders"])
        );
        assert_eq!(trace.rows[0]["top_unique_ids"], json!(["model.pkg.orders"]));
    }

    #[test]
    fn provider_trace_keeps_safe_array_params() {
        let stdout = r#"{"type":"item.completed","item":{"type":"mcp_tool_call","server":"nova","tool":"search","arguments":{"query":"orders","resource_types":["model",{"secret":"x"}],"limit":5},"result":{"data":[]},"error":null,"status":"completed"}}"#;
        let trace = read_provider_tool_trace(stdout);
        assert_eq!(trace.errors, Vec::<String>::new());
        assert_eq!(trace.rows.len(), 1);
        assert_eq!(
            trace.rows[0]["params_summary"]["resource_types"],
            json!(["model"])
        );
        assert_eq!(trace.rows[0]["params_summary"]["limit"], 5);
    }

    #[test]
    fn provider_trace_normalizes_custom_mcp_aliases() {
        let stdout = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"mcp__dbt_nova__search_indicator","input":{"query":"gmv"}},{"type":"tool_use","name":"dbt-nova.get_context","input":{"id_or_name":"model.pkg.orders"}},{"type":"tool_use","name":"other_server.search","input":{"query":"gmv"}},{"type":"tool_use","name":"other_server.not_a_nova_tool","input":{}}]}}"#;
        let trace = read_provider_tool_trace(stdout);
        assert_eq!(trace.errors, Vec::<String>::new());
        assert_eq!(trace.rows.len(), 2);
        assert_eq!(trace.rows[0]["tool"], "search_indicator");
        assert_eq!(trace.rows[1]["tool"], "get_context");
    }

    #[test]
    fn provider_final_answer_reads_claude_result_event() {
        let stdout = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_1","name":"mcp__nova__search","input":{"query":"prompt-only term"}}]}}
{"type":"result","subtype":"success","result":"Final answer uses model.pkg.orders only."}"#;
        let final_answer = read_provider_final_answer(stdout).expect("final answer");
        assert_eq!(final_answer, "Final answer uses model.pkg.orders only.");
        assert!(!final_answer.contains("prompt-only term"));
    }

    #[test]
    fn provider_final_answer_reads_codex_completed_assistant_message() {
        let stdout = r#"{"type":"item.completed","item":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Use gross merchandise value from model.pkg.orders."}]}}"#;
        let final_answer = read_provider_final_answer(stdout).expect("final answer");
        assert_eq!(
            final_answer,
            "Use gross merchandise value from model.pkg.orders."
        );
    }

    #[test]
    fn provider_final_answer_ignores_tool_payload_text() {
        let stdout = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_1","name":"mcp__nova__search","input":{"query":"must not leak"}}]}}
{"type":"assistant","message":{"content":[{"type":"text","text":"Clean final answer."}]}}"#;
        let final_answer = read_provider_final_answer(stdout).expect("final answer");
        assert_eq!(final_answer, "Clean final answer.");
    }

    #[test]
    fn provider_final_answer_reads_opencode_text_events() {
        let stdout = r#"{"type":"tool_use","part":{"type":"tool","tool":"bash","state":{"output":"tool output with 12.0%"}}}
{"type":"text","part":{"type":"text","text":"Final answer only."}}"#;
        let final_answer = read_provider_final_answer(stdout).expect("final answer");
        assert_eq!(final_answer, "Final answer only.");
    }
}
