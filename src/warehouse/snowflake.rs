#![forbid(unsafe_code)]

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

use crate::config::DbtNovaConfig;
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
const DEFAULT_EXTERNAL_BROWSER_TIMEOUT_SECONDS: u64 = 120;
const MAX_SAFE_JSON_INTEGER: i64 = 9_007_199_254_740_991;
const MIN_SAFE_JSON_INTEGER: i64 = -MAX_SAFE_JSON_INTEGER;
const PREFLIGHT_SHOW_LIMIT: u16 = 1;
const SESSION_EXPIRY_SAFETY_WINDOW_SECONDS: u64 = 60;
const MAX_BROWSER_CALLBACK_REQUEST_BYTES: usize = 8192;
const STATEMENT_STILL_EXECUTING_CODE: &str = "333333";
const STATEMENT_ASYNC_EXECUTION_CODE: &str = "333334";
const SUPPORTED_SNOWFLAKE_AUTH_MODES: &str = "keypair, oauth, pat, or externalbrowser";
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
    ExternalBrowser {
        user: String,
        account_identifier: String,
        timeout: Duration,
        open_browser: bool,
        callback_port: Option<u16>,
        session_cache: Arc<TokioMutex<Option<SnowflakeSession>>>,
    },
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
        let http = Client::builder()
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

enum SnowflakeAuthorization {
    Bearer {
        token: String,
        token_type: &'static str,
    },
    Session {
        token: String,
    },
}

#[derive(Serialize)]
struct ExternalBrowserAuthenticatorRequest<'a> {
    data: ExternalBrowserAuthenticatorRequestData<'a>,
}

#[derive(Serialize)]
struct ExternalBrowserAuthenticatorRequestData<'a> {
    #[serde(rename = "CLIENT_APP_ID")]
    client_app_id: &'static str,
    #[serde(rename = "CLIENT_APP_VERSION")]
    client_app_version: &'static str,
    #[serde(rename = "ACCOUNT_NAME")]
    account_name: &'a str,
    #[serde(rename = "LOGIN_NAME")]
    login_name: &'a str,
    #[serde(rename = "AUTHENTICATOR")]
    authenticator: &'static str,
    #[serde(rename = "BROWSER_MODE_REDIRECT_PORT")]
    browser_mode_redirect_port: String,
}

#[derive(Debug, Deserialize)]
struct ExternalBrowserAuthenticatorResponse {
    #[serde(default)]
    success: Option<bool>,
    #[serde(default)]
    data: Option<ExternalBrowserAuthenticatorResponseData>,
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    message: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ExternalBrowserAuthenticatorResponseData {
    #[serde(rename = "ssoUrl", alias = "SSO_URL", default)]
    sso_url: Option<String>,
    #[serde(rename = "proofKey", alias = "PROOF_KEY", default)]
    proof_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExternalBrowserAuthenticatorInfo {
    sso_url: String,
    proof_key: String,
}

#[derive(Serialize)]
struct ExternalBrowserLoginRequest<'a> {
    data: ExternalBrowserLoginRequestData<'a>,
}

#[derive(Serialize)]
struct ExternalBrowserLoginRequestData<'a> {
    #[serde(rename = "CLIENT_APP_ID")]
    client_app_id: &'static str,
    #[serde(rename = "CLIENT_APP_VERSION")]
    client_app_version: &'static str,
    #[serde(rename = "ACCOUNT_NAME")]
    account_name: &'a str,
    #[serde(rename = "LOGIN_NAME")]
    login_name: &'a str,
    #[serde(rename = "AUTHENTICATOR")]
    authenticator: &'static str,
    #[serde(rename = "TOKEN")]
    token: &'a str,
    #[serde(rename = "PROOF_KEY", skip_serializing_if = "Option::is_none")]
    proof_key: Option<&'a str>,
}

#[derive(Debug, Deserialize)]
struct ExternalBrowserLoginResponse {
    #[serde(default)]
    success: Option<bool>,
    #[serde(default)]
    data: Option<ExternalBrowserLoginResponseData>,
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    message: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ExternalBrowserLoginResponseData {
    #[serde(rename = "token", alias = "sessionToken", default)]
    token: Option<String>,
    #[serde(rename = "validityInSeconds", alias = "validity_in_seconds", default)]
    validity_in_seconds: Option<Value>,
    #[serde(rename = "masterToken", alias = "master_token", default)]
    master_token: Option<String>,
    #[serde(
        rename = "masterValidityInSeconds",
        alias = "master_validity_in_seconds",
        default
    )]
    master_validity_in_seconds: Option<Value>,
    #[serde(rename = "idToken", alias = "id_token", default)]
    id_token: Option<String>,
    #[serde(
        rename = "idTokenValidityInSeconds",
        alias = "id_token_validity_in_seconds",
        default
    )]
    id_token_validity_in_seconds: Option<Value>,
}

#[derive(Debug, PartialEq, Eq)]
struct BrowserCallback {
    token: String,
    proof_key: Option<String>,
    origin: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
enum BrowserCallbackRequest {
    Callback(BrowserCallback),
    Preflight(BrowserCallbackPreflight),
}

#[derive(Debug, PartialEq, Eq)]
struct BrowserCallbackPreflight {
    origin: Option<String>,
    requested_headers: Option<String>,
}

fn decode_external_browser_authenticator_response(
    status: StatusCode,
    body: &str,
) -> Result<ExternalBrowserAuthenticatorInfo> {
    let response = decode_json_response::<ExternalBrowserAuthenticatorResponse>(status, body)?;
    if response.success == Some(false) {
        return Err(snowflake_err(format!(
            "external browser authenticator request failed: {}",
            summarize_snowflake_auth_response(
                response.code.as_deref(),
                response.message.as_deref()
            )
        )));
    }
    let data = response
        .data
        .ok_or_else(|| snowflake_err("external browser authenticator response missing data"))?;
    let sso_url = data
        .sso_url
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| snowflake_err("external browser authenticator response missing ssoUrl"))?;
    let proof_key = data
        .proof_key
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| snowflake_err("external browser authenticator response missing proofKey"))?;
    Ok(ExternalBrowserAuthenticatorInfo { sso_url, proof_key })
}

fn decode_external_browser_login_response(
    status: StatusCode,
    body: &str,
) -> Result<SnowflakeSession> {
    let response = decode_json_response::<ExternalBrowserLoginResponse>(status, body)?;
    if response.success == Some(false) {
        return Err(snowflake_err(format!(
            "external browser login failed: {}",
            summarize_snowflake_auth_response(
                response.code.as_deref(),
                response.message.as_deref()
            )
        )));
    }
    let data = response
        .data
        .ok_or_else(|| snowflake_err("external browser login response missing data"))?;
    let token = data
        .token
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| snowflake_err("external browser login response missing session token"))?;
    Ok(SnowflakeSession {
        token,
        expires_at: expires_at_from_validity(data.validity_in_seconds.as_ref()),
        master_token: data.master_token.filter(|value| !value.trim().is_empty()),
        master_expires_at: expires_at_from_validity(data.master_validity_in_seconds.as_ref()),
        id_token: data.id_token.filter(|value| !value.trim().is_empty()),
        id_token_expires_at: expires_at_from_validity(data.id_token_validity_in_seconds.as_ref()),
    })
}

