use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use flate2::read::GzDecoder;
use tar::Archive;
use tracing::info;

use crate::config::{ArtifactFetchPolicy, DbtNovaConfig};
use crate::error::{DbtNovaError, Result};
use crate::manifest::prebuilt_assets::PrebuiltAssetsMetadata;
use crate::utils::{sanitize_uri, unique_suffix};

/// Result of file-based prebuilt artifact materialization.
#[derive(Debug, Clone)]
pub struct FileArtifactMaterialization {
    pub metadata: PrebuiltAssetsMetadata,
    pub storage_materialized: bool,
    pub models_materialized: bool,
}

/// Materialize prebuilt artifacts from file URIs.
///
/// This is the P2 resolver MVP and currently supports only local file sources
/// (`file://` or plain paths).
///
/// # Errors
///
/// Returns an error when URIs are invalid, metadata validation fails, or archive
/// extraction cannot be completed safely.
pub fn materialize_file_artifacts(
    config: &DbtNovaConfig,
    expected_manifest_hash: &str,
) -> Result<Option<FileArtifactMaterialization>> {
    if !config.remote_artifact_mode_enabled() {
        return Ok(None);
    }

    let metadata_path = resolve_file_artifact_uri(
        "DBT_NOVA_METADATA_ARTIFACT_URI",
        &config.metadata_artifact_uri,
    )?;
    let metadata_raw = read_small_text_file(&metadata_path, config.manifest_max_bytes)?;
    let metadata = PrebuiltAssetsMetadata::from_json_str(&metadata_raw)?;
    validate_metadata_against_runtime(&metadata, config, expected_manifest_hash)?;

    let storage_archive_path = resolve_file_artifact_uri(
        "DBT_NOVA_STORAGE_ARTIFACT_URI",
        &config.storage_artifact_uri,
    )?;
    ensure_regular_file("DBT_NOVA_STORAGE_ARTIFACT_URI", &storage_archive_path)?;

    let models_archive_path = if config.models_artifact_uri.trim().is_empty() {
        None
    } else {
        let path =
            resolve_file_artifact_uri("DBT_NOVA_MODELS_ARTIFACT_URI", &config.models_artifact_uri)?;
        ensure_regular_file("DBT_NOVA_MODELS_ARTIFACT_URI", &path)?;
        if !metadata.has_models_artifact() {
            return Err(DbtNovaError::InvalidParams(
                "DBT_NOVA_MODELS_ARTIFACT_URI is set but metadata contract has no artifact_name_models"
                    .to_string(),
            ));
        }
        Some(path)
    };

    let storage_target = config.storage_root_dir()?;
    let storage_present = storage_instance_present(config)?;
    let should_materialize_storage = should_materialize(
        "storage artifacts",
        config.artifact_fetch_policy,
        storage_present,
    )?;
    ensure_materialization_allowed(config, should_materialize_storage, "storage artifacts")?;
    let storage_materialized = if should_materialize_storage {
        extract_archive_atomically(&storage_archive_path, &storage_target)?;
        true
    } else {
        false
    };

    let mut models_materialized = false;
    if let Some(models_archive_path) = models_archive_path {
        let models_target = PathBuf::from(&config.search.embedding_cache_dir);
        let models_present = directory_has_files(&models_target)?;
        let should_materialize_models = should_materialize(
            "models artifacts",
            config.artifact_fetch_policy,
            models_present,
        )?;
        ensure_materialization_allowed(config, should_materialize_models, "models artifacts")?;
        if should_materialize_models {
            extract_archive_atomically(&models_archive_path, &models_target)?;
            models_materialized = true;
        }
    }

    info!(
        storage_uri = %sanitize_uri(&config.storage_artifact_uri),
        metadata_uri = %sanitize_uri(&config.metadata_artifact_uri),
        models_uri = %sanitize_uri(&config.models_artifact_uri),
        storage_materialized,
        models_materialized,
        "file artifact materialization evaluated"
    );

    Ok(Some(FileArtifactMaterialization {
        metadata,
        storage_materialized,
        models_materialized,
    }))
}

