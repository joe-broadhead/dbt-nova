use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::time::{Instant, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::cli::args::{StorageCleanupArgs, StorageInspectArgs, StoragePruneArgs};
use crate::cli::output::{CliEnvelope, error_envelope};
use crate::config::DbtNovaConfig;
use crate::error::{DbtNovaError, Result};
use crate::utils::dir_in_use;

use super::{DispatchError, DispatchResult, cleanup_storage_dir, prune_storage_instances};

#[derive(Debug, Serialize)]
pub struct StorageInspectData {
    pub storage_root: String,
    pub instances_dir: String,
    pub configured_instance_id: String,
    pub instance_count: usize,
    pub instances: Vec<StorageInstanceInfo>,
}

#[derive(Debug, Serialize)]
pub struct StorageInstanceInfo {
    pub instance_id: String,
    pub path: String,
    pub in_use: bool,
    pub size_bytes: u64,
    pub modified_ms: Option<u128>,
    pub current_version: Option<String>,
    pub version_count: usize,
}

#[derive(Debug, Serialize)]
pub struct StoragePruneData {
    pub storage_root: String,
    pub instances_dir: String,
    pub configured_instance_id: String,
    pub max_keep: usize,
    pub max_bytes: u64,
    pub instances_before: usize,
    pub instances_after: usize,
    pub pruned_instances: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct StorageCleanupData {
    pub storage_root: String,
    pub instance_id: String,
    pub instance_path: String,
    pub existed_before: bool,
    pub in_use_before: bool,
    pub removed: bool,
}

#[derive(Debug, Deserialize)]
struct ManifestCurrentFile {
    version: String,
}

/// Runs the `storage inspect` CLI command.
///
/// # Errors
/// Returns an error when storage metadata cannot be inspected or output rendering fails.
pub fn run_inspect_command(args: &StorageInspectArgs) -> DispatchResult {
    let started = Instant::now();
    let config = build_storage_config(args.storage_instance_id.as_deref()).map_err(|error| {
        render_or_propagate_error(
            args.json,
            "storage inspect",
            error,
            started.elapsed().as_millis(),
        )
    })?;
    let payload = inspect_storage(&config).map_err(|error| {
        render_or_propagate_error(
            args.json,
            "storage inspect",
            error,
            started.elapsed().as_millis(),
        )
    })?;

    if args.json {
        let envelope =
            CliEnvelope::success("storage inspect", &payload, started.elapsed().as_millis());
        let out = serde_json::to_string_pretty(&envelope).map_err(|error| DispatchError {
            error: DbtNovaError::ServerError(error.to_string()),
            rendered: false,
        })?;
        println!("{out}");
    } else {
        print_storage_inspect_human(&payload);
    }

    Ok(())
}

/// Runs the `storage prune` CLI command.
///
/// # Errors
/// Returns an error when pruning fails or output rendering fails.
pub fn run_prune_command(args: &StoragePruneArgs) -> DispatchResult {
    let started = Instant::now();
    let mut config =
        build_storage_config(args.storage_instance_id.as_deref()).map_err(|error| {
            render_or_propagate_error(
                args.json,
                "storage prune",
                error,
                started.elapsed().as_millis(),
            )
        })?;

    let max_keep = args
        .max_keep
        .unwrap_or(config.storage_max_instances.saturating_sub(1));
    let max_bytes = args.max_bytes.unwrap_or(config.storage_max_bytes);
    config.storage_max_bytes = max_bytes;

    let instances_dir = config.storage_instances_dir().map_err(|error| {
        render_or_propagate_error(
            args.json,
            "storage prune",
            error,
            started.elapsed().as_millis(),
        )
    })?;
    let before = list_instance_ids(&instances_dir).map_err(|error| {
        render_or_propagate_error(
            args.json,
            "storage prune",
            error,
            started.elapsed().as_millis(),
        )
    })?;

    prune_storage_instances(&config, max_keep, Some(config.storage_instance_id.as_str())).map_err(
        |error| {
            render_or_propagate_error(
                args.json,
                "storage prune",
                error,
                started.elapsed().as_millis(),
            )
        },
    )?;

    let after = list_instance_ids(&instances_dir).map_err(|error| {
        render_or_propagate_error(
            args.json,
            "storage prune",
            error,
            started.elapsed().as_millis(),
        )
    })?;
    let after_set: BTreeSet<&str> = after.iter().map(String::as_str).collect();
    let pruned_instances = before
        .iter()
        .filter(|id| !after_set.contains(id.as_str()))
        .cloned()
        .collect::<Vec<_>>();

    let payload = StoragePruneData {
        storage_root: config
            .storage_root_dir()
            .map_err(|error| {
                render_or_propagate_error(
                    args.json,
                    "storage prune",
                    error,
                    started.elapsed().as_millis(),
                )
            })?
            .to_string_lossy()
            .to_string(),
        instances_dir: instances_dir.to_string_lossy().to_string(),
        configured_instance_id: config.storage_instance_id.clone(),
        max_keep,
        max_bytes,
        instances_before: before.len(),
        instances_after: after.len(),
        pruned_instances,
    };

    if args.json {
        let envelope =
            CliEnvelope::success("storage prune", &payload, started.elapsed().as_millis());
        let out = serde_json::to_string_pretty(&envelope).map_err(|error| DispatchError {
            error: DbtNovaError::ServerError(error.to_string()),
            rendered: false,
        })?;
        println!("{out}");
    } else {
        print_storage_prune_human(&payload);
    }

    Ok(())
}

/// Runs the `storage cleanup` CLI command.
///
/// # Errors
/// Returns an error when cleanup fails or output rendering fails.
pub fn run_cleanup_command(args: &StorageCleanupArgs) -> DispatchResult {
    let started = Instant::now();
    let config = build_storage_config(args.storage_instance_id.as_deref()).map_err(|error| {
        render_or_propagate_error(
            args.json,
            "storage cleanup",
            error,
            started.elapsed().as_millis(),
        )
    })?;

    let storage_root = config.storage_root_dir().map_err(|error| {
        render_or_propagate_error(
            args.json,
            "storage cleanup",
            error,
            started.elapsed().as_millis(),
        )
    })?;
    let instance_path = config.storage_instance_root_dir().map_err(|error| {
        render_or_propagate_error(
            args.json,
            "storage cleanup",
            error,
            started.elapsed().as_millis(),
        )
    })?;
    let existed_before = instance_path.exists();
    let in_use_before = existed_before && dir_in_use(&instance_path);

    cleanup_storage_dir(&config).map_err(|error| {
        render_or_propagate_error(
            args.json,
            "storage cleanup",
            error,
            started.elapsed().as_millis(),
        )
    })?;

    let payload = StorageCleanupData {
        storage_root: storage_root.to_string_lossy().to_string(),
        instance_id: config.storage_instance_id.clone(),
        instance_path: instance_path.to_string_lossy().to_string(),
        existed_before,
        in_use_before,
        removed: existed_before && !instance_path.exists(),
    };

    if args.json {
        let envelope =
            CliEnvelope::success("storage cleanup", &payload, started.elapsed().as_millis());
        let out = serde_json::to_string_pretty(&envelope).map_err(|error| DispatchError {
            error: DbtNovaError::ServerError(error.to_string()),
            rendered: false,
        })?;
        println!("{out}");
    } else {
        print_storage_cleanup_human(&payload);
    }

    Ok(())
}

fn build_storage_config(storage_instance_id_override: Option<&str>) -> Result<DbtNovaConfig> {
    let mut config = DbtNovaConfig::from_env();
    if let Some(storage_instance_id) = storage_instance_id_override {
        let trimmed = storage_instance_id.trim();
        if trimmed.is_empty() {
            return Err(DbtNovaError::InvalidParams(
                "--storage-instance-id cannot be empty".to_string(),
            ));
        }
        config.storage_instance_id = trimmed.to_string();
    }
    config.ensure_storage_instance_id();
    config.validate()?;
    let _ = config.storage_root_dir()?;
    let _ = config.storage_instances_dir()?;
    let _ = config.storage_base_dir()?;
    Ok(config)
}

fn inspect_storage(config: &DbtNovaConfig) -> Result<StorageInspectData> {
    let storage_root = config.storage_root_dir()?;
    let instances_dir = config.storage_instances_dir()?;
    let instances = list_instance_infos(&instances_dir)?;
    Ok(StorageInspectData {
        storage_root: storage_root.to_string_lossy().to_string(),
        instances_dir: instances_dir.to_string_lossy().to_string(),
        configured_instance_id: config.storage_instance_id.clone(),
        instance_count: instances.len(),
        instances,
    })
}

fn list_instance_infos(instances_dir: &Path) -> Result<Vec<StorageInstanceInfo>> {
    if !instances_dir.exists() {
        return Ok(Vec::new());
    }

    let mut instances = Vec::new();
    for entry in fs::read_dir(instances_dir)
        .map_err(|error| DbtNovaError::ServerError(format!("Storage scan failed: {error}")))?
    {
        let entry = entry
            .map_err(|error| DbtNovaError::ServerError(format!("Storage scan failed: {error}")))?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(instance_id) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };

        let metadata = entry.metadata().map_err(|error| {
            DbtNovaError::ServerError(format!("Storage metadata failed: {error}"))
        })?;
        let modified_ms = metadata
            .modified()
            .ok()
            .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_millis());

        let versions_dir = path.join("versions");
        let version_count = count_subdirs(&versions_dir)?;
        let current_version = read_current_version(&path)?;

        instances.push(StorageInstanceInfo {
            instance_id: instance_id.to_string(),
            path: path.to_string_lossy().to_string(),
            in_use: dir_in_use(&path),
            size_bytes: dir_size_bytes(&path),
            modified_ms,
            current_version,
            version_count,
        });
    }

    instances.sort_by(|left, right| left.instance_id.cmp(&right.instance_id));
    Ok(instances)
}

