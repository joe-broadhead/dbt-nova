#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::env;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use reqwest::{Client, StatusCode, Url};
use serde::{Deserialize, Serialize};
use serde_json::{Map as JsonMap, Value};
use sha2::{Digest, Sha256};
use simple_asn1::{ASN1Block, BigInt, oid};
use tokio::time::sleep;
use tracing::warn;

use crate::error::{DbtNovaError, Result};
use crate::params::ExecuteSqlParams;
use crate::responses::SuccessResponse;
use crate::warehouse::SqlProvider;
use crate::warehouse::preflight::{
    PreflightReport, ProbePresence, build_configuration_failure_response, build_preflight_response,
    empty_preflight_probe_message, preflight_probe_has_rows, run_connectivity_check,
    run_optional_object_check,
};

const DEFAULT_ROW_LIMIT: u64 = 1_000;
const DEFAULT_BYTE_LIMIT: u64 = 25_000_000;
const DEFAULT_TIMEOUT_MS: u64 = 30_000;
const DEFAULT_STATEMENT_TIMEOUT_S: u64 = 60;
const DEFAULT_POLL_INTERVAL_MS: u64 = 1_000;
const DEFAULT_MAX_POLL_SECONDS: u64 = 600;
const DEFAULT_MAX_CHUNKS: usize = 50;
const DEFAULT_JWT_LIFETIME_SECONDS: u64 = 3_300;

fn snowflake_err(message: impl Into<String>) -> DbtNovaError {
    DbtNovaError::ServerError(format!("Snowflake error: {}", message.into()))
}

fn snowflake_http(status: StatusCode, body: &str) -> DbtNovaError {
    DbtNovaError::ServerError(format!(
        "Snowflake API error (HTTP {}): {}",
        status.as_u16(),
        summarize_error_body(status, body)
    ))
}

#[derive(Clone)]
enum SnowflakeAuthConfig {
    KeyPair {
        user: String,
        account_identifier: String,
        private_key_pem: String,
    },
    OAuth {
        token: String,
    },
    ProgrammaticAccessToken {
        token: String,
    },
}

/// Configuration for Snowflake SQL API execution.
#[derive(Clone)]
pub struct SnowflakeSqlConfig {
    pub base_url: String,
    pub warehouse: String,
    pub database: Option<String>,
    pub schema: Option<String>,
    pub role: Option<String>,
    pub timeout: Duration,
    pub default_statement_timeout_s: u64,
    pub poll_interval: Duration,
    pub max_poll: Duration,
    pub max_chunks: usize,
    auth: SnowflakeAuthConfig,
}

impl SnowflakeSqlConfig {
    /// Build configuration from environment variables.
    ///
    /// # Errors
    /// Returns an error when required Snowflake configuration or credentials are missing.
    pub fn from_env() -> Result<Self> {
        let (base_url, account_identifier) = resolve_base_url_from_env()?;

        let warehouse = read_required_env(
            "DBT_NOVA_SNOWFLAKE_WAREHOUSE",
            "DBT_NOVA_SNOWFLAKE_WAREHOUSE is required when DBT_NOVA_SQL_PROVIDER=snowflake",
        )?;

        let database = read_optional_env("DBT_NOVA_SNOWFLAKE_DATABASE");
        let schema = read_optional_env("DBT_NOVA_SNOWFLAKE_SCHEMA");
        let role = read_optional_env("DBT_NOVA_SNOWFLAKE_ROLE");

        let auth = resolve_auth_from_env(account_identifier)?;

        let timeout = Duration::from_millis(
            env_u64("DBT_NOVA_SNOWFLAKE_TIMEOUT_MS", DEFAULT_TIMEOUT_MS).max(1_000),
        );
        let default_statement_timeout_s = env_u64(
            "DBT_NOVA_SNOWFLAKE_STATEMENT_TIMEOUT_S",
            DEFAULT_STATEMENT_TIMEOUT_S,
        )
        .max(1);
        let poll_interval = Duration::from_millis(
            env_u64(
                "DBT_NOVA_SNOWFLAKE_POLL_INTERVAL_MS",
                DEFAULT_POLL_INTERVAL_MS,
            )
            .max(1),
        );
        let max_poll = Duration::from_secs(
            env_u64(
                "DBT_NOVA_SNOWFLAKE_MAX_POLL_SECONDS",
                DEFAULT_MAX_POLL_SECONDS,
            )
            .max(1),
        );
        let max_chunks = env_usize("DBT_NOVA_SNOWFLAKE_MAX_CHUNKS", DEFAULT_MAX_CHUNKS).max(1);

        Ok(Self {
            base_url,
            warehouse,
            database,
            schema,
            role,
            timeout,
            default_statement_timeout_s,
            poll_interval,
            max_poll,
            max_chunks,
            auth,
        })
    }
}

/// Minimal async Snowflake SQL API client.
#[derive(Clone)]
pub struct SnowflakeSqlClient {
    http: Client,
    cfg: SnowflakeSqlConfig,
}

