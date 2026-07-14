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
    let searcher = get_searcher_with_fixture("ambiguous_name.json");
    let ambiguous_name = "duplicate_entity".to_string();

    let params = DiffEntitiesParams {
        entity1: ambiguous_name,
        entity1_resource_type: None,
        entity2: "model.pkg.upstream".to_string(),
        entity2_resource_type: None,
        compare_fields: vec!["columns".to_string()],
    };
    let result = searcher.diff_entities(&params).await;
    match result {
        Err(DbtNovaError::AmbiguousName { .. }) => {}
        other => panic!("expected AmbiguousName error, got: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_diff_entities_default_remains_columns_only() {
    let searcher = get_searcher_with_fixture("diff_sections.json");
    let params = DiffEntitiesParams {
        entity1: "model.pkg.sales_monthly_store".to_string(),
        entity1_resource_type: None,
        entity2: "model.pkg.store_productivity_monthly".to_string(),
        entity2_resource_type: None,
        compare_fields: vec![],
    };
    let result = searcher.diff_entities(&params).await.json();

    assert_eq!(result["success"].as_bool(), Some(true), "{result:#?}");
    let differences = result["data"]["differences"]
        .as_object()
        .expect("differences object");
    assert_eq!(
        differences.keys().collect::<Vec<_>>(),
        vec!["columns"],
        "empty compare_fields must preserve the columns-only default"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_diff_entities_all_reports_manifest_native_modelling_sections() {
    let searcher = get_searcher_with_fixture("diff_sections.json");
    let params = DiffEntitiesParams {
        entity1: "model.pkg.sales_monthly_store".to_string(),
        entity1_resource_type: None,
        entity2: "model.pkg.store_productivity_monthly".to_string(),
        entity2_resource_type: None,
        compare_fields: vec!["all".to_string()],
    };
    let result = searcher.diff_entities(&params).await.json();

    assert_eq!(result["success"].as_bool(), Some(true), "{result:#?}");
    let data = &result["data"];
    let differences = &data["differences"];

    assert_eq!(
        differences["grain"]["same_time_field"].as_bool(),
        Some(true)
    );
    assert_eq!(
        differences["grain"]["shared"].as_array(),
        Some(&vec![JsonValue::String("store_id".to_string())])
    );
    assert_eq!(
        data["summary"]["grain"]["shared_primary_key"].as_u64(),
        Some(2)
    );

    let indicators = &differences["indicators"];
    assert_eq!(indicators["counts"]["canonical_in_both"].as_u64(), Some(3));
    assert!(
        indicators["in_both"].as_array().is_some_and(|values| values
            .contains(&JsonValue::String("gross_sales".to_string()))
            && values.contains(&JsonValue::String("orders_count".to_string()))
            && values.contains(&JsonValue::String("customers_count".to_string()))),
        "shared canonical indicators missing: {indicators:#?}"
    );
    assert!(
        indicators["near_duplicates"]
            .as_array()
            .is_some_and(|rows| rows.iter().any(|row| {
                row["name1"] == "avg_order_value"
                    && row["name2"] == "revenue_per_order"
                    && row["reason"] == "identical_expression"
            })),
        "identical-expression near duplicate missing: {indicators:#?}"
    );

    assert_eq!(
        differences["governance"]["sensitivity"]["equal"].as_bool(),
        Some(false)
    );
    assert_eq!(
        differences["governance"]["compliance"]["only_in_second"].as_array(),
        Some(&vec![JsonValue::String("internal".to_string())])
    );
    assert_eq!(
        data["summary"]["governance"]["changed_fields"].as_u64(),
        Some(3)
    );

    let tests = &differences["tests"];
    assert_eq!(tests["entity1_total"].as_u64(), Some(2));
    assert_eq!(tests["entity2_total"].as_u64(), Some(2));
    assert!(
        tests["shared_column_differences"]
            .as_array()
            .is_some_and(|rows| rows.iter().any(|row| {
                row["column"] == "month_start"
                    && row["only_in_second"].as_array().is_some_and(|types| {
                        types.contains(&JsonValue::String("not_null".to_string()))
                    })
            })),
        "shared-column test differences missing: {tests:#?}"
    );
}
