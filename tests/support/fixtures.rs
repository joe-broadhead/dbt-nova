use std::ops::Deref;
use std::path::PathBuf;

use dbt_nova::error::Result;
use dbt_nova::{DbtNovaConfig, ManifestSearch};

use crate::support_config::{TestStorageGuard, apply_test_storage, test_search_config};

pub struct FixtureSearchEnv {
    searcher: ManifestSearch,
    _guard: TestStorageGuard,
}

impl Deref for FixtureSearchEnv {
    type Target = ManifestSearch;

    fn deref(&self) -> &Self::Target {
        &self.searcher
    }
}

pub fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

pub fn load_fixture(name: &str) -> Result<FixtureSearchEnv> {
    let guard = TestStorageGuard::new();
    let mut cfg = DbtNovaConfig {
        manifest_path: fixture_path(name).to_string_lossy().to_string(),
        search: test_search_config(),
        ..Default::default()
    };
    apply_test_storage(&mut cfg, &guard);
    let searcher = ManifestSearch::new(cfg)?.search;
    Ok(FixtureSearchEnv {
        searcher,
        _guard: guard,
    })
}
