use super::*;

pub(super) fn resolve_eval_results_path(
    raw_path: &str,
    label: &str,
) -> crate::error::Result<PathBuf> {
    let trimmed = raw_path.trim();
    if trimmed.is_empty() {
        return Err(DbtNovaError::InvalidParams(format!(
            "{label} must not be empty"
        )));
    }
    resolve_eval_results_candidate(&PathBuf::from(trimmed), label)
}

pub(super) fn resolve_mcp_eval_results_path(
    raw_path: &str,
    label: &str,
) -> crate::error::Result<PathBuf> {
    let (root, candidate) = mcp_eval_candidate_path(raw_path, label)?;
    let results_path = resolve_eval_results_candidate(&candidate, label)?;
    ensure_mcp_eval_path_under_root(&results_path, &root, label)?;
    Ok(results_path)
}

pub(super) fn resolve_eval_results_candidate(
    candidate: &Path,
    label: &str,
) -> crate::error::Result<PathBuf> {
    let results_path = if candidate.is_dir() {
        candidate.join("results.json")
    } else {
        candidate.to_path_buf()
    };
    if !results_path.exists() {
        return Err(DbtNovaError::InvalidParams(format!(
            "{label} eval results '{}' do not exist; pass a result directory or results.json path",
            results_path.display()
        )));
    }
    let canonical = results_path.canonicalize().map_err(|error| {
        DbtNovaError::InvalidParams(format!(
            "failed to resolve {label} eval results '{}': {error}",
            results_path.display()
        ))
    })?;
    if !canonical.is_file() {
        return Err(DbtNovaError::InvalidParams(format!(
            "{label} eval results '{}' is not a file",
            canonical.display()
        )));
    }
    Ok(canonical)
}

pub(super) fn build_eval_comparison_report(
    before_results: &Path,
    after_results: &Path,
    trace_root: Option<&Path>,
) -> crate::error::Result<EvalComparisonReport> {
    let before_json = read_eval_results_json(before_results)?;
    let after_json = read_eval_results_json(after_results)?;
    let before = summarize_eval_results(&before_json, before_results, trace_root)?;
    let after = summarize_eval_results(&after_json, after_results, trace_root)?;
    let delta = compare_eval_summaries(&before, &after);
    let mut report = EvalComparisonReport {
        schema_version: "eval_comparison.v1",
        before,
        after,
        delta,
        markdown: String::new(),
    };
    report.markdown = render_eval_comparison_markdown(&report);
    Ok(report)
}

pub(super) fn read_eval_results_json(path: &Path) -> crate::error::Result<JsonValue> {
    let raw = fs::read_to_string(path).map_err(|error| {
        DbtNovaError::ServerError(format!(
            "failed to read eval results '{}': {error}",
            path.display()
        ))
    })?;
    serde_json::from_str(&raw).map_err(|error| {
        DbtNovaError::InvalidParams(format!(
            "failed to parse eval results '{}': {error}",
            path.display()
        ))
    })
}

