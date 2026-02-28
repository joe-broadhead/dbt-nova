use std::fs;

use crate::config::DbtNovaConfig;
use crate::error::{DbtNovaError, Result};
use crate::utils::{dir_in_use, prune_dirs};

pub mod args;
pub mod output;
pub mod server_cmd;

/// Dispatches parsed CLI commands to their handlers.
///
/// # Errors
/// Returns an error when the selected command fails validation or execution.
pub async fn dispatch(command: args::Command) -> Result<()> {
    match command {
        args::Command::Server(server) => match server.command {
            args::ServerCommand::Start => server_cmd::start_from_env().await,
        },
        args::Command::Manifest(_)
        | args::Command::Tool(_)
        | args::Command::Config(_)
        | args::Command::Storage(_)
        | args::Command::Health(_) => Err(DbtNovaError::InvalidParams(
            "CLI command group is not implemented yet in issue #40 scope".to_string(),
        )),
    }
}

/// Removes the configured storage instance directory when it is not in use.
///
/// # Errors
/// Returns an error when the instance path cannot be resolved or removal fails.
pub fn cleanup_storage_dir(config: &DbtNovaConfig) -> Result<()> {
    let instance_root = config.storage_instance_root_dir()?;
    if instance_root.exists() {
        if dir_in_use(&instance_root) {
            tracing::warn!(
                storage_base = %instance_root.display(),
                "storage directory in use; skipping cleanup"
            );
            return Ok(());
        }
        fs::remove_dir_all(&instance_root)
            .map_err(|error| DbtNovaError::ServerError(format!("Cleanup failed: {error}")))?;
    }

    Ok(())
}

/// Prunes storage instance directories based on retention policy.
///
/// # Errors
/// Returns an error when storage paths cannot be resolved or pruning fails.
pub fn prune_storage_instances(
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

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use tempfile::TempDir;

    use crate::config::DbtNovaConfig;
    use crate::error::DbtNovaError;

    use super::{cleanup_storage_dir, dispatch, prune_storage_instances};

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

    #[tokio::test]
    async fn dispatch_non_server_commands_return_invalid_params() {
        let result = dispatch(super::args::Command::Manifest(super::args::ManifestArgs {
            command: super::args::ManifestCommand::Load,
        }))
        .await;

        match result {
            Err(DbtNovaError::InvalidParams(message)) => {
                assert!(message.contains("issue #40"));
            }
            _ => panic!("expected invalid params error"),
        }
    }
}
