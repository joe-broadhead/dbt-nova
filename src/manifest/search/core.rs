use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use moka::sync::Cache as MokaCache;
use serde_json::Value as JsonValue;
use tokio::sync::{Notify, RwLock};

use crate::config::core::LayerRule;
use crate::config::{ArtifactFetchPolicy, DbtNovaConfig};
use crate::error::{DbtNovaError, Result};
use crate::manifest::bootstrap::prepare_runtime_config;
use crate::manifest::entity::{ArchivedEntity, Entity};
use crate::manifest::semantic_cache::{self, SemanticCacheComponent};
use crate::manifest::store::EntityStore;
use crate::manifest::tantivy_search::TantivySearcher;
use crate::manifest::vector_search::{Reranker, SparseSearcher, VectorSearcher};
use crate::utils::CircuitBreaker;
use crate::utils::SearchPersona;

use super::cache::EntityCache;

/// Central in-memory search structure built from a dbt manifest.
pub struct ManifestSearch {
    pub(crate) config: DbtNovaConfig,
    // === Primary Storage - Full entity data ===
    pub(crate) entities: Arc<EntityStore>, // unique_id → full entity data (on disk)

    // === Lineage - Direct from manifest ===
    pub(crate) parent_map: HashMap<String, Vec<String>>, // unique_id → upstream unique_ids
    pub(crate) child_map: HashMap<String, Vec<String>>,  // unique_id → downstream unique_ids

    // === Classification Indexes ===
    pub(crate) by_resource_type: HashMap<String, Vec<String>>, // "model" → [unique_ids]
    pub(crate) by_package: HashMap<String, Vec<String>>,       // package → [unique_ids]
    pub(crate) by_tag: HashMap<String, HashSet<String>>,       // tag → {unique_ids}
    pub(crate) by_database_schema: HashMap<String, Vec<String>>, // "db.schema" → [unique_ids]
    pub(crate) name_to_keys: HashMap<String, Vec<String>>, // name → [unique_ids] (names not unique!)
    pub(crate) resource_type_by_id: HashMap<String, String>, // unique_id → resource_type

    // === Tantivy Full-Text Search Index ===
    pub(crate) tantivy: TantivySearcher,
    pub(crate) vector_search: Option<Arc<VectorSearcher>>,
    pub(crate) sparse_search: Option<Arc<SparseSearcher>>,
    pub(crate) reranker: Option<Arc<Reranker>>,
    pub(crate) compiled_layer_rules: Vec<CompiledLayerRule>,

    // === Test Coverage Index ===
    pub(crate) tests_by_entity: HashMap<String, Vec<String>>, // entity_id → [test_ids]
    pub(crate) tests_by_column: HashMap<String, Vec<String>>, // "entity_id:column_name" → [test_ids]

    // === Path Index for fast glob matching ===
    pub(crate) by_path_prefix: HashMap<String, Vec<String>>, // "models/staging" → [unique_ids]

    // === Column lineage precomputed SQL aliases ===
    pub(crate) column_lineage_aliases: HashMap<String, HashMap<String, String>>,

    // === Metadata ===
    pub(crate) manifest_metadata: JsonValue,
    pub(crate) manifest_health: JsonValue,
    pub(crate) artifact_consumer: JsonValue,
    pub(crate) bootstrap: JsonValue,
    pub(crate) search_init_warnings: HashMap<String, String>,

    // === Stats ===
    pub(crate) entity_counts: HashMap<String, usize>,

    // === Hot Entity Cache ===
    pub(crate) entity_cache: Option<EntityCache>,

    // === Lineage Cache ===
    pub(crate) lineage_cache: Option<MokaCache<String, Arc<JsonValue>>>,

    // === Cache Metrics ===
    pub(crate) entity_cache_hits: AtomicU64,
    pub(crate) entity_cache_misses: AtomicU64,
    pub(crate) lineage_cache_hits: AtomicU64,
    pub(crate) lineage_cache_misses: AtomicU64,

    // === Manifest info ===
    pub(crate) manifest_source_uri: String,
    pub(crate) manifest_hash: String,
    pub(crate) manifest_len: u64,
    pub(crate) manifest_modified_ms: u128,
    pub(crate) manifest_version: String,
    pub(crate) loaded_at_ms: u128,

    // === Circuit breakers for optional search components ===
    pub(crate) vector_breaker: CircuitBreaker,
    pub(crate) sparse_breaker: CircuitBreaker,
    pub(crate) reranker_breaker: CircuitBreaker,

    // Hold shared locks while this instance is alive to prevent cleanup/pruning.
    pub(crate) _in_use_locks: Option<InUseLocks>,
}

#[derive(Debug, Clone)]
pub(crate) struct CompiledLayerRule {
    pub(crate) layer: String,
    pub(crate) path_prefix: Option<String>,
    pub(crate) name_prefix: Option<String>,
    pub(crate) name_regex: Option<regress::Regex>,
    pub(crate) tag: Option<String>,
    pub(crate) resource_type: Option<String>,
}

pub(crate) fn compile_layer_rules(rules: &[LayerRule]) -> Vec<CompiledLayerRule> {
    let mut compiled = Vec::new();
    for rule in rules {
        let name_regex = if let Some(pattern) = rule.name_regex.as_ref() {
            match regress::Regex::new(pattern) {
                Ok(regex) => Some(regex),
                Err(err) => {
                    tracing::warn!(
                        layer = %rule.layer,
                        regex = %pattern,
                        error = %err,
                        "invalid layer rule regex; skipping rule"
                    );
                    continue;
                }
            }
        } else {
            None
        };

        compiled.push(CompiledLayerRule {
            layer: rule.layer.clone(),
            path_prefix: rule.path_prefix.clone(),
            name_prefix: rule.name_prefix.clone(),
            name_regex,
            tag: rule.tag.clone(),
            resource_type: rule.resource_type.clone(),
        });
    }
    compiled
}

