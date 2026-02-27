use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs::{self, File};
use std::io::{BufReader, Write};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::{Duration, Instant};

use fs4::FileExt;
use moka::sync::Cache as MokaCache;
use serde::de::{self, DeserializeSeed, Deserializer, MapAccess, Visitor};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::config::DbtNovaConfig;
use crate::error::{DbtNovaError, Result};
use crate::manifest::entity::Entity;
use crate::manifest::lineage_sql::{extract_ref_calls, find_sql_aliases, sql_for_matching};
use crate::manifest::rkyv_indexes;
use crate::manifest::rkyv_types::{PersistedIndexes, RKYV_SCHEMA_VERSION};
use crate::manifest::search::{EntityCache, InUseLocks, ManifestSearch, compile_layer_rules};
use crate::manifest::source::{ManifestSignature, manifest_signature, resolve_manifest};
use crate::manifest::store::{EntityStore, EntityStoreBuilder};
use crate::manifest::tantivy_search::TantivySearcher;
use crate::manifest::vector_search::{Reranker, SparseSearcher, VectorSearcher};
use crate::utils::{CircuitBreaker, IN_USE_LOCK_FILENAME, prune_dirs, unique_suffix};
use tracing::{info, instrument, warn};

struct ManifestAccumulator {
    store_builder: Option<EntityStoreBuilder>,
    seen_unique_ids: HashSet<String>,
    by_resource_type: HashMap<String, Vec<String>>,
    by_package: HashMap<String, Vec<String>>,
    by_tag: HashMap<String, HashSet<String>>,
    by_database_schema: HashMap<String, Vec<String>>,
    name_to_keys: HashMap<String, Vec<String>>,
    entity_counts: HashMap<String, usize>,
    unique_id_to_resource_type: HashMap<String, String>,
    unique_id_to_path: HashMap<String, String>,
    unique_id_to_tag_strings: HashMap<String, Vec<String>>,
    parent_map: HashMap<String, HashSet<String>>,
    child_map: HashMap<String, HashSet<String>>,
    manifest_metadata: JsonValue,
}

fn map_vec_to_set(map: HashMap<String, Vec<String>>) -> HashMap<String, HashSet<String>> {
    map.into_iter()
        .map(|(k, v)| (k, v.into_iter().collect()))
        .collect()
}

fn map_set_to_vec(map: HashMap<String, HashSet<String>>) -> HashMap<String, Vec<String>> {
    map.into_iter()
        .map(|(k, v)| (k, v.into_iter().collect()))
        .collect()
}

impl ManifestAccumulator {
    fn new(storage_dir: &Path, build_store: bool) -> Result<Self> {
        Ok(Self {
            store_builder: if build_store {
                Some(EntityStoreBuilder::new(storage_dir)?)
            } else {
                None
            },
            seen_unique_ids: HashSet::new(),
            by_resource_type: HashMap::new(),
            by_package: HashMap::new(),
            by_tag: HashMap::new(),
            by_database_schema: HashMap::new(),
            name_to_keys: HashMap::new(),
            entity_counts: HashMap::new(),
            unique_id_to_resource_type: HashMap::new(),
            unique_id_to_path: HashMap::new(),
            unique_id_to_tag_strings: HashMap::new(),
            parent_map: HashMap::new(),
            child_map: HashMap::new(),
            manifest_metadata: JsonValue::Null,
        })
    }

