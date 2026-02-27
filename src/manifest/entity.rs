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
}

#[derive(Clone, Debug, Archive, Serialize, Deserialize)]
pub struct ColumnMetaSummary {
    pub name: String,
    pub description: Option<String>,
    pub role: Option<String>,
    pub semantic_type: Option<String>,
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

fn archived_option_str(value: &ArchivedOption<ArchivedString>) -> Option<&str> {
    value.as_ref().map(ArchivedString::as_str)
}

fn get_string(value: &serde_json::Value, key: &str) -> Option<String> {
    value.get(key).and_then(|v| v.as_str()).map(str::to_string)
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
        let nova = column
            .get("meta")
            .and_then(|m| m.get("nova"))
            .and_then(|v| v.as_object());
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
        let primary_key = column
            .get("meta")
            .and_then(|m| m.get("primary_key"))
            .and_then(|v| {
                v.as_bool()
                    .or_else(|| v.as_str().map(|s| s.eq_ignore_ascii_case("true")))
            })
            .unwrap_or(false);

        if description.is_none() && role.is_none() && semantic_type.is_none() && !primary_key {
            continue;
        }
        out.push(ColumnMetaSummary {
            name: name.clone(),
            description,
            role,
            semantic_type,
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
    let nova = value
        .get("meta")
        .and_then(|m| m.get("nova"))
        .and_then(|v| v.as_object())?;

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
                    Some(NovaMeasure {
                        name: name.to_string(),
                        measure_type,
                        expression,
                        description,
                        field,
                        synonyms,
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
    Some(NovaMetric {
        name: name.to_string(),
        description,
        expression,
        synonyms,
        template,
        grain,
        recommended_filters,
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
}
