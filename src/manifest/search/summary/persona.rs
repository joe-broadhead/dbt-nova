use std::collections::HashSet;

use serde_json::Value as JsonValue;

use crate::manifest::entity::{ArchivedEntity, ArchivedNovaGrain, ArchivedNovaMeta};
use crate::manifest::lineage_sql::{extract_ref_calls, sql_for_matching};
use crate::utils::{SearchPersona, tokenize_alnum_lowercase};

use super::{
    ManifestSearch, compact_json_object, has_apparent_malformed_ref, path_present, truncate_str,
};

#[derive(Clone, Copy)]
#[allow(clippy::struct_excessive_bools)]
struct AnalystSelectionSignals {
    has_metric_definition: bool,
    has_measure_definition: bool,
    has_grain: bool,
    has_time_field: bool,
    dimension_overlap: usize,
}

impl ManifestSearch {
    pub(super) fn persona_payload(
        &self,
        persona: SearchPersona,
        unique_id: &str,
        entity: &ArchivedEntity,
        query_tokens: Option<&[String]>,
    ) -> Option<JsonValue> {
        match persona {
            SearchPersona::Analyst => Self::analyst_payload(entity, query_tokens),
            SearchPersona::Engineer => Some(self.engineer_payload(unique_id, entity)),
            SearchPersona::Governance => Some(self.governance_payload(unique_id, entity)),
            SearchPersona::Default => None,
        }
    }

    #[allow(clippy::too_many_lines)]
    fn analyst_payload(
        entity: &ArchivedEntity,
        query_tokens: Option<&[String]>,
    ) -> Option<JsonValue> {
        let mut obj = serde_json::Map::new();
        obj.insert(
            "focus".to_string(),
            JsonValue::String("business_discovery".to_string()),
        );

        if let Some(definition) = entity
            .description_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| truncate_str(s, 180))
        {
            obj.insert(
                "business_definition".to_string(),
                JsonValue::String(definition),
            );
        }

        let mut metric_names: Vec<String> = Vec::new();
        let mut measure_names: Vec<String> = Vec::new();
        if let Some(nova) = entity.nova_meta() {
            measure_names = nova
                .measures
                .iter()
                .map(|m| m.name.as_str().to_string())
                .take(6)
                .collect();
            if !measure_names.is_empty() {
                obj.insert(
                    "candidate_measures".to_string(),
                    serde_json::json!(measure_names),
                );
            }

            if let Some(metric) = nova.metric.as_ref() {
                metric_names.push(metric.name.as_str().to_string());
            }
            for metric in nova.metrics.iter().take(5) {
                metric_names.push(metric.name.as_str().to_string());
            }
            if !metric_names.is_empty() {
                obj.insert(
                    "candidate_metrics".to_string(),
                    serde_json::json!(metric_names),
                );
            }

            let grain = Self::preferred_grain(nova);
            if let Some(time_field) = grain.and_then(|g| {
                g.time_field
                    .as_ref()
                    .map(rkyv::string::ArchivedString::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
            }) {
                obj.insert(
                    "time_field".to_string(),
                    JsonValue::String(time_field.to_string()),
                );
            }
            let key_dimensions =
                Self::analyst_key_dimensions(entity, grain, &metric_names, &measure_names);
            if !key_dimensions.is_empty() {
                obj.insert(
                    "key_dimensions".to_string(),
                    serde_json::json!(key_dimensions),
                );
            }

            let signals = AnalystSelectionSignals {
                has_metric_definition: Self::has_metric_definition(nova),
                has_measure_definition: Self::has_measure_definition(nova),
                has_grain: grain.is_some_and(Self::grain_has_signal),
                has_time_field: grain
                    .and_then(|g| {
                        g.time_field
                            .as_ref()
                            .map(rkyv::string::ArchivedString::as_str)
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                    })
                    .is_some(),
                dimension_overlap: query_tokens
                    .filter(|tokens| !tokens.is_empty())
                    .map_or(0usize, |tokens| {
                        Self::dimension_overlap_count(&key_dimensions, tokens)
                    }),
            };
            let confidence_band = Self::analyst_confidence_band(signals);

            obj.insert(
                "selection_signals".to_string(),
                serde_json::json!({
                    "has_metric_definition": signals.has_metric_definition,
                    "has_measure_definition": signals.has_measure_definition,
                    "has_grain": signals.has_grain,
                    "has_time_field": signals.has_time_field,
                    "dimension_overlap": signals.dimension_overlap,
                    "confidence_band": confidence_band,
                }),
            );
            obj.insert(
                "selection_rationale".to_string(),
                JsonValue::String(Self::analyst_selection_rationale(signals)),
            );
        } else {
            let key_dimensions =
                Self::analyst_key_dimensions(entity, None, &metric_names, &measure_names);
            if !key_dimensions.is_empty() {
                obj.insert(
                    "key_dimensions".to_string(),
                    serde_json::json!(key_dimensions),
                );
            }
        }

        compact_json_object(&mut obj);
        if obj.len() <= 1 {
            None
        } else {
            Some(JsonValue::Object(obj))
        }
    }

