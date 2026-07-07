//! Tests for overlap, grain, and modelling consistency tooling.
use super::common::*;
use crate::config::SearchConfig;

fn modeling_env() -> TestSearchEnv {
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

fn result_rows(result: &JsonValue) -> Vec<&JsonValue> {
    result
        .get("data")
        .and_then(JsonValue::as_array)
        .map(|rows| rows.iter().collect())
        .unwrap_or_default()
}

fn finding_mentions_entity(finding: &JsonValue, unique_id: &str) -> bool {
    finding["entities"].as_array().is_some_and(|entities| {
        entities
            .iter()
            .any(|entity| entity["unique_id"] == unique_id)
    })
}

fn finding_code_present(findings: &[&JsonValue], code: &str) -> bool {
    findings
        .iter()
        .any(|finding| finding["code"].as_str() == Some(code))
}

#[tokio::test(flavor = "multi_thread")]
async fn test_compare_grains_matches_entity_and_metric_grain() {
    let searcher = modeling_env();
    let result = searcher
        .compare_grains(&CompareGrainsParams {
            entity1: "fact_orders_canonical".to_string(),
            entity1_resource_type: Some("model".to_string()),
            entity2: "orders_semantic_templates".to_string(),
            entity2_resource_type: Some("model".to_string()),
        })
        .await
        .json();

    let data = result.get("data").expect("data");
    assert_eq!(
        data.get("exact_match").and_then(JsonValue::as_bool),
        Some(false)
    );
    assert_eq!(
        data.get("same_time_field").and_then(JsonValue::as_bool),
        Some(true)
    );
    assert_eq!(
        data.get("shared_dimensions")
            .and_then(JsonValue::as_array)
            .map(Vec::len),
        Some(2)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_find_entity_overlap_surfaces_closest_order_entities() {
    let searcher = modeling_env();
    let result = searcher
        .find_entity_overlap(&FindEntityOverlapParams {
            id_or_name: None,
            resource_type: None,
            resource_types: vec!["model".to_string()],
            pagination: PaginationParams {
                limit: Some(10),
                offset: 0,
            },
            min_score: None,
        })
        .await
        .json();
    let rows = result_rows(&result);

    assert!(!rows.is_empty());
    let top = rows[0];
    let entity1 = top
        .get("entity1")
        .and_then(|value| value.get("unique_id"))
        .and_then(JsonValue::as_str)
        .expect("entity1 id");
    let entity2 = top
        .get("entity2")
        .and_then(|value| value.get("unique_id"))
        .and_then(JsonValue::as_str)
        .expect("entity2 id");
    let pair = [entity1, entity2];
    assert!(pair.contains(&"model.pkg.fact_orders_canonical"));
    assert!(pair.contains(&"model.pkg.fact_orders_channel"));
    let evidence = top.get("evidence").expect("evidence");
    assert_eq!(
        evidence
            .get("shared_indicators")
            .and_then(JsonValue::as_array)
            .and_then(|items| items.first())
            .and_then(JsonValue::as_str),
        Some("gmv")
    );
    assert_eq!(
        evidence
            .get("shared_column_names")
            .and_then(JsonValue::as_array)
            .map(Vec::len),
        Some(3)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_modelling_consistency_report_surfaces_duplicate_indicators() {
    let searcher = modeling_env();
    let result = searcher
        .modelling_consistency_report(&ModellingConsistencyReportParams {
            resource_types: vec!["model".to_string()],
            pagination: PaginationParams {
                limit: Some(10),
                offset: 0,
            },
            min_score: None,
        })
        .await
        .json();

    let data = result.get("data").expect("data");
    let summary = data.get("summary").expect("summary");
    assert_eq!(
        data["agent_modelling_schema_version"].as_str(),
        Some("agent_modelling.v1")
    );
    assert!(
        data["agent_modelling_finding_count"]
            .as_u64()
            .is_some_and(|count| count > 0)
    );
    let agent_modelling_findings = data["agent_modelling_findings"]
        .as_array()
        .expect("agent modelling findings");
    assert_eq!(
        data["agent_modelling_finding_count"].as_u64(),
        Some(agent_modelling_findings.len() as u64)
    );
    assert!(
        summary["section_counts"]["agent_modelling_findings"]
            .as_u64()
            .is_some_and(|count| count > 0)
    );
    assert_eq!(
        summary["agent_modelling"]["total"].as_u64(),
        data["agent_modelling_finding_count"].as_u64()
    );
    assert_eq!(
        summary["agent_modelling"]["truncated"].as_bool(),
        Some(false)
    );
    assert!(
        summary["agent_modelling"]["top_codes"]
            .as_array()
            .is_some_and(|rows| !rows.is_empty())
    );
    assert!(
        summary["agent_modelling"]["top_categories"]
            .as_array()
            .is_some_and(|rows| !rows.is_empty())
    );
    assert!(
        summary["section_counts"]["duplicate_indicators"]
            .as_u64()
            .is_some_and(|count| count > 0)
    );
    assert!(
        summary["top_duplicate_indicator_groups"]
            .as_array()
            .is_some_and(|rows| !rows.is_empty())
    );
    assert!(
        summary["overlap_evidence_categories"]["shared_column_names"]
            .as_u64()
            .is_some_and(|count| count > 0)
    );
    assert!(
        summary["overlap_examples"]
            .as_array()
            .is_some_and(|rows| rows.iter().any(|row| row["shared_column_examples"]
                .as_array()
                .is_some_and(|examples| !examples.is_empty())))
    );
    assert!(summary["drill_down_hints"].as_array().is_some_and(|rows| {
        rows.iter()
            .any(|row| row["tool"].as_str() == Some("find_entity_overlap"))
    }));
    let duplicate_indicators = data
        .get("duplicate_indicators")
        .and_then(JsonValue::as_array)
        .expect("duplicate_indicators");
    assert!(!duplicate_indicators.is_empty());
    assert!(duplicate_indicators.iter().any(|row| {
        row.get("indicator_name").and_then(JsonValue::as_str) == Some("average_order_value")
            && row.get("parent_count").and_then(JsonValue::as_u64) == Some(2)
    }));
    let overlap_candidates = data
        .get("overlap_candidates")
        .and_then(JsonValue::as_array)
        .expect("overlap_candidates");
    assert!(!overlap_candidates.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_agent_modelling_findings_cover_indicator_queryability_and_grain_rules() {
    let searcher = get_searcher_with_fixture("agent_modelling_findings.json");
    let result = searcher
        .modelling_consistency_report(&ModellingConsistencyReportParams {
            resource_types: vec![],
            pagination: PaginationParams {
                limit: Some(100),
                offset: 0,
            },
            min_score: None,
        })
        .await
        .json();

    let data = result.get("data").expect("data");
    let findings = data["agent_modelling_findings"]
        .as_array()
        .expect("agent modelling findings")
        .iter()
        .collect::<Vec<_>>();

    for code in [
        "duplicate_canonical_indicator",
        "duplicate_indicator_without_canonical_parent",
        "indicator_parent_not_queryable",
        "metric_output_column_missing",
        "metric_grain_field_not_in_output",
        "metric_missing_time_field",
        "entity_multiple_grain_variants",
        "semantic_model_missing_primary_entity",
        "semantic_model_missing_time_dimension",
    ] {
        assert!(
            finding_code_present(&findings, code),
            "expected finding code {code} in {findings:#?}"
        );
    }

    assert!(
        findings
            .iter()
            .all(|finding| finding["evidence"].is_object())
    );
    assert!(findings.iter().all(|finding| {
        finding["recommendation"]
            .as_str()
            .is_some_and(|value| !value.is_empty())
    }));
    assert!(findings.iter().all(|finding| {
        finding["drill_down_hints"]
            .as_array()
            .is_some_and(|hints| !hints.is_empty())
    }));

    let duplicate_canonical = findings
        .iter()
        .find(|finding| finding["code"] == "duplicate_canonical_indicator")
        .expect("duplicate canonical finding");
    assert_eq!(duplicate_canonical["severity"].as_str(), Some("blocker"));
    assert_eq!(
        duplicate_canonical["evidence"]["inconsistent_grains"].as_bool(),
        Some(true)
    );

    let not_queryable = findings
        .iter()
        .find(|finding| finding["code"] == "indicator_parent_not_queryable")
        .expect("not queryable finding");
    assert_eq!(not_queryable["severity"].as_str(), Some("blocker"));
    assert_eq!(
        not_queryable["evidence"]["queryable"].as_bool(),
        Some(false)
    );
    assert_eq!(
        not_queryable["evidence"]["queryable_via"].as_str(),
        Some("none")
    );

    assert!(
        !findings.iter().any(|finding| {
            finding["severity"] == "blocker"
                && finding_mentions_entity(finding, "model.pkg.clean_metric_model")
        }),
        "clean relation-backed metric model should not trigger blockers: {findings:#?}"
    );
    assert!(
        !findings.iter().any(|finding| {
            finding_mentions_entity(finding, "metric.pkg.semantic_revenue")
                && matches!(
                    finding["code"].as_str(),
                    Some("indicator_parent_not_queryable" | "metric_missing_time_field")
                )
        }),
        "semantic-layer-backed MetricFlow metric should not look relation-backed or time-field broken: {findings:#?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_modelling_consistency_report_applies_section_offset() {
    let searcher = modeling_env();
    let page_1 = searcher
        .modelling_consistency_report(&ModellingConsistencyReportParams {
            resource_types: vec!["model".to_string()],
            pagination: PaginationParams {
                limit: Some(1),
                offset: 0,
            },
            min_score: None,
        })
        .await
        .json();
    let page_2 = searcher
        .modelling_consistency_report(&ModellingConsistencyReportParams {
            resource_types: vec!["model".to_string()],
            pagination: PaginationParams {
                limit: Some(1),
                offset: 1,
            },
            min_score: None,
        })
        .await
        .json();

    let overlap_page_1 = page_1
        .get("data")
        .and_then(|data| data.get("overlap_candidates"))
        .and_then(JsonValue::as_array)
        .expect("page_1 overlap_candidates");
    let overlap_page_2 = page_2
        .get("data")
        .and_then(|data| data.get("overlap_candidates"))
        .and_then(JsonValue::as_array)
        .expect("page_2 overlap_candidates");

    assert_eq!(overlap_page_1.len(), 1);
    assert_eq!(overlap_page_2.len(), 1);
    assert_ne!(overlap_page_1[0], overlap_page_2[0]);
}
