use std::fmt::Write as _;
use std::fs::{self, create_dir_all};
use std::path::{Component, Path, PathBuf};
use std::time::Instant;

use serde::Serialize;
use serde_json::{Map as JsonMap, Value as JsonValue};

use crate::cli::args::{
    ManifestLoadArgs, TraceInspectArgs, TraceRedactArgs, TraceReplayArgs, TraceSummarizeArgs,
};
use crate::cli::manifest::{build_manifest_load_config, execute_manifest_load};
use crate::cli::output::{CliEnvelope, error_envelope};
use crate::cli::tool::dispatch_tool;
use crate::error::{DbtNovaError, Result as DbtNovaResult};
use crate::manifest::search::ManifestSearch;
use crate::params::{
    TraceInspectParams, TraceRedactParams, TraceReplayParams, TraceSummarizeParams,
};
use crate::responses::SuccessResponse;
use crate::utils::tool_trace::{
    ToolTraceParseWarning, ToolTraceRead, ToolTraceRedactionReport, ToolTraceSummary,
    extract_response_truncated, extract_top_unique_ids, extract_unique_ids, read_tool_trace_file,
    redact_tool_trace_file, summarize_tool_trace,
};

use super::{DispatchError, DispatchResult};

pub const MCP_ENABLE_TRACE_WRITES_ENV: &str = "DBT_NOVA_MCP_ENABLE_TRACE_WRITES";

#[derive(Debug, Serialize)]
struct TraceInspectData {
    schema_version: &'static str,
    path: String,
    rows: Vec<serde_json::Value>,
    parse_warnings: Vec<ToolTraceParseWarning>,
    summary: ToolTraceSummary,
}

#[derive(Debug, Serialize)]
struct TraceSummarizeData {
    schema_version: &'static str,
    path: String,
    report_md_path: Option<String>,
    parse_warnings: Vec<ToolTraceParseWarning>,
    summary: ToolTraceSummary,
}

#[derive(Debug, Serialize)]
struct TraceReplayData {
    schema_version: &'static str,
    path: String,
    manifest: TraceReplayManifest,
    row_count: usize,
    parse_warnings: Vec<ToolTraceParseWarning>,
    supported_tools: Vec<&'static str>,
    counts: TraceReplayCounts,
    results: Vec<TraceReplayRow>,
}

#[derive(Debug, Serialize)]
struct TraceReplayManifest {
    manifest_hash: String,
    manifest_version: String,
    source: String,
    entity_count: usize,
}

#[derive(Debug, Default, Serialize)]
struct TraceReplayCounts {
    replayed: usize,
    changed: usize,
    skipped: usize,
    failed: usize,
    unsupported: usize,
}

#[derive(Debug, Serialize)]
struct TraceReplayRow {
    tool_call_index: u64,
    tool: String,
    status: &'static str,
    reason: Option<String>,
    replay_duration_ms: Option<u64>,
    original: TraceReplayShape,
    replayed: Option<TraceReplayShape>,
    changes: Vec<TraceReplayChange>,
}