pub(crate) struct InUseLocks {
    #[allow(dead_code)]
    pub(crate) instance_root: File,
    #[allow(dead_code)]
    pub(crate) version_dir: File,
}

impl ManifestSearch {
    /// Access the loaded configuration.
    #[must_use]
    pub fn config(&self) -> &DbtNovaConfig {
        &self.config
    }

    pub(crate) fn page_limit(&self, requested: usize) -> usize {
        let default_limit = self.config.search.default_limit.max(1);
        let limit = if requested == 0 {
            default_limit
        } else {
            requested
        };
        let max_page_size = self.config.search.max_page_size;
        if max_page_size == 0 {
            limit
        } else {
            limit.min(max_page_size)
        }
    }

    /// Resolve an ID or name to `unique_id` values.
    #[must_use]
    pub fn resolve_id_or_name(&self, id_or_name: &str, resource_type: Option<&str>) -> Vec<String> {
        // First try as unique_id
        if self.entities.contains(id_or_name) {
            return vec![id_or_name.to_string()];
        }

        // Try as name
        if let Some(keys) = self.name_to_keys.get(id_or_name) {
            if let Some(rt) = resource_type {
                return keys
                    .iter()
                    .filter(|k| self.resource_type_by_id.get(*k).is_some_and(|t| t == rt))
                    .cloned()
                    .collect();
            }
            return keys.clone();
        }

        vec![]
    }

    /// Get entity by `unique_id` (cached, async).
    ///
    /// # Errors
    /// Returns an error if the entity store lookup fails.
    pub async fn get_entity(&self, unique_id: &str) -> Result<Option<Entity>> {
        self.get_entity_arc(unique_id)
            .await
            .map(|entity| entity.map(|value| value.as_ref().clone()))
    }

    /// Get entity by `unique_id` (cached, async) without cloning cache values.
    ///
    /// # Errors
    /// Returns an error if the entity store lookup fails.
    pub async fn get_entity_arc(&self, unique_id: &str) -> Result<Option<Arc<Entity>>> {
        if let Some(cache) = &self.entity_cache {
            if let Some(entity) = cache.get_arc(unique_id) {
                self.entity_cache_hits.fetch_add(1, Ordering::Relaxed);
                return Ok(Some(entity));
            }
            self.entity_cache_misses.fetch_add(1, Ordering::Relaxed);
        }

        let store = self.entities.clone();
        let key = unique_id.to_string();
        let entity = tokio::task::spawn_blocking(move || store.get_blocking(&key))
            .await
            .map_err(|e| DbtNovaError::ServerError(e.to_string()))??;
        let entity = entity.map(Arc::new);
        if let (Some(entity), Some(cache)) = (entity.as_ref(), &self.entity_cache) {
            cache.insert_arc(unique_id.to_string(), Arc::clone(entity));
        }
        Ok(entity)
    }

    /// Get entity by `unique_id` as an archived reference.
    ///
    /// # Errors
    /// Returns an error if the entity store lookup fails.
    pub fn get_entity_archived(&self, unique_id: &str) -> Result<Option<&ArchivedEntity>> {
        self.entities.get_archived(unique_id)
    }

    /// Create summary of entity (`unique_id`, name, `resource_type`).
    ///
    /// # Errors
    /// Returns an error if the entity cannot be loaded.
    /// Build a standard summary payload for an entity.
    pub fn entity_summary(&self, unique_id: &str) -> Result<JsonValue> {
        let entity = self.get_entity_archived(unique_id)?;
        Ok(self.summary_from_archived(unique_id, entity, SearchPersona::Default, None))
    }

    /// Insert `unique_id` into a JSON entity payload if missing.
    pub(crate) fn insert_unique_id(entity: &mut JsonValue, unique_id: &str) {
        if let Some(obj) = entity.as_object_mut() {
            obj.insert(
                "unique_id".to_string(),
                JsonValue::String(unique_id.to_string()),
            );
        }
    }

    pub(crate) fn with_unique_id(mut entity: JsonValue, unique_id: &str) -> JsonValue {
        Self::insert_unique_id(&mut entity, unique_id);
        entity
    }

    /// Resolve a single `unique_id` from a name or id, erroring on ambiguity.
    ///
    /// # Errors
    /// Returns an error when the entity cannot be found or is ambiguous.
    pub(crate) fn resolve_single_id(
        &self,
        id_or_name: &str,
        resource_type: Option<&str>,
    ) -> Result<String> {
        let keys = self.resolve_id_or_name(id_or_name, resource_type);
        if keys.is_empty() {
            return Err(self.entity_not_found(id_or_name, resource_type));
        }
        if keys.len() > 1 {
            return Err(DbtNovaError::AmbiguousName {
                name: id_or_name.to_string(),
                count: keys.len(),
                matches: keys,
            });
        }
        Ok(keys[0].clone())
    }

    /// Resolve an entity by `unique_id` or name, returning the `unique_id` and entity.
    pub(crate) async fn resolve_entity(
        &self,
        id_or_name: &str,
        resource_type: Option<&str>,
    ) -> Result<(String, Entity)> {
        let unique_id = self.resolve_single_id(id_or_name, resource_type)?;
        let entity = self
            .get_entity(&unique_id)
            .await?
            .ok_or_else(|| self.entity_not_found(&unique_id, resource_type))?;
        Ok((unique_id, entity))
    }

