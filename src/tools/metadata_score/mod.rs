mod column;
mod diagnostics;
mod documentation;
mod entry;
mod governance;
mod helpers;
mod quality;
mod semantic;

pub(crate) use diagnostics::{
    build_column_score_diagnostics, build_entity_score_diagnostics, metadata_score_scoring_contract,
};
pub use helpers::{array_tier_score, description_tier_score, grade_from_score};

use serde_json::Value as JsonValue;

#[derive(Clone, Copy)]
pub(crate) struct CategoryScore {
    pub(crate) score: u8,
    pub(crate) weight: f32,
    pub(crate) max: u8,
}

impl CategoryScore {
    pub(crate) fn weighted(self) -> f32 {
        if self.max == 0 {
            return 0.0;
        }
        (f32::from(self.score) / f32::from(self.max) * 100.0) * self.weight
    }
}

pub(crate) struct CategoryBreakdown {
    pub(crate) score: u8,
    pub(crate) breakdown: JsonValue,
    pub(crate) summary: JsonValue,
}

pub(crate) struct ScoredMetadata {
    pub(crate) overall_score: u8,
    pub(crate) grade: String,
    pub(crate) categories: JsonValue,
    pub(crate) breakdown: Option<JsonValue>,
    pub(crate) diagnostics: JsonValue,
    pub(crate) recommendations: Vec<JsonValue>,
}