pub(super) fn summarize_eval_results(
    report: &JsonValue,
    results_path: &Path,
    trace_root: Option<&Path>,
) -> crate::error::Result<EvalComparisonRunSummary> {
    let results_dir = results_path.parent().unwrap_or_else(|| Path::new("."));
    let suite_name = required_json_string(report, "suite_name")?;
    let mode = required_json_string(report, "mode")?;
    let output_dir = optional_json_string(report, "output_dir")
        .unwrap_or_else(|| results_dir.display().to_string());
    let gate_status =
        optional_json_string(report, "gate_status").unwrap_or_else(|| "unknown".to_string());
    let cases = report
        .get("cases")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| DbtNovaError::InvalidParams("eval results missing cases[]".to_string()))?;

    let mut pass_count_sum = 0usize;
    let mut fail_count_sum = 0usize;
    let mut error_count_sum = 0usize;
    let mut case_statuses = BTreeMap::new();
    let mut passing_cases = Vec::new();
    let mut failing_cases = Vec::new();
    let mut metrics = EvalComparisonMetrics::default();
    let mut distinct_tools = BTreeSet::new();
    let mut trace_case_count = 0usize;
    let mut warnings = Vec::new();

    for (index, case) in cases.iter().enumerate() {
        let id = case
            .get("id")
            .and_then(JsonValue::as_str)
            .map_or_else(|| format!("case_{}", index + 1), str::to_string);
        let pass_count = json_usize_or_else(case, "pass_count", || {
            count_assertions_with_status(case, "pass")
        });
        let fail_count = json_usize_or_else(case, "fail_count", || {
            count_assertions_with_status(case, "fail")
        });
        let error_count = json_usize_or_else(case, "error_count", || {
            count_assertions_with_status(case, "error")
        });
        pass_count_sum = pass_count_sum.saturating_add(pass_count);
        fail_count_sum = fail_count_sum.saturating_add(fail_count);
        error_count_sum = error_count_sum.saturating_add(error_count);

        let status = case_status(pass_count, fail_count, error_count);
        if status == "pass" {
            passing_cases.push(id.clone());
        } else {
            failing_cases.push(id.clone());
        }
        case_statuses.insert(id.clone(), status);

        if let Some(trace) = read_case_trace_metrics(case, results_dir, trace_root, &mut warnings) {
            trace_case_count += 1;
            metrics.add_trace(&trace.metrics);
            distinct_tools.extend(trace.distinct_tools);
        }
    }

    if !distinct_tools.is_empty() {
        metrics.distinct_tool_count = Some(usize_to_u64(distinct_tools.len()));
    }

    let pass_count = json_usize_or(report, "pass_count", pass_count_sum);
    let fail_count = json_usize_or(report, "fail_count", fail_count_sum);
    let error_count = json_usize_or(report, "error_count", error_count_sum);
    let assertion_count = json_usize_or(
        report,
        "assertion_count",
        pass_count
            .saturating_add(fail_count)
            .saturating_add(error_count),
    );
    let pass_rate = optional_json_f64(report, "pass_rate").unwrap_or_else(|| {
        if assertion_count == 0 {
            0.0
        } else {
            ratio(pass_count, assertion_count)
        }
    });

    Ok(EvalComparisonRunSummary {
        results_path: results_path.display().to_string(),
        suite_name,
        mode,
        output_dir,
        gate_status,
        case_count: cases.len(),
        assertion_count,
        pass_count,
        fail_count,
        error_count,
        pass_rate,
        passing_cases,
        failing_cases,
        case_statuses,
        metrics,
        trace_case_count,
        warnings,
    })
}

pub(super) fn read_case_trace_metrics(
    case: &JsonValue,
    results_dir: &Path,
    trace_root: Option<&Path>,
    warnings: &mut Vec<String>,
) -> Option<EvalComparisonCaseTrace> {
    let case_id = case
        .get("id")
        .and_then(JsonValue::as_str)
        .unwrap_or("unknown");
    let trace_path = case
        .get("artifacts")
        .and_then(|artifacts| artifacts.get("tool_trace"))
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty())?;
    let resolved = resolve_case_trace_path(trace_path, results_dir, trace_root, warnings)?;
    let read = read_tool_trace_file(&resolved);
    if read.missing {
        warnings.push(format!(
            "case '{case_id}' trace artifact '{}' was not found",
            resolved.display()
        ));
        return None;
    }
    if let Some(error) = read.read_error.as_ref() {
        warnings.push(format!(
            "case '{case_id}' trace artifact read failed: {error}"
        ));
        return None;
    }
    if !read.parse_warnings.is_empty() {
        warnings.push(format!(
            "case '{case_id}' trace artifact had {} malformed row(s)",
            read.parse_warnings.len()
        ));
    }
    let telemetry = eval_case_telemetry_from_trace(&read.rows);
    let mut distinct_tools = BTreeSet::new();
    for row in &read.rows {
        if let Some(tool) = row.get("tool").and_then(JsonValue::as_str) {
            distinct_tools.insert(tool.to_string());
        }
    }
    Some(EvalComparisonCaseTrace {
        metrics: EvalComparisonMetrics {
            tool_call_count: Some(usize_to_u64(telemetry.tool_call_count)),
            distinct_tool_count: Some(usize_to_u64(telemetry.distinct_tool_count)),
            total_response_bytes: telemetry.total_response_bytes,
            duration_ms: sum_json_u64_field(&read.rows, "duration_ms"),
            input_tokens: telemetry.input_tokens,
            output_tokens: telemetry.output_tokens,
            total_tokens: telemetry.total_tokens,
        },
        distinct_tools,
    })
}

