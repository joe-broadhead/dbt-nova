//! Tests for path docs matching and undocumented reporting.
#[path = "support/config.rs"]
mod support_config;
#[path = "support/fixtures.rs"]
mod support_fixtures;
#[path = "support/json.rs"]
mod support_json;

use dbt_nova::params::PaginationParams;
use dbt_nova::params::{DetailLevel, FindByPathParams, GetUndocumentedParams};
use support_fixtures::load_fixture;
use support_json::json;

#[tokio::test(flavor = "multi_thread")]
async fn glob_matches_paths() {
    let searcher = load_fixture("minimal.json").unwrap();
    let params = FindByPathParams {
        path_pattern: "models/**".into(),
        resource_types: vec![],
        detail: Some(DetailLevel::Standard),
        pagination: PaginationParams {
            limit: Some(10),
            offset: 0,
        },
    };
    let result = json(searcher.find_by_path(&params).await);
    assert_eq!(
        result.get("count").and_then(serde_json::Value::as_u64),
        Some(1)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn undocumented_entities_and_columns_reported() {
    let searcher = load_fixture("undocumented.json").unwrap();
    let params = GetUndocumentedParams {
        resource_type: "model".into(),
        id_or_name: None,
        package: None,
        path_prefix: None,
        include_columns: true,
        include_full: false,
        pagination: PaginationParams {
            limit: Some(10),
            offset: 0,
        },
    };
    let result = json(searcher.get_undocumented(&params).await);
    let summary = result.get("data").and_then(|d| d.get("summary")).unwrap();
    assert_eq!(
        summary
            .get("entities_missing_docs")
            .and_then(serde_json::Value::as_u64),
        Some(1)
    );
    assert_eq!(
        summary
            .get("columns_missing_docs")
            .and_then(serde_json::Value::as_u64),
        Some(2)
    );
}
