use std::collections::HashSet;

use serde_json::Value as JsonValue;

use crate::manifest::entity::{ArchivedEntity, ArchivedNovaGrain, ArchivedNovaMeta};
use crate::manifest::lineage_sql::{extract_ref_calls, sql_for_matching};
use crate::utils::{SearchPersona, tokenize_alnum_lowercase};

use super::core::ManifestSearch;

const NOVA_SUMMARY_SYNONYM_LIMIT: usize = 5;

#[derive(Clone, Copy)]
#[allow(clippy::struct_excessive_bools)]
struct SummaryProfile {
    include_package_name: bool,
    include_alias: bool,
    include_relation_fields: bool,
    include_original_file_path: bool,
    include_description: bool,
    description_limit: usize,
    include_columns_total: bool,
    include_primary_key_columns: bool,
    include_columns: bool,
    columns_limit: usize,
    include_columns_truncated: bool,
    include_layer: bool,
    include_materialization: bool,
    include_has_compiled_sql: bool,
    include_build_config: bool,
    include_upstream_downstream: bool,
    include_tests_summary: bool,
    include_doc_coverage: bool,
    include_documentation_status: bool,
    include_metadata_score: bool,
    include_nova_required_missing: bool,
    include_nova_summary: bool,
    include_nova_domains: bool,
    include_nova_role: bool,
    include_nova_measures: bool,
    include_nova_metrics: bool,
    include_nova_governance: bool,
    include_nova_tier: bool,
    include_nova_canonical: bool,
    include_has_nova_meta: bool,
    include_persona_payload: bool,
}

#[derive(Clone, Copy)]
#[allow(clippy::struct_excessive_bools)]
struct AnalystSelectionSignals {
    has_metric_definition: bool,
    has_measure_definition: bool,
    has_grain: bool,
    has_time_field: bool,
    dimension_overlap: usize,
}

impl SummaryProfile {
    fn empty() -> Self {
        Self {
            include_package_name: false,
            include_alias: false,
            include_relation_fields: false,
            include_original_file_path: false,
            include_description: false,
            description_limit: 0,
            include_columns_total: false,
            include_primary_key_columns: false,
            include_columns: false,
            columns_limit: 0,
            include_columns_truncated: false,
            include_layer: false,
            include_materialization: false,
            include_has_compiled_sql: false,
            include_build_config: false,
            include_upstream_downstream: false,
            include_tests_summary: false,
            include_doc_coverage: false,
            include_documentation_status: false,
            include_metadata_score: false,
            include_nova_required_missing: false,
            include_nova_summary: false,
            include_nova_domains: false,
            include_nova_role: false,
            include_nova_measures: false,
            include_nova_metrics: false,
            include_nova_governance: false,
            include_nova_tier: false,
            include_nova_canonical: false,
            include_has_nova_meta: false,
            include_persona_payload: false,
        }
    }

    fn for_persona(persona: SearchPersona) -> Self {
        match persona {
            SearchPersona::Analyst => Self::analyst(),
            SearchPersona::Engineer => Self::engineer(),
            SearchPersona::Governance => Self::governance(),
            SearchPersona::Default => Self::default_persona(),
        }
    }

    fn analyst() -> Self {
        Self {
            include_relation_fields: true,
            include_description: true,
            description_limit: 120,
            include_columns_total: true,
            include_primary_key_columns: true,
            include_columns: false,
            columns_limit: 0,
            include_columns_truncated: false,
            include_nova_domains: true,
            include_nova_role: true,
            include_nova_measures: false,
            include_nova_metrics: false,
            include_persona_payload: true,
            ..Self::empty()
        }
    }

    fn engineer() -> Self {
        Self {
            include_original_file_path: true,
            include_package_name: true,
            include_layer: true,
            include_alias: true,
            include_relation_fields: true,
            include_materialization: true,
            include_has_compiled_sql: true,
            include_build_config: false,
            include_upstream_downstream: true,
            include_tests_summary: true,
            include_doc_coverage: true,
            include_persona_payload: true,
            ..Self::empty()
        }
    }