    /// Return a standardized "entity not found" error for the given id or name.
    pub(crate) fn entity_not_found(
        &self,
        id_or_name: &str,
        resource_type: Option<&str>,
    ) -> DbtNovaError {
        let mut available: Vec<String> = self.by_resource_type.keys().cloned().collect();
        available.sort();
        DbtNovaError::EntityNotFound {
            query: id_or_name.to_string(),
            resource_type: resource_type.map(ToString::to_string),
            available_resource_types: available,
        }
    }

    /// Normalize and validate a resource type key against loaded manifest types.
    ///
    /// # Errors
    /// Returns `INVALID_PARAMS` when `resource_type` is empty or not present in
    /// the loaded manifest.
    pub(crate) fn normalize_resource_type_key(&self, resource_type: &str) -> Result<String> {
        let requested = resource_type.trim();
        if requested.is_empty() {
            return Err(DbtNovaError::InvalidParams(
                "resource_type cannot be empty".to_string(),
            ));
        }

        if let Some(canonical) = self
            .by_resource_type
            .keys()
            .find(|key| key.eq_ignore_ascii_case(requested))
        {
            return Ok(canonical.clone());
        }

        let mut available: Vec<String> = self.by_resource_type.keys().cloned().collect();
        available.sort();
        Err(DbtNovaError::InvalidParams(format!(
            "resource_type '{requested}' is invalid; allowed values: {}",
            available.join(", ")
        )))
    }

    /// Get entity column names in a deterministic order.
    #[must_use]
    pub fn get_entity_columns(&self, entity: &Entity) -> Vec<String> {
        let mut columns = entity.column_names.clone();
        columns.sort();
        columns
    }

    /// Get candidate entities for a path pattern using the prefix index
    /// Return candidate file paths matching a glob pattern prefix.
    pub fn get_path_candidates(&self, pattern: &str) -> Vec<String> {
        // Extract static prefix before glob characters
        let glob_chars = ['*', '?', '['];
        let prefix_end = pattern
            .find(|c| glob_chars.contains(&c))
            .unwrap_or(pattern.len());

        let static_prefix = &pattern[..prefix_end];

        // Trim trailing slash from prefix
        let static_prefix = static_prefix.trim_end_matches('/');

        if static_prefix.is_empty() {
            // No static prefix - must scan all entities
            return self.entities.ids().cloned().collect();
        }

        // Try to find the longest matching prefix in our index
        let parts: Vec<&str> = static_prefix.split('/').collect();
        let mut best_prefix = String::new();
        let mut current_prefix = String::new();

        for (i, part) in parts.iter().enumerate() {
            if i > 0 {
                current_prefix.push('/');
            }
            current_prefix.push_str(part);

            if self.by_path_prefix.contains_key(&current_prefix) {
                best_prefix.clone_from(&current_prefix);
            }
        }

        if best_prefix.is_empty() {
            // No matching prefix found - scan all entities
            return self.entities.ids().cloned().collect();
        }

        // Return entities under this prefix
        self.by_path_prefix
            .get(&best_prefix)
            .cloned()
            .unwrap_or_default()
    }

    /// Total number of entities loaded in this manifest.
    pub fn entity_count(&self) -> usize {
        self.entities.len()
    }

    /// Returns whether vector search was initialized successfully.
    #[must_use]
    pub fn vector_search_ready(&self) -> bool {
        self.vector_search
            .as_ref()
            .is_some_and(|searcher| searcher.query_ready())
    }

    /// Returns whether sparse search was initialized successfully.
    #[must_use]
    pub fn sparse_search_ready(&self) -> bool {
        self.sparse_search
            .as_ref()
            .is_some_and(|searcher| searcher.query_ready())
    }

    /// Returns whether the reranker was initialized successfully.
    #[must_use]
    pub fn reranker_ready(&self) -> bool {
        self.reranker
            .as_ref()
            .is_some_and(|reranker| reranker.query_ready())
    }

    /// Returns whether all enabled semantic capabilities are ready to serve queries.
    #[must_use]
    pub fn ready_for_traffic(&self) -> bool {
        (!self.config.search.enable_vector_search || self.vector_search_ready())
            && (!self.config.search.enable_sparse_search || self.sparse_search_ready())
            && (!self.config.search.enable_reranker || self.reranker_ready())
    }

