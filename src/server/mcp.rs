use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use rmcp::{
    ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{
        Implementation, JsonObject, ProtocolVersion, ServerCapabilities, ServerInfo,
        ToolsCapability,
    },
    serde_json, tool, tool_handler, tool_router,
};
use tracing::instrument;

use crate::cli::agent_readiness_cmd::build_agent_readiness_tool_response;
use crate::cli::audit_cmd::build_metadata_audit_tool_response;
use crate::cli::config_cmd::{
    build_config_show_tool_response, build_config_validate_tool_response,
};
use crate::cli::eval_cmd::{
    build_agent_eval_tool_response, build_eval_gate_tool_response,
    build_eval_history_tool_response, build_eval_init_tool_response, build_eval_run_tool_response,
    build_eval_validate_tool_response,
};
use crate::cli::manifest::build_manifest_warm_tool_response;
use crate::cli::nova_meta_cmd::build_nova_meta_tool_response;
use crate::cli::storage_cmd::{
    build_storage_cleanup_tool_response, build_storage_inspect_tool_response,
    build_storage_prune_tool_response,
};
use crate::cli::trace_cmd::{
    build_trace_inspect_tool_response, build_trace_redact_tool_response,
    build_trace_replay_tool_response, build_trace_summarize_tool_response,
};
use crate::config::DbtNovaConfig;
use crate::error::DbtNovaError;
use crate::manifest::search::{ManifestSearch, ManifestSearchHandle};
use crate::params::{
    BatchGetParams, ColumnInventoryParams, CompareGrainsParams, ConfigShowParams,
    ConfigValidateParams, DiffEntitiesParams, ExecuteSqlParams, FindByPathParams,
    FindEntityOverlapParams, GetAgentReadinessParams, GetColumnLineageParams, GetColumnsParams,
    GetContextParams, GetEntityParams, GetEvalGateParams, GetEvalHistoryParams, GetImpactParams,
    GetLineageParams, GetMetadataAuditParams, GetMetadataScoreParams, GetRecipeParams,
    GetSqlParams, GetTestCoverageParams, GetUndocumentedParams, IndicatorInventoryParams,
    InitEvalSuiteParams, ListEntitiesParams, ModellingConsistencyReportParams, PaginationParams,
    ParentGroupMode, ReloadManifestParams, RunAgentEvalParams, RunEvalParams, RunRecipeParams,
    SearchColumnsParams, SearchIndicatorParams, SearchParams, SearchRecipesParams,
    StorageCleanupParams, StorageInspectParams, StoragePruneParams, TraceInspectParams,
    TraceRedactParams, TraceReplayParams, TraceSummarizeParams, ValidateDagParams,
    ValidateEvalSuiteParams, ValidateNovaMetaParams, WarmManifestParams,
};
use crate::responses::SuccessResponse;
use crate::server::health::build_manifest_health_payload;
use crate::utils::{ToolMetricsStore, ToolRateLimiter};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// Thin MCP server wrapper around the core `ManifestSearch`.
#[derive(Clone)]
pub struct DbtNovaServer {
    searcher: ManifestSearchHandle,
    tool_router: ToolRouter<Self>,
    metrics: Arc<ToolMetricsStore>,
    rate_limiter: Arc<OnceLock<Option<Arc<ToolRateLimiter>>>>,
    search_concurrency: Arc<OnceLock<Option<Arc<ConcurrencyLimiter>>>>,
    sql_concurrency: Arc<OnceLock<Option<Arc<ConcurrencyLimiter>>>>,
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for DbtNovaServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::default(),
            capabilities: ServerCapabilities {
                tools: Some(ToolsCapability::default()),
                ..ServerCapabilities::default()
            },
            server_info: Implementation {
                name: "dbt-nova".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                ..Implementation::default()
            },
            instructions: Some("DBT Manifest Search and Analysis MCP Server. Use 'search' for full-text discovery, 'search_indicator' for canonical measures and metrics, 'get_entity' for complete entity data, and 'execute_sql' to run warehouse queries with the configured SQL provider.".into()),
        }
    }
}

impl DbtNovaServer {
    /// Create a new MCP server wrapper for an existing search handle.
    #[must_use]
    pub fn new(searcher: ManifestSearchHandle) -> Self {
        let exposed_tools = crate::tools::catalog::MCP_TOOL_NAMES
            .iter()
            .map(|name| (*name).to_string())
            .collect();
        Self::new_with_exposed_tools(searcher, &exposed_tools)
    }

    /// Create a new MCP server wrapper and expose only the provided MCP tool names.
    #[must_use]
    pub fn new_with_exposed_tools(
        searcher: ManifestSearchHandle,
        exposed_tools: &BTreeSet<String>,
    ) -> Self {
        let mut tool_router = Self::tool_router();
        filter_tool_router(&mut tool_router, exposed_tools);
        if disable_tool_schemas() {
            tracing::info!("tool schemas disabled (DBT_NOVA_DISABLE_TOOL_SCHEMAS=1)");
            strip_tool_schemas(&mut tool_router);
        }
        Self {
            searcher,
            tool_router,
            metrics: Arc::new(ToolMetricsStore::default()),
            rate_limiter: Arc::new(OnceLock::new()),
            search_concurrency: Arc::new(OnceLock::new()),
            sql_concurrency: Arc::new(OnceLock::new()),
        }
    }

    fn serialization_error_response(err: &impl std::fmt::Display) -> String {
        serde_json::json!({
            "success": false,
            "error": format!("Serialization error: {err}"),
            "error_code": "SERVER_ERROR"
        })
        .to_string()
    }

    fn error_response(err: &DbtNovaError) -> String {
        serde_json::to_string(&err.to_response())
            .unwrap_or_else(|ser_err| Self::serialization_error_response(&ser_err))
    }

    fn serialize_budgeted_value(
        value: serde_json::Value,
        config: &DbtNovaConfig,
    ) -> std::result::Result<String, serde_json::Error> {
        Self::serialize_budgeted_value_with_pagination(value, config, None)
    }

    fn serialize_budgeted_value_with_pagination(
        value: serde_json::Value,
        config: &DbtNovaConfig,
        pagination: Option<&PaginationParams>,
    ) -> std::result::Result<String, serde_json::Error> {
        let mut budgeted = apply_mcp_response_budget(value, config);
        apply_mcp_next_offset_meta(&mut budgeted, config, pagination);
        serde_json::to_string(&budgeted)
    }

    fn permit_from_result(
        permit_result: Result<Option<ConcurrencyPermit>, DbtNovaError>,
    ) -> std::result::Result<Option<ConcurrencyPermit>, String> {
        permit_result.map_err(|err| Self::error_response(&err))
    }

    async fn acquire_sql_permit_for_tool(&self) -> Result<Option<ConcurrencyPermit>, DbtNovaError> {
        let Ok(searcher) = self.searcher.get().await else {
            return Ok(None);
        };
        let timeout = sql_queue_timeout(searcher.config());
        self.acquire_sql_permit(searcher.config(), timeout).await
    }

