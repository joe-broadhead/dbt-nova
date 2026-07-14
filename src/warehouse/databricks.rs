#![forbid(unsafe_code)]
//! Minimal, production-grade Databricks SQL Statement Execution client.
//!
//! Designed for MCP / agent tooling: safe defaults, structured JSON results,
//! robust polling, and retries for GET/poll calls.
//!
//! Required env vars:
//!   - `DATABRICKS_HOST`
//!   - `DATABRICKS_ACCESS_TOKEN`
//!   - One of:
//!       - `DATABRICKS_HTTP_PATH` (e.g. /sql/1.0/warehouses/<`warehouse_id`>)
//!       - `DATABRICKS_SQL_WAREHOUSE_ID`
//!
//! Optional env vars:
//!   - `DATABRICKS_WAIT_TIMEOUT_S`        (default 10; clamped to 0 or 5–50)
//!   - `DATABRICKS_POLL_INTERVAL_MS`      (default 1000)
//!   - `DATABRICKS_MAX_POLL_SECONDS`      (default 600)
//!   - `DATABRICKS_TIMEOUT_MS`            (default derived from `wait_timeout`, min 30000)
//!   - `DATABRICKS_MAX_GET_RETRIES`       (default 2)

use reqwest::{Client, Method, StatusCode};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map as JsonMap, Value};
use std::collections::{HashMap, HashSet};
use std::env;
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use tracing::{debug, warn};

use crate::error::{DbtNovaError, Result};
use crate::params::ExecuteSqlParams;
use crate::responses::SuccessResponse;
use crate::utils::{redact_sensitive_text, summarize_http_error_body};
use crate::warehouse::SqlProvider;
use crate::warehouse::preflight::{
    PreflightReport, ProbePresence, build_configuration_failure_response, build_preflight_response,
    empty_preflight_probe_message, preflight_probe_has_rows, run_connectivity_check,
    run_optional_object_check,
};

fn dbx_err(message: impl Into<String>) -> DbtNovaError {
    DbtNovaError::DatabricksError {
        message: message.into(),
        status: None,
        body: None,
    }
}

fn dbx_http(status: StatusCode, body: &str) -> DbtNovaError {
    let body_summary = summarize_http_error_body(status.as_u16(), body);
    DbtNovaError::DatabricksError {
        message: format!("Databricks API error (HTTP {status})"),
        status: Some(status.as_u16()),
        body: Some(body_summary),
    }
}

/// Configuration for Databricks SQL statement execution.
#[derive(Debug, Clone)]
pub struct DatabricksSqlConfig {
    pub host: String,
    pub token: String,
    pub warehouse_id: String,

    pub timeout: Duration,

    pub default_wait_timeout_s: u64,
    pub poll_interval: Duration,
    pub max_poll: Duration,

    /// Retries only apply to GET/poll requests (idempotent).
    pub max_get_retries: usize,
}

impl DatabricksSqlConfig {
    /// Build configuration from environment variables.
    ///
    /// # Errors
    /// Returns an error if required environment variables are missing or invalid.
    pub fn from_env() -> Result<Self> {
        let host = env::var("DATABRICKS_HOST")
            .map_err(|_| dbx_err("DATABRICKS_HOST environment variable not set"))?;
        let host = normalize_host(&host)?;

        let token = env::var("DATABRICKS_ACCESS_TOKEN")
            .map_err(|_| dbx_err("Missing DATABRICKS_ACCESS_TOKEN"))?;

        let warehouse_id = resolve_warehouse_id_from_env()?;

        let default_wait_timeout_s = clamp_wait_timeout_s(
            env::var("DATABRICKS_WAIT_TIMEOUT_S")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(10),
        );

        let poll_interval = Duration::from_millis(
            env::var("DATABRICKS_POLL_INTERVAL_MS")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(1000),
        );

        let max_poll = Duration::from_secs(
            env::var("DATABRICKS_MAX_POLL_SECONDS")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(600),
        );

        // Timeout must not undercut wait_timeout; polling GETs should be quick.
        let derived_timeout_ms = (default_wait_timeout_s.saturating_mul(1000))
            .saturating_add(5_000)
            .max(30_000);

        let timeout = Duration::from_millis(
            env::var("DATABRICKS_TIMEOUT_MS")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(derived_timeout_ms),
        );

        let max_get_retries = env::var("DATABRICKS_MAX_GET_RETRIES")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(2);

        Ok(Self {
            host,
            token,
            warehouse_id,
            timeout,
            default_wait_timeout_s,
            poll_interval,
            max_poll,
            max_get_retries,
        })
    }
}

/// Minimal Databricks SQL client for executing statements and polling results.
#[derive(Clone)]
pub struct DatabricksSqlClient {
    http: Client,
    cfg: DatabricksSqlConfig,
}

