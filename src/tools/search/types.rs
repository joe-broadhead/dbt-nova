use super::{
    ArchivedEntity, ArchivedNovaMeta, BTreeMap, HashMap, HashSet, IndicatorRankingConfig,
    MetadataSupportConfig, PersonaWeights, SearchDeadline, SearchHit, SearchPersona, Serialize,
    bool_to_f32, merge_signal_values, usize_to_f32,
};

#[derive(Debug, Clone, Serialize)]
pub(super) struct IndicatorGrainSummary {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(super) primary_key: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) time_field: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(super) dimensions: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub(super) struct IndicatorExecutionMetadata {
    pub(super) indicator_source: &'static str,
    pub(super) execution_surface: &'static str,
    pub(super) queryable: bool,
    pub(super) direct_sql_queryable: bool,
    pub(super) queryable_via: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) execution_note: Option<&'static str>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct IndicatorSearchRow {
    pub(super) indicator_name: String,
    pub(super) indicator_type: String,
    pub(super) canonical: bool,
    pub(super) match_type: String,
    pub(super) score: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) expression: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) field: Option<String>,
    pub(super) parent_unique_id: String,
    pub(super) parent_name: String,
    pub(super) parent_resource_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) relation_name: Option<String>,
    #[serde(flatten)]
    pub(super) execution: IndicatorExecutionMetadata,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(super) domains: Vec<String>,
    pub(super) grain: IndicatorGrainSummary,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) support_signals: Option<MetadataSupportSignals>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) explain: Option<IndicatorScoreExplain>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct IndicatorInventoryRow {
    pub(super) indicator_name: String,
    pub(super) indicator_type: String,
    pub(super) canonical: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(super) synonyms: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) expression: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) field: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) measure_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) template: Option<bool>,
    pub(super) parent_unique_id: String,
    pub(super) parent_name: String,
    pub(super) parent_resource_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) relation_name: Option<String>,
    #[serde(flatten)]
    pub(super) execution: IndicatorExecutionMetadata,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(super) domains: Vec<String>,
    pub(super) grain: IndicatorGrainSummary,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct ColumnInventoryRow {
    pub(super) column_name: String,
    pub(super) annotated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) semantic_type: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(super) synonyms: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(super) example_values: Vec<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub(super) primary_key: bool,
    pub(super) parent_unique_id: String,
    pub(super) parent_name: String,
    pub(super) parent_resource_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) relation_name: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(super) domains: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct ColumnSearchRow {
    pub(super) column_name: String,
    pub(super) match_type: String,
    pub(super) score: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) matched_value: Option<String>,
    pub(super) annotated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) semantic_type: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(super) synonyms: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(super) example_values: Vec<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub(super) primary_key: bool,
    pub(super) parent_unique_id: String,
    pub(super) parent_name: String,
    pub(super) parent_resource_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) relation_name: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(super) domains: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct IndicatorParentGroupItem {
    pub(super) indicator_name: String,
    pub(super) indicator_type: String,
    pub(super) canonical: bool,
    pub(super) match_type: String,
    pub(super) score: f32,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct IndicatorParentGroup {
    pub(super) parent_unique_id: String,
    pub(super) parent_name: String,
    pub(super) parent_resource_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) relation_name: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(super) domains: Vec<String>,
    pub(super) best_score: f32,
    pub(super) indicator_count: usize,
    pub(super) grain: IndicatorGrainSummary,
    pub(super) indicators: Vec<IndicatorParentGroupItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) support_signals: Option<MetadataSupportSignals>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub(super) struct MetadataSupportSignals {
    #[serde(
        rename = "matched_parent_synonyms",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub(super) parent_synonyms: Vec<String>,
    #[serde(rename = "matched_domains", skip_serializing_if = "Vec::is_empty")]
    pub(super) domains: Vec<String>,
    #[serde(rename = "matched_use_cases", skip_serializing_if = "Vec::is_empty")]
    pub(super) use_cases: Vec<String>,
    #[serde(rename = "matched_dimensions", skip_serializing_if = "Vec::is_empty")]
    pub(super) dimensions: Vec<String>,
    #[serde(rename = "matched_column_names", skip_serializing_if = "Vec::is_empty")]
    pub(super) column_names: Vec<String>,
    #[serde(rename = "matched_column_roles", skip_serializing_if = "Vec::is_empty")]
    pub(super) column_roles: Vec<String>,
    #[serde(
        rename = "matched_column_semantic_types",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub(super) column_semantic_types: Vec<String>,
    #[serde(
        rename = "matched_example_values",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub(super) example_values: Vec<String>,
    #[serde(
        rename = "matched_exact_phrases",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub(super) exact_phrases: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) best_single_field_query_coverage: Option<f32>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct RetrieverContribution {
    pub(super) rank: usize,
    pub(super) score: f32,
}

