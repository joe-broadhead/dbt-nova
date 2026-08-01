#![forbid(unsafe_code)]

mod auth;

use std::collections::HashMap;
use std::env;
use std::process::Command;
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use reqwest::{Client, StatusCode, Url};
use serde::{Deserialize, Serialize};
use serde_json::{Map as JsonMap, Value};
use sha2::{Digest, Sha256};
use simple_asn1::{ASN1Block, BigInt, oid};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Mutex as TokioMutex;
use tokio::time::{sleep, timeout};
use tracing::warn;

use auth::{
    ExternalBrowserAuthenticatorInfo, ExternalBrowserAuthenticatorRequest,
    ExternalBrowserAuthenticatorRequestData, ExternalBrowserLoginRequest,
    ExternalBrowserLoginRequestData, SnowflakeAuthorization, bind_browser_callback_listener,
    decode_external_browser_authenticator_response, decode_external_browser_login_response,
    generate_keypair_jwt, normalize_account_url, open_external_browser_url,
    receive_browser_callback, resolve_auth_from_env, validate_external_browser_runtime,
};

use crate::config::DbtNovaConfig;
use crate::error::{DbtNovaError, Result};
use crate::params::ExecuteSqlParams;
use crate::responses::SuccessResponse;
use crate::utils::http_client::async_client_builder;
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
const DEFAULT_EXTERNAL_BROWSER_TIMEOUT_SECONDS: u64 = 120;
const MAX_SAFE_JSON_INTEGER: i64 = 9_007_199_254_740_991;
const MIN_SAFE_JSON_INTEGER: i64 = -MAX_SAFE_JSON_INTEGER;
const PREFLIGHT_SHOW_LIMIT: u16 = 1;
const SESSION_EXPIRY_SAFETY_WINDOW_SECONDS: u64 = 60;
const MAX_BROWSER_CALLBACK_REQUEST_BYTES: usize = 1024 * 1024;
const STATEMENT_STILL_EXECUTING_CODE: &str = "333333";
const STATEMENT_ASYNC_EXECUTION_CODE: &str = "333334";
const SUPPORTED_SNOWFLAKE_AUTH_MODES: &str = "keypair, oauth, pat, wif, or externalbrowser";
const EXTERNAL_BROWSER_AUTHENTICATOR: &str = "EXTERNALBROWSER";

type ExternalBrowserSessionCache = Arc<TokioMutex<Option<SnowflakeSession>>>;
type ExternalBrowserSessionCacheMap = StdMutex<HashMap<String, ExternalBrowserSessionCache>>;

