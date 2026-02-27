//! Tests for column lineage tool responses.
use super::common::*;

// Column Lineage Tests
#[tokio::test(flavor = "multi_thread")]
async fn test_get_column_lineage_entity_not_found() {
    let searcher = get_searcher();
    let params = GetColumnLineageParams {
        id_or_name: "nonexistent_model".to_string(),
        resource_type: None,
        column_name: "some_column".to_string(),
        direction: "upstream".to_string(),
        depth: None,
        confidence: Some("medium".to_string()),
    };
    let result = searcher.get_column_lineage(&params).await.json();
    let success = result
        .get("success")
        .expect("response missing 'success' field")
        .as_bool()
        .expect("'success' field should be boolean");
    assert!(!success, "Should fail for nonexistent entity");
    let error_code = result.get("error_code").and_then(|e| e.as_str());
    assert_eq!(error_code, Some("NOT_FOUND"));
}
#[tokio::test(flavor = "multi_thread")]
async fn test_get_column_lineage_column_not_found() {
    let searcher = get_searcher();
    // Find a model that we know exists to validate the error message for missing columns.
    let models = searcher.by_resource_type.get("model").unwrap();
    let first_model_id = &models[0];
    let params = GetColumnLineageParams {
        id_or_name: first_model_id.clone(),
        resource_type: None,
        column_name: "nonexistent_column_xyz_123".to_string(),
        direction: "upstream".to_string(),
        depth: None,
        confidence: Some("medium".to_string()),
    };
    let result = searcher.get_column_lineage(&params).await.json();
    let success = result
        .get("success")
        .expect("response missing 'success' field")
        .as_bool()
        .expect("'success' field should be boolean");
    assert!(!success, "Should fail for nonexistent column");
    let error_code = result.get("error_code").and_then(|e| e.as_str());
    assert_eq!(error_code, Some("NOT_FOUND"));
}
#[tokio::test(flavor = "multi_thread")]
async fn test_get_column_lineage_invalid_direction() {
    let searcher = get_searcher();
    let model_id = "model.nova_test.int__campaign_features".to_string();
    assert!(
        searcher.has_entity(&model_id),
        "fixture should contain {model_id}"
    );
    let params = GetColumnLineageParams {
        id_or_name: model_id,
        resource_type: None,
        column_name: "campaign_name".to_string(),
        direction: "invalid".to_string(),
        depth: None,
        confidence: Some("medium".to_string()),
    };
    let result = searcher.get_column_lineage(&params).await.json();
    let success = result
        .get("success")
        .expect("response missing 'success' field")
        .as_bool()
        .expect("'success' field should be boolean");
    assert!(!success, "Should fail for invalid direction");
    let error_code = result.get("error_code").and_then(|e| e.as_str());
    assert_eq!(error_code, Some("INVALID_PARAMS"));
}
#[tokio::test(flavor = "multi_thread")]
async fn test_get_column_lineage_upstream_high_confidence() {
    // Use a fixture where upstream and downstream share an exact column name ("id")
    // so high-confidence matching must return concrete lineage rows.
    let searcher = get_searcher_with_fixture("ambiguous_name.json");
    let test_model = "model.pkg.downstream".to_string();
    assert!(
        searcher
            .parent_map
            .get(&test_model)
            .is_some_and(|parents| !parents.is_empty()),
        "fixture should include upstream dependencies for {test_model}"
    );
    let params = GetColumnLineageParams {
        id_or_name: test_model,
        resource_type: None,
        column_name: "id".to_string(),
        direction: "upstream".to_string(),
        depth: Some(1),
        confidence: Some("high".to_string()),
    };
    let result = searcher.get_column_lineage(&params).await.json();
    let success = result
        .get("success")
        .expect("response missing 'success' field")
        .as_bool()
        .expect("'success' field should be boolean");
    assert!(success, "Should succeed for valid column lineage request");
    // Verify response structure
    let data = result.get("data").unwrap();
    assert!(data.get("start_entity").is_some());
    assert!(data.get("start_column").is_some());
    assert!(data.get("direction").is_some());
    assert!(data.get("lineage").is_some());
    let lineage = data
        .get("lineage")
        .and_then(|lineage| lineage.as_array())
        .expect("lineage should be an array");
    assert!(
        !lineage.is_empty(),
        "high confidence lineage should produce at least one match"
    );
    // With high confidence, we should only get exact name matches
    for item in lineage {
        let confidence = item.get("confidence").and_then(|c| c.as_str());
        assert_eq!(
            confidence,
            Some("high"),
            "High confidence mode should only return high confidence matches"
        );
    }
}
#[tokio::test(flavor = "multi_thread")]
async fn test_get_column_lineage_downstream() {
    let searcher = get_searcher();
    let test_model = "model.nova_test.stg__traffic_sessions".to_string();
    assert!(
        searcher
            .child_map
            .get(&test_model)
            .is_some_and(|children| !children.is_empty()),
        "fixture should include downstream dependencies for {test_model}"
    );
    let params = GetColumnLineageParams {
        id_or_name: test_model,
        resource_type: None,
        column_name: "session_id".to_string(),
        direction: "downstream".to_string(),
        depth: Some(2),
        confidence: Some("medium".to_string()),
    };
    let result = searcher.get_column_lineage(&params).await.json();
    let success = result
        .get("success")
        .expect("response missing 'success' field")
        .as_bool()
        .expect("'success' field should be boolean");
    assert!(success, "Should succeed for downstream column lineage");
    let data = result.get("data").unwrap();
    assert_eq!(
        data.get("direction").and_then(|d| d.as_str()),
        Some("downstream")
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn test_get_column_lineage_depth_limit() {
    let searcher = get_searcher();
    let test_model = "model.nova_test.int__campaign_features".to_string();
    assert!(
        searcher
            .parent_map
            .get(&test_model)
            .is_some_and(|parents| !parents.is_empty()),
        "fixture should include upstream dependencies for {test_model}"
    );
    let params = GetColumnLineageParams {
        id_or_name: test_model,
        resource_type: None,
        column_name: "campaign_name".to_string(),
        direction: "upstream".to_string(),
        depth: Some(1),
        confidence: Some("low".to_string()),
    };
    let result = searcher.get_column_lineage(&params).await.json();
    let success = result
        .get("success")
        .expect("response missing 'success' field")
        .as_bool()
        .expect("'success' field should be boolean");
    assert!(success, "Should succeed with depth limit");
    // All results should be depth 1
    if let Some(lineage) = result
        .get("data")
        .and_then(|d| d.get("lineage"))
        .and_then(|l| l.as_array())
    {
        for item in lineage {
            let depth = item
                .get("depth")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            assert_eq!(depth, 1, "All results should be at depth 1");
        }
    }
}
#[tokio::test(flavor = "multi_thread")]
async fn test_get_column_lineage_confidence_levels() {
    let searcher = get_searcher();
    let model_id = "model.nova_test.fct__orders".to_string();
    assert!(
        searcher
            .parent_map
            .get(&model_id)
            .is_some_and(|parents| parents.len() >= 2),
        "fixture should include a model with multiple upstream dependencies"
    );
    let column = "order_id".to_string();
    // High confidence
    let params_high = GetColumnLineageParams {
        id_or_name: model_id.clone(),
        resource_type: None,
        column_name: column.clone(),
        direction: "upstream".to_string(),
        depth: Some(3),
        confidence: Some("high".to_string()),
    };
    let result_high = searcher.get_column_lineage(&params_high).await.json();
    // Low confidence
    let params_low = GetColumnLineageParams {
        id_or_name: model_id,
        resource_type: None,
        column_name: column,
        direction: "upstream".to_string(),
        depth: Some(3),
        confidence: Some("low".to_string()),
    };
    let result_low = searcher.get_column_lineage(&params_low).await.json();
    // Low confidence should have >= results than high confidence
    let count_high = result_high
        .get("count")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let count_low = result_low
        .get("count")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    assert!(
        count_low >= count_high,
        "Low confidence should find >= matches than high confidence (high={count_high}, low={count_low})"
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn test_get_column_lineage_response_structure() {
    let searcher = get_searcher();
    let test_model = "model.nova_test.int__campaign_features".to_string();
    assert!(
        searcher.has_entity(&test_model),
        "fixture should contain {test_model}"
    );
    let params = GetColumnLineageParams {
        id_or_name: test_model,
        resource_type: None,
        column_name: "campaign_name".to_string(),
        direction: "upstream".to_string(),
        depth: None,
        confidence: Some("low".to_string()),
    };
    let result = searcher.get_column_lineage(&params).await.json();
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
    let data = result.get("data").unwrap();
    // Check required fields in response
    assert!(data.get("start_entity").is_some());
    assert!(data.get("start_column").is_some());
    assert!(data.get("direction").is_some());
    assert!(data.get("lineage").is_some());
    // Check lineage item structure (if any results)
    if let Some(lineage) = data.get("lineage").and_then(|l| l.as_array())
        && !lineage.is_empty()
    {
        let item = &lineage[0];
        assert!(
            item.get("unique_id").is_some(),
            "lineage item should have unique_id"
        );
        assert!(item.get("name").is_some(), "lineage item should have name");
        assert!(
            item.get("column").is_some(),
            "lineage item should have column"
        );
        assert!(
            item.get("confidence").is_some(),
            "lineage item should have confidence"
        );
        assert!(
            item.get("match_reason").is_some(),
            "lineage item should have match_reason"
        );
        assert!(
            item.get("depth").is_some(),
            "lineage item should have depth"
        );
        assert!(item.get("path").is_some(), "lineage item should have path");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_get_column_lineage_ambiguous_name_returns_error() {
    let searcher = get_searcher_with_fixture("ambiguous_name.json");
    let ambiguous_name = "duplicate_entity".to_string();

    let params = GetColumnLineageParams {
        id_or_name: ambiguous_name,
        resource_type: None,
        column_name: "id".to_string(),
        direction: "upstream".to_string(),
        depth: Some(1),
        confidence: Some("medium".to_string()),
    };
    let result = searcher.get_column_lineage(&params).await;
    match result {
        Err(DbtNovaError::AmbiguousName { .. }) => {}
        other => panic!("expected AmbiguousName error, got: {other:?}"),
    }
}