    fn add_entity(
        &mut self,
        unique_id: &str,
        entity: &Entity,
        forced_resource_type: Option<&str>,
    ) -> Result<()> {
        if !self.seen_unique_ids.insert(unique_id.to_string()) {
            tracing::warn!(
                unique_id = unique_id,
                "duplicate unique_id encountered; skipping"
            );
            return Ok(());
        }

        if let Some(builder) = self.store_builder.as_mut() {
            builder.add(unique_id, entity)?;
        }

        let resource_type = forced_resource_type
            .or(entity.resource_type.as_deref())
            .unwrap_or("");
        let name = entity.name.as_deref().unwrap_or("");
        let package = entity.package_name.as_deref().unwrap_or("");

        if !resource_type.is_empty() {
            self.unique_id_to_resource_type
                .insert(unique_id.to_string(), resource_type.to_string());
            self.by_resource_type
                .entry(resource_type.to_string())
                .or_default()
                .push(unique_id.to_string());
            *self
                .entity_counts
                .entry(resource_type.to_string())
                .or_default() += 1;
        }

        self.by_package
            .entry(package.to_string())
            .or_default()
            .push(unique_id.to_string());

        self.name_to_keys
            .entry(name.to_string())
            .or_default()
            .push(unique_id.to_string());

        if !entity.tags.is_empty() {
            let mut tag_strings = Vec::new();
            for tag in &entity.tags {
                tag_strings.push(tag.clone());
                self.by_tag
                    .entry(tag.clone())
                    .or_default()
                    .insert(unique_id.to_string());
            }
            self.unique_id_to_tag_strings
                .insert(unique_id.to_string(), tag_strings);
        }

        let db = entity.database.as_deref().unwrap_or("");
        let schema = entity.schema.as_deref().unwrap_or("");
        if !db.is_empty() || !schema.is_empty() {
            let key = format!("{db}.{schema}");
            self.by_database_schema
                .entry(key)
                .or_default()
                .push(unique_id.to_string());
        }

        if let Some(path) = entity.original_file_path.as_deref() {
            self.unique_id_to_path
                .insert(unique_id.to_string(), path.to_string());
        }

        Ok(())
    }

    fn record_dependencies(&mut self, unique_id: &str, payload: &JsonValue) {
        let deps = payload
            .get("depends_on")
            .and_then(|d| d.get("nodes"))
            .and_then(|n| n.as_array());
        let Some(nodes) = deps else { return };

        for node in nodes {
            let Some(dep_id) = node.as_str() else {
                continue;
            };
            let dep_id = dep_id.to_string();

            let parents = self.parent_map.entry(unique_id.to_string()).or_default();
            parents.insert(dep_id.clone());

            let children = self.child_map.entry(dep_id).or_default();
            children.insert(unique_id.to_string());
        }
    }

    fn finish_store(&mut self) -> Result<EntityStore> {
        self.store_builder
            .take()
            .ok_or_else(|| DbtNovaError::ServerError("Entity store already finalized".to_string()))?
            .finish()
    }
}

struct EntitiesSeed<'a> {
    accumulator: &'a mut ManifestAccumulator,
    forced_resource_type: Option<&'static str>,
}

impl<'de> DeserializeSeed<'de> for EntitiesSeed<'_> {
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> std::result::Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(EntitiesVisitor {
            accumulator: self.accumulator,
            forced_resource_type: self.forced_resource_type,
        })
    }
}

struct EntitiesVisitor<'a> {
    accumulator: &'a mut ManifestAccumulator,
    forced_resource_type: Option<&'static str>,
}

impl<'de> Visitor<'de> for EntitiesVisitor<'_> {
    type Value = ();

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a map of entities")
    }

    fn visit_map<M>(self, mut map: M) -> std::result::Result<Self::Value, M::Error>
    where
        M: MapAccess<'de>,
    {
        while let Some((unique_id, payload)) = map.next_entry::<String, JsonValue>()? {
            let entity = Entity::from_json(&unique_id, &payload);
            self.accumulator
                .add_entity(&unique_id, &entity, self.forced_resource_type)
                .map_err(de::Error::custom)?;
            self.accumulator.record_dependencies(&unique_id, &payload);
        }
        Ok(())
    }

    fn visit_unit<E>(self) -> std::result::Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(())
    }

    fn visit_none<E>(self) -> std::result::Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(())
    }
}

struct ManifestSeed<'a> {
    accumulator: &'a mut ManifestAccumulator,
}

const MANIFEST_SIGNATURE_FILENAME: &str = "manifest.signature.json";
const MANIFEST_CURRENT_FILENAME: &str = "manifest.current.json";

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct ManifestCurrent {
    version: String,
    updated_ms: u128,
}

impl<'de> DeserializeSeed<'de> for ManifestSeed<'_> {
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> std::result::Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_map(ManifestVisitor {
            accumulator: self.accumulator,
        })
    }
}

struct ManifestVisitor<'a> {
    accumulator: &'a mut ManifestAccumulator,
}

