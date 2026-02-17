use std::fs;

use dbt_nova::error::{DbtNovaError, Result};
use dbt_nova::{
    ManifestSearchHandle,
    config::DbtNovaConfig,
    server::mcp::DbtNovaServer,
    utils::{dir_in_use, prune_dirs, sanitize_uri},
};
use tracing::{error, info, warn};
use tracing_subscriber::fmt::format::FmtSpan;

fn cleanup_storage_dir(config: &DbtNovaConfig) -> Result<()> {
    let instance_root = config.storage_instance_root_dir()?;
    if instance_root.exists() {
        if dir_in_use(&instance_root) {
            warn!(
                storage_base = %instance_root.display(),
                "storage directory in use; skipping cleanup"
            );
            return Ok(());
        }
        fs::remove_dir_all(&instance_root)
            .map_err(|e| DbtNovaError::ServerError(format!("Cleanup failed: {e}")))?;
    }

    Ok(())
}

fn prune_storage_instances(
    config: &DbtNovaConfig,
    max_keep: usize,
    exclude_instance: Option<&str>,
) -> Result<()> {
    let storage_root = config.storage_instances_dir()?;
    let mut exclude = Vec::new();
    if let Some(instance) = exclude_instance {
        exclude.push(instance);
    }
    prune_dirs(
        &storage_root,
        max_keep,
        0,
        config.storage_max_bytes,
        &exclude,
    )
}

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    // Load configuration from environment (falls back to defaults).
    let mut config = DbtNovaConfig::from_env();
    config.ensure_storage_instance_id();
    config.validate()?;

    info!("dbt-nova starting");
    if config.manifest_uri.trim().is_empty() {
        info!(manifest_path = %sanitize_uri(&config.manifest_path), "loading manifest");
    } else {
        info!(manifest_uri = %sanitize_uri(&config.manifest_uri), "loading manifest");
    }
    if let Ok(storage_base) = config.storage_instance_root_dir() {
        info!(storage_base = %storage_base.display(), "storage base");
    }

    // Clean any previous on-disk state before building fresh indexes.
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

    // Build in-memory search indexes from the manifest in the background.
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

    // Start MCP server over stdio.
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

fn init_tracing() {
    let filter = std::env::var("DBT_NOVA_LOG")
        .or_else(|_| std::env::var("RUST_LOG"))
        .ok();
    if let Some(filter) = filter
        && let Err(err) = tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(std::io::stderr)
            .with_target(false)
            .with_span_events(FmtSpan::CLOSE)
            .try_init()
    {
        tracing::warn!(error = %err, "failed to initialize tracing subscriber");
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
    use std::fs;
    use std::path::Path;

    use tempfile::TempDir;

    use super::{cleanup_storage_dir, prune_storage_instances};
    use dbt_nova::config::DbtNovaConfig;

    fn fixture_manifest_path() -> String {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("nova_manifest.json")
            .to_string_lossy()
            .to_string()
    }

    fn test_config(storage_root: &Path, instance_id: &str) -> DbtNovaConfig {
        DbtNovaConfig {
            manifest_path: fixture_manifest_path(),
            manifest_refresh_secs: 0,
            storage_dir: storage_root.join(".dbt-nova").to_string_lossy().to_string(),
            storage_instance_id: instance_id.to_string(),
            storage_max_bytes: 1,
            ..Default::default()
        }
    }

    #[test]
    fn cleanup_storage_dir_removes_instance_root() {
        let temp_dir = TempDir::new().expect("temp dir");
        let config = test_config(temp_dir.path(), "active");
        let instance_root = config
            .storage_instance_root_dir()
            .expect("instance root path");
        fs::create_dir_all(&instance_root).expect("create instance root");
        fs::write(instance_root.join("payload.txt"), b"stale").expect("write payload");

        cleanup_storage_dir(&config).expect("cleanup succeeds");
        assert!(!instance_root.exists());
    }

    #[test]
    fn prune_storage_instances_preserves_excluded_instance() {
        let temp_dir = TempDir::new().expect("temp dir");
        let config = test_config(temp_dir.path(), "active");
        let instances_dir = config.storage_instances_dir().expect("instances dir");
        let active_dir = instances_dir.join("active");
        let stale_dir = instances_dir.join("stale");
        fs::create_dir_all(&active_dir).expect("create active dir");
        fs::create_dir_all(&stale_dir).expect("create stale dir");
        fs::write(stale_dir.join("payload.bin"), vec![1_u8; 32]).expect("write stale payload");

        prune_storage_instances(&config, 0, Some("active")).expect("prune succeeds");
        assert!(active_dir.exists());
        assert!(!stale_dir.exists());
    }
}