    /// Returns the top-level health status label for a loaded manifest.
    #[must_use]
    pub fn health_status_label(&self) -> &'static str {
        if self.ready_for_traffic() {
            "ready"
        } else {
            "degraded"
        }
    }

    /// Build a health snapshot payload for diagnostics.
    #[allow(clippy::too_many_lines)]
    pub async fn health_snapshot(&self) -> JsonValue {
        fn state_label(state: crate::utils::CircuitBreakerState) -> &'static str {
            match state {
                crate::utils::CircuitBreakerState::Closed => "closed",
                crate::utils::CircuitBreakerState::Open => "open",
                crate::utils::CircuitBreakerState::HalfOpen => "half_open",
            }
        }

        let (vector_state, vector_failures, vector_elapsed) = self.vector_breaker.state().await;
        let (sparse_state, sparse_failures, sparse_elapsed) = self.sparse_breaker.state().await;
        let (rerank_state, rerank_failures, rerank_elapsed) = self.reranker_breaker.state().await;

        let cache_json = if let Some(cache) = &self.entity_cache {
            serde_json::json!({
                "enabled": true,
                "backend": EntityCache::BACKEND_LABEL,
                "len": cache.len(),
                "hits": self.entity_cache_hits.load(Ordering::Relaxed),
                "misses": self.entity_cache_misses.load(Ordering::Relaxed),
            })
        } else {
            serde_json::json!({
                "enabled": false,
            })
        };
        let now_ms = now_ms();
        let manifest_age_ms = now_ms.saturating_sub(self.manifest_modified_ms);
        let loaded_age_ms = now_ms.saturating_sub(self.loaded_at_ms);

        let (manifest_cache_hits, manifest_cache_misses) =
            crate::manifest::source::manifest_cache_stats();
        let vector_model_name = if self.config.search.embedding_model.trim().is_empty() {
            crate::config::SearchConfig::default().embedding_model
        } else {
            self.config.search.embedding_model.trim().to_string()
        };
        let vector_cache = semantic_cache::cache_paths(
            &self.config.search,
            SemanticCacheComponent::Dense,
            &vector_model_name,
            &self.manifest_hash,
        );
        let sparse_cache = semantic_cache::cache_paths(
            &self.config.search,
            SemanticCacheComponent::Sparse,
            semantic_cache::default_sparse_model_name(),
            &self.manifest_hash,
        );

        let lineage_cache_json = if let Some(cache) = &self.lineage_cache {
            serde_json::json!({
                "enabled": true,
                "len": cache.entry_count(),
                "hits": self.lineage_cache_hits.load(Ordering::Relaxed),
                "misses": self.lineage_cache_misses.load(Ordering::Relaxed),
            })
        } else {
            serde_json::json!({
                "enabled": false,
            })
        };

        serde_json::json!({
            "ready_for_traffic": self.ready_for_traffic(),
            "manifest": {
                "source_uri": self.manifest_source_uri,
                "hash": self.manifest_hash,
                "len": self.manifest_len,
                "modified_ms": self.manifest_modified_ms,
                "version": self.manifest_version,
                "age_ms": manifest_age_ms,
                "loaded_at_ms": self.loaded_at_ms,
                "loaded_age_ms": loaded_age_ms,
            },
            "manifest_health": self.manifest_health,
            "artifact_consumer": self.artifact_consumer,
            "bootstrap": self.bootstrap,
            "manifest_cache": {
                "hits": manifest_cache_hits,
                "misses": manifest_cache_misses,
            },
            "search": {
                "vector": {
                    "enabled": self.config.search.enable_vector_search,
                    "ready": self.vector_search_ready(),
                    "warning": self.search_init_warnings.get("vector"),
                    "query_model_files_present": self
                        .vector_search
                        .as_ref()
                        .is_some_and(|searcher| searcher.query_model_files_present()),
                    "query_model_initialized": self
                        .vector_search
                        .as_ref()
                        .is_some_and(|searcher| searcher.query_model_initialized()),
                    "cache": {
                        "expected_path": vector_cache.compressed_path,
                        "manifest_hash": vector_cache.manifest_hash,
                        "model_slug": vector_cache.model_slug,
                        "present": vector_cache.present(),
                    },
                },
                "sparse": {
                    "enabled": self.config.search.enable_sparse_search,
                    "ready": self.sparse_search_ready(),
                    "warning": self.search_init_warnings.get("sparse"),
                    "query_model_files_present": self
                        .sparse_search
                        .as_ref()
                        .is_some_and(|searcher| searcher.query_model_files_present()),
                    "query_model_initialized": self
                        .sparse_search
                        .as_ref()
                        .is_some_and(|searcher| searcher.query_model_initialized()),
                    "cache": {
                        "expected_path": sparse_cache.compressed_path,
                        "manifest_hash": sparse_cache.manifest_hash,
                        "model_slug": sparse_cache.model_slug,
                        "present": sparse_cache.present(),
                    },
                },
                "reranker": {
                    "enabled": self.config.search.enable_reranker,
                    "ready": self.reranker_ready(),
                    "warning": self.search_init_warnings.get("reranker"),
                    "query_model_files_present": self
                        .reranker
                        .as_ref()
                        .is_some_and(|reranker| reranker.query_model_files_present()),
                    "query_model_initialized": self
                        .reranker
                        .as_ref()
                        .is_some_and(|reranker| reranker.initialized()),
                },
            },
            "circuit_breakers": {
                "vector": {
                    "state": state_label(vector_state),
                    "failures": vector_failures,
                    "open_elapsed_ms": vector_elapsed.map(|d| d.as_millis()),
                },
                "sparse": {
                    "state": state_label(sparse_state),
                    "failures": sparse_failures,
                    "open_elapsed_ms": sparse_elapsed.map(|d| d.as_millis()),
                },
                "reranker": {
                    "state": state_label(rerank_state),
                    "failures": rerank_failures,
                    "open_elapsed_ms": rerank_elapsed.map(|d| d.as_millis()),
                },
            },
            "entity_cache": cache_json,
            "lineage_cache": lineage_cache_json,
        })
    }

    /// Fast check for whether a `unique_id` exists in the store.
    pub fn has_entity(&self, unique_id: &str) -> bool {
        self.entities.contains(unique_id)
    }

    pub(crate) fn lineage_cache_get(&self, key: &str) -> Option<JsonValue> {
        let cache = self.lineage_cache.as_ref()?;
        if let Some(value) = cache.get(key) {
            self.lineage_cache_hits.fetch_add(1, Ordering::Relaxed);
            return Some(value.as_ref().clone());
        }
        self.lineage_cache_misses.fetch_add(1, Ordering::Relaxed);
        None
    }

    pub(crate) fn lineage_cache_insert(&self, key: String, value: JsonValue) {
        if let Some(cache) = &self.lineage_cache {
            cache.insert(key, Arc::new(value));
        }
    }

    pub(crate) fn sql_aliases_for(&self, unique_id: &str) -> Option<&HashMap<String, String>> {
        self.column_lineage_aliases.get(unique_id)
    }
}

/// Thread-safe handle to the currently loaded manifest search instance.
#[derive(Clone)]
pub struct ManifestSearchHandle {
    state: Arc<RwLock<ManifestSearchState>>,
    notify: Arc<Notify>,
    refresh_stats: Arc<RwLock<RefreshStats>>,
    config: Arc<RwLock<DbtNovaConfig>>,
}

