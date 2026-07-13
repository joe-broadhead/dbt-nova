use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::error::{DbtNovaError, Result};

mod env;
mod extended_meta;

pub use extended_meta::{ExtendedMetaFieldConfig, ExtendedMetaFieldMode, ExtendedMetaSearchConfig};

/// Weight multipliers for persona-aware ranking signals.
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(default)]
pub struct PersonaWeights {
    pub bm25: f32,
    pub ngram: f32,
    pub fuzzy: f32,
    pub vector: f32,
    pub sparse: f32,
    pub indicator: f32,
    pub measures: f32,
    pub metrics: f32,
    pub synonyms: f32,
    pub docs: f32,
    pub tests: f32,
    pub tags: f32,
    pub path: f32,
}

impl Default for PersonaWeights {
    fn default() -> Self {
        Self {
            bm25: 1.0,
            ngram: 1.0,
            fuzzy: 1.0,
            vector: 1.0,
            sparse: 1.0,
            indicator: 1.0,
            measures: 1.0,
            metrics: 1.0,
            synonyms: 1.0,
            docs: 1.0,
            tests: 1.0,
            tags: 1.0,
            path: 1.0,
        }
    }
}

impl PersonaWeights {
    pub(crate) fn apply_overrides(&mut self, raw: &str) {
        for pair in raw.split(',') {
            let pair = pair.trim();
            if pair.is_empty() {
                continue;
            }
            let mut parts = pair.splitn(2, '=');
            let key = parts.next().unwrap_or("").trim().to_lowercase();
            let value = parts.next().unwrap_or("").trim();
            let Ok(parsed) = value.parse::<f32>() else {
                continue;
            };
            match key.as_str() {
                "bm25" => self.bm25 = parsed,
                "ngram" => self.ngram = parsed,
                "fuzzy" => self.fuzzy = parsed,
                "vector" => self.vector = parsed,
                "sparse" => self.sparse = parsed,
                "indicator" => self.indicator = parsed,
                "measures" => self.measures = parsed,
                "metrics" => self.metrics = parsed,
                "synonyms" => self.synonyms = parsed,
                "docs" | "documentation" => self.docs = parsed,
                "tests" => self.tests = parsed,
                "tags" => self.tags = parsed,
                "path" => self.path = parsed,
                _ => {}
            }
        }
    }
}

/// Field boost weights for search ranking.
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(default)]
pub struct FieldBoosts {
    pub alias: f32,
    pub name: f32,
    pub description: f32,
    pub column: f32,
    pub tag: f32,
    pub path: f32,
    pub code: f32,
    pub nova_synonyms: f32,
    pub nova_domains: f32,
    pub nova_use_cases: f32,
    pub nova_measures: f32,
    pub nova_metric: f32,
    pub nova_sensitivity: f32,
    pub nova_pii: f32,
    pub nova_compliance: f32,
}

impl Default for FieldBoosts {
    fn default() -> Self {
        Self {
            alias: 18.0,
            name: 12.0,
            description: 6.0,
            column: 4.0,
            tag: 3.0,
            path: 2.0,
            code: 1.5,
            nova_synonyms: 7.0,
            nova_domains: 4.0,
            nova_use_cases: 4.0,
            nova_measures: 8.0,
            nova_metric: 10.0,
            nova_sensitivity: 6.0,
            nova_pii: 8.0,
            nova_compliance: 6.0,
        }
    }
}

/// Named persona profiles for search weighting.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct PersonaProfile<T> {
    pub analyst: T,
    pub engineer: T,
    pub governance: T,
    pub default: T,
}