    fn governance() -> Self {
        Self {
            include_package_name: true,
            include_documentation_status: true,
            include_tests_summary: true,
            include_doc_coverage: true,
            include_metadata_score: true,
            include_nova_required_missing: true,
            include_nova_governance: true,
            include_nova_domains: true,
            include_nova_tier: true,
            include_nova_canonical: true,
            include_has_nova_meta: true,
            include_persona_payload: true,
            ..Self::empty()
        }
    }

    fn default_persona() -> Self {
        Self {
            include_package_name: true,
            include_alias: true,
            include_relation_fields: true,
            include_original_file_path: true,
            include_description: true,
            description_limit: 120,
            ..Self::empty()
        }
    }

    fn standard() -> Self {
        Self {
            include_package_name: true,
            include_alias: true,
            include_relation_fields: true,
            include_original_file_path: true,
            include_description: true,
            description_limit: 120,
            include_primary_key_columns: true,
            include_nova_summary: true,
            ..Self::empty()
        }
    }
}

impl ManifestSearch {
    #[allow(clippy::too_many_lines)]
    pub(crate) fn summary_from_archived(
        &self,
        unique_id: &str,
        entity: Option<&ArchivedEntity>,
        persona: SearchPersona,
        query_tokens: Option<&[String]>,
    ) -> JsonValue {
        let Some(entity) = entity else {
            return Self::empty_summary(unique_id);
        };

        self.build_summary(
            unique_id,
            entity,
            SummaryProfile::for_persona(persona),
            Some(persona),
            query_tokens,
        )
    }

    pub(crate) fn summary_for_standard(
        &self,
        unique_id: &str,
        entity: &ArchivedEntity,
    ) -> JsonValue {
        self.build_summary(unique_id, entity, SummaryProfile::standard(), None, None)
    }

