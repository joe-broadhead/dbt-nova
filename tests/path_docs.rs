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
async fn glob_reports_exact_total_beyond_first_page() {
    let guard = tempfile::TempDir::new().unwrap();
    let manifest_path = guard.path().join("nova_manifest.json");
    let mut nodes = serde_json::Map::new();
    for index in 0..3 {
        let unique_id = format!("model.pkg.model_{index}");
        nodes.insert(
            unique_id.clone(),
            serde_json::json!({
                "name": format!("model_{index}"),
                "resource_type": "model",
                "package_name": "pkg",
                "database": "db",
                "schema": "sch",
                "raw_code": "select 1 as col",
                "columns": {},
                "tags": [],
                "original_file_path": format!("models/model_{index}.sql"),
                "unique_id": unique_id
            }),
        );
    }
    std::fs::write(
        &manifest_path,
        serde_json::to_vec(&serde_json::json!({
            "metadata": {
                "dbt_version": "1.10.2",
                "dbt_schema_version": "https://schemas.getdbt.com/dbt/manifest/v12.json"
            },
            "nodes": nodes,
            "sources": {},
            "macros": {},
            "docs": {},
            "groups": {},
            "exposures": {},
            "metrics": {},
            "parent_map": {},
            "child_map": {}
        }))
        .unwrap(),
    )
    .unwrap();

    let searcher = support_fixtures::load_manifest_path(&manifest_path).unwrap();
    let params = FindByPathParams {
        path_pattern: "models/**".into(),
        resource_types: vec![],
        detail: Some(DetailLevel::Compact),
        pagination: PaginationParams {
            limit: Some(1),
            offset: 0,
        },
    };
    let result = json(searcher.find_by_path(&params).await);
    assert_eq!(result["count"].as_u64(), Some(1));
    assert_eq!(result["total_available"].as_u64(), Some(3));
    assert_eq!(result["truncated"].as_bool(), Some(true));
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

#[tokio::test(flavor = "multi_thread")]
async fn undocumented_paginates_entities_and_columns_as_one_stream() {
    let searcher = load_fixture("undocumented.json").unwrap();
    let first_page = GetUndocumentedParams {
        resource_type: "model".into(),
        id_or_name: None,
        package: None,
        path_prefix: None,
        include_columns: true,
        include_full: false,
        pagination: PaginationParams {
            limit: Some(1),
            offset: 0,
        },
    };
    let result = json(searcher.get_undocumented(&first_page).await);
    assert_eq!(result["count"].as_u64(), Some(1));
    assert_eq!(result["total_available"].as_u64(), Some(3));
    assert_eq!(result["truncated"].as_bool(), Some(true));
    assert_eq!(
        result["data"]["summary"]["columns_missing_docs"].as_u64(),
        Some(2)
    );

    let second_page = GetUndocumentedParams {
        pagination: PaginationParams {
            limit: Some(1),
            offset: 1,
        },
        ..first_page
    };
    let result = json(searcher.get_undocumented(&second_page).await);
    assert_eq!(result["count"].as_u64(), Some(1));
    assert_eq!(result["data"]["entities"].as_array().map(Vec::len), Some(0));
    assert_eq!(
        result["data"]["undocumented_columns"]
            .as_array()
            .map(Vec::len),
        Some(1)
    );
}
