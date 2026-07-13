//! Integration tests for manifest loading and indexing.
#[path = "support/config.rs"]
mod support_config;
#[path = "support/fixtures.rs"]
mod support_fixtures;

use dbt_nova::error::DbtNovaError;
use dbt_nova::params::PaginationParams;
use dbt_nova::params::{DetailLevel, ListEntitiesParams, SearchParams};

#[tokio::test(flavor = "multi_thread")]
async fn loads_minimal_manifest_and_indexes() {
    let searcher = support_fixtures::load_fixture("minimal.json").expect("fixture should load");

    // list_entities should return the single model
    let list_params = ListEntitiesParams {
        resource_type: "model".into(),
        package: None,
        tags: vec![],
        database_schema: None,
        governance: None,
        tier: vec![],
        canonical: None,
        detail: Some(DetailLevel::Standard),
        pagination: PaginationParams {
            limit: Some(10),
            offset: 0,
        },
    };
    let list = searcher.list_entities(&list_params).await.unwrap();
    assert_eq!(
        list.get("count").and_then(serde_json::Value::as_u64),
        Some(1)
    );

    // search should find the model term
    let search_params = SearchParams {
        query: "model_a".to_string(),
        resource_types: vec!["model".into()],
        persona: None,
        detail: Some(DetailLevel::Standard),
        min_score: None,
        fuzzy: false,
        include_highlights: false,
        include_sql: false,
        explain: false,
        pagination: PaginationParams {
            limit: Some(5),
            offset: 0,
        },
    };
    let search = searcher.search(&search_params).await.unwrap();
    assert_eq!(
        search.get("count").and_then(serde_json::Value::as_u64),
        Some(1)
    );
}

#[test]
fn missing_manifest_returns_error() {
    let guard = support_config::TestStorageGuard::new();
    let mut cfg = dbt_nova::DbtNovaConfig {
        manifest_path: "tests/fixtures/does_not_exist.json".to_string(),
        search: support_config::test_search_config(),
        ..Default::default()
    };
    support_config::apply_test_storage(&mut cfg, &guard);
    match dbt_nova::ManifestSearch::new(cfg) {
        Err(DbtNovaError::ManifestError(message)) => {
            assert!(
                message.contains("Failed to read manifest file"),
                "unexpected manifest error: {message}"
            );
        }
        _ => panic!("expected ManifestError"),
    }
}

#[test]
fn malformed_manifest_returns_parse_error() {
    // Write a tiny malformed JSON to a temp file
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), "{ not json }").unwrap();
    let guard = support_config::TestStorageGuard::new();
    let mut cfg = dbt_nova::DbtNovaConfig {
        manifest_path: tmp.path().to_string_lossy().to_string(),
        search: support_config::test_search_config(),
        ..Default::default()
    };
    support_config::apply_test_storage(&mut cfg, &guard);
    match dbt_nova::ManifestSearch::new(cfg) {
        Err(DbtNovaError::ManifestError(message)) => {
            assert!(
                message.contains("Failed to parse manifest JSON"),
                "unexpected manifest error: {message}"
            );
        }
        _ => panic!("expected ManifestError"),
    }
}
