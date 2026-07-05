//! Integration tests for entity lineage behavior.
#[path = "support/config.rs"]
mod support_config;
#[path = "support/fixtures.rs"]
mod support_fixtures;
#[path = "support/json.rs"]
mod support_json;

use std::fs;

use dbt_nova::params::{DetailLevel, GetLineageParams, ValidateDagDetail, ValidateDagParams};
use dbt_nova::{DbtNovaConfig, ManifestSearch};
use support_fixtures::{fixture_path, load_fixture};
use support_json::json;

#[tokio::test(flavor = "multi_thread")]
async fn cycle_is_reported_invalid() {
    let searcher = load_fixture("cycle.json").unwrap();
    let result = json(
        searcher
            .validate_dag(&ValidateDagParams {
                detail: ValidateDagDetail::Full,
            })
            .await,
    );
    let data = result.get("data").unwrap();
    assert_eq!(data.get("valid").and_then(|v| v.as_bool()), Some(false));
    assert!(
        data.get("issues")
            .and_then(|i| i.as_array())
            .unwrap()
            .iter()
            .any(|issue| issue.get("type") == Some(&serde_json::Value::String("cycle".into())))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn depth_limit_applies() {
    let searcher = load_fixture("alias.json").unwrap();
    let params = GetLineageParams {
        id_or_name: "model.pkg.downstream".into(),
        direction: "upstream".into(),
        depth: Some(1),
        resource_types: vec![],
        detail: Some(DetailLevel::Standard),
    };
    let result = json(searcher.get_lineage(&params).await);
    let count = result
        .get("count")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    assert_eq!(count, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn lineage_depth_clamps_to_config_max() {
    let guard = support_config::TestStorageGuard::new();
    let mut cfg = DbtNovaConfig {
        manifest_path: fixture_path("alias.json").to_string_lossy().to_string(),
        search: support_config::test_search_config(),
        ..Default::default()
    };
    support_config::apply_test_storage(&mut cfg, &guard);
    cfg.lineage_max_depth = 1;
    let searcher = ManifestSearch::new(cfg)
        .expect("fixture manifest must be present")
        .search;

    let params = GetLineageParams {
        id_or_name: "model.pkg.downstream".into(),
        direction: "upstream".into(),
        depth: Some(99),
        resource_types: vec![],
        detail: Some(DetailLevel::Standard),
    };
    let result = json(searcher.get_lineage(&params).await);
    let data = result.get("data").expect("data");
    assert_eq!(data.get("depth").and_then(|v| v.as_u64()), Some(1));
}

#[tokio::test(flavor = "multi_thread")]
async fn truncated_lineage_returns_deterministic_subset() {
    let manifest = r#"{
      "metadata": {"dbt_version": "1.10.0"},
      "nodes": {
        "model.pkg.root": {"name": "root", "resource_type": "model", "package_name": "pkg", "depends_on": {"nodes": [], "macros": []}},
        "model.pkg.a": {"name": "a", "resource_type": "model", "package_name": "pkg", "depends_on": {"nodes": ["model.pkg.root"], "macros": []}},
        "model.pkg.b": {"name": "b", "resource_type": "model", "package_name": "pkg", "depends_on": {"nodes": ["model.pkg.root"], "macros": []}},
        "model.pkg.c": {"name": "c", "resource_type": "model", "package_name": "pkg", "depends_on": {"nodes": ["model.pkg.root"], "macros": []}}
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
        "model.pkg.a": ["model.pkg.root"],
        "model.pkg.b": ["model.pkg.root"],
        "model.pkg.c": ["model.pkg.root"]
      },
      "child_map": {
        "model.pkg.root": ["model.pkg.c", "model.pkg.a", "model.pkg.b"]
      }
    }"#;
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest_path = temp.path().join("nova_manifest.json");
    fs::write(&manifest_path, manifest).expect("write manifest");

    let mut outputs = Vec::new();
    for instance in ["lineage-determinism-a", "lineage-determinism-b"] {
        let guard = support_config::TestStorageGuard::new();
        let mut cfg = DbtNovaConfig {
            manifest_path: manifest_path.to_string_lossy().to_string(),
            lineage_max_results: 2,
            search: support_config::test_search_config(),
            ..Default::default()
        };
        support_config::apply_test_storage(&mut cfg, &guard);
        cfg.storage_instance_id = instance.to_string();
        let searcher = ManifestSearch::new(cfg)
            .expect("synthetic manifest should load")
            .search;
        let result = json(
            searcher
                .get_lineage(&GetLineageParams {
                    id_or_name: "model.pkg.root".to_string(),
                    direction: "downstream".to_string(),
                    depth: Some(1),
                    resource_types: vec![],
                    detail: Some(DetailLevel::Standard),
                })
                .await,
        );
        outputs.push(result["data"].clone());
    }

    assert_eq!(outputs[0], outputs[1]);
    let ids: Vec<&str> = outputs[0]["entities"]
        .as_array()
        .expect("entities")
        .iter()
        .filter_map(|entity| entity.get("unique_id").and_then(serde_json::Value::as_str))
        .collect();
    assert_eq!(ids, vec!["model.pkg.a", "model.pkg.b"]);
}
