//! Protocol-level MCP smoke test over stdio transport.
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdout, Command};
use tokio::time::timeout;

use dbt_nova::tools::catalog::MCP_TOOL_COUNT;

async fn write_json_line(
    stdin: &mut tokio::process::ChildStdin,
    payload: &Value,
) -> Result<(), std::io::Error> {
    stdin.write_all(payload.to_string().as_bytes()).await?;
    stdin.write_all(b"\n").await?;
    stdin.flush().await
}

async fn read_message(stdout: &mut BufReader<ChildStdout>, max_wait: Duration) -> Value {
    loop {
        let mut line = String::new();
        let read = timeout(max_wait, stdout.read_line(&mut line))
            .await
            .expect("timed out waiting for MCP message")
            .expect("failed reading MCP message");
        assert!(read > 0, "MCP server closed stdio unexpectedly");
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        return serde_json::from_str(line).expect("MCP server emitted non-JSON output on stdout");
    }
}

fn id_matches(message: &Value, expected: i64) -> bool {
    message
        .get("id")
        .and_then(Value::as_i64)
        .is_some_and(|id| id == expected)
        || message
            .get("id")
            .and_then(Value::as_str)
            .is_some_and(|id| id == expected.to_string())
}

async fn read_response(stdout: &mut BufReader<ChildStdout>, request_id: i64) -> Value {
    loop {
        let message = read_message(stdout, Duration::from_secs(20)).await;
        if id_matches(&message, request_id) {
            return message;
        }
    }
}

async fn initialize_stdio_session(
    stdin: &mut tokio::process::ChildStdin,
    stdout: &mut BufReader<ChildStdout>,
) {
    write_json_line(
        stdin,
        &json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": {"name": "dbt-nova-test", "version": "1.0.0"}
            }
        }),
    )
    .await
    .expect("failed to write initialize request");

    let initialize_response = read_response(stdout, 1).await;
    assert!(
        initialize_response.get("result").is_some(),
        "initialize response should include result, got: {initialize_response}"
    );

    write_json_line(
        stdin,
        &json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
            "params": {}
        }),
    )
    .await
    .expect("failed to write initialized notification");
}

#[tokio::test(flavor = "multi_thread")]
async fn mcp_stdio_round_trip_supports_initialize_and_tools_list() {
    let binary = PathBuf::from(env!("CARGO_BIN_EXE_dbt-nova"));
    assert!(
        binary.exists(),
        "dbt-nova binary not found at {}",
        binary.display()
    );
    let manifest_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("nova_manifest.json");
    let storage_dir = tempfile::tempdir().expect("tempdir for protocol smoke storage");

    let mut child = Command::new(&binary)
        .arg("server")
        .arg("start")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .env("DBT_MANIFEST_PATH", &manifest_path)
        .env("DBT_NOVA_STORAGE_DIR", storage_dir.path())
        .env("DBT_NOVA_STORAGE_INSTANCE_ID", "tests-mcp-protocol")
        .env("DBT_NOVA_SEARCH_ENABLE_VECTOR", "false")
        .env("DBT_NOVA_SEARCH_ENABLE_SPARSE", "false")
        .env("DBT_NOVA_SEARCH_ENABLE_RERANKER", "false")
        .spawn()
        .expect("failed to spawn dbt-nova MCP server");

    let mut stdin = child.stdin.take().expect("child stdin unavailable");
    let stdout = child.stdout.take().expect("child stdout unavailable");
    let mut stdout = BufReader::new(stdout);

    initialize_stdio_session(&mut stdin, &mut stdout).await;

    write_json_line(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list",
            "params": {}
        }),
    )
    .await
    .expect("failed to write tools/list request");

    let tools_response = read_response(&mut stdout, 2).await;
    let tools = tools_response
        .get("result")
        .and_then(|result| result.get("tools"))
        .and_then(Value::as_array)
        .expect("tools/list response missing result.tools");
    assert_eq!(tools.len(), MCP_TOOL_COUNT);
    let has_search = tools
        .iter()
        .any(|tool| tool.get("name").and_then(Value::as_str) == Some("search"));
    assert!(has_search, "tools/list should include search tool");

    child.start_kill().expect("failed to terminate MCP child");
    let _ = child.wait().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn mcp_stdio_honors_tool_allowlist_and_denylist() {
    let binary = PathBuf::from(env!("CARGO_BIN_EXE_dbt-nova"));
    let manifest_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("nova_manifest.json");
    let storage_dir = tempfile::tempdir().expect("tempdir for protocol smoke storage");

    let mut child = Command::new(&binary)
        .arg("server")
        .arg("start")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .env("DBT_MANIFEST_PATH", &manifest_path)
        .env("DBT_NOVA_STORAGE_DIR", storage_dir.path())
        .env("DBT_NOVA_STORAGE_INSTANCE_ID", "tests-mcp-tool-filtering")
        .env("DBT_NOVA_SEARCH_ENABLE_VECTOR", "false")
        .env("DBT_NOVA_SEARCH_ENABLE_SPARSE", "false")
        .env("DBT_NOVA_SEARCH_ENABLE_RERANKER", "false")
        .env("DBT_NOVA_TOOL_ALLOWLIST", "search,execute_sql")
        .env("DBT_NOVA_TOOL_DENYLIST", "execute_sql")
        .spawn()
        .expect("failed to spawn dbt-nova MCP server");

    let mut stdin = child.stdin.take().expect("child stdin unavailable");
    let stdout = child.stdout.take().expect("child stdout unavailable");
    let mut stdout = BufReader::new(stdout);

    initialize_stdio_session(&mut stdin, &mut stdout).await;

    write_json_line(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list",
            "params": {}
        }),
    )
    .await
    .expect("failed to write tools/list request");

    let tools_response = read_response(&mut stdout, 2).await;
    let tools = tools_response
        .get("result")
        .and_then(|result| result.get("tools"))
        .and_then(Value::as_array)
        .expect("tools/list response missing result.tools");
    assert_eq!(
        tools.len(),
        1,
        "unexpected tools/list payload: {tools_response}"
    );
    assert_eq!(tools[0].get("name").and_then(Value::as_str), Some("search"));

    write_json_line(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "execute_sql",
                "arguments": {"sql": "select 1"}
            }
        }),
    )
    .await
    .expect("failed to write tools/call request");

    let call_response = read_response(&mut stdout, 3).await;
    assert!(
        call_response.get("error").is_some(),
        "filtered tool call should fail: {call_response}"
    );

    child.start_kill().expect("failed to terminate MCP child");
    let _ = child.wait().await;
}
