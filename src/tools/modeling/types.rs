use super::{
    ArchivedEntity, ArchivedNovaMeta, BTreeMap, BTreeSet, JsonValue, ManifestSearch, Serialize,
};

#[derive(Debug, Clone, Serialize)]
pub(super) struct EntityRef {
    pub(super) unique_id: String,
    pub(super) name: String,
    pub(super) resource_type: String,
    pub(super) relation_name: Option<String>,
    pub(super) canonical: bool,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct GrainVariant {
    pub(super) sources: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(super) primary_key: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) time_field: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(super) dimensions: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct GrainComparison {
    pub(super) entity1: EntityRef,
    pub(super) entity2: EntityRef,
    pub(super) entity1_grain_variants: Vec<GrainVariant>,
    pub(super) entity2_grain_variants: Vec<GrainVariant>,
    pub(super) exact_match: bool,
    pub(super) same_time_field: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(super) shared_primary_key: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(super) entity1_only_primary_key: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(super) entity2_only_primary_key: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(super) shared_dimensions: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(super) entity1_only_dimensions: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(super) entity2_only_dimensions: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[allow(clippy::struct_field_names)]
pub(super) struct EntityOverlapEvidence {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(super) shared_name_tokens: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(super) shared_column_names: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(super) shared_parent_synonyms: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(super) shared_domains: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(super) shared_indicators: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(super) shared_column_semantic_types: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(super) shared_dimensions: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) shared_time_field: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct EntityOverlapRow {
    pub(super) entity1: EntityRef,
    pub(super) entity2: EntityRef,
    pub(super) score: f32,
    pub(super) surface_overlap_count: usize,
    pub(super) shared_value_count: usize,
    pub(super) evidence: EntityOverlapEvidence,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct DuplicateIndicatorParent {
    pub(super) unique_id: String,
    pub(super) name: String,
    pub(super) resource_type: String,
    pub(super) relation_name: Option<String>,
    pub(super) canonical: bool,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct DuplicateIndicatorRow {
    pub(super) indicator_name: String,
    pub(super) indicator_type: String,
    pub(super) parent_count: usize,
    pub(super) canonical_parent_count: usize,
    pub(super) parents_without_grain: usize,
    pub(super) inconsistent_grains: bool,
    pub(super) parents: Vec<DuplicateIndicatorParent>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(super) grain_signatures: Vec<GrainVariant>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct MultiGrainEntityRow {
    pub(super) entity: EntityRef,
    pub(super) grain_variant_count: usize,
    pub(super) grain_variants: Vec<GrainVariant>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum AgentModellingSeverity {
    Blocker,
    High,
    Medium,
    Low,
}

impl AgentModellingSeverity {
    pub(super) fn sort_rank(self) -> u8 {
        match self {
            Self::Blocker => 0,
            Self::High => 1,
            Self::Medium => 2,
            Self::Low => 3,
        }
    }

    pub(super) fn summary_key(self) -> &'static str {
        match self {
            Self::Blocker => "blockers",
            Self::High => "high",
            Self::Medium => "medium",
            Self::Low => "low",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct ModelingEntityRef {
    pub(super) unique_id: String,
    pub(super) name: String,
    pub(super) resource_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) relation_name: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct ModelingIndicatorRef {
    pub(super) indicator_name: String,
    pub(super) indicator_type: String,
    pub(super) parent_unique_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) source: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct AgentModellingFinding {
    pub(super) code: &'static str,
    pub(super) severity: AgentModellingSeverity,
    pub(super) category: &'static str,
    pub(super) message: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(super) entities: Vec<ModelingEntityRef>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(super) indicators: Vec<ModelingIndicatorRef>,
    pub(super) evidence: JsonValue,
    pub(super) recommendation: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(super) drill_down_hints: Vec<JsonValue>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct ModellingConsistencyReport {
    pub(super) summary: JsonValue,
    pub(super) agent_modelling_schema_version: &'static str,
    pub(super) entity_count: usize,
    pub(super) overlap_candidate_count: usize,
    pub(super) duplicate_indicator_count: usize,
    pub(super) canonical_conflict_count: usize,
    pub(super) multi_grain_entity_count: usize,
    pub(super) agent_modelling_finding_count: usize,
    pub(super) overlap_candidates: Vec<EntityOverlapRow>,
    pub(super) duplicate_indicators: Vec<DuplicateIndicatorRow>,
    pub(super) canonical_indicator_conflicts: Vec<DuplicateIndicatorRow>,
    pub(super) entities_with_multiple_grain_variants: Vec<MultiGrainEntityRow>,
    pub(super) agent_modelling_findings: Vec<AgentModellingFinding>,
}

#[derive(Clone)]
pub(super) struct EntityOverlapProfile {
    pub(super) unique_id: String,
    pub(super) name: String,
    pub(super) resource_type: String,
    pub(super) relation_name: Option<String>,
    pub(super) canonical: bool,
    pub(super) name_tokens: BTreeSet<String>,
    pub(super) column_names: BTreeSet<String>,
    pub(super) parent_synonyms: BTreeSet<String>,
    pub(super) domains: BTreeSet<String>,
    pub(super) indicator_names: BTreeSet<String>,
    pub(super) typed_indicators: BTreeSet<(String, String)>,
    pub(super) indicator_profiles: BTreeMap<(String, String), IndicatorOverlapIndicatorProfile>,
    pub(super) column_semantic_types: BTreeSet<String>,
    pub(super) grain_variants: Vec<GrainVariant>,
}

pub(super) struct OverlapRowsResult {
    pub(super) rows: Vec<EntityOverlapRow>,
    pub(super) candidate_pairs_truncated: bool,
}

pub(super) struct CandidatePairs {
    pub(super) pairs: BTreeSet<(usize, usize)>,
    pub(super) truncated: bool,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct ModellingReportPage {
    pub(super) limit: usize,
    pub(super) offset: usize,
    pub(super) overlap_candidate_generation_truncated: bool,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct AgentModellingSummaryInput<'a> {
    pub(super) findings: &'a [AgentModellingFinding],
    pub(super) truncated: bool,
}

pub(super) struct AgentModellingContext<'a> {
    pub(super) search: &'a ManifestSearch,
    pub(super) semantic_model_measure_names: BTreeSet<String>,
    pub(super) semantic_metric_names: BTreeSet<String>,
}

pub(super) struct MetricSurfaceContext<'a> {
    pub(super) unique_id: &'a str,
    pub(super) entity: &'a ArchivedEntity,
    pub(super) nova: &'a ArchivedNovaMeta,
    pub(super) semantic_metric_parent: bool,
    pub(super) relation_backed: bool,
    pub(super) column_names: &'a BTreeSet<String>,
}

#[derive(Clone)]
pub(super) struct IndicatorOverlapIndicatorProfile {
    pub(super) canonical: bool,
    pub(super) grain_variants: Vec<GrainVariant>,
}

impl EntityOverlapProfile {
    pub(super) fn is_comparable(&self) -> bool {
        !self.parent_synonyms.is_empty()
            || !self.domains.is_empty()
            || !self.indicator_names.is_empty()
            || !self.column_names.is_empty()
            || !self.column_semantic_types.is_empty()
            || !self.grain_variants.is_empty()
            || !self.name_tokens.is_empty()
    }

    pub(super) fn entity_ref(&self) -> EntityRef {
        EntityRef {
            unique_id: self.unique_id.clone(),
            name: self.name.clone(),
            resource_type: self.resource_type.clone(),
            relation_name: self.relation_name.clone(),
            canonical: self.canonical,
        }
    }
}
