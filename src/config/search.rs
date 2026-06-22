use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeSet;
use std::thread::available_parallelism;

use super::{env_string, parse_bool, parse_f32, parse_u64, parse_usize, set_string};
use crate::error::{DbtNovaError, Result};

const EXTENDED_META_DEFAULT_MAX_FIELDS: usize = 32;
const EXTENDED_META_HARD_MAX_FIELDS: usize = 128;
const EXTENDED_META_DEFAULT_MAX_VALUES_PER_FIELD: usize = 64;
const EXTENDED_META_HARD_MAX_VALUES_PER_FIELD: usize = 1024;
const EXTENDED_META_DEFAULT_MAX_BYTES_PER_VALUE: usize = 4096;
const EXTENDED_META_HARD_MAX_BYTES_PER_VALUE: usize = 65_536;

/// Supported value treatment for allowlisted non-Nova dbt metadata fields.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ExtendedMetaFieldMode {
    /// Index scalar metadata as exact/filterable keywords.
    #[default]
    Keyword,
    /// Index scalar metadata as full text.
    Text,
    /// Index string arrays as repeated keyword values.
    StringArray,
    /// Index boolean metadata values.
    Bool,
}

/// One allowlisted non-Nova dbt metadata path for future extended-meta indexing.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct ExtendedMetaFieldConfig {
    /// Logical dbt metadata path, such as `meta.owner` or `columns.*.meta.semantic_group`.
    pub path: String,
    /// Stable search alias used for future fielded search and summaries.
    pub alias: String,
    /// Value mode for this path.
    pub mode: ExtendedMetaFieldMode,
    /// Optional ranking boost applied by later indexing work.
    pub boost: f32,
    /// Whether this field is eligible for future summary payloads.
    pub summary: bool,
}

impl Default for ExtendedMetaFieldConfig {
    fn default() -> Self {
        Self {
            path: String::new(),
            alias: String::new(),
            mode: ExtendedMetaFieldMode::default(),
            boost: 1.0,
            summary: false,
        }
    }
}

/// Default-off extended metadata search configuration.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct ExtendedMetaSearchConfig {
    /// Explicit allowlist of non-Nova dbt metadata fields.
    pub fields: Vec<ExtendedMetaFieldConfig>,
    /// Maximum configured fields accepted.
    pub max_fields: usize,
    /// Maximum values indexed per configured field.
    pub max_values_per_field: usize,
    /// Maximum UTF-8 bytes retained per value.
    pub max_bytes_per_value: usize,
}

impl Default for ExtendedMetaSearchConfig {
    fn default() -> Self {
        Self {
            fields: Vec::new(),
            max_fields: EXTENDED_META_DEFAULT_MAX_FIELDS,
            max_values_per_field: EXTENDED_META_DEFAULT_MAX_VALUES_PER_FIELD,
            max_bytes_per_value: EXTENDED_META_DEFAULT_MAX_BYTES_PER_VALUE,
        }
    }
}

impl ExtendedMetaSearchConfig {
    /// Validate configured extended metadata paths and caps.
    ///
    /// # Errors
    ///
    /// Returns an error when the allowlist contains unsafe paths, duplicate
    /// aliases, unsupported caps, or values that cannot be indexed safely.
    pub fn validate(&self) -> Result<()> {
        validate_bounded_usize(
            "search.extended_meta.max_fields",
            self.max_fields,
            EXTENDED_META_HARD_MAX_FIELDS,
        )?;
        validate_bounded_usize(
            "search.extended_meta.max_values_per_field",
            self.max_values_per_field,
            EXTENDED_META_HARD_MAX_VALUES_PER_FIELD,
        )?;
        validate_bounded_usize(
            "search.extended_meta.max_bytes_per_value",
            self.max_bytes_per_value,
            EXTENDED_META_HARD_MAX_BYTES_PER_VALUE,
        )?;

        if self.fields.len() > self.max_fields {
            return Err(DbtNovaError::InvalidParams(format!(
                "search.extended_meta.fields configures {} fields but max_fields is {}",
                self.fields.len(),
                self.max_fields
            )));
        }

        let mut aliases = BTreeSet::new();
        let mut paths = BTreeSet::new();
        for (index, field) in self.fields.iter().enumerate() {
            validate_extended_meta_path(index, &field.path)?;
            let alias = validate_extended_meta_alias(index, &field.alias)?;
            if !aliases.insert(alias.to_string()) {
                return Err(DbtNovaError::InvalidParams(format!(
                    "search.extended_meta.fields[{index}].alias '{alias}' is configured more than once"
                )));
            }

            let path = field.path.trim();
            if !paths.insert(path.to_string()) {
                return Err(DbtNovaError::InvalidParams(format!(
                    "search.extended_meta.fields[{index}].path '{path}' is configured more than once"
                )));
            }

            if !field.boost.is_finite() || field.boost < 0.0 {
                return Err(DbtNovaError::InvalidParams(format!(
                    "search.extended_meta.fields[{index}].boost must be a finite number greater than or equal to 0"
                )));
            }
        }

        Ok(())
    }
}