#[derive(Debug, Default, Serialize)]
struct TraceReplayShape {
    success: Option<bool>,
    count: Option<u64>,
    total_available: Option<u64>,
    response_truncated: bool,
    error_code: Option<String>,
    selected_unique_ids: Vec<String>,
    top_unique_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
struct TraceReplayChange {
    field: &'static str,
    before: JsonValue,
    after: JsonValue,
}

#[derive(Debug, Serialize)]
struct TraceMcpSafetyPolicy {
    filesystem_root: String,
    trace_writes_enabled_env: &'static str,
    local_paths_must_stay_under_filesystem_root: bool,
    write_operations_require_opt_in: bool,
}

const TRACE_REPLAY_SCHEMA_VERSION: &str = "trace_replay.v1";
const TRACE_REPLAY_SUPPORTED_TOOLS: [&str; 6] = [
    "search",
    "search_indicator",
    "search_columns",
    "get_entity",
    "get_context",
    "get_lineage",
];

/// Runs the `trace inspect` CLI command.
///
/// # Errors
/// Returns an error when the trace cannot be read or output cannot be rendered.
pub fn run_inspect_command(args: &TraceInspectArgs) -> DispatchResult {
    let started = Instant::now();
    let read = read_trace_path(Path::new(&args.path)).map_err(|error| {
        render_or_propagate_error(
            args.json,
            "trace inspect",
            error,
            started.elapsed().as_millis(),
        )
    })?;
    let summary = summarize_tool_trace(&read);
    let payload = TraceInspectData {
        schema_version: "trace_inspect.v1",
        path: read.path,
        rows: read.rows,
        parse_warnings: read.parse_warnings,
        summary,
    };
    print_or_json(
        "trace inspect",
        args.json,
        &payload,
        started.elapsed().as_millis(),
    )
}

/// Runs the `trace summarize` CLI command.
///
/// # Errors
/// Returns an error when the trace cannot be read, Markdown cannot be written,
/// or output cannot be rendered.
pub fn run_summarize_command(args: &TraceSummarizeArgs) -> DispatchResult {
    let started = Instant::now();
    let read = read_trace_path(Path::new(&args.path)).map_err(|error| {
        render_or_propagate_error(
            args.json,
            "trace summarize",
            error,
            started.elapsed().as_millis(),
        )
    })?;
    let summary = summarize_tool_trace(&read);
    let markdown = render_markdown_summary(&read, &summary);
    if let Some(path) = args.report_md_path.as_deref() {
        write_text_path(Path::new(path), &markdown).map_err(|error| {
            render_or_propagate_error(
                args.json,
                "trace summarize",
                error,
                started.elapsed().as_millis(),
            )
        })?;
    }
    let payload = TraceSummarizeData {
        schema_version: "trace_summarize.v1",
        path: read.path,
        report_md_path: args.report_md_path.clone(),
        parse_warnings: read.parse_warnings,
        summary,
    };

    if args.json {
        print_json_envelope("trace summarize", &payload, started.elapsed().as_millis())
    } else if args.report_md_path.is_some() {
        println!("trace summary written");
        if let Some(path) = args.report_md_path.as_deref() {
            println!("  report_md_path: {path}");
        }
        println!("  row_count: {}", payload.summary.row_count);
        println!("  parse_warnings: {}", payload.summary.parse_warning_count);
        Ok(())
    } else {
        println!("{markdown}");
        Ok(())
    }
}

/// Runs the `trace redact` CLI command.
///
/// # Errors
/// Returns an error when redaction fails or output cannot be rendered.
pub fn run_redact_command(args: &TraceRedactArgs) -> DispatchResult {
    let started = Instant::now();
    let report =
        redact_tool_trace_file(Path::new(&args.path), Path::new(&args.out)).map_err(|error| {
            render_or_propagate_error(
                args.json,
                "trace redact",
                error,
                started.elapsed().as_millis(),
            )
        })?;

    if args.json {
        print_json_envelope("trace redact", &report, started.elapsed().as_millis())
    } else {
        print_redaction_human(&report);
        Ok(())
    }
}

/// Runs the `trace replay` CLI command.
///
/// # Errors
/// Returns an error when the trace or manifest cannot be loaded, replay fails
/// unexpectedly, or output cannot be rendered.
pub async fn run_replay_command(args: &TraceReplayArgs) -> DispatchResult {
    let started = Instant::now();
    let read = read_trace_path(Path::new(&args.path)).map_err(|error| {
        render_or_propagate_error(
            args.json,
            "trace replay",
            error,
            started.elapsed().as_millis(),
        )
    })?;
    let config = build_manifest_load_config(&ManifestLoadArgs {
        manifest_path: args.manifest_path.clone(),
        manifest_uri: args.manifest_uri.clone(),
        storage_instance_id: args.storage_instance_id.clone(),
        cleanup_storage_on_start: args.cleanup_storage_on_start,
        read_only: args.read_only,
        json: false,
    })
    .map_err(|error| {
        render_or_propagate_error(
            args.json,
            "trace replay",
            error,
            started.elapsed().as_millis(),
        )
    })?;
    let load = execute_manifest_load(config).await.map_err(|error| {
        render_or_propagate_error(
            args.json,
            "trace replay",
            error,
            started.elapsed().as_millis(),
        )
    })?;
    let payload = replay_trace_read(&load.search, read)
        .await
        .map_err(|error| {
            render_or_propagate_error(
                args.json,
                "trace replay",
                error,
                started.elapsed().as_millis(),
            )
        })?;

    if args.json {
        print_json_envelope("trace replay", &payload, started.elapsed().as_millis())
    } else {
        print_replay_human(&payload);
        Ok(())
    }
}

/// Builds the MCP/CLI-tool response for trace inspection.
///
/// # Errors
/// Returns an error when the trace path is unsafe, missing, or unreadable.
pub fn build_trace_inspect_tool_response(params: &TraceInspectParams) -> DbtNovaResult<JsonValue> {
    let path = resolve_mcp_trace_existing_file(&params.path, "path")?;
    let read = read_trace_path(&path)?;
    let summary = summarize_tool_trace(&read);
    let count = summary.row_count;
    let payload = with_trace_safety_policy(serde_json::to_value(TraceInspectData {
        schema_version: "trace_inspect.v1",
        path: read.path,
        rows: read.rows,
        parse_warnings: read.parse_warnings,
        summary,
    })?)?;
    success_value(payload, count)
}

/// Builds the MCP/CLI-tool response for trace summarization.
///
/// # Errors
/// Returns an error when paths are unsafe, the trace is unreadable, or a report
/// write is requested without explicit trace write opt-in.
pub fn build_trace_summarize_tool_response(
    params: &TraceSummarizeParams,
) -> DbtNovaResult<JsonValue> {
    let path = resolve_mcp_trace_existing_file(&params.path, "path")?;
    let read = read_trace_path(&path)?;
    let summary = summarize_tool_trace(&read);
    let count = summary.row_count;
    let markdown = render_markdown_summary(&read, &summary);
    let report_md_path = params
        .report_md_path
        .as_deref()
        .map(|path| {
            require_mcp_trace_writes("summarize_tool_trace")?;
            resolve_mcp_trace_writable_path(path, "report_md_path")
        })
        .transpose()?;
    if let Some(path) = report_md_path.as_ref() {
        write_text_path(path, &markdown)?;
    }
    let payload = with_trace_safety_policy(serde_json::to_value(TraceSummarizeData {
        schema_version: "trace_summarize.v1",
        path: read.path,
        report_md_path: report_md_path.map(|path| path.display().to_string()),
        parse_warnings: read.parse_warnings,
        summary,
    })?)?;
    success_value(payload, count)
}

/// Builds the MCP/CLI-tool response for trace redaction.
///
/// # Errors
/// Returns an error when trace writes are disabled, paths are unsafe, or
/// redaction cannot read/write the requested files.
pub fn build_trace_redact_tool_response(params: &TraceRedactParams) -> DbtNovaResult<JsonValue> {
    require_mcp_trace_writes("redact_tool_trace")?;
    let path = resolve_mcp_trace_existing_file(&params.path, "path")?;
    let out = resolve_mcp_trace_writable_path(&params.out, "out")?;
    let report = redact_tool_trace_file(&path, &out)?;
    let count = report.rows_written;
    let payload = with_trace_safety_policy(serde_json::to_value(report)?)?;
    success_value(payload, count)
}

/// Builds the MCP/CLI-tool response for trace replay.
///
/// # Errors
/// Returns an error when the trace path is unsafe, missing, unreadable, or the
/// replay report cannot be serialized.
pub async fn build_trace_replay_tool_response(
    searcher: &ManifestSearch,
    params: &TraceReplayParams,
) -> DbtNovaResult<JsonValue> {
    let path = resolve_mcp_trace_existing_file(&params.path, "path")?;
    let read = read_trace_path(&path)?;
    let count = read.rows.len();
    let report = replay_trace_read(searcher, read).await?;
    let payload = with_trace_safety_policy(serde_json::to_value(report)?)?;
    success_value(payload, count)
}

async fn replay_trace_read(
    searcher: &ManifestSearch,
    read: ToolTraceRead,
) -> DbtNovaResult<TraceReplayData> {
    let mut counts = TraceReplayCounts::default();
    let mut results = Vec::with_capacity(read.rows.len());
    for (index, row) in read.rows.iter().enumerate() {
        let result = replay_trace_row(searcher, row, index).await;
        counts.ingest(result.status);
        results.push(result);
    }

    Ok(TraceReplayData {
        schema_version: TRACE_REPLAY_SCHEMA_VERSION,
        path: read.path,
        manifest: TraceReplayManifest {
            manifest_hash: searcher.manifest_hash.clone(),
            manifest_version: searcher.manifest_version.clone(),
            source: crate::utils::sanitize_uri(&searcher.manifest_source_uri),
            entity_count: searcher.entity_count(),
        },
        row_count: results.len(),
        parse_warnings: read.parse_warnings,
        supported_tools: TRACE_REPLAY_SUPPORTED_TOOLS.to_vec(),
        counts,
        results,
    })
}

async fn replay_trace_row(
    searcher: &ManifestSearch,
    row: &JsonValue,
    index: usize,
) -> TraceReplayRow {
    let tool = trace_row_tool(row);
    let original = trace_shape_from_row(row);
    let tool_call_index = trace_row_index(row, index);
    if tool != "execute_sql" && !TRACE_REPLAY_SUPPORTED_TOOLS.contains(&tool.as_str()) {
        return TraceReplayRow {
            tool_call_index,
            tool: tool.clone(),
            status: "unsupported",
            reason: Some(format!("tool '{tool}' is not supported by trace replay")),
            replay_duration_ms: None,
            original,
            replayed: None,
            changes: Vec::new(),
        };
    }
    let Some(params) = replay_params_for_row(row, &tool) else {
        return TraceReplayRow {
            tool_call_index,
            tool,
            status: "skipped",
            reason: Some(replay_skip_reason(row, tool_call_index)),
            replay_duration_ms: None,
            original,
            replayed: None,
            changes: Vec::new(),
        };
    };

    let started = Instant::now();
    match dispatch_tool(searcher, &tool, params).await {
        Ok(response) => {
            let replayed = trace_shape_from_response(&response);
            let changes = compare_trace_shapes(&original, &replayed);
            let status = if changes.is_empty() {
                "replayed"
            } else {
                "changed"
            };
            TraceReplayRow {
                tool_call_index,
                tool,
                status,
                reason: None,
                replay_duration_ms: Some(elapsed_ms_to_u64(started)),
                original,
                replayed: Some(replayed),
                changes,
            }
        }
        Err(error) => TraceReplayRow {
            tool_call_index,
            tool,
            status: "failed",
            reason: Some(error.to_string()),
            replay_duration_ms: Some(elapsed_ms_to_u64(started)),
            original,
            replayed: Some(TraceReplayShape {
                success: Some(false),
                error_code: Some(error.error_code().to_string()),
                ..TraceReplayShape::default()
            }),
            changes: Vec::new(),
        },
    }
}

impl TraceReplayCounts {
    fn ingest(&mut self, status: &str) {
        match status {
            "replayed" => self.replayed += 1,
            "changed" => self.changed += 1,
            "skipped" => self.skipped += 1,
            "failed" => self.failed += 1,
            "unsupported" => self.unsupported += 1,
            _ => {}
        }
    }
}

fn replay_params_for_row(row: &JsonValue, tool: &str) -> Option<JsonValue> {
    if tool == "execute_sql" {
        return None;
    }
    if !TRACE_REPLAY_SUPPORTED_TOOLS.contains(&tool) {
        return None;
    }
    let params = row.get("params_summary")?.as_object()?;
    if params_summary_has_sensitive_signal(params) {
        return None;
    }
    build_replay_params(tool, params)
}

fn replay_skip_reason(row: &JsonValue, index: u64) -> String {
    let tool = trace_row_tool(row);
    if tool == "execute_sql" {
        return "execute_sql is skipped by default because Nova traces do not store raw SQL"
            .to_string();
    }
    if !TRACE_REPLAY_SUPPORTED_TOOLS.contains(&tool.as_str()) {
        return format!("tool '{tool}' is not supported by trace replay");
    }
    let Some(params) = row.get("params_summary").and_then(JsonValue::as_object) else {
        return format!("row {index} does not include params_summary");
    };
    if params_summary_has_sensitive_signal(params) {
        return "params_summary contains sensitive or unsafe parameter evidence".to_string();
    }
    "params_summary is missing required safe scalar parameters for replay".to_string()
}

fn build_replay_params(tool: &str, params: &JsonMap<String, JsonValue>) -> Option<JsonValue> {
    let mut out = JsonMap::new();
    match tool {
        "search" => {
            copy_required_string(params, &mut out, "query")?;
            copy_optional_string_array(params, &mut out, "resource_types")?;
            copy_optional_string(params, &mut out, "persona")?;
            copy_optional_string(params, &mut out, "detail")?;
            copy_optional_f64(params, &mut out, "min_score")?;
            copy_optional_bool(params, &mut out, "fuzzy")?;
            copy_optional_bool(params, &mut out, "include_highlights")?;
            copy_optional_bool(params, &mut out, "include_sql")?;
            copy_optional_bool(params, &mut out, "explain")?;
            copy_optional_u64(params, &mut out, "limit")?;
            copy_optional_u64(params, &mut out, "offset")?;
        }
        "search_indicator" => {
            copy_required_string(params, &mut out, "query")?;
            copy_optional_string_array(params, &mut out, "resource_types")?;
            copy_optional_string_array(params, &mut out, "indicator_types")?;
            copy_optional_string(params, &mut out, "persona")?;
            copy_optional_string(params, &mut out, "detail")?;
            copy_optional_string(params, &mut out, "group_mode")?;
            copy_optional_u64(params, &mut out, "max_parent_groups")?;
            copy_optional_bool(params, &mut out, "include_support_signals")?;
            copy_optional_f64(params, &mut out, "min_score")?;
            copy_optional_bool(params, &mut out, "explain")?;
            copy_optional_u64(params, &mut out, "limit")?;
            copy_optional_u64(params, &mut out, "offset")?;
        }
        "search_columns" => {
            copy_required_string(params, &mut out, "query")?;
            copy_optional_string_array(params, &mut out, "resource_types")?;
            copy_optional_string_array(params, &mut out, "roles")?;
            copy_optional_string_array(params, &mut out, "semantic_types")?;
            copy_optional_f64(params, &mut out, "min_score")?;
            copy_optional_u64(params, &mut out, "limit")?;
            copy_optional_u64(params, &mut out, "offset")?;
        }
        "get_entity" => {
            copy_required_string(params, &mut out, "id_or_name")?;
            copy_optional_string(params, &mut out, "resource_type")?;
            copy_optional_string(params, &mut out, "detail")?;
        }
        "get_context" => {
            copy_required_string(params, &mut out, "id_or_name")?;
            copy_optional_string(params, &mut out, "resource_type")?;
            copy_optional_bool(params, &mut out, "include_columns")?;
            copy_optional_bool(params, &mut out, "include_upstream")?;
            copy_optional_bool(params, &mut out, "include_downstream")?;
            copy_optional_bool(params, &mut out, "include_tests")?;
            copy_optional_bool(params, &mut out, "include_docs")?;
            copy_optional_bool(params, &mut out, "include_sql")?;
            copy_optional_string(params, &mut out, "context_mode")?;
            copy_optional_u64(params, &mut out, "lineage_depth")?;
            copy_optional_u64(params, &mut out, "upstream_limit")?;
            copy_optional_u64(params, &mut out, "downstream_limit")?;
        }
        "get_lineage" => {
            copy_required_string(params, &mut out, "id_or_name")?;
            copy_required_string(params, &mut out, "direction")?;
            copy_optional_u64(params, &mut out, "depth")?;
            copy_optional_string_array(params, &mut out, "resource_types")?;
            copy_optional_string(params, &mut out, "detail")?;
        }
        _ => return None,
    }
    Some(JsonValue::Object(out))
}

fn copy_required_string(
    input: &JsonMap<String, JsonValue>,
    output: &mut JsonMap<String, JsonValue>,
    key: &'static str,
) -> Option<()> {
    let value = safe_replay_string(input.get(key)?)?;
    output.insert(key.to_string(), JsonValue::String(value));
    Some(())
}

fn copy_optional_string(
    input: &JsonMap<String, JsonValue>,
    output: &mut JsonMap<String, JsonValue>,
    key: &'static str,
) -> Option<()> {
    let Some(value) = input.get(key) else {
        return Some(());
    };
    let value = safe_replay_string(value)?;
    output.insert(key.to_string(), JsonValue::String(value));
    Some(())
}

fn copy_optional_string_array(
    input: &JsonMap<String, JsonValue>,
    output: &mut JsonMap<String, JsonValue>,
    key: &'static str,
) -> Option<()> {
    let Some(value) = input.get(key) else {
        return Some(());
    };
    let items = value.as_array()?;
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        out.push(JsonValue::String(safe_replay_string(item)?));
    }
    output.insert(key.to_string(), JsonValue::Array(out));
    Some(())
}

