use serde_json::Value as JsonValue;

use crate::config::DbtNovaConfig;
use crate::error::{DbtNovaError, Result};
use crate::manifest::search::ManifestSearch;
use crate::params::ExecuteSqlParams;
use crate::warehouse::resolve_sql_provider;
use sqlparser::ast::Statement;
use sqlparser::dialect::{GenericDialect, SnowflakeDialect};
use sqlparser::parser::Parser;
use tracing::{instrument, warn};

const PROVIDER_DEFAULT_ROW_LIMIT: u64 = 1_000;
const PROVIDER_DEFAULT_BYTE_LIMIT: u64 = 25_000_000;
const PROVIDER_DEFAULT_MAX_CHUNKS: usize = 50;

/// Validate SQL statements to only allow safe, read-only queries.
///
/// # Errors
/// Returns an error if the statement is empty, invalid, or not permitted.
pub(crate) fn validate_sql_statement(statement: &str) -> Result<()> {
    validate_sql_statement_for_provider(statement, "generic")
}

/// Validate SQL statements for a specific provider dialect.
///
/// # Errors
/// Returns an error if the statement is empty, invalid, or not permitted.
pub(crate) fn validate_sql_statement_for_provider(statement: &str, provider: &str) -> Result<()> {
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

    if provider.eq_ignore_ascii_case("snowflake") && ast.len() > 1 {
        reject_statement!(
            "Snowflake provider does not support multi-statement execute_sql requests"
        );
    }

    for stmt in &ast {
        match stmt {
            Statement::Query(_) => {}
            Statement::Explain { statement, .. } => {
                if !matches!(statement.as_ref(), Statement::Query(_)) {
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
        provider.validate_runtime(self.config())?;
        if bounded.preflight_only {
            return provider.preflight(&bounded).await;
        }
        let statement = bounded.statement.trim();
        if provider.name().eq_ignore_ascii_case("snowflake") {
            validate_sql_statement_for_provider(statement, provider.name())?;
        } else {
            validate_sql_statement(statement)?;
        }
        provider.execute(&bounded).await
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
    fn validate_sql_statement_supports_snowflake_dialect() {
        validate_sql_statement_for_provider(
            "select * from analytics.orders qualify row_number() over (partition by customer_id order by order_ts desc) = 1",
            "snowflake",
        )
        .expect("snowflake qualify should parse");
    }

    #[test]
    fn validate_sql_statement_rejects_snowflake_multi_statement() {
        let err = validate_sql_statement_for_provider("select 1; select 2", "snowflake")
            .expect_err("snowflake multi-statement should be rejected");
        assert!(err.to_string().contains("multi-statement"));
    }
}
