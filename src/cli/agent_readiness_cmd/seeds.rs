use super::{
    ArchivedEntity, BTreeSet, EntityReadinessFinding, GoldenQuestionSeed,
    IndicatorReadinessFinding, JsonValue, MAX_GOLDEN_QUESTION_SEEDS, MAX_SUGGESTED_META_PATCHES,
    ManifestSearch, Result, SuggestedMetaPatch, entity_nova_meta_json, json, stable_id_fragment,
    stable_seed_id,
};

pub(super) fn build_golden_question_seeds(
    search: &ManifestSearch,
    target_ids: &[String],
    entity_findings: &[EntityReadinessFinding],
    indicator_findings: &[IndicatorReadinessFinding],
) -> Result<Vec<GoldenQuestionSeed>> {
    let mut seeds = Vec::new();
    let mut seen = BTreeSet::new();

    for unique_id in target_ids {
        let Some(entity) = search.get_entity_archived(unique_id)? else {
            continue;
        };
        append_canonical_indicator_seeds(entity, unique_id, &mut seeds, &mut seen);
        if seeds.len() >= MAX_GOLDEN_QUESTION_SEEDS {
            return Ok(seeds);
        }
    }

    for finding in indicator_findings {
        append_indicator_review_seed(finding, &mut seeds, &mut seen);
        if seeds.len() >= MAX_GOLDEN_QUESTION_SEEDS {
            return Ok(seeds);
        }
    }

    for finding in entity_findings {
        append_entity_review_seeds(finding, &mut seeds, &mut seen);
        if seeds.len() >= MAX_GOLDEN_QUESTION_SEEDS {
            break;
        }
    }

    seeds.sort_by(|left, right| {
        left.priority
            .cmp(&right.priority)
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(seeds)
}

pub(super) fn append_canonical_indicator_seeds(
    entity: &ArchivedEntity,
    unique_id: &str,
    seeds: &mut Vec<GoldenQuestionSeed>,
    seen: &mut BTreeSet<String>,
) {
    let entity_json = entity.to_json_value();
    let nova = entity_nova_meta_json(&entity_json);
    let Some(nova) = nova.as_deref() else {
        return;
    };
    let entity_canonical = nova
        .get("canonical")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false);
    append_metric_seed_from_object(
        entity,
        unique_id,
        nova.get("metric"),
        entity_canonical,
        seeds,
        seen,
    );
    if let Some(metrics) = nova.get("metrics").and_then(JsonValue::as_array) {
        for metric in metrics {
            append_metric_seed_from_object(
                entity,
                unique_id,
                Some(metric),
                entity_canonical,
                seeds,
                seen,
            );
        }
    }
    if let Some(measures) = nova.get("measures").and_then(JsonValue::as_array) {
        for measure in measures {
            append_measure_seed_from_object(
                entity,
                unique_id,
                measure,
                entity_canonical,
                seeds,
                seen,
            );
        }
    }
}

pub(super) fn append_metric_seed_from_object(
    entity: &ArchivedEntity,
    unique_id: &str,
    metric: Option<&JsonValue>,
    entity_canonical: bool,
    seeds: &mut Vec<GoldenQuestionSeed>,
    seen: &mut BTreeSet<String>,
) {
    let Some(metric) = metric.and_then(JsonValue::as_object) else {
        return;
    };
    let Some(name) = metric.get("name").and_then(JsonValue::as_str) else {
        return;
    };
    if !string_value_present(metric.get("expression")) {
        return;
    }
    let canonical = metric
        .get("canonical")
        .and_then(JsonValue::as_bool)
        .unwrap_or(entity_canonical);
    if !canonical {
        return;
    }
    let query = indicator_seed_query(entity, Some(metric), name);
    let resource_type = entity.resource_type_str().unwrap_or("model");
    push_seed(
        seeds,
        seen,
        GoldenQuestionSeed {
            id: stable_seed_id("bridge", unique_id, Some("metric"), Some(name)),
            seed_type: "bridge",
            priority: 1,
            persona: "analyst",
            question: format!("Find the canonical {name} metric for this project."),
            expected_entities: vec![unique_id.to_string()],
            expected_indicators: vec![name.to_string()],
            recommended_assertions: vec![json!({
                "type": "search_indicator_rank",
                "query": query,
                "expected": name,
                "max_rank": 3,
                "resource_types": [resource_type],
                "indicator_types": ["metric"],
                "persona": "analyst"
            })],
            rationale: "Canonical metric has an explicit expression, so it can seed a deterministic bridge eval.".to_string(),
            review_required: true,
            date_policy: "not_date_sensitive",
        },
    );
}

pub(super) fn append_measure_seed_from_object(
    entity: &ArchivedEntity,
    unique_id: &str,
    measure: &JsonValue,
    entity_canonical: bool,
    seeds: &mut Vec<GoldenQuestionSeed>,
    seen: &mut BTreeSet<String>,
) {
    let Some(measure) = measure.as_object() else {
        return;
    };
    let Some(name) = measure.get("name").and_then(JsonValue::as_str) else {
        return;
    };
    if !string_value_present(measure.get("expression"))
        && !string_value_present(measure.get("field"))
    {
        return;
    }
    let canonical = measure
        .get("canonical")
        .and_then(JsonValue::as_bool)
        .unwrap_or(entity_canonical);
    if !canonical {
        return;
    }
    let query = indicator_seed_query(entity, Some(measure), name);
    let resource_type = entity.resource_type_str().unwrap_or("model");
    push_seed(
        seeds,
        seen,
        GoldenQuestionSeed {
            id: stable_seed_id("bridge", unique_id, Some("measure"), Some(name)),
            seed_type: "bridge",
            priority: 2,
            persona: "analyst",
            question: format!("Find the canonical {name} measure for this project."),
            expected_entities: vec![unique_id.to_string()],
            expected_indicators: vec![name.to_string()],
            recommended_assertions: vec![json!({
                "type": "search_indicator_rank",
                "query": query,
                "expected": name,
                "max_rank": 3,
                "resource_types": [resource_type],
                "indicator_types": ["measure"],
                "persona": "analyst"
            })],
            rationale: "Canonical measure has execution metadata, so it can seed a deterministic bridge eval.".to_string(),
            review_required: true,
            date_policy: "not_date_sensitive",
        },
    );
}

pub(super) fn append_indicator_review_seed(
    finding: &IndicatorReadinessFinding,
    seeds: &mut Vec<GoldenQuestionSeed>,
    seen: &mut BTreeSet<String>,
) {
    let indicator_label = finding
        .indicator_name
        .as_deref()
        .unwrap_or("unnamed indicator");
    push_seed(
        seeds,
        seen,
        GoldenQuestionSeed {
            id: stable_seed_id(
                "manual_review",
                &finding.unique_id,
                Some(finding.indicator_type.as_str()),
                finding.indicator_name.as_deref().or(Some(finding.issue.as_str())),
            ),
            seed_type: "manual_review",
            priority: 2,
            persona: "analyst",
            question: format!(
                "Review {indicator_label} on {} before turning it into a gated eval.",
                finding.unique_id
            ),
            expected_entities: vec![finding.unique_id.clone()],
            expected_indicators: finding.indicator_name.iter().cloned().collect(),
            recommended_assertions: vec![json!({
                "type": "manual_review",
                "instruction": format!("Resolve indicator readiness issue: {}", finding.issue)
            })],
            rationale: "Indicator metadata is ambiguous, so the first seed should request review rather than assert false ground truth.".to_string(),
            review_required: true,
            date_policy: "not_date_sensitive",
        },
    );
}

pub(super) fn append_entity_review_seeds(
    finding: &EntityReadinessFinding,
    seeds: &mut Vec<GoldenQuestionSeed>,
    seen: &mut BTreeSet<String>,
) {
    if !finding.signals.has_nova_meta || !finding.signals.has_description {
        push_seed(
            seeds,
            seen,
            GoldenQuestionSeed {
                id: stable_seed_id("manual_review", &finding.unique_id, Some("context"), None),
                seed_type: "manual_review",
                priority: 3,
                persona: "analyst",
                question: format!(
                    "Review whether {} has enough business context for analyst questions.",
                    finding.unique_id
                ),
                expected_entities: vec![finding.unique_id.clone()],
                expected_indicators: Vec::new(),
                recommended_assertions: vec![json!({
                    "type": "metadata_score_min",
                    "id_or_name": finding.unique_id,
                    "threshold": 0.70,
                    "persona": "analyst"
                })],
                rationale: "Entity context is incomplete; make the first eval a review or metadata score gate before answer-correctness checks.".to_string(),
                review_required: true,
                date_policy: "not_date_sensitive",
            },
        );
    }
    if !finding.signals.has_nova_meta {
        push_seed(
            seeds,
            seen,
            GoldenQuestionSeed {
                id: stable_seed_id("manual_review", &finding.unique_id, Some("governance"), None),
                seed_type: "manual_review",
                priority: 3,
                persona: "governance",
                question: format!(
                    "Review governance classification for {} before production agent use.",
                    finding.unique_id
                ),
                expected_entities: vec![finding.unique_id.clone()],
                expected_indicators: Vec::new(),
                recommended_assertions: vec![json!({
                    "type": "metadata_score_min",
                    "id_or_name": finding.unique_id,
                    "threshold": 0.65,
                    "persona": "governance"
                })],
                rationale: "Governance coverage is missing or weak; keep this seed advisory until sensitivity and PII fields are reviewed.".to_string(),
                review_required: true,
                date_policy: "not_date_sensitive",
            },
        );
    }
}

#[derive(Clone, Copy)]
pub(super) enum MetaPatchTarget<'a> {
    Entity,
    Column(&'a str),
    Indicator {
        name: Option<&'a str>,
        kind: &'a str,
    },
}

pub(super) struct MetaPatchContent<'a> {
    pub(super) field_path: &'a str,
    pub(super) suggested_value: JsonValue,
    pub(super) placeholder: bool,
    pub(super) rationale: &'a str,
    pub(super) confidence: f32,
    pub(super) evidence: JsonValue,
}