fn ensure_materialization_allowed(
    config: &DbtNovaConfig,
    should_materialize: bool,
    label: &str,
) -> Result<()> {
    if config.storage_read_only && should_materialize {
        return Err(DbtNovaError::ServerError(format!(
            "Storage is read-only; cannot materialize {label} (set DBT_NOVA_ARTIFACT_FETCH_POLICY=never and pre-materialize assets)"
        )));
    }
    Ok(())
}

fn should_materialize(label: &str, policy: ArtifactFetchPolicy, is_present: bool) -> Result<bool> {
    match policy {
        ArtifactFetchPolicy::Always => Ok(true),
        ArtifactFetchPolicy::IfMissing => Ok(!is_present),
        ArtifactFetchPolicy::Never => {
            if is_present {
                Ok(false)
            } else {
                Err(DbtNovaError::ServerError(format!(
                    "artifact fetch policy is 'never' but no {label} are present locally"
                )))
            }
        }
    }
}

fn validate_metadata_against_runtime(
    metadata: &PrebuiltAssetsMetadata,
    config: &DbtNovaConfig,
    expected_manifest_hash: &str,
) -> Result<()> {
    let instance_id = config.storage_instance_id.trim();
    if metadata.storage_instance_id.trim() != instance_id {
        return Err(DbtNovaError::ServerError(format!(
            "storage_instance_id mismatch: metadata='{}' runtime='{}'",
            metadata.storage_instance_id, instance_id
        )));
    }
    if metadata.manifest_hash.trim() != expected_manifest_hash.trim() {
        return Err(DbtNovaError::ServerError(format!(
            "manifest hash mismatch: metadata='{}' runtime='{}'",
            metadata.manifest_hash, expected_manifest_hash
        )));
    }
    Ok(())
}

fn storage_instance_present(config: &DbtNovaConfig) -> Result<bool> {
    let versions_root = config.storage_instance_root_dir()?.join("versions");
    if !versions_root.is_dir() {
        return Ok(false);
    }
    for entry in fs::read_dir(&versions_root)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            return Ok(true);
        }
    }
    Ok(false)
}

fn directory_has_files(path: &Path) -> Result<bool> {
    if !path.is_dir() {
        return Ok(false);
    }
    let mut stack = vec![path.to_path_buf()];
    while let Some(next) = stack.pop() {
        for entry in fs::read_dir(&next)? {
            let entry = entry?;
            let ty = entry.file_type()?;
            if ty.is_file() {
                return Ok(true);
            }
            if ty.is_dir() {
                stack.push(entry.path());
            }
        }
    }
    Ok(false)
}

fn read_small_text_file(path: &Path, max_bytes: u64) -> Result<String> {
    let mut file = File::open(path)?;
    let metadata = file.metadata()?;
    if max_bytes > 0 && metadata.len() > max_bytes {
        return Err(DbtNovaError::ServerError(format!(
            "artifact metadata exceeds configured size limit ({} > {})",
            metadata.len(),
            max_bytes
        )));
    }

    let mut raw = String::new();
    file.read_to_string(&mut raw)?;
    Ok(raw)
}

fn resolve_file_artifact_uri(name: &str, uri: &str) -> Result<PathBuf> {
    let trimmed = uri.trim();
    if trimmed.is_empty() {
        return Err(DbtNovaError::InvalidParams(format!(
            "{name} cannot be empty"
        )));
    }
    let lower = trimmed.to_ascii_lowercase();

    if let Some(rest) = lower.strip_prefix("file://") {
        let original_rest = &trimmed[(trimmed.len() - rest.len())..];
        if rest.starts_with("localhost/") {
            let host_path = &original_rest["localhost/".len()..];
            let normalized = format!("/{}", host_path.trim_start_matches('/'));
            return Ok(PathBuf::from(normalized));
        }
        if original_rest.starts_with('/') {
            return Ok(PathBuf::from(original_rest));
        }
        return Err(DbtNovaError::InvalidParams(format!(
            "{name} must use an absolute file:// URI"
        )));
    }

    if lower.contains("://") {
        return Err(DbtNovaError::ServerError(format!(
            "{name} uses a non-file URI scheme; file resolver currently supports only file:// or plain local paths"
        )));
    }

    Ok(PathBuf::from(trimmed))
}

