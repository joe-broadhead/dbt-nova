use serde::{Deserialize, Serialize};

use super::{env_string, parse_bool, parse_f32, parse_u64, parse_usize, set_string};

/// Weight multipliers for persona-aware ranking signals.
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(default)]
pub struct PersonaWeights {
    pub bm25: f32,
    pub ngram: f32,
    pub fuzzy: f32,
    pub vector: f32,
    pub sparse: f32,
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
    /// Multiplier applied when query tokens match nova synonyms
    pub nova_synonym_match_multiplier: f32,
    /// Multiplier applied for canonical models
    pub nova_canonical_multiplier: f32,
    /// Additional multiplier when canonical models match nova meta terms
    pub nova_canonical_match_multiplier: f32,
    /// Bonus score added when canonical models match nova meta terms
    pub nova_canonical_match_bonus: f32,
    /// Boost applied to exact matches for engineer persona
    pub engineer_exact_match_multiplier: f32,
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
    /// Batch size for embedding model inference
    pub embedding_batch_size: usize,
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
            nova_synonym_match_multiplier: 1.2,
            nova_canonical_multiplier: 1.08,
            nova_canonical_match_multiplier: 1.35,
            nova_canonical_match_bonus: 2.5,
            engineer_exact_match_multiplier: 2.0,
            enable_rrf: true,
            rrf_k: 60.0,
            rrf_overfetch: 3,
            search_circuit_failure_threshold: 3,
            search_circuit_open_seconds: 60,
            search_timeout_ms: 30_000,
            search_max_concurrent: 4,
            search_max_queue: 8,
            enable_vector_search: true,
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
            embedding_batch_size: 128,
            enable_sparse_search: true,
            sparse_top_k: 200,
            enable_reranker: true,
            reranker_model: "jinaai/jina-reranker-v2-base-multilingual".to_string(),
            rerank_top_n: 20,
        }
    }
}

impl SearchConfig {
    /// Load search configuration overrides from environment variables.
    #[must_use]
    pub fn from_env() -> Self {
        let mut config = Self::default();
        apply_search_env(&mut config);
        config
    }
}

fn apply_search_limits_env(config: &mut SearchConfig) {
    crate::env_config!(
        config,
        max_page_size,
        "DBT_NOVA_MAX_PAGE_SIZE",
        parse_usize,
        |v: &usize| *v > 0
    );
    crate::env_config!(
        config,
        max_offset,
        "DBT_NOVA_MAX_OFFSET",
        parse_usize,
        |v: &usize| *v > 0
    );
    crate::env_config!(
        config,
        lineage_cache_size,
        "DBT_NOVA_LINEAGE_CACHE_SIZE",
        parse_usize
    );
    crate::env_config!(
        config,
        column_lineage_precompute,
        "DBT_NOVA_COLUMN_LINEAGE_PRECOMPUTE",
        parse_bool
    );
    crate::env_config!(
        config,
        default_limit,
        "DBT_NOVA_DEFAULT_LIMIT",
        parse_usize,
        |v: &usize| *v > 0
    );
    crate::env_config!(
        config,
        min_word_length,
        "DBT_NOVA_MIN_WORD_LENGTH",
        parse_usize,
        |v: &usize| *v > 0
    );
    crate::env_config!(
        config,
        max_query_length,
        "DBT_NOVA_MAX_QUERY_LENGTH",
        parse_usize,
        |v: &usize| *v > 0
    );
    crate::env_config!(
        config,
        max_path_pattern_length,
        "DBT_NOVA_MAX_PATH_PATTERN_LENGTH",
        parse_usize,
        |v: &usize| *v > 0
    );
}

fn apply_search_index_env(config: &mut SearchConfig) {
    set_string("DBT_NOVA_INDEX_DIR", &mut config.index_dir);
    crate::env_config!(
        config,
        index_writer_heap_bytes,
        "DBT_NOVA_INDEX_WRITER_HEAP_BYTES",
        parse_usize,
        |v: &usize| *v > 0
    );
    crate::env_config!(
        config,
        max_sql_chunk_bytes,
        "DBT_NOVA_MAX_SQL_CHUNK_BYTES",
        parse_usize,
        |v: &usize| *v > 0
    );
    crate::env_config!(
        config,
        dedup_fetch_multiplier,
        "DBT_NOVA_SEARCH_DEDUP_FETCH_MULTIPLIER",
        parse_usize,
        |v: &usize| *v > 0
    );
}

