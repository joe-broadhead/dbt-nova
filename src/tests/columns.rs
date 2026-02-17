//! Tests for `get_columns` tool responses.
use super::common::*;

// Get Columns Tests
#[tokio::test(flavor = "multi_thread")]
async fn test_get_columns_success() {
    let searcher = get_searcher();
    let params = GetColumnsParams {
        id_or_name: "int__campaign_features".to_string(),
    };
    let result = searcher.get_columns(&params).await.json();
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
    // This model has columns
    if let Some(data) = result.get("data") {
        let columns = data.get("columns").and_then(|c| c.as_array());
        assert!(columns.is_some(), "Should return columns array");
        assert!(!columns.unwrap().is_empty(), "Should have columns");
    }
}
#[tokio::test(flavor = "multi_thread")]
async fn test_get_columns_not_found() {
    let searcher = get_searcher();
    let params = GetColumnsParams {
        id_or_name: "nonexistent_entity".to_string(),
    };
    let result = searcher.get_columns(&params).await.json();
    let success = result
        .get("success")
        .expect("response missing 'success' field")
        .as_bool()
        .expect("'success' field should be boolean");
    assert!(!success, "Should fail for nonexistent entity");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_get_columns_are_sorted_by_name() {
    let searcher = get_searcher();
    let params = GetColumnsParams {
        id_or_name: "int__campaign_features".to_string(),
    };
    let result = searcher.get_columns(&params).await.json();
    let columns = result
        .get("data")
        .and_then(|data| data.get("columns"))
        .and_then(|value| value.as_array())
        .expect("columns array");

    let mut names: Vec<String> = columns
        .iter()
        .filter_map(|column| {
            column
                .get("name")
                .and_then(|value| value.as_str())
                .map(ToString::to_string)
        })
        .collect();
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted, "columns should be ordered by name");

    names.dedup();
    assert_eq!(
        names.len(),
        sorted.len(),
        "columns should not duplicate names"
    );
}
