//! End-to-end tests for dbt Semantic Layer and catalog.json ingestion.
#[path = "support/config.rs"]
mod support_config;
#[path = "support/json.rs"]
mod support_json;

use std::fs;
use std::path::Path;

use dbt_nova::params::{
    DetailLevel, GetColumnsParams, GetMetadataScoreParams, IndicatorInventoryParams,
    ModellingConsistencyReportParams, PaginationParams, SearchIndicatorParams,
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
      "nodes": {
        "model.pkg.orders_relation": {
          "name": "orders_relation",
          "resource_type": "model",
          "package_name": "pkg",
          "relation_name": "analytics.pkg.orders_relation",
          "columns": {},
          "depends_on": {"nodes": [], "macros": []},
          "meta": {
            "nova": {
              "metrics": [
                {
                  "name": "relation_average_order_value",
                  "expression": "sum(amount) / nullif(count(distinct order_id), 0)"
                }
              ]
            }
          }
        },
        "analysis.pkg.revenue_note": {
          "name": "revenue_note",
          "resource_type": "analysis",
          "package_name": "pkg",
          "depends_on": {"nodes": [], "macros": []},
          "meta": {
            "nova": {
              "metrics": [
                {
                  "name": "metadata_only_revenue",
                  "description": "Revenue definition without a deterministic execution surface"
                }
              ]
            }
          }
        }
      },
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
    let gross_revenue = inventory_rows
        .iter()
        .find(|row| row["indicator_name"] == "gross_revenue" && row["indicator_type"] == "metric")
        .expect("metricflow metric should be inventoried");
    assert_eq!(gross_revenue["indicator_source"], "dbt_metric");
    assert_eq!(gross_revenue["execution_surface"], "semantic_layer");
    assert_eq!(gross_revenue["queryable"].as_bool(), Some(true));
    assert_eq!(gross_revenue["queryable_via"], "metricflow");
    assert!(
        gross_revenue["execution_note"]
            .as_str()
            .unwrap_or_default()
            .contains("MetricFlow"),
        "metricflow metric should explain the semantic-layer execution path: {inventory}"
    );

    let order_total = inventory_rows
        .iter()
        .find(|row| row["indicator_name"] == "order_total" && row["indicator_type"] == "measure")
        .expect("semantic model measure should be inventoried");
    assert_eq!(order_total["indicator_source"], "dbt_semantic_model");
    assert_eq!(order_total["execution_surface"], "semantic_layer");
    assert_eq!(order_total["queryable"].as_bool(), Some(true));
    assert_eq!(order_total["queryable_via"], "metricflow");

    let relation_metric = inventory_rows
        .iter()
        .find(|row| {
            row["indicator_name"] == "relation_average_order_value"
                && row["indicator_type"] == "metric"
        })
        .expect("relation-backed nova metric should be inventoried");
    assert_eq!(relation_metric["indicator_source"], "nova_meta");
    assert_eq!(relation_metric["execution_surface"], "relation");
    assert_eq!(relation_metric["queryable"].as_bool(), Some(true));
    assert_eq!(relation_metric["queryable_via"], "relation_name");
    assert_eq!(
        relation_metric["relation_name"],
        "analytics.pkg.orders_relation"
    );
    assert!(
        relation_metric.get("execution_note").is_none(),
        "relation-backed nova metric should not need an execution note: {inventory}"
    );

    let metadata_only_metric = inventory_rows
        .iter()
        .find(|row| row["indicator_name"] == "metadata_only_revenue")
        .expect("metadata-only nova metric should be inventoried");
    assert_eq!(metadata_only_metric["indicator_source"], "nova_meta");
    assert_eq!(metadata_only_metric["execution_surface"], "metadata_only");
    assert_eq!(metadata_only_metric["queryable"].as_bool(), Some(false));
    assert_eq!(metadata_only_metric["queryable_via"], "none");
    assert!(
        metadata_only_metric["execution_note"]
            .as_str()
            .unwrap_or_default()
            .contains("No deterministic relation"),
        "metadata-only rows should explain why they are not queryable: {inventory}"
    );

    let search = json(
        searcher
            .search_indicator(&SearchIndicatorParams {
                query: "gross revenue".to_string(),
                pagination: PaginationParams {
                    limit: Some(5),
                    offset: 0,
                },
                detail: Some(DetailLevel::Compact),
                ..Default::default()
            })
            .await,
    );
    let search_rows = search["data"].as_array().expect("search rows");
    let gross_revenue_search = search_rows
        .iter()
        .find(|row| row["indicator_name"] == "gross_revenue" && row["indicator_type"] == "metric")
        .expect("metricflow metric should be searchable");
    assert_eq!(gross_revenue_search["indicator_source"], "dbt_metric");
    assert_eq!(gross_revenue_search["execution_surface"], "semantic_layer");
    assert_eq!(gross_revenue_search["queryable"].as_bool(), Some(true));
    assert_eq!(gross_revenue_search["queryable_via"], "metricflow");

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

#[tokio::test(flavor = "multi_thread")]
async fn agent_modelling_findings_use_catalog_and_semantic_artifact_evidence() {
    let manifest = r#"{
      "metadata": {"dbt_version": "1.10.0"},
      "nodes": {
        "model.pkg.orders": {
          "name": "orders",
          "resource_type": "model",
          "package_name": "pkg",
          "relation_name": "analytics.pkg.orders",
          "columns": {
            "amount": {"name": "amount", "data_type": "text"},
            "declared_only": {"name": "declared_only", "data_type": "integer"},
            "order_date": {"name": "order_date", "data_type": "date"}
          },
          "depends_on": {"nodes": [], "macros": []},
          "meta": {
            "nova": {
              "grain": {"time_field": "order_date"},
              "measures": [
                {"name": "amount", "field": "amount"},
                {"name": "declared_only", "field": "declared_only"}
              ]
            }
          }
        },
        "model.pkg.catalog_shadow": {
          "name": "catalog_shadow",
          "resource_type": "model",
          "package_name": "pkg",
          "relation_name": "analytics.pkg.catalog_shadow",
          "columns": {},
          "depends_on": {"nodes": [], "macros": []}
        }
      },
      "sources": {},
      "macros": {},
      "docs": {},
      "groups": {},
      "exposures": {},
      "metrics": {
        "metric.pkg.broken_revenue": {
          "name": "broken_revenue",
          "resource_type": "metric",
          "package_name": "pkg",
          "type": "simple",
          "type_params": {"measure": {"name": "missing_measure"}},
          "meta": {"nova": {"metrics": [{"name": "overlay_metric"}]}}
        },
        "metric.pkg.clean_revenue": {
          "name": "clean_revenue",
          "resource_type": "metric",
          "package_name": "pkg",
          "type": "simple",
          "type_params": {"measure": {"name": "known_measure"}}
        }
      },
      "saved_queries": {},
      "semantic_models": {
        "semantic_model.pkg.clean_orders": {
          "name": "clean_orders",
          "resource_type": "semantic_model",
          "package_name": "pkg",
          "entities": [{"name": "order", "type": "primary", "expr": "order_id"}],
          "dimensions": [{"name": "ordered_at", "type": "time", "expr": "ordered_at"}],
          "measures": [{"name": "known_measure", "agg": "sum", "expr": "amount"}]
        }
      },
      "unit_tests": {}
    }"#;
    let catalog = serde_json::json!({
        "nodes": {
            "model.pkg.orders": {
                "columns": {
                    "amount": {"type": "numeric", "comment": "Warehouse amount"},
                    "catalog_only_revenue": {"type": "numeric", "comment": "Warehouse-only revenue"}
                }
            },
            "model.pkg.catalog_shadow": {
                "columns": {
                    "catalog_only_units": {"type": "integer", "comment": "Warehouse-only units"}
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

    let report = json(
        searcher
            .modelling_consistency_report(&ModellingConsistencyReportParams {
                resource_types: vec![],
                pagination: PaginationParams {
                    limit: Some(100),
                    offset: 0,
                },
                min_score: None,
            })
            .await,
    );
    let findings = report["data"]["agent_modelling_findings"]
        .as_array()
        .expect("agent modelling findings");
    let finding = |code: &str| {
        findings
            .iter()
            .find(|finding| finding["code"] == code)
            .unwrap_or_else(|| panic!("expected {code} in {findings:#?}"))
    };

    let type_drift = finding("catalog_type_drift_on_indicator_field");
    assert_eq!(type_drift["severity"].as_str(), Some("medium"));
    assert_eq!(type_drift["evidence"]["field"].as_str(), Some("amount"));
    assert_eq!(
        type_drift["evidence"]["manifest_data_type"].as_str(),
        Some("text")
    );
    assert_eq!(
        type_drift["evidence"]["catalog_data_type"].as_str(),
        Some("numeric")
    );

    let missing = finding("catalog_missing_indicator_field");
    assert_eq!(missing["severity"].as_str(), Some("high"));
    assert_eq!(missing["evidence"]["field"].as_str(), Some("declared_only"));

    let catalog_only = findings
        .iter()
        .find(|finding| {
            finding["code"] == "catalog_only_candidate_measure_column"
                && finding["evidence"]["column_name"] == "catalog_only_revenue"
        })
        .unwrap_or_else(|| panic!("expected catalog-only revenue finding in {findings:#?}"));
    assert_eq!(catalog_only["severity"].as_str(), Some("low"));
    assert_eq!(
        catalog_only["evidence"]["column_name"].as_str(),
        Some("catalog_only_revenue")
    );
    assert!(
        findings.iter().any(|finding| {
            finding["code"] == "catalog_only_candidate_measure_column"
                && finding["evidence"]["column_name"] == "catalog_only_units"
                && finding["entities"].as_array().is_some_and(|entities| {
                    entities
                        .iter()
                        .any(|entity| entity["unique_id"] == "model.pkg.catalog_shadow")
                })
        }),
        "catalog-only candidate should not require explicit meta.nova: {findings:#?}"
    );

    let unresolved = finding("semantic_metric_unresolved_measure_ref");
    assert_eq!(unresolved["severity"].as_str(), Some("high"));
    assert!(
        unresolved["evidence"]["missing_measure_refs"]
            .as_array()
            .is_some_and(|refs| refs.iter().any(|value| value == "missing_measure"))
    );
    assert!(
        !findings.iter().any(|finding| {
            finding["code"] == "semantic_metric_unresolved_measure_ref"
                && finding["entities"].as_array().is_some_and(|entities| {
                    entities
                        .iter()
                        .any(|entity| entity["unique_id"] == "metric.pkg.clean_revenue")
                })
        }),
        "clean semantic metric should not produce unresolved-reference findings: {findings:#?}"
    );

    let inventory = json(
        searcher
            .indicator_inventory(&IndicatorInventoryParams {
                resource_types: vec![],
                indicator_types: vec![],
                canonical_only: false,
                pagination: PaginationParams {
                    limit: Some(100),
                    offset: 0,
                },
            })
            .await,
    );
    let inventory_rows = inventory["data"].as_array().expect("inventory rows");
    assert!(
        inventory_rows.iter().any(|row| {
            row["parent_unique_id"] == "metric.pkg.broken_revenue"
                && row["indicator_name"] == "broken_revenue"
                && row["indicator_source"] == "dbt_metric"
        }),
        "MetricFlow-derived metric should survive explicit meta.nova overlay: {inventory}"
    );
    assert!(
        inventory_rows.iter().any(|row| {
            row["parent_unique_id"] == "metric.pkg.broken_revenue"
                && row["indicator_name"] == "overlay_metric"
        }),
        "explicit meta.nova overlay metric should extend the MetricFlow metric: {inventory}"
    );

    let metric_only_report = json(
        searcher
            .modelling_consistency_report(&ModellingConsistencyReportParams {
                resource_types: vec!["metric".to_string()],
                pagination: PaginationParams {
                    limit: Some(100),
                    offset: 0,
                },
                min_score: None,
            })
            .await,
    );
    let metric_only_findings = metric_only_report["data"]["agent_modelling_findings"]
        .as_array()
        .expect("metric-only agent modelling findings");
    assert!(
        metric_only_findings.iter().any(|finding| {
            finding["code"] == "semantic_metric_unresolved_measure_ref"
                && finding["entities"].as_array().is_some_and(|entities| {
                    entities
                        .iter()
                        .any(|entity| entity["unique_id"] == "metric.pkg.broken_revenue")
                })
        }),
        "metric-only report should still surface broken MetricFlow references: {metric_only_report}"
    );
    assert!(
        !metric_only_findings.iter().any(|finding| {
            finding["code"] == "semantic_metric_unresolved_measure_ref"
                && finding["entities"].as_array().is_some_and(|entities| {
                    entities
                        .iter()
                        .any(|entity| entity["unique_id"] == "metric.pkg.clean_revenue")
                })
        }),
        "metric-only report should resolve clean references against all semantic models: {metric_only_report}"
    );
}
