use super::{
    BTreeSet, BufRead, BufReader, DbtNovaError, EvalGateReport, GateConfigStatus, JsonValue, Path,
    fs, load_suite_with_hash, ratio, server_error, telemetry_path_for_suite,
};

pub(super) fn read_telemetry_rows_for_suite(
    suite_name: &str,
) -> crate::error::Result<Vec<JsonValue>> {
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

pub(super) fn build_eval_gate_report(
    suite_name: &str,
    rows: &[JsonValue],
) -> crate::error::Result<EvalGateReport> {
    let latest_rows = latest_telemetry_rows(rows);
    if latest_rows.is_empty() {
        return Ok(empty_eval_gate_report(suite_name));
    }

    let summary = summarize_latest_gate_telemetry(&latest_rows);
    let gate = gate_config_status_from_suite_path(summary.suite_path.as_deref())?;

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
        return Ok(summary.into_report(suite_name, false, gate_configured, threshold, message));
    }

    let (allowed, message) = gate_allowed_message(threshold, summary.pass_rate);
    Ok(summary.into_report(suite_name, allowed, gate_configured, threshold, message))
}

struct EvalGateTelemetrySummary {
    total_evals: usize,
    pass_rate: f64,
    failed_eval_ids: Vec<String>,
    failed_case_ids: Vec<String>,
    telemetry_timestamp: Option<String>,
    output_dir: Option<String>,
    suite_path: Option<String>,
}

impl EvalGateTelemetrySummary {
    fn into_report(
        self,
        suite_name: &str,
        allowed: bool,
        gate_configured: bool,
        threshold: Option<f64>,
        message: String,
    ) -> EvalGateReport {
        let failed_evals = self.failed_eval_ids.len();
        EvalGateReport {
            suite_name: suite_name.to_string(),
            allowed,
            blocked: !allowed,
            gate_configured,
            threshold,
            pass_rate: self.pass_rate,
            total_evals: self.total_evals,
            failed_evals,
            failed_eval_ids: self.failed_eval_ids,
            failed_case_ids: self.failed_case_ids,
            telemetry_timestamp: self.telemetry_timestamp,
            output_dir: self.output_dir,
            suite_path: self.suite_path,
            message,
        }
    }
}

fn empty_eval_gate_report(suite_name: &str) -> EvalGateReport {
    EvalGateReport {
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
    }
}

fn summarize_latest_gate_telemetry(latest_rows: &[&JsonValue]) -> EvalGateTelemetrySummary {
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
    let failed_case_ids = failed_rows
        .iter()
        .filter_map(|row| row.get("case_id").and_then(JsonValue::as_str))
        .map(str::to_string)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let first = latest_rows[0];
    EvalGateTelemetrySummary {
        total_evals,
        pass_rate: ratio(pass_count, total_evals),
        failed_eval_ids: failed_rows
            .iter()
            .map(|row| telemetry_eval_id(row))
            .collect(),
        failed_case_ids,
        telemetry_timestamp: first
            .get("timestamp")
            .and_then(JsonValue::as_str)
            .map(str::to_string),
        output_dir: first
            .get("output_dir")
            .and_then(JsonValue::as_str)
            .map(str::to_string),
        suite_path: first
            .get("suite_path")
            .and_then(JsonValue::as_str)
            .map(str::to_string),
    }
}

fn gate_allowed_message(threshold: Option<f64>, pass_rate: f64) -> (bool, String) {
    if let Some(threshold) = threshold {
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
    }
}

pub(super) fn latest_telemetry_rows(rows: &[JsonValue]) -> Vec<&JsonValue> {
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

pub(super) fn latest_run_incomplete_message(
    rows: &[&JsonValue],
    current_suite_hash: &str,
) -> Option<String> {
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

pub(super) fn gate_config_status_from_suite_path(
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
pub(super) fn suite_file_hash(path: &str) -> crate::error::Result<String> {
    let raw = fs::read(path).map_err(|error| {
        DbtNovaError::InvalidParams(format!(
            "failed to read eval suite '{path}' for hash: {error}"
        ))
    })?;
    Ok(blake3::hash(&raw).to_hex().to_string())
}

pub(super) fn telemetry_eval_id(row: &JsonValue) -> String {
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

pub(super) fn print_gate_report(report: &EvalGateReport) {
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