fn copy_optional_bool(
    input: &JsonMap<String, JsonValue>,
    output: &mut JsonMap<String, JsonValue>,
    key: &'static str,
) -> Option<()> {
    if let Some(value) = input.get(key) {
        output.insert(key.to_string(), JsonValue::Bool(value.as_bool()?));
    }
    Some(())
}

fn copy_optional_u64(
    input: &JsonMap<String, JsonValue>,
    output: &mut JsonMap<String, JsonValue>,
    key: &'static str,
) -> Option<()> {
    if let Some(value) = input.get(key) {
        output.insert(key.to_string(), JsonValue::from(value.as_u64()?));
    }
    Some(())
}

fn copy_optional_f64(
    input: &JsonMap<String, JsonValue>,
    output: &mut JsonMap<String, JsonValue>,
    key: &'static str,
) -> Option<()> {
    if let Some(value) = input.get(key) {
        output.insert(key.to_string(), JsonValue::from(value.as_f64()?));
    }
    Some(())
}

fn safe_replay_string(value: &JsonValue) -> Option<String> {
    let value = value.as_str()?.trim();
    if value.is_empty()
        || value == "[REDACTED]"
        || value.contains("://")
        || value.to_ascii_lowercase().contains("bearer ")
    {
        return None;
    }
    Some(value.to_string())
}

