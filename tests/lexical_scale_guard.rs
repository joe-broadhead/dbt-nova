//! Bounded lexical-mode scale guard for agent-critical manifest paths.
//!
//! Run with `scripts/check_lexical_scale.sh`. The test is ignored by default so
//! normal `cargo test --all-features` runs do not inherit timing variance.

#[path = "support/config.rs"]
mod support_config;
#[path = "support/synthetic_manifest.rs"]
mod synthetic_manifest;

use std::path::Path;
use std::time::{Duration, Instant};

use dbt_nova::config::SearchConfig;
use dbt_nova::params::{
    ColumnInventoryParams, ContextLimits, ContextMode, DetailLevel, GetContextParams,
    IndicatorInventoryParams, PaginationParams, SearchParams,
};
use dbt_nova::{DbtNovaConfig, ManifestSearch};
use serde_json::Value as JsonValue;
use synthetic_manifest::{SyntheticManifestConfig, write_synthetic_manifest};

struct LoadedSearch {
    searcher: ManifestSearch,
    _guard: support_config::TestStorageGuard,
}

struct ScaleRunConfig {
    manifest: SyntheticManifestConfig,
    load_iterations: usize,
    runtime_iterations: usize,
}

impl ScaleRunConfig {
    fn from_env() -> Self {
        Self {
            manifest: SyntheticManifestConfig {
                models: env_usize("DBT_NOVA_SCALE_MODELS", 300),
                packages: env_usize("DBT_NOVA_SCALE_PACKAGES", 3),
                columns_per_model: env_usize("DBT_NOVA_SCALE_COLUMNS", 8),
                ref_fanout: env_usize("DBT_NOVA_SCALE_REF_FANOUT", 2),
                metric_every: env_usize("DBT_NOVA_SCALE_METRIC_EVERY", 10),
            },
            load_iterations: env_usize("DBT_NOVA_SCALE_LOAD_ITERATIONS", 2),
            runtime_iterations: env_usize("DBT_NOVA_SCALE_ITERATIONS", 5),
        }
    }
}

struct ScaleThresholds {
    load_p95_ms: u128,
    search_p95_ms: u128,
    indicator_inventory_p95_ms: u128,
    column_inventory_p95_ms: u128,
    context_p95_ms: u128,
    health_p95_ms: u128,
}

