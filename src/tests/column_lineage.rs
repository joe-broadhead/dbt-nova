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
    // Find a model with at least one column to exercise validation on direction only.
    let models = searcher.by_resource_type.get("model").unwrap();
    let mut model_with_columns = None;
    let mut column_name = String::new();
    for model_id in models {
        if let Some(entity) = searcher.get_entity(model_id).await.unwrap() {
            let cols = entity.column_names();
            if !cols.is_empty() {
                model_with_columns = Some(model_id.clone());
                column_name = cols[0].clone();
                break;
            }
        }
    }
    let model_id = model_with_columns.expect("Should find a model with columns");
    let params = GetColumnLineageParams {
        id_or_name: model_id,
        resource_type: None,
        column_name,
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
    let searcher = get_searcher();
    // Find a model with upstream dependencies and a shared column name to avoid flaky matches.
    // Prefer int_* models because they usually depend on upstream staging models.
    let models = searcher.by_resource_type.get("model").unwrap();
    let mut test_model = None;
    let mut test_column = None;
    for model_id in models {
        // Skip if no upstream deps
        let upstream = searcher.parent_map.get(model_id);
        if upstream.is_none() || upstream.unwrap().is_empty() {
            continue;
        }
        if let Some(entity) = searcher.get_entity(model_id).await.unwrap() {
            let cols = entity.column_names();
            // Find a column that might exist upstream
            for col_name in &cols {
                // Check if this column exists in any upstream entity
                for upstream_id in upstream.unwrap() {
                    if let Some(upstream_entity) = searcher.get_entity(upstream_id).await.unwrap() {
                        let upstream_cols = upstream_entity.column_names();
                        if upstream_cols.iter().any(|c| c == col_name) {
                            test_model = Some(model_id.clone());
                            test_column = Some(col_name.clone());
                            break;
                        }
                    }
                }
                if test_column.is_some() {
                    break;
                }
            }
        }
        if test_model.is_some() {
            break;
        }
    }
    if test_model.is_none() {
        // Skip test if we can't find a suitable model
        println!(
            "Skipping test_get_column_lineage_upstream_high_confidence: Some(no suitable model found"
        );
        return;
    }
    let params = GetColumnLineageParams {
        id_or_name: test_model.unwrap(),
        resource_type: None,
        column_name: test_column.unwrap(),
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
    // With high confidence, we should only get exact name matches
    if let Some(lineage) = data.get("lineage").and_then(|l| l.as_array()) {
        for item in lineage {
            let confidence = item.get("confidence").and_then(|c| c.as_str());
            assert_eq!(
                confidence,
                Some("high"),
                "High confidence mode should only return high confidence matches"
            );
        }
    }
}
#[tokio::test(flavor = "multi_thread")]
async fn test_get_column_lineage_downstream() {
    let searcher = get_searcher();
    // Find a source with columns and downstream dependencies to validate downstream traversal.
    let sources = searcher.by_resource_type.get("source");
    if sources.is_none() {
        println!("Skipping test_get_column_lineage_downstream: no sources found");
        return;
    }
    let mut test_source = None;
    let mut test_column = None;
    for source_id in sources.unwrap() {
        // Check if source has downstream
        let downstream = searcher.child_map.get(source_id);
        if downstream.is_none() || downstream.unwrap().is_empty() {
            continue;
        }
        if let Some(entity) = searcher.get_entity(source_id).await.unwrap() {
            let cols = entity.column_names();
            if !cols.is_empty() {
                test_source = Some(source_id.clone());
                test_column = Some(cols[0].clone());
                break;
            }
        }
    }
    if test_source.is_none() {
        println!("Skipping test_get_column_lineage_downstream: no suitable source found");
        return;
    }
    let params = GetColumnLineageParams {
        id_or_name: test_source.unwrap(),
        resource_type: None,
        column_name: test_column.clone().unwrap(),
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
    // Find any model with columns and upstream deps
    let models = searcher.by_resource_type.get("model").unwrap();
    let mut test_model = None;
    let mut test_column = None;
    for model_id in models {
        let upstream = searcher.parent_map.get(model_id);
        if upstream.is_none() || upstream.unwrap().is_empty() {
            continue;
        }
        if let Some(entity) = searcher.get_entity(model_id).await.unwrap() {
            let cols = entity.column_names();
            if !cols.is_empty() {
                test_model = Some(model_id.clone());
                test_column = Some(cols[0].clone());
                break;
            }
        }
    }
    if test_model.is_none() {
        println!("Skipping test_get_column_lineage_depth_limit: no suitable model found");
        return;
    }
    let params = GetColumnLineageParams {
        id_or_name: test_model.unwrap(),
        resource_type: None,
        column_name: test_column.unwrap(),
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
    // Test that different confidence levels return different amounts of results
    let models = searcher.by_resource_type.get("model").unwrap();
    let mut test_model = None;
    let mut test_column = None;
    for model_id in models {
        let upstream = searcher.parent_map.get(model_id);
        if upstream.is_none() || upstream.unwrap().len() < 2 {
            continue;
        }
        if let Some(entity) = searcher.get_entity(model_id).await.unwrap() {
            let cols = entity.column_names();
            if cols.len() >= 3 {
                test_model = Some(model_id.clone());
                test_column = Some(cols[0].clone());
                break;
            }
        }
    }
    if test_model.is_none() {
        println!("Skipping test_get_column_lineage_confidence_levels: no suitable model found");
        return;
    }
    let model_id = test_model.unwrap();
    let column = test_column.unwrap();
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
    // Find any model with columns
    let models = searcher.by_resource_type.get("model").unwrap();
    let mut test_model = None;
    let mut test_column = None;
    for model_id in models {
        if let Some(entity) = searcher.get_entity(model_id).await.unwrap() {
            let cols = entity.column_names();
            if !cols.is_empty() {
                test_model = Some(model_id.clone());
                test_column = Some(cols[0].clone());
                break;
            }
        }
    }
    let params = GetColumnLineageParams {
        id_or_name: test_model.unwrap(),
        resource_type: None,
        column_name: test_column.clone().unwrap(),
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
    let searcher = get_searcher();
    let Some(ambiguous_name) = searcher
        .name_to_keys
        .iter()
        .find(|(_, keys)| keys.len() > 1)
        .map(|(name, _)| name.clone())
    else {
        println!("Skipping ambiguity test: no ambiguous names in fixture");
        return;
    };

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
