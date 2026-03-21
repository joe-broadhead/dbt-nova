use std::time::Instant;

use crate::manifest::search::ManifestSearch;
use serde_json::Value as JsonValue;

use crate::cli::args::{HealthCheckArgs, ManifestLoadArgs};
use crate::cli::manifest::{build_manifest_load_config, execute_manifest_load};
use crate::cli::output::{CliEnvelope, error_envelope};
use crate::config::DbtNovaConfig;
use crate::error::{DbtNovaError, Result};

use super::{DispatchError, DispatchResult};

/// Runs the `health check` CLI command.
///
/// # Errors
/// Returns an error when configuration validation, manifest loading, or output serialization fails.
pub async fn run_check_command(args: &HealthCheckArgs) -> DispatchResult {
    let started = Instant::now();
    let config = build_health_check_config(args)
        .map_err(|error| render_or_propagate_error(args, error, started.elapsed().as_millis()))?;
    let load_result = execute_manifest_load(config)
        .await
        .map_err(|error| render_or_propagate_error(args, error, started.elapsed().as_millis()))?;
    let payload = build_cli_health_payload(&load_result.search).await;

    if args.json {
        let envelope =
            CliEnvelope::success("health check", &payload, started.elapsed().as_millis());
        let out = serde_json::to_string_pretty(&envelope).map_err(|error| DispatchError {
            error: DbtNovaError::ServerError(error.to_string()),
            rendered: false,
        })?;
        println!("{out}");
    } else {
        print_human_summary(&payload);
    }

    Ok(())
}

pub(crate) async fn build_cli_health_payload(searcher: &ManifestSearch) -> JsonValue {
    let mut payload = serde_json::json!({
        "status": "ready",
        "entity_count": searcher.entity_count(),
    });
    let details = searcher.health_snapshot().await;
    if let (Some(base), Some(extra)) = (payload.as_object_mut(), details.as_object()) {
        for (key, value) in extra {
            base.insert(key.clone(), value.clone());
        }
    }
    if let Some(base) = payload.as_object_mut() {
        let config = searcher.config();
        base.insert(
            "search_concurrency".to_string(),
            static_concurrency_snapshot(
                config.search.search_max_concurrent,
                config.search.search_max_queue,
            ),
        );
        base.insert(
            "sql_concurrency".to_string(),
            static_concurrency_snapshot(config.sql_max_concurrent, config.sql_max_queue),
        );
        // CLI mode has no long-lived MCP server metrics store.
        base.insert("tool_metrics".to_string(), serde_json::json!({}));
    }
    payload
}

fn static_concurrency_snapshot(max_concurrent: usize, max_queue: usize) -> JsonValue {
    if max_concurrent == 0 {
        return serde_json::json!({"enabled": false});
    }
    serde_json::json!({
        "enabled": true,
        "max_concurrent": max_concurrent,
        "available_slots": max_concurrent,
        "in_flight": 0,
        "saturated": false,
        "max_queue": max_queue,
        "available_queue": max_queue,
        "queued": 0,
        "queue_saturated": false,
    })
}

fn render_or_propagate_error(
    args: &HealthCheckArgs,
    error: DbtNovaError,
    elapsed_ms: u128,
) -> DispatchError {
    if args.json {
        let envelope = error_envelope("health check", &error, elapsed_ms);
        if let Ok(json) = serde_json::to_string_pretty(&envelope) {
            println!("{json}");
            return DispatchError {
                error,
                rendered: true,
            };
        }
    }
    DispatchError {
        error,
        rendered: false,
    }
}

/// Builds health-check configuration from environment defaults plus CLI overrides.
///
/// # Errors
/// Returns an error if manifest override flags are invalid or resulting config validation fails.
pub fn build_health_check_config(args: &HealthCheckArgs) -> Result<DbtNovaConfig> {
    let manifest_load_args = ManifestLoadArgs {
        manifest_path: args.manifest_path.clone(),
        manifest_uri: args.manifest_uri.clone(),
        ..ManifestLoadArgs::default()
    };
    let mut config = build_manifest_load_config(&manifest_load_args)?;
    // Health checks should not prune or clean storage as a side effect.
    config.cleanup_storage_on_start = false;
    config.storage_max_instances = 0;
    Ok(config)
}

