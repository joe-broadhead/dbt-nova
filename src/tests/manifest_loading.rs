//! Tests for manifest loading counts and indexes.
use super::common::*;

// Manifest Loading Tests
#[test]
fn test_load_manifest_success() {
    let searcher = get_searcher();
    assert!(searcher.entity_count() > 0, "entities should not be empty");
    assert!(
        !searcher.parent_map.is_empty(),
        "parent_map should not be empty"
    );
    assert!(
        !searcher.child_map.is_empty(),
        "child_map should not be empty"
    );
}
#[test]
fn test_load_manifest_invalid_path() {
    let cfg = DbtNovaConfig {
        manifest_path: "nonexistent.json".to_string(),
        ..Default::default()
    };
    let result = ManifestSearch::new(cfg);
    assert!(result.is_err(), "Should fail for nonexistent file");
}
#[test]
fn test_manifest_indexes_populated() {
    let searcher = get_searcher();
    // Check resource type index
    assert!(
        searcher.by_resource_type.contains_key("model"),
        "Should have models"
    );
    assert!(
        searcher.by_resource_type.contains_key("source"),
        "Should have sources"
    );
    assert!(
        searcher.by_resource_type.contains_key("macro"),
        "Should have macros"
    );
    // Check name index
    assert!(
        !searcher.name_to_keys.is_empty(),
        "name_to_keys should be populated"
    );
    // Check Tantivy index for full-text search
    assert!(
        searcher.tantivy.doc_count().unwrap_or(0) > 0,
        "tantivy index should be populated"
    );
}
#[test]
fn test_entity_counts() {
    let searcher = get_searcher();
    let model_count = searcher.by_resource_type.get("model").map_or(0, Vec::len);
    let source_count = searcher.by_resource_type.get("source").map_or(0, Vec::len);
    let macro_count = searcher.by_resource_type.get("macro").map_or(0, Vec::len);
    assert!(model_count > 0, "Should have models");
    assert!(source_count > 0, "Should have sources");
    assert!(macro_count > 0, "Should have macros");
}
