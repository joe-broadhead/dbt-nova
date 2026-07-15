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
use serde_json::Value as JsonValue;
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

fn assert_object_has_fields(value: &JsonValue, context: &str, fields: &[&str]) {
    let obj = value
        .as_object()
        .unwrap_or_else(|| panic!("{context} must be an object: {value:#?}"));
    for field in fields {
        assert!(
            obj.contains_key(*field),
            "{context} missing required field `{field}` in {value:#?}"
        );
    }
}

fn assert_non_empty_string(value: &JsonValue, context: &str, field: &str) {
    let text = value
        .get(field)
        .and_then(JsonValue::as_str)
        .unwrap_or_else(|| panic!("{context}.{field} must be a string: {value:#?}"));
    assert!(
        !text.trim().is_empty(),
        "{context}.{field} must not be empty"
    );
}

fn assert_indicator_execution_surface_contract(
    row: &JsonValue,
    expected_surface: &str,
    expected_queryable: bool,
    expected_direct_sql_queryable: bool,
    expected_queryable_via: &str,
) {
    assert_object_has_fields(
        row,
        "indicator execution surface",
        &[
            "indicator_name",
            "indicator_type",
            "indicator_source",
            "execution_surface",
            "queryable",
            "direct_sql_queryable",
            "queryable_via",
        ],
    );
    assert_eq!(row["execution_surface"], expected_surface);
    assert_eq!(row["queryable"].as_bool(), Some(expected_queryable));
    assert_eq!(
        row["direct_sql_queryable"].as_bool(),
        Some(expected_direct_sql_queryable)
    );
    assert_eq!(row["queryable_via"], expected_queryable_via);
}

fn assert_agent_modelling_finding_contract(finding: &JsonValue) {
    assert_object_has_fields(
        finding,
        "agent_modelling_findings[]",
        &[
            "code",
            "severity",
            "category",
            "message",
            "evidence",
            "recommendation",
        ],
    );
    assert_non_empty_string(finding, "agent_modelling_findings[]", "code");
    assert!(
        matches!(
            finding.get("severity").and_then(JsonValue::as_str),
            Some("blocker" | "high" | "medium" | "low")
        ),
        "finding severity must stay in the v1 enum: {finding:#?}"
    );
    assert_non_empty_string(finding, "agent_modelling_findings[]", "category");
    assert_non_empty_string(finding, "agent_modelling_findings[]", "message");
    assert!(
        finding.get("evidence").is_some_and(JsonValue::is_object),
        "finding evidence must be an object: {finding:#?}"
    );
    assert_non_empty_string(finding, "agent_modelling_findings[]", "recommendation");

    if let Some(entities) = finding.get("entities").and_then(JsonValue::as_array) {
        for entity in entities {
            assert_object_has_fields(
                entity,
                "agent_modelling_findings[].entities[]",
                &["unique_id", "name", "resource_type"],
            );
        }
    }
    if let Some(indicators) = finding.get("indicators").and_then(JsonValue::as_array) {
        for indicator in indicators {
            assert_object_has_fields(
                indicator,
                "agent_modelling_findings[].indicators[]",
                &["indicator_name", "indicator_type", "parent_unique_id"],
            );
        }
    }
    if let Some(hints) = finding
        .get("drill_down_hints")
        .and_then(JsonValue::as_array)
    {
        for hint in hints {
            assert_object_has_fields(
                hint,
                "agent_modelling_findings[].drill_down_hints[]",
                &["purpose", "tool", "arguments"],
            );
        }
    }
}

