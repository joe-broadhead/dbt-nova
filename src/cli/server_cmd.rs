use std::io;
use std::sync::Arc;
use std::time::Duration;

use axum::{
    Json, Router,
    extract::State,
    http::{StatusCode, header},
    middleware,
    response::{IntoResponse, Response},
    routing::get,
};
use rmcp::transport::{
    StreamableHttpServerConfig, StreamableHttpService,
    streamable_http_server::session::local::LocalSessionManager,
};
use serde_json::json;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tower_http::limit::RequestBodyLimitLayer;
use tracing::{error, info, warn};

use crate::cli::args::{ServerStartArgs, ServerTransportArg};
use crate::config::{DbtNovaConfig, HostedAuthMode, ServerTransport};
use crate::error::{DbtNovaError, Result};
use crate::manifest::bootstrap::prepare_runtime_config;
use crate::manifest::search::ManifestSearchHandle;
use crate::server::correlation::correlate_http_request;
use crate::server::health::build_manifest_health_payload;
use crate::server::identity::{HostedIdentityVerifier, verify_hosted_identity_request};
use crate::server::mcp::DbtNovaServer;
use crate::utils::{ToolMetricsStore, sanitize_uri};

use super::prepare_storage;

#[derive(Debug, Clone)]
struct HttpServerSettings {
    host: String,
    port: u16,
    path: String,
    stateful_mode: bool,
    sse_keep_alive_secs: u64,
    sse_retry_secs: u64,
    max_body_bytes: usize,
    allowed_hosts: Vec<String>,
    metrics_enabled: bool,
    hosted_identity_verifier: Option<Arc<HostedIdentityVerifier>>,
}

#[derive(Clone)]
struct HostedProbeState {
    searcher: ManifestSearchHandle,
    metrics: Arc<ToolMetricsStore>,
    metrics_enabled: bool,
}

impl HttpServerSettings {
    fn from_config(config: &DbtNovaConfig) -> Result<Self> {
        Ok(Self {
            host: config.http_host.clone(),
            port: config.http_port,
            path: config.http_path.trim().to_string(),
            stateful_mode: config.http_stateful_mode,
            sse_keep_alive_secs: config.http_sse_keep_alive_secs,
            sse_retry_secs: config.http_sse_retry_secs,
            max_body_bytes: config.http_max_body_bytes,
            allowed_hosts: parse_http_allowed_hosts(&config.http_allowed_hosts),
            metrics_enabled: config.metrics_enabled,
            hosted_identity_verifier: HostedIdentityVerifier::from_config(&config.hosted_auth)?
                .map(Arc::new),
        })
    }
}

/// Starts dbt-nova server mode using environment-derived configuration.
///
/// # Errors
/// Returns an error when configuration validation, manifest loading, or server startup fails.
pub async fn start_from_env() -> Result<()> {
    start_with_config(DbtNovaConfig::from_env()).await
}

/// Starts dbt-nova server mode using environment-derived configuration plus CLI overrides.
///
/// # Errors
/// Returns an error when configuration validation, manifest loading, or server startup fails.
pub async fn start_from_args(args: &ServerStartArgs) -> Result<()> {
    start_with_config(build_start_config(args)).await
}

/// Starts dbt-nova server mode with an explicit configuration.
///
/// # Errors
/// Returns an error when storage prep, manifest loading, or MCP transport startup fails.
pub async fn start_with_config(config: DbtNovaConfig) -> Result<()> {
    let shutdown = CancellationToken::new();
    spawn_signal_shutdown(shutdown.clone());
    start_with_config_and_shutdown(config, shutdown).await
}

