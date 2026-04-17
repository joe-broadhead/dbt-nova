use std::borrow::Cow;

use rkyv::option::ArchivedOption;
use rkyv::string::ArchivedString;
use rkyv_derive::{Archive, Deserialize, Serialize};
use serde_json::Value as JsonValue;
use tracing::warn;

macro_rules! archived_str_accessor {
    ($( $name:ident => $field:ident ),+ $(,)?) => {
        $(
            #[must_use]
            pub fn $name(&self) -> Option<&str> {
                archived_option_str(&self.$field)
            }
        )+
    };
}

#[derive(Clone, Debug, Archive, Serialize, Deserialize)]
pub struct NovaMeasure {
    pub name: String,
    pub measure_type: Option<String>,
    pub expression: Option<String>,
    pub description: Option<String>,
    pub field: Option<String>,
    pub synonyms: Vec<String>,
    pub canonical: bool,
}

#[derive(Clone, Debug, Archive, Serialize, Deserialize)]
pub struct NovaMetric {
    pub name: String,
    pub description: Option<String>,
    pub expression: Option<String>,
    pub synonyms: Vec<String>,
    pub template: bool,
    pub grain: Option<NovaGrain>,
    pub recommended_filters: Vec<NovaRecommendedFilter>,
    pub canonical: bool,
}

#[derive(Clone, Debug, Archive, Serialize, Deserialize)]
pub struct NovaGrain {
    pub primary_key: Vec<String>,
    pub time_field: Option<String>,
    pub dimensions: Vec<String>,
}

#[derive(Clone, Debug, Archive, Serialize, Deserialize)]
pub struct NovaRecommendedFilter {
    pub field: String,
    pub operator: Option<String>,
    pub values: Vec<String>,
    pub label: Option<String>,
}

#[derive(Clone, Debug, Archive, Serialize, Deserialize)]
pub struct NovaGovernance {
    pub sensitivity: Option<String>,
    pub pii: Option<String>,
    pub compliance: Vec<String>,
}

#[derive(Clone, Debug, Archive, Serialize, Deserialize)]
pub struct NovaSearchCandidates {
    pub analyst: bool,
    pub engineer: bool,
    pub governance: bool,
}

impl NovaSearchCandidates {
    #[must_use]
    pub fn has_non_default_flags(&self) -> bool {
        !self.analyst || !self.engineer || !self.governance
    }
}

#[derive(Clone, Debug, Archive, Serialize, Deserialize)]
pub struct NovaSearchMeta {
    pub candidates: Option<NovaSearchCandidates>,
}

#[derive(Clone, Debug, Archive, Serialize, Deserialize)]
pub struct NovaMeta {
    pub role: Option<String>,
    pub semantic_type: Option<String>,
    pub synonyms: Vec<String>,
    pub domains: Vec<String>,
    pub use_cases: Vec<String>,
    pub example_values: Vec<String>,
    pub canonical: bool,
    pub tier: Option<String>,
    pub grain: Option<NovaGrain>,
    pub measures: Vec<NovaMeasure>,
    pub metric: Option<NovaMetric>,
    pub metrics: Vec<NovaMetric>,
    pub governance: Option<NovaGovernance>,
    pub search: Option<NovaSearchMeta>,
}

#[derive(Clone, Debug, Archive, Serialize, Deserialize)]
pub struct ColumnMetaSummary {
    pub name: String,
    pub description: Option<String>,
    pub role: Option<String>,
    pub semantic_type: Option<String>,
    pub synonyms: Vec<String>,
    pub example_values: Vec<String>,
    pub primary_key: bool,
}

/// Lightweight typed wrapper around a dbt manifest entity.
///
/// Stores common fields extracted for fast access plus the raw JSON payload for
/// full fidelity responses and deep inspection when needed.
#[derive(Clone, Debug, Archive, Serialize, Deserialize)]
pub struct Entity {
    pub unique_id: String,
    pub resource_type: Option<String>,
    pub name: Option<String>,
    pub alias: Option<String>,
    pub package_name: Option<String>,
    pub database: Option<String>,
    pub schema: Option<String>,
    pub relation_name: Option<String>,
    pub original_file_path: Option<String>,
    pub materialization: Option<String>,
    pub description: Option<String>,
    pub tags: Vec<String>,
    pub column_names: Vec<String>,
    pub column_meta: Vec<ColumnMetaSummary>,
    pub columns_documented_count: usize,
    pub doc_blocks_present: bool,
    pub nova_meta: Option<NovaMeta>,
    pub has_compiled_sql: bool,
    /// Raw JSON payload for full fidelity responses.
    pub payload_json: String,
}

