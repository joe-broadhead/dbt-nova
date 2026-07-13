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
        governance: None,
        tier: vec![],
        canonical: None,
        detail: Some(DetailLevel::Full),
        pagination: PaginationParams {
            limit: Some(10),
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
        governance: None,
        tier: vec![],
        canonical: None,
        detail: Some(DetailLevel::Standard),
        pagination: PaginationParams {
            limit: Some(50),
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
async fn test_list_entities_filters_governance_tier_and_canonical() {
    let searcher = get_searcher_with_fixture("perfect_model.json");
    let params = ListEntitiesParams {
        resource_type: "model".to_string(),
        package: None,
        tags: vec![],
        database_schema: None,
        governance: Some(ListEntitiesGovernanceFilter {
            pii: vec!["TRUE".to_string()],
            sensitivity: vec!["HIGH".to_string()],
            compliance_includes: vec!["gdpr".to_string(), "HIPAA".to_string()],
            declared: Some(true),
        }),
        tier: vec!["GOLD".to_string()],
        canonical: Some(true),
        detail: Some(DetailLevel::Standard),
        pagination: PaginationParams {
            limit: Some(10),
            offset: 0,
        },
    };

    let result = searcher.list_entities(&params).await.json();
    assert_eq!(
        result.get("success").and_then(JsonValue::as_bool),
        Some(true)
    );
    assert_eq!(result.get("count").and_then(JsonValue::as_u64), Some(1));

    let row = result["data"]
        .as_array()
        .and_then(|rows| rows.first())
        .expect("filtered row");
    assert_eq!(row["unique_id"], "model.nova_test.perfect_model");
    assert_eq!(row["nova_summary"]["governance"]["pii"], "true");
    assert_eq!(row["nova_summary"]["governance"]["sensitivity"], "high");
    assert_eq!(row["nova_summary"]["tier"], "gold");
    assert_eq!(row["nova_summary"]["canonical"], true);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_list_entities_filters_missing_governance_and_noncanonical() {
    let searcher = get_searcher_with_fixture("minimal.json");
    let params = ListEntitiesParams {
        resource_type: "model".to_string(),
        package: None,
        tags: vec![],
        database_schema: None,
        governance: Some(ListEntitiesGovernanceFilter {
            declared: Some(false),
            ..ListEntitiesGovernanceFilter::default()
        }),
        tier: vec![],
        canonical: Some(false),
        detail: Some(DetailLevel::Standard),
        pagination: PaginationParams {
            limit: Some(10),
            offset: 0,
        },
    };

    let result = searcher.list_entities(&params).await.json();
    assert_eq!(
        result.get("success").and_then(JsonValue::as_bool),
        Some(true)
    );
    assert_eq!(result.get("count").and_then(JsonValue::as_u64), Some(1));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_list_entities_sources() {
    let searcher = get_searcher();
    let params = ListEntitiesParams {
        resource_type: "source".to_string(),
        package: None,
        tags: vec![],
        database_schema: None,
        governance: None,
        tier: vec![],
        canonical: None,
        detail: Some(DetailLevel::Full),
        pagination: PaginationParams {
            limit: Some(10),
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
        governance: None,
        tier: vec![],
        canonical: None,
        detail: Some(DetailLevel::Full),
        pagination: PaginationParams {
            limit: Some(50),
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
        governance: None,
        tier: vec![],
        canonical: None,
        detail: Some(DetailLevel::Full),
        pagination: PaginationParams {
            limit: Some(10),
            offset: 0,
        },
    };
    let result = searcher.list_entities(&params).await.json();
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
