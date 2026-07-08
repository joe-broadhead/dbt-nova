use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{LazyLock, Mutex};

use super::{
    AnnIndex, DeferredInit, build_optional_component, model_repo_dir,
    refuse_incomplete_cache_build, run_component_operation, snapshot_dir_from_repo_dir,
    validate_local_model_files, validate_proxy_env_vars,
};
use crate::config::{SearchColdStartPolicy, SearchConfig};
use tempfile::TempDir;

static ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

fn write_file(root: &Path, relative_path: &str) -> PathBuf {
    let path = root.join(relative_path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("parent dir");
    }
    fs::write(&path, b"test").expect("write file");
    path
}

fn with_proxy_env<F>(key: &str, value: Option<&str>, f: F)
where
    F: FnOnce(),
{
    let _guard = ENV_LOCK.lock().expect("env lock");
    let original = std::env::var_os(key);
    unsafe {
        match value {
            Some(next) => std::env::set_var(key, next),
            None => std::env::remove_var(key),
        }
    }
    f();
    unsafe {
        match original {
            Some(previous) => std::env::set_var(key, previous),
            None => std::env::remove_var(key),
        }
    }
}

#[test]
fn proxy_validation_accepts_absolute_url() {
    with_proxy_env("HTTPS_PROXY", Some("http://proxy.internal:8080"), || {
        let result = validate_proxy_env_vars();
        assert!(result.is_ok());
    });
}

#[test]
fn proxy_validation_rejects_non_url_values() {
    with_proxy_env("HTTPS_PROXY", Some("proxy.internal:8080"), || {
        let result = validate_proxy_env_vars();
        assert!(result.is_err());
        let error_text = result
            .expect_err("invalid proxy should return error")
            .to_string();
        assert!(error_text.contains("Invalid proxy environment variable 'HTTPS_PROXY'"));
    });
}

#[test]
fn build_optional_component_converts_panics_into_disabled_warnings() {
    let result = build_optional_component::<(), String, _>(
        "vector search",
        "DBT_NOVA_SEARCH_ENABLE_VECTOR",
        || panic!("missing onnx/model.onnx"),
    );
    let (component, warning) = result.into_parts();
    assert!(component.is_none());
    let warning = warning.expect("panic should produce warning");
    assert!(warning.contains("vector search initialization failed"));
    assert!(warning.contains("DBT_NOVA_EMBEDDINGS_CACHE_DIR"));
    assert!(warning.contains("DBT_NOVA_SEARCH_ENABLE_VECTOR=false"));
    assert!(warning.contains("missing onnx/model.onnx"));
}

#[test]
fn run_component_operation_converts_panics_into_server_errors() {
    let error = run_component_operation::<(), String, _>(
        "sparse search",
        "embedding generation",
        "DBT_NOVA_SEARCH_ENABLE_SPARSE",
        || panic!("failed to retrieve model.onnx"),
    )
    .expect_err("panic should surface as server error");
    let message = error.to_string();
    assert!(message.contains("sparse search embedding generation failed"));
    assert!(message.contains("DBT_NOVA_EMBEDDINGS_CACHE_DIR"));
    assert!(message.contains("DBT_NOVA_SEARCH_ENABLE_SPARSE=false"));
    assert!(message.contains("failed to retrieve model.onnx"));
}

#[test]
fn refuse_incomplete_cache_build_degrades_when_batches_fail() {
    let disabled = refuse_incomplete_cache_build::<()>(
        "vector search",
        "DBT_NOVA_SEARCH_ENABLE_VECTOR",
        2,
        0,
        Some("missing onnx/model.onnx"),
        SearchColdStartPolicy::Degrade,
    )
    .expect("degrade policy should not error")
    .expect("all failed batches should disable component");
    let (component, warning) = disabled.into_parts();
    assert!(component.is_none());
    let warning = warning.expect("warning");
    assert!(warning.contains("incomplete manifest-scoped cache payload"));
    assert!(warning.contains("expected 2 entries, produced 0"));
    assert!(warning.contains("missing onnx/model.onnx"));
}

#[test]
fn refuse_incomplete_cache_build_errors_in_build_policy() {
    let error = refuse_incomplete_cache_build::<()>(
        "sparse search",
        "DBT_NOVA_SEARCH_ENABLE_SPARSE",
        2,
        1,
        Some("failed to retrieve model.onnx"),
        SearchColdStartPolicy::Build,
    )
    .expect_err("build policy should fail incomplete cache payloads");
    let message = error.to_string();
    assert!(message.contains("incomplete manifest-scoped cache payload"));
    assert!(message.contains("expected 2 entries, produced 1"));
    assert!(message.contains("failed to retrieve model.onnx"));
}

