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
const AGENT_MODELLING_TOP_BUCKETS: usize = 5;
const DEFAULT_MODELLING_OVERLAP_MIN_SCORE: f32 = 50.0;
const DEFAULT_MODELLING_SECTION_LIMIT: usize = 10;
const MAX_OVERLAP_BUCKET_SIZE: usize = 512;
const MAX_OVERLAP_CANDIDATE_PAIRS: usize = 250_000;
const AGENT_MODELLING_SEVERITY_ORDER: [AgentModellingSeverity; 4] = [
    AgentModellingSeverity::Blocker,
    AgentModellingSeverity::High,
    AgentModellingSeverity::Medium,
    AgentModellingSeverity::Low,
];

struct ModellingOverlapSection {
    rows: Vec<EntityOverlapRow>,
    total: usize,
    above_threshold: usize,
    applied_min_score: f32,
    candidate_pairs_truncated: bool,
}

mod agent_context;
mod agent_findings;
mod report;
mod types;

use agent_context::{
    ColumnSemanticRef, FactLikeParent, IndicatorExecutionSurface, SemanticLabelRef,
    agent_modelling_summary, analyst_candidate_disabled, best_grain_pair,
    canonical_indicator_refs_for_entity, collect_column_name_drift_findings,
    collect_column_role_conflict_findings, column_governance_present, column_primary_key_names,
    compare_duplicate_indicator_rows, compare_overlap_rows, direct_parent_ids, direct_parent_refs,
    duplicate_indicator_drill_down_hints, duplicate_indicator_refs,
    duplicate_indicator_summary_row, duplicate_parent_entity_refs, entity_columns_drill_down_hints,
    entity_drill_down_hints, entity_exposes_metric_or_measure, entity_is_analyst_facing,
    entity_layer, execution_surface, fact_like_direct_parents, fact_parent_entity_refs,
    fact_parent_grain_signatures, grain_field_names, grain_signature_key, grain_time_field,
    has_canonical_metric_or_measure, index_entity_indicator_labels, indicator_refs_for_entity,
    indicator_source_for_entity, is_distinctive_column_name, is_helper_layer,
    is_measure_like_data_type, metric_looks_ratio, metric_missing_time_field_finding,
    metric_ratio_signals, metricflow_measure_references, modeling_entity_ref,
    modeling_entity_ref_from_entity_ref, modeling_metric_ref, modelling_drill_down_hints,
    modelling_has_next_page, normalize_value, normalized_entity_column_names,
    normalized_resource_type_filter, nova_grain_primary_key_names,
    overlap_evidence_categories_for_row, overlap_evidence_category_counts, paginate_section,
    pii_like_column_signal, resource_type_allowed, score_from_overlap,
    semantic_label_drill_down_hints, semantic_label_entities, semantic_label_indicators,
    semantic_model_measure_refs, sort_agent_modelling_findings, sorted_difference,
    sorted_intersection, source_direct_parent_refs, truncate_agent_modelling_findings,
};
use agent_findings::{
    collect_column_semantic_ambiguity_findings, collect_duplicate_indicator_findings,
    collect_entity_agent_modelling_findings, collect_multi_grain_entity_findings,
    collect_semantic_label_collision_findings, semantic_metric_names, semantic_model_measure_names,
};
use report::{
    build_agent_modelling_findings, build_entity_grain_variants, build_entity_profile,
    build_modelling_consistency_summary, compare_entity_grains, duplicate_indicator_rows,
    multi_grain_entity_rows, overlap_rows,
};
use types::{
    AgentModellingContext, AgentModellingFinding, AgentModellingSeverity,
    AgentModellingSummaryInput, CandidatePairs, DuplicateIndicatorParent, DuplicateIndicatorRow,
    EntityOverlapEvidence, EntityOverlapProfile, EntityOverlapRow, EntityRef, GrainComparison,
    GrainVariant, IndicatorOverlapIndicatorProfile, MetricSurfaceContext, ModelingEntityRef,
    ModelingIndicatorRef, ModellingConsistencyReport, ModellingReportPage, MultiGrainEntityRow,
    OverlapRowsResult,
};

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
        let overlap_result = overlap_rows(&profiles, focus_unique_id.as_deref());
        let mut rows = overlap_result.rows;
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
        if total > end || overlap_result.candidate_pairs_truncated {
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
        let section_limit = self.modelling_section_limit(params.pagination.limit);
        let section_offset = params.pagination.offset;
        let overlap_section = modelling_overlap_section(&profiles, params.min_score);
        let overlap = paginate_section(&overlap_section.rows, section_offset, section_limit);

        let duplicate_indicator_rows = duplicate_indicator_rows(&profiles, usize::MAX);
        let duplicate_indicator_count = duplicate_indicator_rows.len();
        let duplicate_indicators =
            paginate_section(&duplicate_indicator_rows, section_offset, section_limit);
        let canonical_conflict_rows: Vec<DuplicateIndicatorRow> = duplicate_indicator_rows
            .iter()
            .filter(|row| row.canonical_parent_count > 1)
            .cloned()
            .collect();
        let canonical_conflict_count = canonical_conflict_rows.len();
        let canonical_conflicts =
            paginate_section(&canonical_conflict_rows, section_offset, section_limit);
        let multi_grain_entity_rows_all = multi_grain_entity_rows(&profiles);
        let multi_grain_entity_count = multi_grain_entity_rows_all.len();
        let multi_grain_entities =
            paginate_section(&multi_grain_entity_rows_all, section_offset, section_limit);
        let mut agent_modelling_findings_all = if self.config.agent_modelling_audit.enabled {
            build_agent_modelling_findings(
                self,
                &profiles,
                &duplicate_indicator_rows,
                &multi_grain_entity_rows_all,
            )?
        } else {
            Vec::new()
        };
        sort_agent_modelling_findings(&mut agent_modelling_findings_all);
        let agent_modelling_max_findings = self.config.agent_modelling_audit.max_findings;
        let agent_modelling_finding_count = agent_modelling_findings_all.len();
        let agent_modelling_findings_truncated =
            agent_modelling_finding_count > agent_modelling_max_findings;
        let agent_modelling_findings = truncate_agent_modelling_findings(
            &agent_modelling_findings_all,
            agent_modelling_max_findings,
        );
        let summary = build_modelling_consistency_summary(
            params,
            ModellingReportPage {
                limit: section_limit,
                offset: section_offset,
                applied_min_score: overlap_section.applied_min_score,
                overlap_candidates_total: overlap_section.total,
                overlap_candidates_above_threshold: overlap_section.above_threshold,
                overlap_candidate_generation_truncated: overlap_section.candidate_pairs_truncated,
            },
            &overlap_section.rows,
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
            applied_min_score: overlap_section.applied_min_score,
            overlap_candidates_total: overlap_section.total,
            overlap_candidates_above_threshold: overlap_section.above_threshold,
            overlap_candidate_count: overlap_section.above_threshold,
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

    fn modelling_section_limit(&self, requested: Option<usize>) -> usize {
        self.page_limit(requested.or(Some(DEFAULT_MODELLING_SECTION_LIMIT)))
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

fn modelling_overlap_section(
    profiles: &[EntityOverlapProfile],
    min_score: Option<f32>,
) -> ModellingOverlapSection {
    let overlap_result = overlap_rows(profiles, None);
    let mut rows = overlap_result.rows;
    let total = rows.len();
    let applied_min_score = min_score.unwrap_or(DEFAULT_MODELLING_OVERLAP_MIN_SCORE);
    if applied_min_score > 0.0 {
        rows.retain(|row| row.score >= applied_min_score);
    }

    ModellingOverlapSection {
        above_threshold: rows.len(),
        rows,
        total,
        applied_min_score,
        candidate_pairs_truncated: overlap_result.candidate_pairs_truncated,
    }
}

#[cfg(test)]
mod tests;
