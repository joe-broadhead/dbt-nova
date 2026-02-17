use std::fs;
use std::path::Path;

use crate::error::Result;
use crate::manifest::rkyv_cache::{load_rkyv_file, load_rkyv_file_zst, save_rkyv_zst};
use crate::manifest::rkyv_types::{CachedEmbeddings, RKYV_SCHEMA_VERSION};

const EMBEDDINGS_FILE_ZST: &str = "embeddings.rkyv.zst";
const EMBEDDINGS_FILE_RAW: &str = "embeddings.rkyv";

/// Save cached embeddings to disk.
///
/// # Errors
/// Returns an error if serialization, compression, or the write fails.
pub fn save_embeddings(cache: &CachedEmbeddings, storage_dir: &Path) -> Result<()> {
    save_rkyv_zst(cache, &storage_dir.join(EMBEDDINGS_FILE_ZST))
}

#[must_use]
pub fn try_load_embeddings(
    storage_dir: &Path,
    expected_model: &str,
    expected_hash: &str,
    max_decompressed_bytes: u64,
) -> Option<CachedEmbeddings> {
    let zst_path = storage_dir.join(EMBEDDINGS_FILE_ZST);
    if let Some(cached) = load_rkyv_file_zst(&zst_path, max_decompressed_bytes, |cache| {
        cache_valid(cache, expected_model, expected_hash)
    }) {
        return Some(cached);
    }

    let raw_path = storage_dir.join(EMBEDDINGS_FILE_RAW);
    if max_decompressed_bytes > 0
        && fs::metadata(&raw_path).map(|meta| meta.len()).unwrap_or(0) > max_decompressed_bytes
    {
        return None;
    }
    load_rkyv_file(&raw_path, |cache| {
        cache_valid(cache, expected_model, expected_hash)
    })
}

fn cache_valid(cache: &CachedEmbeddings, expected_model: &str, expected_hash: &str) -> bool {
    cache.schema_version == RKYV_SCHEMA_VERSION
        && cache.model_name.as_str() == expected_model
        && cache.manifest_hash.as_str() == expected_hash
}
