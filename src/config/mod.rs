//! Configuration module for dbt-nova.
//!
//! Split into submodules for maintainability:
//! - `core`: Core settings (manifest, storage, logging)
//! - `search`: Search configuration and persona weights
//! - `column_lineage`: Column lineage tracing settings
//! - `hosted_auth`: Default-off hosted HTTP authentication config skeleton
//! - `metadata_score`: Metadata scoring configuration
//! - `warehouse`: Warehouse connection settings

pub mod column_lineage;
pub mod core;
pub mod hosted_auth;
pub mod metadata_score;
pub mod search;
pub mod warehouse;

pub use column_lineage::{
    ColumnLineageConfig, ColumnMatchingConfig, ConfidenceTier, MatchingStrategy,
};
pub use core::{
    AgentModellingAuditConfig, AgentReadinessConfig, AgentReadinessModellingConfig,
    ArtifactFetchPolicy, DbtNovaConfig, GovernanceGateConfig, ResultProfile, RuntimePreset,
    ServerTransport,
};
pub use hosted_auth::{HostedAuthConfig, HostedAuthMode};
pub use metadata_score::{
    MetadataCategoryWeights, MetadataScoreConfig, MetadataScoreWeightProfiles,
};
pub use search::{
    ExtendedMetaFieldConfig, ExtendedMetaFieldMode, ExtendedMetaSearchConfig, PersonaProfile,
    PersonaWeights, SearchColdStartPolicy, SearchConfig,
};

/// Read a string environment variable.
pub(crate) fn env_string(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

/// Set a string field from an environment variable.
pub(crate) fn set_string(name: &str, target: &mut String) {
    if let Some(value) = env_string(name)
        && !value.trim().is_empty()
    {
        *target = value;
    }
}

pub(crate) fn parse_bool(name: &str) -> Option<bool> {
    env_string(name).and_then(|v| match v.trim().to_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    })
}

pub(crate) fn parse_usize(name: &str) -> Option<usize> {
    env_string(name).and_then(|v| v.parse().ok())
}

pub(crate) fn parse_u64(name: &str) -> Option<u64> {
    env_string(name).and_then(|v| v.parse().ok())
}

pub(crate) fn parse_u16(name: &str) -> Option<u16> {
    env_string(name).and_then(|v| v.parse().ok())
}

pub(crate) fn parse_f64(name: &str) -> Option<f64> {
    env_string(name).and_then(|v| v.parse().ok())
}

pub(crate) fn parse_f32(name: &str) -> Option<f32> {
    env_string(name).and_then(|v| v.parse().ok())
}

/// Apply a parsed environment variable to a config field.
#[macro_export]
macro_rules! env_config {
    ($target:expr, $field:ident, $key:expr, $parser:ident) => {{
        if let Some(value) = $parser($key) {
            $target.$field = value;
        }
    }};
    ($target:expr, $field:ident $(.$rest:ident)+, $key:expr, $parser:ident) => {{
        if let Some(value) = $parser($key) {
            $target.$field$(.$rest)+ = value;
        }
    }};
    ($target:expr, $field:ident, $key:expr, $parser:ident, $predicate:expr) => {{
        if let Some(value) = $parser($key) {
            if $predicate(&value) {
                $target.$field = value;
            }
        }
    }};
    ($target:expr, $field:ident $(.$rest:ident)+, $key:expr, $parser:ident, $predicate:expr) => {{
        if let Some(value) = $parser($key) {
            if $predicate(&value) {
                $target.$field$(.$rest)+ = value;
            }
        }
    }};
}
