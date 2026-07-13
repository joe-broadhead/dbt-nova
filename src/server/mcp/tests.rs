use std::collections::BTreeSet;
use std::path::Path;

use tempfile::TempDir;

use super::response_budget::serialized_len;
use super::*;
use crate::tests::common::fixture_manifest_path_string;
use crate::tools::catalog::MCP_TOOL_NAMES;

fn test_config(storage_root: &Path) -> crate::config::DbtNovaConfig {
    let mut config = crate::config::DbtNovaConfig {
        manifest_path: fixture_manifest_path_string(),
        manifest_refresh_secs: 0,
        storage_dir: storage_root.join(".dbt-nova").to_string_lossy().to_string(),
        storage_instance_id: "server-mcp-tests".to_string(),
        cleanup_storage_on_start: true,
        ..Default::default()
    };
    config.search.enable_vector_search = false;
    config.search.enable_sparse_search = false;
    config.search.enable_reranker = false;
    config
}

fn write_test_eval_results(dir: &Path, status: &str, pass_rate: f64) {
    let pass_count = usize::from(status == "pass");
    let fail_count = usize::from(status == "fail");
    std::fs::create_dir_all(dir).expect("create eval results dir");
    let payload = serde_json::json!({
        "suite_name": "mcp-compare-smoke",
        "version": 1,
        "mode": "bridge",
        "output_dir": dir.display().to_string(),
        "assertion_count": 1,
        "pass_count": pass_count,
        "fail_count": fail_count,
        "error_count": 0,
        "pass_rate": pass_rate,
        "gate_status": status,
        "cases": [{
            "id": "case",
            "pass_count": pass_count,
            "fail_count": fail_count,
            "error_count": 0,
            "assertions": [{"name": "tool_success", "status": status}]
        }]
    });
    std::fs::write(
        dir.join("results.json"),
        serde_json::to_string_pretty(&payload).expect("serialize eval results"),
    )
    .expect("write eval results");
}

async fn spawn_ready_server(storage_root: &Path) -> DbtNovaServer {
    let handle = ManifestSearchHandle::spawn(test_config(storage_root));
    handle
        .wait_ready()
        .await
        .expect("fixture manifest should load");
    DbtNovaServer::new(handle)
}

fn tool_response_body(response: ToolCallResponse) -> String {
    match response {
        Ok(body) | Err(body) => body,
    }
}

fn tool_response_json(response: ToolCallResponse) -> serde_json::Value {
    let body = tool_response_body(response);
    serde_json::from_str(&body).expect("tool response JSON")
}

