use std::collections::HashMap;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::{Duration, Instant};

use blake3;
use moka::sync::Cache as MokaCache;
use serde_json::Value as JsonValue;

use crate::config::DbtNovaConfig;
use crate::error::{DbtNovaError, Result};
use crate::manifest::bootstrap::prepare_runtime_config;
use crate::manifest::prebuilt_assets_resolver::materialize_file_artifacts;
use crate::manifest::rkyv_indexes;
use crate::manifest::rkyv_types::{PersistedIndexes, RKYV_SCHEMA_VERSION};
use crate::manifest::search::{
    CompiledLayerRule, EntityCache, InUseLocks, ManifestSearch, compile_layer_rules,
};
use crate::manifest::source::{ManifestSignature, manifest_signature, resolve_manifest};
use crate::manifest::store::EntityStore;
use crate::manifest::tantivy_search::TantivySearcher;
use crate::manifest::vector_search::{Reranker, SparseSearcher, VectorSearcher};
use crate::utils::{CircuitBreaker, prune_dirs, sanitize_uri};
use tracing::{info, instrument, warn};

mod parse;
mod runtime;
mod storage;

use parse::{ManifestAccumulator, map_set_to_vec, map_vec_to_set, parse_manifest_file};
#[cfg(test)]
use runtime::has_apparent_malformed_ref;
use runtime::{
    build_column_lineage_aliases, build_manifest_health, combine_index_build_results, panic_message,
};
use storage::{
    MANIFEST_SIGNATURE_FILENAME, acquire_build_lock, acquire_in_use_locks, current_time_ms,
    manifest_signature_matches_for_reuse, read_current_version, read_manifest_signature,
    write_current_version, write_manifest_signature,
};

/// Structured output from manifest loading/index build.
pub struct ManifestLoadResult {
    pub search: ManifestSearch,
    pub entity_store_reused: bool,
    pub tantivy_reused: bool,
    pub indexes_reused: bool,
    pub elapsed_ms: u128,
}

struct StoragePreparation {
    instance_root: PathBuf,
    versions_root: PathBuf,
    signature: ManifestSignature,
    version_id: String,
    storage_dir: PathBuf,
    signature_path: PathBuf,
    build_lock: Option<File>,
    artifact_consumer_status: JsonValue,
    bootstrap_status: JsonValue,
}

struct ReuseArtifacts {
    current_version: Option<String>,
    storage_dir: PathBuf,
    signature_path: PathBuf,
    entities: Option<EntityStore>,
    reuse_store: bool,
    tantivy_opened: Option<TantivySearcher>,
}

struct CachedIndexData {
    by_path_prefix: HashMap<String, Vec<String>>,
    tests_by_entity: HashMap<String, Vec<String>>,
    tests_by_column: HashMap<String, Vec<String>>,
    used_cached_indexes: bool,
}

struct PathAndTestIndexes {
    by_path_prefix: HashMap<String, Vec<String>>,
    tests_by_entity: HashMap<String, Vec<String>>,
    tests_by_column: HashMap<String, Vec<String>>,
}

struct SearchBackends {
    tantivy: TantivySearcher,
    vector_search: Option<Arc<VectorSearcher>>,
    sparse_search: Option<Arc<SparseSearcher>>,
    search_init_warnings: HashMap<String, String>,
}

struct RuntimeComponents {
    entity_cache: Option<EntityCache>,
    lineage_cache: Option<MokaCache<String, Arc<JsonValue>>>,
    column_lineage_aliases: HashMap<String, HashMap<String, String>>,
    vector_breaker: CircuitBreaker,
    sparse_breaker: CircuitBreaker,
    reranker_breaker: CircuitBreaker,
    reranker: Option<Arc<Reranker>>,
    compiled_layer_rules: Vec<CompiledLayerRule>,
    manifest_health: JsonValue,
    parent_map: HashMap<String, Vec<String>>,
    child_map: HashMap<String, Vec<String>>,
    resource_type_by_id: HashMap<String, String>,
    search_init_warnings: HashMap<String, String>,
}

struct PreparedManifestData {
    accumulator: ManifestAccumulator,
    entities: EntityStore,
    cached_indexes: CachedIndexData,
    indexes_reused: bool,
}

struct ManifestParseContext<'a> {
    path: &'a Path,
    source_uri: &'a str,
    load_start: Instant,
}

struct PersistContext<'a> {
    signature_path: &'a Path,
    storage_dir: &'a Path,
    config: &'a DbtNovaConfig,
    needs_build: bool,
    update_current_version: bool,
    indexes_reused: bool,
    parent_map: &'a HashMap<String, Vec<String>>,
    child_map: &'a HashMap<String, Vec<String>>,
    accumulator: &'a ManifestAccumulator,
    cached_indexes: &'a CachedIndexData,
}

struct AssembleContext {
    config: DbtNovaConfig,
    entities: EntityStore,
    storage: StoragePreparation,
    runtime: RuntimeComponents,
    accumulator: ManifestAccumulator,
    cached_indexes: CachedIndexData,
    search_backends: SearchBackends,
    in_use_locks: InUseLocks,
    manifest_source_uri: String,
}

