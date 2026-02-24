use std::future::Future;
use std::pin::Pin;

use serde_json::Value as JsonValue;

use crate::config::DbtNovaConfig;
use crate::error::{DbtNovaError, Result};
use crate::params::ExecuteSqlParams;

pub mod bigquery;
pub mod databricks;
pub mod duckdb;

/// Return true when a preflight probe can be considered "present".
///
/// Providers may surface row presence either as concrete rows or as an aggregate
/// `total_row_count` without materialized rows. This helper keeps non-empty
/// probe semantics consistent across providers.
pub(crate) fn preflight_probe_has_rows(rows_len: usize, total_row_count: Option<u64>) -> bool {
    rows_len > 0 || total_row_count.is_some_and(|count| count > 0)
}

/// Standard message used when an object-level preflight probe is empty.
pub(crate) fn empty_preflight_probe_message(check: &str) -> String {
    format!("Preflight {check} probe returned no rows; target may not exist or may be inaccessible")
}

pub trait SqlProvider: Send + Sync {
    fn name(&self) -> &'static str;
    fn execute<'a>(
        &'a self,
        params: &'a ExecuteSqlParams,
    ) -> Pin<Box<dyn Future<Output = Result<JsonValue>> + Send + 'a>>;
    fn preflight<'a>(
        &'a self,
        _params: &'a ExecuteSqlParams,
    ) -> Pin<Box<dyn Future<Output = Result<JsonValue>> + Send + 'a>> {
        Box::pin(async move {
            Err(DbtNovaError::InvalidParams(format!(
                "SQL preflight not supported for provider '{}'",
                self.name()
            )))
        })
    }
}

struct SqlProviderRegistry {
    providers: Vec<&'static dyn SqlProvider>,
}

impl SqlProviderRegistry {
    fn default() -> Self {
        Self {
            providers: vec![
                &databricks::DATABRICKS_PROVIDER,
                &bigquery::BIGQUERY_PROVIDER,
                &duckdb::DUCKDB_PROVIDER,
            ],
        }
    }

    fn by_name(&self, name: &str) -> Option<&'static dyn SqlProvider> {
        self.providers
            .iter()
            .copied()
            .find(|provider| provider.name().eq_ignore_ascii_case(name))
    }

    fn names(&self) -> Vec<&'static str> {
        self.providers.iter().map(|p| p.name()).collect()
    }
}

/// Resolve the configured SQL provider.
///
/// # Errors
/// Returns an error if the configured provider name is unknown.
pub fn resolve_sql_provider(config: &DbtNovaConfig) -> Result<&'static dyn SqlProvider> {
    let name = config.sql_provider.trim();
    let name = if name.is_empty() { "databricks" } else { name };

    let registry = SqlProviderRegistry::default();
    if let Some(provider) = registry.by_name(name) {
        return Ok(provider);
    }

    Err(DbtNovaError::InvalidParams(format!(
        "Unknown SQL provider: {} (available: {})",
        name,
        registry.names().join(", ")
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DbtNovaConfig;

    #[test]
    fn default_sql_provider_is_databricks() {
        let cfg = DbtNovaConfig::default();
        let provider = match resolve_sql_provider(&cfg) {
            Ok(provider) => provider,
            Err(err) => panic!("provider resolved: {err}"),
        };
        assert_eq!(provider.name(), "databricks");
    }

    #[test]
    fn unknown_sql_provider_returns_error() {
        let cfg = DbtNovaConfig {
            sql_provider: "unknown".to_string(),
            ..Default::default()
        };
        let Err(err) = resolve_sql_provider(&cfg) else {
            panic!("expected error");
        };
        assert!(err.to_string().contains("Unknown SQL provider"));
    }

    #[test]
    fn bigquery_sql_provider_resolves() {
        let cfg = DbtNovaConfig {
            sql_provider: "bigquery".to_string(),
            ..Default::default()
        };
        let provider = match resolve_sql_provider(&cfg) {
            Ok(provider) => provider,
            Err(err) => panic!("provider resolved: {err}"),
        };
        assert_eq!(provider.name(), "bigquery");
    }

    #[test]
    fn duckdb_sql_provider_resolves() {
        let cfg = DbtNovaConfig {
            sql_provider: "duckdb".to_string(),
            ..Default::default()
        };
        let provider = match resolve_sql_provider(&cfg) {
            Ok(provider) => provider,
            Err(err) => panic!("provider resolved: {err}"),
        };
        assert_eq!(provider.name(), "duckdb");
    }

    #[test]
    fn preflight_probe_has_rows_checks_rows_and_totals() {
        assert!(preflight_probe_has_rows(1, None));
        assert!(preflight_probe_has_rows(0, Some(1)));
        assert!(!preflight_probe_has_rows(0, None));
        assert!(!preflight_probe_has_rows(0, Some(0)));
    }
}