enum ManifestSearchState {
    Loading {
        started_at: Instant,
    },
    Ready(Arc<ManifestSearch>),
    Refreshing {
        started_at: Instant,
        active: Arc<ManifestSearch>,
    },
    Failed(String),
}

#[derive(Default, Clone)]
struct RefreshStats {
    attempts: u64,
    successes: u64,
    failures: u64,
    last_attempt_ms: Option<u128>,
    last_success_ms: Option<u128>,
    last_failure_ms: Option<u128>,
    last_error: Option<String>,
}

/// High-level status for manifest loading/refresh.
pub enum ManifestStatus {
    Loading {
        elapsed_ms: u128,
    },
    Ready {
        entity_count: usize,
    },
    Refreshing {
        elapsed_ms: u128,
        entity_count: usize,
    },
    Failed {
        error: String,
    },
}

impl ManifestSearchHandle {
    /// Spawn manifest loading + index build in the background.
    /// Returns immediately; tool calls return `INDEX_BUILDING` until ready.
    #[must_use]
    pub fn spawn(config: DbtNovaConfig) -> Self {
        let state = Arc::new(RwLock::new(ManifestSearchState::Loading {
            started_at: Instant::now(),
        }));
        let notify = Arc::new(Notify::new());
        let refresh_stats = Arc::new(RwLock::new(RefreshStats::default()));
        let config_arc = Arc::new(RwLock::new(config.clone()));
        let should_refresh = config.manifest_refresh_secs > 0 && !config.storage_read_only;

        let state_clone = state.clone();
        let notify_clone = notify.clone();
        tokio::spawn(async move {
            let result = tokio::task::spawn_blocking(move || ManifestSearch::new(config)).await;
            let mut guard = state_clone.write().await;
            match result {
                Ok(Ok(loaded)) => {
                    *guard = ManifestSearchState::Ready(Arc::new(loaded.search));
                }
                Ok(Err(err)) => {
                    *guard = ManifestSearchState::Failed(err.to_string());
                }
                Err(err) => {
                    *guard = ManifestSearchState::Failed(err.to_string());
                }
            }
            notify_clone.notify_waiters();
        });

        if should_refresh {
            let refresh_state_handle = state.clone();
            let refresh_notify = notify.clone();
            let refresh_stats_shared = refresh_stats.clone();
            let refresh_config = config_arc.clone();
            tokio::spawn(async move {
                refresh_loop(
                    refresh_state_handle,
                    refresh_notify,
                    refresh_stats_shared,
                    refresh_config,
                )
                .await;
            });
        }

        Self {
            state,
            notify,
            refresh_stats,
            config: config_arc,
        }
    }

    /// Get the active manifest instance or return an error when unavailable.
    ///
    /// # Errors
    /// Returns `INDEX_BUILDING` while loading or a server error on initialization failure.
    pub async fn get(&self) -> Result<Arc<ManifestSearch>> {
        let guard = self.state.read().await;
        match &*guard {
            ManifestSearchState::Ready(searcher) => Ok(searcher.clone()),
            ManifestSearchState::Refreshing { active, .. } => Ok(active.clone()),
            ManifestSearchState::Loading { started_at } => {
                Err(DbtNovaError::IndexBuildInProgress {
                    elapsed_ms: started_at.elapsed().as_millis(),
                })
            }
            ManifestSearchState::Failed(err) => Err(DbtNovaError::ServerError(format!(
                "Manifest initialization failed: {err}"
            ))),
        }
    }

    /// Wait until a manifest is ready (or return a failure).
    ///
    /// # Errors
    /// Returns the underlying initialization error if the manifest fails to load.
    pub async fn wait_ready(&self) -> Result<Arc<ManifestSearch>> {
        loop {
            match self.get().await {
                Ok(searcher) => return Ok(searcher),
                Err(DbtNovaError::IndexBuildInProgress { .. }) => {
                    self.notify.notified().await;
                }
                Err(err) => return Err(err),
            }
        }
    }

    /// Get high-level status for loading or refresh state.
    pub async fn status(&self) -> ManifestStatus {
        let guard = self.state.read().await;
        match &*guard {
            ManifestSearchState::Loading { started_at } => ManifestStatus::Loading {
                elapsed_ms: started_at.elapsed().as_millis(),
            },
            ManifestSearchState::Ready(searcher) => ManifestStatus::Ready {
                entity_count: searcher.entity_count(),
            },
            ManifestSearchState::Refreshing { started_at, active } => ManifestStatus::Refreshing {
                elapsed_ms: started_at.elapsed().as_millis(),
                entity_count: active.entity_count(),
            },
            ManifestSearchState::Failed(err) => ManifestStatus::Failed { error: err.clone() },
        }
    }

    /// Return refresh stats for diagnostics.
    pub async fn refresh_stats_snapshot(&self) -> JsonValue {
        let stats = self.refresh_stats.read().await;
        serde_json::json!({
            "refresh": {
                "attempts": stats.attempts,
                "successes": stats.successes,
                "failures": stats.failures,
                "last_attempt_ms": stats.last_attempt_ms,
                "last_success_ms": stats.last_success_ms,
                "last_failure_ms": stats.last_failure_ms,
                "last_error": stats.last_error,
            }
        })
    }

