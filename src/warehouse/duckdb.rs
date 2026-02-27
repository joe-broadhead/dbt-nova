#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::Instant;

use base64::Engine as _;
use duckdb::params_from_iter;
use duckdb::types::Value as DuckValue;
use duckdb::{AccessMode, Config, Connection};
use serde_json::{Map as JsonMap, Value, json};
use tokio::sync::{OwnedSemaphorePermit, RwLock, Semaphore};

use crate::error::{DbtNovaError, Result};
use crate::params::ExecuteSqlParams;
use crate::responses::SuccessResponse;
use crate::warehouse::SqlProvider;
use crate::warehouse::preflight::{
    PreflightReport, ProbePresence, build_configuration_failure_response, build_preflight_response,
    run_connectivity_check_sync, run_optional_object_check_sync,
};

const DEFAULT_ROW_LIMIT: u64 = 1_000;
const DEFAULT_BYTE_LIMIT: u64 = 25_000_000;
const DEFAULT_DUCKDB_POOL_MAX_SIZE: usize = 10;

static DUCKDB_POOL_REGISTRY: OnceLock<RwLock<HashMap<DuckDbPoolKey, Arc<DuckDbConnectionPool>>>> =
    OnceLock::new();

#[derive(Debug, Clone)]
struct DuckDbConfig {
    path: PathBuf,
    file_search_path: Option<String>,
}

#[derive(Debug, Clone, Eq, Hash, PartialEq)]
struct DuckDbPoolKey {
    path: PathBuf,
    file_search_path: Option<String>,
}

impl DuckDbPoolKey {
    fn from_config(config: &DuckDbConfig) -> Self {
        Self {
            path: config.path.clone(),
            file_search_path: config.file_search_path.clone(),
        }
    }
}

#[derive(Debug)]
struct DuckDbPoolState {
    idle: Vec<Connection>,
    created: usize,
}

impl DuckDbPoolState {
    fn new() -> Self {
        Self {
            idle: Vec::new(),
            created: 0,
        }
    }
}

#[derive(Debug)]
struct DuckDbConnectionPool {
    config: DuckDbConfig,
    max_size: usize,
    slots: Arc<Semaphore>,
    state: StdMutex<DuckDbPoolState>,
}

impl DuckDbConnectionPool {
    fn new(config: DuckDbConfig, max_size: usize) -> Self {
        let effective_max_size = max_size.max(1);
        Self {
            config,
            max_size: effective_max_size,
            slots: Arc::new(Semaphore::new(effective_max_size)),
            state: StdMutex::new(DuckDbPoolState::new()),
        }
    }

    async fn acquire_slot(&self) -> Result<OwnedSemaphorePermit> {
        Arc::clone(&self.slots)
            .acquire_owned()
            .await
            .map_err(|_| duckdb_runtime_error("DuckDB connection pool semaphore is closed"))
    }

    fn checkout_connection(&self) -> Result<Connection> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(connection) = state.idle.pop() {
            return Ok(connection);
        }
        if state.created >= self.max_size {
            return Err(duckdb_runtime_error(
                "DuckDB connection pool exhausted unexpectedly",
            ));
        }
        state.created = state.created.saturating_add(1);
        drop(state);

        match open_connection(&self.config) {
            Ok(connection) => Ok(connection),
            Err(err) => {
                let mut state = self
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                state.created = state.created.saturating_sub(1);
                Err(err)
            }
        }
    }

    fn return_connection(&self, connection: Connection) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.idle.len() < self.max_size {
            state.idle.push(connection);
            return;
        }
        state.created = state.created.saturating_sub(1);
    }
}

impl DuckDbConfig {
    fn from_env() -> Result<Self> {
        let raw_path = env::var("DBT_NOVA_DUCKDB_PATH").map_err(|_| {
            DbtNovaError::InvalidParams(
                "DBT_NOVA_DUCKDB_PATH environment variable is required when DBT_NOVA_SQL_PROVIDER=duckdb"
                    .to_string(),
            )
        })?;

        let trimmed = raw_path.trim();
        if trimmed.is_empty() {
            return Err(DbtNovaError::InvalidParams(
                "DBT_NOVA_DUCKDB_PATH cannot be empty".to_string(),
            ));
        }

        let path = PathBuf::from(trimmed);
        validate_duckdb_path(&path)?;

        let file_search_path = env::var("DBT_NOVA_DUCKDB_FILE_SEARCH_PATH")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());

