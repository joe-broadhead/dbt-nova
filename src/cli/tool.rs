use std::future::Future;
use std::io::Read;
use std::pin::Pin;
use std::time::{Duration, Instant};

use serde::de::DeserializeOwned;
use serde_json::Value as JsonValue;

use crate::cli::args::{ManifestLoadArgs, ToolCallArgs};
use crate::cli::health_cmd::build_cli_health_payload;
use crate::cli::manifest::{build_manifest_load_config, execute_manifest_load};
use crate::cli::output::{CliEnvelope, error_envelope};
use crate::error::{DbtNovaError, Result};
use crate::manifest::search::ManifestSearch;
use crate::params::{
    BatchGetParams, DiffEntitiesParams, ExecuteSqlParams, FindByPathParams, GetColumnLineageParams,
    GetColumnsParams, GetContextParams, GetEntityParams, GetImpactParams, GetLineageParams,
    GetMetadataScoreParams, GetRecipeParams, GetSqlParams, GetTestCoverageParams,
    GetUndocumentedParams, ListEntitiesParams, RunRecipeParams, SearchParams, SearchRecipesParams,
    ValidateDagParams,
};
use crate::responses::SuccessResponse;

use super::{DispatchError, DispatchResult};

type ToolFuture<'a> = Pin<Box<dyn Future<Output = Result<JsonValue>> + Send + 'a>>;
type ToolDispatchFn = for<'a> fn(&'a ManifestSearch, JsonValue) -> ToolFuture<'a>;

#[derive(Clone, Copy)]
struct ToolRegistryEntry {
    name: &'static str,
    dispatch: ToolDispatchFn,
}

const TOOL_REGISTRY: [ToolRegistryEntry; 26] = [
    ToolRegistryEntry {
        name: "search",
        dispatch: dispatch_search,
    },
    ToolRegistryEntry {
        name: "get_entity",
        dispatch: dispatch_get_entity,
    },
    ToolRegistryEntry {
        name: "list_entities",
        dispatch: dispatch_list_entities,
    },
    ToolRegistryEntry {
        name: "get_lineage",
        dispatch: dispatch_get_lineage,
    },
    ToolRegistryEntry {
        name: "get_sql",
        dispatch: dispatch_get_sql,
    },
    ToolRegistryEntry {
        name: "get_columns",
        dispatch: dispatch_get_columns,
    },
    ToolRegistryEntry {
        name: "diff_entities",
        dispatch: dispatch_diff_entities,
    },
    ToolRegistryEntry {
        name: "get_impact",
        dispatch: dispatch_get_impact,
    },
    ToolRegistryEntry {
        name: "validate_dag",
        dispatch: dispatch_validate_dag,
    },
    ToolRegistryEntry {
        name: "show_metadata",
        dispatch: dispatch_show_metadata,
    },
    ToolRegistryEntry {
        name: "health",
        dispatch: dispatch_health,
    },
    ToolRegistryEntry {
        name: "reload_manifest",
        dispatch: dispatch_reload_manifest,
    },
    ToolRegistryEntry {
        name: "list_tags",
        dispatch: dispatch_list_tags,
    },
    ToolRegistryEntry {
        name: "list_packages",
        dispatch: dispatch_list_packages,
    },
    ToolRegistryEntry {
        name: "list_databases",
        dispatch: dispatch_list_databases,
    },
    ToolRegistryEntry {
        name: "get_column_lineage",
        dispatch: dispatch_get_column_lineage,
    },
    ToolRegistryEntry {
        name: "get_test_coverage",
        dispatch: dispatch_get_test_coverage,
    },
    ToolRegistryEntry {
        name: "get_metadata_score",
        dispatch: dispatch_get_metadata_score,
    },
    ToolRegistryEntry {
        name: "batch_get_entities",
        dispatch: dispatch_batch_get_entities,
    },
    ToolRegistryEntry {
        name: "find_by_path",
        dispatch: dispatch_find_by_path,
    },
    ToolRegistryEntry {
        name: "search_recipes",
        dispatch: dispatch_search_recipes,
    },
    ToolRegistryEntry {
        name: "get_recipe",
        dispatch: dispatch_get_recipe,
    },
    ToolRegistryEntry {
        name: "run_recipe",
        dispatch: dispatch_run_recipe,
    },
    ToolRegistryEntry {
        name: "get_undocumented",
        dispatch: dispatch_get_undocumented,
    },
    ToolRegistryEntry {
        name: "get_context",
        dispatch: dispatch_get_context,
    },
    ToolRegistryEntry {
        name: "execute_sql",
        dispatch: dispatch_execute_sql,
    },
];