impl Entity {
    #[must_use]
    pub fn from_json(unique_id: &str, payload: &JsonValue) -> Self {
        let payload_json = match serde_json::to_string(payload) {
            Ok(payload_json) => payload_json,
            Err(err) => {
                warn!(
                    unique_id = unique_id,
                    error = %err,
                    "failed to serialize entity payload; falling back to null"
                );
                "null".to_string()
            }
        };
        let column_names = get_column_names(payload);
        let column_meta = get_column_meta(payload, &column_names);
        Self {
            unique_id: unique_id.to_string(),
            resource_type: get_string(payload, "resource_type"),
            name: get_string(payload, "name"),
            alias: get_string(payload, "alias"),
            package_name: get_string(payload, "package_name"),
            database: get_string(payload, "database"),
            schema: get_string(payload, "schema"),
            relation_name: get_string(payload, "relation_name"),
            original_file_path: get_string(payload, "original_file_path")
                .map(|p| p.trim_start_matches("./").to_string()),
            materialization: get_materialization(payload),
            description: get_string(payload, "description"),
            tags: get_tags(payload),
            column_names,
            column_meta,
            columns_documented_count: get_columns_documented_count(payload),
            doc_blocks_present: has_doc_blocks(payload),
            nova_meta: get_nova_meta(payload),
            has_compiled_sql: has_compiled_sql(payload),
            payload_json,
        }
    }

    #[must_use]
    pub fn to_json_value(&self) -> serde_json::Value {
        match serde_json::from_str(&self.payload_json) {
            Ok(payload) => payload,
            Err(err) => {
                warn!(
                    unique_id = %self.unique_id,
                    error = %err,
                    "failed to parse entity payload_json; returning null payload"
                );
                JsonValue::Null
            }
        }
    }

    #[must_use]
    pub fn column_names(&self) -> Vec<String> {
        self.column_names.clone()
    }

    pub fn column_names_iter(&self) -> impl Iterator<Item = &str> {
        self.column_names.iter().map(String::as_str)
    }
}

impl ArchivedEntity {
    #[must_use]
    pub fn payload_json(&self) -> &str {
        self.payload_json.as_str()
    }

    #[must_use]
    pub fn to_json_value(&self) -> serde_json::Value {
        match serde_json::from_str(self.payload_json()) {
            Ok(payload) => payload,
            Err(err) => {
                warn!(
                    payload_len = self.payload_json().len(),
                    error = %err,
                    "failed to parse archived entity payload_json; returning null payload"
                );
                JsonValue::Null
            }
        }
    }

    #[must_use]
    pub fn column_names(&self) -> Vec<String> {
        self.column_names
            .iter()
            .map(ArchivedString::as_str)
            .map(str::to_string)
            .collect()
    }

    pub fn column_names_iter(&self) -> impl Iterator<Item = &str> {
        self.column_names.iter().map(ArchivedString::as_str)
    }

    archived_str_accessor!(
        name_str => name,
        alias_str => alias,
        resource_type_str => resource_type,
        package_name_str => package_name,
        database_str => database,
        schema_str => schema,
        relation_name_str => relation_name,
        original_file_path_str => original_file_path,
        materialization_str => materialization,
        description_str => description
    );

    pub fn tags_iter(&self) -> impl Iterator<Item = &str> {
        self.tags.iter().map(ArchivedString::as_str)
    }

    #[must_use]
    pub fn column_meta(&self) -> &[ArchivedColumnMetaSummary] {
        &self.column_meta
    }

    #[must_use]
    pub fn doc_blocks_present(&self) -> bool {
        self.doc_blocks_present
    }

    #[must_use]
    pub fn columns_documented_count(&self) -> usize {
        self.columns_documented_count.to_native() as usize
    }

    #[must_use]
    pub fn has_compiled_sql(&self) -> bool {
        self.has_compiled_sql
    }

    #[must_use]
    pub fn nova_meta(&self) -> Option<&ArchivedNovaMeta> {
        self.nova_meta.as_ref()
    }
}

impl ArchivedNovaSearchCandidates {
    #[must_use]
    pub fn has_non_default_flags(&self) -> bool {
        !self.analyst || !self.engineer || !self.governance
    }
}

