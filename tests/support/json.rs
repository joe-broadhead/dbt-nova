use dbt_nova::error::{DbtNovaError, Result};
use serde_json::Value as JsonValue;

pub fn json(res: Result<JsonValue>) -> JsonValue {
    res.unwrap_or_else(|e| e.to_response())
}

#[allow(dead_code)]
pub fn json_err(res: Result<JsonValue>) -> DbtNovaError {
    res.err()
        .unwrap_or_else(|| DbtNovaError::ServerError("expected error".to_string()))
}
