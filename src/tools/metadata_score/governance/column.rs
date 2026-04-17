use serde_json::Value as JsonValue;

use crate::manifest::entity::column_nova_meta_json;
use crate::manifest::search::ManifestSearch;

use crate::tools::metadata_score::CategoryBreakdown;
use crate::tools::metadata_score::helpers::{array_tier_score, push_recommendation};

impl ManifestSearch {
    #[allow(clippy::unused_self)]
    pub(crate) fn score_column_governance(
        &self,
        column_info: &JsonValue,
        include_breakdown: bool,
        include_recommendations: bool,
        recommendations: &mut Vec<JsonValue>,
    ) -> CategoryBreakdown {
        let nova_json = column_nova_meta_json(column_info);
        let governance = nova_json.as_deref().and_then(|n| n.get("governance"));

        let sensitivity = governance
            .and_then(|g| g.get("sensitivity"))
            .and_then(|s| s.as_str());
        let sensitivity_score = if sensitivity.is_some() { 40 } else { 0 };
        if include_recommendations && sensitivity_score == 0 {
            push_recommendation(
                recommendations,
                "governance",
                40,
                "Add governance.sensitivity classification".to_string(),
                "columns.meta.nova.governance.sensitivity",
            );
        }

        let pii = governance.and_then(|g| g.get("pii"));
        let pii_score = if pii.is_some() { 40 } else { 0 };
        if include_recommendations && pii_score == 0 {
            push_recommendation(
                recommendations,
                "governance",
                40,
                "Add governance.pii classification".to_string(),
                "columns.meta.nova.governance.pii",
            );
        }

        let compliance_count = governance
            .and_then(|g| g.get("compliance"))
            .and_then(JsonValue::as_array)
            .map_or(0, Vec::len);
        let compliance_score = array_tier_score(compliance_count, 20);
        if include_recommendations && compliance_score < 20 {
            push_recommendation(
                recommendations,
                "governance",
                20 - compliance_score,
                "Add governance.compliance list".to_string(),
                "columns.meta.nova.governance.compliance",
            );
        }

        let score = sensitivity_score + pii_score + compliance_score;
        let breakdown = if include_breakdown {
            serde_json::json!({
                "sensitivity": {"score": sensitivity_score, "max": 40, "present": sensitivity.is_some()},
                "pii": {"score": pii_score, "max": 40, "present": pii.is_some()},
                "compliance": {"score": compliance_score, "max": 20, "count": compliance_count}
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