impl DatabricksSqlClient {
    /// Create a new client from explicit config.
    ///
    /// # Errors
    /// Returns an error if the HTTP client cannot be constructed.
    pub fn new(cfg: DatabricksSqlConfig) -> Result<Self> {
        let http = Client::builder()
            .timeout(cfg.timeout)
            .user_agent(format!("dbt-nova/{}", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| dbx_err(format!("Failed to build reqwest client: {e}")))?;

        Ok(Self { http, cfg })
    }

    /// Create a new client from environment variables.
    ///
    /// # Errors
    /// Returns an error if configuration loading fails.
    pub fn from_env() -> Result<Self> {
        Self::new(DatabricksSqlConfig::from_env()?)
    }

    /// Execute with safe defaults (`row_limit=1000`, `byte_limit=25MB`, `fetch_all_chunks=true`).
    ///
    /// # Errors
    /// Returns an error if statement execution fails or times out.
    pub async fn query(&self, statement: &str) -> Result<QueryResult> {
        self.execute(statement, ExecuteOptions::default()).await
    }

    /// Execute a statement with explicit options.
    ///
    /// # Errors
    /// Returns an error if the statement fails to execute or result polling fails.
    pub async fn execute(&self, statement: &str, opts: ExecuteOptions) -> Result<QueryResult> {
        let start = Instant::now();

        let warehouse_id = opts
            .warehouse_id
            .clone()
            .unwrap_or_else(|| self.cfg.warehouse_id.clone());

        let wait_timeout_s = clamp_wait_timeout_s(
            opts.wait_timeout_s
                .unwrap_or(self.cfg.default_wait_timeout_s),
        );

        let poll_interval = opts.poll_interval.unwrap_or(self.cfg.poll_interval);
        let max_poll = opts.max_poll.unwrap_or(self.cfg.max_poll);

        let req = ExecuteStatementRequest {
            statement: statement.to_string(),
            warehouse_id,

            // Keep it simple for agent results: INLINE + JSON_ARRAY.
            disposition: Some(Disposition::Inline),
            format: Some(ResultFormat::JsonArray),

            // Databricks accepts "0s" or "5s".."50s"
            wait_timeout: Some(format!("{wait_timeout_s}s")),

            // If wait_timeout expires, CONTINUE returns a statement_id for polling.
            on_wait_timeout: Some(opts.on_wait_timeout.unwrap_or(OnWaitTimeout::Continue)),

            byte_limit: opts.byte_limit,
            row_limit: opts.row_limit,

            parameters: if opts.parameters.is_empty() {
                None
            } else {
                Some(opts.parameters.clone())
            },
        };

        // Execute (POST): single attempt (avoid accidental double-exec).
        let mut resp = self.sql_execute_statement(&req).await?;
        let statement_id = resp
            .statement_id
            .clone()
            .ok_or_else(|| dbx_err("Databricks response missing statement_id"))?;

        // If not terminal, poll (GET with retries).
        if !is_terminal_state(&resp.status.state) {
            resp = self
                .poll_statement(&statement_id, poll_interval, max_poll)
                .await?;
        }

        // Terminal handling
        match resp.status.state.as_str() {
            "SUCCEEDED" => {
                let mut out = self.process_success(&statement_id, resp, opts).await?;
                out.elapsed_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
                Ok(out)
            }
            "FAILED" => {
                let msg = format_statement_error(resp.status.error.as_ref());
                Err(dbx_err(format!(
                    "Statement FAILED (id={statement_id}): {msg}"
                )))
            }
            "CANCELED" => Err(dbx_err(format!("Statement CANCELED (id={statement_id})"))),
            other => Err(dbx_err(format!(
                "Unexpected terminal state '{other}' (id={statement_id})"
            ))),
        }
    }

    /// Get full statement status/manifest/result (useful for debugging).
    ///
    /// # Errors
    ///
    /// Returns an error if the Databricks API request fails.
    pub async fn get_statement(&self, statement_id: &str) -> Result<StatementResponse> {
        self.sql_get_statement(statement_id).await
    }

    /// Cancel a running statement.
    ///
    /// # Errors
    ///
    /// Returns an error if the Databricks API request fails.
    pub async fn cancel_statement(&self, statement_id: &str) -> Result<()> {
        self.sql_cancel_statement(statement_id).await
    }

    /* -------------------- Internal: SQL API calls -------------------- */

    fn url(&self, path: &str) -> String {
        if path.starts_with("http://") || path.starts_with("https://") {
            path.to_string()
        } else if path.starts_with('/') {
            format!("{host}{path}", host = self.cfg.host)
        } else {
            format!("{host}/{path}", host = self.cfg.host)
        }
    }

    async fn sql_execute_statement(
        &self,
        req: &ExecuteStatementRequest,
    ) -> Result<StatementResponse> {
        let url = self.url("/api/2.0/sql/statements");
        debug!("POST {}", url);

        let rb = self.http.post(url).bearer_auth(&self.cfg.token).json(req);
        send_json_once(rb).await
    }

    async fn sql_get_statement(&self, statement_id: &str) -> Result<StatementResponse> {
        let url = self.url(&format!("/api/2.0/sql/statements/{statement_id}"));
        debug!("GET {}", url);

        let rb = self.http.get(url).bearer_auth(&self.cfg.token);
        self.send_json_get_with_retries(rb, Method::GET).await
    }

    async fn sql_cancel_statement(&self, statement_id: &str) -> Result<()> {
        let url = self.url(&format!("/api/2.0/sql/statements/{statement_id}/cancel"));
        debug!("POST {}", url);

        let rb = self
            .http
            .post(url)
            .bearer_auth(&self.cfg.token)
            .json(&serde_json::json!({}));
        let resp = rb
            .send()
            .await
            .map_err(|e| dbx_err(format!("Cancel request failed: {e}")))?;
        if resp.status().is_success() {
            return Ok(());
        }
        let status = resp.status();
        let body = match resp.text().await {
            Ok(body) => body,
            Err(err) => format!("<failed to read response body: {err}>"),
        };
        Err(dbx_http(status, &body))
    }

    async fn sql_get_result_chunk(
        &self,
        statement_id: &str,
        chunk_index: u32,
    ) -> Result<ResultData> {
        let url = self.url(&format!(
            "/api/2.0/sql/statements/{statement_id}/result/chunks/{chunk_index}"
        ));
        debug!("GET {}", url);

        let rb = self.http.get(url).bearer_auth(&self.cfg.token);
        self.send_json_get_with_retries(rb, Method::GET).await
    }

    async fn poll_statement(
        &self,
        statement_id: &str,
        poll_interval: Duration,
        max_poll: Duration,
    ) -> Result<StatementResponse> {
        let start = Instant::now();
        loop {
            if start.elapsed() >= max_poll {
                return Err(self.poll_timeout_error(statement_id, max_poll).await);
            }

            let remaining = max_poll
                .checked_sub(start.elapsed())
                .unwrap_or_else(|| Duration::from_secs(0));
            tokio::time::sleep(poll_interval.min(remaining)).await;
            if start.elapsed() >= max_poll {
                return Err(self.poll_timeout_error(statement_id, max_poll).await);
            }

            let resp = self.sql_get_statement(statement_id).await?;
            if is_terminal_state(&resp.status.state) {
                return Ok(resp);
            }
        }
    }

    async fn poll_timeout_error(&self, statement_id: &str, max_poll: Duration) -> DbtNovaError {
        if let Err(err) = self.sql_cancel_statement(statement_id).await {
            warn!(
                statement_id,
                error = %err,
                "failed to cancel Databricks statement after local poll timeout"
            );
        }
        dbx_err(format!(
            "Statement polling timed out after {max_poll:?} (id={statement_id})"
        ))
    }

    async fn process_success(
        &self,
        statement_id: &str,
        resp: StatementResponse,
        opts: ExecuteOptions,
    ) -> Result<QueryResult> {
        let manifest = resp.manifest.as_ref();
        let provider_truncated = manifest.and_then(|m| m.truncated).unwrap_or(false);
        let mut local_truncated = false;

        // Columns/types (be tolerant: DDL can return no manifest).
        let (columns, column_types, stats, chunk_indices) = if let Some(m) = manifest {
            let cols = m
                .schema
                .columns
                .iter()
                .map(|c| c.name.clone())
                .collect::<Vec<_>>();
            let types = m
                .schema
                .columns
                .iter()
                .map(|c| {
                    c.type_name
                        .clone()
                        .or(c.type_text.clone())
                        .unwrap_or_default()
                })
                .collect::<Vec<_>>();

            let stats = QueryStats {
                total_row_count: m.total_row_count,
                total_byte_count: m.total_byte_count,
                total_chunk_count: m.total_chunk_count,
            };

            let mut idxs = m.chunks.iter().map(|c| c.chunk_index).collect::<Vec<_>>();
            idxs.sort_unstable();
            (cols, types, stats, idxs)
        } else {
            (vec![], vec![], QueryStats::default(), vec![])
        };

        let mut rows: Vec<Vec<Value>> = Vec::new();
        let mut approx_bytes = 0u64;
        if let Some(data) = resp.result.as_ref().and_then(|r| r.data_array.as_ref()) {
            append_limited_rows(
                &mut rows,
                data,
                opts.row_limit,
                opts.byte_limit,
                &mut approx_bytes,
                &mut local_truncated,
            )?;
        }

        let mut included_chunks: HashSet<u32> = HashSet::new();
        if let Some(r) = resp.result.as_ref()
            && let Some(ci) = r.chunk_index
        {
            included_chunks.insert(ci);
        }

        let mut fetched_chunks = included_chunks.len() as u64;

        // Optionally fetch remaining chunks (kept small + bounded).
        if opts.fetch_all_chunks && !chunk_indices.is_empty() && !local_truncated {
            let max_chunks = opts.max_chunks.unwrap_or(50);
            for idx in chunk_indices.into_iter().take(max_chunks) {
                if local_truncated {
                    break;
                }
                if included_chunks.contains(&idx) {
                    continue;
                }
                let chunk = self.sql_get_result_chunk(statement_id, idx).await?;
                if let Some(data) = chunk.data_array.as_ref() {
                    append_limited_rows(
                        &mut rows,
                        data,
                        opts.row_limit,
                        opts.byte_limit,
                        &mut approx_bytes,
                        &mut local_truncated,
                    )?;
                }
                included_chunks.insert(idx);
                fetched_chunks += 1;
            }
        }

        Ok(QueryResult {
            statement_id: statement_id.to_string(),
            state: "SUCCEEDED".to_string(),
            truncated: provider_truncated || local_truncated,

            columns,
            column_types,
            rows,

            stats,
            fetched_chunks,
            elapsed_ms: 0, // filled by caller
        })
    }

    async fn send_json_get_with_retries<T: DeserializeOwned>(
        &self,
        builder: reqwest::RequestBuilder,
        method: Method,
    ) -> Result<T> {
        let attempts = self.cfg.max_get_retries.saturating_add(1);

        for attempt in 1..=attempts {
            let b = builder.try_clone().ok_or_else(|| {
                dbx_err("Request cannot be retried (reqwest builder not cloneable)")
            })?;

            match b.send().await {
                Ok(resp) => {
                    let status = resp.status();
                    if status.is_success() {
                        return resp.json::<T>().await.map_err(|e| {
                            dbx_err(format!("Failed parsing JSON (HTTP {status}): {e}"))
                        });
                    }

                    // Retry on common transient statuses
                    let retryable = matches!(
                        status,
                        StatusCode::TOO_MANY_REQUESTS
                            | StatusCode::INTERNAL_SERVER_ERROR
                            | StatusCode::BAD_GATEWAY
                            | StatusCode::SERVICE_UNAVAILABLE
                            | StatusCode::GATEWAY_TIMEOUT
                    );

                    if attempt < attempts && retryable {
                        let backoff = backoff_delay(attempt);
                        warn!(
                            "GET failed HTTP {} (attempt {}/{}), retrying in {:?}",
                            status, attempt, attempts, backoff
                        );
                        tokio::time::sleep(backoff).await;
                        continue;
                    }

                    let body = match resp.text().await {
                        Ok(body) => body,
                        Err(err) => format!("<failed to read response body: {err}>"),
                    };
                    return Err(dbx_http(status, &body));
                }
                Err(e) => {
                    // Retry on transient network/timeouts
                    let retryable = e.is_timeout() || e.is_connect() || e.is_request();
                    if attempt < attempts && retryable {
                        let backoff = backoff_delay(attempt);
                        warn!(
                            "GET request error (attempt {}/{}): {}. Retrying in {:?}",
                            attempt, attempts, e, backoff
                        );
                        tokio::time::sleep(backoff).await;
                        continue;
                    }
                    return Err(dbx_err(format!("Request failed: {e}")));
                }
            }
        }

        Err(dbx_err(format!(
            "Unreachable: retry loop exhausted for {method:?}"
        )))
    }
}

/* -------------------- Public request/response types -------------------- */

/// Options that control Databricks SQL execution and polling behavior.
#[derive(Debug, Clone)]
pub struct ExecuteOptions {
    /// Override `warehouse_id` for this call (can also be an `http_path`).
    pub warehouse_id: Option<String>,

