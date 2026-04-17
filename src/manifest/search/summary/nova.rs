use std::borrow::Cow;
use std::collections::HashSet;

use serde_json::Value as JsonValue;

use crate::manifest::entity::entity_nova_meta_json;
use crate::manifest::entity::{ArchivedNovaGrain, ArchivedNovaMeta};
use crate::manifest::search::{SemanticPreviewItem, match_nova_semantics};

use super::{ManifestSearch, collect_limited_strings, path_present, truncate_str};

const NOVA_SUMMARY_SYNONYM_LIMIT: usize = 5;
const SEMANTIC_PREVIEW_LIMIT: usize = 2;
const SEMANTIC_PREVIEW_DESCRIPTION_LIMIT: usize = 100;
const SEMANTIC_PREVIEW_EXPRESSION_LIMIT: usize = 140;

impl ManifestSearch {
    pub(super) fn nova_required_missing_from_json(
        &self,
        resource_type: &str,
        entity_json: &JsonValue,
    ) -> Vec<String> {
        let required: &[String] = match self.config.governance_required_fields.get(resource_type) {
            Some(values) => values.as_slice(),
            None => &[],
        };
        if required.is_empty() {
            return Vec::new();
        }
        let mut required_fields = required.to_vec();
        required_fields.sort();
        required_fields.dedup();

        let nova_json = entity_nova_meta_json(entity_json).map_or(JsonValue::Null, Cow::into_owned);
        if nova_json.is_null() {
            return required_fields;
        }
        let mut missing = Vec::new();
        for field in &required_fields {
            let normalized = field.strip_prefix("nova.").unwrap_or(field);
            if !path_present(&nova_json, normalized) {
                missing.push(field.clone());
            }
        }
        missing.sort();
        missing.dedup();
        missing
    }

    fn nova_grain_summary(grain: &ArchivedNovaGrain) -> Option<JsonValue> {
        let mut obj = serde_json::Map::new();
        if !grain.primary_key.is_empty() {
            let keys: Vec<String> = grain
                .primary_key
                .iter()
                .map(|k| k.as_str().to_string())
                .collect();
            obj.insert("primary_key".to_string(), serde_json::json!(keys));
        }
        if let Some(time_field) = grain
            .time_field
            .as_ref()
            .map(rkyv::string::ArchivedString::as_str)
        {
            obj.insert(
                "time_field".to_string(),
                JsonValue::String(time_field.to_string()),
            );
        }
        if !grain.dimensions.is_empty() {
            let dims: Vec<String> = grain
                .dimensions
                .iter()
                .map(|d| d.as_str().to_string())
                .collect();
            obj.insert("dimensions".to_string(), serde_json::json!(dims));
        }
        if obj.is_empty() {
            None
        } else {
            Some(JsonValue::Object(obj))
        }
    }

    pub(super) fn nova_measures_summary(nova: &ArchivedNovaMeta) -> Vec<JsonValue> {
        let mut out = Vec::new();
        for measure in nova.measures.iter() {
            let mut obj = serde_json::Map::new();
            obj.insert(
                "name".to_string(),
                JsonValue::String(measure.name.as_str().to_string()),
            );
            if let Some(measure_type) = measure
                .measure_type
                .as_ref()
                .map(rkyv::string::ArchivedString::as_str)
            {
                obj.insert(
                    "type".to_string(),
                    JsonValue::String(measure_type.to_string()),
                );
            }
            if let Some(field) = measure
                .field
                .as_ref()
                .map(rkyv::string::ArchivedString::as_str)
            {
                obj.insert("field".to_string(), JsonValue::String(field.to_string()));
            }
            if measure.canonical || nova.canonical {
                obj.insert("canonical".to_string(), JsonValue::from(true));
            }
            if let Some(desc) = measure
                .description
                .as_ref()
                .map(rkyv::string::ArchivedString::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| truncate_str(s, 80))
            {
                obj.insert("description".to_string(), JsonValue::String(desc));
            }
            out.push(JsonValue::Object(obj));
        }
        out
    }