/// Runs the `tool call` CLI command.
///
/// # Errors
/// Returns an error if parameter sources are invalid, manifest loading fails, or tool execution fails.
pub async fn run_call_command(args: &ToolCallArgs) -> DispatchResult {
    let started = Instant::now();
    let tool_entry = resolve_tool_entry(&args.tool_name)
        .map_err(|error| render_or_propagate_error(args, error, started.elapsed().as_millis()))?;
    if tool_entry.name == "reload_manifest" {
        return Err(render_or_propagate_error(
            args,
            DbtNovaError::InvalidParams(
                "tool 'reload_manifest' is not available in CLI mode".to_string(),
            ),
            started.elapsed().as_millis(),
        ));
    }
    let params = resolve_params_value(args)
        .map_err(|error| render_or_propagate_error(args, error, started.elapsed().as_millis()))?;
    let config = build_manifest_load_config(&manifest_load_args_from_tool_args(args))
        .map_err(|error| render_or_propagate_error(args, error, started.elapsed().as_millis()))?;
    let load_result = execute_manifest_load(config)
        .await
        .map_err(|error| render_or_propagate_error(args, error, started.elapsed().as_millis()))?;
    let result = dispatch_tool(&load_result.search, &args.tool_name, params)
        .await
        .map_err(|error| render_or_propagate_error(args, error, started.elapsed().as_millis()))?;

    if args.json {
        let envelope = CliEnvelope::success(
            format!("tool call {}", args.tool_name),
            result,
            started.elapsed().as_millis(),
        );
        let out = serde_json::to_string_pretty(&envelope).map_err(|error| DispatchError {
            error: DbtNovaError::ServerError(error.to_string()),
            rendered: false,
        })?;
        println!("{out}");
    } else {
        print_human_result(&result).map_err(|error| DispatchError {
            error,
            rendered: false,
        })?;
    }

    Ok(())
}

fn print_human_result(result: &JsonValue) -> Result<()> {
    let out = serde_json::to_string_pretty(result)
        .map_err(|error| DbtNovaError::ServerError(error.to_string()))?;
    println!("{out}");
    Ok(())
}

fn render_or_propagate_error(
    args: &ToolCallArgs,
    error: DbtNovaError,
    elapsed_ms: u128,
) -> DispatchError {
    if args.json {
        let envelope = error_envelope(format!("tool call {}", args.tool_name), &error, elapsed_ms);
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

fn manifest_load_args_from_tool_args(args: &ToolCallArgs) -> ManifestLoadArgs {
    ManifestLoadArgs {
        manifest_path: args.manifest_path.clone(),
        manifest_uri: args.manifest_uri.clone(),
        storage_instance_id: args.storage_instance_id.clone(),
        cleanup_storage_on_start: args.cleanup_storage_on_start,
        read_only: args.read_only,
        json: false,
    }
}

fn resolve_params_value(args: &ToolCallArgs) -> Result<JsonValue> {
    resolve_params_value_with_stdin(args, || {
        let mut raw = String::new();
        std::io::stdin().read_to_string(&mut raw).map_err(|error| {
            DbtNovaError::InvalidParams(format!("failed to read stdin: {error}"))
        })?;
        Ok(raw)
    })
}

fn resolve_params_value_with_stdin(
    args: &ToolCallArgs,
    read_stdin: impl FnOnce() -> Result<String>,
) -> Result<JsonValue> {
    let explicit_sources = usize::from(args.params_json.is_some())
        + usize::from(args.params_file.is_some())
        + usize::from(args.params_stdin);
    if explicit_sources > 1 {
        return Err(DbtNovaError::InvalidParams(
            "only one of --params-json, --params-file, or --params-stdin may be set".to_string(),
        ));
    }

    if let Some(inline) = args.params_json.as_ref() {
        return parse_params_payload("--params-json", inline);
    }
    if let Some(path) = args.params_file.as_ref() {
        let raw = std::fs::read_to_string(path).map_err(|error| {
            DbtNovaError::InvalidParams(format!("failed to read params file '{path}': {error}"))
        })?;
        return parse_params_payload("--params-file", &raw);
    }
    if args.params_stdin {
        let raw = read_stdin()?;
        return parse_params_payload("--params-stdin", &raw);
    }

    Ok(serde_json::json!({}))
}

fn parse_params_payload(source: &str, raw: &str) -> Result<JsonValue> {
    if raw.trim().is_empty() {
        return Err(DbtNovaError::InvalidParams(format!(
            "empty JSON payload from {source}"
        )));
    }
    serde_json::from_str(raw).map_err(|error| {
        DbtNovaError::InvalidParams(format!("failed to parse JSON from {source}: {error}"))
    })
}

fn decode_tool_params<T: DeserializeOwned>(tool_name: &str, value: JsonValue) -> Result<T> {
    serde_json::from_value(value).map_err(|error| {
        DbtNovaError::InvalidParams(format!("invalid params for '{tool_name}': {error}"))
    })
}

fn decode_empty_params(tool_name: &str, value: JsonValue) -> Result<()> {
    match value {
        JsonValue::Null => Ok(()),
        JsonValue::Object(map) if map.is_empty() => Ok(()),
        JsonValue::Object(_) => Err(DbtNovaError::InvalidParams(format!(
            "tool '{tool_name}' does not accept parameters"
        ))),
        _ => Err(DbtNovaError::InvalidParams(format!(
            "tool '{tool_name}' expects an object parameter payload"
        ))),
    }
}

fn tool_registry_names() -> Vec<&'static str> {
    TOOL_REGISTRY.iter().map(|entry| entry.name).collect()
}

fn resolve_tool_entry(tool_name: &str) -> Result<ToolRegistryEntry> {
    TOOL_REGISTRY
        .iter()
        .find(|entry| entry.name == tool_name)
        .copied()
        .ok_or_else(|| {
            DbtNovaError::InvalidParams(format!(
                "unknown tool '{}'; supported tools: {}",
                tool_name,
                tool_registry_names().join(", ")
            ))
        })
}

async fn dispatch_tool(
    searcher: &ManifestSearch,
    tool_name: &str,
    params: JsonValue,
) -> Result<JsonValue> {
    let entry = resolve_tool_entry(tool_name)?;
    (entry.dispatch)(searcher, params).await
}

macro_rules! typed_dispatch {
    ($fn_name:ident, $tool_name:literal, $params_ty:ty, $method:ident) => {
        fn $fn_name(searcher: &ManifestSearch, params: JsonValue) -> ToolFuture<'_> {
            Box::pin(async move {
                let decoded: $params_ty = decode_tool_params($tool_name, params)?;
                searcher.$method(&decoded).await
            })
        }
    };
}