    pub wait_timeout_s: Option<u64>,
    pub on_wait_timeout: Option<OnWaitTimeout>,

    pub row_limit: Option<u64>,
    pub byte_limit: Option<u64>,

    pub parameters: Vec<StatementParameter>,

    pub poll_interval: Option<Duration>,
    pub max_poll: Option<Duration>,

    /// If true, fetch remaining chunks via `/result/chunks/*` (bounded by `max_chunks`).
    pub fetch_all_chunks: bool,
    pub max_chunks: Option<usize>,
}

impl Default for ExecuteOptions {
    fn default() -> Self {
        Self {
            warehouse_id: None,

            wait_timeout_s: None,
            on_wait_timeout: None,

            // Safe defaults for agents: avoid huge payloads.
            row_limit: Some(1000),
            byte_limit: Some(25_000_000),

            parameters: vec![],

            poll_interval: None,
            max_poll: None,

            fetch_all_chunks: true,
            max_chunks: Some(50),
        }
    }
}

/// Finalized query result with rows, schema, and execution stats.
#[derive(Debug, Serialize)]
pub struct QueryResult {
    pub statement_id: String,
    pub state: String,

    pub truncated: bool,

    pub columns: Vec<String>,
    pub column_types: Vec<String>,
    pub rows: Vec<Vec<Value>>,

