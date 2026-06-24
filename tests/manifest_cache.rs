//! Tests for on-disk index cache persistence.
#[path = "support/config.rs"]
mod support_config;

use std::fs;
use std::path::PathBuf;

use dbt_nova::manifest::rkyv_cache::{CacheLoadFailure, save_rkyv};
use dbt_nova::manifest::rkyv_embeddings;
use dbt_nova::manifest::rkyv_sparse_embeddings;
use dbt_nova::manifest::rkyv_types::{
    CachedEmbeddings, CachedSparseEmbeddings, RKYV_SCHEMA_VERSION,
};
use dbt_nova::manifest::semantic_cache::{self, SemanticCacheComponent};
use dbt_nova::{DbtNovaConfig, ManifestSearch, config::SearchConfig};

#[test]
fn indexes_cache_persists_to_disk() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join("nova_manifest.json");
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let manifest_path = temp_dir.path().join("manifest.json");
    fs::copy(&fixture, &manifest_path).expect("fixture copy");

    let guard = support_config::TestStorageGuard::new();
    let mut cfg = DbtNovaConfig {
        manifest_path: manifest_path.to_string_lossy().to_string(),
        search: support_config::test_search_config(),
        ..Default::default()
    };
    support_config::apply_test_storage(&mut cfg, &guard);
    cfg.storage_read_only = false;

    let _searcher = ManifestSearch::new(cfg.clone())
        .expect("manifest search")
        .search;

    let instance_root = cfg.storage_instance_root_dir().expect("instance root dir");
    let current_path = instance_root.join("manifest.current.json");
    let current: serde_json::Value =
        serde_json::from_slice(&fs::read(&current_path).expect("read current version"))
            .expect("parse current version");
    let version = current
        .get("version")
        .and_then(|v| v.as_str())
        .expect("current version");

    let indexes_path = instance_root
        .join("versions")
        .join(version)
        .join("indexes.rkyv");
    assert!(
        indexes_path.exists(),
        "expected indexes cache at {}",
        indexes_path.display()
    );
}

#[test]
fn embeddings_cache_respects_decompression_limit() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let search = SearchConfig {
        embedding_cache_dir: temp_dir.path().to_string_lossy().to_string(),
        ..SearchConfig::default()
    };
    let cache = CachedEmbeddings {
        schema_version: RKYV_SCHEMA_VERSION,
        model_name: "test-model".to_string(),
        manifest_hash: "test-hash".to_string(),
        entity_ids: vec!["id-1".to_string()],
        dense_embeddings: vec![vec![0.0_f32; 512]],
        is_quantized: false,
        sparse_indices: None,
        sparse_values: None,
        ann_hyperplanes: None,
        ann_bucket_keys: None,
        ann_bucket_values: None,
    };
    rkyv_embeddings::save_embeddings(&cache, &search).expect("save embeddings");

    assert!(matches!(
        rkyv_embeddings::load_embeddings(&search, "test-model", "test-hash", Some(1), 1),
        rkyv_embeddings::EmbeddingsCacheLoad::Miss { .. }
    ));
}

#[test]
fn sparse_embeddings_cache_respects_decompression_limit() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let search = SearchConfig {
        embedding_cache_dir: temp_dir.path().to_string_lossy().to_string(),
        ..SearchConfig::default()
    };
    let cache = CachedSparseEmbeddings {
        schema_version: RKYV_SCHEMA_VERSION,
        model_name: "sparse-model".to_string(),
        manifest_hash: "sparse-hash".to_string(),
        entity_ids: vec!["id-1".to_string()],
        sparse_indices: vec![vec![1, 2, 3, 4, 5]],
        sparse_values: vec![vec![0.1_f32; 5]],
    };
    rkyv_sparse_embeddings::save_sparse_embeddings(&cache, &search)
        .expect("save sparse embeddings");

    assert!(matches!(
        rkyv_sparse_embeddings::load_sparse_embeddings(
            &search,
            "sparse-model",
            "sparse-hash",
            Some(1),
            1
        ),
        rkyv_sparse_embeddings::SparseEmbeddingsCacheLoad::Miss { .. }
    ));
}

#[test]
fn raw_embeddings_cache_respects_size_limit() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let search = SearchConfig {
        embedding_cache_dir: temp_dir.path().to_string_lossy().to_string(),
        ..SearchConfig::default()
    };
    let cache = CachedEmbeddings {
        schema_version: RKYV_SCHEMA_VERSION,
        model_name: "raw-model".to_string(),
        manifest_hash: "raw-hash".to_string(),
        entity_ids: vec!["id-1".to_string()],
        dense_embeddings: vec![vec![0.0_f32; 512]],
        is_quantized: false,
        sparse_indices: None,
        sparse_values: None,
        ann_hyperplanes: None,
        ann_bucket_keys: None,
        ann_bucket_values: None,
    };
    let paths = semantic_cache::cache_paths(
        &search,
        SemanticCacheComponent::Dense,
        "raw-model",
        "raw-hash",
    );
    save_rkyv(&cache, &paths.raw_path).expect("save raw embeddings");

    match rkyv_embeddings::load_embeddings(&search, "raw-model", "raw-hash", Some(1), 1) {
        rkyv_embeddings::EmbeddingsCacheLoad::Miss { failure, .. } => {
            assert!(matches!(
                failure,
                rkyv_embeddings::EmbeddingsCacheFailure::Load(CacheLoadFailure::TooLarge {
                    path,
                    max_bytes,
                    ..
                }) if path == paths.raw_path && max_bytes == 1
            ));
        }
        other => panic!("expected size-limited miss, got {other:?}"),
    }
}

