use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use rkyv::string::ArchivedString;
use serde::Serialize;
use serde_json::{Map as JsonMap, Value as JsonValue, json};
use tracing::instrument;

use crate::error::{DbtNovaError, Result};
use crate::manifest::entity::{
    ArchivedEntity, ArchivedNovaGrain, ArchivedNovaMeta, ArchivedNovaMetric, column_nova_meta_json,
};
use crate::manifest::search::ManifestSearch;
use crate::manifest::store::EntityStore;
use crate::params::{
    CompareGrainsParams, FindEntityOverlapParams, ModellingConsistencyReportParams,
};
use crate::responses::SuccessResponse;
use crate::utils::tokenize_alnum_lowercase;

const AGENT_MODELLING_SCHEMA_VERSION: &str = "agent_modelling.v1";
const AGENT_MODELLING_MAX_FINDINGS: usize = 100;
const AGENT_MODELLING_TOP_BUCKETS: usize = 5;
const AGENT_SURFACE_TOO_MANY_PARENTS_THRESHOLD: usize = 7;
const AGENT_MODELLING_SEVERITY_ORDER: [AgentModellingSeverity; 4] = [
    AgentModellingSeverity::Blocker,
    AgentModellingSeverity::High,
    AgentModellingSeverity::Medium,
    AgentModellingSeverity::Low,
];

impl ManifestSearch {
    /// Compare effective grain information between two entities.
    ///
    /// # Errors
    /// Returns an error if either entity cannot be resolved or loaded.
    #[instrument(skip(self, params), fields(tool = "compare_grains", entity1 = %params.entity1, entity2 = %params.entity2))]
    pub async fn compare_grains(&self, params: &CompareGrainsParams) -> Result<JsonValue> {
        let left_id =
            self.resolve_single_id(&params.entity1, params.entity1_resource_type.as_deref())?;
        let right_id =
            self.resolve_single_id(&params.entity2, params.entity2_resource_type.as_deref())?;
        let left = self.get_entity_archived(&left_id)?.ok_or_else(|| {
            self.entity_not_found(&left_id, params.entity1_resource_type.as_deref())
        })?;
        let right = self.get_entity_archived(&right_id)?.ok_or_else(|| {
            self.entity_not_found(&right_id, params.entity2_resource_type.as_deref())
        })?;

        let left_profile =
            build_entity_profile(&left_id, left, self.config.search.min_word_length.max(1));
        let right_profile =
            build_entity_profile(&right_id, right, self.config.search.min_word_length.max(1));
        let comparison = compare_entity_grains(&left_profile, &right_profile);

        Ok(serde_json::to_value(SuccessResponse::new(comparison, 1))?)
    }

    /// Find overlapping entities using shared semantic evidence.
    ///
    /// # Errors
    /// Returns an error if the focus entity cannot be resolved or pagination exceeds configured limits.
    #[instrument(skip(self, params), fields(tool = "find_entity_overlap", limit = ?params.pagination.limit, offset = params.pagination.offset))]
    pub async fn find_entity_overlap(&self, params: &FindEntityOverlapParams) -> Result<JsonValue> {
        if params.pagination.offset > self.config.search.max_offset {
            return Err(DbtNovaError::InvalidParams(format!(
                "Offset exceeds maximum of {}",
                self.config.search.max_offset
            )));
        }

        let resource_filter = normalized_resource_type_filter(&params.resource_types);
        let focus_unique_id = params
            .id_or_name
            .as_ref()
            .map(|id_or_name| self.resolve_single_id(id_or_name, params.resource_type.as_deref()))
            .transpose()?;

        let profiles = self.collect_overlap_profiles(resource_filter.as_ref())?;
        let mut rows = overlap_rows(&profiles, focus_unique_id.as_deref());
        if let Some(min_score) = params.min_score {
            rows.retain(|row| row.score >= min_score);
        }
        let total = rows.len();
        let limit = self.page_limit(params.pagination.limit);
        let start = params.pagination.offset.min(total);
        let end = (start + limit).min(total);
        let results: Vec<JsonValue> = rows
            .into_iter()
            .skip(start)
            .take(end.saturating_sub(start))
            .map(serde_json::to_value)
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|error| DbtNovaError::ServerError(error.to_string()))?;
        let count = results.len();
        let mut response = SuccessResponse::new(results, count).with_total(total);
        if total > end {
            response = response.with_truncated(true);
        }
        Ok(serde_json::to_value(response)?)
    }

    /// Project-level report for semantic overlap, grain drift, and indicator consistency.
    ///
    /// # Errors
    /// Returns an error if pagination exceeds configured limits.
    #[instrument(skip(self, params), fields(tool = "modelling_consistency_report", limit = ?params.pagination.limit, offset = params.pagination.offset))]
    pub async fn modelling_consistency_report(
        &self,
        params: &ModellingConsistencyReportParams,
    ) -> Result<JsonValue> {
        if params.pagination.offset > self.config.search.max_offset {
            return Err(DbtNovaError::InvalidParams(format!(
                "Offset exceeds maximum of {}",
                self.config.search.max_offset
            )));
        }

        let resource_filter = normalized_resource_type_filter(&params.resource_types);
        let profiles = self.collect_overlap_profiles(resource_filter.as_ref())?;
        let section_limit = self.page_limit(params.pagination.limit);
        let section_offset = params.pagination.offset;

        let mut overlap_rows_all = overlap_rows(&profiles, None);
        if let Some(min_score) = params.min_score {
            overlap_rows_all.retain(|row| row.score >= min_score);
        }
        let overlap_count = overlap_rows_all.len();
        let overlap = paginate_section(overlap_rows_all.clone(), section_offset, section_limit);

        let duplicate_indicator_rows = duplicate_indicator_rows(&profiles, usize::MAX);
        let duplicate_indicator_count = duplicate_indicator_rows.len();
        let duplicate_indicators = paginate_section(
            duplicate_indicator_rows.clone(),
            section_offset,
            section_limit,
        );
        let canonical_conflict_rows: Vec<DuplicateIndicatorRow> = duplicate_indicator_rows
            .iter()
            .filter(|row| row.canonical_parent_count > 1)
            .cloned()
            .collect();
        let canonical_conflict_count = canonical_conflict_rows.len();
        let canonical_conflicts = paginate_section(
            canonical_conflict_rows.clone(),
            section_offset,
            section_limit,
        );
        let multi_grain_entity_rows_all = multi_grain_entity_rows(&profiles);
        let multi_grain_entity_count = multi_grain_entity_rows_all.len();
        let multi_grain_entities = paginate_section(
            multi_grain_entity_rows_all.clone(),
            section_offset,
            section_limit,
        );
        let mut agent_modelling_findings_all = build_agent_modelling_findings(
            self,
            &profiles,
            &duplicate_indicator_rows,
            &multi_grain_entity_rows_all,
        )?;
        sort_agent_modelling_findings(&mut agent_modelling_findings_all);
        let agent_modelling_finding_count = agent_modelling_findings_all.len();
        let agent_modelling_findings_truncated =
            agent_modelling_finding_count > AGENT_MODELLING_MAX_FINDINGS;
        let agent_modelling_findings =
            truncate_agent_modelling_findings(&agent_modelling_findings_all);
        let summary = build_modelling_consistency_summary(
            params,
            ModellingReportPage {
                limit: section_limit,
                offset: section_offset,
            },
            &overlap_rows_all,
            &duplicate_indicator_rows,
            &canonical_conflict_rows,
            &multi_grain_entity_rows_all,
            AgentModellingSummaryInput {
                findings: &agent_modelling_findings_all,
                truncated: agent_modelling_findings_truncated,
            },
        );

        let report = ModellingConsistencyReport {
            summary,
            agent_modelling_schema_version: AGENT_MODELLING_SCHEMA_VERSION,
            entity_count: profiles.len(),
            overlap_candidate_count: overlap_count,
            duplicate_indicator_count,
            canonical_conflict_count,
            multi_grain_entity_count,
            agent_modelling_finding_count,
            overlap_candidates: overlap,
            duplicate_indicators,
            canonical_indicator_conflicts: canonical_conflicts,
            entities_with_multiple_grain_variants: multi_grain_entities,
            agent_modelling_findings,
        };

        Ok(serde_json::to_value(SuccessResponse::new(report, 1))?)
    }

    fn collect_overlap_profiles(
        &self,
        resource_filter: Option<&HashSet<String>>,
    ) -> Result<Vec<EntityOverlapProfile>> {
        let mut profiles = Vec::new();
        let min_word_len = self.config.search.min_word_length.max(1);

        for unique_id in self.entities.ids() {
            let Some(entity) = self.get_entity_archived(unique_id)? else {
                continue;
            };
            if !resource_type_allowed(entity.resource_type_str(), resource_filter) {
                continue;
            }
            let profile = build_entity_profile(unique_id, entity, min_word_len);
            if !profile.is_comparable() {
                continue;
            }
            profiles.push(profile);
        }
        profiles.sort_by(|left, right| left.unique_id.cmp(&right.unique_id));
        Ok(profiles)
    }
}

#[derive(Debug, Clone, Serialize)]
struct EntityRef {
    unique_id: String,
    name: String,
    resource_type: String,
    relation_name: Option<String>,
    canonical: bool,
}

#[derive(Debug, Clone, Serialize)]
struct GrainVariant {
    sources: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    primary_key: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    time_field: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    dimensions: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
struct GrainComparison {
    entity1: EntityRef,
    entity2: EntityRef,
    entity1_grain_variants: Vec<GrainVariant>,
    entity2_grain_variants: Vec<GrainVariant>,
    exact_match: bool,
    same_time_field: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    shared_primary_key: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    entity1_only_primary_key: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    entity2_only_primary_key: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    shared_dimensions: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    entity1_only_dimensions: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    entity2_only_dimensions: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[allow(clippy::struct_field_names)]
struct EntityOverlapEvidence {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    shared_name_tokens: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    shared_column_names: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    shared_parent_synonyms: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    shared_domains: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    shared_indicators: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    shared_column_semantic_types: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    shared_dimensions: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    shared_time_field: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct EntityOverlapRow {
    entity1: EntityRef,
    entity2: EntityRef,
    score: f32,
    surface_overlap_count: usize,
    shared_value_count: usize,
    evidence: EntityOverlapEvidence,
}

#[derive(Debug, Clone, Serialize)]
struct DuplicateIndicatorParent {
    unique_id: String,
    name: String,
    resource_type: String,
    relation_name: Option<String>,
    canonical: bool,
}

#[derive(Debug, Clone, Serialize)]
struct DuplicateIndicatorRow {
    indicator_name: String,
    indicator_type: String,
    parent_count: usize,
    canonical_parent_count: usize,
    parents_without_grain: usize,
    inconsistent_grains: bool,
    parents: Vec<DuplicateIndicatorParent>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    grain_signatures: Vec<GrainVariant>,
}

#[derive(Debug, Clone, Serialize)]
struct MultiGrainEntityRow {
    entity: EntityRef,
    grain_variant_count: usize,
    grain_variants: Vec<GrainVariant>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum AgentModellingSeverity {
    Blocker,
    High,
    Medium,
    Low,
}

impl AgentModellingSeverity {
    fn sort_rank(self) -> u8 {
        match self {
            Self::Blocker => 0,
            Self::High => 1,
            Self::Medium => 2,
            Self::Low => 3,
        }
    }