fn list_instance_ids(instances_dir: &Path) -> Result<Vec<String>> {
    if !instances_dir.exists() {
        return Ok(Vec::new());
    }

    let mut ids = Vec::new();
    for entry in fs::read_dir(instances_dir)
        .map_err(|error| DbtNovaError::ServerError(format!("Storage scan failed: {error}")))?
    {
        let entry = entry
            .map_err(|error| DbtNovaError::ServerError(format!("Storage scan failed: {error}")))?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if let Some(instance_id) = path.file_name().and_then(|name| name.to_str()) {
            ids.push(instance_id.to_string());
        }
    }
    ids.sort();
    Ok(ids)
}

fn count_subdirs(path: &Path) -> Result<usize> {
    if !path.exists() {
        return Ok(0);
    }
    let mut count = 0usize;
    for entry in fs::read_dir(path)
        .map_err(|error| DbtNovaError::ServerError(format!("Storage scan failed: {error}")))?
    {
        let entry = entry
            .map_err(|error| DbtNovaError::ServerError(format!("Storage scan failed: {error}")))?;
        if entry.path().is_dir() {
            count = count.saturating_add(1);
        }
    }
    Ok(count)
}

fn read_current_version(instance_path: &Path) -> Result<Option<String>> {
    let current_path = instance_path.join("manifest.current.json");
    if !current_path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(&current_path).map_err(|error| {
        DbtNovaError::ServerError(format!(
            "Storage metadata failed: unable to read {}: {error}",
            current_path.display()
        ))
    })?;
    let parsed: ManifestCurrentFile = serde_json::from_str(&content).map_err(|error| {
        DbtNovaError::ServerError(format!(
            "Storage metadata failed: invalid {}: {error}",
            current_path.display()
        ))
    })?;
    if parsed.version.trim().is_empty() {
        return Ok(None);
    }
    Ok(Some(parsed.version))
}

