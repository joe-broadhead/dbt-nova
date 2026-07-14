#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::env;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use reqwest::{Client, Url};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map as JsonMap, Value};
use tokio::sync::RwLock;
use tracing::warn;

use crate::error::{DbtNovaError, Result};
use crate::params::ExecuteSqlParams;
use crate::responses::SuccessResponse;
use crate::utils::{
    redact_sensitive_text, resolve_gcp_access_token_async, resolve_gcp_project_id,
    summarize_http_error_body,
};
use crate::warehouse::SqlProvider;
use crate::warehouse::preflight::{
    PreflightReport, ProbePresence, build_configuration_failure_response, build_preflight_response,
    empty_preflight_probe_message, preflight_probe_has_rows, run_connectivity_check,
    run_optional_object_check,
};

const DEFAULT_ROW_LIMIT: u64 = 1_000;
const DEFAULT_BYTE_LIMIT: u64 = 25_000_000;
const DEFAULT_POLL_INTERVAL_MS: u64 = 1_000;
const DEFAULT_MAX_POLL_SECONDS: u64 = 600;
const DEFAULT_MAX_CHUNKS: usize = 50;
const DEFAULT_TIMEOUT_MS: u64 = 30_000;
const DEFAULT_TOKEN_CACHE_TTL_SECS: u64 = 3_000;

static BIGQUERY_RUNTIME_CACHE: OnceLock<RwLock<Option<CachedBigQueryRuntime>>> = OnceLock::new();

fn bq_err(message: impl Into<String>) -> DbtNovaError {
    DbtNovaError::ServerError(format!("BigQuery error: {}", message.into()))
}

#[derive(Debug, Clone)]
struct BigQueryConfig {
    project_id: String,
    location: Option<String>,
    access_token: String,
    timeout: Duration,
    api_base_url: String,
}

#[derive(Debug, Clone)]
struct CachedBigQueryRuntime {
    config: BigQueryConfig,
    client: Client,
    loaded_at: Instant,
}

impl BigQueryConfig {
    async fn from_env_async() -> Result<Self> {
        let project_id = resolve_gcp_project_id(&["DBT_NOVA_BIGQUERY_PROJECT_ID"]).ok_or_else(
            || {
                bq_err(
                    "Missing BigQuery project id. Set DBT_NOVA_BIGQUERY_PROJECT_ID, DBT_NOVA_GCP_PROJECT_ID, or GOOGLE_CLOUD_PROJECT",
                )
            },
        )?;
        let project_id = normalize_project_id(&project_id)?;

        let location = env::var("DBT_NOVA_BIGQUERY_LOCATION")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());

        let access_token = resolve_gcp_access_token_async(&["DBT_NOVA_BIGQUERY_ACCESS_TOKEN"])
            .await
            .map_err(|err| bq_err(format!("Missing BigQuery access token. {err}")))?;

        let timeout_ms = env::var("DBT_NOVA_BIGQUERY_TIMEOUT_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(DEFAULT_TIMEOUT_MS)
            .max(1_000);
        let api_base_url = env::var("DBT_NOVA_BIGQUERY_API_BASE_URL")
            .ok()
            .map(|value| value.trim().trim_end_matches('/').to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "https://bigquery.googleapis.com".to_string());
        validate_bigquery_api_base_url(&api_base_url)?;

        Ok(Self {
            project_id,
            location,
            access_token,
            timeout: Duration::from_millis(timeout_ms),
            api_base_url,
        })
    }
}

fn bigquery_runtime_cache() -> &'static RwLock<Option<CachedBigQueryRuntime>> {
    BIGQUERY_RUNTIME_CACHE.get_or_init(|| RwLock::new(None))
}

fn bigquery_token_cache_ttl() -> Duration {
    let ttl_secs = env::var("DBT_NOVA_BIGQUERY_TOKEN_CACHE_TTL_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(DEFAULT_TOKEN_CACHE_TTL_SECS)
        .max(60);
    Duration::from_secs(ttl_secs)
}

fn build_bigquery_client(timeout: Duration) -> Result<Client> {
    Client::builder()
        .timeout(timeout)
        .build()
        .map_err(|err| bq_err(format!("failed to build HTTP client: {err}")))
}

async fn load_bigquery_runtime() -> Result<CachedBigQueryRuntime> {
    let config = BigQueryConfig::from_env_async().await?;
    let client = build_bigquery_client(config.timeout)?;
    Ok(CachedBigQueryRuntime {
        config,
        client,
        loaded_at: Instant::now(),
    })
}

async fn cached_bigquery_runtime() -> Result<CachedBigQueryRuntime> {
    let cache_ttl = bigquery_token_cache_ttl();
    let cache = bigquery_runtime_cache();

    {
        let guard = cache.read().await;
        if let Some(runtime) = guard.as_ref()
            && runtime.loaded_at.elapsed() < cache_ttl
        {
            return Ok(runtime.clone());
        }
    }

    let runtime = load_bigquery_runtime().await?;
    let mut guard = cache.write().await;
    if let Some(existing) = guard.as_ref()
        && existing.loaded_at.elapsed() < cache_ttl
    {
        return Ok(existing.clone());
    }
    *guard = Some(runtime.clone());
    Ok(runtime)
}

fn normalize_project_id(project_id: &str) -> Result<String> {
    let trimmed = project_id.trim();
    if trimmed.is_empty()
        || !trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(DbtNovaError::InvalidParams(format!(
            "Invalid BigQuery project id '{project_id}'"
        )));
    }
    Ok(trimmed.to_string())
}

