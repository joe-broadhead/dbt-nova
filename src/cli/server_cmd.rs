use crate::config::DbtNovaConfig;
use crate::error::{DbtNovaError, Result};
use crate::manifest::search::ManifestSearchHandle;
use crate::server::mcp::DbtNovaServer;
use crate::utils::sanitize_uri;
use tracing::{error, info};

use super::{cleanup_storage_dir, prune_storage_instances};

/// Starts dbt-nova server mode using environment-derived configuration.
///
/// # Errors
/// Returns an error when configuration validation, manifest loading, or server startup fails.
pub async fn start_from_env() -> Result<()> {
    let mut config = DbtNovaConfig::from_env();
    config.ensure_storage_instance_id();
    config.validate()?;
    start_with_config(config).await
}

/// Starts dbt-nova server mode with an explicit configuration.
///
/// # Errors
/// Returns an error when storage prep, manifest loading, or MCP transport startup fails.
pub async fn start_with_config(config: DbtNovaConfig) -> Result<()> {
    info!("dbt-nova starting");
    if config.manifest_uri.trim().is_empty() {
        info!(manifest_path = %sanitize_uri(&config.manifest_path), "loading manifest");
    } else {
        info!(manifest_uri = %sanitize_uri(&config.manifest_uri), "loading manifest");
    }
    if let Ok(storage_base) = config.storage_instance_root_dir() {
        info!(storage_base = %storage_base.display(), "storage base");
    }

    if config.cleanup_storage_on_start {
        cleanup_storage_dir(&config)?;
        if config.storage_max_instances > 0 {
            let max_keep = config.storage_max_instances.saturating_sub(1);
            prune_storage_instances(&config, max_keep, None)?;
        }
    } else if config.storage_max_instances > 0 {
        let max_keep = config.storage_max_instances.saturating_sub(1);
        prune_storage_instances(&config, max_keep, Some(config.storage_instance_id.as_str()))?;
    }

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
    let transport = rmcp::transport::io::stdio();
    let running = rmcp::serve_server(server, transport)
        .await
        .map_err(|e| DbtNovaError::ServerError(e.to_string()))?;
    let cancel = running.cancellation_token();
    tokio::spawn(async move {
        if let Err(err) = wait_for_shutdown_signal().await {
            error!(error = %err, "shutdown signal handler failed");
        }
        cancel.cancel();
    });
    running
        .waiting()
        .await
        .map_err(|e| DbtNovaError::ServerError(e.to_string()))?;
    Ok(())
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
