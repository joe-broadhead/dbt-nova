use std::collections::HashMap;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use dbt_nova::params::{ExecuteSqlParams, RunRecipeParams};
use dbt_nova::warehouse::SqlProvider;
use dbt_nova::warehouse::duckdb::DUCKDB_PROVIDER;
use dbt_nova::{DbtNovaConfig, ManifestSearch};
use serde_json::{Value as JsonValue, json};
use tempfile::TempDir;

#[path = "support/config.rs"]
mod support_config;

static ENV_MUTEX: OnceLock<Mutex<()>> = OnceLock::new();

fn lock_env() -> std::sync::MutexGuard<'static, ()> {
    ENV_MUTEX
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|err| err.into_inner())
}

struct EnvGuard {
    old_duckdb_path: Option<String>,
    old_file_search_path: Option<String>,
    old_allow_external_access: Option<String>,
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match &self.old_duckdb_path {
            Some(value) => {
                // SAFETY: tests serialize env mutation through `ENV_MUTEX`.
                unsafe { std::env::set_var("DBT_NOVA_DUCKDB_PATH", value) };
            }
            None => {
                // SAFETY: tests serialize env mutation through `ENV_MUTEX`.
                unsafe { std::env::remove_var("DBT_NOVA_DUCKDB_PATH") };
            }
        }
        match &self.old_file_search_path {
            Some(value) => {
                // SAFETY: tests serialize env mutation through `ENV_MUTEX`.
                unsafe { std::env::set_var("DBT_NOVA_DUCKDB_FILE_SEARCH_PATH", value) };
            }
            None => {
                // SAFETY: tests serialize env mutation through `ENV_MUTEX`.
                unsafe { std::env::remove_var("DBT_NOVA_DUCKDB_FILE_SEARCH_PATH") };
            }
        }
        match &self.old_allow_external_access {
            Some(value) => {
                // SAFETY: tests serialize env mutation through `ENV_MUTEX`.
                unsafe { std::env::set_var("DBT_NOVA_DUCKDB_ALLOW_EXTERNAL_ACCESS", value) };
            }
            None => {
                // SAFETY: tests serialize env mutation through `ENV_MUTEX`.
                unsafe { std::env::remove_var("DBT_NOVA_DUCKDB_ALLOW_EXTERNAL_ACCESS") };
            }
        }
    }
}

fn configure_duckdb_env(path: Option<&Path>, file_search_path: Option<&str>) -> EnvGuard {
    configure_duckdb_env_with_external_access(path, file_search_path, false)
}

fn configure_duckdb_env_with_external_access(
    path: Option<&Path>,
    file_search_path: Option<&str>,
    allow_external_access: bool,
) -> EnvGuard {
    let guard = EnvGuard {
        old_duckdb_path: std::env::var("DBT_NOVA_DUCKDB_PATH").ok(),
        old_file_search_path: std::env::var("DBT_NOVA_DUCKDB_FILE_SEARCH_PATH").ok(),
        old_allow_external_access: std::env::var("DBT_NOVA_DUCKDB_ALLOW_EXTERNAL_ACCESS").ok(),
    };

    match path {
        Some(path) => {
            // SAFETY: tests serialize env mutation through `ENV_MUTEX`.
            unsafe { std::env::set_var("DBT_NOVA_DUCKDB_PATH", path.to_string_lossy().as_ref()) };
        }
        None => {
            // SAFETY: tests serialize env mutation through `ENV_MUTEX`.
            unsafe { std::env::remove_var("DBT_NOVA_DUCKDB_PATH") };
        }
    }
    match file_search_path {
        Some(value) => {
            // SAFETY: tests serialize env mutation through `ENV_MUTEX`.
            unsafe { std::env::set_var("DBT_NOVA_DUCKDB_FILE_SEARCH_PATH", value) };
        }
        None => {
            // SAFETY: tests serialize env mutation through `ENV_MUTEX`.
            unsafe { std::env::remove_var("DBT_NOVA_DUCKDB_FILE_SEARCH_PATH") };
        }
    }
    if allow_external_access {
        // SAFETY: tests serialize env mutation through `ENV_MUTEX`.
        unsafe { std::env::set_var("DBT_NOVA_DUCKDB_ALLOW_EXTERNAL_ACCESS", "true") };
    } else {
        // SAFETY: tests serialize env mutation through `ENV_MUTEX`.
        unsafe { std::env::remove_var("DBT_NOVA_DUCKDB_ALLOW_EXTERNAL_ACCESS") };
    }

    guard
}

