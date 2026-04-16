//! Integration tests for Tantivy search behavior and ranking.
use dbt_nova::params::PaginationParams;
use dbt_nova::params::{DetailLevel, SearchParams};
use dbt_nova::{DbtNovaConfig, ManifestSearch};
use serde_json::json;
use std::io::Write;
#[path = "support/config.rs"]
mod support_config;
#[path = "support/fixtures.rs"]
mod support_fixtures;

fn create_test_manifest() -> tempfile::NamedTempFile {
    // Minimal manifest fixture that exercises name/alias/description/tags/SQL/doc searches.
    let manifest = json!({
        "metadata": {
            "dbt_schema_version": "https://schemas.getdbt.com/dbt/manifest/v12.json",
            "project_name": "test_project"
        },
        "nodes": {
            "model.test.customers": {
                "name": "customers",
                "alias": "dim_customers",
                "resource_type": "model",
                "package_name": "test",
                "description": "Customer dimension table with order history",
                "original_file_path": "models/marts/customers.sql",
                "raw_code": "select customer_id, email from stg_customers",
                "tags": ["core", "pii"],
                "columns": {
                    "customer_id": {"name": "customer_id", "description": "PK"},
                    "email": {"name": "email"}
                }
            },
            "model.test.orders": {
                "name": "orders",
                "alias": "fct_orders",
                "resource_type": "model",
                "package_name": "test",
                "description": "Orders fact table",
                "original_file_path": "models/marts/orders.sql",
                "raw_code": "select order_id, customer_id from stg_orders",
                "tags": ["core"]
            },
            "model.test.stg_customers": {
                "name": "stg_customers",
                "resource_type": "model",
                "package_name": "test",
                "description": "Staged customer data",
                "original_file_path": "models/staging/stg_customers.sql",
                "raw_code": "select * from raw.customers"
            },
            "model.test.stg_orders": {
                "name": "stg_orders",
                "resource_type": "model",
                "package_name": "test",
                "description": "Staged order data",
                "original_file_path": "models/staging/stg_orders.sql",
                "raw_code": "select * from raw.orders"
            },
            "test.test.unique_customers": {
                "name": "unique_customers",
                "resource_type": "test",
                "package_name": "test"
            }
        },
        "sources": {
            "source.test.raw.customers": {
                "name": "customers",
                "resource_type": "source",
                "package_name": "test",
                "source_name": "raw"
            }
        },
        "macros": {
            "macro.test.generate_schema_name": {
                "name": "generate_schema_name",
                "resource_type": "macro",
                "package_name": "test",
                "macro_sql": "{% macro generate_schema_name() %}"
            }
        },
        "docs": {
            "doc.test.__overview__": {
                "name": "__overview__",
                "resource_type": "doc",
                "package_name": "test",
                "block_contents": "Project overview documentation"
            }
        },
        "groups": {},
        "exposures": {},
        "metrics": {},
        "parent_map": {},
        "child_map": {}
    });

    let mut file = tempfile::NamedTempFile::new().unwrap();
    file.write_all(manifest.to_string().as_bytes()).unwrap();
    file
}

fn create_searcher(manifest_file: &tempfile::NamedTempFile) -> support_fixtures::FixtureSearchEnv {
    // Use a temporary storage root to avoid polluting local state during tests.
    support_fixtures::load_manifest_path(manifest_file.path())
        .expect("failed to build searcher from test manifest")
}

#[tokio::test(flavor = "multi_thread")]
async fn tantivy_search_by_name() {
    let manifest_file = create_test_manifest();
    let searcher = create_searcher(&manifest_file);

    let params = SearchParams {
        query: "customers".to_string(),
        resource_types: vec![],
        persona: None,
        detail: DetailLevel::Full,
        min_score: None,
        fuzzy: false,
        include_highlights: false,
        include_sql: false,
        explain: false,
        pagination: PaginationParams {
            limit: 10,
            offset: 0,
        },
    };

    let result = searcher.search(&params).await.unwrap();
    let data = result["data"].as_array().unwrap();

    // Should find multiple entities with "customers" in name.
    assert!(data.len() >= 2);

    // Results should have score field for ranking.
    assert!(data[0]["score"].is_number());
}