    /// Trigger a manifest reload with new URI/refresh settings.
    ///
    /// # Errors
    /// Returns an error if no manifest source is provided or the reload fails.
    #[allow(clippy::too_many_lines)]
    pub async fn reload(&self, params: &crate::params::ReloadManifestParams) -> Result<JsonValue> {
        let previous = { self.config.read().await.clone() };
        let previous_auto = auto_instance_id(&previous);
        let mut next = previous.clone();
        let mut changed_source = false;
        let mut explicit_manifest_source = false;
        let mut explicit_storage_instance_id = false;

        if let Some(uri) = params.manifest_uri.as_ref() {
            let trimmed = uri.trim();
            if !trimmed.is_empty() {
                next.manifest_uri = trimmed.to_string();
                changed_source = true;
                explicit_manifest_source = true;
            }
        }
        if let Some(path) = params.manifest_path.as_ref() {
            let trimmed = path.trim();
            if !trimmed.is_empty() {
                next.manifest_path = trimmed.to_string();
                next.manifest_path_explicit = true;
                next.manifest_uri = String::new();
                changed_source = true;
                explicit_manifest_source = true;
            }
        }
        if let Some(refresh_secs) = params.refresh_secs {
            next.manifest_refresh_secs = refresh_secs;
        }
        if let Some(instance_id) = params.storage_instance_id.as_ref() {
            let trimmed = instance_id.trim();
            if !trimmed.is_empty() {
                next.storage_instance_id = trimmed.to_string();
                explicit_storage_instance_id = true;
            }
        } else if changed_source {
            let previous_id = previous.storage_instance_id.trim().to_string();
            if previous_id.is_empty() || previous_id == previous_auto {
                next.storage_instance_id = String::new();
            }
        }

        reset_bootstrap_applied_fields_for_reload(
            &mut next,
            previous.bootstrap_status.as_ref(),
            explicit_manifest_source,
            explicit_storage_instance_id,
        );

        let bootstrap_resolution = prepare_runtime_config_for_reload(&mut next)?;

        {
            let mut guard = self.config.write().await;
            *guard = next.clone();
        }

        let active = {
            let mut guard = self.state.write().await;
            match &*guard {
                ManifestSearchState::Refreshing { started_at, .. }
                | ManifestSearchState::Loading { started_at } => {
                    return Err(DbtNovaError::IndexBuildInProgress {
                        elapsed_ms: started_at.elapsed().as_millis(),
                    });
                }
                ManifestSearchState::Ready(current) => {
                    let active = current.clone();
                    *guard = ManifestSearchState::Refreshing {
                        started_at: Instant::now(),
                        active: active.clone(),
                    };
                    Some(active)
                }
                ManifestSearchState::Failed(_) => {
                    *guard = ManifestSearchState::Loading {
                        started_at: Instant::now(),
                    };
                    None
                }
            }
        };

        let response = serde_json::json!({
            "status": "refreshing",
            "manifest_path": next.manifest_path,
            "manifest_uri": next.manifest_uri,
            "manifest_refresh_secs": next.manifest_refresh_secs,
            "storage_instance_id": next.storage_instance_id,
            "bootstrap": bootstrap_resolution.status,
        });
        let build_config = next.clone();
        let state_clone = self.state.clone();
        let notify_clone = self.notify.clone();
        let refresh_stats_clone = self.refresh_stats.clone();
        tokio::spawn(async move {
            {
                let mut refresh_stats_guard = refresh_stats_clone.write().await;
                refresh_stats_guard.attempts += 1;
                refresh_stats_guard.last_attempt_ms = Some(now_ms());
            }
            let attempt = refresh_stats_clone.read().await.attempts;
            tracing::info!(attempts = attempt, "manifest reload attempt started");

            let result = tokio::task::spawn_blocking(move || ManifestSearch::new(build_config))
                .await
                .map_err(|e| DbtNovaError::ServerError(e.to_string()))
                .and_then(|r| r);

            let mut guard = state_clone.write().await;
            match result {
                Ok(loaded) => {
                    *guard = ManifestSearchState::Ready(Arc::new(loaded.search));
                    let mut refresh_stats_guard = refresh_stats_clone.write().await;
                    refresh_stats_guard.successes += 1;
                    refresh_stats_guard.last_success_ms = Some(now_ms());
                    refresh_stats_guard.last_error = None;
                    tracing::info!(
                        attempts = refresh_stats_guard.attempts,
                        successes = refresh_stats_guard.successes,
                        failures = refresh_stats_guard.failures,
                        "manifest reload succeeded"
                    );
                }
                Err(err) => {
                    if let Some(active) = active {
                        *guard = ManifestSearchState::Ready(active);
                    } else {
                        *guard = ManifestSearchState::Failed(err.to_string());
                    }
                    let mut refresh_stats_guard = refresh_stats_clone.write().await;
                    refresh_stats_guard.failures += 1;
                    refresh_stats_guard.last_failure_ms = Some(now_ms());
                    refresh_stats_guard.last_error = Some(err.to_string());
                    tracing::warn!(
                        error = %err,
                        attempts = refresh_stats_guard.attempts,
                        successes = refresh_stats_guard.successes,
                        failures = refresh_stats_guard.failures,
                        "manifest reload failed"
                    );
                }
            }
            notify_clone.notify_waiters();
        });

        Ok(response)
    }
}

