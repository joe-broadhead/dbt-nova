use serde_json::Value as JsonValue;

use crate::config::DbtNovaConfig;
use crate::config::warehouse::DEFAULT_SQL_PROVIDER;
use crate::error::{DbtNovaError, Result};
use crate::manifest::search::ManifestSearch;
use crate::params::ExecuteSqlParams;
use crate::warehouse::{available_sql_provider_names, resolve_sql_provider};
use sqlparser::ast::{Expr, ObjectName, Query, SetExpr, Statement, TableFactor, Visit, Visitor};
use sqlparser::dialect::{GenericDialect, SnowflakeDialect};
use sqlparser::parser::Parser;
use std::ops::ControlFlow;
use tracing::{instrument, warn};

const PROVIDER_DEFAULT_ROW_LIMIT: u64 = 1_000;
const PROVIDER_DEFAULT_BYTE_LIMIT: u64 = 25_000_000;
const PROVIDER_DEFAULT_MAX_CHUNKS: usize = 50;
const DUCKDB_EXTERNAL_ACCESS_FUNCTIONS: &[&str] = &[
    "csv_scan",
    "delta_scan",
    "glob",
    "iceberg_scan",
    "parquet_scan",
    "read_blob",
    "read_csv",
    "read_csv_auto",
    "read_file",
    "read_json",
    "read_json_auto",
    "read_json_objects",
    "read_json_objects_auto",
    "read_ndjson",
    "read_ndjson_auto",
    "read_ndjson_objects",
    "read_ndjson_objects_auto",
    "read_parquet",
    "read_text",
];

/// Validate SQL statements for a specific provider dialect.
///
/// # Errors
/// Returns an error if the statement is empty, invalid, or not permitted.
pub fn validate_sql_statement_for_provider(statement: &str, provider: &str) -> Result<()> {
    if statement.trim().is_empty() {
        return Err(DbtNovaError::InvalidParams(
            "statement cannot be empty".to_string(),
        ));
    }

    macro_rules! reject_statement {
        ($message:expr) => {
            return Err(DbtNovaError::InvalidParams($message.to_string()));
        };
    }

    let ast = if provider.eq_ignore_ascii_case("snowflake") {
        let dialect = SnowflakeDialect {};
        Parser::parse_sql(&dialect, statement)
    } else {
        let dialect = GenericDialect {};
        Parser::parse_sql(&dialect, statement)
    }
    .map_err(|e| DbtNovaError::InvalidParams(format!("Invalid SQL syntax: {e}")))?;

    if ast.len() != 1 {
        reject_statement!("execute_sql does not support multi-statement requests");
    }

    let deny_duckdb_external_access =
        provider.eq_ignore_ascii_case("duckdb") || provider.eq_ignore_ascii_case("generic");

    for stmt in &ast {
        match stmt {
            Statement::Query(query) => {
                validate_query_body_is_read_only(query)?;
                if deny_duckdb_external_access {
                    validate_no_duckdb_external_access(stmt)?;
                }
            }
            Statement::Explain { statement, .. } => {
                if let Statement::Query(query) = statement.as_ref() {
                    validate_query_body_is_read_only(query)?;
                    if deny_duckdb_external_access {
                        validate_no_duckdb_external_access(statement.as_ref())?;
                    }
                } else {
                    reject_statement!("Only EXPLAIN SELECT statements are allowed");
                }
            }
            Statement::Drop { .. } => {
                reject_statement!("DROP statements are not allowed");
            }
            Statement::Delete { .. } => {
                reject_statement!("DELETE statements are not allowed");
            }
            Statement::Truncate { .. } => {
                reject_statement!("TRUNCATE statements are not allowed");
            }
            Statement::Insert { .. } => {
                reject_statement!("INSERT statements are not allowed");
            }
            Statement::Update { .. } => {
                reject_statement!("UPDATE statements are not allowed");
            }
            Statement::CreateTable { .. } | Statement::CreateView { .. } => {
                reject_statement!("CREATE statements are not allowed");
            }
            Statement::AlterTable { .. } => {
                reject_statement!("ALTER statements are not allowed");
            }
            _ => {
                reject_statement!("Statement type not allowed: only SELECT queries permitted");
            }
        }
    }

    Ok(())
}