pub(super) fn resolve_case_trace_path(
    raw_path: &str,
    results_dir: &Path,
    trace_root: Option<&Path>,
    warnings: &mut Vec<String>,
) -> Option<PathBuf> {
    let mut blocked_paths = Vec::new();
    for candidate in trace_path_candidates(raw_path, results_dir) {
        if !candidate.exists() || !candidate.is_file() {
            continue;
        }
        let Ok(canonical) = candidate.canonicalize() else {
            continue;
        };
        if let Some(root) = trace_root
            && let Err(error) = ensure_mcp_eval_path_under_root(&canonical, root, "tool_trace")
        {
            blocked_paths.push(format!("{} ({error})", canonical.display()));
            continue;
        }
        return Some(canonical);
    }
    if blocked_paths.is_empty() {
        warnings.push(format!(
            "trace artifact '{raw_path}' was referenced but could not be resolved"
        ));
    } else {
        warnings.push(format!(
            "trace artifact path was skipped by MCP path policy: {}",
            blocked_paths.join(", ")
        ));
    }
    None
}

pub(super) fn trace_path_candidates(raw_path: &str, results_dir: &Path) -> BTreeSet<PathBuf> {
    let mut candidates = BTreeSet::new();
    let path = PathBuf::from(raw_path);
    candidates.insert(path.clone());
    if path.is_absolute() {
        return candidates;
    }
    candidates.insert(results_dir.join(&path));
    if let Some(file_name) = path.file_name() {
        candidates.insert(results_dir.join("tool-calls").join(file_name));
    }
    if let Some(results_dir_name) = results_dir.file_name()
        && let Some(stripped) = strip_first_component(&path, results_dir_name)
    {
        candidates.insert(results_dir.join(stripped));
    }
    candidates
}

pub(super) fn strip_first_component(path: &Path, expected: &std::ffi::OsStr) -> Option<PathBuf> {
    let mut components = path.components();
    match components.next()? {
        Component::Normal(first) if first == expected => {}
        _ => return None,
    }
    let mut stripped = PathBuf::new();
    for component in components {
        stripped.push(component.as_os_str());
    }
    (!stripped.as_os_str().is_empty()).then_some(stripped)
}

pub(super) fn compare_eval_summaries(
    before: &EvalComparisonRunSummary,
    after: &EvalComparisonRunSummary,
) -> EvalComparisonDelta {
    let mut newly_passing_cases = Vec::new();
    let mut newly_failing_cases = Vec::new();
    let mut unchanged_failing_cases = Vec::new();
    let mut added_cases = Vec::new();
    let mut removed_cases = Vec::new();
    let mut changed_cases = Vec::new();
    let case_ids = before
        .case_statuses
        .keys()
        .chain(after.case_statuses.keys())
        .cloned()
        .collect::<BTreeSet<_>>();

    for id in case_ids {
        let before_status = before.case_statuses.get(&id).cloned();
        let after_status = after.case_statuses.get(&id).cloned();
        if before_status != after_status {
            changed_cases.push(EvalCaseStatusDelta {
                id: id.clone(),
                before: before_status.clone(),
                after: after_status.clone(),
            });
        }
        match (before_status.as_deref(), after_status.as_deref()) {
            (None, Some(status)) => {
                added_cases.push(id.clone());
                if status == "pass" {
                    newly_passing_cases.push(id);
                } else {
                    newly_failing_cases.push(id);
                }
            }
            (Some(_), None) => removed_cases.push(id),
            (Some(before_status), Some("pass")) if before_status != "pass" => {
                newly_passing_cases.push(id);
            }
            (Some("pass"), Some(after_status)) if after_status != "pass" => {
                newly_failing_cases.push(id);
            }
            (Some(status), Some(_)) if status != "pass" => unchanged_failing_cases.push(id),
            _ => {}
        }
    }

    EvalComparisonDelta {
        suite_name_changed: before.suite_name != after.suite_name,
        mode_changed: before.mode != after.mode,
        pass_rate: after.pass_rate - before.pass_rate,
        pass_rate_percentage_points: (after.pass_rate - before.pass_rate) * 100.0,
        case_count: usize_delta(after.case_count, before.case_count),
        assertion_count: usize_delta(after.assertion_count, before.assertion_count),
        pass_count: usize_delta(after.pass_count, before.pass_count),
        fail_count: usize_delta(after.fail_count, before.fail_count),
        error_count: usize_delta(after.error_count, before.error_count),
        metrics: EvalComparisonMetricDeltas::between(&before.metrics, &after.metrics),
        newly_passing_cases,
        newly_failing_cases,
        unchanged_failing_cases,
        added_cases,
        removed_cases,
        changed_cases,
    }
}

