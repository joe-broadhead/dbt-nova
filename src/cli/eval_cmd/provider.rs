use std::collections::BTreeSet;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use serde_json::{Value as JsonValue, json};
use tokio::process::Command;

use crate::cli::args::EvalAgentRunArgs;
use crate::error::DbtNovaError;
use crate::tools::catalog::MCP_TOOL_NAMES;
use crate::utils::tool_trace::TRACE_ENV;

use super::server_error;

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
    let mut command = Command::new(&invocation.command);
    command
        .args(&invocation.args)
        .current_dir(std::env::current_dir().map_err(|error| server_error(error.to_string()))?)
        .env(TRACE_ENV, trace_path)
        .env("DBT_NOVA_EVAL_CASE_DIR", case_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
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

pub(super) fn provider_invocation(
    args: &EvalAgentRunArgs,
    prompt: &str,
    trace_path: &Path,
) -> crate::error::Result<ProviderInvocation> {
    let command = args.provider_command.clone().map_or_else(
        || default_provider_command(&args.provider),
        validate_custom_command,
    )?;
    let raw_args = if let Some(json) = args.provider_args_json.as_ref() {
        let values: Vec<String> = serde_json::from_str(json).map_err(|error| {
            DbtNovaError::InvalidParams(format!("invalid --provider-args-json: {error}"))
        })?;
        substitute_provider_args(values, prompt, trace_path, args)
    } else {
        default_provider_args(&args.provider, prompt)?
    };
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
            "--dir".to_string(),
            std::env::current_dir()
                .map_err(|error| server_error(error.to_string()))?
                .display()
                .to_string(),
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
    let mut rows = Vec::new();
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
        rows.extend(provider_event_tool_rows(&event));
    }
    ToolTraceRead { rows, errors }
}

fn provider_event_tool_rows(event: &JsonValue) -> Vec<JsonValue> {
    let mut rows = Vec::new();
    if let Some(row) = codex_mcp_tool_row(event) {
        rows.push(row);
    }
    rows.extend(claude_tool_use_rows(event));
    if let Some(row) = opencode_mcp_tool_row(event) {
        rows.push(row);
    }
    rows
}

fn codex_mcp_tool_row(event: &JsonValue) -> Option<JsonValue> {
    if event.get("type").and_then(JsonValue::as_str) != Some("item.completed") {
        return None;
    }
    let item = event.get("item")?;
    if item.get("type").and_then(JsonValue::as_str) != Some("mcp_tool_call") {
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

fn claude_tool_use_rows(event: &JsonValue) -> Vec<JsonValue> {
    let mut rows = Vec::new();
    let content = event
        .get("message")
        .and_then(|message| message.get("content"))
        .and_then(JsonValue::as_array);
    let Some(content) = content else {
        return rows;
    };
    for part in content {
        if part.get("type").and_then(JsonValue::as_str) != Some("tool_use") {
            continue;
        }
        let Some(name) = part.get("name").and_then(JsonValue::as_str) else {
            continue;
        };
        if let Some(tool) = normalize_provider_nova_tool_name(name) {
            rows.push(provider_tool_row(
                "provider_stdout_claude",
                &tool,
                true,
                part.get("input"),
                None,
            ));
        }
    }
    rows
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
    let candidate = name
        .strip_prefix("mcp__")
        .and_then(|suffix| suffix.rsplit_once("__").map(|(_, tool)| tool))
        .or_else(|| name.rsplit_once("__").map(|(_, tool)| tool))
        .or_else(|| name.rsplit_once('.').map(|(_, tool)| tool))
        .unwrap_or(name)
        .trim();
    is_nova_tool_name(candidate).then(|| candidate.to_string())
}

fn is_nova_tool_name(name: &str) -> bool {
    MCP_TOOL_NAMES.contains(&name)
}

fn provider_tool_row(
    transport: &str,
    tool: &str,
    success: bool,
    params: Option<&JsonValue>,
    response: Option<&JsonValue>,
) -> JsonValue {
    json!({
        "transport": transport,
        "tool": tool,
        "success": success,
        "duration_ms": 0,
        "params_summary": provider_params_summary(params),
        "selected_unique_ids": provider_selected_unique_ids(response),
    })
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
    for key in ["query", "persona", "id_or_name", "recipe_id", "direction"] {
        if let Some(value) = map.get(key).and_then(provider_safe_scalar) {
            summary.insert(key.to_string(), value);
        }
    }
    JsonValue::Object(summary)
}

fn provider_safe_scalar(value: &JsonValue) -> Option<JsonValue> {
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
    use std::path::Path;

    use serde_json::json;

    use super::{provider_invocation, read_provider_tool_trace};
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
    fn provider_trace_reads_claude_mcp_tool_use_events() {
        let stdout = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"mcp__nova__search_indicator","input":{"query":"gmv"}},{"type":"tool_use","name":"mcp__nova__get_context","input":{"id_or_name":"model.pkg.orders"}}]}}"#;
        let trace = read_provider_tool_trace(stdout);
        assert_eq!(trace.errors, Vec::<String>::new());
        assert_eq!(trace.rows.len(), 2);
        assert_eq!(trace.rows[0]["tool"], "search_indicator");
        assert_eq!(trace.rows[1]["tool"], "get_context");
        assert_eq!(
            trace.rows[1]["params_summary"]["id_or_name"],
            "model.pkg.orders"
        );
    }

    #[test]
    fn provider_trace_normalizes_custom_mcp_aliases() {
        let stdout = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"mcp__dbt_nova__search_indicator","input":{"query":"gmv"}},{"type":"tool_use","name":"dbt-nova.get_context","input":{"id_or_name":"model.pkg.orders"}},{"type":"tool_use","name":"other_server.not_a_nova_tool","input":{}}]}}"#;
        let trace = read_provider_tool_trace(stdout);
        assert_eq!(trace.errors, Vec::<String>::new());
        assert_eq!(trace.rows.len(), 2);
        assert_eq!(trace.rows[0]["tool"], "search_indicator");
        assert_eq!(trace.rows[1]["tool"], "get_context");
    }
}
