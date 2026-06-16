//! Tests for `batch_get_entities` tool responses.
use super::common::*;
use std::path::Path;
use tempfile::TempDir;

// Batch Get Tests
#[tokio::test(flavor = "multi_thread")]
async fn test_batch_get_entities_all_found() {
    let searcher = get_searcher();
    // Get some model IDs
    let models = searcher.by_resource_type.get("model").unwrap();
    let ids: Vec<String> = models.iter().take(3).cloned().collect();
    let params = BatchGetParams {
        unique_ids: ids.clone(),
        detail: Some(DetailLevel::Full),
    };
    let result = searcher.batch_get_entities(&params).await.json();
    let success = result
        .get("success")
        .expect("response missing 'success' field")
        .as_bool()
        .expect("'success' field should be boolean");
    assert!(
        success,
        "Expected success=true but got error: {:?}",
        result.get("error")
    );
    let data = result.get("data").unwrap();
    assert_eq!(
        data.get("found_count")
            .and_then(serde_json::Value::as_u64)
            .unwrap(),
        3
    );
    assert_eq!(
        data.get("not_found_count")
            .and_then(serde_json::Value::as_u64)
            .unwrap(),
        0
    );
    assert!(
        data.get("not_found")
            .and_then(|n| n.as_array())
            .unwrap()
            .is_empty()
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn test_batch_get_entities_some_not_found() {
    let searcher = get_searcher();
    let models = searcher.by_resource_type.get("model").unwrap();
    let mut ids: Vec<String> = models.iter().take(2).cloned().collect();
    ids.push("nonexistent.model.xyz".to_string());
    ids.push("also.nonexistent.abc".to_string());
    let params = BatchGetParams {
        unique_ids: ids,
        detail: Some(DetailLevel::Standard),
    };
    let result = searcher.batch_get_entities(&params).await.json();
    let success = result
        .get("success")
        .expect("response missing 'success' field")
        .as_bool()
        .expect("'success' field should be boolean");
    assert!(
        success,
        "Expected success=true but got error: {:?}",
        result.get("error")
    );
    let data = result.get("data").unwrap();
    assert_eq!(
        data.get("found_count")
            .and_then(serde_json::Value::as_u64)
            .unwrap(),
        2
    );
    assert_eq!(
        data.get("not_found_count")
            .and_then(serde_json::Value::as_u64)
            .unwrap(),
        2
    );
    let not_found = data.get("not_found").and_then(|n| n.as_array()).unwrap();
    assert!(
        not_found
            .iter()
            .any(|id| id.as_str() == Some("nonexistent.model.xyz"))
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn test_batch_get_entities_include_full_false() {
    let searcher = get_searcher();
    let models = searcher.by_resource_type.get("model").unwrap();
    let ids: Vec<String> = models.iter().take(1).cloned().collect();
    let params = BatchGetParams {
        unique_ids: ids,
        detail: Some(DetailLevel::Standard),
    };
    let result = searcher.batch_get_entities(&params).await.json();
    let entities = result
        .get("data")
        .and_then(|d| d.get("entities"))
        .and_then(|e| e.as_array())
        .unwrap();
    if !entities.is_empty() {
        let first = &entities[0];
        // Summary should have basic fields
        assert!(first.get("unique_id").is_some());
        assert!(first.get("name").is_some());
        // Should NOT have full details
        assert!(first.get("raw_code").is_none());
    }
}
#[tokio::test(flavor = "multi_thread")]
async fn test_batch_get_entities_empty_list() {
    let searcher = get_searcher();
    let params = BatchGetParams {
        unique_ids: vec![],
        detail: Some(DetailLevel::Full),
    };
    let result = searcher.batch_get_entities(&params).await.json();
    let success = result
        .get("success")
        .expect("response missing 'success' field")
        .as_bool()
        .expect("'success' field should be boolean");
    assert!(
        success,
        "Expected success=true but got error: {:?}",
        result.get("error")
    );
    let data = result.get("data").unwrap();
    assert_eq!(
        data.get("found_count")
            .and_then(serde_json::Value::as_u64)
            .unwrap(),
        0
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_batch_get_entities_over_limit_returns_error() {
    let guard = TempDir::new().expect("test temp dir");
    let manifest_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/nova_manifest.json");
    let mut cfg = DbtNovaConfig {
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
    cfg.batch_get_max_items = 1;
    let searcher = ManifestSearch::new(cfg)
        .expect("Failed to load fixture manifest")
        .search;
    let models = match searcher.by_resource_type.get("model") {
        Some(m) if m.len() >= 2 => m,
        _ => return,
    };

    let params = BatchGetParams {
        unique_ids: vec![models[0].clone(), models[1].clone()],
        detail: Some(DetailLevel::Standard),
    };
    let result = searcher.batch_get_entities(&params).await.json();
    let success = result
        .get("success")
        .expect("response missing 'success' field")
        .as_bool()
        .expect("'success' field should be boolean");
    assert!(
        !success,
        "Expected success=false but got: {:?}",
        result.get("data")
    );
    assert_eq!(
        result.get("error_code").and_then(|v| v.as_str()),
        Some("INVALID_PARAMS")
    );
}
