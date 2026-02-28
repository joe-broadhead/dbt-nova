use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Instant;

use serde::Serialize;

use crate::cli::args::ManifestLoadArgs;
use crate::cli::output::{CliEnvelope, error_envelope};
use crate::config::DbtNovaConfig;
use crate::error::{DbtNovaError, Result};
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
    let started = Instant::now();
    let config = build_manifest_load_config(args)
        .map_err(|error| render_or_propagate_error(args, error, started.elapsed().as_millis()))?;
    let load_result = execute_manifest_load(config)
        .await
        .map_err(|error| render_or_propagate_error(args, error, started.elapsed().as_millis()))?;
    let payload = payload_from_result(load_result)
        .map_err(|error| render_or_propagate_error(args, error, started.elapsed().as_millis()))?;

    if args.json {
        let envelope =
            CliEnvelope::success("manifest load", &payload, started.elapsed().as_millis());
        let json = serde_json::to_string_pretty(&envelope).map_err(|error| DispatchError {
            error: DbtNovaError::ServerError(error.to_string()),
            rendered: false,
        })?;
        println!("{json}");
    } else {
        print_human_summary(&payload);
    }

    Ok(())
}

fn render_or_propagate_error(
    args: &ManifestLoadArgs,
    error: DbtNovaError,
    elapsed_ms: u128,
) -> DispatchError {
    if args.json {
        let envelope = error_envelope("manifest load", &error, elapsed_ms);
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
    println!("  entity_counts:");
    for (resource_type, count) in &payload.entity_counts {
        println!("    {resource_type}: {count}");
    }
}

fn ready_label(ready: bool) -> &'static str {
    if ready { "ready" } else { "not_ready" }
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

    if let Some(path) = args.manifest_path.as_ref() {
        let trimmed = path.trim();
        if trimmed.is_empty() {
            return Err(DbtNovaError::InvalidParams(
                "--manifest-path cannot be empty".to_string(),
            ));
        }
        config.manifest_path = trimmed.to_string();
        config.manifest_uri.clear();
    }

    if let Some(uri) = args.manifest_uri.as_ref() {
        let trimmed = uri.trim();
        if trimmed.is_empty() {
            return Err(DbtNovaError::InvalidParams(
                "--manifest-uri cannot be empty".to_string(),
            ));
        }
        config.manifest_uri = trimmed.to_string();
    }

    if let Some(instance_id) = args.storage_instance_id.as_ref() {
        let trimmed = instance_id.trim();
        if trimmed.is_empty() {
            return Err(DbtNovaError::InvalidParams(
                "--storage-instance-id cannot be empty".to_string(),
            ));
        }
        config.storage_instance_id = trimmed.to_string();
    }

    if args.cleanup_storage_on_start {
        config.cleanup_storage_on_start = true;
    }
    if args.read_only {
        config.storage_read_only = true;
    }

    // `manifest load` is explicitly one-shot because this command does not start
    // the background refresh task. Keep configured refresh_secs unchanged so
    // remote cache freshness rules still apply during source resolution.
    config.ensure_storage_instance_id();
    config.ensure_embedding_cache_dir();
    config.validate()?;
    Ok(config)
}

async fn execute_manifest_load(
    config: DbtNovaConfig,
) -> Result<crate::manifest::loader::ManifestLoadResult> {
    prepare_storage(&config)?;
    tokio::task::spawn_blocking(move || ManifestSearch::new(config))
        .await
        .map_err(|error| DbtNovaError::ServerError(error.to_string()))?
}

#[cfg(test)]
mod tests {
    use super::{build_manifest_load_config, execute_manifest_load};
    use crate::cli::args::ManifestLoadArgs;
    use crate::config::DbtNovaConfig;

    fn fixture_manifest_path() -> String {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("nova_manifest.json")
            .to_string_lossy()
            .to_string()
    }

    #[test]
    fn build_manifest_load_config_uses_cli_overrides() {
        let args = ManifestLoadArgs {
            manifest_path: Some(fixture_manifest_path()),
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

    #[tokio::test]
    async fn execute_manifest_load_succeeds_for_fixture() {
        let args = ManifestLoadArgs {
            manifest_path: Some(fixture_manifest_path()),
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
        let args = ManifestLoadArgs {
            manifest_path: Some("tests/fixtures/missing-manifest.json".to_string()),
            ..ManifestLoadArgs::default()
        };
        let mut config = build_manifest_load_config(&args).expect("config");
        config.search.enable_vector_search = false;
        config.search.enable_sparse_search = false;
        config.search.enable_reranker = false;
        let Err(err) = execute_manifest_load(config).await else {
            panic!("load should fail");
        };

        assert!(err.to_string().contains("Manifest error"));
    }
}
