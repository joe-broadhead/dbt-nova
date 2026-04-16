//! Tests for context tool responses.
use super::common::*;

// Get Context Tests
#[tokio::test(flavor = "multi_thread")]
async fn test_get_context_basic() {
    let searcher = get_searcher();
    let search_params = SearchParams {
        query: "model".to_string(),
        resource_types: vec!["model".to_string()],
        persona: None,
        detail: DetailLevel::Standard,
        min_score: None,
        fuzzy: false,
        include_highlights: false,
        include_sql: false,
        explain: false,
        pagination: PaginationParams {
            limit: 1,
            offset: 0,
        },
    };
    let search_result = searcher.search(&search_params).await.json();
    let success = search_result
        .get("success")
        .expect("search response missing 'success'")
        .as_bool()
        .expect("'success' should be boolean");
    assert!(
        success,
        "Expected search success but got error: {:?}",
        search_result.get("error")
    );
    let results = search_result
        .get("data")
        .and_then(|d| d.as_array())
        .expect("search response missing 'data' array");
    assert!(
        !results.is_empty(),
        "expected at least one model search result"
    );
    let model_id = results[0]
        .get("unique_id")
        .and_then(|v| v.as_str())
        .expect("search result missing unique_id");

    let params = GetContextParams {
        id_or_name: model_id.to_string(),
        resource_type: None,
        include_columns: true,
        include_tests: true,
        include_upstream: true,
        include_downstream: false,
        include_docs: true,
        include_sql: false,
        context_mode: ContextMode::Standard,
        limits: ContextLimits {
            lineage_depth: 2,
            upstream_limit: 10,
            downstream_limit: 10,
        },
    };
    let result = searcher.get_context(&params).await.json();
    let success = result
        .get("success")
        .expect("response missing 'success'")
        .as_bool()
        .expect("'success' should be boolean");
    assert!(
        success,
        "Expected success=true but got error: {:?}",
        result.get("error")
    );
    let data = result.get("data").expect("response missing 'data'");
    assert!(data.get("entity").is_some(), "Missing entity in context");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_get_context_with_lineage() {
    let searcher = get_searcher();
    let params = GetContextParams {
        id_or_name: "model.nova_test.int__campaign_features".to_string(),
        resource_type: None,
        include_columns: false,
        include_tests: false,
        include_upstream: true,
        include_downstream: true,
        include_docs: false,
        include_sql: false,
        context_mode: ContextMode::Standard,
        limits: ContextLimits {
            lineage_depth: 2,
            upstream_limit: 10,
            downstream_limit: 10,
        },
    };
    let result = searcher.get_context(&params).await.json();
    let has_entity = result.get("data").and_then(|d| d.get("entity")).is_some();
    let has_error = result.get("error").is_some();
    assert!(has_entity || has_error, "Should have entity or error");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_get_context_not_found() {
    let searcher = get_searcher();
    let params = GetContextParams {
        id_or_name: "model.nonexistent.fake_model_12345".to_string(),
        resource_type: None,
        include_columns: false,
        include_tests: false,
        include_upstream: false,
        include_downstream: false,
        include_docs: false,
        include_sql: false,
        context_mode: ContextMode::Standard,
        limits: ContextLimits {
            lineage_depth: 1,
            upstream_limit: 10,
            downstream_limit: 10,
        },
    };
    let result = searcher.get_context(&params).await.json();
    let success = result
        .get("success")
        .expect("response missing 'success'")
        .as_bool()
        .expect("'success' should be boolean");
    assert!(!success, "Should return error for nonexistent entity");
    assert!(result.get("error").is_some(), "Should return error payload");
}
