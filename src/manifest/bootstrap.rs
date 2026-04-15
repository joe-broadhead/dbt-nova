use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value as JsonValue, json};
use tracing::{info, warn};

use crate::config::DbtNovaConfig;
use crate::error::{DbtNovaError, Result};
use crate::manifest::prebuilt_assets::{PrebuiltAssetsBootstrap, PrebuiltAssetsMetadata};
use crate::manifest::prebuilt_assets_resolver::{
    ensure_regular_file, read_small_text_file, resolve_artifact_uri_to_local,
};
use crate::utils::sanitize_uri;

/// Result of bootstrap URI evaluation.
#[derive(Debug, Clone)]
pub struct BootstrapResolution {
    pub status: JsonValue,
}

fn current_time_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis())
}

fn default_manifest_path() -> &'static str {
    "manifest.json"
}

fn should_apply_manifest_uri(config: &DbtNovaConfig) -> bool {
    if !config.manifest_uri.trim().is_empty() {
        return false;
    }
    if config.manifest_path_explicit {
        return false;
    }
    let manifest_path = config.manifest_path.trim();
    manifest_path.is_empty() || manifest_path == default_manifest_path()
}

fn apply_if_empty(target: &mut String, value: &str, field_name: &str, applied: &mut Vec<String>) {
    if target.trim().is_empty() {
        *target = value.to_string();
        applied.push(field_name.to_string());
    }
}

fn bootstrap_disabled_status() -> JsonValue {
    json!({
        "enabled": false,
        "uri": "",
        "contract_version": JsonValue::Null,
        "loaded": false,
        "validated": false,
        "applied_fields": [],
        "manifest_hash": JsonValue::Null,
        "last_evaluated_at_ms": JsonValue::Null,
    })
}

fn validate_models_metadata_contract(
    config: &DbtNovaConfig,
    bootstrap: &PrebuiltAssetsBootstrap,
) -> Result<()> {
    if bootstrap.models_artifact_uri.trim().is_empty() {
        return Ok(());
    }
    if !config.models_artifact_uri.trim().is_empty() {
        return Ok(());
    }

    let (metadata_field, metadata_uri) = if config.metadata_artifact_uri.trim().is_empty() {
        (
            "DBT_NOVA_METADATA_ARTIFACT_URI (from bootstrap)",
            bootstrap.metadata_artifact_uri.as_str(),
        )
    } else {
        (
            "DBT_NOVA_METADATA_ARTIFACT_URI",
            config.metadata_artifact_uri.as_str(),
        )
    };

    let metadata_local = resolve_artifact_uri_to_local(config, metadata_field, metadata_uri)?;
    ensure_regular_file(metadata_field, &metadata_local)?;
    let metadata_raw = read_small_text_file(Path::new(&metadata_local), config.manifest_max_bytes)?;
    let metadata = PrebuiltAssetsMetadata::from_json_str(&metadata_raw)?;
    if metadata.artifact_name_models.trim().is_empty() {
        return Err(DbtNovaError::InvalidParams(
            "bootstrap models_artifact_uri is set but metadata contract artifact_name_models is empty"
                .to_string(),
        ));
    }

    Ok(())
}

