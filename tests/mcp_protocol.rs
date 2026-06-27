//! Protocol-level MCP smoke test over stdio transport.
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::time::{sleep, timeout};

use dbt_nova::tools::catalog::{MCP_TOOL_COUNT, MCP_TOOL_NAMES};

const MCP_SMOKE_ENV_REMOVE: &[&str] = &[
    "DBT_NOVA_MANIFEST_URI",
    "DBT_NOVA_BOOTSTRAP_URI",
    "DBT_NOVA_STORAGE_ARTIFACT_URI",
    "DBT_NOVA_METADATA_ARTIFACT_URI",
    "DBT_NOVA_MODELS_ARTIFACT_URI",
    "DBT_NOVA_PRUNE_ALLOW_IDS",
    "DBT_NOVA_PRUNE_DENY_IDS",
    "DBT_NOVA_SERVER_TRANSPORT",
    "DBT_NOVA_TOOL_ALLOWLIST",
    "DBT_NOVA_TOOL_DENYLIST",
    "DBT_NOVA_TRACE_TOOL_CALLS_PATH",
    "DBT_NOVA_STORAGE_READ_ONLY",
    "DBT_NOVA_TOOL_RATE_LIMITS",
    "DBT_NOVA_TOOL_RATE_LIMIT_WINDOW_SECS",
    "DBT_NOVA_RESULT_PROFILE",
    "DBT_NOVA_MCP_RESULT_PROFILE",
    "DBT_NOVA_MCP_MAX_RESPONSE_BYTES",
    "DBT_NOVA_MCP_ENABLE_EVAL_RUN",
    "DBT_NOVA_MCP_ENABLE_EVAL_WRITES",
    "DBT_NOVA_MCP_ENABLE_AGENT_EVAL",
    "DBT_NOVA_MCP_ENABLE_CUSTOM_AGENT_PROVIDER",
    "DBT_NOVA_MCP_ENABLE_TRACE_WRITES",
    "DBT_NOVA_MCP_ENABLE_MANIFEST_RELOAD",
    "DBT_NOVA_MCP_ENABLE_MANIFEST_WARM",
    "DBT_NOVA_MCP_ENABLE_STORAGE_ADMIN",
];

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