fn ensure_regular_file(name: &str, path: &Path) -> Result<()> {
    if !path.exists() {
        return Err(DbtNovaError::ServerError(format!(
            "{name} does not exist: {}",
            path.display()
        )));
    }
    if !path.is_file() {
        return Err(DbtNovaError::ServerError(format!(
            "{name} must point to a file: {}",
            path.display()
        )));
    }
    Ok(())
}

fn extract_archive_atomically(archive_path: &Path, target_dir: &Path) -> Result<()> {
    let parent = target_dir.parent().ok_or_else(|| {
        DbtNovaError::ServerError(format!(
            "cannot materialize archive; target has no parent: {}",
            target_dir.display()
        ))
    })?;
    fs::create_dir_all(parent)?;

    let stage_root = parent.join(format!(".nova-artifacts-stage-{}", unique_suffix()));
    fs::create_dir_all(&stage_root)?;

    let extracted_root = stage_root.join("extract-root");
    fs::create_dir_all(&extracted_root)?;

    extract_tar_gz(archive_path, &extracted_root)?;
    let payload_root = single_directory_child(&extracted_root)?;
    swap_directory_atomically(&payload_root, target_dir)?;

    let _ = fs::remove_dir_all(&stage_root);
    Ok(())
}

fn extract_tar_gz(archive_path: &Path, output_root: &Path) -> Result<()> {
    let file = File::open(archive_path)?;
    let decoder = GzDecoder::new(file);
    let mut archive = Archive::new(decoder);

    for entry_result in archive.entries()? {
        let mut entry = entry_result?;
        let entry_path = entry.path()?.into_owned();
        validate_archive_entry_path(&entry_path)?;
        let entry_type = entry.header().entry_type();
        if !(entry_type.is_dir() || entry_type.is_file()) {
            return Err(DbtNovaError::ServerError(format!(
                "archive contains unsupported entry type for path: {}",
                entry_path.display()
            )));
        }

        let out_path = output_root.join(&entry_path);
        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent)?;
        }
        entry.unpack(&out_path)?;
    }
    Ok(())
}

fn validate_archive_entry_path(path: &Path) -> Result<()> {
    if path.is_absolute() {
        return Err(DbtNovaError::ServerError(format!(
            "archive contains absolute path entry: {}",
            path.display()
        )));
    }
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(DbtNovaError::ServerError(format!(
            "archive contains parent traversal entry: {}",
            path.display()
        )));
    }
    Ok(())
}

fn single_directory_child(root: &Path) -> Result<PathBuf> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        entries.push(entry);
    }
    if entries.len() != 1 {
        return Err(DbtNovaError::ServerError(format!(
            "archive must contain exactly one top-level directory entry; found {}",
            entries.len()
        )));
    }
    let entry = entries.remove(0);
    if !entry.file_type()?.is_dir() {
        return Err(DbtNovaError::ServerError(
            "archive top-level entry must be a directory".to_string(),
        ));
    }
    Ok(entry.path())
}

fn swap_directory_atomically(source_dir: &Path, target_dir: &Path) -> Result<()> {
    let mut backup_dir = None;
    if target_dir.exists() {
        let file_name = target_dir
            .file_name()
            .unwrap_or_else(|| OsStr::new("target"))
            .to_string_lossy()
            .to_string();
        let backup = target_dir.with_file_name(format!("{file_name}.backup-{}", unique_suffix()));
        fs::rename(target_dir, &backup)?;
        backup_dir = Some(backup);
    }

    if let Err(err) = fs::rename(source_dir, target_dir) {
        if let Some(backup) = backup_dir.as_ref() {
            let _ = fs::rename(backup, target_dir);
        }
        return Err(DbtNovaError::ServerError(format!(
            "failed to materialize archive into {}: {err}",
            target_dir.display()
        )));
    }

    if let Some(backup) = backup_dir {
        let _ = fs::remove_dir_all(backup);
    }
    Ok(())
}