impl<'de> Visitor<'de> for ManifestVisitor<'_> {
    type Value = ();

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a manifest object")
    }

    fn visit_map<M>(self, mut map: M) -> std::result::Result<Self::Value, M::Error>
    where
        M: MapAccess<'de>,
    {
        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "nodes" => {
                    map.next_value_seed(EntitiesSeed {
                        accumulator: self.accumulator,
                        forced_resource_type: None,
                    })?;
                }
                "sources" => {
                    map.next_value_seed(EntitiesSeed {
                        accumulator: self.accumulator,
                        forced_resource_type: Some("source"),
                    })?;
                }
                "macros" => {
                    map.next_value_seed(EntitiesSeed {
                        accumulator: self.accumulator,
                        forced_resource_type: Some("macro"),
                    })?;
                }
                "docs" => {
                    map.next_value_seed(EntitiesSeed {
                        accumulator: self.accumulator,
                        forced_resource_type: Some("doc"),
                    })?;
                }
                "groups" => {
                    map.next_value_seed(EntitiesSeed {
                        accumulator: self.accumulator,
                        forced_resource_type: Some("group"),
                    })?;
                }
                "exposures" => {
                    map.next_value_seed(EntitiesSeed {
                        accumulator: self.accumulator,
                        forced_resource_type: Some("exposure"),
                    })?;
                }
                "metrics" => {
                    map.next_value_seed(EntitiesSeed {
                        accumulator: self.accumulator,
                        forced_resource_type: Some("metric"),
                    })?;
                }
                "saved_queries" => {
                    map.next_value_seed(EntitiesSeed {
                        accumulator: self.accumulator,
                        forced_resource_type: Some("saved_query"),
                    })?;
                }
                "semantic_models" => {
                    map.next_value_seed(EntitiesSeed {
                        accumulator: self.accumulator,
                        forced_resource_type: Some("semantic_model"),
                    })?;
                }
                "unit_tests" => {
                    map.next_value_seed(EntitiesSeed {
                        accumulator: self.accumulator,
                        forced_resource_type: Some("unit_test"),
                    })?;
                }
                "parent_map" => {
                    let value: Option<HashMap<String, Vec<String>>> = map.next_value()?;
                    if let Some(v) = value
                        && !v.is_empty()
                    {
                        self.accumulator.parent_map = map_vec_to_set(v);
                    }
                }
                "child_map" => {
                    let value: Option<HashMap<String, Vec<String>>> = map.next_value()?;
                    if let Some(v) = value
                        && !v.is_empty()
                    {
                        self.accumulator.child_map = map_vec_to_set(v);
                    }
                }
                "metadata" => {
                    let value: Option<JsonValue> = map.next_value()?;
                    if let Some(v) = value {
                        self.accumulator.manifest_metadata = v;
                    }
                }
                _ => {
                    let _: de::IgnoredAny = map.next_value()?;
                }
            }
        }

        Ok(())
    }
}