#[test]
fn mcp_success_serialization_includes_api_contract_marker() {
    let config = crate::config::DbtNovaConfig {
        mcp_max_response_bytes: 0,
        ..Default::default()
    };
    let response = serde_json::json!({
        "success": true,
        "count": 1,
        "data": [{"unique_id": "model.pkg.orders"}]
    });

    let serialized =
        DbtNovaServer::serialize_budgeted_value(response, &config).expect("serialize response");
    let payload: serde_json::Value = serde_json::from_str(&serialized).expect("response JSON");

    assert_eq!(
        payload["api"]["envelope"],
        serde_json::json!(crate::responses::RESPONSE_ENVELOPE_ID)
    );
    assert_eq!(
        payload["api"]["nova_version"],
        serde_json::json!(env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn mcp_error_serialization_includes_api_contract_marker() {
    let serialized =
        DbtNovaServer::error_response(&DbtNovaError::InvalidParams("missing id".to_string()));
    let payload: serde_json::Value = serde_json::from_str(&serialized).expect("response JSON");

    assert_eq!(payload["success"], serde_json::json!(false));
    assert_eq!(
        payload["api"]["envelope"],
        serde_json::json!(crate::responses::RESPONSE_ENVELOPE_ID)
    );
    assert_eq!(payload["error_code"], serde_json::json!("INVALID_PARAMS"));
}

#[test]
fn mcp_response_budget_prunes_large_object_payloads() {
    let mut columns = serde_json::Map::new();
    for idx in 0..500 {
        columns.insert(
            format!("column_{idx:04}"),
            serde_json::json!({
                "name": format!("column_{idx:04}"),
                "description": "short but repeated metadata",
                "data_type": "string"
            }),
        );
    }
    let config = crate::config::DbtNovaConfig {
        mcp_max_response_bytes: 4096,
        mcp_max_string_chars: 128,
        ..Default::default()
    };
    let response = serde_json::json!({
        "success": true,
        "count": 1,
        "data": {
            "unique_id": "model.pkg.large",
            "name": "large",
            "columns": columns
        }
    });

    let budgeted = apply_mcp_response_budget(response, &config);
    let response_bytes = serialized_len(&budgeted);

    assert!(
        response_bytes <= config.mcp_max_response_bytes,
        "response_bytes={response_bytes}"
    );
    assert_eq!(
        budgeted["_nova_result_meta"]["truncated"],
        serde_json::json!(true)
    );
    assert!(
        budgeted["_nova_result_meta"]["omitted_paths"]
            .as_array()
            .is_some_and(|paths| !paths.is_empty())
    );
}

#[test]
fn mcp_response_budget_preserves_singleton_object_shape() {
    let mut columns = serde_json::Map::new();
    for idx in 0..100 {
        columns.insert(
            format!("column_{idx:03}"),
            serde_json::json!({
                "name": format!("column_{idx:03}"),
                "description": "x".repeat(200)
            }),
        );
    }
    let config = crate::config::DbtNovaConfig {
        mcp_max_response_bytes: 256,
        mcp_max_string_chars: 16,
        ..Default::default()
    };
    let response = serde_json::json!({
        "success": true,
        "count": 1,
        "data": {
            "unique_id": "model.pkg.large",
            "columns": columns
        }
    });

    let budgeted = apply_mcp_response_budget(response, &config);

    assert!(
        serialized_len(&budgeted) <= config.mcp_max_response_bytes,
        "response_bytes={}",
        serialized_len(&budgeted)
    );
    assert_eq!(budgeted["count"], serde_json::json!(1));
    assert!(budgeted["data"].as_object().is_some());
    assert!(budgeted["data"].as_array().is_none());
}

#[test]
fn mcp_response_budget_keeps_payload_when_only_meta_exceeds_budget() {
    let config = crate::config::DbtNovaConfig {
        mcp_max_response_bytes: 100,
        mcp_max_string_chars: 10,
        ..Default::default()
    };
    let response = serde_json::json!({
        "success": true,
        "data": {
            "text": "x".repeat(1000)
        }
    });

    let budgeted = apply_mcp_response_budget(response, &config);

    assert!(
        serialized_len(&budgeted) <= config.mcp_max_response_bytes,
        "response_bytes={}",
        serialized_len(&budgeted)
    );
    assert!(budgeted["data"]["text"].as_str().is_some());
    assert!(budgeted.get("_nova_result_meta").is_none());
}

#[test]
fn mcp_response_budget_updates_count_when_data_array_is_truncated() {
    let config = crate::config::DbtNovaConfig {
        mcp_max_response_bytes: 8192,
        mcp_max_string_chars: 64,
        ..Default::default()
    };
    let rows: Vec<_> = (0..100)
        .map(|idx| {
            serde_json::json!({
                "unique_id": format!("model.pkg.item_{idx:03}"),
                "description": "x".repeat(300)
            })
        })
        .collect();
    let response = serde_json::json!({
        "success": true,
        "count": rows.len(),
        "data": rows
    });

    let budgeted = apply_mcp_response_budget(response, &config);
    let returned_count = budgeted["data"].as_array().map_or(0, Vec::len);

    assert!(
        returned_count < 100,
        "data was not truncated: returned_count={returned_count}"
    );
    assert!(returned_count > 0);
    assert_eq!(budgeted["count"], serde_json::json!(returned_count));
    assert_eq!(budgeted["truncated"], serde_json::json!(true));
    assert_eq!(
        budgeted["_nova_result_meta"]["original_count"],
        serde_json::json!(100)
    );
}

#[test]
fn mcp_response_budget_updates_count_when_sql_rows_are_truncated() {
    let config = crate::config::DbtNovaConfig {
        mcp_max_response_bytes: 8192,
        mcp_max_string_chars: 64,
        ..Default::default()
    };
    let rows: Vec<_> = (0..100)
        .map(|idx| serde_json::json!([format!("row_{idx:03}"), "x".repeat(300)]))
        .collect();
    let response = serde_json::json!({
        "success": true,
        "count": rows.len(),
        "data": {
            "rows": rows,
            "columns": ["id", "description"],
            "column_types": ["Text", "Text"],
            "stats": {
                "total_row_count": 100,
                "total_chunk_count": 1
            },
            "state": "SUCCEEDED",
            "provider": "duckdb",
            "elapsed_ms": 12,
            "statement_id": "stmt_123",
            "duckdb_path": "tests/fixtures/tokenomics.duckdb",
            "truncated": false
        }
    });

    let budgeted = apply_mcp_response_budget(response, &config);
    let returned_count = budgeted["data"]["rows"].as_array().map_or(0, Vec::len);

    assert!(
        returned_count < 100,
        "rows were not truncated: returned_count={returned_count}"
    );
    assert!(returned_count > 0);
    assert!(budgeted["data"]["rows"].as_array().is_some());
    assert_eq!(budgeted["count"], serde_json::json!(returned_count));
    assert_eq!(budgeted["truncated"], serde_json::json!(true));
    assert_eq!(budgeted["data"]["truncated"], serde_json::json!(true));
}

#[test]
fn mcp_response_budget_updates_count_when_nested_entities_are_truncated() {
    let config = crate::config::DbtNovaConfig {
        mcp_max_response_bytes: 8192,
        mcp_max_string_chars: 64,
        ..Default::default()
    };
    let entities: Vec<_> = (0..100)
        .map(|idx| {
            serde_json::json!({
                "unique_id": format!("model.pkg.entity_{idx:03}"),
                "description": "x".repeat(300)
            })
        })
        .collect();
    let not_found: Vec<_> = (0..100)
        .map(|idx| format!("model.pkg.missing_{idx:03}"))
        .collect();
    let response = serde_json::json!({
        "success": true,
        "count": entities.len(),
        "data": {
            "entities": entities,
            "not_found": not_found,
            "found_count": 100,
            "not_found_count": 100
        }
    });

    let budgeted = apply_mcp_response_budget(response, &config);
    let returned_entities = budgeted["data"]["entities"].as_array().map_or(0, Vec::len);
    let returned_not_found = budgeted["data"]["not_found"].as_array().map_or(0, Vec::len);

    assert!(
        returned_entities < 100,
        "entities were not truncated: returned_entities={returned_entities}"
    );
    assert!(returned_entities > 0);
    assert_eq!(budgeted["count"], serde_json::json!(returned_entities));
    assert_eq!(
        budgeted["data"]["found_count"],
        serde_json::json!(returned_entities)
    );
    assert_eq!(
        budgeted["data"]["not_found_count"],
        serde_json::json!(returned_not_found)
    );
    assert_eq!(budgeted["truncated"], serde_json::json!(true));
    assert_eq!(budgeted["data"]["truncated"], serde_json::json!(true));
}

#[test]
fn mcp_response_budget_updates_count_when_data_columns_are_truncated() {
    let config = crate::config::DbtNovaConfig {
        mcp_max_response_bytes: 8192,
        mcp_max_string_chars: 64,
        ..Default::default()
    };
    let columns: Vec<_> = (0..100)
        .map(|idx| {
            serde_json::json!({
                "name": format!("column_{idx:03}"),
                "description": "x".repeat(300),
                "data_type": "string"
            })
        })
        .collect();
    let response = serde_json::json!({
        "success": true,
        "count": columns.len(),
        "data": {
            "unique_id": "model.pkg.wide",
            "columns": columns
        }
    });

    let budgeted = apply_mcp_response_budget(response, &config);
    let returned_columns = budgeted["data"]["columns"].as_array().map_or(0, Vec::len);

    assert!(
        returned_columns < 100,
        "columns were not truncated: returned_columns={returned_columns}"
    );
    assert!(returned_columns > 0);
    assert_eq!(budgeted["count"], serde_json::json!(returned_columns));
    assert_eq!(budgeted["truncated"], serde_json::json!(true));
    assert_eq!(budgeted["data"]["truncated"], serde_json::json!(true));
    assert_eq!(
        budgeted["_nova_result_meta"]["original_count"],
        serde_json::json!(100)
    );
}

#[test]
fn mcp_response_budget_keeps_lineage_count_when_only_edges_are_truncated() {
    let config = crate::config::DbtNovaConfig {
        mcp_max_response_bytes: 8192,
        mcp_max_string_chars: 64,
        ..Default::default()
    };
    let entities = vec![
        serde_json::json!({"unique_id": "model.pkg.root", "description": "root"}),
        serde_json::json!({"unique_id": "model.pkg.child", "description": "child"}),
    ];
    let edges: Vec<_> = (0..100)
        .map(|idx| {
            serde_json::json!({
                "source": format!("model.pkg.source_{idx:03}"),
                "target": format!("model.pkg.target_{idx:03}"),
                "description": "x".repeat(300)
            })
        })
        .collect();
    let response = serde_json::json!({
        "success": true,
        "count": entities.len(),
        "data": {
            "root_id": "model.pkg.root",
            "entities": entities,
            "edges": edges
        }
    });

    let budgeted = apply_mcp_response_budget(response, &config);
    let returned_edges = budgeted["data"]["edges"].as_array().map_or(0, Vec::len);

    assert!(
        returned_edges < 100,
        "edges were not truncated: returned_edges={returned_edges}"
    );
    assert_eq!(budgeted["count"], serde_json::json!(2));
    assert_eq!(budgeted["truncated"], serde_json::json!(true));
    assert_eq!(budgeted["data"]["truncated"], serde_json::json!(true));
}

#[test]
fn mcp_response_budget_updates_reference_count_when_references_are_truncated() {
    let config = crate::config::DbtNovaConfig {
        mcp_max_response_bytes: 8192,
        mcp_max_string_chars: 64,
        ..Default::default()
    };
    let references: Vec<_> = (0..100)
        .map(|idx| {
            serde_json::json!({
                "kind": "recipe_sql",
                "unique_id": format!("analysis.pkg.recipe_{idx:03}"),
                "detail": {
                    "match": "textual",
                    "snippet": "type_of_channel ".repeat(50)
                }
            })
        })
        .collect();
    let response = serde_json::json!({
        "success": true,
        "count": 0,
        "data": {
            "start_entity": "model.pkg.sales_daily_channel",
            "start_column": "type_of_channel",
            "lineage": [],
            "reference_count": references.len(),
            "references": references
        }
    });

    let budgeted = apply_mcp_response_budget(response, &config);
    let returned_references = budgeted["data"]["references"]
        .as_array()
        .map_or(0, Vec::len);

    assert!(
        returned_references < 100,
        "references were not truncated: returned_references={returned_references}"
    );
    assert!(returned_references > 0);
    assert_eq!(budgeted["count"], serde_json::json!(0));
    assert_eq!(
        budgeted["data"]["reference_count"],
        serde_json::json!(returned_references)
    );
    assert_eq!(budgeted["truncated"], serde_json::json!(true));
    assert_eq!(budgeted["data"]["truncated"], serde_json::json!(true));
}

#[test]
fn mcp_response_budget_updates_undocumented_summary_counts_when_truncated() {
    let config = crate::config::DbtNovaConfig {
        mcp_max_response_bytes: 8192,
        mcp_max_string_chars: 64,
        ..Default::default()
    };
    let entities: Vec<_> = (0..80)
        .map(|idx| {
            serde_json::json!({
                "unique_id": format!("model.pkg.entity_{idx:03}"),
                "description": "x".repeat(300)
            })
        })
        .collect();
    let undocumented_columns: Vec<_> = (0..80)
        .map(|idx| {
            serde_json::json!({
                "unique_id": format!("model.pkg.entity_{idx:03}"),
                "column": format!("column_{idx:03}"),
                "description": "x".repeat(300)
            })
        })
        .collect();
    let response = serde_json::json!({
        "success": true,
        "count": 160,
        "data": {
            "entities": entities,
            "summary": {
                "entities_missing_docs": 80,
                "columns_missing_docs": 80,
                "entities_returned": 80,
                "columns_returned": 80,
                "items_returned": 160
            },
            "undocumented_columns": undocumented_columns
        }
    });

    let budgeted = apply_mcp_response_budget(response, &config);
    let returned_entities = budgeted["data"]["entities"].as_array().map_or(0, Vec::len);
    let returned_columns = budgeted["data"]["undocumented_columns"]
        .as_array()
        .map_or(0, Vec::len);
    let returned_total = returned_entities + returned_columns;

    assert!(returned_total < 160);
    assert!(returned_total > 0);
    assert_eq!(budgeted["count"], serde_json::json!(returned_total));
    assert_eq!(
        budgeted["data"]["summary"]["entities_returned"],
        serde_json::json!(returned_entities)
    );
    assert_eq!(
        budgeted["data"]["summary"]["columns_returned"],
        serde_json::json!(returned_columns)
    );
    assert_eq!(
        budgeted["data"]["summary"]["items_returned"],
        serde_json::json!(returned_total)
    );
}

#[test]
fn mcp_response_budget_fallback_zeroes_nested_collection_counts() {
    let config = crate::config::DbtNovaConfig {
        mcp_max_response_bytes: 256,
        mcp_max_string_chars: 16,
        ..Default::default()
    };
    let entities: Vec<_> = (0..100)
        .map(|idx| {
            serde_json::json!({
                "unique_id": format!("model.pkg.entity_{idx:03}"),
                "description": "x".repeat(500)
            })
        })
        .collect();
    let response = serde_json::json!({
        "success": true,
        "count": entities.len(),
        "data": {
            "entities": entities,
            "found_count": 100
        }
    });

    let budgeted = apply_mcp_response_budget(response, &config);

    assert!(
        serialized_len(&budgeted) <= config.mcp_max_response_bytes,
        "response_bytes={}",
        serialized_len(&budgeted)
    );
    assert_eq!(budgeted["count"], serde_json::json!(0));
    assert_eq!(budgeted["data"]["found_count"], serde_json::json!(0));
    assert_eq!(
        budgeted["data"]["entities"]
            .as_array()
            .map_or(usize::MAX, Vec::len),
        0
    );
    assert_eq!(budgeted["truncated"], serde_json::json!(true));
}

#[test]
fn mcp_compact_profile_defaults_indicator_search_for_agents() {
    let config = crate::config::DbtNovaConfig::default();
    let mut params = SearchIndicatorParams::default();

    apply_mcp_search_indicator_defaults(&mut params, &config);

    assert_eq!(params.detail, Some(crate::params::DetailLevel::Compact));
    assert_eq!(params.pagination.limit, Some(config.mcp_default_limit));
    assert_eq!(params.group_mode, Some(ParentGroupMode::Top));
    assert_eq!(params.max_parent_groups, Some(1));
}

#[test]
fn mcp_profile_defaults_preserve_explicit_detail_and_group_mode() {
    let config = crate::config::DbtNovaConfig {
        mcp_default_limit: 10,
        mcp_max_page_size: 20,
        ..Default::default()
    };
    let mut params = SearchIndicatorParams {
        detail: Some(crate::params::DetailLevel::Standard),
        group_mode: Some(ParentGroupMode::All),
        pagination: PaginationParams {
            limit: Some(500),
            offset: 0,
        },
        ..SearchIndicatorParams::default()
    };

    apply_mcp_search_indicator_defaults(&mut params, &config);

    assert_eq!(params.detail, Some(crate::params::DetailLevel::Standard));
    assert_eq!(params.group_mode, Some(ParentGroupMode::All));
    assert_eq!(params.pagination.limit, Some(20));
}

#[test]
fn mcp_pagination_meta_adds_next_offset_when_more_results_exist() {
    let config = crate::config::DbtNovaConfig {
        mcp_max_response_bytes: 0,
        ..Default::default()
    };
    let response = serde_json::json!({
        "success": true,
        "count": 2,
        "total_available": 5,
        "truncated": true,
        "data": [{"unique_id": "model.pkg.a"}, {"unique_id": "model.pkg.b"}]
    });
    let pagination = PaginationParams {
        limit: Some(2),
        offset: 2,
    };

    let serialized = DbtNovaServer::serialize_budgeted_value_with_pagination(
        response,
        &config,
        Some(&pagination),
    )
    .expect("serialize response");
    let payload: serde_json::Value = serde_json::from_str(&serialized).expect("response JSON");

    assert_eq!(
        payload["_nova_result_meta"]["next_offset"],
        serde_json::json!(4)
    );
}

#[test]
fn mcp_pagination_meta_survives_when_truncation_meta_disabled() {
    let config = crate::config::DbtNovaConfig {
        mcp_max_response_bytes: 0,
        mcp_include_truncation_meta: false,
        ..Default::default()
    };
    let response = serde_json::json!({
        "success": true,
        "count": 2,
        "total_available": 5,
        "truncated": true,
        "data": [{"unique_id": "model.pkg.a"}, {"unique_id": "model.pkg.b"}]
    });
    let pagination = PaginationParams {
        limit: Some(2),
        offset: 2,
    };

    let serialized = DbtNovaServer::serialize_budgeted_value_with_pagination(
        response,
        &config,
        Some(&pagination),
    )
    .expect("serialize response");
    let payload: serde_json::Value = serde_json::from_str(&serialized).expect("response JSON");

    assert_eq!(
        payload["_nova_result_meta"],
        serde_json::json!({"next_offset": 4})
    );
}

#[test]
fn mcp_pagination_meta_omits_next_offset_when_no_items_returned() {
    let config = crate::config::DbtNovaConfig {
        mcp_max_response_bytes: 0,
        ..Default::default()
    };
    let response = serde_json::json!({
        "success": true,
        "count": 0,
        "total_available": 5,
        "truncated": true,
        "data": []
    });
    let pagination = PaginationParams {
        limit: Some(2),
        offset: 2,
    };

    let serialized = DbtNovaServer::serialize_budgeted_value_with_pagination(
        response,
        &config,
        Some(&pagination),
    )
    .expect("serialize response");
    let payload: serde_json::Value = serde_json::from_str(&serialized).expect("response JSON");

    assert!(
        payload
            .get("_nova_result_meta")
            .and_then(|meta| meta.get("next_offset"))
            .is_none()
    );
}

#[test]
fn mcp_tool_catalog_matches_registered_router_names() {
    let router_names = DbtNovaServer::tool_router()
        .map
        .keys()
        .map(|name| name.as_ref().to_string())
        .collect::<BTreeSet<_>>();
    let catalog_names = MCP_TOOL_NAMES
        .iter()
        .map(|name| (*name).to_string())
        .collect::<BTreeSet<_>>();

    assert_eq!(MCP_TOOL_NAMES.len(), catalog_names.len());
    assert_eq!(catalog_names, router_names);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn health_reports_ready_status_and_metrics_payload() {
    let temp_dir = TempDir::new().expect("temp dir");
    let server = spawn_ready_server(temp_dir.path()).await;

    let payload = tool_response_json(server.health().await);
    assert_eq!(payload["success"], serde_json::json!(true));
    assert_eq!(payload["data"]["status"], serde_json::json!("ready"));
    assert!(payload["data"]["tool_metrics"].is_object());
    assert!(payload["data"]["search_concurrency"].is_object());
    assert!(payload["data"]["sql_concurrency"].is_object());
    assert!(payload["data"]["artifact_consumer"].is_object());
    assert_eq!(
        payload["data"]["artifact_consumer"]["enabled"],
        serde_json::json!(false)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn health_applies_mcp_response_budget() {
    let temp_dir = TempDir::new().expect("temp dir");
    let mut config = test_config(temp_dir.path());
    config.mcp_max_response_bytes = 512;
    config.mcp_max_string_chars = 16;
    let handle = ManifestSearchHandle::spawn(config.clone());
    handle
        .wait_ready()
        .await
        .expect("fixture manifest should load");
    let server = DbtNovaServer::new(handle);

    let out = tool_response_body(server.health().await);
    let payload: serde_json::Value = serde_json::from_str(&out).expect("health response JSON");

    assert!(
        out.len() <= config.mcp_max_response_bytes,
        "response_bytes={}",
        out.len()
    );
    assert_eq!(payload["success"], serde_json::json!(true));
    assert_eq!(payload["count"], serde_json::json!(1));
    assert_eq!(payload["truncated"], serde_json::json!(true));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn health_reports_degraded_when_enabled_semantic_components_are_not_query_ready() {
    let temp_dir = TempDir::new().expect("temp dir");
    let mut config = test_config(temp_dir.path());
    config.search.enable_vector_search = true;
    config.search.enable_sparse_search = true;
    config.search.enable_reranker = true;
    config.search.embedding_cache_dir = temp_dir.path().join("cache").to_string_lossy().to_string();

    let handle = ManifestSearchHandle::spawn(config);
    handle
        .wait_ready()
        .await
        .expect("fixture manifest should still load");
    let server = DbtNovaServer::new(handle);

    let payload = tool_response_json(server.health().await);
    assert_eq!(payload["success"], serde_json::json!(true));
    assert_eq!(payload["data"]["status"], serde_json::json!("degraded"));
    assert_eq!(
        payload["data"]["ready_for_traffic"],
        serde_json::json!(false)
    );
    assert_eq!(
        payload["data"]["search"]["vector"]["ready"],
        serde_json::json!(false)
    );
    assert_eq!(
        payload["data"]["search"]["sparse"]["ready"],
        serde_json::json!(false)
    );
    assert_eq!(
        payload["data"]["search"]["reranker"]["ready"],
        serde_json::json!(false)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn search_and_list_tags_record_metrics() {
    let temp_dir = TempDir::new().expect("temp dir");
    let server = spawn_ready_server(temp_dir.path()).await;

    let search_params = SearchParams {
        query: "orders".to_string(),
        persona: Some("analyst".to_string()),
        ..SearchParams::default()
    };
    let search_response = tool_response_json(server.search(Parameters(search_params)).await);
    assert_eq!(search_response["success"], serde_json::json!(true));

    let list_tags_response = tool_response_json(server.list_tags().await);
    assert_eq!(list_tags_response["success"], serde_json::json!(true));

    let health_payload = tool_response_json(server.health().await);
    let tool_metrics = &health_payload["data"]["tool_metrics"];
    assert!(tool_metrics["search"]["calls"].as_u64().unwrap_or(0) >= 1);
    assert!(
        tool_metrics["search.analyst"]["calls"]
            .as_u64()
            .unwrap_or(0)
            >= 1
    );
    assert!(tool_metrics["list_tags"]["calls"].as_u64().unwrap_or(0) >= 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn metadata_score_project_scope_applies_mcp_pagination_defaults() {
    let temp_dir = TempDir::new().expect("temp dir");
    let mut config = test_config(temp_dir.path());
    config.mcp_default_limit = 2;
    config.mcp_max_page_size = 3;
    config.mcp_max_response_bytes = 0;

    let handle = ManifestSearchHandle::spawn(config);
    handle
        .wait_ready()
        .await
        .expect("fixture manifest should load");
    let server = DbtNovaServer::new(handle);

    let response = tool_response_json(
        server
            .get_metadata_score(Parameters(GetMetadataScoreParams {
                scope: Some("project".to_string()),
                include_breakdown: false,
                include_recommendations: false,
                resource_types: vec!["model".to_string()],
                ..GetMetadataScoreParams::default()
            }))
            .await,
    );

    assert_eq!(response["success"], serde_json::json!(true));
    assert_eq!(response["count"], serde_json::json!(2));
    assert_eq!(response["data"]["limit"], serde_json::json!(2));
    assert_eq!(response["data"]["offset"], serde_json::json!(0));
    assert_eq!(
        response["_nova_result_meta"]["next_offset"],
        serde_json::json!(2)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_readiness_returns_report_contract() {
    let temp_dir = TempDir::new().expect("temp dir");
    let mut config = test_config(temp_dir.path());
    config.mcp_max_response_bytes = 0;
    let handle = ManifestSearchHandle::spawn(config);
    handle
        .wait_ready()
        .await
        .expect("fixture manifest should load");
    let server = DbtNovaServer::new(handle);

    let response = tool_response_json(
        server
            .get_agent_readiness(Parameters(GetAgentReadinessParams {
                personas_json: Some(r#"["engineer"]"#.to_string()),
                eval_gate_json: Some(
                    r#"{"allowed":true,"blocked":false,"message":"gate passed"}"#.to_string(),
                ),
                ..GetAgentReadinessParams::default()
            }))
            .await,
    );

    assert_eq!(response["success"], serde_json::json!(true));
    assert_eq!(response["count"], serde_json::json!(1));
    assert_eq!(
        response["data"]["schema_version"],
        serde_json::json!("agent_readiness.v1")
    );
    assert_eq!(
        response["data"]["config"]["personas"],
        serde_json::json!(["engineer"])
    );
    assert_eq!(
        response["data"]["eval_status"]["status"],
        serde_json::json!("allowed")
    );
    assert!(response["data"]["persona_scores"]["engineer"].is_object());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn metadata_audit_returns_gate_data() {
    let temp_dir = TempDir::new().expect("temp dir");
    let mut config = test_config(temp_dir.path());
    config.mcp_max_response_bytes = 0;
    let handle = ManifestSearchHandle::spawn(config);
    handle
        .wait_ready()
        .await
        .expect("fixture manifest should load");
    let server = DbtNovaServer::new(handle);

    let response = tool_response_json(
        server
            .get_metadata_audit(Parameters(GetMetadataAuditParams {
                resource_types_json: Some(r#"["model"]"#.to_string()),
                personas_json: Some(r#"["engineer"]"#.to_string()),
                thresholds_json: Some(
                    r#"{"project":{"engineer":{"min_score":101,"severity":"required"}}}"#
                        .to_string(),
                ),
                include_recommendations: Some(false),
                ..GetMetadataAuditParams::default()
            }))
            .await,
    );

    assert_eq!(response["success"], serde_json::json!(true));
    assert_eq!(response["count"], serde_json::json!(1));
    assert_eq!(
        response["data"]["selection_mode"],
        serde_json::json!("project")
    );
    assert_eq!(response["data"]["gate_status"], serde_json::json!("fail"));
    assert_eq!(
        response["data"]["summary"]["required_fail_count"],
        serde_json::json!(1)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn validate_nova_meta_returns_report_contract() {
    let root = std::env::current_dir()
        .expect("cwd")
        .canonicalize()
        .expect("canonical cwd");
    let project_dir = TempDir::new_in(&root).expect("temp project");
    let project_relative = project_dir
        .path()
        .strip_prefix(&root)
        .expect("relative temp project")
        .display()
        .to_string();
    let models_dir = project_dir.path().join("models");
    std::fs::create_dir_all(&models_dir).expect("models dir");
    std::fs::write(
        models_dir.join("orders.yml"),
        r"
version: 2
models:
  - name: fct_orders
    meta:
      nova:
        canonical: true
    columns:
      - name: order_id
        meta:
          nova:
            role: identifier
",
    )
    .expect("fixture");

    let temp_dir = TempDir::new().expect("temp dir");
    let mut config = test_config(temp_dir.path());
    config.mcp_max_response_bytes = 0;
    let handle = ManifestSearchHandle::spawn(config);
    handle
        .wait_ready()
        .await
        .expect("fixture manifest should load");
    let server = DbtNovaServer::new(handle);

    let response = tool_response_json(
        server
            .validate_nova_meta(Parameters(ValidateNovaMetaParams {
                project_dir: Some(project_relative),
                paths: vec!["models/orders.yml".to_string()],
                resource_kind: Some(crate::params::NovaMetaResourceKindParam::Model),
                resource_name: Some("fct_orders".to_string()),
                column: None,
            }))
            .await,
    );

    assert_eq!(response["success"], serde_json::json!(true));
    assert_eq!(response["count"], serde_json::json!(1));
    assert_eq!(response["data"]["target_count"], serde_json::json!(2));
    assert_eq!(response["data"]["error_count"], serde_json::json!(0));
    assert_eq!(
        response["data"]["selector"]["resource_kind"],
        serde_json::json!("model")
    );
    assert_eq!(
        response["data"]["selector"]["paths"],
        serde_json::json!(["models/orders.yml"])
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn validate_eval_suite_returns_report_contract() {
    let root = std::env::current_dir()
        .expect("cwd")
        .canonicalize()
        .expect("canonical cwd");
    let suite_dir = TempDir::new_in(&root).expect("temp suite dir");
    let suite_path = suite_dir.path().join("suite.yml");
    std::fs::write(
        &suite_path,
        r"
version: 1
name: mcp-eval-smoke
cases:
  - id: one
    assertions:
      - type: tool_success
        tool: search
        params: {}
",
    )
    .expect("suite fixture");

    let temp_dir = TempDir::new().expect("temp dir");
    let mut config = test_config(temp_dir.path());
    config.mcp_max_response_bytes = 0;
    let handle = ManifestSearchHandle::spawn(config);
    handle
        .wait_ready()
        .await
        .expect("fixture manifest should load");
    let server = DbtNovaServer::new(handle);

    let response = tool_response_json(
        server
            .validate_eval_suite(Parameters(ValidateEvalSuiteParams {
                suite: suite_path.display().to_string(),
            }))
            .await,
    );

    assert_eq!(response["success"], serde_json::json!(true));
    assert_eq!(response["count"], serde_json::json!(1));
    assert_eq!(response["data"]["valid"], serde_json::json!(true));
    assert_eq!(
        response["data"]["suite_name"],
        serde_json::json!("mcp-eval-smoke")
    );
    assert!(response["data"]["safety_policy"].is_object());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn compare_eval_runs_returns_markdown_contract() {
    let root = std::env::current_dir()
        .expect("cwd")
        .canonicalize()
        .expect("canonical cwd");
    let comparison_dir = TempDir::new_in(&root).expect("temp comparison dir");
    let before_dir = comparison_dir.path().join("before");
    let after_dir = comparison_dir.path().join("after");
    write_test_eval_results(&before_dir, "pass", 1.0);
    write_test_eval_results(&after_dir, "fail", 0.0);

    let temp_dir = TempDir::new().expect("temp dir");
    let mut config = test_config(temp_dir.path());
    config.mcp_max_response_bytes = 0;
    let handle = ManifestSearchHandle::spawn(config);
    handle
        .wait_ready()
        .await
        .expect("fixture manifest should load");
    let server = DbtNovaServer::new(handle);

    let response = tool_response_json(
        server
            .compare_eval_runs(Parameters(CompareEvalRunsParams {
                before: before_dir.display().to_string(),
                after: after_dir.display().to_string(),
            }))
            .await,
    );

    assert_eq!(response["success"], serde_json::json!(true));
    assert_eq!(
        response["data"]["schema_version"],
        serde_json::json!("eval_comparison.v1")
    );
    assert!(
        response["data"]["markdown"]
            .as_str()
            .expect("markdown")
            .contains("Newly failing")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn warm_manifest_rejects_without_mcp_opt_in() {
    let temp_dir = TempDir::new().expect("temp dir");
    let mut config = test_config(temp_dir.path());
    config.mcp_max_response_bytes = 0;
    let handle = ManifestSearchHandle::spawn(config);
    handle
        .wait_ready()
        .await
        .expect("fixture manifest should load");
    let server = DbtNovaServer::new(handle);

    let response = tool_response_json(
        server
            .warm_manifest(Parameters(WarmManifestParams {
                vector: true,
                ..WarmManifestParams::default()
            }))
            .await,
    );

    assert_eq!(response["success"], serde_json::json!(false));
    assert_eq!(response["error_code"], serde_json::json!("INVALID_PARAMS"));
    assert!(
        response["error"]
            .as_str()
            .unwrap_or_default()
            .contains("DBT_NOVA_MCP_ENABLE_MANIFEST_WARM=1")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reload_manifest_rejects_source_changes_without_mcp_opt_in() {
    let env_key = crate::cli::manifest::MCP_ENABLE_MANIFEST_RELOAD_ENV;
    let original = std::env::var_os(env_key);
    // SAFETY: this test is the only test that mutates this MCP reload opt-in variable.
    unsafe { std::env::remove_var(env_key) };

    let temp_dir = TempDir::new().expect("temp dir");
    let mut config = test_config(temp_dir.path());
    config.mcp_max_response_bytes = 0;
    let handle = ManifestSearchHandle::spawn(config);
    handle
        .wait_ready()
        .await
        .expect("fixture manifest should load");
    let server = DbtNovaServer::new(handle);

    let response = tool_response_json(
        server
            .reload_manifest(Parameters(ReloadManifestParams {
                manifest_path: Some(fixture_manifest_path_string()),
                ..ReloadManifestParams::default()
            }))
            .await,
    );

    match original {
        Some(value) => {
            // SAFETY: this test restores the process variable it changed above.
            unsafe { std::env::set_var(env_key, value) };
        }
        None => {
            // SAFETY: this test restores the process variable it changed above.
            unsafe { std::env::remove_var(env_key) };
        }
    }

    assert_eq!(response["success"], serde_json::json!(false));
    assert_eq!(response["error_code"], serde_json::json!("INVALID_PARAMS"));
    assert!(
        response["error"]
            .as_str()
            .unwrap_or_default()
            .contains("DBT_NOVA_MCP_ENABLE_MANIFEST_RELOAD=1")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn validate_nova_meta_rejects_unsafe_paths() {
    let root = std::env::current_dir()
        .expect("cwd")
        .canonicalize()
        .expect("canonical cwd");
    let project_dir = TempDir::new_in(&root).expect("temp project");
    let project_relative = project_dir
        .path()
        .strip_prefix(&root)
        .expect("relative temp project")
        .display()
        .to_string();

    let temp_dir = TempDir::new().expect("temp dir");
    let mut config = test_config(temp_dir.path());
    config.mcp_max_response_bytes = 0;
    let handle = ManifestSearchHandle::spawn(config);
    handle
        .wait_ready()
        .await
        .expect("fixture manifest should load");
    let server = DbtNovaServer::new(handle);

    let response = tool_response_json(
        server
            .validate_nova_meta(Parameters(ValidateNovaMetaParams {
                project_dir: Some(project_relative),
                paths: vec!["../Cargo.toml".to_string()],
                ..ValidateNovaMetaParams::default()
            }))
            .await,
    );

    assert_eq!(response["success"], serde_json::json!(false));
    assert_eq!(response["error_code"], serde_json::json!("INVALID_PARAMS"));
    assert!(
        response["error"]
            .as_str()
            .unwrap_or_default()
            .contains("must stay inside project_dir")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sql_concurrency_rejects_when_limit_is_saturated() {
    let temp_dir = TempDir::new().expect("temp dir");
    let mut config = test_config(temp_dir.path());
    config.sql_max_concurrent = 1;
    config.sql_max_queue = 0;
    config.sql_queue_timeout_ms = 5;

    let handle = ManifestSearchHandle::spawn(config);
    handle
        .wait_ready()
        .await
        .expect("fixture manifest should load");
    let server = DbtNovaServer::new(handle);
    let searcher = server.searcher.get().await.expect("searcher ready");

    let first_permit = server
        .acquire_sql_permit(searcher.config(), Some(Duration::from_millis(5)))
        .await
        .expect("first SQL permit")
        .expect("permit should be enabled");

    let second_attempt = server
        .acquire_sql_permit(searcher.config(), Some(Duration::from_millis(5)))
        .await;
    let err = second_attempt.expect_err("second SQL permit should be rejected");
    assert!(err.to_string().contains("SQL concurrency limit exceeded"));

    drop(first_permit);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn search_concurrency_rejects_indicator_inventory_when_limit_is_saturated() {
    let temp_dir = TempDir::new().expect("temp dir");
    let mut config = test_config(temp_dir.path());
    config.search.search_max_concurrent = 1;
    config.search.search_max_queue = 0;
    config.search.search_timeout_ms = 50;

    let handle = ManifestSearchHandle::spawn(config);
    handle
        .wait_ready()
        .await
        .expect("fixture manifest should load");
    let server = DbtNovaServer::new(handle);
    let searcher = server.searcher.get().await.expect("searcher ready");

    let first_permit = server
        .acquire_search_permit(searcher.config(), Some(Duration::from_millis(5)))
        .await
        .expect("first search permit")
        .expect("permit should be enabled");

    let response = tool_response_json(
        server
            .indicator_inventory(Parameters(IndicatorInventoryParams::default()))
            .await,
    );
    assert_eq!(response["success"], serde_json::json!(false));
    assert!(
        response["error"]
            .as_str()
            .unwrap_or_default()
            .contains("Search concurrency limit exceeded")
    );

    drop(first_permit);
}