fn validate_bigquery_api_base_url(value: &str) -> Result<()> {
    let url = Url::parse(value).map_err(|err| {
        DbtNovaError::InvalidParams(format!("Invalid DBT_NOVA_BIGQUERY_API_BASE_URL: {err}"))
    })?;
    match url.scheme() {
        "https" => Ok(()),
        "http" if is_loopback_host(url.host_str().unwrap_or_default()) => Ok(()),
        "http" => Err(DbtNovaError::InvalidParams(
            "DBT_NOVA_BIGQUERY_API_BASE_URL may use http:// only for loopback test servers"
                .to_string(),
        )),
        _ => Err(DbtNovaError::InvalidParams(
            "DBT_NOVA_BIGQUERY_API_BASE_URL must start with https://, or http:// for loopback tests"
                .to_string(),
        )),
    }
}

fn is_loopback_host(host: &str) -> bool {
    matches!(host, "localhost" | "127.0.0.1" | "::1")
}

fn normalize_dataset_identifier(dataset: &str, context: &str) -> Result<String> {
    let trimmed = dataset.trim();
    if trimmed.is_empty()
        || !trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
        || !trimmed
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
    {
        return Err(DbtNovaError::InvalidParams(format!(
            "Invalid {context} identifier '{dataset}'"
        )));
    }
    Ok(trimmed.to_string())
}

fn normalize_relation_name(relation: &str, default_project: &str) -> Result<String> {
    let trimmed = relation.trim();
    if trimmed.is_empty() {
        return Err(DbtNovaError::InvalidParams(
            "preflight_relation cannot be empty".to_string(),
        ));
    }

    let parts: Vec<&str> = trimmed.split('.').collect();
    let (project, dataset, table) = match parts.as_slice() {
        [dataset, table] => (
            default_project.to_string(),
            normalize_dataset_identifier(dataset, "dataset")?,
            normalize_dataset_identifier(table, "table")?,
        ),
        [project, dataset, table] => (
            normalize_project_id(project)?,
            normalize_dataset_identifier(dataset, "dataset")?,
            normalize_dataset_identifier(table, "table")?,
        ),
        _ => {
            return Err(DbtNovaError::InvalidParams(format!(
                "Invalid relation '{trimmed}': expected dataset.table or project.dataset.table"
            )));
        }
    };

    Ok(format!("`{project}.{dataset}.{table}`"))
}

#[derive(Debug, Clone)]
struct ExecuteSettings {
    row_limit: u64,
    byte_limit: u64,
    wait_timeout_s: Option<u64>,
    poll_interval: Duration,
    max_poll: Duration,
    fetch_all_chunks: bool,
    max_chunks: usize,
    parameters: Vec<QueryParameter>,
}

