//! Tests for `get_test_coverage` tool responses.
use super::common::*;

fn first_model_with_tests(searcher: &ManifestSearch) -> String {
    searcher
        .tests_by_entity
        .iter()
        .find_map(|(entity_id, tests)| {
            (entity_id.starts_with("model.") && !tests.is_empty() && searcher.has_entity(entity_id))
                .then(|| entity_id.clone())
        })
        .expect("fixture should include at least one model with tests")
}

async fn first_model_name_with_tests(searcher: &ManifestSearch) -> String {
    for entity_id in searcher.tests_by_entity.keys() {
        if !entity_id.starts_with("model.") {
            continue;
        }
        if let Some(entity) = searcher.get_entity(entity_id).await.unwrap()
            && let Some(name) = entity.name.as_deref()
        {
            return name.to_string();
        }
    }
    panic!("fixture should include a named model with tests");
}

// Test Coverage Tests
#[tokio::test(flavor = "multi_thread")]
async fn test_get_test_coverage_entity_not_found() {
    let searcher = get_searcher();
    let params = GetTestCoverageParams {
        id_or_name: "nonexistent_model".to_string(),
        resource_type: None,
        include_full: true,
        columns_limit: Some(50),
    };
    let result = searcher.get_test_coverage(&params).await.json();
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
async fn test_get_test_coverage_response_structure() {
    let searcher = get_searcher();
    let model_id = first_model_with_tests(&searcher);
    let params = GetTestCoverageParams {
        id_or_name: model_id.clone(),
        resource_type: None,
        include_full: true,
        columns_limit: Some(50),
    };
    let result = searcher.get_test_coverage(&params).await.json();
    let success = result
        .get("success")
        .expect("response missing 'success' field")
        .as_bool()
        .expect("'success' field should be boolean");
    assert!(
        success,
        "Failed for model '{}': {:?}",
        model_id,
        result.get("error")
    );
    let data = result.get("data").unwrap();
    // Check required top-level fields
    assert!(data.get("unique_id").is_some());
    assert!(data.get("name").is_some());
    assert!(data.get("resource_type").is_some());
    assert!(data.get("summary").is_some());
    assert!(data.get("schema_tests").is_some());
    assert!(data.get("data_tests").is_some());
    assert!(data.get("columns_without_tests").is_some());
    assert!(data.get("columns_without_tests_truncated").is_some());
    assert!(data.get("columns_without_tests_total").is_some());
    assert!(data.get("coverage_gaps").is_some());
    // Check summary structure
    let summary = data.get("summary").unwrap();
    assert!(summary.get("total_tests").is_some());
    assert!(summary.get("schema_tests").is_some());
    assert!(summary.get("data_tests").is_some());
    assert!(summary.get("test_types").is_some());
    assert!(summary.get("columns_tested").is_some());
    assert!(summary.get("columns_total").is_some());
    assert!(summary.get("coverage_percentage").is_some());
}
#[tokio::test(flavor = "multi_thread")]
async fn test_get_test_coverage_include_full_false() {
    let searcher = get_searcher();
    let model_id = first_model_with_tests(&searcher);
    let params = GetTestCoverageParams {
        id_or_name: model_id.clone(),
        resource_type: None,
        include_full: false,
        columns_limit: Some(50),
    };
    let result = searcher.get_test_coverage(&params).await.json();
    let success = result
        .get("success")
        .expect("response missing 'success' field")
        .as_bool()
        .expect("'success' field should be boolean");
    assert!(
        success,
        "Failed for model '{}': {:?}",
        model_id,
        result.get("error")
    );
    // With include_full=false, schema_tests items should NOT have test_metadata
    if let Some(schema_tests) = result
        .get("data")
        .and_then(|d| d.get("schema_tests"))
        .and_then(|s| s.as_array())
        && !schema_tests.is_empty()
    {
        let first = &schema_tests[0];
        // Should have basic fields
        assert!(first.get("unique_id").is_some());
        assert!(first.get("test_type").is_some());
        // Should NOT have full metadata when include_full=false
        assert!(
            first.get("test_metadata").is_none(),
            "include_full=false should not include test_metadata"
        );
    }
}
#[tokio::test(flavor = "multi_thread")]
async fn test_tests_by_entity_index_populated() {
    let searcher = get_searcher();
    // Verify tests_by_entity index is populated
    assert!(
        !searcher.tests_by_entity.is_empty(),
        "tests_by_entity should not be empty"
    );
    // Count total tests indexed
    let total_indexed: usize = searcher.tests_by_entity.values().map(Vec::len).sum();
    assert!(total_indexed > 0, "Should have tests indexed by entity");
}
#[tokio::test(flavor = "multi_thread")]
async fn test_tests_by_column_index_populated() {
    let searcher = get_searcher();
    // Verify tests_by_column index is populated (for schema tests)
    assert!(
        !searcher.tests_by_column.is_empty(),
        "tests_by_column should not be empty"
    );
    // Keys should be in format "entity_id:column_name"
    for key in searcher.tests_by_column.keys() {
        assert!(
            key.contains(':'),
            "Column test key should be in format 'entity_id:column_name'"
        );
    }
}
#[tokio::test(flavor = "multi_thread")]
async fn test_get_test_coverage_test_types() {
    let searcher = get_searcher();
    let model_id = first_model_with_tests(&searcher);
    let params = GetTestCoverageParams {
        id_or_name: model_id.clone(),
        resource_type: None,
        include_full: true,
        columns_limit: Some(50),
    };
    let result = searcher.get_test_coverage(&params).await.json();
    let success = result
        .get("success")
        .expect("response missing 'success' field")
        .as_bool()
        .expect("'success' field should be boolean");
    assert!(
        success,
        "get_test_coverage failed for model '{}': {:?}",
        model_id,
        result.get("error")
    );
    // Check test_types breakdown
    let test_types = result
        .get("data")
        .and_then(|d| d.get("summary"))
        .and_then(|s| s.get("test_types"))
        .and_then(|t| t.as_object());
    assert!(test_types.is_some(), "Should have test_types in summary");
    // Verify test_types values are counts
    if let Some(types) = test_types {
        for (_, count) in types {
            assert!(count.is_u64(), "Test type counts should be integers");
        }
    }
}
#[tokio::test(flavor = "multi_thread")]
async fn test_get_test_coverage_coverage_percentage() {
    let searcher = get_searcher();
    // Find any model
    let models = searcher.by_resource_type.get("model").unwrap();
    let model_id = &models[0];
    let params = GetTestCoverageParams {
        id_or_name: model_id.clone(),
        resource_type: None,
        include_full: false,
        columns_limit: Some(50),
    };
    let result = searcher.get_test_coverage(&params).await.json();
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
    let coverage_pct = result
        .get("data")
        .and_then(|d| d.get("summary"))
        .and_then(|s| s.get("coverage_percentage"))
        .and_then(serde_json::Value::as_u64);
    assert!(coverage_pct.is_some(), "Should have coverage_percentage");
    let pct = coverage_pct.unwrap();
    assert!(pct <= 100, "Coverage percentage should be <= 100");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_get_test_coverage_zero_columns_reports_zero_percentage() {
    let searcher = get_searcher();

    let mut model_id = None;
    let models = searcher
        .by_resource_type
        .get("model")
        .expect("fixture should expose model entities");
    for candidate in models {
        if let Some(entity) = searcher.get_entity(candidate).await.unwrap()
            && entity.column_names().is_empty()
        {
            model_id = Some(candidate.clone());
            break;
        }
    }
    let model_id = model_id.expect("fixture should include at least one zero-column model");

    let params = GetTestCoverageParams {
        id_or_name: model_id,
        resource_type: None,
        include_full: false,
        columns_limit: Some(50),
    };
    let result = searcher.get_test_coverage(&params).await.json();
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

    let columns_total = result
        .get("data")
        .and_then(|d| d.get("summary"))
        .and_then(|s| s.get("columns_total"))
        .and_then(serde_json::Value::as_u64);
    assert_eq!(columns_total, Some(0));

    let coverage_pct = result
        .get("data")
        .and_then(|d| d.get("summary"))
        .and_then(|s| s.get("coverage_percentage"))
        .and_then(serde_json::Value::as_u64);
    assert_eq!(coverage_pct, Some(0));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_get_test_coverage_by_name() {
    let searcher = get_searcher();
    let model_name = first_model_name_with_tests(&searcher).await;
    let params = GetTestCoverageParams {
        id_or_name: model_name,
        resource_type: None,
        include_full: false,
        columns_limit: Some(50),
    };
    let result = searcher.get_test_coverage(&params).await.json();
    // Should succeed (resolves name to unique_id)
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
async fn test_get_test_coverage_ambiguous_name_returns_error() {
    let searcher = get_searcher_with_fixture("ambiguous_name.json");
    let ambiguous_name = "duplicate_entity".to_string();

    let params = GetTestCoverageParams {
        id_or_name: ambiguous_name,
        resource_type: None,
        include_full: false,
        columns_limit: Some(50),
    };
    let result = searcher.get_test_coverage(&params).await;
    match result {
        Err(DbtNovaError::AmbiguousName { .. }) => {}
        other => panic!("expected AmbiguousName error, got: {other:?}"),
    }
}