    async fn handle_async<F, Fut>(&self, tool: &'static str, persona: Option<&str>, f: F) -> String
    where
        F: FnOnce(Arc<ManifestSearch>) -> Fut,
        Fut: Future<Output = Result<serde_json::Value, DbtNovaError>>,
    {
        self.handle_async_result(tool, persona, |searcher| async move {
            f(searcher).await.map(|value| (value, None))
        })
        .await
    }

    async fn handle_async_paged<F, Fut>(
        &self,
        tool: &'static str,
        persona: Option<&str>,
        f: F,
    ) -> String
    where
        F: FnOnce(Arc<ManifestSearch>) -> Fut,
        Fut: Future<Output = Result<(serde_json::Value, PaginationParams), DbtNovaError>>,
    {
        self.handle_async_result(tool, persona, |searcher| async move {
            f(searcher)
                .await
                .map(|(value, pagination)| (value, Some(pagination)))
        })
        .await
    }

    async fn handle_async_result<F, Fut>(
        &self,
        tool: &'static str,
        persona: Option<&str>,
        f: F,
    ) -> String
    where
        F: FnOnce(Arc<ManifestSearch>) -> Fut,
        Fut: Future<Output = Result<(serde_json::Value, Option<PaginationParams>), DbtNovaError>>,
    {
        let start = Instant::now();
        let mut success = false;
        let searcher = match self.searcher.get().await {
            Ok(searcher) => searcher,
            Err(err) => {
                let out = Self::error_response(&err);
                let duration_ms = elapsed_ms_to_u64(start.elapsed());
                let parsed_trace_response = serde_json::from_str::<serde_json::Value>(&out).ok();
                crate::utils::tool_trace::record_tool_call(
                    "mcp",
                    tool,
                    None,
                    parsed_trace_response.as_ref(),
                    success,
                    duration_ms,
                );
                self.record_metrics(tool, persona, duration_ms, success);
                return out;
            }
        };

        if !self.check_rate_limit(tool, searcher.config()) {
            let out = serde_json::json!({
                "success": false,
                "error": format!("Rate limit exceeded for tool '{}'", tool),
                "error_code": "RATE_LIMITED",
            })
            .to_string();
            let duration_ms = elapsed_ms_to_u64(start.elapsed());
            let parsed_trace_response = serde_json::from_str::<serde_json::Value>(&out).ok();
            crate::utils::tool_trace::record_tool_call(
                "mcp",
                tool,
                None,
                parsed_trace_response.as_ref(),
                success,
                duration_ms,
            );
            self.record_metrics(tool, persona, duration_ms, success);
            return out;
        }

        let config = searcher.config().clone();
        let out = match f(searcher).await {
            Ok((v, pagination)) => match Self::serialize_budgeted_value_with_pagination(
                v,
                &config,
                pagination.as_ref(),
            ) {
                Ok(out) => {
                    success = true;
                    out
                }
                Err(e) => Self::serialization_error_response(&e),
            },
            Err(e) => Self::error_response(&e),
        };
        let duration_ms = elapsed_ms_to_u64(start.elapsed());
        let parsed_trace_response = serde_json::from_str::<serde_json::Value>(&out).ok();
        crate::utils::tool_trace::record_tool_call(
            "mcp",
            tool,
            None,
            parsed_trace_response.as_ref(),
            success,
            duration_ms,
        );
        self.record_metrics(tool, persona, duration_ms, success);
        out
    }
}

fn apply_mcp_response_budget(
    mut value: serde_json::Value,
    config: &DbtNovaConfig,
) -> serde_json::Value {
    let budget = config.mcp_max_response_bytes;
    if budget == 0 {
        return value;
    }
    let initial_bytes = serialized_len(&value);
    if initial_bytes <= budget {
        return value;
    }
    let original_shape = ResponseShapeSnapshot::capture(&value);

    let mut omitted_paths = Vec::new();
    truncate_json_for_budget(
        &mut value,
        "$".to_string(),
        config.mcp_max_string_chars.max(1),
        50,
        50,
        &mut omitted_paths,
    );
    if serialized_len(&value) > budget {
        truncate_json_for_budget(
            &mut value,
            "$".to_string(),
            1024,
            20,
            20,
            &mut omitted_paths,
        );
    }
    if serialized_len(&value) > budget {
        truncate_json_for_budget(&mut value, "$".to_string(), 512, 5, 5, &mut omitted_paths);
    }
    normalize_budgeted_response_shape(&mut value, &original_shape, !omitted_paths.is_empty());
    finalize_mcp_budgeted_response(
        value,
        budget,
        config.mcp_include_truncation_meta,
        omitted_paths,
        original_shape.total_count_hint,
    )
}

fn serialized_len(value: &serde_json::Value) -> usize {
    serde_json::to_string(value).map_or(0, |serialized| serialized.len())
}

fn attach_result_meta(
    value: &mut serde_json::Value,
    response_bytes: usize,
    budget_bytes: usize,
    truncated: bool,
    mut omitted_paths: Vec<String>,
    original_count: Option<u64>,
) {
    omitted_paths.sort();
    omitted_paths.dedup();
    let mut meta = serde_json::json!({
        "response_bytes": response_bytes,
        "budget_bytes": budget_bytes,
        "truncated": truncated,
        "omitted_paths": omitted_paths
    });
    if let Some(original_count) = original_count
        && let Some(obj) = meta.as_object_mut()
    {
        obj.insert(
            "original_count".to_string(),
            serde_json::Value::from(original_count),
        );
    }
    if let Some(obj) = value.as_object_mut() {
        obj.insert("_nova_result_meta".to_string(), meta);
    }
}

fn update_result_meta_response_bytes(value: &mut serde_json::Value) {
    let response_bytes = serialized_len(value);
    if let Some(obj) = value.as_object_mut()
        && let Some(meta) = obj
            .get_mut("_nova_result_meta")
            .and_then(serde_json::Value::as_object_mut)
    {
        meta.insert(
            "response_bytes".to_string(),
            serde_json::Value::from(response_bytes),
        );
    }
}

fn apply_mcp_next_offset_meta(
    value: &mut serde_json::Value,
    config: &DbtNovaConfig,
    pagination: Option<&PaginationParams>,
) {
    if !config.mcp_include_truncation_meta {
        return;
    }
    let Some(pagination) = pagination else {
        return;
    };
    let total_available = value
        .get("total_available")
        .and_then(serde_json::Value::as_u64);
    let count = value.get("count").and_then(serde_json::Value::as_u64);
    let (Some(total_available), Some(count)) = (total_available, count) else {
        return;
    };
    if count == 0 {
        return;
    }
    let Some(offset) = u64::try_from(pagination.offset).ok() else {
        return;
    };
    let next_offset = offset.saturating_add(count);
    if next_offset >= total_available {
        return;
    }
    let truncated = response_is_truncated(value);

    let Some(obj) = value.as_object_mut() else {
        return;
    };
    let had_result_meta = obj.contains_key("_nova_result_meta");
    let meta = obj
        .entry("_nova_result_meta".to_string())
        .or_insert_with(|| serde_json::json!({}));
    let Some(meta) = meta.as_object_mut() else {
        return;
    };
    meta.insert(
        "next_offset".to_string(),
        serde_json::Value::from(next_offset),
    );
    meta.entry("truncated".to_string())
        .or_insert_with(|| serde_json::Value::from(truncated));
    update_result_meta_response_bytes(value);
    let mut refresh_response_bytes = false;
    if config.mcp_max_response_bytes > 0
        && serialized_len(value) > config.mcp_max_response_bytes
        && let Some(obj) = value.as_object_mut()
    {
        if had_result_meta {
            if let Some(meta) = obj
                .get_mut("_nova_result_meta")
                .and_then(serde_json::Value::as_object_mut)
            {
                meta.remove("next_offset");
            }
            refresh_response_bytes = true;
        } else {
            obj.remove("_nova_result_meta");
        }
    }
    if refresh_response_bytes {
        update_result_meta_response_bytes(value);
    }
}

fn response_is_truncated(value: &serde_json::Value) -> bool {
    value
        .get("truncated")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
        || value
            .get("_nova_result_meta")
            .and_then(|meta| meta.get("truncated"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
}

fn finalize_mcp_budgeted_response(
    value: serde_json::Value,
    budget: usize,
    include_meta: bool,
    mut omitted_paths: Vec<String>,
    original_count: Option<u64>,
) -> serde_json::Value {
    if serialized_len(&value) <= budget {
        if !include_meta {
            return value;
        }
        let mut with_meta = value.clone();
        let response_bytes = serialized_len(&with_meta);
        attach_result_meta(
            &mut with_meta,
            response_bytes,
            budget,
            true,
            omitted_paths.clone(),
            original_count,
        );
        trim_result_meta_paths_for_budget(&mut with_meta, budget);
        update_result_meta_response_bytes(&mut with_meta);
        if serialized_len(&with_meta) <= budget {
            return with_meta;
        }
        return value;
    }

    omitted_paths.push("$".to_string());
    let mut fallback = compact_truncated_response(&value);
    if include_meta {
        let response_bytes = serialized_len(&fallback);
        attach_result_meta(
            &mut fallback,
            response_bytes,
            budget,
            true,
            omitted_paths,
            original_count,
        );
        trim_result_meta_for_budget(&mut fallback, budget);
        update_result_meta_response_bytes(&mut fallback);
    }
    fallback
}

fn trim_result_meta_paths_for_budget(value: &mut serde_json::Value, budget: usize) {
    if serialized_len(value) <= budget {
        return;
    }
    if let Some(obj) = value.as_object_mut()
        && let Some(meta) = obj
            .get_mut("_nova_result_meta")
            .and_then(serde_json::Value::as_object_mut)
    {
        meta.insert("omitted_paths".to_string(), serde_json::json!(["$"]));
    }
}

fn compact_truncated_response(value: &serde_json::Value) -> serde_json::Value {
    let mut obj = serde_json::Map::new();
    if let Some(source) = value.as_object() {
        for key in ["success", "total_available", "persona"] {
            if let Some(child) = source.get(key) {
                obj.insert(key.to_string(), child.clone());
            }
        }
        match source.get("data") {
            Some(serde_json::Value::Array(_)) => {
                obj.insert("count".to_string(), serde_json::Value::from(0));
                obj.insert("data".to_string(), serde_json::Value::Array(Vec::new()));
            }
            Some(serde_json::Value::Object(data)) if data.contains_key("rows") => {
                obj.insert("count".to_string(), serde_json::Value::from(0));
                obj.insert(
                    "data".to_string(),
                    serde_json::json!({
                        "rows": [],
                        "truncated": true
                    }),
                );
            }
            Some(serde_json::Value::Object(data)) if data_contains_collection_payload(data) => {
                compact_collection_payload(data, &mut obj);
            }
            Some(serde_json::Value::Object(_)) => {
                if let Some(count) = source.get("count") {
                    obj.insert("count".to_string(), count.clone());
                }
                obj.insert("data".to_string(), serde_json::json!({"_truncated": true}));
            }
            Some(serde_json::Value::String(_)) => {
                if let Some(count) = source.get("count") {
                    obj.insert("count".to_string(), count.clone());
                }
                obj.insert(
                    "data".to_string(),
                    serde_json::Value::String("[truncated]".to_string()),
                );
            }
            Some(
                serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_),
            )
            | None => {
                if let Some(count) = source.get("count") {
                    obj.insert("count".to_string(), count.clone());
                }
                obj.insert("data".to_string(), serde_json::Value::Null);
            }
        }
    } else {
        obj.insert("data".to_string(), serde_json::Value::Null);
    }
    obj.insert("truncated".to_string(), serde_json::Value::from(true));
    serde_json::Value::Object(obj)
}

fn data_contains_collection_payload(data: &serde_json::Map<String, serde_json::Value>) -> bool {
    [
        "columns",
        "entities",
        "lineage",
        "edges",
        "not_found",
        "undocumented_columns",
    ]
    .iter()
    .any(|key| data.get(*key).is_some_and(serde_json::Value::is_array))
}

fn compact_collection_payload(
    data: &serde_json::Map<String, serde_json::Value>,
    obj: &mut serde_json::Map<String, serde_json::Value>,
) {
    let mut compact = serde_json::Map::new();

    for key in [
        "columns",
        "entities",
        "lineage",
        "edges",
        "not_found",
        "undocumented_columns",
    ] {
        if data.get(key).is_some_and(serde_json::Value::is_array) {
            compact.insert(key.to_string(), serde_json::Value::Array(Vec::new()));
        }
    }
    for key in ["found_count", "not_found_count"] {
        if data.contains_key(key) {
            compact.insert(key.to_string(), serde_json::Value::from(0));
        }
    }
    if let Some(summary) = data
        .get("summary")
        .and_then(serde_json::Value::as_object)
        .filter(|summary| {
            ["entities_returned", "columns_returned", "items_returned"]
                .iter()
                .any(|key| summary.contains_key(*key))
        })
    {
        let mut compact_summary = summary.clone();
        for key in ["entities_returned", "columns_returned", "items_returned"] {
            if compact_summary.contains_key(key) {
                compact_summary.insert(key.to_string(), serde_json::Value::from(0));
            }
        }
        compact.insert(
            "summary".to_string(),
            serde_json::Value::Object(compact_summary),
        );
    }

    compact.insert("truncated".to_string(), serde_json::Value::from(true));
    obj.insert("count".to_string(), serde_json::Value::from(0));
    obj.insert("data".to_string(), serde_json::Value::Object(compact));
}

fn trim_result_meta_for_budget(value: &mut serde_json::Value, budget: usize) {
    trim_result_meta_paths_for_budget(value, budget);
    if serialized_len(value) <= budget {
        return;
    }
    if let Some(obj) = value.as_object_mut() {
        obj.retain(|key, _| key == "_nova_result_meta");
    }
}

fn result_count_hint(value: &serde_json::Value) -> Option<u64> {
    value
        .get("total_available")
        .or_else(|| value.get("count"))
        .and_then(serde_json::Value::as_u64)
}

struct ResponseShapeSnapshot {
    count: Option<u64>,
    total_count_hint: Option<u64>,
    data_rows_len: Option<u64>,
    data_object_array_lens: BTreeMap<String, u64>,
}

impl ResponseShapeSnapshot {
    fn capture(value: &serde_json::Value) -> Self {
        let mut data_object_array_lens = BTreeMap::new();
        let data_rows_len = value
            .get("data")
            .and_then(serde_json::Value::as_object)
            .and_then(|data| {
                for (key, child) in data {
                    if let Some(len) = array_len(Some(child)) {
                        data_object_array_lens.insert(key.clone(), len);
                    }
                }
                data.get("rows")
            })
            .and_then(|rows| array_len(Some(rows)));

        Self {
            count: value.get("count").and_then(serde_json::Value::as_u64),
            total_count_hint: result_count_hint(value),
            data_rows_len,
            data_object_array_lens,
        }
    }

    fn original_data_array_len(&self, key: &str) -> Option<u64> {
        self.data_object_array_lens.get(key).copied()
    }
}

fn array_len(value: Option<&serde_json::Value>) -> Option<u64> {
    value
        .and_then(serde_json::Value::as_array)
        .and_then(|items| u64::try_from(items.len()).ok())
}

fn normalize_budgeted_response_shape(
    value: &mut serde_json::Value,
    original_shape: &ResponseShapeSnapshot,
    budget_truncated: bool,
) {
    let Some(obj) = value.as_object_mut() else {
        return;
    };
    if budget_truncated {
        obj.insert("truncated".to_string(), serde_json::Value::from(true));
    }
    if let Some(returned_count) = array_len(obj.get("data")) {
        normalize_budgeted_count(obj, returned_count, original_shape.total_count_hint);
        return;
    }

    let Some(data) = obj
        .get_mut("data")
        .and_then(serde_json::Value::as_object_mut)
    else {
        return;
    };

    let mut top_count = None;
    let mut data_truncated = false;

    if let Some(returned_count) = array_len(data.get("rows")) {
        top_count = Some(returned_count);
        data_truncated |= original_shape
            .data_rows_len
            .is_some_and(|original_len| original_len > returned_count);
        data.insert("truncated".to_string(), serde_json::Value::from(true));
    }

    let entities_returned = array_len(data.get("entities"));
    if let Some(returned_count) = entities_returned {
        update_data_returned_count(data, "found_count", returned_count);
        data_truncated |= original_shape
            .original_data_array_len("entities")
            .is_some_and(|original_len| original_len > returned_count);
        if original_shape
            .original_data_array_len("entities")
            .is_some_and(|original_len| original_shape.count == Some(original_len))
        {
            top_count = Some(returned_count);
        }
    }

    if let Some(returned_count) = array_len(data.get("lineage")) {
        data_truncated |= original_shape
            .original_data_array_len("lineage")
            .is_some_and(|original_len| original_len > returned_count);
        if original_shape
            .original_data_array_len("lineage")
            .is_some_and(|original_len| original_shape.count == Some(original_len))
        {
            top_count = Some(returned_count);
        }
    }

    if let Some(returned_count) = array_len(data.get("not_found")) {
        update_data_returned_count(data, "not_found_count", returned_count);
        data_truncated |= original_shape
            .original_data_array_len("not_found")
            .is_some_and(|original_len| original_len > returned_count);
    }

    if let Some(returned_count) = array_len(data.get("edges")) {
        data_truncated |= original_shape
            .original_data_array_len("edges")
            .is_some_and(|original_len| original_len > returned_count);
    }

    if !data.contains_key("rows")
        && let Some(returned_count) = array_len(data.get("columns"))
    {
        data_truncated |= original_shape
            .original_data_array_len("columns")
            .is_some_and(|original_len| original_len > returned_count);
        if original_shape
            .original_data_array_len("columns")
            .is_some_and(|original_len| original_shape.count == Some(original_len))
        {
            top_count = Some(returned_count);
        }
    }

    if normalize_undocumented_summary(data) {
        top_count = data
            .get("summary")
            .and_then(|summary| summary.get("items_returned"))
            .and_then(serde_json::Value::as_u64);
    }

    if data_truncated || budget_truncated {
        data.insert("truncated".to_string(), serde_json::Value::from(true));
    }

    let Some(returned_count) = top_count else {
        return;
    };
    normalize_budgeted_count(obj, returned_count, original_shape.total_count_hint);
}

fn update_data_returned_count(
    data: &mut serde_json::Map<String, serde_json::Value>,
    key: &str,
    returned_count: u64,
) {
    if data.contains_key(key) {
        data.insert(key.to_string(), serde_json::Value::from(returned_count));
    }
}

fn normalize_undocumented_summary(data: &mut serde_json::Map<String, serde_json::Value>) -> bool {
    let entities_returned = array_len(data.get("entities"));
    let columns_returned = array_len(data.get("undocumented_columns"));
    let Some(summary) = data
        .get_mut("summary")
        .and_then(serde_json::Value::as_object_mut)
    else {
        return false;
    };
    if !["entities_returned", "columns_returned", "items_returned"]
        .iter()
        .any(|key| summary.contains_key(*key))
    {
        return false;
    }

    if let Some(entities_returned) = entities_returned
        && summary.contains_key("entities_returned")
    {
        summary.insert(
            "entities_returned".to_string(),
            serde_json::Value::from(entities_returned),
        );
    }
    if let Some(columns_returned) = columns_returned
        && summary.contains_key("columns_returned")
    {
        summary.insert(
            "columns_returned".to_string(),
            serde_json::Value::from(columns_returned),
        );
    }
    let items_returned = entities_returned
        .unwrap_or_else(|| {
            summary
                .get("entities_returned")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0)
        })
        .saturating_add(columns_returned.unwrap_or_else(|| {
            summary
                .get("columns_returned")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0)
        }));
    if summary.contains_key("items_returned") {
        summary.insert(
            "items_returned".to_string(),
            serde_json::Value::from(items_returned),
        );
    }
    true
}

fn normalize_budgeted_count(
    obj: &mut serde_json::Map<String, serde_json::Value>,
    returned_count: u64,
    original_count: Option<u64>,
) -> bool {
    let count_changed = obj
        .get("count")
        .and_then(serde_json::Value::as_u64)
        .is_some_and(|count| count != returned_count);
    if count_changed {
        obj.insert("count".to_string(), serde_json::Value::from(returned_count));
    }
    if original_count.is_some_and(|count| count > returned_count) || count_changed {
        obj.insert("truncated".to_string(), serde_json::Value::from(true));
        true
    } else {
        false
    }
}