fn params_summary_has_sensitive_signal(params: &JsonMap<String, JsonValue>) -> bool {
    params.iter().any(|(key, value)| {
        sensitive_replay_fragment(key) || value_contains_sensitive_signal(value)
    })
}

fn value_contains_sensitive_signal(value: &JsonValue) -> bool {
    match value {
        JsonValue::String(value) => {
            let lowered = value.to_ascii_lowercase();
            sensitive_replay_fragment(value)
                || lowered == "[redacted]"
                || lowered.contains("bearer ")
                || lowered.contains("-----begin ")
                || (lowered.contains("://")
                    && ["token", "secret", "password", "api_key", "apikey"]
                        .iter()
                        .any(|needle| lowered.contains(needle)))
        }
        JsonValue::Array(items) => items.iter().any(value_contains_sensitive_signal),
        JsonValue::Object(map) => map.iter().any(|(key, value)| {
            sensitive_replay_fragment(key) || value_contains_sensitive_signal(value)
        }),
        JsonValue::Null | JsonValue::Bool(_) | JsonValue::Number(_) => false,
    }
}

fn sensitive_replay_fragment(value: &str) -> bool {
    let lowered = value.to_ascii_lowercase();
    [
        "authorization",
        "credential",
        "password",
        "passwd",
        "private_key",
        "secret",
        "session",
        "token",
        "access_key",
        "api_key",
        "apikey",
        "proof_key",
        "raw_output",
    ]
    .iter()
    .any(|fragment| lowered.contains(fragment))
}

fn trace_shape_from_row(row: &JsonValue) -> TraceReplayShape {
    TraceReplayShape {
        success: row.get("success").and_then(JsonValue::as_bool),
        count: row.get("result_count").and_then(JsonValue::as_u64),
        total_available: row.get("total_available").and_then(JsonValue::as_u64),
        response_truncated: row
            .get("response_truncated")
            .and_then(JsonValue::as_bool)
            .unwrap_or(false),
        error_code: row
            .get("error_code")
            .and_then(JsonValue::as_str)
            .map(ToString::to_string),
        selected_unique_ids: string_array_field(row, "selected_unique_ids"),
        top_unique_ids: string_array_field(row, "top_unique_ids"),
    }
}

fn trace_shape_from_response(response: &JsonValue) -> TraceReplayShape {
    TraceReplayShape {
        success: response.get("success").and_then(JsonValue::as_bool),
        count: response.get("count").and_then(JsonValue::as_u64),
        total_available: response.get("total_available").and_then(JsonValue::as_u64),
        response_truncated: extract_response_truncated(response),
        error_code: response_error_code(response),
        selected_unique_ids: extract_unique_ids(response),
        top_unique_ids: extract_top_unique_ids(response),
    }
}

fn response_error_code(response: &JsonValue) -> Option<String> {
    response
        .get("error_code")
        .and_then(JsonValue::as_str)
        .or_else(|| {
            response
                .get("error")
                .and_then(|error| error.get("error_code"))
                .and_then(JsonValue::as_str)
        })
        .map(ToString::to_string)
}

fn compare_trace_shapes(
    original: &TraceReplayShape,
    replayed: &TraceReplayShape,
) -> Vec<TraceReplayChange> {
    let mut changes = Vec::new();
    push_option_change(
        &mut changes,
        "success",
        original.success,
        replayed.success,
        JsonValue::Bool,
    );
    push_option_change(
        &mut changes,
        "count",
        original.count,
        replayed.count,
        JsonValue::from,
    );
    push_option_change(
        &mut changes,
        "total_available",
        original.total_available,
        replayed.total_available,
        JsonValue::from,
    );
    if original.response_truncated != replayed.response_truncated {
        changes.push(TraceReplayChange {
            field: "response_truncated",
            before: JsonValue::Bool(original.response_truncated),
            after: JsonValue::Bool(replayed.response_truncated),
        });
    }
    if original.error_code != replayed.error_code
        && (original.error_code.is_some() || replayed.error_code.is_some())
    {
        changes.push(TraceReplayChange {
            field: "error_code",
            before: string_option_json(original.error_code.as_deref()),
            after: string_option_json(replayed.error_code.as_deref()),
        });
    }
    push_vec_change(
        &mut changes,
        "selected_unique_ids",
        &original.selected_unique_ids,
        &replayed.selected_unique_ids,
    );
    push_vec_change(
        &mut changes,
        "top_unique_ids",
        &original.top_unique_ids,
        &replayed.top_unique_ids,
    );
    changes
}

