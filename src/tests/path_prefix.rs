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
    // Find a common prefix in the manifest
    let mut common_prefix = None;
    for prefix in searcher.by_path_prefix.keys() {
        if !prefix.contains('/') && !prefix.contains('.') {
            // Top-level directory
            if let Some(count) = searcher.by_path_prefix.get(prefix).map(Vec::len)
                && count > 0
            {
                common_prefix = Some(prefix.clone());
                break;
            }
        }
    }
    if common_prefix.is_none() {
        println!("Skipping test: no suitable prefix found");
        return;
    }
    let prefix = common_prefix.unwrap();
    let params = FindByPathParams {
        path_pattern: format!("{prefix}/**"),
        resource_types: vec![],
        detail: DetailLevel::Standard,
        pagination: PaginationParams {
            limit: 100,
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
    // Find a prefix that exists
    let existing_prefix = searcher.by_path_prefix.keys().next().cloned();
    if let Some(prefix) = existing_prefix {
        let pattern = format!("{prefix}/*");
        let candidates = searcher.get_path_candidates(&pattern);
        // Should return fewer candidates than total entities
        // (unless the prefix covers everything)
        assert!(
            !candidates.is_empty(),
            "Should return candidates for existing prefix"
        );
    }
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
