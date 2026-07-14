//! Integration tests for get_context tool behavior.
use dbt_nova::params::ContextLimits;
use dbt_nova::params::{ContextMode, GetContextParams};
use dbt_nova::{DbtNovaConfig, ManifestSearch};
use serde_json::json;
use std::io::Write;
#[path = "support/config.rs"]
mod support_config;
#[path = "support/fixtures.rs"]
mod support_fixtures;

fn create_test_manifest() -> tempfile::NamedTempFile {
    let manifest = json!({
        "metadata": {
            "dbt_schema_version": "https://schemas.getdbt.com/dbt/manifest/v12.json",
            "project_name": "test_project"
        },
        "nodes": {
            "model.test.customers": {
                "name": "customers",
                "resource_type": "model",
                "package_name": "test",
                "description": "Customer dimension table",
                "database": "analytics",
                "schema": "dbt_test",
                "original_file_path": "models/staging/customers.sql",
                "raw_code": "select * from {{ ref('stg_customers') }}",
                "compiled_code": "select * from analytics.stg_customers",
                "tags": ["pii", "core"],
                "columns": {
                    "customer_id": {
                        "name": "customer_id",
                        "description": "Primary key",
                        "data_type": "integer",
                        "meta": {"primary_key": true}
                    },
                    "email": {
                        "name": "email",
                        "description": "Customer email",
                        "data_type": "varchar"
                    },
                    "created_at": {
                        "name": "created_at",
                        "data_type": "timestamp"
                    }
                }
            },
            "model.test.orders": {
                "name": "orders",
                "resource_type": "model",
                "package_name": "test",
                "description": "Orders fact table",
                "database": "analytics",
                "schema": "dbt_test",
                "original_file_path": "models/staging/orders.sql",
                "raw_code": "select * from {{ ref('stg_orders') }} join {{ ref('customers') }}",
                "tags": ["core"]
            },
            "test.test.unique_customers_customer_id": {
                "name": "unique_customers_customer_id",
                "resource_type": "test",
                "package_name": "test",
                "attached_node": "model.test.customers",
                "column_name": "customer_id",
                "test_metadata": {"name": "unique"}
            },
            "test.test.not_null_customers_customer_id": {
                "name": "not_null_customers_customer_id",
                "resource_type": "test",
                "package_name": "test",
                "attached_node": "model.test.customers",
                "column_name": "customer_id",
                "test_metadata": {"name": "not_null"}
            },
            "test.test.not_null_customers_email": {
                "name": "not_null_customers_email",
                "resource_type": "test",
                "package_name": "test",
                "attached_node": "model.test.customers",
                "column_name": "email",
                "test_metadata": {"name": "not_null"}
            }
        },
        "sources": {
            "source.test.raw.users": {
                "name": "users",
                "resource_type": "source",
                "package_name": "test",
                "source_name": "raw",
                "description": "Raw user data"
            }
        },
        "docs": {
            "doc.test.customers": {
                "name": "customers",
                "resource_type": "doc",
                "package_name": "test",
                "block_contents": "Documentation about the customers model"
            }
        },
        "macros": {},
        "groups": {},
        "exposures": {},
        "metrics": {},
        "parent_map": {
            "model.test.customers": ["source.test.raw.users"],
            "model.test.orders": ["model.test.customers"]
        },
        "child_map": {
            "source.test.raw.users": ["model.test.customers"],
            "model.test.customers": [
                "test.test.unique_customers_customer_id",
                "test.test.not_null_customers_customer_id",
                "test.test.not_null_customers_email",
                "model.test.orders"
            ]
        }
    });

    let mut file = tempfile::NamedTempFile::new().unwrap();
    file.write_all(manifest.to_string().as_bytes()).unwrap();
    file
}

fn create_searcher(manifest_file: &tempfile::NamedTempFile) -> support_fixtures::FixtureSearchEnv {
    support_fixtures::load_manifest_path(manifest_file.path())
        .expect("failed to build searcher from test manifest")
}

#[tokio::test(flavor = "multi_thread")]
async fn context_includes_entity_basics() {
    let manifest_file = create_test_manifest();
    let searcher = create_searcher(&manifest_file);

    let params = GetContextParams {
        id_or_name: "model.test.customers".to_string(),
        resource_type: None,
        include_columns: true,
        include_upstream: false,
        upstream_include_tests: false,
        include_downstream: false,
        downstream_include_tests: false,
        include_tests: false,
        include_docs: false,
        include_sql: false,
        context_mode: ContextMode::Standard,
        limits: ContextLimits {
            lineage_depth: 1,
            upstream_limit: 10,
            downstream_limit: 10,
        },
    };

    let result = searcher.get_context(&params).await.unwrap();
    let data = result.get("data").unwrap();

    assert_eq!(data["unique_id"], "model.test.customers");
    assert_eq!(data["entity"]["name"], "customers");
    assert_eq!(data["entity"]["resource_type"], "model");
    assert_eq!(data["entity"]["package_name"], "test");
    assert_eq!(data["entity"]["description"], "Customer dimension table");
}