fn validate_bounded_usize(name: &str, value: usize, hard_max: usize) -> Result<()> {
    if value == 0 || value > hard_max {
        return Err(DbtNovaError::InvalidParams(format!(
            "{name} must be between 1 and {hard_max}"
        )));
    }
    Ok(())
}

fn validate_extended_meta_alias(index: usize, alias: &str) -> Result<&str> {
    let alias = alias.trim();
    if alias.is_empty() {
        return Err(DbtNovaError::InvalidParams(format!(
            "search.extended_meta.fields[{index}].alias is required"
        )));
    }
    let mut chars = alias.chars();
    let Some(first) = chars.next() else {
        return Err(DbtNovaError::InvalidParams(format!(
            "search.extended_meta.fields[{index}].alias is required"
        )));
    };
    if !first.is_ascii_lowercase() {
        return Err(DbtNovaError::InvalidParams(format!(
            "search.extended_meta.fields[{index}].alias '{alias}' must start with a lowercase ASCII letter"
        )));
    }
    if !chars.all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_') {
        return Err(DbtNovaError::InvalidParams(format!(
            "search.extended_meta.fields[{index}].alias '{alias}' must contain only lowercase ASCII letters, digits, and underscores"
        )));
    }

    Ok(alias)
}

fn validate_extended_meta_path(index: usize, path: &str) -> Result<()> {
    let path = path.trim();
    if path.is_empty() {
        return Err(DbtNovaError::InvalidParams(format!(
            "search.extended_meta.fields[{index}].path is required"
        )));
    }

    let segments = path.split('.').collect::<Vec<_>>();
    if segments.iter().any(|segment| segment.trim().is_empty()) {
        return Err(DbtNovaError::InvalidParams(format!(
            "search.extended_meta.fields[{index}].path '{path}' contains an empty segment"
        )));
    }
    for (segment_index, segment) in segments.iter().enumerate() {
        let valid_segment = *segment == "*"
            || segment
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-');
        if !valid_segment {
            return Err(DbtNovaError::InvalidParams(format!(
                "search.extended_meta.fields[{index}].path '{path}' must use dot-separated ASCII key names"
            )));
        }
        if *segment == "*" && !(segment_index == 1 && segments.first() == Some(&"columns")) {
            return Err(DbtNovaError::InvalidParams(format!(
                "search.extended_meta.fields[{index}].path '{path}' may only use '*' in 'columns.*.meta.' paths"
            )));
        }
        if let Some(sensitive) = contains_sensitive_segment(segment) {
            return Err(DbtNovaError::InvalidParams(format!(
                "search.extended_meta.fields[{index}].path '{path}' is not allowed because segment '{segment}' matches sensitive key '{sensitive}'"
            )));
        }
    }

    if !is_supported_extended_meta_path(&segments) {
        return Err(DbtNovaError::InvalidParams(format!(
            "search.extended_meta.fields[{index}].path '{path}' must start with 'meta.' or 'columns.*.meta.'"
        )));
    }
    if is_nova_meta_path(&segments) {
        return Err(DbtNovaError::InvalidParams(format!(
            "search.extended_meta.fields[{index}].path '{path}' targets meta.nova, which is already indexed by Nova"
        )));
    }

    Ok(())
}

fn is_supported_extended_meta_path(segments: &[&str]) -> bool {
    if segments.len() >= 2 && segments[0] == "meta" {
        return true;
    }
    segments.len() >= 4 && segments[0] == "columns" && segments[1] == "*" && segments[2] == "meta"
}

fn is_nova_meta_path(segments: &[&str]) -> bool {
    (segments.len() >= 2 && segments[0] == "meta" && segments[1] == "nova")
        || (segments.len() >= 4
            && segments[0] == "columns"
            && segments[1] == "*"
            && segments[2] == "meta"
            && segments[3] == "nova")
}

fn contains_sensitive_segment(segment: &str) -> Option<&'static str> {
    let normalized = normalize_sensitive_segment(segment);
    if normalized.contains("private_key") || normalized.contains("privatekey") {
        return Some("private_key");
    }
    if normalized.contains("api_key") || normalized.contains("apikey") {
        return Some("api_key");
    }

    for token in normalized.split('_') {
        match token {
            "token" => return Some("token"),
            "secret" => return Some("secret"),
            "password" => return Some("password"),
            "credential" | "credentials" => return Some("credential"),
            _ => {}
        }
    }
    None
}

