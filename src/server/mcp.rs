use std::collections::BTreeSet;
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

use crate::error::DbtNovaError;
use crate::manifest::search::{ManifestSearch, ManifestSearchHandle};
use crate::params::{
    BatchGetParams, ColumnInventoryParams, CompareGrainsParams, DiffEntitiesParams,
    ExecuteSqlParams, FindByPathParams, FindEntityOverlapParams, GetColumnLineageParams,
    GetColumnsParams, GetContextParams, GetEntityParams, GetImpactParams, GetLineageParams,
    GetMetadataScoreParams, GetRecipeParams, GetSqlParams, GetTestCoverageParams,
    GetUndocumentedParams, IndicatorInventoryParams, ListEntitiesParams,
    ModellingConsistencyReportParams, ReloadManifestParams, RunRecipeParams, SearchColumnsParams,
    SearchIndicatorParams, SearchParams, SearchRecipesParams, ValidateDagParams,
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
        let start = Instant::now();
        let mut success = false;
        let searcher = match self.searcher.get().await {
            Ok(searcher) => searcher,
            Err(err) => {
                let out = Self::error_response(&err);
                self.record_metrics(tool, persona, elapsed_ms_to_u64(start.elapsed()), success);
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
            self.record_metrics(tool, persona, elapsed_ms_to_u64(start.elapsed()), success);
            return out;
        }

        let out = match f(searcher).await {
            Ok(v) => match serde_json::to_string(&v) {
                Ok(out) => {
                    success = true;
                    out
                }
                Err(e) => Self::serialization_error_response(&e),
            },
            Err(e) => Self::error_response(&e),
        };
        self.record_metrics(tool, persona, elapsed_ms_to_u64(start.elapsed()), success);
        out
    }

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
            .handle_async(tool, persona, |searcher| async move {
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
        self.handle_bounded_search("search", persona.as_deref(), |searcher| async move {
            searcher.search(&params.0).await
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
        self.handle_bounded_search(
            "search_indicator",
            persona.as_deref(),
            |searcher| async move { searcher.search_indicator(&params.0).await },
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
        self.handle_bounded_search("indicator_inventory", None, |searcher| async move {
            searcher.indicator_inventory(&params.0).await
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
        self.handle_bounded_search("search_columns", None, |searcher| async move {
            searcher.search_columns(&params.0).await
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
        self.handle_bounded_search("column_inventory", None, |searcher| async move {
            searcher.column_inventory(&params.0).await
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
        self.handle_bounded_search("find_entity_overlap", None, |searcher| async move {
            searcher.find_entity_overlap(&params.0).await
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
        self.handle_bounded_search(
            "modelling_consistency_report",
            None,
            |searcher| async move { searcher.modelling_consistency_report(&params.0).await },
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
            searcher.get_entity_data(&params.0).await
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
        self.handle_async("list_entities", None, |searcher| async move {
            searcher.list_entities(&params.0).await
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
            searcher.get_lineage(&params.0).await
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
        if let Some(base) = payload.as_object_mut() {
            if let Ok(searcher) = self.searcher.get().await {
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
        match serde_json::to_string(&SuccessResponse::new(payload, 1)) {
            Ok(out) => out,
            Err(err) => serde_json::json!({
                "success": false,
                "error": format!("Serialization error: {}", err),
                "error_code": "SERVER_ERROR"
            })
            .to_string(),
        }
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
            Ok(payload) => match serde_json::to_string(&SuccessResponse::new(payload, 1)) {
                Ok(out) => {
                    success = true;
                    out
                }
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
        self.handle_async("get_metadata_score", None, |searcher| async move {
            searcher.get_metadata_score(&params.0).await
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
            searcher.batch_get_entities(&params.0).await
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
        self.handle_async("find_by_path", None, |searcher| async move {
            searcher.find_by_path(&params.0).await
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
        self.handle_async("search_recipes", None, |searcher| async move {
            searcher.search_recipes(&params.0).await
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
        self.handle_async("get_undocumented", None, |searcher| async move {
            searcher.get_undocumented(&params.0).await
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
    use std::path::Path;

    use tempfile::TempDir;

    use super::*;
    use crate::tests::common::fixture_manifest_path_string;

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
