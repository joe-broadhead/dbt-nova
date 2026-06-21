use std::future::Future;
use std::io::Read;
use std::pin::Pin;
use std::time::{Duration, Instant};

use serde::de::DeserializeOwned;
use serde_json::Value as JsonValue;

use crate::cli::agent_readiness_cmd::build_agent_readiness_tool_response;
use crate::cli::args::{ManifestLoadArgs, ManifestReloadArgs, ToolCallArgs};
use crate::cli::audit_cmd::build_metadata_audit_tool_response;
use crate::cli::eval_cmd::{
    build_agent_eval_tool_response, build_eval_gate_tool_response,
    build_eval_history_tool_response, build_eval_init_tool_response, build_eval_run_tool_response,
    build_eval_validate_tool_response,
};
use crate::cli::health_cmd::build_cli_health_payload;
use crate::cli::manifest::{
    build_manifest_load_config, build_manifest_reload_config, execute_manifest_load,
};
use crate::cli::nova_meta_cmd::build_nova_meta_tool_response;
use crate::cli::output::{CliEnvelope, error_envelope};
use crate::error::{DbtNovaError, Result};
use crate::manifest::search::ManifestSearch;
use crate::params::{
    BatchGetParams, ColumnInventoryParams, CompareGrainsParams, DiffEntitiesParams,
    ExecuteSqlParams, FindByPathParams, FindEntityOverlapParams, GetAgentReadinessParams,
    GetColumnLineageParams, GetColumnsParams, GetContextParams, GetEntityParams, GetEvalGateParams,
    GetEvalHistoryParams, GetImpactParams, GetLineageParams, GetMetadataAuditParams,
    GetMetadataScoreParams, GetRecipeParams, GetSqlParams, GetTestCoverageParams,
    GetUndocumentedParams, IndicatorInventoryParams, InitEvalSuiteParams, ListEntitiesParams,
    ModellingConsistencyReportParams, ReloadManifestParams, RunAgentEvalParams, RunEvalParams,
    RunRecipeParams, SearchColumnsParams, SearchIndicatorParams, SearchParams, SearchRecipesParams,
    ValidateDagParams, ValidateEvalSuiteParams, ValidateNovaMetaParams,
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

const TOOL_REGISTRY: [ToolRegistryEntry; 42] = [
    ToolRegistryEntry {
        name: "search",
        dispatch: dispatch_search,
    },
    ToolRegistryEntry {
        name: "search_indicator",
        dispatch: dispatch_search_indicator,
    },
    ToolRegistryEntry {
        name: "indicator_inventory",
        dispatch: dispatch_indicator_inventory,
    },
    ToolRegistryEntry {
        name: "search_columns",
        dispatch: dispatch_search_columns,
    },
    ToolRegistryEntry {
        name: "column_inventory",
        dispatch: dispatch_column_inventory,
    },
    ToolRegistryEntry {
        name: "compare_grains",
        dispatch: dispatch_compare_grains,
    },
    ToolRegistryEntry {
        name: "find_entity_overlap",
        dispatch: dispatch_find_entity_overlap,
    },
    ToolRegistryEntry {
        name: "modelling_consistency_report",
        dispatch: dispatch_modelling_consistency_report,
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
        name: "validate_nova_meta",
        dispatch: dispatch_validate_nova_meta,
    },
    ToolRegistryEntry {
        name: "validate_eval_suite",
        dispatch: dispatch_validate_eval_suite,
    },
    ToolRegistryEntry {
        name: "get_eval_gate",
        dispatch: dispatch_get_eval_gate,
    },
    ToolRegistryEntry {
        name: "get_eval_history",
        dispatch: dispatch_get_eval_history,
    },
    ToolRegistryEntry {
        name: "run_eval",
        dispatch: dispatch_run_eval,
    },
    ToolRegistryEntry {
        name: "init_eval_suite",
        dispatch: dispatch_init_eval_suite,
    },
    ToolRegistryEntry {
        name: "run_agent_eval",
        dispatch: dispatch_run_agent_eval,
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
        name: "get_metadata_audit",
        dispatch: dispatch_get_metadata_audit,
    },
    ToolRegistryEntry {
        name: "get_agent_readiness",
        dispatch: dispatch_get_agent_readiness,
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
        return run_reload_manifest_tool_call(args, started).await;
    }
    let params = resolve_params_value(args)
        .map_err(|error| render_or_propagate_error(args, error, started.elapsed().as_millis()))?;
    let trace_params = params.clone();
    let config = build_manifest_load_config(&manifest_load_args_from_tool_args(args))
        .map_err(|error| render_or_propagate_error(args, error, started.elapsed().as_millis()))?;
    let load_result = execute_manifest_load(config)
        .await
        .map_err(|error| render_or_propagate_error(args, error, started.elapsed().as_millis()))?;
    let result = match dispatch_tool(&load_result.search, &args.tool_name, params).await {
        Ok(result) => result,
        Err(error) => {
            let trace_error = trace_error_response(&error);
            crate::utils::tool_trace::record_tool_call(
                "cli",
                &args.tool_name,
                Some(&trace_params),
                Some(&trace_error),
                false,
                elapsed_ms_to_u64(started.elapsed()),
            );
            return Err(render_or_propagate_error(
                args,
                error,
                started.elapsed().as_millis(),
            ));
        }
    };
    crate::utils::tool_trace::record_tool_call(
        "cli",
        &args.tool_name,
        Some(&trace_params),
        Some(&result),
        true,
        elapsed_ms_to_u64(started.elapsed()),
    );

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

async fn run_reload_manifest_tool_call(args: &ToolCallArgs, started: Instant) -> DispatchResult {
    let params = resolve_params_value(args)
        .map_err(|error| render_or_propagate_error(args, error, started.elapsed().as_millis()))?;
    let trace_params = params.clone();
    let reload_params: ReloadManifestParams = decode_tool_params("reload_manifest", params)
        .map_err(|error| render_or_propagate_error(args, error, started.elapsed().as_millis()))?;
    let reload_args = build_reload_args_from_tool_call(args, &reload_params);
    let config = build_manifest_reload_config(&reload_args)
        .map_err(|error| render_or_propagate_error(args, error, started.elapsed().as_millis()))?;
    let load_result = execute_manifest_load(config)
        .await
        .map_err(|error| render_or_propagate_error(args, error, started.elapsed().as_millis()))?;
    let payload = serde_json::json!({
        "status": "reloaded",
        "manifest_path": load_result.search.config().manifest_path,
        "manifest_uri": load_result.search.config().manifest_uri,
        "manifest_refresh_secs": load_result.search.config().manifest_refresh_secs,
        "storage_instance_id": load_result.search.config().storage_instance_id,
        "manifest_hash": load_result.search.manifest_hash,
        "manifest_version": load_result.search.manifest_version,
        "entity_count": load_result.search.entity_count(),
    });
    let result = serde_json::to_value(SuccessResponse::new(payload, 1)).map_err(|error| {
        render_or_propagate_error(
            args,
            DbtNovaError::ServerError(error.to_string()),
            started.elapsed().as_millis(),
        )
    })?;
    crate::utils::tool_trace::record_tool_call(
        "cli",
        &args.tool_name,
        Some(&trace_params),
        Some(&result),
        true,
        elapsed_ms_to_u64(started.elapsed()),
    );

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

fn build_reload_args_from_tool_call(
    args: &ToolCallArgs,
    params: &ReloadManifestParams,
) -> ManifestReloadArgs {
    let mut manifest_path = args.manifest_path.clone();
    let mut manifest_uri = args.manifest_uri.clone();

    if let Some(uri) = normalize_param_string(params.manifest_uri.as_deref()) {
        manifest_uri = Some(uri);
    }
    if let Some(path) = normalize_param_string(params.manifest_path.as_deref()) {
        manifest_path = Some(path);
        manifest_uri = None;
    }

    let storage_instance_id = normalize_param_string(params.storage_instance_id.as_deref())
        .or_else(|| args.storage_instance_id.clone());

    ManifestReloadArgs {
        manifest_path,
        manifest_uri,
        refresh_secs: params.refresh_secs,
        storage_instance_id,
        cleanup_storage_on_start: args.cleanup_storage_on_start,
        read_only: args.read_only,
        json: false,
    }
}

fn normalize_param_string(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|trimmed| !trimmed.is_empty())
        .map(ToString::to_string)
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

fn trace_error_response(error: &DbtNovaError) -> JsonValue {
    serde_json::json!({
        "success": false,
        "error": error.to_string(),
        "error_code": error.error_code(),
    })
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

pub(crate) async fn dispatch_tool(
    searcher: &ManifestSearch,
    tool_name: &str,
    params: JsonValue,
) -> Result<JsonValue> {
    let entry = resolve_tool_entry(tool_name)?;
    (entry.dispatch)(searcher, params).await
}

fn elapsed_ms_to_u64(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
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

macro_rules! timed_dispatch {
    ($fn_name:ident, $tool_name:literal, $params_ty:ty, $method:ident) => {
        fn $fn_name(searcher: &ManifestSearch, params: JsonValue) -> ToolFuture<'_> {
            Box::pin(async move {
                let decoded: $params_ty = decode_tool_params($tool_name, params)?;
                run_search_with_timeout(
                    searcher.config().search.search_timeout_ms,
                    searcher.$method(&decoded),
                )
                .await
            })
        }
    };
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

fn dispatch_search_indicator(searcher: &ManifestSearch, params: JsonValue) -> ToolFuture<'_> {
    Box::pin(async move {
        let decoded: SearchIndicatorParams = decode_tool_params("search_indicator", params)?;
        run_search_with_timeout(
            searcher.config().search.search_timeout_ms,
            searcher.search_indicator(&decoded),
        )
        .await
    })
}

timed_dispatch!(
    dispatch_indicator_inventory,
    "indicator_inventory",
    IndicatorInventoryParams,
    indicator_inventory
);

fn dispatch_search_columns(searcher: &ManifestSearch, params: JsonValue) -> ToolFuture<'_> {
    Box::pin(async move {
        let decoded: SearchColumnsParams = decode_tool_params("search_columns", params)?;
        run_search_with_timeout(
            searcher.config().search.search_timeout_ms,
            searcher.search_columns(&decoded),
        )
        .await
    })
}

timed_dispatch!(
    dispatch_column_inventory,
    "column_inventory",
    ColumnInventoryParams,
    column_inventory
);

timed_dispatch!(
    dispatch_compare_grains,
    "compare_grains",
    CompareGrainsParams,
    compare_grains
);
timed_dispatch!(
    dispatch_find_entity_overlap,
    "find_entity_overlap",
    FindEntityOverlapParams,
    find_entity_overlap
);
timed_dispatch!(
    dispatch_modelling_consistency_report,
    "modelling_consistency_report",
    ModellingConsistencyReportParams,
    modelling_consistency_report
);

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

fn dispatch_validate_nova_meta(_searcher: &ManifestSearch, params: JsonValue) -> ToolFuture<'_> {
    Box::pin(async move {
        let decoded: ValidateNovaMetaParams = decode_tool_params("validate_nova_meta", params)?;
        build_nova_meta_tool_response(&decoded)
    })
}

fn dispatch_validate_eval_suite(_searcher: &ManifestSearch, params: JsonValue) -> ToolFuture<'_> {
    Box::pin(async move {
        let decoded: ValidateEvalSuiteParams = decode_tool_params("validate_eval_suite", params)?;
        build_eval_validate_tool_response(&decoded)
    })
}

fn dispatch_get_eval_gate(_searcher: &ManifestSearch, params: JsonValue) -> ToolFuture<'_> {
    Box::pin(async move {
        let decoded: GetEvalGateParams = decode_tool_params("get_eval_gate", params)?;
        build_eval_gate_tool_response(&decoded)
    })
}