fn summarize_snowflake_auth_response(code: Option<&str>, _message: Option<&str>) -> String {
    let code = code.unwrap_or("unknown");
    format!("{code}: request failed; check Snowflake externalbrowser configuration")
}

fn expires_at_from_validity(value: Option<&Value>) -> Option<Instant> {
    parse_optional_u64(value)
        .and_then(|seconds| Instant::now().checked_add(Duration::from_secs(seconds)))
}

async fn bind_browser_callback_listener(callback_port: Option<u16>) -> Result<TcpListener> {
    TcpListener::bind(("127.0.0.1", callback_port.unwrap_or(0)))
        .await
        .map_err(|err| snowflake_err(format!("failed to bind external browser callback: {err}")))
}

async fn receive_browser_callback(
    listener: TcpListener,
    expected_proof_key: &str,
    auth_timeout: Duration,
) -> Result<BrowserCallback> {
    let started_at = Instant::now();
    loop {
        let Some(remaining) = auth_timeout.checked_sub(started_at.elapsed()) else {
            return Err(snowflake_err(
                "timed out waiting for Snowflake browser SSO callback",
            ));
        };
        let accept = timeout(remaining, listener.accept())
            .await
            .map_err(|_| snowflake_err("timed out waiting for Snowflake browser SSO callback"))?;
        let (mut socket, peer_addr) = accept
            .map_err(|err| snowflake_err(format!("failed to accept browser callback: {err}")))?;
        if !peer_addr.ip().is_loopback() {
            let _ = write_browser_callback_response(&mut socket, false, None).await;
            return Err(snowflake_err(
                "external browser callback must originate from loopback",
            ));
        }

        let parsed = read_browser_callback_request(&mut socket, remaining)
            .await
            .and_then(|request| parse_browser_callback_request(&request, expected_proof_key));
        match parsed {
            Ok(BrowserCallbackRequest::Callback(callback)) => {
                let _ =
                    write_browser_callback_response(&mut socket, true, callback.origin.as_deref())
                        .await;
                return Ok(callback);
            }
            Ok(BrowserCallbackRequest::Preflight(preflight)) => {
                let _ = write_browser_preflight_response(&mut socket, &preflight).await;
            }
            Err(err) => {
                let _ = write_browser_callback_response(&mut socket, false, None).await;
                return Err(err);
            }
        }
    }
}

async fn read_browser_callback_request(
    socket: &mut tokio::net::TcpStream,
    auth_timeout: Duration,
) -> Result<String> {
    let mut request = Vec::with_capacity(1024);
    let mut buffer = [0u8; 1024];
    let mut expected_length = None;
    let deadline = Instant::now()
        .checked_add(auth_timeout)
        .ok_or_else(|| snowflake_err("Snowflake browser SSO callback timeout is too large"))?;
    loop {
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            return Err(snowflake_err(
                "timed out reading Snowflake browser SSO callback",
            ));
        };
        let bytes_read = timeout(remaining, socket.read(&mut buffer))
            .await
            .map_err(|_| snowflake_err("timed out reading Snowflake browser SSO callback"))?
            .map_err(|err| snowflake_err(format!("failed to read browser callback: {err}")))?;
        if bytes_read == 0 {
            break;
        }
        request.extend_from_slice(&buffer[..bytes_read]);
        if request.len() > MAX_BROWSER_CALLBACK_REQUEST_BYTES {
            return Err(snowflake_err(
                "external browser callback request is too large",
            ));
        }
        if expected_length.is_none()
            && let Some(header_end) = browser_callback_header_end(&request)
        {
            let headers = std::str::from_utf8(&request[..header_end])
                .map_err(|_| snowflake_err("external browser callback was not valid UTF-8"))?;
            let content_length = parse_content_length_header(headers)?;
            let total_length = header_end
                .checked_add(4)
                .and_then(|value| value.checked_add(content_length))
                .ok_or_else(|| snowflake_err("external browser callback request is too large"))?;
            if total_length > MAX_BROWSER_CALLBACK_REQUEST_BYTES {
                return Err(snowflake_err(
                    "external browser callback request is too large",
                ));
            }
            expected_length = Some(total_length);
        }
        if expected_length.is_some_and(|length| request.len() >= length) {
            break;
        }
    }

    String::from_utf8(request)
        .map_err(|_| snowflake_err("external browser callback was not valid UTF-8"))
}

async fn write_browser_callback_response(
    socket: &mut tokio::net::TcpStream,
    ok: bool,
    origin: Option<&str>,
) -> Result<()> {
    let body = if ok {
        "Snowflake authentication complete. You can close this tab."
    } else {
        "Snowflake authentication failed. Return to dbt-nova for details."
    };
    let status = if ok { "200 OK" } else { "400 Bad Request" };
    let cors = origin.map_or_else(String::new, |origin| {
        format!("Access-Control-Allow-Origin: {origin}\r\nVary: Origin\r\n")
    });
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\n{cors}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    socket
        .write_all(response.as_bytes())
        .await
        .map_err(|err| snowflake_err(format!("failed to write browser callback response: {err}")))
}

async fn write_browser_preflight_response(
    socket: &mut tokio::net::TcpStream,
    preflight: &BrowserCallbackPreflight,
) -> Result<()> {
    let origin = preflight.origin.as_deref().unwrap_or("null");
    let requested_headers = preflight
        .requested_headers
        .as_deref()
        .unwrap_or("content-type");
    let response = format!(
        "HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: {origin}\r\nAccess-Control-Allow-Methods: POST, GET, OPTIONS\r\nAccess-Control-Allow-Headers: {requested_headers}\r\nAccess-Control-Max-Age: 86400\r\nVary: Origin\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    );
    socket
        .write_all(response.as_bytes())
        .await
        .map_err(|err| snowflake_err(format!("failed to write browser callback response: {err}")))
}

fn parse_browser_callback_request(
    request: &str,
    expected_proof_key: &str,
) -> Result<BrowserCallbackRequest> {
    let request_line = request
        .lines()
        .next()
        .ok_or_else(|| snowflake_err("external browser callback was empty"))?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let target = parts.next().unwrap_or_default();
    let version = parts.next().unwrap_or_default();
    if !matches!(method, "GET" | "POST" | "OPTIONS")
        || !version.starts_with("HTTP/")
        || parts.next().is_some()
    {
        return Err(snowflake_err(
            "external browser callback must be an HTTP GET or POST request",
        ));
    }
    if !target.starts_with('/') {
        return Err(snowflake_err(
            "external browser callback target must be a rooted path",
        ));
    }
    let url = Url::parse(&format!("http://127.0.0.1{target}"))
        .map_err(|err| snowflake_err(format!("invalid external browser callback URL: {err}")))?;
    if url.path() != "/" {
        return Err(snowflake_err(
            "external browser callback path must be the root path",
        ));
    }

    if method == "OPTIONS" {
        return Ok(BrowserCallbackRequest::Preflight(
            BrowserCallbackPreflight {
                origin: request_header_value(request, "Origin").map(str::to_string),
                requested_headers: request_header_value(request, "Access-Control-Request-Headers")
                    .map(str::to_string),
            },
        ));
    }

    let (token, proof_key) = if method == "GET" {
        token_and_proof_key_from_pairs(
            url.query_pairs()
                .map(|(key, value)| (key.into_owned(), value.into_owned())),
        )
    } else {
        token_and_proof_key_from_post_body(
            browser_callback_body(request),
            request_header_value(request, "Content-Type").unwrap_or_default(),
        )?
    };

    if proof_key
        .as_deref()
        .is_some_and(|value| value != expected_proof_key)
    {
        return Err(snowflake_err(
            "external browser callback proof key did not match",
        ));
    }

    let token = token
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| snowflake_err("external browser callback missing token"))?;
    Ok(BrowserCallbackRequest::Callback(BrowserCallback {
        token,
        proof_key,
        origin: request_header_value(request, "Origin").map(str::to_string),
    }))
}

