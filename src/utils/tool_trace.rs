use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions, create_dir_all};
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::{Value, json};
use tracing::warn;

use crate::error::{DbtNovaError, Result};
use crate::utils::sanitize_uri;

pub const TRACE_ENV: &str = "DBT_NOVA_TRACE_TOOL_CALLS_PATH";
const MAX_PARAM_KEYS: usize = 50;
const MAX_ARRAY_ITEMS: usize = 20;
const MAX_SUMMARY_STRING_CHARS: usize = 256;
const MAX_SELECTED_UNIQUE_IDS: usize = 200;
const MAX_TOP_UNIQUE_IDS: usize = 20;
const MAX_UNIQUE_ID_CHARS: usize = 512;
const REDACTED_VALUE: &str = "[REDACTED]";
const TRACE_SUMMARY_SCHEMA_VERSION: &str = "tool_trace_summary.v1";
const TRACE_REDACTION_SCHEMA_VERSION: &str = "tool_trace_redaction.v1";
const SAFE_PARAM_SUMMARY_KEYS: [&str; 36] = [
    "query",
    "persona",
    "id_or_name",
    "resource_type",
    "resource_types",
    "roles",
    "semantic_types",
    "recipe_id",
    "direction",
    "depth",
    "detail",
    "group_mode",
    "indicator_types",
    "include_support_signals",
    "max_parent_groups",
    "min_score",
    "fuzzy",
    "include_highlights",
    "include_sql",
    "explain",
    "include_columns",
    "include_upstream",
    "include_downstream",
    "include_tests",
    "include_docs",
    "context_mode",
    "lineage_depth",
    "upstream_limit",
    "downstream_limit",
    "preflight_only",
    "row_limit",
    "byte_limit",
    "max_poll_seconds",
    "limit",
    "offset",
    "scope",
];
const SENSITIVE_KEY_FRAGMENTS: [&str; 13] = [
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
];
const UNINITIALIZED_TOOL_CALL_INDEX: u64 = u64::MAX;
static TOOL_CALL_INDEX: AtomicU64 = AtomicU64::new(UNINITIALIZED_TOOL_CALL_INDEX);