fn create_fixture_database() -> (TempDir, PathBuf) {
    let temp_dir = TempDir::new().expect("temp dir");
    let db_path = temp_dir.path().join("provider.duckdb");

    let connection = duckdb::Connection::open(&db_path).expect("open db");
    connection
        .execute_batch(
            "CREATE SCHEMA analytics;
             CREATE TABLE analytics.orders (
                 order_id INTEGER,
                 country_code TEXT,
                 amount INTEGER
             );
             INSERT INTO analytics.orders VALUES
                 (1, 'GB', 120),
                 (2, 'GB', 140),
                 (3, 'US', 220);",
        )
        .expect("seed db");

    (temp_dir, db_path)
}

fn create_external_csv(temp_dir: &TempDir) -> (PathBuf, PathBuf) {
    let external_dir = temp_dir.path().join("external_data");
    std::fs::create_dir_all(&external_dir).expect("create external data directory");
    let csv_path = external_dir.join("external_orders.csv");
    std::fs::write(
        &csv_path,
        "order_id,country_code,amount\n1,GB,120\n2,GB,140\n3,US,220\n",
    )
    .expect("write external csv");
    (external_dir, csv_path)
}

struct ToolSearchEnv {
    searcher: ManifestSearch,
    _guard: support_config::TestStorageGuard,
}

impl Deref for ToolSearchEnv {
    type Target = ManifestSearch;

    fn deref(&self) -> &Self::Target {
        &self.searcher
    }
}

fn create_duckdb_searcher_for_fixture(fixture_name: &str) -> ToolSearchEnv {
    let manifest_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(fixture_name);
    assert!(
        manifest_path.exists(),
        "fixture manifest missing at {}",
        manifest_path.display()
    );

    let guard = support_config::TestStorageGuard::new();
    let mut cfg = DbtNovaConfig {
        manifest_path: manifest_path.to_string_lossy().to_string(),
        search: support_config::test_search_config(),
        sql_provider: "duckdb".to_string(),
        ..Default::default()
    };
    support_config::apply_test_storage(&mut cfg, &guard);
    let searcher = ManifestSearch::new(cfg)
        .expect("create manifest searcher")
        .search;
    ToolSearchEnv {
        searcher,
        _guard: guard,
    }
}

fn create_duckdb_searcher() -> ToolSearchEnv {
    create_duckdb_searcher_for_fixture("nova_manifest.json")
}

fn base_params(statement: &str) -> ExecuteSqlParams {
    ExecuteSqlParams {
        statement: statement.to_string(),
        warehouse_id: None,
        preflight_only: false,
        preflight_catalog: None,
        preflight_schema: None,
        preflight_relation: None,
        row_limit: None,
        byte_limit: None,
        wait_timeout_s: None,
        poll_interval_ms: None,
        max_poll_seconds: None,
        parameters: None,
        parameter_types: None,
        fetch_all_chunks: None,
        max_chunks: None,
    }
}

#[test]
fn duckdb_execute_supports_named_params_and_row_limit_truncation() {
    let _env_lock = lock_env();
    let (_temp_dir, db_path) = create_fixture_database();
    let _env_guard = configure_duckdb_env(Some(&db_path), None);

    let mut params = base_params(
        "SELECT order_id, country_code, amount
         FROM analytics.orders
         WHERE country_code = :country
         ORDER BY order_id",
    );
    params.row_limit = Some(1);
    params.parameters = Some(HashMap::from([("country".to_string(), json!("GB"))]));

    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    let payload = runtime
        .block_on(DUCKDB_PROVIDER.execute(&params))
        .expect("duckdb execute should succeed");

    assert_eq!(payload["success"], json!(true));
    assert_eq!(payload["data"]["provider"], json!("duckdb"));
    assert_eq!(payload["data"]["truncated"], json!(true));
    assert_eq!(payload["data"]["rows"].as_array().map_or(0, Vec::len), 1);
    assert_eq!(
        payload["data"]["rows"][0],
        json!([1, "GB", 120]),
        "first ordered GB row should be returned"
    );
}

#[test]
fn duckdb_execute_honors_byte_limit_truncation() {
    let _env_lock = lock_env();
    let (_temp_dir, db_path) = create_fixture_database();
    let _env_guard = configure_duckdb_env(Some(&db_path), None);

    let mut params = base_params(
        "SELECT order_id, country_code, amount
         FROM analytics.orders
         WHERE country_code = :country
         ORDER BY order_id",
    );
    params.byte_limit = Some(1);
    params.parameters = Some(HashMap::from([("country".to_string(), json!("GB"))]));

    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    let payload = runtime
        .block_on(DUCKDB_PROVIDER.execute(&params))
        .expect("duckdb execute should succeed");

    assert_eq!(payload["success"], json!(true));
    assert_eq!(payload["data"]["provider"], json!("duckdb"));
    assert_eq!(payload["data"]["truncated"], json!(true));
    assert_eq!(
        payload["data"]["rows"]
            .as_array()
            .map_or(usize::MAX, Vec::len),
        0
    );
}