    #[allow(clippy::too_many_lines)]
    fn build_summary(
        &self,
        unique_id: &str,
        entity: &ArchivedEntity,
        profile: SummaryProfile,
        persona: Option<SearchPersona>,
        query_tokens: Option<&[String]>,
    ) -> JsonValue {
        let mut obj = serde_json::Map::new();
        let mut entity_json_cache: Option<JsonValue> = None;
        Self::insert_identity_fields(&mut obj, unique_id, entity);

        if profile.include_package_name {
            obj.insert(
                "package_name".to_string(),
                json_string_or_null(entity.package_name_str()),
            );
        }
        if profile.include_alias {
            obj.insert("alias".to_string(), json_string_or_null(entity.alias_str()));
        }
        if profile.include_relation_fields {
            Self::insert_relation_fields(&mut obj, entity);
        }
        if profile.include_original_file_path {
            obj.insert(
                "original_file_path".to_string(),
                json_string_or_null(entity.original_file_path_str()),
            );
        }
        if profile.include_description
            && let Some(desc) = entity
                .description_str()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| {
                    if profile.description_limit > 0 {
                        truncate_str(s, profile.description_limit)
                    } else {
                        s.to_string()
                    }
                })
        {
            obj.insert("description".to_string(), JsonValue::String(desc));
        }
        if profile.include_columns_total {
            let columns_total = entity.column_names_iter().count();
            if columns_total > 0 {
                obj.insert(
                    "columns_total".to_string(),
                    JsonValue::from(columns_total as u64),
                );
            }
        }
        if profile.include_primary_key_columns {
            let primary_key_columns = Self::primary_key_columns(entity);
            if !primary_key_columns.is_empty() {
                obj.insert(
                    "primary_key_columns".to_string(),
                    serde_json::json!(primary_key_columns),
                );
            }
        }
        if profile.include_columns {
            let mut columns = Self::column_detail_summaries(entity);
            let columns_truncated =
                profile.columns_limit > 0 && columns.len() > profile.columns_limit;
            if columns_truncated {
                columns.truncate(profile.columns_limit);
            }
            if !columns.is_empty() {
                obj.insert("columns".to_string(), JsonValue::Array(columns));
                if profile.include_columns_truncated {
                    obj.insert(
                        "columns_truncated".to_string(),
                        JsonValue::from(columns_truncated),
                    );
                }
            }
        }
        if profile.include_layer
            && let Some(layer) = self.layer_for(entity)
        {
            obj.insert("layer".to_string(), JsonValue::String(layer));
        }
        if profile.include_materialization {
            obj.insert(
                "materialization".to_string(),
                json_string_or_null(entity.materialization_str()),
            );
        }
        if profile.include_has_compiled_sql {
            obj.insert(
                "has_compiled_sql".to_string(),
                JsonValue::from(entity.has_compiled_sql()),
            );
        }
        if profile.include_build_config {
            let entity_json = entity_json_cache.get_or_insert_with(|| entity.to_json_value());
            let build_config = Self::build_config_summary(entity_json);
            if !build_config.is_null() {
                obj.insert("build_config".to_string(), build_config);
            }
        }
        if profile.include_upstream_downstream {
            obj.insert(
                "upstream_count".to_string(),
                JsonValue::from(self.upstream_count(unique_id) as u64),
            );
            obj.insert(
                "downstream_count".to_string(),
                JsonValue::from(self.downstream_count(unique_id) as u64),
            );
        }
        if profile.include_tests_summary {
            obj.insert(
                "tests_summary".to_string(),
                self.tests_summary(unique_id, entity),
            );
        }
        if profile.include_doc_coverage {
            obj.insert("doc_coverage".to_string(), Self::doc_coverage(entity));
        }
        if profile.include_documentation_status {
            obj.insert(
                "documentation_status".to_string(),
                Self::documentation_status(entity),
            );
        }
        if profile.include_metadata_score {
            let entity_json = entity_json_cache.get_or_insert_with(|| entity.to_json_value());
            let weights = self.config.metadata_score.persona_weights.governance;
            let score = self.score_entity(unique_id, entity_json, false, false, weights);
            obj.insert(
                "metadata_score".to_string(),
                serde_json::json!({
                    "overall_score": score.overall_score,
                    "grade": score.grade
                }),
            );
        }
        if profile.include_nova_required_missing {
            let entity_json = entity_json_cache.get_or_insert_with(|| entity.to_json_value());
            let missing = self.nova_required_missing_from_json(
                entity.resource_type_str().unwrap_or(""),
                entity_json,
            );
            if !missing.is_empty() {
                obj.insert(
                    "nova_required_missing".to_string(),
                    serde_json::json!(missing),
                );
            }
        }
        let has_any_nova = profile.include_nova_domains
            || profile.include_nova_role
            || profile.include_nova_measures
            || profile.include_nova_metrics
            || profile.include_nova_governance
            || profile.include_nova_tier
            || profile.include_nova_canonical
            || profile.include_nova_summary
            || profile.include_has_nova_meta;
        if has_any_nova && let Some(nova) = entity.nova_meta() {
            if profile.include_nova_governance
                && let Some(gov) = Self::nova_governance_summary(nova)
            {
                obj.insert("nova_governance".to_string(), gov);
            }
            if profile.include_nova_domains && !nova.domains.is_empty() {
                let domains: Vec<String> = nova
                    .domains
                    .iter()
                    .map(|v| v.as_str().to_string())
                    .collect();
                obj.insert("nova_domains".to_string(), serde_json::json!(domains));
            }
            if profile.include_nova_tier
                && let Some(tier) = nova.tier.as_ref().map(rkyv::string::ArchivedString::as_str)
            {
                obj.insert("nova_tier".to_string(), JsonValue::String(tier.to_string()));
            }
            if profile.include_nova_canonical && nova.canonical {
                obj.insert("nova_canonical".to_string(), JsonValue::from(true));
            }
            if profile.include_nova_role
                && let Some(role) = nova.role.as_ref().map(rkyv::string::ArchivedString::as_str)
            {
                obj.insert("nova_role".to_string(), JsonValue::String(role.to_string()));
            }
            if profile.include_nova_measures {
                let measures = Self::nova_measures_summary(nova);
                if !measures.is_empty() {
                    obj.insert("nova_measures".to_string(), JsonValue::Array(measures));
                }
            }
            if profile.include_nova_metrics {
                let metrics = Self::nova_metrics_summary(nova);
                if !metrics.is_empty() {
                    obj.insert("nova_metrics".to_string(), JsonValue::Array(metrics));
                }
            }
            if profile.include_nova_summary
                && let Some(nova_summary) = Self::nova_summary(nova)
            {
                obj.insert("nova_summary".to_string(), nova_summary);
            }
        }
        if profile.include_has_nova_meta {
            obj.insert(
                "has_nova_meta".to_string(),
                JsonValue::from(entity.nova_meta().is_some()),
            );
        }
        if profile.include_persona_payload
            && let Some(persona) = persona
            && let Some(payload) = self.persona_payload(persona, unique_id, entity, query_tokens)
        {
            obj.insert("persona_payload".to_string(), payload);
        }