fn push_option_change<T: Eq + Copy>(
    changes: &mut Vec<TraceReplayChange>,
    field: &'static str,
    before: Option<T>,
    after: Option<T>,
    to_json: impl Fn(T) -> JsonValue,
) {
    if let (Some(before), Some(after)) = (before, after)
        && before != after
    {
        changes.push(TraceReplayChange {
            field,
            before: to_json(before),
            after: to_json(after),
        });
    }
}

fn push_vec_change(
    changes: &mut Vec<TraceReplayChange>,
    field: &'static str,
    before: &[String],
    after: &[String],
) {
    if !before.is_empty() && !after.is_empty() && before != after {
        changes.push(TraceReplayChange {
            field,
            before: string_vec_json(before),
            after: string_vec_json(after),
        });
    }
}

fn string_vec_json(values: &[String]) -> JsonValue {
    JsonValue::Array(values.iter().cloned().map(JsonValue::String).collect())
}

fn string_option_json(value: Option<&str>) -> JsonValue {
    value.map_or(JsonValue::Null, |value| {
        JsonValue::String(value.to_string())
    })
}

fn trace_row_tool(row: &JsonValue) -> String {
    row.get("tool")
        .and_then(JsonValue::as_str)
        .or_else(|| row.get("tool_name").and_then(JsonValue::as_str))
        .filter(|tool| !tool.trim().is_empty())
        .unwrap_or("unknown")
        .to_string()
}

fn trace_row_index(row: &JsonValue, fallback: usize) -> u64 {
    row.get("tool_call_index")
        .and_then(JsonValue::as_u64)
        .unwrap_or_else(|| u64::try_from(fallback).unwrap_or(u64::MAX))
}

fn string_array_field(row: &JsonValue, key: &str) -> Vec<String> {
    row.get(key)
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
        .filter_map(JsonValue::as_str)
        .map(ToString::to_string)
        .collect()
}

