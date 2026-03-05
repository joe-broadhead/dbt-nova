use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use tempfile::TempDir;

use crate::config::DbtNovaConfig;

#[test]
fn dbfs_read_field_helpers_default_missing_values() {
    let body = serde_json::json!({});
    assert_eq!(dbfs_read_data_field(&body, 7), "");
    assert_eq!(dbfs_read_bytes_read_field(&body, 7), 0);
}

#[test]
fn dbfs_read_field_helpers_use_present_values() {
    let body = serde_json::json!({
        "data": "YWJj",
        "bytes_read": 3
    });
    assert_eq!(dbfs_read_data_field(&body, 0), "YWJj");
    assert_eq!(dbfs_read_bytes_read_field(&body, 0), 3);
}

#[cfg(any(feature = "s3", feature = "gcs"))]
fn sdk_fetch_test_config(cache_dir: &TempDir) -> DbtNovaConfig {
    DbtNovaConfig {
        manifest_cache_dir: cache_dir.path().to_string_lossy().to_string(),
        manifest_refresh_secs: 3600,
        manifest_max_bytes: 1024 * 1024,
        manifest_http_timeout_secs: 0,
        manifest_fetch_timeout_secs: 30,
        ..DbtNovaConfig::default()
    }
}

#[cfg(any(feature = "s3", feature = "gcs"))]
#[test]
fn sdk_fetch_pipeline_uses_fresh_cache_without_invoking_fetcher() {
    let cache_dir = TempDir::new().expect("temp dir");
    let config = sdk_fetch_test_config(&cache_dir);
    let uri = "s3://bucket/path/manifest.json";
    let (cache_path, meta_path) = cache_paths(&config, uri).expect("cache paths");
    write_atomic(&cache_path, br#"{"cached":true}"#).expect("write cache");
    write_cache_meta(
        &meta_path,
        &CacheMeta {
            source_uri: uri.to_string(),
            fetched_at_ms: now_ms(),
            ..CacheMeta::default()
        },
    )
    .expect("write meta");

    let fetch_calls = AtomicUsize::new(0);
    let resolution = fetch_sdk_manifest_with_cache("S3", uri, &config, || {
        fetch_calls.fetch_add(1, Ordering::SeqCst);
        Err(DbtNovaError::ServerError(
            "fetch should not be called for fresh cache".to_string(),
        ))
    })
    .expect("expected fresh cache resolution");

    assert!(resolution.cached, "expected cache hit");
    assert_eq!(fetch_calls.load(Ordering::SeqCst), 0);
    assert_eq!(resolution.local_path, cache_path);
}

#[cfg(any(feature = "s3", feature = "gcs"))]
#[test]
fn sdk_fetch_pipeline_retries_and_persists_successful_download() {
    let cache_dir = TempDir::new().expect("temp dir");
    let mut config = sdk_fetch_test_config(&cache_dir);
    config.manifest_refresh_secs = 1;
    let uri = "gs://bucket/path/manifest.json";

    let fetch_calls = AtomicUsize::new(0);
    let resolution = fetch_sdk_manifest_with_cache("GCS", uri, &config, || {
        let call = fetch_calls.fetch_add(1, Ordering::SeqCst);
        if call == 0 {
            Err(DbtNovaError::ServerError("transient failure".to_string()))
        } else {
            Ok(br#"{"downloaded":true}"#.to_vec())
        }
    })
    .expect("expected successful fetch after retry");

    assert!(!resolution.cached, "expected fresh materialization");
    assert_eq!(fetch_calls.load(Ordering::SeqCst), 2);
    let stored = fs::read_to_string(&resolution.local_path).expect("read cache");
    assert!(stored.contains("\"downloaded\":true"));
}

#[cfg(any(feature = "s3", feature = "gcs"))]
#[test]
fn sdk_fetch_pipeline_falls_back_to_cache_on_size_limit() {
    let cache_dir = TempDir::new().expect("temp dir");
    let mut config = sdk_fetch_test_config(&cache_dir);
    config.manifest_max_bytes = 4;
    config.manifest_refresh_secs = 1;
    let uri = "s3://bucket/path/manifest.json";
    let (cache_path, meta_path) = cache_paths(&config, uri).expect("cache paths");
    write_atomic(&cache_path, br"{}").expect("write cache");
    write_cache_meta(
        &meta_path,
        &CacheMeta {
            source_uri: uri.to_string(),
            fetched_at_ms: 0,
            ..CacheMeta::default()
        },
    )
    .expect("write meta");

    let resolution =
        fetch_sdk_manifest_with_cache("S3", uri, &config, || Ok(br#"{"too_large":true}"#.to_vec()))
            .expect("expected cached fallback");

    assert!(resolution.cached, "expected fallback to existing cache");
    let stored = fs::read_to_string(&resolution.local_path).expect("read cache");
    assert_eq!(stored, "{}");
}