    #[allow(clippy::too_many_lines)]
    fn engineer_payload(&self, unique_id: &str, entity: &ArchivedEntity) -> JsonValue {
        let upstream_count = self.upstream_count(unique_id);
        let downstream_count = self.downstream_count(unique_id);
        let blast_radius_count = upstream_count + downstream_count;
        let has_lineage = upstream_count > 0 || downstream_count > 0;
        let has_compiled_sql = entity.has_compiled_sql();
        let tests = self.tests_summary(unique_id, entity);
        let model_tests = tests
            .get("model_tests")
            .and_then(JsonValue::as_u64)
            .unwrap_or(0);
        let column_tests = tests
            .get("column_tests")
            .and_then(JsonValue::as_u64)
            .unwrap_or(0);
        let tests_total = model_tests + column_tests;
        let doc_coverage_pct = Self::doc_coverage(entity)
            .get("coverage_pct")
            .and_then(JsonValue::as_f64)
            .unwrap_or(0.0);
        let entity_json = entity.to_json_value();
        let has_owner = path_present(&entity_json, "meta.owner");
        let has_primary_key = !Self::primary_key_columns(entity).is_empty();
        let missing_required_fields = self
            .nova_required_missing_from_json(entity.resource_type_str().unwrap_or(""), &entity_json)
            .len();

        let mut risk_points = 0usize;
        if blast_radius_count >= 20 {
            risk_points += 3;
        } else if blast_radius_count >= 8 {
            risk_points += 2;
        } else if blast_radius_count >= 3 {
            risk_points += 1;
        }
        if tests_total == 0 {
            risk_points += 2;
        }
        if !has_compiled_sql {
            risk_points += 1;
        }
        if doc_coverage_pct < 50.0 {
            risk_points += 1;
        }
        if missing_required_fields > 0 {
            risk_points += 1;
        }
        let change_risk = if risk_points >= 5 {
            "high"
        } else if risk_points >= 3 {
            "medium"
        } else {
            "low"
        };

        let mut readiness_points = 0usize;
        if has_compiled_sql {
            readiness_points += 1;
        }
        if tests_total > 0 {
            readiness_points += 1;
        }
        if doc_coverage_pct >= 80.0 {
            readiness_points += 1;
        }
        if has_owner {
            readiness_points += 1;
        }
        if has_primary_key {
            readiness_points += 1;
        }
        if has_lineage {
            readiness_points += 1;
        }
        if missing_required_fields == 0 {
            readiness_points += 1;
        }
        let readiness_band = Self::engineer_readiness_band(readiness_points);
        let selection_rationale = Self::engineer_selection_rationale(
            blast_radius_count,
            tests_total,
            has_compiled_sql,
            doc_coverage_pct,
            has_owner,
            has_primary_key,
            missing_required_fields,
        );
        let recommended_tools = Self::engineer_recommended_tools(
            blast_radius_count,
            tests_total,
            has_compiled_sql,
            doc_coverage_pct,
            missing_required_fields,
        );

        let mut obj = serde_json::Map::new();
        obj.insert(
            "focus".to_string(),
            JsonValue::String("implementation_impact".to_string()),
        );
        obj.insert(
            "blast_radius_count".to_string(),
            JsonValue::from(blast_radius_count as u64),
        );
        obj.insert(
            "change_risk".to_string(),
            JsonValue::String(change_risk.to_string()),
        );
        obj.insert(
            "readiness_band".to_string(),
            JsonValue::String(readiness_band.to_string()),
        );
        obj.insert("impacted_tests".to_string(), tests);
        obj.insert(
            "selection_signals".to_string(),
            serde_json::json!({
                "upstream_count": upstream_count,
                "downstream_count": downstream_count,
                "has_lineage": has_lineage,
                "tests_total": tests_total,
                "documentation_coverage_pct": doc_coverage_pct,
                "has_owner": has_owner,
                "has_primary_key": has_primary_key,
                "missing_required_fields": missing_required_fields,
            }),
        );
        obj.insert(
            "selection_rationale".to_string(),
            JsonValue::String(selection_rationale),
        );
        if !recommended_tools.is_empty() {
            obj.insert(
                "recommended_tools".to_string(),
                serde_json::json!(recommended_tools),
            );
        }
        obj.insert(
            "has_compiled_sql".to_string(),
            JsonValue::from(has_compiled_sql),
        );
        if let Some(path) = entity.original_file_path_str() {
            obj.insert("file_path".to_string(), JsonValue::String(path.to_string()));
        }
        compact_json_object(&mut obj);
        JsonValue::Object(obj)
    }

