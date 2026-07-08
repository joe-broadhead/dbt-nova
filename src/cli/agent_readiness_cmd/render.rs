use super::*;

pub(super) fn append_readiness_triage_summary(out: &mut String, report: &AgentReadinessReport) {
    let weak_spots = report
        .summary
        .category_weak_spots
        .as_array()
        .map_or(&[][..], Vec::as_slice);
    let repeated_fields = report
        .summary
        .top_recommendation_fields
        .as_array()
        .map_or(&[][..], Vec::as_slice);
    if weak_spots.is_empty() && repeated_fields.is_empty() {
        return;
    }

    out.push_str("## Triage Summary\n\n");
    for weak_spot in weak_spots.iter().take(5) {
        let persona = weak_spot
            .get("persona")
            .and_then(JsonValue::as_str)
            .unwrap_or("project");
        let category = weak_spot
            .get("category")
            .and_then(JsonValue::as_str)
            .unwrap_or("metadata");
        let average_score = weak_spot
            .get("average_score")
            .and_then(JsonValue::as_u64)
            .unwrap_or(0);
        let gap = weak_spot
            .get("estimated_point_gap")
            .and_then(JsonValue::as_u64)
            .unwrap_or(0);
        let _ = writeln!(
            out,
            "- `{persona}` `{category}` average score `{average_score}`, estimated point gap `{gap}`"
        );
    }
    for field in repeated_fields.iter().take(5) {
        let field_name = field
            .get("field")
            .and_then(JsonValue::as_str)
            .unwrap_or("metadata");
        let count = field.get("count").and_then(JsonValue::as_u64).unwrap_or(0);
        let impact = field
            .get("estimated_point_impact")
            .and_then(JsonValue::as_u64)
            .unwrap_or(0);
        let _ = writeln!(
            out,
            "- repeated field `{field_name}` appears `{count}` time(s), estimated point impact `{impact}`"
        );
    }
    out.push('\n');
}

pub(super) fn append_markdown_summary(out: &mut String, report: &AgentReadinessReport) {
    let _ = write!(
        out,
        "- gate_status: `{}`\n- readiness_band: `{}`\n- overall_score: `{}` ({})\n- target_count: `{}`\n- blockers: `{}`\n- improvements: `{}`\n- suggested_meta_patches: `{}`\n- golden_question_seeds: `{}`\n\n",
        report.gate_status,
        report.readiness_band,
        report.overall_score,
        report.grade,
        report.summary.target_count,
        report.summary.blocker_count,
        report.summary.improvement_count,
        report.summary.suggested_meta_patch_count,
        report.summary.golden_question_seed_count
    );
}

pub(super) fn append_persona_score_table(out: &mut String, report: &AgentReadinessReport) {
    out.push_str("## Persona Scores\n\n");
    out.push_str("| Persona | Score | Grade | Gate | Scored |\n");
    out.push_str("|---|---:|---|---|---:|\n");
    for persona in &report.config.personas {
        if let Some(score) = report.persona_scores.get(persona) {
            let _ = writeln!(
                out,
                "| {} | {} | {} | `{}` | {} / {} |",
                title_case(persona),
                score.overall_score,
                score.grade,
                score.gate_status,
                score.scored_count,
                score.total_available
            );
        }
    }
    out.push('\n');
}

pub(super) fn append_eval_status(out: &mut String, report: &AgentReadinessReport) {
    out.push_str("## Eval Status\n\n");
    let _ = writeln!(
        out,
        "- status: `{}`\n- supplied: `{}`\n- message: {}\n",
        report.eval_status.status, report.eval_status.supplied, report.eval_status.message
    );
}

pub(super) fn append_entity_findings_table(
    out: &mut String,
    entity_findings: &[EntityReadinessFinding],
) {
    if !entity_findings.is_empty() {
        out.push_str("## Entity Findings\n\n");
        out.push_str("| Entity | Type | Score | Signal gaps | Top recommendation |\n");
        out.push_str("|---|---|---:|---:|---|\n");
        for entity in entity_findings {
            let recommendation = entity
                .recommendations
                .first()
                .map_or("-", |recommendation| recommendation.message.as_str());
            let _ = writeln!(
                out,
                "| `{}` | `{}` | {} ({}) | {} | {} |",
                entity.unique_id,
                entity.resource_type.clone().unwrap_or_default(),
                entity.overall_score,
                entity.grade,
                signal_gap_count(&entity.signals),
                escape_markdown_table_cell(recommendation)
            );
        }
        out.push('\n');
    }
}

