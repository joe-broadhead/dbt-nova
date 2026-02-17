//! Integration tests for end-to-end workflows.
use super::common::*;

// Integration / Workflow Tests
#[tokio::test(flavor = "multi_thread")]
async fn test_full_workflow() {
    let searcher = get_searcher();
    // 1. Search for models (entry point for most workflows).
    let search_params = SearchParams {
        query: "model".to_string(),
        resource_types: vec!["model".to_string()],
        persona: None,
        detail: DetailLevel::Standard,
        min_score: None,
        fuzzy: false,
        include_highlights: false,
        include_sql: false,
        pagination: PaginationParams {
            limit: 5,
            offset: 0,
        },
    };
    let search_result = searcher.search(&search_params).await.json();
    let success = search_result
        .get("success")
        .expect("response missing 'success' field")
        .as_bool()
        .expect("'success' field should be boolean");
    assert!(
        success,
        "Expected success=true but got error: {:?}",
        search_result.get("error")
    );
    // 2. Get first result's unique_id to drive follow-up tool calls.
    let first_id = search_result
        .get("data")
        .and_then(|d| d.as_array())
        .and_then(|a| a.first())
        .and_then(|e| e.get("unique_id"))
        .and_then(|id| id.as_str());
    assert!(first_id.is_some(), "Should get a result");
    let unique_id = first_id.unwrap();
    // 3. Get full entity payload to simulate a deep inspection step.
    let _entity = searcher
        .get_entity(unique_id)
        .await
        .unwrap()
        .expect("Should find entity");
    // 4. Get columns to validate schema availability for downstream tools.
    let col_params = GetColumnsParams {
        id_or_name: unique_id.to_string(),
    };
    let cols_result = searcher.get_columns(&col_params).await.json();
    let success = cols_result
        .get("success")
        .expect("response missing 'success' field")
        .as_bool()
        .expect("'success' field should be boolean");
    assert!(
        success,
        "Expected success=true but got error: {:?}",
        cols_result.get("error")
    );
    // 5. Get lineage to verify graph traversal works on the same entity.
    let lineage_params = GetLineageParams {
        id_or_name: unique_id.to_string(),
        direction: "upstream".to_string(),
        depth: Some(2),
        resource_types: vec![],
        detail: DetailLevel::Standard,
    };
    let lineage_result = searcher.get_lineage(&lineage_params).await.json();
    let success = lineage_result
        .get("success")
        .expect("response missing 'success' field")
        .as_bool()
        .expect("'success' field should be boolean");
    assert!(
        success,
        "Expected success=true but got error: {:?}",
        lineage_result.get("error")
    );
    // 6. Get impact to validate downstream dependency reporting.
    let impact_params = GetImpactParams {
        id_or_name: unique_id.to_string(),
    };
    let impact_result = searcher.get_impact(&impact_params).await.json();
    let success = impact_result
        .get("success")
        .expect("response missing 'success' field")
        .as_bool()
        .expect("'success' field should be boolean");
    assert!(
        success,
        "Expected success=true but got error: {:?}",
        impact_result.get("error")
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn test_100_percent_field_access() {
    let searcher = get_searcher();
    // Get a model with full data to verify raw JSON preservation.
    let key = "model.nova_test.int__campaign_features";
    let e = searcher
        .get_entity(key)
        .await
        .unwrap()
        .expect("Should find entity");
    let entity_json = e.to_json_value();
    // Verify we have access to a broad set of manifest fields.
    assert!(entity_json.get("name").is_some(), "Should have name");
    assert!(
        entity_json.get("resource_type").is_some(),
        "Should have resource_type"
    );
    assert!(
        entity_json.get("package_name").is_some(),
        "Should have package_name"
    );
    assert!(
        entity_json.get("unique_id").is_some(),
        "Should have unique_id"
    );
    assert!(entity_json.get("path").is_some(), "Should have path");
    assert!(
        entity_json.get("original_file_path").is_some(),
        "Should have original_file_path"
    );
    assert!(
        entity_json.get("database").is_some(),
        "Should have database"
    );
    assert!(entity_json.get("schema").is_some(), "Should have schema");
    assert!(entity_json.get("alias").is_some(), "Should have alias");
    assert!(entity_json.get("config").is_some(), "Should have config");
    assert!(entity_json.get("tags").is_some(), "Should have tags");
    assert!(entity_json.get("columns").is_some(), "Should have columns");
    assert!(
        entity_json.get("depends_on").is_some(),
        "Should have depends_on"
    );
    assert!(entity_json.get("fqn").is_some(), "Should have fqn");
    assert!(
        entity_json.get("checksum").is_some(),
        "Should have checksum"
    );
    // These may or may not be present depending on manifest:
    // raw_code, compiled_code, relation_name, etc.
}
#[tokio::test(flavor = "multi_thread")]
async fn test_entity_summary_vs_full() {
    let searcher = get_searcher();
    let key = "model.nova_test.int__campaign_features";
    // Summary should include only key identity fields.
    let summary = searcher.entity_summary(key).unwrap();
    assert!(summary.get("unique_id").is_some());
    assert!(summary.get("name").is_some());
    assert!(summary.get("resource_type").is_some());
    // Full entity should have a richer payload than the summary.
    let full = searcher.get_entity(key).await.unwrap().unwrap();
    let full_json = full.to_json_value();
    assert!(
        full_json.as_object().unwrap().len() > 10,
        "Full entity should have many fields"
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn test_parent_child_map_usage() {
    let searcher = get_searcher();
    // Verify parent_map and child_map are populated from manifest.
    assert!(
        !searcher.parent_map.is_empty(),
        "parent_map should be populated"
    );
    assert!(
        !searcher.child_map.is_empty(),
        "child_map should be populated"
    );
    // Check a known relationship from the fixture manifest.
    let model_key = "model.nova_test.int__campaign_features";
    let parents = searcher.parent_map.get(model_key);
    // This model should have upstream dependencies
    assert!(parents.is_some(), "Model should have parents in parent_map");
}
