use std::collections::BTreeSet;
use std::future::Future;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use rmcp::{
    ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{Implementation, JsonObject, ServerCapabilities, ServerInfo},
    serde_json, tool, tool_handler, tool_router,
};
use tracing::instrument;

use crate::cli::agent_readiness_cmd::build_agent_readiness_tool_response;
use crate::cli::audit_cmd::build_metadata_audit_tool_response;
use crate::cli::config_cmd::{
    build_config_show_tool_response, build_config_validate_tool_response,
};
use crate::cli::eval_cmd::{
    build_agent_eval_tool_response, build_eval_compare_tool_response,
    build_eval_gate_tool_response, build_eval_history_tool_response, build_eval_init_tool_response,
    build_eval_run_tool_response, build_eval_validate_tool_response,
};
use crate::cli::manifest::{
    build_manifest_warm_tool_response, require_mcp_manifest_reload_enabled,
};
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
    BatchGetParams, ColumnInventoryParams, CompareEvalRunsParams, CompareGrainsParams,
    ConfigShowParams, ConfigValidateParams, DiffEntitiesParams, ExecuteSqlParams, FindByPathParams,
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
use crate::responses::{SuccessResponse, attach_response_api_contract};
use crate::server::health::build_manifest_health_payload;
use crate::utils::{ToolMetricsStore, ToolRateLimiter};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

type ToolCallResponse = std::result::Result<String, String>;

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
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("dbt-nova", env!("CARGO_PKG_VERSION")))
            .with_instructions("DBT Manifest Search and Analysis MCP Server. Use 'search' for full-text discovery, 'search_indicator' for canonical measures and metrics, 'get_entity' for complete entity data, and 'execute_sql' to run warehouse queries with the configured SQL provider.")
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
        let value = serde_json::json!({
            "success": false,
            "error": format!("Serialization error: {err}"),
            "error_code": "SERVER_ERROR"
        });
        Self::serialize_unbudgeted_value(value)
    }

    fn error_response(err: &DbtNovaError) -> String {
        Self::serialize_unbudgeted_value(err.to_response())
    }

    fn serialize_unbudgeted_value(mut value: serde_json::Value) -> String {
        attach_response_api_contract(&mut value);
        serde_json::to_string(&value)
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
        let mut value = value;
        attach_response_api_contract(&mut value);
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

    async fn handle_async<F, Fut>(
        &self,
        tool: &'static str,
        persona: Option<&str>,
        f: F,
    ) -> ToolCallResponse
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
    ) -> ToolCallResponse
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
    ) -> ToolCallResponse
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
                return Err(out);
            }
        };

        if !self.check_rate_limit(tool, searcher.config()) {
            let out = Self::serialize_unbudgeted_value(serde_json::json!({
                "success": false,
                "error": format!("Rate limit exceeded for tool '{}'", tool),
                "error_code": "RATE_LIMITED",
            }));
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
            return Err(out);
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
        if success { Ok(out) } else { Err(out) }
    }
}