pub(super) fn append_indicator_findings_table(
    out: &mut String,
    indicator_findings: &[IndicatorReadinessFinding],
) {
    if !indicator_findings.is_empty() {
        out.push_str("## Indicator Findings\n\n");
        out.push_str("| Entity | Indicator | Type | Issue |\n");
        out.push_str("|---|---|---|---|\n");
        for finding in indicator_findings {
            let _ = writeln!(
                out,
                "| `{}` | `{}` | `{}` | {} |",
                finding.unique_id,
                finding.indicator_name.as_deref().unwrap_or("-"),
                finding.indicator_type,
                escape_markdown_table_cell(&finding.issue)
            );
        }
        out.push('\n');
    }
}

pub(super) fn append_suggested_meta_patches_table(
    out: &mut String,
    patches: &[SuggestedMetaPatch],
) {
    if !patches.is_empty() {
        out.push_str("## Suggested Meta Patches\n\n");
        out.push_str("| Target | Field | Suggested value | Rationale |\n");
        out.push_str("|---|---|---|---|\n");
        for patch in patches.iter().take(MAX_MARKDOWN_META_PATCHES) {
            let target = patch
                .column_name
                .as_deref()
                .or(patch.indicator_name.as_deref())
                .unwrap_or(patch.unique_id.as_str());
            let _ = writeln!(
                out,
                "| `{}` | `{}` | `{}` | {} |",
                target,
                patch.field_path,
                escape_markdown_table_cell(&json_inline(&patch.suggested_value)),
                escape_markdown_table_cell(&patch.rationale)
            );
        }
        if patches.len() > MAX_MARKDOWN_META_PATCHES {
            let _ = writeln!(
                out,
                "| `_truncated` | `-` | `-` | {} additional suggestion(s) omitted from Markdown; see JSON report. |",
                patches.len() - MAX_MARKDOWN_META_PATCHES
            );
        }
        out.push('\n');
    }
}

pub(super) fn append_golden_question_seeds_table(out: &mut String, seeds: &[GoldenQuestionSeed]) {
    if !seeds.is_empty() {
        out.push_str("## Golden Question Seeds\n\n");
        out.push_str("| Type | Persona | Question | Suggested assertion |\n");
        out.push_str("|---|---|---|---|\n");
        for seed in seeds.iter().take(MAX_MARKDOWN_GOLDEN_SEEDS) {
            let assertion = seed
                .recommended_assertions
                .first()
                .map_or_else(|| "-".to_string(), json_inline);
            let _ = writeln!(
                out,
                "| `{}` | `{}` | {} | `{}` |",
                seed.seed_type,
                seed.persona,
                escape_markdown_table_cell(&seed.question),
                escape_markdown_table_cell(&assertion)
            );
        }
        if seeds.len() > MAX_MARKDOWN_GOLDEN_SEEDS {
            let _ = writeln!(
                out,
                "| `_truncated` | `-` | {} additional seed(s) omitted from Markdown; see JSON report. | `-` |",
                seeds.len() - MAX_MARKDOWN_GOLDEN_SEEDS
            );
        }
        out.push('\n');
    }
}

pub(super) fn append_next_actions(out: &mut String, next_actions: &[ReadinessNextAction]) {
    out.push_str("## Next Actions\n\n");
    for (index, action) in next_actions.iter().enumerate() {
        let _ = writeln!(
            out,
            "{}. **{}**: {} ({})",
            index + 1,
            title_case(action.category),
            action.action,
            action.evidence
        );
    }
}

pub(super) fn append_findings_table(out: &mut String, title: &str, findings: &[ReadinessFinding]) {
    if findings.is_empty() {
        return;
    }
    let _ = writeln!(out, "## {title}\n");
    out.push_str("| Category | Code | Message |\n");
    out.push_str("|---|---|---|\n");
    for finding in findings {
        let _ = writeln!(
            out,
            "| `{}` | `{}` | {} |",
            finding.category,
            finding.code,
            escape_markdown_table_cell(&finding.message)
        );
    }
    out.push('\n');
}