fn dir_size_bytes(path: &Path) -> u64 {
    let mut total = 0u64;
    let Ok(entries) = fs::read_dir(path) else {
        return 0;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if metadata.is_dir() {
            total = total.saturating_add(dir_size_bytes(&path));
        } else {
            total = total.saturating_add(metadata.len());
        }
    }

    total
}

fn render_or_propagate_error(
    json: bool,
    command: &str,
    error: DbtNovaError,
    elapsed_ms: u128,
) -> DispatchError {
    if json {
        let envelope = error_envelope(command, &error, elapsed_ms);
        if let Ok(rendered) = serde_json::to_string_pretty(&envelope) {
            println!("{rendered}");
            return DispatchError {
                error,
                rendered: true,
            };
        }
    }
    DispatchError {
        error,
        rendered: false,
    }
}

fn print_storage_inspect_human(payload: &StorageInspectData) {
    println!("storage inspect");
    println!("  storage_root: {}", payload.storage_root);
    println!("  instances_dir: {}", payload.instances_dir);
    println!(
        "  configured_instance_id: {}",
        payload.configured_instance_id
    );
    println!("  instance_count: {}", payload.instance_count);
    println!("  instances:");
    for instance in &payload.instances {
        println!("    - instance_id: {}", instance.instance_id);
        println!("      path: {}", instance.path);
        println!("      in_use: {}", instance.in_use);
        println!("      size_bytes: {}", instance.size_bytes);
        println!(
            "      current_version: {}",
            instance.current_version.as_deref().unwrap_or("-")
        );
        println!("      version_count: {}", instance.version_count);
    }
}