pub(super) fn render_eval_comparison_markdown(report: &EvalComparisonReport) -> String {
    let mut out = String::new();
    out.push_str("# Nova Eval Comparison\n\n");
    render_eval_comparison_summary(&mut out, report);
    render_eval_comparison_metrics(&mut out, report);
    render_eval_comparison_case_changes(&mut out, report);
    render_eval_comparison_notes(&mut out, report);
    out
}

fn render_eval_comparison_summary(out: &mut String, report: &EvalComparisonReport) {
    out.push_str("## Summary\n\n");
    let _ = writeln!(
        out,
        "- Suite: `{}` -> `{}`",
        report.before.suite_name, report.after.suite_name
    );
    let _ = writeln!(
        out,
        "- Mode: `{}` -> `{}`",
        report.before.mode, report.after.mode
    );
    let _ = writeln!(
        out,
        "- Results: `{}` -> `{}`",
        report.before.results_path, report.after.results_path
    );
}

fn render_eval_comparison_metrics(out: &mut String, report: &EvalComparisonReport) {
    out.push('\n');
    out.push_str("| Metric | Before | After | Delta |\n");
    out.push_str("|---|---:|---:|---:|\n");
    let _ = writeln!(
        out,
        "| Pass rate | {} | {} | {} |",
        format_percent(report.before.pass_rate),
        format_percent(report.after.pass_rate),
        format_signed_percentage_points(report.delta.pass_rate_percentage_points)
    );
    let _ = writeln!(
        out,
        "| Cases | {} | {} | {} |",
        report.before.case_count,
        report.after.case_count,
        format_signed_i64(report.delta.case_count)
    );
    let _ = writeln!(
        out,
        "| Assertions | {} | {} | {} |",
        report.before.assertion_count,
        report.after.assertion_count,
        format_signed_i64(report.delta.assertion_count)
    );
    let _ = writeln!(
        out,
        "| Passed assertions | {} | {} | {} |",
        report.before.pass_count,
        report.after.pass_count,
        format_signed_i64(report.delta.pass_count)
    );
    let _ = writeln!(
        out,
        "| Failed assertions | {} | {} | {} |",
        report.before.fail_count,
        report.after.fail_count,
        format_signed_i64(report.delta.fail_count)
    );
    let _ = writeln!(
        out,
        "| Error assertions | {} | {} | {} |",
        report.before.error_count,
        report.after.error_count,
        format_signed_i64(report.delta.error_count)
    );
    render_metric_row(
        out,
        "Tool calls",
        report.before.metrics.tool_call_count,
        report.after.metrics.tool_call_count,
        report.delta.metrics.tool_call_count,
    );
    render_metric_row(
        out,
        "Duration ms",
        report.before.metrics.duration_ms,
        report.after.metrics.duration_ms,
        report.delta.metrics.duration_ms,
    );
    render_metric_row(
        out,
        "Input tokens",
        report.before.metrics.input_tokens,
        report.after.metrics.input_tokens,
        report.delta.metrics.input_tokens,
    );
    render_metric_row(
        out,
        "Output tokens",
        report.before.metrics.output_tokens,
        report.after.metrics.output_tokens,
        report.delta.metrics.output_tokens,
    );
    render_metric_row(
        out,
        "Total tokens",
        report.before.metrics.total_tokens,
        report.after.metrics.total_tokens,
        report.delta.metrics.total_tokens,
    );
    render_metric_row(
        out,
        "Response bytes",
        report.before.metrics.total_response_bytes,
        report.after.metrics.total_response_bytes,
        report.delta.metrics.total_response_bytes,
    );
}