fn dispatch_get_eval_history(_searcher: &ManifestSearch, params: JsonValue) -> ToolFuture<'_> {
    Box::pin(async move {
        let decoded: GetEvalHistoryParams = decode_tool_params("get_eval_history", params)?;
        build_eval_history_tool_response(&decoded)
    })
}

fn dispatch_run_eval(searcher: &ManifestSearch, params: JsonValue) -> ToolFuture<'_> {
    Box::pin(async move {
        let decoded: RunEvalParams = decode_tool_params("run_eval", params)?;
        build_eval_run_tool_response(searcher, &decoded).await
    })
}

fn dispatch_init_eval_suite(_searcher: &ManifestSearch, params: JsonValue) -> ToolFuture<'_> {
    Box::pin(async move {
        let decoded: InitEvalSuiteParams = decode_tool_params("init_eval_suite", params)?;
        build_eval_init_tool_response(&decoded)
    })
}

fn dispatch_run_agent_eval(_searcher: &ManifestSearch, params: JsonValue) -> ToolFuture<'_> {
    Box::pin(async move {
        let decoded: RunAgentEvalParams = decode_tool_params("run_agent_eval", params)?;
        build_agent_eval_tool_response(&decoded).await
    })
}

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

fn dispatch_get_metadata_audit(searcher: &ManifestSearch, params: JsonValue) -> ToolFuture<'_> {
    Box::pin(async move {
        let decoded: GetMetadataAuditParams = decode_tool_params("get_metadata_audit", params)?;
        build_metadata_audit_tool_response(searcher, &decoded).await
    })
}