impl Default for PersonaProfile<PersonaWeights> {
    fn default() -> Self {
        Self {
            analyst: PersonaWeights {
                bm25: 1.0,
                ngram: 1.1,
                fuzzy: 1.0,
                vector: 1.5,
                sparse: 1.2,
                indicator: 1.6,
                measures: 1.3,
                metrics: 1.4,
                synonyms: 1.2,
                docs: 1.2,
                tests: 1.2,
                tags: 1.0,
                path: 0.9,
            },
            engineer: PersonaWeights {
                bm25: 1.5,
                ngram: 1.0,
                fuzzy: 0.8,
                vector: 0.8,
                sparse: 1.0,
                indicator: 1.0,
                measures: 0.9,
                metrics: 0.9,
                synonyms: 0.9,
                docs: 0.9,
                tests: 1.0,
                tags: 0.9,
                path: 1.3,
            },
            governance: PersonaWeights {
                bm25: 1.2,
                ngram: 1.0,
                fuzzy: 1.0,
                vector: 1.0,
                sparse: 1.3,
                indicator: 1.0,
                measures: 0.9,
                metrics: 0.9,
                synonyms: 1.0,
                docs: 1.3,
                tests: 1.4,
                tags: 1.4,
                path: 1.0,
            },
            default: PersonaWeights::default(),
        }
    }
}

/// Analyst persona semantic signal multipliers.
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(default)]
pub struct AnalystSemanticConfig {
    pub metric_definition_multiplier: f32,
    pub measure_definition_multiplier: f32,
    pub grain_multiplier: f32,
    pub time_field_multiplier: f32,
    pub dimension_overlap_one_multiplier: f32,
    pub dimension_overlap_two_multiplier: f32,
    pub dimension_overlap_three_plus_multiplier: f32,
    pub missing_metric_or_measure_multiplier: f32,
    pub missing_grain_multiplier: f32,
    pub min_multiplier: f32,
    pub max_multiplier: f32,
}

impl Default for AnalystSemanticConfig {
    fn default() -> Self {
        Self {
            metric_definition_multiplier: 1.09,
            measure_definition_multiplier: 1.05,
            grain_multiplier: 1.05,
            time_field_multiplier: 1.03,
            dimension_overlap_one_multiplier: 1.03,
            dimension_overlap_two_multiplier: 1.06,
            dimension_overlap_three_plus_multiplier: 1.09,
            missing_metric_or_measure_multiplier: 0.96,
            missing_grain_multiplier: 0.97,
            min_multiplier: 0.85,
            max_multiplier: 1.35,
        }
    }
}

/// Tunable weights for indicator-specific ranking and grouped search output.
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(default)]
pub struct IndicatorRankingConfig {
    pub enable_parent_coherence: bool,
    pub parent_group_max_groups: usize,
    pub parent_group_max_indicators: usize,
    pub generic_label_bonus_one_token: f32,
    pub generic_label_bonus_two_tokens: f32,
    pub generic_label_bonus_three_plus_tokens: f32,
    pub parent_synonym_bonus_one_token: f32,
    pub parent_synonym_bonus_two_tokens: f32,
    pub parent_synonym_bonus_three_plus_tokens: f32,
    pub semantic_label_precision_scale: f32,
    pub semantic_label_precision_canonical_bonus: f32,
    pub parent_coherence_indicator_diversity_bonus: f32,
    pub parent_coherence_canonical_indicator_bonus: f32,
    pub parent_coherence_strong_match_bonus: f32,
    pub parent_coherence_support_surface_bonus: f32,
    pub parent_coherence_time_field_bonus: f32,
    pub parent_coherence_dimension_bonus: f32,
    pub parent_coherence_max_bonus: f32,
    pub search_parent_indicator_bonus_scale: f32,
    pub search_missing_indicator_parent_multiplier: f32,
    pub search_parent_indicator_top_k: usize,
    pub indicator_rrf_score_weight: f32,
    pub indicator_reranker_score_weight: f32,
}

