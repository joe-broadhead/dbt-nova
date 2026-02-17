use serde::{Deserialize, Serialize};

use super::{env_string, parse_f64, parse_usize};

/// Column lineage configuration and thresholds.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct ColumnLineageConfig {
    /// Default confidence level: "high", "medium", or "low"
    pub default_confidence: String,
    /// Levenshtein similarity threshold (0.0 - 1.0)
    pub levenshtein_threshold: f64,
    /// Maximum character distance for SQL proximity matching
    pub sql_proximity_max_distance: usize,
    /// Minimum column name length for prefix/suffix matching
    pub min_prefix_suffix_length: usize,
    /// Minimum column name length for Levenshtein matching
    pub min_levenshtein_length: usize,
    /// Maximum lineage matches to return before truncating
    pub max_results: usize,
    /// Maximum depth to traverse for column lineage
    pub max_depth: usize,
    /// Maximum match candidates to consider (prevents memory exhaustion)
    pub max_match_candidates: usize,
    /// Strategy-level configuration and weights
    pub matching: ColumnMatchingConfig,
}

/// Configuration for each column-matching strategy.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct ColumnMatchingConfig {
    pub exact_match: MatchingStrategy,
    pub sql_alias: MatchingStrategy,
    pub sql_proximity: MatchingStrategy,
    pub suffix_match: MatchingStrategy,
    pub prefix_match: MatchingStrategy,
    pub normalized_match: MatchingStrategy,
    pub levenshtein_match: MatchingStrategy,
}

/// Enablement, confidence tier, and weight for a matching strategy.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct MatchingStrategy {
    pub enabled: bool,
    /// Confidence tier reported when this strategy matches
    pub confidence_tier: ConfidenceTier,
    /// Weight used when multiple matches compete
    pub weight: f64,
}

/// Confidence tier used in lineage results.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ConfidenceTier {
    High,
    Medium,
    Low,
}

impl ConfidenceTier {
    /// Return the canonical lowercase string representation of the tier.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            ConfidenceTier::High => "high",
            ConfidenceTier::Medium => "medium",
            ConfidenceTier::Low => "low",
        }
    }
}

impl Default for ColumnLineageConfig {
    fn default() -> Self {
        Self {
            default_confidence: "medium".to_string(),
            levenshtein_threshold: 0.75,
            sql_proximity_max_distance: 100,
            min_prefix_suffix_length: 2,
            min_levenshtein_length: 3,
            max_results: 10_000,
            max_depth: 100,
            max_match_candidates: 10_000,
            matching: ColumnMatchingConfig::default(),
        }
    }
}

impl ColumnLineageConfig {
    /// Load column lineage configuration overrides from environment variables.
    #[must_use]
    pub fn from_env() -> Self {
        let mut config = Self::default();

        crate::env_config!(
            config,
            levenshtein_threshold,
            "DBT_NOVA_LEVENSHTEIN_THRESHOLD",
            parse_f64,
            |v: &f64| (0.0..=1.0).contains(v)
        );

        if let Some(conf) = env_string("DBT_NOVA_DEFAULT_CONFIDENCE") {
            let lc = conf.to_lowercase();
            if matches!(lc.as_str(), "high" | "medium" | "low") {
                config.default_confidence = lc;
            }
        }

        crate::env_config!(
            config,
            sql_proximity_max_distance,
            "DBT_NOVA_SQL_PROXIMITY_MAX_DISTANCE",
            parse_usize,
            |v: &usize| *v > 0
        );
        crate::env_config!(
            config,
            min_prefix_suffix_length,
            "DBT_NOVA_MIN_PREFIX_SUFFIX_LENGTH",
            parse_usize,
            |v: &usize| *v > 0
        );
        crate::env_config!(
            config,
            min_levenshtein_length,
            "DBT_NOVA_MIN_LEVENSHTEIN_LENGTH",
            parse_usize,
            |v: &usize| *v > 0
        );
        crate::env_config!(
            config,
            max_results,
            "DBT_NOVA_MAX_LINEAGE_RESULTS",
            parse_usize,
            |v: &usize| *v > 0
        );
        crate::env_config!(
            config,
            max_depth,
            "DBT_NOVA_COLUMN_LINEAGE_MAX_DEPTH",
            parse_usize,
            |v: &usize| *v > 0
        );
        crate::env_config!(
            config,
            max_match_candidates,
            "DBT_NOVA_COLUMN_LINEAGE_MAX_CANDIDATES",
            parse_usize,
            |v: &usize| *v > 0
        );

        config
    }
}

impl Default for ColumnMatchingConfig {
    fn default() -> Self {
        Self {
            exact_match: MatchingStrategy {
                enabled: true,
                confidence_tier: ConfidenceTier::High,
                weight: 1.0,
            },
            sql_alias: MatchingStrategy {
                enabled: true,
                confidence_tier: ConfidenceTier::Medium,
                weight: 0.9,
            },
            sql_proximity: MatchingStrategy {
                enabled: true,
                confidence_tier: ConfidenceTier::Medium,
                weight: 0.8,
            },
            suffix_match: MatchingStrategy {
                enabled: true,
                confidence_tier: ConfidenceTier::Low,
                weight: 0.4,
            },
            prefix_match: MatchingStrategy {
                enabled: true,
                confidence_tier: ConfidenceTier::Low,
                weight: 0.4,
            },
            normalized_match: MatchingStrategy {
                enabled: true,
                confidence_tier: ConfidenceTier::Low,
                weight: 0.5,
            },
            levenshtein_match: MatchingStrategy {
                enabled: true,
                confidence_tier: ConfidenceTier::Low,
                weight: 0.6,
            },
        }
    }
}

impl Default for MatchingStrategy {
    fn default() -> Self {
        Self {
            enabled: true,
            confidence_tier: ConfidenceTier::Medium,
            weight: 1.0,
        }
    }
}