macro_rules! empty_dispatch {
    ($fn_name:ident, $tool_name:literal, $method:ident) => {
        fn $fn_name(searcher: &ManifestSearch, params: JsonValue) -> ToolFuture<'_> {
            Box::pin(async move {
                decode_empty_params($tool_name, params)?;
                searcher.$method().await
            })
        }
    };
}

async fn run_search_with_timeout<T>(
    timeout_ms: usize,
    future: impl Future<Output = Result<T>>,
) -> Result<T> {
    if timeout_ms == 0 {
        return future.await;
    }
    let timeout_ms_u64 = u64::try_from(timeout_ms).map_err(|_| {
        DbtNovaError::ServerError("search timeout exceeds supported duration".to_string())
    })?;
    match tokio::time::timeout(Duration::from_millis(timeout_ms_u64), future).await {
        Ok(result) => result,
        Err(_) => Err(DbtNovaError::ServerError(format!(
            "Search timed out after {timeout_ms}ms"
        ))),
    }
}

fn dispatch_search(searcher: &ManifestSearch, params: JsonValue) -> ToolFuture<'_> {
    Box::pin(async move {
        let decoded: SearchParams = decode_tool_params("search", params)?;
        run_search_with_timeout(
            searcher.config().search.search_timeout_ms,
            searcher.search(&decoded),
        )
        .await
    })
}