#[allow(clippy::too_many_lines)]
async fn refresh_loop(
    state: Arc<RwLock<ManifestSearchState>>,
    notify: Arc<Notify>,
    refresh_stats: Arc<RwLock<RefreshStats>>,
    config: Arc<RwLock<DbtNovaConfig>>,
) {
    loop {
        let (refresh_secs, read_only) = {
            let guard = config.read().await;
            (guard.manifest_refresh_secs, guard.storage_read_only)
        };
        if refresh_secs == 0 || read_only {
            tokio::time::sleep(Duration::from_secs(1)).await;
            continue;
        }
        tokio::time::sleep(Duration::from_secs(refresh_secs)).await;
        let config_snapshot = { config.read().await.clone() };
        let active = {
            let guard = state.read().await;
            match &*guard {
                ManifestSearchState::Ready(searcher) => Some(searcher.clone()),
                ManifestSearchState::Refreshing { active, .. } => Some(active.clone()),
                ManifestSearchState::Failed(_) | ManifestSearchState::Loading { .. } => None,
            }
        };
        let active_hash = active
            .as_ref()
            .map_or("", |searcher| searcher.manifest_hash.as_str());

        let resolution = match tokio::task::spawn_blocking({
            let config_snapshot = config_snapshot.clone();
            move || crate::manifest::source::resolve_manifest(&config_snapshot)
        })
        .await
        {
            Ok(Ok(res)) => res,
            Ok(Err(err)) => {
                let mut refresh_stats_guard = refresh_stats.write().await;
                refresh_stats_guard.failures += 1;
                refresh_stats_guard.last_failure_ms = Some(now_ms());
                refresh_stats_guard.last_error = Some(err.to_string());
                tracing::warn!(error = %err, "manifest refresh failed to resolve source");
                continue;
            }
            Err(err) => {
                let mut refresh_stats_guard = refresh_stats.write().await;
                refresh_stats_guard.failures += 1;
                refresh_stats_guard.last_failure_ms = Some(now_ms());
                refresh_stats_guard.last_error = Some(err.to_string());
                tracing::warn!(error = %err, "manifest refresh failed to resolve source");
                continue;
            }
        };
        let signature = match tokio::task::spawn_blocking({
            let local_path = resolution.local_path.clone();
            let source_uri = resolution.source_uri.clone();
            move || crate::manifest::source::manifest_signature(&local_path, &source_uri)
        })
        .await
        {
            Ok(Ok(sig)) => sig,
            Ok(Err(err)) => {
                let mut refresh_stats_guard = refresh_stats.write().await;
                refresh_stats_guard.failures += 1;
                refresh_stats_guard.last_failure_ms = Some(now_ms());
                refresh_stats_guard.last_error = Some(err.to_string());
                tracing::warn!(error = %err, "manifest refresh failed to read signature");
                continue;
            }
            Err(err) => {
                let mut refresh_stats_guard = refresh_stats.write().await;
                refresh_stats_guard.failures += 1;
                refresh_stats_guard.last_failure_ms = Some(now_ms());
                refresh_stats_guard.last_error = Some(err.to_string());
                tracing::warn!(error = %err, "manifest refresh failed to read signature");
                continue;
            }
        };

        if signature.content_hash == active_hash {
            continue;
        }

        {
            let mut guard = state.write().await;
            if matches!(&*guard, ManifestSearchState::Refreshing { .. }) {
                continue;
            }
            match &*guard {
                ManifestSearchState::Ready(current) => {
                    *guard = ManifestSearchState::Refreshing {
                        started_at: Instant::now(),
                        active: current.clone(),
                    };
                }
                ManifestSearchState::Failed(_) => {
                    *guard = ManifestSearchState::Loading {
                        started_at: Instant::now(),
                    };
                }
                _ => {
                    continue;
                }
            }
        }

        let state_clone = state.clone();
        let notify_clone = notify.clone();
        let config_clone = config_snapshot.clone();
        let refresh_stats_clone = refresh_stats.clone();
        tokio::spawn(async move {
            {
                let mut refresh_stats_guard = refresh_stats_clone.write().await;
                refresh_stats_guard.attempts += 1;
                refresh_stats_guard.last_attempt_ms = Some(now_ms());
            }
            let current_attempts = refresh_stats_clone.read().await.attempts;
            tracing::info!(
                attempts = current_attempts,
                "manifest refresh attempt started"
            );
            let result = tokio::task::spawn_blocking(move || ManifestSearch::new(config_clone))
                .await
                .map_err(|e| DbtNovaError::ServerError(e.to_string()))
                .and_then(|r| r);

            let mut guard = state_clone.write().await;
            match result {
                Ok(loaded) => {
                    *guard = ManifestSearchState::Ready(Arc::new(loaded.search));
                    let mut refresh_stats_guard = refresh_stats_clone.write().await;
                    refresh_stats_guard.successes += 1;
                    refresh_stats_guard.last_success_ms = Some(now_ms());
                    refresh_stats_guard.last_error = None;
                    tracing::info!(
                        attempts = refresh_stats_guard.attempts,
                        successes = refresh_stats_guard.successes,
                        failures = refresh_stats_guard.failures,
                        "manifest refresh succeeded"
                    );
                }
                Err(err) => {
                    if let ManifestSearchState::Refreshing { active, .. } = &*guard {
                        tracing::warn!(error = %err, "manifest refresh failed; keeping active index");
                        *guard = ManifestSearchState::Ready(active.clone());
                        let mut refresh_stats_guard = refresh_stats_clone.write().await;
                        refresh_stats_guard.failures += 1;
                        refresh_stats_guard.last_failure_ms = Some(now_ms());
                        refresh_stats_guard.last_error = Some(err.to_string());
                        tracing::warn!(
                            error = %err,
                            attempts = refresh_stats_guard.attempts,
                            successes = refresh_stats_guard.successes,
                            failures = refresh_stats_guard.failures,
                            "manifest refresh failed"
                        );
                    } else if let ManifestSearchState::Loading { .. } = &*guard {
                        *guard = ManifestSearchState::Failed(err.to_string());
                        let mut refresh_stats_guard = refresh_stats_clone.write().await;
                        refresh_stats_guard.failures += 1;
                        refresh_stats_guard.last_failure_ms = Some(now_ms());
                        refresh_stats_guard.last_error = Some(err.to_string());
                        tracing::warn!(
                            error = %err,
                            attempts = refresh_stats_guard.attempts,
                            successes = refresh_stats_guard.successes,
                            failures = refresh_stats_guard.failures,
                            "manifest refresh failed"
                        );
                    }
                }
            }
            notify_clone.notify_waiters();
        });
    }
}

