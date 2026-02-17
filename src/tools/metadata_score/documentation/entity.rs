use serde_json::Value as JsonValue;

use crate::manifest::search::ManifestSearch;

use crate::tools::metadata_score::CategoryBreakdown;
use crate::tools::metadata_score::helpers::{
    clamp_to_u8, description_tier_score, expects_columns, has_owner, push_recommendation,
};

impl ManifestSearch {
    #[allow(clippy::unused_self)]
    pub(crate) fn score_documentation(
        &self,
        entity_json: &JsonValue,
        resource_type: Option<&str>,
        include_breakdown: bool,
        include_recommendations: bool,
        recommendations: &mut Vec<JsonValue>,
    ) -> CategoryBreakdown {
        let description = entity_json
            .get("description")
            .and_then(|d| d.as_str())
            .unwrap_or("");
        let desc_score = description_tier_score(description, 30);
        if include_recommendations && desc_score < 30 {
            push_recommendation(
                recommendations,
                "documentation",
                30 - desc_score,
                "Add a clear entity description (50+ chars recommended)".to_string(),
                "description",
            );
        }

        let columns = entity_json
            .get("columns")
            .and_then(|c| c.as_object())
            .cloned()
            .unwrap_or_default();
        let expects_columns = expects_columns(resource_type);
        let (columns_total, columns_with_desc, column_desc_quality) =
            column_description_score(&columns, expects_columns);
        let column_desc_points = clamp_to_u8((column_desc_quality * 40.0).round(), 40);
        if include_recommendations && column_desc_points < 40 && expects_columns {
            let message = if columns_total > 0 && columns_with_desc == columns_total {
                "Improve column description quality (more detail, 50+ chars recommended)"
            } else {
                "Document more columns with meaningful descriptions"
            };
            push_recommendation(
                recommendations,
                "documentation",
                40 - column_desc_points,
                message.to_string(),
                "columns.description",
            );
        }

        let owner_present = has_owner(entity_json);
        let owner_score = if owner_present { 15 } else { 0 };
        if include_recommendations && owner_score == 0 {
            push_recommendation(
                recommendations,
                "documentation",
                15,
                "Define an owner for this entity".to_string(),
                "owner",
            );
        }

        let score = desc_score + column_desc_points + owner_score;
        let breakdown = if include_breakdown {
            serde_json::json!({
                "description": {"score": desc_score, "max": 30, "length": description.trim().len()},
                "column_descriptions": {
                    "score": column_desc_points,
                    "max": 40,
                    "columns_total": columns_total,
                    "columns_described": columns_with_desc,
                    "quality_ratio": column_desc_quality
                },
                "owner": {"score": owner_score, "max": 15, "present": owner_present}
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

#[allow(clippy::cast_precision_loss)]
fn column_description_score(
    columns: &serde_json::Map<String, JsonValue>,
    expects_columns: bool,
) -> (usize, usize, f32) {
    if columns.is_empty() {
        let quality = if expects_columns { 0.0 } else { 1.0 };
        return (0, 0, quality);
    }

    let mut described = 0usize;
    let mut total_quality = 0.0f32;
    for column in columns.values() {
        let desc = column
            .get("description")
            .and_then(|d| d.as_str())
            .unwrap_or("");
        if !desc.trim().is_empty() {
            described += 1;
        }
        let tier = f32::from(description_tier_score(desc, 100)) / 100.0;
        total_quality += tier;
    }
    let quality = total_quality / columns.len() as f32;
    (columns.len(), described, quality)
}