    #[allow(clippy::too_many_lines)]
    fn governance_payload(&self, unique_id: &str, entity: &ArchivedEntity) -> JsonValue {
        let entity_json = entity.to_json_value();
        let upstream_count = self.upstream_count(unique_id);
        let weights = self.config.metadata_score.persona_weights.governance;
        let metadata_score = self.score_entity(unique_id, &entity_json, false, false, weights);
        let tests = self.tests_summary(unique_id, entity);
        let missing = self.nova_required_missing_from_json(
            entity.resource_type_str().unwrap_or(""),
            &entity_json,
        );
        let missing_json = serde_json::json!(missing);
        let governance = entity.nova_meta().and_then(|n| n.governance.as_ref());
        let has_pii = governance.and_then(|g| g.pii.as_ref()).is_some();
        let compliance_len = governance.map_or(0usize, |g| g.compliance.len());
        let doc_coverage = Self::doc_coverage(entity)
            .get("coverage_pct")
            .and_then(JsonValue::as_f64)
            .unwrap_or(0.0);
        let tests_total = tests
            .get("model_tests")
            .and_then(JsonValue::as_u64)
            .unwrap_or(0)
            + tests
                .get("column_tests")
                .and_then(JsonValue::as_u64)
                .unwrap_or(0);
        let gate_policy = &self.config.governance_gate;
        let owner_present = path_present(&entity_json, "meta.owner");
        let required_fields_present = missing.is_empty();
        let tests_present = tests_total > 0;
        let compliance_tags_present = !has_pii || compliance_len > 0;
        let ref_calls = sql_for_matching(&entity_json)
            .map(extract_ref_calls)
            .unwrap_or_default();
        let has_ref_calls = !ref_calls.is_empty();
        let ref_calls_without_dependencies = has_ref_calls && upstream_count == 0;
        let possible_malformed_ref_syntax = ref_calls_without_dependencies
            && sql_for_matching(&entity_json).is_some_and(has_apparent_malformed_ref);

        let required_fields_pass = !gate_policy.require_required_fields || required_fields_present;
        let metadata_grade_pass = metadata_score.overall_score >= gate_policy.min_metadata_score;
        let docs_pass = doc_coverage >= gate_policy.min_documentation_coverage_pct;
        let tests_pass = !gate_policy.require_tests || tests_present;
        let owner_pass = !gate_policy.require_owner || owner_present;
        let compliance_pass = !gate_policy.require_compliance_for_pii || compliance_tags_present;

        let mut failed_reasons = Vec::new();
        if !required_fields_pass {
            failed_reasons.push("missing_required_nova_fields");
        }
        if !metadata_grade_pass {
            failed_reasons.push("metadata_score_below_a_grade");
        }
        if !docs_pass {
            failed_reasons.push("documentation_coverage_below_threshold");
        }
        if !tests_pass {
            failed_reasons.push("test_coverage_missing");
        }
        if !owner_pass {
            failed_reasons.push("owner_missing");
        }
        if !compliance_pass {
            failed_reasons.push("pii_without_compliance_tags");
        }
        let (gate_status, blocking_reasons, advisory_reasons) = if failed_reasons.is_empty() {
            ("pass", Vec::new(), Vec::new())
        } else if gate_policy.block_on_failure {
            ("fail", failed_reasons.clone(), Vec::new())
        } else {
            ("advisory", Vec::new(), failed_reasons.clone())
        };

        let mut risk_points = 0usize;
        if !missing.is_empty() {
            risk_points += 2;
        }
        if metadata_score.overall_score < 50 {
            risk_points += 3;
        } else if metadata_score.overall_score < 70 {
            risk_points += 2;
        } else if metadata_score.overall_score < 85 {
            risk_points += 1;
        }
        if has_pii && compliance_len == 0 {
            risk_points += 2;
        }
        if tests_total == 0 {
            risk_points += 1;
        }
        if doc_coverage < 30.0 {
            risk_points += 2;
        } else if doc_coverage < 60.0 {
            risk_points += 1;
        }
        if ref_calls_without_dependencies {
            risk_points += 1;
        }

        let policy_risk = if risk_points >= 7 {
            "critical"
        } else if risk_points >= 5 {
            "high"
        } else if risk_points >= 3 {
            "medium"
        } else {
            "low"
        };

        let mut obj = serde_json::Map::new();
        obj.insert(
            "focus".to_string(),
            JsonValue::String("governance_assurance".to_string()),
        );
        obj.insert(
            "policy_risk".to_string(),
            JsonValue::String(policy_risk.to_string()),
        );
        obj.insert(
            "gate_status".to_string(),
            JsonValue::String(gate_status.to_string()),
        );
        obj.insert(
            "gate_signals".to_string(),
            serde_json::json!({
                "required_fields_present": required_fields_present,
                "required_fields_pass": required_fields_pass,
                "owner_present": owner_present,
                "tests_present": tests_present,
                "compliance_tags_present": compliance_tags_present,
                "metadata_grade_pass": metadata_grade_pass,
                "docs_pass": docs_pass,
                "tests_pass": tests_pass,
                "owner_pass": owner_pass,
                "compliance_pass": compliance_pass,
                "has_ref_calls": has_ref_calls,
                "ref_calls_without_dependencies": ref_calls_without_dependencies
            }),
        );
        obj.insert(
            "lineage_health".to_string(),
            serde_json::json!({
                "upstream_dependency_count": upstream_count,
                "has_ref_calls": has_ref_calls,
                "ref_calls_without_dependencies": ref_calls_without_dependencies,
                "possible_malformed_ref_syntax": possible_malformed_ref_syntax
            }),
        );
        obj.insert(
            "manifest_health".to_string(),
            serde_json::json!({
                "is_healthy": self
                    .manifest_health
                    .get("is_healthy")
                    .and_then(JsonValue::as_bool)
                    .unwrap_or(true),
                "models_ref_calls_without_dependencies": self
                    .manifest_health
                    .get("models_ref_calls_without_dependencies")
                    .and_then(JsonValue::as_u64)
                    .unwrap_or(0)
            }),
        );
        obj.insert(
            "missing_governance_fields".to_string(),
            missing_json.clone(),
        );
        obj.insert(
            "blocking_reasons".to_string(),
            serde_json::json!(blocking_reasons),
        );
        obj.insert(
            "advisory_reasons".to_string(),
            serde_json::json!(advisory_reasons),
        );
        obj.insert(
            "metadata_grade".to_string(),
            JsonValue::String(metadata_score.grade),
        );
        obj.insert(
            "metadata_score".to_string(),
            JsonValue::from(metadata_score.overall_score),
        );
        obj.insert(
            "documentation_coverage_pct".to_string(),
            JsonValue::from(doc_coverage),
        );
        obj.insert("test_coverage".to_string(), tests);
        let mut quality_warnings = Vec::new();
        if ref_calls_without_dependencies {
            quality_warnings.push("ref_calls_without_dependencies");
        }
        if possible_malformed_ref_syntax {
            quality_warnings.push("possible_malformed_ref_syntax");
        }
        obj.insert(
            "quality_warnings".to_string(),
            serde_json::json!(quality_warnings),
        );
        obj.insert(
            "gate_policy".to_string(),
            serde_json::json!({
                "min_metadata_score": gate_policy.min_metadata_score,
                "min_documentation_coverage_pct": gate_policy.min_documentation_coverage_pct,
                "require_tests": gate_policy.require_tests,
                "require_owner": gate_policy.require_owner,
                "require_required_fields": gate_policy.require_required_fields,
                "require_compliance_for_pii": gate_policy.require_compliance_for_pii,
                "block_on_failure": gate_policy.block_on_failure
            }),
        );

        if let Some(gov) = governance {
            if let Some(sensitivity) = gov
                .sensitivity
                .as_ref()
                .map(rkyv::string::ArchivedString::as_str)
            {
                obj.insert(
                    "sensitivity".to_string(),
                    JsonValue::String(sensitivity.to_string()),
                );
            }
            if let Some(pii) = gov
                .pii
                .as_ref()
                .map(rkyv::string::ArchivedString::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                obj.insert("pii".to_string(), JsonValue::String(pii.to_string()));
            }
            if !gov.compliance.is_empty() {
                let compliance: Vec<String> = gov
                    .compliance
                    .iter()
                    .map(|s| s.as_str().to_string())
                    .collect();
                obj.insert("compliance".to_string(), serde_json::json!(compliance));
            }
        }
        compact_json_object(&mut obj);
        if !obj.contains_key("missing_governance_fields") {
            obj.insert("missing_governance_fields".to_string(), missing_json);
        }
        if !obj.contains_key("blocking_reasons") {
            obj.insert("blocking_reasons".to_string(), JsonValue::Array(Vec::new()));
        }
        JsonValue::Object(obj)
    }