impl ManifestSearch {
    /// Build in-memory search indexes from a dbt manifest file.
    #[instrument(level = "info", skip(config), fields(manifest_path = %config.manifest_path))]
    /// Load the manifest and build indexes into a new search instance.
    ///
    /// # Errors
    /// Returns an error if the manifest cannot be loaded, parsed, or indexed.
    #[allow(clippy::new_ret_no_self)]
    pub fn new(mut config: DbtNovaConfig) -> Result<ManifestLoadResult> {
        let bootstrap_resolution = prepare_runtime_config(&mut config)?;
        let load_start = Instant::now();

        let manifest_resolution = resolve_manifest(&config)?;
        let manifest_path = manifest_resolution.local_path.clone();
        info!(
            source_uri = %manifest_resolution.source_uri,
            cached = manifest_resolution.cached,
            "resolved manifest source"
        );
        let mut storage =
            prepare_storage(&config, &manifest_resolution, bootstrap_resolution.status)?;
        let reused = prepare_reuse_state(&config, &storage)?;
        fs::create_dir_all(&reused.storage_dir)?;
        let in_use_locks = acquire_in_use_locks(&storage.instance_root, &reused.storage_dir)?;
        let needs_build = !reused.reuse_store || reused.tantivy_opened.is_none();
        let entity_store_reused = reused.reuse_store;
        ensure_build_reuse_state(&mut storage, &reused, &config, needs_build)?;
        let tantivy_reused = reused.tantivy_opened.is_some();
        let prepared_manifest_data = prepare_manifest_data(
            &config,
            &reused.storage_dir,
            &storage.signature,
            reused.reuse_store,
            reused.entities,
            &ManifestParseContext {
                path: &manifest_path,
                source_uri: &manifest_resolution.source_uri,
                load_start,
            },
        )?;
        let PreparedManifestData {
            accumulator,
            entities,
            cached_indexes,
            indexes_reused,
        } = prepared_manifest_data;
        let update_current_version = !needs_build
            && !config.storage_read_only
            && reused.storage_dir == storage.storage_dir
            && reused.current_version.as_deref() != Some(storage.version_id.as_str());
        let signature_path = reused.signature_path.clone();
        let storage_dir = reused.storage_dir.clone();
        let search_backends = build_search_backends_for_manifest(
            &mut config,
            &reused.storage_dir,
            &storage.signature,
            &accumulator,
            &entities,
            reused.tantivy_opened,
        )?;
        info!(
            elapsed_ms = load_start.elapsed().as_millis(),
            "tantivy index built"
        );
        let runtime = build_runtime_components(
            &config,
            load_start,
            &entities,
            &accumulator,
            &search_backends,
        )?;
        let persist_context = PersistContext {
            signature_path: &signature_path,
            storage_dir: &storage_dir,
            config: &config,
            needs_build,
            update_current_version,
            indexes_reused,
            parent_map: &runtime.parent_map,
            child_map: &runtime.child_map,
            accumulator: &accumulator,
            cached_indexes: &cached_indexes,
        };
        persist_storage_outputs(&mut storage, &persist_context)?;
        Ok(ManifestLoadResult {
            search: assemble_manifest_search(AssembleContext {
                config,
                entities,
                storage,
                runtime,
                accumulator,
                cached_indexes,
                search_backends,
                in_use_locks,
                manifest_source_uri: manifest_resolution.source_uri.clone(),
            }),
            entity_store_reused,
            tantivy_reused,
            indexes_reused,
            elapsed_ms: load_start.elapsed().as_millis(),
        })
    }
}

fn prepare_storage(
    config: &DbtNovaConfig,
    manifest_resolution: &crate::manifest::source::ManifestResolution,
    bootstrap_status: JsonValue,
) -> Result<StoragePreparation> {
    let storage_root = config.storage_instances_dir()?;
    if config.storage_max_instances > 0 {
        let exclude = [config.storage_instance_id.as_str()];
        prune_dirs(
            &storage_root,
            config.storage_max_instances,
            0,
            config.storage_max_bytes,
            &exclude,
        )?;
    }

    let instance_root = config.storage_instance_root_dir()?;
    fs::create_dir_all(&instance_root)?;
    let versions_root = instance_root.join("versions");
    fs::create_dir_all(&versions_root)?;

    let mut signature = manifest_signature(
        &manifest_resolution.local_path,
        &manifest_resolution.source_uri,
    )
    .map_err(|err| {
        DbtNovaError::ManifestError(format!(
            "Failed to read manifest file {}: {err}",
            manifest_resolution.source_uri
        ))
    })?;
    signature.prune_fingerprint = config.manifest_prune_fingerprint();
    signature.search_index_fingerprint = config.search.index_fingerprint();
    let version_hash = scoped_manifest_hash(&signature);
    let mut version_id = version_hash.chars().take(12).collect::<String>();
    if version_id.is_empty() {
        version_id = "unknown".to_string();
    }

    let storage_dir = versions_root.join(&version_id);
    let signature_path = storage_dir.join(MANIFEST_SIGNATURE_FILENAME);
    let build_lock = Some(acquire_build_lock(
        &instance_root,
        config.storage_build_lock_wait_secs,
    )?);

    let mut artifact_consumer_status = build_artifact_consumer_status(config, None, None);
    if config.remote_artifact_mode_enabled() {
        let evaluated_at_ms = current_time_ms();
        let materialization = materialize_file_artifacts(config, &version_hash)?;
        if let Some(outcome) = materialization {
            let last_materialized_at_ms =
                if outcome.storage_materialized || outcome.models_materialized {
                    Some(evaluated_at_ms)
                } else {
                    None
                };
            artifact_consumer_status = build_artifact_consumer_status(
                config,
                Some(&outcome),
                Some((evaluated_at_ms, last_materialized_at_ms)),
            );
            info!(
                storage_materialized = outcome.storage_materialized,
                models_materialized = outcome.models_materialized,
                "evaluated remote artifact materialization"
            );
        }
    }

    Ok(StoragePreparation {
        instance_root,
        versions_root,
        signature,
        version_id,
        storage_dir,
        signature_path,
        build_lock,
        artifact_consumer_status,
        bootstrap_status,
    })
}

fn load_reusable_artifacts(
    config: &DbtNovaConfig,
    instance_root: &Path,
    versions_root: &Path,
    signature: &ManifestSignature,
    initial_storage_dir: &Path,
    initial_signature_path: &Path,
) -> Result<ReuseArtifacts> {
    let current_version = read_current_version(instance_root)?;
    let mut storage_dir = initial_storage_dir.to_path_buf();
    let mut signature_path = initial_signature_path.to_path_buf();
    let mut entities: Option<EntityStore> = None;
    let mut reuse_store = false;
    let mut tantivy_opened: Option<TantivySearcher> = None;
    let mut matched_signature = false;

    if let Some(current) = &current_version {
        let current_dir = versions_root.join(current);
        let current_sig_path = current_dir.join(MANIFEST_SIGNATURE_FILENAME);
        if let Some(existing) = read_manifest_signature(&current_sig_path)?
            && manifest_signature_matches_for_reuse(&existing, signature)
        {
            storage_dir = current_dir;
            signature_path = current_sig_path;
            matched_signature = true;
            if let Ok(store) = EntityStore::open(&storage_dir) {
                entities = Some(store);
                reuse_store = true;
            }
            if let Ok(opened) = TantivySearcher::open(&storage_dir, &config.search) {
                tantivy_opened = opened;
            }
        }
    }

    if !matched_signature
        && let Some(existing) = read_manifest_signature(initial_signature_path)?
        && manifest_signature_matches_for_reuse(&existing, signature)
    {
        storage_dir = initial_storage_dir.to_path_buf();
        signature_path = initial_signature_path.to_path_buf();
        if let Ok(store) = EntityStore::open(&storage_dir) {
            entities = Some(store);
            reuse_store = true;
        }
        if let Ok(opened) = TantivySearcher::open(&storage_dir, &config.search) {
            tantivy_opened = opened;
        }
    }

    Ok(ReuseArtifacts {
        current_version,
        storage_dir,
        signature_path,
        entities,
        reuse_store,
        tantivy_opened,
    })
}

