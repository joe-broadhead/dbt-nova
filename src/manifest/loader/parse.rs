use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::time::Instant;

use serde::de::{self, DeserializeSeed, Deserializer, MapAccess, Visitor};
use serde_json::Value as JsonValue;
use tracing::info;

use crate::error::{DbtNovaError, Result};
use crate::manifest::entity::Entity;
use crate::manifest::store::{EntityStore, EntityStoreBuilder};

pub(super) struct ManifestAccumulator {
    pub(super) store_builder: Option<EntityStoreBuilder>,
    pub(super) seen_unique_ids: HashSet<String>,
    pub(super) by_resource_type: HashMap<String, Vec<String>>,
    pub(super) by_package: HashMap<String, Vec<String>>,
    pub(super) by_tag: HashMap<String, HashSet<String>>,
    pub(super) by_database_schema: HashMap<String, Vec<String>>,
    pub(super) name_to_keys: HashMap<String, Vec<String>>,
    pub(super) entity_counts: HashMap<String, usize>,
    pub(super) unique_id_to_resource_type: HashMap<String, String>,
    pub(super) unique_id_to_path: HashMap<String, String>,
    pub(super) unique_id_to_tag_strings: HashMap<String, Vec<String>>,
    pub(super) parent_map: HashMap<String, HashSet<String>>,
    pub(super) child_map: HashMap<String, HashSet<String>>,
    pub(super) manifest_metadata: JsonValue,
}

pub(super) fn map_vec_to_set(
    map: HashMap<String, Vec<String>>,
) -> HashMap<String, HashSet<String>> {
    map.into_iter()
        .map(|(k, v)| (k, v.into_iter().collect()))
        .collect()
}

pub(super) fn map_set_to_vec(
    map: HashMap<String, HashSet<String>>,
) -> HashMap<String, Vec<String>> {
    map.into_iter()
        .map(|(k, v)| (k, v.into_iter().collect()))
        .collect()
}

impl ManifestAccumulator {
    pub(super) fn new(storage_dir: &Path, build_store: bool) -> Result<Self> {
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

    pub(super) fn add_entity(
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

    pub(super) fn record_dependencies(&mut self, unique_id: &str, payload: &JsonValue) {
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

    pub(super) fn finish_store(&mut self) -> Result<EntityStore> {
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

pub(super) fn parse_manifest_file(
    manifest_path: &Path,
    source_uri: &str,
    accumulator: &mut ManifestAccumulator,
    load_start: Instant,
) -> Result<()> {
    let file = File::open(manifest_path).map_err(|err| {
        DbtNovaError::ManifestError(format!("Failed to read manifest file {source_uri}: {err}"))
    })?;
    let reader = BufReader::new(file);

    let mut deserializer = serde_json::Deserializer::from_reader(reader);
    ManifestSeed { accumulator }
        .deserialize(&mut deserializer)
        .map_err(|err| {
            DbtNovaError::ManifestError(format!("Failed to parse manifest JSON: {err}"))
        })?;
    info!(
        elapsed_ms = load_start.elapsed().as_millis(),
        "manifest parsed"
    );
    Ok(())
}
