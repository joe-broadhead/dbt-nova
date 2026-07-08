use super::{
    AGENT_MODELLING_SEVERITY_ORDER, AGENT_MODELLING_TOP_BUCKETS, AgentModellingContext,
    AgentModellingFinding, AgentModellingSeverity, ArchivedEntity, ArchivedNovaGrain,
    ArchivedNovaMeta, ArchivedNovaMetric, ArchivedString, BTreeMap, BTreeSet,
    DuplicateIndicatorParent, DuplicateIndicatorRow, EntityOverlapProfile, EntityOverlapRow,
    EntityRef, GrainVariant, HashSet, JsonMap, JsonValue, ManifestSearch, ModelingEntityRef,
    ModelingIndicatorRef, ModellingConsistencyReportParams, MultiGrainEntityRow, Ordering, Result,
    build_entity_grain_variants, column_nova_meta_json, json, tokenize_alnum_lowercase,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum IndicatorExecutionSurface {
    Relation,
    SemanticLayer,
    MetadataOnly,
}

impl IndicatorExecutionSurface {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Relation => "relation",
            Self::SemanticLayer => "semantic_layer",
            Self::MetadataOnly => "metadata_only",
        }
    }

    pub(super) fn queryable(self) -> bool {
        !matches!(self, Self::MetadataOnly)
    }

    pub(super) fn direct_sql_queryable(self) -> bool {
        matches!(self, Self::Relation)
    }

    pub(super) fn queryable_via(self) -> &'static str {
        match self {
            Self::Relation => "relation_name",
            Self::SemanticLayer => "metricflow",
            Self::MetadataOnly => "none",
        }
    }
}

pub(super) fn execution_surface(entity: &ArchivedEntity) -> IndicatorExecutionSurface {
    match entity.resource_type_str() {
        Some("metric" | "semantic_model") => IndicatorExecutionSurface::SemanticLayer,
        _ if entity.relation_name_str().is_some() => IndicatorExecutionSurface::Relation,
        _ => IndicatorExecutionSurface::MetadataOnly,
    }
}

pub(super) fn entity_is_analyst_facing(entity: &ArchivedEntity) -> bool {
    if matches!(entity.resource_type_str(), Some("test" | "macro")) {
        return false;
    }
    let Some(nova) = entity.nova_meta() else {
        return entity.relation_name_str().is_some();
    };
    if let Some(candidates) = nova
        .search
        .as_ref()
        .and_then(|search| search.candidates.as_ref())
        && !candidates.analyst
    {
        return false;
    }
    nova.canonical
        || nova.metric.is_some()
        || !nova.metrics.is_empty()
        || !nova.measures.is_empty()
        || entity.relation_name_str().is_some()
}

pub(super) fn normalized_entity_column_names(entity: &ArchivedEntity) -> BTreeSet<String> {
    entity
        .column_names_iter()
        .map(normalize_value)
        .filter(|value| !value.is_empty())
        .collect()
}

pub(super) fn grain_time_field(grain: &ArchivedNovaGrain) -> Option<&str> {
    grain.time_field.as_ref().map(ArchivedString::as_str)
}

pub(super) fn grain_field_names(grain: &ArchivedNovaGrain) -> Vec<String> {
    let mut fields = BTreeSet::new();
    fields.extend(
        grain
            .primary_key
            .iter()
            .map(ArchivedString::as_str)
            .map(str::to_string),
    );
    if let Some(time_field) = grain_time_field(grain) {
        fields.insert(time_field.to_string());
    }
    fields.extend(
        grain
            .dimensions
            .iter()
            .map(ArchivedString::as_str)
            .map(str::to_string),
    );
    fields.into_iter().collect()
}

#[derive(Clone)]
pub(super) struct FactLikeParent {
    pub(super) entity: ModelingEntityRef,
    pub(super) grain_signatures: Vec<String>,
}

#[derive(Clone)]
pub(super) struct SemanticLabelRef {
    pub(super) entity: ModelingEntityRef,
    pub(super) indicator: ModelingIndicatorRef,
    pub(super) canonical: bool,
    pub(super) ref_key: String,
}

#[derive(Clone)]
pub(super) struct ColumnSemanticRef {
    pub(super) entity: ModelingEntityRef,
    pub(super) column_name: String,
    pub(super) role: Option<String>,
    pub(super) semantic_type: String,
    pub(super) analyst_facing: bool,
}

pub(super) fn index_entity_indicator_labels(
    by_label: &mut BTreeMap<String, BTreeMap<String, SemanticLabelRef>>,
    unique_id: &str,
    entity: &ArchivedEntity,
    nova: &ArchivedNovaMeta,
) {
    for measure in nova.measures.iter() {
        let entry = SemanticLabelRef {
            entity: modeling_entity_ref(unique_id, entity),
            indicator: ModelingIndicatorRef {
                indicator_name: measure.name.as_str().to_string(),
                indicator_type: "measure".to_string(),
                parent_unique_id: unique_id.to_string(),
                source: Some(indicator_source_for_entity(entity).to_string()),
            },
            canonical: nova.canonical || measure.canonical,
            ref_key: format!("{unique_id}:measure.{}", measure.name.as_str()),
        };
        insert_semantic_label_ref(by_label, measure.name.as_str(), &entry);
        for synonym in measure.synonyms.iter() {
            insert_semantic_label_ref(by_label, synonym.as_str(), &entry);
        }
    }
    if let Some(metric) = nova.metric.as_ref() {
        index_metric_labels(by_label, unique_id, entity, nova.canonical, metric);
    }
    for metric in nova.metrics.iter() {
        index_metric_labels(by_label, unique_id, entity, nova.canonical, metric);
    }
}