fn prepare_reuse_state(
    config: &DbtNovaConfig,
    storage: &StoragePreparation,
) -> Result<ReuseArtifacts> {
    let reused = load_reusable_artifacts(
        config,
        &storage.instance_root,
        &storage.versions_root,
        &storage.signature,
        &storage.storage_dir,
        &storage.signature_path,
    )?;
    prune_storage_versions(
        config,
        &storage.versions_root,
        reused.current_version.as_deref(),
        &storage.version_id,
    )?;
    Ok(reused)
}

fn prune_storage_versions(
    config: &DbtNovaConfig,
    versions_root: &Path,
    current_version: Option<&str>,
    version_id: &str,
) -> Result<()> {
    let mut exclude_versions = Vec::new();
    if let Some(current) = current_version {
        exclude_versions.push(current);
    }
    exclude_versions.push(version_id);
    let min_versions = if config.storage_max_instances == 0 {
        config.storage_min_versions
    } else {
        config
            .storage_min_versions
            .min(config.storage_max_instances)
    };
    prune_dirs(
        versions_root,
        config.storage_max_instances,
        min_versions,
        config.storage_max_bytes,
        &exclude_versions,
    )?;
    Ok(())
}

fn load_cached_indexes(
    storage_dir: &Path,
    signature: &ManifestSignature,
    reuse_store: bool,
    accumulator: &mut ManifestAccumulator,
) -> CachedIndexData {
    let mut cached_index_data = CachedIndexData {
        by_path_prefix: HashMap::new(),
        tests_by_entity: HashMap::new(),
        tests_by_column: HashMap::new(),
        used_cached_indexes: false,
    };

    if let Some(cached) = reuse_store
        .then(|| rkyv_indexes::try_load_indexes(storage_dir, &signature.content_hash))
        .flatten()
    {
        info!("loaded indexes from cache");
        accumulator.parent_map = map_vec_to_set(cached.parent_map);
        accumulator.child_map = map_vec_to_set(cached.child_map);
        accumulator.by_resource_type = cached.by_resource_type;
        accumulator.by_package = cached.by_package;
        accumulator.by_tag = cached.by_tag;
        accumulator.by_database_schema = cached.by_database_schema;
        accumulator.name_to_keys = cached.name_to_keys;
        accumulator.unique_id_to_resource_type = cached.unique_id_to_resource_type;
        accumulator.unique_id_to_path = cached.unique_id_to_path;
        accumulator.unique_id_to_tag_strings = cached.unique_id_to_tag_strings;
        accumulator.entity_counts = cached.entity_counts;
        accumulator.manifest_metadata = serde_json::from_str(&cached.manifest_metadata_json)
            .unwrap_or_else(|err| {
                tracing::warn!(
                    error = %err,
                    "failed to decode cached manifest metadata; using null"
                );
                JsonValue::Null
            });
        accumulator.seen_unique_ids = accumulator
            .unique_id_to_resource_type
            .keys()
            .cloned()
            .collect();
        cached_index_data.by_path_prefix = cached.by_path_prefix;
        cached_index_data.tests_by_entity = cached.tests_by_entity;
        cached_index_data.tests_by_column = cached.tests_by_column;
        cached_index_data.used_cached_indexes = true;
    }

    cached_index_data
}

fn ensure_build_reuse_state(
    storage: &mut StoragePreparation,
    reused: &ReuseArtifacts,
    config: &DbtNovaConfig,
    needs_build: bool,
) -> Result<()> {
    if reused.reuse_store {
        info!("reusing entity store from existing storage");
    }
    if reused.tantivy_opened.is_some() {
        info!("reusing tantivy index from existing storage");
    }
    if config.storage_read_only && needs_build {
        storage.build_lock.take();
        return Err(DbtNovaError::ServerError(
            "Storage is read-only and no reusable index is available".to_string(),
        ));
    }
    Ok(())
}

fn prepare_manifest_data(
    config: &DbtNovaConfig,
    storage_dir: &Path,
    signature: &ManifestSignature,
    reuse_store: bool,
    existing_entities: Option<EntityStore>,
    parse_context: &ManifestParseContext<'_>,
) -> Result<PreparedManifestData> {
    let manifest_path = parse_context.path;
    let source_uri = parse_context.source_uri;
    let load_start = parse_context.load_start;
    let mut accumulator = ManifestAccumulator::new(storage_dir, !reuse_store)?;
    let mut cached_indexes =
        load_cached_indexes(storage_dir, signature, reuse_store, &mut accumulator);
    let indexes_reused = cached_indexes.used_cached_indexes;

    let parse_manifest = !reuse_store || !cached_indexes.used_cached_indexes;
    if parse_manifest {
        parse_manifest_file(
            manifest_path,
            source_uri,
            &config.manifest_prune_allow_ids,
            &config.manifest_prune_deny_ids,
            &mut accumulator,
            load_start,
        )?;
    } else {
        info!("reused entity/index caches; skipped manifest parse");
    }

    let entities = match existing_entities {
        Some(store) => store,
        None => accumulator.finish_store()?,
    };
    let entity_count = entities.len();
    info!(
        elapsed_ms = load_start.elapsed().as_millis(),
        entity_count, "entity store built"
    );

    if !cached_indexes.used_cached_indexes {
        let path_and_test_indexes = build_path_and_test_indexes(&accumulator, &entities)?;
        cached_indexes.by_path_prefix = path_and_test_indexes.by_path_prefix;
        cached_indexes.tests_by_entity = path_and_test_indexes.tests_by_entity;
        cached_indexes.tests_by_column = path_and_test_indexes.tests_by_column;
    }

    Ok(PreparedManifestData {
        accumulator,
        entities,
        cached_indexes,
        indexes_reused,
    })
}

