use std::fs;

use crate::config::DbtNovaConfig;
use crate::error::{DbtNovaError, Result};
use crate::utils::{dir_in_use, prune_dirs};

pub mod agent_readiness_cmd;
pub mod args;
pub mod audit_cmd;
pub mod config_cmd;
pub mod eval_cmd;
pub mod health_cmd;
pub mod manifest;
pub mod nova_meta_cmd;
pub mod output;
pub mod server_cmd;
pub mod storage_cmd;
pub mod tool;
mod tool_param_aliases;
pub mod trace_cmd;

pub struct DispatchError {
    pub error: DbtNovaError,
    pub rendered: bool,
}

impl From<DbtNovaError> for DispatchError {
    fn from(error: DbtNovaError) -> Self {
        Self {
            error,
            rendered: false,
        }
    }
}

pub type DispatchResult = std::result::Result<(), DispatchError>;

/// Dispatches parsed CLI commands to their handlers.
///
/// # Errors
/// Returns an error when the selected command fails validation or execution.
pub async fn dispatch(command: args::Command) -> DispatchResult {
    match command {
        args::Command::Server(server) => match server.command {
            args::ServerCommand::Start(args) => {
                server_cmd::start_from_args(&args).await.map_err(Into::into)
            }
        },
        args::Command::Manifest(manifest_args) => match manifest_args.command {
            args::ManifestCommand::Load(load_args) => manifest::run_load_command(&load_args).await,
            args::ManifestCommand::Reload(reload_args) => {
                manifest::run_reload_command(&reload_args).await
            }
            args::ManifestCommand::Warm(warm_args) => manifest::run_warm_command(&warm_args).await,
        },
        args::Command::Tool(tool_args) => match tool_args.command {
            args::ToolCommand::Call(call_args) => tool::run_call_command(&call_args).await,
        },
        args::Command::Audit(audit_args) => match audit_args.command {
            args::AuditCommand::AgentReadiness(readiness_args) => {
                agent_readiness_cmd::run_agent_readiness_command(&readiness_args).await
            }
            args::AuditCommand::MetadataScore(metadata_args) => {
                audit_cmd::run_metadata_score_command(&metadata_args).await
            }
            args::AuditCommand::NovaMeta(nova_meta_args) => {
                nova_meta_cmd::run_nova_meta_command(&nova_meta_args)
            }
        },
        args::Command::Config(config_args) => match config_args.command {
            args::ConfigCommand::Show(show_args) => config_cmd::run_show_command(&show_args),
            args::ConfigCommand::Validate(validate_args) => {
                config_cmd::run_validate_command(&validate_args)
            }
        },
        args::Command::Storage(storage_args) => match storage_args.command {
            args::StorageCommand::Inspect(inspect_args) => {
                storage_cmd::run_inspect_command(&inspect_args)
            }
            args::StorageCommand::Prune(prune_args) => storage_cmd::run_prune_command(&prune_args),
            args::StorageCommand::Cleanup(cleanup_args) => {
                storage_cmd::run_cleanup_command(&cleanup_args)
            }
        },
        args::Command::Trace(trace_args) => match trace_args.command {
            args::TraceCommand::Inspect(inspect_args) => {
                trace_cmd::run_inspect_command(&inspect_args)
            }
            args::TraceCommand::Summarize(summarize_args) => {
                trace_cmd::run_summarize_command(&summarize_args)
            }
            args::TraceCommand::Redact(redact_args) => trace_cmd::run_redact_command(&redact_args),
            args::TraceCommand::Replay(replay_args) => {
                trace_cmd::run_replay_command(&replay_args).await
            }
        },
        args::Command::Health(health_args) => match health_args.command {
            args::HealthCommand::Check(check_args) => {
                health_cmd::run_check_command(&check_args).await
            }
        },
        args::Command::Eval(eval_args) => match eval_args.command {
            args::EvalCommand::Init(init_args) => eval_cmd::run_init_command(&init_args),
            args::EvalCommand::Run(run_args) => eval_cmd::run_eval_command(&run_args).await,
            args::EvalCommand::Agent(agent_args) => match agent_args.command {
                args::EvalAgentCommand::Run(run_args) => {
                    eval_cmd::run_agent_eval_command(&run_args).await
                }
            },
            args::EvalCommand::Compare(compare_args) => {
                eval_cmd::run_compare_command(&compare_args)
            }
            args::EvalCommand::Gate(gate_args) => eval_cmd::run_gate_command(&gate_args),
            args::EvalCommand::History(history_args) => {
                eval_cmd::run_history_command(&history_args)
            }
            args::EvalCommand::Validate(validate_args) => {
                eval_cmd::run_validate_command(&validate_args)
            }
        },
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
    if max_keep == 0 {
        return prune_all_stale_instances(&storage_root, &exclude);
    }
    prune_dirs(
        &storage_root,
        max_keep,
        0,
        config.storage_max_bytes,
        &exclude,
    )
}

fn prune_all_stale_instances(storage_root: &std::path::Path, exclude: &[&str]) -> Result<()> {
    if !storage_root.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(storage_root)
        .map_err(|error| DbtNovaError::ServerError(format!("Storage scan failed: {error}")))?
    {
        let entry = entry
            .map_err(|error| DbtNovaError::ServerError(format!("Storage scan failed: {error}")))?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if exclude.contains(&name) || dir_in_use(&path) {
            continue;
        }
        fs::remove_dir_all(&path)
            .map_err(|error| DbtNovaError::ServerError(format!("Storage prune failed: {error}")))?;
    }
    Ok(())
}

/// Prepares storage directories before loading/building manifest indexes.
///
/// # Errors
/// Returns an error if cleanup or pruning fails.
pub fn prepare_storage(config: &DbtNovaConfig) -> Result<()> {
    if config.cleanup_storage_on_start {
        cleanup_storage_dir(config)?;
        if config.storage_max_instances > 0 {
            let max_keep = config.storage_max_instances.saturating_sub(1);
            prune_storage_instances(config, max_keep, None)?;
        }
    } else if config.storage_max_instances > 0 {
        let max_keep = config.storage_max_instances.saturating_sub(1);
        prune_storage_instances(config, max_keep, Some(config.storage_instance_id.as_str()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use tempfile::TempDir;

    use super::{cleanup_storage_dir, dispatch, prepare_storage, prune_storage_instances};
    use crate::config::DbtNovaConfig;
    use crate::tests::common::fixture_manifest_path_string;

    fn test_config(storage_root: &Path, instance_id: &str) -> DbtNovaConfig {
        DbtNovaConfig {
            manifest_path: fixture_manifest_path_string(),
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

    #[test]
    fn prune_storage_instances_zero_max_keep_removes_stale_without_byte_limit() {
        let temp_dir = TempDir::new().expect("temp dir");
        let mut config = test_config(temp_dir.path(), "active");
        config.storage_max_bytes = 0;
        let instances_dir = config.storage_instances_dir().expect("instances dir");
        let active_dir = instances_dir.join("active");
        let old_orders_dir = instances_dir.join("old-orders");
        let old_users_dir = instances_dir.join("old-users");
        fs::create_dir_all(&active_dir).expect("create active dir");
        fs::create_dir_all(&old_orders_dir).expect("create old-orders dir");
        fs::create_dir_all(&old_users_dir).expect("create old-users dir");

        prune_storage_instances(&config, 0, Some("active")).expect("prune succeeds");
        assert!(active_dir.exists());
        assert!(!old_orders_dir.exists());
        assert!(!old_users_dir.exists());
    }

    #[test]
    fn prune_storage_instances_zero_max_keep_missing_root_is_noop() {
        let temp_dir = TempDir::new().expect("temp dir");
        let mut config = test_config(temp_dir.path(), "active");
        config.storage_max_bytes = 0;

        prune_storage_instances(&config, 0, Some("active")).expect("missing root should be noop");
    }

    #[tokio::test]
    async fn dispatch_manifest_reload_succeeds_with_fixture_path() {
        let result = dispatch(super::args::Command::Manifest(super::args::ManifestArgs {
            command: super::args::ManifestCommand::Reload(super::args::ManifestReloadArgs {
                manifest_path: Some(fixture_manifest_path_string()),
                json: true,
                ..super::args::ManifestReloadArgs::default()
            }),
        }))
        .await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn dispatch_config_show_defaults_succeeds() {
        let result = dispatch(super::args::Command::Config(super::args::ConfigArgs {
            command: super::args::ConfigCommand::Show(super::args::ConfigShowArgs {
                defaults: true,
                json: true,
            }),
        }))
        .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn dispatch_storage_inspect_succeeds() {
        let result = dispatch(super::args::Command::Storage(super::args::StorageArgs {
            command: super::args::StorageCommand::Inspect(super::args::StorageInspectArgs {
                storage_instance_id: None,
                json: true,
            }),
        }))
        .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn dispatch_audit_nova_meta_succeeds_for_valid_fixture() {
        let temp_dir = TempDir::new().expect("temp dir");
        let models_dir = temp_dir.path().join("models");
        fs::create_dir_all(&models_dir).expect("create models dir");
        fs::write(
            models_dir.join("orders.yml"),
            r#"
version: 2
models:
  - name: fct_orders
    meta:
      nova:
        canonical: true
        grain:
          primary_key: ["order_id"]
          time_field: order_date
        measures:
          - name: orders
            type: count_distinct
            expression: "count(distinct order_id)"
            description: "Orders"
            field: order_id
    columns:
      - name: order_id
        meta:
          nova:
            role: identifier
      - name: order_date
        meta:
          nova:
            role: time
"#,
        )
        .expect("write fixture");

        let result = dispatch(super::args::Command::Audit(super::args::AuditArgs {
            command: super::args::AuditCommand::NovaMeta(super::args::NovaMetaAuditArgs {
                project_dir: Some(temp_dir.path().to_string_lossy().to_string()),
                path: Vec::new(),
                resource_kind: None,
                resource_name: None,
                column: None,
                json: true,
            }),
        }))
        .await;

        assert!(result.is_ok());
    }

    #[test]
    fn prepare_storage_prunes_excluded_instance() {
        let temp_dir = TempDir::new().expect("temp dir");
        let mut config = test_config(temp_dir.path(), "active");
        config.storage_max_instances = 1;
        let instances_dir = config.storage_instances_dir().expect("instances dir");
        let active_dir = instances_dir.join("active");
        let stale_dir = instances_dir.join("stale");
        fs::create_dir_all(&active_dir).expect("create active");
        fs::create_dir_all(&stale_dir).expect("create stale");
        fs::write(stale_dir.join("payload.bin"), vec![1_u8; 2048]).expect("write stale");

        prepare_storage(&config).expect("prepare succeeds");
        assert!(active_dir.exists());
        assert!(!stale_dir.exists());
    }
}
