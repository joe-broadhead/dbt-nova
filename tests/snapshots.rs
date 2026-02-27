//! Snapshot tests for tool payload stability.
#[path = "support/config.rs"]
mod support_config;
#[path = "support/fixtures.rs"]
mod support_fixtures;
#[path = "support/json.rs"]
mod support_json;

use dbt_nova::params::{
    DetailLevel, GetColumnsParams, GetEntityParams, GetLineageParams, GetMetadataScoreParams,
    GetTestCoverageParams, PaginationParams, SearchParams,
};
use serde_json::Value as JsonValue;
use support_fixtures::load_fixture;
use support_json::json;

fn canonicalize_lineage_snapshot(mut payload: JsonValue) -> JsonValue {
    if let Some(edges) = payload
        .get_mut("data")
        .and_then(|data| data.get_mut("edges"))
        .and_then(|edges| edges.as_array_mut())
    {
        edges.sort_by(|left, right| {
            let left_key = (
                left.get("from")
                    .and_then(JsonValue::as_str)
                    .unwrap_or_default(),
                left.get("to")
                    .and_then(JsonValue::as_str)
                    .unwrap_or_default(),
            );
            let right_key = (
                right
                    .get("from")
                    .and_then(JsonValue::as_str)
                    .unwrap_or_default(),
                right
                    .get("to")
                    .and_then(JsonValue::as_str)
                    .unwrap_or_default(),
            );
            left_key.cmp(&right_key)
        });
    }

    if let Some(entities) = payload
        .get_mut("data")
        .and_then(|data| data.get_mut("entities"))
        .and_then(|entities| entities.as_array_mut())
    {
        entities.sort_by(|left, right| {
            let left_key = left
                .get("unique_id")
                .and_then(JsonValue::as_str)
                .unwrap_or_default();
            let right_key = right
                .get("unique_id")
                .and_then(JsonValue::as_str)
                .unwrap_or_default();
            left_key.cmp(right_key)
        });
    }

    payload
}

#[tokio::test(flavor = "multi_thread")]
async fn snapshot_get_entity_orders() {
    let searcher = load_fixture("nova_manifest.json").unwrap();
    let params = GetEntityParams {
        id_or_name: "model.nova_test.fct__orders".into(),
        resource_type: None,
        detail: DetailLevel::Standard,
    };
    let result = json(searcher.get_entity_data(&params).await);
    insta::assert_json_snapshot!("get_entity_orders", result);
}

#[tokio::test(flavor = "multi_thread")]
async fn snapshot_metadata_score_orders() {
    let searcher = load_fixture("nova_manifest.json").unwrap();
    let params = GetMetadataScoreParams {
        id_or_name: Some("model.nova_test.fct__orders".into()),
        resource_type: None,
        persona: Some("governance".into()),
        scope: Some("entity".into()),
        include_breakdown: false,
        include_recommendations: false,
        resource_types: Vec::new(),
        limit: Some(1000),
        offset: None,
    };
    let result = json(searcher.get_metadata_score(&params).await);
    insta::assert_json_snapshot!("metadata_score_orders", result);
}

#[tokio::test(flavor = "multi_thread")]
async fn snapshot_search_standard_models() {
    let searcher = load_fixture("nova_manifest.json").unwrap();
    let params = SearchParams {
        query: "int__campaign_features".into(),
        resource_types: vec!["model".into()],
        persona: None,
        detail: DetailLevel::Standard,
        min_score: None,
        fuzzy: false,
        include_highlights: false,
        include_sql: false,
        pagination: PaginationParams {
            limit: 5,
            offset: 0,
        },
    };
    let result = json(searcher.search(&params).await);
    insta::assert_json_snapshot!("search_standard_models", result);
}

#[tokio::test(flavor = "multi_thread")]
async fn snapshot_get_lineage_orders_upstream() {
    let searcher = load_fixture("nova_manifest.json").unwrap();
    let params = GetLineageParams {
        id_or_name: "model.nova_test.fct__orders".into(),
        direction: "upstream".into(),
        depth: Some(2),
        resource_types: vec![],
        detail: DetailLevel::Standard,
    };
    let result = canonicalize_lineage_snapshot(json(searcher.get_lineage(&params).await));
    insta::assert_json_snapshot!("get_lineage_orders_upstream", result);
}

#[tokio::test(flavor = "multi_thread")]
async fn snapshot_get_columns_campaign_features() {
    let searcher = load_fixture("nova_manifest.json").unwrap();
    let params = GetColumnsParams {
        id_or_name: "model.nova_test.int__campaign_features".into(),
    };
    let result = json(searcher.get_columns(&params).await);
    insta::assert_json_snapshot!("get_columns_campaign_features", result);
}

#[tokio::test(flavor = "multi_thread")]
async fn snapshot_get_test_coverage_traffic_sessions() {
    let searcher = load_fixture("nova_manifest.json").unwrap();
    let params = GetTestCoverageParams {
        id_or_name: "model.nova_test.stg__traffic_sessions".into(),
        resource_type: None,
        include_full: false,
        columns_limit: Some(50),
    };
    let result = json(searcher.get_test_coverage(&params).await);
    insta::assert_json_snapshot!("get_test_coverage_traffic_sessions", result);
}