impl SnowflakeSqlClient {
    /// Create a client from explicit config.
    ///
    /// # Errors
    /// Returns an error when the HTTP client cannot be created.
    pub fn new(cfg: SnowflakeSqlConfig) -> Result<Self> {
        let http = Client::builder()
            .timeout(cfg.timeout)
            .user_agent(format!("dbt-nova/{}", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|err| snowflake_err(format!("failed to build HTTP client: {err}")))?;
        Ok(Self { http, cfg })
    }

    /// Create a client from environment variables.
    ///
    /// # Errors
    /// Returns an error when required environment variables are missing or invalid.
    pub fn from_env() -> Result<Self> {
        Self::new(SnowflakeSqlConfig::from_env()?)
    }

    /// Execute a statement through the Snowflake SQL API.
    ///
    /// # Errors
    /// Returns an error when submission, polling, partition fetch, or result processing fails.
    pub async fn execute(
        &self,
        statement: &str,
        opts: SnowflakeExecuteOptions,
    ) -> Result<SnowflakeQueryResult> {
        let started = Instant::now();
        let settings = opts.resolve(&self.cfg);
        let request = StatementRequest {
            statement: statement.to_string(),
            timeout: Some(settings.statement_timeout_s),
            warehouse: settings.warehouse.clone(),
            database: self.cfg.database.clone(),
            schema: self.cfg.schema.clone(),
            role: self.cfg.role.clone(),
            bindings: if settings.bindings.is_empty() {
                None
            } else {
                Some(settings.bindings.clone())
            },
            parameters: session_parameters(settings.row_limit),
        };

        let request_id = uuid::Uuid::new_v4().to_string();
        let mut response = self.submit_statement(&request, &request_id).await?;
        let statement_handle = response
            .statement_handle
            .clone()
            .ok_or_else(|| snowflake_err("Snowflake response missing statementHandle"))?;

        if response.is_pending() {
            response = self
                .poll_statement(&statement_handle, settings.poll_interval, settings.max_poll)
                .await?;
        }

        if let Some(message) = response.failure_message() {
            return Err(snowflake_err(message));
        }

        let mut result = self
            .process_success(&statement_handle, response, &settings)
            .await?;
        result.elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        Ok(result)
    }

    async fn submit_statement(
        &self,
        request: &StatementRequest,
        request_id: &str,
    ) -> Result<StatementResponse> {
        let builder = self
            .authorized(self.http.post(self.statements_url()))?
            .query(&[("async", "true"), ("requestId", request_id)])
            .json(request);
        send_json(builder).await
    }

    async fn get_statement(&self, statement_handle: &str) -> Result<StatementResponse> {
        let builder = self.authorized(self.http.get(self.statement_url(statement_handle)))?;
        send_json(builder).await
    }

    async fn get_partition(
        &self,
        statement_handle: &str,
        partition: usize,
    ) -> Result<StatementResponse> {
        let builder = self
            .authorized(self.http.get(self.statement_url(statement_handle)))?
            .query(&[("partition", partition.to_string())]);
        send_json(builder).await
    }

    async fn cancel_statement(&self, statement_handle: &str) -> Result<()> {
        let builder = self.authorized(self.http.post(self.cancel_url(statement_handle)))?;
        let _: Value = send_json(builder).await?;
        Ok(())
    }

    async fn poll_statement(
        &self,
        statement_handle: &str,
        poll_interval: Duration,
        max_poll: Duration,
    ) -> Result<StatementResponse> {
        let started = Instant::now();
        loop {
            if started.elapsed() >= max_poll {
                if let Err(err) = self.cancel_statement(statement_handle).await {
                    warn!(
                        statement_handle,
                        error = %err,
                        "failed to cancel Snowflake statement after local poll timeout"
                    );
                }
                return Err(snowflake_err(format!(
                    "Timed out waiting for Snowflake statement {statement_handle}"
                )));
            }

            sleep(poll_interval).await;
            let response = self.get_statement(statement_handle).await?;
            if let Some(message) = response.failure_message() {
                return Err(snowflake_err(message));
            }
            if !response.is_pending() {
                return Ok(response);
            }
        }
    }

    async fn process_success(
        &self,
        statement_handle: &str,
        mut response: StatementResponse,
        settings: &ResolvedSnowflakeExecuteOptions,
    ) -> Result<SnowflakeQueryResult> {
        let metadata = response
            .result_set_meta_data
            .clone()
            .ok_or_else(|| snowflake_err("Snowflake success response missing resultSetMetaData"))?;

        let columns: Vec<String> = metadata
            .row_type
            .iter()
            .map(|field| field.name.clone())
            .collect();
        let column_types: Vec<String> = metadata
            .row_type
            .iter()
            .map(|field| field.type_name.to_ascii_uppercase())
            .collect();
        let partition_count = metadata.partition_info.len();
        let mut total_row_count = metadata.num_rows_u64();
        let total_byte_count = metadata.total_uncompressed_bytes();

        let mut rows = Vec::new();
        let mut approx_bytes = 0u64;
        let mut truncated = false;
        let mut fetched_chunks = 0u64;

        let mut next_partition = if let Some(data) = response.data.take() {
            fetched_chunks = fetched_chunks.saturating_add(1);
            append_rows(
                &mut rows,
                &metadata.row_type,
                &data,
                settings.row_limit,
                settings.byte_limit,
                &mut approx_bytes,
                &mut truncated,
            )?;
            1usize
        } else {
            0usize
        };

        if !settings.fetch_all_chunks && partition_count > next_partition {
            truncated = true;
        }

        while settings.fetch_all_chunks && !truncated && next_partition < partition_count {
            let max_chunks_u64 = u64::try_from(settings.max_chunks).unwrap_or(u64::MAX);
            if fetched_chunks >= max_chunks_u64 {
                truncated = true;
                break;
            }

            let page = self.get_partition(statement_handle, next_partition).await?;
            if let Some(message) = page.failure_message() {
                return Err(snowflake_err(message));
            }
            if total_row_count.is_none()
                && let Some(page_metadata) = page.result_set_meta_data.as_ref()
            {
                total_row_count = page_metadata.num_rows_u64();
            }
            if let Some(data) = page.data.as_ref() {
                fetched_chunks = fetched_chunks.saturating_add(1);
                append_rows(
                    &mut rows,
                    &metadata.row_type,
                    data,
                    settings.row_limit,
                    settings.byte_limit,
                    &mut approx_bytes,
                    &mut truncated,
                )?;
            }
            next_partition = next_partition.saturating_add(1);
        }

        Ok(SnowflakeQueryResult {
            statement_id: statement_handle.to_string(),
            state: "SUCCEEDED".to_string(),
            provider: "snowflake".to_string(),
            account_url: self.cfg.base_url.clone(),
            warehouse: settings.warehouse.clone(),
            database: self.cfg.database.clone(),
            schema: self.cfg.schema.clone(),
            role: self.cfg.role.clone(),
            columns,
            column_types,
            rows,
            elapsed_ms: 0,
            fetched_chunks,
            stats: SnowflakeQueryStats {
                total_row_count,
                total_byte_count,
                total_chunk_count: u64::try_from(partition_count).ok(),
            },
            truncated,
        })
    }

    fn authorized(&self, builder: reqwest::RequestBuilder) -> Result<reqwest::RequestBuilder> {
        let auth = self.authorization()?;
        Ok(builder
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .bearer_auth(auth.token)
            .header("X-Snowflake-Authorization-Token-Type", auth.token_type))
    }

    fn authorization(&self) -> Result<SnowflakeAuthorization> {
        match &self.cfg.auth {
            SnowflakeAuthConfig::OAuth { token } => Ok(SnowflakeAuthorization {
                token: token.clone(),
                token_type: "OAUTH",
            }),
            SnowflakeAuthConfig::ProgrammaticAccessToken { token } => Ok(SnowflakeAuthorization {
                token: token.clone(),
                token_type: "PROGRAMMATIC_ACCESS_TOKEN",
            }),
            SnowflakeAuthConfig::KeyPair {
                user,
                account_identifier,
                private_key_pem,
            } => Ok(SnowflakeAuthorization {
                token: generate_keypair_jwt(account_identifier, user, private_key_pem)?,
                token_type: "KEYPAIR_JWT",
            }),
        }
    }

    fn statements_url(&self) -> String {
        format!("{}/api/v2/statements", self.cfg.base_url)
    }

    fn statement_url(&self, statement_handle: &str) -> String {
        format!("{}/api/v2/statements/{statement_handle}", self.cfg.base_url)
    }

    fn cancel_url(&self, statement_handle: &str) -> String {
        format!(
            "{}/api/v2/statements/{statement_handle}/cancel",
            self.cfg.base_url
        )
    }
}

struct SnowflakeAuthorization {
    token: String,
    token_type: &'static str,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ErrorResponseBody {
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    message: Option<String>,
    #[serde(rename = "sqlState", default)]
    sql_state: Option<String>,
}

/// Options controlling one Snowflake SQL execution.
#[derive(Debug, Clone)]
pub struct SnowflakeExecuteOptions {
    pub warehouse: Option<String>,
    pub statement_timeout_s: Option<u64>,
    pub row_limit: Option<u64>,
    pub byte_limit: Option<u64>,
    pub poll_interval: Option<Duration>,
    pub max_poll: Option<Duration>,
    pub fetch_all_chunks: bool,
    pub max_chunks: Option<usize>,
    pub bindings: HashMap<String, SnowflakeBinding>,
}

impl Default for SnowflakeExecuteOptions {
    fn default() -> Self {
        Self {
            warehouse: None,
            statement_timeout_s: None,
            row_limit: Some(DEFAULT_ROW_LIMIT),
            byte_limit: Some(DEFAULT_BYTE_LIMIT),
            poll_interval: None,
            max_poll: None,
            fetch_all_chunks: true,
            max_chunks: Some(DEFAULT_MAX_CHUNKS),
            bindings: HashMap::new(),
        }
    }
}

impl SnowflakeExecuteOptions {
    fn resolve(self, config: &SnowflakeSqlConfig) -> ResolvedSnowflakeExecuteOptions {
        ResolvedSnowflakeExecuteOptions {
            warehouse: self.warehouse.unwrap_or_else(|| config.warehouse.clone()),
            statement_timeout_s: self
                .statement_timeout_s
                .unwrap_or(config.default_statement_timeout_s)
                .max(1),
            row_limit: self.row_limit.unwrap_or(DEFAULT_ROW_LIMIT).max(1),
            byte_limit: self.byte_limit.unwrap_or(DEFAULT_BYTE_LIMIT).max(1),
            poll_interval: self.poll_interval.unwrap_or(config.poll_interval),
            max_poll: self.max_poll.unwrap_or(config.max_poll),
            fetch_all_chunks: self.fetch_all_chunks,
            max_chunks: self.max_chunks.unwrap_or(config.max_chunks).max(1),
            bindings: self.bindings,
        }
    }
}

struct ResolvedSnowflakeExecuteOptions {
    warehouse: String,
    statement_timeout_s: u64,
    row_limit: u64,
    byte_limit: u64,
    poll_interval: Duration,
    max_poll: Duration,
    fetch_all_chunks: bool,
    max_chunks: usize,
    bindings: HashMap<String, SnowflakeBinding>,
}

/// Final Snowflake query result normalized to Nova's provider response contract.
#[derive(Debug, Serialize)]
pub struct SnowflakeQueryResult {
    pub statement_id: String,
    pub state: String,
    pub provider: String,
    pub account_url: String,
    pub warehouse: String,
    pub database: Option<String>,
    pub schema: Option<String>,
    pub role: Option<String>,
    pub columns: Vec<String>,
    pub column_types: Vec<String>,
    pub rows: Vec<Vec<Value>>,
    pub elapsed_ms: u64,
    pub fetched_chunks: u64,
    pub stats: SnowflakeQueryStats,
    pub truncated: bool,
}

/// Optional Snowflake statement statistics.
#[derive(Debug, Serialize)]
pub struct SnowflakeQueryStats {
    pub total_row_count: Option<u64>,
    pub total_byte_count: Option<u64>,
    pub total_chunk_count: Option<u64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct StatementRequest {
    statement: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    timeout: Option<u64>,
    warehouse: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    database: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    schema: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bindings: Option<HashMap<String, SnowflakeBinding>>,
    parameters: JsonMap<String, Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SnowflakeBinding {
    #[serde(rename = "type")]
    type_name: String,
    value: Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StatementResponse {
    #[serde(default)]
    statement_handle: Option<String>,
    #[serde(default)]
    statement_status_url: Option<String>,
    #[serde(default)]
    result_set_meta_data: Option<ResultSetMetadata>,
    #[serde(default)]
    data: Option<Vec<Vec<Value>>>,
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    message: Option<String>,
    #[serde(rename = "sqlState", default)]
    sql_state: Option<String>,
}

impl StatementResponse {
    fn is_pending(&self) -> bool {
        self.result_set_meta_data.is_none()
            && self.data.is_none()
            && self.failure_message().is_none()
            && (self.statement_status_url.is_some() || self.statement_handle.is_some())
    }

    fn failure_message(&self) -> Option<String> {
        if self.result_set_meta_data.is_some() || self.data.is_some() {
            return None;
        }
        if self.statement_status_url.is_some() {
            return None;
        }
        let code = self.code.as_deref()?;
        let message = self.message.as_deref().unwrap_or("statement failed");
        let sql_state = self
            .sql_state
            .as_deref()
            .map(|state| format!(" sqlState={state}"))
            .unwrap_or_default();
        Some(format!(
            "Snowflake statement error {code}:{sql_state} {message}"
        ))
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ResultSetMetadata {
    #[serde(default)]
    num_rows: Option<Value>,
    #[serde(default)]
    row_type: Vec<ResultColumn>,
    #[serde(default)]
    partition_info: Vec<PartitionInfo>,
}

impl ResultSetMetadata {
    fn num_rows_u64(&self) -> Option<u64> {
        parse_optional_u64(self.num_rows.as_ref())
    }

    fn total_uncompressed_bytes(&self) -> Option<u64> {
        let mut total = 0u64;
        let mut seen = false;
        for partition in &self.partition_info {
            if let Some(size) = parse_optional_u64(partition.uncompressed_size.as_ref()) {
                total = total.saturating_add(size);
                seen = true;
            }
        }
        seen.then_some(total)
    }
}

#[derive(Debug, Clone, Deserialize)]
struct ResultColumn {
    name: String,
    #[serde(rename = "type")]
    type_name: String,
    #[serde(default)]
    scale: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PartitionInfo {
    #[serde(default)]
    uncompressed_size: Option<Value>,
}

async fn send_json<T: for<'de> Deserialize<'de>>(builder: reqwest::RequestBuilder) -> Result<T> {
    let response = builder
        .send()
        .await
        .map_err(|err| snowflake_err(format!("HTTP request failed: {err}")))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|err| snowflake_err(format!("failed to read response body: {err}")))?;

    if !status.is_success() {
        return Err(snowflake_http(status, &body));
    }

    serde_json::from_str(&body).map_err(|err| {
        snowflake_err(format!(
            "failed to parse JSON response: {err}; response_body_bytes={}",
            body.len()
        ))
    })
}

fn summarize_error_body(status: StatusCode, body: &str) -> String {
    if status == StatusCode::UNAUTHORIZED {
        return "authorization failed; check Snowflake credentials".to_string();
    }

    match serde_json::from_str::<ErrorResponseBody>(body) {
        Ok(parsed) => {
            let code = parsed.code.as_deref().unwrap_or("unknown");
            let sql_state = parsed
                .sql_state
                .as_deref()
                .filter(|value| !value.is_empty())
                .map(|value| format!(" sqlState={value}"))
                .unwrap_or_default();
            let message = parsed.message.as_deref().unwrap_or("request failed");
            format!("{code}:{sql_state} {}", truncate_for_error(message, 512))
        }
        Err(_) => format!("non-JSON response ({} bytes)", body.len()),
    }
}

fn truncate_for_error(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let truncated: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{truncated}...")
    } else {
        truncated
    }
}

fn session_parameters(row_limit: u64) -> JsonMap<String, Value> {
    JsonMap::from_iter([
        (
            "binary_output_format".to_string(),
            Value::String("HEX".to_string()),
        ),
        (
            "date_output_format".to_string(),
            Value::String("YYYY-MM-DD".to_string()),
        ),
        (
            "time_output_format".to_string(),
            Value::String("HH24:MI:SS.FF9".to_string()),
        ),
        (
            "timestamp_ntz_output_format".to_string(),
            Value::String("YYYY-MM-DD HH24:MI:SS.FF9".to_string()),
        ),
        (
            "timestamp_ltz_output_format".to_string(),
            Value::String("YYYY-MM-DD HH24:MI:SS.FF9 TZHTZM".to_string()),
        ),
        (
            "timestamp_tz_output_format".to_string(),
            Value::String("YYYY-MM-DD HH24:MI:SS.FF9 TZHTZM".to_string()),
        ),
        (
            "query_tag".to_string(),
            Value::String(format!("dbt-nova/{}", env!("CARGO_PKG_VERSION"))),
        ),
        ("rows_per_resultset".to_string(), Value::from(row_limit)),
    ])
}

fn append_rows(
    output: &mut Vec<Vec<Value>>,
    schema_fields: &[ResultColumn],
    rows: &[Vec<Value>],
    row_limit: u64,
    byte_limit: u64,
    approx_bytes: &mut u64,
    truncated: &mut bool,
) -> Result<()> {
    for row in rows {
        if u64::try_from(output.len()).unwrap_or(u64::MAX) >= row_limit {
            *truncated = true;
            break;
        }

        let mut converted = Vec::with_capacity(schema_fields.len());
        for (idx, field) in schema_fields.iter().enumerate() {
            let value = row.get(idx).unwrap_or(&Value::Null);
            converted.push(parse_cell_value(value, field));
        }

        let row_bytes = u64::try_from(
            serde_json::to_vec(&converted)
                .map_err(|err| snowflake_err(format!("failed to serialize row: {err}")))?
                .len(),
        )
        .unwrap_or(u64::MAX);
        if approx_bytes.saturating_add(row_bytes) > byte_limit {
            *truncated = true;
            break;
        }

        *approx_bytes = approx_bytes.saturating_add(row_bytes);
        output.push(converted);
    }
    Ok(())
}

fn parse_cell_value(value: &Value, field: &ResultColumn) -> Value {
    if value.is_null() {
        return Value::Null;
    }
    let Some(text) = value.as_str() else {
        return value.clone();
    };

    match field.type_name.to_ascii_uppercase().as_str() {
        "FIXED" | "NUMBER" | "DECIMAL" | "NUMERIC" => {
            if parse_optional_u64(field.scale.as_ref()).unwrap_or(0) == 0 {
                return text
                    .parse::<i64>()
                    .map_or_else(|_| Value::String(text.to_string()), Value::from);
            }
            text.parse::<f64>()
                .map_or_else(|_| Value::String(text.to_string()), Value::from)
        }
        "REAL" | "FLOAT" | "FLOAT4" | "FLOAT8" | "DOUBLE" | "DOUBLE PRECISION" => text
            .parse::<f64>()
            .map_or_else(|_| Value::String(text.to_string()), Value::from),
        "BOOLEAN" => match text.to_ascii_lowercase().as_str() {
            "true" => Value::Bool(true),
            "false" => Value::Bool(false),
            _ => Value::String(text.to_string()),
        },
        "VARIANT" | "OBJECT" | "ARRAY" => {
            serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.to_string()))
        }
        _ => Value::String(text.to_string()),
    }
}

fn parse_optional_u64(value: Option<&Value>) -> Option<u64> {
    match value? {
        Value::Number(number) => number.as_u64(),
        Value::String(text) => text.parse::<u64>().ok(),
        _ => None,
    }
}

#[derive(Debug)]
struct RewrittenSql {
    sql: String,
    ordered_parameters: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RewriteState {
    Code,
    SingleQuotedString,
    DoubleQuotedIdentifier,
    LineComment,
    BlockComment,
}

fn is_identifier_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

fn is_identifier_continue(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn colon_starts_named_parameter(bytes: &[u8], index: usize) -> bool {
    let Some(next) = bytes.get(index + 1) else {
        return false;
    };
    if *next == b':' || !is_identifier_start(*next) {
        return false;
    }

    !matches!(
        index.checked_sub(1).and_then(|previous| bytes.get(previous)),
        Some(previous)
            if is_identifier_continue(*previous)
                || matches!(*previous, b'"' | b']' | b')' | b'$')
    )
}

fn rewrite_named_parameters(
    statement: &str,
    parameters: &HashMap<String, Value>,
) -> Result<RewrittenSql> {
    let mut rewritten = String::with_capacity(statement.len());
    let mut ordered_parameters = Vec::new();
    let mut state = RewriteState::Code;
    let bytes = statement.as_bytes();
    let mut index = 0usize;

    while index < bytes.len() {
        match state {
            RewriteState::Code => {
                if bytes[index] == b'\'' {
                    rewritten.push('\'');
                    index += 1;
                    state = RewriteState::SingleQuotedString;
                    continue;
                }
                if bytes[index] == b'"' {
                    rewritten.push('"');
                    index += 1;
                    state = RewriteState::DoubleQuotedIdentifier;
                    continue;
                }
                if bytes[index] == b'-' && index + 1 < bytes.len() && bytes[index + 1] == b'-' {
                    rewritten.push_str("--");
                    index += 2;
                    state = RewriteState::LineComment;
                    continue;
                }
                if bytes[index] == b'/' && index + 1 < bytes.len() && bytes[index + 1] == b'*' {
                    rewritten.push_str("/*");
                    index += 2;
                    state = RewriteState::BlockComment;
                    continue;
                }
                if bytes[index] == b':' {
                    if index + 1 < bytes.len() && bytes[index + 1] == b':' {
                        rewritten.push_str("::");
                        index += 2;
                        continue;
                    }
                    if colon_starts_named_parameter(bytes, index) {
                        let mut end = index + 2;
                        while end < bytes.len() && is_identifier_continue(bytes[end]) {
                            end += 1;
                        }
                        let name = &statement[index + 1..end];
                        if !parameters.contains_key(name) {
                            return Err(DbtNovaError::InvalidParams(format!(
                                "Missing value for SQL parameter :{name}"
                            )));
                        }
                        rewritten.push('?');
                        ordered_parameters.push(name.to_string());
                        index = end;
                        continue;
                    }
                    rewritten.push(':');
                    index += 1;
                    continue;
                }

                let next = statement[index..]
                    .chars()
                    .next()
                    .ok_or_else(|| snowflake_err("failed to parse SQL while rewriting"))?;
                rewritten.push(next);
                index += next.len_utf8();
            }
            RewriteState::SingleQuotedString => {
                if bytes[index] == b'\'' {
                    if index + 1 < bytes.len() && bytes[index + 1] == b'\'' {
                        rewritten.push_str("''");
                        index += 2;
                    } else {
                        rewritten.push('\'');
                        index += 1;
                        state = RewriteState::Code;
                    }
                    continue;
                }
                let next = statement[index..]
                    .chars()
                    .next()
                    .ok_or_else(|| snowflake_err("failed to parse SQL while rewriting"))?;
                rewritten.push(next);
                index += next.len_utf8();
            }
            RewriteState::DoubleQuotedIdentifier => {
                if bytes[index] == b'"' {
                    if index + 1 < bytes.len() && bytes[index + 1] == b'"' {
                        rewritten.push_str("\"\"");
                        index += 2;
                    } else {
                        rewritten.push('"');
                        index += 1;
                        state = RewriteState::Code;
                    }
                    continue;
                }
                let next = statement[index..]
                    .chars()
                    .next()
                    .ok_or_else(|| snowflake_err("failed to parse SQL while rewriting"))?;
                rewritten.push(next);
                index += next.len_utf8();
            }
            RewriteState::LineComment => {
                let next = statement[index..]
                    .chars()
                    .next()
                    .ok_or_else(|| snowflake_err("failed to parse SQL while rewriting"))?;
                rewritten.push(next);
                index += next.len_utf8();
                if next == '\n' {
                    state = RewriteState::Code;
                }
            }
            RewriteState::BlockComment => {
                if bytes[index] == b'*' && index + 1 < bytes.len() && bytes[index + 1] == b'/' {
                    rewritten.push_str("*/");
                    index += 2;
                    state = RewriteState::Code;
                    continue;
                }
                let next = statement[index..]
                    .chars()
                    .next()
                    .ok_or_else(|| snowflake_err("failed to parse SQL while rewriting"))?;
                rewritten.push(next);
                index += next.len_utf8();
            }
        }
    }

    Ok(RewrittenSql {
        sql: rewritten,
        ordered_parameters,
    })
}

fn build_bindings(
    ordered_parameters: &[String],
    parameters: &HashMap<String, Value>,
    parameter_types: Option<HashMap<String, String>>,
) -> Result<HashMap<String, SnowflakeBinding>> {
    let parameter_types = parameter_types.unwrap_or_default();
    for key in parameter_types.keys() {
        if !parameters.contains_key(key) {
            return Err(DbtNovaError::InvalidParams(format!(
                "parameter_types contains '{key}' but parameters does not"
            )));
        }
    }

    let mut bindings = HashMap::with_capacity(ordered_parameters.len());
    for (index, name) in ordered_parameters.iter().enumerate() {
        let value = parameters.get(name).ok_or_else(|| {
            DbtNovaError::InvalidParams(format!("Missing value for SQL parameter :{name}"))
        })?;
        let type_name = parameter_types.get(name).map_or_else(
            || infer_binding_type(value).unwrap_or_default(),
            |value| value.to_ascii_uppercase(),
        );
        if type_name.is_empty() {
            return Err(DbtNovaError::InvalidParams(format!(
                "Snowflake null parameter :{name} requires explicit parameter_types"
            )));
        }
        bindings.insert(
            (index + 1).to_string(),
            SnowflakeBinding {
                type_name,
                value: binding_value(value)?,
            },
        );
    }
    Ok(bindings)
}

fn infer_binding_type(value: &Value) -> Option<String> {
    match value {
        Value::Bool(_) => Some("BOOLEAN".to_string()),
        Value::Number(number) => {
            if number.is_i64() || number.is_u64() {
                Some("FIXED".to_string())
            } else {
                Some("REAL".to_string())
            }
        }
        Value::String(_) => Some("TEXT".to_string()),
        Value::Null | Value::Array(_) | Value::Object(_) => None,
    }
}

fn binding_value(value: &Value) -> Result<Value> {
    match value {
        Value::Null => Ok(Value::Null),
        Value::Bool(flag) => Ok(Value::String(flag.to_string())),
        Value::Number(number) => Ok(Value::String(number.to_string())),
        Value::String(text) => Ok(Value::String(text.clone())),
        Value::Array(_) | Value::Object(_) => Err(DbtNovaError::InvalidParams(
            "Snowflake SQL parameters must be scalar JSON values".to_string(),
        )),
    }
}

fn execute_settings(params: &ExecuteSqlParams) -> Result<(String, SnowflakeExecuteOptions)> {
    let statement = params.statement.trim();
    if statement.is_empty() {
        return Err(DbtNovaError::InvalidParams(
            "statement cannot be empty".to_string(),
        ));
    }

    let parameters = params.parameters.clone().unwrap_or_default();
    let rewritten = rewrite_named_parameters(statement, &parameters)?;
    let bindings = build_bindings(
        &rewritten.ordered_parameters,
        &parameters,
        params.parameter_types.clone(),
    )?;

    let mut opts = SnowflakeExecuteOptions {
        warehouse: params.warehouse_id.clone(),
        statement_timeout_s: params.wait_timeout_s,
        row_limit: params.row_limit.or(Some(DEFAULT_ROW_LIMIT)),
        byte_limit: params.byte_limit.or(Some(DEFAULT_BYTE_LIMIT)),
        bindings,
        ..SnowflakeExecuteOptions::default()
    };
    if let Some(ms) = params.poll_interval_ms {
        opts.poll_interval = Some(Duration::from_millis(ms));
    }
    if let Some(seconds) = params.max_poll_seconds {
        opts.max_poll = Some(Duration::from_secs(seconds));
    }
    if let Some(fetch_all_chunks) = params.fetch_all_chunks {
        opts.fetch_all_chunks = fetch_all_chunks;
    }
    if let Some(max_chunks) = params.max_chunks {
        opts.max_chunks = Some(max_chunks);
    }

    Ok((rewritten.sql, opts))
}

async fn execute_snowflake(params: &ExecuteSqlParams) -> Result<Value> {
    let client = SnowflakeSqlClient::from_env()?;
    let (statement, opts) = execute_settings(params)?;
    let result = client.execute(&statement, opts).await?;
    let count = result.rows.len();
    serde_json::to_value(SuccessResponse::new(result, count)).map_err(Into::into)
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
    Ok(trimmed.to_ascii_uppercase())
}

fn normalize_preflight_relation(relation: &str) -> Result<String> {
    let trimmed = relation.trim();
    if trimmed.is_empty() {
        return Err(DbtNovaError::InvalidParams(
            "preflight_relation cannot be empty".to_string(),
        ));
    }

    let parts: Vec<&str> = trimmed.split('.').collect();
    if parts.is_empty() || parts.len() > 3 {
        return Err(DbtNovaError::InvalidParams(format!(
            "Invalid relation '{trimmed}': expected table, schema.table, or database.schema.table"
        )));
    }

    let mut normalized = Vec::with_capacity(parts.len());
    for part in parts {
        normalized.push(normalize_preflight_identifier(part, "relation")?);
    }
    Ok(normalized.join("."))
}

fn sql_string_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn catalog_preflight_statement(catalog: &str) -> String {
    format!("SHOW DATABASES LIKE {}", sql_string_literal(catalog))
}

fn schema_preflight_statement(catalog: &str, schema: &str) -> String {
    format!(
        "SHOW SCHEMAS LIKE {} IN DATABASE {catalog}",
        sql_string_literal(schema)
    )
}

fn relation_preflight_statement(relation: &str) -> String {
    format!("SELECT 1 AS relation_access_check FROM {relation} LIMIT 1")
}

async fn run_preflight_statement(
    client: &SnowflakeSqlClient,
    statement: &str,
    warehouse: Option<String>,
) -> Result<SnowflakeQueryResult> {
    client
        .execute(
            statement,
            SnowflakeExecuteOptions {
                warehouse,
                row_limit: Some(1),
                byte_limit: Some(1024),
                statement_timeout_s: Some(10),
                max_poll: Some(Duration::from_secs(30)),
                fetch_all_chunks: false,
                max_chunks: Some(1),
                ..SnowflakeExecuteOptions::default()
            },
        )
        .await
}

fn preflight_result_has_rows(result: &SnowflakeQueryResult) -> bool {
    preflight_probe_has_rows(result.rows.len(), result.stats.total_row_count)
}

fn detail_field(key: &str, value: impl AsRef<str>) -> JsonMap<String, Value> {
    JsonMap::from_iter([(key.to_string(), Value::String(value.as_ref().to_string()))])
}

#[allow(clippy::too_many_lines)]
async fn preflight_snowflake(params: &ExecuteSqlParams) -> Result<Value> {
    let mut metadata = JsonMap::new();
    metadata.insert(
        "warehouse".to_string(),
        params
            .warehouse_id
            .clone()
            .map_or(Value::Null, Value::String),
    );

    let client = match SnowflakeSqlClient::from_env() {
        Ok(client) => client,
        Err(err) => {
            return build_configuration_failure_response(
                "snowflake",
                metadata,
                err.to_string(),
                "Set DBT_NOVA_SNOWFLAKE_ACCOUNT or DBT_NOVA_SNOWFLAKE_ACCOUNT_URL, DBT_NOVA_SNOWFLAKE_WAREHOUSE, and Snowflake auth variables",
            );
        }
    };

    let check_warehouse = params
        .warehouse_id
        .clone()
        .or_else(|| Some(client.cfg.warehouse.clone()));
    metadata.insert(
        "account_url".to_string(),
        Value::String(client.cfg.base_url.clone()),
    );
    metadata.insert(
        "warehouse".to_string(),
        check_warehouse.clone().map_or(Value::Null, Value::String),
    );
    metadata.insert(
        "database".to_string(),
        client
            .cfg
            .database
            .clone()
            .map_or(Value::Null, Value::String),
    );
    metadata.insert(
        "schema".to_string(),
        client.cfg.schema.clone().map_or(Value::Null, Value::String),
    );
    metadata.insert(
        "role".to_string(),
        client.cfg.role.clone().map_or(Value::Null, Value::String),
    );

    let mut report = PreflightReport::new();
    run_connectivity_check(
        &mut report,
        "Verify warehouse is running and credentials allow SQL execution",
        || async {
            run_preflight_statement(
                &client,
                "SELECT 1 AS connectivity_check",
                check_warehouse.clone(),
            )
            .await
            .map(|_| ())
        },
    )
    .await;

    let client_for_catalog = client.clone();
    run_optional_object_check(
        &mut report,
        params.preflight_catalog.as_deref(),
        "catalog_access",
        |catalog| normalize_preflight_identifier(catalog, "catalog"),
        |catalog| {
            let catalog = catalog.clone();
            let warehouse = check_warehouse.clone();
            let client = client_for_catalog.clone();
            async move {
                let statement = catalog_preflight_statement(&catalog);
                let result = run_preflight_statement(&client, &statement, warehouse).await?;
                Ok(if preflight_result_has_rows(&result) {
                    ProbePresence::Present
                } else {
                    ProbePresence::Empty
                })
            }
        },
        |catalog| detail_field("catalog", catalog),
        |catalog| detail_field("catalog", catalog),
        "Use an unquoted database identifier (letters, digits, _, $)",
        "Verify database exists and role has access",
        &empty_preflight_probe_message("catalog_access"),
    )
    .await;

    let default_catalog = client.cfg.database.clone();
    let client_for_schema = client.clone();
    run_optional_object_check(
        &mut report,
        params.preflight_schema.as_deref(),
        "schema_access",
        |schema| {
            let catalog = params
                .preflight_catalog
                .as_deref()
                .map(|catalog| normalize_preflight_identifier(catalog, "catalog"))
                .transpose()?
                .or_else(|| default_catalog.clone())
                .ok_or_else(|| {
                    DbtNovaError::InvalidParams(
                        "preflight_schema requires DBT_NOVA_SNOWFLAKE_DATABASE or preflight_catalog"
                            .to_string(),
                    )
                })?;
            let schema = normalize_preflight_identifier(schema, "schema")?;
            Ok((catalog, schema))
        },
        |(catalog, schema)| {
            let catalog = catalog.clone();
            let schema = schema.clone();
            let warehouse = check_warehouse.clone();
            let client = client_for_schema.clone();
            async move {
                let statement = schema_preflight_statement(&catalog, &schema);
                let result = run_preflight_statement(&client, &statement, warehouse).await?;
                Ok(if preflight_result_has_rows(&result) {
                    ProbePresence::Present
                } else {
                    ProbePresence::Empty
                })
            }
        },
        |schema| detail_field("schema", schema),
        |(catalog, schema)| {
            JsonMap::from_iter([
                ("catalog".to_string(), Value::String(catalog.clone())),
                ("schema".to_string(), Value::String(schema.clone())),
            ])
        },
        "Use valid unquoted database and schema identifiers",
        "Verify schema exists and role has access",
        &empty_preflight_probe_message("schema_access"),
    )
    .await;

    let client_for_relation = client.clone();
    run_optional_object_check(
        &mut report,
        params.preflight_relation.as_deref(),
        "relation_access",
        normalize_preflight_relation,
        |relation| {
            let relation = relation.clone();
            let warehouse = check_warehouse.clone();
            let client = client_for_relation.clone();
            async move {
                let statement = relation_preflight_statement(&relation);
                let result = run_preflight_statement(&client, &statement, warehouse).await?;
                Ok(if preflight_result_has_rows(&result) {
                    ProbePresence::Present
                } else {
                    ProbePresence::Empty
                })
            }
        },
        |relation| detail_field("relation", relation),
        |relation| detail_field("relation", relation),
        "Use unquoted identifiers like table, schema.table, or database.schema.table",
        "Verify relation exists and role has SELECT permissions",
        &empty_preflight_probe_message("relation_access"),
    )
    .await;

    build_preflight_response("snowflake", metadata, report)
}

pub struct SnowflakeProvider;

pub static SNOWFLAKE_PROVIDER: SnowflakeProvider = SnowflakeProvider;

impl SqlProvider for SnowflakeProvider {
    fn name(&self) -> &'static str {
        "snowflake"
    }

    fn execute<'a>(
        &'a self,
        params: &'a ExecuteSqlParams,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value>> + Send + 'a>> {
        Box::pin(async move { execute_snowflake(params).await })
    }

    fn preflight<'a>(
        &'a self,
        params: &'a ExecuteSqlParams,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value>> + Send + 'a>> {
        Box::pin(async move { preflight_snowflake(params).await })
    }
}

fn read_required_env(name: &str, message: &str) -> Result<String> {
    let value = env::var(name).map_err(|_| DbtNovaError::InvalidParams(message.to_string()))?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(DbtNovaError::InvalidParams(message.to_string()));
    }
    Ok(trimmed.to_string())
}

fn read_optional_env(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn env_u64(name: &str, default_value: u64) -> u64 {
    env::var(name)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(default_value)
}

fn env_usize(name: &str, default_value: usize) -> usize {
    env::var(name)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(default_value)
}

fn resolve_base_url_from_env() -> Result<(String, Option<String>)> {
    let account = read_optional_env("DBT_NOVA_SNOWFLAKE_ACCOUNT");
    let url = if let Some(url) = read_optional_env("DBT_NOVA_SNOWFLAKE_ACCOUNT_URL") {
        normalize_account_url(&url)?
    } else {
        let account = account.as_deref().ok_or_else(|| {
            DbtNovaError::InvalidParams(
                "DBT_NOVA_SNOWFLAKE_ACCOUNT or DBT_NOVA_SNOWFLAKE_ACCOUNT_URL is required when DBT_NOVA_SQL_PROVIDER=snowflake".to_string(),
            )
        })?;
        normalize_account_url(&format!("{account}.snowflakecomputing.com"))?
    };
    Ok((url, account))
}

fn normalize_account_url(input: &str) -> Result<String> {
    let trimmed = input.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err(DbtNovaError::InvalidParams(
            "Snowflake account URL cannot be empty".to_string(),
        ));
    }
    let url = if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        format!("https://{trimmed}")
    };
    let parsed = Url::parse(&url).map_err(|err| {
        DbtNovaError::InvalidParams(format!("Invalid Snowflake account URL '{input}': {err}"))
    })?;
    if parsed.scheme() != "https" {
        return Err(DbtNovaError::InvalidParams(
            "Snowflake account URL must use https://".to_string(),
        ));
    }
    let host = parsed.host_str().ok_or_else(|| {
        DbtNovaError::InvalidParams("Snowflake account URL must include a host".to_string())
    })?;
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(DbtNovaError::InvalidParams(
            "Snowflake account URL must not include credentials".to_string(),
        ));
    }
    if parsed.path() != "/" || parsed.query().is_some() || parsed.fragment().is_some() {
        return Err(DbtNovaError::InvalidParams(
            "Snowflake account URL must not include a path, query, or fragment".to_string(),
        ));
    }