mod response_budget;
use response_budget::{apply_mcp_next_offset_meta, apply_mcp_response_budget};

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
    ) -> ToolCallResponse
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
    ) -> ToolCallResponse
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
    ) -> ToolCallResponse
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
            Err(response) => return Err(response),
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
    async fn search(&self, params: Parameters<SearchParams>) -> ToolCallResponse {
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
    async fn search_indicator(
        &self,
        params: Parameters<SearchIndicatorParams>,
    ) -> ToolCallResponse {
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
    async fn indicator_inventory(
        &self,
        params: Parameters<IndicatorInventoryParams>,
    ) -> ToolCallResponse {
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
    async fn search_columns(&self, params: Parameters<SearchColumnsParams>) -> ToolCallResponse {
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
    async fn column_inventory(
        &self,
        params: Parameters<ColumnInventoryParams>,
    ) -> ToolCallResponse {
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
    async fn compare_grains(&self, params: Parameters<CompareGrainsParams>) -> ToolCallResponse {
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
    async fn find_entity_overlap(
        &self,
        params: Parameters<FindEntityOverlapParams>,
    ) -> ToolCallResponse {
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
    ) -> ToolCallResponse {
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
    async fn get_entity(&self, params: Parameters<GetEntityParams>) -> ToolCallResponse {
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
    async fn list_entities(&self, params: Parameters<ListEntitiesParams>) -> ToolCallResponse {
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
    async fn get_lineage(&self, params: Parameters<GetLineageParams>) -> ToolCallResponse {
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
    async fn get_sql(&self, params: Parameters<GetSqlParams>) -> ToolCallResponse {
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
    async fn get_columns(&self, params: Parameters<GetColumnsParams>) -> ToolCallResponse {
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
    async fn diff_entities(&self, params: Parameters<DiffEntitiesParams>) -> ToolCallResponse {
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
    async fn get_impact(&self, params: Parameters<GetImpactParams>) -> ToolCallResponse {
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
    async fn validate_dag(&self, params: Parameters<ValidateDagParams>) -> ToolCallResponse {
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
    async fn validate_nova_meta(
        &self,
        params: Parameters<ValidateNovaMetaParams>,
    ) -> ToolCallResponse {
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
    async fn validate_eval_suite(
        &self,
        params: Parameters<ValidateEvalSuiteParams>,
    ) -> ToolCallResponse {
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
    async fn get_eval_gate(&self, params: Parameters<GetEvalGateParams>) -> ToolCallResponse {
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
    async fn get_eval_history(&self, params: Parameters<GetEvalHistoryParams>) -> ToolCallResponse {
        self.handle_async("get_eval_history", None, |_searcher| async move {
            build_eval_history_tool_response(&params.0)
        })
        .await
    }

    /// Compare two eval result directories or results.json files.
    #[tool(
        name = "compare_eval_runs",
        description = "Compare two local eval result directories or results.json files and return PR-ready Markdown plus structured pass-rate, case, and trace-counter deltas. Paths are scoped under the server working directory."
    )]
    #[instrument(level = "info", skip(self, params))]
    async fn compare_eval_runs(
        &self,
        params: Parameters<CompareEvalRunsParams>,
    ) -> ToolCallResponse {
        self.handle_async("compare_eval_runs", None, |_searcher| async move {
            build_eval_compare_tool_response(&params.0)
        })
        .await
    }

    /// Run deterministic bridge evals against the loaded MCP manifest.
    #[tool(
        name = "run_eval",
        description = "Run deterministic bridge eval assertions against the currently loaded MCP manifest. Disabled unless DBT_NOVA_MCP_ENABLE_EVAL_RUN=1 is set."
    )]
    #[instrument(level = "info", skip(self, params))]
    async fn run_eval(&self, params: Parameters<RunEvalParams>) -> ToolCallResponse {
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
    async fn init_eval_suite(&self, params: Parameters<InitEvalSuiteParams>) -> ToolCallResponse {
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
    async fn run_agent_eval(&self, params: Parameters<RunAgentEvalParams>) -> ToolCallResponse {
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
    async fn inspect_tool_trace(&self, params: Parameters<TraceInspectParams>) -> ToolCallResponse {
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
    async fn summarize_tool_trace(
        &self,
        params: Parameters<TraceSummarizeParams>,
    ) -> ToolCallResponse {
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
    async fn redact_tool_trace(&self, params: Parameters<TraceRedactParams>) -> ToolCallResponse {
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
    async fn replay_tool_trace(&self, params: Parameters<TraceReplayParams>) -> ToolCallResponse {
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
    async fn show_metadata(&self) -> ToolCallResponse {
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
    async fn health(&self) -> ToolCallResponse {
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
            Err(err) => return Err(Self::serialization_error_response(&err)),
        };
        if let Some(searcher) = searcher.as_ref() {
            return Self::serialize_budgeted_value(response, searcher.config())
                .map_err(|err| Self::serialization_error_response(&err));
        }
        Ok(Self::serialize_unbudgeted_value(response))
    }

    /// Reload the manifest and rebuild indexes.
    #[tool(
        name = "reload_manifest",
        description = "Reload the current manifest source and rebuild indexes in the background. Source, refresh, or storage changes require DBT_NOVA_MCP_ENABLE_MANIFEST_RELOAD=1."
    )]
    #[instrument(level = "info", skip(self, params))]
    async fn reload_manifest(&self, params: Parameters<ReloadManifestParams>) -> ToolCallResponse {
        let start = Instant::now();
        let mut success = false;
        let reload_params = params.0;
        let out = if let Err(err) = require_mcp_manifest_reload_enabled(&reload_params) {
            Self::error_response(&err)
        } else {
            match self.searcher.reload(&reload_params).await {
                Ok(payload) => match serde_json::to_value(SuccessResponse::new(payload, 1)) {
                    Ok(response) => {
                        if let Ok(searcher) = self.searcher.get().await {
                            match Self::serialize_budgeted_value(response, searcher.config()) {
                                Ok(out) => {
                                    success = true;
                                    out
                                }
                                Err(err) => Self::serialization_error_response(&err),
                            }
                        } else {
                            success = true;
                            Self::serialize_unbudgeted_value(response)
                        }
                    }
                    Err(err) => Self::serialize_unbudgeted_value(serde_json::json!({
                        "success": false,
                        "error": format!("Serialization error: {}", err),
                        "error_code": "SERVER_ERROR"
                    })),
                },
                Err(err) => Self::error_response(&err),
            }
        };
        self.record_metrics(
            "reload_manifest",
            None,
            elapsed_ms_to_u64(start.elapsed()),
            success,
        );
        if success { Ok(out) } else { Err(out) }
    }

    /// Warm semantic caches for the current manifest source.
    #[tool(
        name = "warm_manifest",
        description = "Warm vector/sparse/reranker semantic caches for the current manifest source. Disabled unless DBT_NOVA_MCP_ENABLE_MANIFEST_WARM=1 is set; read-only storage is rejected."
    )]
    #[instrument(level = "info", skip(self, params))]
    async fn warm_manifest(&self, params: Parameters<WarmManifestParams>) -> ToolCallResponse {
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
    async fn show_config(&self, params: Parameters<ConfigShowParams>) -> ToolCallResponse {
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
    async fn validate_config(&self, params: Parameters<ConfigValidateParams>) -> ToolCallResponse {
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
    async fn inspect_storage(&self, params: Parameters<StorageInspectParams>) -> ToolCallResponse {
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
    async fn prune_storage(&self, params: Parameters<StoragePruneParams>) -> ToolCallResponse {
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
    async fn cleanup_storage(&self, params: Parameters<StorageCleanupParams>) -> ToolCallResponse {
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
    async fn list_tags(&self) -> ToolCallResponse {
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
    async fn list_packages(&self) -> ToolCallResponse {
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
    async fn list_databases(&self) -> ToolCallResponse {
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
    async fn get_column_lineage(
        &self,
        params: Parameters<GetColumnLineageParams>,
    ) -> ToolCallResponse {
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
    async fn get_test_coverage(
        &self,
        params: Parameters<GetTestCoverageParams>,
    ) -> ToolCallResponse {
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
    async fn get_metadata_score(
        &self,
        params: Parameters<GetMetadataScoreParams>,
    ) -> ToolCallResponse {
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
    async fn get_metadata_audit(
        &self,
        params: Parameters<GetMetadataAuditParams>,
    ) -> ToolCallResponse {
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
    async fn get_agent_readiness(
        &self,
        params: Parameters<GetAgentReadinessParams>,
    ) -> ToolCallResponse {
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
    async fn batch_get_entities(&self, params: Parameters<BatchGetParams>) -> ToolCallResponse {
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
    async fn find_by_path(&self, params: Parameters<FindByPathParams>) -> ToolCallResponse {
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
    async fn search_recipes(&self, params: Parameters<SearchRecipesParams>) -> ToolCallResponse {
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
    async fn get_recipe(&self, params: Parameters<GetRecipeParams>) -> ToolCallResponse {
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
    async fn run_recipe(&self, params: Parameters<RunRecipeParams>) -> ToolCallResponse {
        let permit = match Self::permit_from_result(self.acquire_sql_permit_for_tool().await) {
            Ok(permit) => permit,
            Err(response) => return Err(response),
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
    async fn get_undocumented(
        &self,
        params: Parameters<GetUndocumentedParams>,
    ) -> ToolCallResponse {
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
    async fn get_context(&self, params: Parameters<GetContextParams>) -> ToolCallResponse {
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
    async fn execute_sql(&self, params: Parameters<ExecuteSqlParams>) -> ToolCallResponse {
        let permit = match Self::permit_from_result(self.acquire_sql_permit_for_tool().await) {
            Ok(permit) => permit,
            Err(response) => return Err(response),
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
mod tests;