impl Default for IndicatorRankingConfig {
    fn default() -> Self {
        Self {
            enable_parent_coherence: true,
            parent_group_max_groups: 5,
            parent_group_max_indicators: 4,
            generic_label_bonus_one_token: 2.5,
            generic_label_bonus_two_tokens: 1.5,
            generic_label_bonus_three_plus_tokens: 0.75,
            parent_synonym_bonus_one_token: 1.5,
            parent_synonym_bonus_two_tokens: 1.0,
            parent_synonym_bonus_three_plus_tokens: 0.5,
            semantic_label_precision_scale: 0.14,
            semantic_label_precision_canonical_bonus: 0.12,
            parent_coherence_indicator_diversity_bonus: 0.28,
            parent_coherence_canonical_indicator_bonus: 0.14,
            parent_coherence_strong_match_bonus: 0.08,
            parent_coherence_support_surface_bonus: 0.06,
            parent_coherence_time_field_bonus: 0.06,
            parent_coherence_dimension_bonus: 0.06,
            parent_coherence_max_bonus: 0.95,
            search_parent_indicator_bonus_scale: 0.55,
            search_missing_indicator_parent_multiplier: 0.75,
            search_parent_indicator_top_k: 3,
            indicator_rrf_score_weight: 1.0,
            indicator_reranker_score_weight: 1.0,
        }
    }
}

/// Tunable weights for metadata-derived support signals used in ranking and output.
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(default)]
pub struct MetadataSupportConfig {
    pub parent_synonym_weight: f32,
    pub domain_weight: f32,
    pub use_case_weight: f32,
    pub dimension_weight: f32,
    pub column_name_weight: f32,
    pub column_role_weight: f32,
    pub semantic_type_weight: f32,
    pub example_value_weight: f32,
    pub max_bonus: f32,
    pub max_values_per_field: usize,
}

impl Default for MetadataSupportConfig {
    fn default() -> Self {
        Self {
            parent_synonym_weight: 0.4,
            domain_weight: 0.35,
            use_case_weight: 0.25,
            dimension_weight: 0.45,
            column_name_weight: 0.2,
            column_role_weight: 0.2,
            semantic_type_weight: 0.35,
            example_value_weight: 0.5,
            max_bonus: 1.25,
            max_values_per_field: 4,
        }
    }
}