fn apply_text_match_env(config: &mut SearchConfig) {
    crate::env_config!(
        config,
        enable_ngram,
        "DBT_NOVA_SEARCH_ENABLE_NGRAM",
        parse_bool
    );
    crate::env_config!(
        config,
        ngram_min,
        "DBT_NOVA_SEARCH_NGRAM_MIN",
        parse_usize,
        |v: &usize| *v > 0
    );
    crate::env_config!(
        config,
        ngram_max,
        "DBT_NOVA_SEARCH_NGRAM_MAX",
        parse_usize,
        |v: &usize| *v > 0
    );
    crate::env_config!(
        config,
        ngram_boost,
        "DBT_NOVA_SEARCH_NGRAM_BOOST",
        parse_f32,
        |v: &f32| *v >= 0.0
    );
    crate::env_config!(
        config,
        fuzzy_min_length,
        "DBT_NOVA_FUZZY_MIN_LENGTH",
        parse_usize,
        |v: &usize| *v > 0
    );
    crate::env_config!(
        config,
        fuzzy_mid_length,
        "DBT_NOVA_FUZZY_MID_LENGTH",
        parse_usize,
        |v: &usize| *v > 0
    );
    crate::env_config!(
        config,
        fuzzy_max_distance,
        "DBT_NOVA_FUZZY_MAX_DISTANCE",
        parse_usize,
        |v: &usize| *v > 0
    );
}

fn apply_highlight_format_env(config: &mut SearchConfig) {
    if let Some(value) = env_string("DBT_NOVA_SEARCH_HIGHLIGHT_FORMAT") {
        let normalized = value.trim().to_lowercase();
        if matches!(normalized.as_str(), "html" | "text" | "plain") {
            config.highlight_format = if normalized == "plain" {
                "text".to_string()
            } else {
                normalized
            };
        }
    }
}

fn apply_highlight_and_suggestion_env(config: &mut SearchConfig) {
    crate::env_config!(
        config,
        highlight_max_chars,
        "DBT_NOVA_SEARCH_HIGHLIGHT_MAX_CHARS",
        parse_usize,
        |v: &usize| *v > 0
    );
    crate::env_config!(
        config,
        highlight_max_fields,
        "DBT_NOVA_SEARCH_HIGHLIGHT_MAX_FIELDS",
        parse_usize
    );
    apply_highlight_format_env(config);
    crate::env_config!(
        config,
        enable_suggestions,
        "DBT_NOVA_SEARCH_ENABLE_SUGGESTIONS",
        parse_bool
    );
    crate::env_config!(
        config,
        suggestions_limit,
        "DBT_NOVA_SEARCH_SUGGESTIONS_LIMIT",
        parse_usize,
        |v: &usize| *v > 0
    );
}

fn apply_fusion_and_runtime_env(config: &mut SearchConfig) {
    crate::env_config!(config, enable_rrf, "DBT_NOVA_SEARCH_ENABLE_RRF", parse_bool);
    crate::env_config!(
        config,
        rrf_k,
        "DBT_NOVA_SEARCH_RRF_K",
        parse_f32,
        |v: &f32| *v > 0.0
    );
    crate::env_config!(
        config,
        rrf_overfetch,
        "DBT_NOVA_SEARCH_RRF_OVERFETCH",
        parse_usize,
        |v: &usize| *v > 0
    );
    crate::env_config!(
        config,
        search_circuit_failure_threshold,
        "DBT_NOVA_SEARCH_CIRCUIT_FAILURE_THRESHOLD",
        parse_usize,
        |v: &usize| *v > 0
    );
    crate::env_config!(
        config,
        search_circuit_open_seconds,
        "DBT_NOVA_SEARCH_CIRCUIT_OPEN_SECONDS",
        parse_u64,
        |v: &u64| *v > 0
    );
    crate::env_config!(
        config,
        search_timeout_ms,
        "DBT_NOVA_SEARCH_TIMEOUT_MS",
        parse_usize
    );
    crate::env_config!(
        config,
        search_max_concurrent,
        "DBT_NOVA_SEARCH_MAX_CONCURRENT",
        parse_usize
    );
    crate::env_config!(
        config,
        search_max_queue,
        "DBT_NOVA_SEARCH_MAX_QUEUE",
        parse_usize
    );
}