fn browser_callback_header_end(request: &[u8]) -> Option<usize> {
    request.windows(4).position(|window| window == b"\r\n\r\n")
}

fn parse_content_length_header(headers: &str) -> Result<usize> {
    let Some(value) = request_header_value(headers, "Content-Length") else {
        return Ok(0);
    };
    value.parse::<usize>().map_err(|err| {
        snowflake_err(format!(
            "invalid external browser callback Content-Length: {err}"
        ))
    })
}

fn request_header_value<'a>(request: &'a str, name: &str) -> Option<&'a str> {
    request.lines().skip(1).find_map(|line| {
        let (header, value) = line.split_once(':')?;
        header
            .trim()
            .eq_ignore_ascii_case(name)
            .then_some(value.trim())
    })
}

fn browser_callback_body(request: &str) -> &str {
    request
        .split_once("\r\n\r\n")
        .map(|(_, body)| body)
        .unwrap_or_default()
}

fn token_and_proof_key_from_post_body(
    body: &str,
    content_type: &str,
) -> Result<(Option<String>, Option<String>)> {
    if content_type
        .to_ascii_lowercase()
        .starts_with("application/json")
    {
        let parsed = serde_json::from_str::<Value>(body)
            .map_err(|err| snowflake_err(format!("invalid browser callback JSON body: {err}")))?;
        let token = parsed
            .get("token")
            .and_then(Value::as_str)
            .map(str::to_string);
        let proof_key = parsed
            .get("proofKey")
            .or_else(|| parsed.get("proof_key"))
            .and_then(Value::as_str)
            .map(str::to_string);
        return Ok((token, proof_key));
    }

    let form_url = Url::parse(&format!("http://127.0.0.1/?{body}")).map_err(|err| {
        snowflake_err(format!("invalid browser callback form-encoded body: {err}"))
    })?;
    Ok(token_and_proof_key_from_pairs(form_url.query_pairs().map(
        |(key, value)| (key.into_owned(), value.into_owned()),
    )))
}

fn token_and_proof_key_from_pairs<I>(pairs: I) -> (Option<String>, Option<String>)
where
    I: IntoIterator<Item = (String, String)>,
{
    let mut token = None;
    let mut proof_key = None;
    for (key, value) in pairs {
        match key.as_str() {
            "token" => token = Some(value),
            "proofKey" | "proof_key" => proof_key = Some(value),
            _ => {}
        }
    }
    (token, proof_key)
}

fn open_external_browser_url(url: &str, open_browser: bool) {
    if !open_browser {
        eprintln!("Open this Snowflake SSO URL to authenticate dbt-nova:\n{url}");
        return;
    }

    let mut command = browser_open_command(url);
    if let Err(err) = command.spawn() {
        warn!(
            error = %err,
            "failed to open system browser for Snowflake externalbrowser auth"
        );
        eprintln!("Open this Snowflake SSO URL to authenticate dbt-nova:\n{url}");
    }
}

fn browser_open_command(url: &str) -> Command {
    #[cfg(target_os = "macos")]
    {
        let mut command = Command::new("open");
        command.arg(url);
        command
    }
    #[cfg(target_os = "windows")]
    {
        let mut command = Command::new("cmd");
        command.args(["/C", "start", "", url]);
        command
    }
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        let mut command = Command::new("xdg-open");
        command.arg(url);
        command
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

fn resolve_auth_from_env(account: Option<String>, base_url: &str) -> Result<SnowflakeAuthConfig> {
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

    resolve_auth_from_mode(&auth, account, base_url)
}

fn resolve_auth_from_mode(
    auth: &str,
    account: Option<String>,
    base_url: &str,
) -> Result<SnowflakeAuthConfig> {
    match auth {
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
        "externalbrowser" | "external_browser" | "browser" => {
            ensure_external_browser_allowed_from_env(streamable_http_env_binds_non_loopback())?;
            let timeout = Duration::from_secs(
                env_u64(
                    "DBT_NOVA_SNOWFLAKE_EXTERNAL_BROWSER_TIMEOUT_S",
                    DEFAULT_EXTERNAL_BROWSER_TIMEOUT_SECONDS,
                )
                .max(1),
            );
            let open_browser = env_bool("DBT_NOVA_SNOWFLAKE_EXTERNAL_BROWSER_OPEN", true)?;
            let callback_port =
                env_u16_optional("DBT_NOVA_SNOWFLAKE_EXTERNAL_BROWSER_CALLBACK_PORT")?;
            build_external_browser_auth_config(
                base_url,
                account,
                read_optional_env("DBT_NOVA_SNOWFLAKE_USER"),
                timeout,
                open_browser,
                callback_port,
            )
        }
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
            "Unsupported DBT_NOVA_SNOWFLAKE_AUTH '{other}' (expected {SUPPORTED_SNOWFLAKE_AUTH_MODES})"
        ))),
    }
}

fn validate_external_browser_runtime(config: &DbtNovaConfig) -> Result<()> {
    validate_external_browser_runtime_for_auth(
        config,
        read_optional_env("DBT_NOVA_SNOWFLAKE_AUTH").as_deref(),
    )
}

fn validate_external_browser_runtime_for_auth(
    config: &DbtNovaConfig,
    auth_mode: Option<&str>,
) -> Result<()> {
    validate_external_browser_runtime_for_auth_with_ci(config, auth_mode, env_bool("CI", false)?)
}

fn validate_external_browser_runtime_for_auth_with_ci(
    config: &DbtNovaConfig,
    auth_mode: Option<&str>,
    running_in_ci: bool,
) -> Result<()> {
    if auth_mode.is_some_and(auth_mode_is_external_browser) {
        ensure_external_browser_allowed(config.http_transport_binds_non_loopback(), running_in_ci)?;
    }
    Ok(())
}

fn auth_mode_is_external_browser(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "externalbrowser" | "external_browser" | "browser"
    )
}