    fn summary_key(self) -> &'static str {
        match self {
            Self::Blocker => "blockers",
            Self::High => "high",
            Self::Medium => "medium",
            Self::Low => "low",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct ModelingEntityRef {
    unique_id: String,
    name: String,
    resource_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    relation_name: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct ModelingIndicatorRef {
    indicator_name: String,
    indicator_type: String,
    parent_unique_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct AgentModellingFinding {
    code: &'static str,
    severity: AgentModellingSeverity,
    category: &'static str,
    message: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    entities: Vec<ModelingEntityRef>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    indicators: Vec<ModelingIndicatorRef>,
    evidence: JsonValue,
    recommendation: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    drill_down_hints: Vec<JsonValue>,
}

#[derive(Debug, Clone, Serialize)]
struct ModellingConsistencyReport {
    summary: JsonValue,
    agent_modelling_schema_version: &'static str,
    entity_count: usize,
    overlap_candidate_count: usize,
    duplicate_indicator_count: usize,
    canonical_conflict_count: usize,
    multi_grain_entity_count: usize,
    agent_modelling_finding_count: usize,
    overlap_candidates: Vec<EntityOverlapRow>,
    duplicate_indicators: Vec<DuplicateIndicatorRow>,
    canonical_indicator_conflicts: Vec<DuplicateIndicatorRow>,
    entities_with_multiple_grain_variants: Vec<MultiGrainEntityRow>,
    agent_modelling_findings: Vec<AgentModellingFinding>,
}

#[derive(Clone)]
struct EntityOverlapProfile {
    unique_id: String,
    name: String,
    resource_type: String,
    relation_name: Option<String>,
    canonical: bool,
    name_tokens: BTreeSet<String>,
    column_names: BTreeSet<String>,
    parent_synonyms: BTreeSet<String>,
    domains: BTreeSet<String>,
    indicator_names: BTreeSet<String>,
    typed_indicators: BTreeSet<(String, String)>,
    indicator_profiles: BTreeMap<(String, String), IndicatorOverlapIndicatorProfile>,
    column_semantic_types: BTreeSet<String>,
    grain_variants: Vec<GrainVariant>,
}

#[derive(Debug, Clone, Copy)]
struct ModellingReportPage {
    limit: usize,
    offset: usize,
}

#[derive(Debug, Clone, Copy)]
struct AgentModellingSummaryInput<'a> {
    findings: &'a [AgentModellingFinding],
    truncated: bool,
}

struct AgentModellingContext<'a> {
    search: &'a ManifestSearch,
    semantic_model_measure_names: BTreeSet<String>,
    semantic_metric_names: BTreeSet<String>,
}

struct MetricSurfaceContext<'a> {
    unique_id: &'a str,
    entity: &'a ArchivedEntity,
    nova: &'a ArchivedNovaMeta,
    semantic_metric_parent: bool,
    relation_backed: bool,
    column_names: &'a BTreeSet<String>,
}

#[derive(Clone)]
struct IndicatorOverlapIndicatorProfile {
    canonical: bool,
    grain_variants: Vec<GrainVariant>,
}

impl EntityOverlapProfile {
    fn is_comparable(&self) -> bool {
        !self.parent_synonyms.is_empty()
            || !self.domains.is_empty()
            || !self.indicator_names.is_empty()
            || !self.column_names.is_empty()
            || !self.column_semantic_types.is_empty()
            || !self.grain_variants.is_empty()
            || !self.name_tokens.is_empty()
    }

    fn entity_ref(&self) -> EntityRef {
        EntityRef {
            unique_id: self.unique_id.clone(),
            name: self.name.clone(),
            resource_type: self.resource_type.clone(),
            relation_name: self.relation_name.clone(),
            canonical: self.canonical,
        }
    }
}

fn build_entity_profile(
    unique_id: &str,
    entity: &ArchivedEntity,
    min_word_len: usize,
) -> EntityOverlapProfile {
    let nova = entity.nova_meta();
    let mut name_tokens = BTreeSet::new();
    if let Some(name) = entity.name_str() {
        name_tokens.extend(tokenize_alnum_lowercase(name, min_word_len));
    }
    if let Some(alias) = entity.alias_str() {
        name_tokens.extend(tokenize_alnum_lowercase(alias, min_word_len));
    }

    let parent_synonyms = nova.map_or_else(BTreeSet::new, |nova| {
        nova.synonyms
            .iter()
            .map(ArchivedString::as_str)
            .map(normalize_value)
            .filter(|value| !value.is_empty())
            .collect()
    });
    let column_names = entity
        .column_names_iter()
        .map(normalize_value)
        .filter(|value| is_distinctive_column_name(value, min_word_len))
        .collect();
    let domains = nova.map_or_else(BTreeSet::new, |nova| {
        nova.domains
            .iter()
            .map(ArchivedString::as_str)
            .map(normalize_value)
            .filter(|value| !value.is_empty())
            .collect()
    });

    let mut indicator_names = BTreeSet::new();
    let mut typed_indicators = BTreeSet::new();
    let entity_canonical = nova.is_some_and(|nova| nova.canonical);
    let entity_grain_variants = build_entity_grain_variants(nova);
    let indicator_profiles = build_indicator_profiles(
        nova,
        entity_canonical,
        &entity_grain_variants,
        &mut indicator_names,
        &mut typed_indicators,
    );

    let column_semantic_types = entity
        .column_meta()
        .iter()
        .filter_map(|column| column.semantic_type.as_ref().map(ArchivedString::as_str))
        .map(normalize_value)
        .filter(|value| !value.is_empty())
        .collect();

    EntityOverlapProfile {
        unique_id: unique_id.to_string(),
        name: entity.name_str().unwrap_or(unique_id).to_string(),
        resource_type: entity.resource_type_str().unwrap_or("unknown").to_string(),
        relation_name: entity.relation_name_str().map(str::to_string),
        canonical: nova.is_some_and(|nova| nova.canonical),
        name_tokens,
        column_names,
        parent_synonyms,
        domains,
        indicator_names,
        typed_indicators,
        indicator_profiles,
        column_semantic_types,
        grain_variants: build_grain_variants(nova),
    }
}

fn build_indicator_profiles(
    nova: Option<&ArchivedNovaMeta>,
    entity_canonical: bool,
    entity_grain_variants: &[GrainVariant],
    indicator_names: &mut BTreeSet<String>,
    typed_indicators: &mut BTreeSet<(String, String)>,
) -> BTreeMap<(String, String), IndicatorOverlapIndicatorProfile> {
    let mut indicator_profiles = BTreeMap::new();
    let Some(nova) = nova else {
        return indicator_profiles;
    };

    for measure in nova.measures.iter() {
        let name = normalize_value(measure.name.as_str());
        if name.is_empty() {
            continue;
        }
        indicator_names.insert(name.clone());
        let key = ("measure".to_string(), name);
        typed_indicators.insert(key.clone());
        indicator_profiles.insert(
            key,
            IndicatorOverlapIndicatorProfile {
                canonical: entity_canonical || measure.canonical,
                grain_variants: entity_grain_variants.to_vec(),
            },
        );
    }

    if let Some(metric) = nova.metric.as_ref() {
        insert_metric_indicator_profile(
            metric,
            entity_canonical,
            entity_grain_variants,
            indicator_names,
            typed_indicators,
            &mut indicator_profiles,
        );
    }
    for metric in nova.metrics.iter() {
        insert_metric_indicator_profile(
            metric,
            entity_canonical,
            entity_grain_variants,
            indicator_names,
            typed_indicators,
            &mut indicator_profiles,
        );
    }

    indicator_profiles
}

fn insert_metric_indicator_profile(
    metric: &ArchivedNovaMetric,
    entity_canonical: bool,
    entity_grain_variants: &[GrainVariant],
    indicator_names: &mut BTreeSet<String>,
    typed_indicators: &mut BTreeSet<(String, String)>,
    indicator_profiles: &mut BTreeMap<(String, String), IndicatorOverlapIndicatorProfile>,
) {
    let name = normalize_value(metric.name.as_str());
    if name.is_empty() {
        return;
    }
    indicator_names.insert(name.clone());
    let key = ("metric".to_string(), name);
    typed_indicators.insert(key.clone());
    indicator_profiles.insert(
        key,
        IndicatorOverlapIndicatorProfile {
            canonical: entity_canonical || metric.canonical,
            grain_variants: metric_grain_variants(metric, entity_grain_variants),
        },
    );
}

fn build_entity_grain_variants(nova: Option<&ArchivedNovaMeta>) -> Vec<GrainVariant> {
    nova.and_then(|nova| nova.grain.as_ref())
        .map(|grain| vec![grain_variant_from_archived("entity".to_string(), grain)])
        .unwrap_or_default()
}

fn metric_grain_variants(
    metric: &ArchivedNovaMetric,
    entity_grain_variants: &[GrainVariant],
) -> Vec<GrainVariant> {
    metric.grain.as_ref().map_or_else(
        || entity_grain_variants.to_vec(),
        |grain| {
            vec![grain_variant_from_archived(
                metric.name.as_str().to_string(),
                grain,
            )]
        },
    )
}

fn grain_variant_from_archived(source: String, grain: &ArchivedNovaGrain) -> GrainVariant {
    GrainVariant {
        sources: vec![source],
        primary_key: grain
            .primary_key
            .iter()
            .map(ArchivedString::as_str)
            .map(str::to_string)
            .collect(),
        time_field: grain
            .time_field
            .as_ref()
            .map(ArchivedString::as_str)
            .map(str::to_string),
        dimensions: grain
            .dimensions
            .iter()
            .map(ArchivedString::as_str)
            .map(str::to_string)
            .collect(),
    }
}

fn build_grain_variants(nova: Option<&ArchivedNovaMeta>) -> Vec<GrainVariant> {
    let Some(nova) = nova else {
        return Vec::new();
    };

    let mut variants: Vec<GrainVariant> = Vec::new();
    let mut positions: HashMap<String, usize> = HashMap::new();

    let mut push_variant = |source: String, grain: &ArchivedNovaGrain| {
        let candidate = grain_variant_from_archived(source, grain);
        let key = grain_signature_key(&candidate);
        if let Some(index) = positions.get(&key).copied() {
            if !variants[index]
                .sources
                .iter()
                .any(|existing| existing == &candidate.sources[0])
            {
                variants[index].sources.push(candidate.sources[0].clone());
            }
        } else {
            positions.insert(key, variants.len());
            variants.push(candidate);
        }
    };

    if let Some(grain) = nova.grain.as_ref() {
        push_variant("entity".to_string(), grain);
    }
    if let Some(metric) = nova.metric.as_ref()
        && let Some(grain) = metric.grain.as_ref()
    {
        push_variant(format!("metric:{}", metric.name.as_str()), grain);
    }
    for metric in nova.metrics.iter() {
        if let Some(grain) = metric.grain.as_ref() {
            push_variant(format!("metric:{}", metric.name.as_str()), grain);
        }
    }

    variants
}

fn compare_entity_grains(
    left: &EntityOverlapProfile,
    right: &EntityOverlapProfile,
) -> GrainComparison {
    let best_pair = best_grain_pair(left, right);
    let left_grain = best_pair.map(|(left_grain, _)| left_grain);
    let right_grain = best_pair.map(|(_, right_grain)| right_grain);

    let left_pk = left_grain.map_or_else(BTreeSet::new, |grain| {
        grain.primary_key.iter().cloned().collect()
    });
    let right_pk = right_grain.map_or_else(BTreeSet::new, |grain| {
        grain.primary_key.iter().cloned().collect()
    });
    let left_dims = left_grain.map_or_else(BTreeSet::new, |grain| {
        grain.dimensions.iter().cloned().collect()
    });
    let right_dims = right_grain.map_or_else(BTreeSet::new, |grain| {
        grain.dimensions.iter().cloned().collect()
    });
    let shared_primary_key = sorted_intersection(&left_pk, &right_pk);
    let left_only_primary_key = sorted_difference(&left_pk, &right_pk);
    let right_only_primary_key = sorted_difference(&right_pk, &left_pk);
    let shared_dimensions = sorted_intersection(&left_dims, &right_dims);
    let left_only_dimensions = sorted_difference(&left_dims, &right_dims);
    let right_only_dimensions = sorted_difference(&right_dims, &left_dims);
    let same_time_field = left_grain
        .and_then(|grain| grain.time_field.as_deref())
        .zip(right_grain.and_then(|grain| grain.time_field.as_deref()))
        .is_some_and(|(left_time, right_time)| left_time == right_time);
    let exact_match = left_grain
        .zip(right_grain)
        .is_some_and(|(left_grain, right_grain)| {
            grain_signature_key(left_grain) == grain_signature_key(right_grain)
        });

    GrainComparison {
        entity1: left.entity_ref(),
        entity2: right.entity_ref(),
        entity1_grain_variants: left.grain_variants.clone(),
        entity2_grain_variants: right.grain_variants.clone(),
        exact_match,
        same_time_field,
        shared_primary_key,
        entity1_only_primary_key: left_only_primary_key,
        entity2_only_primary_key: right_only_primary_key,
        shared_dimensions,
        entity1_only_dimensions: left_only_dimensions,
        entity2_only_dimensions: right_only_dimensions,
    }
}

fn overlap_rows(
    profiles: &[EntityOverlapProfile],
    focus_unique_id: Option<&str>,
) -> Vec<EntityOverlapRow> {
    let candidate_pairs = overlap_candidate_pairs(profiles);
    let mut rows = Vec::new();

    for (left_index, right_index) in candidate_pairs {
        let left = &profiles[left_index];
        let right = &profiles[right_index];
        if let Some(focus_unique_id) = focus_unique_id
            && left.unique_id != focus_unique_id
            && right.unique_id != focus_unique_id
        {
            continue;
        }
        let evidence = overlap_evidence(left, right);
        let surface_overlap_count = evidence.surface_overlap_count();
        if surface_overlap_count == 0 {
            continue;
        }
        let shared_value_count = evidence.shared_value_count();
        let score = score_from_overlap(surface_overlap_count, shared_value_count);
        rows.push(EntityOverlapRow {
            entity1: left.entity_ref(),
            entity2: right.entity_ref(),
            score,
            surface_overlap_count,
            shared_value_count,
            evidence,
        });
    }

    rows.sort_by(compare_overlap_rows);
    rows
}

fn overlap_candidate_pairs(profiles: &[EntityOverlapProfile]) -> BTreeSet<(usize, usize)> {
    let mut buckets: BTreeMap<String, Vec<usize>> = BTreeMap::new();

    for (index, profile) in profiles.iter().enumerate() {
        for key in overlap_bucket_keys(profile) {
            buckets.entry(key).or_default().push(index);
        }
    }

    let mut pairs = BTreeSet::new();
    for indices in buckets.into_values() {
        for (offset, left_index) in indices.iter().enumerate() {
            for right_index in indices.iter().skip(offset + 1) {
                pairs.insert((*left_index, *right_index));
            }
        }
    }
    pairs
}

fn overlap_bucket_keys(profile: &EntityOverlapProfile) -> BTreeSet<String> {
    let mut keys = BTreeSet::new();
    for token in &profile.name_tokens {
        keys.insert(format!("tok:{token}"));
    }
    for synonym in &profile.parent_synonyms {
        keys.insert(format!("syn:{synonym}"));
    }
    for domain in &profile.domains {
        keys.insert(format!("dom:{domain}"));
    }
    for indicator in &profile.indicator_names {
        keys.insert(format!("ind:{indicator}"));
    }
    for column_name in &profile.column_names {
        keys.insert(format!("col:{column_name}"));
    }
    for semantic_type in &profile.column_semantic_types {
        keys.insert(format!("stype:{semantic_type}"));
    }
    for grain in &profile.grain_variants {
        if let Some(time_field) = &grain.time_field {
            keys.insert(format!("time:{}", normalize_value(time_field)));
        }
        for dimension in &grain.dimensions {
            keys.insert(format!("dim:{}", normalize_value(dimension)));
        }
    }
    keys
}

fn overlap_evidence(
    left: &EntityOverlapProfile,
    right: &EntityOverlapProfile,
) -> EntityOverlapEvidence {
    let best_pair = best_grain_pair(left, right);
    let shared_time_field = best_pair
        .and_then(|(left_grain, right_grain)| {
            left_grain
                .time_field
                .as_deref()
                .zip(right_grain.time_field.as_deref())
        })
        .filter(|(left_time, right_time)| left_time == right_time)
        .map(|(time_field, _)| time_field.to_string());
    let shared_dimensions = best_pair.map_or_else(Vec::new, |(left_grain, right_grain)| {
        sorted_intersection(
            &left_grain.dimensions.iter().cloned().collect(),
            &right_grain.dimensions.iter().cloned().collect(),
        )
    });

    EntityOverlapEvidence {
        shared_name_tokens: sorted_intersection(&left.name_tokens, &right.name_tokens),
        shared_column_names: sorted_intersection(&left.column_names, &right.column_names),
        shared_parent_synonyms: sorted_intersection(&left.parent_synonyms, &right.parent_synonyms),
        shared_domains: sorted_intersection(&left.domains, &right.domains),
        shared_indicators: sorted_intersection(&left.indicator_names, &right.indicator_names),
        shared_column_semantic_types: sorted_intersection(
            &left.column_semantic_types,
            &right.column_semantic_types,
        ),
        shared_dimensions,
        shared_time_field,
    }
}

impl EntityOverlapEvidence {
    fn surface_overlap_count(&self) -> usize {
        usize::from(!self.shared_name_tokens.is_empty())
            + usize::from(!self.shared_column_names.is_empty())
            + usize::from(!self.shared_parent_synonyms.is_empty())
            + usize::from(!self.shared_domains.is_empty())
            + usize::from(!self.shared_indicators.is_empty())
            + usize::from(!self.shared_column_semantic_types.is_empty())
            + usize::from(!self.shared_dimensions.is_empty())
            + usize::from(self.shared_time_field.is_some())
    }

