use std::io;
use std::time::Duration;

use axum::Router;
use rmcp::transport::{
    StreamableHttpServerConfig, StreamableHttpService,
    streamable_http_server::session::local::LocalSessionManager,
};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

use crate::cli::args::{ServerStartArgs, ServerTransportArg};
use crate::config::{DbtNovaConfig, ServerTransport};
use crate::error::{DbtNovaError, Result};
use crate::manifest::bootstrap::prepare_runtime_config;
use crate::manifest::search::ManifestSearchHandle;
use crate::server::mcp::DbtNovaServer;
use crate::utils::sanitize_uri;

use super::prepare_storage;

#[derive(Debug, Clone)]
struct HttpServerSettings {
    host: String,
    port: u16,
    path: String,
    stateful_mode: bool,
    sse_keep_alive_secs: u64,
    sse_retry_secs: u64,
}

impl HttpServerSettings {
    fn from_config(config: &DbtNovaConfig) -> Self {
        Self {
            host: config.http_host.clone(),
            port: config.http_port,
            path: config.http_path.clone(),
            stateful_mode: config.http_stateful_mode,
            sse_keep_alive_secs: config.http_sse_keep_alive_secs,
            sse_retry_secs: config.http_sse_retry_secs,
        }
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
    let mut config = DbtNovaConfig::from_env();
    let explicit_env_http_host = std::env::var("DBT_NOVA_HTTP_HOST")
        .ok()
        .is_some_and(|value| !value.trim().is_empty());
    let explicit_env_http_port = std::env::var("DBT_NOVA_HTTP_PORT").is_ok();
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
        config.http_path.clone_from(path);
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
    config.apply_http_platform_port_fallback(
        explicit_http_host,
        explicit_http_port,
        std::env::var("PORT")
            .ok()
            .and_then(|value| value.parse().ok()),
    );
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

    let transport = config.server_transport;
    let http_settings = HttpServerSettings::from_config(&config);
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

    let server = DbtNovaServer::new(searcher);
    match transport {
        ServerTransport::Stdio => serve_stdio(server, shutdown).await,
        ServerTransport::StreamableHttp => {
            serve_streamable_http(server, http_settings, shutdown).await
        }
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
    settings: HttpServerSettings,
    shutdown: CancellationToken,
) -> Result<()> {
    let service: StreamableHttpService<DbtNovaServer, LocalSessionManager> =
        StreamableHttpService::new(
            move || Ok::<DbtNovaServer, io::Error>(server.clone()),
            std::sync::Arc::default(),
            StreamableHttpServerConfig {
                stateful_mode: settings.stateful_mode,
                sse_keep_alive: secs_to_option(settings.sse_keep_alive_secs),
                sse_retry: secs_to_option(settings.sse_retry_secs),
                cancellation_token: shutdown.clone(),
            },
        );

    let app = if settings.path == "/" {
        Router::new().fallback_service(service)
    } else {
        Router::new().nest_service(settings.path.as_str(), service)
    };

    let bind_addr = format!("{}:{}", settings.host, settings.port);
    let listener = TcpListener::bind(bind_addr.as_str())
        .await
        .map_err(|error| {
            DbtNovaError::ServerError(format!("HTTP bind failed on {bind_addr}: {error}"))
        })?;
    let local_addr = listener
        .local_addr()
        .map_err(|error| DbtNovaError::ServerError(format!("HTTP local_addr failed: {error}")))?;
    info!(
        transport = "streamable_http",
        bind_addr = %local_addr,
        http_path = %settings.path,
        stateful_mode = settings.stateful_mode,
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

fn secs_to_option(value: u64) -> Option<Duration> {
    if value == 0 {
        None
    } else {
        Some(Duration::from_secs(value))
    }
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
    use std::time::Duration;

    use reqwest::Client;
    use serde_json::json;
    use tempfile::TempDir;
    use tokio::net::TcpListener;
    use tokio_util::sync::CancellationToken;

    use super::{build_start_config, start_with_config_and_shutdown};
    use crate::cli::args::{ServerStartArgs, ServerTransportArg};
    use crate::config::{DbtNovaConfig, SearchConfig, ServerTransport};
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

    #[tokio::test]
    async fn http_transport_serves_initialize_requests() {
        let temp_dir = TempDir::new().expect("temp dir");
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let port = listener.local_addr().expect("local addr").port();
        drop(listener);

        let shutdown = CancellationToken::new();
        let config = DbtNovaConfig {
            manifest_path: fixture_manifest_path_string(),
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
        };

        let server_task = tokio::spawn(start_with_config_and_shutdown(config, shutdown.clone()));
        tokio::time::sleep(Duration::from_millis(300)).await;

        let response = Client::new()
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
}
