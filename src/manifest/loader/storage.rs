use std::fs::{self, File};
use std::io::ErrorKind;
use std::io::{BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use fs4::FileExt;
use serde::{Deserialize, Serialize};

use crate::config::DbtNovaConfig;
use crate::config::search::{LEGACY_STORAGE_FORMAT_VERSION, STORAGE_FORMAT_VERSION};
use crate::error::{DbtNovaError, Result};
use crate::manifest::rkyv_indexes;
use crate::manifest::search::InUseLocks;
use crate::manifest::source::ManifestSignature;
use crate::manifest::store::EntityStore;
use crate::manifest::tantivy_search::TantivySearcher;
use crate::utils::{IN_USE_LOCK_FILENAME, unique_suffix};

pub(super) const MANIFEST_SIGNATURE_FILENAME: &str = "manifest.signature.json";
const MANIFEST_CURRENT_FILENAME: &str = "manifest.current.json";
const BUILD_LOCK_FILENAME: &str = ".build.lock";

pub(super) enum BuildLockAttempt {
    Acquired(File),
    Busy,
}

pub(super) struct StorageVersionSelection {
    pub(super) signature: ManifestSignature,
    pub(super) version_id: String,
    pub(super) storage_dir: PathBuf,
    pub(super) signature_path: PathBuf,
    pub(super) build_lock: Option<File>,
}

struct PublishedStorageVersion {
    version_id: String,
    storage_dir: PathBuf,
    signature_path: PathBuf,
    signature: ManifestSignature,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct ManifestCurrent {
    version: String,
    updated_ms: u128,
}

pub(super) fn read_manifest_signature(path: &Path) -> Result<Option<ManifestSignature>> {
    if !path.exists() {
        return Ok(None);
    }
    let file = File::open(path)?;
    let sig = serde_json::from_reader(BufReader::new(file))
        .map_err(|e| DbtNovaError::ServerError(format!("Invalid manifest signature: {e}")))?;
    Ok(Some(sig))
}

pub(super) fn write_manifest_signature(path: &Path, sig: &ManifestSignature) -> Result<()> {
    write_json_atomic(path, sig).map_err(|e| {
        DbtNovaError::ServerError(format!("Failed to write manifest signature: {e}"))
    })?;
    Ok(())
}

pub(super) fn manifest_signature_matches_for_reuse(
    existing: &ManifestSignature,
    expected: &ManifestSignature,
) -> bool {
    !existing.content_hash.is_empty()
        && !expected.content_hash.is_empty()
        && existing.content_hash == expected.content_hash
        && existing.prune_fingerprint == expected.prune_fingerprint
        && existing.search_index_fingerprint == expected.search_index_fingerprint
        && existing.effective_storage_format_version()
            == expected.effective_storage_format_version()
}

pub(super) fn manifest_signature_matches_for_runtime_read(
    config: &DbtNovaConfig,
    existing: &ManifestSignature,
    expected: &ManifestSignature,
) -> bool {
    manifest_signature_matches_for_reuse(existing, expected)
        || (config.storage_read_only
            && existing.effective_storage_format_version() == LEGACY_STORAGE_FORMAT_VERSION
            && expected.effective_storage_format_version() == STORAGE_FORMAT_VERSION
            && !existing.content_hash.is_empty()
            && existing.content_hash == expected.content_hash
            && existing.prune_fingerprint == expected.prune_fingerprint
            && existing.search_index_fingerprint == config.search.legacy_index_fingerprint())
}

pub(super) fn read_current_version(path: &Path) -> Result<Option<String>> {
    let current_path = path.join(MANIFEST_CURRENT_FILENAME);
    if !current_path.exists() {
        return Ok(None);
    }
    let file = File::open(current_path)?;
    let current: ManifestCurrent = serde_json::from_reader(BufReader::new(file))
        .map_err(|e| DbtNovaError::ServerError(format!("Invalid manifest current file: {e}")))?;
    if current.version.trim().is_empty() {
        return Ok(None);
    }
    Ok(Some(current.version))
}

pub(super) fn write_current_version(path: &Path, version: &str) -> Result<()> {
    let current_path = path.join(MANIFEST_CURRENT_FILENAME);
    let updated_ms = current_time_ms();
    let current = ManifestCurrent {
        version: version.to_string(),
        updated_ms,
    };
    write_json_atomic(&current_path, &current).map_err(|e| {
        DbtNovaError::ServerError(format!("Failed to write manifest current file: {e}"))
    })?;
    Ok(())
}

fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let data = serde_json::to_vec(value)
        .map_err(|e| DbtNovaError::ServerError(format!("Failed to encode JSON: {e}")))?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("manifest.json");
    let tmp = path.with_file_name(format!("{file_name}.{}.tmp", unique_suffix()));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)?;
    file.write_all(&data)?;
    file.sync_all()?;
    fs::rename(tmp, path)?;
    Ok(())
}