pub(super) fn print_human_summary(report: &AgentReadinessReport) {
    println!("agent readiness audit complete");
    println!("  gate_status: {}", report.gate_status);
    println!("  readiness_band: {}", report.readiness_band);
    println!(
        "  overall_score: {} ({})",
        report.overall_score, report.grade
    );
    println!("  target_count: {}", report.summary.target_count);
    println!("  blockers: {}", report.summary.blocker_count);
    println!("  improvements: {}", report.summary.improvement_count);
    println!("  eval_status: {}", report.eval_status.status);
}

pub(super) fn write_report(path: &str, contents: &str) -> Result<()> {
    let path = Path::new(path);
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, contents)?;
    Ok(())
}

pub(super) fn render_or_propagate_error(
    command_name: &str,
    json: bool,
    error: DbtNovaError,
    elapsed_ms: u128,
) -> DispatchError {
    if json {
        let envelope = error_envelope(command_name, &error, elapsed_ms);
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

pub(super) fn parse_json_input<T>(
    inline: Option<&str>,
    path: Option<&str>,
    default_inline: &str,
) -> Result<T>
where
    T: for<'de> Deserialize<'de>,
{
    let raw = raw_optional_input(inline, path)?.unwrap_or_else(|| default_inline.to_string());
    serde_json::from_str(&raw).map_err(|error| {
        DbtNovaError::InvalidParams(format!("failed to parse JSON input: {error}"))
    })
}

pub(super) fn parse_json_array_input(
    inline: Option<&str>,
    path: Option<&str>,
    default_inline: &str,
) -> Result<Vec<String>> {
    parse_json_input(inline, path, default_inline)
}

pub(super) fn raw_optional_input(
    inline: Option<&str>,
    path: Option<&str>,
) -> Result<Option<String>> {
    if let Some(inline) = inline {
        return Ok(Some(inline.to_string()));
    }
    if let Some(path) = path {
        return fs::read_to_string(path).map(Some).map_err(|error| {
            DbtNovaError::InvalidParams(format!("failed to read {path}: {error}"))
        });
    }
    Ok(None)
}

pub(super) fn json_u8(data: &JsonValue, field: &str) -> Result<u8> {
    let raw = data.get(field).and_then(JsonValue::as_u64).ok_or_else(|| {
        DbtNovaError::ServerError(format!("metadata score response missing {field}"))
    })?;
    u8::try_from(raw).map_err(|_| {
        DbtNovaError::ServerError(format!("metadata score field {field} out of range"))
    })
}

pub(super) fn json_string(data: &JsonValue, field: &str) -> Result<String> {
    data.get(field)
        .and_then(JsonValue::as_str)
        .map(ToString::to_string)
        .ok_or_else(|| {
            DbtNovaError::ServerError(format!("metadata score response missing {field}"))
        })
}

pub(super) fn json_usize_optional(data: &JsonValue, field: &str) -> Option<usize> {
    data.get(field)
        .and_then(JsonValue::as_u64)
        .and_then(|value| usize::try_from(value).ok())
}

pub(super) fn json_string_array(data: &JsonValue, field: &str) -> Vec<String> {
    data.get(field)
        .and_then(JsonValue::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(JsonValue::as_str)
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default()
}

pub(super) fn average_score(scores: impl IntoIterator<Item = u8>) -> u8 {
    let mut total = 0usize;
    let mut count = 0usize;
    for score in scores {
        total += usize::from(score);
        count += 1;
    }
    if count == 0 {
        return 0;
    }
    let rounded = (total + (count / 2)) / count;
    u8::try_from(rounded.min(100)).unwrap_or(100)
}

pub(super) fn current_timestamp_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis())
}

pub(super) fn title_case(value: &str) -> String {
    value
        .split(['_', '-'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub(super) fn escape_markdown_table_cell(value: &str) -> String {
    value.replace('|', "\\|").replace('\n', " ")
}

pub(super) fn json_inline(value: &JsonValue) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "null".to_string())
}
