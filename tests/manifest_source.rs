//! Tests for manifest source providers (file/http/dbfs).
use std::fs;
use std::net::{Shutdown, TcpListener};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use dbt_nova::config::DbtNovaConfig;
use dbt_nova::manifest::source::resolve_manifest;
use tempfile::TempDir;

static ENV_MUTEX: OnceLock<Mutex<()>> = OnceLock::new();

fn lock_env() -> std::sync::MutexGuard<'static, ()> {
    ENV_MUTEX
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

#[test]
fn http_manifest_falls_back_to_cache_on_error() {
    let temp = TempDir::new().expect("temp dir");
    let uri = "http://127.0.0.1:1/manifest-test.json";

    let hash = blake3::hash(uri.as_bytes()).to_hex().to_string();
    let cache_path = temp.path().join(format!("{}.json", hash));
    let meta_path = temp.path().join(format!("{}.meta.json", hash));

    fs::write(&cache_path, b"{}").expect("cache write");
    let fetched_at_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let meta = serde_json::json!({
        "source_uri": uri,
        "fetched_at_ms": fetched_at_ms,
        "etag": null,
        "last_modified": null,
        "remote_modified_ms": null,
    });
    fs::write(&meta_path, serde_json::to_vec(&meta).unwrap()).expect("meta write");

    let cfg = DbtNovaConfig {
        manifest_uri: uri.to_string(),
        manifest_allow_http: true,
        manifest_cache_dir: temp.path().to_string_lossy().to_string(),
        manifest_refresh_secs: 300,
        ..Default::default()
    };

    let res = resolve_manifest(&cfg).expect("resolve manifest");
    assert!(res.cached);
    assert_eq!(res.local_path, cache_path);
}

#[test]
fn file_manifest_resolves_local_path() {
    let temp = TempDir::new().expect("temp dir");
    let manifest_path = temp.path().join("manifest-file.json");
    fs::write(&manifest_path, b"{}").expect("write manifest");

    let cfg = DbtNovaConfig {
        manifest_path: manifest_path.to_string_lossy().to_string(),
        manifest_uri: String::new(),
        ..Default::default()
    };

    let res = resolve_manifest(&cfg).expect("resolve manifest");
    assert!(!res.cached);
    assert_eq!(res.local_path, manifest_path);
}

#[test]
fn http_manifest_rejected_by_default() {
    let cfg = DbtNovaConfig {
        manifest_uri: "http://example.com/manifest-http-default-check.json".to_string(),
        ..Default::default()
    };

    let err = resolve_manifest(&cfg).expect_err("http should be disabled by default");
    assert!(
        err.to_string().contains(
            "http manifest URIs are disabled; use https:// or set DBT_NOVA_MANIFEST_ALLOW_HTTP=true"
        ),
        "unexpected error: {err}"
    );
}

#[test]
fn dbfs_manifest_uses_provider() {
    let _env_guard = lock_env();

    let temp = TempDir::new().expect("temp dir");
    let uri = "dbfs:///mnt/analytics/manifest-file.json";

    let hash = blake3::hash(uri.as_bytes()).to_hex().to_string();
    let cache_path = temp.path().join(format!("{}.json", hash));
    let meta_path = temp.path().join(format!("{}.meta.json", hash));

    fs::write(&cache_path, b"{}").expect("cache write");
    let fetched_at_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let meta = serde_json::json!({
        "source_uri": uri,
        "fetched_at_ms": fetched_at_ms,
        "etag": null,
        "last_modified": null,
        "remote_modified_ms": null,
    });
    fs::write(&meta_path, serde_json::to_vec(&meta).unwrap()).expect("meta write");

    unsafe {
        std::env::set_var("DATABRICKS_HOST", "https://example.databricks.com");
        std::env::set_var("DATABRICKS_ACCESS_TOKEN", "test-token");
    }

    let cfg = DbtNovaConfig {
        manifest_uri: uri.to_string(),
        manifest_cache_dir: temp.path().to_string_lossy().to_string(),
        manifest_refresh_secs: 300,
        ..Default::default()
    };

    let res = resolve_manifest(&cfg).expect("resolve manifest");
    assert!(res.cached);
    assert_eq!(res.local_path, cache_path);

    unsafe {
        std::env::remove_var("DATABRICKS_HOST");
        std::env::remove_var("DATABRICKS_ACCESS_TOKEN");
    }
}

#[test]
fn dbfs_manifest_rejects_legacy_token_env() {
    let _env_guard = lock_env();

    let temp = TempDir::new().expect("temp dir");
    let uri = "dbfs:///mnt/analytics/manifest-file.json";
    unsafe {
        std::env::set_var("DATABRICKS_HOST", "https://example.databricks.com");
        std::env::set_var("DATABRICKS_TOKEN", "legacy-token");
        std::env::remove_var("DATABRICKS_ACCESS_TOKEN");
    }

    let cfg = DbtNovaConfig {
        manifest_uri: uri.to_string(),
        manifest_cache_dir: temp.path().to_string_lossy().to_string(),
        manifest_refresh_secs: 300,
        ..Default::default()
    };

    let res = resolve_manifest(&cfg);
    assert!(res.is_err(), "legacy token env should be rejected");
    let err = res.expect_err("error");
    assert!(
        err.to_string().contains("DATABRICKS_ACCESS_TOKEN not set"),
        "unexpected error: {err}"
    );

    unsafe {
        std::env::remove_var("DATABRICKS_HOST");
        std::env::remove_var("DATABRICKS_TOKEN");
    }
}

#[test]
#[ignore = "requires local socket bind timing behavior; run explicitly in environments that allow loopback bind"]
fn http_manifest_times_out_without_cache() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral localhost port");
    let addr = listener.local_addr().expect("local addr");
    let handle = thread::spawn(move || {
        if let Ok((stream, _)) = listener.accept() {
            thread::sleep(Duration::from_millis(1500));
            let _ = stream.shutdown(Shutdown::Both);
        }
    });

    let temp = TempDir::new().expect("temp dir");
    let uri = format!("http://{}/manifest-timeout.json", addr);
    let cfg = DbtNovaConfig {
        manifest_uri: uri,
        manifest_allow_http: true,
        manifest_cache_dir: temp.path().to_string_lossy().to_string(),
        manifest_http_connect_timeout_secs: 1,
        manifest_http_timeout_secs: 1,
        manifest_fetch_timeout_secs: 1,
        ..Default::default()
    };

    let res = resolve_manifest(&cfg);
    assert!(res.is_err(), "expected timeout error");
    let _ = handle.join();
}
