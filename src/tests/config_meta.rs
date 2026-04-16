//! Regression tests for dbt 1.11+ `config.meta` fallback behavior.
use super::common::*;

fn config_meta_searcher() -> TestSearchEnv {
    get_searcher_with_fixture("config_meta_only.json")
}

#[tokio::test(flavor = "multi_thread")]
async fn test_get_columns_supports_config_meta_primary_key() {
    let searcher = config_meta_searcher();
    let result = searcher
        .get_columns(&GetColumnsParams {
            id_or_name: "config_meta_model".to_string(),
        })
        .await
        .json();

    assert_eq!(result["success"].as_bool(), Some(true));
    assert_eq!(
        result["data"]["primary_key_columns"].as_array(),
        Some(&vec![JsonValue::String("order_id".to_string())])
    );

    let columns = result["data"]["columns"]
        .as_array()
        .expect("expected columns array");
    let order_id = columns
        .iter()
        .find(|column| column["name"].as_str() == Some("order_id"))
        .expect("missing order_id column");
    assert_eq!(
        order_id["meta"]["nova"]["role"].as_str(),
        Some("identifier")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_metadata_score_supports_config_meta_only() {
    let searcher = config_meta_searcher();
    let result = searcher
        .get_metadata_score(&GetMetadataScoreParams {
            id_or_name: Some("config_meta_model".to_string()),
            scope: Some("entity".to_string()),
            include_breakdown: true,
            include_recommendations: false,
            ..Default::default()
        })
        .await
        .json();

    assert_eq!(result["success"].as_bool(), Some(true));
    assert!(
        result["data"]["categories"]["semantic"]["score"]
            .as_u64()
            .is_some_and(|score| score > 0)
    );
    assert!(
        result["data"]["categories"]["governance"]["score"]
            .as_u64()
            .is_some_and(|score| score > 0)
    );
    assert_eq!(
        result["data"]["breakdown"]["quality"]["primary_key"]["present"].as_bool(),
        Some(true)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_search_supports_config_meta_only_nova_metadata() {
    let searcher = config_meta_searcher();
    let result = searcher
        .search(&SearchParams {
            query: "revenue".to_string(),
            resource_types: vec!["model".to_string()],
            persona: None,
            detail: DetailLevel::Standard,
            min_score: None,
            fuzzy: false,
            include_highlights: false,
            include_sql: false,
            explain: false,
            pagination: PaginationParams {
                limit: 5,
                offset: 0,
            },
        })
        .await
        .json();

    assert_eq!(result["success"].as_bool(), Some(true));
    let rows = result["data"].as_array().expect("expected data array");
    assert_eq!(
        rows.first().and_then(|row| row["unique_id"].as_str()),
        Some("model.pkg.config_meta_model")
    );
}