fn build_start_config(args: &ServerStartArgs) -> DbtNovaConfig {
    let explicit_env_http_host = std::env::var("DBT_NOVA_HTTP_HOST")
        .ok()
        .is_some_and(|value| !value.trim().is_empty());
    let explicit_env_http_port = std::env::var("DBT_NOVA_HTTP_PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok());
    apply_start_args(
        DbtNovaConfig::from_env(),
        args,
        explicit_env_http_host,
        explicit_env_http_port.is_some(),
        std::env::var("PORT")
            .ok()
            .and_then(|value| value.parse().ok()),
    )
}

fn apply_start_args(
    mut config: DbtNovaConfig,
    args: &ServerStartArgs,
    explicit_env_http_host: bool,
    explicit_env_http_port: bool,
    platform_port: Option<u16>,
) -> DbtNovaConfig {
    if let Some(transport) = args.transport {
        config.server_transport = match transport {
            ServerTransportArg::Stdio => ServerTransport::Stdio,
            ServerTransportArg::StreamableHttp => ServerTransport::StreamableHttp,
        };
    }
    if let Some(host) = args.http_host.as_ref()
        && !host.trim().is_empty()
    {
        config.http_host.clone_from(host);
    }
    if let Some(port) = args.http_port {
        config.http_port = port;
    }
    if let Some(path) = args.http_path.as_ref()
        && !path.trim().is_empty()
    {
        config.http_path = path.trim().to_string();
    }
    if let Some(stateful_mode) = args.http_stateful_mode {
        config.http_stateful_mode = stateful_mode;
    }
    let explicit_http_host = args
        .http_host
        .as_ref()
        .is_some_and(|host| !host.trim().is_empty())
        || explicit_env_http_host;
    let explicit_http_port = args.http_port.is_some() || explicit_env_http_port;
    config.apply_http_platform_port_fallback(explicit_http_host, explicit_http_port, platform_port);
    config
}

async fn start_with_config_and_shutdown(
    mut config: DbtNovaConfig,
    shutdown: CancellationToken,
) -> Result<()> {
    let _bootstrap_resolution = prepare_runtime_config(&mut config)?;
    info!("dbt-nova starting");
    if config.manifest_uri.trim().is_empty() {
        info!(manifest_path = %sanitize_uri(&config.manifest_path), "loading manifest");
    } else {
        info!(manifest_uri = %sanitize_uri(&config.manifest_uri), "loading manifest");
    }
    if let Ok(storage_base) = config.storage_instance_root_dir() {
        info!(storage_base = %storage_base.display(), "storage base");
    }

    prepare_storage(&config)?;
    if config.server_transport == ServerTransport::StreamableHttp {
        log_streamable_http_auth_posture(&config);
    }

    let transport = config.server_transport;
    let http_settings = if transport == ServerTransport::StreamableHttp {
        Some(HttpServerSettings::from_config(&config)?)
    } else {
        None
    };
    let exposed_tools = config.resolved_mcp_tool_names();
    let searcher = ManifestSearchHandle::spawn(config);
    let ready_handle = searcher.clone();
    tokio::spawn(async move {
        match ready_handle.wait_ready().await {
            Ok(loaded) => {
                info!(entity_count = loaded.entity_count(), "manifest loaded");
            }
            Err(err) => {
                error!(error = %err, "manifest load failed");
            }
        }
    });

    let server = DbtNovaServer::new_with_exposed_tools(searcher.clone(), &exposed_tools);
    match transport {
        ServerTransport::Stdio => serve_stdio(server, shutdown).await,
        ServerTransport::StreamableHttp => {
            serve_streamable_http(
                server,
                searcher,
                http_settings.expect("HTTP settings are initialized for streamable HTTP"),
                shutdown,
            )
            .await
        }
    }
}

fn log_streamable_http_auth_posture(config: &DbtNovaConfig) {
    if config.hosted_auth.mode != HostedAuthMode::Off {
        info!(
            http_host = %config.http_host,
            http_port = config.http_port,
            http_path = %config.http_path,
            auth_mode = config.hosted_auth.mode.as_str(),
            "streamable HTTP hosted identity verification enabled; keep authorization and proxy/network access controls outside Nova"
        );
        return;
    }
    if config.http_transport_binds_non_loopback() {
        warn!(
            http_host = %config.http_host,
            http_port = config.http_port,
            http_path = %config.http_path,
            "streamable HTTP transport has no built-in authentication; ensure an authenticating reverse proxy is enforcing access before exposing this endpoint"
        );
    } else {
        warn!(
            http_host = %config.http_host,
            http_port = config.http_port,
            http_path = %config.http_path,
            "streamable HTTP transport has no built-in authentication; keep this listener bound to loopback or place it behind an authenticating reverse proxy before exposure"
        );
    }
}

fn spawn_signal_shutdown(shutdown: CancellationToken) {
    tokio::spawn(async move {
        if let Err(err) = wait_for_shutdown_signal().await {
            error!(error = %err, "shutdown signal handler failed");
        }
        shutdown.cancel();
    });
}

async fn serve_stdio(server: DbtNovaServer, shutdown: CancellationToken) -> Result<()> {
    let transport = rmcp::transport::io::stdio();
    let running = rmcp::serve_server(server, transport)
        .await
        .map_err(|e| DbtNovaError::ServerError(e.to_string()))?;
    let cancel = running.cancellation_token();
    tokio::spawn(async move {
        shutdown.cancelled().await;
        cancel.cancel();
    });
    running
        .waiting()
        .await
        .map_err(|e| DbtNovaError::ServerError(e.to_string()))?;
    Ok(())
}

async fn serve_streamable_http(
    server: DbtNovaServer,
    searcher: ManifestSearchHandle,
    settings: HttpServerSettings,
    shutdown: CancellationToken,
) -> Result<()> {
    let mut transport_config = StreamableHttpServerConfig::default();
    transport_config.stateful_mode = settings.stateful_mode;
    transport_config.sse_keep_alive = secs_to_option(settings.sse_keep_alive_secs);
    transport_config.sse_retry = secs_to_option(settings.sse_retry_secs);
    transport_config.cancellation_token = shutdown.clone();
    for host in &settings.allowed_hosts {
        if !transport_config.allowed_hosts.contains(host) {
            transport_config.allowed_hosts.push(host.clone());
        }
    }

    let metrics = server.tool_metrics();
    let service: StreamableHttpService<DbtNovaServer, LocalSessionManager> =
        StreamableHttpService::new(
            move || Ok::<DbtNovaServer, io::Error>(server.clone()),
            std::sync::Arc::default(),
            transport_config,
        );

    let base_app = Router::new()
        .route("/healthz", get(http_liveness))
        .route("/readyz", get(http_readiness))
        .route("/metrics", get(http_metrics))
        .with_state(HostedProbeState {
            searcher,
            metrics,
            metrics_enabled: settings.metrics_enabled,
        });

    let app = if settings.path == "/" {
        base_app.fallback_service(service)
    } else {
        base_app.nest_service(settings.path.as_str(), service)
    };
    let app = if settings.max_body_bytes > 0 {
        app.layer(RequestBodyLimitLayer::new(settings.max_body_bytes))
    } else {
        app
    };
    let app = if let Some(verifier) = settings.hosted_identity_verifier {
        app.layer(middleware::from_fn_with_state(
            verifier,
            verify_hosted_identity_request,
        ))
    } else {
        app
    };
    let app = app.layer(middleware::from_fn(correlate_http_request));

    let bind_host = settings.host.clone();
    let bind_port = settings.port;
    let listener = TcpListener::bind((bind_host.as_str(), bind_port))
        .await
        .map_err(|error| {
            DbtNovaError::ServerError(format!(
                "HTTP bind failed on {bind_host}:{bind_port}: {error}"
            ))
        })?;
    let local_addr = listener
        .local_addr()
        .map_err(|error| DbtNovaError::ServerError(format!("HTTP local_addr failed: {error}")))?;
    info!(
        transport = "streamable_http",
        bind_addr = %local_addr,
        http_path = %settings.path,
        stateful_mode = settings.stateful_mode,
        max_body_bytes = settings.max_body_bytes,
        "dbt-nova HTTP server listening"
    );

    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            shutdown.cancelled_owned().await;
        })
        .await
        .map_err(|error| DbtNovaError::ServerError(format!("HTTP server failed: {error}")))?;
    Ok(())
}

