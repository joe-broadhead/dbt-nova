use super::{
    DispatchResult, EvalAgentRunArgs, EvalCard, EvalCardGate, EvalCardManifestScope,
    EvalCardRunContext, EvalCardTelemetry, EvalCardTelemetryEvidence, EvalCaseReport,
    EvalGateReport, EvalReport, EvalSuite, JsonValue, Path, build_eval_gate_report, fs,
    latest_telemetry_rows, ratio, read_telemetry_rows_for_suite, render_eval_card_markdown,
    render_markdown, render_tsv, server_error,
};

pub(super) fn build_report(
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
pub(super) struct EvalReportCardSummary {
    pub(super) suite_name: String,
    pub(super) version: u32,
    pub(super) mode: &'static str,
    pub(super) output_dir: String,
    pub(super) assertion_count: usize,
    pub(super) pass_count: usize,
    pub(super) fail_count: usize,
    pub(super) error_count: usize,
    pub(super) pass_rate: f64,
    pub(super) fail_under: f64,
    pub(super) gate_status: &'static str,
    pub(super) run_case_count: usize,
}

pub(super) fn refresh_eval_card(
    report: &mut EvalReport,
    suite: &EvalSuite,
    context: &EvalCardRunContext,
) {
    let summary = EvalReportCardSummary::from_report(report);
    report.eval_card = build_eval_card(
        suite,
        &summary,
        context,
        eval_card_telemetry_evidence(&report.suite_name, context.telemetry_requested),
    );
}

pub(super) fn build_eval_card(
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

pub(super) fn eval_card_telemetry_evidence(
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

pub(super) fn missing_eval_card_telemetry(telemetry_requested: bool) -> EvalCardTelemetryEvidence {
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

pub(super) fn default_eval_card_purpose(mode: &str) -> String {
    match mode {
        "agent" => "Summarizes provider-backed agent tool-use evidence for this eval suite.",
        _ => "Summarizes deterministic Nova bridge eval evidence for this eval suite.",
    }
    .to_string()
}

impl EvalReportCardSummary {
    pub(super) fn from_report(report: &EvalReport) -> Self {
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
    pub(super) fn from_report(report: &EvalGateReport) -> Self {
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

pub(super) fn agent_manifest_source(args: &EvalAgentRunArgs) -> Option<String> {
    args.manifest_path
        .as_deref()
        .or(args.manifest_uri.as_deref())
        .map(crate::utils::sanitize_uri)
}

pub(super) fn write_report_artifacts(
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