typed_dispatch!(
    dispatch_get_entity,
    "get_entity",
    GetEntityParams,
    get_entity_data
);
typed_dispatch!(
    dispatch_list_entities,
    "list_entities",
    ListEntitiesParams,
    list_entities
);
typed_dispatch!(
    dispatch_get_lineage,
    "get_lineage",
    GetLineageParams,
    get_lineage
);
typed_dispatch!(dispatch_get_sql, "get_sql", GetSqlParams, get_sql);
typed_dispatch!(
    dispatch_get_columns,
    "get_columns",
    GetColumnsParams,
    get_columns
);
typed_dispatch!(
    dispatch_diff_entities,
    "diff_entities",
    DiffEntitiesParams,
    diff_entities
);
typed_dispatch!(
    dispatch_get_impact,
    "get_impact",
    GetImpactParams,
    get_impact
);
typed_dispatch!(
    dispatch_validate_dag,
    "validate_dag",
    ValidateDagParams,
    validate_dag
);
empty_dispatch!(dispatch_show_metadata, "show_metadata", show_metadata);
empty_dispatch!(dispatch_list_tags, "list_tags", list_tags);
empty_dispatch!(dispatch_list_packages, "list_packages", list_packages);
empty_dispatch!(dispatch_list_databases, "list_databases", list_databases);
typed_dispatch!(
    dispatch_get_column_lineage,
    "get_column_lineage",
    GetColumnLineageParams,
    get_column_lineage
);
typed_dispatch!(
    dispatch_get_test_coverage,
    "get_test_coverage",
    GetTestCoverageParams,
    get_test_coverage
);
typed_dispatch!(
    dispatch_get_metadata_score,
    "get_metadata_score",
    GetMetadataScoreParams,
    get_metadata_score
);
typed_dispatch!(
    dispatch_batch_get_entities,
    "batch_get_entities",
    BatchGetParams,
    batch_get_entities
);
typed_dispatch!(
    dispatch_find_by_path,
    "find_by_path",
    FindByPathParams,
    find_by_path
);
typed_dispatch!(
    dispatch_search_recipes,
    "search_recipes",
    SearchRecipesParams,
    search_recipes
);
typed_dispatch!(
    dispatch_get_recipe,
    "get_recipe",
    GetRecipeParams,
    get_recipe
);
typed_dispatch!(
    dispatch_run_recipe,
    "run_recipe",
    RunRecipeParams,
    run_recipe
);
typed_dispatch!(
    dispatch_get_undocumented,
    "get_undocumented",
    GetUndocumentedParams,
    get_undocumented
);
typed_dispatch!(
    dispatch_get_context,
    "get_context",
    GetContextParams,
    get_context
);
typed_dispatch!(
    dispatch_execute_sql,
    "execute_sql",
    ExecuteSqlParams,
    execute_sql
);

fn dispatch_health(searcher: &ManifestSearch, params: JsonValue) -> ToolFuture<'_> {
    Box::pin(async move {
        decode_empty_params("health", params)?;
        let payload = build_cli_health_payload(searcher).await;
        serde_json::to_value(SuccessResponse::new(payload, 1))
            .map_err(|error| DbtNovaError::ServerError(error.to_string()))
    })
}

