use serde::Serialize;
use serde_json::Value as JsonValue;

use crate::error::DbtNovaError;

#[derive(Debug, Serialize)]
pub struct CliMeta {
    pub elapsed_ms: u128,
    pub timestamp_ms: u128,
    pub version: &'static str,
}

#[derive(Debug, Serialize)]
pub struct CliEnvelope<T>
where
    T: Serialize,
{
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
            command: command.into(),
            status: "success",
            data: Some(data),
            meta: CliMeta {
                elapsed_ms,
                timestamp_ms: timestamp_ms(),
                version: env!("CARGO_PKG_VERSION"),
            },
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
        command: command.into(),
        status: "error",
        data: None,
        meta: CliMeta {
            elapsed_ms,
            timestamp_ms: timestamp_ms(),
            version: env!("CARGO_PKG_VERSION"),
        },
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

    use super::exit_code;

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