#[tokio::test(flavor = "multi_thread")]
async fn tantivy_search_by_alias() {
    let manifest_file = create_test_manifest();
    let searcher = create_searcher(&manifest_file);

    let params = SearchParams {
        query: "dim_customers".to_string(),
        resource_types: vec![],
        persona: None,
        detail: DetailLevel::Full,
        min_score: None,
        fuzzy: false,
        include_highlights: false,
        include_sql: false,
        explain: false,
        pagination: PaginationParams {
            limit: 10,
            offset: 0,
        },
    };

    let result = searcher.search(&params).await.unwrap();
    let data = result["data"].as_array().unwrap();

    assert!(!data.is_empty());
    // The customer model with alias should be found.
    let found = data
        .iter()
        .any(|e| e["unique_id"] == "model.test.customers");
    assert!(found, "Should find model by alias");
}

#[tokio::test(flavor = "multi_thread")]
async fn tantivy_search_by_description() {
    let manifest_file = create_test_manifest();
    let searcher = create_searcher(&manifest_file);

    let params = SearchParams {
        query: "order history".to_string(),
        resource_types: vec![],
        persona: None,
        detail: DetailLevel::Full,
        min_score: None,
        fuzzy: false,
        include_highlights: false,
        include_sql: false,
        explain: false,
        pagination: PaginationParams {
            limit: 10,
            offset: 0,
        },
    };

    let result = searcher.search(&params).await.unwrap();
    let data = result["data"].as_array().unwrap();

    assert!(!data.is_empty());
    // Should find customer model via description.
    let found = data
        .iter()
        .any(|e| e["unique_id"] == "model.test.customers");
    assert!(found, "Should find model by description term");
}

#[tokio::test(flavor = "multi_thread")]
async fn tantivy_search_by_column_name() {
    let manifest_file = create_test_manifest();
    let searcher = create_searcher(&manifest_file);

    let params = SearchParams {
        query: "email".to_string(),
        resource_types: vec![],
        persona: None,
        detail: DetailLevel::Full,
        min_score: None,
        fuzzy: false,
        include_highlights: false,
        include_sql: false,
        explain: false,
        pagination: PaginationParams {
            limit: 10,
            offset: 0,
        },
    };

    let result = searcher.search(&params).await.unwrap();
    let data = result["data"].as_array().unwrap();

    assert!(!data.is_empty());
    // Should find customer model via column name.
    let found = data
        .iter()
        .any(|e| e["unique_id"] == "model.test.customers");
    assert!(found, "Should find model by column name");
}

#[tokio::test(flavor = "multi_thread")]
async fn tantivy_search_by_tags() {
    let manifest_file = create_test_manifest();
    let searcher = create_searcher(&manifest_file);

    let params = SearchParams {
        query: "pii".to_string(),
        resource_types: vec![],
        persona: None,
        detail: DetailLevel::Full,
        min_score: None,
        fuzzy: false,
        include_highlights: false,
        include_sql: false,
        explain: false,
        pagination: PaginationParams {
            limit: 10,
            offset: 0,
        },
    };

    let result = searcher.search(&params).await.unwrap();
    let data = result["data"].as_array().unwrap();

    assert!(!data.is_empty());
    // Should find entities tagged with "pii".
    let found = data
        .iter()
        .any(|e| e["unique_id"] == "model.test.customers");
    assert!(found, "Should find model by tag");
}