fn archived_option_str(value: &ArchivedOption<ArchivedString>) -> Option<&str> {
    value.as_ref().map(ArchivedString::as_str)
}

fn get_string(value: &serde_json::Value, key: &str) -> Option<String> {
    value.get(key).and_then(|v| v.as_str()).map(str::to_string)
}

#[must_use]
pub fn entity_meta_field_json<'a>(value: &'a JsonValue, field: &str) -> Option<&'a JsonValue> {
    value
        .get("meta")
        .and_then(|meta| meta.get(field))
        .filter(|meta| !meta.is_null())
        .or_else(|| {
            value
                .get("config")
                .and_then(|config| config.get("meta"))
                .and_then(|meta| meta.get(field))
                .filter(|meta| !meta.is_null())
        })
}

#[must_use]
pub fn entity_nova_meta_json(value: &JsonValue) -> Option<Cow<'_, JsonValue>> {
    merged_meta_value_json(
        value.get("meta").and_then(|meta| meta.get("nova")),
        value
            .get("config")
            .and_then(|config| config.get("meta"))
            .and_then(|meta| meta.get("nova")),
    )
}

#[must_use]
pub fn column_meta_json(column: &JsonValue) -> Option<&JsonValue> {
    column
        .get("meta")
        .filter(|meta| !meta.is_null())
        .or_else(|| column.get("config").and_then(|config| config.get("meta")))
}

#[must_use]
pub fn normalized_column_meta_json(column: &JsonValue) -> Option<JsonValue> {
    let legacy = column.get("meta").filter(|meta| !meta.is_null());
    let config = column
        .get("config")
        .and_then(|config| config.get("meta"))
        .filter(|meta| !meta.is_null());

    match (legacy, config) {
        (Some(JsonValue::Object(legacy_obj)), Some(JsonValue::Object(config_obj))) => {
            let mut merged = JsonValue::Object(config_obj.clone());
            merge_json_value(&mut merged, &JsonValue::Object(legacy_obj.clone()));
            Some(merged)
        }
        (Some(value), _) | (None, Some(value)) => Some(value.clone()),
        (None, None) => None,
    }
}

#[must_use]
pub fn column_meta_field_json<'a>(column: &'a JsonValue, field: &str) -> Option<&'a JsonValue> {
    column
        .get("meta")
        .and_then(|meta| meta.get(field))
        .filter(|meta| !meta.is_null())
        .or_else(|| {
            column
                .get("config")
                .and_then(|config| config.get("meta"))
                .and_then(|meta| meta.get(field))
                .filter(|meta| !meta.is_null())
        })
}

#[must_use]
pub fn column_nova_meta_json(column: &JsonValue) -> Option<Cow<'_, JsonValue>> {
    merged_meta_value_json(
        column.get("meta").and_then(|meta| meta.get("nova")),
        column
            .get("config")
            .and_then(|config| config.get("meta"))
            .and_then(|meta| meta.get("nova")),
    )
}

#[must_use]
pub fn column_primary_key_json(column: &JsonValue) -> Option<&JsonValue> {
    column_meta_field_json(column, "primary_key")
}

#[must_use]
pub fn column_primary_key_bool(column: &JsonValue) -> bool {
    parse_bool_like(column_primary_key_json(column)).unwrap_or(false)
}

fn merge_json_value(target: &mut JsonValue, overlay: &JsonValue) {
    if overlay.is_null() {
        return;
    }
    match (target, overlay) {
        (JsonValue::Object(target_obj), JsonValue::Object(overlay_obj)) => {
            for (key, overlay_value) in overlay_obj {
                match target_obj.get_mut(key) {
                    Some(target_value) => merge_json_value(target_value, overlay_value),
                    None => {
                        target_obj.insert(key.clone(), overlay_value.clone());
                    }
                }
            }
        }
        (target_value, overlay_value) => *target_value = overlay_value.clone(),
    }
}

fn merged_meta_value_json<'a>(
    legacy: Option<&'a JsonValue>,
    config: Option<&'a JsonValue>,
) -> Option<Cow<'a, JsonValue>> {
    let legacy = legacy.filter(|value| !value.is_null());
    let config = config.filter(|value| !value.is_null());
    match (legacy, config) {
        (Some(JsonValue::Object(legacy_obj)), Some(JsonValue::Object(config_obj))) => {
            let mut merged = JsonValue::Object(config_obj.clone());
            merge_json_value(&mut merged, &JsonValue::Object(legacy_obj.clone()));
            Some(Cow::Owned(merged))
        }
        (Some(value), _) | (None, Some(value)) => Some(Cow::Borrowed(value)),
        (None, None) => None,
    }
}

