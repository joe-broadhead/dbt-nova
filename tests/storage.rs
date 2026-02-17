//! Integration tests for entity store persistence.
use dbt_nova::manifest::store::EntityStore;
use dbt_nova::{DbtNovaConfig, ManifestSearch};
use std::fs;
use std::path::PathBuf;

#[path = "support/config.rs"]
mod support_config;

#[tokio::test(flavor = "multi_thread")]
async fn entity_store_persists_on_disk() {
    let guard = support_config::TestStorageGuard::new();
    let manifest_path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/nova_manifest.json");
    let mut cfg = DbtNovaConfig {
        manifest_path: manifest_path.to_string_lossy().to_string(),
        search: support_config::test_search_config(),
        ..Default::default()
    };
    support_config::apply_test_storage(&mut cfg, &guard);
    let searcher = ManifestSearch::new(cfg).expect("fixture manifest must be present");
    let key = "model.nova_test.int__campaign_features";
    let entity = searcher
        .get_entity(key)
        .await
        .expect("store lookup should succeed")
        .expect("entity should exist");
    assert_eq!(entity.name.as_deref(), Some("int__campaign_features"));
}

#[test]
fn missing_checksum_rejects_entity_store() {
    let guard = support_config::TestStorageGuard::new();
    let manifest_path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/nova_manifest.json");
    let mut cfg = DbtNovaConfig {
        manifest_path: manifest_path.to_string_lossy().to_string(),
        search: support_config::test_search_config(),
        ..Default::default()
    };
    support_config::apply_test_storage(&mut cfg, &guard);
    let _searcher = ManifestSearch::new(cfg.clone()).expect("fixture manifest must be present");

    let instance_root = cfg.storage_instance_root_dir().expect("instance root dir");
    let current_path = instance_root.join("manifest.current.json");
    let current: serde_json::Value =
        serde_json::from_slice(&fs::read(&current_path).expect("read current version"))
            .expect("parse current version");
    let version = current
        .get("version")
        .and_then(|v| v.as_str())
        .expect("current version");
    let storage_dir = instance_root.join("versions").join(version);
    let checksum_path = storage_dir.join("entities.checksum.json");
    fs::remove_file(&checksum_path).expect("remove checksum");

    let err = match EntityStore::open(&storage_dir) {
        Ok(_) => panic!("expected checksum error"),
        Err(err) => err,
    };
    assert!(
        err.to_string().contains("checksum file missing"),
        "unexpected error: {err}"
    );
}

#[test]
fn corrupted_entity_store_is_rejected() {
    let guard = support_config::TestStorageGuard::new();
    let manifest_path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/nova_manifest.json");
    let mut cfg = DbtNovaConfig {
        manifest_path: manifest_path.to_string_lossy().to_string(),
        search: support_config::test_search_config(),
        ..Default::default()
    };
    support_config::apply_test_storage(&mut cfg, &guard);
    let searcher = ManifestSearch::new(cfg.clone()).expect("fixture manifest must be present");

    let instance_root = cfg.storage_instance_root_dir().expect("instance root dir");
    let current_path = instance_root.join("manifest.current.json");
    let current: serde_json::Value =
        serde_json::from_slice(&fs::read(&current_path).expect("read current version"))
            .expect("parse current version");
    let version = current
        .get("version")
        .and_then(|v| v.as_str())
        .expect("current version");
    let storage_dir = instance_root.join("versions").join(version);
    drop(searcher);

    let data_path = storage_dir.join("entities.bin");
    let mut bytes = fs::read(&data_path).expect("read entities");
    bytes[0] = bytes[0].wrapping_add(1);
    fs::write(&data_path, &bytes).expect("write corrupted entities");

    let err = match EntityStore::open(&storage_dir) {
        Ok(_) => panic!("expected checksum mismatch"),
        Err(err) => err,
    };
    assert!(
        err.to_string().contains("checksum mismatch"),
        "unexpected error: {err}"
    );
}
