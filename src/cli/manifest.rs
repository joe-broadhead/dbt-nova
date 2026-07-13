use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::Instant;

use serde::Serialize;
use serde_json::Value as JsonValue;

use crate::cli::args::{ManifestLoadArgs, ManifestReloadArgs, ManifestWarmArgs};
use crate::cli::output::{CliEnvelope, error_envelope};
use crate::config::DbtNovaConfig;
use crate::config::search::SearchConfig;
use crate::error::{DbtNovaError, Result};
use crate::manifest::bootstrap::prepare_runtime_config;
use crate::manifest::rkyv_embeddings::{self, EmbeddingsCacheLoad};
use crate::manifest::rkyv_sparse_embeddings::{self, SparseEmbeddingsCacheLoad};
use crate::manifest::search::ManifestSearch;
use crate::manifest::semantic_cache::default_sparse_model_name;
use crate::params::{ReloadManifestParams, WarmManifestParams};
use crate::responses::SuccessResponse;
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

#[derive(Debug, Clone, Copy, Serialize)]
pub struct SearchReadyInfo {
    pub vector: bool,
    pub sparse: bool,
    pub reranker: bool,
}

#[derive(Debug, Serialize)]
pub struct ManifestWarmData {
    pub source: String,
    pub manifest_hash: String,
    pub manifest_version: String,
    pub elapsed_ms: u128,
    pub force: bool,
    pub requested: SearchReadyInfo,
    pub persisted: SearchReadyInfo,
    pub ready: SearchReadyInfo,
    pub search_warnings: BTreeMap<String, String>,
    pub cache_paths: BTreeMap<String, String>,
}

#[derive(Debug)]
struct WarmCacheVerification {
    persisted: SearchReadyInfo,
    cache_paths: BTreeMap<String, String>,
}

#[derive(Debug, Serialize)]
struct ManifestWarmMcpSafetyPolicy {
    enabled_env: &'static str,
    uses_current_manifest_source: bool,
    storage_read_only_allowed: bool,
}

const MCP_ENABLE_MANIFEST_WARM_ENV: &str = "DBT_NOVA_MCP_ENABLE_MANIFEST_WARM";
pub const MCP_ENABLE_MANIFEST_RELOAD_ENV: &str = "DBT_NOVA_MCP_ENABLE_MANIFEST_RELOAD";

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

/// Runs the `manifest warm` CLI command.
///
/// # Errors
/// Returns an error if configuration is invalid, manifest loading fails, or requested caches
/// cannot be warmed successfully.
pub async fn run_warm_command(args: &ManifestWarmArgs) -> DispatchResult {
    let started = Instant::now();
    let warm_started_at = std::time::SystemTime::now();
    let config = build_manifest_warm_config(args).map_err(|error| {
        render_or_propagate_error(
            "manifest warm",
            args.json,
            error,
            started.elapsed().as_millis(),
        )
    })?;
    let requested = requested_warm_components(args, &config.search);
    let mut load_result = execute_manifest_load(config).await.map_err(|error| {
        render_or_propagate_error(
            "manifest warm",
            args.json,
            error,
            started.elapsed().as_millis(),
        )
    })?;
    warm_requested_query_models(&mut load_result.search, requested).map_err(|error| {
        render_or_propagate_error(
            "manifest warm",
            args.json,
            error,
            started.elapsed().as_millis(),
        )
    })?;

    let cache_verification =
        verify_requested_warm_caches(&load_result.search, requested, args.force, warm_started_at)
            .map_err(|error| {
            render_or_propagate_error(
                "manifest warm",
                args.json,
                error,
                started.elapsed().as_millis(),
            )
        })?;

    let payload = warm_payload_from_result(&load_result, requested, cache_verification, args.force);
    if args.json {
        let output = render_success_json("manifest warm", &payload, started.elapsed().as_millis())
            .map_err(|error| DispatchError {
                error,
                rendered: false,
            })?;
        println!("{output}");
    } else {
        print_warm_summary(&payload);
    }

    Ok(())
}