pub(super) fn current_time_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis())
}

pub(super) fn acquire_build_lock(storage_dir: &Path, wait_secs: u64) -> Result<File> {
    let file = open_build_lock(storage_dir)?;
    let start = Instant::now();
    let wait = Duration::from_secs(wait_secs);
    let mut warned = false;
    loop {
        match file.try_lock_exclusive() {
            Ok(()) => return Ok(file),
            Err(err) => {
                if !warned {
                    tracing::info!(
                        wait_secs,
                        "storage build lock held by another process; waiting"
                    );
                    warned = true;
                }
                if start.elapsed() >= wait {
                    return Err(DbtNovaError::ServerError(format!(
                        "Storage lock failed after {wait_secs}s: {err}"
                    )));
                }
                std::thread::sleep(Duration::from_millis(250));
            }
        }
    }
}

fn try_acquire_build_lock(storage_dir: &Path) -> Result<BuildLockAttempt> {
    let file = open_build_lock(storage_dir)?;
    match file.try_lock_exclusive() {
        Ok(()) => Ok(BuildLockAttempt::Acquired(file)),
        Err(err) if err.kind() == ErrorKind::WouldBlock => Ok(BuildLockAttempt::Busy),
        Err(err) => Err(DbtNovaError::ServerError(format!(
            "Storage lock failed: {err}"
        ))),
    }
}

fn open_build_lock(storage_dir: &Path) -> Result<File> {
    let lock_path = storage_dir.join(BUILD_LOCK_FILENAME);
    Ok(fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)?)
}

pub(super) fn select_storage_version_or_lock(
    config: &DbtNovaConfig,
    instance_root: &Path,
    versions_root: &Path,
    signature: ManifestSignature,
    version_id: String,
    storage_dir: PathBuf,
    signature_path: PathBuf,
) -> Result<StorageVersionSelection> {
    let mut selection = StorageVersionSelection {
        signature,
        version_id,
        storage_dir,
        signature_path,
        build_lock: None,
    };

    if let Some(published) = find_lockless_storage_version(
        config,
        instance_root,
        versions_root,
        &selection.signature,
        &selection.storage_dir,
        &selection.signature_path,
        &selection.version_id,
    )? {
        tracing::info!(
            requested_version_id = %selection.version_id,
            served_version_id = %published.version_id,
            "serving complete storage version without acquiring build lock"
        );
        selection.signature = published.signature;
        selection.version_id = published.version_id;
        selection.storage_dir = published.storage_dir;
        selection.signature_path = published.signature_path;
        return Ok(selection);
    }

    selection.build_lock = match try_acquire_build_lock(instance_root)? {
        BuildLockAttempt::Acquired(lock) => Some(lock),
        BuildLockAttempt::Busy => serve_current_or_wait_for_build_lock(
            config,
            instance_root,
            versions_root,
            &mut selection,
        )?,
    };
    Ok(selection)
}

fn serve_current_or_wait_for_build_lock(
    config: &DbtNovaConfig,
    instance_root: &Path,
    versions_root: &Path,
    selection: &mut StorageVersionSelection,
) -> Result<Option<File>> {
    if let Some(published) =
        load_complete_published_storage_version(config, instance_root, versions_root)?
    {
        tracing::info!(
            requested_version_id = %selection.version_id,
            served_version_id = %published.version_id,
            "serving published storage version while another build holds the lock"
        );
        selection.signature = published.signature;
        selection.version_id = published.version_id;
        selection.storage_dir = published.storage_dir;
        selection.signature_path = published.signature_path;
        return Ok(None);
    }
    acquire_build_lock(instance_root, config.storage_build_lock_wait_secs).map(Some)
}