fn print_human_summary(payload: &JsonValue) {
    println!("health check");
    println!("  status: {}", string_field(payload, "status", "unknown"));
    if let Some(entity_count) = payload.get("entity_count").and_then(JsonValue::as_u64) {
        println!("  entity_count: {entity_count}");
    }

    if let Some(manifest) = payload.get("manifest") {
        println!(
            "  manifest_source: {}",
            string_field(manifest, "source_uri", "unknown")
        );
        println!(
            "  manifest_hash: {}",
            string_field(manifest, "hash", "unknown")
        );
        println!(
            "  manifest_version: {}",
            string_field(manifest, "version", "unknown")
        );
        if let Some(loaded_age_ms) = manifest.get("loaded_age_ms").and_then(JsonValue::as_u64) {
            println!("  loaded_age_ms: {loaded_age_ms}");
        }
    }

    if let Some(circuit_breakers) = payload.get("circuit_breakers") {
        println!(
            "  vector_breaker: {}",
            string_field(
                circuit_breakers.get("vector").unwrap_or(&JsonValue::Null),
                "state",
                "unknown"
            )
        );
        println!(
            "  sparse_breaker: {}",
            string_field(
                circuit_breakers.get("sparse").unwrap_or(&JsonValue::Null),
                "state",
                "unknown"
            )
        );
        println!(
            "  reranker_breaker: {}",
            string_field(
                circuit_breakers.get("reranker").unwrap_or(&JsonValue::Null),
                "state",
                "unknown"
            )
        );
    }

    if let Some(manifest_cache) = payload.get("manifest_cache") {
        let hits = manifest_cache
            .get("hits")
            .and_then(JsonValue::as_u64)
            .unwrap_or(0);
        let misses = manifest_cache
            .get("misses")
            .and_then(JsonValue::as_u64)
            .unwrap_or(0);
        println!("  manifest_cache_hits: {hits}");
        println!("  manifest_cache_misses: {misses}");
    }

    if let Some(search) = payload.get("search") {
        print_search_warning(search, "vector");
        print_search_warning(search, "sparse");
        print_search_warning(search, "reranker");
    }
}

fn string_field<'a>(value: &'a JsonValue, key: &str, fallback: &'a str) -> &'a str {
    value
        .get(key)
        .and_then(JsonValue::as_str)
        .unwrap_or(fallback)
}

fn print_search_warning(payload: &JsonValue, component: &str) {
    let Some(warning) = payload
        .get(component)
        .and_then(|value| value.get("warning"))
        .and_then(JsonValue::as_str)
    else {
        return;
    };
    println!("  {component}_warning: {warning}");
}

#[cfg(test)]
mod tests {
    use super::{build_cli_health_payload, build_health_check_config};
    use crate::cli::args::HealthCheckArgs;
    use crate::cli::manifest::execute_manifest_load;
    use crate::tests::common::fixture_manifest_path_string;

    #[test]
    fn build_health_check_config_uses_manifest_path_override() {
        let args = HealthCheckArgs {
            manifest_path: Some(fixture_manifest_path_string()),
            manifest_uri: None,
            json: false,
        };
        let config = build_health_check_config(&args).expect("config");
        assert!(config.manifest_uri.is_empty());
        assert!(
            config
                .manifest_path
                .ends_with("tests/fixtures/nova_manifest.json")
        );
    }

    #[test]
    fn build_health_check_config_rejects_empty_manifest_uri() {
        let args = HealthCheckArgs {
            manifest_path: None,
            manifest_uri: Some("   ".to_string()),
            json: false,
        };
        let err = build_health_check_config(&args).expect_err("empty uri should fail");
        assert!(err.to_string().contains("--manifest-uri cannot be empty"));
    }

    #[test]
    fn build_health_check_config_disables_storage_maintenance() {
        let args = HealthCheckArgs {
            manifest_path: Some(fixture_manifest_path_string()),
            manifest_uri: None,
            json: false,
        };
        let config = build_health_check_config(&args).expect("config");
        assert!(!config.cleanup_storage_on_start);
        assert_eq!(config.storage_max_instances, 0);
    }

    #[tokio::test]
    async fn build_cli_health_payload_reports_ready_status() {
        let args = HealthCheckArgs {
            manifest_path: Some(fixture_manifest_path_string()),
            manifest_uri: None,
            json: false,
        };
        let mut config = build_health_check_config(&args).expect("config");
        config.search.enable_vector_search = false;
        config.search.enable_sparse_search = false;
        config.search.enable_reranker = false;
        let loaded = execute_manifest_load(config).await.expect("load");
        let payload = build_cli_health_payload(&loaded.search).await;

        assert_eq!(payload["status"], serde_json::json!("ready"));
        assert!(payload["entity_count"].as_u64().is_some());
        assert!(payload["manifest"].is_object());
        assert!(payload["manifest_cache"].is_object());
        assert!(payload["search_concurrency"].is_object());
        assert!(payload["sql_concurrency"].is_object());
    }

    #[tokio::test]
    async fn build_cli_health_payload_includes_search_warnings() {
        let args = HealthCheckArgs {
            manifest_path: Some(fixture_manifest_path_string()),
            manifest_uri: None,
            json: false,
        };
        let mut config = build_health_check_config(&args).expect("config");
        config.search.enable_vector_search = false;
        config.search.enable_sparse_search = false;
        config.search.enable_reranker = false;
        let mut loaded = execute_manifest_load(config).await.expect("load");
        loaded.search.search_init_warnings.insert(
            "vector".to_string(),
            "Vector search initialization failed".to_string(),
        );

        let payload = build_cli_health_payload(&loaded.search).await;
        assert_eq!(
            payload["search"]["vector"]["warning"]
                .as_str()
                .unwrap_or(""),
            "Vector search initialization failed"
        );
        assert!(payload["search"]["vector"]["ready"].is_boolean());
    }
}