pub(crate) fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis())
}

fn auto_instance_id(config: &DbtNovaConfig) -> String {
    let mut temp = config.clone();
    temp.storage_instance_id = String::new();
    temp.ensure_storage_instance_id();
    temp.storage_instance_id
}

fn bootstrap_reload_fetch_policy(config: &DbtNovaConfig) -> ArtifactFetchPolicy {
    if config.bootstrap_uri.trim().is_empty() {
        config.artifact_fetch_policy
    } else {
        ArtifactFetchPolicy::Always
    }
}

fn prepare_runtime_config_for_reload(
    config: &mut DbtNovaConfig,
) -> Result<crate::manifest::bootstrap::BootstrapResolution> {
    // Force bootstrap re-evaluation on reload so updates to the remote
    // bootstrap contract are visible without restarting the process.
    config.bootstrap_status = None;
    let original_fetch_policy = config.artifact_fetch_policy;
    config.artifact_fetch_policy = bootstrap_reload_fetch_policy(config);
    let runtime_config_result = prepare_runtime_config(config);
    config.artifact_fetch_policy = original_fetch_policy;
    runtime_config_result
}

fn reset_bootstrap_applied_fields_for_reload(
    next: &mut DbtNovaConfig,
    bootstrap_status: Option<&JsonValue>,
    explicit_manifest_source: bool,
    explicit_storage_instance_id: bool,
) {
    let Some(status) = bootstrap_status else {
        return;
    };
    let Some(applied_fields) = status.get("applied_fields").and_then(JsonValue::as_array) else {
        return;
    };

    for field in applied_fields.iter().filter_map(JsonValue::as_str) {
        match field {
            "manifest_uri" if !explicit_manifest_source => next.manifest_uri.clear(),
            "storage_instance_id" if !explicit_storage_instance_id => {
                next.storage_instance_id.clear();
            }
            "storage_artifact_uri" => {
                next.storage_artifact_uri.clear();
            }
            "metadata_artifact_uri" => {
                next.metadata_artifact_uri.clear();
            }
            "models_artifact_uri" => {
                next.models_artifact_uri.clear();
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{bootstrap_reload_fetch_policy, reset_bootstrap_applied_fields_for_reload};
    use crate::config::{ArtifactFetchPolicy, DbtNovaConfig};

    #[test]
    fn reset_bootstrap_fields_clears_previously_applied_values() {
        let mut config = DbtNovaConfig {
            manifest_uri: "dbfs:/old/manifest.json".to_string(),
            storage_instance_id: "old-instance".to_string(),
            storage_artifact_uri: "dbfs:/old/storage.tar.gz".to_string(),
            metadata_artifact_uri: "dbfs:/old/metadata.json".to_string(),
            models_artifact_uri: "dbfs:/old/models.tar.gz".to_string(),
            ..DbtNovaConfig::default()
        };
        let status = json!({
            "applied_fields": [
                "manifest_uri",
                "storage_instance_id",
                "storage_artifact_uri",
                "metadata_artifact_uri",
                "models_artifact_uri"
            ]
        });

        reset_bootstrap_applied_fields_for_reload(&mut config, Some(&status), false, false);

        assert!(config.manifest_uri.is_empty());
        assert!(config.storage_instance_id.is_empty());
        assert!(config.storage_artifact_uri.is_empty());
        assert!(config.metadata_artifact_uri.is_empty());
        assert!(config.models_artifact_uri.is_empty());
    }

    #[test]
    fn reset_bootstrap_fields_preserves_explicit_reload_overrides() {
        let mut config = DbtNovaConfig {
            manifest_uri: "dbfs:/manual/manifest.json".to_string(),
            storage_instance_id: "manual-instance".to_string(),
            storage_artifact_uri: "dbfs:/old/storage.tar.gz".to_string(),
            metadata_artifact_uri: "dbfs:/old/metadata.json".to_string(),
            models_artifact_uri: "dbfs:/old/models.tar.gz".to_string(),
            ..DbtNovaConfig::default()
        };
        let status = json!({
            "applied_fields": [
                "manifest_uri",
                "storage_instance_id",
                "storage_artifact_uri",
                "metadata_artifact_uri",
                "models_artifact_uri"
            ]
        });

        reset_bootstrap_applied_fields_for_reload(&mut config, Some(&status), true, true);

        assert_eq!(config.manifest_uri, "dbfs:/manual/manifest.json");
        assert_eq!(config.storage_instance_id, "manual-instance");
        assert!(config.storage_artifact_uri.is_empty());
        assert!(config.metadata_artifact_uri.is_empty());
        assert!(config.models_artifact_uri.is_empty());
    }

    #[test]
    fn bootstrap_reload_fetch_policy_uses_always_when_bootstrap_uri_is_set() {
        let config = DbtNovaConfig {
            bootstrap_uri: "dbfs:/FileStore/nova/prod/nova-bootstrap.json".to_string(),
            artifact_fetch_policy: ArtifactFetchPolicy::IfMissing,
            ..DbtNovaConfig::default()
        };

        assert_eq!(
            bootstrap_reload_fetch_policy(&config),
            ArtifactFetchPolicy::Always
        );
    }

    #[test]
    fn bootstrap_reload_fetch_policy_preserves_runtime_policy_without_bootstrap() {
        let config = DbtNovaConfig {
            bootstrap_uri: String::new(),
            artifact_fetch_policy: ArtifactFetchPolicy::Never,
            ..DbtNovaConfig::default()
        };

        assert_eq!(
            bootstrap_reload_fetch_policy(&config),
            ArtifactFetchPolicy::Never
        );
    }
}
