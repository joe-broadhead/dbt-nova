use std::path::Path;

use crate::error::Result;
use crate::manifest::rkyv_cache::{load_rkyv_file, save_rkyv};
use crate::manifest::rkyv_types::{PersistedIndexes, RKYV_SCHEMA_VERSION};

const INDEXES_FILE: &str = "indexes.rkyv";

/// Persist computed indexes to disk.
///
/// # Errors
/// Returns an error if serialization or writing the cache fails.
pub fn save_indexes(indexes: &PersistedIndexes, storage_dir: &Path) -> Result<()> {
    let path = storage_dir.join(INDEXES_FILE);
    save_rkyv(indexes, &path)
}

#[must_use]
pub fn try_load_indexes(storage_dir: &Path, expected_hash: &str) -> Option<PersistedIndexes> {
    let path = storage_dir.join(INDEXES_FILE);
    load_rkyv_file::<PersistedIndexes, _>(&path, |cache| {
        cache.schema_version == RKYV_SCHEMA_VERSION && cache.manifest_hash.as_str() == expected_hash
    })
}
