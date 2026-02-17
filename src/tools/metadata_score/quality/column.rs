use serde_json::Value as JsonValue;

use crate::manifest::search::ManifestSearch;

use crate::tools::metadata_score::CategoryBreakdown;
use crate::tools::metadata_score::helpers::{array_tier_score, push_recommendation};

impl ManifestSearch {
    pub(crate) fn score_column_quality(
        &self,
        entity_id: &str,
        column_name: &str,
        column_info: &JsonValue,
        include_breakdown: bool,
        include_recommendations: bool,
        recommendations: &mut Vec<JsonValue>,
    ) -> CategoryBreakdown {
        let data_type_present = column_info.get("data_type").is_some();
        let data_type_score = if data_type_present { 40 } else { 0 };
        if include_recommendations && data_type_score == 0 {
            push_recommendation(
                recommendations,
                "quality",
                40,
                "Add column data type".to_string(),
                "columns.data_type",
            );
        }

        let key = format!("{entity_id}:{column_name}");
        let tests = self.tests_by_column.get(&key).cloned().unwrap_or_default();
        let test_score = if tests.is_empty() { 0 } else { 40 };

        let constraint_count = column_info
            .get("constraints")
            .and_then(JsonValue::as_array)
            .map_or(0, Vec::len);
        let constraints_score = array_tier_score(constraint_count, 20);

        let score = data_type_score + test_score + constraints_score;
        if include_recommendations && score < 100 {
            push_recommendation(
                recommendations,
                "quality",
                100 - score,
                "Add data types, tests, and constraints to the column".to_string(),
                "columns",
            );
        }
        let breakdown = if include_breakdown {
            serde_json::json!({
                "data_type": {"score": data_type_score, "max": 40, "present": data_type_present},
                "tests": {"score": test_score, "max": 40, "present": !tests.is_empty()},
                "constraints": {"score": constraints_score, "max": 20, "count": constraint_count}
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