fn normalize_sensitive_segment(segment: &str) -> String {
    let mut normalized = String::with_capacity(segment.len());
    let mut previous_was_separator = true;
    for ch in segment.chars() {
        if ch.is_ascii_uppercase() {
            if !previous_was_separator {
                normalized.push('_');
            }
            normalized.push(ch.to_ascii_lowercase());
            previous_was_separator = false;
        } else if ch.is_ascii_alphanumeric() {
            normalized.push(ch.to_ascii_lowercase());
            previous_was_separator = false;
        } else if !previous_was_separator {
            normalized.push('_');
            previous_was_separator = true;
        }
    }
    normalized.trim_matches('_').to_string()
}

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
            onnx_threads: default_onnx_threads(),
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
    /// Load search configuration overrides from environment variables.
    #[must_use]
    pub fn from_env() -> Self {
        let mut config = Self::default();
        apply_search_env(&mut config);
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

fn default_onnx_threads() -> usize {
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
    apply_indicator_ranking_env(config);
    apply_metadata_support_env(config);
    apply_extended_meta_env(config);
}

#[cfg(test)]
mod tests {
    use super::{
        ExtendedMetaFieldConfig, ExtendedMetaFieldMode, ExtendedMetaSearchConfig, SearchConfig,
    };
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn with_env_vars<R>(vars: &[(&str, Option<&str>)], run: impl FnOnce() -> R) -> R {
        let previous = vars
            .iter()
            .map(|(key, _)| (*key, std::env::var(key).ok()))
            .collect::<Vec<_>>();
        for (key, value) in vars {
            match value {
                Some(value) => {
                    // SAFETY: callers serialize environment mutation with `ENV_LOCK`.
                    unsafe { std::env::set_var(key, value) };
                }
                None => {
                    // SAFETY: callers serialize environment mutation with `ENV_LOCK`.
                    unsafe { std::env::remove_var(key) };
                }
            }
        }

        let result = run();

        for (key, value) in previous {
            match value {
                Some(value) => {
                    // SAFETY: callers serialize environment mutation with `ENV_LOCK`.
                    unsafe { std::env::set_var(key, value) };
                }
                None => {
                    // SAFETY: callers serialize environment mutation with `ENV_LOCK`.
                    unsafe { std::env::remove_var(key) };
                }
            }
        }

        result
    }

    #[test]
    fn default_sparse_batch_size_is_smaller_than_dense_batch_size() {
        let config = SearchConfig::default();
        assert_eq!(config.embedding_batch_size, 128);
        assert_eq!(config.sparse_embedding_batch_size, 16);
    }

    #[test]
    fn semantic_components_are_disabled_by_default() {
        let config = SearchConfig::default();
        assert!(!config.enable_vector_search);
        assert!(!config.enable_sparse_search);
        assert!(!config.enable_reranker);
    }

    #[test]
    fn sparse_batch_size_falls_back_to_general_embedding_batch_size_override() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let vars = [
            ("DBT_NOVA_SEARCH_EMBEDDING_BATCH_SIZE", Some("24")),
            ("DBT_NOVA_SEARCH_SPARSE_EMBEDDING_BATCH_SIZE", None),
        ];
        let previous = vars.map(|(key, _)| (key, std::env::var(key).ok()));
        for (key, value) in vars {
            match value {
                Some(value) => {
                    // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                    unsafe { std::env::set_var(key, value) };
                }
                None => {
                    // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                    unsafe { std::env::remove_var(key) };
                }
            }
        }

        let config = SearchConfig::from_env();

        for (key, value) in previous {
            match value {
                Some(value) => {
                    // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                    unsafe { std::env::set_var(key, value) };
                }
                None => {
                    // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                    unsafe { std::env::remove_var(key) };
                }
            }
        }

        assert_eq!(config.embedding_batch_size, 24);
        assert_eq!(config.sparse_embedding_batch_size, 24);
    }

    #[test]
    fn sparse_specific_batch_size_override_wins_over_general_override() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let vars = [
            ("DBT_NOVA_SEARCH_EMBEDDING_BATCH_SIZE", Some("24")),
            ("DBT_NOVA_SEARCH_SPARSE_EMBEDDING_BATCH_SIZE", Some("12")),
        ];
        let previous = vars.map(|(key, _)| (key, std::env::var(key).ok()));
        for (key, value) in vars {
            match value {
                Some(value) => {
                    // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                    unsafe { std::env::set_var(key, value) };
                }
                None => {
                    // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                    unsafe { std::env::remove_var(key) };
                }
            }
        }

        let config = SearchConfig::from_env();

        for (key, value) in previous {
            match value {
                Some(value) => {
                    // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                    unsafe { std::env::set_var(key, value) };
                }
                None => {
                    // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                    unsafe { std::env::remove_var(key) };
                }
            }
        }

        assert_eq!(config.embedding_batch_size, 24);
        assert_eq!(config.sparse_embedding_batch_size, 12);
    }

    #[test]
    fn indicator_ranking_env_overrides_apply() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let vars = [
            (
                "DBT_NOVA_SEARCH_INDICATOR_PARENT_GROUP_MAX_GROUPS",
                Some("7"),
            ),
            (
                "DBT_NOVA_SEARCH_INDICATOR_RERANKER_SCORE_WEIGHT",
                Some("0.25"),
            ),
        ];
        let previous = vars.map(|(key, _)| (key, std::env::var(key).ok()));
        for (key, value) in vars {
            match value {
                Some(value) => {
                    // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                    unsafe { std::env::set_var(key, value) };
                }
                None => {
                    // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                    unsafe { std::env::remove_var(key) };
                }
            }
        }

        let config = SearchConfig::from_env();

        for (key, value) in previous {
            match value {
                Some(value) => {
                    // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                    unsafe { std::env::set_var(key, value) };
                }
                None => {
                    // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                    unsafe { std::env::remove_var(key) };
                }
            }
        }

        assert_eq!(config.indicator_ranking.parent_group_max_groups, 7);
        assert!(
            (config.indicator_ranking.indicator_reranker_score_weight - 0.25).abs() < f32::EPSILON
        );
    }

    #[test]
    fn metadata_support_env_overrides_apply() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let vars = [
            (
                "DBT_NOVA_SEARCH_METADATA_SUPPORT_MAX_VALUES_PER_FIELD",
                Some("6"),
            ),
            (
                "DBT_NOVA_SEARCH_METADATA_SUPPORT_EXAMPLE_VALUE_WEIGHT",
                Some("0.75"),
            ),
        ];
        let previous = vars.map(|(key, _)| (key, std::env::var(key).ok()));
        for (key, value) in vars {
            match value {
                Some(value) => {
                    // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                    unsafe { std::env::set_var(key, value) };
                }
                None => {
                    // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                    unsafe { std::env::remove_var(key) };
                }
            }
        }

        let config = SearchConfig::from_env();

        for (key, value) in previous {
            match value {
                Some(value) => {
                    // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                    unsafe { std::env::set_var(key, value) };
                }
                None => {
                    // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                    unsafe { std::env::remove_var(key) };
                }
            }
        }

        assert_eq!(config.metadata_support.max_values_per_field, 6);
        assert!((config.metadata_support.example_value_weight - 0.75).abs() < f32::EPSILON);
    }

    #[test]
    fn extended_meta_is_default_off() {
        let config = SearchConfig::default();
        assert!(config.extended_meta.fields.is_empty());
        assert_eq!(config.index_fingerprint(), "");
        config.validate().expect("default config should validate");
    }

    #[test]
    fn extended_meta_env_overrides_apply() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let fields = r#"[
            {
                "path": "meta.owner",
                "alias": "owner",
                "mode": "keyword",
                "boost": 1.25,
                "summary": true
            },
            {
                "path": "columns.*.meta.semantic_group",
                "alias": "semantic_group",
                "mode": "string_array"
            }
        ]"#;
        let vars = [
            ("DBT_NOVA_SEARCH_EXTENDED_META_FIELDS_JSON", Some(fields)),
            ("DBT_NOVA_SEARCH_EXTENDED_META_MAX_FIELDS", Some("12")),
            (
                "DBT_NOVA_SEARCH_EXTENDED_META_MAX_VALUES_PER_FIELD",
                Some("9"),
            ),
            (
                "DBT_NOVA_SEARCH_EXTENDED_META_MAX_BYTES_PER_VALUE",
                Some("2048"),
            ),
        ];

        let config = with_env_vars(&vars, SearchConfig::from_env);

        config
            .validate()
            .expect("extended meta config should validate");
        assert_eq!(config.extended_meta.fields.len(), 2);
        assert_eq!(config.extended_meta.fields[0].path, "meta.owner");
        assert_eq!(config.extended_meta.fields[0].alias, "owner");
        assert_eq!(
            config.extended_meta.fields[1].mode,
            ExtendedMetaFieldMode::StringArray
        );
        assert_eq!(config.extended_meta.max_fields, 12);
        assert_eq!(config.extended_meta.max_values_per_field, 9);
        assert_eq!(config.extended_meta.max_bytes_per_value, 2048);
        assert_eq!(config.index_fingerprint().len(), 64);
    }

    #[test]
    fn extended_meta_invalid_mode_env_fails_validation() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let fields = r#"[{"path":"meta.owner","alias":"owner","mode":"number"}]"#;
        let vars = [("DBT_NOVA_SEARCH_EXTENDED_META_FIELDS_JSON", Some(fields))];

        let config = with_env_vars(&vars, SearchConfig::from_env);

        let error = config
            .validate()
            .expect_err("invalid extended meta mode should fail validation");
        let message = error.to_string();
        assert!(message.contains("DBT_NOVA_SEARCH_EXTENDED_META_FIELDS_JSON"));
        assert!(message.contains("keyword|text|string_array|bool"));
    }

    #[test]
    fn extended_meta_rejects_sensitive_paths() {
        let config = SearchConfig {
            extended_meta: ExtendedMetaSearchConfig {
                fields: vec![ExtendedMetaFieldConfig {
                    path: "meta.accessToken".to_string(),
                    alias: "owner".to_string(),
                    mode: ExtendedMetaFieldMode::Keyword,
                    boost: 1.0,
                    summary: false,
                }],
                ..Default::default()
            },
            ..Default::default()
        };

        let error = config
            .validate()
            .expect_err("sensitive extended meta path should fail");
        assert!(error.to_string().contains("token"));
    }

    #[test]
    fn extended_meta_rejects_non_nova_scope_and_discovery_wildcards() {
        let nova_path = SearchConfig {
            extended_meta: ExtendedMetaSearchConfig {
                fields: vec![ExtendedMetaFieldConfig {
                    path: "meta.nova.owner".to_string(),
                    alias: "owner".to_string(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            ..Default::default()
        };
        let wildcard_path = SearchConfig {
            extended_meta: ExtendedMetaSearchConfig {
                fields: vec![ExtendedMetaFieldConfig {
                    path: "meta.*".to_string(),
                    alias: "anything".to_string(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            ..Default::default()
        };

        assert!(
            nova_path
                .validate()
                .expect_err("meta.nova path should fail")
                .to_string()
                .contains("meta.nova")
        );
        assert!(
            wildcard_path
                .validate()
                .expect_err("schema discovery wildcard should fail")
                .to_string()
                .contains("columns.*.meta")
        );
    }

    #[test]
    fn extended_meta_caps_limit_configured_fields() {
        let config = SearchConfig {
            extended_meta: ExtendedMetaSearchConfig {
                max_fields: 1,
                fields: vec![
                    ExtendedMetaFieldConfig {
                        path: "meta.owner".to_string(),
                        alias: "owner".to_string(),
                        ..Default::default()
                    },
                    ExtendedMetaFieldConfig {
                        path: "meta.domain".to_string(),
                        alias: "domain".to_string(),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            },
            ..Default::default()
        };

        let error = config
            .validate()
            .expect_err("too many configured fields should fail");
        assert!(error.to_string().contains("max_fields"));
    }

    #[test]
    fn extended_meta_index_fingerprint_is_order_independent_and_changes_with_config() {
        let field_a = ExtendedMetaFieldConfig {
            path: "meta.owner".to_string(),
            alias: "owner".to_string(),
            ..Default::default()
        };
        let field_b = ExtendedMetaFieldConfig {
            path: "columns.*.meta.semantic_group".to_string(),
            alias: "semantic_group".to_string(),
            mode: ExtendedMetaFieldMode::StringArray,
            ..Default::default()
        };
        let config_a = SearchConfig {
            extended_meta: ExtendedMetaSearchConfig {
                fields: vec![field_a.clone(), field_b.clone()],
                ..Default::default()
            },
            ..Default::default()
        };
        let config_b = SearchConfig {
            extended_meta: ExtendedMetaSearchConfig {
                fields: vec![field_b, field_a],
                ..Default::default()
            },
            ..Default::default()
        };
        let config_c = SearchConfig {
            extended_meta: ExtendedMetaSearchConfig {
                max_bytes_per_value: 1024,
                fields: config_a.extended_meta.fields.clone(),
                ..Default::default()
            },
            ..Default::default()
        };

        config_a.validate().expect("config a should validate");
        config_b.validate().expect("config b should validate");
        config_c.validate().expect("config c should validate");
        assert_eq!(config_a.index_fingerprint(), config_b.index_fingerprint());
        assert_ne!(config_a.index_fingerprint(), config_c.index_fingerprint());
    }
}
