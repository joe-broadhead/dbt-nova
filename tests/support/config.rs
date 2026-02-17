use std::path::{Path, PathBuf};
use std::sync::Once;

use dbt_nova::DbtNovaConfig;
use dbt_nova::config::SearchConfig;
use fs4::FileExt;
use tempfile::TempDir;

static TEST_STORAGE_CLEANUP: Once = Once::new();

pub fn test_search_config() -> SearchConfig {
    let mut config = SearchConfig::default();
    let cache_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(".dbt-nova")
        .join(".fastembed_cache");
    config.embedding_cache_dir = cache_dir.to_string_lossy().to_string();

    let enable_embeddings = env_flag("DBT_NOVA_TEST_ENABLE_EMBEDDINGS");
    if enable_embeddings {
        config.enable_vector_search = true;
        config.enable_sparse_search = env_flag("DBT_NOVA_TEST_ENABLE_SPARSE");
        config.enable_reranker = env_flag("DBT_NOVA_TEST_ENABLE_RERANKER");
    } else {
        config.enable_vector_search = false;
        config.enable_sparse_search = false;
        config.enable_reranker = false;
    }

    config
}

pub struct TestStorageGuard {
    dir: TempDir,
}

impl TestStorageGuard {
    pub fn new() -> Self {
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("dbt-nova-tests");
        let _ = std::fs::create_dir_all(&base);
        TEST_STORAGE_CLEANUP.call_once(|| cleanup_test_storage(&base));
        let dir = TempDir::new_in(base).expect("test temp dir");
        Self { dir }
    }

    pub fn path(&self) -> &Path {
        self.dir.path()
    }
}

pub fn apply_test_storage(cfg: &mut DbtNovaConfig, guard: &TestStorageGuard) {
    cfg.storage_dir = guard.path().to_string_lossy().to_string();
    cfg.storage_instance_id = "tests".to_string();
    cfg.storage_max_instances = 1;
    cfg.cleanup_storage_on_start = true;
}

fn env_flag(name: &str) -> bool {
    std::env::var(name)
        .ok()
        .map(|v| {
            matches!(
                v.trim().to_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

fn cleanup_test_storage(base: &Path) {
    let cleanup_all = env_flag("DBT_NOVA_TEST_CLEANUP_ALL");
    let max_age_secs = std::env::var("DBT_NOVA_TEST_CLEANUP_AGE_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(300);

    let lock_path = base.join(".cleanup.lock");
    let lock_file = match std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(true)
        .open(&lock_path)
    {
        Ok(file) => file,
        Err(_) => return,
    };

    if lock_file.try_lock_exclusive().is_err() {
        return;
    }

    let now = std::time::SystemTime::now();
    let entries = match std::fs::read_dir(base) {
        Ok(entries) => entries,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if cleanup_all {
            let _ = std::fs::remove_dir_all(&path);
            continue;
        }

        let meta = match entry.metadata() {
            Ok(meta) => meta,
            Err(_) => continue,
        };
        let modified = meta.modified().or_else(|_| meta.created());
        let age_ok = match modified {
            Ok(mtime) => now
                .duration_since(mtime)
                .map(|age| age.as_secs() >= max_age_secs)
                .unwrap_or(false),
            Err(_) => false,
        };
        if age_ok {
            let _ = std::fs::remove_dir_all(&path);
        }
    }
}
