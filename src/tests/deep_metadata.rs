//! Tests for nested metadata preservation and access.
use super::common::*;

// Deeply Nested Metadata Preservation Tests
#[tokio::test(flavor = "multi_thread")]
async fn test_deeply_nested_column_meta_preservation() {
    // This test verifies that nested Nova metadata structures like:
    //   meta.nova.role
    //   meta.nova.semantic_type
    //   meta.nova.synonyms
    // are fully preserved when loading the manifest.
    let searcher = get_searcher();
    // The stg__traffic_sessions model has a 'referring_domain' column
    // with nested meta.nova structure
    let model_key = "model.nova_test.stg__traffic_sessions";
    let entity = searcher
        .get_entity(model_key)
        .await
        .unwrap()
        .expect("Should find the model");
    let entity_json = entity.to_json_value();
    // Navigate to columns.referring_domain
    let columns = entity_json.get("columns");
    assert!(columns.is_some(), "Model should have columns");
    let referring_domain = columns.unwrap().get("referring_domain");
    assert!(
        referring_domain.is_some(),
        "Should have referring_domain column"
    );
    let col = referring_domain.unwrap();
    // Level 1: Basic column fields
    assert_eq!(
        col.get("name").and_then(|v| v.as_str()),
        Some("referring_domain"),
        "Column name should be preserved"
    );
    assert_eq!(
        col.get("data_type").and_then(|v| v.as_str()),
        Some("string"),
        "Column data_type should be preserved"
    );
    assert_eq!(
        col.get("description").and_then(|v| v.as_str()),
        Some("Domain name of the referring website."),
        "Column description should be preserved"
    );
    // Level 2: meta object
    let meta = col.get("meta");
    assert!(meta.is_some(), "Column should have meta");
    // Level 3: meta.nova object
    let nova = meta.unwrap().get("nova");
    assert!(nova.is_some(), "meta should have nova");
    let nova = nova.unwrap();
    // Level 3 fields: role + semantic type
    assert_eq!(
        nova.get("role").and_then(|v| v.as_str()),
        Some("dimension"),
        "meta.nova.role should be 'dimension'"
    );
    assert_eq!(
        nova.get("semantic_type").and_then(|v| v.as_str()),
        Some("referrer_domain"),
        "meta.nova.semantic_type should be 'referrer_domain'"
    );
    // Level 4: meta.nova.synonyms array
    let synonyms = nova.get("synonyms");
    assert!(synonyms.is_some(), "meta.nova should have synonyms");
    let synonyms = synonyms.unwrap().as_array();
    assert!(synonyms.is_some(), "synonyms should be an array");
    let synonyms = synonyms.unwrap();
    assert!(
        synonyms
            .iter()
            .any(|v| v.as_str() == Some("referral_domain")),
        "synonyms should contain 'referral_domain'"
    );
    // Also verify other column-level fields are preserved
    assert!(
        col.get("constraints").is_some(),
        "Column should have constraints"
    );
    assert!(col.get("config").is_some(), "Column should have config");
    assert!(col.get("tags").is_some(), "Column should have tags");
}
#[tokio::test(flavor = "multi_thread")]
async fn test_get_columns_preserves_nested_meta() {
    // Verify the get_columns tool also returns the deeply nested meta
    let searcher = get_searcher();
    let params = GetColumnsParams {
        id_or_name: "stg__traffic_sessions".to_string(),
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
    let data = result.get("data");
    assert!(data.is_some(), "Should have data");
    let columns = data.unwrap().get("columns").and_then(|c| c.as_array());
    assert!(columns.is_some(), "Should have columns array");
    let columns = columns.unwrap();
    // Find referring_domain column
    let referring_domain = columns
        .iter()
        .find(|c| c.get("name").and_then(|n| n.as_str()) == Some("referring_domain"));
    assert!(
        referring_domain.is_some(),
        "Should find referring_domain in columns"
    );
    let col = referring_domain.unwrap();
    // Verify the full nested path is accessible through get_columns
    let semantic_type = col
        .get("meta")
        .and_then(|m| m.get("nova"))
        .and_then(|cx| cx.get("semantic_type"))
        .and_then(|n| n.as_str());
    assert_eq!(
        semantic_type,
        Some("referrer_domain"),
        "get_columns should preserve meta.nova.semantic_type"
    );
    // Verify synonyms are preserved
    let synonyms = col
        .get("meta")
        .and_then(|m| m.get("nova"))
        .and_then(|cx| cx.get("synonyms"))
        .and_then(|s| s.as_array());
    assert!(synonyms.is_some(), "meta.nova.synonyms should exist");
    let synonyms = synonyms.unwrap();
    assert!(
        synonyms
            .iter()
            .any(|v| v.as_str() == Some("referral_domain")),
        "meta.nova.synonyms should contain 'referral_domain'"
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn test_search_returns_full_nested_meta_when_include_full() {
    // Verify search with include_full=true returns deeply nested metadata
    let searcher = get_searcher();
    let params = SearchParams {
        query: "traffic".to_string(),
        resource_types: vec!["model".to_string()],
        persona: None,
        detail: Some(DetailLevel::Full),
        min_score: None,
        fuzzy: false,
        include_highlights: false,
        include_sql: false,
        explain: false,
        pagination: PaginationParams {
            limit: Some(5),
            offset: 0,
        },
    };
    let result = searcher.search(&params).await.json();
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
    let data = result.get("data").and_then(|d| d.as_array());
    assert!(data.is_some(), "Should have data");
    // Find stg__traffic_sessions in results
    let target = data
        .unwrap()
        .iter()
        .find(|e| e.get("name").and_then(|n| n.as_str()) == Some("stg__traffic_sessions"));
    if let Some(entity) = target {
        // With include_full=true, we should have the complete nested structure
        let semantic_type = entity
            .get("columns")
            .and_then(|c| c.get("referring_domain"))
            .and_then(|rd| rd.get("meta"))
            .and_then(|m| m.get("nova"))
            .and_then(|cx| cx.get("semantic_type"))
            .and_then(|n| n.as_str());
        assert_eq!(
            semantic_type,
            Some("referrer_domain"),
            "Full search result should preserve nova meta"
        );
    }
}
