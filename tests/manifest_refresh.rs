//! Tests for manifest refresh detection and swap behavior.
#[path = "support/config.rs"]
mod support_config;

use std::fs;
use std::time::{Duration, Instant};

use dbt_nova::manifest::search::ManifestStatus;
use dbt_nova::{DbtNovaConfig, ManifestSearchHandle};
use serde_json::Value as JsonValue;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn manifest_refresh_detects_change_and_swaps() {
    let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
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
    cfg.manifest_refresh_secs = 1;
    cfg.storage_read_only = false;

    let handle = ManifestSearchHandle::spawn(cfg);
    let searcher = handle.wait_ready().await.expect("searcher ready");
    let before = searcher.health_snapshot().await;
    let before_hash = before
        .get("manifest")
        .and_then(|v| v.get("hash"))
        .and_then(|v| v.as_str())
        .expect("manifest hash")
        .to_string();

    let mut manifest: JsonValue =
        serde_json::from_slice(&fs::read(&manifest_path).expect("read manifest"))
            .expect("parse manifest");
    if let Some(obj) = manifest.as_object_mut() {
        obj.insert(
            "generated_at".to_string(),
            JsonValue::String("2099-01-01T00:00:00Z".to_string()),
        );
    }
    fs::write(
        &manifest_path,
        serde_json::to_vec(&manifest).expect("encode manifest"),
    )
    .expect("write manifest");

    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let searcher = handle.get().await.expect("active searcher");
        let current = searcher.health_snapshot().await;
        let current_hash = current
            .get("manifest")
            .and_then(|v| v.get("hash"))
            .and_then(|v| v.as_str())
            .expect("current hash")
            .to_string();
        if current_hash != before_hash {
            break;
        }
        if Instant::now() > deadline {
            panic!("manifest refresh did not swap in time");
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn manifest_refresh_recovers_from_initial_failure() {
    let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join("nova_manifest.json");
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let manifest_path = temp_dir.path().join("manifest.json");
    std::fs::write(&manifest_path, b"{ this is not json }").expect("invalid manifest");

    let guard = support_config::TestStorageGuard::new();
    let mut cfg = DbtNovaConfig {
        manifest_path: manifest_path.to_string_lossy().to_string(),
        search: support_config::test_search_config(),
        manifest_refresh_secs: 1,
        storage_read_only: false,
        ..Default::default()
    };
    support_config::apply_test_storage(&mut cfg, &guard);

    let handle = ManifestSearchHandle::spawn(cfg);

    let fail_deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match handle.status().await {
            ManifestStatus::Failed { .. } => break,
            ManifestStatus::Loading { .. } => {}
            ManifestStatus::Refreshing { .. } => {}
            ManifestStatus::Ready { .. } => {}
        }
        assert!(
            Instant::now() < fail_deadline,
            "manifest did not enter failed state after startup"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    std::fs::copy(&fixture, &manifest_path).expect("restore valid manifest");

    let ready_deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let ManifestStatus::Ready { entity_count } = handle.status().await {
            assert!(entity_count > 0);
            return;
        }
        assert!(
            Instant::now() < ready_deadline,
            "manifest did not recover from initial failure"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