fn truncate_json_for_budget(
    value: &mut serde_json::Value,
    path: String,
    max_string_chars: usize,
    max_array_items: usize,
    max_object_entries: usize,
    omitted_paths: &mut Vec<String>,
) {
    match value {
        serde_json::Value::String(text) => {
            if text.chars().count() > max_string_chars {
                let truncated: String = text.chars().take(max_string_chars).collect();
                *text = format!("{truncated}...[truncated]");
                omitted_paths.push(path);
            }
        }
        serde_json::Value::Array(items) => {
            if items.len() > max_array_items {
                items.truncate(max_array_items);
                omitted_paths.push(path.clone());
            }
            for (idx, item) in items.iter_mut().enumerate() {
                truncate_json_for_budget(
                    item,
                    format!("{path}.{idx}"),
                    max_string_chars,
                    max_array_items,
                    max_object_entries,
                    omitted_paths,
                );
            }
        }
        serde_json::Value::Object(obj) => {
            if path != "$" && obj.len() > max_object_entries {
                prune_object_entries(obj, &path, max_object_entries, omitted_paths);
            }
            for noisy_key in ["block_contents", "compiled_code", "raw_code", "sql"] {
                if let Some(child) = obj.get_mut(noisy_key) {
                    truncate_json_for_budget(
                        child,
                        format!("{path}.{noisy_key}"),
                        max_string_chars.min(2048),
                        max_array_items,
                        max_object_entries,
                        omitted_paths,
                    );
                }
            }
            for (key, child) in obj {
                if matches!(
                    key.as_str(),
                    "block_contents" | "compiled_code" | "raw_code" | "sql"
                ) {
                    continue;
                }
                truncate_json_for_budget(
                    child,
                    format!("{path}.{key}"),
                    max_string_chars,
                    max_array_items,
                    max_object_entries,
                    omitted_paths,
                );
            }
        }
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => {}
    }
}

fn prune_object_entries(
    obj: &mut serde_json::Map<String, serde_json::Value>,
    path: &str,
    max_object_entries: usize,
    omitted_paths: &mut Vec<String>,
) {
    if max_object_entries == 0 {
        obj.clear();
        omitted_paths.push(format!("{path}.*"));
        return;
    }
    let important_keys = [
        "rows",
        "columns",
        "column_types",
        "truncated",
        "stats",
        "state",
        "provider",
        "success",
        "count",
        "total_available",
        "data",
        "parent_groups",
        "unique_id",
        "parent_unique_id",
        "name",
        "resource_type",
        "relation_name",
        "grain",
        "expression",
        "indicator_name",
        "indicator_type",
    ];
    let mut keep: BTreeSet<String> = important_keys
        .iter()
        .filter(|key| obj.contains_key(**key))
        .take(max_object_entries)
        .map(|key| (*key).to_string())
        .collect();
    for key in obj.keys() {
        if keep.len() >= max_object_entries {
            break;
        }
        keep.insert(key.clone());
    }
    if keep.len() < obj.len() {
        let remove_keys: Vec<String> = obj
            .keys()
            .filter(|key| !keep.contains(*key))
            .cloned()
            .collect();
        for key in remove_keys {
            obj.remove(&key);
        }
        omitted_paths.push(format!("{path}.*"));
    }
}

impl DbtNovaServer {
    fn record_metrics(&self, tool: &str, persona: Option<&str>, duration_ms: u64, success: bool) {
        self.metrics.record(tool, duration_ms, success);
        if let Some(persona) = persona {
            let key = format!("{}.{}", tool, persona.to_lowercase());
            self.metrics.record(&key, duration_ms, success);
        }
    }

    fn check_rate_limit(&self, tool: &str, config: &crate::config::DbtNovaConfig) -> bool {
        let limiter = self.rate_limiter.get_or_init(|| {
            if config.tool_rate_limits.trim().is_empty() {
                return None;
            }
            let (limits, default_limit) = ToolRateLimiter::parse_limits(&config.tool_rate_limits);
            if limits.is_empty() && default_limit.is_none() {
                return None;
            }
            Some(Arc::new(ToolRateLimiter::new(
                Duration::from_secs(config.tool_rate_limit_window_secs.max(1)),
                limits,
                default_limit,
            )))
        });
        limiter.as_ref().is_none_or(|rl| rl.allow(tool))
    }

    async fn acquire_search_permit(
        &self,
        config: &crate::config::DbtNovaConfig,
        timeout: Option<Duration>,
    ) -> Result<Option<ConcurrencyPermit>, DbtNovaError> {
        let concurrency = self
            .search_concurrency
            .get_or_init(|| ConcurrencyLimiter::for_search(config).map(Arc::new));
        let Some(concurrency) = concurrency.as_ref() else {
            return Ok(None);
        };
        concurrency.acquire(timeout).await
    }

    async fn acquire_sql_permit(
        &self,
        config: &crate::config::DbtNovaConfig,
        timeout: Option<Duration>,
    ) -> Result<Option<ConcurrencyPermit>, DbtNovaError> {
        let concurrency = self
            .sql_concurrency
            .get_or_init(|| ConcurrencyLimiter::for_sql(config).map(Arc::new));
        let Some(concurrency) = concurrency.as_ref() else {
            return Ok(None);
        };
        concurrency.acquire(timeout).await
    }

    async fn handle_bounded_search<F, Fut>(
        &self,
        tool: &'static str,
        persona: Option<&str>,
        f: F,
    ) -> String
    where
        F: FnOnce(Arc<ManifestSearch>) -> Fut,
        Fut: Future<Output = Result<serde_json::Value, DbtNovaError>>,
    {
        self.handle_bounded_search_result(tool, persona, |searcher| async move {
            f(searcher).await.map(|value| (value, None))
        })
        .await
    }

    async fn handle_bounded_search_paged<F, Fut>(
        &self,
        tool: &'static str,
        persona: Option<&str>,
        f: F,
    ) -> String
    where
        F: FnOnce(Arc<ManifestSearch>) -> Fut,
        Fut: Future<Output = Result<(serde_json::Value, PaginationParams), DbtNovaError>>,
    {
        self.handle_bounded_search_result(tool, persona, |searcher| async move {
            f(searcher)
                .await
                .map(|(value, pagination)| (value, Some(pagination)))
        })
        .await
    }

    async fn handle_bounded_search_result<F, Fut>(
        &self,
        tool: &'static str,
        persona: Option<&str>,
        f: F,
    ) -> String
    where
        F: FnOnce(Arc<ManifestSearch>) -> Fut,
        Fut: Future<Output = Result<(serde_json::Value, Option<PaginationParams>), DbtNovaError>>,
    {
        let search_started = Instant::now();
        let search_timeout_ms = match self.searcher.get().await {
            Ok(searcher) => searcher.config().search.search_timeout_ms as u64,
            Err(_) => 0,
        };
        let permit_timeout = remaining_timeout(search_started, search_timeout_ms);
        let permit_result = if let Ok(searcher) = self.searcher.get().await {
            self.acquire_search_permit(searcher.config(), permit_timeout)
                .await
        } else {
            Ok(None)
        };
        let permit = match Self::permit_from_result(permit_result) {
            Ok(permit) => permit,
            Err(response) => return response,
        };
        let result = self
            .handle_async_result(tool, persona, |searcher| async move {
                let future = f(searcher);
                if search_timeout_ms == 0 {
                    return future.await;
                }
                let Some(remaining) = remaining_timeout(search_started, search_timeout_ms) else {
                    return Err(search_timeout_error(search_timeout_ms));
                };
                if remaining.is_zero() {
                    return Err(search_timeout_error(search_timeout_ms));
                }
                match tokio::time::timeout(remaining, future).await {
                    Ok(result) => result,
                    Err(_) => Err(search_timeout_error(search_timeout_ms)),
                }
            })
            .await;
        drop(permit);
        result
    }
}

#[derive(Debug, Clone, Copy)]
struct ConcurrencyLabels {
    timeout_prefix: &'static str,
    queue_full: &'static str,
    limit_exceeded: &'static str,
    semaphore_closed: &'static str,
}

const SEARCH_CONCURRENCY_LABELS: ConcurrencyLabels = ConcurrencyLabels {
    timeout_prefix: "Search timed out while waiting for an execution slot after",
    queue_full: "Search queue is full; retry later",
    limit_exceeded: "Search concurrency limit exceeded",
    semaphore_closed: "Search concurrency semaphore closed",
};

const SQL_CONCURRENCY_LABELS: ConcurrencyLabels = ConcurrencyLabels {
    timeout_prefix: "SQL execution timed out while waiting for an execution slot after",
    queue_full: "SQL execution queue is full; retry later",
    limit_exceeded: "SQL concurrency limit exceeded",
    semaphore_closed: "SQL concurrency semaphore closed",
};

#[derive(Debug, Clone)]
struct ConcurrencyLimiter {
    slots: Arc<Semaphore>,
    queue: Option<Arc<Semaphore>>,
    max_slots: usize,
    max_queue: usize,
    labels: ConcurrencyLabels,
}

impl ConcurrencyLimiter {
    fn new(max_concurrent: usize, max_queue: usize, labels: ConcurrencyLabels) -> Option<Self> {
        if max_concurrent == 0 {
            return None;
        }
        let slots = Arc::new(Semaphore::new(max_concurrent));
        let queue = if max_queue > 0 {
            Some(Arc::new(Semaphore::new(max_queue)))
        } else {
            None
        };
        Some(Self {
            slots,
            queue,
            max_slots: max_concurrent,
            max_queue,
            labels,
        })
    }

    fn for_search(config: &crate::config::DbtNovaConfig) -> Option<Self> {
        Self::new(
            config.search.search_max_concurrent,
            config.search.search_max_queue,
            SEARCH_CONCURRENCY_LABELS,
        )
    }

    fn for_sql(config: &crate::config::DbtNovaConfig) -> Option<Self> {
        Self::new(
            config.sql_max_concurrent,
            config.sql_max_queue,
            SQL_CONCURRENCY_LABELS,
        )
    }

    async fn acquire(
        &self,
        timeout: Option<Duration>,
    ) -> Result<Option<ConcurrencyPermit>, DbtNovaError> {
        if let Ok(slot) = self.slots.clone().try_acquire_owned() {
            return Ok(Some(ConcurrencyPermit { slot, queue: None }));
        }
        if let Some(queue) = &self.queue {
            match queue.clone().try_acquire_owned() {
                Ok(queue_permit) => {
                    let slot = if let Some(timeout) = timeout {
                        match tokio::time::timeout(timeout, self.slots.clone().acquire_owned())
                            .await
                        {
                            Ok(result) => result.map_err(|_| {
                                DbtNovaError::ServerError(self.labels.semaphore_closed.into())
                            })?,
                            Err(_) => {
                                return Err(DbtNovaError::ServerError(format!(
                                    "{} {}ms",
                                    self.labels.timeout_prefix,
                                    timeout.as_millis()
                                )));
                            }
                        }
                    } else {
                        self.slots.clone().acquire_owned().await.map_err(|_| {
                            DbtNovaError::ServerError(self.labels.semaphore_closed.into())
                        })?
                    };
                    return Ok(Some(ConcurrencyPermit {
                        slot,
                        queue: Some(queue_permit),
                    }));
                }
                Err(_) => {
                    return Err(DbtNovaError::ServerError(
                        self.labels.queue_full.to_string(),
                    ));
                }
            }
        }
        Err(DbtNovaError::ServerError(
            self.labels.limit_exceeded.to_string(),
        ))
    }

    fn snapshot(&self) -> serde_json::Value {
        let available_slots = self.slots.available_permits();
        let in_flight = self.max_slots.saturating_sub(available_slots);
        let available_queue = self
            .queue
            .as_ref()
            .map_or(0usize, |queue| queue.available_permits());
        let queued = self.max_queue.saturating_sub(available_queue);
        let saturated = available_slots == 0;
        let queue_saturated = self.max_queue > 0 && saturated && available_queue == 0;

        serde_json::json!({
            "enabled": true,
            "max_concurrent": self.max_slots,
            "available_slots": available_slots,
            "in_flight": in_flight,
            "saturated": saturated,
            "max_queue": self.max_queue,
            "available_queue": available_queue,
            "queued": queued,
            "queue_saturated": queue_saturated,
        })
    }
}

#[derive(Debug)]
struct ConcurrencyPermit {
    // Fields are intentionally unused: holding the permits enforces concurrency limits (RAII).
    #[allow(dead_code)]
    slot: OwnedSemaphorePermit,
    #[allow(dead_code)]
    queue: Option<OwnedSemaphorePermit>,
}

fn disable_tool_schemas() -> bool {
    match std::env::var("DBT_NOVA_DISABLE_TOOL_SCHEMAS") {
        Ok(v) => {
            let v = v.trim().to_lowercase();
            !(v.is_empty() || v == "0" || v == "false" || v == "no")
        }
        Err(_) => false,
    }
}

fn strip_tool_schemas(tool_router: &mut ToolRouter<DbtNovaServer>) {
    let mut empty_schema: JsonObject = serde_json::Map::new();
    empty_schema.insert(
        "type".to_string(),
        serde_json::Value::String("object".to_string()),
    );
    empty_schema.insert(
        "properties".to_string(),
        serde_json::Value::Object(serde_json::Map::new()),
    );
    let empty_schema = std::sync::Arc::new(empty_schema);

    for route in tool_router.map.values_mut() {
        route.attr.input_schema = empty_schema.clone();
        route.attr.output_schema = None;
    }
}

fn filter_tool_router(
    tool_router: &mut ToolRouter<DbtNovaServer>,
    exposed_tools: &BTreeSet<String>,
) {
    tool_router
        .map
        .retain(|tool_name, _| exposed_tools.contains(tool_name.as_ref()));
}

fn apply_mcp_pagination_defaults(pagination: &mut PaginationParams, config: &DbtNovaConfig) {
    let default_limit = config.mcp_default_limit.max(1);
    let requested = pagination.limit.unwrap_or(0);
    let mut effective = if requested == 0 {
        default_limit
    } else {
        requested
    };
    if config.mcp_max_page_size > 0 {
        effective = effective.min(config.mcp_max_page_size);
    }
    pagination.limit = Some(effective);
}