fn build_path_and_test_indexes(
    accumulator: &ManifestAccumulator,
    entities: &EntityStore,
) -> Result<PathAndTestIndexes> {
    let mut by_path_prefix: HashMap<String, Vec<String>> = HashMap::new();
    for (unique_id, path) in &accumulator.unique_id_to_path {
        let parts: Vec<&str> = path.split('/').collect();
        let mut prefix = String::new();
        for (i, part) in parts.iter().enumerate() {
            if i > 0 {
                prefix.push('/');
            }
            prefix.push_str(part);
            by_path_prefix
                .entry(prefix.clone())
                .or_default()
                .push(unique_id.clone());
        }
    }

    let mut tests_by_entity: HashMap<String, Vec<String>> = HashMap::new();
    let mut tests_by_column: HashMap<String, Vec<String>> = HashMap::new();
    if let Some(tests) = accumulator.by_resource_type.get("test") {
        for test_id in tests {
            if let Some(test) = entities.get_blocking(test_id)? {
                let test_json = test.to_json_value();
                if let Some(attached) = test_json.get("attached_node").and_then(|a| a.as_str()) {
                    tests_by_entity
                        .entry(attached.to_string())
                        .or_default()
                        .push(test_id.clone());

                    if let Some(col) = test_json.get("column_name").and_then(|c| c.as_str()) {
                        let key = format!("{attached}:{col}");
                        tests_by_column
                            .entry(key)
                            .or_default()
                            .push(test_id.clone());
                    }
                } else if let Some(attached) = test_json
                    .get("depends_on")
                    .and_then(|d| d.get("nodes"))
                    .and_then(|n| n.as_array())
                    .and_then(|nodes| nodes.iter().find_map(|n| n.as_str()))
                {
                    tests_by_entity
                        .entry(attached.to_string())
                        .or_default()
                        .push(test_id.clone());
                }

                if let Some(deps) = test_json
                    .get("depends_on")
                    .and_then(|d| d.get("nodes"))
                    .and_then(|n| n.as_array())
                {
                    for dep in deps {
                        if let Some(dep_id) = dep.as_str() {
                            let entry = tests_by_entity.entry(dep_id.to_string()).or_default();
                            if !entry.contains(test_id) {
                                entry.push(test_id.clone());
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(PathAndTestIndexes {
        by_path_prefix,
        tests_by_entity,
        tests_by_column,
    })
}

fn build_search_indexes(
    storage_dir: &Path,
    accumulator: &ManifestAccumulator,
    entities: &EntityStore,
    search_config: &crate::config::SearchConfig,
    tantivy_opened: Option<TantivySearcher>,
) -> Result<SearchBackends> {
    std::thread::scope(|scope| -> Result<_> {
        let mut opened = tantivy_opened;
        let tantivy_handle = if opened.is_none() {
            Some(scope.spawn(|| {
                TantivySearcher::build(
                    storage_dir,
                    &accumulator.unique_id_to_resource_type,
                    &accumulator.unique_id_to_path,
                    &accumulator.unique_id_to_tag_strings,
                    search_config,
                )
            }))
        } else {
            None
        };

        let tantivy_result = if let Some(opened) = opened.take() {
            Ok(opened)
        } else {
            match tantivy_handle {
                Some(handle) => match handle.join() {
                    Ok(result) => result,
                    Err(err) => Err(DbtNovaError::ServerError(format!(
                        "Tantivy index build thread panicked: {}",
                        panic_message(&err)
                    ))),
                },
                None => Err(DbtNovaError::ServerError(
                    "Tantivy build handle missing".to_string(),
                )),
            }
        };

        let vector_result = VectorSearcher::build(entities, search_config);
        let sparse_result = SparseSearcher::build(entities, search_config);

        let (tantivy, vector_search, sparse_search) =
            combine_index_build_results(tantivy_result, vector_result, sparse_result)?;
        let (vector_search, vector_warning) = vector_search.into_parts();
        let (sparse_search, sparse_warning) = sparse_search.into_parts();
        let mut search_init_warnings = HashMap::new();
        if let Some(warning) = vector_warning {
            search_init_warnings.insert("vector".to_string(), warning);
        }
        if let Some(warning) = sparse_warning {
            search_init_warnings.insert("sparse".to_string(), warning);
        }

        Ok(SearchBackends {
            tantivy,
            vector_search: vector_search.map(Arc::new),
            sparse_search: sparse_search.map(Arc::new),
            search_init_warnings,
        })
    })
}

fn build_search_backends_for_manifest(
    config: &mut DbtNovaConfig,
    storage_dir: &Path,
    signature: &ManifestSignature,
    accumulator: &ManifestAccumulator,
    entities: &EntityStore,
    tantivy_opened: Option<TantivySearcher>,
) -> Result<SearchBackends> {
    config.search.manifest_hash = Some(scoped_manifest_hash(signature));
    build_search_indexes(
        storage_dir,
        accumulator,
        entities,
        &config.search,
        tantivy_opened,
    )
}

fn scoped_manifest_hash(signature: &ManifestSignature) -> String {
    if signature.prune_fingerprint.is_empty() && signature.search_index_fingerprint.is_empty() {
        signature.content_hash.clone()
    } else {
        let payload = serde_json::json!({
            "content_hash": signature.content_hash.as_str(),
            "prune_fingerprint": signature.prune_fingerprint.as_str(),
            "search_index_fingerprint": signature.search_index_fingerprint.as_str(),
        });
        blake3::hash(payload.to_string().as_bytes())
            .to_hex()
            .to_string()
    }
}

fn build_runtime_components(
    config: &DbtNovaConfig,
    load_start: Instant,
    entities: &EntityStore,
    accumulator: &ManifestAccumulator,
    search_backends: &SearchBackends,
) -> Result<RuntimeComponents> {
    let entity_cache = EntityCache::build(config.entity_cache_size);
    let lineage_cache = if config.search.lineage_cache_size > 0 {
        Some(
            MokaCache::builder()
                .max_capacity(config.search.lineage_cache_size as u64)
                .build(),
        )
    } else {
        None
    };
    let column_lineage_aliases = build_column_lineage_aliases(entities, config)?;

    let breaker_threshold = config.search.search_circuit_failure_threshold;
    let breaker_duration = Duration::from_secs(config.search.search_circuit_open_seconds);
    let vector_breaker = CircuitBreaker::new(breaker_threshold, breaker_duration);
    let sparse_breaker = CircuitBreaker::new(breaker_threshold, breaker_duration);
    let reranker_breaker = CircuitBreaker::new(breaker_threshold, breaker_duration);

    if search_backends.vector_search.is_some() {
        info!(
            elapsed_ms = load_start.elapsed().as_millis(),
            "vector index built"
        );
    }

    if search_backends.sparse_search.is_some() {
        info!(
            elapsed_ms = load_start.elapsed().as_millis(),
            "sparse index built"
        );
    }

    let mut search_init_warnings = search_backends.search_init_warnings.clone();
    let (reranker, reranker_warning) = Reranker::build(&config.search)?.into_parts();
    if let Some(warning) = reranker_warning {
        search_init_warnings.insert("reranker".to_string(), warning);
    }
    let compiled_layer_rules = compile_layer_rules(&config.layer_rules);
    let manifest_health = build_manifest_health(
        entities,
        &accumulator.parent_map,
        &accumulator.unique_id_to_resource_type,
    )?;

    Ok(RuntimeComponents {
        entity_cache,
        lineage_cache,
        column_lineage_aliases,
        vector_breaker,
        sparse_breaker,
        reranker_breaker,
        reranker: reranker.map(Arc::new),
        compiled_layer_rules,
        manifest_health,
        parent_map: map_set_to_vec(accumulator.parent_map.clone()),
        child_map: map_set_to_vec(accumulator.child_map.clone()),
        resource_type_by_id: accumulator.unique_id_to_resource_type.clone(),
        search_init_warnings,
    })
}

fn persist_storage_outputs(
    storage: &mut StoragePreparation,
    context: &PersistContext<'_>,
) -> Result<()> {
    if context.needs_build {
        write_manifest_signature(context.signature_path, &storage.signature)?;
    }
    if context.needs_build || context.update_current_version {
        write_current_version(&storage.instance_root, &storage.version_id)?;
    }

    let persist_indexes = !context.indexes_reused && !context.config.storage_read_only;
    if persist_indexes {
        let manifest_metadata_json = serde_json::to_string(&context.accumulator.manifest_metadata)
            .unwrap_or_else(|err| {
                tracing::warn!(
                    error = %err,
                    "failed to encode manifest metadata for cache; using null"
                );
                "null".to_string()
            });
        let indexes = PersistedIndexes {
            schema_version: RKYV_SCHEMA_VERSION,
            manifest_hash: storage.signature.content_hash.clone(),
            parent_map: context.parent_map.clone(),
            child_map: context.child_map.clone(),
            by_resource_type: context.accumulator.by_resource_type.clone(),
            by_package: context.accumulator.by_package.clone(),
            by_tag: context.accumulator.by_tag.clone(),
            by_database_schema: context.accumulator.by_database_schema.clone(),
            name_to_keys: context.accumulator.name_to_keys.clone(),
            by_path_prefix: context.cached_indexes.by_path_prefix.clone(),
            tests_by_entity: context.cached_indexes.tests_by_entity.clone(),
            tests_by_column: context.cached_indexes.tests_by_column.clone(),
            unique_id_to_resource_type: context.accumulator.unique_id_to_resource_type.clone(),
            unique_id_to_path: context.accumulator.unique_id_to_path.clone(),
            unique_id_to_tag_strings: context.accumulator.unique_id_to_tag_strings.clone(),
            entity_counts: context.accumulator.entity_counts.clone(),
            manifest_metadata_json,
        };
        if let Err(err) = rkyv_indexes::save_indexes(&indexes, context.storage_dir) {
            tracing::warn!(error = %err, "failed to save index cache");
        }
    }

    storage.build_lock.take();
    Ok(())
}

fn assemble_manifest_search(context: AssembleContext) -> ManifestSearch {
    let AssembleContext {
        config,
        entities,
        storage,
        runtime,
        accumulator,
        cached_indexes,
        search_backends,
        in_use_locks,
        manifest_source_uri,
    } = context;
    ManifestSearch {
        config,
        entities: Arc::new(entities),
        parent_map: runtime.parent_map,
        child_map: runtime.child_map,
        by_resource_type: accumulator.by_resource_type,
        by_package: accumulator.by_package,
        by_tag: accumulator.by_tag,
        by_database_schema: accumulator.by_database_schema,
        name_to_keys: accumulator.name_to_keys,
        resource_type_by_id: runtime.resource_type_by_id,
        tantivy: search_backends.tantivy,
        vector_search: search_backends.vector_search,
        sparse_search: search_backends.sparse_search,
        reranker: runtime.reranker,
        compiled_layer_rules: runtime.compiled_layer_rules,
        tests_by_entity: cached_indexes.tests_by_entity,
        tests_by_column: cached_indexes.tests_by_column,
        by_path_prefix: cached_indexes.by_path_prefix,
        column_lineage_aliases: runtime.column_lineage_aliases,
        manifest_metadata: accumulator.manifest_metadata,
        manifest_health: runtime.manifest_health,
        artifact_consumer: storage.artifact_consumer_status,
        bootstrap: storage.bootstrap_status,
        entity_counts: accumulator.entity_counts,
        entity_cache: runtime.entity_cache,
        lineage_cache: runtime.lineage_cache,
        search_init_warnings: runtime.search_init_warnings,
        entity_cache_hits: AtomicU64::new(0),
        entity_cache_misses: AtomicU64::new(0),
        lineage_cache_hits: AtomicU64::new(0),
        lineage_cache_misses: AtomicU64::new(0),
        manifest_source_uri,
        manifest_hash: scoped_manifest_hash(&storage.signature),
        manifest_len: storage.signature.len,
        manifest_modified_ms: storage.signature.modified_ms,
        manifest_version: storage.version_id,
        loaded_at_ms: current_time_ms(),
        vector_breaker: runtime.vector_breaker,
        sparse_breaker: runtime.sparse_breaker,
        reranker_breaker: runtime.reranker_breaker,
        _in_use_locks: Some(in_use_locks),
    }
}

fn artifact_fetch_policy_label(policy: crate::config::ArtifactFetchPolicy) -> &'static str {
    match policy {
        crate::config::ArtifactFetchPolicy::IfMissing => "if_missing",
        crate::config::ArtifactFetchPolicy::Always => "always",
        crate::config::ArtifactFetchPolicy::Never => "never",
    }
}

fn build_artifact_consumer_status(
    config: &DbtNovaConfig,
    outcome: Option<&crate::manifest::prebuilt_assets_resolver::FileArtifactMaterialization>,
    timing: Option<(u128, Option<u128>)>,
) -> JsonValue {
    let (last_evaluated_at_ms, last_materialized_at_ms) = timing
        .map_or((None, None), |(evaluated, materialized)| {
            (Some(evaluated), materialized)
        });

    serde_json::json!({
        "enabled": config.remote_artifact_mode_enabled(),
        "storage_read_only": config.storage_read_only,
        "consumer_mode_hint": if config.storage_read_only {
            "strict_read_only_reuse"
        } else {
            "writable_hydration"
        },
        "guidance": if config.storage_read_only {
            "Strict read-only mode requires local artifacts to already exist; use DBT_NOVA_ARTIFACT_FETCH_POLICY=never after pre-materialization."
        } else {
            "Writable mode supports first-run bootstrap/artifact hydration with DBT_NOVA_ARTIFACT_FETCH_POLICY=if_missing or always."
        },
        "fetch_policy": artifact_fetch_policy_label(config.artifact_fetch_policy),
        "allow_http": config.artifact_allow_http,
        "timeout_secs": config.artifact_timeout_secs,
        "cache_dir": config
            .artifacts_cache_dir()
            .ok()
            .map(|path| path.to_string_lossy().to_string()),
        "storage_uri": sanitize_uri(&config.storage_artifact_uri),
        "metadata_uri": sanitize_uri(&config.metadata_artifact_uri),
        "models_uri": sanitize_uri(&config.models_artifact_uri),
        "metadata_validated": outcome.is_some(),
        "metadata_contract_version": outcome.map(|value| value.metadata.contract_version.clone()),
        "storage_materialized": outcome.is_some_and(|value| value.storage_materialized),
        "models_materialized": outcome.is_some_and(|value| value.models_materialized),
        "last_evaluated_at_ms": last_evaluated_at_ms,
        "last_materialized_at_ms": last_materialized_at_ms,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::manifest::entity::Entity;
    use crate::manifest::store::EntityStoreBuilder;
    use tempfile::tempdir;

    #[test]
    fn combine_index_build_results_aggregates_failures() {
        let result = combine_index_build_results::<u8, u8, u8>(
            Err(DbtNovaError::ServerError("tantivy panic".to_string())),
            Err(DbtNovaError::ServerError("vector panic".to_string())),
            Ok(crate::manifest::vector_search::SearchComponentBuild::unavailable()),
        );

        match result {
            Ok(_) => panic!("expected aggregated failure"),
            Err(err) => {
                let message = err.to_string();
                assert!(message.contains("tantivy"));
                assert!(message.contains("vector"));
                assert!(!message.contains("sparse"));
            }
        }
    }

    #[test]
    fn combine_index_build_results_returns_values() {
        let result = combine_index_build_results(
            Ok(1u8),
            Ok(crate::manifest::vector_search::SearchComponentBuild::ready(
                2u8,
            )),
            Ok(crate::manifest::vector_search::SearchComponentBuild::<u8>::unavailable()),
        );
        match result {
            Ok((tantivy, vector, sparse)) => {
                let (vector, vector_warning) = vector.into_parts();
                let (sparse, sparse_warning) = sparse.into_parts();
                assert_eq!(tantivy, 1);
                assert_eq!(vector, Some(2));
                assert_eq!(vector_warning, None);
                assert_eq!(sparse, None);
                assert_eq!(sparse_warning, None);
            }
            Err(err) => panic!("unexpected error: {err}"),
        }
    }

    #[test]
    fn combine_index_build_results_preserves_component_warnings() {
        let result = combine_index_build_results(
            Ok(1u8),
            Ok(
                crate::manifest::vector_search::SearchComponentBuild::<u8>::disabled(
                    "vector warning".to_string(),
                ),
            ),
            Ok(
                crate::manifest::vector_search::SearchComponentBuild::<u8>::disabled(
                    "sparse warning".to_string(),
                ),
            ),
        )
        .expect("warnings should not fail aggregate build");
        let (_, vector, sparse) = result;
        let (vector, vector_warning) = vector.into_parts();
        let (sparse, sparse_warning) = sparse.into_parts();
        assert!(vector.is_none());
        assert!(sparse.is_none());
        assert_eq!(vector_warning.as_deref(), Some("vector warning"));
        assert_eq!(sparse_warning.as_deref(), Some("sparse warning"));
    }

    #[test]
    fn artifact_consumer_status_exposes_mode_guidance() {
        let config = DbtNovaConfig {
            storage_read_only: true,
            storage_artifact_uri: "s3://bucket/storage.tar.gz".to_string(),
            metadata_artifact_uri: "s3://bucket/metadata.json".to_string(),
            ..DbtNovaConfig::default()
        };

        let payload = build_artifact_consumer_status(&config, None, None);
        assert_eq!(
            payload["consumer_mode_hint"].as_str(),
            Some("strict_read_only_reuse")
        );
        assert_eq!(payload["storage_read_only"].as_bool(), Some(true));
        assert!(
            payload["guidance"]
                .as_str()
                .unwrap_or_default()
                .contains("DBT_NOVA_ARTIFACT_FETCH_POLICY=never")
        );
    }

    #[test]
    fn malformed_ref_detection_flags_single_brace_calls() {
        assert!(has_apparent_malformed_ref(
            "select * from { ref('orders') }"
        ));
        assert!(has_apparent_malformed_ref(
            "select * from {  ref('orders') }"
        ));
        assert!(!has_apparent_malformed_ref(
            "select * from {{ ref('orders') }}"
        ));
        assert!(!has_apparent_malformed_ref(
            "select * from {{ref('orders')}} join {{ ref('users') }} on 1=1"
        ));
    }

    #[test]
    fn manifest_health_reports_ref_calls_without_dependencies() {
        let temp = tempdir().unwrap_or_else(|err| panic!("tempdir failed: {err}"));
        let mut builder = EntityStoreBuilder::new(temp.path())
            .unwrap_or_else(|err| panic!("builder init failed: {err}"));

        let bad_payload = serde_json::json!({
            "name": "metric__bad",
            "resource_type": "model",
            "raw_code": "select * from { ref('orders') }"
        });
        let bad = Entity::from_json("model.pkg.metric__bad", &bad_payload);
        builder
            .add("model.pkg.metric__bad", &bad)
            .unwrap_or_else(|err| panic!("failed to add bad entity: {err}"));

        let good_payload = serde_json::json!({
            "name": "metric__good",
            "resource_type": "model",
            "raw_code": "select * from {{ ref('orders') }}"
        });
        let good = Entity::from_json("model.pkg.metric__good", &good_payload);
        builder
            .add("model.pkg.metric__good", &good)
            .unwrap_or_else(|err| panic!("failed to add good entity: {err}"));

        let store = builder
            .finish()
            .unwrap_or_else(|err| panic!("failed to finalize store: {err}"));
        let mut parent_map = HashMap::new();
        parent_map.insert(
            "model.pkg.metric__good".to_string(),
            HashSet::from(["model.pkg.orders".to_string()]),
        );
        let resource_type_by_id = HashMap::from([
            ("model.pkg.metric__bad".to_string(), "model".to_string()),
            ("model.pkg.metric__good".to_string(), "model".to_string()),
        ]);

        let health = build_manifest_health(&store, &parent_map, &resource_type_by_id)
            .unwrap_or_else(|err| panic!("manifest health build failed: {err}"));
        assert_eq!(
            health
                .get("models_ref_calls_without_dependencies")
                .and_then(JsonValue::as_u64),
            Some(1)
        );
        assert_eq!(
            health
                .get("malformed_ref_candidate_count")
                .and_then(JsonValue::as_u64),
            Some(1)
        );
        let sample = health
            .get("malformed_ref_candidate_sample")
            .and_then(JsonValue::as_array)
            .cloned()
            .unwrap_or_default();
        assert_eq!(sample.len(), 1);
        assert_eq!(sample[0].as_str(), Some("model.pkg.metric__bad"));
    }

    #[test]
    fn build_column_lineage_aliases_skips_invalid_payload_json() {
        let temp = tempdir().unwrap_or_else(|err| panic!("tempdir failed: {err}"));
        let mut builder = EntityStoreBuilder::new(temp.path())
            .unwrap_or_else(|err| panic!("builder init failed: {err}"));

        let valid_payload = serde_json::json!({
            "name": "metric__valid",
            "resource_type": "model",
            "raw_code": "select revenue as revenue_alias from {{ ref('orders') }}"
        });
        let valid = Entity::from_json("model.pkg.metric__valid", &valid_payload);
        builder
            .add("model.pkg.metric__valid", &valid)
            .unwrap_or_else(|err| panic!("failed to add valid entity: {err}"));

        let mut invalid = Entity::from_json(
            "model.pkg.metric__invalid",
            &serde_json::json!({
                "name": "metric__invalid",
                "resource_type": "model",
                "raw_code": "select 1"
            }),
        );
        invalid.payload_json = "{not-json".to_string();
        builder
            .add("model.pkg.metric__invalid", &invalid)
            .unwrap_or_else(|err| panic!("failed to add invalid entity: {err}"));

        let store = builder
            .finish()
            .unwrap_or_else(|err| panic!("failed to finalize store: {err}"));
        let config = DbtNovaConfig::default();

        let aliases = build_column_lineage_aliases(&store, &config)
            .unwrap_or_else(|err| panic!("failed to build aliases: {err}"));

        assert!(
            aliases.contains_key("model.pkg.metric__valid"),
            "expected valid entity to produce aliases"
        );
        assert!(
            !aliases.contains_key("model.pkg.metric__invalid"),
            "invalid payload_json should be skipped"
        );
    }

    #[test]
    fn manifest_signature_reuse_match_uses_content_hash() {
        let expected = ManifestSignature {
            path: "/tmp/new/path/manifest.json".to_string(),
            len: 12_345,
            modified_ms: 9_999,
            content_hash: "same-hash".to_string(),
            prune_fingerprint: String::new(),
            search_index_fingerprint: String::new(),
            source_uri: "file:///new/path".to_string(),
        };
        let existing = ManifestSignature {
            path: "/tmp/old/path/manifest.json".to_string(),
            len: 1,
            modified_ms: 1,
            content_hash: "same-hash".to_string(),
            prune_fingerprint: String::new(),
            search_index_fingerprint: String::new(),
            source_uri: "file:///old/path".to_string(),
        };
        assert!(
            manifest_signature_matches_for_reuse(&existing, &expected),
            "content hash should be sufficient for reusable index matching"
        );
    }

    #[test]
    fn manifest_signature_reuse_match_rejects_mismatch_or_missing_hash() {
        let expected = ManifestSignature {
            path: "/tmp/new/path/manifest.json".to_string(),
            len: 12_345,
            modified_ms: 9_999,
            content_hash: "expected-hash".to_string(),
            prune_fingerprint: String::new(),
            search_index_fingerprint: String::new(),
            source_uri: "file:///new/path".to_string(),
        };
        let different_hash = ManifestSignature {
            path: "/tmp/old/path/manifest.json".to_string(),
            len: 12_345,
            modified_ms: 1,
            content_hash: "different-hash".to_string(),
            prune_fingerprint: String::new(),
            search_index_fingerprint: String::new(),
            source_uri: "file:///old/path".to_string(),
        };
        let missing_hash = ManifestSignature {
            path: "/tmp/old/path/manifest.json".to_string(),
            len: 12_345,
            modified_ms: 1,
            content_hash: String::new(),
            prune_fingerprint: String::new(),
            search_index_fingerprint: String::new(),
            source_uri: "file:///old/path".to_string(),
        };
        assert!(!manifest_signature_matches_for_reuse(
            &different_hash,
            &expected
        ));
        assert!(!manifest_signature_matches_for_reuse(
            &missing_hash,
            &expected
        ));
    }

    #[test]
    fn manifest_signature_reuse_match_rejects_prune_fingerprint_mismatch() {
        let expected = ManifestSignature {
            path: "/tmp/new/path/manifest.json".to_string(),
            len: 12_345,
            modified_ms: 9_999,
            content_hash: "same-hash".to_string(),
            prune_fingerprint: "fingerprint-a".to_string(),
            search_index_fingerprint: String::new(),
            source_uri: "file:///new/path".to_string(),
        };
        let existing = ManifestSignature {
            path: "/tmp/old/path/manifest.json".to_string(),
            len: 12_345,
            modified_ms: 1,
            content_hash: "same-hash".to_string(),
            prune_fingerprint: "fingerprint-b".to_string(),
            search_index_fingerprint: String::new(),
            source_uri: "file:///old/path".to_string(),
        };
        assert!(!manifest_signature_matches_for_reuse(&existing, &expected));
    }

    #[test]
    fn manifest_signature_reuse_match_rejects_search_index_fingerprint_mismatch() {
        let expected = ManifestSignature {
            content_hash: "same-hash".to_string(),
            search_index_fingerprint: "fingerprint-a".to_string(),
            ..ManifestSignature::default()
        };
        let existing = ManifestSignature {
            content_hash: "same-hash".to_string(),
            search_index_fingerprint: "fingerprint-b".to_string(),
            ..ManifestSignature::default()
        };
        assert!(!manifest_signature_matches_for_reuse(&existing, &expected));
    }

    #[test]
    fn scoped_manifest_hash_includes_scope_fingerprints() {
        let unscoped = ManifestSignature {
            content_hash: "same-hash".to_string(),
            prune_fingerprint: String::new(),
            search_index_fingerprint: String::new(),
            ..ManifestSignature::default()
        };
        let pruned = ManifestSignature {
            content_hash: "same-hash".to_string(),
            prune_fingerprint: "fingerprint-a".to_string(),
            ..ManifestSignature::default()
        };
        let search_scoped = ManifestSignature {
            content_hash: "same-hash".to_string(),
            search_index_fingerprint: "fingerprint-a".to_string(),
            ..ManifestSignature::default()
        };
        assert_eq!(scoped_manifest_hash(&unscoped), "same-hash");
        assert_ne!(scoped_manifest_hash(&pruned), "same-hash");
        assert_eq!(scoped_manifest_hash(&pruned).len(), 64);
        assert_ne!(scoped_manifest_hash(&search_scoped), "same-hash");
        assert_eq!(scoped_manifest_hash(&search_scoped).len(), 64);
        assert_ne!(
            scoped_manifest_hash(&pruned),
            scoped_manifest_hash(&search_scoped)
        );
    }

    #[test]
    fn prepare_storage_validates_artifacts_with_scoped_manifest_hash() {
        let temp = tempdir().unwrap_or_else(|err| panic!("tempdir failed: {err}"));
        let manifest_path = temp.path().join("manifest.json");
        std::fs::write(&manifest_path, br#"{"metadata":{"dbt_version":"1.10.0"}}"#)
            .expect("write manifest");

        let mut config = DbtNovaConfig {
            storage_dir: temp.path().join("storage").to_string_lossy().to_string(),
            storage_instance_id: "analytics-prod".to_string(),
            manifest_prune_allow_ids: vec!["model.pkg.orders".to_string()],
            artifact_fetch_policy: crate::config::ArtifactFetchPolicy::Never,
            ..DbtNovaConfig::default()
        };
        std::fs::create_dir_all(
            temp.path()
                .join("storage")
                .join("instances")
                .join("analytics-prod")
                .join("versions")
                .join("existing"),
        )
        .expect("create existing storage marker");

        let mut signature =
            manifest_signature(&manifest_path, manifest_path.to_string_lossy().as_ref())
                .expect("manifest signature");
        signature.prune_fingerprint = config.manifest_prune_fingerprint();
        let scoped_hash = scoped_manifest_hash(&signature);

        let metadata_path = temp.path().join("nova-build-metadata.json");
        std::fs::write(
            &metadata_path,
            format!(
                r#"{{
  "contract_version":"v1",
  "manifest_hash":"{scoped_hash}",
  "manifest_version":"v12",
  "entity_count":1,
  "storage_instance_id":"analytics-prod",
  "dbt_nova_version":"0.0.4",
  "build_timestamp":"2026-04-25T00:00:00Z",
  "artifact_name_storage":"storage.tar.gz",
  "artifact_name_models":""
}}"#
            ),
        )
        .expect("write metadata");
        config.storage_artifact_uri = temp
            .path()
            .join("unused-storage.tar.gz")
            .to_string_lossy()
            .to_string();
        config.metadata_artifact_uri = metadata_path.to_string_lossy().to_string();

        let resolution = crate::manifest::source::ManifestResolution {
            local_path: manifest_path.clone(),
            source_uri: manifest_path.to_string_lossy().to_string(),
            cached: false,
        };
        prepare_storage(&config, &resolution, JsonValue::Null)
            .expect("scoped metadata hash should validate");
    }

    #[test]
    fn manifest_search_reports_scoped_manifest_hash_when_pruned() {
        let temp = tempdir().unwrap_or_else(|err| panic!("tempdir failed: {err}"));
        let manifest_path = temp.path().join("manifest.json");
        std::fs::copy(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/minimal.json"),
            &manifest_path,
        )
        .expect("copy fixture manifest");

        let config = DbtNovaConfig {
            manifest_path: manifest_path.to_string_lossy().to_string(),
            storage_dir: temp.path().join("storage").to_string_lossy().to_string(),
            storage_instance_id: "scoped-report".to_string(),
            manifest_prune_allow_ids: vec!["model.pkg.model_a".to_string()],
            ..DbtNovaConfig::default()
        };
        let mut signature =
            manifest_signature(&manifest_path, manifest_path.to_string_lossy().as_ref())
                .expect("manifest signature");
        signature.prune_fingerprint = config.manifest_prune_fingerprint();
        let expected_hash = scoped_manifest_hash(&signature);

        assert_ne!(expected_hash, signature.content_hash);

        let loaded = ManifestSearch::new(config).expect("load manifest search");

        assert_eq!(loaded.search.manifest_hash, expected_hash);
    }

    #[test]
    fn manifest_search_updates_current_pointer_after_scoped_fallback_reuse() {
        let temp = tempdir().unwrap_or_else(|err| panic!("tempdir failed: {err}"));
        let manifest_path = temp.path().join("manifest.json");
        std::fs::copy(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/minimal.json"),
            &manifest_path,
        )
        .expect("copy fixture manifest");

        let base = DbtNovaConfig {
            manifest_path: manifest_path.to_string_lossy().to_string(),
            storage_dir: temp.path().join("storage").to_string_lossy().to_string(),
            storage_instance_id: "scoped-current".to_string(),
            ..DbtNovaConfig::default()
        };
        let instance_root = base.storage_instance_root_dir().expect("instance root");
        let allow_config = DbtNovaConfig {
            manifest_prune_allow_ids: vec!["model.pkg.model_a".to_string()],
            ..base.clone()
        };
        let deny_config = DbtNovaConfig {
            manifest_prune_deny_ids: vec!["model.pkg.unused".to_string()],
            ..base
        };

        let allow_first =
            ManifestSearch::new(allow_config.clone()).expect("load allow-scoped manifest first");
        let allow_version = allow_first.search.manifest_version;
        let deny_loaded =
            ManifestSearch::new(deny_config).expect("load alternate prune-scoped manifest");
        let deny_version = deny_loaded.search.manifest_version;

        assert_ne!(allow_version, deny_version);
        assert_eq!(
            read_current_version(&instance_root).expect("read current after alternate"),
            Some(deny_version)
        );

        let allow_second =
            ManifestSearch::new(allow_config).expect("reuse original prune-scoped manifest");

        assert!(allow_second.entity_store_reused);
        assert_eq!(allow_second.search.manifest_version, allow_version);
        assert_eq!(
            read_current_version(&instance_root).expect("read refreshed current"),
            Some(allow_version)
        );
    }

    #[test]
    fn reusable_artifacts_fall_back_to_computed_version_when_current_differs() {
        let temp = tempdir().unwrap_or_else(|err| panic!("tempdir failed: {err}"));
        let instance_root = temp.path().join("instance");
        let versions_root = instance_root.join("versions");
        let current_dir = versions_root.join("current-scope");
        let computed_dir = versions_root.join("computed-scope");
        std::fs::create_dir_all(&current_dir).expect("create current version dir");
        std::fs::create_dir_all(&computed_dir).expect("create computed version dir");

        let expected = ManifestSignature {
            content_hash: "same-hash".to_string(),
            prune_fingerprint: "fingerprint-a".to_string(),
            ..ManifestSignature::default()
        };
        let other_scope = ManifestSignature {
            content_hash: "same-hash".to_string(),
            prune_fingerprint: "fingerprint-b".to_string(),
            ..ManifestSignature::default()
        };
        write_manifest_signature(&current_dir.join(MANIFEST_SIGNATURE_FILENAME), &other_scope)
            .expect("write current signature");
        write_manifest_signature(&computed_dir.join(MANIFEST_SIGNATURE_FILENAME), &expected)
            .expect("write computed signature");
        write_current_version(&instance_root, "current-scope").expect("write current pointer");

        let reused = load_reusable_artifacts(
            &DbtNovaConfig::default(),
            &instance_root,
            &versions_root,
            &expected,
            &computed_dir,
            &computed_dir.join(MANIFEST_SIGNATURE_FILENAME),
        )
        .expect("load reusable artifacts");

        assert_eq!(reused.current_version.as_deref(), Some("current-scope"));
        assert_eq!(reused.storage_dir, computed_dir);
        assert_eq!(
            reused.signature_path,
            reused.storage_dir.join(MANIFEST_SIGNATURE_FILENAME)
        );
    }
}
