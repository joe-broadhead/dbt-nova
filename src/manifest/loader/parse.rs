use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::time::Instant;

use serde::de::{self, DeserializeSeed, Deserializer, MapAccess, Visitor};
use serde_json::Value as JsonValue;
use tracing::{info, warn};

use crate::error::{DbtNovaError, Result};
use crate::manifest::entity::Entity;
use crate::manifest::store::{EntityStore, EntityStoreBuilder};
use crate::utils::{GlobPattern, compile_glob, glob_match_compiled};

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
        .map(|(k, v)| {
            let mut values: Vec<String> = v.into_iter().collect();
            values.sort_unstable();
            (k, values)
        })
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
    pruner: Option<&'a mut ManifestPruner>,
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
            pruner: self.pruner,
            forced_resource_type: self.forced_resource_type,
        })
    }
}

struct EntitiesVisitor<'a> {
    accumulator: &'a mut ManifestAccumulator,
    pruner: Option<&'a mut ManifestPruner>,
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
        let mut pruner = self.pruner;
        while let Some((unique_id, payload)) = map.next_entry::<String, JsonValue>()? {
            let keep = match pruner.as_deref_mut() {
                Some(matcher) => {
                    matcher.should_keep_entity(&unique_id, &payload, self.forced_resource_type)
                }
                None => true,
            };
            if !keep {
                continue;
            }
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
    pruner: Option<&'a mut ManifestPruner>,
}

impl<'de> DeserializeSeed<'de> for ManifestSeed<'_> {
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> std::result::Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_map(ManifestVisitor {
            accumulator: self.accumulator,
            pruner: self.pruner,
        })
    }
}

struct ManifestVisitor<'a> {
    accumulator: &'a mut ManifestAccumulator,
    pruner: Option<&'a mut ManifestPruner>,
}

