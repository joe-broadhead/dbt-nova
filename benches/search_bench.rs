//! Microbenchmarks for core query paths.
//!
//! These are not performance tests for production workloads; they are intended
//! to detect regressions in the core search and lineage calls using the
//! standard fixture manifest.

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use std::time::Duration;
use tokio::runtime::Runtime;

use dbt_nova::params::{
    DetailLevel, GetEntityParams, GetLineageParams, IndicatorInventoryParams, PaginationParams,
    SearchColumnsParams, SearchParams,
};
use serde_json::json;

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
        detail: Some(DetailLevel::Standard),
    };

    c.bench_function("lineage_upstream_fct_orders", |b| {
        b.iter(|| {
            rt.block_on(env.get_lineage(black_box(&params)))
                .expect("lineage failed");
        })
    });
}

fn bench_indicator_inventory(c: &mut Criterion) {
    let env = fixtures::load_fixture("nova_manifest.json").unwrap();
    let rt = Runtime::new().unwrap();
    let params = IndicatorInventoryParams::default();

    c.bench_function("indicator_inventory_scan", |b| {
        b.iter(|| {
            rt.block_on(env.indicator_inventory(black_box(&params)))
                .expect("indicator inventory failed");
        })
    });
}

fn bench_search_columns(c: &mut Criterion) {
    let env = fixtures::load_fixture("nova_manifest.json").unwrap();
    let rt = Runtime::new().unwrap();
    let params = SearchColumnsParams {
        query: "customer revenue".to_string(),
        ..Default::default()
    };

    c.bench_function("search_columns_scan", |b| {
        b.iter(|| {
            rt.block_on(env.search_columns(black_box(&params)))
                .expect("column search failed");
        })
    });
}

fn large_entity_count() -> usize {
    std::env::var("DBT_NOVA_BENCH_LARGE_ENTITY_COUNT")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(10_000)
}

fn load_large_synthetic_fixture(entity_count: usize) -> fixtures::FixtureSearchEnv {
    let temp_dir = tempfile::tempdir().expect("large manifest tempdir");
    let manifest_path = temp_dir.path().join("large_manifest.json");
    let mut nodes = serde_json::Map::new();

    for index in 0..entity_count {
        let name = format!("model_{index:05}");
        let unique_id = format!("model.large.{name}");
        nodes.insert(
            unique_id.clone(),
            json!({
                "name": name,
                "resource_type": "model",
                "package_name": "large",
                "description": format!("Synthetic benchmark model {index} for revenue customer search."),
                "database": "analytics",
                "schema": "benchmark",
                "path": format!("models/benchmark/{name}.sql"),
                "original_file_path": format!("models/benchmark/{name}.sql"),
                "unique_id": unique_id,
                "fqn": ["large", "benchmark", name],
                "alias": name,
                "checksum": {"name": "sha256", "checksum": format!("{index:064}")},
                "config": {
                    "enabled": true,
                    "meta": {
                        "nova": {
                            "canonical": index % 10 == 0,
                            "grain": {"entities": ["customer"], "time": "day"},
                            "measures": [
                                {
                                    "name": "gross_revenue",
                                    "expr": "gross_revenue",
                                    "agg": "sum",
                                    "canonical": index % 10 == 0,
                                    "synonyms": ["sales", "bookings"]
                                }
                            ]
                        }
                    }
                },
                "tags": ["benchmark", "revenue"],
                "columns": {
                    "customer_id": {
                        "name": "customer_id",
                        "description": "Synthetic customer identifier.",
                        "data_type": "string",
                        "meta": {"nova": {"role": "identifier", "semantic_type": "customer_id"}},
                        "config": {"meta": {}, "tags": []},
                        "constraints": [],
                        "tags": []
                    },
                    "gross_revenue": {
                        "name": "gross_revenue",
                        "description": "Synthetic gross revenue amount.",
                        "data_type": "numeric",
                        "meta": {"nova": {"role": "measure", "semantic_type": "money"}},
                        "config": {"meta": {}, "tags": []},
                        "constraints": [],
                        "tags": []
                    }
                },
                "depends_on": {"nodes": [], "macros": []},
                "raw_code": "select customer_id, gross_revenue from source_table",
                "compiled_code": "select customer_id, gross_revenue from source_table"
            }),
        );
    }

    let manifest = json!({
        "metadata": {
            "dbt_schema_version": "https://schemas.getdbt.com/dbt/manifest/v12.json",
            "dbt_version": "1.10.2",
            "project_name": "large"
        },
        "selectors": {},
        "nodes": nodes,
        "sources": {},
        "macros": {},
        "docs": {},
        "exposures": {},
        "metrics": {},
        "semantic_models": {},
        "parent_map": {},
        "child_map": {}
    });
    std::fs::write(
        &manifest_path,
        serde_json::to_vec(&manifest).expect("serialize large manifest"),
    )
    .expect("write large manifest");
    fixtures::load_manifest_path(&manifest_path).expect("load large synthetic manifest")
}

fn bench_large_indicator_inventory(c: &mut Criterion) {
    let entity_count = large_entity_count();
    let env = load_large_synthetic_fixture(entity_count);
    let rt = Runtime::new().unwrap();
    let params = IndicatorInventoryParams::default();

    c.bench_function("large_indicator_inventory_scan", |b| {
        b.iter(|| {
            rt.block_on(env.indicator_inventory(black_box(&params)))
                .expect("large indicator inventory failed");
        })
    });
}

fn bench_large_archived_access(c: &mut Criterion) {
    let entity_count = large_entity_count();
    let env = load_large_synthetic_fixture(entity_count);
    let ids = (0..entity_count)
        .map(|index| format!("model.large.model_{index:05}"))
        .collect::<Vec<_>>();

    c.bench_function("large_archived_access_checked", |b| {
        b.iter(|| {
            for id in black_box(&ids) {
                black_box(env.get_entity_archived(id).expect("archived access failed"));
            }
        })
    });
}

fn bench_large_concurrent_search_and_lookup(c: &mut Criterion) {
    let entity_count = large_entity_count();
    let env = load_large_synthetic_fixture(entity_count);
    let rt = Runtime::new().unwrap();
    let search_params = SearchParams {
        query: "gross revenue customer".to_string(),
        persona: Some("analyst".to_string()),
        pagination: PaginationParams {
            limit: Some(25),
            offset: 0,
        },
        ..Default::default()
    };
    let entity_params = GetEntityParams {
        id_or_name: "model.large.model_00000".to_string(),
        resource_type: Some("model".to_string()),
        detail: Some(DetailLevel::Compact),
    };

    c.bench_function("large_concurrent_search_and_get_entity", |b| {
        b.iter(|| {
            rt.block_on(async {
                let (a, b, c, d, entity) = tokio::join!(
                    env.search(black_box(&search_params)),
                    env.search(black_box(&search_params)),
                    env.search(black_box(&search_params)),
                    env.search(black_box(&search_params)),
                    env.get_entity_data(black_box(&entity_params)),
                );
                a.expect("search a failed");
                b.expect("search b failed");
                c.expect("search c failed");
                d.expect("search d failed");
                entity.expect("get_entity failed");
            });
        })
    });
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .sample_size(50)
        .measurement_time(Duration::from_secs(7));
    targets = bench_search, bench_lineage, bench_indicator_inventory, bench_search_columns,
        bench_large_indicator_inventory, bench_large_archived_access,
        bench_large_concurrent_search_and_lookup
}
criterion_main!(benches);