pub(super) fn index_metric_labels(
    by_label: &mut BTreeMap<String, BTreeMap<String, SemanticLabelRef>>,
    unique_id: &str,
    entity: &ArchivedEntity,
    entity_canonical: bool,
    metric: &ArchivedNovaMetric,
) {
    let entry = SemanticLabelRef {
        entity: modeling_entity_ref(unique_id, entity),
        indicator: modeling_metric_ref(unique_id, entity, metric.name.as_str()),
        canonical: entity_canonical || metric.canonical,
        ref_key: format!("{unique_id}:metric.{}", metric.name.as_str()),
    };
    insert_semantic_label_ref(by_label, metric.name.as_str(), &entry);
    for synonym in metric.synonyms.iter() {
        insert_semantic_label_ref(by_label, synonym.as_str(), &entry);
    }
}

pub(super) fn insert_semantic_label_ref(
    by_label: &mut BTreeMap<String, BTreeMap<String, SemanticLabelRef>>,
    label: &str,
    entry: &SemanticLabelRef,
) {
    let label = normalize_value(label);
    if label.is_empty() {
        return;
    }
    by_label
        .entry(label)
        .or_default()
        .entry(entry.ref_key.clone())
        .or_insert_with(|| entry.clone());
}

pub(super) fn semantic_label_entities(refs: &[SemanticLabelRef]) -> Vec<ModelingEntityRef> {
    let mut seen = BTreeSet::new();
    let mut entities = Vec::new();
    for entry in refs.iter().take(12) {
        if seen.insert(entry.entity.unique_id.clone()) {
            entities.push(entry.entity.clone());
        }
        if entities.len() >= 8 {
            break;
        }
    }
    entities
}

pub(super) fn semantic_label_indicators(refs: &[SemanticLabelRef]) -> Vec<ModelingIndicatorRef> {
    refs.iter()
        .take(8)
        .map(|entry| entry.indicator.clone())
        .collect()
}

pub(super) fn semantic_label_drill_down_hints(label: &str) -> Vec<JsonValue> {
    vec![json!({
        "purpose": "search_indicator",
        "tool": "search_indicator",
        "arguments": {
            "query": label,
            "indicator_types": ["metric", "measure"],
            "limit": 10,
            "detail": "compact"
        }
    })]
}

pub(super) fn collect_column_role_conflict_findings(
    by_semantic_type: BTreeMap<String, Vec<ColumnSemanticRef>>,
    findings: &mut Vec<AgentModellingFinding>,
) {
    for (semantic_type, refs) in by_semantic_type {
        let roles = refs
            .iter()
            .filter_map(|entry| entry.role.as_ref().map(|role| normalize_value(role)))
            .filter(|role| !role.is_empty())
            .collect::<BTreeSet<_>>();
        if roles.len() <= 1 {
            continue;
        }
        findings.push(AgentModellingFinding {
            code: "column_semantic_role_conflict",
            severity: AgentModellingSeverity::Medium,
            category: "column_semantics",
            message: format!("Column semantic type `{semantic_type}` appears with multiple roles."),
            entities: column_semantic_entities(&refs),
            indicators: Vec::new(),
            evidence: json!({
                "semantic_type": semantic_type,
                "roles": roles.into_iter().collect::<Vec<_>>(),
                "columns": column_semantic_refs_json(&refs)
            }),
            recommendation: "Normalize column roles or use more precise semantic types."
                .to_string(),
            drill_down_hints: column_semantic_drill_down_hints(&refs),
        });
    }
}

pub(super) fn collect_column_name_drift_findings(
    by_column_name: BTreeMap<String, Vec<ColumnSemanticRef>>,
    findings: &mut Vec<AgentModellingFinding>,
) {
    for (column_name, refs) in by_column_name {
        let semantic_types = refs
            .iter()
            .map(|entry| entry.semantic_type.clone())
            .filter(|semantic_type| !semantic_type.is_empty())
            .collect::<BTreeSet<_>>();
        if semantic_types.len() <= 1 || !refs.iter().any(|entry| entry.analyst_facing) {
            continue;
        }
        findings.push(AgentModellingFinding {
            code: "column_name_semantic_drift",
            severity: AgentModellingSeverity::Medium,
            category: "column_semantics",
            message: format!("Column name `{column_name}` maps to multiple semantic types."),
            entities: column_semantic_entities(&refs),
            indicators: Vec::new(),
            evidence: json!({
                "column_name": column_name,
                "semantic_types": semantic_types.into_iter().collect::<Vec<_>>(),
                "columns": column_semantic_refs_json(&refs)
            }),
            recommendation:
                "Rename ambiguous columns or add semantic_type/synonyms to disambiguate."
                    .to_string(),
            drill_down_hints: column_semantic_drill_down_hints(&refs),
        });
    }
}

pub(super) fn column_semantic_entities(refs: &[ColumnSemanticRef]) -> Vec<ModelingEntityRef> {
    let mut seen = BTreeSet::new();
    let mut entities = Vec::new();
    for entry in refs.iter().take(12) {
        if seen.insert(entry.entity.unique_id.clone()) {
            entities.push(entry.entity.clone());
        }
        if entities.len() >= 8 {
            break;
        }
    }
    entities
}