    let mut normalized = format!("https://{host}");
    if let Some(port) = parsed.port() {
        normalized.push(':');
        normalized.push_str(&port.to_string());
    }
    Ok(normalized)
}

fn resolve_auth_from_env(account: Option<String>) -> Result<SnowflakeAuthConfig> {
    let auth = read_optional_env("DBT_NOVA_SNOWFLAKE_AUTH")
        .map(|value| value.to_ascii_lowercase())
        .or_else(|| {
            if read_optional_env("DBT_NOVA_SNOWFLAKE_PAT").is_some() {
                Some("pat".to_string())
            } else if read_optional_env("DBT_NOVA_SNOWFLAKE_OAUTH_TOKEN").is_some() {
                Some("oauth".to_string())
            } else {
                Some("keypair".to_string())
            }
        })
        .unwrap_or_else(|| "keypair".to_string());

    match auth.as_str() {
        "oauth" => Ok(SnowflakeAuthConfig::OAuth {
            token: read_required_env(
                "DBT_NOVA_SNOWFLAKE_OAUTH_TOKEN",
                "DBT_NOVA_SNOWFLAKE_OAUTH_TOKEN is required for Snowflake OAuth auth",
            )?,
        }),
        "pat" | "programmatic_access_token" => Ok(SnowflakeAuthConfig::ProgrammaticAccessToken {
            token: read_required_env(
                "DBT_NOVA_SNOWFLAKE_PAT",
                "DBT_NOVA_SNOWFLAKE_PAT is required for Snowflake PAT auth",
            )?,
        }),
        "keypair" | "snowflake_jwt" => {
            if read_optional_env("DBT_NOVA_SNOWFLAKE_PRIVATE_KEY_PASSPHRASE").is_some() {
                return Err(DbtNovaError::InvalidParams(
                    "Encrypted Snowflake private keys are not supported by dbt-nova yet; provide an unencrypted PKCS#8 or RSA PEM key".to_string(),
                ));
            }
            let user = read_required_env(
                "DBT_NOVA_SNOWFLAKE_USER",
                "DBT_NOVA_SNOWFLAKE_USER is required for Snowflake keypair auth",
            )?;
            let account_identifier = read_optional_env("DBT_NOVA_SNOWFLAKE_JWT_ACCOUNT")
                .or(account)
                .ok_or_else(|| {
                    DbtNovaError::InvalidParams(
                        "DBT_NOVA_SNOWFLAKE_JWT_ACCOUNT or DBT_NOVA_SNOWFLAKE_ACCOUNT is required for Snowflake keypair auth".to_string(),
                    )
                })?;
            let private_key_pem = resolve_private_key_pem()?;
            Ok(SnowflakeAuthConfig::KeyPair {
                user,
                account_identifier: normalize_jwt_identifier(&account_identifier),
                private_key_pem,
            })
        }
        other => Err(DbtNovaError::InvalidParams(format!(
            "Unsupported DBT_NOVA_SNOWFLAKE_AUTH '{other}' (expected keypair, oauth, or pat)"
        ))),
    }
}

fn resolve_private_key_pem() -> Result<String> {
    if let Some(value) = read_optional_env("DBT_NOVA_SNOWFLAKE_PRIVATE_KEY_PEM") {
        if value.contains('\n') {
            return Ok(value);
        }
        return Ok(value.replace("\\n", "\n"));
    }

    let path = read_required_env(
        "DBT_NOVA_SNOWFLAKE_PRIVATE_KEY_PATH",
        "DBT_NOVA_SNOWFLAKE_PRIVATE_KEY_PATH or DBT_NOVA_SNOWFLAKE_PRIVATE_KEY_PEM is required for Snowflake keypair auth",
    )?;
    std::fs::read_to_string(&path).map_err(|err| {
        DbtNovaError::InvalidParams(format!(
            "Failed to read DBT_NOVA_SNOWFLAKE_PRIVATE_KEY_PATH '{path}': {err}"
        ))
    })
}

fn normalize_jwt_identifier(value: &str) -> String {
    value.trim().replace('.', "-").to_ascii_uppercase()
}

#[derive(Serialize)]
struct SnowflakeJwtClaims {
    iss: String,
    sub: String,
    iat: u64,
    exp: u64,
}

fn generate_keypair_jwt(
    account_identifier: &str,
    user: &str,
    private_key_pem: &str,
) -> Result<String> {
    let fingerprint = public_key_fingerprint(private_key_pem)?;
    let account = normalize_jwt_identifier(account_identifier);
    let user = user.trim().to_ascii_uppercase();
    let qualified_user = format!("{account}.{user}");
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|err| snowflake_err(format!("system clock before UNIX epoch: {err}")))?
        .as_secs();
    let claims = SnowflakeJwtClaims {
        iss: format!("{qualified_user}.{fingerprint}"),
        sub: qualified_user,
        iat: now,
        exp: now.saturating_add(DEFAULT_JWT_LIFETIME_SECONDS),
    };
    let key = EncodingKey::from_rsa_pem(private_key_pem.as_bytes())
        .map_err(|err| snowflake_err(format!("failed to parse Snowflake private key: {err}")))?;
    jsonwebtoken::encode(&Header::new(Algorithm::RS256), &claims, &key)
        .map_err(|err| snowflake_err(format!("failed to generate Snowflake JWT: {err}")))
}

