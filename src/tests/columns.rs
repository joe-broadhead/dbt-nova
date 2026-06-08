//! Tests for `get_columns` tool responses.
use super::common::*;
use crate::config::SearchConfig;

fn column_semantics_env() -> TestSearchEnv {
    get_searcher_with_fixture_config(
        "semantic_preview_ranking.json",
        SearchConfig {
            enable_vector_search: false,
            enable_sparse_search: false,
            enable_reranker: false,
            ..Default::default()
        },
    )
}

fn result_rows(result: &JsonValue) -> Vec<&JsonValue> {
    result
        .get("data")
        .and_then(JsonValue::as_array)
        .map(|rows| rows.iter().collect())
        .unwrap_or_default()
}

fn column_row<'a>(
    rows: &'a [&JsonValue],
    parent_unique_id: &str,
    column_name: &str,
) -> &'a JsonValue {
    rows.iter()
        .copied()
        .find(|row| {
            row.get("parent_unique_id").and_then(JsonValue::as_str) == Some(parent_unique_id)
                && row.get("column_name").and_then(JsonValue::as_str) == Some(column_name)
        })
        .unwrap_or_else(|| panic!("missing column row for {parent_unique_id}:{column_name}"))
}

// Get Columns Tests
#[tokio::test(flavor = "multi_thread")]
async fn test_get_columns_success() {
    let searcher = get_searcher();
    let params = GetColumnsParams {
        id_or_name: "int__campaign_features".to_string(),
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
    // This model has columns
    if let Some(data) = result.get("data") {
        let columns = data.get("columns").and_then(|c| c.as_array());
        assert!(columns.is_some(), "Should return columns array");
        assert!(!columns.unwrap().is_empty(), "Should have columns");
    }
}
#[tokio::test(flavor = "multi_thread")]
async fn test_get_columns_not_found() {
    let searcher = get_searcher();
    let params = GetColumnsParams {
        id_or_name: "nonexistent_entity".to_string(),
    };
    let result = searcher.get_columns(&params).await.json();
    let success = result
        .get("success")
        .expect("response missing 'success' field")
        .as_bool()
        .expect("'success' field should be boolean");
    assert!(!success, "Should fail for nonexistent entity");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_get_columns_are_sorted_by_name() {
    let searcher = get_searcher();
    let params = GetColumnsParams {
        id_or_name: "int__campaign_features".to_string(),
    };
    let result = searcher.get_columns(&params).await.json();
    let columns = result
        .get("data")
        .and_then(|data| data.get("columns"))
        .and_then(|value| value.as_array())
        .expect("columns array");

    let mut names: Vec<String> = columns
        .iter()
        .filter_map(|column| {
            column
                .get("name")
                .and_then(|value| value.as_str())
                .map(ToString::to_string)
        })
        .collect();
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted, "columns should be ordered by name");

    names.dedup();
    assert_eq!(
        names.len(),
        sorted.len(),
        "columns should not duplicate names"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_column_inventory_lists_all_columns_with_context() {
    let searcher = column_semantics_env();
    let result = searcher
        .column_inventory(&ColumnInventoryParams {
            resource_types: vec!["model".to_string()],
            roles: vec![],
            semantic_types: vec![],
            annotated_only: false,
            pagination: PaginationParams {
                limit: Some(50),
                offset: 0,
            },
        })
        .await
        .json();
    let rows = result_rows(&result);

    let country_code = column_row(&rows, "model.pkg.fact_orders_canonical", "country_code");
    assert_eq!(
        country_code.get("role").and_then(JsonValue::as_str),
        Some("dimension")
    );
    assert_eq!(
        country_code
            .get("semantic_type")
            .and_then(JsonValue::as_str),
        Some("country_code")
    );
    assert_eq!(
        country_code
            .get("synonyms")
            .and_then(JsonValue::as_array)
            .and_then(|values| values.first())
            .and_then(JsonValue::as_str),
        Some("market")
    );
    assert_eq!(
        country_code
            .get("example_values")
            .and_then(JsonValue::as_array)
            .map(Vec::len),
        Some(2)
    );
    let order_id = column_row(&rows, "model.pkg.fact_orders_canonical", "order_id");
    assert_eq!(
        order_id.get("annotated").and_then(JsonValue::as_bool),
        Some(false)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_column_inventory_filters_annotated_dimension_columns() {
    let searcher = column_semantics_env();
    let result = searcher
        .column_inventory(&ColumnInventoryParams {
            resource_types: vec!["model".to_string()],
            roles: vec!["dimension".to_string()],
            semantic_types: vec![],
            annotated_only: true,
            pagination: PaginationParams {
                limit: Some(50),
                offset: 0,
            },
        })
        .await
        .json();
    let rows = result_rows(&result);

    assert!(!rows.is_empty());
    assert!(
        rows.iter()
            .all(|row| row.get("annotated").and_then(JsonValue::as_bool) == Some(true))
    );
    assert!(
        rows.iter()
            .all(|row| row.get("role").and_then(JsonValue::as_str) == Some("dimension"))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_search_columns_matches_example_values() {
    let searcher = column_semantics_env();
    let result = searcher
        .search_columns(&SearchColumnsParams {
            query: "alpha".to_string(),
            resource_types: vec!["model".to_string()],
            roles: vec![],
            semantic_types: vec![],
            pagination: PaginationParams {
                limit: Some(10),
                offset: 0,
            },
            min_score: None,
        })
        .await
        .json();
    let rows = result_rows(&result);

    assert!(!rows.is_empty());
    assert_eq!(
        rows[0].get("column_name").and_then(JsonValue::as_str),
        Some("country_code")
    );
    assert_eq!(
        rows[0].get("match_type").and_then(JsonValue::as_str),
        Some("example_value")
    );
    assert_eq!(
        rows[0].get("matched_value").and_then(JsonValue::as_str),
        Some("alpha")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_search_columns_matches_synonyms() {
    let searcher = column_semantics_env();
    let result = searcher
        .search_columns(&SearchColumnsParams {
            query: "market".to_string(),
            resource_types: vec!["model".to_string()],
            roles: vec![],
            semantic_types: vec![],
            pagination: PaginationParams {
                limit: Some(10),
                offset: 0,
            },
            min_score: None,
        })
        .await
        .json();
    let rows = result_rows(&result);

    assert!(!rows.is_empty());
    assert_eq!(
        rows[0].get("column_name").and_then(JsonValue::as_str),
        Some("country_code")
    );
    assert_eq!(
        rows[0].get("match_type").and_then(JsonValue::as_str),
        Some("synonym")
    );
    assert_eq!(
        rows[0].get("matched_value").and_then(JsonValue::as_str),
        Some("market")
    );
}
