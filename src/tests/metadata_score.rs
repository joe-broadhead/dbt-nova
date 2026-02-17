//! Tests for metadata scoring logic and outputs.
use super::common::*;
use crate::tools::metadata_score::{array_tier_score, description_tier_score, grade_from_score};

#[tokio::test(flavor = "multi_thread")]
async fn test_metadata_score_entity_not_found() {
    let searcher = get_searcher();
    let params = GetMetadataScoreParams {
        id_or_name: Some("nonexistent_entity".to_string()),
        resource_type: None,
        persona: None,
        scope: Some("entity".to_string()),
        include_breakdown: true,
        include_recommendations: true,
        resource_types: Vec::new(),
        limit: Some(DEFAULT_METADATA_SCORE_LIMIT),
        offset: None,
    };
    let result = searcher.get_metadata_score(&params).await.json();
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
async fn test_metadata_score_response_structure() {
    let searcher = get_searcher();
    let params = GetMetadataScoreParams {
        id_or_name: Some("model.nova_test.int__campaign_features".to_string()),
        resource_type: None,
        persona: Some("analyst".to_string()),
        scope: Some("entity".to_string()),
        include_breakdown: true,
        include_recommendations: true,
        resource_types: Vec::new(),
        limit: Some(DEFAULT_METADATA_SCORE_LIMIT),
        offset: None,
    };
    let result = searcher.get_metadata_score(&params).await.json();
    let success = result
        .get("success")
        .expect("response missing 'success' field")
        .as_bool()
        .expect("'success' field should be boolean");
    assert!(
        success,
        "Expected success response, got {:?}",
        result.get("error")
    );
    let data = result.get("data").unwrap();
    assert!(data.get("unique_id").is_some());
    assert!(data.get("overall_score").is_some());
    assert!(data.get("grade").is_some());
    assert!(data.get("categories").is_some());
    assert!(data.get("breakdown").is_some());
    assert!(data.get("recommendations").is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_metadata_score_persona_weights() {
    let searcher = get_searcher();
    let params = GetMetadataScoreParams {
        id_or_name: Some("model.nova_test.int__campaign_features".to_string()),
        resource_type: None,
        persona: Some("engineer".to_string()),
        scope: Some("entity".to_string()),
        include_breakdown: false,
        include_recommendations: false,
        resource_types: Vec::new(),
        limit: Some(DEFAULT_METADATA_SCORE_LIMIT),
        offset: None,
    };
    let result = searcher.get_metadata_score(&params).await.json();
    let data = result.get("data").unwrap();
    let categories = data.get("categories").unwrap();
    let documentation = categories.get("documentation").unwrap();
    let weight = documentation
        .get("weight")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(0.0);
    assert!(
        (weight - 0.20).abs() < 0.0001,
        "Expected engineer documentation weight 0.20, got {weight}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_metadata_score_project_scope() {
    let searcher = get_searcher();
    let params = GetMetadataScoreParams {
        id_or_name: None,
        resource_type: None,
        persona: None,
        scope: Some("project".to_string()),
        include_breakdown: false,
        include_recommendations: false,
        resource_types: vec!["model".to_string()],
        // Explicitly cap results to validate truncation behavior for project scope.
        limit: Some(2),
        offset: None,
    };
    let result = searcher.get_metadata_score(&params).await.json();
    let success = result
        .get("success")
        .expect("response missing 'success' field")
        .as_bool()
        .expect("'success' field should be boolean");
    assert!(success, "Expected success response");
    let data = result.get("data").unwrap();
    let entities = data.get("entities").and_then(|e| e.as_array()).unwrap();
    assert!(entities.len() <= 2);
    assert!(data.get("overall_score").is_some());
    assert!(
        result.get("total_available").is_some(),
        "Expected total_available for project scope"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_metadata_score_project_scope_is_deterministic_and_sorted() {
    let searcher = get_searcher();
    let params = GetMetadataScoreParams {
        scope: Some("project".to_string()),
        include_breakdown: false,
        include_recommendations: false,
        resource_types: vec!["model".to_string()],
        limit: Some(5),
        offset: Some(1),
        ..Default::default()
    };

    let result1 = searcher.get_metadata_score(&params).await.json();
    let result2 = searcher.get_metadata_score(&params).await.json();
    let entities1 = result1["data"]["entities"].as_array().unwrap();
    let entities2 = result2["data"]["entities"].as_array().unwrap();
    assert_eq!(
        entities1, entities2,
        "Project scope should be deterministic"
    );

    let unique_ids: Vec<&str> = entities1
        .iter()
        .filter_map(|entity| entity["unique_id"].as_str())
        .collect();
    let mut sorted_ids = unique_ids.clone();
    sorted_ids.sort_unstable();
    assert_eq!(
        unique_ids, sorted_ids,
        "Project scope entities should be sorted by unique_id"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_metadata_score_project_scope_offset_pages_results() {
    let searcher = get_searcher();
    let page_one = GetMetadataScoreParams {
        scope: Some("project".to_string()),
        include_breakdown: false,
        include_recommendations: false,
        resource_types: vec!["model".to_string()],
        limit: Some(2),
        offset: Some(0),
        ..Default::default()
    };
    let result1 = searcher.get_metadata_score(&page_one).await.json();
    let page_two = GetMetadataScoreParams {
        scope: Some("project".to_string()),
        include_breakdown: false,
        include_recommendations: false,
        resource_types: vec!["model".to_string()],
        limit: Some(2),
        offset: Some(1),
        ..Default::default()
    };
    let result2 = searcher.get_metadata_score(&page_two).await.json();
    let entities1 = result1["data"]["entities"].as_array().unwrap();
    let entities2 = result2["data"]["entities"].as_array().unwrap();
    assert!(
        entities1.len() >= 2 && !entities2.is_empty(),
        "Expected enough entities to validate pagination"
    );
    assert_eq!(
        entities1[1]["unique_id"].as_str(),
        entities2[0]["unique_id"].as_str(),
        "Offset pagination should shift results deterministically"
    );
}

#[test]
fn test_description_tier_scoring() {
    assert_eq!(description_tier_score("", 30), 0);
    assert_eq!(description_tier_score("Short desc", 30), 6); // 1-19 chars
    assert_eq!(
        description_tier_score("A brief description of the model", 30),
        15
    ); // 20-49
    assert_eq!(description_tier_score(&"x".repeat(75), 30), 24); // 50-99
    assert_eq!(description_tier_score(&"x".repeat(150), 30), 30); // 100+
}

#[test]
fn test_array_tier_scoring() {
    assert_eq!(array_tier_score(0, 15), 0);
    assert_eq!(array_tier_score(1, 15), 6);
    assert_eq!(array_tier_score(2, 15), 11); // 0.7 * 15 = 10.5 rounded
    assert_eq!(array_tier_score(3, 15), 15);
    assert_eq!(array_tier_score(10, 15), 15); // 3+ all same
}

#[test]
fn test_grade_boundaries() {
    assert_eq!(grade_from_score(100), "A");
    assert_eq!(grade_from_score(90), "A");
    assert_eq!(grade_from_score(89), "B");
    assert_eq!(grade_from_score(80), "B");
    assert_eq!(grade_from_score(79), "C");
    assert_eq!(grade_from_score(70), "C");
    assert_eq!(grade_from_score(69), "D");
    assert_eq!(grade_from_score(60), "D");
    assert_eq!(grade_from_score(59), "F");
    assert_eq!(grade_from_score(0), "F");
}

#[tokio::test]
async fn test_measure_quality_scoring() {
    let searcher = get_searcher_with_fixture("measure_test.json");

    // Test case 1: No measures -> semantic score should be 0.
    let params1 = GetMetadataScoreParams {
        id_or_name: Some("model.nova_test.model_extra_a".to_string()),
        ..Default::default()
    };
    let result1 = searcher.get_metadata_score(&params1).await.json();
    let score1 = result1["data"]["categories"]["semantic"]["score"]
        .as_u64()
        .unwrap();
    assert_eq!(score1, 0);

    // Test case 2: Measures without expressions.
    // base_score=6, scale_score(6, 15, 12) = round(6/15*12) = 5.
    let params2 = GetMetadataScoreParams {
        id_or_name: Some("model.nova_test.measures_no_expression".to_string()),
        ..Default::default()
    };
    let result2 = searcher.get_metadata_score(&params2).await.json();
    let score2 = result2["data"]["categories"]["semantic"]["score"]
        .as_u64()
        .unwrap();
    assert_eq!(score2, 5);

    // Test case 3: Measures with expressions.
    // base_score=10, scale_score(10, 15, 12) = round(10/15*12) = 8.
    let params3 = GetMetadataScoreParams {
        id_or_name: Some("model.nova_test.measures_with_expression".to_string()),
        ..Default::default()
    };
    let result3 = searcher.get_metadata_score(&params3).await.json();
    let score3 = result3["data"]["categories"]["semantic"]["score"]
        .as_u64()
        .unwrap();
    assert_eq!(score3, 8);

    // Test case 4: Measures with expressions AND synonyms.
    // base_score=15, scale_score(15, 15, 12) = 12.
    let params4 = GetMetadataScoreParams {
        id_or_name: Some("model.nova_test.measures_with_expression_and_synonyms".to_string()),
        ..Default::default()
    };
    let result4 = searcher.get_metadata_score(&params4).await.json();
    let score4 = result4["data"]["categories"]["semantic"]["score"]
        .as_u64()
        .unwrap();
    assert_eq!(score4, 12);
}

#[tokio::test]
async fn test_metric_quality_scoring() {
    let searcher = get_searcher_with_fixture("metric_test.json");

    // Test case 1: No metrics
    let params1 = GetMetadataScoreParams {
        id_or_name: Some("model.nova_test.model_extra_a".to_string()),
        ..Default::default()
    };
    let result1 = searcher.get_metadata_score(&params1).await.json();
    let score1 = result1["data"]["categories"]["semantic"]["score"]
        .as_u64()
        .unwrap();
    assert_eq!(score1, 0);

    // Test case 2: Metrics, no expressions
    // base_score=5, scale_score(5, 15, 12) = round(5/15*12) = 4
    let params2 = GetMetadataScoreParams {
        id_or_name: Some("model.nova_test.metrics_no_expression".to_string()),
        ..Default::default()
    };
    let result2 = searcher.get_metadata_score(&params2).await.json();
    let score2 = result2["data"]["categories"]["semantic"]["score"]
        .as_u64()
        .unwrap();
    assert_eq!(score2, 4);

    // Test case 3: Metrics with expressions
    // base_score=10, scale_score(10, 15, 12) = round(10/15*12) = 8
    let params3 = GetMetadataScoreParams {
        id_or_name: Some("model.nova_test.metrics_with_expression".to_string()),
        ..Default::default()
    };
    let result3 = searcher.get_metadata_score(&params3).await.json();
    let score3 = result3["data"]["categories"]["semantic"]["score"]
        .as_u64()
        .unwrap();
    assert_eq!(score3, 8);

    // Test case 4: Metrics with expressions AND synonyms
    // base_score=15, scale_score(15, 15, 12) = 12
    let params4 = GetMetadataScoreParams {
        id_or_name: Some("model.nova_test.metrics_with_expression_and_synonyms".to_string()),
        ..Default::default()
    };
    let result4 = searcher.get_metadata_score(&params4).await.json();
    let score4 = result4["data"]["categories"]["semantic"]["score"]
        .as_u64()
        .unwrap();
    assert_eq!(score4, 12);
}

#[tokio::test]
async fn test_metadata_score_perfect_entity() {
    let searcher = get_searcher_with_fixture("perfect_model.json");
    let params = GetMetadataScoreParams {
        id_or_name: Some("model.nova_test.perfect_model".to_string()),
        ..Default::default()
    };
    let result = searcher.get_metadata_score(&params).await.json();
    let score = result["data"]["overall_score"].as_u64().unwrap();
    let grade = result["data"]["grade"].as_str().unwrap();
    assert!(score >= 95, "Expected score >= 95, got {score}");
    assert_eq!(grade, "A");
}

#[tokio::test]
async fn test_metadata_score_empty_metadata() {
    let searcher = get_searcher();
    let params = GetMetadataScoreParams {
        id_or_name: Some("model.nova_test.model_extra_a".to_string()),
        ..Default::default()
    };
    let result = searcher.get_metadata_score(&params).await.json();
    let score = result["data"]["overall_score"].as_u64().unwrap();
    let grade = result["data"]["grade"].as_str().unwrap();
    assert!(score <= 20, "Expected score <= 20, got {score}");
    assert_eq!(grade, "F");
}

#[tokio::test]
async fn test_persona_score_differences() {
    let searcher = get_searcher();
    let entity = "model.nova_test.int__campaign_features";

    let analyst_params = GetMetadataScoreParams {
        id_or_name: Some(entity.to_string()),
        persona: Some("analyst".to_string()),
        ..Default::default()
    };
    let analyst_result = searcher.get_metadata_score(&analyst_params).await.json();
    let analyst_score = analyst_result["data"]["overall_score"].as_u64().unwrap();

    let engineer_params = GetMetadataScoreParams {
        id_or_name: Some(entity.to_string()),
        persona: Some("engineer".to_string()),
        ..Default::default()
    };
    let engineer_result = searcher.get_metadata_score(&engineer_params).await.json();
    let engineer_score = engineer_result["data"]["overall_score"].as_u64().unwrap();

    let governance_params = GetMetadataScoreParams {
        id_or_name: Some(entity.to_string()),
        persona: Some("governance".to_string()),
        ..Default::default()
    };
    let governance_result = searcher.get_metadata_score(&governance_params).await.json();
    let governance_score = governance_result["data"]["overall_score"].as_u64().unwrap();

    assert!(
        analyst_score != engineer_score || engineer_score != governance_score,
        "Persona weights should produce different scores"
    );
}

#[tokio::test]
async fn test_default_persona_fallback() {
    let searcher = get_searcher();
    let entity = "model.nova_test.int__campaign_features";

    let unknown_params = GetMetadataScoreParams {
        id_or_name: Some(entity.to_string()),
        persona: Some("unknown_persona".to_string()),
        ..Default::default()
    };
    let unknown_result = searcher.get_metadata_score(&unknown_params).await.json();
    let unknown_score = unknown_result["data"]["overall_score"].as_u64().unwrap();

    let default_params = GetMetadataScoreParams {
        id_or_name: Some(entity.to_string()),
        persona: None,
        ..Default::default()
    };
    let default_result = searcher.get_metadata_score(&default_params).await.json();
    let default_score = default_result["data"]["overall_score"].as_u64().unwrap();

    assert_eq!(unknown_score, default_score);
}

#[tokio::test]
async fn test_column_scope_response() {
    let searcher = get_searcher();
    let params = GetMetadataScoreParams {
        id_or_name: Some("model.nova_test.int__campaign_features".to_string()),
        scope: Some("column".to_string()),
        ..Default::default()
    };
    let result = searcher.get_metadata_score(&params).await.json();
    assert!(result["success"].as_bool().unwrap());
    let data = &result["data"];

    // Should have columns array
    let columns = data["columns"].as_array().unwrap();
    assert!(!columns.is_empty());

    // Each column should have scoring structure
    for col in columns {
        assert!(col.get("column").is_some());
        assert!(col.get("overall_score").is_some());
        assert!(col.get("grade").is_some());
        assert!(col.get("categories").is_some());
    }

    // Overall score should be average of column scores
    assert!(data.get("overall_score").is_some());
}

#[tokio::test]
async fn test_model_level_data_test_scoring() {
    let searcher = get_searcher_with_fixture("model_level_test.json");
    let params = GetMetadataScoreParams {
        id_or_name: Some("model.nova_test.model_with_model_level_tests".to_string()),
        include_breakdown: true,
        ..Default::default()
    };
    let result = searcher.get_metadata_score(&params).await.json();
    let quality_breakdown = &result["data"]["breakdown"]["quality"];
    let model_test_score = quality_breakdown["model_test_coverage"]["score"]
        .as_u64()
        .unwrap();
    assert_eq!(model_test_score, 4);
}
