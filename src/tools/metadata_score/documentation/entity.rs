use serde_json::Value as JsonValue;

use crate::manifest::search::ManifestSearch;

use crate::tools::metadata_score::CategoryBreakdown;
use crate::tools::metadata_score::helpers::{
    clamp_to_u8, description_progress_recommendation, description_tier_score, expects_columns,
    has_owner, push_recommendation,
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
            let desc_len = description.trim().len();
            push_recommendation(
                recommendations,
                "documentation",
                30 - desc_score,
                description_progress_recommendation(
                    "description",
                    desc_len,
                    desc_score,
                    30,
                    "Add a clear entity description (50+ chars recommended).",
                ),
                "description",
            );
        }

        let columns = entity_json
            .get("columns")
            .and_then(|c| c.as_object())
            .cloned()
            .unwrap_or_default();
        let expects_columns = expects_columns(resource_type);
        let column_description_quality = column_description_score(&columns, expects_columns);
        let column_desc_points =
            clamp_to_u8((column_description_quality.quality * 40.0).round(), 40);
        if include_recommendations && column_desc_points < 40 && expects_columns {
            let message = if column_description_quality.columns_total == 0 {
                "Add column descriptions for expected columns.".to_string()
            } else if column_description_quality.columns_with_desc
                < column_description_quality.columns_total
            {
                format!(
                    "Document {}/{} columns; include business meaning, grain, and common usage.",
                    column_description_quality.columns_with_desc,
                    column_description_quality.columns_total
                )
            } else {
                format!(
                    "All columns have descriptions, but only {}/{} reach full description-tier credit; 100+ chars score full credit.",
                    column_description_quality.columns_full_credit,
                    column_description_quality.columns_total
                )
            };
            push_recommendation(
                recommendations,
                "documentation",
                40 - column_desc_points,
                message,
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
                    "columns_total": column_description_quality.columns_total,
                    "columns_described": column_description_quality.columns_with_desc,
                    "columns_50_chars_or_more": column_description_quality.columns_good_enough,
                    "columns_100_chars_or_more": column_description_quality.columns_full_credit,
                    "quality_ratio": column_description_quality.quality
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

struct ColumnDescriptionQuality {
    columns_total: usize,
    columns_with_desc: usize,
    columns_good_enough: usize,
    columns_full_credit: usize,
    quality: f32,
}

#[allow(clippy::cast_precision_loss)]
fn column_description_score(
    columns: &serde_json::Map<String, JsonValue>,
    expects_columns: bool,
) -> ColumnDescriptionQuality {
    if columns.is_empty() {
        let quality = if expects_columns { 0.0 } else { 1.0 };
        return ColumnDescriptionQuality {
            columns_total: 0,
            columns_with_desc: 0,
            columns_good_enough: 0,
            columns_full_credit: 0,
            quality,
        };
    }

    let mut described = 0usize;
    let mut good_enough = 0usize;
    let mut full_credit = 0usize;
    let mut total_quality = 0.0f32;
    for column in columns.values() {
        let desc = column
            .get("description")
            .and_then(|d| d.as_str())
            .unwrap_or("");
        if !desc.trim().is_empty() {
            described += 1;
        }
        let len = desc.trim().len();
        if len >= 50 {
            good_enough += 1;
        }
        if len >= 100 {
            full_credit += 1;
        }
        let tier = f32::from(description_tier_score(desc, 100)) / 100.0;
        total_quality += tier;
    }
    let quality = total_quality / columns.len() as f32;
    ColumnDescriptionQuality {
        columns_total: columns.len(),
        columns_with_desc: described,
        columns_good_enough: good_enough,
        columns_full_credit: full_credit,
        quality,
    }
}
