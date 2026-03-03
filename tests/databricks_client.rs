//! Integration tests for the Databricks SQL client (HTTP mock).
use std::time::Duration;

use dbt_nova::warehouse::databricks::{DatabricksSqlClient, DatabricksSqlConfig, ExecuteOptions};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn test_config(host: String) -> DatabricksSqlConfig {
    DatabricksSqlConfig {
        host,
        token: "test-token".to_string(),
        warehouse_id: "test-warehouse".to_string(),
        timeout: Duration::from_secs(5),
        default_wait_timeout_s: 5,
        poll_interval: Duration::from_millis(5),
        max_poll: Duration::from_millis(100),
        max_get_retries: 0,
    }
}

#[tokio::test]
#[ignore = "requires local socket bind for wiremock; run explicitly in environments that allow loopback bind"]
async fn databricks_query_succeeds_inline() {
    // Mock a one-shot SQL response where results are embedded inline.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/2.0/sql/statements"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "statement_id": "stmt-1",
            "status": { "state": "SUCCEEDED" },
            "manifest": {
                "schema": { "columns": [{ "name": "col1", "type_name": "STRING" }] },
                "chunks": [{ "chunk_index": 0 }],
                "truncated": false,
                "total_chunk_count": 1,
                "total_row_count": 1,
                "total_byte_count": 10
            },
            "result": {
                "chunk_index": 0,
                "data_array": [["hello"]]
            }
        })))
        .mount(&server)
        .await;

    let client = DatabricksSqlClient::new(test_config(server.uri())).unwrap();
    let result = client.query("select 'hello'").await.unwrap();

    assert_eq!(result.columns, vec!["col1".to_string()]);
    assert_eq!(result.column_types, vec!["STRING".to_string()]);
    assert_eq!(result.rows.len(), 1);
}

#[tokio::test]
#[ignore = "requires local socket bind for wiremock; run explicitly in environments that allow loopback bind"]
async fn databricks_query_polls_and_fetches_chunks() {
    // Mock a multi-step query that transitions from RUNNING to SUCCEEDED and fetches chunks.
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/api/2.0/sql/statements"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "statement_id": "stmt-2",
            "status": { "state": "RUNNING" }
        })))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/api/2.0/sql/statements/stmt-2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "statement_id": "stmt-2",
            "status": { "state": "SUCCEEDED" },
            "manifest": {
                "schema": { "columns": [{ "name": "c1", "type_name": "LONG" }] },
                "chunks": [{ "chunk_index": 0 }, { "chunk_index": 1 }],
                "truncated": false,
                "total_chunk_count": 2,
                "total_row_count": 2,
                "total_byte_count": 20
            },
            "result": { "chunk_index": 0, "data_array": [[1]] }
        })))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/api/2.0/sql/statements/stmt-2/result/chunks/1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "chunk_index": 1,
            "data_array": [[2]]
        })))
        .mount(&server)
        .await;

    let client = DatabricksSqlClient::new(test_config(server.uri())).unwrap();
    let result = client
        .execute(
            "select 1 union all select 2",
            ExecuteOptions {
                poll_interval: Some(Duration::from_millis(1)),
                max_poll: Some(Duration::from_millis(50)),
                ..ExecuteOptions::default()
            },
        )
        .await
        .unwrap();

    assert_eq!(result.columns, vec!["c1".to_string()]);
    assert_eq!(result.rows.len(), 2);
    assert_eq!(result.fetched_chunks, 2);
}

#[tokio::test]
#[ignore = "requires local socket bind for wiremock; run explicitly in environments that allow loopback bind"]
async fn databricks_query_failed_returns_error() {
    // Mock a FAILED status to ensure error surfaces with message context.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/2.0/sql/statements"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "statement_id": "stmt-3",
            "status": { "state": "FAILED", "error": { "message": "boom" } }
        })))
        .mount(&server)
        .await;

    let client = DatabricksSqlClient::new(test_config(server.uri())).unwrap();
    let err = client.query("select explode()").await.err().unwrap();
    let msg = format!("{err}");
    assert!(msg.contains("FAILED"), "unexpected error: {msg}");
}
