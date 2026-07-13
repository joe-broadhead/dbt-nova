use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde_json::{Value as JsonValue, json};
use tempfile::TempDir;

use super::patches::append_indicator_meta_patches;
use super::{
    EvalReadinessStatus, IndicatorReadinessFinding, ReadinessFinding, ReadinessThresholdConfig,
    ThresholdRule, ThresholdSeverity, build_agent_readiness_load_config,
    build_agent_readiness_report, build_agent_readiness_tool_response, evaluate_threshold,
    parse_eval_status, parse_json_input, render_markdown_report, write_report,
};
use crate::cli::args::AgentReadinessArgs;
use crate::cli::manifest::execute_manifest_load;
use crate::params::GetAgentReadinessParams;
use crate::tests::common::fixture_manifest_path_string;

fn fixture_path_string(fixture_name: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(format!("tests/fixtures/{fixture_name}"))
        .to_string_lossy()
        .to_string()
}

async fn readiness_report_for_fixture(
    fixture_name: &str,
    storage_instance_id: &str,
) -> super::AgentReadinessReport {
    let temp_dir = TempDir::new().expect("temp dir");
    let args = AgentReadinessArgs {
        manifest_path: Some(fixture_path_string(fixture_name)),
        storage_instance_id: Some(storage_instance_id.to_string()),
        cleanup_storage_on_start: true,
        ..AgentReadinessArgs::default()
    };
    let inputs = super::parse_readiness_inputs(&args).expect("inputs");
    let mut config = build_agent_readiness_load_config(&args).expect("config");
    config.storage_dir = temp_dir.path().to_string_lossy().to_string();
    let loaded = execute_manifest_load(config).await.expect("load");
    build_agent_readiness_report(&loaded.search, &inputs)
        .await
        .expect("report")
}

fn assert_object_has_fields(value: &JsonValue, context: &str, fields: &[&str]) {
    let obj = value
        .as_object()
        .unwrap_or_else(|| panic!("{context} must be an object: {value:#?}"));
    for field in fields {
        assert!(
            obj.contains_key(*field),
            "{context} missing required field `{field}` in {value:#?}"
        );
    }
}

fn assert_non_empty_string(value: &JsonValue, context: &str, field: &str) {
    let text = value
        .get(field)
        .and_then(JsonValue::as_str)
        .unwrap_or_else(|| panic!("{context}.{field} must be a string: {value:#?}"));
    assert!(
        !text.trim().is_empty(),
        "{context}.{field} must not be empty"
    );
}

fn assert_array_field<'a>(value: &'a JsonValue, context: &str, field: &str) -> &'a Vec<JsonValue> {
    value
        .get(field)
        .and_then(JsonValue::as_array)
        .unwrap_or_else(|| panic!("{context}.{field} must be an array: {value:#?}"))
}

fn assert_readiness_finding_contract(value: &JsonValue, context: &str) {
    assert_object_has_fields(
        value,
        context,
        &["severity", "category", "code", "message", "evidence"],
    );
    assert_non_empty_string(value, context, "severity");
    assert_non_empty_string(value, context, "category");
    assert_non_empty_string(value, context, "code");
    assert_non_empty_string(value, context, "message");
    assert!(
        value.get("evidence").is_some_and(JsonValue::is_object),
        "{context}.evidence must be an object: {value:#?}"
    );
}

fn assert_agent_readiness_top_level_contract(value: &JsonValue, context: &str) {
    assert_object_has_fields(
        value,
        context,
        &[
            "schema_version",
            "generated_at_ms",
            "manifest",
            "config",
            "scoring_contract",
            "overall_score",
            "grade",
            "readiness_band",
            "gate_status",
            "summary",
            "persona_scores",
            "blocking_findings",
            "improvement_findings",
            "entity_findings",
            "indicator_findings",
            "suggested_meta_patches",
            "golden_question_seeds",
            "eval_status",
            "next_actions",
        ],
    );
    assert_eq!(value["schema_version"], json!("agent_readiness.v1"));
    assert!(value["generated_at_ms"].is_number());
    assert!(value["overall_score"].is_number());
    assert_non_empty_string(value, context, "grade");
    assert_non_empty_string(value, context, "readiness_band");
    assert_non_empty_string(value, context, "gate_status");
}

