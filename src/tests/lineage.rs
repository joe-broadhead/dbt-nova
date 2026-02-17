//! Tests for `get_lineage` tool responses.
use super::common::*;

// Lineage Tests
#[tokio::test(flavor = "multi_thread")]
async fn test_get_lineage_upstream() {
    let searcher = get_searcher();
    let params = GetLineageParams {
        id_or_name: "model.nova_test.int__campaign_features".to_string(),
        direction: "upstream".to_string(),
        depth: Some(1),
        resource_types: vec![],
        detail: DetailLevel::Standard,
    };
    let result = searcher.get_lineage(&params).await.json();
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
    // This model depends on stg__traffic_sessions
    let data = result
        .get("data")
        .and_then(|d| d.get("entities"))
        .and_then(|d| d.as_array());
    assert!(data.is_some(), "Should return lineage data");
}
#[tokio::test(flavor = "multi_thread")]
async fn test_get_lineage_downstream() {
    let searcher = get_searcher();
    let params = GetLineageParams {
        id_or_name: "model.nova_test.stg__traffic_sessions".to_string(),
        direction: "downstream".to_string(),
        depth: Some(1),
        resource_types: vec![],
        detail: DetailLevel::Standard,
    };
    let result = searcher.get_lineage(&params).await.json();
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
#[tokio::test(flavor = "multi_thread")]
async fn test_get_lineage_full_depth() {
    let searcher = get_searcher();
    let params = GetLineageParams {
        id_or_name: "model.nova_test.stg__traffic_sessions".to_string(),
        direction: "downstream".to_string(),
        depth: None, // Unlimited
        resource_types: vec![],
        detail: DetailLevel::Standard,
    };
    let result = searcher.get_lineage(&params).await.json();
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
#[tokio::test(flavor = "multi_thread")]
async fn test_get_lineage_filter_by_type() {
    let searcher = get_searcher();
    let params = GetLineageParams {
        id_or_name: "model.nova_test.int__campaign_features".to_string(),
        direction: "upstream".to_string(),
        depth: None,
        resource_types: vec!["model".to_string()],
        detail: DetailLevel::Standard,
    };
    let result = searcher.get_lineage(&params).await.json();
    // All results should be models
    if let Some(data) = result
        .get("data")
        .and_then(|d| d.get("entities"))
        .and_then(|d| d.as_array())
    {
        for item in data {
            let rt = item.get("resource_type").and_then(|r| r.as_str());
            assert_eq!(
                rt,
                Some("model"),
                "Filtered lineage should only contain models"
            );
        }
    }
}
#[tokio::test(flavor = "multi_thread")]
async fn test_get_lineage_invalid_direction() {
    let searcher = get_searcher();
    let params = GetLineageParams {
        id_or_name: "model.nova_test.int__campaign_features".to_string(),
        direction: "invalid".to_string(),
        depth: None,
        resource_types: vec![],
        detail: DetailLevel::Standard,
    };
    let result = searcher.get_lineage(&params).await.json();
    let success = result
        .get("success")
        .expect("response missing 'success' field")
        .as_bool()
        .expect("'success' field should be boolean");
    assert!(!success, "Should fail for invalid direction");
}