impl ManifestSearch {
    /// Build in-memory search indexes from a dbt manifest file.
    #[instrument(level = "info", skip(config), fields(manifest_path = %config.manifest_path))]
    /// Load the manifest and build indexes into a new search instance.
    ///
    /// # Errors
    /// Returns an error if the manifest cannot be loaded, parsed, or indexed.
    #[allow(clippy::too_many_lines)]
    pub fn new(mut config: DbtNovaConfig) -> Result<Self> {
        config.ensure_storage_instance_id();
        config.ensure_embedding_cache_dir();
        let load_start = Instant::now();

        let manifest_resolution = resolve_manifest(&config)?;
        let manifest_path = manifest_resolution.local_path;
        info!(
            source_uri = %manifest_resolution.source_uri,
            cached = manifest_resolution.cached,
            "resolved manifest source"
        );

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

        let signature = manifest_signature(&manifest_path, &manifest_resolution.source_uri)
            .map_err(|err| {
                DbtNovaError::ManifestError(format!(
                    "Failed to read manifest file {}: {err}",
                    manifest_resolution.source_uri
                ))
            })?;
        let mut version_id = signature.content_hash.chars().take(12).collect::<String>();
        if version_id.is_empty() {
            version_id = "unknown".to_string();
        }

        let mut storage_dir = versions_root.join(&version_id);
        let mut signature_path = storage_dir.join(MANIFEST_SIGNATURE_FILENAME);

        let mut build_lock = Some(acquire_build_lock(
            &instance_root,
            config.storage_build_lock_wait_secs,
        )?);

        let mut entities: Option<EntityStore> = None;
        let mut reuse_store = false;
        let mut tantivy_opened: Option<TantivySearcher> = None;
        let current_version = read_current_version(&instance_root)?;
        if let Some(current) = &current_version {
            let current_dir = versions_root.join(current);
            let current_sig_path = current_dir.join(MANIFEST_SIGNATURE_FILENAME);
            if let Some(existing) = read_manifest_signature(&current_sig_path)?
                && existing == signature
            {
                storage_dir = current_dir;
                signature_path = current_sig_path;
                if let Ok(store) = EntityStore::open(&storage_dir) {
                    entities = Some(store);
                    reuse_store = true;
                }
                if let Ok(opened) = TantivySearcher::open(&storage_dir, &config.search) {
                    tantivy_opened = opened;
                }
            }
        } else if let Some(existing) = read_manifest_signature(&signature_path)?
            && existing == signature
        {
            if let Ok(store) = EntityStore::open(&storage_dir) {
                entities = Some(store);
                reuse_store = true;
            }
            if let Ok(opened) = TantivySearcher::open(&storage_dir, &config.search) {
                tantivy_opened = opened;
            }
        }

        let mut exclude_versions = Vec::new();
        if let Some(current) = &current_version {
            exclude_versions.push(current.as_str());
        }
        exclude_versions.push(version_id.as_str());
        let min_versions = if config.storage_max_instances == 0 {
            config.storage_min_versions
        } else {
            config
                .storage_min_versions
                .min(config.storage_max_instances)
        };
        prune_dirs(
            &versions_root,
            config.storage_max_instances,
            min_versions,
            config.storage_max_bytes,
            &exclude_versions,
        )?;

        fs::create_dir_all(&storage_dir)?;
        let in_use_locks = acquire_in_use_locks(&instance_root, &storage_dir)?;

        let needs_build = !reuse_store || tantivy_opened.is_none();
        if reuse_store {
            info!("reusing entity store from existing storage");
        }
        if tantivy_opened.is_some() {
            info!("reusing tantivy index from existing storage");
        }
        if config.storage_read_only && needs_build {
            build_lock.take();
            return Err(DbtNovaError::ServerError(
                "Storage is read-only and no reusable index is available".to_string(),
            ));
        }
        let mut accumulator = ManifestAccumulator::new(&storage_dir, !reuse_store)?;

        let mut by_path_prefix: HashMap<String, Vec<String>> = HashMap::new();
        let mut tests_by_entity: HashMap<String, Vec<String>> = HashMap::new();
        let mut tests_by_column: HashMap<String, Vec<String>> = HashMap::new();
        let mut used_cached_indexes = false;

        if let Some(cached) = reuse_store
            .then(|| rkyv_indexes::try_load_indexes(&storage_dir, &signature.content_hash))
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
            by_path_prefix = cached.by_path_prefix;
            tests_by_entity = cached.tests_by_entity;
            tests_by_column = cached.tests_by_column;
            used_cached_indexes = true;
        }

        let parse_manifest = !reuse_store || !used_cached_indexes;
        if parse_manifest {
            let file = File::open(&manifest_path).map_err(|err| {
                DbtNovaError::ManifestError(format!(
                    "Failed to read manifest file {}: {err}",
                    manifest_resolution.source_uri
                ))
            })?;
            let reader = BufReader::new(file);

            let mut deserializer = serde_json::Deserializer::from_reader(reader);
            ManifestSeed {
                accumulator: &mut accumulator,
            }
            .deserialize(&mut deserializer)
            .map_err(|err| {
                DbtNovaError::ManifestError(format!("Failed to parse manifest JSON: {err}"))
            })?;
            info!(
                elapsed_ms = load_start.elapsed().as_millis(),
                "manifest parsed"
            );
        } else {
            info!("reused entity/index caches; skipped manifest parse");
        }

        let entities = match entities {
            Some(store) => store,
            None => accumulator.finish_store()?,
        };
        let entity_count = entities.len();
        info!(
            elapsed_ms = load_start.elapsed().as_millis(),
            entity_count, "entity store built"
        );

