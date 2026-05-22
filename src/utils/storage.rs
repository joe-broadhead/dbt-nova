use std::cmp::min;
use std::path::Path;
use std::time::SystemTime;

use fs4::FileExt;

use crate::error::{DbtNovaError, Result};

/// Lock filename used to detect active storage directories.
pub const IN_USE_LOCK_FILENAME: &str = ".in_use.lock";

/// Returns true when a storage directory is currently locked by another process.
pub fn dir_in_use(path: &Path) -> bool {
    let lock_path = path.join(IN_USE_LOCK_FILENAME);
    let Ok(lock_file) = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
    else {
        return false;
    };

    if lock_file.try_lock_exclusive().is_err() {
        return true;
    }
    if let Err(err) = lock_file.unlock() {
        tracing::warn!(error = %err, "failed to release storage lock");
    }
    false
}

/// Prune directories with max/min counts and a max byte limit.
///
/// # Errors
/// Returns an error if directory scanning or removal fails.
pub fn prune_dirs(
    root: &Path,
    max_keep: usize,
    min_keep: usize,
    max_bytes: u64,
    exclude_names: &[&str],
) -> Result<()> {
    if (max_keep == 0 && max_bytes == 0) || !root.exists() {
        return Ok(());
    }

    let mut dirs: Vec<(SystemTime, u64, std::path::PathBuf)> = Vec::new();
    for entry in std::fs::read_dir(root)
        .map_err(|e| DbtNovaError::ServerError(format!("Storage scan failed: {e}")))?
    {
        let entry =
            entry.map_err(|e| DbtNovaError::ServerError(format!("Storage scan failed: {e}")))?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if exclude_names.contains(&name) {
            continue;
        }
        if dir_in_use(&path) {
            continue;
        }
        let meta = entry
            .metadata()
            .map_err(|e| DbtNovaError::ServerError(format!("Storage metadata failed: {e}")))?;
        let modified = meta
            .modified()
            .or_else(|_| meta.created())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        let size = dir_size_bytes(&path);
        dirs.push((modified, size, path));
    }

    if dirs.is_empty() {
        return Ok(());
    }

    let mut total_bytes: u64 = dirs.iter().map(|(_, size, _)| *size).sum();
    dirs.sort_by_key(|(modified, _, _)| *modified);

    let mut keep_limit = max_keep;
    if keep_limit == 0 {
        keep_limit = usize::MAX;
    }
    let min_keep = min(min_keep, keep_limit);

    while (dirs.len() > keep_limit)
        || (max_bytes > 0 && total_bytes > max_bytes && dirs.len() > min_keep)
    {
        let (_, size, path) = dirs.remove(0);
        std::fs::remove_dir_all(&path)
            .map_err(|e| DbtNovaError::ServerError(format!("Storage prune failed: {e}")))?;
        total_bytes = total_bytes.saturating_sub(size);
    }

    Ok(())
}

fn dir_size_bytes(path: &Path) -> u64 {
    let mut total = 0u64;
    let Ok(entries) = std::fs::read_dir(path) else {
        return 0;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if meta.is_dir() {
            total = total.saturating_add(dir_size_bytes(&path));
        } else {
            total = total.saturating_add(meta.len());
        }
    }

    total
}

#[cfg(test)]
mod tests {
    use std::fs::{self, File, OpenOptions};
    use std::io::Write;
    use std::path::Path;
    use std::time::{Duration, SystemTime};

    use fs4::FileExt;
    use tempfile::TempDir;

    use super::{IN_USE_LOCK_FILENAME, dir_in_use, prune_dirs};

    fn set_dir_modified(path: &Path, age: Duration) {
        let modified = SystemTime::now()
            .checked_sub(age)
            .expect("test timestamp should be representable");
        filetime::set_file_mtime(path, filetime::FileTime::from_system_time(modified))
            .expect("set directory mtime");
    }

    fn create_dir_with_file(root: &Path, name: &str, bytes: usize) -> std::path::PathBuf {
        let dir = root.join(name);
        fs::create_dir(&dir).expect("create storage dir");
        let mut file = File::create(dir.join("payload.bin")).expect("create payload");
        file.write_all(&vec![b'x'; bytes]).expect("write payload");
        dir
    }

    #[test]
    fn dir_in_use_missing_lock_is_read_only() {
        let temp = TempDir::new().expect("temp dir");
        let storage_dir = create_dir_with_file(temp.path(), "storage", 8);
        let lock_path = storage_dir.join(IN_USE_LOCK_FILENAME);

        assert!(!dir_in_use(&storage_dir));
        assert!(
            !lock_path.exists(),
            "checking an unlocked directory should not create a lock file"
        );
    }

    #[test]
    fn prune_dirs_respects_excludes_and_keeps_newest_entries() {
        let temp = TempDir::new().expect("temp dir");
        let root = temp.path();

        let old = create_dir_with_file(root, "old", 8);
        let keep = create_dir_with_file(root, "keep", 8);
        let newest = create_dir_with_file(root, "newest", 8);
        set_dir_modified(&old, Duration::from_secs(30));
        set_dir_modified(&keep, Duration::from_secs(20));
        set_dir_modified(&newest, Duration::from_secs(10));

        prune_dirs(root, 1, 0, 0, &["keep"]).expect("prune dirs");

        assert!(
            !old.exists(),
            "oldest non-excluded directory should be pruned"
        );
        assert!(keep.exists(), "excluded directory should be preserved");
        assert!(newest.exists(), "newest directory should be preserved");
    }

    #[test]
    fn prune_dirs_preserves_locked_storage_directories() {
        let temp = TempDir::new().expect("temp dir");
        let root = temp.path();

        let locked = create_dir_with_file(root, "locked", 16);
        let lock_file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(locked.join(IN_USE_LOCK_FILENAME))
            .expect("open lock file");
        lock_file.try_lock_exclusive().expect("lock storage dir");

        let removable = create_dir_with_file(root, "removable", 16);

        prune_dirs(root, 0, 0, 1, &[]).expect("prune dirs");

        assert!(locked.exists(), "locked directory should be preserved");
        assert!(
            !removable.exists(),
            "unlocked directory should be pruned to satisfy byte cap"
        );

        lock_file.unlock().expect("unlock storage dir");
    }
}