/// Builds the MCP/CLI-tool response for semantic cache warmup.
///
/// # Errors
/// Returns an error when warmup is not enabled, storage is read-only, manifest loading
/// fails, or requested semantic caches cannot be warmed.
pub async fn build_manifest_warm_tool_response(
    search: &ManifestSearch,
    params: &WarmManifestParams,
) -> Result<JsonValue> {
    require_mcp_manifest_warm_enabled()?;
    if search.config().storage_read_only {
        return Err(DbtNovaError::InvalidParams(
            "warm_manifest cannot run while storage is read-only".to_string(),
        ));
    }

    let started_at = Instant::now();
    let warm_started_at = std::time::SystemTime::now();
    let mut config = search.config().clone();
    let requested = requested_warm_components_from_flags(
        params.vector,
        params.sparse,
        params.reranker,
        &config.search,
    );
    apply_manifest_warm_settings(&mut config, requested, params.force)?;
    let mut load_result = execute_manifest_load(config).await?;
    warm_requested_query_models(&mut load_result.search, requested)?;
    let cache_verification = verify_requested_warm_caches(
        &load_result.search,
        requested,
        params.force,
        warm_started_at,
    )?;
    let mut payload = serde_json::to_value(warm_payload_from_result(
        &load_result,
        requested,
        cache_verification,
        params.force,
    ))
    .map_err(|error| DbtNovaError::ServerError(error.to_string()))?;
    if let Some(object) = payload.as_object_mut() {
        object.insert(
            "safety_policy".to_string(),
            serde_json::to_value(ManifestWarmMcpSafetyPolicy {
                enabled_env: MCP_ENABLE_MANIFEST_WARM_ENV,
                uses_current_manifest_source: true,
                storage_read_only_allowed: false,
            })
            .map_err(|error| DbtNovaError::ServerError(error.to_string()))?,
        );
        object.insert(
            "tool_elapsed_ms".to_string(),
            serde_json::json!(started_at.elapsed().as_millis()),
        );
    }
    serde_json::to_value(SuccessResponse::new(payload, 1))
        .map_err(|error| DbtNovaError::ServerError(error.to_string()))
}

