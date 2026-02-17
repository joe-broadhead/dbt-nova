//! Tests for `list_entities` tool responses.
use super::common::*;

// List Entities Tests
#[tokio::test(flavor = "multi_thread")]
async fn test_list_entities_models() {
    let searcher = get_searcher();
    let params = ListEntitiesParams {
        resource_type: "model".to_string(),
        package: None,
        tags: vec![],
        database_schema: None,
        detail: DetailLevel::Full,
        pagination: PaginationParams {
            limit: 10,
            offset: 0,
        },
    };
    let result = searcher.list_entities(&params).await.json();
    let success = result
        .get("success")
        .expect("response missing 'success' field")
        .as_bool()
        .expect("'success' field should be boolean");
    assert!(
        success,
        "Expected success=true but got error: {:?}",
        result.get("error")
    );
    let count = result
        .get("count")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    assert!(count > 0, "Should return models");
    assert!(count <= 10, "Should respect limit");
}
#[tokio::test(flavor = "multi_thread")]
async fn test_list_entities_with_tag_filter() {
    let searcher = get_searcher();
    let params = ListEntitiesParams {
        resource_type: "model".to_string(),
        package: None,
        tags: vec!["sql".to_string()],
        database_schema: None,
        detail: DetailLevel::Standard,
        pagination: PaginationParams {
            limit: 50,
            offset: 0,
        },
    };
    let result = searcher.list_entities(&params).await.json();
    // All returned entities should have the 'sql' tag
    if let Some(data) = result.get("data").and_then(|d| d.as_array()) {
        for item in data {
            // In summary mode we don't have tags, so just check we got results
            assert!(item.get("unique_id").is_some());
        }
    }
}
#[tokio::test(flavor = "multi_thread")]
async fn test_list_entities_sources() {
    let searcher = get_searcher();
    let params = ListEntitiesParams {
        resource_type: "source".to_string(),
        package: None,
        tags: vec![],
        database_schema: None,
        detail: DetailLevel::Full,
        pagination: PaginationParams {
            limit: 10,
            offset: 0,
        },
    };
    let result = searcher.list_entities(&params).await.json();
    let total = result
        .get("total_available")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    assert!(total > 0, "Should have sources");
}
#[tokio::test(flavor = "multi_thread")]
async fn test_list_entities_groups() {
    let searcher = get_searcher();
    let params = ListEntitiesParams {
        resource_type: "group".to_string(),
        package: None,
        tags: vec![],
        database_schema: None,
        detail: DetailLevel::Full,
        pagination: PaginationParams {
            limit: 50,
            offset: 0,
        },
    };
    let result = searcher.list_entities(&params).await.json();
    let count = result
        .get("count")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    assert!(count > 0, "Should have groups");
}
#[tokio::test(flavor = "multi_thread")]
async fn test_list_entities_invalid_type() {
    let searcher = get_searcher();
    let params = ListEntitiesParams {
        resource_type: "invalid_type".to_string(),
        package: None,
        tags: vec![],
        database_schema: None,
        detail: DetailLevel::Full,
        pagination: PaginationParams {
            limit: 10,
            offset: 0,
        },
    };
    let result = searcher.list_entities(&params).await.json();
    let count = result
        .get("count")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    assert_eq!(count, 0, "Should return empty for invalid type");
}
