//! Tests for path prefix index behavior.
use super::common::*;

// Path Prefix Index Tests
#[tokio::test(flavor = "multi_thread")]
async fn test_path_prefix_index_populated() {
    let searcher = get_searcher();
    // Verify the path prefix index exists and has entries
    assert!(
        !searcher.by_path_prefix.is_empty(),
        "by_path_prefix should be populated"
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn test_path_prefix_index_structure() {
    let searcher = get_searcher();
    // Check that paths are indexed at multiple levels
    // e.g., "models/staging/file.sql" should create entries for:
    // "models", "models/staging", "models/staging/file.sql"
    let mut found_nested = false;
    for key in searcher.by_path_prefix.keys() {
        if key.contains('/') {
            found_nested = true;
            break;
        }
    }
    assert!(found_nested, "Should have nested path prefixes");
}
#[tokio::test(flavor = "multi_thread")]
async fn test_find_by_path_uses_index() {
    let searcher = get_searcher();
    let prefix = "models".to_string();
    assert!(
        searcher.by_path_prefix.contains_key(&prefix),
        "fixture should contain '{prefix}' path prefix"
    );
    let params = FindByPathParams {
        path_pattern: format!("{prefix}/**"),
        resource_types: vec![],
        detail: Some(DetailLevel::Standard),
        pagination: PaginationParams {
            limit: Some(100),
            offset: 0,
        },
    };
    let result = searcher.find_by_path(&params).await.json();
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
    // Should find entities under this prefix
    let count = result
        .get("count")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    assert!(count > 0, "Should find entities under prefix '{prefix}'");
}
#[tokio::test(flavor = "multi_thread")]
async fn test_get_path_candidates_with_static_prefix() {
    let searcher = get_searcher();
    let candidates = searcher.get_path_candidates("models/*");
    assert!(
        !candidates.is_empty(),
        "Should return candidates for static 'models' prefix"
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn test_get_path_candidates_no_prefix() {
    let searcher = get_searcher();
    // Pattern with no static prefix should return all entities
    let candidates = searcher.get_path_candidates("**");
    assert_eq!(
        candidates.len(),
        searcher.entity_count(),
        "** pattern should return all entities"
    );
}
