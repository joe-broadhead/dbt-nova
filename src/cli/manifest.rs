use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::time::Instant;

use serde::Serialize;

use crate::cli::args::{ManifestLoadArgs, ManifestReloadArgs};
use crate::cli::output::{CliEnvelope, error_envelope};
use crate::config::DbtNovaConfig;
use crate::error::{DbtNovaError, Result};
use crate::manifest::bootstrap::prepare_runtime_config;
use crate::manifest::search::ManifestSearch;
use crate::utils::sanitize_uri;

use super::{DispatchError, DispatchResult, prepare_storage};

#[derive(Debug, Serialize)]
pub struct ManifestLoadData {
    pub source: String,
    pub entity_count: usize,
    pub manifest_hash: String,
    pub manifest_version: String,
    pub storage_path: String,
    pub elapsed_ms: u128,
    pub reused: ReuseInfo,
    pub search_ready: SearchReadyInfo,
    pub search_warnings: BTreeMap<String, String>,
    pub entity_counts: BTreeMap<String, usize>,
}

#[derive(Debug, Serialize)]
pub struct ReuseInfo {
    pub entity_store: bool,
    pub tantivy: bool,
    pub indexes: bool,
}

#[derive(Debug, Serialize)]
pub struct SearchReadyInfo {
    pub vector: bool,
    pub sparse: bool,
    pub reranker: bool,
}

/// Runs the `manifest load` CLI command.
///
/// # Errors
/// Returns an error if configuration is invalid, manifest loading fails, or output serialization fails.
pub async fn run_load_command(args: &ManifestLoadArgs) -> DispatchResult {
    run_manifest_command("manifest load", args.json, build_manifest_load_config(args)).await
}

/// Runs the `manifest reload` CLI command.
///
/// # Errors
/// Returns an error if configuration is invalid, manifest loading fails, or output serialization fails.
pub async fn run_reload_command(args: &ManifestReloadArgs) -> DispatchResult {
    run_manifest_command(
        "manifest reload",
        args.json,
        build_manifest_reload_config(args),
    )
    .await
}

async fn run_manifest_command(
    command_name: &'static str,
    json: bool,
    config_result: Result<DbtNovaConfig>,
) -> DispatchResult {
    let started = Instant::now();
    let config = config_result.map_err(|error| {
        render_or_propagate_error(command_name, json, error, started.elapsed().as_millis())
    })?;
    let load_result = execute_manifest_load(config).await.map_err(|error| {
        render_or_propagate_error(command_name, json, error, started.elapsed().as_millis())
    })?;
    let payload = payload_from_result(load_result).map_err(|error| {
        render_or_propagate_error(command_name, json, error, started.elapsed().as_millis())
    })?;

    if json {
        let output = render_success_json(command_name, &payload, started.elapsed().as_millis())
            .map_err(|error| DispatchError {
                error,
                rendered: false,
            })?;
        println!("{output}");
    } else {
        print_human_summary(&payload);
    }

    Ok(())
}

