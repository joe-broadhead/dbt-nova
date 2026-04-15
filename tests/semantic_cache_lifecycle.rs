use std::path::{Path, PathBuf};

use dbt_nova::ManifestSearch;
use dbt_nova::config::{DbtNovaConfig, SearchColdStartPolicy, SearchConfig};
use dbt_nova::manifest::rkyv_embeddings::{EmbeddingsCacheLoad, load_embeddings, save_embeddings};
use dbt_nova::manifest::rkyv_sparse_embeddings::{
    SparseEmbeddingsCacheLoad, load_sparse_embeddings, save_sparse_embeddings,
};
use dbt_nova::manifest::rkyv_types::{
    CachedEmbeddings, CachedSparseEmbeddings, RKYV_SCHEMA_VERSION,
};
use tempfile::TempDir;

fn fixture_manifest_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("nova_manifest.json")
}

fn manifest_config(workspace: &TempDir, search: SearchConfig) -> DbtNovaConfig {
    DbtNovaConfig {
        manifest_path: fixture_manifest_path().to_string_lossy().to_string(),
        storage_dir: workspace
            .path()
            .join("storage")
            .to_string_lossy()
            .to_string(),
        storage_instance_id: "semantic-cache-tests".to_string(),
        search,
        ..DbtNovaConfig::default()
    }
}

async fn fixture_manifest_hash(workspace: &TempDir, cache_root: &Path) -> String {
    let baseline = SearchConfig {
        enable_vector_search: false,
        enable_sparse_search: false,
        enable_reranker: false,
        embedding_cache_dir: cache_root.to_string_lossy().to_string(),
        ..SearchConfig::default()
    };

    let loaded = ManifestSearch::new(manifest_config(workspace, baseline))
        .expect("baseline manifest load should succeed")
        .search;
    let health = loaded.health_snapshot().await;
    health["manifest"]["hash"]
        .as_str()
        .expect("manifest hash should be present")
        .to_string()
}

#[test]
fn manifest_scoped_semantic_caches_can_coexist_for_multiple_hashes() {
    let workspace = TempDir::new().expect("tempdir");
    let cache_root = workspace.path().join("cache");
    let search = SearchConfig {
        embedding_cache_dir: cache_root.to_string_lossy().to_string(),
        ..SearchConfig::default()
    };

    let cache_a = CachedEmbeddings {
        schema_version: RKYV_SCHEMA_VERSION,
        model_name: "intfloat/multilingual-e5-base".to_string(),
        manifest_hash: "hash-a".to_string(),
        entity_ids: vec!["model.a".to_string()],
        dense_embeddings: vec![vec![1.0, 0.0]],
        is_quantized: false,
        sparse_indices: None,
        sparse_values: None,
        ann_hyperplanes: None,
        ann_bucket_keys: None,
        ann_bucket_values: None,
    };
    let cache_b = CachedEmbeddings {
        manifest_hash: "hash-b".to_string(),
        entity_ids: vec!["model.b".to_string()],
        dense_embeddings: vec![vec![0.0, 1.0]],
        ..cache_a.clone()
    };
    save_embeddings(&cache_a, &search).expect("save dense cache a");
    save_embeddings(&cache_b, &search).expect("save dense cache b");

    let sparse_a = CachedSparseEmbeddings {
        schema_version: RKYV_SCHEMA_VERSION,
        model_name: "Qdrant/Splade_PP_en_v1".to_string(),
        manifest_hash: "hash-a".to_string(),
        entity_ids: vec!["model.a".to_string()],
        sparse_indices: vec![vec![1, 2]],
        sparse_values: vec![vec![0.3, 0.7]],
    };
    let sparse_b = CachedSparseEmbeddings {
        manifest_hash: "hash-b".to_string(),
        entity_ids: vec!["model.b".to_string()],
        sparse_indices: vec![vec![3, 4]],
        sparse_values: vec![vec![0.4, 0.6]],
        ..sparse_a.clone()
    };
    save_sparse_embeddings(&sparse_a, &search).expect("save sparse cache a");
    save_sparse_embeddings(&sparse_b, &search).expect("save sparse cache b");

    assert!(
        cache_root
            .join("manifests")
            .join("hash-a")
            .join("dense__intfloat--multilingual-e5-base.rkyv.zst")
            .is_file()
    );
    assert!(
        cache_root
            .join("manifests")
            .join("hash-b")
            .join("dense__intfloat--multilingual-e5-base.rkyv.zst")
            .is_file()
    );
    assert!(
        cache_root
            .join("manifests")
            .join("hash-a")
            .join("sparse__Qdrant--Splade_PP_en_v1.rkyv.zst")
            .is_file()
    );
    assert!(
        cache_root
            .join("manifests")
            .join("hash-b")
            .join("sparse__Qdrant--Splade_PP_en_v1.rkyv.zst")
            .is_file()
    );

    assert!(matches!(
        load_embeddings(
            &search,
            "intfloat/multilingual-e5-base",
            "hash-a",
            1024 * 1024
        ),
        EmbeddingsCacheLoad::Hit { .. }
    ));
    assert!(matches!(
        load_embeddings(
            &search,
            "intfloat/multilingual-e5-base",
            "hash-b",
            1024 * 1024
        ),
        EmbeddingsCacheLoad::Hit { .. }
    ));
    assert!(matches!(
        load_sparse_embeddings(&search, "Qdrant/Splade_PP_en_v1", "hash-a", 1024 * 1024),
        SparseEmbeddingsCacheLoad::Hit { .. }
    ));
    assert!(matches!(
        load_sparse_embeddings(&search, "Qdrant/Splade_PP_en_v1", "hash-b", 1024 * 1024),
        SparseEmbeddingsCacheLoad::Hit { .. }
    ));
}