    fn shared_value_count(&self) -> usize {
        self.shared_name_tokens.len()
            + self.shared_column_names.len()
            + self.shared_parent_synonyms.len()
            + self.shared_domains.len()
            + self.shared_indicators.len()
            + self.shared_column_semantic_types.len()
            + self.shared_dimensions.len()
            + usize::from(self.shared_time_field.is_some())
    }
}

fn duplicate_indicator_rows(
    profiles: &[EntityOverlapProfile],
    limit: usize,
) -> Vec<DuplicateIndicatorRow> {
    let mut by_indicator: BTreeMap<(String, String), Vec<&EntityOverlapProfile>> = BTreeMap::new();
    for profile in profiles {
        for (indicator_type, indicator_name) in &profile.typed_indicators {
            by_indicator
                .entry((indicator_type.clone(), indicator_name.clone()))
                .or_default()
                .push(profile);
        }
    }

    let mut rows = Vec::new();
    for ((indicator_type, indicator_name), parents) in by_indicator {
        if parents.len() < 2 {
            continue;
        }
        let indicator_key = (indicator_type.clone(), indicator_name.clone());
        let canonical_parent_count = parents
            .iter()
            .filter(|profile| {
                profile
                    .indicator_profiles
                    .get(&indicator_key)
                    .is_some_and(|details| details.canonical)
            })
            .count();
        let parents_without_grain = parents
            .iter()
            .filter(|profile| {
                profile
                    .indicator_profiles
                    .get(&indicator_key)
                    .is_none_or(|details| details.grain_variants.is_empty())
            })
            .count();
        let mut grain_signatures: BTreeMap<String, GrainVariant> = BTreeMap::new();
        for profile in &parents {
            let Some(details) = profile.indicator_profiles.get(&indicator_key) else {
                continue;
            };
            for grain in &details.grain_variants {
                grain_signatures
                    .entry(grain_signature_key(grain))
                    .or_insert_with(|| grain.clone());
            }
        }
        let inconsistent_grains = grain_signatures.len() > 1
            || (parents_without_grain > 0 && parents_without_grain < parents.len());
        rows.push(DuplicateIndicatorRow {
            indicator_name,
            indicator_type,
            parent_count: parents.len(),
            canonical_parent_count,
            parents_without_grain,
            inconsistent_grains,
            parents: parents
                .iter()
                .map(|profile| DuplicateIndicatorParent {
                    unique_id: profile.unique_id.clone(),
                    name: profile.name.clone(),
                    resource_type: profile.resource_type.clone(),
                    relation_name: profile.relation_name.clone(),
                    canonical: profile
                        .indicator_profiles
                        .get(&indicator_key)
                        .is_some_and(|details| details.canonical),
                })
                .collect(),
            grain_signatures: grain_signatures.into_values().collect(),
        });
    }

    rows.sort_by(compare_duplicate_indicator_rows);
    rows.truncate(limit);
    rows
}

fn multi_grain_entity_rows(profiles: &[EntityOverlapProfile]) -> Vec<MultiGrainEntityRow> {
    let mut rows: Vec<MultiGrainEntityRow> = profiles
        .iter()
        .filter(|profile| profile.grain_variants.len() > 1)
        .map(|profile| MultiGrainEntityRow {
            entity: profile.entity_ref(),
            grain_variant_count: profile.grain_variants.len(),
            grain_variants: profile.grain_variants.clone(),
        })
        .collect();
    rows.sort_by(|left, right| {
        right
            .grain_variant_count
            .cmp(&left.grain_variant_count)
            .then_with(|| left.entity.unique_id.cmp(&right.entity.unique_id))
    });
    rows
}

fn build_modelling_consistency_summary(
    params: &ModellingConsistencyReportParams,
    page: ModellingReportPage,
    overlap_rows: &[EntityOverlapRow],
    duplicate_indicator_rows: &[DuplicateIndicatorRow],
    canonical_conflict_rows: &[DuplicateIndicatorRow],
    multi_grain_entity_rows: &[MultiGrainEntityRow],
    agent_modelling: AgentModellingSummaryInput<'_>,
) -> JsonValue {
    let overlap_evidence_categories = overlap_evidence_category_counts(overlap_rows);
    let overlap_examples = overlap_rows
        .iter()
        .take(5)
        .map(|row| {
            json!({
                "entity1": &row.entity1.unique_id,
                "entity2": &row.entity2.unique_id,
                "score": row.score,
                "surface_overlap_count": row.surface_overlap_count,
                "evidence_categories": overlap_evidence_categories_for_row(row),
                "shared_column_examples": row.evidence.shared_column_names.iter().take(5).collect::<Vec<_>>(),
                "shared_name_token_examples": row.evidence.shared_name_tokens.iter().take(5).collect::<Vec<_>>(),
                "shared_indicator_examples": row.evidence.shared_indicators.iter().take(5).collect::<Vec<_>>(),
                "shared_domain_examples": row.evidence.shared_domains.iter().take(5).collect::<Vec<_>>(),
                "shared_time_field": row.evidence.shared_time_field
            })
        })
        .collect::<Vec<_>>();
    let top_duplicate_indicator_groups = duplicate_indicator_rows
        .iter()
        .take(5)
        .map(duplicate_indicator_summary_row)
        .collect::<Vec<_>>();
    let top_canonical_conflicts = canonical_conflict_rows
        .iter()
        .take(5)
        .map(duplicate_indicator_summary_row)
        .collect::<Vec<_>>();
    let top_multi_grain_entities = multi_grain_entity_rows
        .iter()
        .take(5)
        .map(|row| {
            json!({
                "unique_id": &row.entity.unique_id,
                "name": &row.entity.name,
                "resource_type": &row.entity.resource_type,
                "grain_variant_count": row.grain_variant_count,
                "variant_sources": row.grain_variants.iter().flat_map(|variant| variant.sources.iter()).take(8).collect::<Vec<_>>()
            })
        })
        .collect::<Vec<_>>();
    let next_offset = modelling_has_next_page(
        page.limit,
        page.offset,
        overlap_rows,
        multi_grain_entity_rows,
    )
    .then_some(page.offset.saturating_add(page.limit));

    json!({
        "section_counts": {
            "overlap_candidates": overlap_rows.len(),
            "duplicate_indicators": duplicate_indicator_rows.len(),
            "canonical_indicator_conflicts": canonical_conflict_rows.len(),
            "entities_with_multiple_grain_variants": multi_grain_entity_rows.len(),
            "agent_modelling_findings": agent_modelling.findings.len()
        },
        "agent_modelling": agent_modelling_summary(
            agent_modelling.findings,
            agent_modelling.truncated
        ),
        "page": {
            "limit": page.limit,
            "offset": page.offset,
            "next_offset": next_offset
        },
        "overlap_evidence_categories": overlap_evidence_categories,
        "overlap_examples": overlap_examples,
        "top_duplicate_indicator_groups": top_duplicate_indicator_groups,
        "top_canonical_conflicts": top_canonical_conflicts,
        "top_multi_grain_entities": top_multi_grain_entities,
        "drill_down_hints": modelling_drill_down_hints(params, page.limit, page.offset, overlap_rows, multi_grain_entity_rows)
    })
}

fn build_agent_modelling_findings(
    search: &ManifestSearch,
    profiles: &[EntityOverlapProfile],
    duplicate_indicator_rows: &[DuplicateIndicatorRow],
    multi_grain_entity_rows: &[MultiGrainEntityRow],
) -> Result<Vec<AgentModellingFinding>> {
    let mut findings = Vec::new();
    let context = AgentModellingContext {
        search,
        semantic_model_measure_names: semantic_model_measure_names(&search.entities)?,
        semantic_metric_names: semantic_metric_names(&search.entities)?,
    };
    collect_duplicate_indicator_findings(duplicate_indicator_rows, &mut findings);
    collect_semantic_label_collision_findings(&context, profiles, &mut findings)?;
    collect_column_semantic_ambiguity_findings(&context, profiles, &mut findings)?;
    collect_multi_grain_entity_findings(&search.entities, multi_grain_entity_rows, &mut findings)?;
    collect_entity_agent_modelling_findings(&context, profiles, &mut findings)?;
    Ok(findings)
}

fn collect_duplicate_indicator_findings(
    duplicate_indicator_rows: &[DuplicateIndicatorRow],
    findings: &mut Vec<AgentModellingFinding>,
) {
    for row in duplicate_indicator_rows {
        if row.canonical_parent_count > 1 {
            let severity = if row.inconsistent_grains {
                AgentModellingSeverity::Blocker
            } else {
                AgentModellingSeverity::High
            };
            findings.push(AgentModellingFinding {
                code: "duplicate_canonical_indicator",
                severity,
                category: "indicator_resolution",
                message: format!(
                    "Indicator `{}` has {} canonical parents.",
                    row.indicator_name, row.canonical_parent_count
                ),
                entities: duplicate_parent_entity_refs(&row.parents),
                indicators: duplicate_indicator_refs(row),
                evidence: json!({
                    "indicator_name": &row.indicator_name,
                    "indicator_type": &row.indicator_type,
                    "parent_count": row.parent_count,
                    "canonical_parent_count": row.canonical_parent_count,
                    "inconsistent_grains": row.inconsistent_grains,
                    "grain_variant_count": row.grain_signatures.len()
                }),
                recommendation: "Choose one canonical execution surface for this business indicator at this grain. If both are legitimate, rename or domain-scope one indicator.".to_string(),
                drill_down_hints: duplicate_indicator_drill_down_hints(row),
            });
        } else if row.parent_count > 1 && row.canonical_parent_count == 0 {
            findings.push(AgentModellingFinding {
                code: "duplicate_indicator_without_canonical_parent",
                severity: AgentModellingSeverity::Medium,
                category: "indicator_resolution",
                message: format!(
                    "Indicator `{}` has multiple parents but no canonical parent.",
                    row.indicator_name
                ),
                entities: duplicate_parent_entity_refs(&row.parents),
                indicators: duplicate_indicator_refs(row),
                evidence: json!({
                    "indicator_name": &row.indicator_name,
                    "indicator_type": &row.indicator_type,
                    "parent_count": row.parent_count,
                    "canonical_parent_count": row.canonical_parent_count,
                    "inconsistent_grains": row.inconsistent_grains,
                    "grain_variant_count": row.grain_signatures.len()
                }),
                recommendation: "Mark one parent indicator canonical or clarify names so agents can choose a preferred definition.".to_string(),
                drill_down_hints: duplicate_indicator_drill_down_hints(row),
            });
        }
    }
}

fn collect_multi_grain_entity_findings(
    entities: &EntityStore,
    multi_grain_entity_rows: &[MultiGrainEntityRow],
    findings: &mut Vec<AgentModellingFinding>,
) -> Result<()> {
    for row in multi_grain_entity_rows {
        let entity = entities.get_archived(&row.entity.unique_id)?;
        let analyst_facing = entity.is_some_and(entity_is_analyst_facing);
        findings.push(AgentModellingFinding {
            code: "entity_multiple_grain_variants",
            severity: if analyst_facing {
                AgentModellingSeverity::High
            } else {
                AgentModellingSeverity::Medium
            },
            category: "grain_safety",
            message: format!(
                "Entity `{}` has {} declared grain variants.",
                row.entity.unique_id, row.grain_variant_count
            ),
            entities: vec![modeling_entity_ref_from_entity_ref(&row.entity)],
            indicators: Vec::new(),
            evidence: json!({
                "grain_variant_count": row.grain_variant_count,
                "analyst_facing": analyst_facing,
                "variant_sources": row.grain_variants.iter().flat_map(|variant| variant.sources.iter()).take(10).collect::<Vec<_>>()
            }),
            recommendation: "Separate base entity grain from metric-specific grain, or split metrics into clearer execution surfaces.".to_string(),
            drill_down_hints: vec![json!({
                "purpose": "inspect_multi_grain_entity",
                "tool": "get_entity",
                "arguments": {
                    "id_or_name": &row.entity.unique_id,
                    "detail": "standard"
                }
            })],
        });
    }
    Ok(())
}

fn collect_entity_agent_modelling_findings(
    context: &AgentModellingContext<'_>,
    profiles: &[EntityOverlapProfile],
    findings: &mut Vec<AgentModellingFinding>,
) -> Result<()> {
    for profile in profiles {
        let Some(entity) = context.search.entities.get_archived(&profile.unique_id)? else {
            continue;
        };
        let nova = entity.nova_meta();
        if let Some(nova) = nova {
            collect_indicator_parent_not_queryable_findings(
                &profile.unique_id,
                entity,
                nova,
                findings,
            );
            collect_metric_surface_findings(&profile.unique_id, entity, nova, findings);
            collect_semantic_model_grain_findings(&profile.unique_id, entity, nova, findings);
            collect_canonical_primary_key_finding(&profile.unique_id, entity, nova, findings);
            collect_cross_grain_and_multi_fact_findings(
                context,
                &profile.unique_id,
                entity,
                nova,
                findings,
            )?;
            collect_helper_layer_findings(context, &profile.unique_id, entity, nova, findings);
        }
        collect_parent_lineage_findings(context, &profile.unique_id, entity, findings)?;
        collect_governance_findings(&profile.unique_id, entity, nova, findings);
        collect_catalog_integrity_findings(&profile.unique_id, entity, nova, findings);
        collect_semantic_metric_reference_findings(
            &profile.unique_id,
            entity,
            &context.semantic_model_measure_names,
            findings,
        );
    }
    Ok(())
}

fn semantic_model_measure_names(entities: &EntityStore) -> Result<BTreeSet<String>> {
    let mut names = BTreeSet::new();
    for unique_id in entities.ids() {
        let Some(entity) = entities.get_archived(unique_id)? else {
            continue;
        };
        if entity.resource_type_str() != Some("semantic_model") {
            continue;
        }
        let Some(nova) = entity.nova_meta() else {
            continue;
        };
        names.extend(
            nova.measures
                .iter()
                .map(|measure| normalize_value(measure.name.as_str()))
                .filter(|value| !value.is_empty()),
        );
    }
    Ok(names)
}

fn semantic_metric_names(entities: &EntityStore) -> Result<BTreeSet<String>> {
    let mut names = BTreeSet::new();
    for unique_id in entities.ids() {
        let Some(entity) = entities.get_archived(unique_id)? else {
            continue;
        };
        if entity.resource_type_str() != Some("metric") {
            continue;
        }
        if let Some(name) = entity.name_str().map(normalize_value)
            && !name.is_empty()
        {
            names.insert(name);
        }
        if let Some(nova) = entity.nova_meta() {
            if let Some(metric) = nova.metric.as_ref() {
                let name = normalize_value(metric.name.as_str());
                if !name.is_empty() {
                    names.insert(name);
                }
            }
            names.extend(
                nova.metrics
                    .iter()
                    .map(|metric| normalize_value(metric.name.as_str()))
                    .filter(|value| !value.is_empty()),
            );
        }
    }
    Ok(names)
}

fn collect_semantic_label_collision_findings(
    context: &AgentModellingContext<'_>,
    profiles: &[EntityOverlapProfile],
    findings: &mut Vec<AgentModellingFinding>,
) -> Result<()> {
    let mut by_label = BTreeMap::<String, BTreeMap<String, SemanticLabelRef>>::new();
    for profile in profiles {
        let Some(entity) = context.search.entities.get_archived(&profile.unique_id)? else {
            continue;
        };
        let Some(nova) = entity.nova_meta() else {
            continue;
        };
        index_entity_indicator_labels(&mut by_label, &profile.unique_id, entity, nova);
    }

    for (label, refs_by_key) in by_label {
        let refs = refs_by_key.into_values().collect::<Vec<_>>();
        if refs.len() <= 1 {
            continue;
        }
        let canonical_count = refs.iter().filter(|entry| entry.canonical).count();
        let severity = if canonical_count > 1 {
            AgentModellingSeverity::High
        } else {
            AgentModellingSeverity::Medium
        };
        findings.push(AgentModellingFinding {
            code: "semantic_label_collision",
            severity,
            category: "indicator_resolution",
            message: format!("Semantic label `{label}` maps to multiple indicators."),
            entities: semantic_label_entities(&refs),
            indicators: semantic_label_indicators(&refs),
            evidence: json!({
                "label": label,
                "refs": refs.iter().map(|entry| entry.ref_key.as_str()).collect::<Vec<_>>(),
                "canonical_count": canonical_count
            }),
            recommendation: "Use domain-scoped names/synonyms such as gross_revenue, net_revenue, web_revenue, or finance_revenue.".to_string(),
            drill_down_hints: semantic_label_drill_down_hints(&label),
        });
    }
    Ok(())
}

fn collect_column_semantic_ambiguity_findings(
    context: &AgentModellingContext<'_>,
    profiles: &[EntityOverlapProfile],
    findings: &mut Vec<AgentModellingFinding>,
) -> Result<()> {
    let mut by_semantic_type = BTreeMap::<String, Vec<ColumnSemanticRef>>::new();
    let mut by_column_name = BTreeMap::<String, Vec<ColumnSemanticRef>>::new();
    for profile in profiles {
        let Some(entity) = context.search.entities.get_archived(&profile.unique_id)? else {
            continue;
        };
        let analyst_facing = entity_is_analyst_facing(entity);
        for column in entity.column_meta() {
            let Some(semantic_type) = column
                .semantic_type
                .as_ref()
                .map(|value| normalize_value(value.as_str()))
                .filter(|value| !value.is_empty())
            else {
                continue;
            };
            let column_ref = ColumnSemanticRef {
                entity: modeling_entity_ref(&profile.unique_id, entity),
                column_name: column.name.as_str().to_string(),
                role: column.role.as_ref().map(|role| role.as_str().to_string()),
                semantic_type: semantic_type.clone(),
                analyst_facing,
            };
            by_semantic_type
                .entry(semantic_type)
                .or_default()
                .push(column_ref.clone());
            by_column_name
                .entry(normalize_value(column.name.as_str()))
                .or_default()
                .push(column_ref);
        }
    }

    collect_column_role_conflict_findings(by_semantic_type, findings);
    collect_column_name_drift_findings(by_column_name, findings);
    Ok(())
}

fn collect_indicator_parent_not_queryable_findings(
    unique_id: &str,
    entity: &ArchivedEntity,
    nova: &ArchivedNovaMeta,
    findings: &mut Vec<AgentModellingFinding>,
) {
    let execution = execution_surface(entity);
    if execution != IndicatorExecutionSurface::MetadataOnly {
        return;
    }
    for indicator in indicator_refs_for_entity(unique_id, entity, nova) {
        findings.push(AgentModellingFinding {
            code: "indicator_parent_not_queryable",
            severity: AgentModellingSeverity::Blocker,
            category: "queryability",
            message: format!(
                "Indicator `{}` is attached to metadata-only parent `{unique_id}`.",
                indicator.indicator_name
            ),
            entities: vec![modeling_entity_ref(unique_id, entity)],
            indicators: vec![indicator],
            evidence: json!({
                "execution_surface": execution.as_str(),
                "queryable": execution.queryable(),
                "queryable_via": execution.queryable_via()
            }),
            recommendation: "Move the indicator to a queryable dbt model, expose it through dbt Semantic Layer / MetricFlow, or mark the entity as non-analyst-facing.".to_string(),
            drill_down_hints: entity_drill_down_hints(unique_id),
        });
    }
}

fn collect_metric_surface_findings(
    unique_id: &str,
    entity: &ArchivedEntity,
    nova: &ArchivedNovaMeta,
    findings: &mut Vec<AgentModellingFinding>,
) {
    let resource_type = entity.resource_type_str();
    let semantic_metric_parent = resource_type == Some("metric");
    let relation_backed = entity.relation_name_str().is_some();
    let column_names = normalized_entity_column_names(entity);
    let context = MetricSurfaceContext {
        unique_id,
        entity,
        nova,
        semantic_metric_parent,
        relation_backed,
        column_names: &column_names,
    };

    if let Some(metric) = nova.metric.as_ref() {
        collect_single_metric_surface_findings(&context, metric, findings);
    }
    for metric in nova.metrics.iter() {
        collect_single_metric_surface_findings(&context, metric, findings);
    }
}

fn collect_single_metric_surface_findings(
    context: &MetricSurfaceContext<'_>,
    metric: &ArchivedNovaMetric,
    findings: &mut Vec<AgentModellingFinding>,
) {
    let effective_grain = metric.grain.as_ref().or(context.nova.grain.as_ref());
    if !context.semantic_metric_parent && effective_grain.and_then(grain_time_field).is_none() {
        findings.push(metric_missing_time_field_finding(
            context.unique_id,
            context.entity,
            metric,
        ));
    }

    if !context.relation_backed {
        return;
    }

    let metric_name = metric.name.as_str();
    if !metric.template && !context.column_names.contains(&normalize_value(metric_name)) {
        findings.push(AgentModellingFinding {
            code: "metric_output_column_missing",
            severity: AgentModellingSeverity::Medium,
            category: "queryability",
            message: format!(
                "Relation-backed metric `{metric_name}` is not exposed as an output column."
            ),
            entities: vec![modeling_entity_ref(context.unique_id, context.entity)],
            indicators: vec![modeling_metric_ref(
                context.unique_id,
                context.entity,
                metric_name,
            )],
            evidence: json!({
                "metric_name": metric_name,
                "relation_name": context.entity.relation_name_str(),
                "template": metric.template,
                "column_present": false
            }),
            recommendation: "Expose the metric value as a column named after the metric, or mark the metric as a template if it is not a direct output.".to_string(),
            drill_down_hints: entity_columns_drill_down_hints(context.unique_id),
        });
    }

    if let Some(grain) = effective_grain {
        let missing_fields = grain_field_names(grain)
            .into_iter()
            .filter(|field| !context.column_names.contains(&normalize_value(field)))
            .collect::<Vec<_>>();
        if !missing_fields.is_empty() {
            findings.push(AgentModellingFinding {
                code: "metric_grain_field_not_in_output",
                severity: AgentModellingSeverity::High,
                category: "grain_safety",
                message: format!(
                    "Relation-backed metric `{metric_name}` declares grain fields missing from output columns."
                ),
                entities: vec![modeling_entity_ref(context.unique_id, context.entity)],
                indicators: vec![modeling_metric_ref(
                    context.unique_id,
                    context.entity,
                    metric_name,
                )],
                evidence: json!({
                    "metric_name": metric_name,
                    "missing_fields": missing_fields,
                    "relation_name": context.entity.relation_name_str()
                }),
                recommendation: "Ensure every declared grain field is present on the relation-backed metric model.".to_string(),
                drill_down_hints: entity_columns_drill_down_hints(context.unique_id),
            });
        }
    }
}

fn collect_semantic_model_grain_findings(
    unique_id: &str,
    entity: &ArchivedEntity,
    nova: &ArchivedNovaMeta,
    findings: &mut Vec<AgentModellingFinding>,
) {
    if entity.resource_type_str() != Some("semantic_model") || nova.measures.is_empty() {
        return;
    }
    let primary_key = nova
        .grain
        .as_ref()
        .is_some_and(|grain| !grain.primary_key.is_empty());
    let time_field = nova.grain.as_ref().and_then(grain_time_field).is_some();
    if !primary_key {
        findings.push(AgentModellingFinding {
            code: "semantic_model_missing_primary_entity",
            severity: AgentModellingSeverity::Medium,
            category: "grain_safety",
            message: format!("Semantic model `{unique_id}` has measures but no primary entity."),
            entities: vec![modeling_entity_ref(unique_id, entity)],
            indicators: semantic_model_measure_refs(unique_id, entity, nova),
            evidence: json!({
                "measure_count": nova.measures.len(),
                "primary_entity_present": false
            }),
            recommendation: "Add a primary entity to the semantic model so agents can reason about row identity and joinability.".to_string(),
            drill_down_hints: entity_drill_down_hints(unique_id),
        });
    }
    if !time_field {
        findings.push(AgentModellingFinding {
            code: "semantic_model_missing_time_dimension",
            severity: AgentModellingSeverity::High,
            category: "grain_safety",
            message: format!("Semantic model `{unique_id}` has measures but no time dimension."),
            entities: vec![modeling_entity_ref(unique_id, entity)],
            indicators: semantic_model_measure_refs(unique_id, entity, nova),
            evidence: json!({
                "measure_count": nova.measures.len(),
                "time_dimension_present": false
            }),
            recommendation: "Add a time dimension to the semantic model, or mark the measures as non-temporal if they cannot support period analysis.".to_string(),
            drill_down_hints: entity_drill_down_hints(unique_id),
        });
    }
}

fn collect_canonical_primary_key_finding(
    unique_id: &str,
    entity: &ArchivedEntity,
    nova: &ArchivedNovaMeta,
    findings: &mut Vec<AgentModellingFinding>,
) {
    if !nova.canonical || entity.resource_type_str() != Some("model") {
        return;
    }
    let column_primary_keys = column_primary_key_names(entity);
    let grain_primary_keys = nova_grain_primary_key_names(nova);
    if !column_primary_keys.is_empty() || !grain_primary_keys.is_empty() {
        return;
    }
    findings.push(AgentModellingFinding {
        code: "canonical_entity_missing_primary_key",
        severity: AgentModellingSeverity::High,
        category: "grain_safety",
        message: format!("Canonical model `{unique_id}` has no declared primary key."),
        entities: vec![modeling_entity_ref(unique_id, entity)],
        indicators: indicator_refs_for_entity(unique_id, entity, nova),
        evidence: json!({
            "canonical": true,
            "column_primary_keys": column_primary_keys,
            "grain_primary_key": grain_primary_keys,
            "primary_key_present": false
        }),
        recommendation: "Declare primary key columns via `meta.nova.grain.primary_key` or column-level `meta.primary_key`, then add `unique` and `not_null` tests.".to_string(),
        drill_down_hints: entity_columns_drill_down_hints(unique_id),
    });
}

fn collect_cross_grain_and_multi_fact_findings(
    context: &AgentModellingContext<'_>,
    unique_id: &str,
    entity: &ArchivedEntity,
    nova: &ArchivedNovaMeta,
    findings: &mut Vec<AgentModellingFinding>,
) -> Result<()> {
    let fact_parents = fact_like_direct_parents(context, unique_id)?;
    if entity_exposes_metric_or_measure(nova) && fact_parents.len() >= 2 {
        let grain_signatures = fact_parent_grain_signatures(&fact_parents);
        let severity = if grain_signatures.len() > 1 {
            AgentModellingSeverity::High
        } else {
            AgentModellingSeverity::Medium
        };
        findings.push(AgentModellingFinding {
            code: "multi_fact_metric_model",
            severity,
            category: "cross_grain_risk",
            message: format!(
                "Entity `{unique_id}` exposes indicators while joining multiple fact-like parents."
            ),
            entities: vec![modeling_entity_ref(unique_id, entity)],
            indicators: indicator_refs_for_entity(unique_id, entity, nova),
            evidence: json!({
                "fact_like_parent_count": fact_parents.len(),
                "fact_like_parents": fact_parent_entity_refs(&fact_parents),
                "grain_signatures": grain_signatures
            }),
            recommendation: "Verify this model aggregates each fact input to the output grain before joining. If this is a canonical KPI, document the output grain and tests or expose it through dbt Semantic Layer / MetricFlow.".to_string(),
            drill_down_hints: entity_drill_down_hints(unique_id),
        });
    }

    if let Some(metric) = nova.metric.as_ref() {
        collect_single_metric_cross_grain_findings(
            context,
            unique_id,
            entity,
            metric,
            &fact_parents,
            findings,
        );
    }
    for metric in nova.metrics.iter() {
        collect_single_metric_cross_grain_findings(
            context,
            unique_id,
            entity,
            metric,
            &fact_parents,
            findings,
        );
    }
    Ok(())
}

fn collect_single_metric_cross_grain_findings(
    context: &AgentModellingContext<'_>,
    unique_id: &str,
    entity: &ArchivedEntity,
    metric: &ArchivedNovaMetric,
    fact_parents: &[FactLikeParent],
    findings: &mut Vec<AgentModellingFinding>,
) {
    if !metric_looks_ratio(metric) {
        return;
    }
    let metric_name = metric.name.as_str();
    let execution = execution_surface(entity);
    let ratio_signals = metric_ratio_signals(metric);
    if execution == IndicatorExecutionSurface::MetadataOnly {
        findings.push(AgentModellingFinding {
            code: "ratio_like_metric_without_deterministic_surface",
            severity: AgentModellingSeverity::Blocker,
            category: "cross_grain_risk",
            message: format!(
                "Ratio-like metric `{metric_name}` has no deterministic execution surface."
            ),
            entities: vec![modeling_entity_ref(unique_id, entity)],
            indicators: vec![modeling_metric_ref(unique_id, entity, metric_name)],
            evidence: json!({
                "metric_name": metric_name,
                "ratio_signals": ratio_signals,
                "execution_surface": execution.as_str(),
                "queryable": execution.queryable(),
                "queryable_via": execution.queryable_via()
            }),
            recommendation: "Do not leave a ratio/cross-grain KPI as metadata-only. Expose a dbt model, MetricFlow metric, OSI-derived semantic artifact, recipe, or saved query.".to_string(),
            drill_down_hints: entity_drill_down_hints(unique_id),
        });
    }

    let metric_label = normalize_value(metric_name);
    if fact_parents.len() >= 2 && !context.semantic_metric_names.contains(&metric_label) {
        findings.push(AgentModellingFinding {
            code: "cross_grain_kpi_without_semantic_artifact",
            severity: AgentModellingSeverity::High,
            category: "cross_grain_risk",
            message: format!(
                "Ratio-like KPI `{metric_name}` combines fact-like parents without a matching semantic metric."
            ),
            entities: vec![modeling_entity_ref(unique_id, entity)],
            indicators: vec![modeling_metric_ref(unique_id, entity, metric_name)],
            evidence: json!({
                "metric_name": metric_name,
                "fact_like_parent_count": fact_parents.len(),
                "fact_like_parents": fact_parent_entity_refs(fact_parents),
                "semantic_metric_with_same_label": false
            }),
            recommendation: "Model this KPI as a deterministic dbt model or dbt Semantic Layer metric; do not require agents to infer cross-fact joins.".to_string(),
            drill_down_hints: entity_drill_down_hints(unique_id),
        });
    }
}

fn collect_parent_lineage_findings(
    context: &AgentModellingContext<'_>,
    unique_id: &str,
    entity: &ArchivedEntity,
    findings: &mut Vec<AgentModellingFinding>,
) -> Result<()> {
    if !entity_is_analyst_facing(entity) {
        return Ok(());
    }
    let source_parents = source_direct_parent_refs(context, unique_id)?;
    if !source_parents.is_empty() {
        findings.push(AgentModellingFinding {
            code: "analyst_facing_model_depends_on_source",
            severity: AgentModellingSeverity::High,
            category: "layering",
            message: format!("Analyst-facing entity `{unique_id}` depends directly on a source."),
            entities: vec![modeling_entity_ref(unique_id, entity)],
            indicators: entity
                .nova_meta()
                .map(|nova| indicator_refs_for_entity(unique_id, entity, nova))
                .unwrap_or_default(),
            evidence: json!({
                "source_parent_count": source_parents.len(),
                "source_parents": source_parents
            }),
            recommendation: "Route raw source access through staging/base models before exposing analyst-facing metrics or marts.".to_string(),
            drill_down_hints: entity_drill_down_hints(unique_id),
        });
    }

    let parent_count = direct_parent_ids(context.search, unique_id).len();
    if parent_count >= AGENT_SURFACE_TOO_MANY_PARENTS_THRESHOLD {
        findings.push(AgentModellingFinding {
            code: "agent_surface_too_many_parents",
            severity: AgentModellingSeverity::Medium,
            category: "layering",
            message: format!("Analyst-facing entity `{unique_id}` has {parent_count} direct parents."),
            entities: vec![modeling_entity_ref(unique_id, entity)],
            indicators: entity
                .nova_meta()
                .map(|nova| indicator_refs_for_entity(unique_id, entity, nova))
                .unwrap_or_default(),
            evidence: json!({
                "direct_parent_count": parent_count,
                "threshold": AGENT_SURFACE_TOO_MANY_PARENTS_THRESHOLD,
                "direct_parents": direct_parent_refs(context, unique_id)?
            }),
            recommendation: "Split the model into clearer intermediate concepts, or document why this wide analyst surface is intentionally curated.".to_string(),
            drill_down_hints: entity_drill_down_hints(unique_id),
        });
    }
    Ok(())
}

fn collect_helper_layer_findings(
    context: &AgentModellingContext<'_>,
    unique_id: &str,
    entity: &ArchivedEntity,
    nova: &ArchivedNovaMeta,
    findings: &mut Vec<AgentModellingFinding>,
) {
    let Some(layer) = entity_layer(context.search, entity) else {
        return;
    };
    if !is_helper_layer(&layer) {
        return;
    }
    if has_canonical_metric_or_measure(nova) {
        findings.push(AgentModellingFinding {
            code: "non_mart_model_exposes_canonical_indicator",
            severity: AgentModellingSeverity::Medium,
            category: "layering",
            message: format!(
                "Helper-layer entity `{unique_id}` exposes a canonical indicator."
            ),
            entities: vec![modeling_entity_ref(unique_id, entity)],
            indicators: canonical_indicator_refs_for_entity(unique_id, entity, nova),
            evidence: json!({
                "layer": layer,
                "canonical_indicator_present": true
            }),
            recommendation: "Move canonical indicators to the analyst-facing mart, or de-rank the helper with `search.candidates.analyst: false`.".to_string(),
            drill_down_hints: entity_drill_down_hints(unique_id),
        });
    }
    if !analyst_candidate_disabled(nova) {
        findings.push(AgentModellingFinding {
            code: "helper_ranked_as_analyst_candidate",
            severity: AgentModellingSeverity::Low,
            category: "layering",
            message: format!("Helper-layer entity `{unique_id}` is still an analyst candidate."),
            entities: vec![modeling_entity_ref(unique_id, entity)],
            indicators: indicator_refs_for_entity(unique_id, entity, nova),
            evidence: json!({
                "layer": layer,
                "analyst_candidate": true
            }),
            recommendation: "Set `meta.nova.search.candidates.analyst: false` for helper models that should remain searchable but not rank first.".to_string(),
            drill_down_hints: entity_drill_down_hints(unique_id),
        });
    }
}

fn collect_governance_findings(
    unique_id: &str,
    entity: &ArchivedEntity,
    nova: Option<&ArchivedNovaMeta>,
    findings: &mut Vec<AgentModellingFinding>,
) {
    if !entity_is_analyst_facing(entity) {
        return;
    }
    let entity_governance_present = nova.is_some_and(|nova| nova.governance.is_some());
    if !entity_governance_present {
        findings.push(AgentModellingFinding {
            code: "analyst_surface_missing_governance",
            severity: AgentModellingSeverity::Medium,
            category: "governance",
            message: format!("Analyst-facing entity `{unique_id}` has no Nova governance block."),
            entities: vec![modeling_entity_ref(unique_id, entity)],
            indicators: nova
                .map(|nova| indicator_refs_for_entity(unique_id, entity, nova))
                .unwrap_or_default(),
            evidence: json!({
                "governance_present": false,
                "analyst_facing": true
            }),
            recommendation: "Add `meta.nova.governance.sensitivity`, `pii`, and compliance fields for analyst-facing surfaces.".to_string(),
            drill_down_hints: entity_drill_down_hints(unique_id),
        });
    }

    let entity_json = entity.to_json_value();
    let Some(columns) = entity_json.get("columns").and_then(JsonValue::as_object) else {
        return;
    };
    for (column_name, column) in columns {
        let Some(pii_signal) = pii_like_column_signal(column_name) else {
            continue;
        };
        let column_governance_present = column_governance_present(column);
        if entity_governance_present || column_governance_present {
            continue;
        }
        findings.push(AgentModellingFinding {
            code: "pii_like_column_without_governance",
            severity: AgentModellingSeverity::Medium,
            category: "governance",
            message: format!(
                "PII-like column `{column_name}` appears on analyst-facing entity `{unique_id}` without governance classification."
            ),
            entities: vec![modeling_entity_ref(unique_id, entity)],
            indicators: nova
                .map(|nova| indicator_refs_for_entity(unique_id, entity, nova))
                .unwrap_or_default(),
            evidence: json!({
                "column_name": column_name,
                "pii_signal": pii_signal,
                "entity_governance_present": entity_governance_present,
                "column_governance_present": column_governance_present
            }),
            recommendation: "Classify PII at entity or column level so agents can apply governance caveats.".to_string(),
            drill_down_hints: entity_columns_drill_down_hints(unique_id),
        });
    }
}

fn collect_catalog_integrity_findings(
    unique_id: &str,
    entity: &ArchivedEntity,
    nova: Option<&ArchivedNovaMeta>,
    findings: &mut Vec<AgentModellingFinding>,
) {
    let entity_json = entity.to_json_value();
    let Some(columns) = entity_json.get("columns").and_then(JsonValue::as_object) else {
        return;
    };
    if let Some(nova) = nova {
        collect_catalog_indicator_field_findings(unique_id, entity, nova, columns, findings);
    }
    collect_catalog_only_candidate_measure_columns(unique_id, entity, columns, findings);
}

fn collect_catalog_indicator_field_findings(
    unique_id: &str,
    entity: &ArchivedEntity,
    nova: &ArchivedNovaMeta,
    columns: &serde_json::Map<String, JsonValue>,
    findings: &mut Vec<AgentModellingFinding>,
) {
    for measure in nova.measures.iter() {
        let Some(field) = measure.field.as_ref().map(ArchivedString::as_str) else {
            continue;
        };
        let Some(column) = columns.get(field) else {
            continue;
        };
        let Some(drift) = column.get("catalog_drift").and_then(JsonValue::as_object) else {
            continue;
        };
        if drift
            .get("type_mismatch")
            .and_then(JsonValue::as_bool)
            .unwrap_or(false)
        {
            findings.push(AgentModellingFinding {
                code: "catalog_type_drift_on_indicator_field",
                severity: AgentModellingSeverity::Medium,
                category: "catalog_reality",
                message: format!(
                    "Measure `{}` uses field `{field}` with manifest/catalog type drift.",
                    measure.name.as_str()
                ),
                entities: vec![modeling_entity_ref(unique_id, entity)],
                indicators: vec![ModelingIndicatorRef {
                    indicator_name: measure.name.as_str().to_string(),
                    indicator_type: "measure".to_string(),
                    parent_unique_id: unique_id.to_string(),
                    source: Some(indicator_source_for_entity(entity).to_string()),
                }],
                evidence: json!({
                    "field": field,
                    "manifest_data_type": drift.get("manifest_data_type"),
                    "catalog_data_type": drift.get("catalog_data_type")
                }),
                recommendation: "Update dbt column metadata or investigate warehouse schema drift before relying on this measure.".to_string(),
                drill_down_hints: entity_columns_drill_down_hints(unique_id),
            });
        }
        if drift
            .get("missing_in_catalog")
            .and_then(JsonValue::as_bool)
            .unwrap_or(false)
        {
            findings.push(AgentModellingFinding {
                code: "catalog_missing_indicator_field",
                severity: AgentModellingSeverity::High,
                category: "catalog_reality",
                message: format!(
                    "Measure `{}` uses field `{field}` that is missing from catalog reality.",
                    measure.name.as_str()
                ),
                entities: vec![modeling_entity_ref(unique_id, entity)],
                indicators: vec![ModelingIndicatorRef {
                    indicator_name: measure.name.as_str().to_string(),
                    indicator_type: "measure".to_string(),
                    parent_unique_id: unique_id.to_string(),
                    source: Some(indicator_source_for_entity(entity).to_string()),
                }],
                evidence: json!({
                    "field": field,
                    "manifest_data_type": drift.get("manifest_data_type"),
                    "missing_in_catalog": true
                }),
                recommendation: "The dbt manifest declares an indicator field that is absent from catalog reality. Refresh or repair dbt docs or the warehouse schema.".to_string(),
                drill_down_hints: entity_columns_drill_down_hints(unique_id),
            });
        }
    }
}

fn collect_catalog_only_candidate_measure_columns(
    unique_id: &str,
    entity: &ArchivedEntity,
    columns: &serde_json::Map<String, JsonValue>,
    findings: &mut Vec<AgentModellingFinding>,
) {
    if !entity_is_analyst_facing(entity) {
        return;
    }
    for (column_name, column) in columns {
        let catalog_only = column
            .get("catalog_drift")
            .and_then(|drift| drift.get("catalog_only"))
            .and_then(JsonValue::as_bool)
            .unwrap_or(false);
        if !catalog_only {
            continue;
        }
        let data_type = column
            .get("data_type")
            .and_then(JsonValue::as_str)
            .or_else(|| column.get("catalog_data_type").and_then(JsonValue::as_str));
        if !data_type.is_some_and(is_measure_like_data_type) {
            continue;
        }
        findings.push(AgentModellingFinding {
            code: "catalog_only_candidate_measure_column",
            severity: AgentModellingSeverity::Low,
            category: "catalog_reality",
            message: format!(
                "Catalog-only column `{column_name}` looks measure-like on an analyst-facing entity."
            ),
            entities: vec![modeling_entity_ref(unique_id, entity)],
            indicators: Vec::new(),
            evidence: json!({
                "column_name": column_name,
                "catalog_data_type": data_type,
                "catalog_only": true
            }),
            recommendation: "Consider documenting this warehouse-only measure-like column in dbt if analysts should use it.".to_string(),
            drill_down_hints: entity_columns_drill_down_hints(unique_id),
        });
    }
}

fn collect_semantic_metric_reference_findings(
    unique_id: &str,
    entity: &ArchivedEntity,
    semantic_model_measure_names: &BTreeSet<String>,
    findings: &mut Vec<AgentModellingFinding>,
) {
    if entity.resource_type_str() != Some("metric") {
        return;
    }
    let entity_json = entity.to_json_value();
    let measure_refs = metricflow_measure_references(&entity_json);
    let missing = measure_refs
        .into_iter()
        .filter(|name| !semantic_model_measure_names.contains(&normalize_value(name)))
        .collect::<Vec<_>>();
    if missing.is_empty() {
        return;
    }
    let metric_name = entity.name_str().unwrap_or(unique_id);
    findings.push(AgentModellingFinding {
        code: "semantic_metric_unresolved_measure_ref",
        severity: AgentModellingSeverity::High,
        category: "semantic_artifact_integrity",
        message: format!(
            "dbt metric `{metric_name}` references measure(s) that are absent from semantic models."
        ),
        entities: vec![modeling_entity_ref(unique_id, entity)],
        indicators: entity
            .nova_meta()
            .and_then(|nova| nova.metric.as_ref())
            .map(|metric| vec![modeling_metric_ref(unique_id, entity, metric.name.as_str())])
            .unwrap_or_default(),
        evidence: json!({
            "missing_measure_refs": missing,
            "semantic_model_measure_count": semantic_model_measure_names.len()
        }),
        recommendation: "Fix the dbt semantic metric reference or ensure the referenced semantic model measure is present in the manifest.".to_string(),
        drill_down_hints: entity_drill_down_hints(unique_id),
    });
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IndicatorExecutionSurface {
    Relation,
    SemanticLayer,
    MetadataOnly,
}

impl IndicatorExecutionSurface {
    fn as_str(self) -> &'static str {
        match self {
            Self::Relation => "relation",
            Self::SemanticLayer => "semantic_layer",
            Self::MetadataOnly => "metadata_only",
        }
    }

    fn queryable(self) -> bool {
        !matches!(self, Self::MetadataOnly)
    }

    fn queryable_via(self) -> &'static str {
        match self {
            Self::Relation => "relation_name",
            Self::SemanticLayer => "metricflow",
            Self::MetadataOnly => "none",
        }
    }
}

fn execution_surface(entity: &ArchivedEntity) -> IndicatorExecutionSurface {
    match entity.resource_type_str() {
        Some("metric" | "semantic_model") => IndicatorExecutionSurface::SemanticLayer,
        _ if entity.relation_name_str().is_some() => IndicatorExecutionSurface::Relation,
        _ => IndicatorExecutionSurface::MetadataOnly,
    }
}

fn entity_is_analyst_facing(entity: &ArchivedEntity) -> bool {
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

fn normalized_entity_column_names(entity: &ArchivedEntity) -> BTreeSet<String> {
    entity
        .column_names_iter()
        .map(normalize_value)
        .filter(|value| !value.is_empty())
        .collect()
}

fn grain_time_field(grain: &ArchivedNovaGrain) -> Option<&str> {
    grain.time_field.as_ref().map(ArchivedString::as_str)
}

fn grain_field_names(grain: &ArchivedNovaGrain) -> Vec<String> {
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
struct FactLikeParent {
    entity: ModelingEntityRef,
    grain_signatures: Vec<String>,
}

#[derive(Clone)]
struct SemanticLabelRef {
    entity: ModelingEntityRef,
    indicator: ModelingIndicatorRef,
    canonical: bool,
    ref_key: String,
}

#[derive(Clone)]
struct ColumnSemanticRef {
    entity: ModelingEntityRef,
    column_name: String,
    role: Option<String>,
    semantic_type: String,
    analyst_facing: bool,
}

fn index_entity_indicator_labels(
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

fn index_metric_labels(
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

fn insert_semantic_label_ref(
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

fn semantic_label_entities(refs: &[SemanticLabelRef]) -> Vec<ModelingEntityRef> {
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

fn semantic_label_indicators(refs: &[SemanticLabelRef]) -> Vec<ModelingIndicatorRef> {
    refs.iter()
        .take(8)
        .map(|entry| entry.indicator.clone())
        .collect()
}

fn semantic_label_drill_down_hints(label: &str) -> Vec<JsonValue> {
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

fn collect_column_role_conflict_findings(
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

fn collect_column_name_drift_findings(
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

fn column_semantic_entities(refs: &[ColumnSemanticRef]) -> Vec<ModelingEntityRef> {
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

fn column_semantic_refs_json(refs: &[ColumnSemanticRef]) -> Vec<JsonValue> {
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

fn column_semantic_drill_down_hints(refs: &[ColumnSemanticRef]) -> Vec<JsonValue> {
    refs.first()
        .map(|entry| entity_columns_drill_down_hints(&entry.entity.unique_id))
        .unwrap_or_default()
}

fn direct_parent_ids<'a>(search: &'a ManifestSearch, unique_id: &str) -> &'a [String] {
    search.parent_map.get(unique_id).map_or(&[], Vec::as_slice)
}

fn direct_parent_refs(
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

fn source_direct_parent_refs(
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

fn fact_like_direct_parents(
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

fn fact_parent_entity_refs(parents: &[FactLikeParent]) -> Vec<ModelingEntityRef> {
    parents
        .iter()
        .take(8)
        .map(|parent| parent.entity.clone())
        .collect()
}

fn fact_parent_grain_signatures(parents: &[FactLikeParent]) -> Vec<String> {
    let mut signatures = BTreeSet::new();
    for parent in parents {
        signatures.extend(parent.grain_signatures.iter().cloned());
    }
    signatures.into_iter().collect()
}

fn is_fact_like_entity(entity: &ArchivedEntity) -> bool {
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

fn entity_has_measure_role_columns(entity: &ArchivedEntity) -> bool {
    entity.column_meta().iter().any(|column| {
        column.role.as_ref().is_some_and(|role| {
            matches!(
                normalize_value(role.as_str()).as_str(),
                "fact" | "measure" | "metric"
            )
        })
    })
}

fn entity_grain_signatures(entity: &ArchivedEntity) -> Vec<String> {
    let mut signatures = BTreeSet::new();
    for variant in build_entity_grain_variants(entity.nova_meta()) {
        if let Some(signature) = grain_variant_signature(&variant) {
            signatures.insert(signature);
        }
    }
    signatures.into_iter().collect()
}

fn grain_variant_signature(variant: &GrainVariant) -> Option<String> {
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

fn column_primary_key_names(entity: &ArchivedEntity) -> Vec<String> {
    entity
        .column_meta()
        .iter()
        .filter(|column| column.primary_key)
        .map(|column| column.name.as_str().to_string())
        .collect()
}

fn nova_grain_primary_key_names(nova: &ArchivedNovaMeta) -> Vec<String> {
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

fn entity_exposes_indicator(nova: &ArchivedNovaMeta) -> bool {
    nova.metric.is_some() || !nova.metrics.is_empty() || !nova.measures.is_empty()
}

fn entity_exposes_metric_or_measure(nova: &ArchivedNovaMeta) -> bool {
    entity_exposes_indicator(nova)
}

fn has_canonical_metric_or_measure(nova: &ArchivedNovaMeta) -> bool {
    (nova.canonical && entity_exposes_indicator(nova))
        || nova.measures.iter().any(|measure| measure.canonical)
        || nova.metric.as_ref().is_some_and(|metric| metric.canonical)
        || nova.metrics.iter().any(|metric| metric.canonical)
}

fn canonical_indicator_refs_for_entity(
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

fn metric_looks_ratio(metric: &ArchivedNovaMetric) -> bool {
    !metric_ratio_signals(metric).is_empty()
}

fn metric_ratio_signals(metric: &ArchivedNovaMetric) -> Vec<&'static str> {
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

fn entity_layer(search: &ManifestSearch, entity: &ArchivedEntity) -> Option<String> {
    search
        .layer_for(entity)
        .map(|layer| layer.trim().to_ascii_lowercase())
        .filter(|layer| !layer.is_empty())
        .or_else(|| inferred_entity_layer(entity))
}

fn inferred_entity_layer(entity: &ArchivedEntity) -> Option<String> {
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

fn is_helper_layer(layer: &str) -> bool {
    matches!(
        layer.trim().to_ascii_lowercase().as_str(),
        "staging" | "stage" | "stg" | "intermediate" | "int"
    )
}

fn analyst_candidate_disabled(nova: &ArchivedNovaMeta) -> bool {
    nova.search
        .as_ref()
        .and_then(|search| search.candidates.as_ref())
        .is_some_and(|candidates| !candidates.analyst)
}

fn pii_like_column_signal(column_name: &str) -> Option<&'static str> {
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

fn column_governance_present(column: &JsonValue) -> bool {
    column_nova_meta_json(column).is_some_and(|nova| {
        nova.get("governance")
            .is_some_and(|governance| !governance.is_null())
    })
}

fn is_measure_like_data_type(data_type: &str) -> bool {
    let normalized = data_type.trim().to_lowercase();
    [
        "int", "integer", "bigint", "smallint", "numeric", "decimal", "number", "float", "double",
        "real",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
}

fn metricflow_measure_references(entity_json: &JsonValue) -> Vec<String> {
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

fn collect_metricflow_named_reference(value: Option<&JsonValue>, refs: &mut BTreeSet<String>) {
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

fn indicator_source_for_entity(entity: &ArchivedEntity) -> &'static str {
    match entity.resource_type_str() {
        Some("metric") => "dbt_metric",
        Some("semantic_model") => "dbt_semantic_model",
        _ => "nova_meta",
    }
}

fn modeling_entity_ref(unique_id: &str, entity: &ArchivedEntity) -> ModelingEntityRef {
    ModelingEntityRef {
        unique_id: unique_id.to_string(),
        name: entity.name_str().unwrap_or(unique_id).to_string(),
        resource_type: entity.resource_type_str().unwrap_or("unknown").to_string(),
        relation_name: entity.relation_name_str().map(str::to_string),
    }
}

fn modeling_entity_ref_from_entity_ref(entity: &EntityRef) -> ModelingEntityRef {
    ModelingEntityRef {
        unique_id: entity.unique_id.clone(),
        name: entity.name.clone(),
        resource_type: entity.resource_type.clone(),
        relation_name: entity.relation_name.clone(),
    }
}

fn duplicate_parent_entity_refs(parents: &[DuplicateIndicatorParent]) -> Vec<ModelingEntityRef> {
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

fn duplicate_indicator_refs(row: &DuplicateIndicatorRow) -> Vec<ModelingIndicatorRef> {
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

fn indicator_source_for_resource_type(resource_type: &str) -> &'static str {
    match resource_type {
        "metric" => "dbt_metric",
        "semantic_model" => "dbt_semantic_model",
        _ => "nova_meta",
    }
}

fn modeling_metric_ref(
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

fn indicator_refs_for_entity(
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

fn semantic_model_measure_refs(
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

fn metric_missing_time_field_finding(
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

fn entity_drill_down_hints(unique_id: &str) -> Vec<JsonValue> {
    vec![json!({
        "purpose": "inspect_entity",
        "tool": "get_entity",
        "arguments": {
            "id_or_name": unique_id,
            "detail": "standard"
        }
    })]
}

fn entity_columns_drill_down_hints(unique_id: &str) -> Vec<JsonValue> {
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

fn duplicate_indicator_drill_down_hints(row: &DuplicateIndicatorRow) -> Vec<JsonValue> {
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

fn truncate_agent_modelling_findings(
    findings: &[AgentModellingFinding],
) -> Vec<AgentModellingFinding> {
    findings
        .iter()
        .take(AGENT_MODELLING_MAX_FINDINGS)
        .cloned()
        .collect()
}

fn sort_agent_modelling_findings(findings: &mut [AgentModellingFinding]) {
    findings.sort_by(compare_agent_modelling_findings);
}

fn compare_agent_modelling_findings(
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

fn first_finding_entity_id(finding: &AgentModellingFinding) -> &str {
    finding
        .entities
        .first()
        .map_or("", |entity| entity.unique_id.as_str())
}

fn first_finding_indicator_name(finding: &AgentModellingFinding) -> &str {
    finding
        .indicators
        .first()
        .map_or("", |indicator| indicator.indicator_name.as_str())
}

fn agent_modelling_summary(findings: &[AgentModellingFinding], truncated: bool) -> JsonValue {
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

fn top_agent_modelling_buckets(counts: BTreeMap<String, usize>, key: &str) -> Vec<JsonValue> {
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

fn overlap_evidence_category_counts(rows: &[EntityOverlapRow]) -> JsonValue {
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

fn overlap_evidence_categories_for_row(row: &EntityOverlapRow) -> Vec<JsonValue> {
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

fn duplicate_indicator_summary_row(row: &DuplicateIndicatorRow) -> JsonValue {
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

fn modelling_drill_down_hints(
    params: &ModellingConsistencyReportParams,
    section_limit: usize,
    section_offset: usize,
    overlap_rows: &[EntityOverlapRow],
    multi_grain_entity_rows: &[MultiGrainEntityRow],
) -> Vec<JsonValue> {
    let mut hints = Vec::new();
    if modelling_has_next_page(
        section_limit,
        section_offset,
        overlap_rows,
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

fn modelling_has_next_page(
    section_limit: usize,
    section_offset: usize,
    overlap_rows: &[EntityOverlapRow],
    multi_grain_entity_rows: &[MultiGrainEntityRow],
) -> bool {
    [overlap_rows.len(), multi_grain_entity_rows.len()]
        .into_iter()
        .any(|total| section_offset.saturating_add(section_limit) < total)
}

fn grain_signature_key(grain: &GrainVariant) -> String {
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

fn sorted_intersection(values1: &BTreeSet<String>, values2: &BTreeSet<String>) -> Vec<String> {
    values1.intersection(values2).cloned().collect()
}

fn sorted_difference(values1: &BTreeSet<String>, values2: &BTreeSet<String>) -> Vec<String> {
    values1.difference(values2).cloned().collect()
}

fn best_grain_pair<'a>(
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

fn score_from_overlap(surface_overlap_count: usize, shared_value_count: usize) -> f32 {
    let combined = surface_overlap_count.saturating_mul(10) + shared_value_count;
    f32::from(u16::try_from(combined).unwrap_or(u16::MAX))
}

fn normalize_value(value: &str) -> String {
    value.trim().to_lowercase()
}

fn is_distinctive_column_name(value: &str, min_word_len: usize) -> bool {
    let tokens = tokenize_alnum_lowercase(value, min_word_len);
    tokens.len() >= 2
        || (tokens.len() == 1 && tokens[0].chars().count() >= min_word_len.saturating_mul(3))
}

fn normalized_resource_type_filter(resource_types: &[String]) -> Option<HashSet<String>> {
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

fn resource_type_allowed(
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

fn compare_overlap_rows(left: &EntityOverlapRow, right: &EntityOverlapRow) -> Ordering {
    right
        .score
        .partial_cmp(&left.score)
        .unwrap_or(Ordering::Equal)
        .then_with(|| right.surface_overlap_count.cmp(&left.surface_overlap_count))
        .then_with(|| right.shared_value_count.cmp(&left.shared_value_count))
        .then_with(|| left.entity1.unique_id.cmp(&right.entity1.unique_id))
        .then_with(|| left.entity2.unique_id.cmp(&right.entity2.unique_id))
}

fn compare_duplicate_indicator_rows(
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

fn paginate_section<T>(rows: Vec<T>, offset: usize, limit: usize) -> Vec<T> {
    rows.into_iter().skip(offset).take(limit).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grain_variant(
        primary_key: &[&str],
        time_field: Option<&str>,
        dimensions: &[&str],
    ) -> GrainVariant {
        GrainVariant {
            sources: vec!["test".to_string()],
            primary_key: primary_key
                .iter()
                .map(|value| (*value).to_string())
                .collect(),
            time_field: time_field.map(str::to_string),
            dimensions: dimensions
                .iter()
                .map(|value| (*value).to_string())
                .collect(),
        }
    }

    fn profile_with(
        unique_id: &str,
        column_names: &[&str],
        grain_variants: Vec<GrainVariant>,
    ) -> EntityOverlapProfile {
        EntityOverlapProfile {
            unique_id: unique_id.to_string(),
            name: unique_id.to_string(),
            resource_type: "model".to_string(),
            relation_name: None,
            canonical: false,
            name_tokens: BTreeSet::new(),
            column_names: column_names
                .iter()
                .map(|value| normalize_value(value))
                .collect(),
            parent_synonyms: BTreeSet::new(),
            domains: BTreeSet::new(),
            indicator_names: BTreeSet::new(),
            typed_indicators: BTreeSet::new(),
            indicator_profiles: BTreeMap::new(),
            column_semantic_types: BTreeSet::new(),
            grain_variants,
        }
    }

    fn profile_with_indicator(
        unique_id: &str,
        indicator_type: &str,
        indicator_name: &str,
        canonical: bool,
        entity_grain_variants: Vec<GrainVariant>,
        indicator_grain_variants: Vec<GrainVariant>,
    ) -> EntityOverlapProfile {
        let mut profile = profile_with(unique_id, &[], entity_grain_variants);
        let key = (indicator_type.to_string(), normalize_value(indicator_name));
        profile.indicator_names.insert(key.1.clone());
        profile.typed_indicators.insert(key.clone());
        profile.indicator_profiles.insert(
            key,
            IndicatorOverlapIndicatorProfile {
                canonical,
                grain_variants: indicator_grain_variants,
            },
        );
        profile
    }

    fn agent_finding(
        code: &'static str,
        severity: AgentModellingSeverity,
        category: &'static str,
        entity_id: &str,
        indicator_name: &str,
        message: &str,
    ) -> AgentModellingFinding {
        AgentModellingFinding {
            code,
            severity,
            category,
            message: message.to_string(),
            entities: vec![ModelingEntityRef {
                unique_id: entity_id.to_string(),
                name: entity_id.to_string(),
                resource_type: "model".to_string(),
                relation_name: None,
            }],
            indicators: vec![ModelingIndicatorRef {
                indicator_name: indicator_name.to_string(),
                indicator_type: "metric".to_string(),
                parent_unique_id: entity_id.to_string(),
                source: Some("nova_meta".to_string()),
            }],
            evidence: json!({}),
            recommendation: "Fix the deterministic modelling issue.".to_string(),
            drill_down_hints: Vec::new(),
        }
    }

    #[test]
    fn agent_modelling_summary_counts_and_sorts_buckets() {
        let findings = vec![
            agent_finding(
                "beta_code",
                AgentModellingSeverity::High,
                "queryability",
                "model.pkg.b",
                "beta",
                "beta",
            ),
            agent_finding(
                "alpha_code",
                AgentModellingSeverity::Blocker,
                "grain_safety",
                "model.pkg.a",
                "alpha",
                "alpha",
            ),
            agent_finding(
                "alpha_code",
                AgentModellingSeverity::Medium,
                "grain_safety",
                "model.pkg.c",
                "alpha",
                "alpha duplicate",
            ),
        ];

        let summary = agent_modelling_summary(&findings, true);
        assert_eq!(summary["total"].as_u64(), Some(3));
        assert_eq!(summary["blockers"].as_u64(), Some(1));
        assert_eq!(summary["high"].as_u64(), Some(1));
        assert_eq!(summary["medium"].as_u64(), Some(1));
        assert_eq!(summary["low"].as_u64(), Some(0));
        assert_eq!(summary["truncated"].as_bool(), Some(true));
        assert_eq!(summary["top_codes"][0]["code"].as_str(), Some("alpha_code"));
        assert_eq!(summary["top_codes"][0]["count"].as_u64(), Some(2));
        assert_eq!(
            summary["top_categories"][0]["category"].as_str(),
            Some("grain_safety")
        );
        assert_eq!(summary["top_categories"][0]["count"].as_u64(), Some(2));
    }

    #[test]
    fn agent_modelling_findings_sort_by_contract_order() {
        let mut findings = vec![
            agent_finding(
                "later_low",
                AgentModellingSeverity::Low,
                "queryability",
                "model.pkg.low",
                "low",
                "low",
            ),
            agent_finding(
                "second_blocker",
                AgentModellingSeverity::Blocker,
                "queryability",
                "model.pkg.b",
                "b",
                "second",
            ),
            agent_finding(
                "first_blocker",
                AgentModellingSeverity::Blocker,
                "grain_safety",
                "model.pkg.a",
                "a",
                "first",
            ),
            agent_finding(
                "middle_high",
                AgentModellingSeverity::High,
                "grain_safety",
                "model.pkg.high",
                "high",
                "middle",
            ),
        ];

        sort_agent_modelling_findings(&mut findings);

        let codes = findings
            .iter()
            .map(|finding| finding.code)
            .collect::<Vec<_>>();
        assert_eq!(
            codes,
            vec![
                "first_blocker",
                "second_blocker",
                "middle_high",
                "later_low"
            ]
        );
    }

    #[test]
    fn agent_modelling_findings_truncate_after_sorting() {
        let mut findings = (0..AGENT_MODELLING_MAX_FINDINGS)
            .map(|index| {
                agent_finding(
                    "low_code",
                    AgentModellingSeverity::Low,
                    "queryability",
                    &format!("model.pkg.low_{index:03}"),
                    "low",
                    "low",
                )
            })
            .collect::<Vec<_>>();
        findings.push(agent_finding(
            "blocker_code",
            AgentModellingSeverity::Blocker,
            "queryability",
            "model.pkg.blocker",
            "blocker",
            "blocker",
        ));

        sort_agent_modelling_findings(&mut findings);
        let truncated = findings.len() > AGENT_MODELLING_MAX_FINDINGS;
        let bounded = truncate_agent_modelling_findings(&findings);

        assert!(truncated);
        assert_eq!(bounded.len(), AGENT_MODELLING_MAX_FINDINGS);
        assert!(matches!(
            bounded[0].severity,
            AgentModellingSeverity::Blocker
        ));
        assert_eq!(bounded[0].code, "blocker_code");
    }

    #[test]
    fn compare_entity_grains_prefers_best_matching_variant() {
        let left = profile_with(
            "model.pkg.left",
            &[],
            vec![
                grain_variant(&["session_id"], Some("session_date"), &["platform_name"]),
                grain_variant(
                    &["order_id"],
                    Some("order_date"),
                    &["country_code", "sales_channel"],
                ),
            ],
        );
        let right = profile_with(
            "model.pkg.right",
            &[],
            vec![grain_variant(
                &["order_id"],
                Some("order_date"),
                &["country_code", "sales_channel"],
            )],
        );

        let comparison = compare_entity_grains(&left, &right);
        assert!(comparison.exact_match);
        assert!(comparison.same_time_field);
        assert_eq!(comparison.shared_primary_key, vec!["order_id".to_string()]);
        assert_eq!(
            comparison.shared_dimensions,
            vec!["country_code".to_string(), "sales_channel".to_string()]
        );
    }

    #[test]
    fn compare_entity_grains_treats_reordered_fields_as_exact_match() {
        let left = profile_with(
            "model.pkg.left",
            &[],
            vec![grain_variant(
                &["order_id", "line_id"],
                Some("order_date"),
                &["country_code", "sales_channel"],
            )],
        );
        let right = profile_with(
            "model.pkg.right",
            &[],
            vec![grain_variant(
                &["line_id", "order_id"],
                Some("order_date"),
                &["sales_channel", "country_code"],
            )],
        );

        let comparison = compare_entity_grains(&left, &right);
        assert!(comparison.exact_match);
        assert!(comparison.same_time_field);
        assert_eq!(
            comparison.shared_primary_key,
            vec!["line_id".to_string(), "order_id".to_string()]
        );
        assert_eq!(
            comparison.shared_dimensions,
            vec!["country_code".to_string(), "sales_channel".to_string()]
        );
    }

    #[test]
    fn overlap_evidence_includes_shared_column_names() {
        let left = profile_with(
            "model.pkg.left",
            &["order_id", "gmv_amount", "country_code"],
            vec![],
        );
        let right = profile_with(
            "model.pkg.right",
            &["order_id", "gmv_amount", "customer_id"],
            vec![],
        );

        let evidence = overlap_evidence(&left, &right);
        assert_eq!(
            evidence.shared_column_names,
            vec!["gmv_amount".to_string(), "order_id".to_string()]
        );
        assert!(evidence.surface_overlap_count() > 0);
    }

    #[test]
    fn overlap_candidate_pairs_use_shared_column_names() {
        let profiles = vec![
            profile_with("model.pkg.left", &["order_id", "gmv_amount"], vec![]),
            profile_with("model.pkg.right", &["order_id", "gmv_amount"], vec![]),
            profile_with("model.pkg.other", &["promotion_id"], vec![]),
        ];

        let pairs = overlap_candidate_pairs(&profiles);
        assert!(pairs.contains(&(0, 1)));
        assert!(!pairs.contains(&(0, 2)));
    }

    #[test]
    fn duplicate_indicator_rows_use_indicator_specific_grains() {
        let profiles = vec![
            profile_with_indicator(
                "model.pkg.left",
                "metric",
                "conversion_rate",
                false,
                vec![
                    grain_variant(&["session_id"], Some("session_date"), &["platform_name"]),
                    grain_variant(&["order_id"], Some("order_date"), &["country_code"]),
                ],
                vec![grain_variant(
                    &["order_id"],
                    Some("order_date"),
                    &["country_code"],
                )],
            ),
            profile_with_indicator(
                "model.pkg.right",
                "metric",
                "conversion_rate",
                false,
                vec![grain_variant(
                    &["order_id"],
                    Some("order_date"),
                    &["country_code"],
                )],
                vec![grain_variant(
                    &["order_id"],
                    Some("order_date"),
                    &["country_code"],
                )],
            ),
        ];

        let rows = duplicate_indicator_rows(&profiles, 10);
        assert_eq!(rows.len(), 1);
        assert!(!rows[0].inconsistent_grains);
        assert_eq!(rows[0].grain_signatures.len(), 1);
    }

    #[test]
    fn duplicate_indicator_rows_count_indicator_level_canonical_flags() {
        let profiles = vec![
            profile_with_indicator(
                "model.pkg.left",
                "measure",
                "gmv",
                true,
                vec![grain_variant(
                    &["order_id"],
                    Some("order_date"),
                    &["country_code"],
                )],
                vec![grain_variant(
                    &["order_id"],
                    Some("order_date"),
                    &["country_code"],
                )],
            ),
            profile_with_indicator(
                "model.pkg.right",
                "measure",
                "gmv",
                false,
                vec![grain_variant(
                    &["order_id"],
                    Some("order_date"),
                    &["country_code"],
                )],
                vec![grain_variant(
                    &["order_id"],
                    Some("order_date"),
                    &["country_code"],
                )],
            ),
        ];

        let rows = duplicate_indicator_rows(&profiles, 10);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].canonical_parent_count, 1);
        assert_eq!(rows[0].parents.len(), 2);
        assert!(rows[0].parents.iter().any(|parent| parent.canonical));
    }
}