fn assert_agent_readiness_manifest_contract(value: &JsonValue) {
    assert_object_has_fields(
        &value["manifest"],
        "agent_readiness.manifest",
        &[
            "source",
            "hash",
            "version",
            "entity_count",
            "resource_counts",
            "search_ready",
        ],
    );
    assert_non_empty_string(&value["manifest"], "agent_readiness.manifest", "source");
    assert!(
        !value["manifest"]["source"]
            .as_str()
            .unwrap_or_default()
            .contains("token="),
        "manifest source must stay sanitized: {value:#?}"
    );
    assert_object_has_fields(
        &value["manifest"]["search_ready"],
        "agent_readiness.manifest.search_ready",
        &["vector", "sparse", "reranker"],
    );
}

fn assert_agent_readiness_config_contract(value: &JsonValue) {
    assert_object_has_fields(
        &value["config"],
        "agent_readiness.config",
        &[
            "personas",
            "resource_types",
            "metadata_only",
            "read_only",
            "storage_instance_id",
            "thresholds",
        ],
    );
    assert!(
        !assert_array_field(&value["config"], "agent_readiness.config", "personas").is_empty(),
        "agent_readiness.config.personas must not be empty"
    );
    assert_eq!(value["config"]["metadata_only"], json!(true));
}

fn assert_agent_readiness_summary_contract(value: &JsonValue) {
    assert_object_has_fields(
        &value["summary"],
        "agent_readiness.summary",
        &[
            "target_count",
            "scored_count",
            "blocker_count",
            "improvement_count",
            "indicator_count",
            "ambiguous_indicator_count",
            "suggested_meta_patch_count",
            "golden_question_seed_count",
            "score_buckets",
            "grade_buckets",
            "worst_entities_by_persona",
            "category_weak_spots",
            "top_recommendation_fields",
            "drill_down_hints",
            "agent_modelling",
        ],
    );
    assert_object_has_fields(
        &value["summary"]["agent_modelling"],
        "agent_readiness.summary.agent_modelling",
        &[
            "total",
            "blockers",
            "high",
            "medium",
            "low",
            "truncated",
            "top_codes",
            "top_categories",
        ],
    );
}

fn assert_agent_readiness_persona_scores_contract(value: &JsonValue) {
    let persona_scores = value["persona_scores"]
        .as_object()
        .expect("persona_scores must be an object");
    assert!(
        !persona_scores.is_empty(),
        "persona_scores must not be empty"
    );
    for (persona, score) in persona_scores {
        assert_object_has_fields(
            score,
            &format!("agent_readiness.persona_scores.{persona}"),
            &[
                "overall_score",
                "grade",
                "gate_status",
                "scored_count",
                "total_available",
                "quality_summary",
                "metadata_summary",
            ],
        );
    }
}

fn assert_agent_readiness_findings_contract(value: &JsonValue, context: &str) {
    for (field, item_context) in [
        ("blocking_findings", "agent_readiness.blocking_findings[]"),
        (
            "improvement_findings",
            "agent_readiness.improvement_findings[]",
        ),
    ] {
        for finding in assert_array_field(value, context, field) {
            assert_readiness_finding_contract(finding, item_context);
        }
    }

    if let Some(entity_finding) = assert_array_field(value, context, "entity_findings").first() {
        assert_object_has_fields(
            entity_finding,
            "agent_readiness.entity_findings[]",
            &[
                "unique_id",
                "overall_score",
                "grade",
                "persona_scores",
                "signals",
                "diagnostics",
                "recommendations",
            ],
        );
        assert_object_has_fields(
            &entity_finding["signals"],
            "agent_readiness.entity_findings[].signals",
            &[
                "has_description",
                "has_owner",
                "has_nova_meta",
                "has_primary_key",
                "has_tests",
                "has_compiled_sql",
                "column_count",
                "documented_column_count",
                "test_count",
                "upstream_count",
                "downstream_count",
            ],
        );
    }

    if let Some(indicator_finding) =
        assert_array_field(value, context, "indicator_findings").first()
    {
        assert_object_has_fields(
            indicator_finding,
            "agent_readiness.indicator_findings[]",
            &["unique_id", "indicator_type", "issue"],
        );
    }
}

fn assert_agent_readiness_patch_seed_contract(value: &JsonValue, context: &str) {
    for patch in assert_array_field(value, context, "suggested_meta_patches") {
        assert_object_has_fields(
            patch,
            "agent_readiness.suggested_meta_patches[]",
            &[
                "id",
                "target_type",
                "unique_id",
                "field_path",
                "suggested_value",
                "placeholder",
                "rationale",
                "severity",
                "confidence",
                "evidence",
            ],
        );
        assert!(
            patch["evidence"].is_object(),
            "suggested_meta_patches[].evidence must be an object: {patch:#?}"
        );
    }

    for seed in assert_array_field(value, context, "golden_question_seeds") {
        assert_object_has_fields(
            seed,
            "agent_readiness.golden_question_seeds[]",
            &[
                "id",
                "seed_type",
                "priority",
                "persona",
                "question",
                "expected_entities",
                "expected_indicators",
                "recommended_assertions",
                "rationale",
                "review_required",
                "date_policy",
            ],
        );
    }
}