fn assert_agent_modelling_report_contract(report: &JsonValue) {
    assert_eq!(report["success"].as_bool(), Some(true), "{report:#?}");
    assert_eq!(report["count"].as_u64(), Some(1), "{report:#?}");
    let data = &report["data"];
    assert_object_has_fields(
        data,
        "modelling_consistency_report.data",
        &[
            "summary",
            "agent_modelling_schema_version",
            "entity_count",
            "applied_min_score",
            "overlap_candidates_total",
            "overlap_candidates_above_threshold",
            "overlap_candidate_count",
            "duplicate_indicator_count",
            "canonical_conflict_count",
            "multi_grain_entity_count",
            "agent_modelling_finding_count",
            "overlap_candidates",
            "overlap_candidates_total",
            "overlap_candidates_above_threshold",
            "duplicate_indicators",
            "canonical_indicator_conflicts",
            "entities_with_multiple_grain_variants",
            "agent_modelling_findings",
        ],
    );
    assert_eq!(data["agent_modelling_schema_version"], "agent_modelling.v1");
    assert_object_has_fields(
        &data["summary"],
        "modelling_consistency_report.data.summary",
        &[
            "section_counts",
            "agent_modelling",
            "page",
            "overlap_evidence_categories",
            "overlap_examples",
            "top_duplicate_indicator_groups",
            "top_canonical_conflicts",
            "top_multi_grain_entities",
            "drill_down_hints",
        ],
    );
    assert_object_has_fields(
        &data["summary"]["section_counts"],
        "modelling_consistency_report.data.summary.section_counts",
        &[
            "overlap_candidates",
            "duplicate_indicators",
            "canonical_indicator_conflicts",
            "entities_with_multiple_grain_variants",
            "agent_modelling_findings",
        ],
    );
    assert_object_has_fields(
        &data["summary"]["page"],
        "modelling_consistency_report.data.summary.page",
        &[
            "limit",
            "offset",
            "next_offset",
            "applied_min_score",
            "overlap_candidates_total",
            "overlap_candidates_above_threshold",
            "overlap_candidate_generation_truncated",
        ],
    );
    assert_object_has_fields(
        &data["summary"]["agent_modelling"],
        "modelling_consistency_report.data.summary.agent_modelling",
        &[
            "total",
            "blockers",
            "high",
            "medium",
            "low",
            "truncated",
            "top_codes",
            "top_categories",
        ],
    );
    let findings = data["agent_modelling_findings"]
        .as_array()
        .expect("agent_modelling_findings must be an array");
    assert_eq!(
        data["agent_modelling_finding_count"].as_u64(),
        u64::try_from(findings.len()).ok(),
        "finding count must match array length"
    );
    assert_eq!(
        data["summary"]["agent_modelling"]["total"].as_u64(),
        u64::try_from(findings.len()).ok(),
        "summary total must match array length"
    );
    for finding in findings {
        assert_agent_modelling_finding_contract(finding);
    }
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
        },
        "metric.pkg.revenue_per_session": {
          "name": "revenue_per_session",
          "resource_type": "metric",
          "package_name": "pkg",
          "label": "Revenue per session",
          "description": "Revenue divided by sessions",
          "type": "ratio",
          "type_params": {
            "numerator": {"name": "order_total"},
            "denominator": {"name": "sessions"}
          }
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
          "measures": [
            {"name": "order_total", "agg": "sum", "expr": "amount", "label": "Order total"},
            {"name": "sessions", "agg": "sum", "expr": "session_count", "label": "Sessions"}
          ]
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
    assert_indicator_execution_surface_contract(
        gross_revenue,
        "semantic_layer",
        true,
        false,
        "metricflow",
    );
    assert_eq!(gross_revenue["indicator_source"], "dbt_metric");
    assert_eq!(gross_revenue["execution_surface"], "semantic_layer");
    assert_eq!(gross_revenue["queryable"].as_bool(), Some(true));
    assert_eq!(gross_revenue["direct_sql_queryable"].as_bool(), Some(false));
    assert_eq!(gross_revenue["queryable_via"], "metricflow");
    assert!(
        gross_revenue["execution_note"]
            .as_str()
            .unwrap_or_default()
            .contains("MetricFlow"),
        "metricflow metric should explain the semantic-layer execution path: {inventory}"
    );

    let revenue_per_session = inventory_rows
        .iter()
        .find(|row| {
            row["indicator_name"] == "revenue_per_session" && row["indicator_type"] == "metric"
        })
        .expect("MetricFlow ratio metric should be inventoried");
    assert_indicator_execution_surface_contract(
        revenue_per_session,
        "semantic_layer",
        true,
        false,
        "metricflow",
    );
    assert_eq!(revenue_per_session["indicator_source"], "dbt_metric");
    assert_eq!(revenue_per_session["execution_surface"], "semantic_layer");
    assert_eq!(revenue_per_session["queryable"].as_bool(), Some(true));
    assert_eq!(
        revenue_per_session["direct_sql_queryable"].as_bool(),
        Some(false)
    );
    assert_eq!(revenue_per_session["queryable_via"], "metricflow");

    let order_total = inventory_rows
        .iter()
        .find(|row| row["indicator_name"] == "order_total" && row["indicator_type"] == "measure")
        .expect("semantic model measure should be inventoried");
    assert_indicator_execution_surface_contract(
        order_total,
        "semantic_layer",
        true,
        false,
        "metricflow",
    );
    assert_eq!(order_total["indicator_source"], "dbt_semantic_model");
    assert_eq!(order_total["execution_surface"], "semantic_layer");
    assert_eq!(order_total["queryable"].as_bool(), Some(true));
    assert_eq!(order_total["direct_sql_queryable"].as_bool(), Some(false));
    assert_eq!(order_total["queryable_via"], "metricflow");

    let relation_metric = inventory_rows
        .iter()
        .find(|row| {
            row["indicator_name"] == "relation_average_order_value"
                && row["indicator_type"] == "metric"
        })
        .expect("relation-backed nova metric should be inventoried");
    assert_indicator_execution_surface_contract(
        relation_metric,
        "relation",
        true,
        true,
        "relation_name",
    );
    assert_eq!(relation_metric["indicator_source"], "nova_meta");
    assert_eq!(relation_metric["execution_surface"], "relation");
    assert_eq!(relation_metric["queryable"].as_bool(), Some(true));
    assert_eq!(
        relation_metric["direct_sql_queryable"].as_bool(),
        Some(true)
    );
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
    assert_indicator_execution_surface_contract(
        metadata_only_metric,
        "metadata_only",
        false,
        false,
        "none",
    );
    assert_eq!(metadata_only_metric["indicator_source"], "nova_meta");
    assert_eq!(metadata_only_metric["execution_surface"], "metadata_only");
    assert_eq!(metadata_only_metric["queryable"].as_bool(), Some(false));
    assert_eq!(
        metadata_only_metric["direct_sql_queryable"].as_bool(),
        Some(false)
    );
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
    assert_indicator_execution_surface_contract(
        gross_revenue_search,
        "semantic_layer",
        true,
        false,
        "metricflow",
    );
    assert_eq!(gross_revenue_search["indicator_source"], "dbt_metric");
    assert_eq!(gross_revenue_search["execution_surface"], "semantic_layer");
    assert_eq!(gross_revenue_search["queryable"].as_bool(), Some(true));
    assert_eq!(
        gross_revenue_search["direct_sql_queryable"].as_bool(),
        Some(false)
    );
    assert_eq!(gross_revenue_search["queryable_via"], "metricflow");

    let ratio_search = json(
        searcher
            .search_indicator(&SearchIndicatorParams {
                query: "revenue per session".to_string(),
                pagination: PaginationParams {
                    limit: Some(5),
                    offset: 0,
                },
                detail: Some(DetailLevel::Compact),
                ..Default::default()
            })
            .await,
    );
    let ratio_search_rows = ratio_search["data"].as_array().expect("ratio search rows");
    let revenue_per_session_search = ratio_search_rows
        .iter()
        .find(|row| {
            row["indicator_name"] == "revenue_per_session" && row["indicator_type"] == "metric"
        })
        .expect("MetricFlow ratio metric should be searchable");
    assert_indicator_execution_surface_contract(
        revenue_per_session_search,
        "semantic_layer",
        true,
        false,
        "metricflow",
    );
    assert_eq!(revenue_per_session_search["indicator_source"], "dbt_metric");
    assert_eq!(
        revenue_per_session_search["execution_surface"],
        "semantic_layer"
    );
    assert_eq!(
        revenue_per_session_search["queryable"].as_bool(),
        Some(true)
    );
    assert_eq!(
        revenue_per_session_search["direct_sql_queryable"].as_bool(),
        Some(false)
    );
    assert_eq!(revenue_per_session_search["queryable_via"], "metricflow");

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
async fn catalog_backed_non_temporal_grain_time_field_emits_diagnostic() {
    let manifest = r#"{
      "metadata": {"dbt_version": "1.10.0"},
      "nodes": {
        "model.pkg.monthly_orders": {
          "name": "monthly_orders",
          "resource_type": "model",
          "package_name": "pkg",
          "columns": {
            "month": {"name": "month", "data_type": "integer"},
            "month_start": {"name": "month_start", "data_type": "date"},
            "country": {"name": "country", "data_type": "text"}
          },
          "depends_on": {"nodes": [], "macros": []},
          "meta": {
            "nova": {
              "grain": {
                "time_field": "month",
                "dimensions": ["country"]
              }
            }
          }
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
            "model.pkg.monthly_orders": {
                "columns": {
                    "month": {"type": "integer", "comment": "Calendar month number"},
                    "month_start": {"type": "date", "comment": "Month start date"},
                    "country": {"type": "varchar", "comment": "Country code"}
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

    let score = json(
        searcher
            .get_metadata_score(&GetMetadataScoreParams {
                id_or_name: Some("model.pkg.monthly_orders".to_string()),
                resource_type: Some("model".to_string()),
                scope: Some("entity".to_string()),
                include_breakdown: true,
                include_recommendations: false,
                ..GetMetadataScoreParams::default()
            })
            .await,
    );
    let diagnostics = score["data"]["diagnostics"]
        .as_array()
        .expect("diagnostics");
    let diagnostic = diagnostics
        .iter()
        .find(|item| item["code"] == "grain_time_field_not_temporal")
        .unwrap_or_else(|| panic!("missing grain time-field diagnostic: {score:#?}"));

    assert_eq!(
        diagnostic["field"].as_str(),
        Some("meta.nova.grain.time_field")
    );
    assert_eq!(diagnostic["column"].as_str(), Some("month"));
    assert_eq!(diagnostic["resolved_type"].as_str(), Some("integer"));
    assert_eq!(
        diagnostic["evidence_source"].as_str(),
        Some("dbt catalog column type")
    );
    assert!(
        diagnostic["hint"]
            .as_str()
            .is_some_and(|hint| hint.contains("integer month fields are dimensions")),
        "{diagnostic:#?}"
    );

    let manifest_only_temp = tempfile::tempdir().expect("manifest-only tempdir");
    let manifest_only_path = manifest_only_temp.path().join("nova_manifest.json");
    fs::write(&manifest_only_path, manifest).expect("write manifest-only manifest");
    let (manifest_only_searcher, _guard) = load_searcher(&manifest_only_path, None);
    let manifest_only_score = json(
        manifest_only_searcher
            .get_metadata_score(&GetMetadataScoreParams {
                id_or_name: Some("model.pkg.monthly_orders".to_string()),
                resource_type: Some("model".to_string()),
                scope: Some("entity".to_string()),
                include_breakdown: true,
                include_recommendations: false,
                ..GetMetadataScoreParams::default()
            })
            .await,
    );
    assert!(
        manifest_only_score["data"]["diagnostics"]
            .as_array()
            .is_some_and(|items| items
                .iter()
                .all(|item| item["code"] != "grain_time_field_not_temporal")),
        "manifest-only load must not emit catalog-backed diagnostic: {manifest_only_score:#?}"
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
    assert_agent_modelling_report_contract(&report);
    assert_eq!(report["data"]["applied_min_score"].as_f64(), Some(50.0));
    assert_eq!(
        report["data"]["summary"]["page"]["applied_min_score"].as_f64(),
        Some(50.0)
    );

    let exhaustive_report = json(
        searcher
            .modelling_consistency_report(&ModellingConsistencyReportParams {
                resource_types: vec![],
                pagination: PaginationParams {
                    limit: Some(100),
                    offset: 0,
                },
                min_score: Some(0.0),
            })
            .await,
    );
    assert_agent_modelling_report_contract(&exhaustive_report);
    assert_eq!(
        exhaustive_report["data"]["applied_min_score"].as_f64(),
        Some(0.0)
    );
    assert_eq!(
        exhaustive_report["data"]["overlap_candidate_count"],
        exhaustive_report["data"]["overlap_candidates_total"],
        "min_score=0 should restore exhaustive overlap rows"
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
    assert_agent_modelling_report_contract(&metric_only_report);
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