pub(super) fn column_semantic_refs_json(refs: &[ColumnSemanticRef]) -> Vec<JsonValue> {
    refs.iter()
        .take(12)
        .map(|entry| {
            json!({
                "entity_unique_id": entry.entity.unique_id.as_str(),
                "column_name": entry.column_name.as_str(),
                "role": entry.role.as_deref(),
                "semantic_type": entry.semantic_type.as_str(),
                "analyst_facing": entry.analyst_facing
            })
        })
        .collect()
}

pub(super) fn column_semantic_drill_down_hints(refs: &[ColumnSemanticRef]) -> Vec<JsonValue> {
    refs.first()
        .map(|entry| entity_columns_drill_down_hints(&entry.entity.unique_id))
        .unwrap_or_default()
}

pub(super) fn direct_parent_ids<'a>(search: &'a ManifestSearch, unique_id: &str) -> &'a [String] {
    search.parent_map.get(unique_id).map_or(&[], Vec::as_slice)
}

pub(super) fn direct_parent_refs(
    context: &AgentModellingContext<'_>,
    unique_id: &str,
) -> Result<Vec<ModelingEntityRef>> {
    let mut refs = Vec::new();
    for parent_id in direct_parent_ids(context.search, unique_id).iter().take(12) {
        if let Some(parent) = context.search.entities.get_archived(parent_id)? {
            refs.push(modeling_entity_ref(parent_id, parent));
        }
    }
    Ok(refs)
}

pub(super) fn source_direct_parent_refs(
    context: &AgentModellingContext<'_>,
    unique_id: &str,
) -> Result<Vec<ModelingEntityRef>> {
    let mut refs = Vec::new();
    for parent_id in direct_parent_ids(context.search, unique_id) {
        let Some(parent) = context.search.entities.get_archived(parent_id)? else {
            continue;
        };
        if parent.resource_type_str() == Some("source") {
            refs.push(modeling_entity_ref(parent_id, parent));
        }
    }
    Ok(refs)
}

pub(super) fn fact_like_direct_parents(
    context: &AgentModellingContext<'_>,
    unique_id: &str,
) -> Result<Vec<FactLikeParent>> {
    let mut parents = Vec::new();
    for parent_id in direct_parent_ids(context.search, unique_id) {
        let Some(parent) = context.search.entities.get_archived(parent_id)? else {
            continue;
        };
        if is_fact_like_entity(parent) {
            parents.push(FactLikeParent {
                entity: modeling_entity_ref(parent_id, parent),
                grain_signatures: entity_grain_signatures(parent),
            });
        }
    }
    Ok(parents)
}

pub(super) fn fact_parent_entity_refs(parents: &[FactLikeParent]) -> Vec<ModelingEntityRef> {
    parents
        .iter()
        .take(8)
        .map(|parent| parent.entity.clone())
        .collect()
}

pub(super) fn fact_parent_grain_signatures(parents: &[FactLikeParent]) -> Vec<String> {
    let mut signatures = BTreeSet::new();
    for parent in parents {
        signatures.extend(parent.grain_signatures.iter().cloned());
    }
    signatures.into_iter().collect()
}

pub(super) fn is_fact_like_entity(entity: &ArchivedEntity) -> bool {
    let name = entity
        .name_str()
        .map(|value| value.trim().to_ascii_lowercase())
        .unwrap_or_default();
    name.starts_with("fct_")
        || name.starts_with("fact_")
        || entity
            .nova_meta()
            .is_some_and(entity_exposes_metric_or_measure)
        || entity_has_measure_role_columns(entity)
}

pub(super) fn entity_has_measure_role_columns(entity: &ArchivedEntity) -> bool {
    entity.column_meta().iter().any(|column| {
        column.role.as_ref().is_some_and(|role| {
            matches!(
                normalize_value(role.as_str()).as_str(),
                "fact" | "measure" | "metric"
            )
        })
    })
}

pub(super) fn entity_grain_signatures(entity: &ArchivedEntity) -> Vec<String> {
    let mut signatures = BTreeSet::new();
    for variant in build_entity_grain_variants(entity.nova_meta()) {
        if let Some(signature) = grain_variant_signature(&variant) {
            signatures.insert(signature);
        }
    }
    signatures.into_iter().collect()
}

pub(super) fn grain_variant_signature(variant: &GrainVariant) -> Option<String> {
    if variant.primary_key.is_empty()
        && variant.time_field.is_none()
        && variant.dimensions.is_empty()
    {
        return None;
    }
    Some(format!(
        "primary_key={};time_field={};dimensions={}",
        variant.primary_key.join(","),
        variant.time_field.as_deref().unwrap_or(""),
        variant.dimensions.join(",")
    ))
}

pub(super) fn column_primary_key_names(entity: &ArchivedEntity) -> Vec<String> {
    entity
        .column_meta()
        .iter()
        .filter(|column| column.primary_key)
        .map(|column| column.name.as_str().to_string())
        .collect()
}