    pub stats: QueryStats,

    /// How many chunks were actually fetched/merged.
    pub fetched_chunks: u64,

    /// Total wall time for execute+poll+fetch.
    pub elapsed_ms: u64,
}

/// Optional stats returned by Databricks for a statement.
#[derive(Debug, Serialize, Default)]
pub struct QueryStats {
    pub total_row_count: Option<u64>,
    pub total_byte_count: Option<u64>,
    pub total_chunk_count: Option<u64>,
}

/* -------------------- Internal API models -------------------- */

#[derive(Debug, Serialize)]
struct ExecuteStatementRequest {
    statement: String,
    warehouse_id: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    format: Option<ResultFormat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    disposition: Option<Disposition>,

    #[serde(skip_serializing_if = "Option::is_none")]
    wait_timeout: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    on_wait_timeout: Option<OnWaitTimeout>,

    #[serde(skip_serializing_if = "Option::is_none")]
    byte_limit: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    row_limit: Option<u64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    parameters: Option<Vec<StatementParameter>>,
}

/// Named parameter passed to Databricks SQL statements.
#[derive(Debug, Clone, Serialize)]
pub struct StatementParameter {
    pub name: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,

    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub type_: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum ResultFormat {
    JsonArray,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum Disposition {
    Inline,
}

/// Behavior when wait timeout is reached.
#[derive(Debug, Serialize, Deserialize, Clone, Copy)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum OnWaitTimeout {
    Continue,
    Cancel,
}

/// Databricks SQL statement response payload.
#[derive(Debug, Deserialize)]
pub struct StatementResponse {
    #[serde(default)]
    pub statement_id: Option<String>,

    pub status: StatementStatus,

    #[serde(default)]
    pub manifest: Option<StatementManifest>,

    #[serde(default)]
    pub result: Option<ResultData>,
}

/// Databricks statement status.
#[derive(Debug, Deserialize)]
pub struct StatementStatus {
    pub state: String,