async fn initialize_stdio_session(stdin: &mut ChildStdin, stdout: &mut BufReader<ChildStdout>) {
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

fn fixture_manifest_path(file_name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(file_name)
}

fn relative_to_repo(path: &Path) -> String {
    path.strip_prefix(env!("CARGO_MANIFEST_DIR"))
        .expect("test artifact should be under repo root")
        .to_string_lossy()
        .to_string()
}

fn scrub_mcp_smoke_env(command: &mut Command) {
    for env_key in MCP_SMOKE_ENV_REMOVE {
        command.env_remove(env_key);
    }
    command.env("DBT_NOVA_SERVER_TRANSPORT", "stdio");
    command.env("DBT_NOVA_STORAGE_READ_ONLY", "false");
}

async fn spawn_stdio_server(
    manifest_path: &Path,
    storage_dir: &Path,
    storage_instance_id: &str,
) -> (Child, ChildStdin, BufReader<ChildStdout>) {
    let binary = PathBuf::from(env!("CARGO_BIN_EXE_dbt-nova"));
    assert!(
        binary.exists(),
        "dbt-nova binary not found at {}",
        binary.display()
    );

    let duckdb_path = storage_dir.join("catalog-smoke.duckdb");
    drop(duckdb::Connection::open(&duckdb_path).expect("create DuckDB smoke database file"));
    let mut command = Command::new(&binary);
    command
        .arg("server")
        .arg("start")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .env("DBT_MANIFEST_PATH", manifest_path)
        .env("DBT_NOVA_STORAGE_DIR", storage_dir)
        .env("DBT_NOVA_STORAGE_INSTANCE_ID", storage_instance_id)
        .env("DBT_NOVA_SEARCH_ENABLE_VECTOR", "false")
        .env("DBT_NOVA_SEARCH_ENABLE_SPARSE", "false")
        .env("DBT_NOVA_SEARCH_ENABLE_RERANKER", "false")
        .env("DBT_NOVA_SQL_PROVIDER", "duckdb")
        .env("DBT_NOVA_DUCKDB_PATH", duckdb_path);
    scrub_mcp_smoke_env(&mut command);
    let mut child = command
        .spawn()
        .expect("failed to spawn dbt-nova MCP server");

    let stdin = child.stdin.take().expect("child stdin unavailable");
    let stdout = child.stdout.take().expect("child stdout unavailable");
    (child, stdin, BufReader::new(stdout))
}

async fn call_tool(
    stdin: &mut ChildStdin,
    stdout: &mut BufReader<ChildStdout>,
    request_id: i64,
    name: &str,
    arguments: Value,
) -> Value {
    write_json_line(
        stdin,
        &json!({
            "jsonrpc": "2.0",
            "id": request_id,
            "method": "tools/call",
            "params": {
                "name": name,
                "arguments": arguments
            }
        }),
    )
    .await
    .unwrap_or_else(|error| panic!("failed to write tools/call for {name}: {error}"));

    let response = read_response(stdout, request_id).await;
    assert!(
        response.get("error").is_none(),
        "{name} returned JSON-RPC error: {response}"
    );
    let text = response
        .get("result")
        .and_then(|result| result.get("content"))
        .and_then(Value::as_array)
        .and_then(|content| content.first())
        .and_then(|entry| entry.get("text"))
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{name} response missing result.content[0].text: {response}"));
    serde_json::from_str(text)
        .unwrap_or_else(|error| panic!("{name} response text was not JSON ({error}): {text}"))
}

async fn wait_for_ready(
    stdin: &mut ChildStdin,
    stdout: &mut BufReader<ChildStdout>,
    next_request_id: &mut i64,
) {
    for _ in 0..60 {
        let payload = call_tool(stdin, stdout, *next_request_id, "health", json!({})).await;
        *next_request_id += 1;
        if payload["data"]["ready_for_traffic"] == json!(true) {
            return;
        }
        sleep(Duration::from_millis(100)).await;
    }
    panic!("MCP server did not report ready_for_traffic=true");
}

fn write_eval_results(dir: &Path, status: &str, pass_rate: f64) {
    std::fs::create_dir_all(dir).expect("create eval result dir");
    let passed = status == "pass";
    let payload = json!({
        "suite_name": "mcp-protocol-smoke",
        "version": 1,
        "mode": "bridge",
        "output_dir": dir.display().to_string(),
        "assertion_count": 1,
        "pass_count": usize::from(passed),
        "fail_count": usize::from(!passed),
        "error_count": 0,
        "pass_rate": pass_rate,
        "gate_status": status,
        "cases": [{
            "id": "case",
            "pass_count": usize::from(passed),
            "fail_count": usize::from(!passed),
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

fn success_tool_cases(workspace: &Path) -> Vec<(&'static str, Value)> {
    let fct_orders = "model.starter_eval.fct_orders";
    let stg_orders = "model.starter_eval.stg_orders";
    let dim_customers = "model.starter_eval.dim_customers";
    let trace_path = relative_to_repo(&workspace.join("trace.jsonl"));
    let before_dir = relative_to_repo(&workspace.join("before"));
    let after_dir = relative_to_repo(&workspace.join("after"));
    vec![
        (
            "search",
            json!({"query":"gross revenue orders","resource_types":["model"],"limit":3}),
        ),
        (
            "search_indicator",
            json!({"query":"gross revenue","indicator_types":["measure"],"resource_types":["model"],"limit":3,"detail":"compact","group_mode":"top"}),
        ),
        (
            "indicator_inventory",
            json!({"resource_types":["model"],"limit":3}),
        ),
        (
            "search_columns",
            json!({"query":"order date","resource_types":["model"],"limit":3}),
        ),
        (
            "column_inventory",
            json!({"resource_types":["model"],"limit":3}),
        ),
        (
            "compare_grains",
            json!({"entity1":fct_orders,"entity2":dim_customers}),
        ),
        (
            "find_entity_overlap",
            json!({"id_or_name":fct_orders,"resource_types":["model"],"limit":3}),
        ),
        (
            "modelling_consistency_report",
            json!({"resource_types":["model"],"limit":3}),
        ),
        (
            "get_entity",
            json!({"id_or_name":fct_orders,"detail":"compact"}),
        ),
        (
            "list_entities",
            json!({"resource_type":"model","limit":3,"detail":"compact"}),
        ),
        (
            "get_lineage",
            json!({"id_or_name":fct_orders,"direction":"upstream","depth":1,"detail":"compact"}),
        ),
        ("get_sql", json!({"id_or_name":fct_orders,"compiled":true})),
        ("get_columns", json!({"id_or_name":fct_orders})),
        (
            "diff_entities",
            json!({"entity1":stg_orders,"entity2":fct_orders}),
        ),
        ("get_impact", json!({"id_or_name":stg_orders})),
        ("validate_dag", json!({"detail":"summary"})),
        (
            "validate_nova_meta",
            json!({"project_dir":"tests/fixtures"}),
        ),
        ("validate_eval_suite", json!({"suite":"evals/starter.yml"})),
        ("get_eval_gate", json!({"suite":"starter"})),
        (
            "get_eval_history",
            json!({"suite":"starter","since":"2026-01-01"}),
        ),
        (
            "compare_eval_runs",
            json!({"before":before_dir,"after":after_dir}),
        ),
        ("inspect_tool_trace", json!({"path":trace_path})),
        ("summarize_tool_trace", json!({"path":trace_path})),
        ("replay_tool_trace", json!({"path":trace_path})),
        ("show_metadata", json!({})),
        ("health", json!({})),
        ("show_config", json!({"defaults":false})),
        ("validate_config", json!({})),
        ("inspect_storage", json!({})),
        ("list_tags", json!({})),
        ("list_packages", json!({})),
        ("list_databases", json!({})),
        (
            "get_column_lineage",
            json!({"id_or_name":fct_orders,"column_name":"customer_id","direction":"upstream","depth":1}),
        ),
        (
            "get_test_coverage",
            json!({"id_or_name":fct_orders,"include_full":false,"columns_limit":5}),
        ),
        (
            "get_metadata_score",
            json!({"id_or_name":fct_orders,"persona":"analyst","include_breakdown":false,"include_recommendations":false}),
        ),
        (
            "get_metadata_audit",
            json!({"selection_mode":"entities","entity_ids_json":"[\"model.starter_eval.fct_orders\"]","resource_types_json":"[\"model\"]","personas_json":"[\"analyst\"]","include_breakdown":false,"include_recommendations":false}),
        ),
        (
            "get_agent_readiness",
            json!({"personas_json":"[\"analyst\"]"}),
        ),
        (
            "batch_get_entities",
            json!({"unique_ids":[fct_orders,dim_customers],"detail":"compact"}),
        ),
        (
            "find_by_path",
            json!({"path_pattern":"models/marts/*.sql","resource_types":["model"],"limit":3,"detail":"compact"}),
        ),
        (
            "search_recipes",
            json!({"query":"weekly revenue","include_queries":true,"limit":3}),
        ),
        (
            "get_recipe",
            json!({"recipe_id":"commerce/weekly_revenue","include_queries":true,"include_sql":false}),
        ),
        (
            "get_undocumented",
            json!({"resource_type":"model","id_or_name":"model.starter_eval.raw_events_sparse","include_columns":true,"include_full":false,"limit":3}),
        ),
        (
            "get_context",
            json!({"id_or_name":fct_orders,"include_columns":true,"include_upstream":true,"include_downstream":false,"include_tests":true,"include_docs":false,"include_sql":false,"lineage_depth":1}),
        ),
        (
            "execute_sql",
            json!({"statement":"select 1 as ok","row_limit":1,"fetch_all_chunks":false}),
        ),
    ]
}

fn expected_error_tool_cases(workspace: &Path) -> Vec<(&'static str, Value, &'static str)> {
    let trace_path = relative_to_repo(&workspace.join("trace.jsonl"));
    vec![
        (
            "reload_manifest",
            json!({"manifest_path":"tests/fixtures/starter_eval_manifest.json"}),
            "INVALID_PARAMS",
        ),
        ("warm_manifest", json!({}), "INVALID_PARAMS"),
        (
            "run_eval",
            json!({"suite":"evals/starter.yml","case_ids":["canonical_revenue_discovery"]}),
            "INVALID_PARAMS",
        ),
        (
            "init_eval_suite",
            json!({"persona":"analyst","out":relative_to_repo(&workspace.join("generated.yml"))}),
            "INVALID_PARAMS",
        ),
        (
            "run_agent_eval",
            json!({"suite":"evals/starter.yml","provider":"opencode","case_ids":["analyst_revenue_lookup_flow"],"timeout_secs":1}),
            "INVALID_PARAMS",
        ),
        (
            "redact_tool_trace",
            json!({"path":trace_path,"out":relative_to_repo(&workspace.join("redacted.jsonl"))}),
            "INVALID_PARAMS",
        ),
        ("prune_storage", json!({"max_keep":1}), "INVALID_PARAMS"),
        ("cleanup_storage", json!({}), "INVALID_PARAMS"),
        (
            "run_recipe",
            json!({"recipe_id":"missing/recipe","query_indexes":[1],"row_limit":1}),
            "INVALID_PARAMS",
        ),
    ]
}

#[tokio::test(flavor = "multi_thread")]
async fn mcp_stdio_round_trip_supports_initialize_and_tools_list() {
    let manifest_path = fixture_manifest_path("nova_manifest.json");
    let storage_dir = tempfile::tempdir().expect("tempdir for protocol smoke storage");

    let (mut child, mut stdin, mut stdout) =
        spawn_stdio_server(&manifest_path, storage_dir.path(), "tests-mcp-protocol").await;

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
    let manifest_path = fixture_manifest_path("nova_manifest.json");
    let storage_dir = tempfile::tempdir().expect("tempdir for protocol smoke storage");

    let mut command = Command::new(&binary);
    command
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
        .env("DBT_NOVA_SEARCH_ENABLE_RERANKER", "false");
    scrub_mcp_smoke_env(&mut command);
    command
        .env("DBT_NOVA_TOOL_ALLOWLIST", "search,execute_sql")
        .env("DBT_NOVA_TOOL_DENYLIST", "execute_sql");
    let mut child = command
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

#[tokio::test(flavor = "multi_thread")]
async fn mcp_stdio_no_warm_catalog_smoke_calls_every_tool_after_readiness() {
    let manifest_path = fixture_manifest_path("starter_eval_manifest.json");
    let storage_dir = tempfile::tempdir().expect("tempdir for protocol smoke storage");
    let workspace = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR"))
        .expect("temp workspace under repo root for scoped MCP paths");
    let trace_path = workspace.path().join("trace.jsonl");
    std::fs::write(
        &trace_path,
        r#"{"tool":"search","success":true,"response_bytes":42,"response_truncated":false,"params_summary":{"query":"gross revenue","resource_types":["model"],"limit":3}}"#,
    )
    .expect("write trace fixture");
    write_eval_results(&workspace.path().join("before"), "pass", 1.0);
    write_eval_results(&workspace.path().join("after"), "fail", 0.0);

    let (mut child, mut stdin, mut stdout) = spawn_stdio_server(
        &manifest_path,
        storage_dir.path(),
        "tests-mcp-catalog-smoke",
    )
    .await;

    initialize_stdio_session(&mut stdin, &mut stdout).await;

    let mut next_request_id = 2;
    wait_for_ready(&mut stdin, &mut stdout, &mut next_request_id).await;

    let mut called = Vec::new();
    for (name, arguments) in success_tool_cases(workspace.path()) {
        let payload = call_tool(&mut stdin, &mut stdout, next_request_id, name, arguments).await;
        next_request_id += 1;
        assert_ne!(
            payload.get("error_code").and_then(Value::as_str),
            Some("INDEX_BUILDING"),
            "{name} returned INDEX_BUILDING after readiness: {payload}"
        );
        assert_eq!(
            payload.get("success"),
            Some(&json!(true)),
            "{name} should succeed: {payload}"
        );
        called.push(name);
    }

    for (name, arguments, expected_error_code) in expected_error_tool_cases(workspace.path()) {
        let payload = call_tool(&mut stdin, &mut stdout, next_request_id, name, arguments).await;
        next_request_id += 1;
        assert_ne!(
            payload.get("error_code").and_then(Value::as_str),
            Some("INDEX_BUILDING"),
            "{name} returned INDEX_BUILDING after readiness: {payload}"
        );
        assert_eq!(
            payload.get("success"),
            Some(&json!(false)),
            "{name} should return a tool error payload: {payload}"
        );
        assert_eq!(
            payload.get("error_code").and_then(Value::as_str),
            Some(expected_error_code),
            "{name} returned unexpected error payload: {payload}"
        );
        called.push(name);
    }

    called.sort_unstable();
    let mut expected = MCP_TOOL_NAMES.to_vec();
    expected.sort_unstable();
    assert_eq!(
        called, expected,
        "catalog smoke must exercise every MCP tool exactly once"
    );

    child.start_kill().expect("failed to terminate MCP child");
    let _ = child.wait().await;
}
