use serde_json::Value as JsonValue;

use crate::config::MetadataCategoryWeights;
use crate::manifest::search::ManifestSearch;

use super::super::CategoryScore;
use crate::tools::metadata_score::helpers::{clamp_to_u8, grade_from_score};
use crate::tools::metadata_score::{
    build_column_score_diagnostics, metadata_score_scoring_contract,
};

impl ManifestSearch {
    pub(crate) fn score_column(
        &self,
        entity_id: &str,
        column_name: &str,
        column_info: &JsonValue,
        include_breakdown: bool,
        include_recommendations: bool,
        weights: MetadataCategoryWeights,
    ) -> JsonValue {
        let mut recommendations = Vec::new();
        let doc_score = self.score_column_documentation(
            column_info,
            include_breakdown,
            include_recommendations,
            &mut recommendations,
        );
        let semantic_score = self.score_column_semantic(
            column_info,
            include_breakdown,
            include_recommendations,
            &mut recommendations,
        );
        let governance_score = self.score_column_governance(
            column_info,
            include_breakdown,
            include_recommendations,
            &mut recommendations,
        );
        let quality_score = self.score_column_quality(
            entity_id,
            column_name,
            column_info,
            include_breakdown,
            include_recommendations,
            &mut recommendations,
        );

        let doc = CategoryScore {
            score: doc_score.score,
            weight: weights.documentation,
            max: 100,
        };
        let semantic = CategoryScore {
            score: semantic_score.score,
            weight: weights.semantic,
            max: 100,
        };
        let governance = CategoryScore {
            score: governance_score.score,
            weight: weights.governance,
            max: 100,
        };
        let quality = CategoryScore {
            score: quality_score.score,
            weight: weights.quality,
            max: 100,
        };

        let overall = clamp_to_u8(
            (doc.weighted() + semantic.weighted() + governance.weighted() + quality.weighted())
                .round(),
            100,
        );

        let mut obj = serde_json::Map::new();
        obj.insert(
            "column".to_string(),
            JsonValue::String(column_name.to_string()),
        );
        obj.insert(
            "overall_score".to_string(),
            JsonValue::Number(overall.into()),
        );
        obj.insert(
            "grade".to_string(),
            JsonValue::String(grade_from_score(overall).to_string()),
        );
        obj.insert(
            "categories".to_string(),
            serde_json::json!({
                "documentation": { "score": doc.score, "weight": doc.weight, "weighted": doc.weighted() },
                "semantic": { "score": semantic.score, "weight": semantic.weight, "weighted": semantic.weighted() },
                "governance": { "score": governance.score, "weight": governance.weight, "weighted": governance.weighted() },
                "quality": { "score": quality.score, "weight": quality.weight, "weighted": quality.weighted() }
            }),
        );
        obj.insert(
            "diagnostics".to_string(),
            build_column_score_diagnostics(column_name, column_info),
        );
        obj.insert(
            "scoring_contract".to_string(),
            metadata_score_scoring_contract(),
        );

        if include_breakdown {
            obj.insert(
                "breakdown".to_string(),
                serde_json::json!({
                    "documentation": doc_score.breakdown,
                    "semantic": semantic_score.breakdown,
                    "governance": governance_score.breakdown,
                    "quality": quality_score.breakdown
                }),
            );
        }

        if include_recommendations {
            obj.insert(
                "recommendations".to_string(),
                JsonValue::Array(recommendations),
            );
        }

        JsonValue::Object(obj)
    }
}