        Ok(Self {
            path,
            file_search_path,
        })
    }
}

fn duckdb_pool_registry() -> &'static RwLock<HashMap<DuckDbPoolKey, Arc<DuckDbConnectionPool>>> {
    DUCKDB_POOL_REGISTRY.get_or_init(|| RwLock::new(HashMap::new()))
}

fn resolve_duckdb_pool_max_size(
    explicit_pool_size: Option<usize>,
    fallback_sql_concurrency: Option<usize>,
) -> usize {
    explicit_pool_size
        .or(fallback_sql_concurrency)
        .unwrap_or(DEFAULT_DUCKDB_POOL_MAX_SIZE)
        .max(1)
}

fn duckdb_pool_max_size() -> usize {
    let explicit_pool_size = env::var("DBT_NOVA_DUCKDB_POOL_MAX_SIZE")
        .ok()
        .and_then(|value| value.parse::<usize>().ok());
    let fallback_sql_concurrency = env::var("DBT_NOVA_SQL_MAX_CONCURRENT")
        .ok()
        .and_then(|value| value.parse::<usize>().ok());
    resolve_duckdb_pool_max_size(explicit_pool_size, fallback_sql_concurrency)
}

async fn cached_duckdb_pool(config: &DuckDbConfig) -> Arc<DuckDbConnectionPool> {
    let key = DuckDbPoolKey::from_config(config);
    let registry = duckdb_pool_registry();

    {
        let guard = registry.read().await;
        if let Some(pool) = guard.get(&key) {
            return Arc::clone(pool);
        }
    }

    let mut guard = registry.write().await;
    if let Some(pool) = guard.get(&key) {
        return Arc::clone(pool);
    }
    let pool = Arc::new(DuckDbConnectionPool::new(
        config.clone(),
        duckdb_pool_max_size(),
    ));
    guard.insert(key, Arc::clone(&pool));
    pool
}

fn validate_duckdb_path(path: &Path) -> Result<()> {
    if !path.exists() {
        return Err(DbtNovaError::InvalidParams(format!(
            "DBT_NOVA_DUCKDB_PATH does not exist: {}",
            path.display()
        )));
    }
    if !path.is_file() {
        return Err(DbtNovaError::InvalidParams(format!(
            "DBT_NOVA_DUCKDB_PATH must point to a DuckDB file: {}",
            path.display()
        )));
    }
    std::fs::File::open(path).map_err(|err| {
        DbtNovaError::InvalidParams(format!(
            "DBT_NOVA_DUCKDB_PATH is not readable ({}): {err}",
            path.display()
        ))
    })?;
    Ok(())
}

fn duckdb_runtime_error(message: impl Into<String>) -> DbtNovaError {
    DbtNovaError::ServerError(format!("DuckDB error: {}", message.into()))
}

fn sql_string_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn open_connection(config: &DuckDbConfig) -> Result<Connection> {
    let open_config = Config::default()
        .access_mode(AccessMode::ReadOnly)
        .map_err(|err| {
            duckdb_runtime_error(format!("failed to configure read-only mode: {err}"))
        })?;

    let connection = Connection::open_with_flags(&config.path, open_config)
        .map_err(|err| duckdb_runtime_error(format!("failed to open database: {err}")))?;

    if let Some(file_search_path) = &config.file_search_path {
        let statement = format!(
            "SET file_search_path = {}",
            sql_string_literal(file_search_path)
        );
        connection.execute_batch(&statement).map_err(|err| {
            duckdb_runtime_error(format!(
                "failed to apply DBT_NOVA_DUCKDB_FILE_SEARCH_PATH: {err}"
            ))
        })?;
    }

    Ok(connection)
}