    #[serde(default)]
    pub error: Option<StatementError>,
}

/// Databricks error metadata for failed statements.
#[derive(Debug, Deserialize)]
pub struct StatementError {
    #[serde(default)]
    pub error_code: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
}

/// Statement manifest describing schema and chunk layout.
#[derive(Debug, Deserialize)]
pub struct StatementManifest {
    pub schema: StatementSchema,

    #[serde(default)]
    pub chunks: Vec<ChunkInfo>,

    #[serde(default)]
    pub truncated: Option<bool>,

    #[serde(default)]
    pub total_chunk_count: Option<u64>,
    #[serde(default)]
    pub total_row_count: Option<u64>,
    #[serde(default)]
    pub total_byte_count: Option<u64>,
}

/// Column schema information for result sets.
#[derive(Debug, Deserialize)]
pub struct StatementSchema {
    pub columns: Vec<StatementColumn>,
}

/// Column descriptor (name and optional type metadata).
#[derive(Debug, Deserialize)]
pub struct StatementColumn {
    pub name: String,

    #[serde(default)]
    pub type_name: Option<String>,
    #[serde(default)]
    pub type_text: Option<String>,
}

/// Chunk reference for segmented results.
#[derive(Debug, Deserialize)]
pub struct ChunkInfo {
    pub chunk_index: u32,
}

/// Result rows payload for a statement (inline or chunked).
#[derive(Debug, Deserialize)]
pub struct ResultData {
    #[serde(default)]
    pub chunk_index: Option<u32>,

    #[serde(default)]
    pub data_array: Option<Vec<Vec<Value>>>,
}

/* -------------------- Small utilities -------------------- */

async fn send_json_once<T: DeserializeOwned>(builder: reqwest::RequestBuilder) -> Result<T> {
    let resp = builder
        .send()
        .await
        .map_err(|e| dbx_err(format!("Request failed: {e}")))?;
    let status = resp.status();
    if status.is_success() {
        return resp
            .json::<T>()
            .await
            .map_err(|e| dbx_err(format!("Failed parsing JSON (HTTP {status}): {e}")));
    }
    let body = match resp.text().await {
        Ok(body) => body,
        Err(err) => format!("<failed to read response body: {err}>"),
    };
    Err(dbx_http(status, &body))
}

fn normalize_host(host: &str) -> Result<String> {
    let trimmed = host.trim();
    if trimmed.is_empty() {
        return Err(dbx_err("DATABRICKS_HOST is empty"));
    }
    let mut h = if trimmed.starts_with("https://") || trimmed.starts_with("http://") {
        trimmed.to_string()
    } else {
        format!("https://{trimmed}")
    };
    while h.ends_with('/') {
        h.pop();
    }
    Ok(h)
}

fn resolve_warehouse_id_from_env() -> Result<String> {
    if let Ok(w) = env::var("DATABRICKS_SQL_WAREHOUSE_ID") {
        return normalize_warehouse_id(&w);
    }
    if let Ok(p) = env::var("DATABRICKS_HTTP_PATH") {
        return normalize_warehouse_id(&p);
    }
    Err(dbx_err(
        "No warehouse id/http path provided (set DATABRICKS_HTTP_PATH or DATABRICKS_SQL_WAREHOUSE_ID)",
    ))
}

fn normalize_warehouse_id(input: &str) -> Result<String> {
    let trimmed = input.trim().trim_matches('/');
    if trimmed.is_empty() {
        return Err(dbx_err("warehouse id/http path is empty"));
    }
    if trimmed.contains('/') {
        return extract_warehouse_id(trimmed).ok_or_else(|| {
            dbx_err("warehouse id/http path must be <warehouse_id> or /sql/1.0/warehouses/<warehouse_id>")
        });
    }
    Ok(trimmed.to_string())
}

fn extract_warehouse_id(http_path: &str) -> Option<String> {
    let parts: Vec<&str> = http_path.trim_matches('/').split('/').collect();

    // /sql/1.0/warehouses/{warehouse_id}
    if parts.len() >= 4 && parts[0] == "sql" && parts.get(2) == Some(&"warehouses") {
        return Some(parts[3].to_string());
    }

    None
}

fn clamp_wait_timeout_s(v: u64) -> u64 {
    match v {
        0 => 0,
        1..=4 => 5,
        5..=50 => v,
        _ => 50,
    }
}

fn is_terminal_state(state: &str) -> bool {
    matches!(state, "SUCCEEDED" | "FAILED" | "CANCELED" | "CLOSED")
}

fn format_statement_error(err: Option<&StatementError>) -> String {
    match err {
        None => "Unknown error".to_string(),
        Some(e) => match (&e.error_code, &e.message) {
            (Some(code), Some(msg)) if !msg.trim().is_empty() => {
                format!("{}: message redacted", redact_sensitive_text(code))
            }
            (Some(code), _) => redact_sensitive_text(code),
            (None, Some(msg)) if !msg.trim().is_empty() => "message redacted".to_string(),
            (None, Some(_) | None) => "Unknown error".to_string(),
        },
    }
}

fn backoff_delay(attempt_1_based: usize) -> Duration {
    // Simple exponential backoff, capped.
    let base_ms = 200u64;
    let pow = u32::try_from((attempt_1_based.saturating_sub(1)).min(6)).unwrap_or(0);
    let ms = base_ms.saturating_mul(2u64.saturating_pow(pow)).min(5_000);
    Duration::from_millis(ms)
}

fn append_limited_rows(
    rows: &mut Vec<Vec<Value>>,
    chunk_rows: &[Vec<Value>],
    row_limit: Option<u64>,
    byte_limit: Option<u64>,
    approx_bytes: &mut u64,
    truncated: &mut bool,
) -> Result<()> {
    for row in chunk_rows {
        if let Some(limit) = row_limit {
            let row_count = u64::try_from(rows.len()).unwrap_or(u64::MAX);
            if row_count >= limit {
                *truncated = true;
                break;
            }
        }

        let row_bytes = u64::try_from(
            serde_json::to_vec(row)
                .map_err(|err| dbx_err(format!("failed to serialize result row: {err}")))?
                .len(),
        )
        .unwrap_or(u64::MAX);

        if let Some(limit) = byte_limit
            && approx_bytes.saturating_add(row_bytes) > limit
        {
            *truncated = true;
            break;
        }

        *approx_bytes = approx_bytes.saturating_add(row_bytes);
        rows.push(row.clone());
    }
    Ok(())
}

/// Build Databricks SQL statement parameters from values and optional types.
///
/// # Errors
///
/// Returns an error if type annotations reference missing parameters.
#[allow(clippy::implicit_hasher)]
pub fn build_parameters(
    params: Option<HashMap<String, Value>>,
    types: Option<HashMap<String, String>>,
) -> Result<Vec<StatementParameter>> {
    let params = params.unwrap_or_default();
    let types = types.unwrap_or_default();

    // Validate: types keys must be present in params (helps catch mistakes).
    for k in types.keys() {
        if !params.contains_key(k) {
            return Err(DbtNovaError::InvalidParams(format!(
                "parameter_types contains '{k}' but parameters does not. Provide a value for it."
            )));
        }
    }

    let mut out = Vec::with_capacity(params.len());
    for (name, value) in params {
        let param_type = types.get(&name).cloned();
        out.push(StatementParameter {
            name,
            value: Some(value),
            type_: param_type,
        });
    }
    Ok(out)
}

pub struct DatabricksProvider;

pub static DATABRICKS_PROVIDER: DatabricksProvider = DatabricksProvider;

impl SqlProvider for DatabricksProvider {
    fn name(&self) -> &'static str {
        "databricks"
    }

