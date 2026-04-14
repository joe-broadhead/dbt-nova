use std::path::PathBuf;

use crate::config::SearchConfig;
use crate::manifest::semantic_cache;

pub(crate) fn embeddings_cache_dir(config: &SearchConfig) -> PathBuf {
    semantic_cache::embeddings_cache_dir(config)
}
