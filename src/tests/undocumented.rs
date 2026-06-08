//! Tests for `get_undocumented` tool responses.
use super::common::*;

// Get Undocumented Tests
#[tokio::test(flavor = "multi_thread")]
async fn test_get_undocumented_models() {
    let searcher = get_searcher();
    let params = GetUndocumentedParams {
        resource_type: "model".to_string(),
        id_or_name: None,
        package: None,
        path_prefix: None,
        include_columns: false,
        include_full: false,
        pagination: PaginationParams {
            limit: Some(50),
            offset: 0,
        },
    };
    let result = searcher.get_undocumented(&params).await.json();
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
    let data = result.get("data").unwrap();
    assert!(data.get("entities").is_some());
    assert!(data.get("summary").is_some());
    let summary = data.get("summary").unwrap();
    assert!(summary.get("entities_missing_docs").is_some());
    let entities_len = data
        .get("entities")
        .and_then(serde_json::Value::as_array)
        .map_or(0usize, Vec::len);
    let columns_len = data
        .get("undocumented_columns")
        .and_then(serde_json::Value::as_array)
        .map_or(0usize, Vec::len);
    let count = result
        .get("count")
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or(0);
    assert_eq!(count, entities_len + columns_len);
}
#[tokio::test(flavor = "multi_thread")]
async fn test_get_undocumented_with_columns() {
    let searcher = get_searcher();
    let params = GetUndocumentedParams {
        resource_type: "model".to_string(),
        id_or_name: None,
        package: None,
        path_prefix: None,
        include_columns: true,
        include_full: false,
        pagination: PaginationParams {
            limit: Some(50),
            offset: 0,
        },
    };
    let result = searcher.get_undocumented(&params).await.json();
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
    let data = result.get("data").unwrap();
    assert!(data.get("undocumented_columns").is_some());
    let entities_len = data
        .get("entities")
        .and_then(serde_json::Value::as_array)
        .map_or(0usize, Vec::len);
    let columns_len = data
        .get("undocumented_columns")
        .and_then(serde_json::Value::as_array)
        .map_or(0usize, Vec::len);
    let count = result
        .get("count")
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or(0);
    assert_eq!(count, entities_len + columns_len);
    // Check column structure if any exist
    if let Some(cols) = data.get("undocumented_columns").and_then(|c| c.as_array())
        && !cols.is_empty()
    {
        let first = &cols[0];
        assert!(first.get("entity_unique_id").is_some());
        assert!(first.get("column_name").is_some());
    }
}
#[tokio::test(flavor = "multi_thread")]
async fn test_get_undocumented_respects_limit() {
    let searcher = get_searcher();
    let params = GetUndocumentedParams {
        resource_type: "model".to_string(),
        id_or_name: None,
        package: None,
        path_prefix: None,
        include_columns: false,
        include_full: false,
        pagination: PaginationParams {
            limit: Some(3),
            offset: 0,
        },
    };
    let result = searcher.get_undocumented(&params).await.json();
    let entities = result
        .get("data")
        .and_then(|d| d.get("entities"))
        .and_then(|e| e.as_array())
        .unwrap();
    assert!(entities.len() <= 3, "Should respect limit");
}
#[tokio::test(flavor = "multi_thread")]
async fn test_get_undocumented_invalid_resource_type() {
    let searcher = get_searcher();
    let params = GetUndocumentedParams {
        resource_type: "nonexistent_type".to_string(),
        id_or_name: None,
        package: None,
        path_prefix: None,
        include_columns: false,
        include_full: false,
        pagination: PaginationParams {
            limit: Some(50),
            offset: 0,
        },
    };
    let result = searcher.get_undocumented(&params).await.json();
    let success = result
        .get("success")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(true);
    assert!(!success, "invalid resource_type should return an error");
    let error_code = result
        .get("error_code")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    assert_eq!(error_code, "INVALID_PARAMS");
}
#[tokio::test(flavor = "multi_thread")]
async fn test_get_undocumented_include_full() {
    let searcher = get_searcher();
    let params = GetUndocumentedParams {
        resource_type: "model".to_string(),
        id_or_name: None,
        package: None,
        path_prefix: None,
        include_columns: false,
        include_full: true,
        pagination: PaginationParams {
            limit: Some(5),
            offset: 0,
        },
    };
    let result = searcher.get_undocumented(&params).await.json();
    let entities = result
        .get("data")
        .and_then(|d| d.get("entities"))
        .and_then(|e| e.as_array())
        .unwrap();
    if !entities.is_empty() {
        let first = &entities[0];
        // With include_full=true, should have detailed fields
        assert!(first.get("unique_id").is_some());
        // May have columns, raw_code, etc. depending on entity
    }
}
#[tokio::test(flavor = "multi_thread")]
async fn test_get_undocumented_sources() {
    let searcher = get_searcher();
    let params = GetUndocumentedParams {
        resource_type: "source".to_string(),
        id_or_name: None,
        package: None,
        path_prefix: None,
        include_columns: true,
        include_full: false,
        pagination: PaginationParams {
            limit: Some(20),
            offset: 0,
        },
    };
    let result = searcher.get_undocumented(&params).await.json();
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
}