pub(super) fn entity_patch(
    entity: &ArchivedEntity,
    target: MetaPatchTarget<'_>,
    content: MetaPatchContent<'_>,
) -> SuggestedMetaPatch {
    let (target_type, column_name, indicator_name, indicator_type) = match target {
        MetaPatchTarget::Entity => ("entity", None, None, None),
        MetaPatchTarget::Column(name) => ("column", Some(name), None, None),
        MetaPatchTarget::Indicator { name, kind } => ("indicator", None, name, Some(kind)),
    };
    let unique_id = entity.unique_id.as_str().to_string();
    let severity = meta_patch_severity(&content);
    SuggestedMetaPatch {
        id: stable_meta_patch_id(
            &unique_id,
            column_name,
            indicator_type,
            indicator_name,
            content.field_path,
        ),
        target_type,
        unique_id,
        entity_name: entity.name_str().map(ToString::to_string),
        resource_type: entity.resource_type_str().map(ToString::to_string),
        original_file_path: entity.original_file_path_str().map(ToString::to_string),
        column_name: column_name.map(ToString::to_string),
        indicator_name: indicator_name.map(ToString::to_string),
        indicator_type: indicator_type.map(ToString::to_string),
        field_path: content.field_path.to_string(),
        suggested_value: content.suggested_value,
        placeholder: content.placeholder,
        rationale: content.rationale.to_string(),
        severity,
        confidence: content.confidence,
        evidence: content.evidence,
    }
}

