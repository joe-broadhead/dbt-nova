//! Tests for search tool responses.
use super::common::*;
use crate::config::{GovernanceGateConfig, SearchConfig};
use std::path::Path;
use tempfile::TempDir;

fn search_candidates_env() -> TestSearchEnv {
    get_searcher_with_fixture_config(
        "search_candidates.json",
        SearchConfig {
            enable_vector_search: false,
            enable_sparse_search: false,
            enable_reranker: false,
            ..Default::default()
        },
    )
}

fn search_candidates_baseline_env() -> TestSearchEnv {
    get_searcher_with_fixture_config(
        "search_candidates_baseline.json",
        SearchConfig {
            enable_vector_search: false,
            enable_sparse_search: false,
            enable_reranker: false,
            ..Default::default()
        },
    )
}

fn semantic_preview_env() -> TestSearchEnv {
    get_searcher_with_fixture_config(
        "semantic_preview_ranking.json",
        SearchConfig {
            enable_vector_search: false,
            enable_sparse_search: false,
            enable_reranker: false,
            ..Default::default()
        },
    )
}

fn search_params(query: &str, persona: Option<&str>) -> SearchParams {
    SearchParams {
        query: query.to_string(),
        resource_types: vec![],
        persona: persona.map(str::to_string),
        detail: DetailLevel::Standard,
        min_score: None,
        fuzzy: false,
        include_highlights: false,
        include_sql: false,
        pagination: PaginationParams {
            limit: 10,
            offset: 0,
        },
    }
}

fn result_rows(result: &JsonValue) -> Vec<&JsonValue> {
    result
        .get("data")
        .and_then(JsonValue::as_array)
        .map(|rows| rows.iter().collect())
        .unwrap_or_default()
}

fn row_by_unique_id<'a>(rows: &'a [&JsonValue], unique_id: &str) -> &'a JsonValue {
    rows.iter()
        .copied()
        .find(|row| row.get("unique_id").and_then(JsonValue::as_str) == Some(unique_id))
        .unwrap_or_else(|| panic!("missing row for {unique_id}"))
}

fn row_position(rows: &[&JsonValue], unique_id: &str) -> usize {
    rows.iter()
        .position(|row| row.get("unique_id").and_then(JsonValue::as_str) == Some(unique_id))
        .unwrap_or_else(|| panic!("missing row for {unique_id}"))
}

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