pub(super) fn nova_grain_primary_key_names(nova: &ArchivedNovaMeta) -> Vec<String> {
    nova.grain
        .as_ref()
        .map(|grain| {
            grain
                .primary_key
                .iter()
                .map(ArchivedString::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

pub(super) fn entity_exposes_indicator(nova: &ArchivedNovaMeta) -> bool {
    nova.metric.is_some() || !nova.metrics.is_empty() || !nova.measures.is_empty()
}

pub(super) fn entity_exposes_metric_or_measure(nova: &ArchivedNovaMeta) -> bool {
    entity_exposes_indicator(nova)
}

pub(super) fn has_canonical_metric_or_measure(nova: &ArchivedNovaMeta) -> bool {
    (nova.canonical && entity_exposes_indicator(nova))
        || nova.measures.iter().any(|measure| measure.canonical)
        || nova.metric.as_ref().is_some_and(|metric| metric.canonical)
        || nova.metrics.iter().any(|metric| metric.canonical)
}

pub(super) fn canonical_indicator_refs_for_entity(
    unique_id: &str,
    entity: &ArchivedEntity,
    nova: &ArchivedNovaMeta,
) -> Vec<ModelingIndicatorRef> {
    if nova.canonical {
        return indicator_refs_for_entity(unique_id, entity, nova);
    }
    let source = Some(indicator_source_for_entity(entity).to_string());
    let mut indicators = Vec::new();
    indicators.extend(
        nova.measures
            .iter()
            .filter(|measure| measure.canonical)
            .map(|measure| ModelingIndicatorRef {
                indicator_name: measure.name.as_str().to_string(),
                indicator_type: "measure".to_string(),
                parent_unique_id: unique_id.to_string(),
                source: source.clone(),
            }),
    );
    if let Some(metric) = nova.metric.as_ref()
        && metric.canonical
    {
        indicators.push(ModelingIndicatorRef {
            indicator_name: metric.name.as_str().to_string(),
            indicator_type: "metric".to_string(),
            parent_unique_id: unique_id.to_string(),
            source: source.clone(),
        });
    }
    indicators.extend(
        nova.metrics
            .iter()
            .filter(|metric| metric.canonical)
            .map(|metric| ModelingIndicatorRef {
                indicator_name: metric.name.as_str().to_string(),
                indicator_type: "metric".to_string(),
                parent_unique_id: unique_id.to_string(),
                source: source.clone(),
            }),
    );
    indicators
}

pub(super) fn metric_looks_ratio(metric: &ArchivedNovaMetric) -> bool {
    !metric_ratio_signals(metric).is_empty()
}

pub(super) fn metric_ratio_signals(metric: &ArchivedNovaMetric) -> Vec<&'static str> {
    let name = metric.name.as_str().to_ascii_lowercase();
    let mut signals = Vec::new();
    if name.contains("_per_") {
        signals.push("name_contains_per");
    }
    if name.ends_with("_rate") {
        signals.push("name_ends_with_rate");
    }
    if metric
        .expression
        .as_ref()
        .is_some_and(|expression| expression.as_str().contains('/'))
    {
        signals.push("expression_contains_division");
    }
    signals
}

pub(super) fn entity_layer(search: &ManifestSearch, entity: &ArchivedEntity) -> Option<String> {
    search
        .layer_for(entity)
        .map(|layer| layer.trim().to_ascii_lowercase())
        .filter(|layer| !layer.is_empty())
        .or_else(|| inferred_entity_layer(entity))
}

pub(super) fn inferred_entity_layer(entity: &ArchivedEntity) -> Option<String> {
    let name = entity
        .name_str()
        .map(|value| value.trim().to_ascii_lowercase())
        .unwrap_or_default();
    let path = entity
        .original_file_path_str()
        .map(|value| value.trim().replace('\\', "/").to_ascii_lowercase())
        .unwrap_or_default();
    if name.starts_with("stg_")
        || name.starts_with("stage_")
        || path.contains("/staging/")
        || path.contains("/stage/")
    {
        return Some("staging".to_string());
    }
    if name.starts_with("int_")
        || name.starts_with("intermediate_")
        || path.contains("/intermediate/")
        || path.contains("/int/")
    {
        return Some("intermediate".to_string());
    }
    if name.starts_with("mart_") || path.contains("/marts/") || path.contains("/mart/") {
        return Some("mart".to_string());
    }
    None
}

pub(super) fn is_helper_layer(layer: &str) -> bool {
    matches!(
        layer.trim().to_ascii_lowercase().as_str(),
        "staging" | "stage" | "stg" | "intermediate" | "int"
    )
}

pub(super) fn analyst_candidate_disabled(nova: &ArchivedNovaMeta) -> bool {
    nova.search
        .as_ref()
        .and_then(|search| search.candidates.as_ref())
        .is_some_and(|candidates| !candidates.analyst)
}

pub(super) fn pii_like_column_signal(column_name: &str) -> Option<&'static str> {
    const PII_TERMS: [&str; 7] = [
        "email",
        "phone",
        "address",
        "full_name",
        "first_name",
        "last_name",
        "date_of_birth",
    ];
    let normalized = column_name
        .trim()
        .to_ascii_lowercase()
        .replace(['-', ' '], "_");
    PII_TERMS.into_iter().find(|term| {
        normalized == *term
            || normalized.ends_with(&format!("_{term}"))
            || normalized.starts_with(&format!("{term}_"))
            || normalized.contains(&format!("_{term}_"))
    })
}

pub(super) fn column_governance_present(column: &JsonValue) -> bool {
    column_nova_meta_json(column).is_some_and(|nova| {
        nova.get("governance")
            .is_some_and(|governance| !governance.is_null())
    })
}

pub(super) fn is_measure_like_data_type(data_type: &str) -> bool {
    let normalized = data_type.trim().to_lowercase();
    [
        "int", "integer", "bigint", "smallint", "numeric", "decimal", "number", "float", "double",
        "real",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
}

pub(super) fn metricflow_measure_references(entity_json: &JsonValue) -> Vec<String> {
    let mut refs = BTreeSet::new();
    let Some(type_params) = entity_json.get("type_params") else {
        return Vec::new();
    };
    collect_metricflow_named_reference(type_params.get("measure"), &mut refs);
    collect_metricflow_named_reference(type_params.get("numerator"), &mut refs);
    collect_metricflow_named_reference(type_params.get("denominator"), &mut refs);
    if let Some(measures) = type_params.get("measures").and_then(JsonValue::as_array) {
        for measure in measures {
            collect_metricflow_named_reference(Some(measure), &mut refs);
        }
    }
    refs.into_iter().collect()
}

pub(super) fn collect_metricflow_named_reference(
    value: Option<&JsonValue>,
    refs: &mut BTreeSet<String>,
) {
    match value {
        Some(JsonValue::String(name)) if !name.trim().is_empty() => {
            refs.insert(name.trim().to_string());
        }
        Some(JsonValue::Object(object)) => {
            if let Some(name) = object.get("name").and_then(JsonValue::as_str)
                && !name.trim().is_empty()
            {
                refs.insert(name.trim().to_string());
            }
        }
        Some(JsonValue::Array(items)) => {
            for item in items {
                collect_metricflow_named_reference(Some(item), refs);
            }
        }
        Some(_) | None => {}
    }
}

pub(super) fn indicator_source_for_entity(entity: &ArchivedEntity) -> &'static str {
    match entity.resource_type_str() {
        Some("metric") => "dbt_metric",
        Some("semantic_model") => "dbt_semantic_model",
        _ => "nova_meta",
    }
}

pub(super) fn modeling_entity_ref(unique_id: &str, entity: &ArchivedEntity) -> ModelingEntityRef {
    ModelingEntityRef {
        unique_id: unique_id.to_string(),
        name: entity.name_str().unwrap_or(unique_id).to_string(),
        resource_type: entity.resource_type_str().unwrap_or("unknown").to_string(),
        relation_name: entity.relation_name_str().map(str::to_string),
    }
}

pub(super) fn modeling_entity_ref_from_entity_ref(entity: &EntityRef) -> ModelingEntityRef {
    ModelingEntityRef {
        unique_id: entity.unique_id.clone(),
        name: entity.name.clone(),
        resource_type: entity.resource_type.clone(),
        relation_name: entity.relation_name.clone(),
    }
}

pub(super) fn duplicate_parent_entity_refs(
    parents: &[DuplicateIndicatorParent],
) -> Vec<ModelingEntityRef> {
    parents
        .iter()
        .take(8)
        .map(|parent| ModelingEntityRef {
            unique_id: parent.unique_id.clone(),
            name: parent.name.clone(),
            resource_type: parent.resource_type.clone(),
            relation_name: parent.relation_name.clone(),
        })
        .collect()
}

pub(super) fn duplicate_indicator_refs(row: &DuplicateIndicatorRow) -> Vec<ModelingIndicatorRef> {
    row.parents
        .iter()
        .take(8)
        .map(|parent| ModelingIndicatorRef {
            indicator_name: row.indicator_name.clone(),
            indicator_type: row.indicator_type.clone(),
            parent_unique_id: parent.unique_id.clone(),
            source: Some(indicator_source_for_resource_type(&parent.resource_type).to_string()),
        })
        .collect()
}

pub(super) fn indicator_source_for_resource_type(resource_type: &str) -> &'static str {
    match resource_type {
        "metric" => "dbt_metric",
        "semantic_model" => "dbt_semantic_model",
        _ => "nova_meta",
    }
}

pub(super) fn modeling_metric_ref(
    unique_id: &str,
    entity: &ArchivedEntity,
    metric_name: &str,
) -> ModelingIndicatorRef {
    ModelingIndicatorRef {
        indicator_name: metric_name.to_string(),
        indicator_type: "metric".to_string(),
        parent_unique_id: unique_id.to_string(),
        source: Some(indicator_source_for_entity(entity).to_string()),
    }
}

pub(super) fn indicator_refs_for_entity(
    unique_id: &str,
    entity: &ArchivedEntity,
    nova: &ArchivedNovaMeta,
) -> Vec<ModelingIndicatorRef> {
    let source = Some(indicator_source_for_entity(entity).to_string());
    let mut indicators = Vec::new();
    indicators.extend(nova.measures.iter().map(|measure| ModelingIndicatorRef {
        indicator_name: measure.name.as_str().to_string(),
        indicator_type: "measure".to_string(),
        parent_unique_id: unique_id.to_string(),
        source: source.clone(),
    }));
    if let Some(metric) = nova.metric.as_ref() {
        indicators.push(ModelingIndicatorRef {
            indicator_name: metric.name.as_str().to_string(),
            indicator_type: "metric".to_string(),
            parent_unique_id: unique_id.to_string(),
            source: source.clone(),
        });
    }
    indicators.extend(nova.metrics.iter().map(|metric| ModelingIndicatorRef {
        indicator_name: metric.name.as_str().to_string(),
        indicator_type: "metric".to_string(),
        parent_unique_id: unique_id.to_string(),
        source: source.clone(),
    }));
    indicators
}

pub(super) fn semantic_model_measure_refs(
    unique_id: &str,
    entity: &ArchivedEntity,
    nova: &ArchivedNovaMeta,
) -> Vec<ModelingIndicatorRef> {
    nova.measures
        .iter()
        .take(8)
        .map(|measure| ModelingIndicatorRef {
            indicator_name: measure.name.as_str().to_string(),
            indicator_type: "measure".to_string(),
            parent_unique_id: unique_id.to_string(),
            source: Some(indicator_source_for_entity(entity).to_string()),
        })
        .collect()
}

pub(super) fn metric_missing_time_field_finding(
    unique_id: &str,
    entity: &ArchivedEntity,
    metric: &ArchivedNovaMetric,
) -> AgentModellingFinding {
    let metric_name = metric.name.as_str();
    AgentModellingFinding {
        code: "metric_missing_time_field",
        severity: AgentModellingSeverity::High,
        category: "grain_safety",
        message: format!("Metric `{metric_name}` has no effective time field."),
        entities: vec![modeling_entity_ref(unique_id, entity)],
        indicators: vec![modeling_metric_ref(unique_id, entity, metric_name)],
        evidence: json!({
            "metric_name": metric_name,
            "time_field_present": false,
            "parent_resource_type": entity.resource_type_str().unwrap_or("unknown")
        }),
        recommendation: "Add `meta.nova.metric.grain.time_field` or expose the metric through a semantic artifact with a valid time dimension.".to_string(),
        drill_down_hints: entity_drill_down_hints(unique_id),
    }
}

pub(super) fn entity_drill_down_hints(unique_id: &str) -> Vec<JsonValue> {
    vec![json!({
        "purpose": "inspect_entity",
        "tool": "get_entity",
        "arguments": {
            "id_or_name": unique_id,
            "detail": "standard"
        }
    })]
}

pub(super) fn entity_columns_drill_down_hints(unique_id: &str) -> Vec<JsonValue> {
    vec![
        json!({
            "purpose": "inspect_entity",
            "tool": "get_entity",
            "arguments": {
                "id_or_name": unique_id,
                "detail": "standard"
            }
        }),
        json!({
            "purpose": "inspect_columns",
            "tool": "get_columns",
            "arguments": {
                "id_or_name": unique_id
            }
        }),
    ]
}

pub(super) fn duplicate_indicator_drill_down_hints(row: &DuplicateIndicatorRow) -> Vec<JsonValue> {
    let mut hints = vec![json!({
        "purpose": "search_indicator",
        "tool": "search_indicator",
        "arguments": {
            "query": &row.indicator_name,
            "indicator_types": [&row.indicator_type],
            "limit": 5,
            "detail": "compact"
        }
    })];
    if row.parents.len() >= 2 {
        hints.push(json!({
            "purpose": "compare_top_parent_grains",
            "tool": "compare_grains",
            "arguments": {
                "entity1": &row.parents[0].unique_id,
                "entity2": &row.parents[1].unique_id
            }
        }));
    }
    hints
}

pub(super) fn truncate_agent_modelling_findings(
    findings: &[AgentModellingFinding],
    limit: usize,
) -> Vec<AgentModellingFinding> {
    findings.iter().take(limit).cloned().collect()
}

pub(super) fn sort_agent_modelling_findings(findings: &mut [AgentModellingFinding]) {
    findings.sort_by(compare_agent_modelling_findings);
}

pub(super) fn compare_agent_modelling_findings(
    left: &AgentModellingFinding,
    right: &AgentModellingFinding,
) -> Ordering {
    left.severity
        .sort_rank()
        .cmp(&right.severity.sort_rank())
        .then_with(|| left.category.cmp(right.category))
        .then_with(|| left.code.cmp(right.code))
        .then_with(|| first_finding_entity_id(left).cmp(first_finding_entity_id(right)))
        .then_with(|| first_finding_indicator_name(left).cmp(first_finding_indicator_name(right)))
        .then_with(|| left.message.cmp(&right.message))
}

pub(super) fn first_finding_entity_id(finding: &AgentModellingFinding) -> &str {
    finding
        .entities
        .first()
        .map_or("", |entity| entity.unique_id.as_str())
}

pub(super) fn first_finding_indicator_name(finding: &AgentModellingFinding) -> &str {
    finding
        .indicators
        .first()
        .map_or("", |indicator| indicator.indicator_name.as_str())
}

pub(super) fn agent_modelling_summary(
    findings: &[AgentModellingFinding],
    truncated: bool,
) -> JsonValue {
    let mut severity_counts = BTreeMap::<&'static str, usize>::new();
    for severity in AGENT_MODELLING_SEVERITY_ORDER {
        severity_counts.insert(severity.summary_key(), 0);
    }
    let mut code_counts = BTreeMap::<String, usize>::new();
    let mut category_counts = BTreeMap::<String, usize>::new();

    for finding in findings {
        *severity_counts
            .entry(finding.severity.summary_key())
            .or_default() += 1;
        *code_counts.entry(finding.code.to_string()).or_default() += 1;
        *category_counts
            .entry(finding.category.to_string())
            .or_default() += 1;
    }

    json!({
        "total": findings.len(),
        "blockers": severity_counts["blockers"],
        "high": severity_counts["high"],
        "medium": severity_counts["medium"],
        "low": severity_counts["low"],
        "truncated": truncated,
        "top_codes": top_agent_modelling_buckets(code_counts, "code"),
        "top_categories": top_agent_modelling_buckets(category_counts, "category")
    })
}

pub(super) fn top_agent_modelling_buckets(
    counts: BTreeMap<String, usize>,
    key: &str,
) -> Vec<JsonValue> {
    let mut rows = counts.into_iter().collect::<Vec<_>>();
    rows.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    rows.into_iter()
        .take(AGENT_MODELLING_TOP_BUCKETS)
        .map(|(value, count)| {
            let mut row = JsonMap::new();
            row.insert(key.to_string(), JsonValue::String(value));
            row.insert("count".to_string(), JsonValue::from(count));
            JsonValue::Object(row)
        })
        .collect()
}

pub(super) fn overlap_evidence_category_counts(rows: &[EntityOverlapRow]) -> JsonValue {
    let mut counts = BTreeMap::<String, usize>::new();
    for row in rows {
        for category in overlap_evidence_categories_for_row(row) {
            if let Some(category) = category.as_str() {
                *counts.entry(category.to_string()).or_default() += 1;
            }
        }
    }
    serde_json::to_value(counts).unwrap_or(JsonValue::Null)
}

pub(super) fn overlap_evidence_categories_for_row(row: &EntityOverlapRow) -> Vec<JsonValue> {
    let mut categories = Vec::new();
    if !row.evidence.shared_name_tokens.is_empty() {
        categories.push(json!("shared_name_tokens"));
    }
    if !row.evidence.shared_column_names.is_empty() {
        categories.push(json!("shared_column_names"));
    }
    if !row.evidence.shared_parent_synonyms.is_empty() {
        categories.push(json!("shared_parent_synonyms"));
    }
    if !row.evidence.shared_domains.is_empty() {
        categories.push(json!("shared_domains"));
    }
    if !row.evidence.shared_indicators.is_empty() {
        categories.push(json!("shared_indicators"));
    }
    if !row.evidence.shared_column_semantic_types.is_empty() {
        categories.push(json!("shared_column_semantic_types"));
    }
    if !row.evidence.shared_dimensions.is_empty() {
        categories.push(json!("shared_dimensions"));
    }
    if row.evidence.shared_time_field.is_some() {
        categories.push(json!("shared_time_field"));
    }
    categories
}

pub(super) fn duplicate_indicator_summary_row(row: &DuplicateIndicatorRow) -> JsonValue {
    json!({
        "indicator_name": &row.indicator_name,
        "indicator_type": &row.indicator_type,
        "parent_count": row.parent_count,
        "canonical_parent_count": row.canonical_parent_count,
        "parents_without_grain": row.parents_without_grain,
        "inconsistent_grains": row.inconsistent_grains,
        "parent_examples": row.parents.iter().take(5).map(|parent| {
            json!({
                "unique_id": &parent.unique_id,
                "name": &parent.name,
                "resource_type": &parent.resource_type,
                "canonical": parent.canonical
            })
        }).collect::<Vec<_>>(),
        "grain_variant_count": row.grain_signatures.len()
    })
}

pub(super) fn modelling_drill_down_hints(
    params: &ModellingConsistencyReportParams,
    section_limit: usize,
    section_offset: usize,
    overlap_rows: &[EntityOverlapRow],
    duplicate_indicator_rows: &[DuplicateIndicatorRow],
    canonical_conflict_rows: &[DuplicateIndicatorRow],
    multi_grain_entity_rows: &[MultiGrainEntityRow],
) -> Vec<JsonValue> {
    let mut hints = Vec::new();
    if modelling_has_next_page(
        section_limit,
        section_offset,
        overlap_rows,
        duplicate_indicator_rows,
        canonical_conflict_rows,
        multi_grain_entity_rows,
    ) {
        hints.push(json!({
            "purpose": "fetch_next_report_page",
            "tool": "modelling_consistency_report",
            "arguments": {
                "resource_types": &params.resource_types,
                "limit": section_limit,
                "offset": section_offset.saturating_add(section_limit),
                "min_score": params.min_score
            }
        }));
    }
    if let Some(row) = overlap_rows.first() {
        hints.push(json!({
            "purpose": "inspect_top_overlap_pair",
            "tool": "find_entity_overlap",
            "arguments": {
                "id_or_name": &row.entity1.unique_id,
                "resource_type": &row.entity1.resource_type,
                "resource_types": &params.resource_types,
                "limit": 10,
                "offset": 0
            }
        }));
    }
    if let Some(row) = multi_grain_entity_rows.first() {
        hints.push(json!({
            "purpose": "compare_multi_grain_entity_with_related_model",
            "tool": "compare_grains",
            "arguments": {
                "entity1": &row.entity.unique_id,
                "entity2": "__RELATED_ENTITY_ID__"
            }
        }));
    }
    hints
}

pub(super) fn modelling_has_next_page(
    section_limit: usize,
    section_offset: usize,
    overlap_rows: &[EntityOverlapRow],
    duplicate_indicator_rows: &[DuplicateIndicatorRow],
    canonical_conflict_rows: &[DuplicateIndicatorRow],
    multi_grain_entity_rows: &[MultiGrainEntityRow],
) -> bool {
    [
        overlap_rows.len(),
        duplicate_indicator_rows.len(),
        canonical_conflict_rows.len(),
        multi_grain_entity_rows.len(),
    ]
    .into_iter()
    .any(|total| section_offset.saturating_add(section_limit) < total)
}

pub(super) fn grain_signature_key(grain: &GrainVariant) -> String {
    let mut primary_key = grain.primary_key.clone();
    primary_key.sort();
    primary_key.dedup();

    let mut dimensions = grain.dimensions.clone();
    dimensions.sort();
    dimensions.dedup();

    format!(
        "pk={};time={};dim={}",
        primary_key.join(","),
        grain.time_field.clone().unwrap_or_default(),
        dimensions.join(",")
    )
}

pub(super) fn sorted_intersection(
    values1: &BTreeSet<String>,
    values2: &BTreeSet<String>,
) -> Vec<String> {
    values1.intersection(values2).cloned().collect()
}

pub(super) fn sorted_difference(
    values1: &BTreeSet<String>,
    values2: &BTreeSet<String>,
) -> Vec<String> {
    values1.difference(values2).cloned().collect()
}

pub(super) fn best_grain_pair<'a>(
    left: &'a EntityOverlapProfile,
    right: &'a EntityOverlapProfile,
) -> Option<(&'a GrainVariant, &'a GrainVariant)> {
    let mut best_pair: Option<(&GrainVariant, &GrainVariant)> = None;
    let mut best_score: Option<(u8, u8, usize, usize, usize, usize)> = None;
    let mut best_signature: Option<(String, String)> = None;

    for left_grain in &left.grain_variants {
        for right_grain in &right.grain_variants {
            let left_primary_key: BTreeSet<String> =
                left_grain.primary_key.iter().cloned().collect();
            let right_primary_key: BTreeSet<String> =
                right_grain.primary_key.iter().cloned().collect();
            let left_dimensions: BTreeSet<String> = left_grain.dimensions.iter().cloned().collect();
            let right_dimensions: BTreeSet<String> =
                right_grain.dimensions.iter().cloned().collect();
            let shared_primary_key = left_primary_key.intersection(&right_primary_key).count();
            let shared_dimensions = left_dimensions.intersection(&right_dimensions).count();
            let same_time_field = left_grain
                .time_field
                .as_deref()
                .zip(right_grain.time_field.as_deref())
                .is_some_and(|(left_time, right_time)| left_time == right_time);
            let exact_match = grain_signature_key(left_grain) == grain_signature_key(right_grain);
            let diff_count = left_primary_key
                .symmetric_difference(&right_primary_key)
                .count()
                + left_dimensions
                    .symmetric_difference(&right_dimensions)
                    .count()
                + usize::from(left_grain.time_field != right_grain.time_field);
            let score = (
                u8::from(exact_match),
                u8::from(same_time_field),
                shared_primary_key + shared_dimensions,
                shared_primary_key,
                shared_dimensions,
                usize::MAX - diff_count,
            );
            let signature = (
                grain_signature_key(left_grain),
                grain_signature_key(right_grain),
            );
            let replace = match (&best_score, &best_signature) {
                (Some(current_score), Some(current_signature)) => {
                    score > *current_score
                        || (score == *current_score && signature < *current_signature)
                }
                _ => true,
            };
            if replace {
                best_score = Some(score);
                best_signature = Some(signature);
                best_pair = Some((left_grain, right_grain));
            }
        }
    }

    best_pair
}