/// Enforces the MCP safety gate for live manifest reloads that change source,
/// refresh cadence, or storage identity.
///
/// # Errors
/// Returns an invalid-params error when a state-changing MCP reload is disabled.
pub fn require_mcp_manifest_reload_enabled(params: &ReloadManifestParams) -> Result<()> {
    if !params.changes_runtime_source_or_storage()
        || mcp_env_enabled(MCP_ENABLE_MANIFEST_RELOAD_ENV)
    {
        return Ok(());
    }
    Err(DbtNovaError::InvalidParams(format!(
        "reload_manifest source changes are disabled for MCP use; call reload_manifest with no arguments to reload the current source, or set {MCP_ENABLE_MANIFEST_RELOAD_ENV}=1 to allow manifest_uri/manifest_path/refresh_secs/storage_instance_id changes"
    )))
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
    payload: &impl Serialize,
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

/// Builds a manifest-warm configuration from environment defaults plus CLI overrides.
///
/// # Errors
/// Returns an error if overrides are invalid or resulting configuration fails validation.
pub fn build_manifest_warm_config(args: &ManifestWarmArgs) -> Result<DbtNovaConfig> {
    let mut config = DbtNovaConfig::from_env();
    apply_manifest_common_overrides(
        &mut config,
        args.manifest_path.as_deref(),
        args.manifest_uri.as_deref(),
        args.storage_instance_id.as_deref(),
        false,
        false,
    )?;
    let requested = requested_warm_components(args, &config.search);
    apply_manifest_warm_settings(&mut config, requested, args.force)?;
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

fn requested_warm_components(args: &ManifestWarmArgs, search: &SearchConfig) -> SearchReadyInfo {
    requested_warm_components_from_flags(args.vector, args.sparse, args.reranker, search)
}

fn requested_warm_components_from_flags(
    vector: bool,
    sparse: bool,
    reranker: bool,
    search: &SearchConfig,
) -> SearchReadyInfo {
    let any_explicit = vector || sparse || reranker;
    if any_explicit {
        SearchReadyInfo {
            vector,
            sparse,
            reranker,
        }
    } else {
        SearchReadyInfo {
            vector: search.enable_vector_search,
            sparse: search.enable_sparse_search,
            reranker: search.enable_reranker,
        }
    }
}

fn apply_manifest_warm_settings(
    config: &mut DbtNovaConfig,
    requested: SearchReadyInfo,
    force: bool,
) -> Result<()> {
    if config.storage_read_only {
        return Err(DbtNovaError::InvalidParams(
            "manifest warm cannot run with read-only storage".to_string(),
        ));
    }
    config.cleanup_storage_on_start = false;
    config.search.enable_vector_search = requested.vector;
    config.search.enable_sparse_search = requested.sparse;
    config.search.enable_reranker = requested.reranker;
    config.search.cold_start_policy = crate::config::SearchColdStartPolicy::Build;
    config.search.force_rebuild_semantic_caches = force;
    Ok(())
}

fn require_mcp_manifest_warm_enabled() -> Result<()> {
    if mcp_env_enabled(MCP_ENABLE_MANIFEST_WARM_ENV) {
        return Ok(());
    }
    Err(DbtNovaError::InvalidParams(format!(
        "warm_manifest is disabled for MCP/tool-call use; set {MCP_ENABLE_MANIFEST_WARM_ENV}=1 to enable semantic cache warmup"
    )))
}

fn mcp_env_enabled(key: &str) -> bool {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_ascii_lowercase())
        .is_some_and(|value| matches!(value.as_str(), "1" | "true" | "yes" | "on"))
}

fn warm_requested_query_models(
    search: &mut ManifestSearch,
    requested: SearchReadyInfo,
) -> Result<()> {
    if requested.vector {
        let searcher = search.vector_search.as_ref().ok_or_else(|| {
            let cause = search
                .search_init_warnings
                .get("vector")
                .cloned()
                .unwrap_or_else(|| {
                    "vector search was not initialized during manifest warm".to_string()
                });
            DbtNovaError::ServerError(format!(
                "vector warm could not prepare query-model files. Cause: {cause}"
            ))
        })?;
        searcher.warm_query_model().map_err(|error| {
            DbtNovaError::ServerError(format!(
                "vector warm could not prepare query-model files. Cause: {error}"
            ))
        })?;
        search.search_init_warnings.remove("vector");
    }

    if requested.sparse {
        let searcher = search.sparse_search.as_ref().ok_or_else(|| {
            let cause = search
                .search_init_warnings
                .get("sparse")
                .cloned()
                .unwrap_or_else(|| {
                    "sparse search was not initialized during manifest warm".to_string()
                });
            DbtNovaError::ServerError(format!(
                "sparse warm could not prepare query-model files. Cause: {cause}"
            ))
        })?;
        searcher.warm_query_model().map_err(|error| {
            DbtNovaError::ServerError(format!(
                "sparse warm could not prepare query-model files. Cause: {error}"
            ))
        })?;
        search.search_init_warnings.remove("sparse");
    }

    if requested.reranker {
        let reranker = search.reranker.as_ref().ok_or_else(|| {
            let cause = search
                .search_init_warnings
                .get("reranker")
                .cloned()
                .unwrap_or_else(|| "reranker was not initialized during manifest warm".to_string());
            DbtNovaError::ServerError(format!(
                "reranker warm could not prepare query-model files. Cause: {cause}"
            ))
        })?;
        reranker.warm_query_model().map_err(|error| {
            DbtNovaError::ServerError(format!(
                "reranker warm could not prepare query-model files. Cause: {error}"
            ))
        })?;
        search.search_init_warnings.remove("reranker");
    }

    Ok(())
}

fn configured_vector_model_name(search: &ManifestSearch) -> String {
    if search.config().search.embedding_model.trim().is_empty() {
        crate::config::SearchConfig::default().embedding_model
    } else {
        search.config().search.embedding_model.trim().to_string()
    }
}

fn cache_path_is_fresh_enough(path: &Path, started_at: std::time::SystemTime) -> bool {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .is_ok_and(|modified| {
            modified
                .checked_add(std::time::Duration::from_secs(2))
                .is_some_and(|adjusted| adjusted >= started_at)
        })
}

fn verify_requested_warm_caches(
    search: &ManifestSearch,
    requested: SearchReadyInfo,
    force: bool,
    started_at: std::time::SystemTime,
) -> Result<WarmCacheVerification> {
    let mut persisted = SearchReadyInfo {
        vector: false,
        sparse: false,
        reranker: false,
    };
    let mut cache_paths = BTreeMap::new();

    if requested.vector {
        let model_name = configured_vector_model_name(search);
        match rkyv_embeddings::load_embeddings(
            &search.config().search,
            &model_name,
            &search.manifest_hash,
            None,
            search.config().search.embeddings_max_decompressed_bytes,
        ) {
            EmbeddingsCacheLoad::Hit { paths, .. } => {
                let cache_path = paths.preferred_path();
                if force && !cache_path_is_fresh_enough(&cache_path, started_at) {
                    return Err(DbtNovaError::ServerError(format!(
                        "vector warm completed without rewriting the manifest-scoped cache at {}",
                        cache_path.display()
                    )));
                }
                persisted.vector = true;
                cache_paths.insert(
                    "vector".to_string(),
                    cache_path.to_string_lossy().to_string(),
                );
            }
            EmbeddingsCacheLoad::Miss { paths, failure } => {
                return Err(DbtNovaError::ServerError(format!(
                    "vector warm did not persist a usable manifest-scoped cache at {}. Cause: {}",
                    paths.compressed_path.display(),
                    failure.summary()
                )));
            }
        }
    }

    if requested.sparse {
        match rkyv_sparse_embeddings::load_sparse_embeddings(
            &search.config().search,
            default_sparse_model_name(),
            &search.manifest_hash,
            None,
            search.config().search.embeddings_max_decompressed_bytes,
        ) {
            SparseEmbeddingsCacheLoad::Hit { paths, .. } => {
                let cache_path = paths.preferred_path();
                if force && !cache_path_is_fresh_enough(&cache_path, started_at) {
                    return Err(DbtNovaError::ServerError(format!(
                        "sparse warm completed without rewriting the manifest-scoped cache at {}",
                        cache_path.display()
                    )));
                }
                persisted.sparse = true;
                cache_paths.insert(
                    "sparse".to_string(),
                    cache_path.to_string_lossy().to_string(),
                );
            }
            SparseEmbeddingsCacheLoad::Miss { paths, failure } => {
                return Err(DbtNovaError::ServerError(format!(
                    "sparse warm did not persist a usable manifest-scoped cache at {}. Cause: {}",
                    paths.compressed_path.display(),
                    failure.summary()
                )));
            }
        }
    }

    Ok(WarmCacheVerification {
        persisted,
        cache_paths,
    })
}

fn warm_payload_from_result(
    result: &crate::manifest::loader::ManifestLoadResult,
    requested: SearchReadyInfo,
    cache_verification: WarmCacheVerification,
    force: bool,
) -> ManifestWarmData {
    let search = &result.search;
    let mut search_warnings = BTreeMap::new();
    for (component, warning) in &search.search_init_warnings {
        search_warnings.insert(component.clone(), warning.clone());
    }

    ManifestWarmData {
        source: sanitize_uri(&search.manifest_source_uri),
        manifest_hash: search.manifest_hash.clone(),
        manifest_version: search.manifest_version.clone(),
        elapsed_ms: result.elapsed_ms,
        force,
        requested,
        persisted: cache_verification.persisted,
        ready: SearchReadyInfo {
            vector: search.vector_search_ready(),
            sparse: search.sparse_search_ready(),
            reranker: search.reranker_ready(),
        },
        search_warnings,
        cache_paths: cache_verification.cache_paths,
    }
}

fn print_warm_summary(payload: &ManifestWarmData) {
    println!("manifest semantic caches warmed");
    println!("  source: {}", payload.source);
    println!("  manifest_hash: {}", payload.manifest_hash);
    println!("  manifest_version: {}", payload.manifest_version);
    println!("  elapsed_ms: {}", payload.elapsed_ms);
    println!("  force: {}", payload.force);
    if payload.requested.vector {
        println!("  vector_cache: {}", ready_label(payload.persisted.vector));
        println!("  vector: {}", ready_label(payload.ready.vector));
        if let Some(path) = payload.cache_paths.get("vector") {
            println!("  vector_cache_path: {path}");
        }
    }
    if payload.requested.sparse {
        println!("  sparse_cache: {}", ready_label(payload.persisted.sparse));
        println!("  sparse: {}", ready_label(payload.ready.sparse));
        if let Some(path) = payload.cache_paths.get("sparse") {
            println!("  sparse_cache_path: {path}");
        }
    }
    if payload.requested.reranker {
        println!("  reranker: {}", ready_label(payload.ready.reranker));
    }
    if !payload.search_warnings.is_empty() {
        println!("  search_warnings:");
        for (component, warning) in &payload.search_warnings {
            println!("    {component}: {warning}");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{Duration, SystemTime};

    use super::{
        SearchReadyInfo, build_manifest_load_config, build_manifest_reload_config,
        build_manifest_warm_config, execute_manifest_load, payload_from_result,
        render_success_json, requested_warm_components, verify_requested_warm_caches,
    };
    use crate::cli::args::{ManifestLoadArgs, ManifestReloadArgs, ManifestWarmArgs};
    use crate::config::DbtNovaConfig;
    use crate::config::SearchConfig;
    use crate::manifest::rkyv_embeddings::save_embeddings;
    use crate::manifest::rkyv_sparse_embeddings::save_sparse_embeddings;
    use crate::manifest::rkyv_types::{
        CachedEmbeddings, CachedSparseEmbeddings, RKYV_SCHEMA_VERSION,
    };
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

    #[test]
    fn build_manifest_warm_config_enables_requested_components() {
        let args = ManifestWarmArgs {
            manifest_path: Some(fixture_manifest_path_string()),
            vector: true,
            force: true,
            ..ManifestWarmArgs::default()
        };

        let config = build_manifest_warm_config(&args).expect("warm config");
        assert!(config.search.enable_vector_search);
        assert!(!config.search.enable_sparse_search);
        assert!(!config.search.enable_reranker);
        assert!(matches!(
            config.search.cold_start_policy,
            crate::config::SearchColdStartPolicy::Build
        ));
        assert!(config.search.force_rebuild_semantic_caches);
    }

    #[test]
    fn build_manifest_warm_config_can_enable_reranker() {
        let args = ManifestWarmArgs {
            manifest_path: Some(fixture_manifest_path_string()),
            reranker: true,
            ..ManifestWarmArgs::default()
        };

        let config = build_manifest_warm_config(&args).expect("warm config");
        assert!(!config.search.enable_vector_search);
        assert!(!config.search.enable_sparse_search);
        assert!(config.search.enable_reranker);
    }

    #[test]
    fn requested_warm_components_defaults_to_enabled_config_components() {
        let mut search = SearchConfig {
            enable_vector_search: false,
            enable_sparse_search: false,
            enable_reranker: false,
            ..Default::default()
        };
        let requested = requested_warm_components(&ManifestWarmArgs::default(), &search);
        assert!(!requested.vector);
        assert!(!requested.sparse);
        assert!(!requested.reranker);

        search.enable_sparse_search = true;
        let requested = requested_warm_components(&ManifestWarmArgs::default(), &search);
        assert!(!requested.vector);
        assert!(requested.sparse);
        assert!(!requested.reranker);
    }

    #[test]
    fn requested_warm_components_explicit_flags_override_config() {
        let search = SearchConfig {
            enable_vector_search: false,
            enable_sparse_search: false,
            enable_reranker: false,
            ..Default::default()
        };
        let requested = requested_warm_components(
            &ManifestWarmArgs {
                vector: true,
                ..ManifestWarmArgs::default()
            },
            &search,
        );
        assert!(requested.vector);
        assert!(!requested.sparse);
        assert!(!requested.reranker);
    }

    #[test]
    fn requested_warm_components_honors_explicit_reranker() {
        let search = SearchConfig::default();
        let requested = requested_warm_components(
            &ManifestWarmArgs {
                reranker: true,
                ..ManifestWarmArgs::default()
            },
            &search,
        );
        assert!(!requested.vector);
        assert!(!requested.sparse);
        assert!(requested.reranker);
    }

    #[tokio::test]
    async fn verify_requested_warm_caches_requires_force_rebuild_to_refresh_cache_files() {
        let args = ManifestLoadArgs {
            manifest_path: Some(fixture_manifest_path_string()),
            ..ManifestLoadArgs::default()
        };
        let mut config = build_manifest_load_config(&args).expect("config");
        let temp_dir = TempDir::new().expect("temp dir");
        config.storage_dir = temp_dir
            .path()
            .join("storage")
            .to_string_lossy()
            .to_string();
        config.search.embedding_cache_dir =
            temp_dir.path().join("cache").to_string_lossy().to_string();
        config.search.enable_vector_search = false;
        config.search.enable_sparse_search = false;
        config.search.enable_reranker = false;
        let loaded = execute_manifest_load(config).await.expect("load result");
        let manifest_hash = loaded.search.manifest_hash.clone();
        let search = loaded.search.config().search.clone();

        save_embeddings(
            &CachedEmbeddings {
                schema_version: RKYV_SCHEMA_VERSION,
                model_name: "intfloat/multilingual-e5-base".to_string(),
                manifest_hash: manifest_hash.clone(),
                entity_ids: vec!["model.test".to_string()],
                dense_embeddings: vec![vec![1.0, 0.0]],
                is_quantized: false,
                sparse_indices: None,
                sparse_values: None,
                ann_hyperplanes: None,
                ann_bucket_keys: None,
                ann_bucket_values: None,
            },
            &search,
        )
        .expect("save dense cache");
        save_sparse_embeddings(
            &CachedSparseEmbeddings {
                schema_version: RKYV_SCHEMA_VERSION,
                model_name: "Qdrant/Splade_PP_en_v1".to_string(),
                manifest_hash,
                entity_ids: vec!["model.test".to_string()],
                sparse_indices: vec![vec![1, 2]],
                sparse_values: vec![vec![0.4, 0.6]],
            },
            &search,
        )
        .expect("save sparse cache");

        let error = verify_requested_warm_caches(
            &loaded.search,
            SearchReadyInfo {
                vector: true,
                sparse: true,
                reranker: false,
            },
            true,
            SystemTime::now() + Duration::from_mins(1),
        )
        .expect_err("force warm should require freshly persisted cache files");
        assert!(
            error
                .to_string()
                .contains("completed without rewriting the manifest-scoped cache")
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