fn apply_mcp_metadata_score_pagination_defaults(
    params: &mut GetMetadataScoreParams,
    config: &DbtNovaConfig,
) -> PaginationParams {
    let mut pagination = PaginationParams {
        limit: params.limit,
        offset: params.offset.unwrap_or(0),
    };
    apply_mcp_pagination_defaults(&mut pagination, config);
    params.limit = pagination.limit;
    params.offset = Some(pagination.offset);
    pagination
}

fn apply_mcp_detail_default(
    detail: &mut Option<crate::params::DetailLevel>,
    config: &DbtNovaConfig,
) {
    if detail.is_none() {
        *detail = Some(config.mcp_result_profile.detail_level());
    }
}

fn apply_mcp_search_defaults(params: &mut SearchParams, config: &DbtNovaConfig) {
    apply_mcp_detail_default(&mut params.detail, config);
    apply_mcp_pagination_defaults(&mut params.pagination, config);
}

fn apply_mcp_search_indicator_defaults(params: &mut SearchIndicatorParams, config: &DbtNovaConfig) {
    let detail_was_omitted = params.detail.is_none();
    apply_mcp_detail_default(&mut params.detail, config);
    apply_mcp_pagination_defaults(&mut params.pagination, config);
    if detail_was_omitted
        && config.mcp_result_profile == crate::config::ResultProfile::Compact
        && params.group_mode.is_none()
    {
        params.group_mode = Some(ParentGroupMode::Top);
        params.max_parent_groups = Some(params.max_parent_groups.unwrap_or(1).min(1));
    }
}

fn apply_mcp_entity_detail_default(
    detail: &mut Option<crate::params::DetailLevel>,
    config: &DbtNovaConfig,
) {
    apply_mcp_detail_default(detail, config);
}