    fn execute<'a>(
        &'a self,
        params: &'a ExecuteSqlParams,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value>> + Send + 'a>> {
        Box::pin(async move { execute_sql_internal(params).await })
    }

    fn preflight<'a>(
        &'a self,
        params: &'a ExecuteSqlParams,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value>> + Send + 'a>> {
        Box::pin(async move { preflight_databricks(params).await })
    }
}

static DATABRICKS_CLIENT: OnceLock<DatabricksSqlClient> = OnceLock::new();

fn databricks_client() -> Result<&'static DatabricksSqlClient> {
    if let Some(client) = DATABRICKS_CLIENT.get() {
        return Ok(client);
    }
    let client = DatabricksSqlClient::from_env().map_err(|err| DbtNovaError::DatabricksError {
        message: err.to_string(),
        status: None,
        body: None,
    })?;
    if DATABRICKS_CLIENT.set(client).is_err() {
        // Another thread initialized first.
    }
    DATABRICKS_CLIENT
        .get()
        .ok_or_else(|| dbx_err("Databricks client initialization failed unexpectedly"))
}

async fn execute_sql_internal(params: &ExecuteSqlParams) -> Result<Value> {
    let statement = params.statement.trim();
    let client = databricks_client()?;

    let mut opts = ExecuteOptions::default();
    opts.warehouse_id = params.warehouse_id.clone();
    opts.row_limit = params.row_limit.or(opts.row_limit);
    opts.byte_limit = params.byte_limit.or(opts.byte_limit);
    opts.wait_timeout_s = params.wait_timeout_s;
    if let Some(ms) = params.poll_interval_ms {
        opts.poll_interval = Some(Duration::from_millis(ms));
    }
    if let Some(s) = params.max_poll_seconds {
        opts.max_poll = Some(Duration::from_secs(s));
    }
    if let Some(v) = params.fetch_all_chunks {
        opts.fetch_all_chunks = v;
    }
    if let Some(v) = params.max_chunks {
        opts.max_chunks = Some(v);
    }

    opts.parameters = build_parameters(params.parameters.clone(), params.parameter_types.clone())?;

    let result = client.execute(statement, opts).await?;
    let count = result.rows.len();
    Ok(serde_json::to_value(SuccessResponse::new(result, count))?)
}

fn normalize_preflight_identifier(segment: &str, context: &str) -> Result<String> {
    let trimmed = segment.trim();
    if trimmed.is_empty()
        || !trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
        || !trimmed
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
    {
        return Err(DbtNovaError::InvalidParams(format!(
            "Invalid {context} identifier segment '{segment}'"
        )));
    }
    Ok(trimmed.to_string())
}