fn render_eval_comparison_case_changes(out: &mut String, report: &EvalComparisonReport) {
    out.push_str("\n## Case Changes\n\n");
    render_case_list(out, "Newly passing", &report.delta.newly_passing_cases);
    render_case_list(out, "Newly failing", &report.delta.newly_failing_cases);
    render_case_list(out, "Still failing", &report.delta.unchanged_failing_cases);
    render_case_list(out, "Added", &report.delta.added_cases);
    render_case_list(out, "Removed", &report.delta.removed_cases);
    if report.delta.changed_cases.is_empty() {
        out.push_str("- Status changes: None.\n");
    } else {
        out.push_str("- Status changes:\n");
        for change in &report.delta.changed_cases {
            let before = change.before.as_deref().unwrap_or("missing");
            let after = change.after.as_deref().unwrap_or("missing");
            let _ = writeln!(out, "  - `{}`: `{}` -> `{}`", change.id, before, after);
        }
    }
}

fn render_eval_comparison_notes(out: &mut String, report: &EvalComparisonReport) {
    out.push_str("\n## Notes\n\n");
    if report.delta.suite_name_changed {
        out.push_str("- Suite names differ; confirm this is an intentional comparison.\n");
    }
    if report.delta.mode_changed {
        out.push_str("- Eval modes differ; compare bridge and agent runs carefully.\n");
    }
    if report.delta.pass_rate == 0.0
        && report.delta.newly_passing_cases.is_empty()
        && report.delta.newly_failing_cases.is_empty()
        && report.delta.changed_cases.is_empty()
    {
        out.push_str("- No pass-rate or case-status change was observed.\n");
    }
    if report.before.trace_case_count == 0 && report.after.trace_case_count == 0 {
        out.push_str("- No agent trace artifacts were available, so tool-call and token deltas are omitted.\n");
    }
    render_warning_list(out, "Before warnings", &report.before.warnings);
    render_warning_list(out, "After warnings", &report.after.warnings);
}

pub(super) fn render_metric_row(
    out: &mut String,
    label: &str,
    before: Option<u64>,
    after: Option<u64>,
    delta: Option<i64>,
) {
    if before.is_none() && after.is_none() {
        return;
    }
    let _ = writeln!(
        out,
        "| {label} | {} | {} | {} |",
        format_optional_u64(before),
        format_optional_u64(after),
        delta.map_or_else(|| "n/a".to_string(), format_signed_i64)
    );
}

pub(super) fn render_case_list(out: &mut String, label: &str, cases: &[String]) {
    if cases.is_empty() {
        let _ = writeln!(out, "- {label}: None.");
    } else {
        let _ = writeln!(out, "- {label}: {}", backtick_join(cases));
    }
}

pub(super) fn render_warning_list(out: &mut String, label: &str, warnings: &[String]) {
    if warnings.is_empty() {
        return;
    }
    let _ = writeln!(out, "- {label}:");
    for warning in warnings {
        let _ = writeln!(out, "  - {warning}");
    }
}