fn build_external_browser_auth_config(
    base_url: &str,
    account: Option<String>,
    user: Option<String>,
    timeout: Duration,
    open_browser: bool,
    callback_port: Option<u16>,
) -> Result<SnowflakeAuthConfig> {
    let user = user
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            DbtNovaError::InvalidParams(
                "DBT_NOVA_SNOWFLAKE_USER is required for Snowflake externalbrowser auth"
                    .to_string(),
            )
        })?;
    let account_identifier = account
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            DbtNovaError::InvalidParams(
                "DBT_NOVA_SNOWFLAKE_ACCOUNT is required for Snowflake externalbrowser auth; DBT_NOVA_SNOWFLAKE_ACCOUNT_URL alone is not enough for browser SSO login".to_string(),
            )
        })?;
    let session_cache =
        external_browser_session_cache(base_url, &account_identifier, &user, callback_port);
    Ok(SnowflakeAuthConfig::ExternalBrowser {
        user,
        account_identifier,
        timeout,
        open_browser,
        callback_port,
        session_cache,
    })
}

fn external_browser_session_cache(
    base_url: &str,
    account_identifier: &str,
    user: &str,
    callback_port: Option<u16>,
) -> ExternalBrowserSessionCache {
    let key = format!(
        "{}|{}|{}|{}",
        base_url.to_ascii_lowercase(),
        account_identifier.to_ascii_lowercase(),
        user.to_ascii_lowercase(),
        callback_port.map_or_else(|| "ephemeral".to_string(), |port| port.to_string())
    );
    let caches = EXTERNAL_BROWSER_SESSION_CACHES.get_or_init(|| StdMutex::new(HashMap::new()));
    let mut caches = match caches.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    caches
        .entry(key)
        .or_insert_with(|| Arc::new(TokioMutex::new(None)))
        .clone()
}

fn ensure_external_browser_allowed_from_env(non_loopback_http_bind: bool) -> Result<()> {
    ensure_external_browser_allowed(non_loopback_http_bind, env_bool("CI", false)?)
}

fn ensure_external_browser_allowed(
    non_loopback_http_bind: bool,
    running_in_ci: bool,
) -> Result<()> {
    if running_in_ci {
        return Err(DbtNovaError::InvalidParams(
            "Snowflake externalbrowser auth is interactive and cannot run in CI; use keypair, oauth, or pat auth".to_string(),
        ));
    }
    if non_loopback_http_bind {
        return Err(DbtNovaError::InvalidParams(
            "Snowflake externalbrowser auth is local-only and cannot be used with non-loopback streamable HTTP binds; use keypair, oauth, or pat auth for hosted deployments".to_string(),
        ));
    }
    Ok(())
}

fn streamable_http_env_binds_non_loopback() -> bool {
    let transport = read_optional_env("DBT_NOVA_SERVER_TRANSPORT")
        .map_or_else(|| "stdio".to_string(), |value| value.to_ascii_lowercase());
    if !matches!(
        transport.as_str(),
        "streamable_http" | "streamable-http" | "http"
    ) {
        return false;
    }
    let host = read_optional_env("DBT_NOVA_HTTP_HOST").unwrap_or_else(|| {
        if read_optional_env("PORT").is_some() {
            "0.0.0.0".to_string()
        } else {
            "127.0.0.1".to_string()
        }
    });
    !is_loopback_host(&host)
}

fn is_loopback_host(host: &str) -> bool {
    let normalized = host
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_ascii_lowercase();
    matches!(normalized.as_str(), "localhost" | "127.0.0.1" | "::1")
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
    strip_locator_region_suffix(value.trim())
        .replace('.', "-")
        .to_ascii_uppercase()
}

fn strip_locator_region_suffix(value: &str) -> &str {
    let mut segments = value.split('.');
    let Some(locator) = segments.next() else {
        return value;
    };
    let suffix: Vec<&str> = segments.collect();
    if suffix.is_empty() {
        return value;
    }

    let last = suffix
        .last()
        .map(|segment| segment.to_ascii_lowercase())
        .unwrap_or_default();
    let region_segments = if matches!(last.as_str(), "aws" | "azure" | "gcp") {
        &suffix[..suffix.len().saturating_sub(1)]
    } else {
        suffix.as_slice()
    };

    let region_segments = match region_segments {
        ["fhplus" | "dod", rest @ ..] => rest,
        other => other,
    };

    let has_explicit_locator_suffix = suffix.len() > 1;
    if region_segments.len() == 1
        && looks_like_snowflake_region(region_segments[0])
        && (has_explicit_locator_suffix || looks_like_generated_account_locator(locator))
    {
        locator
    } else {
        value
    }
}

fn looks_like_generated_account_locator(segment: &str) -> bool {
    let bytes = segment.as_bytes();
    bytes.len() == 7
        && bytes[..2].iter().all(u8::is_ascii_alphabetic)
        && bytes[2..].iter().all(u8::is_ascii_digit)
}