#[tokio::test(flavor = "multi_thread")]
async fn context_includes_columns_with_tests() {
    let manifest_file = create_test_manifest();
    let searcher = create_searcher(&manifest_file);

    let params = GetContextParams {
        id_or_name: "model.test.customers".to_string(),
        resource_type: None,
        include_columns: true,
        include_upstream: false,
        upstream_include_tests: false,
        include_downstream: false,
        downstream_include_tests: false,
        include_tests: false,
        include_docs: false,
        include_sql: false,
        context_mode: ContextMode::Standard,
        limits: ContextLimits {
            lineage_depth: 1,
            upstream_limit: 10,
            downstream_limit: 10,
        },
    };

    let result = searcher.get_context(&params).await.unwrap();
    let columns = result["data"]["entity"]["columns"].as_array().unwrap();

    assert_eq!(columns.len(), 3);

    // Find customer_id column
    let customer_id_col = columns.iter().find(|c| c["name"] == "customer_id").unwrap();
    assert_eq!(customer_id_col["description"], "Primary key");
    assert_eq!(customer_id_col["data_type"], "integer");

    // Check that tests are associated with columns
    let tests = customer_id_col["tests"].as_array().unwrap();
    assert!(tests.contains(&json!("unique")));
    assert!(tests.contains(&json!("not_null")));
}

#[tokio::test(flavor = "multi_thread")]
async fn context_includes_upstream_lineage() {
    let manifest_file = create_test_manifest();
    let searcher = create_searcher(&manifest_file);

    let params = GetContextParams {
        id_or_name: "model.test.customers".to_string(),
        resource_type: None,
        include_columns: false,
        include_upstream: true,
        upstream_include_tests: false,
        include_downstream: false,
        downstream_include_tests: false,
        include_tests: false,
        include_docs: false,
        include_sql: false,
        context_mode: ContextMode::Standard,
        limits: ContextLimits {
            lineage_depth: 1,
            upstream_limit: 10,
            downstream_limit: 10,
        },
    };

    let result = searcher.get_context(&params).await.unwrap();
    let upstream = result["data"]["upstream"].as_object().unwrap();

    assert_eq!(upstream["count"], 1);
    assert_eq!(upstream["by_type"]["source"], 1);

    let entities = upstream["entities"].as_array().unwrap();
    assert_eq!(entities[0]["unique_id"], "source.test.raw.users");
}

#[tokio::test(flavor = "multi_thread")]
async fn context_includes_downstream_lineage() {
    let manifest_file = create_test_manifest();
    let searcher = create_searcher(&manifest_file);

    let params = GetContextParams {
        id_or_name: "model.test.customers".to_string(),
        resource_type: None,
        include_columns: false,
        include_upstream: false,
        upstream_include_tests: false,
        include_downstream: true,
        downstream_include_tests: false,
        include_tests: false,
        include_docs: false,
        include_sql: false,
        context_mode: ContextMode::Standard,
        limits: ContextLimits {
            lineage_depth: 1,
            upstream_limit: 10,
            downstream_limit: 1,
        },
    };

    let result = searcher.get_context(&params).await.unwrap();
    let downstream = result["data"]["downstream"].as_object().unwrap();

    assert_eq!(downstream["count"], 1);
    assert_eq!(downstream["by_type"]["model"], 1);
    assert_eq!(downstream["by_type"]["test"], 3);
    assert_eq!(downstream["tests_total"], 3);

    let entities = downstream["entities"].as_array().unwrap();
    assert_eq!(entities[0]["unique_id"], "model.test.orders");
}

#[tokio::test(flavor = "multi_thread")]
async fn context_can_include_test_nodes_in_downstream_lineage() {
    let manifest_file = create_test_manifest();
    let searcher = create_searcher(&manifest_file);

    let params = GetContextParams {
        id_or_name: "model.test.customers".to_string(),
        resource_type: None,
        include_columns: false,
        include_upstream: false,
        upstream_include_tests: false,
        include_downstream: true,
        downstream_include_tests: true,
        include_tests: false,
        include_docs: false,
        include_sql: false,
        context_mode: ContextMode::Standard,
        limits: ContextLimits {
            lineage_depth: 1,
            upstream_limit: 10,
            downstream_limit: 10,
        },
    };

    let result = searcher.get_context(&params).await.unwrap();
    let downstream = result["data"]["downstream"].as_object().unwrap();

    assert_eq!(downstream["count"], 4);
    assert_eq!(downstream["by_type"]["model"], 1);
    assert_eq!(downstream["by_type"]["test"], 3);
    assert!(downstream.get("tests_total").is_none());

    let entity_ids: Vec<&str> = downstream["entities"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|entity| entity["unique_id"].as_str())
        .collect();
    assert!(entity_ids.contains(&"test.test.unique_customers_customer_id"));
    assert!(entity_ids.contains(&"model.test.orders"));
}