#[derive(Debug, Clone, Default, Serialize)]
pub(super) struct RetrievalExplain {
    pub(super) total_score: f32,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub(super) retrievers: BTreeMap<String, RetrieverContribution>,
}

#[derive(Debug, Clone, Serialize)]
#[allow(clippy::struct_excessive_bools)]
pub(super) struct SearchScoreExplain {
    pub(super) base_score: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) retrieval: Option<RetrievalExplain>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) pre_rerank_retrieval_score: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) reranker_score: Option<f32>,
    pub(super) exact_match: bool,
    pub(super) resource_type_multiplier: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) staging_deboost_factor: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) missing_nova_multiplier: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) candidate_false_multiplier: Option<f32>,
    pub(super) measure_match: bool,
    pub(super) metric_match: bool,
    pub(super) synonym_match: bool,
    pub(super) canonical_entity: bool,
    pub(super) canonical_semantic_match: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) engineer_exact_match_multiplier: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) canonical_entity_multiplier: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) measure_match_multiplier: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) metric_match_multiplier: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) synonym_match_multiplier: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) phrase_match_multiplier: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) strongest_match_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) semantic_match_multiplier: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) query_coverage_multiplier: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) semantic_canonical_match_multiplier: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) semantic_canonical_match_bonus: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) canonical_match_multiplier: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) canonical_match_bonus: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) analyst_semantic_multiplier: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) semantic_label_precision_factor: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) metadata_support_factor: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) indicator_parent_factor: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) docs_multiplier: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) tests_multiplier: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) tags_multiplier: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) path_multiplier: Option<f32>,
    pub(super) final_score: f32,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct IndicatorScoreExplain {
    pub(super) match_base: f32,
    pub(super) query_coverage: f32,
    pub(super) base_match_score: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) generic_label_bonus: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) parent_synonym_bonus: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) metadata_support_bonus: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) phrase_match_bonus: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) time_field_bonus: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) dimension_bonus: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) parent_coherence_bonus: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) rrf_bonus: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) reranker_bonus: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) retrieval: Option<RetrievalExplain>,
    pub(super) final_score: f32,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct SearchExplainConfigSnapshot {
    pub(super) rrf_k: f32,
    pub(super) rerank_top_n: usize,
    pub(super) persona_weights: PersonaWeights,
    pub(super) indicator_ranking: IndicatorRankingConfig,
    pub(super) metadata_support: MetadataSupportConfig,
}

#[derive(Debug, Clone, Serialize)]
#[allow(clippy::struct_excessive_bools)]
pub(super) struct SearchExplainPayload {
    pub(super) query_tokens: Vec<String>,
    pub(super) query_has_syntax: bool,
    pub(super) rrf_enabled: bool,
    pub(super) reranker_enabled: bool,
    pub(super) reranker_applied: bool,
    pub(super) retrievers_used: Vec<String>,
    pub(super) config: SearchExplainConfigSnapshot,
}

impl MetadataSupportSignals {
    pub(super) fn surface_count(&self) -> usize {
        usize::from(!self.parent_synonyms.is_empty())
            + usize::from(!self.domains.is_empty())
            + usize::from(!self.use_cases.is_empty())
            + usize::from(!self.dimensions.is_empty())
            + usize::from(!self.column_names.is_empty())
            + usize::from(!self.column_roles.is_empty())
            + usize::from(!self.column_semantic_types.is_empty())
            + usize::from(!self.example_values.is_empty())
            + usize::from(!self.exact_phrases.is_empty())
    }

    pub(super) fn merge_from(&mut self, other: &Self, max_values_per_field: usize) {
        merge_signal_values(
            &mut self.parent_synonyms,
            &other.parent_synonyms,
            max_values_per_field,
        );
        merge_signal_values(&mut self.domains, &other.domains, max_values_per_field);
        merge_signal_values(&mut self.use_cases, &other.use_cases, max_values_per_field);
        merge_signal_values(
            &mut self.dimensions,
            &other.dimensions,
            max_values_per_field,
        );
        merge_signal_values(
            &mut self.column_names,
            &other.column_names,
            max_values_per_field,
        );
        merge_signal_values(
            &mut self.column_roles,
            &other.column_roles,
            max_values_per_field,
        );
        merge_signal_values(
            &mut self.column_semantic_types,
            &other.column_semantic_types,
            max_values_per_field,
        );
        merge_signal_values(
            &mut self.example_values,
            &other.example_values,
            max_values_per_field,
        );
        merge_signal_values(
            &mut self.exact_phrases,
            &other.exact_phrases,
            max_values_per_field,
        );
        self.best_single_field_query_coverage = match (
            self.best_single_field_query_coverage,
            other.best_single_field_query_coverage,
        ) {
            (Some(left), Some(right)) => Some(left.max(right)),
            (Some(value), None) | (None, Some(value)) => Some(value),
            (None, None) => None,
        };
    }
}