pub(super) fn score_from_overlap(surface_overlap_count: usize, shared_value_count: usize) -> f32 {
    let combined = surface_overlap_count.saturating_mul(10) + shared_value_count;
    f32::from(u16::try_from(combined).unwrap_or(u16::MAX))
}

pub(super) fn normalize_value(value: &str) -> String {
    value.trim().to_lowercase()
}

pub(super) fn is_distinctive_column_name(value: &str, min_word_len: usize) -> bool {
    let tokens = tokenize_alnum_lowercase(value, min_word_len);
    tokens.len() >= 2
        || (tokens.len() == 1 && tokens[0].chars().count() >= min_word_len.saturating_mul(3))
}

pub(super) fn normalized_resource_type_filter(
    resource_types: &[String],
) -> Option<HashSet<String>> {
    if resource_types.is_empty() {
        return None;
    }
    Some(
        resource_types
            .iter()
            .map(|resource_type| normalize_value(resource_type))
            .filter(|resource_type| !resource_type.is_empty())
            .collect(),
    )
}

pub(super) fn resource_type_allowed(
    resource_type: Option<&str>,
    allowed_resource_types: Option<&HashSet<String>>,
) -> bool {
    let Some(allowed_resource_types) = allowed_resource_types else {
        return true;
    };
    let Some(resource_type) = resource_type else {
        return false;
    };
    allowed_resource_types.contains(&normalize_value(resource_type))
}

