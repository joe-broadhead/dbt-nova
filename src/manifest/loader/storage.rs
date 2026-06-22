use std::fs::{self, File};
use std::io::{BufReader, Write};
use std::path::Path;
use std::time::{Duration, Instant};

use fs4::FileExt;
use serde::{Deserialize, Serialize};

use crate::error::{DbtNovaError, Result};
use crate::manifest::search::InUseLocks;
use crate::manifest::source::ManifestSignature;
use crate::utils::{IN_USE_LOCK_FILENAME, unique_suffix};

pub(super) const MANIFEST_SIGNATURE_FILENAME: &str = "manifest.signature.json";
const MANIFEST_CURRENT_FILENAME: &str = "manifest.current.json";

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
    let lock_path = storage_dir.join(".build.lock");
    let file = fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)?;
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
