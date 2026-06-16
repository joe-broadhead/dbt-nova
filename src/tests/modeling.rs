//! Tests for overlap, grain, and modelling consistency tooling.
use super::common::*;
use crate::config::SearchConfig;

fn modeling_env() -> TestSearchEnv {
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

#[tokio::test(flavor = "multi_thread")]
async fn test_compare_grains_matches_entity_and_metric_grain() {
    let searcher = modeling_env();
    let result = searcher
        .compare_grains(&CompareGrainsParams {
            entity1: "fact_orders_canonical".to_string(),
            entity1_resource_type: Some("model".to_string()),
            entity2: "orders_semantic_templates".to_string(),
            entity2_resource_type: Some("model".to_string()),
        })
        .await
        .json();

    let data = result.get("data").expect("data");
    assert_eq!(
        data.get("exact_match").and_then(JsonValue::as_bool),
        Some(false)
    );
    assert_eq!(
        data.get("same_time_field").and_then(JsonValue::as_bool),
        Some(true)
    );
    assert_eq!(
        data.get("shared_dimensions")
            .and_then(JsonValue::as_array)
            .map(Vec::len),
        Some(2)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_find_entity_overlap_surfaces_closest_order_entities() {
    let searcher = modeling_env();
    let result = searcher
        .find_entity_overlap(&FindEntityOverlapParams {
            id_or_name: None,
            resource_type: None,
            resource_types: vec!["model".to_string()],
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
    let top = rows[0];
    let entity1 = top
        .get("entity1")
        .and_then(|value| value.get("unique_id"))
        .and_then(JsonValue::as_str)
        .expect("entity1 id");
    let entity2 = top
        .get("entity2")
        .and_then(|value| value.get("unique_id"))
        .and_then(JsonValue::as_str)
        .expect("entity2 id");
    let pair = [entity1, entity2];
    assert!(pair.contains(&"model.pkg.fact_orders_canonical"));
    assert!(pair.contains(&"model.pkg.fact_orders_channel"));
    let evidence = top.get("evidence").expect("evidence");
    assert_eq!(
        evidence
            .get("shared_indicators")
            .and_then(JsonValue::as_array)
            .and_then(|items| items.first())
            .and_then(JsonValue::as_str),
        Some("gmv")
    );
    assert_eq!(
        evidence
            .get("shared_column_names")
            .and_then(JsonValue::as_array)
            .map(Vec::len),
        Some(3)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_modelling_consistency_report_surfaces_duplicate_indicators() {
    let searcher = modeling_env();
    let result = searcher
        .modelling_consistency_report(&ModellingConsistencyReportParams {
            resource_types: vec!["model".to_string()],
            pagination: PaginationParams {
                limit: Some(10),
                offset: 0,
            },
            min_score: None,
        })
        .await
        .json();

    let data = result.get("data").expect("data");
    let duplicate_indicators = data
        .get("duplicate_indicators")
        .and_then(JsonValue::as_array)
        .expect("duplicate_indicators");
    assert!(!duplicate_indicators.is_empty());
    assert!(duplicate_indicators.iter().any(|row| {
        row.get("indicator_name").and_then(JsonValue::as_str) == Some("average_order_value")
            && row.get("parent_count").and_then(JsonValue::as_u64) == Some(2)
    }));
    let overlap_candidates = data
        .get("overlap_candidates")
        .and_then(JsonValue::as_array)
        .expect("overlap_candidates");
    assert!(!overlap_candidates.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_modelling_consistency_report_applies_section_offset() {
    let searcher = modeling_env();
    let page_1 = searcher
        .modelling_consistency_report(&ModellingConsistencyReportParams {
            resource_types: vec!["model".to_string()],
            pagination: PaginationParams {
                limit: Some(1),
                offset: 0,
            },
            min_score: None,
        })
        .await
        .json();
    let page_2 = searcher
        .modelling_consistency_report(&ModellingConsistencyReportParams {
            resource_types: vec!["model".to_string()],
            pagination: PaginationParams {
                limit: Some(1),
                offset: 1,
            },
            min_score: None,
        })
        .await
        .json();

    let overlap_page_1 = page_1
        .get("data")
        .and_then(|data| data.get("overlap_candidates"))
        .and_then(JsonValue::as_array)
        .expect("page_1 overlap_candidates");
    let overlap_page_2 = page_2
        .get("data")
        .and_then(|data| data.get("overlap_candidates"))
        .and_then(JsonValue::as_array)
        .expect("page_2 overlap_candidates");

    assert_eq!(overlap_page_1.len(), 1);
    assert_eq!(overlap_page_2.len(), 1);
    assert_ne!(overlap_page_1[0], overlap_page_2[0]);
}