#[derive(Clone)]
pub(super) struct SearchCandidate<'a> {
    pub(super) unique_id: String,
    pub(super) entity: Option<&'a ArchivedEntity>,
    pub(super) score: f32,
    pub(super) support_signals: Option<MetadataSupportSignals>,
    pub(super) indicator_parent_score: Option<f32>,
    pub(super) explain: Option<SearchScoreExplain>,
}

pub(super) struct SearchScoreOutcome {
    pub(super) score: f32,
    pub(super) explain: Option<SearchScoreExplain>,
}

pub(super) type PreparedIndicatorSearch = (
    Vec<String>,
    bool,
    Option<HashSet<String>>,
    Option<HashSet<String>>,
    SearchPersona,
);

#[derive(Clone, Copy)]
pub(super) struct SearchScoreContext<'a> {
    pub(super) token_set: &'a HashSet<&'a str>,
    pub(super) min_word_len: usize,
    pub(super) persona: SearchPersona,
    pub(super) query_text: &'a str,
    pub(super) support_signals: Option<&'a MetadataSupportSignals>,
    pub(super) has_indicator_parent_scores: bool,
    pub(super) indicator_parent_score: Option<f32>,
}

pub(super) struct IndicatorSearchContext<'a> {
    pub(super) unique_id: &'a str,
    pub(super) entity: &'a ArchivedEntity,
    pub(super) nova: &'a ArchivedNovaMeta,
    pub(super) include_explain: bool,
    pub(super) token_set: &'a HashSet<&'a str>,
    pub(super) query_token_count: usize,
    pub(super) min_word_len: usize,
    pub(super) support_signals: Option<MetadataSupportSignals>,
    pub(super) indicator_config: &'a IndicatorRankingConfig,
    pub(super) metadata_config: &'a MetadataSupportConfig,
    pub(super) phrase_boost_enabled: bool,
}

#[derive(Clone, Copy)]
pub(super) struct ColumnSearchMatch<'a> {
    pub(super) match_type: &'static str,
    pub(super) matched_value: Option<&'a str>,
    pub(super) score: f32,
}

pub(super) struct FusedHitContext<'a> {
    pub(super) persona: SearchPersona,
    pub(super) persona_weights: PersonaWeights,
    pub(super) query_has_syntax: bool,
    pub(super) fetch_limit: usize,
    pub(super) primary_results: &'a [SearchHit],
    pub(super) deadline: SearchDeadline,
}

pub(super) struct FusedHitBundle {
    pub(super) hits: Vec<(String, f32)>,
    pub(super) indicator_parent_scores: HashMap<String, f32>,
    pub(super) retrieval_explain: HashMap<String, RetrievalExplain>,
    pub(super) retrievers_used: Vec<String>,
}

#[derive(Default)]
pub(super) struct ParentIndicatorCoherence {
    pub(super) distinct_indicator_names: HashSet<String>,
    pub(super) canonical_indicator_count: usize,
    pub(super) strong_match_count: usize,
    pub(super) support_surface_count: usize,
    pub(super) has_time_field: bool,
    pub(super) has_dimensions: bool,
}

impl ParentIndicatorCoherence {
    pub(super) fn record(&mut self, row: &IndicatorSearchRow) {
        self.distinct_indicator_names
            .insert(row.indicator_name.clone());
        if row.canonical {
            self.canonical_indicator_count += 1;
        }
        if matches!(row.match_type.as_str(), "name" | "synonym") {
            self.strong_match_count += 1;
        }
        if let Some(signals) = &row.support_signals {
            self.support_surface_count = self.support_surface_count.max(signals.surface_count());
        }
        self.has_time_field |= row.grain.time_field.is_some();
        self.has_dimensions |= !row.grain.dimensions.is_empty();
    }

    pub(super) fn bonus(&self, config: &IndicatorRankingConfig) -> f32 {
        let indicator_diversity_bonus =
            self.distinct_indicator_names.len().saturating_sub(1).min(3);
        let canonical_indicator_count = self.canonical_indicator_count.min(2);
        let strong_match_count = self.strong_match_count.min(3);
        let support_surface_count = self.support_surface_count.min(5);
        let grain_bonus = bool_to_f32(self.has_time_field)
            * config.parent_coherence_time_field_bonus
            + bool_to_f32(self.has_dimensions) * config.parent_coherence_dimension_bonus;

        (usize_to_f32(indicator_diversity_bonus)
            * config.parent_coherence_indicator_diversity_bonus
            + usize_to_f32(canonical_indicator_count)
                * config.parent_coherence_canonical_indicator_bonus
            + usize_to_f32(strong_match_count) * config.parent_coherence_strong_match_bonus
            + usize_to_f32(support_surface_count) * config.parent_coherence_support_surface_bonus
            + grain_bonus)
            .min(config.parent_coherence_max_bonus)
    }
}