fn dispatch_reload_manifest(_searcher: &ManifestSearch, _params: JsonValue) -> ToolFuture<'_> {
    Box::pin(async move {
        Err(DbtNovaError::InvalidParams(
            "tool 'reload_manifest' is not available in CLI mode".to_string(),
        ))
    })
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::time::Duration;

    use tempfile::NamedTempFile;

    use super::{
        dispatch_tool, resolve_params_value_with_stdin, run_call_command, run_search_with_timeout,
        tool_registry_names,
    };
    use crate::cli::args::{ManifestLoadArgs, ToolCallArgs};
    use crate::cli::manifest::{build_manifest_load_config, execute_manifest_load};

    fn fixture_manifest_path() -> String {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("nova_manifest.json")
            .to_string_lossy()
            .to_string()
    }

    async fn fixture_searcher() -> crate::manifest::search::ManifestSearch {
        let args = ManifestLoadArgs {
            manifest_path: Some(fixture_manifest_path()),
            ..ManifestLoadArgs::default()
        };
        let mut config = build_manifest_load_config(&args).expect("config");
        config.search.enable_vector_search = false;
        config.search.enable_sparse_search = false;
        config.search.enable_reranker = false;
        execute_manifest_load(config)
            .await
            .expect("load fixture")
            .search
    }

    #[test]
    fn params_source_runtime_guard_rejects_multiple_explicit_sources() {
        let args = ToolCallArgs {
            tool_name: "search".to_string(),
            params_json: Some("{}".to_string()),
            params_file: Some("params.json".to_string()),
            ..ToolCallArgs::default()
        };
        let err = resolve_params_value_with_stdin(&args, || Ok(String::new())).expect_err("guard");
        assert!(
            err.to_string()
                .contains("only one of --params-json, --params-file, or --params-stdin")
        );
    }

    #[test]
    fn params_default_to_empty_object() {
        let args = ToolCallArgs {
            tool_name: "list_tags".to_string(),
            ..ToolCallArgs::default()
        };
        let value =
            resolve_params_value_with_stdin(&args, || Ok(String::new())).expect("default payload");
        assert_eq!(value, serde_json::json!({}));
    }

    #[test]
    fn params_file_invalid_json_is_rejected() {
        let file = NamedTempFile::new().expect("temp file");
        std::fs::write(file.path(), "{not-json").expect("write");
        let args = ToolCallArgs {
            tool_name: "search".to_string(),
            params_file: Some(file.path().to_string_lossy().to_string()),
            ..ToolCallArgs::default()
        };
        let err =
            resolve_params_value_with_stdin(&args, || Ok(String::new())).expect_err("invalid json");
        assert!(
            err.to_string()
                .contains("failed to parse JSON from --params-file")
        );
    }

    #[test]
    fn params_stdin_invalid_json_is_rejected() {
        let args = ToolCallArgs {
            tool_name: "search".to_string(),
            params_stdin: true,
            ..ToolCallArgs::default()
        };
        let err = resolve_params_value_with_stdin(&args, || Ok("{not-json".to_string()))
            .expect_err("invalid json");
        assert!(
            err.to_string()
                .contains("failed to parse JSON from --params-stdin")
        );
    }

    #[tokio::test]
    async fn dispatch_search_with_valid_params_succeeds() {
        let searcher = fixture_searcher().await;
        let result = dispatch_tool(
            &searcher,
            "search",
            serde_json::json!({
                "query": "campaign",
                "limit": 1
            }),
        )
        .await
        .expect("search");
        assert!(result["data"].is_array());
    }

    #[tokio::test]
    async fn dispatch_health_returns_standard_response_shape() {
        let searcher = fixture_searcher().await;
        let result = dispatch_tool(&searcher, "health", serde_json::json!({}))
            .await
            .expect("health");
        assert_eq!(result["success"], serde_json::json!(true));
        assert_eq!(result["count"], serde_json::json!(1));
        assert!(result["data"].is_object());
        assert!(result["data"]["tool_metrics"].is_object());
        assert!(result["data"]["search_concurrency"].is_object());
        assert!(result["data"]["sql_concurrency"].is_object());
    }

    #[tokio::test]
    async fn dispatch_health_rejects_unexpected_params() {
        let searcher = fixture_searcher().await;
        let err = dispatch_tool(
            &searcher,
            "health",
            serde_json::json!({
                "unexpected": true
            }),
        )
        .await
        .expect_err("health should reject params");
        assert!(
            err.to_string()
                .contains("tool 'health' does not accept parameters")
        );
    }

    #[tokio::test]
    async fn run_search_with_timeout_enforces_timeout() {
        let err = run_search_with_timeout(1, async {
            tokio::time::sleep(Duration::from_millis(50)).await;
            Ok(serde_json::json!({"ok": true}))
        })
        .await
        .expect_err("timeout should fail");
        assert!(err.to_string().contains("Search timed out after 1ms"));
    }

    #[tokio::test]
    async fn dispatch_reload_manifest_returns_cli_mode_error() {
        let searcher = fixture_searcher().await;
        let err = dispatch_tool(&searcher, "reload_manifest", serde_json::json!({}))
            .await
            .expect_err("reload should fail in cli mode");
        assert!(
            err.to_string()
                .contains("tool 'reload_manifest' is not available in CLI mode")
        );
    }

    #[tokio::test]
    async fn dispatch_unknown_tool_returns_invalid_params() {
        let searcher = fixture_searcher().await;
        let err = dispatch_tool(&searcher, "unknown_tool", serde_json::json!({}))
            .await
            .expect_err("unknown tool should fail");
        assert!(err.to_string().contains("unknown tool 'unknown_tool'"));
    }

    #[tokio::test]
    async fn run_call_command_unknown_tool_short_circuits_before_manifest_load() {
        let args = ToolCallArgs {
            tool_name: "unknown_tool".to_string(),
            manifest_path: Some("tests/fixtures/missing-manifest.json".to_string()),
            ..ToolCallArgs::default()
        };
        let Err(err) = run_call_command(&args).await else {
            panic!("unknown tool should fail");
        };
        assert!(
            err.error
                .to_string()
                .contains("unknown tool 'unknown_tool'")
        );
    }

    #[test]
    fn tool_registry_has_full_mcp_name_parity() {
        let expected = vec![
            "search",
            "get_entity",
            "list_entities",
            "get_lineage",
            "get_sql",
            "get_columns",
            "diff_entities",
            "get_impact",
            "validate_dag",
            "show_metadata",
            "health",
            "reload_manifest",
            "list_tags",
            "list_packages",
            "list_databases",
            "get_column_lineage",
            "get_test_coverage",
            "get_metadata_score",
            "batch_get_entities",
            "find_by_path",
            "search_recipes",
            "get_recipe",
            "run_recipe",
            "get_undocumented",
            "get_context",
            "execute_sql",
        ];
        assert_eq!(tool_registry_names(), expected);
    }
}