#[derive(Debug, Serialize)]
struct ToolTraceRow {
    timestamp_ms: u128,
    tool_call_index: u64,
    transport: String,
    tool: String,
    success: bool,
    duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    error_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    params_summary: Option<Value>,
    response_bytes: usize,
    response_truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    result_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    total_available: Option<u64>,
    selected_unique_ids: Vec<String>,
    top_unique_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ToolTraceParseWarning {
    pub line: usize,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolTraceRead {
    pub path: String,
    pub rows: Vec<Value>,
    pub parse_warnings: Vec<ToolTraceParseWarning>,
    pub missing: bool,
    pub read_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolTraceSummary {
    pub schema_version: &'static str,
    pub row_count: usize,
    pub parse_warning_count: usize,
    pub empty: bool,
    pub message: String,
    pub tool_order: Vec<ToolTraceCallSummary>,
    pub tool_counts: BTreeMap<String, usize>,
    pub distinct_tools: Vec<String>,
    pub selected_unique_ids: Vec<String>,
    pub top_unique_ids: Vec<String>,
    pub response_budget: ToolTraceResponseBudget,
    pub truncation: ToolTraceTruncationSummary,
    pub errors: ToolTraceErrorSummary,
    pub semantic_first: ToolTraceSemanticFirstSummary,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolTraceCallSummary {
    pub tool_call_index: u64,
    pub tool: String,
    pub success: Option<bool>,
    pub duration_ms: Option<u64>,
    pub response_bytes: Option<u64>,
    pub response_truncated: bool,
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolTraceResponseBudget {
    pub total_response_bytes: u64,
    pub max_response_bytes: Option<u64>,
    pub max_response_tool: Option<String>,
    pub rows_missing_response_bytes: usize,
    pub by_tool: Vec<ToolTraceToolResponseBudget>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolTraceToolResponseBudget {
    pub tool: String,
    pub call_count: usize,
    pub total_response_bytes: u64,
    pub max_response_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolTraceTruncationSummary {
    pub response_truncated_count: usize,
    pub tools_with_truncation: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolTraceErrorSummary {
    pub failed_tool_call_count: usize,
    pub error_codes: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolTraceSemanticFirstSummary {
    pub status: &'static str,
    pub message: String,
    pub first_search_indicator_index: Option<u64>,
    pub first_execute_sql_index: Option<u64>,
    pub metric_like_queries: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolTraceRedactionReport {
    pub schema_version: &'static str,
    pub input_path: String,
    pub output_path: String,
    pub rows_read: usize,
    pub rows_written: usize,
    pub malformed_rows: usize,
    pub fields_removed: usize,
    pub fields_masked: usize,
    pub parse_warnings: Vec<ToolTraceParseWarning>,
}

/// Append a sanitized tool-call trace row when `DBT_NOVA_TRACE_TOOL_CALLS_PATH` is set.
pub fn record_tool_call(
    transport: &str,
    tool: &str,
    params: Option<&Value>,
    response: Option<&Value>,
    success: bool,
    duration_ms: u64,
) {
    let Ok(path) = std::env::var(TRACE_ENV) else {
        return;
    };
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return;
    }

    let trace_path = Path::new(trimmed);
    if let Some(parent) = trace_path.parent()
        && !parent.as_os_str().is_empty()
        && let Err(error) = create_dir_all(parent)
    {
        warn!(path = %parent.display(), error = %error, "failed to create tool trace directory");
        return;
    }

    let row = ToolTraceRow {
        timestamp_ms: timestamp_ms(),
        tool_call_index: next_tool_call_index(trace_path),
        transport: transport.to_string(),
        tool: tool.to_string(),
        success,
        duration_ms,
        error_code: response.and_then(extract_error_code),
        params_summary: params.map(summarize_params),
        response_bytes: response.map_or(0, serialized_len),
        response_truncated: response.is_some_and(extract_response_truncated),
        result_count: response.and_then(|value| value.get("count").and_then(Value::as_u64)),
        total_available: response.and_then(|value| {
            value
                .get("total_available")
                .and_then(Value::as_u64)
                .or_else(|| {
                    value
                        .get("_nova_result_meta")
                        .and_then(|meta| meta.get("original_count"))
                        .and_then(Value::as_u64)
                })
        }),
        selected_unique_ids: response.map(extract_unique_ids).unwrap_or_default(),
        top_unique_ids: response.map(extract_top_unique_ids).unwrap_or_default(),
    };

    let serialized = match serde_json::to_string(&row) {
        Ok(serialized) => serialized,
        Err(error) => {
            warn!(error = %error, "failed to serialize tool trace row");
            return;
        }
    };

    match OpenOptions::new()
        .create(true)
        .append(true)
        .open(trace_path)
    {
        Ok(mut file) => {
            if let Err(error) = writeln!(file, "{serialized}") {
                warn!(path = %trace_path.display(), error = %error, "failed to write tool trace row");
            }
        }
        Err(error) => {
            warn!(path = %trace_path.display(), error = %error, "failed to open tool trace file");
        }
    }
}

#[must_use]
pub fn read_tool_trace_file(path: &Path) -> ToolTraceRead {
    let path_display = path.display().to_string();
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return ToolTraceRead {
                path: path_display,
                rows: Vec::new(),
                parse_warnings: Vec::new(),
                missing: true,
                read_error: None,
            };
        }
        Err(error) => {
            return ToolTraceRead {
                path: path_display.clone(),
                rows: Vec::new(),
                parse_warnings: Vec::new(),
                missing: false,
                read_error: Some(format!(
                    "failed to read tool trace '{path_display}': {error}"
                )),
            };
        }
    };

    let mut rows = Vec::new();
    let mut parse_warnings = Vec::new();
    for (index, line) in raw.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Value>(line) {
            Ok(row) => rows.push(row),
            Err(error) => parse_warnings.push(ToolTraceParseWarning {
                line: index + 1,
                message: error.to_string(),
            }),
        }
    }
    normalize_tool_trace_indices(&mut rows);
    ToolTraceRead {
        path: path_display,
        rows,
        parse_warnings,
        missing: false,
        read_error: None,
    }
}

#[must_use]
pub fn summarize_tool_trace(read: &ToolTraceRead) -> ToolTraceSummary {
    let tool_order = build_tool_order(&read.rows);
    let aggregate = ToolTraceSummaryAccumulator::from_rows(&read.rows);

    ToolTraceSummary {
        schema_version: TRACE_SUMMARY_SCHEMA_VERSION,
        row_count: read.rows.len(),
        parse_warning_count: read.parse_warnings.len(),
        empty: read.rows.is_empty(),
        message: trace_summary_message(read),
        tool_order,
        tool_counts: aggregate.tool_counts,
        distinct_tools: aggregate.distinct_tools,
        selected_unique_ids: aggregate.selected_unique_ids,
        top_unique_ids: aggregate.top_unique_ids,
        response_budget: aggregate.response_budget,
        truncation: aggregate.truncation,
        errors: aggregate.errors,
        semantic_first: semantic_first_summary(&read.rows),
    }
}

/// Redact a trace file into a conservative JSONL allowlist.
///
/// # Errors
/// Returns an error when input/output paths are unsafe, unreadable, or unwritable.
pub fn redact_tool_trace_file(path: &Path, out: &Path) -> Result<ToolTraceRedactionReport> {
    if trace_paths_conflict(path, out) {
        return Err(DbtNovaError::InvalidParams(
            "--out must be different from --path".to_string(),
        ));
    }

    let read = read_tool_trace_file(path);
    if read.missing {
        return Err(DbtNovaError::InvalidParams(format!(
            "trace file '{}' does not exist",
            path.display()
        )));
    }
    if let Some(error) = &read.read_error {
        return Err(DbtNovaError::ServerError(error.clone()));
    }

    let mut fields_removed = 0usize;
    let mut fields_masked = 0usize;
    let mut lines = Vec::with_capacity(read.rows.len());
    for row in &read.rows {
        let redacted = redact_trace_row(row, &mut fields_removed, &mut fields_masked);
        let serialized = serde_json::to_string(&redacted)
            .map_err(|error| DbtNovaError::ServerError(error.to_string()))?;
        lines.push(serialized);
    }

    if let Some(parent) = out.parent()
        && !parent.as_os_str().is_empty()
    {
        create_dir_all(parent)?;
    }
    let mut body = lines.join("\n");
    if !body.is_empty() {
        body.push('\n');
    }
    fs::write(out, body)?;

    Ok(ToolTraceRedactionReport {
        schema_version: TRACE_REDACTION_SCHEMA_VERSION,
        input_path: path.display().to_string(),
        output_path: out.display().to_string(),
        rows_read: read.rows.len(),
        rows_written: lines.len(),
        malformed_rows: read.parse_warnings.len(),
        fields_removed,
        fields_masked,
        parse_warnings: read.parse_warnings,
    })
}

fn trace_paths_conflict(path: &Path, out: &Path) -> bool {
    if path == out {
        return true;
    }
    let Ok(path_canonical) = path.canonicalize() else {
        return false;
    };
    if let Ok(out_canonical) = out.canonicalize() {
        return path_canonical == out_canonical;
    }

    let Some(file_name) = out.file_name() else {
        return false;
    };
    let parent = out.parent().filter(|parent| !parent.as_os_str().is_empty());
    let parent = parent.unwrap_or_else(|| Path::new("."));
    parent
        .canonicalize()
        .is_ok_and(|canonical| canonical.join(file_name) == path_canonical)
}

fn build_tool_order(rows: &[Value]) -> Vec<ToolTraceCallSummary> {
    rows.iter()
        .enumerate()
        .map(|(index, row)| ToolTraceCallSummary {
            tool_call_index: row
                .get("tool_call_index")
                .and_then(Value::as_u64)
                .unwrap_or_else(|| u64::try_from(index).unwrap_or(u64::MAX)),
            tool: row_tool(row),
            success: row.get("success").and_then(Value::as_bool),
            duration_ms: row.get("duration_ms").and_then(Value::as_u64),
            response_bytes: row.get("response_bytes").and_then(Value::as_u64),
            response_truncated: row
                .get("response_truncated")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            error_code: row
                .get("error_code")
                .and_then(Value::as_str)
                .map(ToString::to_string),
        })
        .collect()
}

#[derive(Debug, Default)]
struct ToolBudgetAccumulator {
    call_count: usize,
    total_response_bytes: u64,
    max_response_bytes: Option<u64>,
}

#[derive(Debug)]
struct ToolTraceSummaryAccumulator {
    tool_counts: BTreeMap<String, usize>,
    distinct_tools: Vec<String>,
    selected_unique_ids: Vec<String>,
    top_unique_ids: Vec<String>,
    response_budget: ToolTraceResponseBudget,
    truncation: ToolTraceTruncationSummary,
    errors: ToolTraceErrorSummary,
}

impl ToolTraceSummaryAccumulator {
    fn from_rows(rows: &[Value]) -> Self {
        let mut builder = ToolTraceSummaryBuilder::default();
        for row in rows {
            builder.ingest(row);
        }
        builder.finish()
    }
}

#[derive(Debug, Default)]
struct ToolTraceSummaryBuilder {
    tool_counts: BTreeMap<String, usize>,
    distinct_tools_set: BTreeSet<String>,
    selected_unique_ids: BTreeSet<String>,
    top_unique_ids: Vec<String>,
    top_unique_id_seen: BTreeSet<String>,
    total_response_bytes: u64,
    max_response_bytes: Option<u64>,
    max_response_tool: Option<String>,
    rows_missing_response_bytes: usize,
    by_tool: BTreeMap<String, ToolBudgetAccumulator>,
    response_truncated_count: usize,
    tools_with_truncation: BTreeSet<String>,
    failed_tool_call_count: usize,
    error_codes: BTreeMap<String, usize>,
}

impl ToolTraceSummaryBuilder {
    fn ingest(&mut self, row: &Value) {
        let tool = row_tool(row);
        self.ingest_tool_counts(&tool);
        self.ingest_errors(row);
        self.ingest_unique_ids(row);
        self.ingest_response_budget(row, &tool);
        self.ingest_truncation(row, &tool);
    }

    fn ingest_tool_counts(&mut self, tool: &str) {
        *self.tool_counts.entry(tool.to_string()).or_insert(0) += 1;
        self.distinct_tools_set.insert(tool.to_string());
    }

    fn ingest_errors(&mut self, row: &Value) {
        if row.get("success").and_then(Value::as_bool) == Some(false) {
            self.failed_tool_call_count += 1;
        }
        if let Some(error_code) = row.get("error_code").and_then(Value::as_str) {
            *self.error_codes.entry(error_code.to_string()).or_insert(0) += 1;
        }
    }

    fn ingest_unique_ids(&mut self, row: &Value) {
        if let Some(ids) = row.get("selected_unique_ids").and_then(Value::as_array) {
            for id in ids.iter().filter_map(Value::as_str) {
                self.selected_unique_ids.insert(id.to_string());
            }
        }
        if let Some(ids) = row.get("top_unique_ids").and_then(Value::as_array) {
            for id in ids.iter().filter_map(Value::as_str) {
                let id = id.to_string();
                if self.top_unique_id_seen.insert(id.clone()) {
                    self.top_unique_ids.push(id);
                }
            }
        }
    }

    fn ingest_response_budget(&mut self, row: &Value, tool: &str) {
        let tool_budget = self.by_tool.entry(tool.to_string()).or_default();
        tool_budget.call_count += 1;

        let Some(bytes) = row.get("response_bytes").and_then(Value::as_u64) else {
            self.rows_missing_response_bytes += 1;
            return;
        };

        self.total_response_bytes = self.total_response_bytes.saturating_add(bytes);
        tool_budget.total_response_bytes = tool_budget.total_response_bytes.saturating_add(bytes);
        tool_budget.max_response_bytes = Some(
            tool_budget
                .max_response_bytes
                .map_or(bytes, |max| max.max(bytes)),
        );
        if self.max_response_bytes.is_none_or(|max| bytes > max) {
            self.max_response_bytes = Some(bytes);
            self.max_response_tool = Some(tool.to_string());
        }
    }

    fn ingest_truncation(&mut self, row: &Value, tool: &str) {
        let truncated = row
            .get("response_truncated")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if truncated {
            self.response_truncated_count += 1;
            self.tools_with_truncation.insert(tool.to_string());
        }
    }

    fn finish(self) -> ToolTraceSummaryAccumulator {
        let by_tool = self
            .by_tool
            .into_iter()
            .map(|(tool, budget)| ToolTraceToolResponseBudget {
                tool,
                call_count: budget.call_count,
                total_response_bytes: budget.total_response_bytes,
                max_response_bytes: budget.max_response_bytes,
            })
            .collect();

        ToolTraceSummaryAccumulator {
            tool_counts: self.tool_counts,
            distinct_tools: self.distinct_tools_set.into_iter().collect(),
            selected_unique_ids: self.selected_unique_ids.into_iter().collect(),
            top_unique_ids: self.top_unique_ids,
            response_budget: ToolTraceResponseBudget {
                total_response_bytes: self.total_response_bytes,
                max_response_bytes: self.max_response_bytes,
                max_response_tool: self.max_response_tool,
                rows_missing_response_bytes: self.rows_missing_response_bytes,
                by_tool,
            },
            truncation: ToolTraceTruncationSummary {
                response_truncated_count: self.response_truncated_count,
                tools_with_truncation: self.tools_with_truncation.into_iter().collect(),
            },
            errors: ToolTraceErrorSummary {
                failed_tool_call_count: self.failed_tool_call_count,
                error_codes: self.error_codes,
            },
        }
    }
}

fn trace_summary_message(read: &ToolTraceRead) -> String {
    let row_count = read.rows.len();
    let warning_count = read.parse_warnings.len();
    if read.rows.is_empty() {
        "trace file contained no valid tool-call rows".to_string()
    } else if read.parse_warnings.is_empty() {
        format!("read {row_count} tool-call row(s)")
    } else {
        format!("read {row_count} tool-call row(s) with {warning_count} malformed row warning(s)")
    }
}

fn row_tool(row: &Value) -> String {
    row.get("tool")
        .and_then(Value::as_str)
        .filter(|tool| !tool.trim().is_empty())
        .unwrap_or("unknown")
        .to_string()
}

fn semantic_first_summary(rows: &[Value]) -> ToolTraceSemanticFirstSummary {
    let first_search_indicator_index = first_tool_call_index(rows, "search_indicator");
    let first_execute_sql_index = first_tool_call_index(rows, "execute_sql");
    let metric_like_queries = metric_like_queries(rows);

    match (first_search_indicator_index, first_execute_sql_index) {
        (Some(search_index), Some(sql_index)) if search_index < sql_index => {
            ToolTraceSemanticFirstSummary {
                status: "pass",
                message: "search_indicator ran before execute_sql".to_string(),
                first_search_indicator_index: Some(search_index),
                first_execute_sql_index: Some(sql_index),
                metric_like_queries,
            }
        }
        (Some(search_index), Some(sql_index)) => ToolTraceSemanticFirstSummary {
            status: "warn",
            message: "execute_sql ran before search_indicator; consider semantic discovery before SQL execution".to_string(),
            first_search_indicator_index: Some(search_index),
            first_execute_sql_index: Some(sql_index),
            metric_like_queries,
        },
        (None, Some(sql_index)) if metric_like_queries.is_empty() => ToolTraceSemanticFirstSummary {
            status: "not_observed",
            message: "execute_sql was observed, but no metric-like query evidence was available to judge semantic-first behavior".to_string(),
            first_search_indicator_index: None,
            first_execute_sql_index: Some(sql_index),
            metric_like_queries,
        },
        (None, Some(sql_index)) => ToolTraceSemanticFirstSummary {
            status: "warn",
            message: "metric-like query evidence reached execute_sql without an earlier search_indicator call".to_string(),
            first_search_indicator_index: None,
            first_execute_sql_index: Some(sql_index),
            metric_like_queries,
        },
        (Some(search_index), None) => ToolTraceSemanticFirstSummary {
            status: "observed_without_sql",
            message: "search_indicator was observed and no execute_sql call was present".to_string(),
            first_search_indicator_index: Some(search_index),
            first_execute_sql_index: None,
            metric_like_queries,
        },
        (None, None) => ToolTraceSemanticFirstSummary {
            status: "not_applicable",
            message: "trace did not include execute_sql or search_indicator calls".to_string(),
            first_search_indicator_index: None,
            first_execute_sql_index: None,
            metric_like_queries,
        },
    }
}

fn first_tool_call_index(rows: &[Value], tool: &str) -> Option<u64> {
    rows.iter()
        .filter(|row| row.get("tool").and_then(Value::as_str) == Some(tool))
        .filter_map(|row| row.get("tool_call_index").and_then(Value::as_u64))
        .min()
}

fn metric_like_queries(rows: &[Value]) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    for row in rows {
        let Some(query) = row
            .get("params_summary")
            .and_then(|params| params.get("query"))
            .and_then(Value::as_str)
        else {
            continue;
        };
        if is_metric_like_query(query) && seen.insert(query.to_ascii_lowercase()) {
            out.push(query.to_string());
        }
    }
    out
}

fn is_metric_like_query(query: &str) -> bool {
    let lowered = query.to_ascii_lowercase();
    [
        "aov",
        "average",
        "conversion",
        "cost",
        "count",
        "gmv",
        "kpi",
        "margin",
        "measure",
        "metric",
        "rate",
        "ratio",
        "revenue",
        "sales",
        "total",
    ]
    .iter()
    .any(|needle| lowered.contains(needle))
}

fn redact_trace_row(row: &Value, fields_removed: &mut usize, fields_masked: &mut usize) -> Value {
    let Some(map) = row.as_object() else {
        return json!({"redaction_warning": "trace row was not an object"});
    };
    let mut out = serde_json::Map::new();
    copy_u128ish_field(map, &mut out, "timestamp_ms", fields_removed);
    copy_u64_field(map, &mut out, "tool_call_index", fields_removed);
    copy_string_field(map, &mut out, "transport", fields_removed, fields_masked);
    copy_string_field(map, &mut out, "tool", fields_removed, fields_masked);
    copy_bool_field(map, &mut out, "success", fields_removed);
    copy_u64_field(map, &mut out, "duration_ms", fields_removed);
    copy_string_field(map, &mut out, "error_code", fields_removed, fields_masked);
    if let Some(params) = map.get("params_summary") {
        let redacted = redact_params_summary(params, fields_removed, fields_masked);
        if !redacted.as_object().is_some_and(serde_json::Map::is_empty) {
            out.insert("params_summary".to_string(), redacted);
        }
    }
    copy_u64_field(map, &mut out, "response_bytes", fields_removed);
    copy_bool_field(map, &mut out, "response_truncated", fields_removed);
    copy_u64_field(map, &mut out, "result_count", fields_removed);
    copy_u64_field(map, &mut out, "total_available", fields_removed);
    copy_string_array_field(
        map,
        &mut out,
        "selected_unique_ids",
        fields_removed,
        fields_masked,
    );
    copy_string_array_field(
        map,
        &mut out,
        "top_unique_ids",
        fields_removed,
        fields_masked,
    );

    for key in map.keys() {
        if !is_redacted_top_level_key(key) && key != "params_summary" {
            *fields_removed += 1;
        }
    }
    Value::Object(out)
}

fn is_redacted_top_level_key(key: &str) -> bool {
    matches!(
        key,
        "timestamp_ms"
            | "tool_call_index"
            | "transport"
            | "tool"
            | "success"
            | "duration_ms"
            | "error_code"
            | "response_bytes"
            | "response_truncated"
            | "result_count"
            | "total_available"
            | "selected_unique_ids"
            | "top_unique_ids"
    )
}

fn copy_u128ish_field(
    input: &serde_json::Map<String, Value>,
    output: &mut serde_json::Map<String, Value>,
    key: &str,
    fields_removed: &mut usize,
) {
    if let Some(value) = input.get(key) {
        if value.as_u64().is_some() {
            output.insert(key.to_string(), value.clone());
        } else {
            *fields_removed += 1;
        }
    }
}

fn copy_u64_field(
    input: &serde_json::Map<String, Value>,
    output: &mut serde_json::Map<String, Value>,
    key: &str,
    fields_removed: &mut usize,
) {
    if let Some(value) = input.get(key) {
        if value.as_u64().is_some() {
            output.insert(key.to_string(), value.clone());
        } else {
            *fields_removed += 1;
        }
    }
}

fn copy_bool_field(
    input: &serde_json::Map<String, Value>,
    output: &mut serde_json::Map<String, Value>,
    key: &str,
    fields_removed: &mut usize,
) {
    if let Some(value) = input.get(key) {
        if value.as_bool().is_some() {
            output.insert(key.to_string(), value.clone());
        } else {
            *fields_removed += 1;
        }
    }
}

fn copy_string_field(
    input: &serde_json::Map<String, Value>,
    output: &mut serde_json::Map<String, Value>,
    key: &str,
    fields_removed: &mut usize,
    fields_masked: &mut usize,
) {
    if let Some(value) = input.get(key) {
        if let Some(value) = value.as_str() {
            output.insert(
                key.to_string(),
                Value::String(redact_string_value(key, value, fields_masked)),
            );
        } else {
            *fields_removed += 1;
        }
    }
}

fn copy_string_array_field(
    input: &serde_json::Map<String, Value>,
    output: &mut serde_json::Map<String, Value>,
    key: &str,
    fields_removed: &mut usize,
    fields_masked: &mut usize,
) {
    if let Some(value) = input.get(key) {
        let Some(items) = value.as_array() else {
            *fields_removed += 1;
            return;
        };
        let mut strings = Vec::new();
        for item in items {
            if let Some(item) = item.as_str() {
                strings.push(Value::String(redact_string_value(key, item, fields_masked)));
            } else {
                *fields_removed += 1;
            }
        }
        output.insert(key.to_string(), Value::Array(strings));
    }
}

fn redact_params_summary(
    params: &Value,
    fields_removed: &mut usize,
    fields_masked: &mut usize,
) -> Value {
    let Some(map) = params.as_object() else {
        *fields_removed += 1;
        return Value::Object(serde_json::Map::new());
    };
    let mut out = serde_json::Map::new();
    for (key, value) in map {
        if !SAFE_PARAM_SUMMARY_KEYS.contains(&key.as_str()) {
            *fields_removed += 1;
            continue;
        }
        if let Some(redacted) = redact_safe_summary_value(key, value, fields_masked) {
            out.insert(key.clone(), redacted);
        } else {
            *fields_removed += 1;
        }
    }
    Value::Object(out)
}

fn redact_safe_summary_value(key: &str, value: &Value, fields_masked: &mut usize) -> Option<Value> {
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) => Some(value.clone()),
        Value::String(value) => Some(Value::String(redact_string_value(
            key,
            value,
            fields_masked,
        ))),
        Value::Array(items) => Some(Value::Array(
            items
                .iter()
                .filter_map(|item| redact_safe_summary_value(key, item, fields_masked))
                .collect(),
        )),
        Value::Object(_) => None,
    }
}

fn redact_string_value(key: &str, value: &str, fields_masked: &mut usize) -> String {
    if sensitive_key(key) || sensitive_value(value) {
        *fields_masked += 1;
        return REDACTED_VALUE.to_string();
    }
    if looks_like_sensitive_uri(value) {
        let sanitized = sanitize_uri(value);
        if sanitized != value {
            *fields_masked += 1;
        }
        return sanitized;
    }
    truncate_string(value, MAX_SUMMARY_STRING_CHARS)
}

fn sensitive_key(key: &str) -> bool {
    let lowered = key.to_ascii_lowercase();
    SENSITIVE_KEY_FRAGMENTS
        .iter()
        .any(|fragment| lowered.contains(fragment))
}

fn sensitive_value(value: &str) -> bool {
    let lowered = value.to_ascii_lowercase();
    lowered.contains("-----begin ")
        || lowered.contains("authorization:")
        || lowered.contains("bearer ")
        || lowered.contains("snowflake token=")
}

fn looks_like_sensitive_uri(value: &str) -> bool {
    let lowered = value.to_ascii_lowercase();
    (lowered.contains("://") || lowered.contains('?'))
        && [
            "access_token",
            "api_key",
            "apikey",
            "password",
            "secret",
            "token",
        ]
        .iter()
        .any(|needle| lowered.contains(needle))
}

pub(crate) fn normalize_tool_trace_indices(rows: &mut [Value]) {
    for (index, row) in rows.iter_mut().enumerate() {
        if let Some(obj) = row.as_object_mut() {
            obj.insert(
                "tool_call_index".to_string(),
                Value::from(u64::try_from(index).unwrap_or(u64::MAX)),
            );
        }
    }
}

fn next_tool_call_index(trace_path: &Path) -> u64 {
    loop {
        let current = TOOL_CALL_INDEX.load(Ordering::Relaxed);
        if current != UNINITIALIZED_TOOL_CALL_INDEX {
            return TOOL_CALL_INDEX.fetch_add(1, Ordering::Relaxed);
        }
        let seed = existing_trace_row_count(trace_path);
        if TOOL_CALL_INDEX
            .compare_exchange(
                UNINITIALIZED_TOOL_CALL_INDEX,
                seed,
                Ordering::Relaxed,
                Ordering::Relaxed,
            )
            .is_ok()
        {
            return TOOL_CALL_INDEX.fetch_add(1, Ordering::Relaxed);
        }
    }
}

fn existing_trace_row_count(trace_path: &Path) -> u64 {
    fs::read_to_string(trace_path).map_or(0, |raw| {
        raw.lines().filter(|line| !line.trim().is_empty()).count() as u64
    })
}

fn serialized_len(value: &Value) -> usize {
    serde_json::to_string(value).map_or(0, |serialized| serialized.len())
}

pub(crate) fn extract_response_truncated(response: &Value) -> bool {
    match response {
        Value::Object(map) => {
            map.get("truncated")
                .and_then(Value::as_bool)
                .unwrap_or(false)
                || map
                    .get("_nova_result_meta")
                    .and_then(|meta| meta.get("truncated"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                || map.values().any(extract_response_truncated)
        }
        Value::Array(items) => items.iter().any(extract_response_truncated),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => false,
    }
}

fn summarize_params(params: &Value) -> Value {
    let Some(map) = params.as_object() else {
        return json!({"type": params_type(params)});
    };

    let keys: Vec<Value> = map
        .keys()
        .take(MAX_PARAM_KEYS)
        .map(|key| Value::String(truncate_string(key, MAX_SUMMARY_STRING_CHARS)))
        .collect();
    let mut summary = serde_json::Map::from_iter([(String::from("keys"), Value::Array(keys))]);
    for key in SAFE_PARAM_SUMMARY_KEYS {
        if let Some(value) = map.get(key).and_then(summarize_safe_value) {
            summary.insert(key.to_string(), value);
        }
    }
    insert_statement_structure_summary(&mut summary, map.get("statement").and_then(Value::as_str));
    Value::Object(summary)
}

fn insert_statement_structure_summary(
    summary: &mut serde_json::Map<String, Value>,
    statement: Option<&str>,
) {
    let Some(statement) = statement.map(str::trim).filter(|value| !value.is_empty()) else {
        return;
    };
    match crate::utils::sql_structure::sql_structure_summary_json(statement) {
        Ok(structure) => {
            summary.insert("statement_structure".to_string(), structure);
        }
        Err(_error) => {
            summary.insert(
                "statement_structure_error".to_string(),
                Value::String("failed to parse SQL structure summary".to_string()),
            );
        }
    }
}

fn summarize_safe_value(value: &Value) -> Option<Value> {
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) => Some(value.clone()),
        Value::String(value) => Some(Value::String(truncate_string(
            value,
            MAX_SUMMARY_STRING_CHARS,
        ))),
        Value::Array(items) => Some(Value::Array(
            items
                .iter()
                .take(MAX_ARRAY_ITEMS)
                .filter_map(summarize_safe_array_value)
                .collect(),
        )),
        Value::Object(_) => None,
    }
}

fn summarize_safe_array_value(value: &Value) -> Option<Value> {
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) => Some(value.clone()),
        Value::String(value) => Some(Value::String(truncate_string(
            value,
            MAX_SUMMARY_STRING_CHARS,
        ))),
        Value::Array(_) | Value::Object(_) => None,
    }
}

fn truncate_string(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

fn params_type(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn extract_error_code(response: &Value) -> Option<String> {
    response
        .get("error_code")
        .and_then(Value::as_str)
        .or_else(|| {
            response
                .get("error")
                .and_then(|error| error.get("error_code"))
                .and_then(Value::as_str)
        })
        .map(ToString::to_string)
}

pub(crate) fn extract_unique_ids(response: &Value) -> Vec<String> {
    let mut out = BTreeSet::new();
    collect_unique_ids(response, &mut out);
    out.into_iter().collect()
}

pub(crate) fn extract_top_unique_ids(response: &Value) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    if let Some(rows) = response.get("data").and_then(Value::as_array) {
        for row in rows {
            push_first_unique_id(row, &mut out, &mut seen);
            if out.len() >= MAX_TOP_UNIQUE_IDS {
                return out;
            }
        }
    }
    if out.is_empty() {
        collect_top_unique_ids(response, &mut out, &mut seen);
    }
    out
}

fn push_first_unique_id(value: &Value, out: &mut Vec<String>, seen: &mut BTreeSet<String>) {
    if let Some(id) = first_unique_id(value) {
        push_ordered_unique_id(&id, out, seen);
    }
}

fn collect_top_unique_ids(value: &Value, out: &mut Vec<String>, seen: &mut BTreeSet<String>) {
    if out.len() >= MAX_TOP_UNIQUE_IDS {
        return;
    }
    match value {
        Value::Object(map) => {
            if let Some(id) = first_unique_id(value) {
                push_ordered_unique_id(&id, out, seen);
                if out.len() >= MAX_TOP_UNIQUE_IDS {
                    return;
                }
            }
            for child in map.values() {
                collect_top_unique_ids(child, out, seen);
                if out.len() >= MAX_TOP_UNIQUE_IDS {
                    return;
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_top_unique_ids(item, out, seen);
                if out.len() >= MAX_TOP_UNIQUE_IDS {
                    return;
                }
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

fn first_unique_id(value: &Value) -> Option<String> {
    let Value::Object(map) = value else {
        return None;
    };
    for key in ["unique_id", "parent_unique_id", "root_id"] {
        if let Some(id) = map.get(key).and_then(Value::as_str)
            && !id.trim().is_empty()
        {
            return Some(truncate_string(id, MAX_UNIQUE_ID_CHARS));
        }
    }
    None
}

fn push_ordered_unique_id(id: &str, out: &mut Vec<String>, seen: &mut BTreeSet<String>) {
    if out.len() >= MAX_TOP_UNIQUE_IDS {
        return;
    }
    if seen.insert(id.to_string()) {
        out.push(id.to_string());
    }
}

fn collect_unique_ids(value: &Value, out: &mut BTreeSet<String>) {
    match value {
        Value::Object(map) => {
            if out.len() >= MAX_SELECTED_UNIQUE_IDS {
                return;
            }
            for key in ["unique_id", "parent_unique_id", "root_id"] {
                if let Some(id) = map.get(key).and_then(Value::as_str)
                    && !id.trim().is_empty()
                {
                    out.insert(truncate_string(id, MAX_UNIQUE_ID_CHARS));
                    if out.len() >= MAX_SELECTED_UNIQUE_IDS {
                        return;
                    }
                }
            }
            for child in map.values() {
                if out.len() >= MAX_SELECTED_UNIQUE_IDS {
                    return;
                }
                collect_unique_ids(child, out);
            }
        }
        Value::Array(items) => {
            for item in items {
                if out.len() >= MAX_SELECTED_UNIQUE_IDS {
                    return;
                }
                collect_unique_ids(item, out);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

fn timestamp_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis())
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use tempfile::{NamedTempFile, TempDir};

    use super::{
        MAX_SELECTED_UNIQUE_IDS, extract_response_truncated, extract_top_unique_ids,
        extract_unique_ids, read_tool_trace_file, redact_tool_trace_file, summarize_params,
        summarize_tool_trace,
    };

    #[test]
    fn summarize_params_keeps_only_safe_context() {
        let summary = summarize_params(&json!({
            "query": "gmv",
            "parameters": {"token": "secret"},
            "roles": ["dimension"],
            "include_upstream": false,
            "lineage_depth": 2,
            "limit": 5
        }));
        assert_eq!(summary["query"], "gmv");
        assert_eq!(summary["roles"], json!(["dimension"]));
        assert_eq!(summary["include_upstream"], false);
        assert_eq!(summary["lineage_depth"], 2);
        assert_eq!(summary["limit"], 5);
        assert!(summary.get("parameters").is_none());
    }

    #[test]
    fn summarize_params_drops_nested_values_from_safe_arrays() {
        let summary = summarize_params(&json!({
            "resource_types": ["model", {"token": "secret"}]
        }));
        assert_eq!(summary["resource_types"], json!(["model"]));
    }

    #[test]
    fn summarize_params_records_statement_structure_without_raw_sql() {
        let summary = summarize_params(&json!({
            "statement": "select country from analytics.orders where order_date = '2026-03-01'",
            "row_limit": 10
        }));

        assert_eq!(summary["row_limit"], 10);
        assert_eq!(
            summary["statement_structure"]["tables"],
            json!(["analytics.orders"])
        );
        assert_eq!(
            summary["statement_structure"]["filters"],
            json!(["order_date = ?"])
        );
        assert!(summary.get("statement").is_none());
        assert!(!summary.to_string().contains("2026-03-01"));
    }

    #[test]
    fn extract_unique_ids_finds_nested_ids() {
        let ids = extract_unique_ids(&json!({
            "data": [{"unique_id": "model.pkg.orders", "parent_unique_id": "model.pkg.parent"}],
            "root_id": "model.pkg.root"
        }));
        assert_eq!(
            ids,
            vec![
                "model.pkg.orders".to_string(),
                "model.pkg.parent".to_string(),
                "model.pkg.root".to_string()
            ]
        );
    }

    #[test]
    fn extract_unique_ids_caps_large_responses() {
        let data: Vec<_> = (0..(MAX_SELECTED_UNIQUE_IDS + 5))
            .map(|index| json!({"unique_id": format!("model.pkg.entity_{index}")}))
            .collect();
        let ids = extract_unique_ids(&json!({"data": data}));
        assert_eq!(ids.len(), MAX_SELECTED_UNIQUE_IDS);
    }

    #[test]
    fn extract_top_unique_ids_preserves_response_order() {
        let ids = extract_top_unique_ids(&json!({
            "data": [
                {"unique_id": "model.pkg.first"},
                {"unique_id": "model.pkg.second", "parent_unique_id": "model.pkg.parent"}
            ],
            "root_id": "model.pkg.root"
        }));
        assert_eq!(
            ids,
            vec![
                "model.pkg.first".to_string(),
                "model.pkg.second".to_string()
            ]
        );
    }

    #[test]
    fn extract_response_truncated_finds_nested_sql_truncation() {
        assert!(extract_response_truncated(&json!({
            "success": true,
            "data": {
                "rows": [],
                "truncated": true
            }
        })));
    }

    #[test]
    fn read_tool_trace_file_reports_parse_warnings() {
        let trace = NamedTempFile::new().expect("trace");
        std::fs::write(
            trace.path(),
            "{\"tool\":\"search\",\"tool_call_index\":99}\nnot-json\n{\"tool\":\"get_context\"}\n",
        )
        .expect("write trace");

        let read = read_tool_trace_file(trace.path());

        assert!(!read.missing);
        assert_eq!(read.rows.len(), 2);
        assert_eq!(read.parse_warnings.len(), 1);
        assert_eq!(read.parse_warnings[0].line, 2);
        assert_eq!(read.rows[0]["tool_call_index"], json!(0));
        assert_eq!(read.rows[1]["tool_call_index"], json!(1));
    }

    #[test]
    fn summarize_tool_trace_reports_order_budgets_and_semantic_first() {
        let trace = NamedTempFile::new().expect("trace");
        std::fs::write(
            trace.path(),
            concat!(
                "{\"tool\":\"search_indicator\",\"success\":true,\"duration_ms\":12,",
                "\"response_bytes\":40,\"response_truncated\":false,",
                "\"selected_unique_ids\":[\"model.pkg.orders\"],",
                "\"top_unique_ids\":[\"model.pkg.orders\"],",
                "\"params_summary\":{\"query\":\"total revenue\"}}\n",
                "{\"tool\":\"execute_sql\",\"success\":false,\"duration_ms\":30,",
                "\"response_bytes\":80,\"response_truncated\":true,",
                "\"error_code\":\"SQL_ERROR\"}\n"
            ),
        )
        .expect("write trace");

        let read = read_tool_trace_file(trace.path());
        let summary = summarize_tool_trace(&read);

        assert_eq!(summary.row_count, 2);
        assert_eq!(summary.tool_order[0].tool, "search_indicator");
        assert_eq!(summary.tool_counts["execute_sql"], 1);
        assert_eq!(summary.response_budget.total_response_bytes, 120);
        assert_eq!(summary.truncation.response_truncated_count, 1);
        assert_eq!(summary.errors.failed_tool_call_count, 1);
        assert_eq!(summary.errors.error_codes["SQL_ERROR"], 1);
        assert_eq!(summary.selected_unique_ids, vec!["model.pkg.orders"]);
        assert_eq!(summary.semantic_first.status, "pass");
    }

    #[test]
    fn redact_tool_trace_file_drops_sensitive_fields_and_keeps_summary_evidence() {
        let trace = NamedTempFile::new().expect("trace");
        std::fs::write(
            trace.path(),
            concat!(
                "{\"tool\":\"execute_sql\",\"success\":true,\"duration_ms\":1,",
                "\"params\":{\"parameters\":{\"token\":\"secret\"}},",
                "\"params_summary\":{\"query\":\"s3://bucket/path?token=secret\",",
                "\"limit\":5,\"keys\":[\"query\",\"token\"],",
                "\"parameters\":{\"password\":\"secret\"}},",
                "\"manifest_uri\":\"s3://bucket/manifest.json?token=secret\",",
                "\"provider_raw_output\":\"secret stdout\",",
                "\"response_bytes\":10,\"response_truncated\":false,",
                "\"selected_unique_ids\":[\"model.pkg.orders\"],",
                "\"top_unique_ids\":[\"model.pkg.orders\"]}\n",
                "not-json\n"
            ),
        )
        .expect("write trace");
        let dir = TempDir::new().expect("dir");
        let out = dir.path().join("trace.redacted.jsonl");

        let report = redact_tool_trace_file(trace.path(), &out).expect("redact");

        assert_eq!(report.rows_written, 1);
        assert_eq!(report.malformed_rows, 1);
        assert!(report.fields_removed >= 4);
        assert!(report.fields_masked >= 1);

        let redacted = std::fs::read_to_string(out).expect("redacted");
        assert!(!redacted.contains("secret"));
        assert!(!redacted.contains("provider_raw_output"));
        assert!(!redacted.contains("parameters"));
        assert!(redacted.contains("[REDACTED]"));
        assert!(redacted.contains("model.pkg.orders"));

        let read = read_tool_trace_file(dir.path().join("trace.redacted.jsonl").as_path());
        let summary = summarize_tool_trace(&read);
        assert_eq!(summary.row_count, 1);
        assert_eq!(summary.tool_order[0].tool, "execute_sql");
    }

    #[test]
    fn redact_tool_trace_file_rejects_same_canonical_output_path() {
        let dir = TempDir::new().expect("dir");
        let trace = dir.path().join("trace.jsonl");
        std::fs::write(&trace, "{\"tool\":\"search\"}\n").expect("write trace");
        let out = dir.path().join(".").join("trace.jsonl");

        let error = redact_tool_trace_file(&trace, &out).expect_err("same canonical path");

        assert!(error.to_string().contains("--out must be different"));
    }
}
