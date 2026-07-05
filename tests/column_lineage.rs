//! Integration tests for column lineage resolution.
#[path = "support/config.rs"]
mod support_config;
#[path = "support/fixtures.rs"]
mod support_fixtures;
#[path = "support/json.rs"]
mod support_json;

use dbt_nova::params::GetColumnLineageParams;
use support_fixtures::load_fixture;
use support_json::json;

#[tokio::test(flavor = "multi_thread")]
async fn sql_alias_detection_matches_downstream_column() {
    // Fixture models a downstream column rename via SQL alias to validate alias matching.
    let searcher = load_fixture("alias.json").unwrap();
    let params = GetColumnLineageParams {
        id_or_name: "model.pkg.upstream".into(),
        resource_type: None,
        column_name: "old_col".into(),
        direction: "downstream".into(),
        depth: Some(1),
        confidence: Some("medium".into()),
    };
    let result = json(searcher.get_column_lineage(&params).await);
    let lineage = result
        .get("data")
        .and_then(|d| d.get("lineage"))
        .and_then(|l| l.as_array())
        .cloned()
        .unwrap_or_default();
    assert_eq!(lineage.len(), 1, "should find alias match");
    let item = &lineage[0];
    let reason = item.get("match_reason").and_then(|v| v.as_str());
    assert!(
        matches!(reason, Some("sql_alias") | Some("sql_proximity")),
        "expected alias/proximity match, got {:?}",
        reason
    );
    assert_eq!(item.get("column").and_then(|v| v.as_str()), Some("new_col"));
}

#[tokio::test(flavor = "multi_thread")]
async fn column_lineage_depth_clamps_to_config_max() {
    let guard = support_config::TestStorageGuard::new();
    let mut cfg = dbt_nova::DbtNovaConfig {
        manifest_path: support_fixtures::fixture_path("alias.json")
            .to_string_lossy()
            .to_string(),
        search: support_config::test_search_config(),
        ..Default::default()
    };
    support_config::apply_test_storage(&mut cfg, &guard);
    cfg.column_lineage.max_depth = 1;
    let searcher = dbt_nova::ManifestSearch::new(cfg)
        .expect("fixture manifest must be present")
        .search;

    let params = GetColumnLineageParams {
        id_or_name: "model.pkg.upstream".into(),
        resource_type: None,
        column_name: "old_col".into(),
        direction: "downstream".into(),
        depth: Some(10),
        confidence: Some("medium".into()),
    };
    let result = json(searcher.get_column_lineage(&params).await);
    let lineage = result
        .get("data")
        .and_then(|d| d.get("lineage"))
        .and_then(|l| l.as_array())
        .cloned()
        .unwrap_or_default();
    for item in lineage {
        let depth = item.get("depth").and_then(|v| v.as_u64()).unwrap_or(0);
        assert!(depth <= 1, "depth exceeded max: {depth}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn type_mismatch_blocks_match() {
    // Fixture models a column rename with incompatible types to ensure type checks prevent false lineage.
    let searcher = load_fixture("type_mismatch.json").unwrap();
    let params = GetColumnLineageParams {
        id_or_name: "model.pkg.src".into(),
        resource_type: None,
        column_name: "num_col".into(),
        direction: "downstream".into(),
        depth: Some(1),
        confidence: Some("low".into()),
    };
    let result = json(searcher.get_column_lineage(&params).await);
    let lineage = result
        .get("data")
        .and_then(|d| d.get("lineage"))
        .and_then(|l| l.as_array())
        .cloned()
        .unwrap_or_default();
    assert_eq!(
        lineage.len(),
        0,
        "type incompatibility should prevent match"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn truncated_column_lineage_returns_deterministic_subset() {
    let manifest = r#"{
      "metadata": {"dbt_version": "1.10.0"},
      "nodes": {
        "model.pkg.root": {
          "name": "root",
          "resource_type": "model",
          "package_name": "pkg",
          "columns": {"old_col": {"name": "old_col", "data_type": "integer"}},
          "depends_on": {"nodes": [], "macros": []}
        },
        "model.pkg.a": {
          "name": "a",
          "resource_type": "model",
          "package_name": "pkg",
          "columns": {"old_col": {"name": "old_col", "data_type": "integer"}},
          "depends_on": {"nodes": ["model.pkg.root"], "macros": []}
        },
        "model.pkg.b": {
          "name": "b",
          "resource_type": "model",
          "package_name": "pkg",
          "columns": {"old_col": {"name": "old_col", "data_type": "integer"}},
          "depends_on": {"nodes": ["model.pkg.root"], "macros": []}
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
        "model.pkg.a": ["model.pkg.root"],
        "model.pkg.b": ["model.pkg.root"]
      },
      "child_map": {
        "model.pkg.root": ["model.pkg.b", "model.pkg.a"]
      }
    }"#;
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest_path = temp.path().join("nova_manifest.json");
    std::fs::write(&manifest_path, manifest).expect("write manifest");

    let mut outputs = Vec::new();
    for instance in [
        "column-lineage-determinism-a",
        "column-lineage-determinism-b",
    ] {
        let guard = support_config::TestStorageGuard::new();
        let mut cfg = dbt_nova::DbtNovaConfig {
            manifest_path: manifest_path.to_string_lossy().to_string(),
            search: support_config::test_search_config(),
            ..Default::default()
        };
        support_config::apply_test_storage(&mut cfg, &guard);
        cfg.storage_instance_id = instance.to_string();
        cfg.column_lineage.max_results = 1;
        let searcher = dbt_nova::ManifestSearch::new(cfg)
            .expect("synthetic manifest should load")
            .search;
        let result = json(
            searcher
                .get_column_lineage(&GetColumnLineageParams {
                    id_or_name: "model.pkg.root".to_string(),
                    resource_type: None,
                    column_name: "old_col".to_string(),
                    direction: "downstream".to_string(),
                    depth: Some(1),
                    confidence: Some("medium".to_string()),
                })
                .await,
        );
        outputs.push(result["data"].clone());
    }

    assert_eq!(outputs[0], outputs[1]);
    let lineage = outputs[0]["lineage"].as_array().expect("lineage");
    assert_eq!(lineage.len(), 1);
    assert_eq!(
        lineage[0]
            .get("unique_id")
            .and_then(serde_json::Value::as_str),
        Some("model.pkg.a")
    );
}
