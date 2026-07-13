use serde_json::Value as JsonValue;

use crate::manifest::search::ManifestSearch;

use crate::tools::metadata_score::CategoryBreakdown;
use crate::tools::metadata_score::helpers::{
    description_progress_recommendation, description_tier_score, has_owner, push_recommendation,
};

impl ManifestSearch {
    #[allow(clippy::unused_self)]
    pub(crate) fn score_column_documentation(
        &self,
        column_info: &JsonValue,
        include_breakdown: bool,
        include_recommendations: bool,
        recommendations: &mut Vec<JsonValue>,
    ) -> CategoryBreakdown {
        let description = column_info
            .get("description")
            .and_then(|d| d.as_str())
            .unwrap_or("");
        let desc_score = description_tier_score(description, 50);
        if include_recommendations && desc_score < 50 {
            let desc_len = description.trim().len();
            push_recommendation(
                recommendations,
                "documentation",
                50 - desc_score,
                description_progress_recommendation(
                    "columns.description",
                    desc_len,
                    desc_score,
                    50,
                    "Add a descriptive column description.",
                ),
                "columns.description",
            );
        }

        let owner_present = has_owner(column_info);
        let owner_score = if owner_present { 50 } else { 0 };
        if include_recommendations && owner_score == 0 {
            push_recommendation(
                recommendations,
                "documentation",
                50,
                "Define an owner for this column".to_string(),
                "columns.owner",
            );
        }

        let score = desc_score + owner_score;
        let breakdown = if include_breakdown {
            serde_json::json!({
                "description": {"score": desc_score, "max": 50, "length": description.trim().len()},
                "owner": {"score": owner_score, "max": 50, "present": owner_present}
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