fn public_key_fingerprint(private_key_pem: &str) -> Result<String> {
    let rsa_private_der = rsa_private_key_der_from_pem(private_key_pem)?;
    let (modulus, exponent) = rsa_public_components_from_private_der(&rsa_private_der)?;
    let public_key_der = rsa_public_spki_der(modulus, exponent)?;
    let digest = Sha256::digest(&public_key_der);
    Ok(format!(
        "SHA256:{}",
        base64::engine::general_purpose::STANDARD.encode(digest)
    ))
}

fn rsa_private_key_der_from_pem(private_key_pem: &str) -> Result<Vec<u8>> {
    let parsed = pem::parse(private_key_pem.as_bytes())
        .map_err(|err| snowflake_err(format!("failed to parse private key PEM: {err}")))?;
    match parsed.tag() {
        "RSA PRIVATE KEY" => Ok(parsed.into_contents()),
        "PRIVATE KEY" => extract_first_octet_string(&parsed.into_contents()),
        "ENCRYPTED PRIVATE KEY" => Err(DbtNovaError::InvalidParams(
            "Encrypted Snowflake private keys are not supported by dbt-nova yet; provide an unencrypted PKCS#8 or RSA PEM key".to_string(),
        )),
        tag => Err(DbtNovaError::InvalidParams(format!(
            "Unsupported Snowflake private key PEM tag '{tag}'"
        ))),
    }
}