    fn preferred_grain(nova: &ArchivedNovaMeta) -> Option<&ArchivedNovaGrain> {
        if let Some(grain) = nova.grain.as_ref().filter(|g| Self::grain_has_signal(g)) {
            return Some(grain);
        }
        if let Some(grain) = nova
            .metric
            .as_ref()
            .and_then(|metric| metric.grain.as_ref())
            .filter(|g| Self::grain_has_signal(g))
        {
            return Some(grain);
        }
        nova.metrics
            .iter()
            .filter_map(|metric| metric.grain.as_ref())
            .find(|grain| Self::grain_has_signal(grain))
    }

    fn analyst_key_dimensions(
        entity: &ArchivedEntity,
        grain: Option<&ArchivedNovaGrain>,
        metric_names: &[String],
        measure_names: &[String],
    ) -> Vec<String> {
        const MAX_DIMENSIONS: usize = 6;
        let mut out: Vec<String> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        let excluded: HashSet<String> = metric_names
            .iter()
            .chain(measure_names.iter())
            .map(|name| name.trim().to_ascii_lowercase())
            .filter(|name| !name.is_empty())
            .collect();

        if let Some(grain) = grain {
            for dimension in grain
                .dimensions
                .iter()
                .map(rkyv::string::ArchivedString::as_str)
            {
                Self::push_dimension_candidate(
                    &mut out,
                    &mut seen,
                    &excluded,
                    dimension,
                    MAX_DIMENSIONS,
                );
                if out.len() >= MAX_DIMENSIONS {
                    return out;
                }
            }
        }

        if out.is_empty() {
            for column in entity.column_meta() {
                let role = column
                    .role
                    .as_ref()
                    .map(rkyv::string::ArchivedString::as_str)
                    .map(str::trim)
                    .map(str::to_ascii_lowercase);
                let include = column.primary_key
                    || matches!(role.as_deref(), Some("dimension" | "identifier" | "time"));
                if !include {
                    continue;
                }
                Self::push_dimension_candidate(
                    &mut out,
                    &mut seen,
                    &excluded,
                    column.name.as_str(),
                    MAX_DIMENSIONS,
                );
                if out.len() >= MAX_DIMENSIONS {
                    return out;
                }
            }
        }

        if out.is_empty() {
            for column_name in entity.column_names_iter() {
                Self::push_dimension_candidate(
                    &mut out,
                    &mut seen,
                    &excluded,
                    column_name,
                    MAX_DIMENSIONS,
                );
                if out.len() >= MAX_DIMENSIONS {
                    break;
                }
            }
        }

        out
    }

