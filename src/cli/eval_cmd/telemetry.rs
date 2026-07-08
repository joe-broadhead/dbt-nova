use super::{
    AssertionResult, BTreeSet, DEFAULT_TELEMETRY_DIR, DateAnchor, DbtNovaError, DispatchResult,
    EvalAgentRunArgs, EvalCaseReport, EvalCaseTelemetry, EvalReport, EvalSuite,
    EvalTelemetryRunContext, IoWrite, JsonValue, OpenOptions, Path, PathBuf, StdCommand,
    SystemTime, UNIX_EPOCH, fs, json, safe_path_segment, server_error,
};

pub(super) fn write_eval_telemetry(
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
    let run_values = TelemetryRunValues {
        timestamp: &timestamp,
        timestamp_ms,
        run_id: &run_id,
        git_sha: git_sha.as_deref(),
        manifest_hash: manifest_hash.as_deref(),
    };

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&telemetry_path)
        .map_err(|error| server_error(error.to_string()))?;
    for case in &report.cases {
        for assertion in &case.assertions {
            let row = eval_telemetry_row(report, context, &run_values, case, assertion);
            write_telemetry_row(&mut file, row)?;
        }
    }
    drop(file);

    if let Some(max_rows) = context.retention {
        apply_telemetry_retention(&telemetry_path, max_rows)?;
    }
    Ok(())
}

struct TelemetryRunValues<'a> {
    timestamp: &'a str,
    timestamp_ms: u64,
    run_id: &'a str,
    git_sha: Option<&'a str>,
    manifest_hash: Option<&'a str>,
}

fn eval_telemetry_row(
    report: &EvalReport,
    context: EvalTelemetryRunContext<'_>,
    run_values: &TelemetryRunValues<'_>,
    case: &EvalCaseReport,
    assertion: &AssertionResult,
) -> serde_json::Map<String, JsonValue> {
    let mut row = serde_json::Map::new();
    row.insert("timestamp".to_string(), json!(run_values.timestamp));
    row.insert("timestamp_ms".to_string(), json!(run_values.timestamp_ms));
    row.insert("run_id".to_string(), json!(run_values.run_id));
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
        json!(telemetry_grade_mode(report.mode, &assertion.name)),
    );
    row.insert("duration_ms".to_string(), json!(context.duration_ms));
    row.insert("output_dir".to_string(), json!(&report.output_dir));
    insert_optional_run_telemetry(&mut row, run_values);
    insert_agent_telemetry(&mut row, context, case.telemetry.as_ref());
    if let Some(date_anchor) = case.date_anchor.as_ref() {
        insert_date_anchor_telemetry(&mut row, date_anchor);
    }
    row
}

fn insert_optional_run_telemetry(
    row: &mut serde_json::Map<String, JsonValue>,
    run_values: &TelemetryRunValues<'_>,
) {
    if let Some(manifest_hash) = run_values.manifest_hash {
        row.insert("manifest_hash".to_string(), json!(manifest_hash));
    }
    if let Some(git_sha) = run_values.git_sha {
        row.insert("git_sha".to_string(), json!(git_sha));
    }
}

fn insert_agent_telemetry(
    row: &mut serde_json::Map<String, JsonValue>,
    context: EvalTelemetryRunContext<'_>,
    telemetry: Option<&EvalCaseTelemetry>,
) {
    let Some(agent) = context.agent else {
        return;
    };
    row.insert("provider".to_string(), json!(agent.provider));
    row.insert(
        "provider_command_preset".to_string(),
        json!(agent.provider_command_preset),
    );
    let Some(telemetry) = telemetry else {
        return;
    };
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

fn write_telemetry_row(
    file: &mut fs::File,
    row: serde_json::Map<String, JsonValue>,
) -> DispatchResult {
    let line = serde_json::to_string(&JsonValue::Object(row))
        .map_err(|error| server_error(error.to_string()))?;
    file.write_all(line.as_bytes())
        .map_err(|error| server_error(error.to_string()))?;
    file.write_all(b"\n")
        .map_err(|error| server_error(error.to_string()))?;
    Ok(())
}

pub(super) fn apply_telemetry_retention(path: &Path, max_rows: usize) -> DispatchResult {
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

pub(super) fn telemetry_path_for_suite(suite_name: &str) -> PathBuf {
    PathBuf::from(DEFAULT_TELEMETRY_DIR).join(format!(
        "{}-{:016x}.jsonl",
        safe_path_segment(suite_name),
        stable_telemetry_hash(suite_name)
    ))
}

pub(super) fn stable_telemetry_hash(value: &str) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0001_0000_01b3);
    }
    hash
}

pub(super) fn insert_date_anchor_telemetry(
    row: &mut serde_json::Map<String, JsonValue>,
    anchor: &DateAnchor,
) {
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

pub(super) fn telemetry_row_matches_since(row: &JsonValue, since_boundary: &str) -> bool {
    row.get("timestamp")
        .and_then(JsonValue::as_str)
        .is_some_and(|timestamp| timestamp >= since_boundary)
}

pub(super) fn validate_since_date(value: &str) -> crate::error::Result<String> {
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

pub(super) fn validate_telemetry_retention(value: Option<usize>) -> crate::error::Result<()> {
    if value == Some(0) {
        return Err(DbtNovaError::InvalidParams(
            "--telemetry-retention must be greater than zero".to_string(),
        ));
    }
    Ok(())
}

pub(super) fn validate_telemetry_suite_name(
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

pub(super) fn telemetry_grade_mode(mode: &str, assertion_name: &str) -> &'static str {
    if assertion_type(assertion_name) == "sql_structure" {
        return "query_structure";
    }
    match mode {
        "agent" => "provider_trace",
        _ => "deterministic",
    }
}

pub(super) fn assertion_type(name: &str) -> &str {
    name.split_once(':').map_or(name, |(prefix, _)| prefix)
}

pub(super) fn agent_provider_command_preset(args: &EvalAgentRunArgs) -> &str {
    if args.provider_command.is_some() || args.provider_args_json.is_some() {
        "custom"
    } else {
        args.provider.as_str()
    }
}

pub(super) fn eval_case_telemetry_from_trace(trace: &[JsonValue]) -> EvalCaseTelemetry {
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

pub(super) fn sum_first_available_u64(trace: &[JsonValue], paths: &[&[&str]]) -> Option<u64> {
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

pub(super) fn json_path_u64(value: &JsonValue, path: &[&str]) -> Option<u64> {
    let mut cursor = value;
    for part in path {
        cursor = cursor.get(*part)?;
    }
    cursor.as_u64()
}

pub(super) fn current_git_sha() -> Option<String> {
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

pub(super) fn timestamp_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        })
}

pub(super) fn format_utc_timestamp_millis(timestamp_ms: u64) -> String {
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

pub(super) fn civil_from_days(days_since_epoch: i64) -> (i32, u32, u32) {
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

pub(super) fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

pub(super) fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}
