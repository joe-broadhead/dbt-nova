//! Tests for `get_impact` tool responses.
use super::common::*;

// Impact Analysis Tests
#[tokio::test(flavor = "multi_thread")]
async fn test_get_impact() {
    let searcher = get_searcher();
    let params = GetImpactParams {
        id_or_name: "model.nova_test.stg__traffic_sessions".to_string(),
    };
    let result = searcher.get_impact(&params).await.json();
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
        assert!(data.get("downstream_count").is_some());
        assert!(data.get("column_count").is_some());
        assert!(data.get("impact_score").is_some());
    }
}
#[tokio::test(flavor = "multi_thread")]
async fn test_get_impact_not_found() {
    let searcher = get_searcher();
    let params = GetImpactParams {
        id_or_name: "nonexistent.model".to_string(),
    };
    let result = searcher.get_impact(&params).await.json();
    let success = result
        .get("success")
        .expect("response missing 'success' field")
        .as_bool()
        .expect("'success' field should be boolean");
    assert!(!success, "Should fail for nonexistent entity");
}
