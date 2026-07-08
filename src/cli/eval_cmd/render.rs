use super::*;

pub(super) fn render_tsv(report: &EvalReport) -> String {
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

pub(super) fn render_markdown(report: &EvalReport) -> String {
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
            if assertion_type(&assertion.name) == "sql_structure" {
                for line in sql_structure_markdown_evidence(&assertion.evidence) {
                    let _ = writeln!(out, "  - {line}");
                }
            }
        }
        out.push('\n');
    }
    out
}

pub(super) fn sql_structure_markdown_evidence(evidence: &JsonValue) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(changed) = evidence
        .get("changed_clauses")
        .and_then(JsonValue::as_array)
        .map(|values| json_string_values(values))
        .filter(|values| !values.is_empty())
    {
        lines.push(format!("Changed clauses: {}", changed.join(", ")));
    }
    let Some(diff) = evidence.get("diff") else {
        return lines;
    };
    for (label, key) in [
        ("Missing SELECT", "missing_select"),
        ("Unexpected SELECT", "unexpected_select"),
        ("Missing FROM", "missing_tables"),
        ("Unexpected FROM", "unexpected_tables"),
        ("Missing JOIN", "missing_joins"),
        ("Unexpected JOIN", "unexpected_joins"),
        ("Missing WHERE", "missing_filters"),
        ("Unexpected WHERE", "unexpected_filters"),
        ("Missing GROUP BY", "missing_group_by"),
        ("Unexpected GROUP BY", "unexpected_group_by"),
    ] {
        if let Some(values) = diff
            .get(key)
            .and_then(JsonValue::as_array)
            .map(|values| json_string_values(values))
            .filter(|values| !values.is_empty())
        {
            lines.push(format!("{label}: {}", values.join("; ")));
        }
    }
    lines
}

pub(super) fn json_string_values(values: &[JsonValue]) -> Vec<String> {
    values
        .iter()
        .filter_map(JsonValue::as_str)
        .map(ToString::to_string)
        .collect()
}

pub(super) fn render_eval_card_markdown(card: &EvalCard) -> String {
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

pub(super) fn tsv_escape(value: &str) -> String {
    value.replace(['\t', '\n', '\r'], " ")
}

pub(super) fn finish_report(
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
    pub(super) fn new(
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

    pub(super) fn with_date_anchor(mut self, date_anchor: Option<DateAnchor>) -> Self {
        self.date_anchor = date_anchor;
        self
    }

    pub(super) fn with_telemetry(mut self, telemetry: EvalCaseTelemetry) -> Self {
        self.telemetry = Some(telemetry);
        self
    }
}

impl AssertionResult {
    pub(super) fn pass(
        name: impl Into<String>,
        message: impl Into<String>,
        evidence: JsonValue,
    ) -> Self {
        Self {
            name: name.into(),
            status: "pass",
            message: message.into(),
            evidence,
        }
    }

    pub(super) fn fail(
        name: impl Into<String>,
        message: impl Into<String>,
        evidence: JsonValue,
    ) -> Self {
        Self {
            name: name.into(),
            status: "fail",
            message: message.into(),
            evidence,
        }
    }

    pub(super) fn error(name: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            status: "error",
            message: message.into(),
            evidence: JsonValue::Null,
        }
    }
}

impl AgentExpected {
    pub(super) fn requires_trace(&self) -> bool {
        !self.must_call.is_empty()
            || !self.must_not_call.is_empty()
            || !self.ordered.is_empty()
            || !self.selected_entities.is_empty()
            || !self.selected_entity_ranks.is_empty()
            || !self.called_with.is_empty()
            || !self.sql_structures.is_empty()
            || self.max_tool_calls.is_some()
            || self.max_distinct_tools.is_some()
            || self.max_total_response_bytes.is_some()
            || !self.max_response_bytes_by_tool.is_empty()
    }
}