#[tokio::test(flavor = "multi_thread")]
async fn tantivy_search_by_file_path() {
    let manifest_file = create_test_manifest();
    let searcher = create_searcher(&manifest_file);

    let params = SearchParams {
        query: "staging".to_string(),
        resource_types: vec![],
        persona: None,
        detail: DetailLevel::Full,
        min_score: None,
        fuzzy: false,
        include_highlights: false,
        include_sql: false,
        explain: false,
        pagination: PaginationParams {
            limit: 10,
            offset: 0,
        },
    };

    let result = searcher.search(&params).await.unwrap();
    let data = result["data"].as_array().unwrap();

    assert!(data.len() >= 2);
    // Should find staging models by file path.
    let stg_customers = data
        .iter()
        .any(|e| e["unique_id"] == "model.test.stg_customers");
    let stg_orders = data
        .iter()
        .any(|e| e["unique_id"] == "model.test.stg_orders");
    assert!(stg_customers, "Should find stg_customers by path");
    assert!(stg_orders, "Should find stg_orders by path");
}

#[tokio::test(flavor = "multi_thread")]
async fn tantivy_search_by_sql_code() {
    let manifest_file = create_test_manifest();
    let searcher = create_searcher(&manifest_file);

    let params = SearchParams {
        query: "select".to_string(),
        resource_types: vec!["model".to_string()],
        persona: None,
        detail: DetailLevel::Full,
        min_score: None,
        fuzzy: false,
        include_highlights: false,
        include_sql: false,
        explain: false,
        pagination: PaginationParams {
            limit: 10,
            offset: 0,
        },
    };

    let result = searcher.search(&params).await.unwrap();
    let data = result["data"].as_array().unwrap();

    // Should find models with "select" in SQL.
    assert!(data.len() >= 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn tantivy_search_filter_by_resource_type() {
    let manifest_file = create_test_manifest();
    let searcher = create_searcher(&manifest_file);

    let params = SearchParams {
        query: "customers".to_string(),
        resource_types: vec!["source".to_string()],
        persona: None,
        detail: DetailLevel::Full,
        min_score: None,
        fuzzy: false,
        include_highlights: false,
        include_sql: false,
        explain: false,
        pagination: PaginationParams {
            limit: 10,
            offset: 0,
        },
    };

    let result = searcher.search(&params).await.unwrap();
    let data = result["data"].as_array().unwrap();

    // Should only find sources.
    for entity in data {
        assert_eq!(entity["resource_type"], "source");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn tantivy_search_multi_word_query() {
    let manifest_file = create_test_manifest();
    let searcher = create_searcher(&manifest_file);

    let params = SearchParams {
        query: "customer order".to_string(),
        resource_types: vec![],
        persona: None,
        detail: DetailLevel::Full,
        min_score: None,
        fuzzy: false,
        include_highlights: false,
        include_sql: false,
        explain: false,
        pagination: PaginationParams {
            limit: 10,
            offset: 0,
        },
    };

    let result = searcher.search(&params).await.unwrap();
    let data = result["data"].as_array().unwrap();

    // Should find entities matching at least one term.
    // The customers model has "order" in description.
    assert!(!data.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn tantivy_search_or_query() {
    let manifest_file = create_test_manifest();
    let searcher = create_searcher(&manifest_file);

    let params = SearchParams {
        query: "customers OR orders".to_string(),
        resource_types: vec![],
        persona: None,
        detail: DetailLevel::Full,
        min_score: None,
        fuzzy: false,
        include_highlights: false,
        include_sql: false,
        explain: false,
        pagination: PaginationParams {
            limit: 10,
            offset: 0,
        },
    };

    let result = searcher.search(&params).await.unwrap();
    let data = result["data"].as_array().unwrap();

    let names: Vec<String> = data
        .iter()
        .filter_map(|e| e.get("name").and_then(|v| v.as_str()).map(String::from))
        .collect();

    assert!(names.contains(&"customers".to_string()));
    assert!(names.contains(&"orders".to_string()));
}

#[tokio::test(flavor = "multi_thread")]
async fn tantivy_search_phrase_query() {
    let manifest_file = create_test_manifest();
    let searcher = create_searcher(&manifest_file);

    let params = SearchParams {
        query: "\"Customer dimension\"".to_string(),
        resource_types: vec![],
        persona: None,
        detail: DetailLevel::Full,
        min_score: None,
        fuzzy: false,
        include_highlights: false,
        include_sql: false,
        explain: false,
        pagination: PaginationParams {
            limit: 10,
            offset: 0,
        },
    };

    let result = searcher.search(&params).await.unwrap();
    let data = result["data"].as_array().unwrap();

    let names: Vec<String> = data
        .iter()
        .filter_map(|e| e.get("name").and_then(|v| v.as_str()).map(String::from))
        .collect();

    assert!(names.contains(&"customers".to_string()));
}

#[tokio::test(flavor = "multi_thread")]
async fn tantivy_search_field_query() {
    let manifest_file = create_test_manifest();
    let searcher = create_searcher(&manifest_file);

    let params = SearchParams {
        query: "name:orders".to_string(),
        resource_types: vec![],
        persona: None,
        detail: DetailLevel::Full,
        min_score: None,
        fuzzy: false,
        include_highlights: false,
        include_sql: false,
        explain: false,
        pagination: PaginationParams {
            limit: 10,
            offset: 0,
        },
    };

    let result = searcher.search(&params).await.unwrap();
    let data = result["data"].as_array().unwrap();

    let names: Vec<String> = data
        .iter()
        .filter_map(|e| e.get("name").and_then(|v| v.as_str()).map(String::from))
        .collect();

    assert!(names.contains(&"orders".to_string()));
}

#[tokio::test(flavor = "multi_thread")]
async fn tantivy_search_prefix_wildcard() {
    let manifest_file = create_test_manifest();
    let searcher = create_searcher(&manifest_file);

    let params = SearchParams {
        query: "stg_*".to_string(),
        resource_types: vec![],
        persona: None,
        detail: DetailLevel::Full,
        min_score: None,
        fuzzy: false,
        include_highlights: false,
        include_sql: false,
        explain: false,
        pagination: PaginationParams {
            limit: 10,
            offset: 0,
        },
    };

    let result = searcher.search(&params).await.unwrap();
    let data = result["data"].as_array().unwrap();

    let names: Vec<String> = data
        .iter()
        .filter_map(|e| e.get("name").and_then(|v| v.as_str()).map(String::from))
        .collect();

    assert!(names.contains(&"stg_customers".to_string()));
    assert!(names.contains(&"stg_orders".to_string()));
}

#[tokio::test(flavor = "multi_thread")]
async fn tantivy_search_stemming() {
    let manifest_file = create_test_manifest();
    let searcher = create_searcher(&manifest_file);

    let params = SearchParams {
        query: "tables".to_string(),
        resource_types: vec![],
        persona: None,
        detail: DetailLevel::Full,
        min_score: None,
        fuzzy: false,
        include_highlights: false,
        include_sql: false,
        explain: false,
        pagination: PaginationParams {
            limit: 10,
            offset: 0,
        },
    };

    let result = searcher.search(&params).await.unwrap();
    let data = result["data"].as_array().unwrap();

    assert!(!data.is_empty(), "Expected stemmed match for 'tables'");
}

#[tokio::test(flavor = "multi_thread")]
async fn tantivy_search_ngram_partial() {
    let manifest_file = create_test_manifest();
    let searcher = create_searcher(&manifest_file);

    let params = SearchParams {
        query: "cust".to_string(),
        resource_types: vec![],
        persona: None,
        detail: DetailLevel::Full,
        min_score: None,
        fuzzy: false,
        include_highlights: false,
        include_sql: false,
        explain: false,
        pagination: PaginationParams {
            limit: 10,
            offset: 0,
        },
    };

    let result = searcher.search(&params).await.unwrap();
    let data = result["data"].as_array().unwrap();

    let names: Vec<String> = data
        .iter()
        .filter_map(|e| e.get("name").and_then(|v| v.as_str()).map(String::from))
        .collect();

    assert!(names.contains(&"customers".to_string()));
}

#[tokio::test(flavor = "multi_thread")]
async fn tantivy_search_with_min_score() {
    let manifest_file = create_test_manifest();
    let guard = support_config::TestStorageGuard::new();
    let mut cfg = DbtNovaConfig {
        manifest_path: manifest_file.path().to_string_lossy().to_string(),
        search: support_config::test_search_config(),
        ..Default::default()
    };
    support_config::apply_test_storage(&mut cfg, &guard);
    cfg.search.enable_rrf = false;
    let searcher = ManifestSearch::new(cfg).unwrap().search;

    let params = SearchParams {
        query: "customers".to_string(),
        resource_types: vec![],
        persona: None,
        detail: DetailLevel::Full,
        min_score: Some(0.5),
        fuzzy: false,
        include_highlights: false,
        include_sql: false,
        explain: false,
        pagination: PaginationParams {
            limit: 10,
            offset: 0,
        },
    };

    let result = searcher.search(&params).await.unwrap();
    let data = result["data"].as_array().unwrap();

    // All results should have score >= 0.5.
    for entity in data {
        let score = entity["score"].as_f64().unwrap();
        assert!(score >= 0.5, "Score {} should be >= 0.5", score);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn tantivy_search_respects_limit() {
    let manifest_file = create_test_manifest();
    let searcher = create_searcher(&manifest_file);

    let params = SearchParams {
        query: "customers".to_string(),
        resource_types: vec![],
        persona: None,
        detail: DetailLevel::Full,
        min_score: None,
        fuzzy: false,
        include_highlights: false,
        include_sql: false,
        explain: false,
        pagination: PaginationParams {
            limit: 1,
            offset: 0,
        },
    };

    let result = searcher.search(&params).await.unwrap();
    let data = result["data"].as_array().unwrap();

    assert!(data.len() <= 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn tantivy_search_results_sorted_by_score() {
    let manifest_file = create_test_manifest();
    let searcher = create_searcher(&manifest_file);

    let params = SearchParams {
        query: "customers".to_string(),
        resource_types: vec![],
        persona: None,
        detail: DetailLevel::Full,
        min_score: None,
        fuzzy: false,
        include_highlights: false,
        include_sql: false,
        explain: false,
        pagination: PaginationParams {
            limit: 10,
            offset: 0,
        },
    };

    let result = searcher.search(&params).await.unwrap();
    let data = result["data"].as_array().unwrap();

    // Verify scores are in descending order.
    let scores: Vec<f64> = data.iter().map(|e| e["score"].as_f64().unwrap()).collect();

    for i in 1..scores.len() {
        assert!(
            scores[i - 1] >= scores[i],
            "Scores should be in descending order"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn tantivy_search_include_full_false() {
    let manifest_file = create_test_manifest();
    let searcher = create_searcher(&manifest_file);

    let params = SearchParams {
        query: "customers".to_string(),
        resource_types: vec![],
        persona: None,
        detail: DetailLevel::Standard,
        min_score: None,
        fuzzy: false,
        include_highlights: false,
        include_sql: false,
        explain: false,
        pagination: PaginationParams {
            limit: 10,
            offset: 0,
        },
    };

    let result = searcher.search(&params).await.unwrap();
    let data = result["data"].as_array().unwrap();

    assert!(!data.is_empty());

    // With include_full=false, should return summary only.
    let first = &data[0];
    assert!(first.get("unique_id").is_some());
    assert!(first.get("name").is_some());
    assert!(first.get("resource_type").is_some());
    assert!(first.get("score").is_some());

    // Should not have full entity fields like raw_code
    // (unless they happen to be in the summary).
}

#[tokio::test(flavor = "multi_thread")]
async fn tantivy_searchhighlights() {
    let manifest_file = create_test_manifest();
    let searcher = create_searcher(&manifest_file);

    let params = SearchParams {
        query: "customers".to_string(),
        resource_types: vec![],
        persona: None,
        detail: DetailLevel::Full,
        min_score: None,
        fuzzy: false,
        include_highlights: true,
        include_sql: true,
        explain: false,
        pagination: PaginationParams {
            limit: 5,
            offset: 0,
        },
    };

    let result = searcher.search(&params).await.unwrap();
    let data = result["data"].as_array().unwrap();

    assert!(!data.is_empty());
    let hashighlights = data.iter().any(|item| {
        item.get("highlights")
            .and_then(|v| v.as_object())
            .map(|h| !h.is_empty())
            .unwrap_or(false)
    });
    assert!(hashighlights);
}

#[tokio::test(flavor = "multi_thread")]
async fn tantivy_search_suggestions() {
    let manifest_file = create_test_manifest();
    let guard = support_config::TestStorageGuard::new();
    let mut cfg = DbtNovaConfig {
        manifest_path: manifest_file.path().to_string_lossy().to_string(),
        search: support_config::test_search_config(),
        ..Default::default()
    };
    support_config::apply_test_storage(&mut cfg, &guard);
    cfg.search.enable_ngram = false;
    let searcher = ManifestSearch::new(cfg).unwrap().search;

    let params = SearchParams {
        query: "custmoers".to_string(),
        resource_types: vec![],
        persona: None,
        detail: DetailLevel::Standard,
        min_score: None,
        fuzzy: false,
        include_highlights: false,
        include_sql: false,
        explain: false,
        pagination: PaginationParams {
            limit: 5,
            offset: 0,
        },
    };

    let result = searcher.search(&params).await.unwrap();
    let suggestions = result["suggestions"].as_array().unwrap();

    assert!(!suggestions.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn tantivy_search_macro_sql() {
    let manifest_file = create_test_manifest();
    let searcher = create_searcher(&manifest_file);

    let params = SearchParams {
        query: "generate_schema_name".to_string(),
        resource_types: vec![],
        persona: None,
        detail: DetailLevel::Full,
        min_score: None,
        fuzzy: false,
        include_highlights: false,
        include_sql: false,
        explain: false,
        pagination: PaginationParams {
            limit: 10,
            offset: 0,
        },
    };

    let result = searcher.search(&params).await.unwrap();
    let data = result["data"].as_array().unwrap();

    // Should find macro by name.
    let found = data
        .iter()
        .any(|e| e["unique_id"] == "macro.test.generate_schema_name");
    assert!(found, "Should find macro by name");
}

#[tokio::test(flavor = "multi_thread")]
async fn tantivy_search_doc_contents() {
    let manifest_file = create_test_manifest();
    let searcher = create_searcher(&manifest_file);

    let params = SearchParams {
        query: "overview".to_string(),
        resource_types: vec![],
        persona: None,
        detail: DetailLevel::Full,
        min_score: None,
        fuzzy: false,
        include_highlights: false,
        include_sql: false,
        explain: false,
        pagination: PaginationParams {
            limit: 10,
            offset: 0,
        },
    };

    let result = searcher.search(&params).await.unwrap();
    let data = result["data"].as_array().unwrap();

    // Should find doc by name or content.
    let found = data
        .iter()
        .any(|e| e["unique_id"] == "doc.test.__overview__");
    assert!(found, "Should find doc by name or content");
}

#[tokio::test(flavor = "multi_thread")]
async fn tantivy_search_empty_query_rejected() {
    let manifest_file = create_test_manifest();
    let searcher = create_searcher(&manifest_file);

    let params = SearchParams {
        query: "".to_string(),
        resource_types: vec![],
        persona: None,
        detail: DetailLevel::Full,
        min_score: None,
        fuzzy: false,
        include_highlights: false,
        include_sql: false,
        explain: false,
        pagination: PaginationParams {
            limit: 10,
            offset: 0,
        },
    };

    let result = searcher.search(&params).await;
    assert!(result.is_err(), "Empty query should be rejected");
}

#[tokio::test(flavor = "multi_thread")]
async fn tantivy_search_short_query_rejected() {
    let manifest_file = create_test_manifest();
    let searcher = create_searcher(&manifest_file);

    let params = SearchParams {
        query: "x".to_string(),
        resource_types: vec![],
        persona: None,
        detail: DetailLevel::Full,
        min_score: None,
        fuzzy: false,
        include_highlights: false,
        include_sql: false,
        explain: false,
        pagination: PaginationParams {
            limit: 10,
            offset: 0,
        },
    };

    let result = searcher.search(&params).await;
    assert!(result.is_err(), "Single character query should be rejected");
}