#[tokio::test(flavor = "multi_thread")]
async fn context_includes_test_coverage() {
    let manifest_file = create_test_manifest();
    let searcher = create_searcher(&manifest_file);

    let params = GetContextParams {
        id_or_name: "model.test.customers".to_string(),
        resource_type: None,
        include_columns: false,
        include_upstream: false,
        upstream_include_tests: false,
        include_downstream: false,
        downstream_include_tests: false,
        include_tests: true,
        include_docs: false,
        include_sql: false,
        context_mode: ContextMode::Standard,
        limits: ContextLimits {
            lineage_depth: 1,
            upstream_limit: 10,
            downstream_limit: 10,
        },
    };

    let result = searcher.get_context(&params).await.unwrap();
    let tests = result["data"]["tests"].as_object().unwrap();

    assert_eq!(tests["total_tests"], 3);
    assert_eq!(tests["schema_tests"], 3);
    assert_eq!(tests["data_tests"], 0);

    // Check test types
    let by_type = tests["by_type"].as_object().unwrap();
    assert_eq!(by_type["unique"], 1);
    assert_eq!(by_type["not_null"], 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn context_includes_related_docs() {
    let manifest_file = create_test_manifest();
    let searcher = create_searcher(&manifest_file);

    let params = GetContextParams {
        id_or_name: "model.test.customers".to_string(),
        resource_type: None,
        include_columns: false,
        include_upstream: false,
        upstream_include_tests: false,
        include_downstream: false,
        downstream_include_tests: false,
        include_tests: false,
        include_docs: true,
        include_sql: false,
        context_mode: ContextMode::Standard,
        limits: ContextLimits {
            lineage_depth: 1,
            upstream_limit: 10,
            downstream_limit: 10,
        },
    };

    let result = searcher.get_context(&params).await.unwrap();
    let docs = result["data"]["docs"].as_array().unwrap();

    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0]["name"], "customers");
    assert_eq!(docs[0]["relation"], "same_name");
}

#[tokio::test(flavor = "multi_thread")]
async fn context_resolves_by_name() {
    let manifest_file = create_test_manifest();
    let searcher = create_searcher(&manifest_file);

    let params = GetContextParams {
        id_or_name: "customers".to_string(),
        resource_type: Some("model".to_string()),
        include_columns: false,
        include_upstream: false,
        upstream_include_tests: false,
        include_downstream: false,
        downstream_include_tests: false,
        include_tests: false,
        include_docs: false,
        include_sql: false,
        context_mode: ContextMode::Standard,
        limits: ContextLimits {
            lineage_depth: 1,
            upstream_limit: 10,
            downstream_limit: 10,
        },
    };

    let result = searcher.get_context(&params).await.unwrap();
    assert_eq!(result["data"]["unique_id"], "model.test.customers");
}

