use serde_json::Value as JsonValue;

use crate::manifest::entity::ArchivedEntity;
use crate::manifest::extended_meta::{
    ExtendedMetaPath, ExtendedMetaValue, collect_capped_extended_meta_values,
    extended_meta_field_name, extended_meta_mode_name, sorted_extended_meta_fields,
};
use crate::utils::SearchPersona;

use super::core::ManifestSearch;

mod nova;
mod persona;

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
    include_nova_compact_summary: bool,
    include_nova_governance: bool,
    include_nova_tier: bool,
    include_nova_canonical: bool,
    include_nova_search_candidates: bool,
    include_has_nova_meta: bool,
    include_semantic_preview: bool,
    include_persona_payload: bool,
    include_extended_meta_summary: bool,
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
            include_nova_compact_summary: false,
            include_nova_governance: false,
            include_nova_tier: false,
            include_nova_canonical: false,
            include_nova_search_candidates: false,
            include_has_nova_meta: false,
            include_semantic_preview: false,
            include_persona_payload: false,
            include_extended_meta_summary: false,
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
            include_nova_search_candidates: true,
            include_nova_measures: false,
            include_nova_metrics: false,
            include_semantic_preview: true,
            include_persona_payload: true,
            include_extended_meta_summary: true,
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
            include_nova_search_candidates: true,
            include_semantic_preview: true,
            include_persona_payload: true,
            include_extended_meta_summary: true,
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
            include_nova_search_candidates: true,
            include_has_nova_meta: true,
            include_semantic_preview: true,
            include_persona_payload: true,
            include_extended_meta_summary: true,
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
            include_nova_search_candidates: true,
            include_semantic_preview: true,
            include_extended_meta_summary: true,
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
            include_nova_search_candidates: true,
            include_nova_summary: true,
            include_semantic_preview: true,
            include_extended_meta_summary: true,
            ..Self::empty()
        }
    }

    fn compact() -> Self {
        Self {
            include_package_name: true,
            include_alias: true,
            include_relation_fields: true,
            include_original_file_path: true,
            include_primary_key_columns: true,
            include_nova_domains: true,
            include_nova_tier: true,
            include_nova_canonical: true,
            include_nova_compact_summary: true,
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

    pub(crate) fn summary_for_compact(
        &self,
        unique_id: &str,
        entity: &ArchivedEntity,
    ) -> JsonValue {
        self.build_summary(unique_id, entity, SummaryProfile::compact(), None, None)
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
            || profile.include_nova_compact_summary
            || profile.include_nova_governance
            || profile.include_nova_tier
            || profile.include_nova_canonical
            || profile.include_nova_search_candidates
            || profile.include_nova_summary
            || profile.include_semantic_preview
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
            if profile.include_nova_search_candidates
                && let Some(candidates) = Self::nova_search_candidates_summary(nova)
            {
                obj.insert("nova_search_candidates".to_string(), candidates);
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
            if profile.include_nova_compact_summary
                && let Some(nova_summary) = Self::nova_compact_summary(nova)
            {
                obj.insert("nova_summary".to_string(), nova_summary);
            }
            if profile.include_nova_summary
                && let Some(nova_summary) = Self::nova_summary(nova)
            {
                obj.insert("nova_summary".to_string(), nova_summary);
            }
            if profile.include_semantic_preview
                && let Some(tokens) = query_tokens
                && let Some(semantic_preview) =
                    Self::semantic_preview(nova, tokens, self.config.search.min_word_length.max(1))
            {
                obj.insert("semantic_preview".to_string(), semantic_preview);
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
        if profile.include_extended_meta_summary {
            let entity_json = entity_json_cache.get_or_insert_with(|| entity.to_json_value());
            if let Some(summary) = self.extended_meta_summary(entity_json) {
                obj.insert("extended_meta_summary".to_string(), summary);
            }
        }
        compact_json_object(&mut obj);
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
        let relation_name = entity.relation_name_str().filter(|s| !s.trim().is_empty());
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

    pub(crate) fn extended_meta_summary(&self, entity_json: &JsonValue) -> Option<JsonValue> {
        let fields = sorted_extended_meta_fields(&self.config.search);
        if fields.is_empty() {
            return None;
        }

        let mut field_summaries = Vec::new();
        for field in fields {
            if !field.summary {
                continue;
            }
            let path = ExtendedMetaPath::from_config_path(&field.path);
            let values = collect_capped_extended_meta_values(
                entity_json,
                &path,
                field.mode,
                self.config.search.extended_meta.max_values_per_field,
                self.config.search.extended_meta.max_bytes_per_value,
            );
            if values.values.is_empty() {
                continue;
            }

            let mut obj = serde_json::Map::new();
            obj.insert("alias".to_string(), JsonValue::String(field.alias.clone()));
            obj.insert("path".to_string(), JsonValue::String(field.path.clone()));
            obj.insert(
                "search_field".to_string(),
                JsonValue::String(extended_meta_field_name(&field.alias)),
            );
            obj.insert(
                "mode".to_string(),
                JsonValue::String(extended_meta_mode_name(field.mode).to_string()),
            );
            obj.insert(
                "values".to_string(),
                JsonValue::Array(
                    values
                        .values
                        .iter()
                        .map(|value| JsonValue::String(value.value.clone()))
                        .collect(),
                ),
            );

            let columns = extended_meta_column_summary(&values.values);
            if !columns.is_empty() {
                obj.insert("columns".to_string(), JsonValue::Array(columns));
            }
            if values.dropped_values > 0 || values.byte_truncated_values > 0 {
                obj.insert("truncated".to_string(), JsonValue::from(true));
            }
            if values.dropped_values > 0 {
                obj.insert(
                    "dropped_values".to_string(),
                    JsonValue::from(values.dropped_values as u64),
                );
            }
            if values.byte_truncated_values > 0 {
                obj.insert(
                    "byte_truncated_values".to_string(),
                    JsonValue::from(values.byte_truncated_values as u64),
                );
            }

            field_summaries.push(JsonValue::Object(obj));
        }

        if field_summaries.is_empty() {
            None
        } else {
            Some(serde_json::json!({ "fields": field_summaries }))
        }
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

    fn empty_summary(unique_id: &str) -> JsonValue {
        serde_json::json!({
            "unique_id": unique_id,
            "missing": true,
        })
    }
}

fn extended_meta_column_summary(values: &[ExtendedMetaValue]) -> Vec<JsonValue> {
    let mut columns: Vec<(String, Vec<String>)> = Vec::new();
    for value in values {
        let Some(column_name) = value.column_name.as_ref() else {
            continue;
        };
        if let Some((_, existing_values)) = columns
            .last_mut()
            .filter(|(name, _)| name.as_str() == column_name.as_str())
        {
            existing_values.push(value.value.clone());
        } else {
            columns.push((column_name.clone(), vec![value.value.clone()]));
        }
    }

    columns
        .into_iter()
        .map(|(name, values)| {
            serde_json::json!({
                "name": name,
                "values": values,
            })
        })
        .collect()
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
