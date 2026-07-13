use serde::Serialize;
use serde_json::Value as JsonValue;

use crate::error::DbtNovaError;
use crate::responses::{ApiContract, response_api_contract};

#[derive(Debug, Serialize)]
pub struct CliMeta {
    pub elapsed_ms: u128,
    pub timestamp_ms: u128,
    pub version: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_response: Option<JsonValue>,
}

impl CliMeta {
    #[must_use]
    pub(crate) fn new(elapsed_ms: u128) -> Self {
        Self {
            elapsed_ms,
            timestamp_ms: timestamp_ms(),
            version: env!("CARGO_PKG_VERSION"),
            tool_response: None,
        }
    }

    #[must_use]
    fn with_tool_response(mut self, tool_response: Option<JsonValue>) -> Self {
        self.tool_response = tool_response;
        self
    }
}

#[derive(Debug, Serialize)]
pub struct CliEnvelope<T>
where
    T: Serialize,
{
    pub api: ApiContract,
    pub command: String,
    pub status: &'static str,
    pub data: Option<T>,
    pub meta: CliMeta,
    pub error: Option<JsonValue>,
}

impl<T> CliEnvelope<T>
where
    T: Serialize,
{
    #[must_use]
    pub fn success(command: impl Into<String>, data: T, elapsed_ms: u128) -> Self {
        Self {
            api: response_api_contract(),
            command: command.into(),
            status: "success",
            data: Some(data),
            meta: CliMeta::new(elapsed_ms),
            error: None,
        }
    }

    #[must_use]
    pub fn success_with_tool_response(
        command: impl Into<String>,
        data: T,
        tool_response: Option<JsonValue>,
        elapsed_ms: u128,
    ) -> Self {
        Self {
            api: response_api_contract(),
            command: command.into(),
            status: "success",
            data: Some(data),
            meta: CliMeta::new(elapsed_ms).with_tool_response(tool_response),
            error: None,
        }
    }
}

#[must_use]
pub fn error_envelope(
    command: impl Into<String>,
    error: &DbtNovaError,
    elapsed_ms: u128,
) -> CliEnvelope<JsonValue> {
    CliEnvelope {
        api: response_api_contract(),
        command: command.into(),
        status: "error",
        data: None,
        meta: CliMeta::new(elapsed_ms),
        error: Some(error.to_response()),
    }
}

#[must_use]
pub fn exit_code(error: &DbtNovaError) -> i32 {
    match error {
        DbtNovaError::InvalidParams(_) | DbtNovaError::InvalidParamsDetailed { .. } => 1,
        DbtNovaError::ManifestError(_) | DbtNovaError::IndexBuildInProgress { .. } => 2,
        DbtNovaError::EntityNotFound { .. }
        | DbtNovaError::AmbiguousName { .. }
        | DbtNovaError::ServerError(_)
        | DbtNovaError::IoError { .. }
        | DbtNovaError::JsonError { .. }
        | DbtNovaError::DatabricksError { .. }
        | DbtNovaError::TantivyError { .. }
        | DbtNovaError::GcpAuthError(_) => 3,
    }
}

fn timestamp_ms() -> u128 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis())
}

#[cfg(test)]
mod tests {
    use crate::error::DbtNovaError;

    use super::{CliEnvelope, error_envelope, exit_code};

    #[test]
    fn success_envelope_includes_api_contract_marker() {
        let envelope = CliEnvelope::success("test command", serde_json::json!({"ok": true}), 7);
        let payload = serde_json::to_value(envelope).expect("serialize envelope");

        assert_eq!(
            payload["api"]["envelope"],
            serde_json::json!(crate::responses::RESPONSE_ENVELOPE_ID)
        );
        assert_eq!(
            payload["api"]["nova_version"],
            serde_json::json!(env!("CARGO_PKG_VERSION"))
        );
        assert!(
            payload
                .get("meta")
                .and_then(|meta| meta.get("api"))
                .is_none()
        );
    }

    #[test]
    fn error_envelope_includes_api_contract_marker() {
        let envelope = error_envelope(
            "test command",
            &DbtNovaError::InvalidParams("bad input".to_string()),
            7,
        );
        let payload = serde_json::to_value(envelope).expect("serialize envelope");

        assert_eq!(
            payload["api"]["envelope"],
            serde_json::json!(crate::responses::RESPONSE_ENVELOPE_ID)
        );
        assert_eq!(payload["status"], serde_json::json!("error"));
    }

    #[test]
    fn exit_code_map_invalid_params_to_one() {
        assert_eq!(exit_code(&DbtNovaError::InvalidParams("x".to_string())), 1);
    }

    #[test]
    fn exit_code_map_manifest_error_to_two() {
        assert_eq!(exit_code(&DbtNovaError::ManifestError("x".to_string())), 2);
    }

    #[test]
    fn exit_code_map_runtime_errors_to_three() {
        assert_eq!(exit_code(&DbtNovaError::ServerError("x".to_string())), 3);
    }
}