#[tool_router]
impl DbtNovaServer {
    /// Full-text search across all entities. Searches names, descriptions, SQL code, file paths, column names, and tags.
    #[tool(
        name = "search",
        description = "Primary discovery tool. Use this to find entities by business terms, names, columns, tags, file paths, or SQL snippets. Start here when you don’t know the unique_id. Supports boolean operators and phrase queries. Returns ranked matches and highlights when enabled."
    )]
    #[instrument(level = "info", skip(self, params))]
    async fn search(&self, params: Parameters<SearchParams>) -> String {
        let persona = params.0.persona.clone();
        self.handle_bounded_search_paged("search", persona.as_deref(), |searcher| async move {
            let mut params = params.0;
            apply_mcp_search_defaults(&mut params, searcher.config());
            let pagination = params.pagination;
            searcher
                .search(&params)
                .await
                .map(|value| (value, pagination))
        })
        .await
    }

    /// Resolve Nova measures and metrics that match the query.
    #[tool(
        name = "search_indicator",
        description = "Analyst-focused semantic discovery. Search Nova measures and metrics by business term, synonym, field, description, or expression, and return the parent execution entity, grain, and match evidence."
    )]
    #[instrument(level = "info", skip(self, params))]
    async fn search_indicator(&self, params: Parameters<SearchIndicatorParams>) -> String {
        let persona = params.0.persona.clone();
        self.handle_bounded_search_paged(
            "search_indicator",
            persona.as_deref(),
            |searcher| async move {
                let mut params = params.0;
                apply_mcp_search_indicator_defaults(&mut params, searcher.config());
                let pagination = params.pagination;
                searcher
                    .search_indicator(&params)
                    .await
                    .map(|value| (value, pagination))
            },
        )
        .await
    }

    /// List Nova measures and metrics deterministically with parent execution context.
    #[tool(
        name = "indicator_inventory",
        description = "Inventory tool for Nova measures and metrics. List canonical or all indicators with parent entity, grain, domains, and core definitions when you need a deterministic semantic catalog."
    )]
    #[instrument(level = "info", skip(self, params))]
    async fn indicator_inventory(&self, params: Parameters<IndicatorInventoryParams>) -> String {
        self.handle_bounded_search_paged("indicator_inventory", None, |searcher| async move {
            let mut params = params.0;
            apply_mcp_pagination_defaults(&mut params.pagination, searcher.config());
            let pagination = params.pagination;
            searcher
                .indicator_inventory(&params)
                .await
                .map(|value| (value, pagination))
        })
        .await
    }

    /// Search columns by semantic metadata and business hints.
    #[tool(
        name = "search_columns",
        description = "Search columns by name, synonym, description, role, semantic_type, or example_values, and return the parent entity context for downstream modelling or analytics work."
    )]
    #[instrument(level = "info", skip(self, params))]
    async fn search_columns(&self, params: Parameters<SearchColumnsParams>) -> String {
        self.handle_bounded_search_paged("search_columns", None, |searcher| async move {
            let mut params = params.0;
            apply_mcp_pagination_defaults(&mut params.pagination, searcher.config());
            let pagination = params.pagination;
            searcher
                .search_columns(&params)
                .await
                .map(|value| (value, pagination))
        })
        .await
    }

    /// List columns deterministically across entities.
    #[tool(
        name = "column_inventory",
        description = "Inventory tool for columns. List columns across models or sources with role, semantic_type, synonyms, example values, and parent entity context."
    )]
    #[instrument(level = "info", skip(self, params))]
    async fn column_inventory(&self, params: Parameters<ColumnInventoryParams>) -> String {
        self.handle_bounded_search_paged("column_inventory", None, |searcher| async move {
            let mut params = params.0;
            apply_mcp_pagination_defaults(&mut params.pagination, searcher.config());
            let pagination = params.pagination;
            searcher
                .column_inventory(&params)
                .await
                .map(|value| (value, pagination))
        })
        .await
    }

    /// Compare grain information between two entities.
    #[tool(
        name = "compare_grains",
        description = "Compare effective grain between two entities, including time field, primary key, dimensions, and any grain variants sourced from entity-level or metric-level Nova metadata."
    )]
    #[instrument(level = "info", skip(self, params))]
    async fn compare_grains(&self, params: Parameters<CompareGrainsParams>) -> String {
        self.handle_bounded_search("compare_grains", None, |searcher| async move {
            searcher.compare_grains(&params.0).await
        })
        .await
    }

    /// Find overlapping entities using shared semantic evidence.
    #[tool(
        name = "find_entity_overlap",
        description = "Detect overlapping entities using shared domains, synonyms, indicator names, semantic types, and grain hints. Useful for cleanup, canonicalization, and architecture reviews."
    )]
    #[instrument(level = "info", skip(self, params))]
    async fn find_entity_overlap(&self, params: Parameters<FindEntityOverlapParams>) -> String {
        self.handle_bounded_search_paged("find_entity_overlap", None, |searcher| async move {
            let mut params = params.0;
            apply_mcp_pagination_defaults(&mut params.pagination, searcher.config());
            let pagination = params.pagination;
            searcher
                .find_entity_overlap(&params)
                .await
                .map(|value| (value, pagination))
        })
        .await
    }

    /// Project-level report for modelling consistency issues.
    #[tool(
        name = "modelling_consistency_report",
        description = "Audit project-level overlap, duplicate indicators, canonical conflicts, and grain drift across entities. Use for cleanup and model architecture workflows."
    )]
    #[instrument(level = "info", skip(self, params))]
    async fn modelling_consistency_report(
        &self,
        params: Parameters<ModellingConsistencyReportParams>,
    ) -> String {
        self.handle_bounded_search_paged(
            "modelling_consistency_report",
            None,
            |searcher| async move {
                let mut params = params.0;
                apply_mcp_pagination_defaults(&mut params.pagination, searcher.config());
                let pagination = params.pagination;
                searcher
                    .modelling_consistency_report(&params)
                    .await
                    .map(|value| (value, pagination))
            },
        )
        .await
    }

    /// Get complete entity data by `unique_id` or name. Returns ALL fields from the manifest.
    #[tool(
        name = "get_entity",
        description = "Fetch the full manifest object for a single entity. Use after search to inspect config, SQL, tags, meta, docs, and relation info. Provide resource_type when names are ambiguous."
    )]
    #[instrument(level = "info", skip(self, params))]
    async fn get_entity(&self, params: Parameters<GetEntityParams>) -> String {
        self.handle_async("get_entity", None, |searcher| async move {
            let mut params = params.0;
            apply_mcp_entity_detail_default(&mut params.detail, searcher.config());
            searcher.get_entity_data(&params).await
        })
        .await
    }

    /// List entities by type with optional filtering.
    #[tool(
        name = "list_entities",
        description = "Inventory tool. List entities of a specific type (model, source, macro, doc, test, seed, snapshot, analysis, exposure, metric, group). Filter by package, tags, or database.schema when you need a scoped catalog."
    )]
    #[instrument(level = "info", skip(self, params))]
    async fn list_entities(&self, params: Parameters<ListEntitiesParams>) -> String {
        self.handle_async_paged("list_entities", None, |searcher| async move {
            let mut params = params.0;
            apply_mcp_entity_detail_default(&mut params.detail, searcher.config());
            apply_mcp_pagination_defaults(&mut params.pagination, searcher.config());
            let pagination = params.pagination;
            searcher
                .list_entities(&params)
                .await
                .map(|value| (value, pagination))
        })
        .await
    }

    /// Get upstream or downstream lineage for an entity.
    #[tool(
        name = "get_lineage",
        description = "Dependency map for impact analysis. Use direction='upstream' to see inputs or 'downstream' to see impacted dependents. Set depth to limit traversal and resource_types to filter results."
    )]
    #[instrument(level = "info", skip(self, params))]
    async fn get_lineage(&self, params: Parameters<GetLineageParams>) -> String {
        self.handle_async("get_lineage", None, |searcher| async move {
            let mut params = params.0;
            apply_mcp_entity_detail_default(&mut params.detail, searcher.config());
            searcher.get_lineage(&params).await
        })
        .await
    }

    /// Get raw or compiled SQL for a model.
    #[tool(
        name = "get_sql",
        description = "Retrieve model SQL. Use compiled=true for rendered SQL (actual runtime query) and compiled=false for raw SQL with Jinja. Best used when validating logic or debugging."
    )]
    #[instrument(level = "info", skip(self, params))]
    async fn get_sql(&self, params: Parameters<GetSqlParams>) -> String {
        self.handle_async("get_sql", None, |searcher| async move {
            searcher.get_sql(&params.0).await
        })
        .await
    }

    /// Get column information for a model or source.
    #[tool(
        name = "get_columns",
        description = "Schema inspection. Returns column names, descriptions, data types, and constraints for a model or source. Use for column discovery and documentation checks."
    )]
    #[instrument(level = "info", skip(self, params))]
    async fn get_columns(&self, params: Parameters<GetColumnsParams>) -> String {
        self.handle_async("get_columns", None, |searcher| async move {
            searcher.get_columns(&params.0).await
        })
        .await
    }

    /// Compare two entities and show differences.
    #[tool(
        name = "diff_entities",
        description = "Compare two entities side-by-side. Use to spot differences in columns, tags, or other fields when validating refactors or migrations."
    )]
    #[instrument(level = "info", skip(self, params))]
    async fn diff_entities(&self, params: Parameters<DiffEntitiesParams>) -> String {
        self.handle_async("diff_entities", None, |searcher| async move {
            searcher.diff_entities(&params.0).await
        })
        .await
    }

    /// Analyze the downstream impact of changing an entity.
    #[tool(
        name = "get_impact",
        description = "Quick blast-radius estimate. Returns downstream count and an impact score to gauge risk before making changes."
    )]
    #[instrument(level = "info", skip(self, params))]
    async fn get_impact(&self, params: Parameters<GetImpactParams>) -> String {
        self.handle_async("get_impact", None, |searcher| async move {
            searcher.get_impact(&params.0).await
        })
        .await
    }

    /// Validate the DAG for cycles and issues.
    #[tool(
        name = "validate_dag",
        description = "Integrity check for the project DAG. Detects cycles and orphaned nodes. Run after adding or modifying dependencies."
    )]
    #[instrument(level = "info", skip(self, params))]
    async fn validate_dag(&self, params: Parameters<ValidateDagParams>) -> String {
        self.handle_async("validate_dag", None, |searcher| async move {
            searcher.validate_dag(&params.0).await
        })
        .await
    }

    /// Validate project YAML meta.nova blocks.
    #[tool(
        name = "validate_nova_meta",
        description = "Validate meta.nova blocks in dbt project YAML using the same schema and semantic checks as audit nova-meta. Uses local server filesystem paths scoped under the server working directory."
    )]
    #[instrument(level = "info", skip(self, params))]
    async fn validate_nova_meta(&self, params: Parameters<ValidateNovaMetaParams>) -> String {
        self.handle_async("validate_nova_meta", None, |_searcher| async move {
            build_nova_meta_tool_response(&params.0)
        })
        .await
    }

    /// Validate an eval suite file without running it.
    #[tool(
        name = "validate_eval_suite",
        description = "Validate a local YAML/JSON eval suite file using the same schema checks as eval validate. Suite paths are scoped under the server working directory."
    )]
    #[instrument(level = "info", skip(self, params))]
    async fn validate_eval_suite(&self, params: Parameters<ValidateEvalSuiteParams>) -> String {
        self.handle_async("validate_eval_suite", None, |_searcher| async move {
            build_eval_validate_tool_response(&params.0)
        })
        .await
    }

    /// Get eval gate status from latest telemetry.
    #[tool(
        name = "get_eval_gate",
        description = "Read eval telemetry and return the same gate report data as eval gate --json for a suite name."
    )]
    #[instrument(level = "info", skip(self, params))]
    async fn get_eval_gate(&self, params: Parameters<GetEvalGateParams>) -> String {
        self.handle_async("get_eval_gate", None, |_searcher| async move {
            build_eval_gate_tool_response(&params.0)
        })
        .await
    }

    /// Get filtered eval telemetry history.
    #[tool(
        name = "get_eval_history",
        description = "Read eval telemetry rows for a suite on or after a YYYY-MM-DD UTC date, matching eval history data without line-oriented CLI output."
    )]
    #[instrument(level = "info", skip(self, params))]
    async fn get_eval_history(&self, params: Parameters<GetEvalHistoryParams>) -> String {
        self.handle_async("get_eval_history", None, |_searcher| async move {
            build_eval_history_tool_response(&params.0)
        })
        .await
    }

    /// Run deterministic bridge evals against the loaded MCP manifest.
    #[tool(
        name = "run_eval",
        description = "Run deterministic bridge eval assertions against the currently loaded MCP manifest. Disabled unless DBT_NOVA_MCP_ENABLE_EVAL_RUN=1 is set."
    )]
    #[instrument(level = "info", skip(self, params))]
    async fn run_eval(&self, params: Parameters<RunEvalParams>) -> String {
        self.handle_async("run_eval", None, |searcher| async move {
            build_eval_run_tool_response(&searcher, &params.0).await
        })
        .await
    }

    /// Write a starter eval suite file.
    #[tool(
        name = "init_eval_suite",
        description = "Write a starter eval suite under the server working directory. Disabled unless DBT_NOVA_MCP_ENABLE_EVAL_WRITES=1 is set."
    )]
    #[instrument(level = "info", skip(self, params))]
    async fn init_eval_suite(&self, params: Parameters<InitEvalSuiteParams>) -> String {
        self.handle_async("init_eval_suite", None, |_searcher| async move {
            build_eval_init_tool_response(&params.0)
        })
        .await
    }

    /// Run provider-backed agent evals.
    #[tool(
        name = "run_agent_eval",
        description = "Run provider-backed agent evals and score tool-use traces. Disabled unless DBT_NOVA_MCP_ENABLE_AGENT_EVAL=1 is set; custom provider commands also require DBT_NOVA_MCP_ENABLE_CUSTOM_AGENT_PROVIDER=1."
    )]
    #[instrument(level = "info", skip(self, params))]
    async fn run_agent_eval(&self, params: Parameters<RunAgentEvalParams>) -> String {
        self.handle_async("run_agent_eval", None, |_searcher| async move {
            build_agent_eval_tool_response(&params.0).await
        })
        .await
    }

    /// Inspect a local tool-call trace JSONL file.
    #[tool(
        name = "inspect_tool_trace",
        description = "Inspect a local Nova tool-call trace JSONL file and return rows, parse warnings, tool order, counts, response byte budgets, truncation, errors, and semantic-first signals. Trace paths are scoped under the server working directory."
    )]
    #[instrument(level = "info", skip(self, params))]
    async fn inspect_tool_trace(&self, params: Parameters<TraceInspectParams>) -> String {
        self.handle_async("inspect_tool_trace", None, |_searcher| async move {
            build_trace_inspect_tool_response(&params.0)
        })
        .await
    }

    /// Summarize a local tool-call trace JSONL file.
    #[tool(
        name = "summarize_tool_trace",
        description = "Summarize a local Nova tool-call trace JSONL file and optionally write a Markdown report. Report writes are disabled unless DBT_NOVA_MCP_ENABLE_TRACE_WRITES=1 is set."
    )]
    #[instrument(level = "info", skip(self, params))]
    async fn summarize_tool_trace(&self, params: Parameters<TraceSummarizeParams>) -> String {
        self.handle_async("summarize_tool_trace", None, |_searcher| async move {
            build_trace_summarize_tool_response(&params.0)
        })
        .await
    }

    /// Redact a local tool-call trace JSONL file.
    #[tool(
        name = "redact_tool_trace",
        description = "Redact a local Nova tool-call trace JSONL file for safe sharing. Writes are disabled unless DBT_NOVA_MCP_ENABLE_TRACE_WRITES=1 is set."
    )]
    #[instrument(level = "info", skip(self, params))]
    async fn redact_tool_trace(&self, params: Parameters<TraceRedactParams>) -> String {
        self.handle_async("redact_tool_trace", None, |_searcher| async move {
            build_trace_redact_tool_response(&params.0)
        })
        .await
    }

    /// Replay deterministic local Nova tool calls from a trace.
    #[tool(
        name = "replay_tool_trace",
        description = "Replay supported deterministic Nova tool calls from a local trace JSONL file against the currently loaded MCP manifest. Unsupported, unsafe, under-specified, and execute_sql rows are skipped with explicit reasons."
    )]
    #[instrument(level = "info", skip(self, params))]
    async fn replay_tool_trace(&self, params: Parameters<TraceReplayParams>) -> String {
        self.handle_async("replay_tool_trace", None, |searcher| async move {
            build_trace_replay_tool_response(&searcher, &params.0).await
        })
        .await
    }

    /// Show manifest metadata and statistics.
    #[tool(
        name = "show_metadata",
        description = "Project overview. Returns manifest metadata (dbt version, project name, invocation info) plus entity counts by type."
    )]
    #[instrument(level = "info", skip(self))]
    async fn show_metadata(&self) -> String {
        self.handle_async("show_metadata", None, |searcher| async move {
            searcher.show_metadata().await
        })
        .await
    }

    /// Health check for readiness and index status.
    #[tool(
        name = "health",
        description = "Health check for dbt-nova. Returns readiness state, manifest/cache diagnostics, refresh stats, tool latency metrics, and concurrency saturation for search and SQL execution."
    )]
    #[instrument(level = "info", skip(self))]
    async fn health(&self) -> String {
        let mut payload = build_manifest_health_payload(&self.searcher).await.payload;
        let searcher = self.searcher.get().await.ok();
        if let Some(base) = payload.as_object_mut() {
            if let Some(searcher) = searcher.as_ref() {
                let concurrency = self.search_concurrency.get_or_init(|| {
                    ConcurrencyLimiter::for_search(searcher.config()).map(Arc::new)
                });
                let snapshot = concurrency.as_ref().map_or_else(
                    || serde_json::json!({"enabled": false}),
                    |state| state.snapshot(),
                );
                base.insert("search_concurrency".to_string(), snapshot);
                let sql_concurrency = self
                    .sql_concurrency
                    .get_or_init(|| ConcurrencyLimiter::for_sql(searcher.config()).map(Arc::new));
                let sql_snapshot = sql_concurrency.as_ref().map_or_else(
                    || serde_json::json!({"enabled": false}),
                    |state| state.snapshot(),
                );
                base.insert("sql_concurrency".to_string(), sql_snapshot);
            } else {
                base.insert(
                    "search_concurrency".to_string(),
                    serde_json::json!({"enabled": false}),
                );
                base.insert(
                    "sql_concurrency".to_string(),
                    serde_json::json!({"enabled": false}),
                );
            }
        }
        if let Some(base) = payload.as_object_mut() {
            base.insert("tool_metrics".to_string(), self.metrics.snapshot());
        }
        let response = match serde_json::to_value(SuccessResponse::new(payload, 1)) {
            Ok(response) => response,
            Err(err) => return Self::serialization_error_response(&err),
        };
        if let Some(searcher) = searcher.as_ref() {
            return Self::serialize_budgeted_value(response, searcher.config())
                .unwrap_or_else(|err| Self::serialization_error_response(&err));
        }
        serde_json::to_string(&response)
            .unwrap_or_else(|err| Self::serialization_error_response(&err))
    }

    /// Reload the manifest from a new source and rebuild indexes.
    #[tool(
        name = "reload_manifest",
        description = "Reload manifest from a new source (manifest_uri or manifest_path) and rebuild indexes in the background. Useful for switching between local and remote manifests without restarting."
    )]
    #[instrument(level = "info", skip(self, params))]
    async fn reload_manifest(&self, params: Parameters<ReloadManifestParams>) -> String {
        let start = Instant::now();
        let mut success = false;
        let out = match self.searcher.reload(&params.0).await {
            Ok(payload) => match serde_json::to_value(SuccessResponse::new(payload, 1)) {
                Ok(response) => match self.searcher.get().await {
                    Ok(searcher) => {
                        match Self::serialize_budgeted_value(response, searcher.config()) {
                            Ok(out) => {
                                success = true;
                                out
                            }
                            Err(err) => Self::serialization_error_response(&err),
                        }
                    }
                    Err(_) => match serde_json::to_string(&response) {
                        Ok(out) => {
                            success = true;
                            out
                        }
                        Err(err) => Self::serialization_error_response(&err),
                    },
                },
                Err(err) => serde_json::json!({
                    "success": false,
                    "error": format!("Serialization error: {}", err),
                    "error_code": "SERVER_ERROR"
                })
                .to_string(),
            },
            Err(err) => match serde_json::to_string(&err.to_response()) {
                Ok(out) => out,
                Err(ser_err) => serde_json::json!({
                    "success": false,
                    "error": format!("Serialization error: {}", ser_err),
                    "error_code": "SERVER_ERROR"
                })
                .to_string(),
            },
        };
        self.record_metrics(
            "reload_manifest",
            None,
            elapsed_ms_to_u64(start.elapsed()),
            success,
        );
        out
    }

    /// Warm semantic caches for the current manifest source.
    #[tool(
        name = "warm_manifest",
        description = "Warm vector/sparse/reranker semantic caches for the current manifest source. Disabled unless DBT_NOVA_MCP_ENABLE_MANIFEST_WARM=1 is set; read-only storage is rejected."
    )]
    #[instrument(level = "info", skip(self, params))]
    async fn warm_manifest(&self, params: Parameters<WarmManifestParams>) -> String {
        self.handle_async("warm_manifest", None, |searcher| async move {
            build_manifest_warm_tool_response(&searcher, &params.0).await
        })
        .await
    }

    /// Show active or default runtime configuration.
    #[tool(
        name = "show_config",
        description = "Operator config inspection. Returns the active runtime configuration, or defaults when defaults=true. Secret credential values are not part of the persisted Nova config."
    )]
    #[instrument(level = "info", skip(self, params))]
    async fn show_config(&self, params: Parameters<ConfigShowParams>) -> String {
        self.handle_async("show_config", None, |searcher| async move {
            build_config_show_tool_response(searcher.config(), &params.0)
        })
        .await
    }

    /// Validate active runtime configuration.
    #[tool(
        name = "validate_config",
        description = "Operator config validation. Validates the active runtime configuration and returns the same JSON payload as config validate."
    )]
    #[instrument(level = "info", skip(self, params))]
    async fn validate_config(&self, params: Parameters<ConfigValidateParams>) -> String {
        self.handle_async("validate_config", None, |searcher| async move {
            build_config_validate_tool_response(searcher.config(), &params.0)
        })
        .await
    }

    /// Inspect Nova storage instances.
    #[tool(
        name = "inspect_storage",
        description = "Operator storage inspection. Lists storage instances and metadata without mutating storage."
    )]
    #[instrument(level = "info", skip(self, params))]
    async fn inspect_storage(&self, params: Parameters<StorageInspectParams>) -> String {
        self.handle_async("inspect_storage", None, |searcher| async move {
            build_storage_inspect_tool_response(searcher.config(), &params.0)
        })
        .await
    }

    /// Prune stale Nova storage instances.
    #[tool(
        name = "prune_storage",
        description = "Operator storage pruning. Destructive; disabled unless DBT_NOVA_MCP_ENABLE_STORAGE_ADMIN=1 is set."
    )]
    #[instrument(level = "info", skip(self, params))]
    async fn prune_storage(&self, params: Parameters<StoragePruneParams>) -> String {
        self.handle_async("prune_storage", None, |searcher| async move {
            build_storage_prune_tool_response(searcher.config(), &params.0)
        })
        .await
    }

    /// Clean up the configured Nova storage instance.
    #[tool(
        name = "cleanup_storage",
        description = "Operator storage cleanup. Destructive; disabled unless DBT_NOVA_MCP_ENABLE_STORAGE_ADMIN=1 is set."
    )]
    #[instrument(level = "info", skip(self, params))]
    async fn cleanup_storage(&self, params: Parameters<StorageCleanupParams>) -> String {
        self.handle_async("cleanup_storage", None, |searcher| async move {
            build_storage_cleanup_tool_response(searcher.config(), &params.0)
        })
        .await
    }

    /// List all tags with counts.
    #[tool(
        name = "list_tags",
        description = "Tag inventory. Lists all tags with counts so you can filter or audit by tag."
    )]
    #[instrument(level = "info", skip(self))]
    async fn list_tags(&self) -> String {
        self.handle_async("list_tags", None, |searcher| async move {
            searcher.list_tags().await
        })
        .await
    }

    /// List all packages with counts.
    #[tool(
        name = "list_packages",
        description = "Package inventory. Lists all packages with counts to help scope ownership or prioritize review."
    )]
    #[instrument(level = "info", skip(self))]
    async fn list_packages(&self) -> String {
        self.handle_async("list_packages", None, |searcher| async move {
            searcher.list_packages().await
        })
        .await
    }

    /// List all database.schema combinations.
    #[tool(
        name = "list_databases",
        description = "Storage inventory. Lists database.schema combinations with counts to help locate physical relations."
    )]
    #[instrument(level = "info", skip(self))]
    async fn list_databases(&self) -> String {
        self.handle_async("list_databases", None, |searcher| async move {
            searcher.list_databases().await
        })
        .await
    }

    /// Trace column lineage upstream or downstream.
    #[tool(
        name = "get_column_lineage",
        description = "Column-level lineage. Trace a column upstream (origins) or downstream (usage). Uses SQL parsing + heuristics. Use confidence=high for exact matches, medium (default) for SQL-based matches, low for fuzzy."
    )]
    #[instrument(level = "info", skip(self, params))]
    async fn get_column_lineage(&self, params: Parameters<GetColumnLineageParams>) -> String {
        self.handle_async("get_column_lineage", None, |searcher| async move {
            searcher.get_column_lineage(&params.0).await
        })
        .await
    }

    /// Get test coverage analysis for an entity.
    #[tool(
        name = "get_test_coverage",
        description = "Quality signal. Returns schema/data tests, test type breakdown, coverage percentage, and gaps (missing PK tests or untested columns). Use before changes or audits."
    )]
    #[instrument(level = "info", skip(self, params))]
    async fn get_test_coverage(&self, params: Parameters<GetTestCoverageParams>) -> String {
        self.handle_async("get_test_coverage", None, |searcher| async move {
            searcher.get_test_coverage(&params.0).await
        })
        .await
    }

    /// Get metadata quality score for entities.
    #[tool(
        name = "get_metadata_score",
        description = "Metadata quality scoring. Returns a 0-100 score with category breakdowns and recommendations for entities, columns, or project scope."
    )]
    #[instrument(level = "info", skip(self, params))]
    async fn get_metadata_score(&self, params: Parameters<GetMetadataScoreParams>) -> String {
        self.handle_async_paged("get_metadata_score", None, |searcher| async move {
            let mut params = params.0;
            let pagination =
                apply_mcp_metadata_score_pagination_defaults(&mut params, searcher.config());
            searcher
                .get_metadata_score(&params)
                .await
                .map(|value| (value, pagination))
        })
        .await
    }

    /// Get the higher-level metadata audit report and gate status.
    #[tool(
        name = "get_metadata_audit",
        description = "Metadata audit report. Runs the same report/gate logic as audit metadata-score without writing files or turning required failures into transport errors."
    )]
    #[instrument(level = "info", skip(self, params))]
    async fn get_metadata_audit(&self, params: Parameters<GetMetadataAuditParams>) -> String {
        self.handle_async("get_metadata_audit", None, |searcher| async move {
            build_metadata_audit_tool_response(&searcher, &params.0).await
        })
        .await
    }

    /// Get the manifest-level agent-readiness report.
    #[tool(
        name = "get_agent_readiness",
        description = "Agent readiness audit. Returns the same agent_readiness.v1 JSON report as the CLI audit command without writing files or applying CLI exit semantics."
    )]
    #[instrument(level = "info", skip(self, params))]
    async fn get_agent_readiness(&self, params: Parameters<GetAgentReadinessParams>) -> String {
        self.handle_async("get_agent_readiness", None, |searcher| async move {
            build_agent_readiness_tool_response(&searcher, &params.0).await
        })
        .await
    }

    /// Get multiple entities in one call.
    #[tool(
        name = "batch_get_entities",
        description = "Bulk fetch. Retrieve multiple entities by unique_id in one call to reduce round trips. Returns found entities and not_found ids."
    )]
    #[instrument(level = "info", skip(self, params))]
    async fn batch_get_entities(&self, params: Parameters<BatchGetParams>) -> String {
        self.handle_async("batch_get_entities", None, |searcher| async move {
            let mut params = params.0;
            apply_mcp_entity_detail_default(&mut params.detail, searcher.config());
            searcher.batch_get_entities(&params).await
        })
        .await
    }

    /// Find entities by file path pattern.
    #[tool(
        name = "find_by_path",
        description = "Path-based lookup. Use glob patterns to find entities by file path (e.g., models/staging/**, models/*.sql). Useful when you know where code lives."
    )]
    #[instrument(level = "info", skip(self, params))]
    async fn find_by_path(&self, params: Parameters<FindByPathParams>) -> String {
        self.handle_async_paged("find_by_path", None, |searcher| async move {
            let mut params = params.0;
            apply_mcp_entity_detail_default(&mut params.detail, searcher.config());
            apply_mcp_pagination_defaults(&mut params.pagination, searcher.config());
            let pagination = params.pagination;
            searcher
                .find_by_path(&params)
                .await
                .map(|value| (value, pagination))
        })
        .await
    }

    /// Search and discover recipe templates.
    #[tool(
        name = "search_recipes",
        description = "Discover reusable analysis recipes. Use `topic` or `query` to narrow results; return query names for deterministic execution planning."
    )]
    #[instrument(level = "info", skip(self, params))]
    async fn search_recipes(&self, params: Parameters<SearchRecipesParams>) -> String {
        self.handle_async_paged("search_recipes", None, |searcher| async move {
            let mut params = params.0;
            apply_mcp_pagination_defaults(&mut params.pagination, searcher.config());
            let pagination = params.pagination;
            searcher
                .search_recipes(&params)
                .await
                .map(|value| (value, pagination))
        })
        .await
    }

    /// Get recipe definition and query ordering.
    #[tool(
        name = "get_recipe",
        description = "Load a recipe by id (directory path). Returns query files and metadata so agents can provide deterministic analysis flows."
    )]
    #[instrument(level = "info", skip(self, params))]
    async fn get_recipe(&self, params: Parameters<GetRecipeParams>) -> String {
        self.handle_async("get_recipe", None, |searcher| async move {
            searcher.get_recipe(&params.0).await
        })
        .await
    }

    /// Run a deterministic analysis recipe by executing all or selected SQL queries.
    #[tool(
        name = "run_recipe",
        description = "Execute a recipe's SQL files in deterministic order, reusing `search` and `execute_sql` controls."
    )]
    #[instrument(level = "info", skip(self, params))]
    async fn run_recipe(&self, params: Parameters<RunRecipeParams>) -> String {
        let permit = match Self::permit_from_result(self.acquire_sql_permit_for_tool().await) {
            Ok(permit) => permit,
            Err(response) => return response,
        };
        let result = self
            .handle_async("run_recipe", None, |searcher| async move {
                searcher.run_recipe(&params.0).await
            })
            .await;
        drop(permit);
        result
    }

    /// Find undocumented entities and columns.
    #[tool(
        name = "get_undocumented",
        description = "Documentation audit. Find entities missing descriptions and optionally undocumented columns. Use for governance or doc coverage checks."
    )]
    #[instrument(level = "info", skip(self, params))]
    async fn get_undocumented(&self, params: Parameters<GetUndocumentedParams>) -> String {
        self.handle_async_paged("get_undocumented", None, |searcher| async move {
            let mut params = params.0;
            apply_mcp_pagination_defaults(&mut params.pagination, searcher.config());
            let pagination = params.pagination;
            searcher
                .get_undocumented(&params)
                .await
                .map(|value| (value, pagination))
        })
        .await
    }

    /// Get rich context for an entity.
    #[tool(
        name = "get_context",
        description = "One-shot context bundle. Returns lineage, columns, tests, docs, and summary stats for an entity. Use when an agent needs full context quickly."
    )]
    #[instrument(level = "info", skip(self, params))]
    async fn get_context(&self, params: Parameters<GetContextParams>) -> String {
        self.handle_async("get_context", None, |searcher| async move {
            searcher.get_context(&params.0).await
        })
        .await
    }

    /// Execute SQL against a Databricks SQL warehouse.
    #[tool(
        name = "execute_sql",
        description = "Run SQL against the configured warehouse provider. Supports provider diagnostics with preflight_only=true and optional catalog/schema/relation checks."
    )]
    #[instrument(level = "info", skip(self, params))]
    async fn execute_sql(&self, params: Parameters<ExecuteSqlParams>) -> String {
        let permit = match Self::permit_from_result(self.acquire_sql_permit_for_tool().await) {
            Ok(permit) => permit,
            Err(response) => return response,
        };
        let result = self
            .handle_async("execute_sql", None, |searcher| async move {
                searcher.execute_sql(&params.0).await
            })
            .await;
        drop(permit);
        result
    }
}

