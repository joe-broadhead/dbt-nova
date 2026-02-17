use serde_json::Value as JsonValue;

use crate::config::MetadataCategoryWeights;
use crate::manifest::search::ManifestSearch;

use super::super::{CategoryScore, ScoredMetadata};
use crate::tools::metadata_score::helpers::{clamp_to_u8, grade_from_score};

impl ManifestSearch {
    #[allow(clippy::too_many_lines)]
    pub(crate) fn score_entity(
        &self,
        unique_id: &str,
        entity_json: &JsonValue,
        include_breakdown: bool,
        include_recommendations: bool,
        weights: MetadataCategoryWeights,
    ) -> ScoredMetadata {
        let resource_type = entity_json.get("resource_type").and_then(|r| r.as_str());

        let mut recommendations = Vec::new();
        let doc = self.score_documentation(
            entity_json,
            resource_type,
            include_breakdown,
            include_recommendations,
            &mut recommendations,
        );
        let semantic = self.score_semantic(
            entity_json,
            resource_type,
            include_breakdown,
            include_recommendations,
            &mut recommendations,
        );
        let governance = self.score_governance(
            entity_json,
            include_breakdown,
            include_recommendations,
            &mut recommendations,
        );
        let quality = self.score_quality(
            unique_id,
            entity_json,
            resource_type,
            include_breakdown,
            include_recommendations,
            &mut recommendations,
        );

        let doc_score = CategoryScore {
            score: doc.score,
            weight: weights.documentation,
            max: 85,
        };
        let semantic_score = CategoryScore {
            score: semantic.score,
            weight: weights.semantic,
            max: 98,
        };
        let governance_score = CategoryScore {
            score: governance.score,
            weight: weights.governance,
            max: 100,
        };
        let quality_score = CategoryScore {
            score: quality.score,
            weight: weights.quality,
            max: 100,
        };

        let overall = clamp_to_u8(
            (doc_score.weighted()
                + semantic_score.weighted()
                + governance_score.weighted()
                + quality_score.weighted())
            .round(),
            100,
        );

        let categories = serde_json::json!({
            "documentation": {
                "score": doc_score.score,
                "weight": doc_score.weight,
                "weighted": doc_score.weighted()
            },
            "semantic": {
                "score": semantic_score.score,
                "weight": semantic_score.weight,
                "weighted": semantic_score.weighted()
            },
            "governance": {
                "score": governance_score.score,
                "weight": governance_score.weight,
                "weighted": governance_score.weighted()
            },
            "quality": {
                "score": quality_score.score,
                "weight": quality_score.weight,
                "weighted": quality_score.weighted(),
                "summary": quality.summary
            }
        });

        let breakdown = if include_breakdown {
            Some(serde_json::json!({
                "documentation": doc.breakdown,
                "semantic": semantic.breakdown,
                "governance": governance.breakdown,
                "quality": quality.breakdown
            }))
        } else {
            None
        };

        ScoredMetadata {
            overall_score: overall,
            grade: grade_from_score(overall).to_string(),
            categories,
            breakdown,
            recommendations: if include_recommendations {
                recommendations
            } else {
                Vec::new()
            },
        }
    }
}