impl ScaleThresholds {
    fn from_env() -> Self {
        Self {
            load_p95_ms: env_u128("DBT_NOVA_SCALE_MAX_LOAD_P95_MS", 30_000),
            search_p95_ms: env_u128("DBT_NOVA_SCALE_MAX_SEARCH_P95_MS", 2_500),
            indicator_inventory_p95_ms: env_u128(
                "DBT_NOVA_SCALE_MAX_INDICATOR_INVENTORY_P95_MS",
                2_500,
            ),
            column_inventory_p95_ms: env_u128("DBT_NOVA_SCALE_MAX_COLUMN_INVENTORY_P95_MS", 2_500),
            context_p95_ms: env_u128("DBT_NOVA_SCALE_MAX_CONTEXT_P95_MS", 2_500),
            health_p95_ms: env_u128("DBT_NOVA_SCALE_MAX_HEALTH_P95_MS", 1_000),
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "run scripts/check_lexical_scale.sh for the bounded CI scale guard"]
async fn default_lexical_manifest_paths_stay_within_scale_guard() {
    let run = ScaleRunConfig::from_env();
    let thresholds = ScaleThresholds::from_env();
    let workspace = tempfile::tempdir().expect("scale workspace");
    let manifest_path = workspace.path().join("synthetic-scale-input.json");
    let summary = write_synthetic_manifest(&manifest_path, run.manifest)
        .expect("write synthetic scale manifest");

    let load_samples = measure_loads(&manifest_path, run.load_iterations, summary.models);
    let loaded = load_searcher(&manifest_path);
    assert_eq!(loaded.searcher.entity_count(), summary.models);

    let search_samples = measure_search(
        &loaded.searcher,
        run.runtime_iterations,
        "revenue customer orders",
    )
    .await;
    let indicator_inventory_samples =
        measure_indicator_inventory(&loaded.searcher, run.runtime_iterations).await;
    let column_inventory_samples =
        measure_column_inventory(&loaded.searcher, run.runtime_iterations).await;
    let context_samples = measure_context(
        &loaded.searcher,
        run.runtime_iterations,
        &summary.target_unique_id,
    )
    .await;
    let health_samples = measure_health(&loaded.searcher, run.runtime_iterations).await;

    let load = sample_stats(&load_samples);
    let search = sample_stats(&search_samples);
    let indicator_inventory = sample_stats(&indicator_inventory_samples);
    let column_inventory = sample_stats(&column_inventory_samples);
    let context = sample_stats(&context_samples);
    let health = sample_stats(&health_samples);

    println!(
        "lexical scale guard models={} packages={} columns_per_model={} ref_fanout={} indicators={} load_p50_ms={} load_p95_ms={} search_p50_ms={} search_p95_ms={} indicator_inventory_p50_ms={} indicator_inventory_p95_ms={} column_inventory_p50_ms={} column_inventory_p95_ms={} context_p50_ms={} context_p95_ms={} health_p50_ms={} health_p95_ms={}",
        summary.models,
        summary.packages,
        summary.columns_per_model,
        summary.ref_fanout,
        summary.indicator_count,
        load.p50_ms,
        load.p95_ms,
        search.p50_ms,
        search.p95_ms,
        indicator_inventory.p50_ms,
        indicator_inventory.p95_ms,
        column_inventory.p50_ms,
        column_inventory.p95_ms,
        context.p50_ms,
        context.p95_ms,
        health.p50_ms,
        health.p95_ms
    );

    assert_under("manifest load", load.p95_ms, thresholds.load_p95_ms, &run);
    assert_under(
        "lexical search",
        search.p95_ms,
        thresholds.search_p95_ms,
        &run,
    );
    assert_under(
        "indicator inventory",
        indicator_inventory.p95_ms,
        thresholds.indicator_inventory_p95_ms,
        &run,
    );
    assert_under(
        "column inventory",
        column_inventory.p95_ms,
        thresholds.column_inventory_p95_ms,
        &run,
    );
    assert_under(
        "get_context",
        context.p95_ms,
        thresholds.context_p95_ms,
        &run,
    );
    assert_under(
        "health snapshot",
        health.p95_ms,
        thresholds.health_p95_ms,
        &run,
    );
}

fn measure_loads(
    manifest_path: &Path,
    iterations: usize,
    expected_entities: usize,
) -> Vec<Duration> {
    let mut samples = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let started = Instant::now();
        let loaded = load_searcher(manifest_path);
        assert_eq!(loaded.searcher.entity_count(), expected_entities);
        samples.push(started.elapsed());
    }
    samples
}

async fn measure_search(
    searcher: &ManifestSearch,
    iterations: usize,
    query: &str,
) -> Vec<Duration> {
    let params = SearchParams {
        query: query.to_string(),
        detail: Some(DetailLevel::Compact),
        pagination: PaginationParams {
            limit: Some(25),
            offset: 0,
        },
        ..Default::default()
    };
    let mut samples = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let started = Instant::now();
        let response = searcher.search(&params).await.expect("search response");
        assert_non_empty_array(&response, "search");
        samples.push(started.elapsed());
    }
    samples
}

async fn measure_indicator_inventory(
    searcher: &ManifestSearch,
    iterations: usize,
) -> Vec<Duration> {
    let params = IndicatorInventoryParams {
        pagination: PaginationParams {
            limit: Some(50),
            offset: 0,
        },
        ..Default::default()
    };
    let mut samples = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let started = Instant::now();
        let response = searcher
            .indicator_inventory(&params)
            .await
            .expect("indicator inventory response");
        assert_non_empty_array(&response, "indicator inventory");
        samples.push(started.elapsed());
    }
    samples
}

