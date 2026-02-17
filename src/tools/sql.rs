use serde_json::Value as JsonValue;

use crate::error::{DbtNovaError, Result};
use crate::manifest::search::ManifestSearch;
use crate::params::ExecuteSqlParams;
use crate::warehouse::resolve_sql_provider;
use sqlparser::ast::Statement;
use sqlparser::dialect::GenericDialect;
use sqlparser::parser::Parser;
use tracing::{instrument, warn};

/// Validate SQL statements to only allow safe, read-only queries.
///
/// # Errors
/// Returns an error if the statement is empty, invalid, or not permitted.
pub(crate) fn validate_sql_statement(statement: &str) -> Result<()> {
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

    let dialect = GenericDialect {};
    let ast = Parser::parse_sql(&dialect, statement)
        .map_err(|e| DbtNovaError::InvalidParams(format!("Invalid SQL syntax: {e}")))?;

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
        let mut bounded = params.clone();
        let config = self.config();

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

    /// Execute SQL against a configured Databricks SQL warehouse.
    ///
    /// # Errors
    /// Returns an error if validation fails or execution fails.
    #[instrument(skip(self, params), fields(tool = "execute_sql", statement_len = params.statement.len(), row_limit = ?params.row_limit, byte_limit = ?params.byte_limit))]
    pub async fn execute_sql(&self, params: &ExecuteSqlParams) -> Result<JsonValue> {
        let bounded = self.apply_sql_limits(params);
        let provider = resolve_sql_provider(self.config())?;
        if bounded.preflight_only {
            return provider.preflight(&bounded).await;
        }
        let statement = bounded.statement.trim();
        validate_sql_statement(statement)?;
        provider.execute(&bounded).await
    }
}
