use crate::config::SearchConfig;
use crate::error::Result;
use crate::manifest::rkyv_cache::{
    CacheLoadFailure, load_rkyv_file_limited, load_rkyv_file_zst, save_rkyv_zst,
};
use crate::manifest::rkyv_types::{CachedSparseEmbeddings, RKYV_SCHEMA_VERSION};
use crate::manifest::semantic_cache::{self, SemanticCacheComponent, SemanticCachePaths};

#[derive(Debug, Clone)]
pub enum SparseEmbeddingsCacheLoad {
    Hit {
        cache: CachedSparseEmbeddings,
        paths: SemanticCachePaths,
    },
    Miss {
        paths: SemanticCachePaths,
        failure: SparseEmbeddingsCacheFailure,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SparseEmbeddingsCacheFailure {
    Load(CacheLoadFailure),
    SchemaVersion { expected: u32, actual: u32 },
    ModelName { expected: String, actual: String },
    ManifestHash { expected: String, actual: String },
}

impl SparseEmbeddingsCacheFailure {
    #[must_use]
    pub fn summary(&self) -> String {
        match self {
            Self::Load(failure) => failure.summary(),
            Self::SchemaVersion { expected, actual } => {
                format!("cache schema version mismatch (expected {expected}, got {actual})")
            }
            Self::ModelName { expected, actual } => {
                format!("cache model mismatch (expected {expected}, got {actual})")
            }
            Self::ManifestHash { expected, actual } => {
                format!("cache manifest hash mismatch (expected {expected}, got {actual})")
            }
        }
    }
}

/// Persist sparse embeddings cache to disk.
///
/// # Errors
/// Returns an error if serialization, compression, or the write fails.
pub fn save_sparse_embeddings(cache: &CachedSparseEmbeddings, config: &SearchConfig) -> Result<()> {
    let paths = semantic_cache::cache_paths(
        config,
        SemanticCacheComponent::Sparse,
        &cache.model_name,
        &cache.manifest_hash,
    );
    save_rkyv_zst(cache, &paths.compressed_path)
}

#[must_use]
pub fn load_sparse_embeddings(
    config: &SearchConfig,
    expected_model: &str,
    expected_hash: &str,
    max_decompressed_bytes: u64,
) -> SparseEmbeddingsCacheLoad {
    let paths = semantic_cache::cache_paths(
        config,
        SemanticCacheComponent::Sparse,
        expected_model,
        expected_hash,
    );
    semantic_cache::warn_if_legacy_cache_present(&paths, SemanticCacheComponent::Sparse);

    let mut first_failure = None;
    match load_rkyv_file_zst(&paths.compressed_path, max_decompressed_bytes) {
        Ok(cache) => match validate_cache(&cache, expected_model, expected_hash) {
            Ok(()) => return SparseEmbeddingsCacheLoad::Hit { cache, paths },
            Err(failure) => first_failure = Some(failure),
        },
        Err(CacheLoadFailure::Missing { .. }) => {}
        Err(failure) => first_failure = Some(SparseEmbeddingsCacheFailure::Load(failure)),
    }

    match load_rkyv_file_limited(&paths.raw_path, max_decompressed_bytes) {
        Ok(cache) => match validate_cache(&cache, expected_model, expected_hash) {
            Ok(()) => SparseEmbeddingsCacheLoad::Hit { cache, paths },
            Err(failure) => SparseEmbeddingsCacheLoad::Miss { paths, failure },
        },
        Err(CacheLoadFailure::Missing { .. }) => SparseEmbeddingsCacheLoad::Miss {
            paths,
            failure: first_failure.unwrap_or_else(|| {
                SparseEmbeddingsCacheFailure::Load(CacheLoadFailure::Missing {
                    path: semantic_cache::cache_paths(
                        config,
                        SemanticCacheComponent::Sparse,
                        expected_model,
                        expected_hash,
                    )
                    .compressed_path,
                })
            }),
        },
        Err(failure) => SparseEmbeddingsCacheLoad::Miss {
            paths,
            failure: SparseEmbeddingsCacheFailure::Load(failure),
        },
    }
}

fn validate_cache(
    cache: &CachedSparseEmbeddings,
    expected_model: &str,
    expected_hash: &str,
) -> std::result::Result<(), SparseEmbeddingsCacheFailure> {
    if cache.schema_version != RKYV_SCHEMA_VERSION {
        return Err(SparseEmbeddingsCacheFailure::SchemaVersion {
            expected: RKYV_SCHEMA_VERSION,
            actual: cache.schema_version,
        });
    }
    if cache.model_name.as_str() != expected_model {
        return Err(SparseEmbeddingsCacheFailure::ModelName {
            expected: expected_model.to_string(),
            actual: cache.model_name.clone(),
        });
    }
    if cache.manifest_hash.as_str() != expected_hash {
        return Err(SparseEmbeddingsCacheFailure::ManifestHash {
            expected: expected_hash.to_string(),
            actual: cache.manifest_hash.clone(),
        });
    }
    Ok(())
}
