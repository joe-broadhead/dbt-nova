use std::fmt::Write as _;
use std::fs::{self, create_dir_all};
use std::path::{Component, Path, PathBuf};
use std::time::Instant;

use serde::Serialize;
use serde_json::Value as JsonValue;

use crate::cli::args::{TraceInspectArgs, TraceRedactArgs, TraceSummarizeArgs};
use crate::cli::output::{CliEnvelope, error_envelope};
use crate::error::{DbtNovaError, Result as DbtNovaResult};
use crate::params::{TraceInspectParams, TraceRedactParams, TraceSummarizeParams};
use crate::responses::SuccessResponse;
use crate::utils::tool_trace::{
    ToolTraceParseWarning, ToolTraceRead, ToolTraceRedactionReport, ToolTraceSummary,
    read_tool_trace_file, redact_tool_trace_file, summarize_tool_trace,
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
struct TraceMcpSafetyPolicy {
    filesystem_root: String,
    trace_writes_enabled_env: &'static str,
    local_paths_must_stay_under_filesystem_root: bool,
    write_operations_require_opt_in: bool,
}

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
        MCP_ENABLE_TRACE_WRITES_ENV, build_trace_inspect_tool_response,
        build_trace_redact_tool_response, read_trace_for_cli, render_markdown_summary,
        run_redact_command, run_summarize_command,
    };
    use crate::cli::args::{TraceRedactArgs, TraceSummarizeArgs};
    use crate::params::{TraceInspectParams, TraceRedactParams};
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
}