#[tokio::test]
async fn manifest_load_degrades_vector_startup_when_manifest_cache_is_missing() {
    let workspace = TempDir::new().expect("tempdir");
    let cache_root = workspace.path().join("cache");
    let search = SearchConfig {
        cold_start_policy: SearchColdStartPolicy::Degrade,
        enable_vector_search: true,
        enable_sparse_search: false,
        enable_reranker: false,
        embedding_cache_dir: cache_root.to_string_lossy().to_string(),
        ..SearchConfig::default()
    };

    let loaded = ManifestSearch::new(manifest_config(&workspace, search))
        .expect("manifest load should succeed")
        .search;

    assert!(!loaded.vector_search_ready());
    let health = loaded.health_snapshot().await;
    let warning = health["search"]["vector"]["warning"]
        .as_str()
        .expect("vector warning should be present");
    assert!(warning.contains("manifest warm --vector"));
    assert!(warning.contains("manifest-scoped cache"));
    assert_eq!(
        health["search"]["vector"]["cache"]["present"],
        serde_json::json!(false)
    );
}

#[tokio::test]
async fn manifest_load_degrades_sparse_startup_when_manifest_cache_is_missing() {
    let workspace = TempDir::new().expect("tempdir");
    let cache_root = workspace.path().join("cache");
    let search = SearchConfig {
        cold_start_policy: SearchColdStartPolicy::Degrade,
        enable_vector_search: false,
        enable_sparse_search: true,
        enable_reranker: false,
        embedding_cache_dir: cache_root.to_string_lossy().to_string(),
        ..SearchConfig::default()
    };

    let loaded = ManifestSearch::new(manifest_config(&workspace, search))
        .expect("manifest load should succeed")
        .search;

    assert!(!loaded.sparse_search_ready());
    let health = loaded.health_snapshot().await;
    let warning = health["search"]["sparse"]["warning"]
        .as_str()
        .expect("sparse warning should be present");
    assert!(warning.contains("manifest warm --sparse"));
    assert!(warning.contains("manifest-scoped cache"));
    assert_eq!(
        health["search"]["sparse"]["cache"]["present"],
        serde_json::json!(false)
    );
}

