//! Edge case tests for boundary conditions and inputs.
use super::common::*;

#[tokio::test(flavor = "multi_thread")]
async fn test_empty_query() {
    let searcher = get_searcher();
    let params = SearchParams {
        query: String::new(),
        ..Default::default()
    };
    let result = searcher.search(&params).await.json();
    let success = result
        .get("success")
        .expect("response missing 'success' field")
        .as_bool()
        .expect("'success' field should be boolean");

    if success {
        let data = result
            .get("data")
            .expect("response missing 'data'")
            .as_array()
            .expect("'data' should be array");
        assert!(data.is_empty(), "Empty query should return no results");
    } else {
        assert!(
            result.get("error").is_some(),
            "Expected error for empty query"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_max_limit() {
    let searcher = get_searcher();
    let params = SearchParams {
        query: "model".to_string(),
        pagination: PaginationParams {
            limit: Some(10_000),
            offset: 0,
        },
        ..Default::default()
    };
    let result = searcher.search(&params).await.json();
    let success = result
        .get("success")
        .expect("response missing 'success' field")
        .as_bool()
        .expect("'success' field should be boolean");
    assert!(success, "Expected success=true for search");

    let data = result
        .get("data")
        .expect("response missing 'data'")
        .as_array()
        .expect("'data' should be array");
    let max_page = searcher.config().search.max_page_size.max(1);
    assert!(
        data.len() <= max_page,
        "Should cap at max_page_size ({}), got {}",
        max_page,
        data.len()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_special_characters_in_query() {
    let searcher = get_searcher();
    let special = ["*", "?", "[", "]", "(", ")", "\"", "'", "\\"];

    for token in special {
        let params = SearchParams {
            query: token.to_string(),
            ..Default::default()
        };
        let result = searcher.search(&params).await.json();
        let success = result
            .get("success")
            .expect("response missing 'success' field")
            .as_bool()
            .expect("'success' field should be boolean");
        if !success {
            assert!(
                result.get("error").is_some(),
                "Expected error for special token: {token}"
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_deeply_nested_lineage() {
    let searcher = get_searcher();
    let params = GetLineageParams {
        id_or_name: "model.nova_test.int__campaign_features".to_string(),
        direction: "upstream".to_string(),
        depth: Some(200),
        resource_types: vec![],
        detail: Some(DetailLevel::Standard),
    };
    let result = searcher.get_lineage(&params).await.json();
    let success = result
        .get("success")
        .expect("response missing 'success' field")
        .as_bool()
        .expect("'success' field should be boolean");
    if !success {
        assert!(result.get("error").is_some(), "Expected error or data");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_nonexistent_entity() {
    let searcher = get_searcher();
    let params = GetEntityParams {
        id_or_name: "model.fake.does_not_exist_12345".to_string(),
        resource_type: None,
        detail: Some(DetailLevel::Standard),
    };
    let result = searcher.get_entity_data(&params).await.json();
    let success = result
        .get("success")
        .expect("response missing 'success' field")
        .as_bool()
        .expect("'success' field should be boolean");
    assert!(!success, "Should return error for nonexistent entity");
}