#[tokio::test(flavor = "multi_thread")]
async fn context_entity_not_found() {
    let manifest_file = create_test_manifest();
    let searcher = create_searcher(&manifest_file);

    let params = GetContextParams {
        id_or_name: "nonexistent".to_string(),
        resource_type: None,
        include_columns: false,
        include_upstream: false,
        upstream_include_tests: false,
        include_downstream: false,
        downstream_include_tests: false,
        include_tests: false,
        include_docs: false,
        include_sql: false,
        context_mode: ContextMode::Standard,
        limits: ContextLimits {
            lineage_depth: 1,
            upstream_limit: 10,
            downstream_limit: 10,
        },
    };

    let result = searcher.get_context(&params).await;
    assert!(result.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn context_respects_lineage_depth() {
    let manifest_file = create_test_manifest();
    let searcher = create_searcher(&manifest_file);

    // With depth 1, orders should see customers but not source
    let params = GetContextParams {
        id_or_name: "model.test.orders".to_string(),
        resource_type: None,
        include_columns: false,
        include_upstream: true,
        upstream_include_tests: false,
        include_downstream: false,
        downstream_include_tests: false,
        include_tests: false,
        include_docs: false,
        include_sql: false,
        context_mode: ContextMode::Standard,
        limits: ContextLimits {
            lineage_depth: 1,
            upstream_limit: 10,
            downstream_limit: 10,
        },
    };

    let result = searcher.get_context(&params).await.unwrap();
    let upstream = result["data"]["upstream"].as_object().unwrap();

    // Only direct parent (customers) should be included
    assert_eq!(upstream["count"], 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn context_respects_lineage_limit() {
    let manifest_file = create_test_manifest();
    let searcher = create_searcher(&manifest_file);

    let params = GetContextParams {
        id_or_name: "source.test.raw.users".to_string(),
        resource_type: None,
        include_columns: false,
        include_upstream: false,
        upstream_include_tests: false,
        include_downstream: true,
        downstream_include_tests: false,
        include_tests: false,
        include_docs: false,
        include_sql: false,
        context_mode: ContextMode::Standard,
        limits: ContextLimits {
            lineage_depth: 10, // High depth
            upstream_limit: 10,
            downstream_limit: 1, // But only 1 result
        },
    };

    let result = searcher.get_context(&params).await.unwrap();
    let downstream = result["data"]["downstream"].as_object().unwrap();

    // Only 1 entity even though there are more downstream
    assert_eq!(downstream["count"], 1);
    assert!(downstream["truncated"].as_bool().unwrap());
}

#[tokio::test(flavor = "multi_thread")]
async fn context_lineage_limit_clamps_to_config_max_results() {
    let manifest = json!({
        "metadata": {"dbt_version": "1.10.0"},
        "nodes": {
            "model.test.root": {
                "name": "root",
                "resource_type": "model",
                "package_name": "test"
            },
            "model.test.child_a": {
                "name": "child_a",
                "resource_type": "model",
                "package_name": "test"
            },
            "model.test.child_b": {
                "name": "child_b",
                "resource_type": "model",
                "package_name": "test"
            },
            "model.test.child_c": {
                "name": "child_c",
                "resource_type": "model",
                "package_name": "test"
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
            "model.test.child_a": ["model.test.root"],
            "model.test.child_b": ["model.test.root"],
            "model.test.child_c": ["model.test.root"]
        },
        "child_map": {
            "model.test.root": [
                "model.test.child_a",
                "model.test.child_b",
                "model.test.child_c"
            ]
        }
    });
    let mut file = tempfile::NamedTempFile::new().unwrap();
    file.write_all(manifest.to_string().as_bytes()).unwrap();

    let guard = support_config::TestStorageGuard::new();
    let mut cfg = DbtNovaConfig {
        manifest_path: file.path().to_string_lossy().to_string(),
        lineage_max_results: 2,
        search: support_config::test_search_config(),
        ..Default::default()
    };
    support_config::apply_test_storage(&mut cfg, &guard);
    let searcher = ManifestSearch::new(cfg)
        .expect("synthetic manifest should load")
        .search;

    let result = searcher
        .get_context(&GetContextParams {
            id_or_name: "model.test.root".to_string(),
            resource_type: None,
            include_columns: false,
            include_upstream: false,
            upstream_include_tests: false,
            include_downstream: true,
            downstream_include_tests: false,
            include_tests: false,
            include_docs: false,
            include_sql: false,
            context_mode: ContextMode::Standard,
            limits: ContextLimits {
                lineage_depth: 1,
                upstream_limit: 100,
                downstream_limit: 100,
            },
        })
        .await
        .unwrap();
    let downstream = result["data"]["downstream"].as_object().unwrap();

    assert_eq!(downstream["count"], 2);
    assert_eq!(downstream["by_type"]["model"], 2);
    assert_eq!(downstream["truncated"].as_bool(), Some(true));
}

#[tokio::test(flavor = "multi_thread")]
async fn context_full_aggregation() {
    let manifest_file = create_test_manifest();
    let searcher = create_searcher(&manifest_file);

    let params = GetContextParams {
        id_or_name: "model.test.customers".to_string(),
        resource_type: None,
        include_columns: true,
        include_upstream: true,
        upstream_include_tests: false,
        include_downstream: true,
        downstream_include_tests: false,
        include_tests: true,
        include_docs: true,
        include_sql: false,
        context_mode: ContextMode::Standard,
        limits: ContextLimits {
            lineage_depth: 1,
            upstream_limit: 10,
            downstream_limit: 10,
        },
    };

    let result = searcher.get_context(&params).await.unwrap();
    let data = result["data"].as_object().unwrap();

    // All sections should be present
    assert!(data.contains_key("entity"));
    assert!(data.contains_key("upstream"));
    assert!(data.contains_key("downstream"));
    assert!(data.contains_key("tests"));
    assert!(data.contains_key("docs"));

    // Entity should have columns
    assert!(data["entity"]["columns"].is_array());
}