#[test]
fn duckdb_execute_rejects_parameter_types() {
    let _env_lock = lock_env();
    let (_temp_dir, db_path) = create_fixture_database();
    let _env_guard = configure_duckdb_env(Some(&db_path), None);

    let mut params = base_params("SELECT * FROM analytics.orders WHERE country_code = :country");
    params.parameters = Some(HashMap::from([("country".to_string(), json!("GB"))]));
    params.parameter_types = Some(HashMap::from([("country".to_string(), "TEXT".to_string())]));

    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    let err = runtime
        .block_on(DUCKDB_PROVIDER.execute(&params))
        .expect_err("parameter_types should be rejected by duckdb provider");
    assert!(
        err.to_string().contains("does not support parameter_types"),
        "unexpected error: {err}"
    );
}

#[test]
fn duckdb_execute_external_file_query_requires_bounded_external_access_opt_in() {
    let _env_lock = lock_env();
    let (temp_dir, db_path) = create_fixture_database();
    let (external_dir, _csv_path) = create_external_csv(&temp_dir);
    let params =
        base_params("SELECT COUNT(*) AS row_count FROM read_csv_auto('external_orders.csv')");
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");

    {
        let _env_guard = configure_duckdb_env(Some(&db_path), None);
        let err = runtime
            .block_on(DUCKDB_PROVIDER.execute(&params))
            .expect_err("query should fail without file_search_path");
        assert!(
            err.to_string().contains("DuckDB error"),
            "expected a DuckDB runtime error without file_search_path, got: {err}"
        );
    }

    let file_search_path = external_dir.to_string_lossy().to_string();
    {
        let _env_guard = configure_duckdb_env(Some(&db_path), Some(file_search_path.as_str()));
        let err = runtime
            .block_on(DUCKDB_PROVIDER.execute(&params))
            .expect_err("query should fail without explicit external-access opt-in");
        assert!(
            err.to_string().contains("external access")
                || err.to_string().contains("External access")
                || err
                    .to_string()
                    .contains("file system operations are disabled"),
            "expected external access to stay disabled, got: {err}"
        );
    }

    let _env_guard = configure_duckdb_env_with_external_access(
        Some(&db_path),
        Some(file_search_path.as_str()),
        true,
    );
    let payload = runtime
        .block_on(DUCKDB_PROVIDER.execute(&params))
        .expect("query should succeed when bounded external access is explicitly enabled");
    assert_eq!(payload["success"], json!(true));
    assert_eq!(payload["data"]["provider"], json!("duckdb"));
    assert_eq!(payload["data"]["external_access"], json!(true));
    assert_eq!(payload["data"]["rows"].as_array().map_or(0, Vec::len), 1);
}

#[test]
fn duckdb_preflight_reports_configuration_connectivity_and_relation_access() {
    let _env_lock = lock_env();
    let (_temp_dir, db_path) = create_fixture_database();
    let _env_guard = configure_duckdb_env(Some(&db_path), None);

    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    let catalog_probe_payload = runtime
        .block_on(DUCKDB_PROVIDER.execute(&base_params(
            "SELECT catalog_name FROM information_schema.schemata LIMIT 1",
        )))
        .expect("catalog probe should succeed");
    let preflight_catalog = catalog_probe_payload["data"]["rows"][0][0]
        .as_str()
        .expect("catalog name should be a string")
        .to_string();

    let mut params = base_params("");
    params.preflight_catalog = Some(preflight_catalog);
    params.preflight_schema = Some("analytics".to_string());
    params.preflight_relation = Some("analytics.orders".to_string());

    let payload = runtime
        .block_on(DUCKDB_PROVIDER.preflight(&params))
        .expect("preflight should succeed");

    assert_eq!(payload["success"], json!(true));
    assert_eq!(payload["data"]["provider"], json!("duckdb"));
    assert_eq!(payload["data"]["ready"], json!(true));

    let checks = payload["data"]["checks"]
        .as_array()
        .expect("checks should be an array");
    assert!(
        checks
            .iter()
            .any(|check| check["name"] == "configuration" && check["ok"] == true)
    );
    assert!(
        checks
            .iter()
            .any(|check| check["name"] == "connectivity" && check["ok"] == true)
    );
    assert!(
        checks
            .iter()
            .any(|check| check["name"] == "relation_access" && check["ok"] == true)
    );
}