    fn push_dimension_candidate(
        out: &mut Vec<String>,
        seen: &mut HashSet<String>,
        excluded: &HashSet<String>,
        candidate: &str,
        limit: usize,
    ) {
        if out.len() >= limit {
            return;
        }
        let trimmed = candidate.trim();
        if trimmed.is_empty() {
            return;
        }
        let canonical = trimmed.to_ascii_lowercase();
        if excluded.contains(&canonical) {
            return;
        }
        if !seen.insert(canonical) {
            return;
        }
        out.push(trimmed.to_string());
    }

    fn has_metric_definition(nova: &ArchivedNovaMeta) -> bool {
        nova.metric.iter().chain(nova.metrics.iter()).any(|metric| {
            metric
                .description
                .as_ref()
                .map(rkyv::string::ArchivedString::as_str)
                .map(str::trim)
                .is_some_and(|s| !s.is_empty())
                || metric
                    .expression
                    .as_ref()
                    .map(rkyv::string::ArchivedString::as_str)
                    .map(str::trim)
                    .is_some_and(|s| !s.is_empty())
        })
    }

    fn has_measure_definition(nova: &ArchivedNovaMeta) -> bool {
        nova.measures.iter().any(|measure| {
            measure
                .description
                .as_ref()
                .map(rkyv::string::ArchivedString::as_str)
                .map(str::trim)
                .is_some_and(|s| !s.is_empty())
                || measure
                    .expression
                    .as_ref()
                    .map(rkyv::string::ArchivedString::as_str)
                    .map(str::trim)
                    .is_some_and(|s| !s.is_empty())
                || measure
                    .field
                    .as_ref()
                    .map(rkyv::string::ArchivedString::as_str)
                    .map(str::trim)
                    .is_some_and(|s| !s.is_empty())
        })
    }