    pub(super) fn nova_metrics_summary(nova: &ArchivedNovaMeta) -> Vec<JsonValue> {
        let mut out = Vec::new();
        if let Some(metric) = nova.metric.as_ref() {
            Self::push_metric_summary(&mut out, metric, nova.canonical);
        }
        for metric in nova.metrics.iter() {
            Self::push_metric_summary(&mut out, metric, nova.canonical);
        }
        out
    }

    fn push_metric_summary(
        out: &mut Vec<JsonValue>,
        metric: &crate::manifest::entity::ArchivedNovaMetric,
        entity_canonical: bool,
    ) {
        let mut obj = serde_json::Map::new();
        obj.insert(
            "name".to_string(),
            JsonValue::String(metric.name.as_str().to_string()),
        );
        if let Some(desc) = metric
            .description
            .as_ref()
            .map(rkyv::string::ArchivedString::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| truncate_str(s, 80))
        {
            obj.insert("description".to_string(), JsonValue::String(desc));
        }
        if metric.template {
            obj.insert("template".to_string(), JsonValue::from(true));
        }
        if metric.canonical || entity_canonical {
            obj.insert("canonical".to_string(), JsonValue::from(true));
        }
        if let Some(grain) = metric.grain.as_ref().and_then(Self::nova_grain_summary) {
            obj.insert("grain".to_string(), grain);
        }
        if !obj.is_empty() {
            out.push(JsonValue::Object(obj));
        }
    }

    pub(super) fn semantic_preview(
        nova: &ArchivedNovaMeta,
        query_tokens: &[String],
        min_word_len: usize,
    ) -> Option<JsonValue> {
        if query_tokens.is_empty() {
            return None;
        }
        let token_set: HashSet<&str> = query_tokens.iter().map(String::as_str).collect();
        let matches = match_nova_semantics(nova, &token_set, min_word_len);
        if matches.is_empty() {
            return None;
        }

        let mut obj = serde_json::Map::new();
        let measures = Self::semantic_preview_items(&matches.measures);
        if !measures.is_empty() {
            obj.insert("matched_measures".to_string(), JsonValue::Array(measures));
        }
        let metrics = Self::semantic_preview_items(&matches.metrics);
        if !metrics.is_empty() {
            obj.insert("matched_metrics".to_string(), JsonValue::Array(metrics));
        }
        if matches.has_canonical_match() {
            obj.insert("canonical_match".to_string(), JsonValue::from(true));
        }

        if obj.is_empty() {
            None
        } else {
            Some(JsonValue::Object(obj))
        }
    }