static EXTERNAL_BROWSER_SESSION_CACHES: OnceLock<ExternalBrowserSessionCacheMap> = OnceLock::new();

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
    WorkloadIdentityFederation {
        provider: String,
        token_source: SnowflakeWifTokenSource,
    },
    ExternalBrowser {
        user: String,
        account_identifier: String,
        timeout: Duration,
        open_browser: bool,
        callback_port: Option<u16>,
        session_cache: Arc<TokioMutex<Option<SnowflakeSession>>>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum SnowflakeWifTokenSource {
    Inline(String),
    FilePath(String),
}

impl SnowflakeWifTokenSource {
    fn token(&self) -> Result<String> {
        let token = match self {
            Self::Inline(token) => token.clone(),
            Self::FilePath(path) => std::fs::read_to_string(path).map_err(|err| {
                DbtNovaError::InvalidParams(format!(
                    "Failed to read DBT_NOVA_SNOWFLAKE_WIF_TOKEN_PATH '{path}': {err}"
                ))
            })?,
        };
        let token = token.trim();
        if token.is_empty() {
            return Err(DbtNovaError::InvalidParams(
                "Snowflake workload identity token must not be empty".to_string(),
            ));
        }
        Ok(token.to_string())
    }
}

#[derive(Clone, Debug)]
struct SnowflakeSession {
    token: String,
    expires_at: Option<Instant>,
    master_token: Option<String>,
    master_expires_at: Option<Instant>,
    id_token: Option<String>,
    id_token_expires_at: Option<Instant>,
}

impl SnowflakeSession {
    fn is_valid(&self) -> bool {
        let primary_valid = match self.expires_at {
            Some(expires_at) => Instant::now()
                .checked_add(session_expiry_safety_window())
                .is_some_and(|minimum_valid_until| expires_at > minimum_valid_until),
            None => true,
        };
        primary_valid
            && optional_cached_token_is_valid(self.master_token.as_ref(), self.master_expires_at)
            && optional_cached_token_is_valid(self.id_token.as_ref(), self.id_token_expires_at)
    }
}

fn optional_cached_token_is_valid(token: Option<&String>, expires_at: Option<Instant>) -> bool {
    token.is_none_or(|_| {
        expires_at.is_none_or(|expires_at| {
            Instant::now()
                .checked_add(session_expiry_safety_window())
                .is_some_and(|minimum_valid_until| expires_at > minimum_valid_until)
        })
    })
}

fn session_expiry_safety_window() -> Duration {
    Duration::from_secs(SESSION_EXPIRY_SAFETY_WINDOW_SECONDS)
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

        let auth = resolve_auth_from_env(account_identifier, &base_url)?;

        let timeout = Duration::from_millis(
            env_u64("DBT_NOVA_SNOWFLAKE_TIMEOUT_MS", DEFAULT_TIMEOUT_MS).max(1_000),
        );
        let default_statement_timeout_s = env_u64(
            "DBT_NOVA_SNOWFLAKE_STATEMENT_TIMEOUT_S",
            DEFAULT_STATEMENT_TIMEOUT_S,
        );
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
        let http = async_client_builder()?
            .timeout(cfg.timeout)
            .user_agent(format!("dbt-nova/{}", env!("CARGO_PKG_VERSION")))
            .gzip(true)
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
        self.send_authorized_json(|| {
            self.http
                .post(self.statements_url())
                .query(&[("async", "true"), ("requestId", request_id)])
                .json(request)
        })
        .await
    }

    async fn get_statement(&self, statement_handle: &str) -> Result<StatementResponse> {
        self.send_authorized_statement_status(|| {
            self.http.get(self.statement_url(statement_handle))
        })
        .await
    }

    async fn get_partition(
        &self,
        statement_handle: &str,
        partition: usize,
    ) -> Result<StatementResponse> {
        self.send_authorized_json(|| {
            self.http
                .get(self.statement_url(statement_handle))
                .query(&[("partition", partition.to_string())])
        })
        .await
    }

    async fn cancel_statement(&self, statement_handle: &str) -> Result<()> {
        let _: Value = self
            .send_authorized_json(|| self.http.post(self.cancel_url(statement_handle)))
            .await?;
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
                return Err(self.poll_timeout_error(statement_handle).await);
            }

            let remaining = max_poll
                .checked_sub(started.elapsed())
                .unwrap_or_else(|| Duration::from_secs(0));
            sleep(poll_interval.min(remaining)).await;
            if started.elapsed() >= max_poll {
                return Err(self.poll_timeout_error(statement_handle).await);
            }

            let response = self.get_statement(statement_handle).await?;
            if let Some(message) = response.failure_message() {
                return Err(snowflake_err(message));
            }
            if !response.is_pending() {
                return Ok(response);
            }
        }
    }

    async fn poll_timeout_error(&self, statement_handle: &str) -> DbtNovaError {
        if let Err(err) = self.cancel_statement(statement_handle).await {
            warn!(
                statement_handle,
                error = %err,
                "failed to cancel Snowflake statement after local poll timeout"
            );
        }
        snowflake_err(format!(
            "Timed out waiting for Snowflake statement {statement_handle}"
        ))
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

    async fn send_authorized_json<T, F>(&self, make_builder: F) -> Result<T>
    where
        T: for<'de> Deserialize<'de>,
        F: Fn() -> reqwest::RequestBuilder,
    {
        let (mut status, mut body) = self.send_authorized_text(make_builder()).await?;
        if self.should_retry_external_browser_auth(status, &body) {
            self.clear_external_browser_session().await;
            (status, body) = self.send_authorized_text(make_builder()).await?;
        }
        decode_json_response(status, &body)
    }

    async fn send_authorized_statement_status<F>(
        &self,
        make_builder: F,
    ) -> Result<StatementResponse>
    where
        F: Fn() -> reqwest::RequestBuilder,
    {
        let (mut status, mut body) = self.send_authorized_text(make_builder()).await?;
        if self.should_retry_external_browser_auth(status, &body) {
            self.clear_external_browser_session().await;
            (status, body) = self.send_authorized_text(make_builder()).await?;
        }
        decode_statement_status_response(status, &body)
    }

    async fn send_authorized_text(
        &self,
        builder: reqwest::RequestBuilder,
    ) -> Result<(StatusCode, String)> {
        send_text(self.authorized(builder).await?).await
    }

    async fn authorized(
        &self,
        builder: reqwest::RequestBuilder,
    ) -> Result<reqwest::RequestBuilder> {
        let auth = self.authorization().await?;
        let builder = builder
            .header("Accept", "application/json")
            .header("Content-Type", "application/json");
        Ok(match auth {
            SnowflakeAuthorization::Bearer { token, token_type } => builder
                .bearer_auth(token)
                .header("X-Snowflake-Authorization-Token-Type", token_type),
            SnowflakeAuthorization::Session { token } => {
                builder.header("Authorization", format!("Snowflake Token=\"{token}\""))
            }
        })
    }

    async fn authorization(&self) -> Result<SnowflakeAuthorization> {
        match &self.cfg.auth {
            SnowflakeAuthConfig::OAuth { token } => Ok(SnowflakeAuthorization::Bearer {
                token: token.clone(),
                token_type: "OAUTH",
            }),
            SnowflakeAuthConfig::ProgrammaticAccessToken { token } => {
                Ok(SnowflakeAuthorization::Bearer {
                    token: token.clone(),
                    token_type: "PROGRAMMATIC_ACCESS_TOKEN",
                })
            }
            SnowflakeAuthConfig::WorkloadIdentityFederation {
                provider,
                token_source,
            } => Ok(SnowflakeAuthorization::Bearer {
                token: format!("WIF.{}.{}", provider, token_source.token()?),
                token_type: "WORKLOAD_IDENTITY_FEDERATION",
            }),
            SnowflakeAuthConfig::KeyPair {
                user,
                account_identifier,
                private_key_pem,
            } => Ok(SnowflakeAuthorization::Bearer {
                token: generate_keypair_jwt(account_identifier, user, private_key_pem)?,
                token_type: "KEYPAIR_JWT",
            }),
            SnowflakeAuthConfig::ExternalBrowser {
                user,
                account_identifier,
                timeout,
                open_browser,
                callback_port,
                session_cache,
            } => {
                let mut cached = session_cache.lock().await;
                if let Some(session) = cached.as_ref()
                    && session.is_valid()
                {
                    return Ok(SnowflakeAuthorization::Session {
                        token: session.token.clone(),
                    });
                }

                let session = self
                    .login_external_browser(
                        account_identifier,
                        user,
                        *timeout,
                        *open_browser,
                        *callback_port,
                    )
                    .await?;
                let token = session.token.clone();
                *cached = Some(session);
                Ok(SnowflakeAuthorization::Session { token })
            }
        }
    }

    async fn clear_external_browser_session(&self) {
        if let SnowflakeAuthConfig::ExternalBrowser { session_cache, .. } = &self.cfg.auth {
            *session_cache.lock().await = None;
        }
    }

    fn should_retry_external_browser_auth(&self, status: StatusCode, body: &str) -> bool {
        if !matches!(self.cfg.auth, SnowflakeAuthConfig::ExternalBrowser { .. }) {
            return false;
        }
        if status == StatusCode::UNAUTHORIZED {
            return true;
        }
        serde_json::from_str::<ErrorResponseBody>(body)
            .ok()
            .and_then(|parsed| parsed.code)
            .is_some_and(|code| matches!(code.as_str(), "390303" | "390111" | "390112"))
    }

    async fn login_external_browser(
        &self,
        account_identifier: &str,
        user: &str,
        auth_timeout: Duration,
        open_browser: bool,
        callback_port: Option<u16>,
    ) -> Result<SnowflakeSession> {
        let listener = bind_browser_callback_listener(callback_port).await?;
        let port = listener
            .local_addr()
            .map_err(|err| snowflake_err(format!("failed to inspect callback listener: {err}")))?
            .port();
        let request = self
            .request_external_browser_authenticator(account_identifier, user, port)
            .await?;

        open_external_browser_url(&request.sso_url, open_browser);
        let callback = receive_browser_callback(listener, &request.proof_key, auth_timeout).await?;
        let session = self
            .exchange_external_browser_token(
                account_identifier,
                user,
                &callback.token,
                callback.proof_key.as_deref(),
            )
            .await?;
        Ok(session)
    }

    async fn request_external_browser_authenticator(
        &self,
        account_identifier: &str,
        user: &str,
        callback_port: u16,
    ) -> Result<ExternalBrowserAuthenticatorInfo> {
        let request = ExternalBrowserAuthenticatorRequest {
            data: ExternalBrowserAuthenticatorRequestData {
                client_app_id: "dbt-nova",
                client_app_version: env!("CARGO_PKG_VERSION"),
                account_name: account_identifier,
                login_name: user,
                authenticator: EXTERNAL_BROWSER_AUTHENTICATOR,
                browser_mode_redirect_port: callback_port.to_string(),
            },
        };
        let (status, body) = send_text(
            self.http
                .post(self.authenticator_request_url())
                .json(&request),
        )
        .await?;
        decode_external_browser_authenticator_response(status, &body)
    }

    async fn exchange_external_browser_token(
        &self,
        account_identifier: &str,
        user: &str,
        callback_token: &str,
        proof_key: Option<&str>,
    ) -> Result<SnowflakeSession> {
        let request = ExternalBrowserLoginRequest {
            data: ExternalBrowserLoginRequestData {
                client_app_id: "dbt-nova",
                client_app_version: env!("CARGO_PKG_VERSION"),
                account_name: account_identifier,
                login_name: user,
                authenticator: EXTERNAL_BROWSER_AUTHENTICATOR,
                token: callback_token,
                proof_key,
            },
        };
        let (status, body) =
            send_text(self.http.post(self.login_request_url()).json(&request)).await?;
        decode_external_browser_login_response(status, &body)
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

    fn authenticator_request_url(&self) -> String {
        format!("{}/session/authenticator-request", self.cfg.base_url)
    }

    fn login_request_url(&self) -> String {
        format!("{}/session/v1/login-request", self.cfg.base_url)
    }
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
            max_chunks: None,
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
                .unwrap_or(config.default_statement_timeout_s),
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
        if self.code.as_deref().is_some_and(is_pending_statement_code) {
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

fn is_pending_statement_code(code: &str) -> bool {
    matches!(
        code,
        STATEMENT_STILL_EXECUTING_CODE | STATEMENT_ASYNC_EXECUTION_CODE
    )
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

#[cfg(test)]
async fn send_json<T: for<'de> Deserialize<'de>>(builder: reqwest::RequestBuilder) -> Result<T> {
    let (status, body) = send_text(builder).await?;
    decode_json_response(status, &body)
}

async fn send_text(builder: reqwest::RequestBuilder) -> Result<(StatusCode, String)> {
    let response = builder
        .send()
        .await
        .map_err(|err| snowflake_err(format!("HTTP request failed: {err}")))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|err| snowflake_err(format!("failed to read response body: {err}")))?;

    Ok((status, body))
}

fn decode_json_response<T: for<'de> Deserialize<'de>>(status: StatusCode, body: &str) -> Result<T> {
    if !status.is_success() {
        return Err(snowflake_http(status, body));
    }

    serde_json::from_str(body).map_err(|err| {
        snowflake_err(format!(
            "failed to parse JSON response: {err}; response_body_bytes={}",
            body.len()
        ))
    })
}

fn decode_statement_status_response(status: StatusCode, body: &str) -> Result<StatementResponse> {
    if status == StatusCode::TOO_MANY_REQUESTS {
        let response = serde_json::from_str::<StatementResponse>(body)
            .map_err(|_| snowflake_http(status, body))?;
        if response.is_pending() {
            return Ok(response);
        }
        return Err(snowflake_http(status, body));
    }

    decode_json_response(status, body)
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
        "FIXED" | "NUMBER" | "DECIMAL" | "NUMERIC" => parse_fixed_numeric_cell(text, field),
        "REAL" | "FLOAT" | "FLOAT4" | "FLOAT8" | "DOUBLE" | "DOUBLE PRECISION" => {
            parse_floating_numeric_cell(text)
        }
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

fn parse_fixed_numeric_cell(text: &str, field: &ResultColumn) -> Value {
    if parse_optional_u64(field.scale.as_ref()) == Some(0)
        && let Ok(integer) = text.parse::<i64>()
        && (MIN_SAFE_JSON_INTEGER..=MAX_SAFE_JSON_INTEGER).contains(&integer)
    {
        return Value::from(integer);
    }
    Value::String(text.to_string())
}

fn parse_floating_numeric_cell(text: &str) -> Value {
    match text.parse::<f64>() {
        Ok(number) if number.is_finite() => Value::from(number),
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
    DollarQuotedString,
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
                if bytes[index] == b'$' && index + 1 < bytes.len() && bytes[index + 1] == b'$' {
                    rewritten.push_str("$$");
                    index += 2;
                    state = RewriteState::DollarQuotedString;
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
                if bytes[index] == b'\\' {
                    rewritten.push('\\');
                    index += 1;
                    if index < bytes.len() {
                        let escaped = statement[index..]
                            .chars()
                            .next()
                            .ok_or_else(|| snowflake_err("failed to parse SQL while rewriting"))?;
                        rewritten.push(escaped);
                        index += escaped.len_utf8();
                    }
                    continue;
                }
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
            RewriteState::DollarQuotedString => {
                if bytes[index] == b'$' && index + 1 < bytes.len() && bytes[index + 1] == b'$' {
                    rewritten.push_str("$$");
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
    format!(
        "SHOW DATABASES STARTS WITH {} LIMIT {PREFLIGHT_SHOW_LIMIT}",
        sql_string_literal(catalog)
    )
}

fn schema_preflight_statement(catalog: &str, schema: &str) -> String {
    format!(
        "SHOW SCHEMAS IN DATABASE {catalog} STARTS WITH {} LIMIT {PREFLIGHT_SHOW_LIMIT}",
        sql_string_literal(schema)
    )
}

fn resolve_schema_preflight_target(
    preflight_catalog: Option<&str>,
    default_catalog: Option<&str>,
    schema: &str,
) -> Result<(String, String)> {
    let catalog = match preflight_catalog {
        Some(catalog) => normalize_preflight_identifier(catalog, "catalog")?,
        None => default_catalog
            .map(|catalog| normalize_preflight_identifier(catalog, "catalog"))
            .transpose()?
            .ok_or_else(|| {
                DbtNovaError::InvalidParams(
                    "preflight_schema requires DBT_NOVA_SNOWFLAKE_DATABASE or preflight_catalog"
                        .to_string(),
                )
            })?,
    };
    let schema = normalize_preflight_identifier(schema, "schema")?;
    Ok((catalog, schema))
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

fn preflight_show_result_has_exact_name(result: &SnowflakeQueryResult, expected: &str) -> bool {
    let Some(name_index) = result
        .columns
        .iter()
        .position(|column| column.eq_ignore_ascii_case("name"))
    else {
        return false;
    };

    result.rows.iter().any(|row| {
        row.get(name_index)
            .and_then(Value::as_str)
            .is_some_and(|name| name.eq_ignore_ascii_case(expected))
    })
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
                Ok(if preflight_show_result_has_exact_name(&result, &catalog) {
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
            resolve_schema_preflight_target(
                params.preflight_catalog.as_deref(),
                default_catalog.as_deref(),
                schema,
            )
        },
        |(catalog, schema)| {
            let catalog = catalog.clone();
            let schema = schema.clone();
            let warehouse = check_warehouse.clone();
            let client = client_for_schema.clone();
            async move {
                let statement = schema_preflight_statement(&catalog, &schema);
                let result = run_preflight_statement(&client, &statement, warehouse).await?;
                Ok(if preflight_show_result_has_exact_name(&result, &schema) {
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

    fn validate_runtime(&self, config: &DbtNovaConfig) -> Result<()> {
        validate_external_browser_runtime(config)
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

fn env_bool(name: &str, default_value: bool) -> Result<bool> {
    let Some(value) = read_optional_env(name) else {
        return Ok(default_value);
    };
    match value.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => Err(DbtNovaError::InvalidParams(format!(
            "{name} must be true or false"
        ))),
    }
}

fn env_u16_optional(name: &str) -> Result<Option<u16>> {
    let Some(value) = read_optional_env(name) else {
        return Ok(None);
    };
    value.parse::<u16>().map(Some).map_err(|err| {
        DbtNovaError::InvalidParams(format!(
            "{name} must be a TCP port between 0 and 65535: {err}"
        ))
    })
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

#[cfg(test)]
mod tests;