/// Search and ranking configuration for the Tantivy index and reranking.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct SearchConfig {
    /// Manifest hash used for embedding cache validation (optional).
    pub manifest_hash: Option<String>,
    /// Default limit for search results
    pub default_limit: usize,
    /// Maximum results per page across list endpoints
    pub max_page_size: usize,
    /// Maximum pagination offset allowed for search requests
    pub max_offset: usize,
    /// Maximum number of lineage responses to cache (0 disables cache)
    pub lineage_cache_size: usize,
    /// Precompute SQL alias mappings for column lineage at index time
    pub column_lineage_precompute: bool,
    /// Minimum query word length for indexing and searching
    pub min_word_length: usize,
    /// Maximum query length in characters (security limit)
    pub max_query_length: usize,
    /// Maximum path pattern length in characters (security limit)
    pub max_path_pattern_length: usize,
    /// Tantivy index directory name (relative to `storage_dir`)
    pub index_dir: String,
    /// Tantivy index writer heap size in bytes
    pub index_writer_heap_bytes: usize,
    /// Maximum bytes of SQL indexed per field
    pub max_sql_chunk_bytes: usize,
    /// Multiplier for fetching extra docs when de-duplicating search hits
    pub dedup_fetch_multiplier: usize,
    /// Enable n-gram indexing for partial matches
    pub enable_ngram: bool,
    /// Minimum n-gram size
    pub ngram_min: usize,
    /// Maximum n-gram size
    pub ngram_max: usize,
    /// Boost applied to n-gram fields
    pub ngram_boost: f32,
    /// Minimum term length before fuzzy matching
    pub fuzzy_min_length: usize,
    /// Term length threshold for medium fuzzy distance
    pub fuzzy_mid_length: usize,
    /// Maximum fuzzy edit distance for long terms
    pub fuzzy_max_distance: usize,
    /// Max highlight snippet length in characters
    pub highlight_max_chars: usize,
    /// Max number of highlighted fields returned per result
    pub highlight_max_fields: usize,
    /// Highlight format: "html" or "text"
    pub highlight_format: String,
    /// Enable "did you mean" suggestions when no results are found
    pub enable_suggestions: bool,
    /// Max number of suggestions to return
    pub suggestions_limit: usize,
    /// Field boost weights for search ranking.
    pub field_boosts: FieldBoosts,
    /// Default search persona when none is provided ("analyst", "engineer", "governance")
    pub default_persona: Option<String>,
    /// Persona-specific weighting profiles for reranking and meta boosts
    pub persona_weights: PersonaProfile<PersonaWeights>,
    /// Analyst persona multipliers for semantic metadata quality signals
    pub analyst_semantic: AnalystSemanticConfig,
    /// Multiplier applied to staging model scores (e.g., 0.6 = de-boost)
    pub staging_deboost_factor: f32,
    /// Multiplier applied when analyst candidate metadata is explicitly false
    pub analyst_candidate_false_deboost_factor: f32,
    /// Multiplier applied when engineer candidate metadata is explicitly false
    pub engineer_candidate_false_deboost_factor: f32,
    /// Multiplier applied when governance candidate metadata is explicitly false
    pub governance_candidate_false_deboost_factor: f32,
    /// Multiplier applied when query tokens match nova measures
    pub nova_measure_match_multiplier: f32,
    /// Multiplier applied when query tokens match nova metrics
    pub nova_metric_match_multiplier: f32,
    /// Persona-specific multiplier applied when Nova semantics match for analyst search
    pub analyst_nova_semantic_match_multiplier: f32,
    /// Persona-specific multiplier applied when Nova semantics match for non-analyst search
    pub non_analyst_nova_semantic_match_multiplier: f32,
    /// Multiplier applied when Nova semantic matches occur on the semantic name itself
    pub nova_semantic_name_match_multiplier: f32,
    /// Multiplier applied when Nova semantic matches occur on semantic synonyms
    pub nova_semantic_synonym_match_multiplier: f32,
    /// Multiplier applied when Nova semantic matches occur in definitions/fields/expressions
    pub nova_semantic_definition_match_multiplier: f32,
    /// Additional multiplier applied when matched Nova semantics are canonical
    pub nova_semantic_canonical_match_multiplier: f32,
    /// Bonus score added when matched Nova semantics are canonical
    pub nova_semantic_canonical_match_bonus: f32,
    /// Multiplier applied when query tokens match nova synonyms
    pub nova_synonym_match_multiplier: f32,
    /// Enable phrase-level name/synonym/indicator boosts in keyword ranking.
    pub enable_phrase_boost: bool,
    /// Multiplier applied for canonical models
    pub nova_canonical_multiplier: f32,
    /// Additional multiplier when canonical models match nova meta terms
    pub nova_canonical_match_multiplier: f32,
    /// Bonus score added when canonical models match nova meta terms
    pub nova_canonical_match_bonus: f32,
    /// Boost applied to exact matches for engineer persona
    pub engineer_exact_match_multiplier: f32,
    /// Indicator-specific ranking and grouping knobs.
    pub indicator_ranking: IndicatorRankingConfig,
    /// Metadata-support signal weights and output caps.
    pub metadata_support: MetadataSupportConfig,
    /// Default-off allowlist for selected non-Nova dbt metadata fields.
    pub extended_meta: ExtendedMetaSearchConfig,
    /// Enable hybrid search with reciprocal rank fusion
    pub enable_rrf: bool,
    /// RRF smoothing constant
    pub rrf_k: f32,
    /// Overfetch multiplier for RRF fusion
    pub rrf_overfetch: usize,
    /// Failure threshold before opening search circuit breakers
    pub search_circuit_failure_threshold: usize,
    /// Open duration for search circuit breakers (seconds)
    pub search_circuit_open_seconds: u64,
    /// Max duration for a search request (milliseconds). Use 0 to disable timeout.
    pub search_timeout_ms: usize,
    /// Max concurrent search requests (0 = unlimited)
    pub search_max_concurrent: usize,
    /// Max queued search requests when concurrency is saturated
    pub search_max_queue: usize,
    /// Enable dense vector search
    pub enable_vector_search: bool,
    /// Cold-start policy when semantic caches are missing.
    pub cold_start_policy: SearchColdStartPolicy,
    /// Embedding model name (fastembed model code or alias)
    pub embedding_model: String,
    /// Max results to return from vector search before fusion
    pub vector_top_k: usize,
    /// Max chars included in embedding text
    pub vector_max_chars: usize,
    /// Max decompressed bytes allowed for embedding caches (0 = unlimited)
    pub embeddings_max_decompressed_bytes: u64,
    /// Enable approximate nearest-neighbor search for dense vectors
    pub enable_vector_ann: bool,
    /// Enable 8-bit quantization for dense vectors to reduce memory
    pub enable_vector_quantization: bool,
    /// Number of random hyperplanes (bits) for ANN bucketing
    pub vector_ann_bits: usize,
    /// Hamming radius to probe additional ANN buckets
    pub vector_ann_hamming: usize,
    /// Max candidates to score when using ANN
    pub vector_ann_max_candidates: usize,
    /// Minimum candidates required before falling back to full scan
    pub vector_ann_min_candidates: usize,
    /// Embeddings cache directory (shared across runs)
    pub embedding_cache_dir: String,
    /// ONNX intra-thread count for vector/sparse/reranker models.
    pub onnx_threads: usize,
    /// Batch size for embedding model inference
    pub embedding_batch_size: usize,
    /// Batch size for sparse embedding model inference
    pub sparse_embedding_batch_size: usize,
    /// Enable sparse vector search (SPLADE)
    pub enable_sparse_search: bool,
    /// Max results to return from sparse search
    pub sparse_top_k: usize,
    /// Enable cross-encoder reranking
    pub enable_reranker: bool,
    /// Reranker model name (fastembed model code)
    pub reranker_model: String,
    /// Max results to rerank with cross-encoder
    pub rerank_top_n: usize,
    /// Ignore existing semantic caches and rebuild them on next load.
    #[serde(skip)]
    pub force_rebuild_semantic_caches: bool,
    /// Environment parsing errors that must surface during validation.
    #[serde(skip)]
    pub env_errors: Vec<String>,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            manifest_hash: None,
            default_limit: 50,
            max_page_size: 2000,
            max_offset: 10_000,
            lineage_cache_size: 2048,
            column_lineage_precompute: true,
            min_word_length: 2,
            max_query_length: 2000,
            max_path_pattern_length: 1000,
            index_dir: "index".to_string(),
            index_writer_heap_bytes: 128_000_000,
            max_sql_chunk_bytes: 256 * 1024,
            dedup_fetch_multiplier: 8,
            enable_ngram: true,
            ngram_min: 3,
            ngram_max: 3,
            ngram_boost: 0.35,
            fuzzy_min_length: 4,
            fuzzy_mid_length: 7,
            fuzzy_max_distance: 2,
            highlight_max_chars: 240,
            highlight_max_fields: 5,
            highlight_format: "text".to_string(),
            enable_suggestions: true,
            suggestions_limit: 7,
            field_boosts: FieldBoosts::default(),
            default_persona: None,
            persona_weights: PersonaProfile::default(),
            analyst_semantic: AnalystSemanticConfig::default(),
            staging_deboost_factor: 0.6,
            analyst_candidate_false_deboost_factor: 0.45,
            engineer_candidate_false_deboost_factor: 1.0,
            governance_candidate_false_deboost_factor: 1.0,
            nova_measure_match_multiplier: 1.15,
            nova_metric_match_multiplier: 1.2,
            analyst_nova_semantic_match_multiplier: 1.35,
            non_analyst_nova_semantic_match_multiplier: 1.05,
            nova_semantic_name_match_multiplier: 1.12,
            nova_semantic_synonym_match_multiplier: 1.08,
            nova_semantic_definition_match_multiplier: 1.03,
            nova_semantic_canonical_match_multiplier: 1.25,
            nova_semantic_canonical_match_bonus: 1.5,
            nova_synonym_match_multiplier: 1.2,
            enable_phrase_boost: true,
            nova_canonical_multiplier: 1.08,
            nova_canonical_match_multiplier: 1.35,
            nova_canonical_match_bonus: 2.5,
            engineer_exact_match_multiplier: 2.0,
            indicator_ranking: IndicatorRankingConfig::default(),
            metadata_support: MetadataSupportConfig::default(),
            extended_meta: ExtendedMetaSearchConfig::default(),
            enable_rrf: true,
            rrf_k: 60.0,
            rrf_overfetch: 3,
            search_circuit_failure_threshold: 3,
            search_circuit_open_seconds: 60,
            search_timeout_ms: 30_000,
            search_max_concurrent: 4,
            search_max_queue: 8,
            enable_vector_search: false,
            cold_start_policy: SearchColdStartPolicy::default(),
            embedding_model: "intfloat/multilingual-e5-base".to_string(),
            vector_top_k: 200,
            vector_max_chars: 4000,
            embeddings_max_decompressed_bytes: 4 * 1024 * 1024 * 1024,
            enable_vector_ann: true,
            enable_vector_quantization: false,
            vector_ann_bits: 16,
            vector_ann_hamming: 1,
            vector_ann_max_candidates: 5000,
            vector_ann_min_candidates: 200,
            embedding_cache_dir: String::new(),
            onnx_threads: env::default_onnx_threads(),
            embedding_batch_size: 128,
            sparse_embedding_batch_size: 16,
            enable_sparse_search: false,
            sparse_top_k: 200,
            enable_reranker: false,
            reranker_model: "jinaai/jina-reranker-v2-base-multilingual".to_string(),
            rerank_top_n: 20,
            force_rebuild_semantic_caches: false,
            env_errors: Vec::new(),
        }
    }
}

