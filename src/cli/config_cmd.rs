use std::time::Instant;

use serde::Serialize;

use crate::cli::args::{ConfigShowArgs, ConfigValidateArgs};
use crate::cli::output::{CliEnvelope, error_envelope};
use crate::config::DbtNovaConfig;
use crate::error::DbtNovaError;
use crate::manifest::bootstrap::prepare_runtime_config;

use super::{DispatchError, DispatchResult};

#[derive(Debug, Serialize)]
pub struct ConfigValidateData {
    pub valid: bool,
    pub storage_instance_id: String,
    pub embedding_cache_dir: String,
}

/// Runs the `config show` CLI command.
///
/// # Errors
/// Returns an error when rendering output fails.
pub fn run_show_command(args: &ConfigShowArgs) -> DispatchResult {
    let started = Instant::now();
    let config = config_for_show(args);

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

    use super::{config_for_show, validate_runtime_config};
    use crate::cli::args::ConfigShowArgs;
    use crate::config::DbtNovaConfig;
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