    fn semantic_preview_items(items: &[SemanticPreviewItem]) -> Vec<JsonValue> {
        items
            .iter()
            .take(SEMANTIC_PREVIEW_LIMIT)
            .map(|item| {
                let mut obj = serde_json::Map::new();
                obj.insert("name".to_string(), JsonValue::String(item.name.clone()));
                if let Some(description) = item
                    .description
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(|value| truncate_str(value, SEMANTIC_PREVIEW_DESCRIPTION_LIMIT))
                {
                    obj.insert("description".to_string(), JsonValue::String(description));
                }
                if let Some(expression) = item
                    .expression
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(|value| truncate_str(value, SEMANTIC_PREVIEW_EXPRESSION_LIMIT))
                {
                    obj.insert("expression".to_string(), JsonValue::String(expression));
                }
                if let Some(field) = item
                    .field
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    obj.insert("field".to_string(), JsonValue::String(field.to_string()));
                }
                if item.canonical {
                    obj.insert("canonical".to_string(), JsonValue::from(true));
                }
                obj.insert(
                    "match_type".to_string(),
                    JsonValue::String(item.match_type.as_str().to_string()),
                );
                JsonValue::Object(obj)
            })
            .collect()
    }

    pub(super) fn nova_summary(nova: &ArchivedNovaMeta) -> Option<JsonValue> {
        let mut obj = serde_json::Map::new();
        if !nova.domains.is_empty() {
            let domains: Vec<String> = nova
                .domains
                .iter()
                .map(|v| v.as_str().to_string())
                .collect();
            obj.insert("domains".to_string(), serde_json::json!(domains));
        }
        if let Some(role) = nova.role.as_ref().map(rkyv::string::ArchivedString::as_str) {
            obj.insert("role".to_string(), JsonValue::String(role.to_string()));
        }
        if let Some(grain) = nova.grain.as_ref().and_then(Self::nova_grain_summary) {
            obj.insert("grain".to_string(), grain);
        }
        let measures = Self::nova_measures_summary(nova);
        if !measures.is_empty() {
            obj.insert("measures".to_string(), JsonValue::Array(measures));
        }
        let metrics = Self::nova_metrics_summary(nova);
        if !metrics.is_empty() {
            obj.insert("metrics".to_string(), JsonValue::Array(metrics));
        }
        if let Some(tier) = nova.tier.as_ref().map(rkyv::string::ArchivedString::as_str) {
            obj.insert("tier".to_string(), JsonValue::String(tier.to_string()));
        }
        if nova.canonical {
            obj.insert("canonical".to_string(), JsonValue::from(true));
        }
        if let Some(candidates) = Self::nova_search_candidates_summary(nova) {
            obj.insert("search_candidates".to_string(), candidates);
        }
        if !nova.use_cases.is_empty() {
            let use_cases: Vec<String> = nova
                .use_cases
                .iter()
                .map(|v| v.as_str().to_string())
                .collect();
            obj.insert("use_cases".to_string(), serde_json::json!(use_cases));
        }
        if !nova.synonyms.is_empty() {
            let (synonyms, truncated) = collect_limited_strings(
                nova.synonyms.iter().map(|s| s.as_str().to_string()),
                NOVA_SUMMARY_SYNONYM_LIMIT,
            );
            obj.insert("synonyms".to_string(), serde_json::json!(synonyms));
            obj.insert("synonyms_truncated".to_string(), JsonValue::from(truncated));
        }
        if obj.is_empty() {
            None
        } else {
            Some(JsonValue::Object(obj))
        }
    }

    pub(super) fn nova_search_candidates_summary(nova: &ArchivedNovaMeta) -> Option<JsonValue> {
        let candidates = nova.search.as_ref()?.candidates.as_ref()?;
        if !candidates.has_non_default_flags() {
            return None;
        }

        let mut obj = serde_json::Map::new();
        if !candidates.analyst {
            obj.insert("analyst".to_string(), JsonValue::from(false));
        }
        if !candidates.engineer {
            obj.insert("engineer".to_string(), JsonValue::from(false));
        }
        if !candidates.governance {
            obj.insert("governance".to_string(), JsonValue::from(false));
        }

        if obj.is_empty() {
            None
        } else {
            Some(JsonValue::Object(obj))
        }
    }

    pub(super) fn nova_governance_summary(nova: &ArchivedNovaMeta) -> Option<JsonValue> {
        let governance = nova.governance.as_ref()?;

        let mut obj = serde_json::Map::new();
        if let Some(sensitivity) = governance
            .sensitivity
            .as_ref()
            .map(rkyv::string::ArchivedString::as_str)
        {
            obj.insert(
                "sensitivity".to_string(),
                JsonValue::String(sensitivity.to_string()),
            );
        }
        if let Some(pii) = governance
            .pii
            .as_ref()
            .map(rkyv::string::ArchivedString::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            obj.insert("pii".to_string(), JsonValue::String(pii.to_string()));
        }
        if !governance.compliance.is_empty() {
            let compliance: Vec<String> = governance
                .compliance
                .iter()
                .map(|v| v.as_str().to_string())
                .collect();
            obj.insert("compliance".to_string(), serde_json::json!(compliance));
        }
        if obj.is_empty() {
            None
        } else {
            Some(JsonValue::Object(obj))
        }
    }
}