fn apply_vector_env(config: &mut SearchConfig) {
    crate::env_config!(
        config,
        enable_vector_search,
        "DBT_NOVA_SEARCH_ENABLE_VECTOR",
        parse_bool
    );
    crate::env_config!(
        config,
        vector_top_k,
        "DBT_NOVA_SEARCH_VECTOR_TOP_K",
        parse_usize,
        |v: &usize| *v > 0
    );
    crate::env_config!(
        config,
        vector_max_chars,
        "DBT_NOVA_SEARCH_VECTOR_MAX_CHARS",
        parse_usize,
        |v: &usize| *v > 0
    );
    crate::env_config!(
        config,
        embeddings_max_decompressed_bytes,
        "DBT_NOVA_EMBEDDINGS_MAX_DECOMPRESSED_BYTES",
        parse_u64,
        |v: &u64| *v > 0
    );
    crate::env_config!(
        config,
        enable_vector_ann,
        "DBT_NOVA_SEARCH_ENABLE_VECTOR_ANN",
        parse_bool
    );
    crate::env_config!(
        config,
        enable_vector_quantization,
        "DBT_NOVA_SEARCH_ENABLE_VECTOR_QUANTIZATION",
        parse_bool
    );
    crate::env_config!(
        config,
        vector_ann_bits,
        "DBT_NOVA_SEARCH_VECTOR_ANN_BITS",
        parse_usize,
        |v: &usize| *v > 0
    );
    crate::env_config!(
        config,
        vector_ann_hamming,
        "DBT_NOVA_SEARCH_VECTOR_ANN_HAMMING",
        parse_usize
    );
    crate::env_config!(
        config,
        vector_ann_max_candidates,
        "DBT_NOVA_SEARCH_VECTOR_ANN_MAX_CANDIDATES",
        parse_usize,
        |v: &usize| *v > 0
    );
    crate::env_config!(
        config,
        vector_ann_min_candidates,
        "DBT_NOVA_SEARCH_VECTOR_ANN_MIN_CANDIDATES",
        parse_usize,
        |v: &usize| *v > 0
    );
    set_string(
        "DBT_NOVA_EMBEDDINGS_CACHE_DIR",
        &mut config.embedding_cache_dir,
    );
    set_string("DBT_NOVA_EMBEDDING_MODEL", &mut config.embedding_model);
    crate::env_config!(
        config,
        embedding_batch_size,
        "DBT_NOVA_SEARCH_EMBEDDING_BATCH_SIZE",
        parse_usize,
        |v: &usize| *v > 0
    );
}

fn apply_sparse_and_reranker_env(config: &mut SearchConfig) {
    crate::env_config!(
        config,
        enable_sparse_search,
        "DBT_NOVA_SEARCH_ENABLE_SPARSE",
        parse_bool
    );
    crate::env_config!(
        config,
        sparse_top_k,
        "DBT_NOVA_SEARCH_SPARSE_TOP_K",
        parse_usize,
        |v: &usize| *v > 0
    );
    crate::env_config!(
        config,
        enable_reranker,
        "DBT_NOVA_SEARCH_ENABLE_RERANKER",
        parse_bool
    );
    set_string("DBT_NOVA_RERANKER_MODEL", &mut config.reranker_model);
    crate::env_config!(
        config,
        rerank_top_n,
        "DBT_NOVA_SEARCH_RERANK_TOP_N",
        parse_usize,
        |v: &usize| *v > 0
    );
}

fn apply_persona_weights_env(var: &str, target: &mut PersonaWeights) {
    if let Some(value) = env_string(var) {
        target.apply_overrides(&value);
    }
}

fn apply_persona_env(config: &mut SearchConfig) {
    if let Some(value) = env_string("DBT_NOVA_SEARCH_DEFAULT_PERSONA") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            config.default_persona = Some(trimmed.to_string());
        }
    }
    apply_persona_weights_env(
        "DBT_NOVA_SEARCH_PERSONA_ANALYST_WEIGHTS",
        &mut config.persona_weights.analyst,
    );
    apply_persona_weights_env(
        "DBT_NOVA_SEARCH_PERSONA_ENGINEER_WEIGHTS",
        &mut config.persona_weights.engineer,
    );
    apply_persona_weights_env(
        "DBT_NOVA_SEARCH_PERSONA_GOVERNANCE_WEIGHTS",
        &mut config.persona_weights.governance,
    );
    apply_persona_weights_env(
        "DBT_NOVA_SEARCH_PERSONA_DEFAULT_WEIGHTS",
        &mut config.persona_weights.default,
    );
}

fn apply_field_boosts_env(config: &mut SearchConfig) {
    apply_core_field_boosts_env(config);
    apply_meta_field_boosts_env(config);
}