/// Apply bootstrap defaults when `DBT_NOVA_BOOTSTRAP_URI` is configured.
///
/// Explicit env/CLI values retain precedence over bootstrap values.
///
/// # Errors
///
/// Returns an error when bootstrap URI resolution, JSON parsing, or validation fails.
pub fn apply_bootstrap_defaults(config: &mut DbtNovaConfig) -> Result<BootstrapResolution> {
    let bootstrap_uri = config.bootstrap_uri.trim().to_string();
    if bootstrap_uri.is_empty() {
        let status = bootstrap_disabled_status();
        config.bootstrap_status = Some(status.clone());
        return Ok(BootstrapResolution { status });
    }
    if let Some(existing) = config.bootstrap_status.clone() {
        let existing_uri = existing
            .get("uri")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        if existing_uri == sanitize_uri(&bootstrap_uri) {
            return Ok(BootstrapResolution { status: existing });
        }
    }

    let evaluated_at_ms = current_time_ms();
    let local_path =
        resolve_artifact_uri_to_local(config, "DBT_NOVA_BOOTSTRAP_URI", &bootstrap_uri)?;
    ensure_regular_file("DBT_NOVA_BOOTSTRAP_URI", &local_path)?;
    let raw = read_small_text_file(Path::new(&local_path), config.manifest_max_bytes)?;
    let bootstrap = PrebuiltAssetsBootstrap::from_json_str(&raw)?;
    validate_models_metadata_contract(config, &bootstrap)?;

    let mut applied_fields = Vec::new();
    if should_apply_manifest_uri(config) {
        config.manifest_uri.clone_from(&bootstrap.manifest_uri);
        applied_fields.push("manifest_uri".to_string());
    }
    apply_if_empty(
        &mut config.storage_instance_id,
        &bootstrap.storage_instance_id,
        "storage_instance_id",
        &mut applied_fields,
    );
    apply_if_empty(
        &mut config.storage_artifact_uri,
        &bootstrap.storage_artifact_uri,
        "storage_artifact_uri",
        &mut applied_fields,
    );
    apply_if_empty(
        &mut config.metadata_artifact_uri,
        &bootstrap.metadata_artifact_uri,
        "metadata_artifact_uri",
        &mut applied_fields,
    );
    if !bootstrap.models_artifact_uri.trim().is_empty() {
        apply_if_empty(
            &mut config.models_artifact_uri,
            &bootstrap.models_artifact_uri,
            "models_artifact_uri",
            &mut applied_fields,
        );
    }

    let status = json!({
        "enabled": true,
        "uri": sanitize_uri(&bootstrap_uri),
        "contract_version": bootstrap.contract_version,
        "loaded": true,
        "validated": true,
        "applied_fields": applied_fields,
        "manifest_hash": bootstrap.manifest_hash,
        "last_evaluated_at_ms": evaluated_at_ms,
    });
    info!(
        uri = %sanitize_uri(&bootstrap_uri),
        applied_fields = ?status["applied_fields"],
        "resolved bootstrap contract"
    );
    config.bootstrap_status = Some(status.clone());
    Ok(BootstrapResolution { status })
}

/// Resolve bootstrap defaults, then apply standard runtime config finalization.
///
/// # Errors
///
/// Returns an error when bootstrap resolution or configuration validation fails.
pub fn prepare_runtime_config(config: &mut DbtNovaConfig) -> Result<BootstrapResolution> {
    let resolution = apply_bootstrap_defaults(config)?;
    config.ensure_storage_instance_id();
    config.ensure_embedding_cache_dir();
    let storage_root = config
        .storage_root_dir()
        .map(|path| path.display().to_string())
        .unwrap_or_default();
    if config.uses_home_storage_root_fallback() {
        warn!(
            manifest_uri = %sanitize_uri(&config.manifest_uri),
            storage_root = %storage_root,
            embedding_cache_dir = %config.search.embedding_cache_dir,
            "using implicit HOME-scoped Nova storage because manifest_uri is set and no manifest cache/storage dir override was provided"
        );
    }
    info!(
        storage_instance_id = %config.storage_instance_id,
        storage_root = %storage_root,
        embedding_cache_dir = %config.search.embedding_cache_dir,
        remote_artifact_mode = config.remote_artifact_mode_enabled(),
        bootstrap_uri = %sanitize_uri(&config.bootstrap_uri),
        storage_artifact_uri = %sanitize_uri(&config.storage_artifact_uri),
        metadata_artifact_uri = %sanitize_uri(&config.metadata_artifact_uri),
        models_artifact_uri = %sanitize_uri(&config.models_artifact_uri),
        "prepared runtime config"
    );
    config.validate()?;
    Ok(resolution)
}

#[cfg(test)]
mod tests {
    use super::{apply_bootstrap_defaults, prepare_runtime_config};
    use crate::config::DbtNovaConfig;
    use tempfile::TempDir;