fn looks_like_snowflake_region(segment: &str) -> bool {
    let region = segment.to_ascii_lowercase();
    let has_region_shape = region.contains('-') || region.chars().any(|ch| ch.is_ascii_digit());
    let has_region_hint = [
        "af",
        "ap",
        "asia",
        "au",
        "australia",
        "ca",
        "canada",
        "central",
        "cn",
        "east",
        "eu",
        "europe",
        "france",
        "germany",
        "il",
        "india",
        "japan",
        "korea",
        "me",
        "north",
        "norway",
        "sa",
        "south",
        "sweden",
        "switzerland",
        "uae",
        "uk",
        "us",
        "west",
    ]
    .iter()
    .any(|hint| {
        region == *hint
            || region.starts_with(&format!("{hint}-"))
            || region.ends_with(&format!("-{hint}"))
            || region.contains(&format!("-{hint}-"))
    });

    has_region_shape && has_region_hint
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
        BrowserCallback, BrowserCallbackPreflight, BrowserCallbackRequest,
        DEFAULT_EXTERNAL_BROWSER_TIMEOUT_SECONDS, ExternalBrowserAuthenticatorRequest,
        ExternalBrowserAuthenticatorRequestData, ExternalBrowserLoginRequest,
        ExternalBrowserLoginRequestData, MAX_BROWSER_CALLBACK_REQUEST_BYTES, Result, ResultColumn,
        SnowflakeAuthConfig, SnowflakeExecuteOptions, SnowflakeQueryResult, SnowflakeQueryStats,
        SnowflakeSqlClient, SnowflakeSqlConfig, build_bindings, build_external_browser_auth_config,
        catalog_preflight_statement, decode_external_browser_authenticator_response,
        decode_external_browser_login_response, decode_statement_status_response,
        external_browser_session_cache, generate_keypair_jwt, normalize_account_url,
        normalize_jwt_identifier, normalize_preflight_relation, parse_browser_callback_request,
        parse_cell_value, preflight_show_result_has_exact_name, public_key_fingerprint,
        read_browser_callback_request, relation_preflight_statement,
        resolve_schema_preflight_target, rewrite_named_parameters, schema_preflight_statement,
        send_json, session_parameters, snowflake_err, summarize_error_body,
        validate_external_browser_runtime_for_auth_with_ci,
    };
    use crate::config::{DbtNovaConfig, ServerTransport};
    use flate2::{Compression, write::GzEncoder};
    use reqwest::Client;
    use reqwest::StatusCode;
    use serde_json::{Value, json};
    use std::collections::HashMap;
    use std::io::Write;
    use std::net::TcpListener as StdTcpListener;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;
    use tokio::sync::Mutex as TokioMutex;
    use tokio::time::timeout;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

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
    fn external_browser_runtime_policy_rejects_configured_non_loopback_http_bind() {
        let config = DbtNovaConfig {
            server_transport: ServerTransport::StreamableHttp,
            http_host: "0.0.0.0".to_string(),
            ..DbtNovaConfig::default()
        };

        let err = validate_external_browser_runtime_for_auth_with_ci(
            &config,
            Some("externalbrowser"),
            false,
        )
        .expect_err("externalbrowser should reject hosted non-loopback binds");
        assert!(err.to_string().contains("non-loopback"));

        validate_external_browser_runtime_for_auth_with_ci(&config, Some("keypair"), false)
            .expect("non-browser auth is allowed for hosted binds");
    }

    #[test]
    fn external_browser_runtime_policy_allows_configured_loopback_http_bind() {
        let config = DbtNovaConfig {
            server_transport: ServerTransport::StreamableHttp,
            http_host: "127.0.0.1".to_string(),
            ..DbtNovaConfig::default()
        };

        validate_external_browser_runtime_for_auth_with_ci(&config, Some("browser"), false)
            .expect("externalbrowser is allowed on loopback binds");
    }

    #[test]
    fn external_browser_runtime_policy_rejects_ci() {
        let config = DbtNovaConfig::default();

        let err = validate_external_browser_runtime_for_auth_with_ci(
            &config,
            Some("external_browser"),
            true,
        )
        .expect_err("externalbrowser should reject CI");
        assert!(err.to_string().contains("CI"));
    }

    #[test]
    fn external_browser_auth_env_requires_user() {
        let Err(err) = build_external_browser_auth_config(
            "https://org-account.snowflakecomputing.com",
            Some("org-account".to_string()),
            None,
            Duration::from_secs(DEFAULT_EXTERNAL_BROWSER_TIMEOUT_SECONDS),
            true,
            None,
        ) else {
            panic!("externalbrowser without user should fail");
        };
        assert!(err.to_string().contains("DBT_NOVA_SNOWFLAKE_USER"));
    }

    #[test]
    fn external_browser_auth_env_resolves_aliases_and_options() {
        let auth = build_external_browser_auth_config(
            "https://org-account.snowflakecomputing.com",
            Some("org-account".to_string()),
            Some("analyst@example.com".to_string()),
            Duration::from_secs(45),
            false,
            Some(4567),
        )
        .expect("externalbrowser auth");
        let SnowflakeAuthConfig::ExternalBrowser {
            user,
            account_identifier,
            timeout,
            open_browser,
            callback_port,
            ..
        } = auth
        else {
            panic!("expected externalbrowser auth");
        };
        assert_eq!(user, "analyst@example.com");
        assert_eq!(account_identifier, "org-account");
        assert_eq!(timeout, Duration::from_secs(45));
        assert!(!open_browser);
        assert_eq!(callback_port, Some(4567));
    }

    #[test]
    fn external_browser_session_cache_reuses_matching_keys() {
        let first = external_browser_session_cache(
            "https://org-account.snowflakecomputing.com",
            "org-account",
            "ANALYST",
            None,
        );
        let second = external_browser_session_cache(
            "https://ORG-ACCOUNT.snowflakecomputing.com",
            "ORG-ACCOUNT",
            "analyst",
            None,
        );
        assert!(Arc::ptr_eq(&first, &second));
    }

    #[test]
    fn external_browser_request_bodies_use_snowflake_field_names() {
        let auth_request = serde_json::to_value(ExternalBrowserAuthenticatorRequest {
            data: ExternalBrowserAuthenticatorRequestData {
                client_app_id: "dbt-nova",
                client_app_version: "0.0.0-test",
                account_name: "org-account",
                login_name: "analyst@example.com",
                authenticator: "EXTERNALBROWSER",
                browser_mode_redirect_port: "4567".to_string(),
            },
        })
        .expect("auth request JSON");
        assert_eq!(
            auth_request["data"]["LOGIN_NAME"],
            json!("analyst@example.com")
        );
        assert_eq!(auth_request["data"]["CLIENT_APP_ID"], json!("dbt-nova"));
        assert_eq!(auth_request["data"]["ACCOUNT_NAME"], json!("org-account"));
        assert_eq!(
            auth_request["data"]["AUTHENTICATOR"],
            json!("EXTERNALBROWSER")
        );
        assert_eq!(
            auth_request["data"]["BROWSER_MODE_REDIRECT_PORT"],
            json!("4567")
        );

        let login_request = serde_json::to_value(ExternalBrowserLoginRequest {
            data: ExternalBrowserLoginRequestData {
                client_app_id: "dbt-nova",
                client_app_version: "0.0.0-test",
                account_name: "org-account",
                login_name: "analyst@example.com",
                authenticator: "EXTERNALBROWSER",
                token: "callback-token",
                proof_key: Some("proof-key"),
            },
        })
        .expect("login request JSON");
        assert_eq!(login_request["data"]["CLIENT_APP_ID"], json!("dbt-nova"));
        assert_eq!(login_request["data"]["ACCOUNT_NAME"], json!("org-account"));
        assert_eq!(login_request["data"]["TOKEN"], json!("callback-token"));
        assert_eq!(login_request["data"]["PROOF_KEY"], json!("proof-key"));

        let token_only_login_request = serde_json::to_value(ExternalBrowserLoginRequest {
            data: ExternalBrowserLoginRequestData {
                client_app_id: "dbt-nova",
                client_app_version: "0.0.0-test",
                account_name: "org-account",
                login_name: "analyst@example.com",
                authenticator: "EXTERNALBROWSER",
                token: "callback-token",
                proof_key: None,
            },
        })
        .expect("token-only login request JSON");
        assert_eq!(
            token_only_login_request["data"]["TOKEN"],
            json!("callback-token")
        );
        assert!(
            token_only_login_request["data"]
                .as_object()
                .expect("login data object")
                .get("PROOF_KEY")
                .is_none()
        );
    }

    #[test]
    fn external_browser_response_decoders_parse_success_and_sanitize_failures() {
        let auth_body = r#"{
            "success": true,
            "data": {
                "ssoUrl": "https://idp.example.com/start",
                "proofKey": "proof-key"
            }
        }"#;
        let auth = decode_external_browser_authenticator_response(StatusCode::OK, auth_body)
            .expect("authenticator response");
        assert_eq!(auth.sso_url, "https://idp.example.com/start");
        assert_eq!(auth.proof_key, "proof-key");

        let login_body = r#"{
            "success": true,
            "data": {
                "token": "session-token",
                "validityInSeconds": 3600,
                "masterToken": "master-token",
                "masterValidityInSeconds": 7200,
                "idToken": "id-token",
                "idTokenValidityInSeconds": 1800
            }
        }"#;
        let session =
            decode_external_browser_login_response(StatusCode::OK, login_body).expect("login");
        assert_eq!(session.token, "session-token");
        assert!(session.expires_at.is_some());
        assert_eq!(session.master_token.as_deref(), Some("master-token"));
        assert!(session.master_expires_at.is_some());
        assert_eq!(session.id_token.as_deref(), Some("id-token"));
        assert!(session.id_token_expires_at.is_some());

        let failure_body = r#"{
            "success": false,
            "code": "390303",
            "message": "Invalid token SECRET_TOKEN_VALUE"
        }"#;
        let err = decode_external_browser_login_response(StatusCode::OK, failure_body)
            .expect_err("login failure");
        assert!(err.to_string().contains("390303"));
        assert!(!err.to_string().contains("SECRET_TOKEN_VALUE"));
    }

    #[test]
    fn external_browser_login_response_accepts_session_token_alias() {
        let login_body = r#"{
            "success": true,
            "data": {
                "sessionToken": "session-token",
                "validityInSeconds": "3600"
            }
        }"#;
        let session =
            decode_external_browser_login_response(StatusCode::OK, login_body).expect("login");
        assert_eq!(session.token, "session-token");
        assert!(session.expires_at.is_some());
    }

    #[test]
    fn browser_callback_parser_extracts_token_and_checks_proof_key() {
        let request =
            "GET /?token=callback%2Ftoken&proofKey=proof-key HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n";
        let callback =
            parse_browser_callback_request(request, "proof-key").expect("browser callback");
        assert_eq!(
            callback,
            BrowserCallbackRequest::Callback(BrowserCallback {
                token: "callback/token".to_string(),
                proof_key: Some("proof-key".to_string()),
                origin: None,
            })
        );

        let err =
            parse_browser_callback_request(request, "other-proof").expect_err("proof mismatch");
        assert!(err.to_string().contains("proof key"));

        let token_only_request = "GET /?token=callback%2Ftoken HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n";
        let callback = parse_browser_callback_request(token_only_request, "proof-key")
            .expect("token-only browser callback");
        assert_eq!(
            callback,
            BrowserCallbackRequest::Callback(BrowserCallback {
                token: "callback/token".to_string(),
                proof_key: None,
                origin: None,
            })
        );
    }

    #[test]
    fn browser_callback_parser_accepts_post_and_preflight() {
        let json_request = "POST / HTTP/1.1\r\nHost: 127.0.0.1\r\nOrigin: https://org-account.snowflakecomputing.com\r\nContent-Type: application/json\r\nContent-Length: 52\r\n\r\n{\"token\":\"callback-token\",\"proofKey\":\"proof-key\"}";
        let callback =
            parse_browser_callback_request(json_request, "proof-key").expect("json callback");
        assert_eq!(
            callback,
            BrowserCallbackRequest::Callback(BrowserCallback {
                token: "callback-token".to_string(),
                proof_key: Some("proof-key".to_string()),
                origin: Some("https://org-account.snowflakecomputing.com".to_string()),
            })
        );

        let form_request = "POST / HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: 40\r\n\r\ntoken=callback%2Ftoken&proof_key=proof-key";
        let callback =
            parse_browser_callback_request(form_request, "proof-key").expect("form callback");
        assert_eq!(
            callback,
            BrowserCallbackRequest::Callback(BrowserCallback {
                token: "callback/token".to_string(),
                proof_key: Some("proof-key".to_string()),
                origin: None,
            })
        );

        let options_request = "OPTIONS / HTTP/1.1\r\nHost: 127.0.0.1\r\nOrigin: https://org-account.snowflakecomputing.com\r\nAccess-Control-Request-Headers: content-type\r\n\r\n";
        let preflight =
            parse_browser_callback_request(options_request, "proof-key").expect("preflight");
        assert_eq!(
            preflight,
            BrowserCallbackRequest::Preflight(BrowserCallbackPreflight {
                origin: Some("https://org-account.snowflakecomputing.com".to_string()),
                requested_headers: Some("content-type".to_string()),
            })
        );
    }

    #[test]
    fn browser_callback_parser_rejects_wrong_method_path_and_missing_token() {
        for request in [
            "POST /?token=callback HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n",
            "GET /callback?token=callback HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n",
            "GET /?proofKey=proof-key HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n",
        ] {
            assert!(
                parse_browser_callback_request(request, "proof-key").is_err(),
                "{request} should be rejected"
            );
        }
    }

    #[tokio::test]
    async fn browser_callback_reader_enforces_total_read_timeout() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind callback listener");
        let port = listener.local_addr().expect("listener addr").port();
        let read = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept callback");
            let started_at = std::time::Instant::now();
            let err = read_browser_callback_request(&mut socket, Duration::from_millis(60))
                .await
                .expect_err("slow callback should time out");
            (started_at.elapsed(), err.to_string())
        });

        let mut stream = TcpStream::connect(("127.0.0.1", port))
            .await
            .expect("connect callback");
        stream
            .write_all(b"POST / HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 4\r\n\r\na")
            .await
            .expect("write first chunk");
        tokio::time::sleep(Duration::from_millis(40)).await;
        stream.write_all(b"b").await.expect("write second chunk");
        tokio::time::sleep(Duration::from_millis(40)).await;
        let _ = stream.write_all(b"c").await;

        let (elapsed, error) = read.await.expect("read task");
        assert!(error.contains("timed out reading"));
        assert!(
            elapsed < Duration::from_millis(120),
            "callback read elapsed {elapsed:?}, expected total timeout bound"
        );
    }

    #[tokio::test]
    async fn browser_callback_reader_rejects_oversized_requests() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind callback listener");
        let port = listener.local_addr().expect("listener addr").port();
        let read = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept callback");
            read_browser_callback_request(&mut socket, Duration::from_secs(1))
                .await
                .expect_err("oversized callback should fail")
                .to_string()
        });

        let mut stream = TcpStream::connect(("127.0.0.1", port))
            .await
            .expect("connect callback");
        let request = format!(
            "POST / HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: {}\r\n\r\n",
            MAX_BROWSER_CALLBACK_REQUEST_BYTES + 1
        );
        stream
            .write_all(request.as_bytes())
            .await
            .expect("write oversized request");

        let error = read.await.expect("read task");
        assert!(error.contains("too large"));
    }

    #[tokio::test]
    async fn external_browser_login_flow_exchanges_callback_token() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/session/authenticator-request"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "success": true,
                "data": {
                    "ssoUrl": "https://idp.example.com/start",
                    "proofKey": "proof-key"
                }
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/session/v1/login-request"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "success": true,
                "data": {
                    "token": "session-token",
                    "validityInSeconds": 3600
                }
            })))
            .mount(&server)
            .await;

        let callback_port = unused_loopback_port();
        let client = SnowflakeSqlClient::new(SnowflakeSqlConfig {
            base_url: server.uri(),
            warehouse: "COMPUTE_WH".to_string(),
            database: None,
            schema: None,
            role: None,
            timeout: Duration::from_secs(5),
            default_statement_timeout_s: 60,
            poll_interval: Duration::from_millis(1),
            max_poll: Duration::from_secs(1),
            max_chunks: 1,
            auth: SnowflakeAuthConfig::ExternalBrowser {
                user: "analyst@example.com".to_string(),
                account_identifier: "org-account".to_string(),
                timeout: Duration::from_secs(5),
                open_browser: false,
                callback_port: Some(callback_port),
                session_cache: Arc::new(TokioMutex::new(None)),
            },
        })
        .expect("client");

        let login = tokio::spawn(async move {
            client
                .login_external_browser(
                    "org-account",
                    "analyst@example.com",
                    Duration::from_secs(5),
                    false,
                    Some(callback_port),
                )
                .await
        });

        let preflight_request = format!(
            "OPTIONS / HTTP/1.1\r\nHost: 127.0.0.1:{callback_port}\r\nOrigin: https://org-account.snowflakecomputing.com\r\nAccess-Control-Request-Headers: content-type\r\n\r\n"
        );
        let preflight_response = send_raw_browser_callback(callback_port, &preflight_request)
            .await
            .expect("preflight response");
        assert!(preflight_response.starts_with("HTTP/1.1 204 No Content"));

        let callback_body = r#"{"token":"callback-token"}"#;
        let callback_request = format!(
            "POST / HTTP/1.1\r\nHost: 127.0.0.1:{callback_port}\r\nOrigin: https://org-account.snowflakecomputing.com\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{callback_body}",
            callback_body.len()
        );
        let callback_response = send_raw_browser_callback(callback_port, &callback_request)
            .await
            .expect("callback response");
        assert!(callback_response.starts_with("HTTP/1.1 200 OK"));

        let session = login.await.expect("login task").expect("login result");
        assert_eq!(session.token, "session-token");
        assert!(session.expires_at.is_some());
    }

    fn unused_loopback_port() -> u16 {
        let listener = StdTcpListener::bind("127.0.0.1:0").expect("bind free port");
        listener.local_addr().expect("local addr").port()
    }

    async fn send_raw_browser_callback(port: u16, request: &str) -> Result<String> {
        let mut last_err = None;
        for _ in 0..100 {
            match TcpStream::connect(("127.0.0.1", port)).await {
                Ok(mut stream) => {
                    stream
                        .write_all(request.as_bytes())
                        .await
                        .map_err(|err| snowflake_err(format!("write callback: {err}")))?;
                    let mut response = String::new();
                    stream
                        .read_to_string(&mut response)
                        .await
                        .map_err(|err| snowflake_err(format!("read callback: {err}")))?;
                    return Ok(response);
                }
                Err(err) => {
                    last_err = Some(err);
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }
        }
        Err(snowflake_err(format!(
            "callback listener did not accept connection: {}",
            last_err.map_or_else(|| "unknown error".to_string(), |err| err.to_string())
        )))
    }

    fn test_snowflake_config(default_statement_timeout_s: u64) -> SnowflakeSqlConfig {
        SnowflakeSqlConfig {
            base_url: "https://org-account.snowflakecomputing.com".to_string(),
            warehouse: "COMPUTE_WH".to_string(),
            database: None,
            schema: None,
            role: None,
            timeout: Duration::from_secs(5),
            default_statement_timeout_s,
            poll_interval: Duration::from_millis(1),
            max_poll: Duration::from_secs(1),
            max_chunks: 1,
            auth: SnowflakeAuthConfig::OAuth {
                token: "oauth-token".to_string(),
            },
        }
    }

    #[test]
    fn execute_options_allow_statement_timeout_zero_sentinel() {
        let request_override = SnowflakeExecuteOptions {
            statement_timeout_s: Some(0),
            ..SnowflakeExecuteOptions::default()
        }
        .resolve(&test_snowflake_config(60));
        assert_eq!(request_override.statement_timeout_s, 0);

        let config_default = SnowflakeExecuteOptions::default().resolve(&test_snowflake_config(0));
        assert_eq!(config_default.statement_timeout_s, 0);
    }

    #[test]
    fn execute_options_use_config_default_max_chunks_when_unset() {
        let mut config = test_snowflake_config(60);
        config.max_chunks = 7;

        let config_default = SnowflakeExecuteOptions::default().resolve(&config);
        assert_eq!(config_default.max_chunks, 7);

        let request_override = SnowflakeExecuteOptions {
            max_chunks: Some(2),
            ..SnowflakeExecuteOptions::default()
        }
        .resolve(&config);
        assert_eq!(request_override.max_chunks, 2);
    }

    #[tokio::test]
    async fn poll_statement_respects_max_poll_when_interval_is_larger() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v2/statements/statement-handle/cancel"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "ok"})))
            .mount(&server)
            .await;

        let mut config = test_snowflake_config(60);
        config.base_url = server.uri();
        let client = SnowflakeSqlClient::new(config).expect("client");

        let result = timeout(
            Duration::from_millis(200),
            client.poll_statement(
                "statement-handle",
                Duration::from_mins(1),
                Duration::from_millis(20),
            ),
        )
        .await
        .expect("poll timeout should be locally bounded");
        let err = result.expect_err("statement should time out");
        assert!(err.to_string().contains("Timed out waiting"));
    }

    #[tokio::test]
    async fn send_json_decodes_gzip_responses() {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder
            .write_all(br#"{"data":[["compressed"]]}"#)
            .expect("write gzip body");
        let body = encoder.finish().expect("finish gzip body");
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/partition"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-encoding", "gzip")
                    .insert_header("content-type", "application/json")
                    .set_body_bytes(body),
            )
            .mount(&server)
            .await;

        let client = Client::builder().gzip(true).build().expect("client");
        let response: Value = send_json(client.get(format!("{}/partition", server.uri())))
            .await
            .expect("decode gzip JSON");
        assert_eq!(response["data"][0][0], json!("compressed"));
    }

    #[test]
    fn statement_status_decodes_429_query_status_as_pending() {
        let body = r#"{
            "code": "333333",
            "message": "Statement is still executing",
            "statementHandle": "536fad38-b564-4dc5-9892-a4543504df6c",
            "statementStatusUrl": "/api/v2/statements/536fad38-b564-4dc5-9892-a4543504df6c"
        }"#;
        let response =
            decode_statement_status_response(StatusCode::TOO_MANY_REQUESTS, body).expect("status");
        assert!(response.is_pending());
        assert_eq!(
            response.statement_handle.as_deref(),
            Some("536fad38-b564-4dc5-9892-a4543504df6c")
        );
    }

    #[test]
    fn statement_status_decodes_async_query_status_as_pending() {
        let body = r#"{
            "code": "333334",
            "message": "Asynchronous execution in progress. Use provided query id to perform query monitoring and management.",
            "statementHandle": "536fad38-b564-4dc5-9892-a4543504df6c",
            "statementStatusUrl": "/api/v2/statements/536fad38-b564-4dc5-9892-a4543504df6c"
        }"#;
        let response =
            decode_statement_status_response(StatusCode::ACCEPTED, body).expect("status");

        assert!(response.is_pending());
        assert_eq!(response.failure_message(), None);
        assert_eq!(
            response.statement_handle.as_deref(),
            Some("536fad38-b564-4dc5-9892-a4543504df6c")
        );
    }

    #[test]
    fn statement_status_url_error_is_terminal() {
        let body = r#"{
            "code": "604",
            "message": "Statement was canceled",
            "sqlState": "57014",
            "statementHandle": "536fad38-b564-4dc5-9892-a4543504df6c",
            "statementStatusUrl": "/api/v2/statements/536fad38-b564-4dc5-9892-a4543504df6c"
        }"#;
        let response =
            decode_statement_status_response(StatusCode::OK, body).expect("terminal status");

        assert!(!response.is_pending());
        let message = response.failure_message().expect("failure message");
        assert!(message.contains("604"));
        assert!(message.contains("57014"));
        assert!(message.contains("Statement was canceled"));
    }

    #[test]
    fn statement_status_keeps_non_pending_429_as_error() {
        let body = r#"{"code":"390505","message":"Too many requests."}"#;
        let err = decode_statement_status_response(StatusCode::TOO_MANY_REQUESTS, body)
            .expect_err("rate limit should stay an error");
        assert!(err.to_string().contains("390505"));
    }

    #[test]
    fn session_parameters_use_sql_api_field_shapes() {
        let params = session_parameters(250);
        assert_eq!(params["binary_output_format"], json!("HEX"));
        assert_eq!(params["rows_per_resultset"], json!(250));
        assert!(!params.contains_key("BINARY_OUTPUT_FORMAT"));
    }

    #[test]
    fn normalize_jwt_identifier_strips_locator_region_suffixes() {
        assert_eq!(normalize_jwt_identifier("xy12345.us-east-1"), "XY12345");
        assert_eq!(normalize_jwt_identifier("xy12345.us-east-2.aws"), "XY12345");
        assert_eq!(
            normalize_jwt_identifier("xy12345.fhplus.us-gov-west-1.aws"),
            "XY12345"
        );
    }

    #[test]
    fn normalize_jwt_identifier_preserves_organization_account_names() {
        assert_eq!(
            normalize_jwt_identifier("myorg.myaccount"),
            "MYORG-MYACCOUNT"
        );
        assert_eq!(
            normalize_jwt_identifier("myorg.us-east-1"),
            "MYORG-US-EAST-1"
        );
        assert_eq!(
            normalize_jwt_identifier("myorg2.us-east-1"),
            "MYORG2-US-EAST-1"
        );
        assert_eq!(
            normalize_jwt_identifier("myorg-myaccount"),
            "MYORG-MYACCOUNT"
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
    fn rewrite_named_parameters_skips_dollar_quoted_literals() {
        let params = HashMap::from([("id".to_string(), json!(42))]);
        let rewritten = rewrite_named_parameters(
            "select $$literal :missing\nand 'quoted' text$$ as body where id = :id",
            &params,
        )
        .expect("rewrite");
        assert_eq!(
            rewritten.sql,
            "select $$literal :missing\nand 'quoted' text$$ as body where id = ?"
        );
        assert_eq!(rewritten.ordered_parameters, vec!["id".to_string()]);
    }

    #[test]
    fn rewrite_named_parameters_skips_backslash_escaped_quotes() {
        let params = HashMap::from([("id".to_string(), json!(42))]);
        let rewritten =
            rewrite_named_parameters("select 'can\\'t :missing' as body where id = :id", &params)
                .expect("rewrite");
        assert_eq!(
            rewritten.sql,
            "select 'can\\'t :missing' as body where id = ?"
        );
        assert_eq!(rewritten.ordered_parameters, vec!["id".to_string()]);
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
    fn parse_cell_value_preserves_fixed_numeric_precision() {
        let decimal_field = ResultColumn {
            name: "amount".to_string(),
            type_name: "DECIMAL".to_string(),
            scale: Some(json!(6)),
        };
        let large_integer_field = ResultColumn {
            name: "external_id".to_string(),
            type_name: "NUMBER".to_string(),
            scale: Some(json!(0)),
        };
        let missing_scale_field = ResultColumn {
            name: "metric".to_string(),
            type_name: "FIXED".to_string(),
            scale: None,
        };

        assert_eq!(
            parse_cell_value(&json!("12345678901234567890.123456"), &decimal_field),
            json!("12345678901234567890.123456")
        );
        assert_eq!(
            parse_cell_value(&json!("9007199254740993"), &large_integer_field),
            json!("9007199254740993")
        );
        assert_eq!(
            parse_cell_value(&json!("42"), &missing_scale_field),
            json!("42")
        );
    }

    #[test]
    fn parse_cell_value_preserves_non_finite_float_text() {
        let float_field = ResultColumn {
            name: "ratio".to_string(),
            type_name: "FLOAT".to_string(),
            scale: None,
        };

        assert_eq!(parse_cell_value(&json!("1.25"), &float_field), json!(1.25));
        assert_eq!(parse_cell_value(&json!("NaN"), &float_field), json!("NaN"));
        assert_eq!(parse_cell_value(&json!("inf"), &float_field), json!("inf"));
        assert_eq!(
            parse_cell_value(&json!("-inf"), &float_field),
            json!("-inf")
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
            "SHOW DATABASES STARTS WITH 'ANALYTICS' LIMIT 1"
        );
        assert_eq!(
            catalog_preflight_statement("ANALYTICS_REPORTING"),
            "SHOW DATABASES STARTS WITH 'ANALYTICS_REPORTING' LIMIT 1"
        );
        assert_eq!(
            schema_preflight_statement("ANALYTICS", "REPORTING"),
            "SHOW SCHEMAS IN DATABASE ANALYTICS STARTS WITH 'REPORTING' LIMIT 1"
        );
        assert_eq!(
            schema_preflight_statement("ANALYTICS", "REPORTING_SCHEMA"),
            "SHOW SCHEMAS IN DATABASE ANALYTICS STARTS WITH 'REPORTING_SCHEMA' LIMIT 1"
        );
        assert_eq!(
            relation_preflight_statement("ANALYTICS.REPORTING.ORDERS"),
            "SELECT 1 AS relation_access_check FROM ANALYTICS.REPORTING.ORDERS LIMIT 1"
        );
    }

    #[test]
    fn schema_preflight_target_normalizes_default_catalog() {
        let target =
            resolve_schema_preflight_target(None, Some("analytics"), "reporting").expect("target");
        assert_eq!(target, ("ANALYTICS".to_string(), "REPORTING".to_string()));
    }

    #[test]
    fn schema_preflight_target_rejects_unsafe_default_catalog() {
        let err = resolve_schema_preflight_target(None, Some("analytics;drop"), "reporting")
            .expect_err("unsafe default catalog should fail");
        assert!(err.to_string().contains("Invalid catalog identifier"));
    }

    #[test]
    fn preflight_show_result_requires_exact_name_match() {
        let mut result = SnowflakeQueryResult {
            statement_id: "01".to_string(),
            state: "SUCCEEDED".to_string(),
            provider: "snowflake".to_string(),
            account_url: "https://example.snowflakecomputing.com".to_string(),
            warehouse: "COMPUTE_WH".to_string(),
            database: None,
            schema: None,
            role: None,
            columns: vec!["created_on".to_string(), "name".to_string()],
            column_types: vec!["TIMESTAMP".to_string(), "TEXT".to_string()],
            rows: vec![vec![Value::Null, json!("ANALYTICS_REPORTING_DEV")]],
            elapsed_ms: 1,
            fetched_chunks: 1,
            stats: SnowflakeQueryStats {
                total_row_count: Some(1),
                total_byte_count: None,
                total_chunk_count: Some(1),
            },
            truncated: false,
        };

        assert!(!preflight_show_result_has_exact_name(
            &result,
            "ANALYTICS_REPORTING"
        ));
        result.rows = vec![vec![Value::Null, json!("ANALYTICS_REPORTING")]];
        assert!(preflight_show_result_has_exact_name(
            &result,
            "analytics_reporting"
        ));
    }
}
