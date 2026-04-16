//! Integration tests for search tool behavior.
#[path = "support/config.rs"]
mod support_config;
#[path = "support/fixtures.rs"]
mod support_fixtures;
#[path = "support/json.rs"]
mod support_json;

use dbt_nova::params::PaginationParams;
use dbt_nova::params::{DetailLevel, SearchParams};
use support_fixtures::load_fixture;
use support_json::json;

#[tokio::test(flavor = "multi_thread")]
async fn rejects_too_short_query() {
    let searcher = load_fixture("minimal.json").unwrap();
    let params = SearchParams {
        query: "a".into(),
        resource_types: vec![],
        persona: None,
        detail: DetailLevel::Full,
        min_score: None,
        fuzzy: false,
        include_highlights: false,
        include_sql: false,
        explain: false,
        pagination: PaginationParams {
            limit: 10,
            offset: 0,
        },
    };
    let result = json(searcher.search(&params).await);
    assert_eq!(result.get("success").and_then(|v| v.as_bool()), Some(false));
    assert_eq!(
        result.get("error_code").and_then(|v| v.as_str()),
        Some("INVALID_PARAMS")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn rejects_offset_over_limit() {
    let searcher = load_fixture("minimal.json").unwrap();
    let params = SearchParams {
        query: "model".into(),
        resource_types: vec![],
        persona: None,
        detail: DetailLevel::Standard,
        min_score: None,
        fuzzy: false,
        include_highlights: false,
        include_sql: false,
        explain: false,
        pagination: PaginationParams {
            limit: 10,
            offset: 50_000,
        },
    };
    let result = json(searcher.search(&params).await);
    assert_eq!(result.get("success").and_then(|v| v.as_bool()), Some(false));
    assert_eq!(
        result.get("error_code").and_then(|v| v.as_str()),
        Some("INVALID_PARAMS")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn include_full_false_returns_summary() {
    let searcher = load_fixture("minimal.json").unwrap();
    let params = SearchParams {
        query: "model".into(),
        resource_types: vec!["model".into()],
        persona: None,
        detail: DetailLevel::Standard,
        min_score: None,
        fuzzy: false,
        include_highlights: false,
        include_sql: false,
        explain: false,
        pagination: PaginationParams {
            limit: 5,
            offset: 0,
        },
    };
    let result = json(searcher.search(&params).await);
    let first = result
        .get("data")
        .and_then(|d| d.as_array())
        .and_then(|a| a.first())
        .unwrap();
    assert!(first.get("unique_id").is_some());
    assert!(first.get("name").is_some());
    assert!(first.get("columns").is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn analyst_search_has_high_signal_persona_payload() {
    let searcher = load_fixture("perfect_model.json").unwrap();
    let params = SearchParams {
        query: "perfect model".into(),
        resource_types: vec!["model".into()],
        persona: Some("analyst".into()),
        detail: DetailLevel::Standard,
        min_score: None,
        fuzzy: false,
        include_highlights: false,
        include_sql: false,
        explain: false,
        pagination: PaginationParams {
            limit: 3,
            offset: 0,
        },
    };
    let result = json(searcher.search(&params).await);
    let results = result
        .get("data")
        .and_then(|d| d.as_array())
        .expect("analyst result array");

    let first = results.first().expect("first analyst result");
    assert!(first.get("build_config").is_none());
    assert!(first.get("tests_summary").is_none());

    let payload = first
        .get("persona_payload")
        .and_then(|v| v.as_object())
        .expect("analyst persona_payload");
    assert_eq!(
        payload.get("focus").and_then(|v| v.as_str()),
        Some("business_discovery")
    );
    assert!(payload.get("business_definition").is_some());
    assert!(payload.get("key_dimensions").is_some());

    let signals = payload
        .get("selection_signals")
        .and_then(|v| v.as_object())
        .expect("analyst selection_signals");
    assert!(signals.get("has_metric_definition").is_some());
    assert!(signals.get("has_measure_definition").is_some());
    assert!(signals.get("has_grain").is_some());
    assert!(signals.get("has_time_field").is_some());
    assert!(signals.get("dimension_overlap").is_some());
    assert!(matches!(
        signals.get("confidence_band").and_then(|v| v.as_str()),
        Some("low" | "medium" | "high")
    ));

    assert!(payload.get("selection_rationale").is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn analyst_key_dimensions_excludes_metric_value_columns() {
    let searcher = load_fixture("analyst_metric_dims.json").unwrap();
    let params = SearchParams {
        query: "country device period checkout step 1".into(),
        resource_types: vec!["model".into()],
        persona: Some("analyst".into()),
        detail: DetailLevel::Standard,
        min_score: None,
        fuzzy: false,
        include_highlights: false,
        include_sql: false,
        explain: false,
        pagination: PaginationParams {
            limit: 3,
            offset: 0,
        },
    };
    let result = json(searcher.search(&params).await);
    let first = result
        .get("data")
        .and_then(|d| d.as_array())
        .and_then(|arr| arr.first())
        .expect("first analyst result");
    let payload = first
        .get("persona_payload")
        .and_then(|v| v.as_object())
        .expect("analyst persona_payload");

    let key_dimensions = payload
        .get("key_dimensions")
        .and_then(|v| v.as_array())
        .expect("key_dimensions");
    let key_dimension_names: Vec<&str> = key_dimensions.iter().filter_map(|v| v.as_str()).collect();
    assert!(key_dimension_names.contains(&"country_code"));
    assert!(key_dimension_names.contains(&"device_type"));
    assert!(!key_dimension_names.contains(&"checkout_step_1"));

    let candidate_metrics = payload
        .get("candidate_metrics")
        .and_then(|v| v.as_array())
        .expect("candidate_metrics");
    assert!(
        candidate_metrics
            .iter()
            .any(|v| v.as_str() == Some("checkout_step_1"))
    );

    assert_eq!(
        payload.get("time_field").and_then(|v| v.as_str()),
        Some("period")
    );

    let signals = payload
        .get("selection_signals")
        .and_then(|v| v.as_object())
        .expect("selection_signals");
    assert_eq!(
        signals.get("has_grain").and_then(|v| v.as_bool()),
        Some(true)
    );
    assert_eq!(
        signals.get("has_time_field").and_then(|v| v.as_bool()),
        Some(true)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn search_respects_resource_type_filter_after_hybrid_fusion() {
    let searcher = load_fixture("perfect_model.json").unwrap();
    let unfiltered = SearchParams {
        query: "model_level_test".into(),
        resource_types: vec![],
        persona: Some("engineer".into()),
        detail: DetailLevel::Standard,
        min_score: None,
        fuzzy: false,
        include_highlights: false,
        include_sql: false,
        explain: false,
        pagination: PaginationParams {
            limit: 10,
            offset: 0,
        },
    };
    let unfiltered_result = json(searcher.search(&unfiltered).await);
    let unfiltered_rows = unfiltered_result
        .get("data")
        .and_then(|d| d.as_array())
        .expect("unfiltered rows");
    assert!(
        unfiltered_rows
            .iter()
            .any(|row| row.get("resource_type").and_then(|v| v.as_str()) == Some("test")),
        "unfiltered query should include test resources"
    );

    let model_only = SearchParams {
        resource_types: vec!["model".into()],
        ..unfiltered
    };
    let filtered_result = json(searcher.search(&model_only).await);
    let filtered_rows = filtered_result
        .get("data")
        .and_then(|d| d.as_array())
        .expect("filtered rows");
    assert!(
        filtered_rows
            .iter()
            .all(|row| row.get("resource_type").and_then(|v| v.as_str()) == Some("model")),
        "model-filtered query should return only models"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn engineer_search_has_impact_focused_payload() {
    let searcher = load_fixture("nova_manifest.json").unwrap();
    let params = SearchParams {
        query: "campaign features rollup".into(),
        resource_types: vec!["model".into()],
        persona: Some("engineer".into()),
        detail: DetailLevel::Standard,
        min_score: None,
        fuzzy: false,
        include_highlights: false,
        include_sql: false,
        explain: false,
        pagination: PaginationParams {
            limit: 3,
            offset: 0,
        },
    };
    let result = json(searcher.search(&params).await);
    let first = result
        .get("data")
        .and_then(|d| d.as_array())
        .and_then(|a| a.first())
        .expect("first engineer result");
    let payload = first
        .get("persona_payload")
        .and_then(|v| v.as_object())
        .expect("engineer persona_payload");
    assert_eq!(
        payload.get("focus").and_then(|v| v.as_str()),
        Some("implementation_impact")
    );
    assert!(
        payload
            .get("blast_radius_count")
            .and_then(|v| v.as_u64())
            .is_some()
    );
    assert!(matches!(
        payload.get("change_risk").and_then(|v| v.as_str()),
        Some("low" | "medium" | "high")
    ));
    assert!(matches!(
        payload.get("readiness_band").and_then(|v| v.as_str()),
        Some("low" | "medium" | "high")
    ));
    assert!(payload.get("impacted_tests").is_some());
    let signals = payload
        .get("selection_signals")
        .and_then(|v| v.as_object())
        .expect("engineer selection_signals");
    for key in [
        "upstream_count",
        "downstream_count",
        "has_lineage",
        "tests_total",
        "documentation_coverage_pct",
        "has_owner",
        "has_primary_key",
        "missing_required_fields",
    ] {
        assert!(signals.get(key).is_some(), "missing engineer signal: {key}");
    }
    let doc_coverage_pct = signals
        .get("documentation_coverage_pct")
        .and_then(|v| v.as_f64())
        .expect("documentation_coverage_pct should be numeric");
    assert!(
        (0.0..=100.0).contains(&doc_coverage_pct),
        "documentation_coverage_pct should be 0..=100, got {doc_coverage_pct}"
    );
    assert!(payload.get("selection_rationale").is_some());
    let recommended_tools = payload
        .get("recommended_tools")
        .and_then(|v| v.as_array())
        .expect("engineer recommended_tools");
    assert!(
        recommended_tools
            .iter()
            .any(|v| v.as_str() == Some("get_lineage")),
        "recommended_tools should include get_lineage"
    );
    assert!(first.get("build_config").is_none());
    assert!(first.get("upstream_count").is_some());
    assert!(first.get("downstream_count").is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn engineer_payload_does_not_flag_docs_gap_when_coverage_is_100pct() {
    let searcher = load_fixture("perfect_model.json").unwrap();
    let params = SearchParams {
        query: "perfect model".into(),
        resource_types: vec!["model".into()],
        persona: Some("engineer".into()),
        detail: DetailLevel::Standard,
        min_score: None,
        fuzzy: false,
        include_highlights: false,
        include_sql: false,
        explain: false,
        pagination: PaginationParams {
            limit: 3,
            offset: 0,
        },
    };
    let result = json(searcher.search(&params).await);
    let first = result
        .get("data")
        .and_then(|d| d.as_array())
        .and_then(|a| a.first())
        .expect("first engineer result");
    let payload = first
        .get("persona_payload")
        .and_then(|v| v.as_object())
        .expect("engineer persona_payload");

    let signals = payload
        .get("selection_signals")
        .and_then(|v| v.as_object())
        .expect("engineer selection_signals");
    let doc_coverage_pct = signals
        .get("documentation_coverage_pct")
        .and_then(|v| v.as_f64())
        .expect("documentation_coverage_pct should be numeric");
    assert_eq!(doc_coverage_pct, 100.0);

    let rationale = payload
        .get("selection_rationale")
        .and_then(|v| v.as_str())
        .expect("selection_rationale");
    assert!(
        !rationale.contains("documentation coverage below target"),
        "rationale should not include docs-gap message when coverage is 100%"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn governance_search_has_policy_payload() {
    let searcher = load_fixture("nova_manifest.json").unwrap();
    let params = SearchParams {
        query: "referring domain test".into(),
        resource_types: vec![],
        persona: Some("governance".into()),
        detail: DetailLevel::Standard,
        min_score: None,
        fuzzy: false,
        include_highlights: false,
        include_sql: false,
        explain: false,
        pagination: PaginationParams {
            limit: 5,
            offset: 0,
        },
    };
    let result = json(searcher.search(&params).await);
    let first = result
        .get("data")
        .and_then(|d| d.as_array())
        .and_then(|a| a.first())
        .expect("first governance result");
    let payload = first
        .get("persona_payload")
        .and_then(|v| v.as_object())
        .expect("governance persona_payload");
    assert_eq!(
        payload.get("focus").and_then(|v| v.as_str()),
        Some("governance_assurance")
    );
    assert!(matches!(
        payload.get("policy_risk").and_then(|v| v.as_str()),
        Some("low" | "medium" | "high" | "critical")
    ));
    assert!(matches!(
        payload.get("gate_status").and_then(|v| v.as_str()),
        Some("pass" | "fail")
    ));
    let gate_signals = payload
        .get("gate_signals")
        .and_then(|v| v.as_object())
        .expect("governance gate_signals");
    for key in [
        "required_fields_pass",
        "metadata_grade_pass",
        "docs_pass",
        "tests_pass",
        "owner_pass",
        "compliance_pass",
    ] {
        assert!(
            gate_signals.get(key).and_then(|v| v.as_bool()).is_some(),
            "missing boolean gate signal: {key}"
        );
    }
    assert!(payload.get("missing_governance_fields").is_some());
    assert!(payload.get("blocking_reasons").is_some());
    assert!(payload.get("metadata_grade").is_some());
    assert!(first.get("metadata_score").is_some());
    assert!(payload.get("lineage_health").is_some());
    assert!(payload.get("manifest_health").is_some());
    assert!(payload.get("quality_warnings").is_some());

    let docs_pass = gate_signals
        .get("docs_pass")
        .and_then(|v| v.as_bool())
        .expect("docs_pass");
    let doc_coverage = payload
        .get("documentation_coverage_pct")
        .and_then(|v| v.as_f64())
        .expect("documentation_coverage_pct");
    assert!(
        (0.0..=100.0).contains(&doc_coverage),
        "documentation_coverage_pct should be 0..=100, got {doc_coverage}"
    );
    let expected_docs_pass = doc_coverage >= 80.0;
    assert_eq!(docs_pass, expected_docs_pass);

    let manifest_health = payload
        .get("manifest_health")
        .and_then(|v| v.as_object())
        .expect("manifest_health object");
    assert!(
        manifest_health
            .get("models_ref_calls_without_dependencies")
            .and_then(|v| v.as_u64())
            .is_some(),
        "manifest_health should expose models_ref_calls_without_dependencies"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn governance_payload_docs_gate_matches_100pct_coverage() {
    let searcher = load_fixture("perfect_model.json").unwrap();
    let params = SearchParams {
        query: "perfect model".into(),
        resource_types: vec!["model".into()],
        persona: Some("governance".into()),
        detail: DetailLevel::Standard,
        min_score: None,
        fuzzy: false,
        include_highlights: false,
        include_sql: false,
        explain: false,
        pagination: PaginationParams {
            limit: 3,
            offset: 0,
        },
    };
    let result = json(searcher.search(&params).await);
    let first = result
        .get("data")
        .and_then(|d| d.as_array())
        .and_then(|a| a.first())
        .expect("first governance result");
    let payload = first
        .get("persona_payload")
        .and_then(|v| v.as_object())
        .expect("governance persona_payload");

    let doc_coverage = payload
        .get("documentation_coverage_pct")
        .and_then(|v| v.as_f64())
        .expect("documentation_coverage_pct");
    assert_eq!(doc_coverage, 100.0);

    let gate_signals = payload
        .get("gate_signals")
        .and_then(|v| v.as_object())
        .expect("governance gate_signals");
    assert_eq!(
        gate_signals.get("docs_pass").and_then(|v| v.as_bool()),
        Some(true)
    );

    let blocking_reasons = payload
        .get("blocking_reasons")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    assert!(
        !blocking_reasons
            .iter()
            .any(|v| v.as_str() == Some("documentation_coverage_below_threshold")),
        "blocking_reasons should not include docs threshold failure when coverage is 100%"
    );
}