fn validate_query_body_is_read_only(query: &Query) -> Result<()> {
    if let Some(with) = &query.with {
        for cte in &with.cte_tables {
            validate_query_body_is_read_only(&cte.query)?;
        }
    }

    validate_set_expr_is_read_only(&query.body)
}

fn validate_set_expr_is_read_only(set_expr: &SetExpr) -> Result<()> {
    match set_expr {
        SetExpr::Select(select) => {
            if select.into.is_some() {
                return Err(DbtNovaError::InvalidParams(
                    "SELECT INTO statements are not allowed".to_string(),
                ));
            }
            Ok(())
        }
        SetExpr::Query(query) => validate_query_body_is_read_only(query),
        SetExpr::SetOperation { left, right, .. } => {
            validate_set_expr_is_read_only(left)?;
            validate_set_expr_is_read_only(right)
        }
        SetExpr::Values(_) | SetExpr::Table(_) => Ok(()),
        SetExpr::Insert(_) | SetExpr::Update(_) | SetExpr::Delete(_) | SetExpr::Merge(_) => Err(
            DbtNovaError::InvalidParams("Only read-only SELECT queries are allowed".to_string()),
        ),
    }
}

struct DuckDbExternalAccessVisitor;

impl Visitor for DuckDbExternalAccessVisitor {
    type Break = String;

    fn pre_visit_expr(&mut self, expr: &Expr) -> ControlFlow<Self::Break> {
        if let Expr::Function(function) = expr
            && let Some(function_name) = blocked_duckdb_external_function(&function.name)
        {
            return ControlFlow::Break(duckdb_external_access_message(function_name));
        }
        ControlFlow::Continue(())
    }

    fn pre_visit_table_factor(&mut self, table_factor: &TableFactor) -> ControlFlow<Self::Break> {
        match table_factor {
            TableFactor::Table {
                name,
                args: Some(_),
                ..
            }
            | TableFactor::Function { name, .. } => {
                if let Some(function_name) = blocked_duckdb_external_function(name) {
                    return ControlFlow::Break(duckdb_external_access_message(function_name));
                }
            }
            TableFactor::TableFunction { expr, .. } => {
                if let Expr::Function(function) = expr
                    && let Some(function_name) = blocked_duckdb_external_function(&function.name)
                {
                    return ControlFlow::Break(duckdb_external_access_message(function_name));
                }
            }
            _ => {}
        }
        ControlFlow::Continue(())
    }
}

fn validate_no_duckdb_external_access(statement: &Statement) -> Result<()> {
    let mut visitor = DuckDbExternalAccessVisitor;
    match statement.visit(&mut visitor) {
        ControlFlow::Continue(()) => Ok(()),
        ControlFlow::Break(message) => Err(DbtNovaError::InvalidParams(message)),
    }
}

fn blocked_duckdb_external_function(name: &ObjectName) -> Option<&'static str> {
    let leaf = name
        .0
        .iter()
        .rev()
        .find_map(|part| part.as_ident().map(|ident| ident.value.as_str()))?;
    DUCKDB_EXTERNAL_ACCESS_FUNCTIONS
        .iter()
        .copied()
        .find(|blocked| leaf.eq_ignore_ascii_case(blocked))
}

fn duckdb_external_access_message(function_name: &str) -> String {
    format!("DuckDB external file access function '{function_name}' is not allowed in execute_sql")
}

impl ManifestSearch {
    fn apply_sql_limits(&self, params: &ExecuteSqlParams) -> ExecuteSqlParams {
        apply_sql_limits_with_config(params, self.config())
    }

    /// Execute SQL against a configured Databricks SQL warehouse.
    ///
    /// # Errors
    /// Returns an error if validation fails or execution fails.
    #[instrument(skip(self, params), fields(tool = "execute_sql", statement_len = params.statement.len(), row_limit = ?params.row_limit, byte_limit = ?params.byte_limit))]
    pub async fn execute_sql(&self, params: &ExecuteSqlParams) -> Result<JsonValue> {
        let bounded = self.apply_sql_limits(params);
        let provider = resolve_sql_provider(self.config())?;
        provider
            .validate_runtime(self.config())
            .map_err(|err| sql_provider_configuration_error(self.config(), provider.name(), err))?;
        if bounded.preflight_only {
            return provider.preflight(&bounded).await.map_err(|err| {
                sql_provider_configuration_error(self.config(), provider.name(), err)
            });
        }
        let statement = bounded.statement.trim();
        validate_sql_statement_for_provider(statement, provider.name())?;
        provider
            .execute(&bounded)
            .await
            .map_err(|err| sql_provider_configuration_error(self.config(), provider.name(), err))
    }
}

