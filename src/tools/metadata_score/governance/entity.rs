use serde_json::Value as JsonValue;

use crate::manifest::search::ManifestSearch;

use crate::tools::metadata_score::CategoryBreakdown;
use crate::tools::metadata_score::helpers::{array_tier_score, has_owner, push_recommendation};

impl ManifestSearch {
    #[allow(clippy::unused_self)]
    pub(crate) fn score_governance(
        &self,
        entity_json: &JsonValue,
        include_breakdown: bool,
        include_recommendations: bool,
        recommendations: &mut Vec<JsonValue>,
    ) -> CategoryBreakdown {
        let nova = entity_json.get("meta").and_then(|m| m.get("nova"));

        let sensitivity = nova
            .and_then(|n| n.get("governance"))
            .and_then(|g| g.get("sensitivity"))
            .and_then(|s| s.as_str());
        let sensitivity_score = if sensitivity.is_some() { 20 } else { 0 };
        if include_recommendations && sensitivity_score == 0 {
            push_recommendation(
                recommendations,
                "governance",
                20,
                "Define meta.nova.governance.sensitivity classification".to_string(),
                "meta.nova.governance.sensitivity",
            );
        }

        let pii = nova
            .and_then(|n| n.get("governance"))
            .and_then(|g| g.get("pii"));
        let pii_score = if pii.is_some() { 30 } else { 0 };
        if include_recommendations && pii_score == 0 {
            push_recommendation(
                recommendations,
                "governance",
                30,
                "Add meta.nova.governance.pii classification".to_string(),
                "meta.nova.governance.pii",
            );
        }

        let compliance_count = nova
            .and_then(|n| n.get("governance"))
            .and_then(|g| g.get("compliance"))
            .and_then(JsonValue::as_array)
            .map_or(0, Vec::len);
        let compliance_score = array_tier_score(compliance_count, 20);
        if include_recommendations && compliance_score < 20 {
            push_recommendation(
                recommendations,
                "governance",
                20 - compliance_score,
                "List compliance requirements in meta.nova.governance.compliance".to_string(),
                "meta.nova.governance.compliance",
            );
        }

        let owner_score = if has_owner(entity_json) { 30 } else { 0 };
        if include_recommendations && owner_score == 0 {
            push_recommendation(
                recommendations,
                "governance",
                30,
                "Define an owner for stewardship".to_string(),
                "owner",
            );
        }

        let score = sensitivity_score + pii_score + compliance_score + owner_score;
        let breakdown = if include_breakdown {
            serde_json::json!({
                "sensitivity": {"score": sensitivity_score, "max": 20, "present": sensitivity.is_some()},
                "pii": {"score": pii_score, "max": 30, "present": pii.is_some()},
                "compliance": {"score": compliance_score, "max": 20, "count": compliance_count},
                "owner": {"score": owner_score, "max": 30, "present": owner_score > 0}
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