    fn grain_has_signal(grain: &ArchivedNovaGrain) -> bool {
        !grain.primary_key.is_empty()
            || !grain.dimensions.is_empty()
            || grain
                .time_field
                .as_ref()
                .map(rkyv::string::ArchivedString::as_str)
                .map(str::trim)
                .is_some_and(|s| !s.is_empty())
    }

    fn dimension_overlap_count(dimensions: &[String], query_tokens: &[String]) -> usize {
        if dimensions.is_empty() || query_tokens.is_empty() {
            return 0;
        }
        let query_token_set: HashSet<&str> = query_tokens.iter().map(String::as_str).collect();
        dimensions
            .iter()
            .filter(|dimension| {
                tokenize_alnum_lowercase(dimension, 2)
                    .iter()
                    .any(|token| query_token_set.contains(token.as_str()))
            })
            .count()
    }

    fn analyst_confidence_band(signals: AnalystSelectionSignals) -> &'static str {
        let mut points = 0usize;
        if signals.has_metric_definition {
            points += 2;
        }
        if signals.has_measure_definition {
            points += 1;
        }
        if signals.has_grain {
            points += 1;
        }
        if signals.has_time_field {
            points += 1;
        }
        if signals.dimension_overlap > 0 {
            points += 1;
        }
        if signals.dimension_overlap > 1 {
            points += 1;
        }
        if points >= 5 {
            "high"
        } else if points >= 3 {
            "medium"
        } else {
            "low"
        }
    }

    fn analyst_selection_rationale(signals: AnalystSelectionSignals) -> String {
        let mut parts: Vec<String> = Vec::new();
        if signals.has_metric_definition {
            parts.push("includes metric definitions".to_string());
        }
        if signals.has_measure_definition {
            parts.push("includes measure definitions".to_string());
        }
        if signals.has_grain {
            parts.push("declares semantic grain".to_string());
        }
        if signals.has_time_field {
            parts.push("has an explicit time field".to_string());
        }
        if signals.dimension_overlap > 0 {
            parts.push(format!(
                "{} query-aligned dimension(s)",
                signals.dimension_overlap
            ));
        }

        if parts.is_empty() {
            "Limited semantic signals; validate this entity with `get_context` before querying."
                .to_string()
        } else {
            format!("Selection signals: {}.", parts.join(", "))
        }
    }

    fn engineer_readiness_band(readiness_points: usize) -> &'static str {
        if readiness_points >= 6 {
            "high"
        } else if readiness_points >= 4 {
            "medium"
        } else {
            "low"
        }
    }

    fn engineer_selection_rationale(
        blast_radius_count: usize,
        tests_total: u64,
        has_compiled_sql: bool,
        doc_coverage_pct: f64,
        has_owner: bool,
        has_primary_key: bool,
        missing_required_fields: usize,
    ) -> String {
        let mut parts: Vec<String> = Vec::new();
        if blast_radius_count >= 20 {
            parts.push("very large blast radius".to_string());
        } else if blast_radius_count >= 8 {
            parts.push("moderate blast radius".to_string());
        }
        if tests_total == 0 {
            parts.push("no local test coverage".to_string());
        }
        if !has_compiled_sql {
            parts.push("compiled SQL missing".to_string());
        }
        if doc_coverage_pct < 80.0 {
            parts.push("documentation coverage below target".to_string());
        }
        if !has_owner {
            parts.push("owner metadata missing".to_string());
        }
        if !has_primary_key {
            parts.push("primary key not declared".to_string());
        }
        if missing_required_fields > 0 {
            parts.push(format!(
                "{missing_required_fields} required Nova field(s) missing"
            ));
        }

        if parts.is_empty() {
            "Implementation readiness signals are strong for this entity.".to_string()
        } else {
            format!("Implementation signals: {}.", parts.join(", "))
        }
    }

    fn engineer_recommended_tools(
        blast_radius_count: usize,
        tests_total: u64,
        has_compiled_sql: bool,
        doc_coverage_pct: f64,
        missing_required_fields: usize,
    ) -> Vec<&'static str> {
        let mut tools: Vec<&'static str> = vec!["get_lineage", "get_columns"];
        if blast_radius_count >= 8 {
            tools.push("get_impact");
        }
        if !has_compiled_sql {
            tools.push("get_sql");
        }
        if tests_total == 0 {
            tools.push("get_test_coverage");
        }
        if doc_coverage_pct < 80.0 || missing_required_fields > 0 {
            tools.push("get_metadata_score");
        }
        tools
    }
}
