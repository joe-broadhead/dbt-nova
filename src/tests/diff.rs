//! Tests for `diff_entities` tool responses.
use super::common::*;

// Diff Entities Tests
#[tokio::test(flavor = "multi_thread")]
async fn test_diff_entities_same() {
    let searcher = get_searcher();
    let params = DiffEntitiesParams {
        entity1: "int__campaign_features".to_string(),
        entity1_resource_type: None,
        entity2: "int__campaign_features".to_string(),
        entity2_resource_type: None,
        compare_fields: vec!["columns".to_string()],
    };
    let result = searcher.diff_entities(&params).await.json();
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
    // Same entity should have no differences
    if let Some(diffs) = result
        .get("data")
        .and_then(|data| data.get("differences"))
        .and_then(|d| d.get("columns"))
    {
        let only_first = diffs.get("only_in_first").and_then(|a| a.as_array());
        let only_second = diffs.get("only_in_second").and_then(|a| a.as_array());
        assert!(
            only_first.is_none_or(Vec::is_empty),
            "Should have no differences"
        );
        assert!(
            only_second.is_none_or(Vec::is_empty),
            "Should have no differences"
        );
    }
}
#[tokio::test(flavor = "multi_thread")]
async fn test_diff_entities_not_found() {
    let searcher = get_searcher();
    let params = DiffEntitiesParams {
        entity1: "nonexistent1".to_string(),
        entity1_resource_type: None,
        entity2: "int__campaign_features".to_string(),
        entity2_resource_type: None,
        compare_fields: vec!["columns".to_string()],
    };
    let result = searcher.diff_entities(&params).await.json();
    let success = result
        .get("success")
        .expect("response missing 'success' field")
        .as_bool()
        .expect("'success' field should be boolean");
    assert!(!success, "Should fail when entity not found");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_diff_entities_ambiguous_name_returns_error() {
    let searcher = get_searcher();
    let Some((ambiguous_name, matches)) = searcher
        .name_to_keys
        .iter()
        .find(|(_, keys)| keys.len() > 1)
        .map(|(name, keys)| (name.clone(), keys.clone()))
    else {
        println!("Skipping ambiguity test: no ambiguous names in fixture");
        return;
    };

    let params = DiffEntitiesParams {
        entity1: ambiguous_name,
        entity1_resource_type: None,
        entity2: matches[0].clone(),
        entity2_resource_type: None,
        compare_fields: vec!["columns".to_string()],
    };
    let result = searcher.diff_entities(&params).await;
    match result {
        Err(DbtNovaError::AmbiguousName { .. }) => {}
        other => panic!("expected AmbiguousName error, got: {other:?}"),
    }
}