fn apply_core_field_boosts_env(config: &mut SearchConfig) {
    crate::env_config!(
        config,
        field_boosts.alias,
        "DBT_NOVA_ALIAS_BOOST",
        parse_f32,
        |v: &f32| *v >= 0.0
    );
    crate::env_config!(
        config,
        field_boosts.name,
        "DBT_NOVA_NAME_BOOST",
        parse_f32,
        |v: &f32| *v >= 0.0
    );
    crate::env_config!(
        config,
        field_boosts.description,
        "DBT_NOVA_DESCRIPTION_BOOST",
        parse_f32,
        |v: &f32| *v >= 0.0
    );
    crate::env_config!(
        config,
        field_boosts.column,
        "DBT_NOVA_COLUMN_BOOST",
        parse_f32,
        |v: &f32| *v >= 0.0
    );
    crate::env_config!(
        config,
        field_boosts.tag,
        "DBT_NOVA_TAG_BOOST",
        parse_f32,
        |v: &f32| *v >= 0.0
    );
    crate::env_config!(
        config,
        field_boosts.path,
        "DBT_NOVA_PATH_BOOST",
        parse_f32,
        |v: &f32| *v >= 0.0
    );
    crate::env_config!(
        config,
        field_boosts.code,
        "DBT_NOVA_CODE_BOOST",
        parse_f32,
        |v: &f32| *v >= 0.0
    );
}

fn apply_meta_field_boosts_env(config: &mut SearchConfig) {
    crate::env_config!(
        config,
        field_boosts.nova_synonyms,
        "DBT_NOVA_META_SYNONYMS_BOOST",
        parse_f32,
        |v: &f32| *v >= 0.0
    );
    crate::env_config!(
        config,
        field_boosts.nova_domains,
        "DBT_NOVA_META_DOMAINS_BOOST",
        parse_f32,
        |v: &f32| *v >= 0.0
    );
    crate::env_config!(
        config,
        field_boosts.nova_use_cases,
        "DBT_NOVA_META_USE_CASES_BOOST",
        parse_f32,
        |v: &f32| *v >= 0.0
    );
    crate::env_config!(
        config,
        field_boosts.nova_measures,
        "DBT_NOVA_META_MEASURES_BOOST",
        parse_f32,
        |v: &f32| *v >= 0.0
    );
    crate::env_config!(
        config,
        field_boosts.nova_metric,
        "DBT_NOVA_META_METRIC_BOOST",
        parse_f32,
        |v: &f32| *v >= 0.0
    );
    crate::env_config!(
        config,
        field_boosts.nova_sensitivity,
        "DBT_NOVA_META_SENSITIVITY_BOOST",
        parse_f32,
        |v: &f32| *v >= 0.0
    );
    crate::env_config!(
        config,
        field_boosts.nova_pii,
        "DBT_NOVA_META_PII_BOOST",
        parse_f32,
        |v: &f32| *v >= 0.0
    );
    crate::env_config!(
        config,
        field_boosts.nova_compliance,
        "DBT_NOVA_META_COMPLIANCE_BOOST",
        parse_f32,
        |v: &f32| *v >= 0.0
    );
}

fn apply_semantic_scoring_env(config: &mut SearchConfig) {
    crate::env_config!(
        config,
        staging_deboost_factor,
        "DBT_NOVA_SEARCH_STAGING_DEBOOST_FACTOR",
        parse_f32,
        |v: &f32| *v >= 0.0
    );
    crate::env_config!(
        config,
        analyst_candidate_false_deboost_factor,
        "DBT_NOVA_SEARCH_ANALYST_CANDIDATE_FALSE_DEBOOST_FACTOR",
        parse_f32,
        |v: &f32| *v >= 0.0
    );
    crate::env_config!(
        config,
        engineer_candidate_false_deboost_factor,
        "DBT_NOVA_SEARCH_ENGINEER_CANDIDATE_FALSE_DEBOOST_FACTOR",
        parse_f32,
        |v: &f32| *v >= 0.0
    );
    crate::env_config!(
        config,
        governance_candidate_false_deboost_factor,
        "DBT_NOVA_SEARCH_GOVERNANCE_CANDIDATE_FALSE_DEBOOST_FACTOR",
        parse_f32,
        |v: &f32| *v >= 0.0
    );
    apply_analyst_semantic_env(config);
    apply_nova_semantic_env(config);
}