#[test]
fn duckdb_preflight_marks_missing_catalog_as_not_ready() {
    let _env_lock = lock_env();
    let (_temp_dir, db_path) = create_fixture_database();
    let _env_guard = configure_duckdb_env(Some(&db_path), None);

    let mut params = base_params("");
    params.preflight_catalog = Some("missing_catalog".to_string());

    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    let payload = runtime
        .block_on(DUCKDB_PROVIDER.preflight(&params))
        .expect("preflight should return a structured response");

    assert_eq!(payload["success"], json!(true));
    assert_eq!(payload["data"]["provider"], json!("duckdb"));
    assert_eq!(payload["data"]["ready"], json!(false));

    let checks = payload["data"]["checks"]
        .as_array()
        .expect("checks should be an array");
    assert!(
        checks
            .iter()
            .any(|check| check["name"] == "catalog_access" && check["ok"] == false)
    );
}

#[test]
fn duckdb_preflight_missing_path_returns_structured_configuration_failure() {
    let _env_lock = lock_env();
    let _env_guard = configure_duckdb_env(None, None);

    let params = base_params("");
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    let payload = runtime
        .block_on(DUCKDB_PROVIDER.preflight(&params))
        .expect("preflight should return a structured response");

    assert_eq!(payload["success"], json!(true));
    assert_eq!(payload["data"]["provider"], json!("duckdb"));
    assert_eq!(payload["data"]["ready"], json!(false));
    assert_eq!(payload["data"]["checks"][0]["name"], json!("configuration"));
    assert_eq!(payload["data"]["checks"][0]["ok"], json!(false));
}

#[test]
fn execute_sql_tool_path_uses_duckdb_provider() {
    let _env_lock = lock_env();
    let (_temp_dir, db_path) = create_fixture_database();
    let _env_guard = configure_duckdb_env(Some(&db_path), None);
    let searcher = create_duckdb_searcher();

    let mut params = base_params(
        "SELECT country_code, COUNT(*) AS orders, SUM(amount) AS revenue
         FROM analytics.orders
         WHERE country_code = :country
         GROUP BY country_code",
    );
    params.parameters = Some(HashMap::from([("country".to_string(), json!("GB"))]));

    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    let payload = runtime
        .block_on(searcher.execute_sql(&params))
        .expect("tool execute_sql should succeed");

    assert_eq!(payload["success"], json!(true));
    assert_eq!(payload["data"]["provider"], json!("duckdb"));
    let rows = payload["data"]["rows"]
        .as_array()
        .expect("rows should be present");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0], json!(["GB", 2, "260"]));
}

#[test]
fn execute_sql_tool_path_blocks_dangerous_statements_before_provider_access() {
    let _env_lock = lock_env();
    let _env_guard = configure_duckdb_env(None, None);
    let searcher = create_duckdb_searcher();

    let params = base_params("DROP TABLE analytics.orders");
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    let err = runtime
        .block_on(searcher.execute_sql(&params))
        .expect_err("dangerous statement should be rejected");

    assert!(
        err.to_string().contains("DROP statements are not allowed"),
        "unexpected error: {err}"
    );
}

#[test]
#[allow(deprecated)]
fn run_recipe_executes_with_duckdb_provider_without_special_handling() {
    let _env_lock = lock_env();
    let (_temp_dir, db_path) = create_fixture_database();
    let _env_guard = configure_duckdb_env(Some(&db_path), None);
    let searcher = create_duckdb_searcher_for_fixture("recipes_by_analysis.json");

    let mut parameters = HashMap::new();
    parameters.insert("COUNTRY_CODE".to_string(), json!("GB"));

    let params = RunRecipeParams {
        recipe_id: "marketplace/weekly_report".to_string(),
        query_names: vec![],
        query_indexes: vec![],
        stop_on_failure: true,
        include_sql: false,
        row_limit: Some(10),
        byte_limit: None,
        max_poll_seconds: None,
        poll_interval_ms: None,
        wait_timeout_s: None,
        parameters: Some(parameters),
        placeholder_types: None,
        sql_parameter_types: None,
        parameter_types: None,
        fetch_all_chunks: None,
        max_chunks: None,
    };

    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    let payload = runtime
        .block_on(searcher.run_recipe(&params))
        .expect("run_recipe should execute successfully on duckdb");

    assert_eq!(payload["success"], json!(true));
    assert_eq!(payload["data"]["executed_queries"], json!(2));
    assert_eq!(payload["data"]["failed_queries"], json!(0));
    let steps = payload["data"]["steps"]
        .as_array()
        .expect("steps should be present");
    assert_eq!(steps.len(), 2);
    assert!(
        steps
            .iter()
            .all(|step| step.get("status").and_then(JsonValue::as_str) == Some("ok")),
        "all recipe steps should succeed: {steps:?}"
    );
}
