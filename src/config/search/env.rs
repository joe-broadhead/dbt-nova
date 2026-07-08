use std::thread::available_parallelism;

use super::super::{env_string, parse_bool, parse_f32, parse_u64, parse_usize, set_string};
use super::{ExtendedMetaFieldConfig, PersonaWeights, SearchColdStartPolicy, SearchConfig};

pub(super) fn default_onnx_threads() -> usize {
    available_parallelism().map_or(1, |threads| threads.get().min(4))
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
    if let Some(value) = env_string("DBT_NOVA_SEARCH_COLD_START_POLICY")
        && let Some(policy) = SearchColdStartPolicy::parse(&value)
    {
        config.cold_start_policy = policy;
    }
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
    crate::env_config!(
        config,
        onnx_threads,
        "DBT_NOVA_SEARCH_ONNX_THREADS",
        parse_usize,
        |v: &usize| *v > 0
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
    let sparse_batch_override = std::env::var("DBT_NOVA_SEARCH_SPARSE_EMBEDDING_BATCH_SIZE")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0);
    let general_batch_override = std::env::var("DBT_NOVA_SEARCH_EMBEDDING_BATCH_SIZE")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0);

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
        sparse_embedding_batch_size,
        "DBT_NOVA_SEARCH_SPARSE_EMBEDDING_BATCH_SIZE",
        parse_usize,
        |v: &usize| *v > 0
    );
    if sparse_batch_override.is_none()
        && let Some(value) = general_batch_override
    {
        config.sparse_embedding_batch_size = value;
    }
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
        analyst_nova_semantic_match_multiplier,
        "DBT_NOVA_SEARCH_ANALYST_SEMANTIC_MATCH_MULTIPLIER",
        parse_f32,
        |v: &f32| *v >= 0.0
    );
    crate::env_config!(
        config,
        non_analyst_nova_semantic_match_multiplier,
        "DBT_NOVA_SEARCH_NON_ANALYST_SEMANTIC_MATCH_MULTIPLIER",
        parse_f32,
        |v: &f32| *v >= 0.0
    );
    crate::env_config!(
        config,
        nova_semantic_name_match_multiplier,
        "DBT_NOVA_SEARCH_SEMANTIC_NAME_MATCH_MULTIPLIER",
        parse_f32,
        |v: &f32| *v >= 0.0
    );
    crate::env_config!(
        config,
        nova_semantic_synonym_match_multiplier,
        "DBT_NOVA_SEARCH_SEMANTIC_SYNONYM_MATCH_MULTIPLIER",
        parse_f32,
        |v: &f32| *v >= 0.0
    );
    crate::env_config!(
        config,
        nova_semantic_definition_match_multiplier,
        "DBT_NOVA_SEARCH_SEMANTIC_DEFINITION_MATCH_MULTIPLIER",
        parse_f32,
        |v: &f32| *v >= 0.0
    );
    crate::env_config!(
        config,
        nova_semantic_canonical_match_multiplier,
        "DBT_NOVA_SEARCH_SEMANTIC_CANONICAL_MATCH_MULTIPLIER",
        parse_f32,
        |v: &f32| *v >= 0.0
    );
    crate::env_config!(
        config,
        nova_semantic_canonical_match_bonus,
        "DBT_NOVA_SEARCH_SEMANTIC_CANONICAL_MATCH_BONUS",
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

fn apply_indicator_ranking_grouping_env(config: &mut SearchConfig) {
    crate::env_config!(
        config,
        indicator_ranking.enable_parent_coherence,
        "DBT_NOVA_SEARCH_INDICATOR_ENABLE_PARENT_COHERENCE",
        parse_bool
    );
    crate::env_config!(
        config,
        indicator_ranking.parent_group_max_groups,
        "DBT_NOVA_SEARCH_INDICATOR_PARENT_GROUP_MAX_GROUPS",
        parse_usize,
        |v: &usize| *v > 0
    );
    crate::env_config!(
        config,
        indicator_ranking.parent_group_max_indicators,
        "DBT_NOVA_SEARCH_INDICATOR_PARENT_GROUP_MAX_INDICATORS",
        parse_usize,
        |v: &usize| *v > 0
    );
}

fn apply_indicator_ranking_bonus_env(config: &mut SearchConfig) {
    crate::env_config!(
        config,
        indicator_ranking.generic_label_bonus_one_token,
        "DBT_NOVA_SEARCH_INDICATOR_GENERIC_LABEL_BONUS_ONE_TOKEN",
        parse_f32,
        |v: &f32| *v >= 0.0
    );
    crate::env_config!(
        config,
        indicator_ranking.generic_label_bonus_two_tokens,
        "DBT_NOVA_SEARCH_INDICATOR_GENERIC_LABEL_BONUS_TWO_TOKENS",
        parse_f32,
        |v: &f32| *v >= 0.0
    );
    crate::env_config!(
        config,
        indicator_ranking.generic_label_bonus_three_plus_tokens,
        "DBT_NOVA_SEARCH_INDICATOR_GENERIC_LABEL_BONUS_THREE_PLUS_TOKENS",
        parse_f32,
        |v: &f32| *v >= 0.0
    );
    crate::env_config!(
        config,
        indicator_ranking.parent_synonym_bonus_one_token,
        "DBT_NOVA_SEARCH_INDICATOR_PARENT_SYNONYM_BONUS_ONE_TOKEN",
        parse_f32,
        |v: &f32| *v >= 0.0
    );
    crate::env_config!(
        config,
        indicator_ranking.parent_synonym_bonus_two_tokens,
        "DBT_NOVA_SEARCH_INDICATOR_PARENT_SYNONYM_BONUS_TWO_TOKENS",
        parse_f32,
        |v: &f32| *v >= 0.0
    );
    crate::env_config!(
        config,
        indicator_ranking.parent_synonym_bonus_three_plus_tokens,
        "DBT_NOVA_SEARCH_INDICATOR_PARENT_SYNONYM_BONUS_THREE_PLUS_TOKENS",
        parse_f32,
        |v: &f32| *v >= 0.0
    );
    crate::env_config!(
        config,
        indicator_ranking.semantic_label_precision_scale,
        "DBT_NOVA_SEARCH_INDICATOR_SEMANTIC_LABEL_PRECISION_SCALE",
        parse_f32,
        |v: &f32| *v >= 0.0
    );
    crate::env_config!(
        config,
        indicator_ranking.semantic_label_precision_canonical_bonus,
        "DBT_NOVA_SEARCH_INDICATOR_SEMANTIC_LABEL_PRECISION_CANONICAL_BONUS",
        parse_f32,
        |v: &f32| *v >= 0.0
    );
}

fn apply_indicator_ranking_coherence_env(config: &mut SearchConfig) {
    crate::env_config!(
        config,
        indicator_ranking.parent_coherence_indicator_diversity_bonus,
        "DBT_NOVA_SEARCH_INDICATOR_PARENT_COHERENCE_DIVERSITY_BONUS",
        parse_f32,
        |v: &f32| *v >= 0.0
    );
    crate::env_config!(
        config,
        indicator_ranking.parent_coherence_canonical_indicator_bonus,
        "DBT_NOVA_SEARCH_INDICATOR_PARENT_COHERENCE_CANONICAL_BONUS",
        parse_f32,
        |v: &f32| *v >= 0.0
    );
    crate::env_config!(
        config,
        indicator_ranking.parent_coherence_strong_match_bonus,
        "DBT_NOVA_SEARCH_INDICATOR_PARENT_COHERENCE_STRONG_MATCH_BONUS",
        parse_f32,
        |v: &f32| *v >= 0.0
    );
    crate::env_config!(
        config,
        indicator_ranking.parent_coherence_support_surface_bonus,
        "DBT_NOVA_SEARCH_INDICATOR_PARENT_COHERENCE_SUPPORT_SURFACE_BONUS",
        parse_f32,
        |v: &f32| *v >= 0.0
    );
    crate::env_config!(
        config,
        indicator_ranking.parent_coherence_time_field_bonus,
        "DBT_NOVA_SEARCH_INDICATOR_PARENT_COHERENCE_TIME_FIELD_BONUS",
        parse_f32,
        |v: &f32| *v >= 0.0
    );
    crate::env_config!(
        config,
        indicator_ranking.parent_coherence_dimension_bonus,
        "DBT_NOVA_SEARCH_INDICATOR_PARENT_COHERENCE_DIMENSION_BONUS",
        parse_f32,
        |v: &f32| *v >= 0.0
    );
    crate::env_config!(
        config,
        indicator_ranking.parent_coherence_max_bonus,
        "DBT_NOVA_SEARCH_INDICATOR_PARENT_COHERENCE_MAX_BONUS",
        parse_f32,
        |v: &f32| *v >= 0.0
    );
    crate::env_config!(
        config,
        indicator_ranking.search_parent_indicator_bonus_scale,
        "DBT_NOVA_SEARCH_INDICATOR_SEARCH_PARENT_BONUS_SCALE",
        parse_f32,
        |v: &f32| *v >= 0.0
    );
    crate::env_config!(
        config,
        indicator_ranking.search_missing_indicator_parent_multiplier,
        "DBT_NOVA_SEARCH_INDICATOR_SEARCH_MISSING_PARENT_MULTIPLIER",
        parse_f32,
        |v: &f32| *v >= 0.0
    );
    crate::env_config!(
        config,
        indicator_ranking.search_parent_indicator_top_k,
        "DBT_NOVA_SEARCH_INDICATOR_SEARCH_PARENT_TOP_K",
        parse_usize,
        |v: &usize| *v > 0
    );
    crate::env_config!(
        config,
        indicator_ranking.indicator_rrf_score_weight,
        "DBT_NOVA_SEARCH_INDICATOR_RRF_SCORE_WEIGHT",
        parse_f32,
        |v: &f32| *v >= 0.0
    );
    crate::env_config!(
        config,
        indicator_ranking.indicator_reranker_score_weight,
        "DBT_NOVA_SEARCH_INDICATOR_RERANKER_SCORE_WEIGHT",
        parse_f32,
        |v: &f32| *v >= 0.0
    );
}

fn apply_indicator_ranking_env(config: &mut SearchConfig) {
    apply_indicator_ranking_grouping_env(config);
    apply_indicator_ranking_bonus_env(config);
    apply_indicator_ranking_coherence_env(config);
}

fn apply_metadata_support_env(config: &mut SearchConfig) {
    crate::env_config!(
        config,
        metadata_support.parent_synonym_weight,
        "DBT_NOVA_SEARCH_METADATA_SUPPORT_PARENT_SYNONYM_WEIGHT",
        parse_f32,
        |v: &f32| *v >= 0.0
    );
    crate::env_config!(
        config,
        metadata_support.domain_weight,
        "DBT_NOVA_SEARCH_METADATA_SUPPORT_DOMAIN_WEIGHT",
        parse_f32,
        |v: &f32| *v >= 0.0
    );
    crate::env_config!(
        config,
        metadata_support.use_case_weight,
        "DBT_NOVA_SEARCH_METADATA_SUPPORT_USE_CASE_WEIGHT",
        parse_f32,
        |v: &f32| *v >= 0.0
    );
    crate::env_config!(
        config,
        metadata_support.dimension_weight,
        "DBT_NOVA_SEARCH_METADATA_SUPPORT_DIMENSION_WEIGHT",
        parse_f32,
        |v: &f32| *v >= 0.0
    );
    crate::env_config!(
        config,
        metadata_support.column_name_weight,
        "DBT_NOVA_SEARCH_METADATA_SUPPORT_COLUMN_NAME_WEIGHT",
        parse_f32,
        |v: &f32| *v >= 0.0
    );
    crate::env_config!(
        config,
        metadata_support.column_role_weight,
        "DBT_NOVA_SEARCH_METADATA_SUPPORT_COLUMN_ROLE_WEIGHT",
        parse_f32,
        |v: &f32| *v >= 0.0
    );
    crate::env_config!(
        config,
        metadata_support.semantic_type_weight,
        "DBT_NOVA_SEARCH_METADATA_SUPPORT_SEMANTIC_TYPE_WEIGHT",
        parse_f32,
        |v: &f32| *v >= 0.0
    );
    crate::env_config!(
        config,
        metadata_support.example_value_weight,
        "DBT_NOVA_SEARCH_METADATA_SUPPORT_EXAMPLE_VALUE_WEIGHT",
        parse_f32,
        |v: &f32| *v >= 0.0
    );
    crate::env_config!(
        config,
        metadata_support.max_bonus,
        "DBT_NOVA_SEARCH_METADATA_SUPPORT_MAX_BONUS",
        parse_f32,
        |v: &f32| *v >= 0.0
    );
    crate::env_config!(
        config,
        metadata_support.max_values_per_field,
        "DBT_NOVA_SEARCH_METADATA_SUPPORT_MAX_VALUES_PER_FIELD",
        parse_usize,
        |v: &usize| *v > 0
    );
}

fn apply_extended_meta_env(config: &mut SearchConfig) {
    if let Some(value) = env_string("DBT_NOVA_SEARCH_EXTENDED_META_FIELDS_JSON") {
        match serde_json::from_str::<Vec<ExtendedMetaFieldConfig>>(&value) {
            Ok(fields) => config.extended_meta.fields = fields,
            Err(err) => {
                config.env_errors.push(format!(
                    "Invalid DBT_NOVA_SEARCH_EXTENDED_META_FIELDS_JSON JSON; expected a JSON array of extended metadata field objects with mode keyword|text|string_array|bool (error: {err})"
                ));
            }
        }
    }
    crate::env_config!(
        config,
        extended_meta.max_fields,
        "DBT_NOVA_SEARCH_EXTENDED_META_MAX_FIELDS",
        parse_usize
    );
    crate::env_config!(
        config,
        extended_meta.max_values_per_field,
        "DBT_NOVA_SEARCH_EXTENDED_META_MAX_VALUES_PER_FIELD",
        parse_usize
    );
    crate::env_config!(
        config,
        extended_meta.max_bytes_per_value,
        "DBT_NOVA_SEARCH_EXTENDED_META_MAX_BYTES_PER_VALUE",
        parse_usize
    );
}

pub(super) fn apply_search_env(config: &mut SearchConfig) {
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
    apply_indicator_ranking_env(config);
    apply_metadata_support_env(config);
    apply_extended_meta_env(config);
}