fn elapsed_ms_to_u64(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
fn read_trace_for_cli(path: &str) -> Result<ToolTraceRead, DbtNovaError> {
    read_trace_path(Path::new(path))
}

fn read_trace_path(path: &Path) -> Result<ToolTraceRead, DbtNovaError> {
    let read = read_tool_trace_file(path);
    if read.missing {
        let path = path.display();
        return Err(DbtNovaError::InvalidParams(format!(
            "trace file '{path}' does not exist"
        )));
    }
    if let Some(error) = &read.read_error {
        return Err(DbtNovaError::ServerError(error.clone()));
    }
    Ok(read)
}

fn print_or_json<T>(
    command: &'static str,
    json: bool,
    payload: &T,
    elapsed_ms: u128,
) -> DispatchResult
where
    T: Serialize,
{
    if json {
        print_json_envelope(command, payload, elapsed_ms)
    } else {
        let out = serde_json::to_string_pretty(payload).map_err(|error| DispatchError {
            error: DbtNovaError::ServerError(error.to_string()),
            rendered: false,
        })?;
        println!("{out}");
        Ok(())
    }
}

fn print_json_envelope<T>(command: &'static str, payload: &T, elapsed_ms: u128) -> DispatchResult
where
    T: Serialize,
{
    let envelope = CliEnvelope::success(command, payload, elapsed_ms);
    let out = serde_json::to_string_pretty(&envelope).map_err(|error| DispatchError {
        error: DbtNovaError::ServerError(error.to_string()),
        rendered: false,
    })?;
    println!("{out}");
    Ok(())
}

fn render_or_propagate_error(
    json: bool,
    command: &'static str,
    error: DbtNovaError,
    elapsed_ms: u128,
) -> DispatchError {
    if json {
        let envelope = error_envelope(command, &error, elapsed_ms);
        if let Ok(json) = serde_json::to_string_pretty(&envelope) {
            println!("{json}");
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
}

fn write_text_path(path: &Path, contents: &str) -> Result<(), DbtNovaError> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        create_dir_all(parent)?;
    }
    fs::write(path, contents)?;
    Ok(())
}

fn success_value(payload: JsonValue, count: usize) -> DbtNovaResult<JsonValue> {
    serde_json::to_value(SuccessResponse::new(payload, count))
        .map_err(|error| DbtNovaError::ServerError(error.to_string()))
}

fn with_trace_safety_policy(mut payload: JsonValue) -> DbtNovaResult<JsonValue> {
    let safety_policy = serde_json::to_value(trace_mcp_safety_policy()?)
        .map_err(|error| DbtNovaError::ServerError(error.to_string()))?;
    let Some(object) = payload.as_object_mut() else {
        return Err(DbtNovaError::ServerError(
            "trace tool payload must be a JSON object".to_string(),
        ));
    };
    object.insert("safety_policy".to_string(), safety_policy);
    Ok(payload)
}

fn trace_mcp_safety_policy() -> DbtNovaResult<TraceMcpSafetyPolicy> {
    Ok(TraceMcpSafetyPolicy {
        filesystem_root: mcp_trace_filesystem_root()?.display().to_string(),
        trace_writes_enabled_env: MCP_ENABLE_TRACE_WRITES_ENV,
        local_paths_must_stay_under_filesystem_root: true,
        write_operations_require_opt_in: true,
    })
}

fn require_mcp_trace_writes(tool_name: &str) -> DbtNovaResult<()> {
    if mcp_trace_writes_enabled() {
        return Ok(());
    }
    Err(DbtNovaError::InvalidParams(format!(
        "{tool_name} is disabled for MCP/tool-call use; set {MCP_ENABLE_TRACE_WRITES_ENV}=1 to enable trace report and redaction writes"
    )))
}

fn mcp_trace_writes_enabled() -> bool {
    std::env::var(MCP_ENABLE_TRACE_WRITES_ENV)
        .ok()
        .map(|value| value.trim().to_ascii_lowercase())
        .is_some_and(|value| matches!(value.as_str(), "1" | "true" | "yes" | "on"))
}

fn resolve_mcp_trace_existing_file(raw_path: &str, label: &str) -> DbtNovaResult<PathBuf> {
    let (_root, candidate) = mcp_trace_candidate_path(raw_path, label)?;
    let canonical = candidate.canonicalize().map_err(|error| {
        DbtNovaError::InvalidParams(format!(
            "failed to resolve {label} '{}': {error}",
            candidate.display()
        ))
    })?;
    let root = mcp_trace_filesystem_root()?;
    ensure_mcp_trace_path_under_root(&canonical, &root, label)?;
    if !canonical.is_file() {
        return Err(DbtNovaError::InvalidParams(format!(
            "{label} '{}' is not a file",
            canonical.display()
        )));
    }
    Ok(canonical)
}

fn resolve_mcp_trace_writable_path(raw_path: &str, label: &str) -> DbtNovaResult<PathBuf> {
    let (root, candidate) = mcp_trace_candidate_path(raw_path, label)?;
    if candidate.exists() {
        let canonical = candidate.canonicalize().map_err(|error| {
            DbtNovaError::InvalidParams(format!(
                "failed to resolve {label} '{}': {error}",
                candidate.display()
            ))
        })?;
        ensure_mcp_trace_path_under_root(&canonical, &root, label)?;
        return Ok(canonical);
    }
    ensure_existing_ancestor_under_trace_root(&candidate, &root, label)?;
    Ok(candidate)
}

fn mcp_trace_candidate_path(raw_path: &str, label: &str) -> DbtNovaResult<(PathBuf, PathBuf)> {
    let trimmed = raw_path.trim();
    if trimmed.is_empty() {
        return Err(DbtNovaError::InvalidParams(format!(
            "{label} must not be empty"
        )));
    }
    let root = mcp_trace_filesystem_root()?;
    let path = PathBuf::from(trimmed);
    reject_mcp_trace_parent_dirs(&path, label)?;
    if !path.is_absolute() {
        reject_mcp_trace_relative_traversal(&path, label)?;
    }
    let candidate = if path.is_absolute() {
        path
    } else {
        root.join(path)
    };
    ensure_mcp_trace_path_under_root(&candidate, &root, label)?;
    Ok((root, candidate))
}

fn reject_mcp_trace_parent_dirs(path: &Path, label: &str) -> DbtNovaResult<()> {
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

fn reject_mcp_trace_relative_traversal(path: &Path, label: &str) -> DbtNovaResult<()> {
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

fn ensure_existing_ancestor_under_trace_root(
    candidate: &Path,
    root: &Path,
    label: &str,
) -> DbtNovaResult<()> {
    let mut ancestor = candidate.parent();
    while let Some(path) = ancestor {
        if path.exists() {
            let canonical = path.canonicalize().map_err(|error| {
                DbtNovaError::InvalidParams(format!(
                    "failed to resolve parent for {label} '{}': {error}",
                    path.display()
                ))
            })?;
            return ensure_mcp_trace_path_under_root(&canonical, root, label);
        }
        ancestor = path.parent();
    }
    Err(DbtNovaError::InvalidParams(format!(
        "{label} has no existing parent under '{}'",
        root.display()
    )))
}

fn ensure_mcp_trace_path_under_root(path: &Path, root: &Path, label: &str) -> DbtNovaResult<()> {
    if path.starts_with(root) {
        return Ok(());
    }
    Err(DbtNovaError::InvalidParams(format!(
        "{label} '{}' is outside server working directory '{}'",
        path.display(),
        root.display()
    )))
}

fn mcp_trace_filesystem_root() -> DbtNovaResult<PathBuf> {
    std::env::current_dir()
        .and_then(|path| path.canonicalize())
        .map_err(|error| {
            DbtNovaError::InvalidParams(format!(
                "failed to resolve server working directory: {error}"
            ))
        })
}

fn render_markdown_summary(read: &ToolTraceRead, summary: &ToolTraceSummary) -> String {
    let mut out = String::new();
    out.push_str("# Nova Tool Trace Summary\n\n");
    append_markdown_overview(&mut out, read, summary);
    append_markdown_tool_order(&mut out, summary);
    append_markdown_tool_counts(&mut out, summary);
    append_markdown_response_budget(&mut out, summary);
    append_markdown_semantic_signal(&mut out, summary);
    append_id_list(
        &mut out,
        "Selected Unique IDs",
        &summary.selected_unique_ids,
    );
    append_id_list(&mut out, "Top Unique IDs", &summary.top_unique_ids);
    append_markdown_parse_warnings(&mut out, read);

    out
}

fn append_markdown_overview(out: &mut String, read: &ToolTraceRead, summary: &ToolTraceSummary) {
    out.push_str("## Overview\n\n");
    let _ = writeln!(out, "- path: `{}`", escape_inline(&read.path));
    let _ = writeln!(out, "- rows: `{}`", summary.row_count);
    let _ = writeln!(out, "- malformed_rows: `{}`", summary.parse_warning_count);
    let _ = writeln!(out, "- distinct_tools: `{}`", summary.distinct_tools.len());
    let _ = writeln!(
        out,
        "- total_response_bytes: `{}`",
        summary.response_budget.total_response_bytes
    );
    let _ = writeln!(
        out,
        "- response_truncated_count: `{}`",
        summary.truncation.response_truncated_count
    );
    out.push('\n');
}

fn append_markdown_tool_order(out: &mut String, summary: &ToolTraceSummary) {
    out.push_str("## Tool Order\n\n");
    if summary.tool_order.is_empty() {
        out.push_str("No valid tool-call rows were found.\n\n");
    } else {
        out.push_str("| # | tool | success | response_bytes | truncated | error |\n");
        out.push_str("|---|------|---------|----------------|-----------|-------|\n");
        for call in &summary.tool_order {
            let _ = writeln!(
                out,
                "| {} | `{}` | {} | {} | {} | {} |",
                call.tool_call_index,
                escape_table(&call.tool),
                call.success
                    .map_or("unknown".to_string(), |value| value.to_string()),
                call.response_bytes
                    .map_or(String::new(), |value| value.to_string()),
                call.response_truncated,
                call.error_code
                    .as_deref()
                    .map_or(String::new(), escape_table)
            );
        }
        out.push('\n');
    }
}

fn append_markdown_tool_counts(out: &mut String, summary: &ToolTraceSummary) {
    out.push_str("## Tool Counts\n\n");
    if summary.tool_counts.is_empty() {
        out.push_str("No tool counts available.\n\n");
    } else {
        out.push_str("| tool | calls |\n");
        out.push_str("|------|-------|\n");
        for (tool, count) in &summary.tool_counts {
            let _ = writeln!(out, "| `{}` | {} |", escape_table(tool), count);
        }
        out.push('\n');
    }
}

fn append_markdown_response_budget(out: &mut String, summary: &ToolTraceSummary) {
    out.push_str("## Response Budget\n\n");
    out.push_str("| tool | calls | total_response_bytes | max_response_bytes |\n");
    out.push_str("|------|-------|----------------------|--------------------|\n");
    for row in &summary.response_budget.by_tool {
        let _ = writeln!(
            out,
            "| `{}` | {} | {} | {} |",
            escape_table(&row.tool),
            row.call_count,
            row.total_response_bytes,
            row.max_response_bytes
                .map_or(String::new(), |value| value.to_string())
        );
    }
    out.push('\n');
}

fn append_markdown_semantic_signal(out: &mut String, summary: &ToolTraceSummary) {
    out.push_str("## Semantic-First Signal\n\n");
    let _ = writeln!(out, "- status: `{}`", summary.semantic_first.status);
    let _ = writeln!(out, "- message: {}", summary.semantic_first.message);
    if !summary.semantic_first.metric_like_queries.is_empty() {
        out.push_str("- metric_like_queries:\n");
        for query in &summary.semantic_first.metric_like_queries {
            let _ = writeln!(out, "  - `{}`", escape_inline(query));
        }
    }
    out.push('\n');
}

fn append_markdown_parse_warnings(out: &mut String, read: &ToolTraceRead) {
    if !read.parse_warnings.is_empty() {
        out.push_str("## Parse Warnings\n\n");
        out.push_str("| line | warning |\n");
        out.push_str("|------|---------|\n");
        for warning in &read.parse_warnings {
            let _ = writeln!(
                out,
                "| {} | {} |",
                warning.line,
                escape_table(&warning.message)
            );
        }
        out.push('\n');
    }
}

fn append_id_list(out: &mut String, title: &str, ids: &[String]) {
    let _ = writeln!(out, "## {title}\n");
    if ids.is_empty() {
        out.push_str("No ids observed.\n\n");
        return;
    }
    for id in ids.iter().take(25) {
        let _ = writeln!(out, "- `{}`", escape_inline(id));
    }
    if ids.len() > 25 {
        let remaining = ids.len() - 25;
        let _ = writeln!(out, "- ... {remaining} more");
    }
    out.push('\n');
}

fn print_redaction_human(report: &ToolTraceRedactionReport) {
    println!("trace redacted");
    println!("  input_path: {}", report.input_path);
    println!("  output_path: {}", report.output_path);
    println!("  rows_read: {}", report.rows_read);
    println!("  rows_written: {}", report.rows_written);
    println!("  malformed_rows: {}", report.malformed_rows);
    println!("  fields_removed: {}", report.fields_removed);
    println!("  fields_masked: {}", report.fields_masked);
}

fn print_replay_human(report: &TraceReplayData) {
    println!("trace replay complete");
    println!("  path: {}", report.path);
    println!("  manifest_hash: {}", report.manifest.manifest_hash);
    println!("  manifest_version: {}", report.manifest.manifest_version);
    println!("  rows: {}", report.row_count);
    println!("  parse_warnings: {}", report.parse_warnings.len());
    println!("  replayed: {}", report.counts.replayed);
    println!("  changed: {}", report.counts.changed);
    println!("  skipped: {}", report.counts.skipped);
    println!("  failed: {}", report.counts.failed);
    println!("  unsupported: {}", report.counts.unsupported);

    let interesting = report
        .results
        .iter()
        .filter(|row| row.status != "replayed")
        .take(10)
        .collect::<Vec<_>>();
    if interesting.is_empty() {
        return;
    }
    println!("  findings:");
    for row in interesting {
        let reason = row.reason.as_deref().unwrap_or("");
        println!(
            "    #{} {} {} {}",
            row.tool_call_index, row.tool, row.status, reason
        );
    }
}

fn escape_inline(value: &str) -> String {
    value.replace('`', "'")
}

fn escape_table(value: &str) -> String {
    escape_inline(value).replace('|', "\\|")
}

#[cfg(test)]
mod tests {
    use tempfile::{NamedTempFile, TempDir};

    use super::{
        MCP_ENABLE_TRACE_WRITES_ENV, TraceReplayShape, build_replay_params,
        build_trace_inspect_tool_response, build_trace_redact_tool_response,
        build_trace_replay_tool_response, compare_trace_shapes, read_trace_for_cli,
        render_markdown_summary, run_redact_command, run_summarize_command,
    };
    use crate::cli::args::{TraceRedactArgs, TraceSummarizeArgs};
    use crate::params::{TraceInspectParams, TraceRedactParams, TraceReplayParams};
    use crate::tests::common::get_searcher_with_fixture;
    use crate::utils::tool_trace::{read_tool_trace_file, summarize_tool_trace};

    #[test]
    fn read_trace_for_cli_rejects_missing_file() {
        let error = read_trace_for_cli("/tmp/dbt-nova-missing-trace.jsonl")
            .expect_err("missing trace should fail");
        assert!(error.to_string().contains("does not exist"));
    }

    #[test]
    fn summarize_writes_markdown_report() {
        let trace = NamedTempFile::new().expect("trace");
        std::fs::write(
            trace.path(),
            r#"{"tool":"search_indicator","success":true,"response_bytes":42,"response_truncated":false,"selected_unique_ids":["model.pkg.orders"],"top_unique_ids":["model.pkg.orders"],"params_summary":{"query":"revenue"}}"#,
        )
        .expect("write trace");
        let dir = TempDir::new().expect("dir");
        let report = dir.path().join("trace.md");
        let args = TraceSummarizeArgs {
            path: trace.path().display().to_string(),
            report_md_path: Some(report.display().to_string()),
            json: false,
        };

        assert!(run_summarize_command(&args).is_ok());

        let markdown = std::fs::read_to_string(report).expect("markdown");
        assert!(markdown.contains("Nova Tool Trace Summary"));
        assert!(markdown.contains("search_indicator"));
        assert!(markdown.contains("model.pkg.orders"));
    }

    #[test]
    fn markdown_summary_handles_empty_trace() {
        let trace = NamedTempFile::new().expect("trace");
        let read = read_tool_trace_file(trace.path());
        let summary = summarize_tool_trace(&read);
        let markdown = render_markdown_summary(&read, &summary);
        assert!(markdown.contains("No valid tool-call rows were found."));
    }

    #[test]
    fn redact_command_fails_for_same_output_path() {
        let trace = NamedTempFile::new().expect("trace");
        let args = TraceRedactArgs {
            path: trace.path().display().to_string(),
            out: trace.path().display().to_string(),
            json: false,
        };
        let error = run_redact_command(&args).expect_err("same path should fail");
        assert!(error.error.to_string().contains("--out must be different"));
    }

    #[test]
    fn trace_inspect_tool_response_returns_summary_contract() {
        let root = std::env::current_dir()
            .expect("cwd")
            .canonicalize()
            .expect("canonical cwd");
        let dir = TempDir::new_in(&root).expect("dir");
        let trace = dir.path().join("trace.jsonl");
        std::fs::write(
            &trace,
            r#"{"tool":"search_indicator","success":true,"response_bytes":42,"response_truncated":false,"params_summary":{"query":"total revenue"}}"#,
        )
        .expect("write trace");

        let response = build_trace_inspect_tool_response(&TraceInspectParams {
            path: trace.display().to_string(),
        })
        .expect("trace inspect response");

        assert_eq!(response["success"], serde_json::json!(true));
        assert_eq!(response["count"], serde_json::json!(1));
        assert_eq!(
            response["data"]["summary"]["semantic_first"]["status"],
            serde_json::json!("observed_without_sql")
        );
        assert!(response["data"]["safety_policy"].is_object());
    }

    #[test]
    fn trace_redact_tool_response_requires_write_opt_in() {
        let previous = std::env::var(MCP_ENABLE_TRACE_WRITES_ENV).ok();
        unsafe { std::env::remove_var(MCP_ENABLE_TRACE_WRITES_ENV) };

        let root = std::env::current_dir()
            .expect("cwd")
            .canonicalize()
            .expect("canonical cwd");
        let dir = TempDir::new_in(&root).expect("dir");
        let trace = dir.path().join("trace.jsonl");
        let out = dir.path().join("trace.redacted.jsonl");
        std::fs::write(
            &trace,
            r#"{"tool":"execute_sql","success":true,"response_bytes":42,"response_truncated":false,"params_summary":{"query":"s3://bucket/path?token=secret"}}"#,
        )
        .expect("write trace");

        let params = TraceRedactParams {
            path: trace.display().to_string(),
            out: out.display().to_string(),
        };
        let error = build_trace_redact_tool_response(&params).expect_err("write gate");
        assert!(error.to_string().contains(MCP_ENABLE_TRACE_WRITES_ENV));

        unsafe { std::env::set_var(MCP_ENABLE_TRACE_WRITES_ENV, "1") };
        let response = build_trace_redact_tool_response(&params).expect("redact response");
        assert_eq!(response["success"], serde_json::json!(true));
        assert_eq!(response["count"], serde_json::json!(1));
        assert!(
            std::fs::read_to_string(out)
                .expect("redacted")
                .contains("[REDACTED]")
        );

        match previous {
            Some(value) => unsafe { std::env::set_var(MCP_ENABLE_TRACE_WRITES_ENV, value) },
            None => unsafe { std::env::remove_var(MCP_ENABLE_TRACE_WRITES_ENV) },
        }
    }

    #[test]
    fn trace_replay_params_preserve_safe_supported_tool_options() {
        let search_columns = serde_json::json!({
            "query": "customer",
            "resource_types": ["model"],
            "roles": ["dimension"],
            "semantic_types": ["entity_id"],
            "min_score": 0.5,
            "limit": 3,
            "offset": 1
        });
        let rebuilt = build_replay_params(
            "search_columns",
            search_columns.as_object().expect("object params"),
        )
        .expect("search_columns params");
        assert_eq!(rebuilt["roles"], serde_json::json!(["dimension"]));
        assert_eq!(rebuilt["semantic_types"], serde_json::json!(["entity_id"]));
        assert_eq!(rebuilt["min_score"], serde_json::json!(0.5));

        let context = serde_json::json!({
            "id_or_name": "model.nova_test.fct__orders",
            "resource_type": "model",
            "include_columns": false,
            "include_upstream": false,
            "include_downstream": true,
            "include_tests": false,
            "include_docs": true,
            "context_mode": "engineer",
            "lineage_depth": 2,
            "upstream_limit": 4,
            "downstream_limit": 5
        });
        let rebuilt =
            build_replay_params("get_context", context.as_object().expect("object params"))
                .expect("get_context params");
        assert_eq!(rebuilt["include_columns"], serde_json::json!(false));
        assert_eq!(rebuilt["include_upstream"], serde_json::json!(false));
        assert_eq!(rebuilt["context_mode"], serde_json::json!("engineer"));
        assert_eq!(rebuilt["lineage_depth"], serde_json::json!(2));

        let lineage = serde_json::json!({
            "id_or_name": "model.nova_test.fct__orders",
            "direction": "upstream",
            "depth": 2
        });
        let rebuilt =
            build_replay_params("get_lineage", lineage.as_object().expect("object params"))
                .expect("get_lineage params");
        assert_eq!(rebuilt["depth"], serde_json::json!(2));
    }

    #[test]
    fn trace_replay_shape_changes_include_error_code() {
        let original = TraceReplayShape {
            success: Some(false),
            error_code: Some("NOT_FOUND".to_string()),
            ..TraceReplayShape::default()
        };
        let replayed = TraceReplayShape {
            success: Some(true),
            ..TraceReplayShape::default()
        };

        let fields = compare_trace_shapes(&original, &replayed)
            .into_iter()
            .map(|change| change.field)
            .collect::<Vec<_>>();

        assert!(fields.contains(&"success"));
        assert!(fields.contains(&"error_code"));
    }

    #[tokio::test]
    async fn trace_replay_tool_response_reports_replay_outcomes() {
        let searcher = get_searcher_with_fixture("nova_manifest.json");
        let root = std::env::current_dir()
            .expect("cwd")
            .canonicalize()
            .expect("canonical cwd");
        let dir = TempDir::new_in(&root).expect("dir");
        let trace = dir.path().join("trace.jsonl");
        std::fs::write(
            &trace,
            [
                r#"{"tool":"get_entity","success":true,"result_count":1,"response_truncated":false,"selected_unique_ids":["model.nova_test.fct__orders"],"top_unique_ids":["model.nova_test.fct__orders"],"params_summary":{"id_or_name":"model.nova_test.fct__orders","resource_type":"model"}}"#,
                r#"{"tool":"get_entity","success":true,"result_count":1,"response_truncated":false,"selected_unique_ids":["model.nova_test.dim__customers"],"top_unique_ids":["model.nova_test.dim__customers"],"params_summary":{"id_or_name":"model.nova_test.fct__orders","resource_type":"model"}}"#,
                r#"{"tool":"execute_sql","success":true,"response_truncated":false,"params_summary":{"query":"select 1"}}"#,
                r#"{"tool":"unknown_tool","success":true,"response_truncated":false,"params_summary":{"query":"orders"}}"#,
                r#"{"tool":"search","success":true,"response_truncated":false,"params_summary":{"query":"orders","token":"secret"}}"#,
                r#"{"tool":"get_entity","success":true,"response_truncated":false,"params_summary":{"id_or_name":"model.nova_test.missing","resource_type":"model"}}"#,
            ]
            .join("\n"),
        )
        .expect("write trace");

        let response = build_trace_replay_tool_response(
            &searcher,
            &TraceReplayParams {
                path: trace.display().to_string(),
            },
        )
        .await
        .expect("replay response");

        assert_eq!(response["success"], serde_json::json!(true));
        assert_eq!(response["count"], serde_json::json!(6));
        assert_eq!(response["data"]["counts"]["replayed"], serde_json::json!(1));
        assert_eq!(response["data"]["counts"]["changed"], serde_json::json!(1));
        assert_eq!(response["data"]["counts"]["skipped"], serde_json::json!(2));
        assert_eq!(
            response["data"]["counts"]["unsupported"],
            serde_json::json!(1)
        );
        assert_eq!(response["data"]["counts"]["failed"], serde_json::json!(1));
        assert_eq!(
            response["data"]["results"][1]["changes"][0]["field"],
            serde_json::json!("selected_unique_ids")
        );
        assert!(
            response["data"]["results"][2]["reason"]
                .as_str()
                .expect("skip reason")
                .contains("execute_sql")
        );
        assert!(
            response["data"]["results"][4]["reason"]
                .as_str()
                .expect("unsafe reason")
                .contains("sensitive")
        );
        assert!(response["data"]["safety_policy"].is_object());
    }
}
