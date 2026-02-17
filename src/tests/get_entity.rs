//! Tests for `get_entity` tool responses.
use super::common::*;

// Get Entity Tests
#[tokio::test(flavor = "multi_thread")]
async fn test_get_entity_by_unique_id() {
    let searcher = get_searcher();
    let key = "model.nova_test.int__campaign_features";
    let e = searcher
        .get_entity(key)
        .await
        .unwrap()
        .expect("Should find entity by unique_id");
    let entity_json = e.to_json_value();
    assert_eq!(
        entity_json.get("name").and_then(|n| n.as_str()),
        Some("int__campaign_features")
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn test_resolve_by_name() {
    let searcher = get_searcher();
    let keys = searcher.resolve_id_or_name("int__campaign_features", None);
    assert!(!keys.is_empty(), "Should resolve name to keys");
}
#[tokio::test(flavor = "multi_thread")]
async fn test_resolve_by_name_with_type_filter() {
    let searcher = get_searcher();
    let keys = searcher.resolve_id_or_name("int__campaign_features", Some("model"));
    assert!(!keys.is_empty(), "Should resolve name with type filter");
    // All resolved keys should be models
    for key in &keys {
        let entity = searcher.get_entity(key).await.unwrap().unwrap();
        let entity_json = entity.to_json_value();
        assert_eq!(
            entity_json.get("resource_type").and_then(|r| r.as_str()),
            Some("model")
        );
    }
}
#[tokio::test(flavor = "multi_thread")]
async fn test_get_entity_not_found() {
    let searcher = get_searcher();
    let keys = searcher.resolve_id_or_name("nonexistent_entity_xyz", None);
    assert!(
        keys.is_empty(),
        "Should return empty for nonexistent entity"
    );
}
