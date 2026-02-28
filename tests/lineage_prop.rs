//! Property tests for lineage graph invariants and limits.
use std::collections::HashSet;

use dbt_nova::config::ColumnLineageConfig;
use dbt_nova::params::PaginationParams;
use dbt_nova::params::{DetailLevel, GetLineageParams, ListEntitiesParams};
use dbt_nova::{DbtNovaConfig, ManifestSearch};
use proptest::prelude::*;
use std::path::PathBuf;

#[path = "support/config.rs"]
mod support_config;

// Property: lineage results contain unique IDs and are capped by max_results.
proptest! {
    #![proptest_config(ProptestConfig { cases: 24, .. ProptestConfig::default() })]
    #[test]
    fn lineage_results_unique_and_capped(seed in any::<u64>(), depth in 1usize..4) {
        // Build a real searcher so the property test exercises production logic end-to-end.
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        let guard = support_config::TestStorageGuard::new();
        let manifest_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/nova_manifest.json");
        let mut cfg = DbtNovaConfig {
            manifest_path: manifest_path.to_string_lossy().to_string(),
            lineage_max_results: 200,
            column_lineage: ColumnLineageConfig {
                max_results: 200,
                ..Default::default()
            },
            search: support_config::test_search_config(),
            ..Default::default()
        };
        support_config::apply_test_storage(&mut cfg, &guard);
        let searcher = ManifestSearch::new(cfg)
            .expect("fixture manifest must be present")
            .search;

        // Use the public list_entities API to pick a valid model ID.
        let list_params = ListEntitiesParams {
            resource_type: "model".to_string(),
            package: None,
            tags: vec![],
            database_schema: None,
            detail: DetailLevel::Standard,
            pagination: PaginationParams { limit: 100, offset: 0 },
};
        let listed = rt.block_on(searcher.list_entities(&list_params)).unwrap();
        let ids: Vec<String> = listed
            .get("data")
            .and_then(|d| d.as_array())
            .into_iter()
            .flatten()
            .filter_map(|v| v.get("unique_id").and_then(|u| u.as_str()).map(|s| s.to_string()))
            .collect();
        prop_assume!(!ids.is_empty());

        // Deterministic selection from the seed for reproducible coverage.
        let idx = (seed as usize) % ids.len();
        let start_id = ids[idx].clone();

        let params = GetLineageParams {
            id_or_name: start_id,
            direction: "downstream".to_string(),
            depth: Some(depth),
            resource_types: vec![],
            detail: DetailLevel::Full,
        };

        if let Ok(res) = rt.block_on(searcher.get_lineage(&params)) {
            let max_results = searcher.config().lineage_max_results;
            let count = res
                .get("count")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0) as usize;
            assert!(count <= max_results, "count {} should not exceed cap {}", count, max_results);

            if let Some(arr) = res
                .get("data")
                .and_then(|d| d.get("entities"))
                .and_then(|d| d.as_array())
            {
                // Property: IDs should be unique within the result set.
                let mut seen: HashSet<String> = HashSet::new();
                for item in arr {
                    if let Some(uid) = item.get("unique_id").and_then(|u| u.as_str()) {
                        assert!(
                            seen.insert(uid.to_string()),
                            "duplicate lineage entry for {}",
                            uid
                        );
                    }
                }
            }
        }
    }
}
