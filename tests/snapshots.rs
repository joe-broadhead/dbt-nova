//! Snapshot tests for tool payload stability.
#[path = "support/config.rs"]
mod support_config;
#[path = "support/fixtures.rs"]
mod support_fixtures;
#[path = "support/json.rs"]
mod support_json;

use dbt_nova::params::{DetailLevel, GetEntityParams, GetMetadataScoreParams};
use support_fixtures::load_fixture;
use support_json::json;

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
