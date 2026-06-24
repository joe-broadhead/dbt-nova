use std::time::Instant;

use serde::Serialize;
use serde_json::Value as JsonValue;

use crate::cli::args::{ConfigShowArgs, ConfigValidateArgs};
use crate::cli::output::{CliEnvelope, error_envelope};
use crate::config::DbtNovaConfig;
use crate::error::{DbtNovaError, Result};
use crate::manifest::bootstrap::prepare_runtime_config;
use crate::params::{ConfigShowParams, ConfigValidateParams};
use crate::responses::SuccessResponse;
use crate::utils::sanitize_uri;

use super::{DispatchError, DispatchResult};

#[derive(Debug, Serialize)]
pub struct ConfigValidateData {
    pub valid: bool,
    pub storage_instance_id: String,
    pub embedding_cache_dir: String,
}

/// Builds the shared MCP/CLI-tool response for config inspection.
///
/// # Errors
/// Returns an error when response serialization fails.
pub fn build_config_show_tool_response(
    active_config: &DbtNovaConfig,
    params: &ConfigShowParams,
) -> Result<JsonValue> {
    let config = if params.defaults {
        DbtNovaConfig::default()
    } else {
        active_config.clone()
    };
    let config = redacted_config_show_value(&config)?;
    serde_json::to_value(SuccessResponse::new(config, 1))
        .map_err(|error| DbtNovaError::ServerError(error.to_string()))
}

/// Builds the shared MCP/CLI-tool response for runtime config validation.
///
/// # Errors
/// Returns an error when configuration validation or response serialization fails.
pub fn build_config_validate_tool_response(
    active_config: &DbtNovaConfig,
    _params: &ConfigValidateParams,
) -> Result<JsonValue> {
    let config = validate_runtime_config(active_config.clone())?;
    let payload = ConfigValidateData {
        valid: true,
        storage_instance_id: config.storage_instance_id.clone(),
        embedding_cache_dir: config.search.embedding_cache_dir.clone(),
    };
    serde_json::to_value(SuccessResponse::new(payload, 1))
        .map_err(|error| DbtNovaError::ServerError(error.to_string()))
}

/// Runs the `config show` CLI command.
///
/// # Errors
/// Returns an error when rendering output fails.
pub fn run_show_command(args: &ConfigShowArgs) -> DispatchResult {
    let started = Instant::now();
    let config = config_for_show(args);
    let config = redacted_config_show_value(&config).map_err(|error| DispatchError {
        error,
        rendered: false,
    })?;

    if args.json {
        let envelope = CliEnvelope::success("config show", &config, started.elapsed().as_millis());
        let out = serde_json::to_string_pretty(&envelope).map_err(|error| DispatchError {
            error: DbtNovaError::ServerError(error.to_string()),
            rendered: false,
        })?;
        println!("{out}");
    } else {
        let out = serde_json::to_string_pretty(&config).map_err(|error| DispatchError {
            error: DbtNovaError::ServerError(error.to_string()),
            rendered: false,
        })?;
        println!("{out}");
    }

    Ok(())
}

/// Runs the `config validate` CLI command.
///
/// # Errors
/// Returns an error when configuration validation fails or output serialization fails.
pub fn run_validate_command(args: &ConfigValidateArgs) -> DispatchResult {
    let started = Instant::now();
    let config = validate_runtime_config(DbtNovaConfig::from_env())
        .map_err(|error| render_or_propagate_error(args, error, started.elapsed().as_millis()))?;

    let payload = ConfigValidateData {
        valid: true,
        storage_instance_id: config.storage_instance_id.clone(),
        embedding_cache_dir: config.search.embedding_cache_dir.clone(),
    };

    if args.json {
        let envelope =
            CliEnvelope::success("config validate", &payload, started.elapsed().as_millis());
        let out = serde_json::to_string_pretty(&envelope).map_err(|error| DispatchError {
            error: DbtNovaError::ServerError(error.to_string()),
            rendered: false,
        })?;
        println!("{out}");
    } else {
        println!("config is valid");
        println!("  storage_instance_id: {}", payload.storage_instance_id);
        println!("  embedding_cache_dir: {}", payload.embedding_cache_dir);
    }

    Ok(())
}

fn render_or_propagate_error(
    args: &ConfigValidateArgs,
    error: DbtNovaError,
    elapsed_ms: u128,
) -> DispatchError {
    if args.json {
        let envelope = error_envelope("config validate", &error, elapsed_ms);
        if let Ok(json) = serde_json::to_string_pretty(&envelope) {
            println!("{json}");
            return DispatchError {
                error,
                rendered: true,
            };
        }
    }
    DispatchError {
        error,
        rendered: false,
    }
}

