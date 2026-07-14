use serde_json::Value as JsonValue;

use crate::error::{DbtNovaError, Result};

pub(crate) fn typed_array_or_json(
    typed_name: &str,
    typed_values: &[String],
    json_name: &str,
    json_value: Option<&str>,
) -> Result<Option<String>> {
    if typed_values.is_empty() {
        return Ok(json_value.map(ToOwned::to_owned));
    }

    if let Some(raw_json) = json_value {
        let parsed: Vec<String> = serde_json::from_str(raw_json).map_err(|error| {
            DbtNovaError::InvalidParams(format!("invalid {json_name}: {error}"))
        })?;
        if parsed != typed_values {
            return Err(DbtNovaError::InvalidParams(format!(
                "{typed_name} and {json_name} differ; pass only one representation or matching values"
            )));
        }
    }

    serde_json::to_string(typed_values)
        .map(Some)
        .map_err(|error| DbtNovaError::InvalidParams(format!("invalid {typed_name}: {error}")))
}

pub(crate) fn typed_json_or_json(
    typed_name: &str,
    typed_value: Option<&JsonValue>,
    json_name: &str,
    json_value: Option<&str>,
) -> Result<Option<String>> {
    let Some(value) = typed_value else {
        return Ok(json_value.map(ToOwned::to_owned));
    };

    if let Some(raw_json) = json_value {
        let parsed: JsonValue = serde_json::from_str(raw_json).map_err(|error| {
            DbtNovaError::InvalidParams(format!("invalid {json_name}: {error}"))
        })?;
        if parsed != *value {
            return Err(DbtNovaError::InvalidParams(format!(
                "{typed_name} and {json_name} differ; pass only one representation or matching values"
            )));
        }
    }

    serde_json::to_string(value)
        .map(Some)
        .map_err(|error| DbtNovaError::InvalidParams(format!("invalid {typed_name}: {error}")))
}
