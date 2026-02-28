//! Tests for on-disk index cache persistence.
#[path = "support/config.rs"]
mod support_config;

use std::fs;
use std::path::PathBuf;

use dbt_nova::manifest::rkyv_embeddings;
use dbt_nova::manifest::rkyv_sparse_embeddings;
use dbt_nova::manifest::rkyv_types::{
    CachedEmbeddings, CachedSparseEmbeddings, RKYV_SCHEMA_VERSION,
};
use dbt_nova::{DbtNovaConfig, ManifestSearch};

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
    rkyv_embeddings::save_embeddings(&cache, temp_dir.path()).expect("save embeddings");

    let loaded =
        rkyv_embeddings::try_load_embeddings(temp_dir.path(), "test-model", "test-hash", 1);
    assert!(loaded.is_none(), "expected cache to be rejected");
}

#[test]
fn sparse_embeddings_cache_respects_decompression_limit() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let cache = CachedSparseEmbeddings {
        schema_version: RKYV_SCHEMA_VERSION,
        model_name: "sparse-model".to_string(),
        manifest_hash: "sparse-hash".to_string(),
        entity_ids: vec!["id-1".to_string()],
        sparse_indices: vec![vec![1, 2, 3, 4, 5]],
        sparse_values: vec![vec![0.1_f32; 5]],
    };
    rkyv_sparse_embeddings::save_sparse_embeddings(&cache, temp_dir.path())
        .expect("save sparse embeddings");

    let loaded = rkyv_sparse_embeddings::try_load_sparse_embeddings(
        temp_dir.path(),
        "sparse-model",
        "sparse-hash",
        1,
    );
    assert!(loaded.is_none(), "expected cache to be rejected");
}