fn normalize_preflight_relation(name: &str) -> Result<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(DbtNovaError::InvalidParams(
            "preflight_relation cannot be empty".to_string(),
        ));
    }

    let parts: Vec<&str> = trimmed.split('.').collect();
    if parts.is_empty() || parts.len() > 3 {
        return Err(DbtNovaError::InvalidParams(format!(
            "Invalid relation '{trimmed}': expected table, schema.table, or catalog.schema.table"
        )));
    }

    let mut normalized = Vec::with_capacity(parts.len());
    for part in parts {
        normalized.push(normalize_preflight_identifier(part, "relation")?);
    }
    Ok(normalized.join("."))
}

fn catalog_preflight_statement(catalog: &str) -> String {
    format!("SHOW SCHEMAS IN {catalog}")
}

fn schema_preflight_statement(schema: &str) -> String {
    format!("SHOW TABLES IN {schema}")
}

fn relation_preflight_statement(relation: &str) -> String {
    format!("SELECT 1 AS relation_access_check FROM {relation} LIMIT 1")
}

async fn run_preflight_statement(
    client: &DatabricksSqlClient,
    statement: &str,
    warehouse_id: Option<String>,
) -> Result<QueryResult> {
    let opts = ExecuteOptions {
        warehouse_id,
        row_limit: Some(1),
        byte_limit: Some(1024),
        wait_timeout_s: Some(10),
        max_poll: Some(Duration::from_secs(30)),
        fetch_all_chunks: false,
        max_chunks: Some(1),
        ..ExecuteOptions::default()
    };
    client.execute(statement, opts).await
}

fn preflight_result_has_rows(result: &QueryResult) -> bool {
    preflight_probe_has_rows(result.rows.len(), result.stats.total_row_count)
}

fn detail_field(key: &str, value: impl AsRef<str>) -> JsonMap<String, Value> {
    let mut details = JsonMap::new();
    details.insert(key.to_string(), Value::String(value.as_ref().to_string()));
    details
}

#[allow(clippy::too_many_lines)]
async fn preflight_databricks(params: &ExecuteSqlParams) -> Result<Value> {
    let warehouse_id = params.warehouse_id.clone();
    let mut metadata = JsonMap::new();
    metadata.insert(
        "warehouse_id".to_string(),
        warehouse_id.clone().map_or(Value::Null, Value::String),
    );

    let client = match databricks_client() {
        Ok(client) => client,
        Err(err) => {
            return build_configuration_failure_response(
                "databricks",
                metadata,
                err.to_string(),
                "Set DATABRICKS_HOST, DATABRICKS_ACCESS_TOKEN, and DATABRICKS_HTTP_PATH or DATABRICKS_SQL_WAREHOUSE_ID",
            );
        }
    };

    let check_warehouse = warehouse_id
        .clone()
        .or_else(|| Some(client.cfg.warehouse_id.clone()));
    metadata.insert(
        "warehouse_id".to_string(),
        check_warehouse.clone().map_or(Value::Null, Value::String),
    );
    let mut report = PreflightReport::new();

    run_connectivity_check(
        &mut report,
        "Verify warehouse is running and credentials allow SQL execution",
        || async {
            run_preflight_statement(
                client,
                "SELECT 1 AS connectivity_check",
                check_warehouse.clone(),
            )
            .await
            .map(|_| ())
        },
    )
    .await;

    run_optional_object_check(
        &mut report,
        params.preflight_catalog.as_deref(),
        "catalog_access",
        |catalog| normalize_preflight_identifier(catalog, "catalog"),
        |catalog| {
            let catalog = catalog.clone();
            let warehouse = check_warehouse.clone();
            async move {
                let statement = catalog_preflight_statement(&catalog);
                let result = run_preflight_statement(client, &statement, warehouse).await?;
                Ok(if preflight_result_has_rows(&result) {
                    ProbePresence::Present
                } else {
                    ProbePresence::Empty
                })
            }
        },
        |catalog| detail_field("catalog", catalog),
        |catalog| detail_field("catalog", catalog),
        "Use an unquoted catalog identifier (letters, digits, _, $)",
        "Verify catalog exists and token has USE CATALOG permissions",
        &empty_preflight_probe_message("catalog_access"),
    )
    .await;

    run_optional_object_check(
        &mut report,
        params.preflight_schema.as_deref(),
        "schema_access",
        |schema| normalize_preflight_identifier(schema, "schema"),
        |schema| {
            let schema = schema.clone();
            let warehouse = check_warehouse.clone();
            async move {
                let statement = schema_preflight_statement(&schema);
                let result = run_preflight_statement(client, &statement, warehouse).await?;
                Ok(if preflight_result_has_rows(&result) {
                    ProbePresence::Present
                } else {
                    ProbePresence::Empty
                })
            }
        },
        |schema| detail_field("schema", schema),
        |schema| detail_field("schema", schema),
        "Use an unquoted schema identifier (letters, digits, _, $)",
        "Verify schema exists and token has USE SCHEMA permissions",
        &empty_preflight_probe_message("schema_access"),
    )
    .await;

    run_optional_object_check(
        &mut report,
        params.preflight_relation.as_deref(),
        "relation_access",
        normalize_preflight_relation,
        |relation| {
            let relation = relation.clone();
            let warehouse = check_warehouse.clone();
            async move {
                let statement = relation_preflight_statement(&relation);
                let result = run_preflight_statement(client, &statement, warehouse).await?;
                Ok(if preflight_result_has_rows(&result) {
                    ProbePresence::Present
                } else {
                    ProbePresence::Empty
                })
            }
        },
        |relation| detail_field("relation", relation),
        |relation| detail_field("relation", relation),
        "Use unquoted identifiers like table, schema.table, or catalog.schema.table",
        "Verify relation exists and has SELECT permissions for this warehouse",
        &empty_preflight_probe_message("relation_access"),
    )
    .await;

    build_preflight_response("databricks", metadata, report)
}