fn extract_first_octet_string(der: &[u8]) -> Result<Vec<u8>> {
    let blocks = simple_asn1::from_der(der)
        .map_err(|err| snowflake_err(format!("failed to decode PKCS#8 private key DER: {err}")))?;
    visit_first_octet_string(&blocks)
        .ok_or_else(|| snowflake_err("PKCS#8 private key did not contain an RSA key"))
}

fn visit_first_octet_string(blocks: &[ASN1Block]) -> Option<Vec<u8>> {
    for block in blocks {
        match block {
            ASN1Block::OctetString(_, value) => return Some(value.clone()),
            ASN1Block::Sequence(_, children) => {
                if let Some(value) = visit_first_octet_string(children) {
                    return Some(value);
                }
            }
            _ => {}
        }
    }
    None
}

fn rsa_public_components_from_private_der(private_der: &[u8]) -> Result<(BigInt, BigInt)> {
    let blocks = simple_asn1::from_der(private_der)
        .map_err(|err| snowflake_err(format!("failed to decode RSA private key DER: {err}")))?;
    let [ASN1Block::Sequence(_, entries)] = blocks.as_slice() else {
        return Err(snowflake_err(
            "RSA private key DER must contain a single sequence",
        ));
    };
    let modulus = match entries.get(1) {
        Some(ASN1Block::Integer(_, value)) => value.clone(),
        _ => return Err(snowflake_err("RSA private key missing modulus")),
    };
    let exponent = match entries.get(2) {
        Some(ASN1Block::Integer(_, value)) => value.clone(),
        _ => return Err(snowflake_err("RSA private key missing public exponent")),
    };
    Ok((modulus, exponent))
}