#[test]
fn ann_index_build_is_deterministic_for_same_inputs() {
    let config = SearchConfig {
        enable_vector_ann: true,
        vector_ann_bits: 8,
        vector_ann_hamming: 1,
        ..SearchConfig::default()
    };
    let embeddings = vec![
        ("model.pkg.orders".to_string(), vec![1.0, 0.0, 0.0]),
        ("model.pkg.customers".to_string(), vec![0.0, 1.0, 0.0]),
        ("model.pkg.payments".to_string(), vec![0.0, 0.0, 1.0]),
    ];

    let first = AnnIndex::build_f32(&embeddings, &config).expect("first ann");
    let second = AnnIndex::build_f32(&embeddings, &config).expect("second ann");

    assert_eq!(first.hyperplanes, second.hyperplanes);
    assert_eq!(first.buckets, second.buckets);
}

#[test]
fn deferred_init_retries_after_failure_and_caches_success() {
    let deferred = DeferredInit::new();
    let attempts = AtomicUsize::new(0);

    let first_error = deferred
        .get_or_try_init(|| {
            attempts.fetch_add(1, Ordering::Relaxed);
            Err(crate::error::DbtNovaError::ServerError(
                "first failure".to_string(),
            ))
        })
        .expect_err("first init should fail");
    assert!(first_error.to_string().contains("first failure"));
    assert!(!deferred.initialized());

    let value = deferred
        .get_or_try_init(|| {
            attempts.fetch_add(1, Ordering::Relaxed);
            Ok(42usize)
        })
        .expect("second init should succeed");
    assert_eq!(*value, 42);
    assert!(deferred.initialized());

    let cached = deferred
        .get_or_try_init(|| {
            attempts.fetch_add(1, Ordering::Relaxed);
            Ok(99usize)
        })
        .expect("cached init should reuse existing value");
    assert_eq!(*cached, 42);
    assert_eq!(attempts.load(Ordering::Relaxed), 2);
}

#[test]
fn snapshot_dir_prefers_ref_target_when_both_ref_and_main_snapshot_exist() {
    let temp_dir = TempDir::new().expect("temp dir");
    let repo_dir = model_repo_dir(temp_dir.path(), "owner/model");
    fs::create_dir_all(repo_dir.join("refs")).expect("refs dir");
    fs::write(repo_dir.join("refs/main"), "commit123").expect("write ref");
    fs::create_dir_all(repo_dir.join("snapshots/main")).expect("main snapshot");
    fs::create_dir_all(repo_dir.join("snapshots/commit123")).expect("commit snapshot");

    let snapshot_dir = snapshot_dir_from_repo_dir(&repo_dir).expect("snapshot dir");
    assert_eq!(snapshot_dir, repo_dir.join("snapshots/commit123"));
}

#[test]
fn snapshot_dir_uses_ref_target_when_main_snapshot_missing() {
    let temp_dir = TempDir::new().expect("temp dir");
    let repo_dir = model_repo_dir(temp_dir.path(), "owner/model");
    fs::create_dir_all(repo_dir.join("refs")).expect("refs dir");
    fs::write(repo_dir.join("refs/main"), "commit456").expect("write ref");
    fs::create_dir_all(repo_dir.join("snapshots/commit456")).expect("commit snapshot");

    let snapshot_dir = snapshot_dir_from_repo_dir(&repo_dir).expect("snapshot dir");
    assert_eq!(snapshot_dir, repo_dir.join("snapshots/commit456"));
}

#[test]
fn validate_local_model_files_requires_tokenizer_and_model_files() {
    let temp_dir = TempDir::new().expect("temp dir");
    let repo_dir = model_repo_dir(temp_dir.path(), "owner/model");
    fs::create_dir_all(repo_dir.join("refs")).expect("refs dir");
    fs::write(repo_dir.join("refs/main"), "main").expect("write ref");
    let snapshot_dir = repo_dir.join("snapshots/main");
    fs::create_dir_all(&snapshot_dir).expect("snapshot dir");

    write_file(&snapshot_dir, "onnx/model.onnx");
    write_file(&snapshot_dir, "tokenizer.json");
    write_file(&snapshot_dir, "config.json");
    write_file(&snapshot_dir, "special_tokens_map.json");

    let error = validate_local_model_files(temp_dir.path(), "owner/model", "onnx/model.onnx", &[])
        .expect_err("missing tokenizer_config.json should fail");
    assert!(error.to_string().contains("tokenizer_config.json"));

    write_file(&snapshot_dir, "tokenizer_config.json");
    validate_local_model_files(temp_dir.path(), "owner/model", "onnx/model.onnx", &[])
        .expect("all required files should validate");
}