fn sql_provider_configuration_error(
    config: &DbtNovaConfig,
    provider_name: &str,
    err: DbtNovaError,
) -> DbtNovaError {
    let message = err.to_string();
    if !is_sql_provider_configuration_error(provider_name, &message) {
        return err;
    }

    let configured = config.sql_provider.trim();
    let selected_by =
        if configured.is_empty() || configured.eq_ignore_ascii_case(DEFAULT_SQL_PROVIDER) {
            format!("default `{DEFAULT_SQL_PROVIDER}`")
        } else {
            format!("configured `sql_provider={configured}`")
        };
    let detail = strip_sql_provider_error_prefixes(&message);
    let available = available_sql_provider_names().join(", ");

    DbtNovaError::ServerError(format!(
        "execute_sql selected SQL provider `{provider_name}` ({selected_by}), but provider configuration is incomplete: {detail}. Set DBT_NOVA_SQL_PROVIDER or config sql_provider to choose a provider. Available providers: {available}"
    ))
}

fn is_sql_provider_configuration_error(provider_name: &str, message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    match provider_name.to_ascii_lowercase().as_str() {
        "databricks" => [
            "databricks_host",
            "databricks_access_token",
            "databricks_http_path",
            "databricks_sql_warehouse_id",
            "environment variable not set",
        ]
        .iter()
        .any(|needle| lower.contains(needle)),
        "bigquery" => [
            "bigquery project id",
            "bigquery access token",
            "google_application_credentials",
            "gcloud",
            "application default credentials",
        ]
        .iter()
        .any(|needle| lower.contains(needle)),
        "duckdb" => [
            "dbt_nova_duckdb_path",
            "duckdb path",
            "duckdb database path",
        ]
        .iter()
        .any(|needle| lower.contains(needle)),
        "snowflake" => [
            "dbt_nova_snowflake",
            "snowflake account",
            "snowflake warehouse",
            "snowflake user",
            "snowflake role",
            "snowflake database",
            "snowflake schema",
            "snowflake private key",
            "snowflake oauth",
            "externalbrowser",
        ]
        .iter()
        .any(|needle| lower.contains(needle)),
        _ => false,
    }
}

fn strip_sql_provider_error_prefixes(message: &str) -> String {
    let mut detail = message.trim();
    loop {
        let stripped = detail
            .strip_prefix("Server error: ")
            .or_else(|| detail.strip_prefix("Invalid parameter: "))
            .or_else(|| detail.strip_prefix("Databricks error: "))
            .or_else(|| detail.strip_prefix("BigQuery error: "))
            .or_else(|| detail.strip_prefix("DuckDB error: "))
            .or_else(|| detail.strip_prefix("Snowflake error: "));
        match stripped {
            Some(next) => detail = next.trim(),
            None => return detail.to_string(),
        }
    }
}

