//! Integration tests for column lineage resolution.
#[path = "support/config.rs"]
mod support_config;
#[path = "support/fixtures.rs"]
mod support_fixtures;
#[path = "support/json.rs"]
mod support_json;

use dbt_nova::params::{GetColumnLineageParams, GetImpactParams};
use support_fixtures::{FixtureSearchEnv, load_fixture, load_manifest_path};
use support_json::json;

fn load_column_reference_fixture() -> (tempfile::TempDir, FixtureSearchEnv) {
    let manifest = r#"{
      "metadata": {"dbt_version": "1.10.0"},
      "nodes": {
        "model.pkg.base_orders": {
          "unique_id": "model.pkg.base_orders",
          "name": "base_orders",
          "resource_type": "model",
          "package_name": "pkg",
          "original_file_path": "models/base_orders.sql",
          "columns": {
            "order_id": {"name": "order_id", "data_type": "integer"},
            "type_of_channel": {"name": "type_of_channel", "data_type": "string"}
          },
          "depends_on": {"nodes": [], "macros": []},
          "compiled_code": "select order_id, type_of_channel from raw.orders"
        },
        "model.pkg.sales_daily_channel": {
          "unique_id": "model.pkg.sales_daily_channel",
          "name": "sales_daily_channel",
          "resource_type": "model",
          "package_name": "pkg",
          "original_file_path": "models/sales_daily_channel.sql",
          "columns": {
            "type_of_channel": {"name": "type_of_channel", "data_type": "string"},
            "orders_count": {"name": "orders_count", "data_type": "integer"}
          },
          "meta": {
            "nova": {
              "grain": {"dimensions": ["type_of_channel"]},
              "measures": [
                {
                  "name": "channel_order_count",
                  "expression": "count(distinct case when type_of_channel is not null then order_id end)",
                  "field": "order_id"
                }
              ],
              "metrics": [
                {
                  "name": "paid_channel_order_share",
                  "expression": "sum(case when type_of_channel = 'paid' then orders_count else 0 end) / nullif(sum(orders_count), 0)"
                }
              ]
            }
          },
          "depends_on": {"nodes": ["model.pkg.base_orders"], "macros": []},
          "compiled_code": "select type_of_channel, count(distinct order_id) as orders_count from analytics.base_orders group by 1"
        },
        "model.pkg.sales_channel_rollup": {
          "unique_id": "model.pkg.sales_channel_rollup",
          "name": "sales_channel_rollup",
          "resource_type": "model",
          "package_name": "pkg",
          "original_file_path": "models/sales_channel_rollup.sql",
          "columns": {
            "type_of_channel": {"name": "type_of_channel", "data_type": "string"},
            "orders_count": {"name": "orders_count", "data_type": "integer"}
          },
          "depends_on": {"nodes": ["model.pkg.sales_daily_channel"], "macros": []},
          "compiled_code": "select type_of_channel, sum(orders_count) as orders_count from analytics.sales_daily_channel group by 1"
        },
        "test.pkg.accepted_values_sales_daily_channel_type_of_channel": {
          "unique_id": "test.pkg.accepted_values_sales_daily_channel_type_of_channel",
          "name": "accepted_values_sales_daily_channel_type_of_channel",
          "resource_type": "test",
          "package_name": "pkg",
          "original_file_path": "models/schema.yml",
          "column_name": "type_of_channel",
          "test_metadata": {"name": "accepted_values", "kwargs": {"column_name": "type_of_channel"}},
          "depends_on": {"nodes": ["model.pkg.sales_daily_channel"], "macros": []}
        },
        "analysis.pkg.channel_recipe": {
          "unique_id": "analysis.pkg.channel_recipe",
          "name": "channel_recipe",
          "resource_type": "analysis",
          "package_name": "pkg",
          "original_file_path": "analyses/recipes/channel_recipe/query.sql",
          "depends_on": {"nodes": ["model.pkg.sales_daily_channel"], "macros": []},
          "compiled_code": "select type_of_channel, sum(orders_count) as orders_count from analytics.sales_daily_channel group by 1"
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
        "model.pkg.sales_daily_channel": ["model.pkg.base_orders"],
        "model.pkg.sales_channel_rollup": ["model.pkg.sales_daily_channel"],
        "test.pkg.accepted_values_sales_daily_channel_type_of_channel": ["model.pkg.sales_daily_channel"],
        "analysis.pkg.channel_recipe": ["model.pkg.sales_daily_channel"]
      },
      "child_map": {
        "model.pkg.base_orders": ["model.pkg.sales_daily_channel"],
        "model.pkg.sales_daily_channel": [
          "model.pkg.sales_channel_rollup",
          "test.pkg.accepted_values_sales_daily_channel_type_of_channel",
          "analysis.pkg.channel_recipe"
        ],
        "model.pkg.sales_channel_rollup": [],
        "test.pkg.accepted_values_sales_daily_channel_type_of_channel": [],
        "analysis.pkg.channel_recipe": []
      }
    }"#;
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest_path = temp.path().join("column_reference_manifest.json");
    std::fs::write(&manifest_path, manifest).expect("write manifest");
    let searcher = load_manifest_path(&manifest_path).expect("load manifest");
    (temp, searcher)
}

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
        include_references: false,
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
async fn unresolved_aggregate_column_returns_definition_and_upstream_candidates() {
    let (_temp, searcher) = load_column_reference_fixture();
    let params = GetColumnLineageParams {
        id_or_name: "model.pkg.sales_daily_channel".into(),
        resource_type: None,
        column_name: "orders_count".into(),
        direction: "upstream".into(),
        depth: Some(1),
        confidence: Some("high".into()),
        include_references: false,
    };

    let result = json(searcher.get_column_lineage(&params).await);
    let data = result.get("data").expect("data");
    assert_eq!(
        data.get("lineage_status").and_then(|value| value.as_str()),
        Some("dependencies_unresolved_or_filtered")
    );
    assert_eq!(
        data.get("lineage")
            .and_then(|value| value.as_array())
            .map(Vec::len),
        Some(0)
    );
    assert_eq!(
        data.get("definition_source")
            .and_then(|value| value.as_str()),
        Some("compiled_sql")
    );
    assert_eq!(
        data.get("definition_confidence")
            .and_then(|value| value.as_str()),
        Some("exact")
    );
    assert_eq!(
        data.get("definition").and_then(|value| value.as_str()),
        Some("count(DISTINCT order_id)")
    );

    let referenced_columns = data
        .get("referenced_columns")
        .and_then(|value| value.as_array())
        .expect("referenced_columns");
    let order_id = referenced_columns
        .iter()
        .find(|item| item.get("name").and_then(|value| value.as_str()) == Some("order_id"))
        .expect("order_id reference");
    assert!(
        order_id
            .get("upstream_entities")
            .and_then(|value| value.as_array())
            .is_some_and(|entities| entities.iter().any(|entity| {
                entity.get("unique_id").and_then(|value| value.as_str())
                    == Some("model.pkg.base_orders")
                    && entity.get("column").and_then(|value| value.as_str()) == Some("order_id")
            }))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn column_references_include_metadata_tests_and_recipe_sql_when_requested() {
    let (_temp, searcher) = load_column_reference_fixture();
    let params = GetColumnLineageParams {
        id_or_name: "model.pkg.sales_daily_channel".into(),
        resource_type: None,
        column_name: "type_of_channel".into(),
        direction: "downstream".into(),
        depth: Some(2),
        confidence: Some("high".into()),
        include_references: true,
    };

    let result = json(searcher.get_column_lineage(&params).await);
    let data = result.get("data").expect("data");
    let references = data
        .get("references")
        .and_then(|value| value.as_array())
        .expect("references");
    for expected in [
        "nova_grain_dimension",
        "nova_measure_expression",
        "nova_metric_expression",
        "recipe_sql",
        "test",
    ] {
        assert!(
            references.iter().any(|reference| reference
                .get("kind")
                .and_then(|value| value.as_str())
                == Some(expected)),
            "missing {expected} in {references:#?}"
        );
    }
    assert!(
        references.iter().any(|reference| {
            reference.get("kind").and_then(|value| value.as_str()) == Some("recipe_sql")
                && reference
                    .get("detail")
                    .and_then(|detail| detail.get("match"))
                    .and_then(|value| value.as_str())
                    == Some("textual")
        }),
        "recipe SQL references should be labeled textual"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn impact_column_scopes_downstream_and_counts_references_by_kind() {
    let (_temp, searcher) = load_column_reference_fixture();
    let params = GetImpactParams {
        id_or_name: "model.pkg.sales_daily_channel".into(),
        column: Some("type_of_channel".into()),
    };

    let result = json(searcher.get_impact(&params).await);
    let data = result.get("data").expect("data");
    assert_eq!(
        data.get("column").and_then(|value| value.as_str()),
        Some("type_of_channel")
    );
    assert_eq!(
        data.get("downstream_count")
            .and_then(|value| value.as_u64()),
        Some(1)
    );
    assert_eq!(
        data.get("references_by_kind")
            .and_then(|counts| counts.get("recipe_sql"))
            .and_then(|value| value.as_u64()),
        Some(1)
    );
    assert_eq!(
        data.get("references_by_kind")
            .and_then(|counts| counts.get("test"))
            .and_then(|value| value.as_u64()),
        Some(1)
    );
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
        include_references: false,
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
        include_references: false,
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
                    include_references: false,
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