fn rsa_public_spki_der(modulus: BigInt, exponent: BigInt) -> Result<Vec<u8>> {
    let rsa_public_key = simple_asn1::to_der(&ASN1Block::Sequence(
        0,
        vec![
            ASN1Block::Integer(0, modulus),
            ASN1Block::Integer(0, exponent),
        ],
    ))
    .map_err(|err| snowflake_err(format!("failed to encode RSA public key DER: {err}")))?;

    let spki = ASN1Block::Sequence(
        0,
        vec![
            ASN1Block::Sequence(
                0,
                vec![
                    ASN1Block::ObjectIdentifier(0, oid!(1, 2, 840, 113_549, 1, 1, 1)),
                    ASN1Block::Null(0),
                ],
            ),
            ASN1Block::BitString(0, rsa_public_key.len().saturating_mul(8), rsa_public_key),
        ],
    );

    simple_asn1::to_der(&spki)
        .map_err(|err| snowflake_err(format!("failed to encode public key SPKI DER: {err}")))
}

#[cfg(test)]
mod tests {
    use super::{
        ResultColumn, build_bindings, catalog_preflight_statement, generate_keypair_jwt,
        normalize_account_url, normalize_jwt_identifier, normalize_preflight_relation,
        parse_cell_value, public_key_fingerprint, relation_preflight_statement,
        rewrite_named_parameters, schema_preflight_statement, session_parameters,
        summarize_error_body,
    };
    use reqwest::StatusCode;
    use serde_json::{Value, json};
    use std::collections::HashMap;