fn apply_analyst_semantic_env(config: &mut SearchConfig) {
    crate::env_config!(
        config,
        analyst_semantic.metric_definition_multiplier,
        "DBT_NOVA_SEARCH_ANALYST_METRIC_DEF_MULTIPLIER",
        parse_f32,
        |v: &f32| *v > 0.0
    );
    crate::env_config!(
        config,
        analyst_semantic.measure_definition_multiplier,
        "DBT_NOVA_SEARCH_ANALYST_MEASURE_DEF_MULTIPLIER",
        parse_f32,
        |v: &f32| *v > 0.0
    );
    crate::env_config!(
        config,
        analyst_semantic.grain_multiplier,
        "DBT_NOVA_SEARCH_ANALYST_GRAIN_MULTIPLIER",
        parse_f32,
        |v: &f32| *v > 0.0
    );
    crate::env_config!(
        config,
        analyst_semantic.time_field_multiplier,
        "DBT_NOVA_SEARCH_ANALYST_TIME_FIELD_MULTIPLIER",
        parse_f32,
        |v: &f32| *v > 0.0
    );
    crate::env_config!(
        config,
        analyst_semantic.dimension_overlap_one_multiplier,
        "DBT_NOVA_SEARCH_ANALYST_DIM_OVERLAP_ONE_MULTIPLIER",
        parse_f32,
        |v: &f32| *v > 0.0
    );
    crate::env_config!(
        config,
        analyst_semantic.dimension_overlap_two_multiplier,
        "DBT_NOVA_SEARCH_ANALYST_DIM_OVERLAP_TWO_MULTIPLIER",
        parse_f32,
        |v: &f32| *v > 0.0
    );
    crate::env_config!(
        config,
        analyst_semantic.dimension_overlap_three_plus_multiplier,
        "DBT_NOVA_SEARCH_ANALYST_DIM_OVERLAP_THREE_PLUS_MULTIPLIER",
        parse_f32,
        |v: &f32| *v > 0.0
    );
    crate::env_config!(
        config,
        analyst_semantic.missing_metric_or_measure_multiplier,
        "DBT_NOVA_SEARCH_ANALYST_MISSING_METRIC_MEASURE_MULTIPLIER",
        parse_f32,
        |v: &f32| *v > 0.0
    );
    crate::env_config!(
        config,
        analyst_semantic.missing_grain_multiplier,
        "DBT_NOVA_SEARCH_ANALYST_MISSING_GRAIN_MULTIPLIER",
        parse_f32,
        |v: &f32| *v > 0.0
    );
    crate::env_config!(
        config,
        analyst_semantic.min_multiplier,
        "DBT_NOVA_SEARCH_ANALYST_MIN_MULTIPLIER",
        parse_f32,
        |v: &f32| *v > 0.0
    );
    crate::env_config!(
        config,
        analyst_semantic.max_multiplier,
        "DBT_NOVA_SEARCH_ANALYST_MAX_MULTIPLIER",
        parse_f32,
        |v: &f32| *v > 0.0
    );
}

fn apply_nova_semantic_env(config: &mut SearchConfig) {
    crate::env_config!(
        config,
        nova_measure_match_multiplier,
        "DBT_NOVA_SEARCH_MEASURE_MATCH_MULTIPLIER",
        parse_f32,
        |v: &f32| *v >= 0.0
    );
    crate::env_config!(
        config,
        nova_metric_match_multiplier,
        "DBT_NOVA_SEARCH_METRIC_MATCH_MULTIPLIER",
        parse_f32,
        |v: &f32| *v >= 0.0
    );
    crate::env_config!(
        config,
        nova_synonym_match_multiplier,
        "DBT_NOVA_SEARCH_SYNONYM_MATCH_MULTIPLIER",
        parse_f32,
        |v: &f32| *v >= 0.0
    );
    crate::env_config!(
        config,
        nova_canonical_multiplier,
        "DBT_NOVA_SEARCH_CANONICAL_MATCH_MULTIPLIER",
        parse_f32,
        |v: &f32| *v >= 0.0
    );
    crate::env_config!(
        config,
        nova_canonical_match_multiplier,
        "DBT_NOVA_SEARCH_CANONICAL_META_MATCH_MULTIPLIER",
        parse_f32,
        |v: &f32| *v >= 0.0
    );
    crate::env_config!(
        config,
        nova_canonical_match_bonus,
        "DBT_NOVA_SEARCH_CANONICAL_META_MATCH_BONUS",
        parse_f32,
        |v: &f32| *v >= 0.0
    );
    crate::env_config!(
        config,
        engineer_exact_match_multiplier,
        "DBT_NOVA_SEARCH_ENGINEER_EXACT_MATCH_MULTIPLIER",
        parse_f32,
        |v: &f32| *v >= 0.0
    );
}

fn apply_search_env(config: &mut SearchConfig) {
    apply_search_limits_env(config);
    apply_search_index_env(config);
    apply_text_match_env(config);
    apply_highlight_and_suggestion_env(config);
    apply_fusion_and_runtime_env(config);
    apply_vector_env(config);
    apply_sparse_and_reranker_env(config);
    apply_persona_env(config);
    apply_field_boosts_env(config);
    apply_semantic_scoring_env(config);
}