fn meta_patch_severity(content: &MetaPatchContent<'_>) -> &'static str {
    let diagnostic = content
        .evidence
        .get("diagnostic")
        .and_then(JsonValue::as_str);
    if matches!(diagnostic, Some("invalid_grain_shape")) {
        return "required";
    }

    let indicator_issue = content
        .evidence
        .get("indicator_issue")
        .and_then(JsonValue::as_str)
        .unwrap_or_default();
    if indicator_issue.contains("not an object") {
        return "required";
    }
    if !indicator_issue.is_empty() {
        return "refinement";
    }

    let signal = content
        .evidence
        .get("signal")
        .and_then(JsonValue::as_str)
        .unwrap_or_default();
    if content
        .evidence
        .get("existing_grain")
        .and_then(JsonValue::as_bool)
        == Some(true)
    {
        return "refinement";
    }
    if matches!(
        signal,
        "missing_canonical_flag"
            | "missing_column_role"
            | "missing_column_semantic_type"
            | "missing_nova_indicators"
    ) {
        return "refinement";
    }
    if content.confidence < 0.65 {
        return "refinement";
    }
    "recommended"
}

pub(super) fn push_meta_patch(
    patches: &mut Vec<SuggestedMetaPatch>,
    seen: &mut BTreeSet<String>,
    patch: SuggestedMetaPatch,
) {
    if patches.len() >= MAX_SUGGESTED_META_PATCHES || !seen.insert(patch.id.clone()) {
        return;
    }
    patches.push(patch);
}

pub(super) fn push_seed(
    seeds: &mut Vec<GoldenQuestionSeed>,
    seen: &mut BTreeSet<String>,
    question_seed: GoldenQuestionSeed,
) {
    if seeds.len() >= MAX_GOLDEN_QUESTION_SEEDS || !seen.insert(question_seed.id.clone()) {
        return;
    }
    seeds.push(question_seed);
}

pub(super) fn json_array_field_non_empty(nova: Option<&JsonValue>, field: &str) -> bool {
    nova.and_then(|nova| nova.get(field))
        .and_then(JsonValue::as_array)
        .is_some_and(|values| !values.is_empty())
}

pub(super) fn json_bool_field_present(nova: Option<&JsonValue>, field: &str) -> bool {
    nova.and_then(|nova| nova.get(field))
        .and_then(JsonValue::as_bool)
        .is_some()
}

pub(super) fn infer_primary_key_column(entity_json: &JsonValue) -> Option<String> {
    let columns = entity_json.get("columns")?.as_object()?;
    let entity_name = entity_json
        .get("name")
        .and_then(JsonValue::as_str)
        .unwrap_or_default()
        .trim_start_matches("dim__")
        .trim_start_matches("fct__")
        .trim_start_matches("stg__")
        .trim_end_matches('s')
        .to_ascii_lowercase();
    let mut candidates = columns
        .keys()
        .filter(|column| {
            let lowered = column.to_ascii_lowercase();
            lowered == "id" || lowered == format!("{entity_name}_id") || lowered.ends_with("_id")
        })
        .cloned()
        .collect::<Vec<_>>();
    candidates.sort();
    candidates.first().cloned()
}

pub(super) fn infer_time_column(entity_json: &JsonValue) -> Option<String> {
    let columns = entity_json.get("columns")?.as_object()?;
    let mut candidates = columns
        .keys()
        .filter(|column| looks_like_time_column(&column.to_ascii_lowercase()))
        .cloned()
        .collect::<Vec<_>>();
    candidates.sort();
    candidates.first().cloned()
}

pub(super) fn looks_like_time_column(lowered: &str) -> bool {
    lowered == "date"
        || lowered.ends_with("_date")
        || lowered.ends_with("_at")
        || lowered.ends_with("_timestamp")
        || lowered.ends_with("_time")
}

pub(super) fn indicator_count_in_nova(nova: Option<&JsonValue>) -> usize {
    let Some(nova) = nova else {
        return 0;
    };
    let measures = nova
        .get("measures")
        .and_then(JsonValue::as_array)
        .map_or(0, Vec::len);
    let metrics = nova
        .get("metrics")
        .and_then(JsonValue::as_array)
        .map_or(0, Vec::len);
    let metric = usize::from(nova.get("metric").is_some_and(|value| !value.is_null()));
    measures + metrics + metric
}

pub(super) fn indicator_meta_base_path(finding: &IndicatorReadinessFinding) -> String {
    match (
        finding.indicator_type.as_str(),
        finding.indicator_name.as_deref(),
    ) {
        ("measure", Some(name)) => format!("meta.nova.measures[name={name}]"),
        ("measure", None) => "meta.nova.measures[]".to_string(),
        ("metric", Some(name)) => format!("meta.nova.metrics[name={name}]"),
        ("metric", None) if finding.resource_type.as_deref() == Some("metric") => {
            "description".to_string()
        }
        ("metric", None) => "meta.nova.metrics[]".to_string(),
        (_, Some(name)) => format!("meta.nova.indicators[name={name}]"),
        _ => "meta.nova.indicators[]".to_string(),
    }
}

pub(super) fn string_value_present(value: Option<&JsonValue>) -> bool {
    value
        .and_then(JsonValue::as_str)
        .is_some_and(|value| !value.trim().is_empty())
}

pub(super) fn indicator_seed_query(
    entity: &ArchivedEntity,
    indicator: Option<&serde_json::Map<String, JsonValue>>,
    name: &str,
) -> String {
    let mut tokens = BTreeSet::new();
    insert_query_tokens(&mut tokens, name);
    if let Some(indicator) = indicator {
        insert_query_tokens_from_value(&mut tokens, indicator.get("description"));
        insert_query_tokens_from_array(&mut tokens, indicator.get("synonyms"));
    }
    let entity_json = entity.to_json_value();
    let nova = entity_nova_meta_json(&entity_json);
    let nova = nova.as_deref();
    if let Some(nova) = nova {
        insert_query_tokens_from_array(&mut tokens, nova.get("domains"));
        insert_query_tokens_from_array(&mut tokens, nova.get("use_cases"));
        insert_query_tokens_from_array(&mut tokens, nova.get("synonyms"));
    }
    tokens.into_iter().take(14).collect::<Vec<_>>().join(" ")
}

pub(super) fn insert_query_tokens(tokens: &mut BTreeSet<String>, value: &str) {
    for token in value
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .map(str::trim)
        .filter(|token| token.len() > 2)
    {
        tokens.insert(token.to_ascii_lowercase());
    }
}

pub(super) fn insert_query_tokens_from_value(
    tokens: &mut BTreeSet<String>,
    value: Option<&JsonValue>,
) {
    if let Some(value) = value.and_then(JsonValue::as_str) {
        insert_query_tokens(tokens, value);
    }
}

pub(super) fn insert_query_tokens_from_array(
    tokens: &mut BTreeSet<String>,
    value: Option<&JsonValue>,
) {
    let Some(values) = value.and_then(JsonValue::as_array) else {
        return;
    };
    for value in values.iter().filter_map(JsonValue::as_str) {
        insert_query_tokens(tokens, value);
    }
}

pub(super) fn stable_meta_patch_id(
    unique_id: &str,
    column_name: Option<&str>,
    indicator_type: Option<&str>,
    indicator_name: Option<&str>,
    field_path: &str,
) -> String {
    [
        "meta_patch",
        unique_id,
        column_name.unwrap_or("-"),
        indicator_type.unwrap_or("-"),
        indicator_name.unwrap_or("-"),
        field_path,
    ]
    .into_iter()
    .map(stable_id_fragment)
    .collect::<Vec<_>>()
    .join("::")
}
