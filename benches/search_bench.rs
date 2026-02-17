//! Microbenchmarks for core query paths.
//!
//! These are not performance tests for production workloads; they are intended
//! to detect regressions in the core search and lineage calls using the
//! standard fixture manifest.

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use std::time::Duration;
use tokio::runtime::Runtime;

use dbt_nova::params::{DetailLevel, GetLineageParams, SearchParams};

#[path = "../tests/support/fixtures.rs"]
mod fixtures;
#[path = "../tests/support/config.rs"]
mod support_config;

fn bench_search(c: &mut Criterion) {
    // Bench hybrid search on a representative query against the fixture manifest.
    let env = fixtures::load_fixture("nova_manifest.json").unwrap();
    let rt = Runtime::new().unwrap();
    let params = SearchParams {
        query: "revenue sessions conversion".to_string(),
        ..Default::default()
    };

    c.bench_function("search_revenue_sessions", |b| {
        b.iter(|| {
            rt.block_on(env.search(black_box(&params)))
                .expect("search failed");
        })
    });
}

fn bench_lineage(c: &mut Criterion) {
    // Bench upstream lineage traversal for a canonical model in the fixture manifest.
    let env = fixtures::load_fixture("nova_manifest.json").unwrap();
    let rt = Runtime::new().unwrap();
    let params = GetLineageParams {
        id_or_name: "model.nova_test.fct__orders".to_string(),
        direction: "upstream".to_string(),
        depth: Some(2),
        resource_types: Vec::new(),
        detail: DetailLevel::Standard,
    };

    c.bench_function("lineage_upstream_fct_orders", |b| {
        b.iter(|| {
            rt.block_on(env.get_lineage(black_box(&params)))
                .expect("lineage failed");
        })
    });
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .sample_size(50)
        .measurement_time(Duration::from_secs(7));
    targets = bench_search, bench_lineage
}
criterion_main!(benches);