    fn write_bootstrap(path: &std::path::Path) {
        let payload = r#"{
  "contract_version":"v1",
  "profile":"prod",
  "storage_instance_id":"analytics-prod",
  "manifest_uri":"dbfs:/FileStore/manifests/prod/manifest.json",
  "storage_artifact_uri":"dbfs:/FileStore/nova/prod/storage.tar.gz",
  "metadata_artifact_uri":"dbfs:/FileStore/nova/prod/metadata.json",
  "models_artifact_uri":"",
  "manifest_hash":"abc123",
  "dbt_nova_version":"0.0.2",
  "build_timestamp":"2026-01-01T10:00:00Z"
}"#;
        std::fs::write(path, payload).expect("write bootstrap");
    }

    fn write_bootstrap_with_uris(path: &std::path::Path, metadata_uri: &str, models_uri: &str) {
        let payload = format!(
            r#"{{
  "contract_version":"v1",
  "profile":"prod",
  "storage_instance_id":"analytics-prod",
  "manifest_uri":"dbfs:/FileStore/manifests/prod/manifest.json",
  "storage_artifact_uri":"dbfs:/FileStore/nova/prod/storage.tar.gz",
  "metadata_artifact_uri":"{metadata_uri}",
  "models_artifact_uri":"{models_uri}",
  "manifest_hash":"abc123",
  "dbt_nova_version":"0.0.2",
  "build_timestamp":"2026-01-01T10:00:00Z"
}}"#
        );
        std::fs::write(path, payload).expect("write bootstrap with custom URIs");
    }

    fn write_metadata(path: &std::path::Path, artifact_name_models: &str) {
        let payload = format!(
            r#"{{
  "contract_version":"v1",
  "manifest_hash":"abc123",
  "manifest_version":"abc123",
  "entity_count":1,
  "storage_instance_id":"analytics-prod",
  "dbt_nova_version":"0.0.2",
  "build_timestamp":"2026-01-01T10:00:00Z",
  "artifact_name_storage":"storage-artifact",
  "artifact_name_models":"{artifact_name_models}"
}}"#
        );
        std::fs::write(path, payload).expect("write metadata");
    }

    fn file_uri(path: &std::path::Path) -> String {
        format!("file://{}", path.display())
    }

    #[test]
    fn apply_bootstrap_defaults_returns_disabled_status_when_unset() {
        let mut config = DbtNovaConfig::default();
        let resolution = apply_bootstrap_defaults(&mut config).expect("bootstrap disabled");
        assert_eq!(resolution.status["enabled"], serde_json::json!(false));
        assert!(config.bootstrap_status.is_some());
    }

    #[test]
    fn apply_bootstrap_defaults_populates_missing_fields() {
        let temp_dir = TempDir::new().expect("tempdir");
        let metadata_path = temp_dir.path().join("metadata.json");
        write_metadata(&metadata_path, "models-artifact");
        let bootstrap_path = temp_dir.path().join("nova-bootstrap.json");
        write_bootstrap_with_uris(
            &bootstrap_path,
            &file_uri(&metadata_path),
            "dbfs:/FileStore/nova/prod/models.tar.gz",
        );

        let mut config = DbtNovaConfig {
            bootstrap_uri: file_uri(&bootstrap_path),
            storage_dir: temp_dir
                .path()
                .join(".dbt-nova")
                .to_string_lossy()
                .to_string(),
            ..DbtNovaConfig::default()
        };

        let resolution = apply_bootstrap_defaults(&mut config).expect("bootstrap loaded");
        assert_eq!(resolution.status["enabled"], serde_json::json!(true));
        assert_eq!(
            config.storage_instance_id, "analytics-prod",
            "bootstrap should fill missing storage instance id"
        );
        assert_eq!(
            config.manifest_uri,
            "dbfs:/FileStore/manifests/prod/manifest.json"
        );
        assert_eq!(
            config.storage_artifact_uri,
            "dbfs:/FileStore/nova/prod/storage.tar.gz"
        );
        assert_eq!(config.metadata_artifact_uri, file_uri(&metadata_path));
        assert_eq!(
            config.models_artifact_uri,
            "dbfs:/FileStore/nova/prod/models.tar.gz"
        );
    }

    #[test]
    fn apply_bootstrap_defaults_keeps_explicit_manifest_path() {
        let temp_dir = TempDir::new().expect("tempdir");
        let bootstrap_path = temp_dir.path().join("nova-bootstrap.json");
        write_bootstrap(&bootstrap_path);

        let mut config = DbtNovaConfig {
            bootstrap_uri: file_uri(&bootstrap_path),
            manifest_path: temp_dir
                .path()
                .join("manifest-custom.json")
                .to_string_lossy()
                .to_string(),
            storage_dir: temp_dir
                .path()
                .join(".dbt-nova")
                .to_string_lossy()
                .to_string(),
            ..DbtNovaConfig::default()
        };

        apply_bootstrap_defaults(&mut config).expect("bootstrap loaded");
        assert!(
            config.manifest_uri.is_empty(),
            "explicit manifest_path should not be overridden by bootstrap manifest_uri"
        );
    }

    #[test]
    fn apply_bootstrap_defaults_keeps_explicit_default_manifest_path() {
        let temp_dir = TempDir::new().expect("tempdir");
        let bootstrap_path = temp_dir.path().join("nova-bootstrap.json");
        write_bootstrap(&bootstrap_path);

        let mut config = DbtNovaConfig {
            bootstrap_uri: file_uri(&bootstrap_path),
            manifest_path: "manifest.json".to_string(),
            manifest_path_explicit: true,
            storage_dir: temp_dir
                .path()
                .join(".dbt-nova")
                .to_string_lossy()
                .to_string(),
            ..DbtNovaConfig::default()
        };

        apply_bootstrap_defaults(&mut config).expect("bootstrap loaded");
        assert!(
            config.manifest_uri.is_empty(),
            "explicit default manifest_path should not be overridden by bootstrap manifest_uri"
        );
    }

    #[test]
    fn apply_bootstrap_defaults_preserves_explicit_runtime_overrides() {
        let temp_dir = TempDir::new().expect("tempdir");
        let bootstrap_path = temp_dir.path().join("nova-bootstrap.json");
        write_bootstrap(&bootstrap_path);

        let mut config = DbtNovaConfig {
            bootstrap_uri: file_uri(&bootstrap_path),
            storage_dir: temp_dir
                .path()
                .join(".dbt-nova")
                .to_string_lossy()
                .to_string(),
            storage_instance_id: "explicit-instance".to_string(),
            storage_artifact_uri: "dbfs:/manual/storage.tar.gz".to_string(),
            ..DbtNovaConfig::default()
        };

        let resolution = apply_bootstrap_defaults(&mut config).expect("bootstrap loaded");
        assert_eq!(config.storage_instance_id, "explicit-instance");
        assert_eq!(config.storage_artifact_uri, "dbfs:/manual/storage.tar.gz");
        let applied_fields = resolution
            .status
            .get("applied_fields")
            .and_then(serde_json::Value::as_array)
            .expect("applied_fields");
        assert!(
            !applied_fields
                .iter()
                .any(|value| value == "storage_instance_id"),
            "explicit storage_instance_id must not be overwritten"
        );
        assert!(
            !applied_fields
                .iter()
                .any(|value| value == "storage_artifact_uri"),
            "explicit storage_artifact_uri must not be overwritten"
        );
    }

    #[test]
    fn prepare_runtime_config_uses_bootstrap_for_validation() {
        let temp_dir = TempDir::new().expect("tempdir");
        let bootstrap_path = temp_dir.path().join("nova-bootstrap.json");
        write_bootstrap(&bootstrap_path);

        let mut config = DbtNovaConfig {
            bootstrap_uri: file_uri(&bootstrap_path),
            storage_dir: temp_dir
                .path()
                .join(".dbt-nova")
                .to_string_lossy()
                .to_string(),
            manifest_allow_http: false,
            artifact_allow_http: false,
            ..DbtNovaConfig::default()
        };

        let resolution = prepare_runtime_config(&mut config).expect("runtime config should pass");
        assert_eq!(resolution.status["enabled"], serde_json::json!(true));
        assert_eq!(config.storage_instance_id, "analytics-prod");
    }

    #[test]
    fn prepare_runtime_config_rejects_bootstrap_home_storage_fallback() {
        let temp_dir = TempDir::new().expect("tempdir");
        let bootstrap_path = temp_dir.path().join("nova-bootstrap.json");
        write_bootstrap(&bootstrap_path);

        let mut config = DbtNovaConfig {
            bootstrap_uri: file_uri(&bootstrap_path),
            ..DbtNovaConfig::default()
        };

        let error = prepare_runtime_config(&mut config)
            .expect_err("bootstrap manifest_uri should require explicit storage anchor");
        assert!(error.to_string().contains("DBT_NOVA_MANIFEST_CACHE_DIR"));
        assert!(error.to_string().contains("DBT_NOVA_STORAGE_DIR"));
    }

    #[test]
    fn apply_bootstrap_defaults_rejects_models_uri_when_metadata_lacks_models_name() {
        let temp_dir = TempDir::new().expect("tempdir");
        let metadata_path = temp_dir.path().join("metadata.json");
        write_metadata(&metadata_path, "");

        let bootstrap_path = temp_dir.path().join("nova-bootstrap.json");
        write_bootstrap_with_uris(
            &bootstrap_path,
            &file_uri(&metadata_path),
            "dbfs:/FileStore/nova/prod/models.tar.gz",
        );

        let mut config = DbtNovaConfig {
            bootstrap_uri: file_uri(&bootstrap_path),
            storage_dir: temp_dir
                .path()
                .join(".dbt-nova")
                .to_string_lossy()
                .to_string(),
            ..DbtNovaConfig::default()
        };

        let error = apply_bootstrap_defaults(&mut config)
            .expect_err("models URI requires metadata artifact_name_models");
        assert!(error.to_string().contains("artifact_name_models"));
    }

    #[test]
    fn apply_bootstrap_defaults_accepts_models_uri_when_metadata_declares_models_name() {
        let temp_dir = TempDir::new().expect("tempdir");
        let metadata_path = temp_dir.path().join("metadata.json");
        write_metadata(&metadata_path, "models-artifact");

        let bootstrap_path = temp_dir.path().join("nova-bootstrap.json");
        write_bootstrap_with_uris(
            &bootstrap_path,
            &file_uri(&metadata_path),
            "dbfs:/FileStore/nova/prod/models.tar.gz",
        );

        let mut config = DbtNovaConfig {
            bootstrap_uri: file_uri(&bootstrap_path),
            storage_dir: temp_dir
                .path()
                .join(".dbt-nova")
                .to_string_lossy()
                .to_string(),
            ..DbtNovaConfig::default()
        };

        let resolution = apply_bootstrap_defaults(&mut config).expect("bootstrap loaded");
        assert_eq!(resolution.status["enabled"], serde_json::json!(true));
    }

    #[test]
    fn apply_bootstrap_defaults_skips_models_metadata_validation_when_models_uri_is_explicit() {
        let temp_dir = TempDir::new().expect("tempdir");
        let metadata_path = temp_dir.path().join("metadata.json");
        write_metadata(&metadata_path, "");

        let bootstrap_path = temp_dir.path().join("nova-bootstrap.json");
        write_bootstrap_with_uris(
            &bootstrap_path,
            &file_uri(&metadata_path),
            "dbfs:/FileStore/nova/prod/models.tar.gz",
        );

        let mut config = DbtNovaConfig {
            bootstrap_uri: file_uri(&bootstrap_path),
            storage_dir: temp_dir
                .path()
                .join(".dbt-nova")
                .to_string_lossy()
                .to_string(),
            models_artifact_uri: "dbfs:/manual/models.tar.gz".to_string(),
            ..DbtNovaConfig::default()
        };

        let resolution = apply_bootstrap_defaults(&mut config).expect("bootstrap loaded");
        assert_eq!(resolution.status["enabled"], serde_json::json!(true));
        assert_eq!(config.models_artifact_uri, "dbfs:/manual/models.tar.gz");
    }

    #[test]
    fn apply_bootstrap_defaults_uses_explicit_metadata_uri_for_models_validation() {
        let temp_dir = TempDir::new().expect("tempdir");
        let explicit_metadata_path = temp_dir.path().join("metadata-explicit.json");
        write_metadata(&explicit_metadata_path, "models-artifact");

        let bootstrap_path = temp_dir.path().join("nova-bootstrap.json");
        let missing_bootstrap_metadata_uri =
            file_uri(&temp_dir.path().join("missing-metadata.json"));
        write_bootstrap_with_uris(
            &bootstrap_path,
            &missing_bootstrap_metadata_uri,
            "dbfs:/FileStore/nova/prod/models.tar.gz",
        );

        let mut config = DbtNovaConfig {
            bootstrap_uri: file_uri(&bootstrap_path),
            storage_dir: temp_dir
                .path()
                .join(".dbt-nova")
                .to_string_lossy()
                .to_string(),
            metadata_artifact_uri: file_uri(&explicit_metadata_path),
            ..DbtNovaConfig::default()
        };

        let resolution = apply_bootstrap_defaults(&mut config).expect("bootstrap loaded");
        assert_eq!(resolution.status["enabled"], serde_json::json!(true));
        assert_eq!(
            config.metadata_artifact_uri,
            file_uri(&explicit_metadata_path)
        );
        assert_eq!(
            config.models_artifact_uri,
            "dbfs:/FileStore/nova/prod/models.tar.gz"
        );
    }
}