pub(super) fn backtick_join(values: &[String]) -> String {
    values
        .iter()
        .map(|value| format!("`{value}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

pub(super) fn format_percent(value: f64) -> String {
    format!("{:.1}%", value * 100.0)
}

pub(super) fn format_signed_percentage_points(value: f64) -> String {
    if value >= 0.0 {
        format!("+{value:.1} pp")
    } else {
        format!("{value:.1} pp")
    }
}

pub(super) fn format_signed_i64(value: i64) -> String {
    if value >= 0 {
        format!("+{value}")
    } else {
        value.to_string()
    }
}

pub(super) fn format_optional_u64(value: Option<u64>) -> String {
    value.map_or_else(|| "n/a".to_string(), |value| value.to_string())
}

pub(super) fn required_json_string(value: &JsonValue, field: &str) -> crate::error::Result<String> {
    value
        .get(field)
        .and_then(JsonValue::as_str)
        .map(str::to_string)
        .ok_or_else(|| DbtNovaError::InvalidParams(format!("eval results missing {field}")))
}

pub(super) fn optional_json_string(value: &JsonValue, field: &str) -> Option<String> {
    value
        .get(field)
        .and_then(JsonValue::as_str)
        .map(str::to_string)
}

pub(super) fn optional_json_f64(value: &JsonValue, field: &str) -> Option<f64> {
    value.get(field).and_then(JsonValue::as_f64)
}

pub(super) fn json_usize_or(value: &JsonValue, field: &str, default: usize) -> usize {
    json_usize(value, field).unwrap_or(default)
}

pub(super) fn json_usize_or_else(
    value: &JsonValue,
    field: &str,
    default: impl FnOnce() -> usize,
) -> usize {
    json_usize(value, field).unwrap_or_else(default)
}

pub(super) fn json_usize(value: &JsonValue, field: &str) -> Option<usize> {
    value
        .get(field)
        .and_then(JsonValue::as_u64)
        .and_then(|value| usize::try_from(value).ok())
}

pub(super) fn count_assertions_with_status(case: &JsonValue, status: &str) -> usize {
    case.get("assertions")
        .and_then(JsonValue::as_array)
        .map_or(0, |assertions| {
            assertions
                .iter()
                .filter(|assertion| {
                    assertion.get("status").and_then(JsonValue::as_str) == Some(status)
                })
                .count()
        })
}

pub(super) fn case_status(pass_count: usize, fail_count: usize, error_count: usize) -> String {
    if error_count > 0 {
        "error"
    } else if fail_count > 0 {
        "fail"
    } else if pass_count > 0 {
        "pass"
    } else {
        "empty"
    }
    .to_string()
}

pub(super) fn sum_json_u64_field(rows: &[JsonValue], field: &str) -> Option<u64> {
    let mut seen = false;
    let mut total = 0_u64;
    for row in rows {
        if let Some(value) = row.get(field).and_then(JsonValue::as_u64) {
            seen = true;
            total = total.saturating_add(value);
        }
    }
    seen.then_some(total)
}

pub(super) fn usize_to_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

pub(super) fn usize_delta(after: usize, before: usize) -> i64 {
    u64_delta(usize_to_u64(after), usize_to_u64(before))
}

pub(super) fn optional_u64_delta(after: Option<u64>, before: Option<u64>) -> Option<i64> {
    Some(u64_delta(after?, before?))
}

pub(super) fn u64_delta(after: u64, before: u64) -> i64 {
    let after = i64::try_from(after).unwrap_or(i64::MAX);
    let before = i64::try_from(before).unwrap_or(i64::MAX);
    after.saturating_sub(before)
}

impl EvalComparisonMetrics {
    pub(super) fn is_empty(&self) -> bool {
        self.tool_call_count.is_none()
            && self.distinct_tool_count.is_none()
            && self.total_response_bytes.is_none()
            && self.duration_ms.is_none()
            && self.input_tokens.is_none()
            && self.output_tokens.is_none()
            && self.total_tokens.is_none()
    }

    pub(super) fn add_trace(&mut self, metrics: &Self) {
        add_optional_u64(&mut self.tool_call_count, metrics.tool_call_count);
        add_optional_u64(&mut self.total_response_bytes, metrics.total_response_bytes);
        add_optional_u64(&mut self.duration_ms, metrics.duration_ms);
        add_optional_u64(&mut self.input_tokens, metrics.input_tokens);
        add_optional_u64(&mut self.output_tokens, metrics.output_tokens);
        add_optional_u64(&mut self.total_tokens, metrics.total_tokens);
    }
}

impl EvalComparisonMetricDeltas {
    pub(super) fn between(before: &EvalComparisonMetrics, after: &EvalComparisonMetrics) -> Self {
        Self {
            tool_call_count: optional_u64_delta(after.tool_call_count, before.tool_call_count),
            distinct_tool_count: optional_u64_delta(
                after.distinct_tool_count,
                before.distinct_tool_count,
            ),
            total_response_bytes: optional_u64_delta(
                after.total_response_bytes,
                before.total_response_bytes,
            ),
            duration_ms: optional_u64_delta(after.duration_ms, before.duration_ms),
            input_tokens: optional_u64_delta(after.input_tokens, before.input_tokens),
            output_tokens: optional_u64_delta(after.output_tokens, before.output_tokens),
            total_tokens: optional_u64_delta(after.total_tokens, before.total_tokens),
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.tool_call_count.is_none()
            && self.distinct_tool_count.is_none()
            && self.total_response_bytes.is_none()
            && self.duration_ms.is_none()
            && self.input_tokens.is_none()
            && self.output_tokens.is_none()
            && self.total_tokens.is_none()
    }
}

pub(super) struct EvalComparisonCaseTrace {
    pub(super) metrics: EvalComparisonMetrics,
    pub(super) distinct_tools: BTreeSet<String>,
}

pub(super) fn add_optional_u64(target: &mut Option<u64>, value: Option<u64>) {
    if let Some(value) = value {
        *target = Some(target.unwrap_or(0).saturating_add(value));
    }
}
