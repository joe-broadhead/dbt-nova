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
        detail: Some(DetailLevel::Standard),
        min_score: None,
        fuzzy: false,
        include_highlights: false,
        include_sql: false,
        explain: false,
        pagination: PaginationParams {
            limit: Some(10),
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

fn indicator_row<'a>(
    rows: &'a [&JsonValue],
    parent_unique_id: &str,
    indicator_name: &str,
) -> &'a JsonValue {
    rows.iter()
        .copied()
        .find(|row| {
            row.get("parent_unique_id").and_then(JsonValue::as_str) == Some(parent_unique_id)
                && row.get("indicator_name").and_then(JsonValue::as_str) == Some(indicator_name)
        })
        .unwrap_or_else(|| panic!("missing indicator row for {parent_unique_id}:{indicator_name}"))
}

fn indicator_row_position(
    rows: &[&JsonValue],
    parent_unique_id: &str,
    indicator_name: &str,
) -> usize {
    rows.iter()
        .position(|row| {
            row.get("parent_unique_id").and_then(JsonValue::as_str) == Some(parent_unique_id)
                && row.get("indicator_name").and_then(JsonValue::as_str) == Some(indicator_name)
        })
        .unwrap_or_else(|| panic!("missing indicator row for {parent_unique_id}:{indicator_name}"))
}

