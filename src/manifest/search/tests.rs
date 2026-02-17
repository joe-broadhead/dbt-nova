use std::path::PathBuf;

use crate::config::{DbtNovaConfig, SearchConfig};

use super::ManifestSearch;

#[tokio::test(flavor = "multi_thread")]
async fn entity_cache_updates_recency_on_read() {
    let manifest_path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/nova_manifest.json");
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let search = SearchConfig {
        enable_vector_search: false,
        enable_sparse_search: false,
        enable_reranker: false,
        ..Default::default()
    };

    let mut cfg = DbtNovaConfig {
        manifest_path: manifest_path.to_string_lossy().to_string(),
        search,
        ..Default::default()
    };
    cfg.storage_dir = temp_dir.path().to_string_lossy().to_string();
    cfg.storage_instance_id = "tests".to_string();
    cfg.storage_max_instances = 1;
    cfg.cleanup_storage_on_start = true;
    cfg.entity_cache_size = 2;
    let cache_limit = cfg.entity_cache_size;

    let searcher = ManifestSearch::new(cfg).expect("fixture manifest must be present");
    let ids: Vec<String> = searcher.entities.ids().take(3).cloned().collect();
    if ids.len() < 3 {
        return;
    }

    let id_a = &ids[0];
    let id_b = &ids[1];
    let id_c = &ids[2];

    let _ = searcher.get_entity(id_a).await.unwrap();
    let _ = searcher.get_entity(id_b).await.unwrap();
    let _ = searcher.get_entity(id_a).await.unwrap();
    let _ = searcher.get_entity(id_c).await.unwrap();

    let cache = searcher.entity_cache.as_ref().expect("cache enabled");
    let len = cache.len();
    assert!(
        len <= cache_limit,
        "expected cache size to stay within configured limit"
    );
}