fn dispatch_get_agent_readiness(searcher: &ManifestSearch, params: JsonValue) -> ToolFuture<'_> {
    Box::pin(async move {
        let decoded: GetAgentReadinessParams = decode_tool_params("get_agent_readiness", params)?;
        build_agent_readiness_tool_response(searcher, &decoded).await
    })
}

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
    use std::time::Duration;

    use tempfile::NamedTempFile;

    use super::{
        build_reload_args_from_tool_call, dispatch_tool, resolve_params_value_with_stdin,
        run_call_command, run_search_with_timeout, tool_registry_names,
    };
    use crate::cli::args::{ManifestLoadArgs, ToolCallArgs};
    use crate::cli::manifest::{build_manifest_load_config, execute_manifest_load};
    use crate::params::ReloadManifestParams;
    use crate::tests::common::fixture_manifest_path_string;
    use crate::tools::catalog::MCP_TOOL_NAMES;

    async fn fixture_searcher() -> crate::manifest::search::ManifestSearch {
        let args = ManifestLoadArgs {
            manifest_path: Some(fixture_manifest_path_string()),
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
    async fn dispatch_get_agent_readiness_returns_standard_response_shape() {
        let searcher = fixture_searcher().await;
        let result = dispatch_tool(
            &searcher,
            "get_agent_readiness",
            serde_json::json!({
                "personas_json": "[\"engineer\"]",
                "eval_gate_json": "{\"allowed\":true,\"blocked\":false,\"message\":\"gate passed\"}"
            }),
        )
        .await
        .expect("agent readiness");
        assert_eq!(result["success"], serde_json::json!(true));
        assert_eq!(result["count"], serde_json::json!(1));
        assert_eq!(
            result["data"]["schema_version"],
            serde_json::json!("agent_readiness.v1")
        );
        assert_eq!(
            result["data"]["eval_status"]["status"],
            serde_json::json!("allowed")
        );
    }

    #[tokio::test]
    async fn dispatch_get_metadata_audit_returns_gate_data() {
        let searcher = fixture_searcher().await;
        let result = dispatch_tool(
            &searcher,
            "get_metadata_audit",
            serde_json::json!({
                "selection_mode": "project",
                "resource_types_json": "[\"model\"]",
                "personas_json": "[\"engineer\"]",
                "thresholds_json": "{\"project\":{\"engineer\":{\"min_score\":101,\"severity\":\"required\"}}}",
                "include_recommendations": false
            }),
        )
        .await
        .expect("metadata audit");
        assert_eq!(result["success"], serde_json::json!(true));
        assert_eq!(result["count"], serde_json::json!(1));
        assert_eq!(
            result["data"]["selection_mode"],
            serde_json::json!("project")
        );
        assert_eq!(result["data"]["gate_status"], serde_json::json!("fail"));
    }

    #[tokio::test]
    async fn dispatch_validate_nova_meta_returns_validation_report() {
        let root = std::env::current_dir()
            .expect("cwd")
            .canonicalize()
            .expect("canonical cwd");
        let temp_dir = tempfile::TempDir::new_in(&root).expect("temp project");
        let project_relative = temp_dir
            .path()
            .strip_prefix(&root)
            .expect("relative temp project")
            .display()
            .to_string();
        let models_dir = temp_dir.path().join("models");
        std::fs::create_dir_all(&models_dir).expect("models dir");
        std::fs::write(
            models_dir.join("orders.yml"),
            r"
version: 2
models:
  - name: fct_orders
    meta:
      nova:
        canonical: true
",
        )
        .expect("fixture");

        let searcher = fixture_searcher().await;
        let result = dispatch_tool(
            &searcher,
            "validate_nova_meta",
            serde_json::json!({
                "project_dir": project_relative,
                "paths": ["models/orders.yml"],
                "resource_kind": "model",
                "resource_name": "fct_orders"
            }),
        )
        .await
        .expect("nova-meta validation");

        assert_eq!(result["success"], serde_json::json!(true));
        assert_eq!(result["count"], serde_json::json!(1));
        assert_eq!(result["data"]["target_count"], serde_json::json!(1));
        assert_eq!(result["data"]["error_count"], serde_json::json!(0));
        assert_eq!(
            result["data"]["selector"]["paths"],
            serde_json::json!(["models/orders.yml"])
        );
    }

    #[tokio::test]
    async fn dispatch_validate_eval_suite_returns_validation_report() {
        let root = std::env::current_dir()
            .expect("cwd")
            .canonicalize()
            .expect("canonical cwd");
        let temp_dir = tempfile::TempDir::new_in(&root).expect("temp suite dir");
        let suite_path = temp_dir.path().join("suite.yml");
        std::fs::write(
            &suite_path,
            r"
version: 1
name: dispatch-eval-smoke
cases:
  - id: one
    assertions:
      - type: tool_success
        tool: search
        params: {}
",
        )
        .expect("suite fixture");

        let searcher = fixture_searcher().await;
        let result = dispatch_tool(
            &searcher,
            "validate_eval_suite",
            serde_json::json!({
                "suite": suite_path.display().to_string()
            }),
        )
        .await
        .expect("eval validation");

        assert_eq!(result["success"], serde_json::json!(true));
        assert_eq!(result["count"], serde_json::json!(1));
        assert_eq!(result["data"]["valid"], serde_json::json!(true));
        assert_eq!(
            result["data"]["suite_name"],
            serde_json::json!("dispatch-eval-smoke")
        );
        assert!(result["data"]["safety_policy"].is_object());
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

    #[test]
    fn build_reload_args_from_tool_call_prefers_non_empty_params() {
        let args = ToolCallArgs {
            tool_name: "reload_manifest".to_string(),
            manifest_path: Some("cli/manifest.json".to_string()),
            manifest_uri: Some("dbfs:/from-cli".to_string()),
            storage_instance_id: Some("from-cli".to_string()),
            cleanup_storage_on_start: true,
            read_only: true,
            ..ToolCallArgs::default()
        };
        let params = ReloadManifestParams {
            manifest_uri: Some("dbfs:/from-params".to_string()),
            manifest_path: Some("  ".to_string()),
            refresh_secs: Some(600),
            storage_instance_id: Some("from-params".to_string()),
        };
        let merged = build_reload_args_from_tool_call(&args, &params);
        assert_eq!(merged.manifest_uri.as_deref(), Some("dbfs:/from-params"));
        assert_eq!(merged.manifest_path.as_deref(), Some("cli/manifest.json"));
        assert_eq!(merged.storage_instance_id.as_deref(), Some("from-params"));
        assert_eq!(merged.refresh_secs, Some(600));
        assert!(merged.cleanup_storage_on_start);
        assert!(merged.read_only);
    }

    #[test]
    fn build_reload_args_from_tool_call_path_takes_precedence_over_uri() {
        let args = ToolCallArgs {
            tool_name: "reload_manifest".to_string(),
            ..ToolCallArgs::default()
        };
        let params = ReloadManifestParams {
            manifest_uri: Some("dbfs:/from-params".to_string()),
            manifest_path: Some("/tmp/from-params.json".to_string()),
            refresh_secs: None,
            storage_instance_id: None,
        };
        let merged = build_reload_args_from_tool_call(&args, &params);
        assert_eq!(
            merged.manifest_path.as_deref(),
            Some("/tmp/from-params.json")
        );
        assert_eq!(merged.manifest_uri, None);
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
        let expected = MCP_TOOL_NAMES.to_vec();
        assert_eq!(tool_registry_names(), expected);
    }
}