fn execute_settings(params: &ExecuteSqlParams) -> Result<ExecuteSettings> {
    let row_limit = params.row_limit.unwrap_or(DEFAULT_ROW_LIMIT).max(1);
    let byte_limit = params.byte_limit.unwrap_or(DEFAULT_BYTE_LIMIT).max(1);
    let poll_interval_ms = params
        .poll_interval_ms
        .unwrap_or(DEFAULT_POLL_INTERVAL_MS)
        .max(1);
    let max_poll_seconds = params
        .max_poll_seconds
        .unwrap_or(DEFAULT_MAX_POLL_SECONDS)
        .max(1);
    let fetch_all_chunks = params.fetch_all_chunks.unwrap_or(true);
    let max_chunks = params.max_chunks.unwrap_or(DEFAULT_MAX_CHUNKS).max(1);

    let parameters =
        build_query_parameters(params.parameters.clone(), params.parameter_types.clone())?;

    Ok(ExecuteSettings {
        row_limit,
        byte_limit,
        wait_timeout_s: params.wait_timeout_s,
        poll_interval: Duration::from_millis(poll_interval_ms),
        max_poll: Duration::from_secs(max_poll_seconds),
        fetch_all_chunks,
        max_chunks,
        parameters,
    })
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct QueryRequest {
    query: String,
    use_legacy_sql: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_results: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    timeout_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    location: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parameter_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    query_parameters: Option<Vec<QueryParameter>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct QueryParameter {
    name: String,
    parameter_type: QueryParameterType,
    parameter_value: QueryParameterValue,
}

#[derive(Debug, Clone, Serialize)]
struct QueryParameterType {
    #[serde(rename = "type")]
    type_name: String,
}

#[derive(Debug, Clone, Serialize)]
struct QueryParameterValue {
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct QueryResponse {
    job_complete: Option<bool>,
    job_reference: Option<JobReference>,
    schema: Option<QuerySchema>,
    rows: Option<Vec<QueryRow>>,
    page_token: Option<String>,
    total_rows: Option<String>,
    total_bytes_processed: Option<String>,
    errors: Option<Vec<QueryError>>,
    status: Option<QueryStatus>,
    location: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JobReference {
    job_id: String,
    location: Option<String>,
}

#[derive(Debug, Deserialize)]
struct QuerySchema {
    fields: Vec<QueryField>,
}

#[derive(Debug, Deserialize, Clone)]
struct QueryField {
    name: String,
    #[serde(rename = "type")]
    field_type: String,
}

#[derive(Debug, Deserialize)]
struct QueryRow {
    f: Vec<QueryCell>,
}

#[derive(Debug, Deserialize)]
struct QueryCell {
    v: Value,
}

#[derive(Debug, Deserialize)]
struct QueryError {
    message: Option<String>,
    reason: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct QueryStatus {
    error_result: Option<QueryError>,
    errors: Option<Vec<QueryError>>,
}

fn query_error(response: &QueryResponse) -> Option<String> {
    if let Some(status) = &response.status {
        if let Some(error) = &status.error_result {
            return Some(format_query_error(error));
        }
        if let Some(errors) = &status.errors
            && let Some(error) = errors.first()
        {
            return Some(format_query_error(error));
        }
    }
    if let Some(errors) = &response.errors
        && let Some(error) = errors.first()
    {
        return Some(format_query_error(error));
    }
    None
}

fn format_query_error(error: &QueryError) -> String {
    let reason = error
        .reason
        .as_deref()
        .map(redact_sensitive_text)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "unknown_reason".to_string());
    if error
        .message
        .as_deref()
        .is_some_and(|message| !message.trim().is_empty())
    {
        format!("{reason}: message redacted")
    } else {
        reason
    }
}

fn parse_u64(value: Option<&String>) -> Option<u64> {
    value.and_then(|v| v.parse::<u64>().ok())
}

fn preflight_query_has_rows(response: &QueryResponse) -> bool {
    let rows_len = response.rows.as_ref().map_or(0usize, Vec::len);
    preflight_probe_has_rows(rows_len, parse_u64(response.total_rows.as_ref()))
}

fn detail_field(key: &str, value: impl AsRef<str>) -> JsonMap<String, Value> {
    let mut details = JsonMap::new();
    details.insert(key.to_string(), Value::String(value.as_ref().to_string()));
    details
}

fn schema_catalog_details(project: &str, dataset: &str) -> JsonMap<String, Value> {
    let mut details = detail_field("schema", dataset);
    details.insert("catalog".to_string(), Value::String(project.to_string()));
    details
}

fn query_url(api_base_url: &str, project_id: &str) -> String {
    format!("{api_base_url}/bigquery/v2/projects/{project_id}/queries")
}

fn query_page_url(api_base_url: &str, project_id: &str, job_id: &str) -> String {
    format!("{api_base_url}/bigquery/v2/projects/{project_id}/queries/{job_id}")
}

fn job_cancel_url(api_base_url: &str, project_id: &str, job_id: &str) -> String {
    format!("{api_base_url}/bigquery/v2/projects/{project_id}/jobs/{job_id}/cancel")
}

async fn send_json<T: DeserializeOwned>(builder: reqwest::RequestBuilder) -> Result<T> {
    let response = builder
        .send()
        .await
        .map_err(|err| bq_err(format!("request failed: {err}")))?;

    let status = response.status();
    if status.is_success() {
        return response
            .json::<T>()
            .await
            .map_err(|err| bq_err(format!("failed to parse BigQuery response: {err}")));
    }

    let body = match response.text().await {
        Ok(body) => body,
        Err(err) => format!("<failed to read body: {err}>"),
    };
    let summary = summarize_http_error_body(status.as_u16(), &body);
    Err(bq_err(format!(
        "BigQuery API error (HTTP {}): {summary}",
        status.as_u16()
    )))
}

async fn post_query(
    client: &Client,
    config: &BigQueryConfig,
    request: &QueryRequest,
) -> Result<QueryResponse> {
    let url = query_url(&config.api_base_url, &config.project_id);
    let builder = client
        .post(url)
        .bearer_auth(&config.access_token)
        .json(request);
    send_json(builder).await
}

async fn get_query_page(
    client: &Client,
    config: &BigQueryConfig,
    job_id: &str,
    max_results: u64,
    page_token: Option<&str>,
    location: Option<&str>,
) -> Result<QueryResponse> {
    let url = query_page_url(&config.api_base_url, &config.project_id, job_id);
    let mut params = vec![("maxResults", max_results.to_string())];
    if let Some(token) = page_token {
        params.push(("pageToken", token.to_string()));
    }
    if let Some(location) = location {
        params.push(("location", location.to_string()));
    }

    let builder = client
        .get(url)
        .bearer_auth(&config.access_token)
        .query(&params);
    send_json(builder).await
}

async fn cancel_query_job(
    client: &Client,
    config: &BigQueryConfig,
    job_id: &str,
    location: Option<&str>,
) -> Result<()> {
    let url = job_cancel_url(&config.api_base_url, &config.project_id, job_id);
    let mut builder = client.post(url).bearer_auth(&config.access_token);
    if let Some(location) = location {
        builder = builder.query(&[("location", location)]);
    }
    send_json::<Value>(builder).await.map(|_| ())
}

fn infer_parameter_type(value: &Value) -> Result<&'static str> {
    match value {
        Value::Null => Err(DbtNovaError::InvalidParams(
            "BigQuery null parameters require explicit parameter_types".to_string(),
        )),
        Value::Bool(_) => Ok("BOOL"),
        Value::Number(number) => {
            if number.is_i64() || number.is_u64() {
                Ok("INT64")
            } else {
                Ok("FLOAT64")
            }
        }
        Value::String(_) => Ok("STRING"),
        Value::Array(_) | Value::Object(_) => Err(DbtNovaError::InvalidParams(
            "BigQuery parameters must be scalar JSON values".to_string(),
        )),
    }
}

fn parameter_value(value: &Value, type_name: &str) -> Result<QueryParameterValue> {
    let value = match value {
        Value::Null => {
            if type_name.trim().is_empty() {
                return Err(DbtNovaError::InvalidParams(
                    "BigQuery null parameters require explicit type".to_string(),
                ));
            }
            None
        }
        Value::Bool(v) => Some(v.to_string()),
        Value::Number(v) => Some(v.to_string()),
        Value::String(v) => Some(v.clone()),
        Value::Array(_) | Value::Object(_) => {
            return Err(DbtNovaError::InvalidParams(
                "BigQuery parameters must be scalar JSON values".to_string(),
            ));
        }
    };
    Ok(QueryParameterValue { value })
}

fn build_query_parameters(
    params: Option<HashMap<String, Value>>,
    types: Option<HashMap<String, String>>,
) -> Result<Vec<QueryParameter>> {
    let params = params.unwrap_or_default();
    let types = types.unwrap_or_default();

    for key in types.keys() {
        if !params.contains_key(key) {
            return Err(DbtNovaError::InvalidParams(format!(
                "parameter_types contains '{key}' but parameters does not"
            )));
        }
    }

    let mut keys: Vec<String> = params.keys().cloned().collect();
    keys.sort();

    let mut output = Vec::with_capacity(keys.len());
    for key in keys {
        let Some(value) = params.get(&key) else {
            continue;
        };
        let type_name = match types.get(&key) {
            Some(explicit) => explicit.clone(),
            None => infer_parameter_type(value)?.to_string(),
        };
        output.push(QueryParameter {
            name: key,
            parameter_type: QueryParameterType {
                type_name: type_name.clone(),
            },
            parameter_value: parameter_value(value, &type_name)?,
        });
    }

    Ok(output)
}

fn to_timeout_ms(wait_timeout_s: Option<u64>) -> Option<u64> {
    wait_timeout_s.map(|seconds| seconds.saturating_mul(1_000))
}

fn parse_cell_value(value: &Value, field_type: &str) -> Value {
    if value.is_null() {
        return Value::Null;
    }

    if let Some(text) = value.as_str() {
        let upper = field_type.to_ascii_uppercase();
        return match upper.as_str() {
            "INT64" | "INTEGER" => text
                .parse::<i64>()
                .map_or_else(|_| Value::from(text.to_string()), Value::from),
            "FLOAT64" | "FLOAT" | "NUMERIC" | "BIGNUMERIC" => text
                .parse::<f64>()
                .map_or_else(|_| Value::from(text.to_string()), Value::from),
            "BOOL" | "BOOLEAN" => match text.to_ascii_lowercase().as_str() {
                "true" => Value::from(true),
                "false" => Value::from(false),
                _ => Value::from(text.to_string()),
            },
            _ => Value::from(text.to_string()),
        };
    }

    value.clone()
}

fn append_rows(
    all_rows: &mut Vec<Vec<Value>>,
    schema_fields: &[QueryField],
    chunk_rows: &[QueryRow],
    row_limit: u64,
    byte_limit: u64,
    approx_bytes: &mut u64,
    truncated: &mut bool,
) -> Result<()> {
    for row in chunk_rows {
        let row_count = u64::try_from(all_rows.len()).unwrap_or(u64::MAX);
        if row_count >= row_limit {
            *truncated = true;
            break;
        }

        let mut output_row = Vec::with_capacity(schema_fields.len());
        for (idx, field) in schema_fields.iter().enumerate() {
            let cell = row.f.get(idx).map_or(&Value::Null, |c| &c.v);
            output_row.push(parse_cell_value(cell, &field.field_type));
        }

        let row_bytes = u64::try_from(
            serde_json::to_vec(&output_row)
                .map_err(|err| bq_err(format!("failed to serialize row: {err}")))?
                .len(),
        )
        .unwrap_or(u64::MAX);

        if approx_bytes.saturating_add(row_bytes) > byte_limit {
            *truncated = true;
            break;
        }

        *approx_bytes = approx_bytes.saturating_add(row_bytes);
        all_rows.push(output_row);
    }

    Ok(())
}

async fn run_query(
    client: &Client,
    config: &BigQueryConfig,
    statement: &str,
    settings: &ExecuteSettings,
) -> Result<(QueryResponse, String, Option<String>)> {
    let request = QueryRequest {
        query: statement.to_string(),
        use_legacy_sql: false,
        max_results: Some(settings.row_limit),
        timeout_ms: to_timeout_ms(settings.wait_timeout_s),
        location: config.location.clone(),
        parameter_mode: (!settings.parameters.is_empty()).then_some("NAMED".to_string()),
        query_parameters: (!settings.parameters.is_empty()).then_some(settings.parameters.clone()),
    };

    let mut response = post_query(client, config, &request).await?;
    if let Some(err) = query_error(&response) {
        return Err(bq_err(format!("query failed: {err}")));
    }

    let job_reference = response
        .job_reference
        .as_ref()
        .ok_or_else(|| bq_err("query response missing job reference"))?;
    let job_id = job_reference.job_id.clone();
    let location = job_reference
        .location
        .clone()
        .or_else(|| response.location.clone())
        .or_else(|| config.location.clone());

    if !response.job_complete.unwrap_or(true) {
        let poll_started = Instant::now();
        loop {
            if poll_started.elapsed() > settings.max_poll {
                return Err(query_poll_timeout_error(
                    client,
                    config,
                    &job_id,
                    location.as_deref(),
                    settings.max_poll,
                )
                .await);
            }

            let remaining = settings
                .max_poll
                .checked_sub(poll_started.elapsed())
                .unwrap_or_else(|| Duration::from_secs(0));
            tokio::time::sleep(settings.poll_interval.min(remaining)).await;
            if poll_started.elapsed() >= settings.max_poll {
                return Err(query_poll_timeout_error(
                    client,
                    config,
                    &job_id,
                    location.as_deref(),
                    settings.max_poll,
                )
                .await);
            }
            response = get_query_page(
                client,
                config,
                &job_id,
                settings.row_limit,
                None,
                location.as_deref(),
            )
            .await?;
            if let Some(err) = query_error(&response) {
                return Err(bq_err(format!("query failed: {err}")));
            }
            if response.job_complete.unwrap_or(true) {
                break;
            }
        }
    }

    Ok((response, job_id, location))
}

async fn query_poll_timeout_error(
    client: &Client,
    config: &BigQueryConfig,
    job_id: &str,
    location: Option<&str>,
    max_poll: Duration,
) -> DbtNovaError {
    if let Err(err) = cancel_query_job(client, config, job_id, location).await {
        warn!(
            job_id,
            error = %err,
            "failed to cancel BigQuery job after local poll timeout"
        );
    }
    bq_err(format!(
        "query polling timed out after {}s (job_id={job_id})",
        max_poll.as_secs()
    ))
}

#[derive(Debug)]
struct QueryExecutionResult {
    job_id: String,
    location: Option<String>,
    columns: Vec<String>,
    column_types: Vec<String>,
    rows: Vec<Vec<Value>>,
    fetched_chunks: u64,
    total_row_count: Option<u64>,
    total_byte_count: Option<u64>,
    truncated: bool,
}

#[allow(clippy::too_many_lines)]
async fn execute_bigquery_statement(
    statement: &str,
    settings: &ExecuteSettings,
) -> Result<QueryExecutionResult> {
    let runtime = cached_bigquery_runtime().await?;
    execute_bigquery_statement_with_runtime(&runtime.client, &runtime.config, statement, settings)
        .await
}

#[allow(clippy::too_many_lines)]
async fn execute_bigquery_statement_with_runtime(
    client: &Client,
    config: &BigQueryConfig,
    statement: &str,
    settings: &ExecuteSettings,
) -> Result<QueryExecutionResult> {
    let (mut response, job_id, location) = run_query(client, config, statement, settings).await?;

    let mut fields = response
        .schema
        .as_ref()
        .map(|schema| schema.fields.clone())
        .unwrap_or_default();
    let mut columns: Vec<String> = fields.iter().map(|f| f.name.clone()).collect();
    let mut column_types: Vec<String> = fields.iter().map(|f| f.field_type.clone()).collect();

    let mut rows: Vec<Vec<Value>> = Vec::new();
    let mut fetched_chunks: u64 = 0;
    let mut approx_bytes: u64 = 0;
    let mut truncated = false;

    if let Some(chunk_rows) = response.rows.as_ref() {
        fetched_chunks = fetched_chunks.saturating_add(1);
        append_rows(
            &mut rows,
            &fields,
            chunk_rows,
            settings.row_limit,
            settings.byte_limit,
            &mut approx_bytes,
            &mut truncated,
        )?;
    }

    let mut total_row_count = parse_u64(response.total_rows.as_ref());
    let mut total_byte_count = parse_u64(response.total_bytes_processed.as_ref());

    let mut next_page_token = response.page_token.take();
    if !settings.fetch_all_chunks && next_page_token.is_some() {
        truncated = true;
    }

    while settings.fetch_all_chunks && !truncated {
        let Some(page_token) = next_page_token.take() else {
            break;
        };

        let max_chunks_u64 = u64::try_from(settings.max_chunks).unwrap_or(u64::MAX);
        if fetched_chunks >= max_chunks_u64 {
            truncated = true;
            break;
        }

        let page = get_query_page(
            client,
            config,
            &job_id,
            settings.row_limit,
            Some(&page_token),
            location.as_deref(),
        )
        .await?;

        if let Some(err) = query_error(&page) {
            return Err(bq_err(format!("query failed: {err}")));
        }

        if fields.is_empty()
            && let Some(schema) = page.schema.as_ref()
        {
            fields = schema.fields.clone();
            columns = fields.iter().map(|f| f.name.clone()).collect();
            column_types = fields.iter().map(|f| f.field_type.clone()).collect();
        }

        if let Some(chunk_rows) = page.rows.as_ref() {
            fetched_chunks = fetched_chunks.saturating_add(1);
            append_rows(
                &mut rows,
                &fields,
                chunk_rows,
                settings.row_limit,
                settings.byte_limit,
                &mut approx_bytes,
                &mut truncated,
            )?;
        }

        if total_row_count.is_none() {
            total_row_count = parse_u64(page.total_rows.as_ref());
        }
        if total_byte_count.is_none() {
            total_byte_count = parse_u64(page.total_bytes_processed.as_ref());
        }

        next_page_token = page.page_token;
    }

    Ok(QueryExecutionResult {
        job_id,
        location,
        columns,
        column_types,
        rows,
        fetched_chunks,
        total_row_count,
        total_byte_count,
        truncated,
    })
}

fn catalog_preflight_statement(project: &str) -> String {
    format!("SELECT schema_name FROM `{project}.INFORMATION_SCHEMA.SCHEMATA` LIMIT 1")
}

fn schema_preflight_statement(project: &str, dataset: &str) -> String {
    format!("SELECT table_name FROM `{project}.{dataset}.INFORMATION_SCHEMA.TABLES` LIMIT 1")
}

fn relation_preflight_statement(relation: &str) -> String {
    format!("SELECT 1 AS relation_access_check FROM {relation} LIMIT 1")
}

async fn execute_bigquery(params: &ExecuteSqlParams) -> Result<Value> {
    let statement = params.statement.trim();
    if statement.is_empty() {
        return Err(DbtNovaError::InvalidParams(
            "statement cannot be empty".to_string(),
        ));
    }

    let settings = execute_settings(params)?;
    let started = Instant::now();
    let execution = execute_bigquery_statement(statement, &settings).await?;

    let payload = serde_json::json!({
        "statement_id": execution.job_id,
        "state": "SUCCEEDED",
        "provider": "bigquery",
        "location": execution.location,
        "columns": execution.columns,
        "column_types": execution.column_types,
        "rows": execution.rows,
        "elapsed_ms": u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        "fetched_chunks": execution.fetched_chunks,
        "stats": {
            "total_row_count": execution.total_row_count,
            "total_byte_count": execution.total_byte_count,
            "total_chunk_count": execution.fetched_chunks,
        },
        "truncated": execution.truncated,
    });

    let count = payload["rows"].as_array().map_or(0usize, Vec::len);
    serde_json::to_value(SuccessResponse::new(payload, count)).map_err(Into::into)
}

#[allow(clippy::too_many_lines)]
async fn preflight_bigquery(params: &ExecuteSqlParams) -> Result<Value> {
    let runtime = match cached_bigquery_runtime().await {
        Ok(runtime) => runtime,
        Err(err) => {
            return build_configuration_failure_response(
                "bigquery",
                JsonMap::new(),
                err.to_string(),
                "Set DBT_NOVA_BIGQUERY_PROJECT_ID/DBT_NOVA_GCP_PROJECT_ID and provide credentials via DBT_NOVA_BIGQUERY_ACCESS_TOKEN, DBT_NOVA_GCP_ACCESS_TOKEN, GOOGLE_APPLICATION_CREDENTIALS, or gcloud ADC",
            );
        }
    };

    let config = runtime.config;
    let client = runtime.client;
    let mut metadata = JsonMap::new();
    metadata.insert(
        "project_id".to_string(),
        Value::String(config.project_id.clone()),
    );
    metadata.insert(
        "location".to_string(),
        config.location.clone().map_or(Value::Null, Value::String),
    );

    let check_settings = ExecuteSettings {
        row_limit: 1,
        byte_limit: 1024,
        wait_timeout_s: Some(10),
        poll_interval: Duration::from_millis(DEFAULT_POLL_INTERVAL_MS),
        max_poll: Duration::from_secs(30),
        fetch_all_chunks: false,
        max_chunks: 1,
        parameters: Vec::new(),
    };
    let client_ref = &client;
    let config_ref = &config;
    let check_settings_ref = &check_settings;
    let mut report = PreflightReport::new();

    run_connectivity_check(
        &mut report,
        "Verify BigQuery credentials and project access",
        || async {
            run_query(
                client_ref,
                config_ref,
                "SELECT 1 AS connectivity_check",
                check_settings_ref,
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
        normalize_project_id,
        |project| {
            let statement = catalog_preflight_statement(project);
            async move {
                let (response, _, _) =
                    run_query(client_ref, config_ref, &statement, check_settings_ref).await?;
                Ok(if preflight_query_has_rows(&response) {
                    ProbePresence::Present
                } else {
                    ProbePresence::Empty
                })
            }
        },
        |catalog| detail_field("catalog", catalog),
        |project| detail_field("catalog", project),
        "Use a valid BigQuery project id",
        "Verify project exists and token has BigQuery metadata permissions",
        &empty_preflight_probe_message("catalog_access"),
    )
    .await;

    let project_for_schema = params
        .preflight_catalog
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(config.project_id.as_str())
        .to_string();

    run_optional_object_check(
        &mut report,
        params.preflight_schema.as_deref(),
        "schema_access",
        |schema| {
            let project = normalize_project_id(&project_for_schema)?;
            let dataset = normalize_dataset_identifier(schema, "dataset")?;
            Ok((project, dataset))
        },
        |(project, dataset)| {
            let statement = schema_preflight_statement(project, dataset);
            async move {
                let (response, _, _) =
                    run_query(client_ref, config_ref, &statement, check_settings_ref).await?;
                Ok(if preflight_query_has_rows(&response) {
                    ProbePresence::Present
                } else {
                    ProbePresence::Empty
                })
            }
        },
        |schema| detail_field("schema", schema),
        |(project, dataset)| schema_catalog_details(project, dataset),
        "Use valid project and dataset identifiers",
        "Verify dataset exists and token has BigQuery dataset metadata permissions",
        &empty_preflight_probe_message("schema_access"),
    )
    .await;

    let default_project = config.project_id.clone();
    run_optional_object_check(
        &mut report,
        params.preflight_relation.as_deref(),
        "relation_access",
        |relation| normalize_relation_name(relation, &default_project),
        |relation| {
            let statement = relation_preflight_statement(relation);
            async move {
                let (response, _, _) =
                    run_query(client_ref, config_ref, &statement, check_settings_ref).await?;
                Ok(if preflight_query_has_rows(&response) {
                    ProbePresence::Present
                } else {
                    ProbePresence::Empty
                })
            }
        },
        |relation| detail_field("relation", relation),
        |relation| detail_field("relation", relation),
        "Use dataset.table or project.dataset.table",
        "Verify table exists and token has BigQuery data viewer permissions",
        &empty_preflight_probe_message("relation_access"),
    )
    .await;

    build_preflight_response("bigquery", metadata, report)
}

pub struct BigQueryProvider;

pub static BIGQUERY_PROVIDER: BigQueryProvider = BigQueryProvider;

impl SqlProvider for BigQueryProvider {
    fn name(&self) -> &'static str {
        "bigquery"
    }

    fn execute<'a>(
        &'a self,
        params: &'a ExecuteSqlParams,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value>> + Send + 'a>> {
        Box::pin(async move { execute_bigquery(params).await })
    }

    fn preflight<'a>(
        &'a self,
        params: &'a ExecuteSqlParams,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value>> + Send + 'a>> {
        Box::pin(async move { preflight_bigquery(params).await })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BigQueryConfig, ExecuteSettings, QueryError, QueryResponse, build_bigquery_client,
        build_query_parameters, catalog_preflight_statement,
        execute_bigquery_statement_with_runtime, format_query_error, normalize_project_id,
        normalize_relation_name, preflight_query_has_rows, relation_preflight_statement,
        schema_preflight_statement, summarize_http_error_body, validate_bigquery_api_base_url,
    };
    use serde_json::json;
    use std::collections::HashMap;
    use std::time::Duration;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn normalize_project_allows_hyphen() {
        let project = normalize_project_id("my-project-123").expect("valid project id");
        assert_eq!(project, "my-project-123");
    }

    #[test]
    fn bigquery_api_base_url_allows_https_and_loopback_http_only() {
        validate_bigquery_api_base_url("https://bigquery.googleapis.com").expect("https");
        validate_bigquery_api_base_url("http://127.0.0.1:8080").expect("loopback");
        validate_bigquery_api_base_url("http://localhost:8080").expect("localhost");

        let err = validate_bigquery_api_base_url("http://metadata.google.internal")
            .expect_err("non-loopback http should fail");
        assert!(err.to_string().contains("loopback"));
    }

    #[test]
    fn bigquery_http_errors_do_not_expose_raw_body() {
        let body = r#"{
            "error": {
                "code": 400,
                "status": "INVALID_ARGUMENT",
                "message": "Bad SQL select * from users where token='raw-token'"
            }
        }"#;
        let summary = summarize_http_error_body(400, body);

        assert!(summary.contains("status=INVALID_ARGUMENT"));
        assert!(summary.contains("message redacted"));
        assert!(!summary.contains("raw-token"));
        assert!(!summary.contains("select *"));
        assert!(!summary.contains("users"));
    }

    #[test]
    fn bigquery_statement_errors_do_not_expose_raw_message() {
        let err = QueryError {
            reason: Some("invalidQuery".to_string()),
            message: Some("Bad SQL select * from users where token='raw-token'".to_string()),
        };
        let formatted = format_query_error(&err);

        assert_eq!(formatted, "invalidQuery: message redacted");
        assert!(!formatted.contains("raw-token"));
        assert!(!formatted.contains("select *"));
        assert!(!formatted.contains("users"));
    }

    #[tokio::test]
    async fn bigquery_query_fetches_additional_result_pages() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/bigquery/v2/projects/test-project/queries"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "jobComplete": true,
                "jobReference": {
                    "jobId": "job_123",
                    "location": "US"
                },
                "schema": {
                    "fields": [{"name": "order_count", "type": "INT64"}]
                },
                "rows": [{"f": [{"v": "1"}]}],
                "pageToken": "next-page",
                "totalRows": "2",
                "totalBytesProcessed": "128"
            })))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/bigquery/v2/projects/test-project/queries/job_123"))
            .and(query_param("maxResults", "10"))
            .and(query_param("pageToken", "next-page"))
            .and(query_param("location", "US"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "jobComplete": true,
                "schema": {
                    "fields": [{"name": "order_count", "type": "INT64"}]
                },
                "rows": [{"f": [{"v": "2"}]}],
                "totalRows": "2",
                "totalBytesProcessed": "128"
            })))
            .mount(&server)
            .await;

        let config = BigQueryConfig {
            project_id: "test-project".to_string(),
            location: Some("US".to_string()),
            access_token: "test-token".to_string(),
            timeout: Duration::from_secs(5),
            api_base_url: server.uri(),
        };
        let client = build_bigquery_client(config.timeout).expect("client");
        let settings = ExecuteSettings {
            row_limit: 10,
            byte_limit: 1_000,
            wait_timeout_s: Some(1),
            poll_interval: Duration::from_millis(1),
            max_poll: Duration::from_secs(1),
            fetch_all_chunks: true,
            max_chunks: 5,
            parameters: Vec::new(),
        };

        let result =
            execute_bigquery_statement_with_runtime(&client, &config, "select 1", &settings)
                .await
                .expect("query execution should succeed");

        assert_eq!(result.job_id, "job_123");
        assert_eq!(result.columns, vec!["order_count".to_string()]);
        assert_eq!(result.column_types, vec!["INT64".to_string()]);
        assert_eq!(result.rows, vec![vec![json!(1)], vec![json!(2)]]);
        assert_eq!(result.fetched_chunks, 2);
        assert_eq!(result.total_row_count, Some(2));
        assert_eq!(result.total_byte_count, Some(128));
        assert!(!result.truncated);
    }

    #[tokio::test]
    async fn bigquery_query_cancels_job_on_local_poll_timeout() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/bigquery/v2/projects/test-project/queries"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "jobComplete": false,
                "jobReference": {
                    "jobId": "job_slow",
                    "location": "US"
                }
            })))
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path(
                "/bigquery/v2/projects/test-project/jobs/job_slow/cancel",
            ))
            .and(query_param("location", "US"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "kind": "bigquery#jobCancelResponse"
            })))
            .expect(1)
            .mount(&server)
            .await;

        let config = BigQueryConfig {
            project_id: "test-project".to_string(),
            location: Some("US".to_string()),
            access_token: "test-token".to_string(),
            timeout: Duration::from_secs(5),
            api_base_url: server.uri(),
        };
        let client = build_bigquery_client(config.timeout).expect("client");
        let settings = ExecuteSettings {
            row_limit: 10,
            byte_limit: 1_000,
            wait_timeout_s: Some(1),
            poll_interval: Duration::from_millis(10),
            max_poll: Duration::from_millis(1),
            fetch_all_chunks: true,
            max_chunks: 5,
            parameters: Vec::new(),
        };

        let err = execute_bigquery_statement_with_runtime(&client, &config, "select 1", &settings)
            .await
            .expect_err("poll timeout should fail");

        assert!(
            err.to_string().contains("timed out"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn normalize_relation_two_part_uses_default_project() {
        let relation =
            normalize_relation_name("analytics.orders", "my-project").expect("valid relation");
        assert_eq!(relation, "`my-project.analytics.orders`");
    }

    #[test]
    fn normalize_relation_three_part_is_preserved() {
        let relation = normalize_relation_name("my-project.analytics.orders", "unused")
            .expect("valid relation");
        assert_eq!(relation, "`my-project.analytics.orders`");
    }

    #[test]
    fn relation_preflight_statement_wraps_backticks() {
        let statement = relation_preflight_statement("`my-project.analytics.orders`");
        assert_eq!(
            statement,
            "SELECT 1 AS relation_access_check FROM `my-project.analytics.orders` LIMIT 1"
        );
    }

    #[test]
    fn schema_preflight_statement_uses_information_schema() {
        let statement = schema_preflight_statement("my-project", "analytics");
        assert_eq!(
            statement,
            "SELECT table_name FROM `my-project.analytics.INFORMATION_SCHEMA.TABLES` LIMIT 1"
        );
    }

    #[test]
    fn catalog_preflight_statement_uses_information_schema() {
        let statement = catalog_preflight_statement("my-project");
        assert_eq!(
            statement,
            "SELECT schema_name FROM `my-project.INFORMATION_SCHEMA.SCHEMATA` LIMIT 1"
        );
    }

    #[test]
    fn build_query_parameters_infers_scalar_types() {
        let mut params = HashMap::new();
        params.insert("is_active".to_string(), json!(true));
        params.insert("order_id".to_string(), json!(42));
        params.insert("ratio".to_string(), json!(1.5));
        params.insert("country".to_string(), json!("FR"));

        let output = build_query_parameters(Some(params), None).expect("parameters should build");
        assert_eq!(output.len(), 4);
        assert_eq!(output[0].name, "country");
        assert_eq!(output[0].parameter_type.type_name, "STRING");
        assert_eq!(output[1].parameter_type.type_name, "BOOL");
        assert_eq!(output[2].parameter_type.type_name, "INT64");
        assert_eq!(output[3].parameter_type.type_name, "FLOAT64");
    }

    #[test]
    fn build_query_parameters_rejects_complex_values() {
        let mut params = HashMap::new();
        params.insert("bad".to_string(), json!({"nested": true}));

        let err = build_query_parameters(Some(params), None)
            .expect_err("complex parameters should be rejected");
        assert!(err.to_string().contains("scalar JSON values"));
    }

    #[test]
    fn preflight_query_has_rows_accepts_materialized_rows() {
        let response: QueryResponse = serde_json::from_value(json!({
            "rows": [{"f": [{"v": "1"}]}]
        }))
        .expect("query response should deserialize");

        assert!(preflight_query_has_rows(&response));
    }

    #[test]
    fn preflight_query_has_rows_accepts_total_rows_without_rows_payload() {
        let response: QueryResponse = serde_json::from_value(json!({
            "totalRows": "1"
        }))
        .expect("query response should deserialize");

        assert!(preflight_query_has_rows(&response));
    }

    #[test]
    fn preflight_query_has_rows_rejects_empty_probe() {
        let response: QueryResponse = serde_json::from_value(json!({
            "rows": [],
            "totalRows": "0"
        }))
        .expect("query response should deserialize");

        assert!(!preflight_query_has_rows(&response));
    }
}