impl SearchConfig {
    /// Apply search configuration overrides from environment variables to this config.
    pub fn apply_env(&mut self) {
        env::apply_search_env(self);
    }

    /// Load search configuration overrides from environment variables.
    #[must_use]
    pub fn from_env() -> Self {
        let mut config = Self::default();
        config.apply_env();
        config
    }

    /// Validate search configuration.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid environment overrides or unsafe extended
    /// metadata search configuration.
    pub fn validate(&self) -> Result<()> {
        if !self.env_errors.is_empty() {
            return Err(DbtNovaError::InvalidParams(self.env_errors.join("; ")));
        }
        self.extended_meta.validate()
    }

    /// Deterministic fingerprint for search-index-affecting config.
    #[must_use]
    pub fn index_fingerprint(&self) -> String {
        if self.extended_meta.fields.is_empty() {
            return String::new();
        }

        let mut fields = self
            .extended_meta
            .fields
            .iter()
            .map(|field| {
                json!({
                    "path": field.path.trim(),
                    "alias": field.alias.trim(),
                    "mode": field.mode,
                    "boost": field.boost,
                    "summary": field.summary,
                })
            })
            .collect::<Vec<_>>();
        fields.sort_by_key(std::string::ToString::to_string);

        let payload = json!({
            "extended_meta": {
                "fields": fields,
                "max_fields": self.extended_meta.max_fields,
                "max_values_per_field": self.extended_meta.max_values_per_field,
                "max_bytes_per_value": self.extended_meta.max_bytes_per_value,
            }
        });
        blake3::hash(payload.to_string().as_bytes())
            .to_hex()
            .to_string()
    }
}

/// Semantic startup behavior when manifest-scoped caches are missing.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SearchColdStartPolicy {
    /// Skip semantic startup work and mark the component unavailable for this run.
    #[default]
    Degrade,
    /// Build missing semantic caches during startup.
    Build,
}

impl SearchColdStartPolicy {
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "degrade" => Some(Self::Degrade),
            "build" => Some(Self::Build),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests;
