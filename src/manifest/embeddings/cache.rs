use std::fs;
use std::path::PathBuf;

use tracing::warn;

use crate::config::SearchConfig;

pub(crate) fn embeddings_cache_dir(config: &SearchConfig) -> PathBuf {
    let raw = config.embedding_cache_dir.trim();
    let path = if raw.is_empty() {
        if let Ok(exe_path) = std::env::current_exe()
            && let Some(parent) = exe_path.parent()
        {
            let bundled = parent.join("models");
            if bundled.is_dir() {
                return bundled;
            }
        }
        PathBuf::from(".dbt-nova").join(".fastembed_cache")
    } else {
        PathBuf::from(raw)
    };
    if let Err(err) = fs::create_dir_all(&path) {
        warn!(error = %err, cache_dir = %path.display(), "failed to create embeddings cache dir");
    }
    path
}
