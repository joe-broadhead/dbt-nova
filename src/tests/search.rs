//! Tests for search tool responses.
use super::common::*;
use crate::config::{GovernanceGateConfig, SearchConfig};
use std::path::Path;
use tempfile::TempDir;

// Search Tool Tests
#[tokio::test(flavor = "multi_thread")]
async fn test_search_by_name() {
    let searcher = get_searcher();
    let params = SearchParams {
        query: "campaign".to_string(),
        resource_types: vec![],
        persona: None,
        detail: DetailLevel::Full,
        min_score: None,
        fuzzy: false,
        include_highlights: false,
        include_sql: false,
        pagination: PaginationParams {
            limit: 10,
            offset: 0,
        },
    };
    let result = searcher.search(&params).await.json();
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
    let count = result
        .get("count")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    assert!(count > 0, "Should find entities matching 'campaign'");
}
#[tokio::test(flavor = "multi_thread")]
async fn test_search_filter_by_resource_type() {
    let searcher = get_searcher();
    let params = SearchParams {
        query: "int".to_string(),
        resource_types: vec!["model".to_string()],
        persona: None,
        detail: DetailLevel::Standard,
        min_score: None,
        fuzzy: false,
        include_highlights: false,
        include_sql: false,
        pagination: PaginationParams {
            limit: 50,
            offset: 0,
        },
    };
    let result = searcher.search(&params).await.json();
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
    // All results should be models
    if let Some(data) = result.get("data").and_then(|d| d.as_array()) {
        for item in data {
            let rt = item.get("resource_type").and_then(|r| r.as_str());
            assert_eq!(rt, Some("model"), "All results should be models");
        }
    }
}
#[tokio::test(flavor = "multi_thread")]
async fn test_search_include_full_false() {
    let searcher = get_searcher();
    let params = SearchParams {
        query: "base".to_string(),
        resource_types: vec![],
        persona: None,
        detail: DetailLevel::Standard,
        min_score: None,
        fuzzy: false,
        include_highlights: false,
        include_sql: false,
        pagination: PaginationParams {
            limit: 5,
            offset: 0,
        },
    };
    let result = searcher.search(&params).await.json();
    // Summary mode should only have unique_id, name, resource_type
    if let Some(data) = result.get("data").and_then(|d| d.as_array())
        && !data.is_empty()
    {
        let first = &data[0];
        assert!(first.get("unique_id").is_some());
        assert!(first.get("name").is_some());
        // Should NOT have full details like columns, raw_code, etc.
    }
}
#[tokio::test(flavor = "multi_thread")]
async fn test_search_respects_limit() {
    let searcher = get_searcher();
    let params = SearchParams {
        query: "model".to_string(),
        resource_types: vec![],
        persona: None,
        detail: DetailLevel::Standard,
        min_score: None,
        fuzzy: false,
        include_highlights: false,
        include_sql: false,
        pagination: PaginationParams {
            limit: 5,
            offset: 0,
        },
    };
    let result = searcher.search(&params).await.json();
    let count = result
        .get("count")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    assert!(count <= 5, "Should respect limit of 5");
}
#[tokio::test(flavor = "multi_thread")]
async fn test_search_invalid_query() {
    let searcher = get_searcher();
    let params = SearchParams {
        query: "x".to_string(), // Too short
        resource_types: vec![],
        persona: None,
        detail: DetailLevel::Standard,
        min_score: None,
        fuzzy: false,
        include_highlights: false,
        include_sql: false,
        pagination: PaginationParams {
            limit: 10,
            offset: 0,
        },
    };
    let result = searcher.search(&params).await.json();
    // Should return error for too-short query
    let success = result
        .get("success")
        .expect("response missing 'success' field")
        .as_bool()
        .expect("'success' field should be boolean");
    assert!(!success, "Should fail for single-character query");
}
#[tokio::test(flavor = "multi_thread")]
async fn test_search_query_too_long() {
    let searcher = get_searcher();
    let max_len = searcher.config().search.max_query_length;
    let long_query = "a".repeat(max_len + 1);
    let params = SearchParams {
        query: long_query,
        resource_types: vec![],
        persona: None,
        detail: DetailLevel::Standard,
        min_score: None,
        fuzzy: false,
        include_highlights: false,
        include_sql: false,
        pagination: PaginationParams {
            limit: 10,
            offset: 0,
        },
    };
    let result = searcher.search(&params).await.json();
    let success = result
        .get("success")
        .expect("response missing 'success' field")
        .as_bool()
        .expect("'success' field should be boolean");
    assert!(!success, "Should fail for query exceeding max length");
    let error = result.get("error").and_then(|e| e.as_str()).unwrap_or("");
    assert!(
        error.contains("exceeds maximum length"),
        "Error should mention maximum length"
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn test_find_by_path_pattern_too_long() {
    let searcher = get_searcher();
    let max_len = searcher.config().search.max_path_pattern_length;
    let long_pattern = "a".repeat(max_len + 1);
    let params = FindByPathParams {
        path_pattern: long_pattern,
        resource_types: vec![],
        detail: DetailLevel::Standard,
        pagination: PaginationParams {
            limit: 10,
            offset: 0,
        },
    };
    let result = searcher.find_by_path(&params).await.json();
    let success = result
        .get("success")
        .expect("response missing 'success' field")
        .as_bool()
        .expect("'success' field should be boolean");
    assert!(
        !success,
        "Should fail for path pattern exceeding max length"
    );
    let error = result.get("error").and_then(|e| e.as_str()).unwrap_or("");
    assert!(
        error.contains("exceeds maximum length"),
        "Error should mention maximum length"
    );
}

fn governance_search_env(policy: GovernanceGateConfig) -> (ManifestSearch, TempDir) {
    let manifest_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/nova_manifest.json");
    let guard = TempDir::new().expect("test temp dir");
    let mut cfg = DbtNovaConfig {
        manifest_path: manifest_path.to_string_lossy().to_string(),
        search: SearchConfig {
            enable_vector_search: false,
            enable_sparse_search: false,
            enable_reranker: false,
            ..Default::default()
        },
        ..Default::default()
    };
    cfg.storage_dir = guard.path().to_string_lossy().to_string();
    cfg.storage_instance_id = "tests-governance-gate".to_string();
    cfg.storage_max_instances = 1;
    cfg.cleanup_storage_on_start = true;
    cfg.governance_gate = policy;
    let searcher = ManifestSearch::new(cfg).expect("Failed to load fixture manifest");
    (searcher, guard)
}

#[tokio::test(flavor = "multi_thread")]
async fn test_governance_persona_gate_policy_supports_advisory_mode() {
    let policy = GovernanceGateConfig {
        min_metadata_score: u8::MAX,
        min_documentation_coverage_pct: 101.0,
        require_tests: false,
        require_owner: false,
        require_required_fields: false,
        require_compliance_for_pii: false,
        block_on_failure: false,
    };
    let (searcher, _guard) = governance_search_env(policy);
    let model_id = searcher
        .by_resource_type
        .get("model")
        .and_then(|models| models.first())
        .cloned()
        .expect("fixture should include model entities");
    let entity = searcher
        .get_entity(&model_id)
        .await
        .expect("model lookup should succeed")
        .expect("selected model should exist in entity store");
    let query = entity
        .name
        .as_ref()
        .map(ToString::to_string)
        .unwrap_or(model_id.clone());
    let params = SearchParams {
        query,
        resource_types: vec!["model".to_string()],
        persona: Some("governance".to_string()),
        detail: DetailLevel::Standard,
        min_score: None,
        fuzzy: false,
        include_highlights: false,
        include_sql: false,
        pagination: PaginationParams {
            limit: 10,
            offset: 0,
        },
    };
    let result = searcher.search(&params).await.json();
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
    let Some(rows) = result.get("data").and_then(JsonValue::as_array) else {
        panic!("expected data array");
    };
    let Some(first) = rows.first() else {
        panic!("expected at least one search row");
    };
    let Some(payload) = first.get("persona_payload") else {
        panic!("expected governance persona_payload on search result");
    };

    assert_eq!(
        payload.get("gate_status").and_then(JsonValue::as_str),
        Some("advisory")
    );
    let advisory_reasons = payload
        .get("advisory_reasons")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    assert!(
        !advisory_reasons.is_empty(),
        "expected non-empty advisory reasons when advisory gate fails"
    );
    let blocking_reasons = payload
        .get("blocking_reasons")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    assert!(
        blocking_reasons.is_empty(),
        "blocking reasons should stay empty in advisory mode"
    );
    assert_eq!(
        payload
            .get("gate_policy")
            .and_then(|p| p.get("block_on_failure"))
            .and_then(JsonValue::as_bool),
        Some(false)
    );
    assert!(payload.get("lineage_health").is_some());
    assert!(payload.get("manifest_health").is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_governance_persona_gate_policy_can_force_pass_band() {
    let policy = GovernanceGateConfig {
        min_metadata_score: 0,
        min_documentation_coverage_pct: 0.0,
        require_tests: false,
        require_owner: false,
        require_required_fields: false,
        require_compliance_for_pii: false,
        block_on_failure: true,
    };
    let (searcher, _guard) = governance_search_env(policy);
    let params = SearchParams {
        query: "model".to_string(),
        resource_types: vec!["model".to_string()],
        persona: Some("governance".to_string()),
        detail: DetailLevel::Standard,
        min_score: None,
        fuzzy: false,
        include_highlights: false,
        include_sql: false,
        pagination: PaginationParams {
            limit: 1,
            offset: 0,
        },
    };
    let result = searcher.search(&params).await.json();
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
    let Some(rows) = result.get("data").and_then(JsonValue::as_array) else {
        panic!("expected data array");
    };
    let Some(first) = rows.first() else {
        panic!("expected at least one search row");
    };
    let Some(payload) = first.get("persona_payload") else {
        panic!("expected governance persona_payload on search result");
    };

    assert_eq!(
        payload.get("gate_status").and_then(JsonValue::as_str),
        Some("pass")
    );
    let advisory_reasons = payload
        .get("advisory_reasons")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    let blocking_reasons = payload
        .get("blocking_reasons")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    assert!(advisory_reasons.is_empty());
    assert!(blocking_reasons.is_empty());
}
