use std::fs;
use std::path::Path;

use crate::error::Result;
use crate::manifest::rkyv_cache::{load_rkyv_file, load_rkyv_file_zst, save_rkyv_zst};
use crate::manifest::rkyv_types::{CachedSparseEmbeddings, RKYV_SCHEMA_VERSION};

const SPARSE_EMBEDDINGS_FILE_ZST: &str = "sparse_embeddings.rkyv.zst";
const SPARSE_EMBEDDINGS_FILE_RAW: &str = "sparse_embeddings.rkyv";

/// Persist sparse embeddings cache to disk.
///
/// # Errors
/// Returns an error if serialization, compression, or the write fails.
pub fn save_sparse_embeddings(cache: &CachedSparseEmbeddings, storage_dir: &Path) -> Result<()> {
    save_rkyv_zst(cache, &storage_dir.join(SPARSE_EMBEDDINGS_FILE_ZST))
}

#[must_use]
pub fn try_load_sparse_embeddings(
    storage_dir: &Path,
    expected_model: &str,
    expected_hash: &str,
    max_decompressed_bytes: u64,
) -> Option<CachedSparseEmbeddings> {
    let zst_path = storage_dir.join(SPARSE_EMBEDDINGS_FILE_ZST);
    if let Some(cache) = load_rkyv_file_zst(&zst_path, max_decompressed_bytes, |cache| {
        cache_valid(cache, expected_model, expected_hash)
    }) {
        return Some(cache);
    }

    let raw_path = storage_dir.join(SPARSE_EMBEDDINGS_FILE_RAW);
    if max_decompressed_bytes > 0
        && fs::metadata(&raw_path).map(|meta| meta.len()).unwrap_or(0) > max_decompressed_bytes
    {
        return None;
    }
    load_rkyv_file(&raw_path, |cache| {
        cache_valid(cache, expected_model, expected_hash)
    })
}

fn cache_valid(cache: &CachedSparseEmbeddings, expected_model: &str, expected_hash: &str) -> bool {
    cache.schema_version == RKYV_SCHEMA_VERSION
        && cache.model_name.as_str() == expected_model
        && cache.manifest_hash.as_str() == expected_hash
}