fn elapsed_ms_to_u64(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn remaining_timeout(start: Instant, timeout_ms: u64) -> Option<Duration> {
    if timeout_ms == 0 {
        return None;
    }
    let elapsed_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
    timeout_ms
        .checked_sub(elapsed_ms)
        .map(Duration::from_millis)
}

fn search_timeout_error(search_timeout_ms: u64) -> DbtNovaError {
    DbtNovaError::ServerError(format!("Search timed out after {search_timeout_ms}ms"))
}

fn sql_queue_timeout(config: &crate::config::DbtNovaConfig) -> Option<Duration> {
    if config.sql_queue_timeout_ms == 0 {
        return None;
    }
    Some(Duration::from_millis(config.sql_queue_timeout_ms))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::Path;

    use tempfile::TempDir;

    use super::*;
    use crate::tests::common::fixture_manifest_path_string;
    use crate::tools::catalog::MCP_TOOL_NAMES;

    fn test_config(storage_root: &Path) -> crate::config::DbtNovaConfig {
        let mut config = crate::config::DbtNovaConfig {
            manifest_path: fixture_manifest_path_string(),
            manifest_refresh_secs: 0,
            storage_dir: storage_root.join(".dbt-nova").to_string_lossy().to_string(),
            storage_instance_id: "server-mcp-tests".to_string(),
            cleanup_storage_on_start: true,
            ..Default::default()
        };
        config.search.enable_vector_search = false;
        config.search.enable_sparse_search = false;
        config.search.enable_reranker = false;
        config
    }

    async fn spawn_ready_server(storage_root: &Path) -> DbtNovaServer {
        let handle = ManifestSearchHandle::spawn(test_config(storage_root));
        handle
            .wait_ready()
            .await
            .expect("fixture manifest should load");
        DbtNovaServer::new(handle)
    }

    #[test]
    fn mcp_response_budget_prunes_large_object_payloads() {
        let mut columns = serde_json::Map::new();
        for idx in 0..500 {
            columns.insert(
                format!("column_{idx:04}"),
                serde_json::json!({
                    "name": format!("column_{idx:04}"),
                    "description": "short but repeated metadata",
                    "data_type": "string"
                }),
            );
        }
        let config = crate::config::DbtNovaConfig {
            mcp_max_response_bytes: 4096,
            mcp_max_string_chars: 128,
            ..Default::default()
        };
        let response = serde_json::json!({
            "success": true,
            "count": 1,
            "data": {
                "unique_id": "model.pkg.large",
                "name": "large",
                "columns": columns
            }
        });

        let budgeted = apply_mcp_response_budget(response, &config);
        let response_bytes = serialized_len(&budgeted);

        assert!(
            response_bytes <= config.mcp_max_response_bytes,
            "response_bytes={response_bytes}"
        );
        assert_eq!(
            budgeted["_nova_result_meta"]["truncated"],
            serde_json::json!(true)
        );
        assert!(
            budgeted["_nova_result_meta"]["omitted_paths"]
                .as_array()
                .is_some_and(|paths| !paths.is_empty())
        );
    }

    #[test]
    fn mcp_response_budget_preserves_singleton_object_shape() {
        let mut columns = serde_json::Map::new();
        for idx in 0..100 {
            columns.insert(
                format!("column_{idx:03}"),
                serde_json::json!({
                    "name": format!("column_{idx:03}"),
                    "description": "x".repeat(200)
                }),
            );
        }
        let config = crate::config::DbtNovaConfig {
            mcp_max_response_bytes: 256,
            mcp_max_string_chars: 16,
            ..Default::default()
        };
        let response = serde_json::json!({
            "success": true,
            "count": 1,
            "data": {
                "unique_id": "model.pkg.large",
                "columns": columns
            }
        });

        let budgeted = apply_mcp_response_budget(response, &config);

        assert!(
            serialized_len(&budgeted) <= config.mcp_max_response_bytes,
            "response_bytes={}",
            serialized_len(&budgeted)
        );
        assert_eq!(budgeted["count"], serde_json::json!(1));
        assert!(budgeted["data"].as_object().is_some());
        assert!(budgeted["data"].as_array().is_none());
    }

    #[test]
    fn mcp_response_budget_keeps_payload_when_only_meta_exceeds_budget() {
        let config = crate::config::DbtNovaConfig {
            mcp_max_response_bytes: 100,
            mcp_max_string_chars: 10,
            ..Default::default()
        };
        let response = serde_json::json!({
            "success": true,
            "data": {
                "text": "x".repeat(1000)
            }
        });

        let budgeted = apply_mcp_response_budget(response, &config);

        assert!(
            serialized_len(&budgeted) <= config.mcp_max_response_bytes,
            "response_bytes={}",
            serialized_len(&budgeted)
        );
        assert!(budgeted["data"]["text"].as_str().is_some());
        assert!(budgeted.get("_nova_result_meta").is_none());
    }

    #[test]
    fn mcp_response_budget_updates_count_when_data_array_is_truncated() {
        let config = crate::config::DbtNovaConfig {
            mcp_max_response_bytes: 8192,
            mcp_max_string_chars: 64,
            ..Default::default()
        };
        let rows: Vec<_> = (0..100)
            .map(|idx| {
                serde_json::json!({
                    "unique_id": format!("model.pkg.item_{idx:03}"),
                    "description": "x".repeat(300)
                })
            })
            .collect();
        let response = serde_json::json!({
            "success": true,
            "count": rows.len(),
            "data": rows
        });

        let budgeted = apply_mcp_response_budget(response, &config);
        let returned_count = budgeted["data"].as_array().map_or(0, Vec::len);

        assert!(
            returned_count < 100,
            "data was not truncated: returned_count={returned_count}"
        );
        assert!(returned_count > 0);
        assert_eq!(budgeted["count"], serde_json::json!(returned_count));
        assert_eq!(budgeted["truncated"], serde_json::json!(true));
        assert_eq!(
            budgeted["_nova_result_meta"]["original_count"],
            serde_json::json!(100)
        );
    }

    #[test]
    fn mcp_response_budget_updates_count_when_sql_rows_are_truncated() {
        let config = crate::config::DbtNovaConfig {
            mcp_max_response_bytes: 8192,
            mcp_max_string_chars: 64,
            ..Default::default()
        };
        let rows: Vec<_> = (0..100)
            .map(|idx| serde_json::json!([format!("row_{idx:03}"), "x".repeat(300)]))
            .collect();
        let response = serde_json::json!({
            "success": true,
            "count": rows.len(),
            "data": {
                "rows": rows,
                "columns": ["id", "description"],
                "column_types": ["Text", "Text"],
                "stats": {
                    "total_row_count": 100,
                    "total_chunk_count": 1
                },
                "state": "SUCCEEDED",
                "provider": "duckdb",
                "elapsed_ms": 12,
                "statement_id": "stmt_123",
                "duckdb_path": "tests/fixtures/tokenomics.duckdb",
                "truncated": false
            }
        });

        let budgeted = apply_mcp_response_budget(response, &config);
        let returned_count = budgeted["data"]["rows"].as_array().map_or(0, Vec::len);

        assert!(
            returned_count < 100,
            "rows were not truncated: returned_count={returned_count}"
        );
        assert!(returned_count > 0);
        assert!(budgeted["data"]["rows"].as_array().is_some());
        assert_eq!(budgeted["count"], serde_json::json!(returned_count));
        assert_eq!(budgeted["truncated"], serde_json::json!(true));
        assert_eq!(budgeted["data"]["truncated"], serde_json::json!(true));
    }

    #[test]
    fn mcp_response_budget_updates_count_when_nested_entities_are_truncated() {
        let config = crate::config::DbtNovaConfig {
            mcp_max_response_bytes: 8192,
            mcp_max_string_chars: 64,
            ..Default::default()
        };
        let entities: Vec<_> = (0..100)
            .map(|idx| {
                serde_json::json!({
                    "unique_id": format!("model.pkg.entity_{idx:03}"),
                    "description": "x".repeat(300)
                })
            })
            .collect();
        let not_found: Vec<_> = (0..100)
            .map(|idx| format!("model.pkg.missing_{idx:03}"))
            .collect();
        let response = serde_json::json!({
            "success": true,
            "count": entities.len(),
            "data": {
                "entities": entities,
                "not_found": not_found,
                "found_count": 100,
                "not_found_count": 100
            }
        });

        let budgeted = apply_mcp_response_budget(response, &config);
        let returned_entities = budgeted["data"]["entities"].as_array().map_or(0, Vec::len);
        let returned_not_found = budgeted["data"]["not_found"].as_array().map_or(0, Vec::len);

        assert!(
            returned_entities < 100,
            "entities were not truncated: returned_entities={returned_entities}"
        );
        assert!(returned_entities > 0);
        assert_eq!(budgeted["count"], serde_json::json!(returned_entities));
        assert_eq!(
            budgeted["data"]["found_count"],
            serde_json::json!(returned_entities)
        );
        assert_eq!(
            budgeted["data"]["not_found_count"],
            serde_json::json!(returned_not_found)
        );
        assert_eq!(budgeted["truncated"], serde_json::json!(true));
        assert_eq!(budgeted["data"]["truncated"], serde_json::json!(true));
    }

    #[test]
    fn mcp_response_budget_updates_count_when_data_columns_are_truncated() {
        let config = crate::config::DbtNovaConfig {
            mcp_max_response_bytes: 8192,
            mcp_max_string_chars: 64,
            ..Default::default()
        };
        let columns: Vec<_> = (0..100)
            .map(|idx| {
                serde_json::json!({
                    "name": format!("column_{idx:03}"),
                    "description": "x".repeat(300),
                    "data_type": "string"
                })
            })
            .collect();
        let response = serde_json::json!({
            "success": true,
            "count": columns.len(),
            "data": {
                "unique_id": "model.pkg.wide",
                "columns": columns
            }
        });

        let budgeted = apply_mcp_response_budget(response, &config);
        let returned_columns = budgeted["data"]["columns"].as_array().map_or(0, Vec::len);

        assert!(
            returned_columns < 100,
            "columns were not truncated: returned_columns={returned_columns}"
        );
        assert!(returned_columns > 0);
        assert_eq!(budgeted["count"], serde_json::json!(returned_columns));
        assert_eq!(budgeted["truncated"], serde_json::json!(true));
        assert_eq!(budgeted["data"]["truncated"], serde_json::json!(true));
        assert_eq!(
            budgeted["_nova_result_meta"]["original_count"],
            serde_json::json!(100)
        );
    }

    #[test]
    fn mcp_response_budget_keeps_lineage_count_when_only_edges_are_truncated() {
        let config = crate::config::DbtNovaConfig {
            mcp_max_response_bytes: 8192,
            mcp_max_string_chars: 64,
            ..Default::default()
        };
        let entities = vec![
            serde_json::json!({"unique_id": "model.pkg.root", "description": "root"}),
            serde_json::json!({"unique_id": "model.pkg.child", "description": "child"}),
        ];
        let edges: Vec<_> = (0..100)
            .map(|idx| {
                serde_json::json!({
                    "source": format!("model.pkg.source_{idx:03}"),
                    "target": format!("model.pkg.target_{idx:03}"),
                    "description": "x".repeat(300)
                })
            })
            .collect();
        let response = serde_json::json!({
            "success": true,
            "count": entities.len(),
            "data": {
                "root_id": "model.pkg.root",
                "entities": entities,
                "edges": edges
            }
        });

        let budgeted = apply_mcp_response_budget(response, &config);
        let returned_edges = budgeted["data"]["edges"].as_array().map_or(0, Vec::len);

        assert!(
            returned_edges < 100,
            "edges were not truncated: returned_edges={returned_edges}"
        );
        assert_eq!(budgeted["count"], serde_json::json!(2));
        assert_eq!(budgeted["truncated"], serde_json::json!(true));
        assert_eq!(budgeted["data"]["truncated"], serde_json::json!(true));
    }

    #[test]
    fn mcp_response_budget_updates_undocumented_summary_counts_when_truncated() {
        let config = crate::config::DbtNovaConfig {
            mcp_max_response_bytes: 8192,
            mcp_max_string_chars: 64,
            ..Default::default()
        };
        let entities: Vec<_> = (0..80)
            .map(|idx| {
                serde_json::json!({
                    "unique_id": format!("model.pkg.entity_{idx:03}"),
                    "description": "x".repeat(300)
                })
            })
            .collect();
        let undocumented_columns: Vec<_> = (0..80)
            .map(|idx| {
                serde_json::json!({
                    "unique_id": format!("model.pkg.entity_{idx:03}"),
                    "column": format!("column_{idx:03}"),
                    "description": "x".repeat(300)
                })
            })
            .collect();
        let response = serde_json::json!({
            "success": true,
            "count": 160,
            "data": {
                "entities": entities,
                "summary": {
                    "entities_missing_docs": 80,
                    "columns_missing_docs": 80,
                    "entities_returned": 80,
                    "columns_returned": 80,
                    "items_returned": 160
                },
                "undocumented_columns": undocumented_columns
            }
        });

        let budgeted = apply_mcp_response_budget(response, &config);
        let returned_entities = budgeted["data"]["entities"].as_array().map_or(0, Vec::len);
        let returned_columns = budgeted["data"]["undocumented_columns"]
            .as_array()
            .map_or(0, Vec::len);
        let returned_total = returned_entities + returned_columns;

        assert!(returned_total < 160);
        assert!(returned_total > 0);
        assert_eq!(budgeted["count"], serde_json::json!(returned_total));
        assert_eq!(
            budgeted["data"]["summary"]["entities_returned"],
            serde_json::json!(returned_entities)
        );
        assert_eq!(
            budgeted["data"]["summary"]["columns_returned"],
            serde_json::json!(returned_columns)
        );
        assert_eq!(
            budgeted["data"]["summary"]["items_returned"],
            serde_json::json!(returned_total)
        );
    }

    #[test]
    fn mcp_response_budget_fallback_zeroes_nested_collection_counts() {
        let config = crate::config::DbtNovaConfig {
            mcp_max_response_bytes: 256,
            mcp_max_string_chars: 16,
            ..Default::default()
        };
        let entities: Vec<_> = (0..100)
            .map(|idx| {
                serde_json::json!({
                    "unique_id": format!("model.pkg.entity_{idx:03}"),
                    "description": "x".repeat(500)
                })
            })
            .collect();
        let response = serde_json::json!({
            "success": true,
            "count": entities.len(),
            "data": {
                "entities": entities,
                "found_count": 100
            }
        });

        let budgeted = apply_mcp_response_budget(response, &config);

        assert!(
            serialized_len(&budgeted) <= config.mcp_max_response_bytes,
            "response_bytes={}",
            serialized_len(&budgeted)
        );
        assert_eq!(budgeted["count"], serde_json::json!(0));
        assert_eq!(budgeted["data"]["found_count"], serde_json::json!(0));
        assert_eq!(
            budgeted["data"]["entities"]
                .as_array()
                .map_or(usize::MAX, Vec::len),
            0
        );
        assert_eq!(budgeted["truncated"], serde_json::json!(true));
    }

    #[test]
    fn mcp_compact_profile_defaults_indicator_search_for_agents() {
        let config = crate::config::DbtNovaConfig::default();
        let mut params = SearchIndicatorParams::default();

        apply_mcp_search_indicator_defaults(&mut params, &config);

        assert_eq!(params.detail, Some(crate::params::DetailLevel::Compact));
        assert_eq!(params.pagination.limit, Some(config.mcp_default_limit));
        assert_eq!(params.group_mode, Some(ParentGroupMode::Top));
        assert_eq!(params.max_parent_groups, Some(1));
    }

    #[test]
    fn mcp_profile_defaults_preserve_explicit_detail_and_group_mode() {
        let config = crate::config::DbtNovaConfig {
            mcp_default_limit: 10,
            mcp_max_page_size: 20,
            ..Default::default()
        };
        let mut params = SearchIndicatorParams {
            detail: Some(crate::params::DetailLevel::Standard),
            group_mode: Some(ParentGroupMode::All),
            pagination: PaginationParams {
                limit: Some(500),
                offset: 0,
            },
            ..SearchIndicatorParams::default()
        };

        apply_mcp_search_indicator_defaults(&mut params, &config);

        assert_eq!(params.detail, Some(crate::params::DetailLevel::Standard));
        assert_eq!(params.group_mode, Some(ParentGroupMode::All));
        assert_eq!(params.pagination.limit, Some(20));
    }

    #[test]
    fn mcp_pagination_meta_adds_next_offset_when_more_results_exist() {
        let config = crate::config::DbtNovaConfig {
            mcp_max_response_bytes: 0,
            ..Default::default()
        };
        let response = serde_json::json!({
            "success": true,
            "count": 2,
            "total_available": 5,
            "truncated": true,
            "data": [{"unique_id": "model.pkg.a"}, {"unique_id": "model.pkg.b"}]
        });
        let pagination = PaginationParams {
            limit: Some(2),
            offset: 2,
        };

        let serialized = DbtNovaServer::serialize_budgeted_value_with_pagination(
            response,
            &config,
            Some(&pagination),
        )
        .expect("serialize response");
        let payload: serde_json::Value = serde_json::from_str(&serialized).expect("response JSON");

        assert_eq!(
            payload["_nova_result_meta"]["next_offset"],
            serde_json::json!(4)
        );
    }

    #[test]
    fn mcp_pagination_meta_omits_next_offset_when_no_items_returned() {
        let config = crate::config::DbtNovaConfig {
            mcp_max_response_bytes: 0,
            ..Default::default()
        };
        let response = serde_json::json!({
            "success": true,
            "count": 0,
            "total_available": 5,
            "truncated": true,
            "data": []
        });
        let pagination = PaginationParams {
            limit: Some(2),
            offset: 2,
        };

        let serialized = DbtNovaServer::serialize_budgeted_value_with_pagination(
            response,
            &config,
            Some(&pagination),
        )
        .expect("serialize response");
        let payload: serde_json::Value = serde_json::from_str(&serialized).expect("response JSON");

        assert!(
            payload
                .get("_nova_result_meta")
                .and_then(|meta| meta.get("next_offset"))
                .is_none()
        );
    }

    #[test]
    fn mcp_tool_catalog_matches_registered_router_names() {
        let router_names = DbtNovaServer::tool_router()
            .map
            .keys()
            .map(|name| name.as_ref().to_string())
            .collect::<BTreeSet<_>>();
        let catalog_names = MCP_TOOL_NAMES
            .iter()
            .map(|name| (*name).to_string())
            .collect::<BTreeSet<_>>();

        assert_eq!(MCP_TOOL_NAMES.len(), catalog_names.len());
        assert_eq!(catalog_names, router_names);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn health_reports_ready_status_and_metrics_payload() {
        let temp_dir = TempDir::new().expect("temp dir");
        let server = spawn_ready_server(temp_dir.path()).await;

        let payload: serde_json::Value =
            serde_json::from_str(&server.health().await).expect("health response JSON");
        assert_eq!(payload["success"], serde_json::json!(true));
        assert_eq!(payload["data"]["status"], serde_json::json!("ready"));
        assert!(payload["data"]["tool_metrics"].is_object());
        assert!(payload["data"]["search_concurrency"].is_object());
        assert!(payload["data"]["sql_concurrency"].is_object());
        assert!(payload["data"]["artifact_consumer"].is_object());
        assert_eq!(
            payload["data"]["artifact_consumer"]["enabled"],
            serde_json::json!(false)
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn health_applies_mcp_response_budget() {
        let temp_dir = TempDir::new().expect("temp dir");
        let mut config = test_config(temp_dir.path());
        config.mcp_max_response_bytes = 512;
        config.mcp_max_string_chars = 16;
        let handle = ManifestSearchHandle::spawn(config.clone());
        handle
            .wait_ready()
            .await
            .expect("fixture manifest should load");
        let server = DbtNovaServer::new(handle);

        let out = server.health().await;
        let payload: serde_json::Value = serde_json::from_str(&out).expect("health response JSON");

        assert!(
            out.len() <= config.mcp_max_response_bytes,
            "response_bytes={}",
            out.len()
        );
        assert_eq!(payload["success"], serde_json::json!(true));
        assert_eq!(payload["count"], serde_json::json!(1));
        assert_eq!(payload["truncated"], serde_json::json!(true));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn health_reports_degraded_when_enabled_semantic_components_are_not_query_ready() {
        let temp_dir = TempDir::new().expect("temp dir");
        let mut config = test_config(temp_dir.path());
        config.search.enable_vector_search = true;
        config.search.enable_sparse_search = true;
        config.search.enable_reranker = true;
        config.search.embedding_cache_dir =
            temp_dir.path().join("cache").to_string_lossy().to_string();

        let handle = ManifestSearchHandle::spawn(config);
        handle
            .wait_ready()
            .await
            .expect("fixture manifest should still load");
        let server = DbtNovaServer::new(handle);

        let payload: serde_json::Value =
            serde_json::from_str(&server.health().await).expect("health response JSON");
        assert_eq!(payload["success"], serde_json::json!(true));
        assert_eq!(payload["data"]["status"], serde_json::json!("degraded"));
        assert_eq!(
            payload["data"]["ready_for_traffic"],
            serde_json::json!(false)
        );
        assert_eq!(
            payload["data"]["search"]["vector"]["ready"],
            serde_json::json!(false)
        );
        assert_eq!(
            payload["data"]["search"]["sparse"]["ready"],
            serde_json::json!(false)
        );
        assert_eq!(
            payload["data"]["search"]["reranker"]["ready"],
            serde_json::json!(false)
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn search_and_list_tags_record_metrics() {
        let temp_dir = TempDir::new().expect("temp dir");
        let server = spawn_ready_server(temp_dir.path()).await;

        let search_params = SearchParams {
            query: "orders".to_string(),
            persona: Some("analyst".to_string()),
            ..SearchParams::default()
        };
        let search_response: serde_json::Value =
            serde_json::from_str(&server.search(Parameters(search_params)).await)
                .expect("search response JSON");
        assert_eq!(search_response["success"], serde_json::json!(true));

        let list_tags_response: serde_json::Value =
            serde_json::from_str(&server.list_tags().await).expect("list_tags response JSON");
        assert_eq!(list_tags_response["success"], serde_json::json!(true));

        let health_payload: serde_json::Value =
            serde_json::from_str(&server.health().await).expect("health response JSON");
        let tool_metrics = &health_payload["data"]["tool_metrics"];
        assert!(tool_metrics["search"]["calls"].as_u64().unwrap_or(0) >= 1);
        assert!(
            tool_metrics["search.analyst"]["calls"]
                .as_u64()
                .unwrap_or(0)
                >= 1
        );
        assert!(tool_metrics["list_tags"]["calls"].as_u64().unwrap_or(0) >= 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn metadata_score_project_scope_applies_mcp_pagination_defaults() {
        let temp_dir = TempDir::new().expect("temp dir");
        let mut config = test_config(temp_dir.path());
        config.mcp_default_limit = 2;
        config.mcp_max_page_size = 3;
        config.mcp_max_response_bytes = 0;

        let handle = ManifestSearchHandle::spawn(config);
        handle
            .wait_ready()
            .await
            .expect("fixture manifest should load");
        let server = DbtNovaServer::new(handle);

        let response: serde_json::Value = serde_json::from_str(
            &server
                .get_metadata_score(Parameters(GetMetadataScoreParams {
                    scope: Some("project".to_string()),
                    include_breakdown: false,
                    include_recommendations: false,
                    resource_types: vec!["model".to_string()],
                    ..GetMetadataScoreParams::default()
                }))
                .await,
        )
        .expect("metadata score response JSON");

        assert_eq!(response["success"], serde_json::json!(true));
        assert_eq!(response["count"], serde_json::json!(2));
        assert_eq!(response["data"]["limit"], serde_json::json!(2));
        assert_eq!(response["data"]["offset"], serde_json::json!(0));
        assert_eq!(
            response["_nova_result_meta"]["next_offset"],
            serde_json::json!(2)
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn agent_readiness_returns_report_contract() {
        let temp_dir = TempDir::new().expect("temp dir");
        let mut config = test_config(temp_dir.path());
        config.mcp_max_response_bytes = 0;
        let handle = ManifestSearchHandle::spawn(config);
        handle
            .wait_ready()
            .await
            .expect("fixture manifest should load");
        let server = DbtNovaServer::new(handle);

        let response: serde_json::Value = serde_json::from_str(
            &server
                .get_agent_readiness(Parameters(GetAgentReadinessParams {
                    personas_json: Some(r#"["engineer"]"#.to_string()),
                    eval_gate_json: Some(
                        r#"{"allowed":true,"blocked":false,"message":"gate passed"}"#.to_string(),
                    ),
                    ..GetAgentReadinessParams::default()
                }))
                .await,
        )
        .expect("agent readiness response JSON");

        assert_eq!(response["success"], serde_json::json!(true));
        assert_eq!(response["count"], serde_json::json!(1));
        assert_eq!(
            response["data"]["schema_version"],
            serde_json::json!("agent_readiness.v1")
        );
        assert_eq!(
            response["data"]["config"]["personas"],
            serde_json::json!(["engineer"])
        );
        assert_eq!(
            response["data"]["eval_status"]["status"],
            serde_json::json!("allowed")
        );
        assert!(response["data"]["persona_scores"]["engineer"].is_object());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn metadata_audit_returns_gate_data() {
        let temp_dir = TempDir::new().expect("temp dir");
        let mut config = test_config(temp_dir.path());
        config.mcp_max_response_bytes = 0;
        let handle = ManifestSearchHandle::spawn(config);
        handle
            .wait_ready()
            .await
            .expect("fixture manifest should load");
        let server = DbtNovaServer::new(handle);

        let response: serde_json::Value = serde_json::from_str(
            &server
                .get_metadata_audit(Parameters(GetMetadataAuditParams {
                    resource_types_json: Some(r#"["model"]"#.to_string()),
                    personas_json: Some(r#"["engineer"]"#.to_string()),
                    thresholds_json: Some(
                        r#"{"project":{"engineer":{"min_score":101,"severity":"required"}}}"#
                            .to_string(),
                    ),
                    include_recommendations: Some(false),
                    ..GetMetadataAuditParams::default()
                }))
                .await,
        )
        .expect("metadata audit response JSON");

        assert_eq!(response["success"], serde_json::json!(true));
        assert_eq!(response["count"], serde_json::json!(1));
        assert_eq!(
            response["data"]["selection_mode"],
            serde_json::json!("project")
        );
        assert_eq!(response["data"]["gate_status"], serde_json::json!("fail"));
        assert_eq!(
            response["data"]["summary"]["required_fail_count"],
            serde_json::json!(1)
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn validate_nova_meta_returns_report_contract() {
        let root = std::env::current_dir()
            .expect("cwd")
            .canonicalize()
            .expect("canonical cwd");
        let project_dir = TempDir::new_in(&root).expect("temp project");
        let project_relative = project_dir
            .path()
            .strip_prefix(&root)
            .expect("relative temp project")
            .display()
            .to_string();
        let models_dir = project_dir.path().join("models");
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
    columns:
      - name: order_id
        meta:
          nova:
            role: identifier
",
        )
        .expect("fixture");

        let temp_dir = TempDir::new().expect("temp dir");
        let mut config = test_config(temp_dir.path());
        config.mcp_max_response_bytes = 0;
        let handle = ManifestSearchHandle::spawn(config);
        handle
            .wait_ready()
            .await
            .expect("fixture manifest should load");
        let server = DbtNovaServer::new(handle);

        let response: serde_json::Value = serde_json::from_str(
            &server
                .validate_nova_meta(Parameters(ValidateNovaMetaParams {
                    project_dir: Some(project_relative),
                    paths: vec!["models/orders.yml".to_string()],
                    resource_kind: Some(crate::params::NovaMetaResourceKindParam::Model),
                    resource_name: Some("fct_orders".to_string()),
                    column: None,
                }))
                .await,
        )
        .expect("nova-meta response JSON");

        assert_eq!(response["success"], serde_json::json!(true));
        assert_eq!(response["count"], serde_json::json!(1));
        assert_eq!(response["data"]["target_count"], serde_json::json!(2));
        assert_eq!(response["data"]["error_count"], serde_json::json!(0));
        assert_eq!(
            response["data"]["selector"]["resource_kind"],
            serde_json::json!("model")
        );
        assert_eq!(
            response["data"]["selector"]["paths"],
            serde_json::json!(["models/orders.yml"])
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn validate_eval_suite_returns_report_contract() {
        let root = std::env::current_dir()
            .expect("cwd")
            .canonicalize()
            .expect("canonical cwd");
        let suite_dir = TempDir::new_in(&root).expect("temp suite dir");
        let suite_path = suite_dir.path().join("suite.yml");
        std::fs::write(
            &suite_path,
            r"
version: 1
name: mcp-eval-smoke
cases:
  - id: one
    assertions:
      - type: tool_success
        tool: search
        params: {}
",
        )
        .expect("suite fixture");

        let temp_dir = TempDir::new().expect("temp dir");
        let mut config = test_config(temp_dir.path());
        config.mcp_max_response_bytes = 0;
        let handle = ManifestSearchHandle::spawn(config);
        handle
            .wait_ready()
            .await
            .expect("fixture manifest should load");
        let server = DbtNovaServer::new(handle);

        let response: serde_json::Value = serde_json::from_str(
            &server
                .validate_eval_suite(Parameters(ValidateEvalSuiteParams {
                    suite: suite_path.display().to_string(),
                }))
                .await,
        )
        .expect("eval validation response JSON");

        assert_eq!(response["success"], serde_json::json!(true));
        assert_eq!(response["count"], serde_json::json!(1));
        assert_eq!(response["data"]["valid"], serde_json::json!(true));
        assert_eq!(
            response["data"]["suite_name"],
            serde_json::json!("mcp-eval-smoke")
        );
        assert!(response["data"]["safety_policy"].is_object());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn warm_manifest_rejects_without_mcp_opt_in() {
        let temp_dir = TempDir::new().expect("temp dir");
        let mut config = test_config(temp_dir.path());
        config.mcp_max_response_bytes = 0;
        let handle = ManifestSearchHandle::spawn(config);
        handle
            .wait_ready()
            .await
            .expect("fixture manifest should load");
        let server = DbtNovaServer::new(handle);

        let response: serde_json::Value = serde_json::from_str(
            &server
                .warm_manifest(Parameters(WarmManifestParams {
                    vector: true,
                    ..WarmManifestParams::default()
                }))
                .await,
        )
        .expect("warm response JSON");

        assert_eq!(response["success"], serde_json::json!(false));
        assert_eq!(response["error_code"], serde_json::json!("INVALID_PARAMS"));
        assert!(
            response["error"]
                .as_str()
                .unwrap_or_default()
                .contains("DBT_NOVA_MCP_ENABLE_MANIFEST_WARM=1")
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn validate_nova_meta_rejects_unsafe_paths() {
        let root = std::env::current_dir()
            .expect("cwd")
            .canonicalize()
            .expect("canonical cwd");
        let project_dir = TempDir::new_in(&root).expect("temp project");
        let project_relative = project_dir
            .path()
            .strip_prefix(&root)
            .expect("relative temp project")
            .display()
            .to_string();

        let temp_dir = TempDir::new().expect("temp dir");
        let mut config = test_config(temp_dir.path());
        config.mcp_max_response_bytes = 0;
        let handle = ManifestSearchHandle::spawn(config);
        handle
            .wait_ready()
            .await
            .expect("fixture manifest should load");
        let server = DbtNovaServer::new(handle);

        let response: serde_json::Value = serde_json::from_str(
            &server
                .validate_nova_meta(Parameters(ValidateNovaMetaParams {
                    project_dir: Some(project_relative),
                    paths: vec!["../Cargo.toml".to_string()],
                    ..ValidateNovaMetaParams::default()
                }))
                .await,
        )
        .expect("nova-meta error response JSON");

        assert_eq!(response["success"], serde_json::json!(false));
        assert_eq!(response["error_code"], serde_json::json!("INVALID_PARAMS"));
        assert!(
            response["error"]
                .as_str()
                .unwrap_or_default()
                .contains("must stay inside project_dir")
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn sql_concurrency_rejects_when_limit_is_saturated() {
        let temp_dir = TempDir::new().expect("temp dir");
        let mut config = test_config(temp_dir.path());
        config.sql_max_concurrent = 1;
        config.sql_max_queue = 0;
        config.sql_queue_timeout_ms = 5;

        let handle = ManifestSearchHandle::spawn(config);
        handle
            .wait_ready()
            .await
            .expect("fixture manifest should load");
        let server = DbtNovaServer::new(handle);
        let searcher = server.searcher.get().await.expect("searcher ready");

        let first_permit = server
            .acquire_sql_permit(searcher.config(), Some(Duration::from_millis(5)))
            .await
            .expect("first SQL permit")
            .expect("permit should be enabled");

        let second_attempt = server
            .acquire_sql_permit(searcher.config(), Some(Duration::from_millis(5)))
            .await;
        let err = second_attempt.expect_err("second SQL permit should be rejected");
        assert!(err.to_string().contains("SQL concurrency limit exceeded"));

        drop(first_permit);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn search_concurrency_rejects_indicator_inventory_when_limit_is_saturated() {
        let temp_dir = TempDir::new().expect("temp dir");
        let mut config = test_config(temp_dir.path());
        config.search.search_max_concurrent = 1;
        config.search.search_max_queue = 0;
        config.search.search_timeout_ms = 50;

        let handle = ManifestSearchHandle::spawn(config);
        handle
            .wait_ready()
            .await
            .expect("fixture manifest should load");
        let server = DbtNovaServer::new(handle);
        let searcher = server.searcher.get().await.expect("searcher ready");

        let first_permit = server
            .acquire_search_permit(searcher.config(), Some(Duration::from_millis(5)))
            .await
            .expect("first search permit")
            .expect("permit should be enabled");

        let response: serde_json::Value = serde_json::from_str(
            &server
                .indicator_inventory(Parameters(IndicatorInventoryParams::default()))
                .await,
        )
        .expect("indicator inventory response JSON");
        assert_eq!(response["success"], serde_json::json!(false));
        assert!(
            response["error"]
                .as_str()
                .unwrap_or_default()
                .contains("Search concurrency limit exceeded")
        );

        drop(first_permit);
    }
}