async fn http_liveness() -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::OK,
        Json(json!({
            "status": "ok",
            "version": env!("CARGO_PKG_VERSION"),
        })),
    )
}

async fn http_readiness(
    State(state): State<HostedProbeState>,
) -> (StatusCode, Json<serde_json::Value>) {
    let snapshot = build_manifest_health_payload(&state.searcher).await;
    let status = if snapshot.ready_for_traffic {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (status, Json(snapshot.payload))
}

async fn http_metrics(State(state): State<HostedProbeState>) -> Response {
    if !state.metrics_enabled {
        return (
            StatusCode::NOT_FOUND,
            [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
            "metrics disabled\n".to_string(),
        )
            .into_response();
    }
    let snapshot = build_manifest_health_payload(&state.searcher).await;
    let body = state.metrics.prometheus_text(snapshot.ready_for_traffic);
    (
        StatusCode::OK,
        [(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        body,
    )
        .into_response()
}

fn secs_to_option(value: u64) -> Option<Duration> {
    if value == 0 {
        None
    } else {
        Some(Duration::from_secs(value))
    }
}

fn parse_http_allowed_hosts(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|host| !host.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

async fn wait_for_shutdown_signal() -> Result<()> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut term = signal(SignalKind::terminate())
            .map_err(|e| DbtNovaError::ServerError(e.to_string()))?;
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                info!("received SIGINT");
            }
            _ = term.recv() => {
                info!("received SIGTERM");
            }
        }
    }

    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c()
            .await
            .map_err(|e| DbtNovaError::ServerError(e.to_string()))?;
        info!("received SIGINT");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::{LazyLock, Mutex};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use reqwest::Client;
    use serde_json::{Value, json};
    use tempfile::TempDir;
    use tokio::net::TcpListener;
    use tokio_util::sync::CancellationToken;

    use super::{
        HttpServerSettings, apply_start_args, build_start_config, start_with_config_and_shutdown,
    };
    use crate::cli::args::{ServerStartArgs, ServerTransportArg};
    use crate::config::{
        DbtNovaConfig, HostedAuthMode, RuntimePreset, SearchConfig, ServerTransport,
    };
    use crate::server::identity::{encode_proxy_identity_for_tests, sign_proxy_identity_for_tests};
    use crate::tests::common::fixture_manifest_path_string;

    static ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    #[test]
    fn build_start_config_prefers_cli_overrides() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let vars = [
            ("DBT_NOVA_SERVER_TRANSPORT", Some("stdio")),
            ("DBT_NOVA_HTTP_HOST", Some("127.0.0.1")),
            ("DBT_NOVA_HTTP_PORT", Some("8080")),
            ("DBT_NOVA_HTTP_PATH", Some("/default")),
            ("DBT_NOVA_HTTP_STATEFUL_MODE", Some("true")),
        ];
        let previous = vars.map(|(key, _)| (key, std::env::var(key).ok()));
        for (key, value) in vars {
            match value {
                Some(value) => {
                    // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                    unsafe { std::env::set_var(key, value) };
                }
                None => {
                    // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                    unsafe { std::env::remove_var(key) };
                }
            }
        }

        let args = ServerStartArgs {
            transport: Some(ServerTransportArg::StreamableHttp),
            http_host: Some("0.0.0.0".to_string()),
            http_port: Some(9090),
            http_path: Some("/mcp".to_string()),
            http_stateful_mode: Some(false),
        };
        let config = build_start_config(&args);

        for (key, value) in previous {
            match value {
                Some(value) => {
                    // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                    unsafe { std::env::set_var(key, value) };
                }
                None => {
                    // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                    unsafe { std::env::remove_var(key) };
                }
            }
        }

        assert_eq!(config.server_transport, ServerTransport::StreamableHttp);
        assert_eq!(config.http_host, "0.0.0.0");
        assert_eq!(config.http_port, 9090);
        assert_eq!(config.http_path, "/mcp");
        assert!(!config.http_stateful_mode);
    }

    #[test]
    fn apply_start_args_applies_cli_overrides_after_runtime_preset() {
        let mut base = DbtNovaConfig::default();
        base.apply_runtime_preset(RuntimePreset::HostedDiscovery);
        let config = apply_start_args(
            base,
            &ServerStartArgs {
                transport: Some(ServerTransportArg::Stdio),
                http_host: Some("127.0.0.1".to_string()),
                http_port: Some(7777),
                http_path: Some("/agent".to_string()),
                http_stateful_mode: Some(false),
            },
            false,
            false,
            None,
        );

        assert_eq!(config.runtime_preset, RuntimePreset::HostedDiscovery);
        assert_eq!(config.server_transport, ServerTransport::Stdio);
        assert_eq!(config.http_host, "127.0.0.1");
        assert_eq!(config.http_port, 7777);
        assert_eq!(config.http_path, "/agent");
        assert!(!config.http_stateful_mode);
        assert!(
            config
                .parsed_tool_denylist()
                .contains(&"execute_sql".to_string())
        );
    }

    #[test]
    fn build_start_config_uses_platform_port_after_cli_transport_override() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let vars = [
            ("DBT_NOVA_SERVER_TRANSPORT", Some("stdio")),
            ("DBT_NOVA_HTTP_HOST", None),
            ("DBT_NOVA_HTTP_PORT", None),
            ("PORT", Some("9090")),
        ];
        let previous = vars.map(|(key, _)| (key, std::env::var(key).ok()));
        for (key, value) in vars {
            match value {
                Some(value) => {
                    // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                    unsafe { std::env::set_var(key, value) };
                }
                None => {
                    // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                    unsafe { std::env::remove_var(key) };
                }
            }
        }

        let config = build_start_config(&ServerStartArgs {
            transport: Some(ServerTransportArg::StreamableHttp),
            http_host: None,
            http_port: None,
            http_path: None,
            http_stateful_mode: None,
        });

        for (key, value) in previous {
            match value {
                Some(value) => {
                    // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                    unsafe { std::env::set_var(key, value) };
                }
                None => {
                    // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                    unsafe { std::env::remove_var(key) };
                }
            }
        }

        assert_eq!(config.server_transport, ServerTransport::StreamableHttp);
        assert_eq!(config.http_port, 9090);
        assert_eq!(config.http_host, "0.0.0.0");
    }

    #[test]
    fn build_start_config_uses_platform_host_fallback_with_explicit_http_port() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let vars = [
            ("DBT_NOVA_SERVER_TRANSPORT", Some("stdio")),
            ("DBT_NOVA_HTTP_HOST", None),
            ("DBT_NOVA_HTTP_PORT", None),
            ("PORT", Some("9090")),
        ];
        let previous = vars.map(|(key, _)| (key, std::env::var(key).ok()));
        for (key, value) in vars {
            match value {
                Some(value) => {
                    // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                    unsafe { std::env::set_var(key, value) };
                }
                None => {
                    // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                    unsafe { std::env::remove_var(key) };
                }
            }
        }

        let config = build_start_config(&ServerStartArgs {
            transport: Some(ServerTransportArg::StreamableHttp),
            http_host: None,
            http_port: Some(8080),
            http_path: None,
            http_stateful_mode: None,
        });

        for (key, value) in previous {
            match value {
                Some(value) => {
                    // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                    unsafe { std::env::set_var(key, value) };
                }
                None => {
                    // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                    unsafe { std::env::remove_var(key) };
                }
            }
        }

        assert_eq!(config.server_transport, ServerTransport::StreamableHttp);
        assert_eq!(config.http_port, 8080);
        assert_eq!(config.http_host, "0.0.0.0");
    }

    #[test]
    fn build_start_config_ignores_invalid_env_http_port_for_platform_fallback() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let vars = [
            ("DBT_NOVA_SERVER_TRANSPORT", Some("stdio")),
            ("DBT_NOVA_HTTP_HOST", None),
            ("DBT_NOVA_HTTP_PORT", Some("not-a-port")),
            ("PORT", Some("9090")),
        ];
        let previous = vars.map(|(key, _)| (key, std::env::var(key).ok()));
        for (key, value) in vars {
            match value {
                Some(value) => {
                    // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                    unsafe { std::env::set_var(key, value) };
                }
                None => {
                    // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                    unsafe { std::env::remove_var(key) };
                }
            }
        }

        let config = build_start_config(&ServerStartArgs {
            transport: Some(ServerTransportArg::StreamableHttp),
            http_host: None,
            http_port: None,
            http_path: None,
            http_stateful_mode: None,
        });

        for (key, value) in previous {
            match value {
                Some(value) => {
                    // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                    unsafe { std::env::set_var(key, value) };
                }
                None => {
                    // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                    unsafe { std::env::remove_var(key) };
                }
            }
        }

        assert_eq!(config.server_transport, ServerTransport::StreamableHttp);
        assert_eq!(config.http_port, 9090);
        assert_eq!(config.http_host, "0.0.0.0");
    }

    #[test]
    fn build_start_config_trims_http_path() {
        let config = build_start_config(&ServerStartArgs {
            transport: Some(ServerTransportArg::StreamableHttp),
            http_host: None,
            http_port: None,
            http_path: Some(" /mcp ".to_string()),
            http_stateful_mode: None,
        });

        assert_eq!(config.http_path, "/mcp");
    }

    #[test]
    fn http_settings_parse_additional_allowed_hosts() {
        let config = DbtNovaConfig {
            http_allowed_hosts: "nova.example.com, nova.example.com:443, ,localhost".to_string(),
            ..Default::default()
        };

        let settings = HttpServerSettings::from_config(&config).expect("settings");

        assert_eq!(settings.max_body_bytes, 16 * 1024 * 1024);
        assert_eq!(
            settings.allowed_hosts,
            vec![
                "nova.example.com".to_string(),
                "nova.example.com:443".to_string(),
                "localhost".to_string()
            ]
        );
    }

    fn http_test_config(temp_dir: &TempDir, manifest_path: String, port: u16) -> DbtNovaConfig {
        DbtNovaConfig {
            manifest_path,
            manifest_refresh_secs: 0,
            storage_dir: temp_dir
                .path()
                .join(".dbt-nova")
                .to_string_lossy()
                .to_string(),
            storage_instance_id: "http-test".to_string(),
            server_transport: ServerTransport::StreamableHttp,
            http_host: "127.0.0.1".to_string(),
            http_port: port,
            http_path: "/mcp".to_string(),
            search: SearchConfig {
                enable_vector_search: false,
                enable_sparse_search: false,
                enable_reranker: false,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    async fn next_test_port() -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let port = listener.local_addr().expect("local addr").port();
        drop(listener);
        port
    }

    async fn wait_for_http_status(
        client: &Client,
        url: &str,
        expected_status: reqwest::StatusCode,
        expected_body_status: Option<&str>,
    ) -> Value {
        // Hosted readiness can lag behind the listener coming up on slower CI runners.
        for _ in 0..120 {
            if let Ok(response) = client.get(url).send().await
                && response.status() == expected_status
            {
                let body: Value = response.json().await.expect("valid JSON probe response");
                if expected_body_status.is_none_or(|status| body["status"] == status) {
                    return body;
                }
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        match expected_body_status {
            Some(status) => {
                panic!(
                    "timed out waiting for {url} to return {expected_status} with status={status}"
                )
            }
            None => panic!("timed out waiting for {url} to return {expected_status}"),
        }
    }

    const TEST_PROXY_SECRET: &[u8] = b"0123456789abcdef0123456789abcdef";

    fn enable_proxy_identity(config: &mut DbtNovaConfig, temp_dir: &TempDir) {
        let secret_file = temp_dir.path().join("nova-proxy-identity-secret");
        std::fs::write(&secret_file, TEST_PROXY_SECRET).expect("write proxy identity secret");
        config.hosted_auth.mode = HostedAuthMode::ProxySignedHeaders;
        config.hosted_auth.required = true;
        config.hosted_auth.proxy_identity_header = "X-Nova-Identity".to_string();
        config.hosted_auth.proxy_signature_header = "X-Nova-Signature".to_string();
        config.hosted_auth.proxy_identity_secret_file = secret_file.display().to_string();
        config.hosted_auth.proxy_identity_max_age_secs = 300;
    }

    fn unix_now_secs_for_test() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock after epoch")
            .as_secs()
    }

    fn signed_proxy_identity_headers(subject: &str, iat: u64) -> (String, String) {
        let identity = encode_proxy_identity_for_tests(&json!({
            "sub": subject,
            "iat": iat,
        }));
        let signature = sign_proxy_identity_for_tests(TEST_PROXY_SECRET, &identity);
        (identity, signature)
    }

    #[tokio::test]
    async fn http_transport_serves_initialize_requests() {
        let temp_dir = TempDir::new().expect("temp dir");
        let port = next_test_port().await;
        let shutdown = CancellationToken::new();
        let config = http_test_config(&temp_dir, fixture_manifest_path_string(), port);

        let server_task = tokio::spawn(start_with_config_and_shutdown(config, shutdown.clone()));
        tokio::time::sleep(Duration::from_millis(300)).await;
        let client = Client::new();

        let liveness = client
            .get(format!("http://127.0.0.1:{port}/healthz"))
            .header("X-Request-ID", "req-test-001")
            .send()
            .await
            .expect("liveness request should succeed");
        assert_eq!(liveness.status(), reqwest::StatusCode::OK);
        assert_eq!(
            liveness
                .headers()
                .get("x-request-id")
                .and_then(|value| value.to_str().ok()),
            Some("req-test-001")
        );
        let liveness_body = liveness.json::<Value>().await.expect("liveness JSON");
        assert_eq!(liveness_body["status"], "ok");
        assert_eq!(liveness_body["version"], env!("CARGO_PKG_VERSION"));

        let readiness = wait_for_http_status(
            &client,
            &format!("http://127.0.0.1:{port}/readyz"),
            reqwest::StatusCode::OK,
            Some("ready"),
        )
        .await;
        assert_eq!(readiness["status"], "ready");
        assert!(
            readiness["entity_count"]
                .as_u64()
                .is_some_and(|value| value > 0),
            "unexpected readiness payload: {readiness}"
        );

        let response = client
            .post(format!("http://127.0.0.1:{port}/mcp"))
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .json(&json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2025-11-25",
                    "capabilities": {},
                    "clientInfo": {
                        "name": "http-test",
                        "version": "1.0"
                    }
                }
            }))
            .send()
            .await
            .expect("HTTP request should succeed");

        let status = response.status();
        let body = response.text().await.expect("body");

        shutdown.cancel();
        let join_result = server_task.await.expect("server task");
        assert!(
            join_result.is_ok(),
            "server should shut down cleanly: {join_result:?}"
        );

        assert!(status.is_success(), "unexpected status: {status}");
        assert!(
            body.contains(r#""jsonrpc":"2.0""#),
            "unexpected body: {body}"
        );
        assert!(body.contains(r#""id":1"#), "unexpected body: {body}");
    }

    #[tokio::test]
    async fn http_transport_accepts_signed_proxy_identity_when_enabled() {
        let temp_dir = TempDir::new().expect("temp dir");
        let port = next_test_port().await;
        let shutdown = CancellationToken::new();
        let mut config = http_test_config(&temp_dir, fixture_manifest_path_string(), port);
        enable_proxy_identity(&mut config, &temp_dir);
        let (identity, signature) =
            signed_proxy_identity_headers("user@example.com", unix_now_secs_for_test());

        let server_task = tokio::spawn(start_with_config_and_shutdown(config, shutdown.clone()));
        tokio::time::sleep(Duration::from_millis(300)).await;
        let client = Client::new();

        let response = client
            .get(format!("http://127.0.0.1:{port}/healthz"))
            .header("X-Nova-Identity", identity)
            .header("X-Nova-Signature", signature)
            .send()
            .await
            .expect("liveness request should succeed");
        let status = response.status();
        let body = response.text().await.expect("liveness body");

        shutdown.cancel();
        let join_result = server_task.await.expect("server task");
        assert!(
            join_result.is_ok(),
            "server should shut down cleanly: {join_result:?}"
        );

        assert_eq!(status, reqwest::StatusCode::OK, "{body}");
        assert!(!body.contains("user@example.com"));
    }

    #[tokio::test]
    async fn http_transport_rejects_missing_proxy_identity_when_enabled() {
        let temp_dir = TempDir::new().expect("temp dir");
        let port = next_test_port().await;
        let shutdown = CancellationToken::new();
        let mut config = http_test_config(&temp_dir, fixture_manifest_path_string(), port);
        enable_proxy_identity(&mut config, &temp_dir);

        let server_task = tokio::spawn(start_with_config_and_shutdown(config, shutdown.clone()));
        tokio::time::sleep(Duration::from_millis(300)).await;
        let client = Client::new();

        let response = client
            .get(format!("http://127.0.0.1:{port}/healthz"))
            .send()
            .await
            .expect("liveness request should succeed");
        let status = response.status();
        let body = response.text().await.expect("auth error body");

        shutdown.cancel();
        let join_result = server_task.await.expect("server task");
        assert!(
            join_result.is_ok(),
            "server should shut down cleanly: {join_result:?}"
        );

        assert_eq!(status, reqwest::StatusCode::UNAUTHORIZED);
        assert!(body.contains("UNAUTHORIZED"));
        assert!(!body.contains("secret"));
    }

    #[tokio::test]
    async fn http_transport_rejects_invalid_proxy_identity_signature() {
        let temp_dir = TempDir::new().expect("temp dir");
        let port = next_test_port().await;
        let shutdown = CancellationToken::new();
        let mut config = http_test_config(&temp_dir, fixture_manifest_path_string(), port);
        enable_proxy_identity(&mut config, &temp_dir);
        let identity = encode_proxy_identity_for_tests(&json!({
            "sub": "user@example.com",
            "iat": unix_now_secs_for_test(),
        }));
        let signature =
            sign_proxy_identity_for_tests(b"abcdef0123456789abcdef0123456789", &identity);

        let server_task = tokio::spawn(start_with_config_and_shutdown(config, shutdown.clone()));
        tokio::time::sleep(Duration::from_millis(300)).await;
        let client = Client::new();

        let response = client
            .get(format!("http://127.0.0.1:{port}/healthz"))
            .header("X-Nova-Identity", identity)
            .header("X-Nova-Signature", signature)
            .send()
            .await
            .expect("liveness request should succeed");
        let status = response.status();
        let body = response.text().await.expect("auth error body");

        shutdown.cancel();
        let join_result = server_task.await.expect("server task");
        assert!(
            join_result.is_ok(),
            "server should shut down cleanly: {join_result:?}"
        );

        assert_eq!(status, reqwest::StatusCode::UNAUTHORIZED);
        assert!(body.contains("hosted identity verification failed"));
        assert!(!body.contains("user@example.com"));
    }

    #[tokio::test]
    async fn http_transport_rejects_stale_proxy_identity() {
        let temp_dir = TempDir::new().expect("temp dir");
        let port = next_test_port().await;
        let shutdown = CancellationToken::new();
        let mut config = http_test_config(&temp_dir, fixture_manifest_path_string(), port);
        enable_proxy_identity(&mut config, &temp_dir);
        config.hosted_auth.proxy_identity_max_age_secs = 1;
        let stale_iat = unix_now_secs_for_test().saturating_sub(10);
        let (identity, signature) = signed_proxy_identity_headers("user@example.com", stale_iat);

        let server_task = tokio::spawn(start_with_config_and_shutdown(config, shutdown.clone()));
        tokio::time::sleep(Duration::from_millis(300)).await;
        let client = Client::new();

        let response = client
            .get(format!("http://127.0.0.1:{port}/healthz"))
            .header("X-Nova-Identity", identity)
            .header("X-Nova-Signature", signature)
            .send()
            .await
            .expect("liveness request should succeed");
        let status = response.status();
        let body = response.text().await.expect("auth error body");

        shutdown.cancel();
        let join_result = server_task.await.expect("server task");
        assert!(
            join_result.is_ok(),
            "server should shut down cleanly: {join_result:?}"
        );

        assert_eq!(status, reqwest::StatusCode::UNAUTHORIZED);
        assert!(body.contains("hosted identity verification failed"));
        assert!(!body.contains("user@example.com"));
    }

    #[tokio::test]
    async fn http_transport_serves_prometheus_metrics() {
        let temp_dir = TempDir::new().expect("temp dir");
        let port = next_test_port().await;
        let shutdown = CancellationToken::new();
        let config = http_test_config(&temp_dir, fixture_manifest_path_string(), port);

        let server_task = tokio::spawn(start_with_config_and_shutdown(config, shutdown.clone()));
        let client = Client::new();
        let _readiness = wait_for_http_status(
            &client,
            &format!("http://127.0.0.1:{port}/readyz"),
            reqwest::StatusCode::OK,
            Some("ready"),
        )
        .await;

        let response = client
            .get(format!("http://127.0.0.1:{port}/metrics"))
            .send()
            .await
            .expect("metrics request should succeed");
        let status = response.status();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string();
        let body = response.text().await.expect("metrics body");

        shutdown.cancel();
        let join_result = server_task.await.expect("server task");
        assert!(
            join_result.is_ok(),
            "server should shut down cleanly: {join_result:?}"
        );

        assert_eq!(status, reqwest::StatusCode::OK);
        assert!(
            content_type.starts_with("text/plain"),
            "unexpected content-type: {content_type}"
        );
        assert!(
            body.contains("# TYPE nova_manifest_ready_for_traffic gauge"),
            "{body}"
        );
        assert!(body.contains("nova_manifest_ready_for_traffic 1"), "{body}");
        assert!(
            body.contains("# TYPE nova_tool_calls_total counter"),
            "{body}"
        );
        assert!(
            body.contains("# TYPE nova_tool_call_duration_milliseconds histogram"),
            "{body}"
        );
    }

    #[tokio::test]
    async fn http_transport_metrics_endpoint_can_be_disabled() {
        let temp_dir = TempDir::new().expect("temp dir");
        let port = next_test_port().await;
        let shutdown = CancellationToken::new();
        let mut config = http_test_config(&temp_dir, fixture_manifest_path_string(), port);
        config.metrics_enabled = false;

        let server_task = tokio::spawn(start_with_config_and_shutdown(config, shutdown.clone()));
        let client = Client::new();
        let _liveness = wait_for_http_status(
            &client,
            &format!("http://127.0.0.1:{port}/healthz"),
            reqwest::StatusCode::OK,
            Some("ok"),
        )
        .await;

        let response = client
            .get(format!("http://127.0.0.1:{port}/metrics"))
            .send()
            .await
            .expect("metrics request should succeed");
        let status = response.status();
        let body = response.text().await.expect("metrics disabled body");

        shutdown.cancel();
        let join_result = server_task.await.expect("server task");
        assert!(
            join_result.is_ok(),
            "server should shut down cleanly: {join_result:?}"
        );

        assert_eq!(status, reqwest::StatusCode::NOT_FOUND);
        assert_eq!(body, "metrics disabled\n");
    }

    #[tokio::test]
    async fn http_transport_rejects_oversized_request_body() {
        let temp_dir = TempDir::new().expect("temp dir");
        let port = next_test_port().await;
        let shutdown = CancellationToken::new();
        let mut config = http_test_config(&temp_dir, fixture_manifest_path_string(), port);
        config.http_max_body_bytes = 32;

        let server_task = tokio::spawn(start_with_config_and_shutdown(config, shutdown.clone()));
        let client = Client::new();
        let _liveness = wait_for_http_status(
            &client,
            &format!("http://127.0.0.1:{port}/healthz"),
            reqwest::StatusCode::OK,
            Some("ok"),
        )
        .await;

        let response = client
            .post(format!("http://127.0.0.1:{port}/mcp"))
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .body(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#)
            .send()
            .await
            .expect("HTTP request should succeed");

        shutdown.cancel();
        let join_result = server_task.await.expect("server task");
        assert!(
            join_result.is_ok(),
            "server should shut down cleanly: {join_result:?}"
        );

        assert_eq!(response.status(), reqwest::StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn http_transport_readyz_reports_failed_state() {
        let temp_dir = TempDir::new().expect("temp dir");
        let port = next_test_port().await;
        let shutdown = CancellationToken::new();
        let config = http_test_config(
            &temp_dir,
            temp_dir
                .path()
                .join("missing-manifest.json")
                .display()
                .to_string(),
            port,
        );

        let server_task = tokio::spawn(start_with_config_and_shutdown(config, shutdown.clone()));
        let client = Client::new();
        let readiness = wait_for_http_status(
            &client,
            &format!("http://127.0.0.1:{port}/readyz"),
            reqwest::StatusCode::SERVICE_UNAVAILABLE,
            Some("failed"),
        )
        .await;

        shutdown.cancel();
        let join_result = server_task.await.expect("server task");
        assert!(
            join_result.is_ok(),
            "server should shut down cleanly: {join_result:?}"
        );

        assert_eq!(readiness["status"], "failed");
        assert!(
            readiness["error"]
                .as_str()
                .is_some_and(|value| value.contains("missing-manifest.json")),
            "unexpected readiness payload: {readiness}"
        );
    }

    #[tokio::test]
    async fn server_start_rejects_invalid_tool_filter_names() {
        let temp_dir = TempDir::new().expect("temp dir");
        let shutdown = CancellationToken::new();
        let mut config = http_test_config(&temp_dir, fixture_manifest_path_string(), 0);
        config.tool_allowlist = "search,unknown_tool".to_string();

        let error = start_with_config_and_shutdown(config, shutdown)
            .await
            .expect_err("invalid tool filter should fail startup");

        assert!(error.to_string().contains("DBT_NOVA_TOOL_ALLOWLIST"));
        assert!(error.to_string().contains("unknown_tool"));
    }
}