#[cfg(test)]
mod tests {
    use super::{
        QueryResult, QueryStats, StatementError, catalog_preflight_statement, dbx_http,
        format_statement_error, normalize_preflight_relation, normalize_warehouse_id,
        preflight_result_has_rows, relation_preflight_statement, schema_preflight_statement,
    };
    use reqwest::StatusCode;
    use serde_json::Value;

    fn sample_preflight_result(rows: Vec<Vec<Value>>, total_row_count: Option<u64>) -> QueryResult {
        QueryResult {
            statement_id: "statement-id".to_string(),
            state: "SUCCEEDED".to_string(),
            truncated: false,
            columns: Vec::new(),
            column_types: Vec::new(),
            rows,
            stats: QueryStats {
                total_row_count,
                total_byte_count: None,
                total_chunk_count: None,
            },
            fetched_chunks: 0,
            elapsed_ms: 0,
        }
    }

    #[test]
    fn normalize_warehouse_id_accepts_raw_id() {
        let got = normalize_warehouse_id("abc123").expect("raw id should parse");
        assert_eq!(got, "abc123");
    }

    #[test]
    fn databricks_http_errors_do_not_expose_raw_body() {
        let err = dbx_http(
            StatusCode::BAD_REQUEST,
            r#"{"error_code":"INVALID_PARAMETER_VALUE","message":"Bad SQL select * from users where token='raw-token'"}"#,
        );
        let response = err.to_response();
        let serialized = response.to_string();

        assert_eq!(response["status"], serde_json::json!(400));
        assert!(
            response["body"]
                .as_str()
                .unwrap_or_default()
                .contains("code=INVALID_PARAMETER_VALUE")
        );
        assert!(!serialized.contains("raw-token"));
        assert!(!serialized.contains("select *"));
        assert!(!serialized.contains("users"));
    }

    #[test]
    fn databricks_statement_errors_do_not_expose_raw_message() {
        let err = StatementError {
            error_code: Some("BAD_REQUEST".to_string()),
            message: Some("Bad SQL select * from users where token='raw-token'".to_string()),
        };
        let formatted = format_statement_error(Some(&err));

        assert_eq!(formatted, "BAD_REQUEST: message redacted");
        assert!(!formatted.contains("raw-token"));
        assert!(!formatted.contains("select *"));
        assert!(!formatted.contains("users"));
    }

    #[test]
    fn normalize_warehouse_id_extracts_modern_http_path() {
        let got = normalize_warehouse_id("/sql/1.0/warehouses/abc123").expect("path should parse");
        assert_eq!(got, "abc123");
    }

    #[test]
    fn normalize_warehouse_id_rejects_legacy_protocolv1_path() {
        let err = normalize_warehouse_id("/sql/protocolv1/o/100/abc123")
            .expect_err("legacy protocolv1 path should be rejected");
        assert!(
            err.to_string().contains("warehouse id/http path must be"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn normalize_preflight_relation_accepts_three_part_name() {
        let relation = normalize_preflight_relation("main.analytics.orders")
            .expect("three-part relation should parse");
        assert_eq!(relation, "main.analytics.orders");
    }

    #[test]
    fn normalize_preflight_relation_rejects_invalid_identifier() {
        let err = normalize_preflight_relation("orders;drop")
            .expect_err("invalid relation should fail validation");
        assert!(
            err.to_string().contains("Invalid relation"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn catalog_preflight_statement_uses_valid_show_syntax() {
        let statement = catalog_preflight_statement("hive_metastore");
        assert_eq!(statement, "SHOW SCHEMAS IN hive_metastore");
        assert!(
            !statement.to_ascii_uppercase().contains("LIMIT"),
            "catalog preflight statement must not include LIMIT"
        );
    }

    #[test]
    fn schema_preflight_statement_uses_valid_show_syntax() {
        let statement = schema_preflight_statement("analytics_reporting");
        assert_eq!(statement, "SHOW TABLES IN analytics_reporting");
        assert!(
            !statement.to_ascii_uppercase().contains("LIMIT"),
            "schema preflight statement must not include LIMIT"
        );
    }

    #[test]
    fn relation_preflight_statement_keeps_limit() {
        let statement = relation_preflight_statement("hive_metastore.schema.orders");
        assert_eq!(
            statement,
            "SELECT 1 AS relation_access_check FROM hive_metastore.schema.orders LIMIT 1"
        );
    }

    #[test]
    fn preflight_result_has_rows_accepts_materialized_rows() {
        let result = sample_preflight_result(vec![vec![Value::from(1)]], None);
        assert!(preflight_result_has_rows(&result));
    }

    #[test]
    fn preflight_result_has_rows_accepts_total_row_count_without_rows() {
        let result = sample_preflight_result(Vec::new(), Some(1));
        assert!(preflight_result_has_rows(&result));
    }

    #[test]
    fn preflight_result_has_rows_rejects_empty_probe() {
        let result = sample_preflight_result(Vec::new(), Some(0));
        assert!(!preflight_result_has_rows(&result));
    }
}
