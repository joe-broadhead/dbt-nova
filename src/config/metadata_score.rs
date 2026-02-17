use serde::{Deserialize, Serialize};

/// Category weights for metadata scoring.
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(default)]
pub struct MetadataCategoryWeights {
    pub documentation: f32,
    pub semantic: f32,
    pub governance: f32,
    pub quality: f32,
}

impl Default for MetadataCategoryWeights {
    fn default() -> Self {
        Self {
            documentation: 0.30,
            semantic: 0.25,
            governance: 0.25,
            quality: 0.20,
        }
    }
}

/// Persona profiles for metadata scoring.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct MetadataScoreWeightProfiles {
    pub analyst: MetadataCategoryWeights,
    pub engineer: MetadataCategoryWeights,
    pub governance: MetadataCategoryWeights,
    pub default: MetadataCategoryWeights,
}

impl Default for MetadataScoreWeightProfiles {
    fn default() -> Self {
        Self {
            analyst: MetadataCategoryWeights {
                documentation: 0.20,
                semantic: 0.45,
                governance: 0.15,
                quality: 0.20,
            },
            engineer: MetadataCategoryWeights {
                documentation: 0.20,
                semantic: 0.15,
                governance: 0.15,
                quality: 0.50,
            },
            governance: MetadataCategoryWeights {
                documentation: 0.15,
                semantic: 0.15,
                governance: 0.55,
                quality: 0.15,
            },
            default: MetadataCategoryWeights::default(),
        }
    }
}

/// Metadata scoring configuration.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct MetadataScoreConfig {
    pub persona_weights: MetadataScoreWeightProfiles,
}

impl MetadataScoreConfig {
    /// Load metadata score configuration overrides from environment variables.
    #[must_use]
    pub fn from_env() -> Self {
        Self::default()
    }
}