fn config_for_show(args: &ConfigShowArgs) -> DbtNovaConfig {
    if args.defaults {
        DbtNovaConfig::default()
    } else {
        DbtNovaConfig::from_env()
    }
}

fn redacted_config_show_value(config: &DbtNovaConfig) -> Result<JsonValue> {
    let mut value = serde_json::to_value(config)
        .map_err(|error| DbtNovaError::ServerError(error.to_string()))?;
    redact_config_value(&mut value, None);
    Ok(value)
}

fn redact_config_value(value: &mut JsonValue, key: Option<&str>) {
    match value {
        JsonValue::String(raw) => {
            if key.is_some_and(is_sensitive_config_key) {
                *raw = "[REDACTED]".to_string();
            } else if key.is_some_and(is_location_config_key) {
                *raw = sanitize_uri(raw);
            }
        }
        JsonValue::Array(values) => {
            for value in values {
                redact_config_value(value, key);
            }
        }
        JsonValue::Object(map) => {
            for (child_key, child_value) in map {
                redact_config_value(child_value, Some(child_key));
            }
        }
        JsonValue::Null | JsonValue::Bool(_) | JsonValue::Number(_) => {}
    }
}

fn is_sensitive_config_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    [
        "token",
        "secret",
        "password",
        "passwd",
        "pwd",
        "credential",
        "authorization",
        "private_key",
        "api_key",
        "apikey",
    ]
    .iter()
    .any(|sensitive| key.contains(sensitive))
}

fn is_location_config_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    key.contains("uri")
        || key.contains("path")
        || key.contains("dir")
        || key == "source"
        || key.ends_with("_source")
}