#[derive(Debug, Clone)]
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
                    if index + 1 < bytes.len() && is_identifier_start(bytes[index + 1]) {
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
                    .ok_or_else(|| duckdb_runtime_error("failed to parse SQL while rewriting"))?;
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
                    .ok_or_else(|| duckdb_runtime_error("failed to parse SQL while rewriting"))?;
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
                    .ok_or_else(|| duckdb_runtime_error("failed to parse SQL while rewriting"))?;
                rewritten.push(next);
                index += next.len_utf8();
            }
            RewriteState::LineComment => {
                let next = statement[index..]
                    .chars()
                    .next()
                    .ok_or_else(|| duckdb_runtime_error("failed to parse SQL while rewriting"))?;
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
                    .ok_or_else(|| duckdb_runtime_error("failed to parse SQL while rewriting"))?;
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

fn json_to_duck_value(value: &Value) -> Result<DuckValue> {
    match value {
        Value::Null => Ok(DuckValue::Null),
        Value::Bool(flag) => Ok(DuckValue::Boolean(*flag)),
        Value::Number(number) => {
            if let Some(signed) = number.as_i64() {
                return Ok(DuckValue::BigInt(signed));
            }
            if let Some(unsigned) = number.as_u64() {
                let signed = i64::try_from(unsigned).map_err(|_| {
                    DbtNovaError::InvalidParams(format!(
                        "SQL parameter value {unsigned} exceeds signed 64-bit integer range"
                    ))
                })?;
                return Ok(DuckValue::BigInt(signed));
            }
            if let Some(float) = number.as_f64() {
                return Ok(DuckValue::Double(float));
            }
            Err(DbtNovaError::InvalidParams(format!(
                "Unsupported numeric SQL parameter value: {number}"
            )))
        }
        Value::String(string) => Ok(DuckValue::Text(string.clone())),
        Value::Array(_) | Value::Object(_) => Err(DbtNovaError::InvalidParams(
            "SQL parameters must be scalar values (null, bool, number, or string)".to_string(),
        )),
    }
}

fn build_bind_values(
    order: &[String],
    parameters: &HashMap<String, Value>,
) -> Result<Vec<DuckValue>> {
    order
        .iter()
        .map(|name| {
            let value = parameters.get(name).ok_or_else(|| {
                DbtNovaError::InvalidParams(format!("Missing value for SQL parameter :{name}"))
            })?;
            json_to_duck_value(value)
        })
        .collect()
}

fn infer_column_type_name(value: duckdb::types::ValueRef<'_>) -> String {
    format!("{:?}", value.data_type())
}

fn duck_value_to_json(value: DuckValue) -> Value {
    match value {
        DuckValue::Null => Value::Null,
        DuckValue::Boolean(value) => json!(value),
        DuckValue::TinyInt(value) => json!(value),
        DuckValue::SmallInt(value) => json!(value),
        DuckValue::Int(value) | DuckValue::Date32(value) => json!(value),
        DuckValue::BigInt(value) | DuckValue::Timestamp(_, value) | DuckValue::Time64(_, value) => {
            json!(value)
        }
        DuckValue::HugeInt(value) => Value::String(value.to_string()),
        DuckValue::UTinyInt(value) => json!(value),
        DuckValue::USmallInt(value) => json!(value),
        DuckValue::UInt(value) => json!(value),
        DuckValue::UBigInt(value) => Value::String(value.to_string()),
        DuckValue::Float(value) => json!(value),
        DuckValue::Double(value) => json!(value),
        DuckValue::Decimal(value) => Value::String(value.to_string()),
        DuckValue::Text(value) | DuckValue::Enum(value) => Value::String(value),
        DuckValue::Blob(value) => {
            Value::String(base64::engine::general_purpose::STANDARD.encode(value))
        }
        DuckValue::Interval {
            months,
            days,
            nanos,
        } => json!({
            "months": months,
            "days": days,
            "nanos": nanos,
        }),
        DuckValue::List(values) | DuckValue::Array(values) => {
            Value::Array(values.into_iter().map(duck_value_to_json).collect())
        }
        DuckValue::Struct(values) => Value::Object(
            values
                .iter()
                .map(|(key, value)| (key.clone(), duck_value_to_json(value.clone())))
                .collect(),
        ),
        DuckValue::Map(values) => Value::Object(
            values
                .iter()
                .map(|(key, value)| (format!("{key:?}"), duck_value_to_json(value.clone())))
                .collect(),
        ),
        DuckValue::Union(value) => duck_value_to_json(*value),
    }
}

fn normalized_limit(limit: Option<u64>, fallback: u64) -> u64 {
    limit.unwrap_or(fallback).max(1)
}

#[allow(clippy::too_many_lines)]
fn execute_duckdb_sync_with_connection(
    connection: &Connection,
    config: &DuckDbConfig,
    params: &ExecuteSqlParams,
) -> Result<Value> {
    if params
        .parameter_types
        .as_ref()
        .is_some_and(|types| !types.is_empty())
    {
        return Err(DbtNovaError::InvalidParams(
            "DuckDB provider does not support parameter_types; remove parameter_types and rely on scalar parameter inference".to_string(),
        ));
    }

    let statement_text = params.statement.trim();
    if statement_text.is_empty() {
        return Err(DbtNovaError::InvalidParams(
            "statement cannot be empty".to_string(),
        ));
    }

    let row_limit = normalized_limit(params.row_limit, DEFAULT_ROW_LIMIT);
    let byte_limit = normalized_limit(params.byte_limit, DEFAULT_BYTE_LIMIT);

    let started = Instant::now();
    let parameters = params.parameters.clone().unwrap_or_default();
    let rewritten = rewrite_named_parameters(statement_text, &parameters)?;
    let bind_values = build_bind_values(&rewritten.ordered_parameters, &parameters)?;

    let mut prepared = connection
        .prepare(&rewritten.sql)
        .map_err(|err| duckdb_runtime_error(format!("failed to prepare SQL statement: {err}")))?;

    // Execute once, then read metadata from the executed statement handle to
    // avoid running the same query twice.
    let mut row_iter = prepared
        .query(params_from_iter(bind_values.iter()))
        .map_err(|err| duckdb_runtime_error(format!("failed to execute SQL statement: {err}")))?;
    let columns = row_iter
        .as_ref()
        .map_or_else(Vec::new, duckdb::Statement::column_names);
    let mut column_types = vec!["UNKNOWN".to_string(); columns.len()];

    let mut rows = Vec::<Value>::new();
    let mut total_row_count = 0u64;
    let mut accumulated_bytes = 0u64;
    let mut truncated = false;

    while let Some(row) = row_iter
        .next()
        .map_err(|err| duckdb_runtime_error(format!("failed while reading SQL rows: {err}")))?
    {
        total_row_count = total_row_count.saturating_add(1);

        let mut row_values = Vec::with_capacity(columns.len());
        for (column_index, column_type_name) in column_types.iter_mut().enumerate() {
            let value = row.get_ref(column_index).map_err(|err| {
                duckdb_runtime_error(format!(
                    "failed to read value at column {column_index}: {err}"
                ))
            })?;
            if column_type_name == "UNKNOWN" {
                *column_type_name = infer_column_type_name(value);
            }
            row_values.push(duck_value_to_json(value.to_owned()));
        }

        let row_payload = Value::Array(row_values);
        let row_size_bytes = u64::try_from(
            serde_json::to_vec(&row_payload)
                .map_err(|err| duckdb_runtime_error(format!("failed to serialize row: {err}")))?
                .len(),
        )
        .unwrap_or(u64::MAX);

        let returned_rows = u64::try_from(rows.len()).unwrap_or(u64::MAX);
        let within_row_limit = returned_rows < row_limit;
        let within_byte_limit = accumulated_bytes.saturating_add(row_size_bytes) <= byte_limit;
        if within_row_limit && within_byte_limit {
            rows.push(row_payload);
            accumulated_bytes = accumulated_bytes.saturating_add(row_size_bytes);
            continue;
        }

        truncated = true;
        break;
    }

    let payload = json!({
        "statement_id": uuid::Uuid::new_v4().to_string(),
        "state": "SUCCEEDED",
        "provider": "duckdb",
        "duckdb_path": config.path.display().to_string(),
        "file_search_path": config.file_search_path,
        "columns": columns,
        "column_types": column_types,
        "rows": rows,
        "elapsed_ms": u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        "fetched_chunks": 1,
        "stats": {
            "total_row_count": if truncated { Value::Null } else { json!(total_row_count) },
            "total_byte_count": if truncated { Value::Null } else { json!(accumulated_bytes) },
            "total_chunk_count": 1,
        },
        "truncated": truncated,
    });

    let count = payload
        .get("rows")
        .and_then(Value::as_array)
        .map_or(0usize, Vec::len);

    serde_json::to_value(SuccessResponse::new(payload, count)).map_err(Into::into)
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct NormalizedRelation {
    display: String,
    sql_reference: String,
}

fn normalize_preflight_identifier(segment: &str, context: &str) -> Result<String> {
    let trimmed = segment.trim();
    if trimmed.is_empty()
        || !trimmed
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        || !trimmed
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_alphabetic() || ch == '_')
    {
        return Err(DbtNovaError::InvalidParams(format!(
            "Invalid {context} identifier segment '{segment}'"
        )));
    }
    Ok(trimmed.to_string())
}

fn normalize_preflight_relation(relation: &str) -> Result<NormalizedRelation> {
    let trimmed = relation.trim();
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

    let mut normalized_parts = Vec::with_capacity(parts.len());
    for part in parts {
        normalized_parts.push(normalize_preflight_identifier(part, "relation")?);
    }

    let display = normalized_parts.join(".");
    let sql_reference = normalized_parts
        .iter()
        .map(|part| format!("\"{part}\""))
        .collect::<Vec<_>>()
        .join(".");

    Ok(NormalizedRelation {
        display,
        sql_reference,
    })
}

fn catalog_preflight_statement(catalog: &str) -> String {
    format!(
        "SELECT schema_name FROM information_schema.schemata WHERE catalog_name = {} LIMIT 1",
        sql_string_literal(catalog)
    )
}

fn schema_preflight_statement(catalog: &str, schema: &str) -> String {
    format!(
        "SELECT table_name FROM information_schema.tables WHERE table_catalog = {} AND table_schema = {} LIMIT 1",
        sql_string_literal(catalog),
        sql_string_literal(schema)
    )
}

fn relation_preflight_statement(relation: &NormalizedRelation) -> String {
    format!(
        "SELECT 1 AS relation_access_check FROM {} LIMIT 1",
        relation.sql_reference
    )
}

fn detail_field(key: &str, value: impl AsRef<str>) -> JsonMap<String, Value> {
    let mut details = JsonMap::new();
    details.insert(key.to_string(), Value::String(value.as_ref().to_string()));
    details
}

fn schema_catalog_details(catalog: &str, schema: &str) -> JsonMap<String, Value> {
    let mut details = detail_field("catalog", catalog);
    details.insert("schema".to_string(), Value::String(schema.to_string()));
    details
}

const DUCKDB_EMPTY_PROBE_MESSAGE: &str = "preflight probe returned no rows for requested target";

fn run_preflight_statement(connection: &Connection, statement: &str) -> Result<ProbePresence> {
    let mut prepared = connection.prepare(statement).map_err(|err| {
        duckdb_runtime_error(format!("failed to prepare preflight statement: {err}"))
    })?;
    let mut rows = prepared.query([]).map_err(|err| {
        duckdb_runtime_error(format!("failed to execute preflight statement: {err}"))
    })?;
    let probe = rows
        .next()
        .map_err(|err| duckdb_runtime_error(format!("failed to read preflight rows: {err}")))?;
    if probe.is_some() {
        Ok(ProbePresence::Present)
    } else {
        Ok(ProbePresence::Empty)
    }
}

#[allow(clippy::too_many_lines)]
fn preflight_duckdb_sync_with_connection(
    connection: &Connection,
    config: &DuckDbConfig,
    params: &ExecuteSqlParams,
) -> Result<Value> {
    let mut report = PreflightReport::new();
    report.push_ok("configuration", JsonMap::new());

    run_connectivity_check_sync(
        &mut report,
        "Verify the DuckDB file is readable and not locked by another process",
        || match run_preflight_statement(connection, "SELECT 1 AS connectivity_check")? {
            ProbePresence::Present => Ok(()),
            ProbePresence::Empty => Err(duckdb_runtime_error(DUCKDB_EMPTY_PROBE_MESSAGE)),
        },
    );

    run_optional_object_check_sync(
        &mut report,
        params.preflight_catalog.as_deref(),
        "catalog_access",
        |catalog| normalize_preflight_identifier(catalog, "catalog"),
        |catalog| {
            let statement = catalog_preflight_statement(catalog);
            run_preflight_statement(connection, &statement)
        },
        |catalog| detail_field("catalog", catalog),
        |catalog| detail_field("catalog", catalog),
        "Use an unquoted catalog identifier (letters, digits, underscore)",
        "Verify the catalog exists in information_schema.schemata",
        DUCKDB_EMPTY_PROBE_MESSAGE,
    );

    let catalog_for_schema = params
        .preflight_catalog
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("main")
        .to_string();
    run_optional_object_check_sync(
        &mut report,
        params.preflight_schema.as_deref(),
        "schema_access",
        |schema| {
            let normalized_catalog =
                normalize_preflight_identifier(&catalog_for_schema, "catalog")?;
            let normalized_schema = normalize_preflight_identifier(schema, "schema")?;
            Ok((normalized_catalog, normalized_schema))
        },
        |(catalog, schema)| {
            let statement = schema_preflight_statement(catalog, schema);
            run_preflight_statement(connection, &statement)
        },
        |schema| detail_field("schema", schema),
        |(catalog, schema)| schema_catalog_details(catalog, schema),
        "Use unquoted catalog/schema identifiers (letters, digits, underscore)",
        "Verify the schema exists in information_schema.tables for the selected catalog",
        DUCKDB_EMPTY_PROBE_MESSAGE,
    );

    run_optional_object_check_sync(
        &mut report,
        params.preflight_relation.as_deref(),
        "relation_access",
        normalize_preflight_relation,
        |relation| {
            let statement = relation_preflight_statement(relation);
            run_preflight_statement(connection, &statement)
        },
        |relation| detail_field("relation", relation),
        |relation| detail_field("relation", &relation.display),
        "Use table, schema.table, or catalog.schema.table with unquoted identifiers",
        "Verify the relation exists and is readable with the configured DuckDB file and file_search_path",
        DUCKDB_EMPTY_PROBE_MESSAGE,
    );

    let mut metadata = JsonMap::new();
    metadata.insert(
        "duckdb_path".to_string(),
        Value::String(config.path.display().to_string()),
    );
    metadata.insert(
        "file_search_path".to_string(),
        config
            .file_search_path
            .clone()
            .map_or(Value::Null, Value::String),
    );
    build_preflight_response("duckdb", metadata, report)
}

fn missing_configuration_preflight_payload(message: &str) -> Result<Value> {
    let mut metadata = JsonMap::new();
    metadata.insert("duckdb_path".to_string(), Value::Null);
    metadata.insert("file_search_path".to_string(), Value::Null);
    build_configuration_failure_response(
        "duckdb",
        metadata,
        message.to_string(),
        "Set DBT_NOVA_DUCKDB_PATH to a readable DuckDB file; optionally set DBT_NOVA_DUCKDB_FILE_SEARCH_PATH for external file-backed objects",
    )
}

fn configuration_failure_preflight_payload(config: &DuckDbConfig, message: &str) -> Result<Value> {
    let mut metadata = JsonMap::new();
    metadata.insert(
        "duckdb_path".to_string(),
        Value::String(config.path.display().to_string()),
    );
    metadata.insert(
        "file_search_path".to_string(),
        config
            .file_search_path
            .clone()
            .map_or(Value::Null, Value::String),
    );
    build_configuration_failure_response(
        "duckdb",
        metadata,
        message.to_string(),
        "Set DBT_NOVA_DUCKDB_PATH to a readable DuckDB file and verify DBT_NOVA_DUCKDB_FILE_SEARCH_PATH if configured",
    )
}

pub struct DuckDbProvider;

pub static DUCKDB_PROVIDER: DuckDbProvider = DuckDbProvider;

async fn execute_duckdb_async(params: &ExecuteSqlParams) -> Result<Value> {
    let config = DuckDbConfig::from_env()?;
    let pool = cached_duckdb_pool(&config).await;
    let permit = pool.acquire_slot().await?;
    let params = params.clone();

    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let connection = pool.checkout_connection()?;
        let result = execute_duckdb_sync_with_connection(&connection, &pool.config, &params);
        pool.return_connection(connection);
        result
    })
    .await
    .map_err(|err| duckdb_runtime_error(format!("join failure while executing SQL: {err}")))?
}