        compact_json_object(&mut obj);
        JsonValue::Object(obj)
    }

    fn persona_payload(
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

    fn insert_identity_fields(
        obj: &mut serde_json::Map<String, JsonValue>,
        unique_id: &str,
        entity: &ArchivedEntity,
    ) {
        obj.insert(
            "unique_id".to_string(),
            JsonValue::String(unique_id.to_string()),
        );
        obj.insert("name".to_string(), json_string_or_null(entity.name_str()));
        obj.insert(
            "resource_type".to_string(),
            json_string_or_null(entity.resource_type_str()),
        );
    }

    fn insert_relation_fields(
        obj: &mut serde_json::Map<String, JsonValue>,
        entity: &ArchivedEntity,
    ) {
        let relation_name = entity
            .relation_name_str()
            .and_then(|s| if s.trim().is_empty() { None } else { Some(s) });
        obj.insert(
            "relation_name".to_string(),
            json_string_or_null(relation_name),
        );
        if relation_name.is_none() {
            obj.insert(
                "database".to_string(),
                json_string_or_null(entity.database_str()),
            );
            obj.insert(
                "schema".to_string(),
                json_string_or_null(entity.schema_str()),
            );
        }
    }

    fn upstream_count(&self, unique_id: &str) -> usize {
        self.parent_map.get(unique_id).map_or(0, Vec::len)
    }

    fn downstream_count(&self, unique_id: &str) -> usize {
        self.child_map.get(unique_id).map_or(0, Vec::len)
    }

    fn tests_summary(&self, unique_id: &str, entity: &ArchivedEntity) -> JsonValue {
        let model_tests = self.tests_by_entity.get(unique_id).map_or(0, Vec::len);
        let mut column_tests = 0usize;
        for column in entity.column_names_iter() {
            let key = format!("{unique_id}:{column}");
            column_tests += self.tests_by_column.get(&key).map_or(0, Vec::len);
        }
        serde_json::json!({
            "model_tests": model_tests,
            "column_tests": column_tests
        })
    }

    fn doc_coverage(entity: &ArchivedEntity) -> JsonValue {
        let columns_total = entity.column_names_iter().count();
        let documented = entity.columns_documented_count();
        serde_json::json!({
            "columns_total": columns_total,
            "columns_documented": documented,
            "coverage_pct": if columns_total > 0 {
                #[allow(clippy::cast_precision_loss)]
                let documented = documented as f64;
                #[allow(clippy::cast_precision_loss)]
                let columns_total = columns_total as f64;
                ((documented / columns_total) * 10_000.0).round() / 100.0
            } else {
                0.0
            }
        })
    }

    fn primary_key_columns(entity: &ArchivedEntity) -> Vec<String> {
        let mut out = Vec::new();
        for column in entity.column_meta() {
            if column.primary_key {
                out.push(column.name.as_str().to_string());
            }
        }
        out
    }

    fn column_detail_summaries(entity: &ArchivedEntity) -> Vec<JsonValue> {
        let mut out = Vec::new();
        for column in entity.column_meta() {
            let mut obj = serde_json::Map::new();
            let mut has_detail = false;

            if let Some(desc) = column
                .description
                .as_ref()
                .map(rkyv::string::ArchivedString::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| truncate_str(s, 80))
            {
                obj.insert("description".to_string(), JsonValue::String(desc));
                has_detail = true;
            }

            if let Some(role) = column
                .role
                .as_ref()
                .map(rkyv::string::ArchivedString::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                obj.insert("role".to_string(), JsonValue::String(role.to_string()));
                has_detail = true;
            }

            if let Some(semantic_type) = column
                .semantic_type
                .as_ref()
                .map(rkyv::string::ArchivedString::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                obj.insert(
                    "semantic_type".to_string(),
                    JsonValue::String(semantic_type.to_string()),
                );
                has_detail = true;
            }

            if has_detail {
                obj.insert(
                    "name".to_string(),
                    JsonValue::String(column.name.as_str().to_string()),
                );
                out.push(JsonValue::Object(obj));
            }
        }
        out
    }

    fn build_config_summary(entity_json: &JsonValue) -> JsonValue {
        let config = entity_json.get("config").and_then(|c| c.as_object());
        let Some(config) = config else {
            return JsonValue::Null;
        };

        let mut obj = serde_json::Map::new();
        for (key, value) in config {
            if value_present(value) {
                obj.insert(key.clone(), value.clone());
            }
        }
        JsonValue::Object(obj)
    }

    fn documentation_status(entity: &ArchivedEntity) -> JsonValue {
        let has_desc = entity
            .description_str()
            .map(str::trim)
            .is_some_and(|s| !s.is_empty());
        let columns_total = entity.column_names_iter().count();
        let columns_documented = entity.columns_documented_count();
        serde_json::json!({
            "has_description": has_desc,
            "columns_total": columns_total,
            "columns_documented": columns_documented,
        })
    }

    pub(crate) fn layer_for(&self, entity: &ArchivedEntity) -> Option<String> {
        let rules = &self.compiled_layer_rules;
        if rules.is_empty() {
            return None;
        }
        let name = entity.name_str().unwrap_or("");
        let path = entity.original_file_path_str().unwrap_or("");
        let resource_type = entity.resource_type_str().unwrap_or("");
        for rule in rules {
            if let Some(rt) = rule.resource_type.as_ref()
                && !rt.eq_ignore_ascii_case(resource_type)
            {
                continue;
            }
            if let Some(tag) = rule.tag.as_ref()
                && !entity.tags_iter().any(|t| t.eq_ignore_ascii_case(tag))
            {
                continue;
            }
            if let Some(prefix) = rule.path_prefix.as_ref()
                && !path.starts_with(prefix)
            {
                continue;
            }
            if let Some(prefix) = rule.name_prefix.as_ref()
                && !name.starts_with(prefix)
            {
                continue;
            }
            if let Some(regex) = rule.name_regex.as_ref()
                && regex.find(name).is_none()
            {
                continue;
            }
            return Some(rule.layer.clone());
        }
        None
    }

    fn nova_required_missing_from_json(
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

        let nova_json = entity_json
            .get("meta")
            .and_then(|m| m.get("nova"))
            .cloned()
            .unwrap_or(JsonValue::Null);
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

    fn nova_measures_summary(nova: &ArchivedNovaMeta) -> Vec<JsonValue> {
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

    fn nova_metrics_summary(nova: &ArchivedNovaMeta) -> Vec<JsonValue> {
        let mut out = Vec::new();
        if let Some(metric) = nova.metric.as_ref() {
            Self::push_metric_summary(&mut out, metric);
        }
        for metric in nova.metrics.iter() {
            Self::push_metric_summary(&mut out, metric);
        }
        out
    }

    fn push_metric_summary(
        out: &mut Vec<JsonValue>,
        metric: &crate::manifest::entity::ArchivedNovaMetric,
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
        if let Some(grain) = metric.grain.as_ref().and_then(Self::nova_grain_summary) {
            obj.insert("grain".to_string(), grain);
        }
        if !obj.is_empty() {
            out.push(JsonValue::Object(obj));
        }
    }

    fn nova_summary(nova: &ArchivedNovaMeta) -> Option<JsonValue> {
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
        if let Some(tier) = nova.tier.as_ref().map(rkyv::string::ArchivedString::as_str) {
            obj.insert("tier".to_string(), JsonValue::String(tier.to_string()));
        }
        if nova.canonical {
            obj.insert("canonical".to_string(), JsonValue::from(true));
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

    fn nova_governance_summary(nova: &ArchivedNovaMeta) -> Option<JsonValue> {
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

    fn empty_summary(unique_id: &str) -> JsonValue {
        serde_json::json!({
            "unique_id": unique_id,
            "missing": true,
        })
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

fn json_string_or_null(value: Option<&str>) -> JsonValue {
    value.map_or(JsonValue::Null, |v| JsonValue::String(v.to_string()))
}

fn compact_json_value(value: &mut JsonValue) {
    match value {
        JsonValue::Object(obj) => {
            compact_json_object(obj);
        }
        JsonValue::Array(arr) => {
            for item in arr.iter_mut() {
                compact_json_value(item);
            }
            arr.retain(value_present);
        }
        _ => {}
    }
}

fn compact_json_object(obj: &mut serde_json::Map<String, JsonValue>) {
    for value in obj.values_mut() {
        compact_json_value(value);
    }
    obj.retain(|key, value| {
        matches!(
            key.as_str(),
            "missing_governance_fields"
                | "blocking_reasons"
                | "advisory_reasons"
                | "quality_warnings"
        ) || value_present(value)
    });
}

fn path_present(root: &JsonValue, path: &str) -> bool {
    let mut current = root;
    for part in path.split('.') {
        match current {
            JsonValue::Object(map) => {
                if let Some(next) = map.get(part) {
                    current = next;
                } else {
                    return false;
                }
            }
            _ => return false,
        }
    }
    value_present(current)
}

fn has_apparent_malformed_ref(sql: &str) -> bool {
    let lower = sql.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let mut cursor = 0usize;

    while let Some(found) = lower[cursor..].find("ref(") {
        let ref_idx = cursor + found;
        let mut left = ref_idx;
        while left > 0 && bytes[left - 1].is_ascii_whitespace() {
            left -= 1;
        }
        if left > 0 && bytes[left - 1] == b'{' {
            let mut second = left - 1;
            while second > 0 && bytes[second - 1].is_ascii_whitespace() {
                second -= 1;
            }
            if second == 0 || bytes[second - 1] != b'{' {
                return true;
            }
        }
        cursor = ref_idx + 4;
    }

    false
}

fn value_present(value: &JsonValue) -> bool {
    match value {
        JsonValue::Null => false,
        JsonValue::String(s) => !s.trim().is_empty(),
        JsonValue::Array(arr) => !arr.is_empty(),
        JsonValue::Object(map) => !map.is_empty(),
        _ => true,
    }
}

fn truncate_str(input: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for (count, ch) in input.chars().enumerate() {
        if count >= max_chars {
            out.push('…');
            break;
        }
        out.push(ch);
    }
    out
}

fn collect_limited_strings<I>(iter: I, limit: usize) -> (Vec<String>, bool)
where
    I: IntoIterator<Item = String>,
{
    if limit == 0 {
        return (Vec::new(), false);
    }
    let mut values = Vec::new();
    let mut truncated = false;
    for value in iter {
        if values.len() >= limit {
            truncated = true;
            break;
        }
        values.push(value);
    }
    (values, truncated)
}