fn assert_agent_readiness_eval_actions_contract(value: &JsonValue, context: &str) {
    assert_object_has_fields(
        &value["eval_status"],
        "agent_readiness.eval_status",
        &[
            "status",
            "supplied",
            "failed_eval_ids",
            "failed_case_ids",
            "message",
        ],
    );

    for action in assert_array_field(value, context, "next_actions") {
        assert_object_has_fields(
            action,
            "agent_readiness.next_actions[]",
            &["priority", "category", "action", "evidence"],
        );
        assert_non_empty_string(action, "agent_readiness.next_actions[]", "category");
        assert_non_empty_string(action, "agent_readiness.next_actions[]", "action");
    }
}

fn assert_agent_readiness_report_contract(report: &super::AgentReadinessReport, context: &str) {
    let value = serde_json::to_value(report).expect("serialize readiness report");
    assert_agent_readiness_top_level_contract(&value, context);
    assert_agent_readiness_manifest_contract(&value);
    assert_agent_readiness_config_contract(&value);
    assert_agent_readiness_summary_contract(&value);
    assert_agent_readiness_persona_scores_contract(&value);
    assert_agent_readiness_findings_contract(&value, context);
    assert_agent_readiness_patch_seed_contract(&value, context);
    assert_agent_readiness_eval_actions_contract(&value, context);
}

#[test]
fn parse_threshold_defaults_are_advisory() {
    let thresholds: ReadinessThresholdConfig =
        parse_json_input(None, None, super::DEFAULT_THRESHOLDS_JSON).expect("thresholds");
    let overall = thresholds.overall.expect("overall threshold");
    assert_eq!(overall.min_score, Some(70));
    assert_eq!(overall.severity, ThresholdSeverity::Advisory);
    let engineer = thresholds.persona.engineer.expect("engineer threshold");
    assert_eq!(engineer.min_score, Some(70));
    assert_eq!(engineer.severity, ThresholdSeverity::Advisory);
}

#[test]
fn parse_thresholds_accepts_modelling_count_rules() {
    let thresholds: ReadinessThresholdConfig = parse_json_input(
        Some(
            r#"{
                  "modelling": {
                    "max_blockers": {"value": 0, "severity": "required"},
                    "max_high": {"value": 3, "severity": "advisory"}
                  }
                }"#,
        ),
        None,
        super::DEFAULT_THRESHOLDS_JSON,
    )
    .expect("thresholds");

    let max_blockers = thresholds
        .modelling
        .max_blockers
        .expect("max blockers threshold");
    assert_eq!(max_blockers.value, 0);
    assert_eq!(max_blockers.severity, ThresholdSeverity::Required);
    let max_high = thresholds.modelling.max_high.expect("max high threshold");
    assert_eq!(max_high.value, 3);
    assert_eq!(max_high.severity, ThresholdSeverity::Advisory);
}