#[tokio::test(flavor = "multi_thread")]
async fn test_search_analyst_candidate_false_deboosts_flagged_model() {
    let searcher = search_candidates_env();
    let result = searcher
        .search(&search_params("gross margin benchmark", Some("analyst")))
        .await
        .json();
    let rows = result_rows(&result);
    assert_eq!(rows.len(), 2, "expected both fixture models in results");

    let curated = row_by_unique_id(&rows, "model.pkg.curated_margin_anchor");
    let helper = row_by_unique_id(&rows, "model.pkg.helper_bridge_anchor");
    let curated_score = curated["score"].as_f64().expect("curated score");
    let helper_score = helper["score"].as_f64().expect("helper score");

    assert!(
        curated_score > helper_score,
        "analyst search should deboost helper model: curated={curated_score}, helper={helper_score}"
    );
    assert_eq!(
        rows[0].get("unique_id").and_then(JsonValue::as_str),
        Some("model.pkg.curated_margin_anchor")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_search_candidate_false_does_not_penalize_engineer_or_governance() {
    let searcher = search_candidates_env();
    let baseline = search_candidates_baseline_env();

    for persona in ["engineer", "governance"] {
        let result = searcher
            .search(&search_params("gross margin benchmark", Some(persona)))
            .await
            .json();
        let baseline_result = baseline
            .search(&search_params("gross margin benchmark", Some(persona)))
            .await
            .json();
        let rows = result_rows(&result);
        let baseline_rows = result_rows(&baseline_result);

        let helper = row_by_unique_id(&rows, "model.pkg.helper_bridge_anchor");
        let baseline_helper = row_by_unique_id(&baseline_rows, "model.pkg.helper_bridge_anchor");
        let helper_score = helper["score"].as_f64().expect("helper score");
        let baseline_helper_score = baseline_helper["score"]
            .as_f64()
            .expect("baseline helper score");

        assert!(
            (baseline_helper_score - helper_score).abs() < 1e-9,
            "{persona} search should not penalize the helper model: baseline={baseline_helper_score}, helper={helper_score}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_search_standard_summary_surfaces_search_candidates_hint() {
    let searcher = search_candidates_env();
    let result = searcher
        .search(&search_params("gross margin benchmark", None))
        .await
        .json();
    let rows = result_rows(&result);
    let helper = row_by_unique_id(&rows, "model.pkg.helper_bridge_anchor");

    assert_eq!(
        helper["nova_search_candidates"]["analyst"].as_bool(),
        Some(false)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_search_exact_match_keeps_flagged_model_discoverable_for_analysts() {
    let searcher = search_candidates_env();
    let result = searcher
        .search(&search_params("helper_bridge_anchor", Some("analyst")))
        .await
        .json();
    let rows = result_rows(&result);

    assert_eq!(
        rows[0].get("unique_id").and_then(JsonValue::as_str),
        Some("model.pkg.helper_bridge_anchor")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_search_analyst_prefers_canonical_nova_measure_over_dbt_metric_entity() {
    let searcher = semantic_preview_env();
    let result = searcher
        .search(&search_params("gmv", Some("analyst")))
        .await
        .json();
    let rows = result_rows(&result);

    assert!(!rows.is_empty());
    assert_eq!(
        rows[0].get("unique_id").and_then(JsonValue::as_str),
        Some("model.pkg.fact_orders_canonical")
    );
    assert!(
        row_position(&rows, "model.pkg.fact_orders_canonical")
            < row_position(&rows, "metric.pkg.gmv")
    );

    let top = rows[0];
    let preview = top
        .get("semantic_preview")
        .and_then(JsonValue::as_object)
        .expect("expected semantic preview");
    let matched_measures = preview
        .get("matched_measures")
        .and_then(JsonValue::as_array)
        .expect("expected matched measures");
    assert_eq!(
        matched_measures[0].get("name").and_then(JsonValue::as_str),
        Some("gmv")
    );
    assert_eq!(
        matched_measures[0]
            .get("expression")
            .and_then(JsonValue::as_str),
        Some("sum(gmv_amount)")
    );
    assert_eq!(
        matched_measures[0]
            .get("match_type")
            .and_then(JsonValue::as_str),
        Some("name")
    );
    assert_eq!(
        matched_measures[0]
            .get("canonical")
            .and_then(JsonValue::as_bool),
        Some(true)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_search_semantic_preview_surfaces_field_matches_and_analyst_payload() {
    let searcher = semantic_preview_env();
    let result = searcher
        .search(&search_params("amount", Some("analyst")))
        .await
        .json();
    let rows = result_rows(&result);
    let top = row_by_unique_id(&rows, "model.pkg.fact_orders_canonical");
    let preview = top
        .get("semantic_preview")
        .and_then(JsonValue::as_object)
        .expect("expected semantic preview");
    let matched_measures = preview
        .get("matched_measures")
        .and_then(JsonValue::as_array)
        .expect("expected matched measures");

    assert_eq!(
        matched_measures[0]
            .get("match_type")
            .and_then(JsonValue::as_str),
        Some("field")
    );
    assert_eq!(
        matched_measures[0].get("field").and_then(JsonValue::as_str),
        Some("gmv_amount")
    );

    let analyst_payload = top
        .get("persona_payload")
        .and_then(JsonValue::as_object)
        .expect("expected analyst payload");
    let analyst_preview = analyst_payload
        .get("semantic_preview")
        .and_then(JsonValue::as_object)
        .expect("expected analyst semantic preview");
    assert_eq!(
        analyst_preview
            .get("matched_measures")
            .and_then(JsonValue::as_array)
            .and_then(|items| items.first())
            .and_then(|item| item.get("name"))
            .and_then(JsonValue::as_str),
        Some("gmv")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_search_metric_level_canonical_boosts_template_model() {
    let searcher = semantic_preview_env();
    let result = searcher
        .search(&search_params("aov", Some("analyst")))
        .await
        .json();
    let rows = result_rows(&result);

    assert!(!rows.is_empty());
    assert_eq!(
        rows[0].get("unique_id").and_then(JsonValue::as_str),
        Some("model.pkg.orders_semantic_templates")
    );

    let top = rows[0];
    let preview = top
        .get("semantic_preview")
        .and_then(JsonValue::as_object)
        .expect("expected semantic preview");
    let matched_metrics = preview
        .get("matched_metrics")
        .and_then(JsonValue::as_array)
        .expect("expected matched metrics");
    assert_eq!(
        matched_metrics[0].get("name").and_then(JsonValue::as_str),
        Some("average_order_value")
    );
    assert_eq!(
        matched_metrics[0]
            .get("canonical")
            .and_then(JsonValue::as_bool),
        Some(true)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_search_indicator_returns_canonical_measure_context() {
    let searcher = semantic_preview_env();
    let result = searcher
        .search_indicator(&SearchIndicatorParams {
            query: "gmv".to_string(),
            resource_types: vec!["model".to_string()],
            indicator_types: vec!["measure".to_string()],
            persona: Some("analyst".to_string()),
            pagination: PaginationParams {
                limit: 10,
                offset: 0,
            },
            min_score: None,
        })
        .await
        .json();
    let rows = result_rows(&result);

    assert!(!rows.is_empty());
    assert_eq!(
        rows[0].get("indicator_name").and_then(JsonValue::as_str),
        Some("gmv")
    );
    assert_eq!(
        rows[0].get("indicator_type").and_then(JsonValue::as_str),
        Some("measure")
    );
    assert_eq!(
        rows[0].get("parent_unique_id").and_then(JsonValue::as_str),
        Some("model.pkg.fact_orders_canonical")
    );
    assert_eq!(
        rows[0].get("canonical").and_then(JsonValue::as_bool),
        Some(true)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_search_indicator_prefers_generic_canonical_measure_for_generic_query() {
    let searcher = semantic_preview_env();
    let result = searcher
        .search_indicator(&SearchIndicatorParams {
            query: "what was gmv for spain last week".to_string(),
            resource_types: vec!["model".to_string()],
            indicator_types: vec!["measure".to_string()],
            persona: Some("analyst".to_string()),
            pagination: PaginationParams {
                limit: 10,
                offset: 0,
            },
            min_score: None,
        })
        .await
        .json();
    let rows = result_rows(&result);

    assert!(!rows.is_empty());
    assert_eq!(
        rows[0].get("indicator_name").and_then(JsonValue::as_str),
        Some("gmv")
    );
    assert_eq!(
        rows[0].get("parent_unique_id").and_then(JsonValue::as_str),
        Some("model.pkg.fact_orders_canonical")
    );
    let parent_groups = result
        .get("parent_groups")
        .and_then(JsonValue::as_array)
        .expect("expected parent_groups");
    assert!(!parent_groups.is_empty());
    assert_eq!(
        parent_groups[0]
            .get("parent_unique_id")
            .and_then(JsonValue::as_str),
        Some("model.pkg.fact_orders_canonical")
    );
    let support_signals = rows[0]
        .get("support_signals")
        .and_then(JsonValue::as_object)
        .expect("expected support_signals");
    assert_eq!(
        support_signals
            .get("matched_example_values")
            .and_then(JsonValue::as_array)
            .and_then(|values| values.first())
            .and_then(JsonValue::as_str),
        Some("spain")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_search_indicator_returns_metric_with_parent_grain() {
    let searcher = semantic_preview_env();
    let result = searcher
        .search_indicator(&SearchIndicatorParams {
            query: "aov".to_string(),
            resource_types: vec!["model".to_string()],
            indicator_types: vec!["metric".to_string()],
            persona: Some("analyst".to_string()),
            pagination: PaginationParams {
                limit: 10,
                offset: 0,
            },
            min_score: None,
        })
        .await
        .json();
    let rows = result_rows(&result);

    assert!(!rows.is_empty());
    assert_eq!(
        rows[0].get("indicator_name").and_then(JsonValue::as_str),
        Some("average_order_value")
    );
    assert_eq!(
        rows[0].get("indicator_type").and_then(JsonValue::as_str),
        Some("metric")
    );
    assert_eq!(
        rows[0].get("parent_unique_id").and_then(JsonValue::as_str),
        Some("model.pkg.orders_semantic_templates")
    );
    assert_eq!(
        rows[0]
            .get("grain")
            .and_then(|grain| grain.get("time_field"))
            .and_then(JsonValue::as_str),
        Some("order_date")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_search_surfaces_metadata_support_signals() {
    let searcher = semantic_preview_env();
    let result = searcher
        .search(&search_params(
            "what was gmv for spain last week",
            Some("analyst"),
        ))
        .await
        .json();
    let rows = result_rows(&result);
    assert_eq!(
        rows[0].get("unique_id").and_then(JsonValue::as_str),
        Some("model.pkg.fact_orders_canonical")
    );
    let top = row_by_unique_id(&rows, "model.pkg.fact_orders_canonical");
    let support_signals = top
        .get("support_signals")
        .and_then(JsonValue::as_object)
        .expect("expected support_signals");

    assert_eq!(
        support_signals
            .get("matched_example_values")
            .and_then(JsonValue::as_array)
            .and_then(|values| values.first())
            .and_then(JsonValue::as_str),
        Some("spain")
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
    let searcher = ManifestSearch::new(cfg)
        .expect("Failed to load fixture manifest")
        .search;
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