fn validate_runtime_config(mut config: DbtNovaConfig) -> crate::error::Result<DbtNovaConfig> {
    let _bootstrap_resolution = prepare_runtime_config(&mut config)?;
    // Mirror runtime storage-path safety checks used by manifest-loading paths so
    // `config validate` cannot report success for values that would fail later.
    let _ = config.manifest_cache_dir()?;
    let _ = config.storage_base_dir()?;
    let _ = config.storage_instance_root_dir()?;
    Ok(config)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{
        build_config_show_tool_response, build_config_validate_tool_response, config_for_show,
        validate_runtime_config,
    };
    use crate::cli::args::ConfigShowArgs;
    use crate::config::DbtNovaConfig;
    use crate::params::{ConfigShowParams, ConfigValidateParams};
    use tempfile::TempDir;

    #[test]
    fn config_show_defaults_matches_struct_default() {
        let args = ConfigShowArgs {
            defaults: true,
            json: false,
        };
        let config = config_for_show(&args);
        assert_eq!(config.manifest_path, DbtNovaConfig::default().manifest_path);
        assert_eq!(
            config.search.embedding_model,
            DbtNovaConfig::default().search.embedding_model
        );
    }

    #[test]
    fn validate_runtime_config_populates_runtime_fields() {
        let config =
            validate_runtime_config(DbtNovaConfig::default()).expect("validation should pass");
        assert!(!config.storage_instance_id.is_empty());
        assert!(!config.search.embedding_cache_dir.is_empty());
    }

    #[test]
    fn config_show_tool_response_defaults_match_cli_defaults() {
        let active_config = DbtNovaConfig {
            manifest_path: "custom-manifest.json".to_string(),
            ..DbtNovaConfig::default()
        };
        let response =
            build_config_show_tool_response(&active_config, &ConfigShowParams { defaults: true })
                .expect("config show response");

        assert_eq!(response["success"], serde_json::json!(true));
        assert_eq!(
            response["data"]["manifest_path"],
            serde_json::json!(DbtNovaConfig::default().manifest_path)
        );
    }

    #[test]
    fn config_show_response_redacts_secret_bearing_locations() {
        let active_config = DbtNovaConfig {
            manifest_path: "/tmp/token/raw-manifest/manifest.json".to_string(),
            manifest_uri: "https://user:pass@example.com/manifest.json?token=raw-token".to_string(),
            storage_artifact_uri:
                "s3://bucket/path/secret/raw-storage/storage.tar.gz?X-Amz-Signature=raw-signature"
                    .to_string(),
            metadata_artifact_uri:
                "https://example.com/metadata.json?access_token=raw-access-token".to_string(),
            models_artifact_uri: "gs://bucket/password/raw-models/models.tar.gz".to_string(),
            bootstrap_uri: "https://example.com/bootstrap.json?api_key=raw-api-key".to_string(),
            search: crate::config::SearchConfig {
                embedding_cache_dir: "/tmp/password/raw-cache/models".to_string(),
                ..crate::config::SearchConfig::default()
            },
            ..DbtNovaConfig::default()
        };

        let response =
            build_config_show_tool_response(&active_config, &ConfigShowParams { defaults: false })
                .expect("config show response");
        let serialized = response.to_string();

        assert!(!serialized.contains("raw-token"));
        assert!(!serialized.contains("raw-signature"));
        assert!(!serialized.contains("raw-access-token"));
        assert!(!serialized.contains("raw-api-key"));
        assert!(!serialized.contains("raw-cache"));
        assert!(!serialized.contains("user:pass"));
        assert!(serialized.contains("[REDACTED]"));
        assert_eq!(
            response["data"]["manifest_uri"],
            serde_json::json!("https://[REDACTED]@example.com/manifest.json?[REDACTED]")
        );
    }

    #[test]
    fn config_redaction_masks_future_sensitive_keys() {
        let mut value = serde_json::json!({
            "credentials": {
                "api_token": "raw-token",
                "safe": "orders"
            },
            "artifact_uri": "https://example.com/artifact.tar.gz?sig=raw-signature"
        });

        super::redact_config_value(&mut value, None);

        assert_eq!(
            value["credentials"]["api_token"],
            serde_json::json!("[REDACTED]")
        );
        assert_eq!(value["credentials"]["safe"], serde_json::json!("orders"));
        assert_eq!(
            value["artifact_uri"],
            serde_json::json!("https://example.com/artifact.tar.gz?[REDACTED]")
        );
        assert!(!value.to_string().contains("raw-token"));
        assert!(!value.to_string().contains("raw-signature"));
    }

    #[test]
    fn config_validate_tool_response_matches_cli_payload() {
        let response = build_config_validate_tool_response(
            &DbtNovaConfig::default(),
            &ConfigValidateParams::default(),
        )
        .expect("config validate response");

        assert_eq!(response["success"], serde_json::json!(true));
        assert_eq!(response["data"]["valid"], serde_json::json!(true));
        assert!(response["data"]["storage_instance_id"].is_string());
        assert!(response["data"]["embedding_cache_dir"].is_string());
    }

    #[test]
    fn validate_runtime_config_rejects_unsafe_storage_dir() {
        let config = DbtNovaConfig {
            storage_dir: "../unsafe".to_string(),
            ..DbtNovaConfig::default()
        };
        let err = validate_runtime_config(config).expect_err("unsafe storage_dir should fail");
        assert!(err.to_string().contains("storage directory"));
    }

    #[test]
    fn validate_runtime_config_rejects_unsafe_storage_instance_id() {
        let config = DbtNovaConfig {
            storage_instance_id: "unsafe/id".to_string(),
            ..DbtNovaConfig::default()
        };
        let err =
            validate_runtime_config(config).expect_err("unsafe storage_instance_id should fail");
        assert!(err.to_string().contains("storage instance id is unsafe"));
    }

    #[test]
    fn validate_runtime_config_accepts_bootstrap_only_inputs() {
        let temp_dir = TempDir::new().expect("temp dir");
        let bootstrap_path = temp_dir.path().join("nova-bootstrap.json");
        fs::write(
            &bootstrap_path,
            r#"{
  "contract_version":"v1",
  "profile":"prod",
  "storage_instance_id":"bootstrap-instance",
  "manifest_uri":"dbfs:/FileStore/manifests/prod/manifest.json",
  "storage_artifact_uri":"dbfs:/FileStore/nova/prod/storage.tar.gz",
  "metadata_artifact_uri":"dbfs:/FileStore/nova/prod/metadata.json",
  "manifest_hash":"abc123",
  "dbt_nova_version":"0.0.2",
  "build_timestamp":"2026-01-01T10:00:00Z"
}"#,
        )
        .expect("write bootstrap");

        let config = DbtNovaConfig {
            storage_dir: temp_dir
                .path()
                .join(".dbt-nova")
                .to_string_lossy()
                .to_string(),
            bootstrap_uri: format!("file://{}", bootstrap_path.display()),
            manifest_path: "manifest.json".to_string(),
            manifest_uri: String::new(),
            storage_instance_id: String::new(),
            storage_artifact_uri: String::new(),
            metadata_artifact_uri: String::new(),
            ..DbtNovaConfig::default()
        };

        let validated =
            validate_runtime_config(config).expect("bootstrap-only config should validate");
        assert_eq!(validated.storage_instance_id, "bootstrap-instance");
        assert_eq!(
            validated.storage_artifact_uri,
            "dbfs:/FileStore/nova/prod/storage.tar.gz"
        );
        assert_eq!(
            validated.metadata_artifact_uri,
            "dbfs:/FileStore/nova/prod/metadata.json"
        );
        assert_eq!(
            validated.manifest_uri,
            "dbfs:/FileStore/manifests/prod/manifest.json"
        );
    }
}