fn find_lockless_storage_version(
    config: &DbtNovaConfig,
    instance_root: &Path,
    versions_root: &Path,
    signature: &ManifestSignature,
    initial_storage_dir: &Path,
    initial_signature_path: &Path,
    initial_version_id: &str,
) -> Result<Option<PublishedStorageVersion>> {
    if let Some(current) = read_current_version(instance_root)? {
        let current_dir = versions_root.join(&current);
        let current_sig_path = current_dir.join(MANIFEST_SIGNATURE_FILENAME);
        if let Some(current_signature) = read_manifest_signature(&current_sig_path)?
            && manifest_signature_matches_for_runtime_read(config, &current_signature, signature)
            && storage_dir_has_lockless_read_artifacts(config, &current_dir, &current_signature)
        {
            return Ok(Some(PublishedStorageVersion {
                version_id: current,
                storage_dir: current_dir,
                signature_path: current_sig_path,
                signature: current_signature,
            }));
        }
    }

    let Some(initial_signature) = read_manifest_signature(initial_signature_path)? else {
        return Ok(None);
    };
    if manifest_signature_matches_for_runtime_read(config, &initial_signature, signature)
        && storage_dir_has_lockless_read_artifacts(config, initial_storage_dir, &initial_signature)
    {
        return Ok(Some(PublishedStorageVersion {
            version_id: initial_version_id.to_string(),
            storage_dir: initial_storage_dir.to_path_buf(),
            signature_path: initial_signature_path.to_path_buf(),
            signature: initial_signature,
        }));
    }
    Ok(None)
}

fn load_complete_published_storage_version(
    config: &DbtNovaConfig,
    instance_root: &Path,
    versions_root: &Path,
) -> Result<Option<PublishedStorageVersion>> {
    let Some(version_id) = read_current_version(instance_root)? else {
        return Ok(None);
    };
    let storage_dir = versions_root.join(&version_id);
    let signature_path = storage_dir.join(MANIFEST_SIGNATURE_FILENAME);
    let Some(signature) = read_manifest_signature(&signature_path)? else {
        return Ok(None);
    };
    if !storage_dir_has_lockless_read_artifacts(config, &storage_dir, &signature) {
        return Ok(None);
    }
    Ok(Some(PublishedStorageVersion {
        version_id,
        storage_dir,
        signature_path,
        signature,
    }))
}

fn storage_dir_has_lockless_read_artifacts(
    config: &DbtNovaConfig,
    storage_dir: &Path,
    signature: &ManifestSignature,
) -> bool {
    if EntityStore::open(storage_dir).is_err() {
        return false;
    }
    if !matches!(
        TantivySearcher::open(storage_dir, &config.search),
        Ok(Some(_))
    ) {
        return false;
    }
    rkyv_indexes::try_load_indexes(storage_dir, &signature.content_hash).is_some()
}

pub(super) fn acquire_in_use_locks(instance_root: &Path, storage_dir: &Path) -> Result<InUseLocks> {
    let instance_root_lock = acquire_in_use_lock(instance_root)?;
    let version_dir_lock = acquire_in_use_lock(storage_dir)?;
    Ok(InUseLocks {
        instance_root: instance_root_lock,
        version_dir: version_dir_lock,
    })
}

fn acquire_in_use_lock(lock_dir: &Path) -> Result<File> {
    let lock_path = lock_dir.join(IN_USE_LOCK_FILENAME);
    let file = fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)?;
    file.lock_shared()
        .map_err(|e| DbtNovaError::ServerError(format!("Storage in-use lock failed: {e}")))?;
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_read_accepts_legacy_storage_only_in_read_only_mode() {
        let mut config = DbtNovaConfig::default();
        let expected = ManifestSignature {
            content_hash: "same-hash".to_string(),
            search_index_fingerprint: config.search.index_fingerprint(),
            storage_format_version: STORAGE_FORMAT_VERSION.to_string(),
            ..ManifestSignature::default()
        };
        let existing = ManifestSignature {
            content_hash: "same-hash".to_string(),
            search_index_fingerprint: config.search.legacy_index_fingerprint(),
            ..ManifestSignature::default()
        };

        assert!(!manifest_signature_matches_for_runtime_read(
            &config, &existing, &expected
        ));

        config.storage_read_only = true;
        assert!(manifest_signature_matches_for_runtime_read(
            &config, &existing, &expected
        ));

        let mismatched = ManifestSignature {
            content_hash: "different-hash".to_string(),
            ..existing
        };
        assert!(!manifest_signature_matches_for_runtime_read(
            &config,
            &mismatched,
            &expected
        ));
    }
}