#[tokio::test]
async fn manifest_load_uses_cached_semantic_indexes_without_local_query_model_files() {
    let workspace = TempDir::new().expect("tempdir");
    let cache_root = workspace.path().join("cache");
    let manifest_hash = fixture_manifest_hash(&workspace, &cache_root).await;

    let seed_search = SearchConfig {
        embedding_cache_dir: cache_root.to_string_lossy().to_string(),
        ..SearchConfig::default()
    };
    save_embeddings(
        &CachedEmbeddings {
            schema_version: RKYV_SCHEMA_VERSION,
            model_name: "intfloat/multilingual-e5-base".to_string(),
            manifest_hash: manifest_hash.clone(),
            entity_ids: vec!["model.test".to_string()],
            dense_embeddings: vec![vec![1.0, 0.0]],
            is_quantized: false,
            sparse_indices: None,
            sparse_values: None,
            ann_hyperplanes: None,
            ann_bucket_keys: None,
            ann_bucket_values: None,
        },
        &seed_search,
    )
    .expect("save dense cache");
    save_sparse_embeddings(
        &CachedSparseEmbeddings {
            schema_version: RKYV_SCHEMA_VERSION,
            model_name: "Qdrant/Splade_PP_en_v1".to_string(),
            manifest_hash: manifest_hash.clone(),
            entity_ids: vec!["model.test".to_string()],
            sparse_indices: vec![vec![1, 2]],
            sparse_values: vec![vec![0.4, 0.6]],
        },
        &seed_search,
    )
    .expect("save sparse cache");

    let search = SearchConfig {
        cold_start_policy: SearchColdStartPolicy::Degrade,
        enable_vector_search: true,
        enable_sparse_search: true,
        enable_reranker: true,
        embedding_cache_dir: cache_root.to_string_lossy().to_string(),
        ..SearchConfig::default()
    };

    let loaded = ManifestSearch::new(manifest_config(&workspace, search))
        .expect("manifest load should succeed with cached semantic indexes")
        .search;

    assert!(!loaded.vector_search_ready());
    assert!(!loaded.sparse_search_ready());
    assert!(!loaded.reranker_ready());

    let health = loaded.health_snapshot().await;
    assert_eq!(
        health["search"]["vector"]["ready"],
        serde_json::json!(false)
    );
    assert_eq!(
        health["search"]["vector"]["query_model_files_present"],
        serde_json::json!(false)
    );
    assert_eq!(
        health["search"]["vector"]["query_model_initialized"],
        serde_json::json!(false)
    );
    assert!(
        health["search"]["vector"]["warning"]
            .as_str()
            .expect("vector warning should be present")
            .contains("not query-ready")
    );
    assert_eq!(
        health["search"]["sparse"]["ready"],
        serde_json::json!(false)
    );
    assert_eq!(
        health["search"]["sparse"]["query_model_files_present"],
        serde_json::json!(false)
    );
    assert_eq!(
        health["search"]["sparse"]["query_model_initialized"],
        serde_json::json!(false)
    );
    assert!(
        health["search"]["sparse"]["warning"]
            .as_str()
            .expect("sparse warning should be present")
            .contains("not query-ready")
    );
    assert_eq!(
        health["search"]["reranker"]["ready"],
        serde_json::json!(false)
    );
    assert_eq!(
        health["search"]["reranker"]["query_model_files_present"],
        serde_json::json!(false)
    );
    assert_eq!(
        health["search"]["reranker"]["query_model_initialized"],
        serde_json::json!(false)
    );
    assert!(
        health["search"]["reranker"]["warning"]
            .as_str()
            .expect("reranker warning should be present")
            .contains("not query-ready")
    );
}
