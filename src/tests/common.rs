//! Shared helpers for test setup and fixture loading.
pub use crate::error::DbtNovaError;
pub use crate::params::*;
pub use crate::utils::{levenshtein_distance, levenshtein_similarity};
pub use crate::{DbtNovaConfig, ManifestSearch};
pub use serde_json::Value as JsonValue;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use tempfile::TempDir;
pub fn unwrap_json(res: Result<JsonValue, DbtNovaError>) -> JsonValue {
    res.unwrap_or_else(|e| e.to_response())
}
pub trait ResultJsonExt {
    fn json(self) -> JsonValue;
}
impl ResultJsonExt for Result<JsonValue, DbtNovaError> {
    fn json(self) -> JsonValue {
        unwrap_json(self)
    }
}
pub struct TestSearchEnv {
    searcher: ManifestSearch,
    _guard: TempDir,
}

impl Deref for TestSearchEnv {
    type Target = ManifestSearch;

    fn deref(&self) -> &Self::Target {
        &self.searcher
    }
}

pub fn get_searcher() -> TestSearchEnv {
    let manifest_path = fixture_manifest_path();
    get_searcher_for_manifest(&manifest_path)
}

fn fixture_manifest_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/nova_manifest.json")
}

pub fn get_searcher_with_fixture(fixture_name: &str) -> TestSearchEnv {
    let manifest_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/fixtures/{fixture_name}"));
    get_searcher_for_manifest(&manifest_path)
}

fn get_searcher_for_manifest(manifest_path: &Path) -> TestSearchEnv {
    assert!(
        manifest_path.exists(),
        "fixture manifest not found at {}",
        manifest_path.display()
    );
    let guard = TempDir::new().expect("test temp dir");
    let mut cfg = crate::DbtNovaConfig {
        manifest_path: manifest_path.to_string_lossy().to_string(),
        search: crate::config::SearchConfig {
            enable_vector_search: false,
            enable_sparse_search: false,
            enable_reranker: false,
            ..Default::default()
        },
        ..Default::default()
    };
    cfg.storage_dir = guard.path().to_string_lossy().to_string();
    cfg.storage_instance_id = "tests".to_string();
    cfg.storage_max_instances = 1;
    cfg.cleanup_storage_on_start = true;
    let searcher = ManifestSearch::new(cfg).expect("Failed to load fixture manifest");
    TestSearchEnv {
        searcher,
        _guard: guard,
    }
}