pub(super) fn compare_overlap_rows(left: &EntityOverlapRow, right: &EntityOverlapRow) -> Ordering {
    right
        .score
        .partial_cmp(&left.score)
        .unwrap_or(Ordering::Equal)
        .then_with(|| right.surface_overlap_count.cmp(&left.surface_overlap_count))
        .then_with(|| right.shared_value_count.cmp(&left.shared_value_count))
        .then_with(|| left.entity1.unique_id.cmp(&right.entity1.unique_id))
        .then_with(|| left.entity2.unique_id.cmp(&right.entity2.unique_id))
}

pub(super) fn compare_duplicate_indicator_rows(
    left: &DuplicateIndicatorRow,
    right: &DuplicateIndicatorRow,
) -> Ordering {
    right
        .parent_count
        .cmp(&left.parent_count)
        .then_with(|| {
            right
                .canonical_parent_count
                .cmp(&left.canonical_parent_count)
        })
        .then_with(|| left.indicator_type.cmp(&right.indicator_type))
        .then_with(|| left.indicator_name.cmp(&right.indicator_name))
}

pub(super) fn paginate_section<T: Clone>(rows: &[T], offset: usize, limit: usize) -> Vec<T> {
    rows.iter().skip(offset).take(limit).cloned().collect()
}
