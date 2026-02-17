use serde_json::{Value as JsonValue, json};
use tantivy::TantivyError;
use thiserror::Error;

/// Crate-wide result type using `DbtNovaError`.
pub type Result<T> = std::result::Result<T, DbtNovaError>;

/// Structured error type for API and internal failures.
#[derive(Debug, Error)]
pub enum DbtNovaError {
    #[error("Manifest error: {0}")]
    ManifestError(String),

    #[error("Entity '{query}' not found")]
    EntityNotFound {
        query: String,
        resource_type: Option<String>,
        available_resource_types: Vec<String>,
    },

    #[error("Ambiguous name '{name}' matches {count} entities")]
    AmbiguousName {
        name: String,
        count: usize,
        matches: Vec<String>,
    },

    #[error("Invalid parameter: {0}")]
    InvalidParams(String),

    #[error("Invalid parameter: {message}")]
    InvalidParamsDetailed { message: String, details: JsonValue },

    #[error("Server error: {0}")]
    ServerError(String),

    #[error("Databricks error: {message}")]
    DatabricksError {
        message: String,
        status: Option<u16>,
        body: Option<String>,
    },

    #[error("Manifest indexing in progress (elapsed_ms: {elapsed_ms})")]
    IndexBuildInProgress { elapsed_ms: u128 },
}

impl DbtNovaError {
    /// Stable machine-readable error code for MCP responses.
    #[must_use]
    pub fn error_code(&self) -> &'static str {
        match self {
            DbtNovaError::EntityNotFound { .. } => "NOT_FOUND",
            DbtNovaError::AmbiguousName { .. } => "AMBIGUOUS",
            DbtNovaError::InvalidParams(_) | DbtNovaError::InvalidParamsDetailed { .. } => {
                "INVALID_PARAMS"
            }
            DbtNovaError::ManifestError(_) => "INITIALIZATION_ERROR",
            DbtNovaError::ServerError(_) | DbtNovaError::DatabricksError { .. } => "SERVER_ERROR",
            DbtNovaError::IndexBuildInProgress { .. } => "INDEX_BUILDING",
        }
    }

    /// Serialize the error into a standard MCP error payload.
    #[must_use]
    pub fn to_response(&self) -> serde_json::Value {
        match self {
            DbtNovaError::AmbiguousName {
                name: _,
                count,
                matches,
            } => json!({
                "success": false,
                "error": self.to_string(),
                "error_code": self.error_code(),
                "matches": matches,
                "count": count,
            }),
            DbtNovaError::EntityNotFound {
                query,
                resource_type,
                available_resource_types,
            } => json!({
                "success": false,
                "error": format!(
                    "{}{}{}",
                    self.to_string(),
                    resource_type
                        .as_ref()
                        .map(|rt| format!(" (resource_type: {rt})"))
                        .unwrap_or_default(),
                    if available_resource_types.is_empty() {
                        String::new()
                    } else {
                        format!(
                            " Available resource types: {}",
                            available_resource_types.join(", ")
                        )
                    }
                ),
                "error_code": self.error_code(),
                "searched": query,
                "resource_type": resource_type,
                "available_resource_types": available_resource_types,
            }),
            DbtNovaError::IndexBuildInProgress { elapsed_ms } => json!({
                "success": false,
                "error": self.to_string(),
                "error_code": self.error_code(),
                "elapsed_ms": elapsed_ms,
            }),
            DbtNovaError::DatabricksError {
                message: _,
                status,
                body,
            } => json!({
                "success": false,
                "error": self.to_string(),
                "error_code": self.error_code(),
                "status": status,
                "body": body,
            }),
            DbtNovaError::InvalidParamsDetailed {
                message: _,
                details,
            } => json!({
                "success": false,
                "error": self.to_string(),
                "error_code": self.error_code(),
                "details": details,
            }),
            _ => json!({
                "success": false,
                "error": self.to_string(),
                "error_code": self.error_code(),
            }),
        }
    }
}

impl From<std::io::Error> for DbtNovaError {
    fn from(err: std::io::Error) -> Self {
        DbtNovaError::ServerError(err.to_string())
    }
}

impl From<serde_json::Error> for DbtNovaError {
    fn from(err: serde_json::Error) -> Self {
        DbtNovaError::ServerError(err.to_string())
    }
}

impl From<TantivyError> for DbtNovaError {
    fn from(err: TantivyError) -> Self {
        DbtNovaError::ServerError(err.to_string())
    }
}