async fn measure_column_inventory(searcher: &ManifestSearch, iterations: usize) -> Vec<Duration> {
    let params = ColumnInventoryParams {
        pagination: PaginationParams {
            limit: Some(100),
            offset: 0,
        },
        ..Default::default()
    };
    let mut samples = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let started = Instant::now();
        let response = searcher
            .column_inventory(&params)
            .await
            .expect("column inventory response");
        assert_non_empty_array(&response, "column inventory");
        samples.push(started.elapsed());
    }
    samples
}

async fn measure_context(
    searcher: &ManifestSearch,
    iterations: usize,
    target_unique_id: &str,
) -> Vec<Duration> {
    let params = GetContextParams {
        id_or_name: target_unique_id.to_string(),
        resource_type: Some("model".to_string()),
        include_columns: true,
        include_upstream: true,
        upstream_include_tests: false,
        include_downstream: true,
        downstream_include_tests: false,
        include_tests: false,
        include_docs: false,
        include_sql: false,
        context_mode: ContextMode::Standard,
        limits: ContextLimits {
            lineage_depth: 1,
            upstream_limit: 10,
            downstream_limit: 10,
        },
    };
    let mut samples = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let started = Instant::now();
        let response = searcher
            .get_context(&params)
            .await
            .expect("context response");
        assert!(
            response
                .get("data")
                .and_then(JsonValue::as_object)
                .is_some()
        );
        samples.push(started.elapsed());
    }
    samples
}

async fn measure_health(searcher: &ManifestSearch, iterations: usize) -> Vec<Duration> {
    let mut samples = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let started = Instant::now();
        let response = searcher.health_snapshot().await;
        assert!(response.get("manifest").is_some());
        assert!(response.get("manifest_health").is_some());
        samples.push(started.elapsed());
    }
    samples
}

fn load_searcher(manifest_path: &Path) -> LoadedSearch {
    let guard = support_config::TestStorageGuard::new();
    let mut cfg = DbtNovaConfig {
        manifest_path: manifest_path.to_string_lossy().to_string(),
        search: lexical_search_config(),
        manifest_refresh_secs: 0,
        ..Default::default()
    };
    support_config::apply_test_storage(&mut cfg, &guard);
    let searcher = ManifestSearch::new(cfg)
        .expect("load synthetic manifest")
        .search;
    LoadedSearch {
        searcher,
        _guard: guard,
    }
}

fn lexical_search_config() -> SearchConfig {
    let mut config = support_config::test_search_config();
    config.enable_vector_search = false;
    config.enable_sparse_search = false;
    config.enable_reranker = false;
    config
}

struct SampleStats {
    p50_ms: u128,
    p95_ms: u128,
}

fn sample_stats(samples: &[Duration]) -> SampleStats {
    assert!(!samples.is_empty(), "sample set must not be empty");
    let mut values = samples.iter().map(Duration::as_millis).collect::<Vec<_>>();
    values.sort_unstable();
    SampleStats {
        p50_ms: percentile(&values, 50),
        p95_ms: percentile(&values, 95),
    }
}

fn percentile(sorted_values: &[u128], percentile: usize) -> u128 {
    let index = ((sorted_values.len() * percentile).div_ceil(100))
        .saturating_sub(1)
        .min(sorted_values.len() - 1);
    sorted_values[index]
}

fn assert_under(label: &str, observed_ms: u128, max_ms: u128, run: &ScaleRunConfig) {
    assert!(
        observed_ms <= max_ms,
        "{label} p95 {observed_ms}ms exceeded guard threshold {max_ms}ms \
         (models={}, packages={}, columns_per_model={}, ref_fanout={}). \
         Re-run scripts/check_lexical_scale.sh locally; raise the threshold only with \
         a documented baseline update.",
        run.manifest.models,
        run.manifest.packages,
        run.manifest.columns_per_model,
        run.manifest.ref_fanout
    );
}

fn assert_non_empty_array(response: &JsonValue, label: &str) {
    let Some(data) = response.get("data").and_then(JsonValue::as_array) else {
        panic!("{label} response missing array data: {response}");
    };
    assert!(!data.is_empty(), "{label} returned no rows: {response}");
}

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

fn env_u128(name: &str, default: u128) -> u128 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<u128>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}