    const TEST_RSA_PRIVATE_KEY_PKCS8: &str = r"-----BEGIN PRIVATE KEY-----
MIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQDJETqse41HRBsc
7cfcq3ak4oZWFCoZlcic525A3FfO4qW9BMtRO/iXiyCCHn8JhiL9y8j5JdVP2Q9Z
IpfElcFd3/guS9w+5RqQGgCR+H56IVUyHZWtTJbKPcwWXQdNUX0rBFcsBzCRESJL
eelOEdHIjG7LRkx5l/FUvlqsyHDVJEQsHwegZ8b8C0fz0EgT2MMEdn10t6Ur1rXz
jMB/wvCg8vG8lvciXmedyo9xJ8oMOh0wUEgxziVDMMovmC+aJctcHUAYubwoGN8T
yzcvnGqL7JSh36Pwy28iPzXZ2RLhAyJFU39vLaHdljwthUaupldlNyCfa6Ofy4qN
ctlUPlN1AgMBAAECggEAdESTQjQ70O8QIp1ZSkCYXeZjuhj081CK7jhhp/4ChK7J
GlFQZMwiBze7d6K84TwAtfQGZhQ7km25E1kOm+3hIDCoKdVSKch/oL54f/BK6sKl
qlIzQEAenho4DuKCm3I4yAw9gEc0DV70DuMTR0LEpYyXcNJY3KNBOTjN5EYQAR9s
2MeurpgK2MdJlIuZaIbzSGd+diiz2E6vkmcufJLtmYUT/k/ddWvEtz+1DnO6bRHh
xuuDMeJA/lGB/EYloSLtdyCF6sII6C6slJJtgfb0bPy7l8VtL5iDyz46IKyzdyzW
tKAn394dm7MYR1RlUBEfqFUyNK7C+pVMVoTwCC2V4QKBgQD64syfiQ2oeUlLYDm4
CcKSP3RnES02bcTyEDFSuGyyS1jldI4A8GXHJ/lG5EYgiYa1RUivge4lJrlNfjyf
dV230xgKms7+JiXqag1FI+3mqjAgg4mYiNjaao8N8O3/PD59wMPeWYImsWXNyeHS
55rUKiHERtCcvdzKl4u35ZtTqQKBgQDNKnX2bVqOJ4WSqCgHRhOm386ugPHfy+8j
m6cicmUR46ND6ggBB03bCnEG9OtGisxTo/TuYVRu3WP4KjoJs2LD5fwdwJqpgtHl
yVsk45Y1Hfo+7M6lAuR8rzCi6kHHNb0HyBmZjysHWZsn79ZM+sQnLpgaYgQGRbKV
DZWlbw7g7QKBgQCl1u+98UGXAP1jFutwbPsx40IVszP4y5ypCe0gqgon3UiY/G+1
zTLp79GGe/SjI2VpQ7AlW7TI2A0bXXvDSDi3/5Dfya9ULnFXv9yfvH1QwWToySpW
Kvd1gYSoiX84/WCtjZOr0e0HmLIb0vw0hqZA4szJSqoxQgvF22EfIWaIaQKBgQCf
34+OmMYw8fEvSCPxDxVvOwW2i7pvV14hFEDYIeZKW2W1HWBhVMzBfFB5SE8yaCQy
pRfOzj9aKOCm2FjjiErVNpkQoi6jGtLvScnhZAt/lr2TXTrl8OwVkPrIaN0bG/AS
aUYxmBPCpXu3UjhfQiWqFq/mFyzlqlgvuCc9g95HPQKBgAscKP8mLxdKwOgX8yFW
GcZ0izY/30012ajdHY+/QK5lsMoxTnn0skdS+spLxaS5ZEO4qvPVb8RAoCkWMMal
2pOhmquJQVDPDLuZHdrIiKiDM20dy9sMfHygWcZjQ4WSxf/J7T9canLZIXFhHAZT
3wc9h4G8BBCtWN2TN/LsGZdB
-----END PRIVATE KEY-----";

