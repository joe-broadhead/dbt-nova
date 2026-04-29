use std::collections::BTreeSet;
use std::fs::{OpenOptions, create_dir_all};
use std::io::Write;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::{Value, json};
use tracing::warn;

pub const TRACE_ENV: &str = "DBT_NOVA_TRACE_TOOL_CALLS_PATH";
const MAX_PARAM_KEYS: usize = 50;
const MAX_ARRAY_ITEMS: usize = 20;
const MAX_SUMMARY_STRING_CHARS: usize = 256;
const MAX_SELECTED_UNIQUE_IDS: usize = 200;
const MAX_UNIQUE_ID_CHARS: usize = 512;

#[derive(Debug, Serialize)]
struct ToolTraceRow {
    timestamp_ms: u128,
    transport: String,
    tool: String,
    success: bool,
    duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    error_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    params_summary: Option<Value>,
    selected_unique_ids: Vec<String>,
}

/// Append a sanitized tool-call trace row when `DBT_NOVA_TRACE_TOOL_CALLS_PATH` is set.
pub fn record_tool_call(
    transport: &str,
    tool: &str,
    params: Option<&Value>,
    response: Option<&Value>,
    success: bool,
    duration_ms: u64,
) {
    let Ok(path) = std::env::var(TRACE_ENV) else {
        return;
    };
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return;
    }

    let trace_path = Path::new(trimmed);
    if let Some(parent) = trace_path.parent()
        && !parent.as_os_str().is_empty()
        && let Err(error) = create_dir_all(parent)
    {
        warn!(path = %parent.display(), error = %error, "failed to create tool trace directory");
        return;
    }

    let row = ToolTraceRow {
        timestamp_ms: timestamp_ms(),
        transport: transport.to_string(),
        tool: tool.to_string(),
        success,
        duration_ms,
        error_code: response.and_then(extract_error_code),
        params_summary: params.map(summarize_params),
        selected_unique_ids: response.map(extract_unique_ids).unwrap_or_default(),
    };

    let serialized = match serde_json::to_string(&row) {
        Ok(serialized) => serialized,
        Err(error) => {
            warn!(error = %error, "failed to serialize tool trace row");
            return;
        }
    };

    match OpenOptions::new()
        .create(true)
        .append(true)
        .open(trace_path)
    {
        Ok(mut file) => {
            if let Err(error) = writeln!(file, "{serialized}") {
                warn!(path = %trace_path.display(), error = %error, "failed to write tool trace row");
            }
        }
        Err(error) => {
            warn!(path = %trace_path.display(), error = %error, "failed to open tool trace file");
        }
    }
}

fn summarize_params(params: &Value) -> Value {
    let Some(map) = params.as_object() else {
        return json!({"type": params_type(params)});
    };

    let keys: Vec<Value> = map
        .keys()
        .take(MAX_PARAM_KEYS)
        .map(|key| Value::String(truncate_string(key, MAX_SUMMARY_STRING_CHARS)))
        .collect();
    let mut summary = serde_json::Map::from_iter([(String::from("keys"), Value::Array(keys))]);
    for key in [
        "query",
        "persona",
        "id_or_name",
        "resource_type",
        "resource_types",
        "recipe_id",
        "direction",
        "limit",
        "offset",
    ] {
        if let Some(value) = map.get(key).and_then(summarize_safe_value) {
            summary.insert(key.to_string(), value);
        }
    }
    Value::Object(summary)
}

fn summarize_safe_value(value: &Value) -> Option<Value> {
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) => Some(value.clone()),
        Value::String(value) => Some(Value::String(truncate_string(
            value,
            MAX_SUMMARY_STRING_CHARS,
        ))),
        Value::Array(items) => Some(Value::Array(
            items
                .iter()
                .take(MAX_ARRAY_ITEMS)
                .filter_map(summarize_safe_array_value)
                .collect(),
        )),
        Value::Object(_) => None,
    }
}

fn summarize_safe_array_value(value: &Value) -> Option<Value> {
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) => Some(value.clone()),
        Value::String(value) => Some(Value::String(truncate_string(
            value,
            MAX_SUMMARY_STRING_CHARS,
        ))),
        Value::Array(_) | Value::Object(_) => None,
    }
}

fn truncate_string(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

fn params_type(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn extract_error_code(response: &Value) -> Option<String> {
    response
        .get("error_code")
        .and_then(Value::as_str)
        .or_else(|| {
            response
                .get("error")
                .and_then(|error| error.get("error_code"))
                .and_then(Value::as_str)
        })
        .map(ToString::to_string)
}

fn extract_unique_ids(response: &Value) -> Vec<String> {
    let mut out = BTreeSet::new();
    collect_unique_ids(response, &mut out);
    out.into_iter().collect()
}

fn collect_unique_ids(value: &Value, out: &mut BTreeSet<String>) {
    match value {
        Value::Object(map) => {
            if out.len() >= MAX_SELECTED_UNIQUE_IDS {
                return;
            }
            for key in ["unique_id", "parent_unique_id", "root_id"] {
                if let Some(id) = map.get(key).and_then(Value::as_str)
                    && !id.trim().is_empty()
                {
                    out.insert(truncate_string(id, MAX_UNIQUE_ID_CHARS));
                    if out.len() >= MAX_SELECTED_UNIQUE_IDS {
                        return;
                    }
                }
            }
            for child in map.values() {
                if out.len() >= MAX_SELECTED_UNIQUE_IDS {
                    return;
                }
                collect_unique_ids(child, out);
            }
        }
        Value::Array(items) => {
            for item in items {
                if out.len() >= MAX_SELECTED_UNIQUE_IDS {
                    return;
                }
                collect_unique_ids(item, out);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

fn timestamp_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{MAX_SELECTED_UNIQUE_IDS, extract_unique_ids, summarize_params};

    #[test]
    fn summarize_params_keeps_only_safe_context() {
        let summary = summarize_params(&json!({
            "query": "gmv",
            "parameters": {"token": "secret"},
            "limit": 5
        }));
        assert_eq!(summary["query"], "gmv");
        assert_eq!(summary["limit"], 5);
        assert!(summary.get("parameters").is_none());
    }

    #[test]
    fn summarize_params_drops_nested_values_from_safe_arrays() {
        let summary = summarize_params(&json!({
            "resource_types": ["model", {"token": "secret"}]
        }));
        assert_eq!(summary["resource_types"], json!(["model"]));
    }

    #[test]
    fn extract_unique_ids_finds_nested_ids() {
        let ids = extract_unique_ids(&json!({
            "data": [{"unique_id": "model.pkg.orders", "parent_unique_id": "model.pkg.parent"}],
            "root_id": "model.pkg.root"
        }));
        assert_eq!(
            ids,
            vec![
                "model.pkg.orders".to_string(),
                "model.pkg.parent".to_string(),
                "model.pkg.root".to_string()
            ]
        );
    }

    #[test]
    fn extract_unique_ids_caps_large_responses() {
        let data: Vec<_> = (0..(MAX_SELECTED_UNIQUE_IDS + 5))
            .map(|index| json!({"unique_id": format!("model.pkg.entity_{index}")}))
            .collect();
        let ids = extract_unique_ids(&json!({"data": data}));
        assert_eq!(ids.len(), MAX_SELECTED_UNIQUE_IDS);
    }
}