// Search Tool Tests
#[tokio::test(flavor = "multi_thread")]
async fn test_search_summaries_include_provenance_tiers() {
    let semantic_searcher = semantic_preview_env();
    let semantic_result = semantic_searcher
        .search(&search_params("gmv", None))
        .await
        .json();
    let semantic_rows = result_rows(&semantic_result);
    let semantic_row = row_by_unique_id(&semantic_rows, "model.pkg.fact_orders_canonical");
    assert_eq!(
        semantic_row
            .pointer("/provenance/tier")
            .and_then(JsonValue::as_str),
        Some("semantic_layer")
    );
    assert_eq!(
        semantic_row
            .pointer("/provenance/freshness/status")
            .and_then(JsonValue::as_str),
        Some("unknown")
    );
    assert!(
        semantic_row
            .pointer("/provenance/readiness/metadata_grade")
            .is_some()
    );

    let curated_searcher = get_searcher();
    let curated_result = curated_searcher
        .search(&search_params("traffic", None))
        .await
        .json();
    let curated_rows = result_rows(&curated_result);
    let curated_row = row_by_unique_id(&curated_rows, "model.nova_test.stg__traffic_sessions");
    assert_eq!(
        curated_row
            .pointer("/provenance/tier")
            .and_then(JsonValue::as_str),
        Some("curated")
    );

    let raw_searcher = get_searcher_with_fixture("undocumented.json");
    let raw_result = raw_searcher
        .search(&search_params("docless", None))
        .await
        .json();
    let raw_rows = result_rows(&raw_result);
    let raw_row = row_by_unique_id(&raw_rows, "model.pkg.docless");
    assert_eq!(
        raw_row
            .pointer("/provenance/tier")
            .and_then(JsonValue::as_str),
        Some("raw")
    );
    assert_eq!(
        raw_row
            .pointer("/provenance/freshness/status")
            .and_then(JsonValue::as_str),
        Some("unknown")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_search_full_detail_includes_provenance() {
    let searcher = get_searcher_with_fixture("undocumented.json");
    let mut params = search_params("docless", None);
    params.detail = Some(DetailLevel::Full);

    let result = searcher.search(&params).await.json();
    let rows = result_rows(&result);
    let row = row_by_unique_id(&rows, "model.pkg.docless");
    assert_eq!(
        row.pointer("/provenance/tier").and_then(JsonValue::as_str),
        Some("raw")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_search_by_name() {
    let searcher = get_searcher();
    let params = SearchParams {
        query: "campaign".to_string(),
        resource_types: vec![],
        persona: None,
        detail: Some(DetailLevel::Full),
        min_score: None,
        fuzzy: false,
        include_highlights: false,
        include_sql: false,
        explain: false,
        pagination: PaginationParams {
            limit: Some(10),
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
        detail: Some(DetailLevel::Standard),
        min_score: None,
        fuzzy: false,
        include_highlights: false,
        include_sql: false,
        explain: false,
        pagination: PaginationParams {
            limit: Some(50),
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
        detail: Some(DetailLevel::Standard),
        min_score: None,
        fuzzy: false,
        include_highlights: false,
        include_sql: false,
        explain: false,
        pagination: PaginationParams {
            limit: Some(5),
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
        detail: Some(DetailLevel::Standard),
        min_score: None,
        fuzzy: false,
        include_highlights: false,
        include_sql: false,
        explain: false,
        pagination: PaginationParams {
            limit: Some(5),
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
        detail: Some(DetailLevel::Standard),
        min_score: None,
        fuzzy: false,
        include_highlights: false,
        include_sql: false,
        explain: false,
        pagination: PaginationParams {
            limit: Some(10),
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
        detail: Some(DetailLevel::Standard),
        min_score: None,
        fuzzy: false,
        include_highlights: false,
        include_sql: false,
        explain: false,
        pagination: PaginationParams {
            limit: Some(10),
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
        detail: Some(DetailLevel::Standard),
        pagination: PaginationParams {
            limit: Some(10),
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
                limit: Some(10),
                offset: 0,
            },
            min_score: None,
            explain: false,
            ..Default::default()
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
    assert!(
        rows[0].get("explain").is_none(),
        "indicator explain should be omitted unless explicitly requested"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_search_indicator_prefers_generic_canonical_measure_for_generic_query() {
    let searcher = semantic_preview_env();
    let result = searcher
        .search_indicator(&SearchIndicatorParams {
            query: "what was gmv for alpha last week".to_string(),
            resource_types: vec!["model".to_string()],
            indicator_types: vec!["measure".to_string()],
            persona: Some("analyst".to_string()),
            pagination: PaginationParams {
                limit: Some(10),
                offset: 0,
            },
            min_score: None,
            explain: false,
            ..Default::default()
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
        Some("alpha")
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
                limit: Some(10),
                offset: 0,
            },
            min_score: None,
            explain: false,
            ..Default::default()
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
async fn test_indicator_inventory_lists_indicator_context_deterministically() {
    let searcher = semantic_preview_env();
    let result = searcher
        .indicator_inventory(&IndicatorInventoryParams {
            resource_types: vec!["model".to_string()],
            indicator_types: vec!["measure".to_string()],
            canonical_only: false,
            pagination: PaginationParams {
                limit: Some(10),
                offset: 0,
            },
        })
        .await
        .json();
    let rows = result_rows(&result);

    assert!(!rows.is_empty());
    let canonical_gmv = indicator_row(&rows, "model.pkg.fact_orders_canonical", "gmv");
    assert_eq!(
        canonical_gmv
            .get("measure_type")
            .and_then(JsonValue::as_str),
        Some("sum")
    );
    assert_eq!(
        canonical_gmv.get("field").and_then(JsonValue::as_str),
        Some("gmv_amount")
    );
    assert_eq!(
        canonical_gmv
            .get("grain")
            .and_then(|grain| grain.get("time_field"))
            .and_then(JsonValue::as_str),
        Some("order_date")
    );
    assert!(
        indicator_row_position(&rows, "model.pkg.fact_orders_canonical", "gmv")
            < indicator_row_position(&rows, "model.pkg.fact_orders_channel", "gmv")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_indicator_inventory_canonical_only_filters_noncanonical_rows() {
    let searcher = semantic_preview_env();
    let result = searcher
        .indicator_inventory(&IndicatorInventoryParams {
            resource_types: vec!["model".to_string()],
            indicator_types: vec![],
            canonical_only: true,
            pagination: PaginationParams {
                limit: Some(20),
                offset: 0,
            },
        })
        .await
        .json();
    let rows = result_rows(&result);

    assert!(!rows.is_empty());
    assert!(
        rows.iter()
            .all(|row| row.get("canonical").and_then(JsonValue::as_bool) == Some(true))
    );
    assert!(rows.iter().all(|row| {
        row.get("parent_unique_id").and_then(JsonValue::as_str)
            != Some("model.pkg.fact_orders_channel")
    }));
    assert!(
        indicator_row(&rows, "model.pkg.fact_orders_canonical", "gmv")
            .get("synonyms")
            .and_then(JsonValue::as_array)
            .is_some()
    );
    let entity_canonical = indicator_row(
        &rows,
        "model.pkg.fact_orders_entity_canonical_only",
        "net_revenue",
    );
    assert_eq!(
        entity_canonical
            .get("canonical")
            .and_then(JsonValue::as_bool),
        Some(true)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_search_surfaces_metadata_support_signals() {
    let searcher = semantic_preview_env();
    let result = searcher
        .search(&search_params(
            "what was gmv for alpha last week",
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
        Some("alpha")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_search_explain_surfaces_retrieval_and_score_breakdown() {
    let searcher = semantic_preview_env();
    let mut params = search_params("what was gmv for alpha last week", Some("analyst"));
    params.explain = true;
    let result = searcher.search(&params).await.json();
    let rows = result_rows(&result);
    let top = row_by_unique_id(&rows, "model.pkg.fact_orders_canonical");
    let explain = top
        .get("explain")
        .and_then(JsonValue::as_object)
        .expect("expected row explain");
    assert_eq!(
        explain.get("canonical_entity").and_then(JsonValue::as_bool),
        Some(true)
    );
    assert!(
        explain
            .get("retrieval")
            .and_then(|value| value.get("retrievers"))
            .and_then(|value| value.get("bm25"))
            .and_then(|value| value.get("rank"))
            .and_then(JsonValue::as_u64)
            .is_some(),
        "expected bm25 retrieval contribution"
    );
    assert_eq!(
        explain.get("final_score").and_then(JsonValue::as_f64),
        top.get("score").and_then(JsonValue::as_f64)
    );

    let payload = result
        .get("explain")
        .and_then(JsonValue::as_object)
        .expect("expected top-level explain payload");
    assert_eq!(
        payload
            .get("query_tokens")
            .and_then(JsonValue::as_array)
            .and_then(|tokens| tokens.first())
            .and_then(JsonValue::as_str),
        Some("what")
    );
    assert!(
        payload
            .get("retrievers_used")
            .and_then(JsonValue::as_array)
            .is_some_and(|retrievers| !retrievers.is_empty())
    );
    assert_eq!(
        payload.get("reranker_applied").and_then(JsonValue::as_bool),
        Some(false)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_search_indicator_explain_surfaces_rrf_breakdown() {
    let searcher = semantic_preview_env();
    let result = searcher
        .search_indicator(&SearchIndicatorParams {
            query: "what was gmv for alpha last week".to_string(),
            resource_types: vec!["model".to_string()],
            indicator_types: vec!["measure".to_string()],
            persona: Some("analyst".to_string()),
            pagination: PaginationParams {
                limit: Some(10),
                offset: 0,
            },
            min_score: None,
            explain: true,
            ..Default::default()
        })
        .await
        .json();
    let rows = result_rows(&result);
    let top = indicator_row(&rows, "model.pkg.fact_orders_canonical", "gmv");
    let explain = top
        .get("explain")
        .and_then(JsonValue::as_object)
        .expect("expected indicator explain");
    assert!(
        explain
            .get("rrf_bonus")
            .and_then(JsonValue::as_f64)
            .is_some(),
        "expected rrf bonus"
    );
    assert!(
        explain
            .get("retrieval")
            .and_then(|value| value.get("retrievers"))
            .and_then(|value| value.get("indicator_local"))
            .and_then(|value| value.get("rank"))
            .and_then(JsonValue::as_u64)
            .is_some(),
        "expected indicator_local retrieval contribution"
    );
    assert_eq!(
        explain.get("final_score").and_then(JsonValue::as_f64),
        top.get("score").and_then(JsonValue::as_f64)
    );

    let payload = result
        .get("explain")
        .and_then(JsonValue::as_object)
        .expect("expected top-level explain payload");
    assert!(
        payload
            .get("retrievers_used")
            .and_then(JsonValue::as_array)
            .is_some_and(|retrievers| retrievers.len() >= 2)
    );
    assert_eq!(
        payload.get("reranker_applied").and_then(JsonValue::as_bool),
        Some(false)
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
        detail: Some(DetailLevel::Standard),
        min_score: None,
        fuzzy: false,
        include_highlights: false,
        include_sql: false,
        explain: false,
        pagination: PaginationParams {
            limit: Some(10),
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
        detail: Some(DetailLevel::Standard),
        min_score: None,
        fuzzy: false,
        include_highlights: false,
        include_sql: false,
        explain: false,
        pagination: PaginationParams {
            limit: Some(1),
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