fn render_or_propagate_error(
    command_name: &str,
    json: bool,
    error: DbtNovaError,
    elapsed_ms: u128,
) -> DispatchError {
    if json {
        let envelope = error_envelope(command_name, &error, elapsed_ms);
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

fn print_human_summary(payload: &ManifestLoadData) {
    println!("manifest loaded successfully");
    println!("  source: {}", payload.source);
    println!("  entity_count: {}", payload.entity_count);
    println!("  manifest_hash: {}", payload.manifest_hash);
    println!("  manifest_version: {}", payload.manifest_version);
    println!("  storage_path: {}", payload.storage_path);
    println!("  elapsed_ms: {}", payload.elapsed_ms);
    println!("  entity_store_reused: {}", payload.reused.entity_store);
    println!("  tantivy_reused: {}", payload.reused.tantivy);
    println!("  indexes_reused: {}", payload.reused.indexes);
    println!(
        "  vector_search: {}",
        ready_label(payload.search_ready.vector)
    );
    println!(
        "  sparse_search: {}",
        ready_label(payload.search_ready.sparse)
    );
    println!("  reranker: {}", ready_label(payload.search_ready.reranker));
    if !payload.search_warnings.is_empty() {
        println!("  search_warnings:");
        for (component, warning) in &payload.search_warnings {
            println!("    {component}: {warning}");
        }
    }
    println!("  entity_counts:");
    for (resource_type, count) in &payload.entity_counts {
        println!("    {resource_type}: {count}");
    }
}

fn ready_label(ready: bool) -> &'static str {
    if ready { "ready" } else { "not_ready" }
}

fn render_success_json(
    command_name: &str,
    payload: &ManifestLoadData,
    elapsed_ms: u128,
) -> Result<String> {
    let envelope = CliEnvelope::success(command_name, payload, elapsed_ms);
    serde_json::to_string_pretty(&envelope)
        .map_err(|error| DbtNovaError::ServerError(error.to_string()))
}

fn payload_from_result(
    result: crate::manifest::loader::ManifestLoadResult,
) -> Result<ManifestLoadData> {
    let search = result.search;
    let storage_path = storage_path_for_search(&search)?;
    let mut entity_counts = BTreeMap::new();
    for (resource_type, count) in &search.entity_counts {
        entity_counts.insert(resource_type.clone(), *count);
    }
    let mut search_warnings = BTreeMap::new();
    for (component, warning) in &search.search_init_warnings {
        search_warnings.insert(component.clone(), warning.clone());
    }

    Ok(ManifestLoadData {
        source: sanitize_uri(&search.manifest_source_uri),
        entity_count: search.entity_count(),
        manifest_hash: search.manifest_hash.clone(),
        manifest_version: search.manifest_version.clone(),
        storage_path: storage_path.to_string_lossy().to_string(),
        elapsed_ms: result.elapsed_ms,
        reused: ReuseInfo {
            entity_store: result.entity_store_reused,
            tantivy: result.tantivy_reused,
            indexes: result.indexes_reused,
        },
        search_ready: SearchReadyInfo {
            vector: search.vector_search_ready(),
            sparse: search.sparse_search_ready(),
            reranker: search.reranker_ready(),
        },
        search_warnings,
        entity_counts,
    })
}

fn storage_path_for_search(search: &ManifestSearch) -> Result<PathBuf> {
    Ok(search
        .config()
        .storage_instance_root_dir()?
        .join("versions")
        .join(&search.manifest_version))
}

/// Builds a manifest-load configuration from environment defaults plus CLI overrides.
///
/// # Errors
/// Returns an error if overrides are invalid or resulting configuration fails validation.
pub fn build_manifest_load_config(args: &ManifestLoadArgs) -> Result<DbtNovaConfig> {
    let mut config = DbtNovaConfig::from_env();
    apply_manifest_common_overrides(
        &mut config,
        args.manifest_path.as_deref(),
        args.manifest_uri.as_deref(),
        args.storage_instance_id.as_deref(),
        args.cleanup_storage_on_start,
        args.read_only,
    )?;
    finalize_manifest_config(config)
}

/// Builds a manifest-reload configuration from environment defaults plus CLI overrides.
///
/// # Errors
/// Returns an error if overrides are invalid or resulting configuration fails validation.
pub fn build_manifest_reload_config(args: &ManifestReloadArgs) -> Result<DbtNovaConfig> {
    let mut config = DbtNovaConfig::from_env();
    apply_manifest_common_overrides(
        &mut config,
        args.manifest_path.as_deref(),
        args.manifest_uri.as_deref(),
        args.storage_instance_id.as_deref(),
        args.cleanup_storage_on_start,
        args.read_only,
    )?;
    if let Some(refresh_secs) = args.refresh_secs {
        config.manifest_refresh_secs = refresh_secs;
    }
    finalize_manifest_config(config)
}

fn is_safe_storage_instance_id(instance_id: &str) -> bool {
    if instance_id.is_empty()
        || instance_id.contains('/')
        || instance_id.contains('\\')
        || matches!(instance_id, "." | "..")
    {
        return false;
    }

    let mut components = Path::new(instance_id).components();
    matches!(
        (components.next(), components.next()),
        (Some(Component::Normal(_)), None)
    )
}

fn apply_manifest_common_overrides(
    config: &mut DbtNovaConfig,
    manifest_path: Option<&str>,
    manifest_uri: Option<&str>,
    storage_instance_id: Option<&str>,
    cleanup_storage_on_start: bool,
    read_only: bool,
) -> Result<()> {
    if let Some(path) = manifest_path {
        let trimmed = path.trim();
        if trimmed.is_empty() {
            return Err(DbtNovaError::InvalidParams(
                "--manifest-path cannot be empty".to_string(),
            ));
        }
        config.manifest_path = trimmed.to_string();
        config.manifest_path_explicit = true;
        config.manifest_uri.clear();
    }

    if let Some(uri) = manifest_uri {
        let trimmed = uri.trim();
        if trimmed.is_empty() {
            return Err(DbtNovaError::InvalidParams(
                "--manifest-uri cannot be empty".to_string(),
            ));
        }
        config.manifest_uri = trimmed.to_string();
    }

    if let Some(instance_id) = storage_instance_id {
        let trimmed = instance_id.trim();
        if trimmed.is_empty() {
            return Err(DbtNovaError::InvalidParams(
                "--storage-instance-id cannot be empty".to_string(),
            ));
        }
        if !is_safe_storage_instance_id(trimmed) {
            return Err(DbtNovaError::InvalidParams(
                "--storage-instance-id must be a single safe path segment".to_string(),
            ));
        }
        config.storage_instance_id = trimmed.to_string();
    }

    if cleanup_storage_on_start {
        config.cleanup_storage_on_start = true;
    }
    if read_only {
        config.storage_read_only = true;
    }

    Ok(())
}

fn finalize_manifest_config(mut config: DbtNovaConfig) -> Result<DbtNovaConfig> {
    // CLI manifest commands are one-shot; they do not start the background
    // refresh task. Keep refresh_secs unchanged so remote cache freshness rules
    // still apply during source resolution.
    let _bootstrap_resolution = prepare_runtime_config(&mut config)?;
    if !is_safe_storage_instance_id(config.storage_instance_id.trim()) {
        return Err(DbtNovaError::InvalidParams(
            "--storage-instance-id must be a single safe path segment".to_string(),
        ));
    }
    Ok(config)
}

pub(crate) async fn execute_manifest_load(
    config: DbtNovaConfig,
) -> Result<crate::manifest::loader::ManifestLoadResult> {
    if !config.storage_read_only {
        prepare_storage(&config)?;
    }
    tokio::task::spawn_blocking(move || ManifestSearch::new(config))
        .await
        .map_err(|error| DbtNovaError::ServerError(error.to_string()))?
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{
        build_manifest_load_config, build_manifest_reload_config, execute_manifest_load,
        payload_from_result, render_success_json,
    };
    use crate::cli::args::{ManifestLoadArgs, ManifestReloadArgs};
    use crate::config::DbtNovaConfig;
    use crate::tests::common::fixture_manifest_path_string;
    use tempfile::TempDir;

    #[test]
    fn build_manifest_load_config_uses_cli_overrides() {
        let args = ManifestLoadArgs {
            manifest_path: Some(fixture_manifest_path_string()),
            manifest_uri: None,
            storage_instance_id: Some("test-instance".to_string()),
            cleanup_storage_on_start: true,
            read_only: true,
            json: true,
        };

        let config = build_manifest_load_config(&args).expect("config");
        assert_eq!(config.storage_instance_id, "test-instance");
        assert!(config.cleanup_storage_on_start);
        assert!(config.storage_read_only);
        assert_eq!(
            config.manifest_refresh_secs,
            DbtNovaConfig::from_env().manifest_refresh_secs
        );
    }

    #[test]
    fn build_manifest_reload_config_uses_refresh_override() {
        let args = ManifestReloadArgs {
            manifest_path: Some(fixture_manifest_path_string()),
            refresh_secs: Some(120),
            storage_instance_id: Some("test-instance".to_string()),
            cleanup_storage_on_start: true,
            read_only: true,
            json: true,
            ..ManifestReloadArgs::default()
        };

        let config = build_manifest_reload_config(&args).expect("config");
        assert_eq!(config.manifest_refresh_secs, 120);
        assert_eq!(config.storage_instance_id, "test-instance");
        assert!(config.cleanup_storage_on_start);
        assert!(config.storage_read_only);
    }

    #[test]
    fn build_manifest_load_config_rejects_unsafe_instance_id() {
        let args = ManifestLoadArgs {
            manifest_path: Some(fixture_manifest_path_string()),
            storage_instance_id: Some("../unsafe".to_string()),
            ..ManifestLoadArgs::default()
        };

        let error = build_manifest_load_config(&args).expect_err("expected unsafe id rejection");
        assert!(
            error
                .to_string()
                .contains("--storage-instance-id must be a single safe path segment")
        );
    }

    #[test]
    fn build_manifest_reload_config_rejects_unsafe_instance_id() {
        let args = ManifestReloadArgs {
            manifest_path: Some(fixture_manifest_path_string()),
            storage_instance_id: Some("../unsafe".to_string()),
            ..ManifestReloadArgs::default()
        };

        let error = build_manifest_reload_config(&args).expect_err("expected unsafe id rejection");
        assert!(
            error
                .to_string()
                .contains("--storage-instance-id must be a single safe path segment")
        );
    }

    #[tokio::test]
    async fn execute_manifest_load_succeeds_for_fixture() {
        let args = ManifestLoadArgs {
            manifest_path: Some(fixture_manifest_path_string()),
            ..ManifestLoadArgs::default()
        };
        let mut config = build_manifest_load_config(&args).expect("config");
        config.search.enable_vector_search = false;
        config.search.enable_sparse_search = false;
        config.search.enable_reranker = false;
        let loaded = execute_manifest_load(config).await.expect("load result");

        assert!(loaded.search.entity_count() > 0);
        assert!(!loaded.search.manifest_hash.is_empty());
        assert!(!loaded.search.manifest_version.is_empty());
    }

    #[tokio::test]
    async fn execute_manifest_load_invalid_path_fails() {
        let temp_dir = TempDir::new().expect("temp dir");
        let args = ManifestLoadArgs {
            manifest_path: Some("tests/fixtures/missing-manifest.json".to_string()),
            storage_instance_id: Some("missing-manifest-test".to_string()),
            cleanup_storage_on_start: true,
            ..ManifestLoadArgs::default()
        };
        let mut config = build_manifest_load_config(&args).expect("config");
        config.storage_dir = temp_dir.path().to_string_lossy().to_string();
        config.search.enable_vector_search = false;
        config.search.enable_sparse_search = false;
        config.search.enable_reranker = false;
        let Err(err) = execute_manifest_load(config).await else {
            panic!("load should fail");
        };

        assert!(matches!(err, crate::error::DbtNovaError::ManifestError(_)));
    }

    #[tokio::test]
    async fn execute_manifest_load_read_only_skips_storage_cleanup() {
        let temp_dir = TempDir::new().expect("temp dir");

        let args = ManifestLoadArgs {
            manifest_path: Some(fixture_manifest_path_string()),
            storage_instance_id: Some("readonly-instance".to_string()),
            ..ManifestLoadArgs::default()
        };

        let mut bootstrap = build_manifest_load_config(&args).expect("bootstrap config");
        bootstrap.storage_dir = temp_dir
            .path()
            .join(".dbt-nova")
            .to_string_lossy()
            .to_string();
        bootstrap.cleanup_storage_on_start = false;
        bootstrap.search.enable_vector_search = false;
        bootstrap.search.enable_sparse_search = false;
        bootstrap.search.enable_reranker = false;
        let _ = execute_manifest_load(bootstrap)
            .await
            .expect("bootstrap load");

        let mut readonly = build_manifest_load_config(&args).expect("readonly config");
        readonly.storage_dir = temp_dir
            .path()
            .join(".dbt-nova")
            .to_string_lossy()
            .to_string();
        readonly.storage_read_only = true;
        readonly.cleanup_storage_on_start = true;
        readonly.search.enable_vector_search = false;
        readonly.search.enable_sparse_search = false;
        readonly.search.enable_reranker = false;

        let marker = readonly
            .storage_instance_root_dir()
            .expect("instance root")
            .join("marker.txt");
        fs::create_dir_all(marker.parent().expect("marker parent")).expect("create marker parent");
        fs::write(&marker, b"keep").expect("write marker");

        let _ = execute_manifest_load(readonly)
            .await
            .expect("read-only load should succeed");

        assert!(
            marker.exists(),
            "read-only manifest load should not clean storage"
        );
    }

    #[tokio::test]
    async fn execute_manifest_load_read_only_reuses_indexes_when_manifest_path_changes() {
        let temp_dir = TempDir::new().expect("temp dir");
        let copied_manifest_path = temp_dir.path().join("copied-manifest.json");
        fs::copy(fixture_manifest_path_string(), &copied_manifest_path).expect("copy manifest");

        let bootstrap_args = ManifestLoadArgs {
            manifest_path: Some(fixture_manifest_path_string()),
            storage_instance_id: Some("readonly-path-change".to_string()),
            ..ManifestLoadArgs::default()
        };
        let mut bootstrap = build_manifest_load_config(&bootstrap_args).expect("bootstrap config");
        bootstrap.storage_dir = temp_dir
            .path()
            .join(".dbt-nova")
            .to_string_lossy()
            .to_string();
        bootstrap.cleanup_storage_on_start = false;
        bootstrap.search.enable_vector_search = false;
        bootstrap.search.enable_sparse_search = false;
        bootstrap.search.enable_reranker = false;
        execute_manifest_load(bootstrap)
            .await
            .expect("bootstrap load should succeed");

        let readonly_args = ManifestLoadArgs {
            manifest_path: Some(copied_manifest_path.to_string_lossy().to_string()),
            storage_instance_id: Some("readonly-path-change".to_string()),
            read_only: true,
            ..ManifestLoadArgs::default()
        };
        let mut readonly = build_manifest_load_config(&readonly_args).expect("readonly config");
        readonly.storage_dir = temp_dir
            .path()
            .join(".dbt-nova")
            .to_string_lossy()
            .to_string();
        readonly.storage_read_only = true;
        readonly.cleanup_storage_on_start = false;
        readonly.search.enable_vector_search = false;
        readonly.search.enable_sparse_search = false;
        readonly.search.enable_reranker = false;

        let loaded = execute_manifest_load(readonly)
            .await
            .expect("read-only load should reuse existing indexes");
        assert!(loaded.entity_store_reused);
        assert!(loaded.tantivy_reused);
    }

    #[tokio::test]
    async fn execute_manifest_load_read_only_without_reusable_index_fails() {
        let temp_dir = TempDir::new().expect("temp dir");
        let args = ManifestLoadArgs {
            manifest_path: Some(fixture_manifest_path_string()),
            ..ManifestLoadArgs::default()
        };
        let mut config = build_manifest_load_config(&args).expect("config");
        config.storage_dir = temp_dir
            .path()
            .join(".dbt-nova")
            .to_string_lossy()
            .to_string();
        config.storage_instance_id = "read-only-no-cache".to_string();
        config.storage_read_only = true;
        config.search.enable_vector_search = false;
        config.search.enable_sparse_search = false;
        config.search.enable_reranker = false;

        let Err(err) = execute_manifest_load(config).await else {
            panic!("read-only without cache should fail");
        };
        assert!(
            err.to_string()
                .contains("Storage is read-only and no reusable index is available")
        );
    }

    #[tokio::test]
    async fn manifest_load_success_json_has_expected_envelope_shape() {
        let args = ManifestLoadArgs {
            manifest_path: Some(fixture_manifest_path_string()),
            ..ManifestLoadArgs::default()
        };
        let mut config = build_manifest_load_config(&args).expect("config");
        config.search.enable_vector_search = false;
        config.search.enable_sparse_search = false;
        config.search.enable_reranker = false;
        let loaded = execute_manifest_load(config).await.expect("load result");
        let payload = payload_from_result(loaded).expect("payload");
        let json = render_success_json("manifest load", &payload, 123).expect("json");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("parse json");

        assert_eq!(parsed["command"], serde_json::json!("manifest load"));
        assert_eq!(parsed["status"], serde_json::json!("success"));
        assert!(parsed["data"].is_object());
        assert_eq!(parsed["error"], serde_json::Value::Null);
        assert_eq!(parsed["meta"]["elapsed_ms"], serde_json::json!(123));
        assert!(parsed["meta"]["timestamp_ms"].as_u64().is_some());
        assert_eq!(
            parsed["meta"]["version"],
            serde_json::json!(env!("CARGO_PKG_VERSION"))
        );
        assert!(parsed["data"]["entity_count"].as_u64().is_some());
        assert!(parsed["data"]["manifest_hash"].as_str().is_some());
        assert!(parsed["data"]["manifest_version"].as_str().is_some());
        assert!(parsed["data"]["storage_path"].as_str().is_some());
        assert!(parsed["data"]["reused"].is_object());
        assert!(parsed["data"]["search_ready"].is_object());
    }
}
