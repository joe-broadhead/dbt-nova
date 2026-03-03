//! Stress tests for concurrency and stability.
#[path = "support/config.rs"]
mod support_config;

use std::path::PathBuf;
use std::sync::Arc;

use dbt_nova::params::PaginationParams;
use dbt_nova::params::{DetailLevel, SearchParams};
use dbt_nova::{DbtNovaConfig, ManifestSearchHandle};
use tokio::task::JoinSet;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_searches() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join("nova_manifest.json");
    let guard = support_config::TestStorageGuard::new();
    let mut cfg = DbtNovaConfig {
        manifest_path: fixture.to_string_lossy().to_string(),
        search: support_config::test_search_config(),
        ..Default::default()
    };
    support_config::apply_test_storage(&mut cfg, &guard);

    // Spawn in background to simulate real service startup behavior.
    let handle = ManifestSearchHandle::spawn(cfg);
    let searcher = handle.wait_ready().await.expect("searcher ready");
    let searcher = Arc::new(searcher);

    let mut tasks = JoinSet::new();
    let total_requests = 50usize;
    // Fire a burst of concurrent searches to catch contention or locking issues.
    for i in 0..total_requests {
        let searcher = Arc::clone(&searcher);
        tasks.spawn(async move {
            let params = SearchParams {
                query: format!("model {}", i),
                detail: DetailLevel::Standard,
                pagination: PaginationParams {
                    limit: 10,
                    offset: 0,
                },
                ..Default::default()
            };
            searcher.search(&params).await.is_ok()
        });
    }

    let mut successes = 0;
    while let Some(result) = tasks.join_next().await {
        if let Ok(true) = result {
            successes += 1;
        }
    }

    assert!(
        successes == total_requests,
        "expected all concurrent searches to succeed ({total_requests}), got {successes}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rapid_health_calls() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join("nova_manifest.json");
    let guard = support_config::TestStorageGuard::new();
    let mut cfg = DbtNovaConfig {
        manifest_path: fixture.to_string_lossy().to_string(),
        search: support_config::test_search_config(),
        ..Default::default()
    };
    support_config::apply_test_storage(&mut cfg, &guard);

    // Rapid health polling should remain stable under repeated calls.
    let handle = ManifestSearchHandle::spawn(cfg);
    let searcher = handle.wait_ready().await.expect("searcher ready");

    // Warm the health path to exercise snapshot caching and concurrency.
    for _ in 0..100 {
        let _ = searcher.health_snapshot().await;
    }

    let health = searcher.health_snapshot().await;
    assert!(health.get("manifest").is_some());
    assert!(health.get("manifest_health").is_some());
}