        if !used_cached_indexes {
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

            if let Some(tests) = accumulator.by_resource_type.get("test") {
                for test_id in tests {
                    if let Some(test) = entities.get_blocking(test_id)? {
                        let test_json = test.to_json_value();
                        if let Some(attached) =
                            test_json.get("attached_node").and_then(|a| a.as_str())
                        {
                            tests_by_entity
                                .entry(attached.to_string())
                                .or_default()
                                .push(test_id.clone());

                            if let Some(col) = test_json.get("column_name").and_then(|c| c.as_str())
                            {
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
                                    let entry =
                                        tests_by_entity.entry(dep_id.to_string()).or_default();
                                    if !entry.contains(test_id) {
                                        entry.push(test_id.clone());
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        config.search.manifest_hash = Some(signature.content_hash.clone());

        let (tantivy, vector_search, sparse_search) = std::thread::scope(|scope| -> Result<_> {
            let mut opened = tantivy_opened;
            let tantivy_handle = if opened.is_none() {
                Some(scope.spawn(|| {
                    TantivySearcher::build(
                        &storage_dir,
                        &accumulator.unique_id_to_resource_type,
                        &accumulator.unique_id_to_path,
                        &accumulator.unique_id_to_tag_strings,
                        &config.search,
                    )
                }))
            } else {
                None
            };

            let vector_handle = scope.spawn(|| VectorSearcher::build(&entities, &config.search));
            let sparse_handle = scope.spawn(|| SparseSearcher::build(&entities, &config.search));

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

            let vector_result = match vector_handle.join() {
                Ok(result) => result,
                Err(err) => Err(DbtNovaError::ServerError(format!(
                    "Vector index build thread panicked: {}",
                    panic_message(&err)
                ))),
            };

            let sparse_result = match sparse_handle.join() {
                Ok(result) => result,
                Err(err) => Err(DbtNovaError::ServerError(format!(
                    "Sparse index build thread panicked: {}",
                    panic_message(&err)
                ))),
            };

            let (tantivy, vector_search, sparse_search) =
                combine_index_build_results(tantivy_result, vector_result, sparse_result)?;

            Ok((
                tantivy,
                vector_search.map(Arc::new),
                sparse_search.map(Arc::new),
            ))
        })?;

        info!(
            elapsed_ms = load_start.elapsed().as_millis(),
            "tantivy index built"
        );

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
        let column_lineage_aliases = build_column_lineage_aliases(&entities, &config)?;

        let breaker_threshold = config.search.search_circuit_failure_threshold;
        let breaker_duration = Duration::from_secs(config.search.search_circuit_open_seconds);
        let vector_breaker = CircuitBreaker::new(breaker_threshold, breaker_duration);
        let sparse_breaker = CircuitBreaker::new(breaker_threshold, breaker_duration);
        let reranker_breaker = CircuitBreaker::new(breaker_threshold, breaker_duration);

        if vector_search.is_some() {
            info!(
                elapsed_ms = load_start.elapsed().as_millis(),
                "vector index built"
            );
        }

        if sparse_search.is_some() {
            info!(
                elapsed_ms = load_start.elapsed().as_millis(),
                "sparse index built"
            );
        }

        let reranker = Reranker::build(&config.search)?.map(Arc::new);
        let compiled_layer_rules = compile_layer_rules(&config.layer_rules);

        let manifest_health = build_manifest_health(
            &entities,
            &accumulator.parent_map,
            &accumulator.unique_id_to_resource_type,
        )?;
        let parent_map = map_set_to_vec(accumulator.parent_map);
        let child_map = map_set_to_vec(accumulator.child_map);
        let resource_type_by_id = accumulator.unique_id_to_resource_type.clone();

        let persist_indexes = !used_cached_indexes && !config.storage_read_only;

        if needs_build {
            write_manifest_signature(&signature_path, &signature)?;
            write_current_version(&instance_root, &version_id)?;
        }
        if persist_indexes {
            let manifest_metadata_json = serde_json::to_string(&accumulator.manifest_metadata)
                .unwrap_or_else(|err| {
                    tracing::warn!(
                        error = %err,
                        "failed to encode manifest metadata for cache; using null"
                    );
                    "null".to_string()
                });
            let indexes = PersistedIndexes {
                schema_version: RKYV_SCHEMA_VERSION,
                manifest_hash: signature.content_hash.clone(),
                parent_map: parent_map.clone(),
                child_map: child_map.clone(),
                by_resource_type: accumulator.by_resource_type.clone(),
                by_package: accumulator.by_package.clone(),
                by_tag: accumulator.by_tag.clone(),
                by_database_schema: accumulator.by_database_schema.clone(),
                name_to_keys: accumulator.name_to_keys.clone(),
                by_path_prefix: by_path_prefix.clone(),
                tests_by_entity: tests_by_entity.clone(),
                tests_by_column: tests_by_column.clone(),
                unique_id_to_resource_type: accumulator.unique_id_to_resource_type.clone(),
                unique_id_to_path: accumulator.unique_id_to_path.clone(),
                unique_id_to_tag_strings: accumulator.unique_id_to_tag_strings.clone(),
                entity_counts: accumulator.entity_counts.clone(),
                manifest_metadata_json,
            };
            if let Err(err) = rkyv_indexes::save_indexes(&indexes, &storage_dir) {
                tracing::warn!(error = %err, "failed to save index cache");
            }
        }
        build_lock.take();

        Ok(Self {
            config,
            entities: Arc::new(entities),
            parent_map,
            child_map,
            by_resource_type: accumulator.by_resource_type,
            by_package: accumulator.by_package,
            by_tag: accumulator.by_tag,
            by_database_schema: accumulator.by_database_schema,
            name_to_keys: accumulator.name_to_keys,
            resource_type_by_id,
            tantivy,
            vector_search,
            sparse_search,
            reranker,
            compiled_layer_rules,
            tests_by_entity,
            tests_by_column,
            by_path_prefix,
            column_lineage_aliases,
            manifest_metadata: accumulator.manifest_metadata,
            manifest_health,
            entity_counts: accumulator.entity_counts,
            entity_cache,
            lineage_cache,
            entity_cache_hits: AtomicU64::new(0),
            entity_cache_misses: AtomicU64::new(0),
            lineage_cache_hits: AtomicU64::new(0),
            lineage_cache_misses: AtomicU64::new(0),
            manifest_source_uri: manifest_resolution.source_uri.clone(),
            manifest_hash: signature.content_hash.clone(),
            manifest_len: signature.len,
            manifest_modified_ms: signature.modified_ms,
            manifest_version: version_id.clone(),
            loaded_at_ms: current_time_ms(),
            vector_breaker,
            sparse_breaker,
            reranker_breaker,
            _in_use_locks: Some(in_use_locks),
        })
    }
}

fn panic_message(err: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = err.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = err.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic".to_string()
    }
}

fn combine_index_build_results<T, V, S>(
    tantivy_result: Result<T>,
    vector_result: Result<Option<V>>,
    sparse_result: Result<Option<S>>,
) -> Result<(T, Option<V>, Option<S>)> {
    let mut failures = Vec::new();
    if let Err(err) = &tantivy_result {
        failures.push(format!("tantivy: {err}"));
    }
    if let Err(err) = &vector_result {
        failures.push(format!("vector: {err}"));
    }
    if let Err(err) = &sparse_result {
        failures.push(format!("sparse: {err}"));
    }

    if !failures.is_empty() {
        return Err(DbtNovaError::ServerError(format!(
            "Manifest index build failed: {}",
            failures.join("; ")
        )));
    }

    let Ok(tantivy) = tantivy_result else {
        return Err(DbtNovaError::ServerError(
            "Manifest index build failed (tantivy result missing)".to_string(),
        ));
    };
    let Ok(vector) = vector_result else {
        return Err(DbtNovaError::ServerError(
            "Manifest index build failed (vector result missing)".to_string(),
        ));
    };
    let Ok(sparse) = sparse_result else {
        return Err(DbtNovaError::ServerError(
            "Manifest index build failed (sparse result missing)".to_string(),
        ));
    };

    Ok((tantivy, vector, sparse))
}

fn build_column_lineage_aliases(
    entities: &EntityStore,
    config: &DbtNovaConfig,
) -> Result<HashMap<String, HashMap<String, String>>> {
    if !config.search.column_lineage_precompute {
        return Ok(HashMap::new());
    }

    let mut aliases = HashMap::new();
    for unique_id in entities.ids().cloned().collect::<Vec<_>>() {
        let Some(entity) = entities.get_blocking(&unique_id)? else {
            continue;
        };
        let payload: JsonValue = match serde_json::from_str(&entity.payload_json) {
            Ok(payload) => payload,
            Err(err) => {
                warn!(
                    unique_id = %unique_id,
                    error = %err,
                    "failed to parse entity payload_json during column lineage alias precompute; skipping entity"
                );
                continue;
            }
        };
        if let Some(sql) = sql_for_matching(&payload) {
            let map = find_sql_aliases(sql);
            if !map.is_empty() {
                aliases.insert(unique_id, map);
            }
        }
    }

    Ok(aliases)
}

fn build_manifest_health(
    entities: &EntityStore,
    parent_map: &HashMap<String, HashSet<String>>,
    resource_type_by_id: &HashMap<String, String>,
) -> Result<JsonValue> {
    const SAMPLE_LIMIT: usize = 25;

    let mut models_total = 0usize;
    let mut models_with_dependencies = 0usize;
    let mut models_without_dependencies = 0usize;
    let mut models_with_ref_calls = 0usize;
    let mut models_ref_calls_without_dependencies = 0usize;
    let mut malformed_ref_candidate_sample = Vec::new();
    let mut ref_without_dependencies_sample = Vec::new();

    for unique_id in entities.ids() {
        if resource_type_by_id.get(unique_id).map(String::as_str) != Some("model") {
            continue;
        }
        models_total += 1;
        let dependency_count = parent_map.get(unique_id).map_or(0, HashSet::len);
        if dependency_count > 0 {
            models_with_dependencies += 1;
        } else {
            models_without_dependencies += 1;
        }

        let Some(entity) = entities.get_blocking(unique_id)? else {
            continue;
        };
        let payload = entity.to_json_value();
        let Some(sql) = sql_for_matching(&payload) else {
            continue;
        };
        let ref_calls = extract_ref_calls(sql);
        if ref_calls.is_empty() {
            continue;
        }
        models_with_ref_calls += 1;

        if dependency_count == 0 {
            models_ref_calls_without_dependencies += 1;
            if ref_without_dependencies_sample.len() < SAMPLE_LIMIT {
                ref_without_dependencies_sample.push(unique_id.clone());
            }
            if has_apparent_malformed_ref(sql)
                && malformed_ref_candidate_sample.len() < SAMPLE_LIMIT
            {
                malformed_ref_candidate_sample.push(unique_id.clone());
            }
        }
    }

    Ok(serde_json::json!({
        "is_healthy": models_ref_calls_without_dependencies == 0,
        "models_total": models_total,
        "models_with_dependencies": models_with_dependencies,
        "models_without_dependencies": models_without_dependencies,
        "models_with_ref_calls": models_with_ref_calls,
        "models_ref_calls_without_dependencies": models_ref_calls_without_dependencies,
        "malformed_ref_candidate_count": malformed_ref_candidate_sample.len(),
        "malformed_ref_candidate_sample": malformed_ref_candidate_sample,
        "ref_calls_without_dependencies_sample": ref_without_dependencies_sample
    }))
}

fn has_apparent_malformed_ref(sql: &str) -> bool {
    let lower = sql.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let mut cursor = 0usize;

    while let Some(found) = lower[cursor..].find("ref(") {
        let ref_idx = cursor + found;
        let mut left = ref_idx;
        while left > 0 && bytes[left - 1].is_ascii_whitespace() {
            left -= 1;
        }
        if left > 0 && bytes[left - 1] == b'{' {
            let mut second = left - 1;
            while second > 0 && bytes[second - 1].is_ascii_whitespace() {
                second -= 1;
            }
            if second == 0 || bytes[second - 1] != b'{' {
                return true;
            }
        }
        cursor = ref_idx + 4;
    }

    false
}

fn read_manifest_signature(path: &Path) -> Result<Option<ManifestSignature>> {
    if !path.exists() {
        return Ok(None);
    }
    let file = File::open(path)?;
    let sig = serde_json::from_reader(BufReader::new(file))
        .map_err(|e| DbtNovaError::ServerError(format!("Invalid manifest signature: {e}")))?;
    Ok(Some(sig))
}

fn write_manifest_signature(path: &Path, sig: &ManifestSignature) -> Result<()> {
    write_json_atomic(path, sig).map_err(|e| {
        DbtNovaError::ServerError(format!("Failed to write manifest signature: {e}"))
    })?;
    Ok(())
}

fn read_current_version(path: &Path) -> Result<Option<String>> {
    let current_path = path.join(MANIFEST_CURRENT_FILENAME);
    if !current_path.exists() {
        return Ok(None);
    }
    let file = File::open(current_path)?;
    let current: ManifestCurrent = serde_json::from_reader(BufReader::new(file))
        .map_err(|e| DbtNovaError::ServerError(format!("Invalid manifest current file: {e}")))?;
    if current.version.trim().is_empty() {
        return Ok(None);
    }
    Ok(Some(current.version))
}

fn write_current_version(path: &Path, version: &str) -> Result<()> {
    let current_path = path.join(MANIFEST_CURRENT_FILENAME);
    let updated_ms = current_time_ms();
    let current = ManifestCurrent {
        version: version.to_string(),
        updated_ms,
    };
    write_json_atomic(&current_path, &current).map_err(|e| {
        DbtNovaError::ServerError(format!("Failed to write manifest current file: {e}"))
    })?;
    Ok(())
}

fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let data = serde_json::to_vec(value)
        .map_err(|e| DbtNovaError::ServerError(format!("Failed to encode JSON: {e}")))?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("manifest.json");
    let tmp = path.with_file_name(format!("{file_name}.{}.tmp", unique_suffix()));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)?;
    file.write_all(&data)?;
    file.sync_all()?;
    fs::rename(tmp, path)?;
    Ok(())
}

fn current_time_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

fn acquire_build_lock(storage_dir: &Path, wait_secs: u64) -> Result<File> {
    let lock_path = storage_dir.join(".build.lock");
    let file = fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)?;
    let start = Instant::now();
    let wait = Duration::from_secs(wait_secs);
    let mut warned = false;
    loop {
        match file.try_lock_exclusive() {
            Ok(()) => return Ok(file),
            Err(err) => {
                if !warned {
                    tracing::info!(
                        wait_secs,
                        "storage build lock held by another process; waiting"
                    );
                    warned = true;
                }
                if start.elapsed() >= wait {
                    return Err(DbtNovaError::ServerError(format!(
                        "Storage lock failed after {wait_secs}s: {err}"
                    )));
                }
                std::thread::sleep(Duration::from_millis(250));
            }
        }
    }
}

fn acquire_in_use_locks(instance_root: &Path, storage_dir: &Path) -> Result<InUseLocks> {
    let instance_root_lock = acquire_in_use_lock(instance_root)?;
    let version_dir_lock = acquire_in_use_lock(storage_dir)?;
    Ok(InUseLocks {
        instance_root: instance_root_lock,
        version_dir: version_dir_lock,
    })
}

fn acquire_in_use_lock(lock_dir: &Path) -> Result<File> {
    let lock_path = lock_dir.join(IN_USE_LOCK_FILENAME);
    let file = fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)?;
    file.lock_shared()
        .map_err(|e| DbtNovaError::ServerError(format!("Storage in-use lock failed: {e}")))?;
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::entity::Entity;
    use crate::manifest::store::EntityStoreBuilder;
    use tempfile::tempdir;

    #[test]
    fn combine_index_build_results_aggregates_failures() {
        let result = combine_index_build_results::<u8, u8, u8>(
            Err(DbtNovaError::ServerError("tantivy panic".to_string())),
            Err(DbtNovaError::ServerError("vector panic".to_string())),
            Ok(None),
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
        let result = combine_index_build_results(Ok(1u8), Ok(Some(2u8)), Ok(None::<u8>));
        match result {
            Ok((tantivy, vector, sparse)) => {
                assert_eq!(tantivy, 1);
                assert_eq!(vector, Some(2));
                assert_eq!(sparse, None);
            }
            Err(err) => panic!("unexpected error: {err}"),
        }
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
}
