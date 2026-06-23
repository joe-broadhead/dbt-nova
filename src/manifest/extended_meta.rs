use serde_json::Value as JsonValue;

use crate::config::{ExtendedMetaFieldConfig, ExtendedMetaFieldMode, SearchConfig};
use crate::manifest::entity::{normalized_column_meta_json, normalized_entity_meta_json};

#[derive(Clone, Debug)]
pub(crate) enum ExtendedMetaPath {
    Entity { segments: Vec<String> },
    ColumnWildcard { segments: Vec<String> },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ExtendedMetaValue {
    pub(crate) column_name: Option<String>,
    pub(crate) value: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct CappedExtendedMetaValues {
    pub(crate) values: Vec<ExtendedMetaValue>,
    pub(crate) dropped_values: usize,
    pub(crate) byte_truncated_values: usize,
}

impl ExtendedMetaPath {
    pub(crate) fn from_config_path(path: &str) -> Self {
        let segments = path.trim().split('.').collect::<Vec<_>>();
        if segments.len() >= 4
            && segments.first() == Some(&"columns")
            && segments.get(1) == Some(&"*")
            && segments.get(2) == Some(&"meta")
        {
            return Self::ColumnWildcard {
                segments: segments[3..]
                    .iter()
                    .map(|segment| (*segment).to_string())
                    .collect(),
            };
        }

        Self::Entity {
            segments: segments
                .get(1..)
                .unwrap_or_default()
                .iter()
                .map(|segment| (*segment).to_string())
                .collect(),
        }
    }
}

pub(crate) fn sorted_extended_meta_fields(config: &SearchConfig) -> Vec<&ExtendedMetaFieldConfig> {
    let mut fields = config.extended_meta.fields.iter().collect::<Vec<_>>();
    fields.sort_by(|left, right| {
        left.alias
            .cmp(&right.alias)
            .then_with(|| left.path.cmp(&right.path))
    });
    fields
}

pub(crate) fn extended_meta_field_name(alias: &str) -> String {
    format!("meta.{}", alias.trim())
}

pub(crate) fn extended_meta_mode_name(mode: ExtendedMetaFieldMode) -> &'static str {
    match mode {
        ExtendedMetaFieldMode::Keyword => "keyword",
        ExtendedMetaFieldMode::Text => "text",
        ExtendedMetaFieldMode::StringArray => "string_array",
        ExtendedMetaFieldMode::Bool => "bool",
    }
}

pub(crate) fn collect_capped_extended_meta_values(
    entity_json: &JsonValue,
    path: &ExtendedMetaPath,
    mode: ExtendedMetaFieldMode,
    max_values: usize,
    max_bytes: usize,
) -> CappedExtendedMetaValues {
    let raw_values = collect_extended_meta_values(entity_json, path, mode);
    let mut out = CappedExtendedMetaValues::default();
    for value in raw_values {
        if out.values.len() >= max_values {
            out.dropped_values += 1;
            continue;
        }
        let Some((capped, byte_truncated)) = capped_non_empty_value(&value.value, max_bytes) else {
            continue;
        };
        if byte_truncated {
            out.byte_truncated_values += 1;
        }
        out.values.push(ExtendedMetaValue {
            column_name: value.column_name,
            value: capped,
        });
    }
    out
}

fn collect_extended_meta_values(
    entity_json: &JsonValue,
    path: &ExtendedMetaPath,
    mode: ExtendedMetaFieldMode,
) -> Vec<ExtendedMetaValue> {
    match path {
        ExtendedMetaPath::Entity { segments } => normalized_entity_meta_json(entity_json)
            .as_ref()
            .and_then(|meta| value_at_path(meta, segments))
            .map(|value| {
                values_for_mode(value, mode)
                    .into_iter()
                    .map(|value| ExtendedMetaValue {
                        column_name: None,
                        value,
                    })
                    .collect()
            })
            .unwrap_or_default(),
        ExtendedMetaPath::ColumnWildcard { segments } => {
            let Some(columns) = entity_json.get("columns").and_then(JsonValue::as_object) else {
                return Vec::new();
            };
            let mut column_names = columns.keys().collect::<Vec<_>>();
            column_names.sort();

            let mut out = Vec::new();
            for column_name in column_names {
                let Some(column) = columns.get(column_name) else {
                    continue;
                };
                let Some(meta) = normalized_column_meta_json(column) else {
                    continue;
                };
                let Some(value) = value_at_path(&meta, segments) else {
                    continue;
                };
                out.extend(values_for_mode(value, mode).into_iter().map(|value| {
                    ExtendedMetaValue {
                        column_name: Some(column_name.clone()),
                        value,
                    }
                }));
            }
            out
        }
    }
}

fn values_for_mode(value: &JsonValue, mode: ExtendedMetaFieldMode) -> Vec<String> {
    match mode {
        ExtendedMetaFieldMode::Keyword | ExtendedMetaFieldMode::Text => match value {
            JsonValue::String(value) => vec![value.clone()],
            JsonValue::Number(value) => vec![value.to_string()],
            _ => Vec::new(),
        },
        ExtendedMetaFieldMode::StringArray => value
            .as_array()
            .map(|values| {
                values
                    .iter()
                    .filter_map(JsonValue::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
        ExtendedMetaFieldMode::Bool => value
            .as_bool()
            .map(|value| value.to_string())
            .into_iter()
            .collect(),
    }
}

fn capped_non_empty_value(value: &str, max_bytes: usize) -> Option<(String, bool)> {
    if max_bytes == 0 {
        return None;
    }

    let value = value.trim();
    if value.is_empty() {
        return None;
    }

    let mut end = value.len().min(max_bytes);
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    let byte_truncated = end < value.len();
    value
        .get(..end)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| (value.to_string(), byte_truncated))
}

fn value_at_path<'a>(root: &'a JsonValue, segments: &[String]) -> Option<&'a JsonValue> {
    let mut current = root;
    for segment in segments {
        current = current.get(segment)?;
    }
    if current.is_null() {
        None
    } else {
        Some(current)
    }
}
