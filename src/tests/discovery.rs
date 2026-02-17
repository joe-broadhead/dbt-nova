//! Tests for discovery tools (metadata, tags, packages, databases).
use super::common::*;

// Metadata & Discovery Tests
#[tokio::test(flavor = "multi_thread")]
async fn test_show_metadata() {
    let searcher = get_searcher();
    let result = searcher.show_metadata().await.json();
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
    if let Some(data) = result.get("data") {
        // Check manifest metadata
        let metadata = data.get("metadata");
        assert!(metadata.is_some());
        let dbt_version = metadata
            .and_then(|m| m.get("dbt_version"))
            .and_then(|v| v.as_str());
        assert_eq!(dbt_version, Some("1.10.2"));
        // Check entity counts
        assert!(data.get("entity_counts").is_some());
        assert!(data.get("total_entities").is_some());
    }
}
#[tokio::test(flavor = "multi_thread")]
async fn test_list_tags() {
    let searcher = get_searcher();
    let result = searcher.list_tags().await.json();
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
    assert!(count > 0, "Should have tags");
    // Check for known tag
    if let Some(data) = result.get("data").and_then(|d| d.as_array()) {
        let has_sql_tag = data
            .iter()
            .any(|t| t.get("tag").and_then(|v| v.as_str()) == Some("sql"));
        assert!(has_sql_tag, "Should have 'sql' tag");
    }
}
#[tokio::test(flavor = "multi_thread")]
async fn test_list_packages() {
    let searcher = get_searcher();
    let result = searcher.list_packages().await.json();
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
    assert!(count > 0, "Should have packages");
}
#[tokio::test(flavor = "multi_thread")]
async fn test_list_databases() {
    let searcher = get_searcher();
    let result = searcher.list_databases().await.json();
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
    assert!(count > 0, "Should have database.schema combinations");
}