fn print_storage_prune_human(payload: &StoragePruneData) {
    println!("storage prune");
    println!("  storage_root: {}", payload.storage_root);
    println!("  instances_dir: {}", payload.instances_dir);
    println!(
        "  configured_instance_id: {}",
        payload.configured_instance_id
    );
    println!("  max_keep: {}", payload.max_keep);
    println!("  max_bytes: {}", payload.max_bytes);
    println!("  instances_before: {}", payload.instances_before);
    println!("  instances_after: {}", payload.instances_after);
    println!("  pruned_instances: {}", payload.pruned_instances.len());
    for instance in &payload.pruned_instances {
        println!("    - {instance}");
    }
}

fn print_storage_cleanup_human(payload: &StorageCleanupData) {
    println!("storage cleanup");
    println!("  storage_root: {}", payload.storage_root);
    println!("  instance_id: {}", payload.instance_id);
    println!("  instance_path: {}", payload.instance_path);
    println!("  existed_before: {}", payload.existed_before);
    println!("  in_use_before: {}", payload.in_use_before);
    println!("  removed: {}", payload.removed);
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::{build_storage_config, inspect_storage};
    use crate::config::DbtNovaConfig;

    #[test]
    fn build_storage_config_rejects_unsafe_instance_id() {
        let err = build_storage_config(Some("unsafe/id")).expect_err("unsafe id should fail");
        assert!(err.to_string().contains("storage instance id is unsafe"));
    }

    #[test]
    fn inspect_storage_returns_empty_instances_for_missing_root() {
        let temp_dir = TempDir::new().expect("temp dir");
        let mut config = DbtNovaConfig {
            storage_dir: temp_dir.path().join(".nova").to_string_lossy().to_string(),
            ..DbtNovaConfig::default()
        };
        config.ensure_storage_instance_id();
        let payload = inspect_storage(&config).expect("inspect");
        assert_eq!(payload.instance_count, 0);
        assert!(payload.instances.is_empty());
    }

    #[test]
    fn inspect_storage_reads_instance_metadata() {
        let temp_dir = TempDir::new().expect("temp dir");
        let mut config = DbtNovaConfig {
            storage_dir: temp_dir.path().join(".nova").to_string_lossy().to_string(),
            ..DbtNovaConfig::default()
        };
        config.ensure_storage_instance_id();
        let instances_dir = config.storage_instances_dir().expect("instances dir");
        let instance_path = instances_dir.join("manifest-abc");
        fs::create_dir_all(instance_path.join("versions").join("version-a")).expect("versions");
        fs::write(
            instance_path.join("manifest.current.json"),
            r#"{"version":"version-a","updated_ms":1}"#,
        )
        .expect("manifest.current");

        let payload = inspect_storage(&config).expect("inspect");
        assert_eq!(payload.instance_count, 1);
        let instance = &payload.instances[0];
        assert_eq!(instance.instance_id, "manifest-abc");
        assert_eq!(instance.current_version.as_deref(), Some("version-a"));
        assert_eq!(instance.version_count, 1);
    }
}