#[test]
fn parse_readiness_inputs_preserves_persona_order() {
    let args = AgentReadinessArgs {
        personas_json: Some(r#"["engineer","analyst","engineer","governance"]"#.to_string()),
        ..AgentReadinessArgs::default()
    };
    let inputs = super::parse_readiness_inputs(&args).expect("inputs");
    assert_eq!(
        inputs.personas,
        vec![
            "engineer".to_string(),
            "analyst".to_string(),
            "governance".to_string()
        ]
    );
}

#[test]
fn evaluate_threshold_supports_required_and_advisory() {
    let required = ThresholdRule {
        min_score: Some(80),
        min_grade: Some("B".to_string()),
        severity: ThresholdSeverity::Required,
    };
    let advisory = ThresholdRule {
        min_score: Some(80),
        min_grade: None,
        severity: ThresholdSeverity::Advisory,
    };
    assert_eq!(
        evaluate_threshold(72, "C", Some(&required)),
        "required_fail"
    );
    assert_eq!(
        evaluate_threshold(72, "C", Some(&advisory)),
        "advisory_fail"
    );
    assert_eq!(evaluate_threshold(88, "B", Some(&required)), "pass");
}

#[test]
fn parse_eval_status_accepts_raw_gate_report() {
    let status = parse_eval_status(
            Some(
                r#"{"suite_name":"analyst","allowed":false,"blocked":true,"gate_configured":true,"threshold":0.9,"pass_rate":0.5,"total_evals":4,"failed_evals":2,"failed_eval_ids":["a"],"failed_case_ids":["case"],"message":"below threshold"}"#,
            ),
            None,
        )
        .expect("status");
    assert_eq!(status.status, "blocked");
    assert_eq!(status.suite_name.as_deref(), Some("analyst"));
    assert_eq!(status.failed_eval_ids, vec!["a"]);
    assert_eq!(status.failed_case_ids, vec!["case"]);
}

#[test]
fn parse_eval_status_accepts_cli_envelope() {
    let status = parse_eval_status(
            Some(
                r#"{"command":"eval gate","status":"success","data":{"suite_name":"analyst","allowed":true,"blocked":false,"gate_configured":false,"pass_rate":1.0,"total_evals":3,"failed_evals":0,"message":"allowed"}}"#,
            ),
            None,
        )
        .expect("status");
    assert_eq!(status.status, "allowed");
    assert_eq!(status.allowed, Some(true));
    assert_eq!(status.total_evals, Some(3));
}

#[test]
fn build_agent_readiness_config_disables_semantic_search() {
    let args = AgentReadinessArgs {
        manifest_path: Some(fixture_manifest_path_string()),
        ..AgentReadinessArgs::default()
    };
    let config = build_agent_readiness_load_config(&args).expect("config");
    assert!(!config.search.enable_vector_search);
    assert!(!config.search.enable_sparse_search);
    assert!(!config.search.enable_reranker);
}

#[tokio::test]
async fn agent_readiness_report_contains_required_sections() {
    let temp_dir = TempDir::new().expect("temp dir");
    let args = AgentReadinessArgs {
        manifest_path: Some(fixture_manifest_path_string()),
        storage_instance_id: Some("agent-readiness-report-test".to_string()),
        cleanup_storage_on_start: true,
        ..AgentReadinessArgs::default()
    };
    let inputs = super::parse_readiness_inputs(&args).expect("inputs");
    let mut config = build_agent_readiness_load_config(&args).expect("config");
    config.storage_dir = temp_dir.path().to_string_lossy().to_string();
    let loaded = execute_manifest_load(config).await.expect("load");
    let report = build_agent_readiness_report(&loaded.search, &inputs)
        .await
        .expect("report");

    assert_eq!(report.schema_version, "agent_readiness.v1");
    assert!(report.config.metadata_only);
    assert_eq!(report.eval_status.status, "not_supplied");
    assert!(report.summary.target_count > 0);
    assert!(report.persona_scores.contains_key("engineer"));
    assert!(report.config.resource_types.contains(&"model".to_string()));
    assert!(!report.manifest.source.contains("token="));
    assert!(report.suggested_meta_patches.len() <= super::MAX_SUGGESTED_META_PATCHES);
    assert!(report.golden_question_seeds.len() <= super::MAX_GOLDEN_QUESTION_SEEDS);
    let markdown = render_markdown_report(&report);
    assert!(markdown.contains("# Nova Agent Readiness"));
    assert!(markdown.contains("## Persona Scores"));
    assert!(markdown.contains("## Next Actions"));
}

#[tokio::test]
async fn agent_readiness_v1_contract_fixtures_lock_required_json_fields() {
    let clean_report =
        readiness_report_for_fixture("perfect_model.json", "agent-readiness-contract-clean").await;
    assert_agent_readiness_report_contract(&clean_report, "clean agent_readiness.v1 fixture");

    let problematic_report = readiness_report_for_fixture(
        "agent_modelling_findings.json",
        "agent-readiness-contract-problematic",
    )
    .await;
    assert_agent_readiness_report_contract(
        &problematic_report,
        "problematic agent_readiness.v1 fixture",
    );
    assert!(
        problematic_report.summary.agent_modelling["blockers"]
            .as_u64()
            .is_some_and(|count| count > 0),
        "problematic fixture must keep blocker modelling evidence"
    );
    assert!(
        problematic_report
            .next_actions
            .iter()
            .any(|action| action.category == "modelling"),
        "problematic fixture must keep modelling next actions"
    );
}

#[tokio::test]
async fn agent_readiness_includes_agent_modelling_summary_and_findings() {
    let report =
        readiness_report_for_fixture("agent_modelling_findings.json", "agent-readiness-modeling")
            .await;

    let agent_modelling = &report.summary.agent_modelling;
    assert!(
        agent_modelling["blockers"]
            .as_u64()
            .is_some_and(|count| count > 0)
    );
    assert!(
        agent_modelling["high"]
            .as_u64()
            .is_some_and(|count| count > 0)
    );
    assert!(
        report.blocking_findings.iter().any(|finding| {
            finding.category == "modelling" && finding.code == "agent_modelling_blocker"
        }),
        "expected modelling blocker in {:#?}",
        report.blocking_findings
    );
    assert!(
        report.improvement_findings.iter().any(|finding| {
            finding.category == "modelling"
                && matches!(
                    finding.code,
                    "agent_modelling_high" | "agent_modelling_medium"
                )
        }),
        "expected modelling improvement in {:#?}",
        report.improvement_findings
    );
    assert!(
        report
            .next_actions
            .iter()
            .any(|action| action.category == "modelling"),
        "expected modelling next action in {:#?}",
        report.next_actions
    );
    assert_eq!(report.readiness_band, "blocked");
    assert_eq!(report.gate_status, "fail");
}

#[tokio::test]
async fn agent_readiness_modelling_high_threshold_can_be_advisory() {
    let temp_dir = TempDir::new().expect("temp dir");
    let args = AgentReadinessArgs {
        manifest_path: Some(fixture_path_string("agent_modelling_findings.json")),
        storage_instance_id: Some("agent-readiness-modeling-advisory".to_string()),
        cleanup_storage_on_start: true,
        thresholds_json: Some(
            r#"{
                  "modelling": {
                    "max_blockers": {"value": 999, "severity": "advisory"},
                    "max_high": {"value": 0, "severity": "advisory"}
                  }
                }"#
            .to_string(),
        ),
        ..AgentReadinessArgs::default()
    };
    let inputs = super::parse_readiness_inputs(&args).expect("inputs");
    let mut config = build_agent_readiness_load_config(&args).expect("config");
    config.storage_dir = temp_dir.path().to_string_lossy().to_string();
    let loaded = execute_manifest_load(config).await.expect("load");
    let report = build_agent_readiness_report(&loaded.search, &inputs)
        .await
        .expect("report");

    let threshold_finding = report
        .improvement_findings
        .iter()
        .find(|finding| finding.code == "agent_modelling_high_threshold_missed")
        .expect("advisory high threshold finding");
    assert_eq!(threshold_finding.severity, "improvement");
    assert_eq!(
        threshold_finding.evidence["threshold"]["severity"],
        json!("advisory")
    );
}