fn get_tags(value: &serde_json::Value) -> Vec<String> {
    let Some(tags) = value.get("tags") else {
        return Vec::new();
    };
    match tags {
        serde_json::Value::Array(values) => values
            .iter()
            .filter_map(|v| v.as_str())
            .map(str::to_string)
            .collect(),
        serde_json::Value::String(tag) => vec![tag.clone()],
        _ => Vec::new(),
    }
}

fn extract_string_array(value: &JsonValue) -> Vec<String> {
    value
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(JsonValue::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn extract_string_array_from_map(
    map: &serde_json::Map<String, JsonValue>,
    key: &str,
) -> Vec<String> {
    map.get(key).map(extract_string_array).unwrap_or_default()
}

fn parse_bool_like(value: Option<&JsonValue>) -> Option<bool> {
    match value {
        Some(JsonValue::Bool(value)) => Some(*value),
        Some(JsonValue::String(value)) if value.eq_ignore_ascii_case("true") => Some(true),
        Some(JsonValue::String(value)) if value.eq_ignore_ascii_case("false") => Some(false),
        _ => None,
    }
}

fn get_column_names(value: &serde_json::Value) -> Vec<String> {
    let Some(columns) = value.get("columns") else {
        return Vec::new();
    };
    match columns.as_object() {
        Some(map) => map.keys().cloned().collect(),
        None => Vec::new(),
    }
}

fn get_column_meta(value: &serde_json::Value, column_names: &[String]) -> Vec<ColumnMetaSummary> {
    let Some(columns) = value.get("columns").and_then(|v| v.as_object()) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for name in column_names {
        let Some(column) = columns.get(name) else {
            continue;
        };
        let description = column
            .get("description")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let nova_json = column_nova_meta_json(column);
        let nova = nova_json.as_deref().and_then(JsonValue::as_object);
        let role = nova
            .and_then(|n| n.get("role"))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let semantic_type = nova
            .and_then(|n| n.get("semantic_type"))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let synonyms = nova
            .map(|n| extract_string_array_from_map(n, "synonyms"))
            .unwrap_or_default();
        let example_values = nova
            .and_then(|n| n.get("example_values"))
            .and_then(|v| v.as_array())
            .map(|values| {
                values
                    .iter()
                    .filter_map(|value| value.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let primary_key = column_primary_key_bool(column);

        if description.is_none()
            && role.is_none()
            && semantic_type.is_none()
            && synonyms.is_empty()
            && example_values.is_empty()
            && !primary_key
        {
            continue;
        }
        out.push(ColumnMetaSummary {
            name: name.clone(),
            description,
            role,
            semantic_type,
            synonyms,
            example_values,
            primary_key,
        });
    }
    out
}

fn has_doc_blocks(value: &serde_json::Value) -> bool {
    value
        .get("doc_blocks")
        .and_then(|v| v.as_array())
        .is_some_and(|blocks| !blocks.is_empty())
}

fn get_materialization(value: &serde_json::Value) -> Option<String> {
    value
        .get("config")
        .and_then(|cfg| cfg.get("materialized"))
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

fn get_columns_documented_count(value: &serde_json::Value) -> usize {
    let Some(columns) = value.get("columns") else {
        return 0;
    };
    let Some(map) = columns.as_object() else {
        return 0;
    };
    map.values()
        .filter_map(|col| col.get("description").and_then(|v| v.as_str()))
        .filter(|desc| !desc.trim().is_empty())
        .count()
}

fn has_compiled_sql(value: &serde_json::Value) -> bool {
    value
        .get("compiled_code")
        .and_then(|v| v.as_str())
        .is_some_and(|s| !s.trim().is_empty())
}

#[allow(clippy::too_many_lines)]
fn get_nova_meta(value: &serde_json::Value) -> Option<NovaMeta> {
    let nova_json = entity_nova_meta_json(value);
    let nova = nova_json.as_deref().and_then(JsonValue::as_object)?;

    let role = nova
        .get("role")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let semantic_type = nova
        .get("semantic_type")
        .and_then(|v| v.as_str())
        .map(str::to_string);

    let synonyms = extract_string_array_from_map(nova, "synonyms");

    let domains = extract_string_array_from_map(nova, "domains");

    let use_cases = extract_string_array_from_map(nova, "use_cases");

    let example_values = extract_string_array_from_map(nova, "example_values");

    let canonical = nova
        .get("canonical")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false);

    let tier = nova
        .get("tier")
        .and_then(|v| v.as_str())
        .map(str::to_string);

    let grain = nova.get("grain").and_then(parse_grain);

    let measures = nova
        .get("measures")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|measure| measure.as_object())
                .filter_map(|measure| {
                    let name = measure.get("name").and_then(|v| v.as_str())?;
                    let measure_type = measure
                        .get("type")
                        .and_then(|v| v.as_str())
                        .map(str::to_string);
                    let expression = measure
                        .get("expression")
                        .and_then(|v| v.as_str())
                        .map(str::to_string);
                    let description = measure
                        .get("description")
                        .and_then(|v| v.as_str())
                        .map(str::to_string);
                    let field = measure
                        .get("field")
                        .and_then(|v| v.as_str())
                        .map(str::to_string);
                    let synonyms = measure
                        .get("synonyms")
                        .map(extract_string_array)
                        .unwrap_or_default();
                    let canonical = measure
                        .get("canonical")
                        .and_then(JsonValue::as_bool)
                        .unwrap_or(false);
                    Some(NovaMeasure {
                        name: name.to_string(),
                        measure_type,
                        expression,
                        description,
                        field,
                        synonyms,
                        canonical,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    let metric = nova.get("metric").and_then(parse_metric);
    let metrics = nova
        .get("metrics")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(parse_metric).collect())
        .unwrap_or_default();

    let governance = nova
        .get("governance")
        .and_then(|v| v.as_object())
        .map(|gov| {
            let sensitivity = gov
                .get("sensitivity")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let pii = match gov.get("pii") {
                Some(JsonValue::Bool(value)) => Some(value.to_string()),
                Some(JsonValue::String(value)) => Some(value.clone()),
                Some(JsonValue::Array(values)) => {
                    let joined = values
                        .iter()
                        .filter_map(|v| v.as_str())
                        .collect::<Vec<_>>()
                        .join(",");
                    if joined.is_empty() {
                        None
                    } else {
                        Some(joined)
                    }
                }
                _ => None,
            };
            let compliance = gov
                .get("compliance")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str())
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            NovaGovernance {
                sensitivity,
                pii,
                compliance,
            }
        });

    let search = nova.get("search").and_then(parse_nova_search);

    Some(NovaMeta {
        role,
        semantic_type,
        synonyms,
        domains,
        use_cases,
        example_values,
        canonical,
        tier,
        grain,
        measures,
        metric,
        metrics,
        governance,
        search,
    })
}

fn parse_nova_search(value: &JsonValue) -> Option<NovaSearchMeta> {
    let obj = value.as_object()?;
    let candidates = obj.get("candidates").and_then(parse_nova_search_candidates);
    candidates.as_ref()?;
    Some(NovaSearchMeta { candidates })
}

fn parse_nova_search_candidates(value: &JsonValue) -> Option<NovaSearchCandidates> {
    let obj = value.as_object()?;
    let analyst = parse_bool_like(obj.get("analyst"));
    let engineer = parse_bool_like(obj.get("engineer"));
    let governance = parse_bool_like(obj.get("governance"));
    if analyst.is_none() && engineer.is_none() && governance.is_none() {
        return None;
    }
    Some(NovaSearchCandidates {
        analyst: analyst.unwrap_or(true),
        engineer: engineer.unwrap_or(true),
        governance: governance.unwrap_or(true),
    })
}

fn parse_grain(value: &JsonValue) -> Option<NovaGrain> {
    let obj = value.as_object()?;
    let primary_key = extract_string_array_from_map(obj, "primary_key");
    let time_field = obj
        .get("time_field")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let dimensions = extract_string_array_from_map(obj, "dimensions");
    Some(NovaGrain {
        primary_key,
        time_field,
        dimensions,
    })
}

fn parse_metric(value: &JsonValue) -> Option<NovaMetric> {
    let metric = value.as_object()?;
    let name = metric.get("name").and_then(|v| v.as_str())?;
    let description = metric
        .get("description")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let expression = metric
        .get("expression")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let synonyms = extract_string_array_from_map(metric, "synonyms");
    let template = metric
        .get("template")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false);
    let grain = metric.get("grain").and_then(parse_grain);
    let recommended_filters = metric
        .get("recommended_filters")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|filter| filter.as_object())
                .filter_map(|filter| {
                    let field = filter.get("field").and_then(|v| v.as_str())?;
                    let operator = filter
                        .get("operator")
                        .and_then(|v| v.as_str())
                        .map(str::to_string);
                    let values = filter
                        .get("values")
                        .map(extract_string_array)
                        .unwrap_or_default();
                    let label = filter
                        .get("label")
                        .and_then(|v| v.as_str())
                        .map(str::to_string);
                    Some(NovaRecommendedFilter {
                        field: field.to_string(),
                        operator,
                        values,
                        label,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let canonical = metric
        .get("canonical")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false);
    Some(NovaMetric {
        name: name.to_string(),
        description,
        expression,
        synonyms,
        template,
        grain,
        recommended_filters,
        canonical,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_json_value_invalid_payload_returns_null() {
        let mut entity = Entity::from_json(
            "model.pkg.invalid",
            &serde_json::json!({
                "name": "invalid"
            }),
        );
        entity.payload_json = "{not-json".to_string();

        assert_eq!(entity.to_json_value(), JsonValue::Null);
    }

    #[test]
    fn get_nova_meta_parses_search_candidates_with_true_defaults() {
        let entity = serde_json::json!({
            "meta": {
                "nova": {
                    "search": {
                        "candidates": {
                            "analyst": false
                        }
                    }
                }
            }
        });

        let meta = get_nova_meta(&entity).expect("expected nova meta");
        let candidates = meta
            .search
            .and_then(|search| search.candidates)
            .expect("expected search candidates");

        assert!(!candidates.analyst);
        assert!(candidates.engineer);
        assert!(candidates.governance);
        assert!(candidates.has_non_default_flags());
    }

    #[test]
    fn get_nova_meta_ignores_empty_search_candidates_block() {
        let entity = serde_json::json!({
            "meta": {
                "nova": {
                    "search": {
                        "candidates": {}
                    }
                }
            }
        });

        let meta = get_nova_meta(&entity).expect("expected nova meta");
        assert!(meta.search.is_none());
    }

    #[test]
    fn get_nova_meta_parses_canonical_measure_and_metric_flags() {
        let entity = serde_json::json!({
            "meta": {
                "nova": {
                    "measures": [
                        {
                            "name": "gmv",
                            "canonical": true
                        }
                    ],
                    "metrics": [
                        {
                            "name": "aov",
                            "canonical": true
                        }
                    ]
                }
            }
        });

        let meta = get_nova_meta(&entity).expect("expected nova meta");
        assert_eq!(meta.measures.len(), 1);
        assert!(meta.measures[0].canonical);
        assert_eq!(meta.metrics.len(), 1);
        assert!(meta.metrics[0].canonical);
    }

    #[test]
    fn entity_nova_meta_prefers_legacy_meta_over_config_meta() {
        let entity = serde_json::json!({
            "meta": {
                "nova": {
                    "role": "dimension"
                }
            },
            "config": {
                "meta": {
                    "nova": {
                        "role": "measure"
                    }
                }
            }
        });

        let nova = entity_nova_meta_json(&entity).expect("expected nova metadata");
        assert_eq!(
            nova.as_ref().get("role").and_then(JsonValue::as_str),
            Some("dimension")
        );
    }

    #[test]
    fn column_meta_helpers_fallback_to_config_meta() {
        let column = serde_json::json!({
            "name": "order_id",
            "config": {
                "meta": {
                    "primary_key": true,
                    "nova": {
                        "role": "identifier"
                    }
                }
            }
        });

        assert!(column_meta_json(&column).is_some());
        assert!(column_primary_key_bool(&column));
        assert_eq!(
            column_nova_meta_json(&column)
                .as_deref()
                .and_then(|nova| nova.get("role"))
                .and_then(JsonValue::as_str),
            Some("identifier")
        );
    }

    #[test]
    fn column_meta_helpers_ignore_null_legacy_fields() {
        let column = serde_json::json!({
            "name": "order_id",
            "meta": {
                "primary_key": null,
                "nova": null
            },
            "config": {
                "meta": {
                    "primary_key": true,
                    "nova": {
                        "role": "identifier"
                    }
                }
            }
        });

        assert!(column_primary_key_bool(&column));
        assert_eq!(
            column_nova_meta_json(&column)
                .as_deref()
                .and_then(|nova| nova.get("role"))
                .and_then(JsonValue::as_str),
            Some("identifier")
        );
    }

    #[test]
    fn column_meta_json_falls_back_when_legacy_meta_is_null() {
        let column = serde_json::json!({
            "name": "order_id",
            "meta": null,
            "config": {
                "meta": {
                    "primary_key": true
                }
            }
        });

        let meta = column_meta_json(&column).expect("expected config meta fallback");
        assert_eq!(meta["primary_key"].as_bool(), Some(true));
    }

    #[test]
    fn entity_nova_meta_merges_partial_legacy_and_config_objects() {
        let entity = serde_json::json!({
            "meta": {
                "nova": {
                    "role": "dimension"
                }
            },
            "config": {
                "meta": {
                    "nova": {
                        "semantic_type": "market",
                        "synonyms": ["country"]
                    }
                }
            }
        });

        let nova = entity_nova_meta_json(&entity).expect("expected nova metadata");
        assert_eq!(nova["role"].as_str(), Some("dimension"));
        assert_eq!(nova["semantic_type"].as_str(), Some("market"));
        assert_eq!(
            nova["synonyms"]
                .as_array()
                .and_then(|values| values.first()),
            Some(&JsonValue::String("country".to_string()))
        );
    }

    #[test]
    fn entity_nova_meta_falls_back_when_legacy_nova_is_null() {
        let entity = serde_json::json!({
            "meta": {
                "nova": null
            },
            "config": {
                "meta": {
                    "nova": {
                        "role": "dimension"
                    }
                }
            }
        });

        let nova = entity_nova_meta_json(&entity).expect("expected nova metadata");
        assert_eq!(nova["role"].as_str(), Some("dimension"));
    }

    #[test]
    fn column_nova_meta_merges_partial_legacy_and_config_objects() {
        let column = serde_json::json!({
            "meta": {
                "nova": {
                    "role": "identifier"
                }
            },
            "config": {
                "meta": {
                    "nova": {
                        "semantic_type": "order_id",
                        "synonyms": ["purchase_id"]
                    }
                }
            }
        });

        let nova = column_nova_meta_json(&column).expect("expected nova metadata");
        assert_eq!(nova["role"].as_str(), Some("identifier"));
        assert_eq!(nova["semantic_type"].as_str(), Some("order_id"));
        assert_eq!(
            nova["synonyms"]
                .as_array()
                .and_then(|values| values.first()),
            Some(&JsonValue::String("purchase_id".to_string()))
        );
    }

    #[test]
    fn normalized_column_meta_ignores_null_legacy_values() {
        let column = serde_json::json!({
            "meta": {
                "primary_key": null,
                "nova": {
                    "role": "identifier",
                    "semantic_type": null
                }
            },
            "config": {
                "meta": {
                    "primary_key": true,
                    "nova": {
                        "semantic_type": "order_id",
                        "synonyms": ["purchase_id"]
                    }
                }
            }
        });

        let meta = normalized_column_meta_json(&column).expect("expected merged meta");
        assert_eq!(meta["primary_key"].as_bool(), Some(true));
        assert_eq!(meta["nova"]["role"].as_str(), Some("identifier"));
        assert_eq!(meta["nova"]["semantic_type"].as_str(), Some("order_id"));
        assert_eq!(
            meta["nova"]["synonyms"]
                .as_array()
                .and_then(|values| values.first()),
            Some(&JsonValue::String("purchase_id".to_string()))
        );
    }

    #[test]
    fn normalized_column_meta_falls_back_when_legacy_meta_is_null() {
        let column = serde_json::json!({
            "meta": null,
            "config": {
                "meta": {
                    "primary_key": true,
                    "nova": {
                        "role": "identifier",
                        "semantic_type": "order_id"
                    }
                }
            }
        });

        let meta = normalized_column_meta_json(&column).expect("expected config meta fallback");
        assert_eq!(meta["primary_key"].as_bool(), Some(true));
        assert_eq!(meta["nova"]["role"].as_str(), Some("identifier"));
        assert_eq!(meta["nova"]["semantic_type"].as_str(), Some("order_id"));
    }
}