#[test]
fn raw_sparse_embeddings_cache_respects_size_limit() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let search = SearchConfig {
        embedding_cache_dir: temp_dir.path().to_string_lossy().to_string(),
        ..SearchConfig::default()
    };
    let cache = CachedSparseEmbeddings {
        schema_version: RKYV_SCHEMA_VERSION,
        model_name: "raw-sparse-model".to_string(),
        manifest_hash: "raw-sparse-hash".to_string(),
        entity_ids: vec!["id-1".to_string()],
        sparse_indices: vec![vec![1, 2, 3, 4, 5]],
        sparse_values: vec![vec![0.1_f32; 5]],
    };
    let paths = semantic_cache::cache_paths(
        &search,
        SemanticCacheComponent::Sparse,
        "raw-sparse-model",
        "raw-sparse-hash",
    );
    save_rkyv(&cache, &paths.raw_path).expect("save raw sparse embeddings");

    match rkyv_sparse_embeddings::load_sparse_embeddings(
        &search,
        "raw-sparse-model",
        "raw-sparse-hash",
        Some(1),
        1,
    ) {
        rkyv_sparse_embeddings::SparseEmbeddingsCacheLoad::Miss { failure, .. } => {
            assert!(matches!(
                failure,
                rkyv_sparse_embeddings::SparseEmbeddingsCacheFailure::Load(
                    CacheLoadFailure::TooLarge {
                        path,
                        max_bytes,
                        ..
                    }
                ) if path == paths.raw_path && max_bytes == 1
            ));
        }
        other => panic!("expected size-limited miss, got {other:?}"),
    }
}

#[test]
fn embeddings_cache_rejects_incomplete_manifest_payload() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let search = SearchConfig {
        embedding_cache_dir: temp_dir.path().to_string_lossy().to_string(),
        ..SearchConfig::default()
    };
    let cache = CachedEmbeddings {
        schema_version: RKYV_SCHEMA_VERSION,
        model_name: "test-model".to_string(),
        manifest_hash: "test-hash".to_string(),
        entity_ids: vec!["id-1".to_string()],
        dense_embeddings: vec![vec![0.0_f32; 4]],
        is_quantized: false,
        sparse_indices: None,
        sparse_values: None,
        ann_hyperplanes: None,
        ann_bucket_keys: None,
        ann_bucket_values: None,
    };
    rkyv_embeddings::save_embeddings(&cache, &search).expect("save embeddings");

    match rkyv_embeddings::load_embeddings(&search, "test-model", "test-hash", Some(2), 1024) {
        rkyv_embeddings::EmbeddingsCacheLoad::Miss { failure, .. } => {
            assert!(matches!(
                failure,
                rkyv_embeddings::EmbeddingsCacheFailure::EntryCount {
                    expected: 2,
                    actual: 1,
                }
            ));
            assert!(failure.summary().contains("expected 2, got 1"));
        }
        other => panic!("expected incomplete cache miss, got {other:?}"),
    }
}

#[test]
fn sparse_embeddings_cache_rejects_incomplete_manifest_payload() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let search = SearchConfig {
        embedding_cache_dir: temp_dir.path().to_string_lossy().to_string(),
        ..SearchConfig::default()
    };
    let cache = CachedSparseEmbeddings {
        schema_version: RKYV_SCHEMA_VERSION,
        model_name: "sparse-model".to_string(),
        manifest_hash: "sparse-hash".to_string(),
        entity_ids: vec!["id-1".to_string()],
        sparse_indices: vec![vec![1, 2]],
        sparse_values: vec![vec![0.1_f32, 0.2_f32]],
    };
    rkyv_sparse_embeddings::save_sparse_embeddings(&cache, &search)
        .expect("save sparse embeddings");

    match rkyv_sparse_embeddings::load_sparse_embeddings(
        &search,
        "sparse-model",
        "sparse-hash",
        Some(2),
        1024,
    ) {
        rkyv_sparse_embeddings::SparseEmbeddingsCacheLoad::Miss { failure, .. } => {
            assert!(matches!(
                failure,
                rkyv_sparse_embeddings::SparseEmbeddingsCacheFailure::EntryCount {
                    expected: 2,
                    actual: 1,
                }
            ));
            assert!(failure.summary().contains("expected 2, got 1"));
        }
        other => panic!("expected incomplete sparse cache miss, got {other:?}"),
    }
}
