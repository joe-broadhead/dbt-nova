use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use rkyv::string::ArchivedString;
use serde::Serialize;
use serde_json::Value as JsonValue;
use tracing::instrument;

use crate::error::{DbtNovaError, Result};
use crate::manifest::entity::{ArchivedEntity, ArchivedNovaGrain, ArchivedNovaMeta};
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
    #[instrument(skip(self, params), fields(tool = "find_entity_overlap", limit = params.pagination.limit, offset = params.pagination.offset))]
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
    #[instrument(skip(self, params), fields(tool = "modelling_consistency_report", limit = params.pagination.limit, offset = params.pagination.offset))]
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

        let mut overlap = overlap_rows(&profiles, None);
        if let Some(min_score) = params.min_score {
            overlap.retain(|row| row.score >= min_score);
        }
        let overlap_count = overlap.len();
        overlap.truncate(section_limit);

        let duplicate_indicators = duplicate_indicator_rows(&profiles, section_limit);
        let duplicate_indicator_count = duplicate_indicator_rows(&profiles, usize::MAX).len();
        let canonical_conflicts: Vec<DuplicateIndicatorRow> =
            duplicate_indicator_rows(&profiles, usize::MAX)
                .into_iter()
                .filter(|row| row.canonical_parent_count > 1)
                .take(section_limit)
                .collect();
        let canonical_conflict_count = duplicate_indicator_rows(&profiles, usize::MAX)
            .into_iter()
            .filter(|row| row.canonical_parent_count > 1)
            .count();
        let mut multi_grain_entities = multi_grain_entity_rows(&profiles);
        let multi_grain_entity_count = multi_grain_entities.len();
        multi_grain_entities.truncate(section_limit);

        let report = ModellingConsistencyReport {
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
    column_semantic_types: BTreeSet<String>,
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

    fn preferred_grain(&self) -> Option<&GrainVariant> {
        self.grain_variants.first()
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
    if let Some(nova) = nova {
        for measure in nova.measures.iter() {
            let name = normalize_value(measure.name.as_str());
            if !name.is_empty() {
                indicator_names.insert(name.clone());
                typed_indicators.insert(("measure".to_string(), name));
            }
        }
        if let Some(metric) = nova.metric.as_ref() {
            let name = normalize_value(metric.name.as_str());
            if !name.is_empty() {
                indicator_names.insert(name.clone());
                typed_indicators.insert(("metric".to_string(), name));
            }
        }
        for metric in nova.metrics.iter() {
            let name = normalize_value(metric.name.as_str());
            if !name.is_empty() {
                indicator_names.insert(name.clone());
                typed_indicators.insert(("metric".to_string(), name));
            }
        }
    }

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
        column_semantic_types,
        grain_variants: build_grain_variants(nova),
    }
}

fn build_grain_variants(nova: Option<&ArchivedNovaMeta>) -> Vec<GrainVariant> {
    let Some(nova) = nova else {
        return Vec::new();
    };

    let mut variants: Vec<GrainVariant> = Vec::new();
    let mut positions: HashMap<String, usize> = HashMap::new();

    let mut push_variant = |source: String, grain: &ArchivedNovaGrain| {
        let candidate = GrainVariant {
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
        };
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
        let canonical_parent_count = parents.iter().filter(|profile| profile.canonical).count();
        let parents_without_grain = parents
            .iter()
            .filter(|profile| profile.preferred_grain().is_none())
            .count();
        let mut grain_signatures: BTreeMap<String, GrainVariant> = BTreeMap::new();
        for profile in &parents {
            for grain in &profile.grain_variants {
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
                    canonical: profile.canonical,
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

fn grain_signature_key(grain: &GrainVariant) -> String {
    format!(
        "pk={};time={};dim={}",
        grain.primary_key.join(","),
        grain.time_field.clone().unwrap_or_default(),
        grain.dimensions.join(",")
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
            column_semantic_types: BTreeSet::new(),
            grain_variants,
        }
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
}