async fn preflight_duckdb_async(params: &ExecuteSqlParams) -> Result<Value> {
    let config = match DuckDbConfig::from_env() {
        Ok(config) => config,
        Err(err) => return missing_configuration_preflight_payload(&err.to_string()),
    };
    let pool = cached_duckdb_pool(&config).await;
    let permit = pool.acquire_slot().await?;
    let params = params.clone();

    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let connection = match pool.checkout_connection() {
            Ok(connection) => connection,
            Err(err) => {
                let message = err.to_string();
                return configuration_failure_preflight_payload(&pool.config, &message);
            }
        };
        let result = preflight_duckdb_sync_with_connection(&connection, &pool.config, &params);
        pool.return_connection(connection);
        result
    })
    .await
    .map_err(|err| duckdb_runtime_error(format!("join failure while running preflight: {err}")))?
}

impl SqlProvider for DuckDbProvider {
    fn name(&self) -> &'static str {
        "duckdb"
    }

    fn execute<'a>(
        &'a self,
        params: &'a ExecuteSqlParams,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value>> + Send + 'a>> {
        Box::pin(execute_duckdb_async(params))
    }

    fn preflight<'a>(
        &'a self,
        params: &'a ExecuteSqlParams,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value>> + Send + 'a>> {
        Box::pin(preflight_duckdb_async(params))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DuckDbConfig, build_bind_values, catalog_preflight_statement,
        configuration_failure_preflight_payload, missing_configuration_preflight_payload,
        normalize_preflight_relation, relation_preflight_statement, resolve_duckdb_pool_max_size,
        rewrite_named_parameters, schema_preflight_statement,
    };
    use serde_json::json;
    use std::collections::HashMap;
    use std::path::PathBuf;

    #[test]
    fn rewrite_named_parameters_replaces_in_order() {
        let mut parameters = HashMap::new();
        parameters.insert("country".to_string(), json!("GB"));
        parameters.insert("week_start".to_string(), json!("2026-02-01"));

        let rewritten = rewrite_named_parameters(
            "select * from report where country = :country and week_start >= :week_start and country = :country",
            &parameters,
        )
        .expect("rewrite should succeed");

        assert_eq!(
            rewritten.sql,
            "select * from report where country = ? and week_start >= ? and country = ?"
        );
        assert_eq!(
            rewritten.ordered_parameters,
            vec![
                "country".to_string(),
                "week_start".to_string(),
                "country".to_string()
            ]
        );
    }

    #[test]
    fn rewrite_named_parameters_skips_literals_comments_and_casts() {
        let mut parameters = HashMap::new();
        parameters.insert("country".to_string(), json!("GB"));

        let sql = "select ':country' as literal, value::double as casted -- :country\nfrom report /* :country */ where country = :country";
        let rewritten = rewrite_named_parameters(sql, &parameters)
            .expect("rewrite should preserve non-code contexts");

        assert_eq!(
            rewritten.sql,
            "select ':country' as literal, value::double as casted -- :country\nfrom report /* :country */ where country = ?"
        );
        assert_eq!(rewritten.ordered_parameters, vec!["country".to_string()]);
    }

    #[test]
    fn rewrite_named_parameters_requires_values() {
        let parameters = HashMap::new();
        let err =
            rewrite_named_parameters("select * from report where country = :country", &parameters)
                .expect_err("missing parameters should fail");
        assert!(
            err.to_string()
                .contains("Missing value for SQL parameter :country")
        );
    }

    #[test]
    fn build_bind_values_rejects_complex_values() {
        let mut parameters = HashMap::new();
        parameters.insert("invalid".to_string(), json!({"nested": true}));

        let err = build_bind_values(&["invalid".to_string()], &parameters)
            .expect_err("object value should fail");
        assert!(
            err.to_string()
                .contains("SQL parameters must be scalar values")
        );
    }

    #[test]
    fn normalize_preflight_relation_accepts_three_part_name() {
        let relation = normalize_preflight_relation("main.analytics.orders")
            .expect("three part relation should normalize");
        assert_eq!(relation.display, "main.analytics.orders");
        assert_eq!(relation.sql_reference, "\"main\".\"analytics\".\"orders\"");
    }

    #[test]
    fn normalize_preflight_relation_rejects_invalid_identifier() {
        let err = normalize_preflight_relation("analytics.orders;drop")
            .expect_err("invalid relation should fail");
        assert!(
            err.to_string()
                .contains("Invalid relation identifier segment")
        );
    }

    #[test]
    fn relation_preflight_statement_uses_quoted_identifiers() {
        let relation = normalize_preflight_relation("analytics.orders")
            .expect("valid relation should normalize");
        let statement = relation_preflight_statement(&relation);
        assert_eq!(
            statement,
            "SELECT 1 AS relation_access_check FROM \"analytics\".\"orders\" LIMIT 1"
        );
    }

    #[test]
    fn schema_preflight_statement_uses_information_schema() {
        let statement = schema_preflight_statement("main", "analytics");
        assert_eq!(
            statement,
            "SELECT table_name FROM information_schema.tables WHERE table_catalog = 'main' AND table_schema = 'analytics' LIMIT 1"
        );
    }

    #[test]
    fn catalog_preflight_statement_uses_information_schema() {
        let statement = catalog_preflight_statement("main");
        assert_eq!(
            statement,
            "SELECT schema_name FROM information_schema.schemata WHERE catalog_name = 'main' LIMIT 1"
        );
    }

    #[test]
    fn missing_configuration_preflight_payload_is_structured() {
        let payload = missing_configuration_preflight_payload("missing path")
            .expect("payload should serialize");
        assert_eq!(payload["success"], json!(true));
        assert_eq!(payload["data"]["provider"], json!("duckdb"));
        assert_eq!(payload["data"]["ready"], json!(false));
        assert_eq!(payload["data"]["checks"][0]["name"], json!("configuration"));
        assert_eq!(payload["data"]["checks"][0]["ok"], json!(false));
        assert_eq!(
            payload["data"]["checks"][0]["message"],
            json!("missing path")
        );
    }

    #[test]
    fn configuration_failure_preflight_payload_is_structured() {
        let config = DuckDbConfig {
            path: PathBuf::from("/tmp/example.duckdb"),
            file_search_path: Some("/tmp/data".to_string()),
        };
        let payload = configuration_failure_preflight_payload(&config, "open failed")
            .expect("payload should serialize");
        assert_eq!(payload["success"], json!(true));
        assert_eq!(payload["data"]["provider"], json!("duckdb"));
        assert_eq!(payload["data"]["ready"], json!(false));
        assert_eq!(payload["data"]["checks"][0]["name"], json!("configuration"));
        assert_eq!(payload["data"]["checks"][0]["ok"], json!(false));
        assert_eq!(
            payload["data"]["checks"][0]["message"],
            json!("open failed")
        );
    }

    #[test]
    fn resolve_duckdb_pool_max_size_prefers_explicit_then_fallback_then_default() {
        assert_eq!(resolve_duckdb_pool_max_size(Some(4), Some(9)), 4);
        assert_eq!(resolve_duckdb_pool_max_size(None, Some(7)), 7);
        assert_eq!(resolve_duckdb_pool_max_size(None, None), 10);
    }

    #[test]
    fn resolve_duckdb_pool_max_size_clamps_zero_to_one() {
        assert_eq!(resolve_duckdb_pool_max_size(Some(0), None), 1);
        assert_eq!(resolve_duckdb_pool_max_size(None, Some(0)), 1);
    }
}
