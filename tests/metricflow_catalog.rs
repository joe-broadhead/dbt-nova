//! End-to-end tests for dbt Semantic Layer and catalog.json ingestion.
#[path = "support/config.rs"]
mod support_config;
#[path = "support/json.rs"]
mod support_json;

use std::fs;
use std::path::Path;

use dbt_nova::params::{
    GetColumnsParams, GetMetadataScoreParams, IndicatorInventoryParams, PaginationParams,
    SearchIndicatorParams,
};
use dbt_nova::{DbtNovaConfig, ManifestSearch};
use support_json::json;

fn load_searcher(
    manifest_path: &Path,
    catalog_path: Option<&Path>,
) -> (ManifestSearch, support_config::TestStorageGuard) {
    let guard = support_config::TestStorageGuard::new();
    let mut cfg = DbtNovaConfig {
        manifest_path: manifest_path.to_string_lossy().to_string(),
        search: support_config::test_search_config(),
        ..Default::default()
    };
    if let Some(catalog_path) = catalog_path {
        cfg.catalog_path = catalog_path.to_string_lossy().to_string();
    }
    support_config::apply_test_storage(&mut cfg, &guard);
    let searcher = ManifestSearch::new(cfg)
        .expect("synthetic manifest should load")
        .search;
    (searcher, guard)
}

#[tokio::test(flavor = "multi_thread")]
async fn metricflow_metrics_are_discoverable_without_nova_meta() {
    let manifest = r#"{
      "metadata": {"dbt_version": "1.10.0"},
      "nodes": {},
      "sources": {},
      "macros": {},
      "docs": {},
      "groups": {},
      "exposures": {},
      "metrics": {
        "metric.pkg.gross_revenue": {
          "name": "gross_revenue",
          "resource_type": "metric",
          "package_name": "pkg",
          "label": "Gross revenue",
          "description": "Total gross revenue",
          "type": "simple",
          "type_params": {"measure": {"name": "order_total"}}
        }
      },
      "saved_queries": {},
      "semantic_models": {
        "semantic_model.pkg.orders": {
          "name": "orders",
          "resource_type": "semantic_model",
          "package_name": "pkg",
          "entities": [{"name": "order", "type": "primary", "expr": "order_id"}],
          "dimensions": [{"name": "ordered_at", "type": "time", "expr": "ordered_at"}],
          "measures": [{"name": "order_total", "agg": "sum", "expr": "amount", "label": "Order total"}]
        }
      },
      "unit_tests": {}
    }"#;
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest_path = temp.path().join("nova_manifest.json");
    fs::write(&manifest_path, manifest).expect("write manifest");
    let (searcher, _guard) = load_searcher(&manifest_path, None);

    let inventory = json(
        searcher
            .indicator_inventory(&IndicatorInventoryParams {
                resource_types: vec![],
                indicator_types: vec![],
                canonical_only: false,
                pagination: PaginationParams {
                    limit: Some(10),
                    offset: 0,
                },
            })
            .await,
    );
    let inventory_rows = inventory["data"].as_array().expect("inventory rows");
    assert!(
        inventory_rows.iter().any(
            |row| row["indicator_name"] == "gross_revenue" && row["indicator_type"] == "metric"
        ),
        "metricflow metric should be inventoried: {inventory}"
    );
    assert!(
        inventory_rows
            .iter()
            .any(|row| row["indicator_name"] == "order_total"
                && row["indicator_type"] == "measure"),
        "semantic model measure should be inventoried: {inventory}"
    );

    let search = json(
        searcher
            .search_indicator(&SearchIndicatorParams {
                query: "gross revenue".to_string(),
                pagination: PaginationParams {
                    limit: Some(5),
                    offset: 0,
                },
                ..Default::default()
            })
            .await,
    );
    assert!(
        search["data"]
            .as_array()
            .expect("search rows")
            .iter()
            .any(
                |row| row["indicator_name"] == "gross_revenue" && row["indicator_type"] == "metric"
            ),
        "metricflow metric should be searchable: {search}"
    );

    let score = json(
        searcher
            .get_metadata_score(&GetMetadataScoreParams {
                id_or_name: Some("metric.pkg.gross_revenue".to_string()),
                resource_type: Some("metric".to_string()),
                persona: Some("analyst".to_string()),
                scope: Some("entity".to_string()),
                include_breakdown: true,
                include_recommendations: false,
                resource_types: vec![],
                limit: None,
                offset: None,
            })
            .await,
    );
    assert_eq!(score["success"].as_bool(), Some(true), "{score}");
    assert!(
        score["data"]["categories"]["semantic"]["score"]
            .as_u64()
            .unwrap_or(0)
            > 0,
        "derived metric metadata should contribute semantic score: {score}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn catalog_json_enriches_columns_and_surfaces_drift() {
    let manifest = r#"{
      "metadata": {"dbt_version": "1.10.0"},
      "nodes": {
        "model.pkg.orders": {
          "name": "orders",
          "resource_type": "model",
          "package_name": "pkg",
          "columns": {
            "amount": {"name": "amount", "data_type": "text"},
            "declared_only": {"name": "declared_only", "data_type": "integer"}
          },
          "depends_on": {"nodes": [], "macros": []}
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
    let catalog = serde_json::json!({
        "nodes": {
            "model.pkg.orders": {
                "columns": {
                    "amount": {"type": "numeric", "comment": "Warehouse amount"},
                    "catalog_only": {"type": "boolean", "comment": "Present in warehouse"}
                }
            }
        }
    });
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest_path = temp.path().join("nova_manifest.json");
    let catalog_path = temp.path().join("catalog.json");
    fs::write(&manifest_path, manifest).expect("write manifest");
    fs::write(
        &catalog_path,
        serde_json::to_vec(&catalog).expect("serialize catalog"),
    )
    .expect("write catalog");
    let (searcher, _guard) = load_searcher(&manifest_path, Some(&catalog_path));

    let columns = json(
        searcher
            .get_columns(&GetColumnsParams {
                id_or_name: "model.pkg.orders".to_string(),
            })
            .await,
    );
    let rows = columns["data"]["columns"].as_array().expect("column rows");
    let amount = rows
        .iter()
        .find(|row| row["name"] == "amount")
        .expect("amount column");
    let catalog_only = rows
        .iter()
        .find(|row| row["name"] == "catalog_only")
        .expect("catalog-only column");
    let declared_only = rows
        .iter()
        .find(|row| row["name"] == "declared_only")
        .expect("declared-only column");

    assert_eq!(amount["data_type"].as_str(), Some("numeric"));
    assert_eq!(amount["manifest_data_type"].as_str(), Some("text"));
    assert_eq!(
        amount["catalog_drift"]["type_mismatch"].as_bool(),
        Some(true)
    );
    assert_eq!(
        catalog_only["catalog_drift"]["catalog_only"].as_bool(),
        Some(true)
    );
    assert_eq!(
        declared_only["catalog_drift"]["missing_in_catalog"].as_bool(),
        Some(true)
    );
}
