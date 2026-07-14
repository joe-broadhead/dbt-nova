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
    assert_eq!(order_id["meta"]["primary_key"].as_bool(), Some(true));
    assert_eq!(
        order_id["meta"]["nova"]["semantic_type"].as_str(),
        Some("order_id")
    );
    assert_eq!(
        order_id["meta"]["nova"]["synonyms"].as_array(),
        Some(&vec![JsonValue::String("purchase_id".to_string())])
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
            detail: Some(DetailLevel::Standard),
            min_score: None,
            fuzzy: false,
            include_highlights: false,
            include_sql: false,
            explain: false,
            pagination: PaginationParams {
                limit: Some(5),
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

#[tokio::test(flavor = "multi_thread")]
async fn test_search_supports_mixed_legacy_and_config_column_nova_metadata() {
    let searcher = config_meta_searcher();
    let result = searcher
        .search(&SearchParams {
            query: "purchase_id".to_string(),
            resource_types: vec!["model".to_string()],
            persona: None,
            detail: Some(DetailLevel::Standard),
            min_score: None,
            fuzzy: false,
            include_highlights: false,
            include_sql: false,
            explain: false,
            pagination: PaginationParams {
                limit: Some(5),
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

#[tokio::test(flavor = "multi_thread")]
async fn test_engineer_persona_uses_config_meta_owner() {
    let searcher = config_meta_searcher();
    let result = searcher
        .search(&SearchParams {
            query: "revenue".to_string(),
            resource_types: vec!["model".to_string()],
            persona: Some("engineer".to_string()),
            detail: Some(DetailLevel::Standard),
            min_score: None,
            fuzzy: false,
            include_highlights: false,
            include_sql: false,
            explain: false,
            pagination: PaginationParams {
                limit: Some(5),
                offset: 0,
            },
        })
        .await
        .json();

    assert_eq!(result["success"].as_bool(), Some(true));
    let row = result["data"]
        .as_array()
        .and_then(|rows| rows.first())
        .expect("expected search row");
    assert_eq!(
        row["unique_id"].as_str(),
        Some("model.pkg.config_meta_model")
    );
    assert_eq!(
        row["persona_payload"]["selection_signals"]["has_owner"].as_bool(),
        Some(true)
    );
    assert!(
        !row["persona_payload"]["selection_rationale"]
            .as_str()
            .unwrap_or_default()
            .contains("owner metadata missing")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_governance_persona_uses_config_meta_owner() {
    let searcher = config_meta_searcher();
    let result = searcher
        .search(&SearchParams {
            query: "revenue".to_string(),
            resource_types: vec!["model".to_string()],
            persona: Some("governance".to_string()),
            detail: Some(DetailLevel::Standard),
            min_score: None,
            fuzzy: false,
            include_highlights: false,
            include_sql: false,
            explain: false,
            pagination: PaginationParams {
                limit: Some(5),
                offset: 0,
            },
        })
        .await
        .json();

    assert_eq!(result["success"].as_bool(), Some(true));
    let row = result["data"]
        .as_array()
        .and_then(|rows| rows.first())
        .expect("expected search row");
    assert_eq!(
        row["persona_payload"]["gate_signals"]["owner_present"].as_bool(),
        Some(true)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_get_context_merges_legacy_and_config_column_meta() {
    let searcher = config_meta_searcher();
    let result = searcher
        .get_context(&GetContextParams {
            id_or_name: "config_meta_model".to_string(),
            resource_type: None,
            include_columns: true,
            include_tests: false,
            include_upstream: false,
            upstream_include_tests: false,
            include_downstream: false,
            downstream_include_tests: false,
            include_docs: false,
            include_sql: false,
            context_mode: ContextMode::Standard,
            limits: ContextLimits {
                lineage_depth: 1,
                upstream_limit: 5,
                downstream_limit: 5,
            },
        })
        .await
        .json();

    assert_eq!(result["success"].as_bool(), Some(true));
    let columns = result["data"]["entity"]["columns"]
        .as_array()
        .expect("expected columns array");
    let order_id = columns
        .iter()
        .find(|column| column["name"].as_str() == Some("order_id"))
        .expect("missing order_id column");

    assert_eq!(order_id["meta"]["primary_key"].as_bool(), Some(true));
    assert_eq!(
        order_id["meta"]["nova"]["role"].as_str(),
        Some("identifier")
    );
    assert_eq!(
        order_id["meta"]["nova"]["semantic_type"].as_str(),
        Some("order_id")
    );
    assert_eq!(
        order_id["meta"]["nova"]["synonyms"].as_array(),
        Some(&vec![JsonValue::String("purchase_id".to_string())])
    );
}
