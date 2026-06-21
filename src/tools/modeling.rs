use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use rkyv::string::ArchivedString;
use serde::Serialize;
use serde_json::{Value as JsonValue, json};
use tracing::instrument;

use crate::error::{DbtNovaError, Result};
use crate::manifest::entity::{
    ArchivedEntity, ArchivedNovaGrain, ArchivedNovaMeta, ArchivedNovaMetric,
};
use crate::manifest::search::ManifestSearch;
use crate::params::{
    CompareGrainsParams, FindEntityOverlapParams, ModellingConsistencyReportParams,
};
use crate::responses::SuccessResponse;
use crate::utils::tokenize_alnum_lowercase;

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
        let summary = build_modelling_consistency_summary(
            params,
            section_limit,
            section_offset,
            &overlap_rows_all,
            &duplicate_indicator_rows,
            &canonical_conflict_rows,
            &multi_grain_entity_rows_all,
        );

        let report = ModellingConsistencyReport {
            summary,
            entity_count: profiles.len(),
            overlap_candidate_count: overlap_count,
            duplicate_indicator_count,
            canonical_conflict_count,
            multi_grain_entity_count,
            overlap_candidates: overlap,
            duplicate_indicators,
            canonical_indicator_conflicts: canonical_conflicts,
            entities_with_multiple_grain_variants: multi_grain_entities,
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

#[derive(Debug, Clone, Serialize)]
struct ModellingConsistencyReport {
    summary: JsonValue,
    entity_count: usize,
    overlap_candidate_count: usize,
    duplicate_indicator_count: usize,
    canonical_conflict_count: usize,
    multi_grain_entity_count: usize,
    overlap_candidates: Vec<EntityOverlapRow>,
    duplicate_indicators: Vec<DuplicateIndicatorRow>,
    canonical_indicator_conflicts: Vec<DuplicateIndicatorRow>,
    entities_with_multiple_grain_variants: Vec<MultiGrainEntityRow>,
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
    section_limit: usize,
    section_offset: usize,
    overlap_rows: &[EntityOverlapRow],
    duplicate_indicator_rows: &[DuplicateIndicatorRow],
    canonical_conflict_rows: &[DuplicateIndicatorRow],
    multi_grain_entity_rows: &[MultiGrainEntityRow],
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
        section_limit,
        section_offset,
        overlap_rows,
        multi_grain_entity_rows,
    )
    .then_some(section_offset.saturating_add(section_limit));

    json!({
        "section_counts": {
            "overlap_candidates": overlap_rows.len(),
            "duplicate_indicators": duplicate_indicator_rows.len(),
            "canonical_indicator_conflicts": canonical_conflict_rows.len(),
            "entities_with_multiple_grain_variants": multi_grain_entity_rows.len()
        },
        "page": {
            "limit": section_limit,
            "offset": section_offset,
            "next_offset": next_offset
        },
        "overlap_evidence_categories": overlap_evidence_categories,
        "overlap_examples": overlap_examples,
        "top_duplicate_indicator_groups": top_duplicate_indicator_groups,
        "top_canonical_conflicts": top_canonical_conflicts,
        "top_multi_grain_entities": top_multi_grain_entities,
        "drill_down_hints": modelling_drill_down_hints(params, section_limit, section_offset, overlap_rows, multi_grain_entity_rows)
    })
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