#[tokio::test]
async fn agent_readiness_tool_response_uses_report_contract() {
    let temp_dir = TempDir::new().expect("temp dir");
    let args = AgentReadinessArgs {
        manifest_path: Some(fixture_manifest_path_string()),
        storage_instance_id: Some("agent-readiness-tool-response-test".to_string()),
        cleanup_storage_on_start: true,
        ..AgentReadinessArgs::default()
    };
    let mut config = build_agent_readiness_load_config(&args).expect("config");
    config.storage_dir = temp_dir.path().to_string_lossy().to_string();
    let loaded = execute_manifest_load(config).await.expect("load");

    let response = build_agent_readiness_tool_response(
        &loaded.search,
        &GetAgentReadinessParams {
            personas_json: Some(r#"["engineer"]"#.to_string()),
            eval_gate_json: Some(
                r#"{"allowed":true,"blocked":false,"message":"gate passed"}"#.to_string(),
            ),
            ..GetAgentReadinessParams::default()
        },
    )
    .await
    .expect("tool response");

    assert_eq!(response["success"], json!(true));
    assert_eq!(response["count"], json!(1));
    assert_eq!(
        response["data"]["schema_version"],
        json!("agent_readiness.v1")
    );
    assert_eq!(response["data"]["config"]["personas"], json!(["engineer"]));
    assert_eq!(response["data"]["eval_status"]["status"], json!("allowed"));
    assert!(response["data"]["entity_findings"].is_array());
    assert!(response["data"]["next_actions"].is_array());
    assert!(response["data"]["suggested_meta_patches"].is_array());
    assert!(response["data"]["golden_question_seeds"].is_array());
}

#[tokio::test]
async fn agent_readiness_read_only_reuses_existing_storage() {
    let temp_dir = TempDir::new().expect("temp dir");
    let first_args = AgentReadinessArgs {
        manifest_path: Some(fixture_path_string("minimal.json")),
        storage_instance_id: Some("agent-readiness-read-only-test".to_string()),
        cleanup_storage_on_start: true,
        ..AgentReadinessArgs::default()
    };
    let mut first_config = build_agent_readiness_load_config(&first_args).expect("config");
    first_config.storage_dir = temp_dir.path().to_string_lossy().to_string();
    let first = execute_manifest_load(first_config)
        .await
        .expect("first load");
    drop(first);

    let read_only_args = AgentReadinessArgs {
        manifest_path: Some(fixture_path_string("minimal.json")),
        storage_instance_id: Some("agent-readiness-read-only-test".to_string()),
        read_only: true,
        ..AgentReadinessArgs::default()
    };
    let inputs = super::parse_readiness_inputs(&read_only_args).expect("inputs");
    let mut read_only_config =
        build_agent_readiness_load_config(&read_only_args).expect("read-only config");
    read_only_config.storage_dir = temp_dir.path().to_string_lossy().to_string();
    let loaded = execute_manifest_load(read_only_config)
        .await
        .expect("read-only load");
    let report = build_agent_readiness_report(&loaded.search, &inputs)
        .await
        .expect("report");
    assert!(report.config.read_only);
}

#[test]
fn report_writer_creates_parent_directories() {
    let temp_dir = TempDir::new().expect("temp dir");
    let path = temp_dir.path().join("nested").join("readiness.json");
    write_report(path.to_str().expect("utf8 path"), "{}").expect("write report");
    assert_eq!(std::fs::read_to_string(path).expect("read report"), "{}");
}

fn markdown_fixture_report() -> super::AgentReadinessReport {
    super::AgentReadinessReport {
        schema_version: "agent_readiness.v1",
        generated_at_ms: 1,
        manifest: markdown_fixture_manifest(),
        config: markdown_fixture_config(),
        scoring_contract: json!({}),
        overall_score: 60,
        grade: "D".to_string(),
        readiness_band: "blocked",
        gate_status: "fail",
        summary: super::ReadinessSummary {
            target_count: 1,
            scored_count: 1,
            blocker_count: 1,
            improvement_count: 0,
            indicator_count: 0,
            ambiguous_indicator_count: 0,
            suggested_meta_patch_count: 1,
            golden_question_seed_count: 1,
            score_buckets: json!({}),
            grade_buckets: json!({}),
            worst_entities_by_persona: json!([]),
            category_weak_spots: json!([]),
            top_recommendation_fields: json!([]),
            drill_down_hints: json!([]),
            agent_modelling: json!({}),
        },
        persona_scores: markdown_fixture_persona_scores(),
        blocking_findings: vec![ReadinessFinding {
            severity: "blocker",
            category: "eval_gate",
            code: "eval_gate_blocked",
            message: "blocked".to_string(),
            evidence: json!({}),
        }],
        improvement_findings: Vec::new(),
        entity_findings: Vec::new(),
        indicator_findings: Vec::new(),
        suggested_meta_patches: vec![markdown_fixture_meta_patch()],
        golden_question_seeds: vec![markdown_fixture_golden_seed()],
        eval_status: markdown_fixture_eval_status(),
        next_actions: vec![super::ReadinessNextAction {
            priority: 1,
            category: "eval_gate",
            action: "Fix eval gate".to_string(),
            evidence: "blocked".to_string(),
        }],
    }
}

fn markdown_fixture_manifest() -> super::ReadinessManifestSummary {
    super::ReadinessManifestSummary {
        source: "target/manifest.json".to_string(),
        hash: "abc".to_string(),
        version: "abc".to_string(),
        entity_count: 1,
        resource_counts: BTreeMap::new(),
        search_ready: super::ReadinessSearchReady {
            vector: false,
            sparse: false,
            reranker: false,
        },
    }
}

fn markdown_fixture_config() -> super::ReadinessConfigSummary {
    super::ReadinessConfigSummary {
        personas: vec!["engineer".to_string()],
        resource_types: vec!["model".to_string()],
        metadata_only: true,
        read_only: false,
        storage_instance_id: "test".to_string(),
        thresholds: ReadinessThresholdConfig::default(),
    }
}

fn markdown_fixture_persona_scores() -> BTreeMap<String, super::PersonaReadinessScore> {
    BTreeMap::from([(
        "engineer".to_string(),
        super::PersonaReadinessScore {
            overall_score: 60,
            grade: "D".to_string(),
            gate_status: "fail",
            threshold: None,
            scored_count: 1,
            total_available: 1,
            quality_summary: json!({}),
            metadata_summary: json!({}),
        },
    )])
}

fn markdown_fixture_meta_patch() -> super::SuggestedMetaPatch {
    super::SuggestedMetaPatch {
        id: "meta_patch::model_pkg_orders::meta_owner".to_string(),
        target_type: "entity",
        unique_id: "model.pkg.orders".to_string(),
        entity_name: Some("orders".to_string()),
        resource_type: Some("model".to_string()),
        original_file_path: Some("models/orders.yml".to_string()),
        column_name: None,
        indicator_name: None,
        indicator_type: None,
        field_path: "meta.owner".to_string(),
        suggested_value: json!("__OWNER_OR_TEAM__"),
        placeholder: true,
        rationale: "Add an owner.".to_string(),
        severity: "improvement",
        confidence: 0.95,
        evidence: json!({"signal": "missing_owner"}),
    }
}

fn markdown_fixture_golden_seed() -> super::GoldenQuestionSeed {
    super::GoldenQuestionSeed {
        id: "golden_seed::manual_review::model_pkg_orders".to_string(),
        seed_type: "manual_review",
        priority: 1,
        persona: "governance",
        question: "Review governance classification for model.pkg.orders.".to_string(),
        expected_entities: vec!["model.pkg.orders".to_string()],
        expected_indicators: Vec::new(),
        recommended_assertions: vec![json!({"type": "metadata_score_min"})],
        rationale: "Governance coverage is missing.".to_string(),
        review_required: true,
        date_policy: "not_date_sensitive",
    }
}

fn markdown_fixture_eval_status() -> EvalReadinessStatus {
    EvalReadinessStatus {
        status: "blocked",
        supplied: true,
        allowed: Some(false),
        blocked: Some(true),
        gate_configured: Some(true),
        threshold: Some(0.9),
        pass_rate: Some(0.5),
        total_evals: Some(2),
        failed_evals: Some(1),
        failed_eval_ids: vec!["case::assertion".to_string()],
        failed_case_ids: vec!["case".to_string()],
        telemetry_timestamp: None,
        suite_name: Some("suite".to_string()),
        message: "blocked".to_string(),
    }
}

#[test]
fn markdown_includes_blockers_and_eval_status() {
    let report = markdown_fixture_report();
    let markdown = render_markdown_report(&report);
    assert!(markdown.contains("## Blockers"));
    assert!(markdown.contains("status: `blocked`"));
    assert!(markdown.contains("## Suggested Meta Patches"));
    assert!(markdown.contains("## Golden Question Seeds"));
    assert!(markdown.contains("Fix eval gate"));
}

#[tokio::test]
async fn readiness_suggests_entity_column_and_governance_patches() {
    let report =
        readiness_report_for_fixture("minimal.json", "agent-readiness-suggestions-minimal").await;

    assert!(report.suggested_meta_patches.iter().any(|patch| {
        patch.target_type == "entity"
            && patch.field_path == "meta.owner"
            && patch.suggested_value == json!("__OWNER_OR_TEAM__")
            && patch.placeholder
    }));
    assert!(report.suggested_meta_patches.iter().any(|patch| {
        patch.target_type == "entity"
            && patch.field_path == "meta.nova.governance.sensitivity"
            && patch.suggested_value == json!("__SENSITIVITY__")
            && patch.placeholder
    }));
    assert!(report.suggested_meta_patches.iter().any(|patch| {
        patch.target_type == "column"
            && patch.column_name.as_deref() == Some("col")
            && patch.field_path == "columns.col.meta.nova.semantic_type"
            && patch.placeholder
    }));

    let markdown = render_markdown_report(&report);
    assert!(markdown.contains("## Suggested Meta Patches"));
    assert!(markdown.contains("meta.nova.governance.sensitivity"));
}

#[tokio::test]
async fn readiness_uses_score_diagnostics_for_invalid_grain_patch() {
    let report = readiness_report_for_fixture(
        "metadata_diagnostics.json",
        "agent-readiness-invalid-grain-diagnostics",
    )
    .await;

    let bad_grain = report
        .entity_findings
        .iter()
        .find(|finding| finding.unique_id == "model.pkg.bad_grain_orders")
        .expect("bad grain finding");
    assert!(bad_grain.diagnostics.as_array().is_some_and(|diagnostics| {
        diagnostics.iter().any(|diagnostic| {
            diagnostic["code"] == json!("invalid_grain_shape")
                && diagnostic["field"] == json!("meta.nova.grain")
        })
    }));
    assert!(report.suggested_meta_patches.iter().any(|patch| {
        patch.unique_id == "model.pkg.bad_grain_orders"
            && patch.field_path == "meta.nova.grain"
            && patch.evidence["diagnostic"] == json!("invalid_grain_shape")
    }));
    assert!(report.scoring_contract["grade_bands"].is_array());
    assert!(report.summary.worst_entities_by_persona.is_array());
    assert!(report.summary.top_recommendation_fields.is_array());
}

#[tokio::test]
async fn readiness_does_not_seed_indicator_patches_for_sources() {
    let report = readiness_report_for_fixture(
        "resource_shape_scoring.json",
        "agent-readiness-source-resource-shape",
    )
    .await;

    assert!(
        report.suggested_meta_patches.iter().all(|patch| {
            patch.unique_id != "source.pkg.raw.orders"
                || !matches!(
                    patch.field_path.as_str(),
                    "meta.nova.measures" | "meta.nova.metrics"
                )
        }),
        "source resources should not receive measure/metric meta patch suggestions"
    );
}

#[tokio::test]
async fn readiness_suggests_metric_patches_without_guessing_ground_truth() {
    let report =
        readiness_report_for_fixture("metric_test.json", "agent-readiness-suggestions-metric")
            .await;

    assert!(report.suggested_meta_patches.iter().any(|patch| {
        patch.target_type == "indicator"
            && patch.indicator_type.as_deref() == Some("metric")
            && patch.indicator_name.as_deref() == Some("a_metric")
            && patch.field_path == "meta.nova.metrics[name=a_metric].expression"
            && patch.suggested_value == json!("__EXPRESSION_OR_FIELD__")
            && patch.placeholder
    }));
    assert!(report.suggested_meta_patches.iter().any(|patch| {
        patch.field_path == "meta.nova.metrics[name=a_metric].canonical"
            && patch.suggested_value == json!("__TRUE_IF_CANONICAL_INDICATOR__")
    }));
    assert!(report.golden_question_seeds.iter().any(|seed| {
        seed.seed_type == "manual_review"
            && seed.expected_indicators == vec!["a_metric".to_string()]
            && seed.review_required
    }));
}

#[tokio::test]
async fn readiness_native_metric_description_patch_uses_top_level_path() {
    let temp_dir = TempDir::new().expect("temp dir");
    let args = AgentReadinessArgs {
        manifest_path: Some(fixture_path_string("minimal.json")),
        storage_instance_id: Some("agent-readiness-native-metric-description".to_string()),
        cleanup_storage_on_start: true,
        ..AgentReadinessArgs::default()
    };
    let mut config = build_agent_readiness_load_config(&args).expect("config");
    config.storage_dir = temp_dir.path().to_string_lossy().to_string();
    let loaded = execute_manifest_load(config).await.expect("load");
    let entity = loaded
        .search
        .get_entity_archived("model.pkg.model_a")
        .expect("entity lookup")
        .expect("model entity");
    let finding = IndicatorReadinessFinding {
        unique_id: "model.pkg.model_a".to_string(),
        name: Some("orders_total".to_string()),
        resource_type: Some("metric".to_string()),
        indicator_name: None,
        indicator_type: "metric".to_string(),
        issue: "metric is missing a description".to_string(),
    };
    let mut patches = Vec::new();
    let mut seen = BTreeSet::new();

    append_indicator_meta_patches(entity, &finding, &mut patches, &mut seen);

    assert!(
        patches
            .iter()
            .any(|patch| patch.field_path == "description")
    );
    assert!(
        !patches
            .iter()
            .any(|patch| patch.field_path == "description.description")
    );
}

#[tokio::test]
async fn readiness_generates_bridge_seeds_for_canonical_metrics() {
    let report =
        readiness_report_for_fixture("tokenomics_manifest.json", "agent-readiness-seeds").await;

    let seed = report
        .golden_question_seeds
        .iter()
        .find(|seed| seed.expected_indicators == vec!["conversion_rate".to_string()])
        .expect("conversion_rate seed");
    assert_eq!(seed.seed_type, "bridge");
    assert_eq!(seed.persona, "analyst");
    assert_eq!(
        seed.expected_entities,
        vec!["model.tokenomics_fixture.base__amplitude_sessions".to_string()]
    );
    assert_eq!(
        seed.recommended_assertions[0]["type"],
        json!("search_indicator_rank")
    );
    assert_eq!(
        seed.recommended_assertions[0]["expected"],
        json!("conversion_rate")
    );

    let markdown = render_markdown_report(&report);
    assert!(markdown.contains("## Golden Question Seeds"));
    assert!(markdown.contains("conversion_rate"));
}

#[tokio::test]
async fn golden_question_seed_builder_allows_empty_seed_sets() {
    let temp_dir = TempDir::new().expect("temp dir");
    let args = AgentReadinessArgs {
        manifest_path: Some(fixture_path_string("minimal.json")),
        storage_instance_id: Some("agent-readiness-empty-seeds".to_string()),
        cleanup_storage_on_start: true,
        ..AgentReadinessArgs::default()
    };
    let mut config = build_agent_readiness_load_config(&args).expect("config");
    config.storage_dir = temp_dir.path().to_string_lossy().to_string();
    let loaded = execute_manifest_load(config).await.expect("load");
    let seeds =
        super::build_golden_question_seeds(&loaded.search, &[], &[], &[]).expect("empty seeds");
    assert!(seeds.is_empty());
}