    #[test]
    fn normalize_account_url_defaults_to_https() {
        let url = normalize_account_url("org-account.snowflakecomputing.com/")
            .expect("valid account url");
        assert_eq!(url, "https://org-account.snowflakecomputing.com");
    }

    #[test]
    fn normalize_account_url_rejects_http_by_default() {
        let err = normalize_account_url("http://localhost:8080").expect_err("http should fail");
        assert!(err.to_string().contains("https"));
    }

    #[test]
    fn normalize_account_url_rejects_paths_queries_and_credentials() {
        for input in [
            "https://acct.snowflakecomputing.com/api",
            "https://acct.snowflakecomputing.com?token=secret",
            "https://user:pass@acct.snowflakecomputing.com",
        ] {
            assert!(
                normalize_account_url(input).is_err(),
                "{input} should be rejected"
            );
        }
    }

    #[test]
    fn snowflake_http_summary_redacts_auth_bodies() {
        let body = r#"{"code":"390303","message":"Invalid OAuth access token. ...TTTTTTTT"}"#;
        let summary = summarize_error_body(StatusCode::UNAUTHORIZED, body);
        assert!(!summary.contains("TTTTTTTT"));
        assert!(summary.contains("authorization failed"));
    }

    #[test]
    fn session_parameters_use_sql_api_field_shapes() {
        let params = session_parameters(250);
        assert_eq!(params["binary_output_format"], json!("HEX"));
        assert_eq!(params["rows_per_resultset"], json!(250));
        assert!(!params.contains_key("BINARY_OUTPUT_FORMAT"));
    }

    #[test]
    fn normalize_jwt_identifier_uppercases_and_replaces_periods() {
        assert_eq!(
            normalize_jwt_identifier("xy12345.us-east-1"),
            "XY12345-US-EAST-1"
        );
    }

    #[test]
    fn public_key_fingerprint_uses_snowflake_sha256_prefix() {
        let fingerprint = public_key_fingerprint(TEST_RSA_PRIVATE_KEY_PKCS8).expect("fingerprint");
        assert!(fingerprint.starts_with("SHA256:"));
        assert_eq!(fingerprint.len(), "SHA256:".len() + 44);
    }

    #[test]
    fn generate_keypair_jwt_returns_signed_token() {
        let token = generate_keypair_jwt("myorg-myaccount", "svc_user", TEST_RSA_PRIVATE_KEY_PKCS8)
            .expect("jwt");
        assert_eq!(token.split('.').count(), 3);
    }

    #[test]
    fn rewrite_named_parameters_uses_snowflake_positional_binds() {
        let params = HashMap::from([("date".to_string(), json!("2024-01-01"))]);
        let rewritten = rewrite_named_parameters(
            "select 'literal :date', amount::number from orders where order_date >= :date",
            &params,
        )
        .expect("rewrite");
        assert_eq!(
            rewritten.sql,
            "select 'literal :date', amount::number from orders where order_date >= ?"
        );
        assert_eq!(rewritten.ordered_parameters, vec!["date".to_string()]);
    }

    #[test]
    fn rewrite_named_parameters_skips_snowflake_variant_paths() {
        let params = HashMap::from([("country".to_string(), json!("GB"))]);
        let rewritten = rewrite_named_parameters(
            "select payload:customer_id::string, metadata:tags[0] from events where country = :country",
            &params,
        )
        .expect("rewrite");
        assert_eq!(
            rewritten.sql,
            "select payload:customer_id::string, metadata:tags[0] from events where country = ?"
        );
        assert_eq!(rewritten.ordered_parameters, vec!["country".to_string()]);
    }

    #[test]
    fn build_bindings_infers_and_numbers_by_sql_order() {
        let params = HashMap::from([
            ("country".to_string(), json!("GB")),
            ("min_amount".to_string(), json!(10)),
        ]);
        let order = vec!["country".to_string(), "min_amount".to_string()];
        let bindings = build_bindings(&order, &params, None).expect("bindings");

        assert_eq!(bindings["1"].type_name, "TEXT");
        assert_eq!(bindings["1"].value, json!("GB"));
        assert_eq!(bindings["2"].type_name, "FIXED");
        assert_eq!(bindings["2"].value, json!("10"));
    }

    #[test]
    fn build_bindings_requires_explicit_type_for_null() {
        let params = HashMap::from([("deleted_at".to_string(), Value::Null)]);
        let order = vec!["deleted_at".to_string()];
        let err = build_bindings(&order, &params, None).expect_err("null should require type");
        assert!(err.to_string().contains("requires explicit"));
    }

    #[test]
    fn parse_cell_value_converts_snowflake_strings_by_metadata() {
        let int_field = ResultColumn {
            name: "count".to_string(),
            type_name: "FIXED".to_string(),
            scale: Some(json!(0)),
        };
        let bool_field = ResultColumn {
            name: "flag".to_string(),
            type_name: "BOOLEAN".to_string(),
            scale: None,
        };
        let variant_field = ResultColumn {
            name: "payload".to_string(),
            type_name: "VARIANT".to_string(),
            scale: None,
        };

        assert_eq!(parse_cell_value(&json!("42"), &int_field), json!(42));
        assert_eq!(parse_cell_value(&json!("true"), &bool_field), json!(true));
        assert_eq!(
            parse_cell_value(&json!("{\"a\":1}"), &variant_field),
            json!({"a": 1})
        );
    }

    #[test]
    fn normalize_preflight_relation_uppercases_safe_unquoted_segments() {
        let relation = normalize_preflight_relation("analytics.orders").expect("valid relation");
        assert_eq!(relation, "ANALYTICS.ORDERS");
    }

    #[test]
    fn normalize_preflight_relation_rejects_injection() {
        let err = normalize_preflight_relation("orders;drop").expect_err("invalid relation");
        assert!(err.to_string().contains("Invalid relation"));
    }

    #[test]
    fn preflight_statements_are_bounded_and_safe() {
        assert_eq!(
            catalog_preflight_statement("ANALYTICS"),
            "SHOW DATABASES LIKE 'ANALYTICS'"
        );
        assert_eq!(
            schema_preflight_statement("ANALYTICS", "REPORTING"),
            "SHOW SCHEMAS LIKE 'REPORTING' IN DATABASE ANALYTICS"
        );
        assert_eq!(
            relation_preflight_statement("ANALYTICS.REPORTING.ORDERS"),
            "SELECT 1 AS relation_access_check FROM ANALYTICS.REPORTING.ORDERS LIMIT 1"
        );
    }
}