impl<'de> Visitor<'de> for ManifestVisitor<'_> {
    type Value = ();

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a manifest object")
    }

    fn visit_map<M>(mut self, mut map: M) -> std::result::Result<Self::Value, M::Error>
    where
        M: MapAccess<'de>,
    {
        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "nodes" => {
                    map.next_value_seed(EntitiesSeed {
                        accumulator: self.accumulator,
                        pruner: self.pruner.as_deref_mut(),
                        forced_resource_type: None,
                    })?;
                }
                "sources" => {
                    map.next_value_seed(EntitiesSeed {
                        accumulator: self.accumulator,
                        pruner: self.pruner.as_deref_mut(),
                        forced_resource_type: Some("source"),
                    })?;
                }
                "macros" => {
                    map.next_value_seed(EntitiesSeed {
                        accumulator: self.accumulator,
                        pruner: self.pruner.as_deref_mut(),
                        forced_resource_type: Some("macro"),
                    })?;
                }
                "docs" => {
                    map.next_value_seed(EntitiesSeed {
                        accumulator: self.accumulator,
                        pruner: self.pruner.as_deref_mut(),
                        forced_resource_type: Some("doc"),
                    })?;
                }
                "groups" => {
                    map.next_value_seed(EntitiesSeed {
                        accumulator: self.accumulator,
                        pruner: self.pruner.as_deref_mut(),
                        forced_resource_type: Some("group"),
                    })?;
                }
                "exposures" => {
                    map.next_value_seed(EntitiesSeed {
                        accumulator: self.accumulator,
                        pruner: self.pruner.as_deref_mut(),
                        forced_resource_type: Some("exposure"),
                    })?;
                }
                "metrics" => {
                    map.next_value_seed(EntitiesSeed {
                        accumulator: self.accumulator,
                        pruner: self.pruner.as_deref_mut(),
                        forced_resource_type: Some("metric"),
                    })?;
                }
                "saved_queries" => {
                    map.next_value_seed(EntitiesSeed {
                        accumulator: self.accumulator,
                        pruner: self.pruner.as_deref_mut(),
                        forced_resource_type: Some("saved_query"),
                    })?;
                }
                "semantic_models" => {
                    map.next_value_seed(EntitiesSeed {
                        accumulator: self.accumulator,
                        pruner: self.pruner.as_deref_mut(),
                        forced_resource_type: Some("semantic_model"),
                    })?;
                }
                "unit_tests" => {
                    map.next_value_seed(EntitiesSeed {
                        accumulator: self.accumulator,
                        pruner: self.pruner.as_deref_mut(),
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

struct CompiledPattern {
    raw: String,
    compiled: GlobPattern,
    matched: bool,
}

impl CompiledPattern {
    fn new(pattern: String) -> Self {
        Self {
            compiled: compile_glob(&pattern, true),
            raw: pattern,
            matched: false,
        }
    }
}

struct ManifestPruner {
    allow_patterns: Vec<CompiledPattern>,
    deny_patterns: Vec<CompiledPattern>,
}

impl ManifestPruner {
    fn from_patterns(allow_ids: &[String], deny_ids: &[String]) -> Option<Self> {
        let allow_patterns: Vec<CompiledPattern> = allow_ids
            .iter()
            .filter_map(|pattern| {
                let trimmed = pattern.trim();
                (!trimmed.is_empty()).then(|| CompiledPattern::new(trimmed.to_string()))
            })
            .collect();
        let deny_patterns: Vec<CompiledPattern> = deny_ids
            .iter()
            .filter_map(|pattern| {
                let trimmed = pattern.trim();
                (!trimmed.is_empty()).then(|| CompiledPattern::new(trimmed.to_string()))
            })
            .collect();
        if allow_patterns.is_empty() && deny_patterns.is_empty() {
            return None;
        }
        Some(Self {
            allow_patterns,
            deny_patterns,
        })
    }

    fn should_keep_entity(
        &mut self,
        unique_id: &str,
        payload: &JsonValue,
        forced_resource_type: Option<&str>,
    ) -> bool {
        let keep_base = self.keep_by_allow_deny(unique_id, true);
        if keep_base {
            return true;
        }
        if self.matches_deny(unique_id, false) || !Self::is_analysis(payload, forced_resource_type)
        {
            return false;
        }
        self.should_auto_include_analysis(payload)
    }

    fn should_auto_include_analysis(&mut self, payload: &JsonValue) -> bool {
        let deps = payload
            .get("depends_on")
            .and_then(|d| d.get("nodes"))
            .and_then(|n| n.as_array());
        let Some(dependencies) = deps else {
            return false;
        };
        if dependencies.is_empty() {
            return false;
        }
        dependencies.iter().all(|value| {
            value
                .as_str()
                .is_some_and(|id| self.keep_by_allow_deny(id, false))
        })
    }

    fn report_unmatched(&self) {
        for pattern in &self.allow_patterns {
            if !pattern.matched {
                warn!(
                    pattern = %pattern.raw,
                    "manifest prune allow pattern did not match any entities"
                );
            }
        }
        for pattern in &self.deny_patterns {
            if !pattern.matched {
                warn!(
                    pattern = %pattern.raw,
                    "manifest prune deny pattern did not match any entities"
                );
            }
        }
    }

    fn is_analysis(payload: &JsonValue, forced_resource_type: Option<&str>) -> bool {
        forced_resource_type == Some("analysis")
            || payload
                .get("resource_type")
                .and_then(JsonValue::as_str)
                .is_some_and(|resource_type| resource_type == "analysis")
    }

    fn matches_mut(patterns: &mut [CompiledPattern], unique_id: &str, mark: bool) -> bool {
        let mut matched = false;
        for pattern in patterns {
            if glob_match_compiled(&pattern.compiled, unique_id) {
                matched = true;
                if mark {
                    pattern.matched = true;
                }
            }
        }
        matched
    }

    fn matches_allow(&mut self, unique_id: &str, mark: bool) -> bool {
        Self::matches_mut(&mut self.allow_patterns, unique_id, mark)
    }

    fn matches_deny(&mut self, unique_id: &str, mark: bool) -> bool {
        Self::matches_mut(&mut self.deny_patterns, unique_id, mark)
    }

    fn keep_by_allow_deny(&mut self, unique_id: &str, mark: bool) -> bool {
        let keep_after_allow = if self.allow_patterns.is_empty() {
            true
        } else {
            self.matches_allow(unique_id, mark)
        };
        keep_after_allow && !self.matches_deny(unique_id, mark)
    }
}

fn prune_lineage_maps(accumulator: &mut ManifestAccumulator) {
    accumulator
        .parent_map
        .retain(|node_id, _| accumulator.seen_unique_ids.contains(node_id));
    for dependencies in accumulator.parent_map.values_mut() {
        dependencies.retain(|dep_id| accumulator.seen_unique_ids.contains(dep_id));
    }
    accumulator
        .child_map
        .retain(|node_id, _| accumulator.seen_unique_ids.contains(node_id));
    for children in accumulator.child_map.values_mut() {
        children.retain(|child_id| accumulator.seen_unique_ids.contains(child_id));
    }
}

pub(super) fn parse_manifest_file(
    manifest_path: &Path,
    source_uri: &str,
    allow_ids: &[String],
    deny_ids: &[String],
    accumulator: &mut ManifestAccumulator,
    load_start: Instant,
) -> Result<()> {
    let file = File::open(manifest_path).map_err(|err| {
        DbtNovaError::ManifestError(format!("Failed to read manifest file {source_uri}: {err}"))
    })?;
    let reader = BufReader::new(file);

    let mut deserializer = serde_json::Deserializer::from_reader(reader);
    let mut pruner = ManifestPruner::from_patterns(allow_ids, deny_ids);
    ManifestSeed {
        accumulator,
        pruner: pruner.as_mut(),
    }
    .deserialize(&mut deserializer)
    .map_err(|err| DbtNovaError::ManifestError(format!("Failed to parse manifest JSON: {err}")))?;
    if let Some(pruner) = pruner.as_ref() {
        pruner.report_unmatched();
    }
    prune_lineage_maps(accumulator);
    info!(
        elapsed_ms = load_start.elapsed().as_millis(),
        "manifest parsed"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{ManifestAccumulator, map_set_to_vec, parse_manifest_file};
    use std::collections::{HashMap, HashSet};
    use std::fs;
    use std::time::Instant;
    use tempfile::tempdir;

    #[test]
    fn map_set_to_vec_sorts_adjacency_for_stable_lineage() {
        let map = HashMap::from([(
            "model.pkg.orders".to_string(),
            HashSet::from([
                "model.pkg.stg_payments".to_string(),
                "model.pkg.stg_customers".to_string(),
                "model.pkg.stg_orders".to_string(),
            ]),
        )]);

        let sorted = map_set_to_vec(map);

        assert_eq!(
            sorted.get("model.pkg.orders").expect("adjacency"),
            &vec![
                "model.pkg.stg_customers".to_string(),
                "model.pkg.stg_orders".to_string(),
                "model.pkg.stg_payments".to_string(),
            ]
        );
    }

    #[test]
    fn parse_manifest_prunes_by_allow_and_strict_analysis_dependencies() {
        let manifest = r#"{
          "metadata": { "dbt_version": "1.10.0" },
          "nodes": {
            "model.pkg.base_a": {
              "name": "base_a",
              "resource_type": "model",
              "package_name": "pkg",
              "depends_on": { "nodes": [], "macros": [] }
            },
            "model.pkg.base_b": {
              "name": "base_b",
              "resource_type": "model",
              "package_name": "pkg",
              "depends_on": { "nodes": [], "macros": [] }
            },
            "analysis.pkg.only_a": {
              "name": "only_a",
              "resource_type": "analysis",
              "package_name": "pkg",
              "depends_on": { "nodes": ["model.pkg.base_a"], "macros": [] }
            },
            "analysis.pkg.a_and_b": {
              "name": "a_and_b",
              "resource_type": "analysis",
              "package_name": "pkg",
              "depends_on": { "nodes": ["model.pkg.base_a", "model.pkg.base_b"], "macros": [] }
            },
            "analysis.pkg.empty_deps": {
              "name": "empty_deps",
              "resource_type": "analysis",
              "package_name": "pkg",
              "depends_on": { "nodes": [], "macros": [] }
            }
          },
          "sources": {},
          "macros": {},
          "docs": {},
          "groups": {},
          "exposures": {},
          "metrics": {},
          "saved_queries": {},
          "semantic_models": {},
          "unit_tests": {},
          "parent_map": {
            "analysis.pkg.only_a": ["model.pkg.base_a"],
            "analysis.pkg.a_and_b": ["model.pkg.base_a", "model.pkg.base_b"]
          },
          "child_map": {
            "model.pkg.base_a": ["analysis.pkg.only_a", "analysis.pkg.a_and_b"],
            "model.pkg.base_b": ["analysis.pkg.a_and_b"]
          }
        }"#;

        let temp = tempdir().expect("tempdir");
        let manifest_path = temp.path().join("manifest.json");
        fs::write(&manifest_path, manifest).expect("write manifest");
        let mut accumulator = ManifestAccumulator::new(temp.path(), false).expect("accumulator");
        let allow_ids = vec!["model.pkg.base_a".to_string()];
        let deny_ids: Vec<String> = Vec::new();

        parse_manifest_file(
            &manifest_path,
            manifest_path.to_string_lossy().as_ref(),
            &allow_ids,
            &deny_ids,
            &mut accumulator,
            Instant::now(),
        )
        .expect("parse manifest");

        assert!(accumulator.seen_unique_ids.contains("model.pkg.base_a"));
        assert!(!accumulator.seen_unique_ids.contains("model.pkg.base_b"));
        assert!(accumulator.seen_unique_ids.contains("analysis.pkg.only_a"));
        assert!(!accumulator.seen_unique_ids.contains("analysis.pkg.a_and_b"));
        assert!(
            !accumulator
                .seen_unique_ids
                .contains("analysis.pkg.empty_deps")
        );

        let parent_only_a = accumulator
            .parent_map
            .get("analysis.pkg.only_a")
            .expect("parent map for only_a");
        assert!(parent_only_a.contains("model.pkg.base_a"));
        assert!(!accumulator.parent_map.contains_key("analysis.pkg.a_and_b"));

        let child_base_a = accumulator
            .child_map
            .get("model.pkg.base_a")
            .expect("child map for base_a");
        assert!(child_base_a.contains("analysis.pkg.only_a"));
        assert!(!child_base_a.contains("analysis.pkg.a_and_b"));
    }

    #[test]
    fn parse_manifest_deny_overrides_auto_analysis_inclusion() {
        let manifest = r#"{
          "metadata": { "dbt_version": "1.10.0" },
          "nodes": {
            "model.pkg.base_a": {
              "name": "base_a",
              "resource_type": "model",
              "package_name": "pkg",
              "depends_on": { "nodes": [], "macros": [] }
            },
            "analysis.pkg.only_a": {
              "name": "only_a",
              "resource_type": "analysis",
              "package_name": "pkg",
              "depends_on": { "nodes": ["model.pkg.base_a"], "macros": [] }
            }
          },
          "sources": {},
          "macros": {},
          "docs": {},
          "groups": {},
          "exposures": {},
          "metrics": {},
          "saved_queries": {},
          "semantic_models": {},
          "unit_tests": {}
        }"#;

        let temp = tempdir().expect("tempdir");
        let manifest_path = temp.path().join("manifest.json");
        fs::write(&manifest_path, manifest).expect("write manifest");
        let mut accumulator = ManifestAccumulator::new(temp.path(), false).expect("accumulator");
        let allow_ids = vec!["model.pkg.base_a".to_string()];
        let deny_ids = vec!["analysis.pkg.*".to_string()];

        parse_manifest_file(
            &manifest_path,
            manifest_path.to_string_lossy().as_ref(),
            &allow_ids,
            &deny_ids,
            &mut accumulator,
            Instant::now(),
        )
        .expect("parse manifest");

        assert!(accumulator.seen_unique_ids.contains("model.pkg.base_a"));
        assert!(!accumulator.seen_unique_ids.contains("analysis.pkg.only_a"));
    }
}