fn apply_sql_limits_with_config(
    params: &ExecuteSqlParams,
    config: &DbtNovaConfig,
) -> ExecuteSqlParams {
    let mut bounded = params.clone();

    if bounded.row_limit.is_none() && config.sql_max_row_limit > 0 {
        bounded.row_limit = Some(PROVIDER_DEFAULT_ROW_LIMIT.min(config.sql_max_row_limit));
    }

    if bounded.byte_limit.is_none() && config.sql_max_byte_limit > 0 {
        bounded.byte_limit = Some(PROVIDER_DEFAULT_BYTE_LIMIT.min(config.sql_max_byte_limit));
    }

    if bounded.max_chunks.is_none() && config.sql_max_chunks > 0 {
        bounded.max_chunks = Some(PROVIDER_DEFAULT_MAX_CHUNKS.min(config.sql_max_chunks));
    }

    if let Some(row_limit) = bounded.row_limit
        && config.sql_max_row_limit > 0
        && row_limit > config.sql_max_row_limit
    {
        warn!(
            requested = row_limit,
            max = config.sql_max_row_limit,
            "clamping execute_sql row_limit"
        );
        bounded.row_limit = Some(config.sql_max_row_limit);
    }

    if let Some(byte_limit) = bounded.byte_limit
        && config.sql_max_byte_limit > 0
        && byte_limit > config.sql_max_byte_limit
    {
        warn!(
            requested = byte_limit,
            max = config.sql_max_byte_limit,
            "clamping execute_sql byte_limit"
        );
        bounded.byte_limit = Some(config.sql_max_byte_limit);
    }

    if let Some(max_chunks) = bounded.max_chunks
        && config.sql_max_chunks > 0
        && max_chunks > config.sql_max_chunks
    {
        warn!(
            requested = max_chunks,
            max = config.sql_max_chunks,
            "clamping execute_sql max_chunks"
        );
        bounded.max_chunks = Some(config.sql_max_chunks);
    }

    if let Some(max_poll_seconds) = bounded.max_poll_seconds
        && config.sql_max_poll_seconds > 0
        && max_poll_seconds > config.sql_max_poll_seconds
    {
        warn!(
            requested = max_poll_seconds,
            max = config.sql_max_poll_seconds,
            "clamping execute_sql max_poll_seconds"
        );
        bounded.max_poll_seconds = Some(config.sql_max_poll_seconds);
    }

    if let Some(poll_interval_ms) = bounded.poll_interval_ms
        && config.sql_min_poll_interval_ms > 0
        && poll_interval_ms < config.sql_min_poll_interval_ms
    {
        warn!(
            requested = poll_interval_ms,
            min = config.sql_min_poll_interval_ms,
            "raising execute_sql poll_interval_ms to configured minimum"
        );
        bounded.poll_interval_ms = Some(config.sql_min_poll_interval_ms);
    }

    bounded
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_execute_sql_params() -> ExecuteSqlParams {
        ExecuteSqlParams {
            statement: "select 1".to_string(),
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
    fn apply_sql_limits_defaults_respect_config_caps() {
        let config = DbtNovaConfig {
            sql_max_row_limit: 500,
            sql_max_byte_limit: 20_000_000,
            sql_max_chunks: 10,
            sql_max_poll_seconds: 45,
            ..DbtNovaConfig::default()
        };

        let params = sample_execute_sql_params();
        let bounded = apply_sql_limits_with_config(&params, &config);

        assert_eq!(bounded.row_limit, Some(500));
        assert_eq!(bounded.byte_limit, Some(20_000_000));
        assert_eq!(bounded.max_chunks, Some(10));
        assert_eq!(bounded.max_poll_seconds, None);
    }

    #[test]
    fn apply_sql_limits_clamps_explicit_values() {
        let config = DbtNovaConfig {
            sql_max_row_limit: 500,
            sql_max_byte_limit: 20_000_000,
            sql_max_chunks: 10,
            sql_max_poll_seconds: 45,
            sql_min_poll_interval_ms: 250,
            ..DbtNovaConfig::default()
        };

        let mut params = sample_execute_sql_params();
        params.row_limit = Some(2_000);
        params.byte_limit = Some(50_000_000);
        params.max_chunks = Some(99);
        params.max_poll_seconds = Some(120);
        params.poll_interval_ms = Some(100);

        let bounded = apply_sql_limits_with_config(&params, &config);

        assert_eq!(bounded.row_limit, Some(500));
        assert_eq!(bounded.byte_limit, Some(20_000_000));
        assert_eq!(bounded.max_chunks, Some(10));
        assert_eq!(bounded.max_poll_seconds, Some(45));
        assert_eq!(bounded.poll_interval_ms, Some(250));
    }

    #[test]
    fn provider_configuration_error_explains_default_selection() {
        let config = DbtNovaConfig::default();
        let err = sql_provider_configuration_error(
            &config,
            "databricks",
            DbtNovaError::ServerError(
                "Databricks error: DATABRICKS_HOST environment variable not set".to_string(),
            ),
        );
        let message = err.to_string();

        assert!(message.contains("execute_sql selected SQL provider `databricks`"));
        assert!(message.contains("default `databricks`"));
        assert!(message.contains("DATABRICKS_HOST environment variable not set"));
        assert!(message.contains("DBT_NOVA_SQL_PROVIDER"));
        assert!(message.contains("Available providers: databricks, bigquery, duckdb, snowflake"));
        assert!(!message.contains("Databricks error: Databricks error"));
    }

    #[test]
    fn provider_configuration_error_explains_explicit_selection() {
        let config = DbtNovaConfig {
            sql_provider: "duckdb".to_string(),
            ..DbtNovaConfig::default()
        };
        let err = sql_provider_configuration_error(
            &config,
            "duckdb",
            DbtNovaError::InvalidParams(
                "DBT_NOVA_DUCKDB_PATH must be set for DuckDB execute_sql".to_string(),
            ),
        );
        let message = err.to_string();

        assert!(message.contains("configured `sql_provider=duckdb`"));
        assert!(message.contains("DBT_NOVA_DUCKDB_PATH must be set"));
    }

    #[test]
    fn provider_configuration_error_leaves_query_failures_unchanged() {
        let config = DbtNovaConfig::default();
        let err = sql_provider_configuration_error(
            &config,
            "databricks",
            DbtNovaError::ServerError("Databricks error: syntax error near from".to_string()),
        );

        assert_eq!(
            err.to_string(),
            "Server error: Databricks error: syntax error near from"
        );
    }

    #[test]
    fn validate_sql_statement_supports_snowflake_dialect() {
        validate_sql_statement_for_provider(
            "select * from analytics.orders qualify row_number() over (partition by customer_id order by order_ts desc) = 1",
            "snowflake",
        )
        .expect("snowflake qualify should parse");
    }

    #[test]
    fn validate_sql_statement_allows_read_only_query_matrix() {
        let cases = [
            "select * from orders",
            "explain select * from orders",
            "with recent as (select * from orders where order_date >= current_date - interval '7 days') select * from recent",
            "select * from orders o join customers c on o.customer_id = c.customer_id",
            "select * from (select customer_id, count(*) as order_count from orders group by customer_id) counts where order_count > 1",
            "values (1), (2)",
        ];

        for sql in cases {
            validate_sql_statement_for_provider(sql, "generic")
                .unwrap_or_else(|err| panic!("expected read-only SQL to pass: {sql}: {err}"));
        }
    }

    #[test]
    fn validate_sql_statement_rejects_dangerous_matrix() {
        let cases = [
            "insert into orders values (1)",
            "update orders set amount = 1",
            "delete from orders",
            "drop table orders",
            "truncate table orders",
            "alter table orders add column x int",
            "create table backup as select * from orders",
            "copy orders to '/tmp/orders.csv'",
            "pragma version",
            "set enable_external_access = true",
            "/* hidden */ delete from orders",
        ];

        for sql in cases {
            validate_sql_statement_for_provider(sql, "generic")
                .expect_err("expected dangerous SQL to fail");
        }
    }

    #[test]
    fn validate_sql_statement_rejects_snowflake_multi_statement() {
        let err = validate_sql_statement_for_provider("select 1; select 2", "snowflake")
            .expect_err("snowflake multi-statement should be rejected");
        assert!(err.to_string().contains("multi-statement"));
    }

    #[test]
    fn validate_sql_statement_rejects_generic_multi_statement() {
        let err = validate_sql_statement_for_provider("select 1; select 2", "databricks")
            .expect_err("multi-statement requests should be rejected");
        assert!(err.to_string().contains("multi-statement"));
    }

    #[test]
    fn validate_sql_statement_rejects_duckdb_external_scan_functions() {
        let cases = [
            "select * from read_csv_auto('/tmp/orders.csv')",
            "select * from parquet_scan('/tmp/orders.parquet')",
            "select read_text('/tmp/secret.txt')",
            "select * from read_csv('http://169.254.169.254/latest/meta-data/')",
            "select * from read_json_objects_auto('/tmp/orders.json')",
            "select * from read_ndjson_auto('/tmp/orders.ndjson')",
            "explain select * from read_json_auto('/tmp/orders.json')",
        ];

        for sql in cases {
            let err = validate_sql_statement_for_provider(sql, "duckdb")
                .expect_err("DuckDB file scan functions should be rejected");
            assert!(
                err.to_string().contains("external file access"),
                "unexpected error for {sql}: {err}"
            );
        }
    }

    #[test]
    fn validate_sql_statement_allows_non_duckdb_read_csv_function_names() {
        validate_sql_statement_for_provider("select read_csv(order_id) from orders", "databricks")
            .expect("non-DuckDB providers may have unrelated UDF names");
    }

    #[test]
    fn validate_sql_statement_rejects_select_into() {
        let err = validate_sql_statement_for_provider(
            "select * into backup_orders from orders",
            "generic",
        )
        .expect_err("SELECT INTO should be rejected");
        assert!(err.to_string().contains("SELECT INTO"));
    }
}
