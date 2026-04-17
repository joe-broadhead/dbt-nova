use serde_json::Value as JsonValue;

use crate::manifest::entity::column_nova_meta_json;
use crate::manifest::search::ManifestSearch;

use crate::tools::metadata_score::CategoryBreakdown;
use crate::tools::metadata_score::helpers::{array_len, array_tier_score, push_recommendation};

impl ManifestSearch {
    #[allow(clippy::unused_self)]
    pub(crate) fn score_column_semantic(
        &self,
        column_info: &JsonValue,
        include_breakdown: bool,
        include_recommendations: bool,
        recommendations: &mut Vec<JsonValue>,
    ) -> CategoryBreakdown {
        let nova = column_nova_meta_json(column_info);

        let role_present = nova.as_deref().and_then(|n| n.get("role")).is_some();
        let semantic_present = nova
            .as_deref()
            .and_then(|n| n.get("semantic_type"))
            .is_some();
        let synonyms_count = array_len(nova.as_deref(), "synonyms");

        let role_score = if role_present { 30 } else { 0 };
        if include_recommendations && role_score == 0 {
            push_recommendation(
                recommendations,
                "semantic",
                30,
                "Add meta.nova.role to column".to_string(),
                "columns.meta.nova.role",
            );
        }

        let semantic_score = if semantic_present { 30 } else { 0 };
        if include_recommendations && semantic_score == 0 {
            push_recommendation(
                recommendations,
                "semantic",
                30,
                "Add meta.nova.semantic_type to column".to_string(),
                "columns.meta.nova.semantic_type",
            );
        }

        let synonyms_score = array_tier_score(synonyms_count, 40);
        if include_recommendations && synonyms_score < 40 {
            push_recommendation(
                recommendations,
                "semantic",
                40 - synonyms_score,
                "Add meta.nova.synonyms to column".to_string(),
                "columns.meta.nova.synonyms",
            );
        }

        let score = role_score + semantic_score + synonyms_score;
        let breakdown = if include_breakdown {
            serde_json::json!({
                "role": {"score": role_score, "max": 30, "present": role_present},
                "semantic_type": {"score": semantic_score, "max": 30, "present": semantic_present},
                "synonyms": {"score": synonyms_score, "max": 40, "count": synonyms_count}
            })
        } else {
            JsonValue::Null
        };

        CategoryBreakdown {
            score,
            breakdown,
            summary: JsonValue::Null,
        }
    }
}
