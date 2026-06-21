use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::Path;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::cli::args::{AgentReadinessArgs, ManifestLoadArgs};
use crate::cli::manifest::{build_manifest_load_config, execute_manifest_load};
use crate::cli::output::{CliEnvelope, error_envelope};
use crate::error::{DbtNovaError, Result};
use crate::manifest::entity::{ArchivedEntity, column_primary_key_bool, entity_nova_meta_json};
use crate::manifest::search::ManifestSearch;
use crate::params::GetMetadataScoreParams;
use crate::tools::metadata_score::grade_from_score;
use crate::utils::{SearchPersona, sanitize_uri};

use super::{DispatchError, DispatchResult};

const DEFAULT_PERSONAS_JSON: &str = r#"["engineer","analyst","governance"]"#;
const DEFAULT_THRESHOLDS_JSON: &str = r#"{
  "overall": { "min_score": 70, "severity": "advisory" },
  "persona": {
    "engineer": { "min_score": 70, "severity": "advisory" },
    "analyst": { "min_score": 65, "severity": "advisory" },
    "governance": { "min_score": 65, "severity": "advisory" }
  }
}"#;
const DEFAULT_RESOURCE_TYPE_PRIORITY: [&str; 3] = ["model", "source", "metric"];
const MAX_ENTITY_FINDINGS: usize = 20;
const MAX_ENTITY_RECOMMENDATIONS: usize = 5;
const MAX_INDICATOR_FINDINGS: usize = 25;

#[derive(Debug, Clone, Serialize)]
struct AgentReadinessReport {
    schema_version: &'static str,
    generated_at_ms: u128,
    manifest: ReadinessManifestSummary,
    config: ReadinessConfigSummary,
    overall_score: u8,
    grade: String,
    readiness_band: &'static str,
    gate_status: &'static str,
    summary: ReadinessSummary,
    persona_scores: BTreeMap<String, PersonaReadinessScore>,
    blocking_findings: Vec<ReadinessFinding>,
    improvement_findings: Vec<ReadinessFinding>,
    entity_findings: Vec<EntityReadinessFinding>,
    indicator_findings: Vec<IndicatorReadinessFinding>,
    eval_status: EvalReadinessStatus,
    next_actions: Vec<ReadinessNextAction>,
}

#[derive(Debug, Clone, Serialize)]
struct ReadinessManifestSummary {
    source: String,
    hash: String,
    version: String,
    entity_count: usize,
    resource_counts: BTreeMap<String, usize>,
    search_ready: ReadinessSearchReady,
}

#[derive(Debug, Clone, Serialize)]
struct ReadinessSearchReady {
    vector: bool,
    sparse: bool,
    reranker: bool,
}

#[derive(Debug, Clone, Serialize)]
struct ReadinessConfigSummary {
    personas: Vec<String>,
    resource_types: Vec<String>,
    metadata_only: bool,
    read_only: bool,
    storage_instance_id: String,
    thresholds: ReadinessThresholdConfig,
}

#[derive(Debug, Clone, Serialize)]
#[allow(clippy::struct_field_names)]
struct ReadinessSummary {
    target_count: usize,
    scored_count: usize,
    blocker_count: usize,
    improvement_count: usize,
    indicator_count: usize,
    ambiguous_indicator_count: usize,
}

#[derive(Debug, Clone, Serialize)]
struct PersonaReadinessScore {
    overall_score: u8,
    grade: String,
    gate_status: &'static str,
    threshold: Option<AppliedThreshold>,
    scored_count: usize,
    total_available: usize,
    quality_summary: JsonValue,
}

#[derive(Debug, Clone, Serialize)]
struct ReadinessFinding {
    severity: &'static str,
    category: &'static str,
    code: &'static str,
    message: String,
    evidence: JsonValue,
}

#[derive(Debug, Clone, Serialize)]
struct EntityReadinessFinding {
    unique_id: String,
    name: Option<String>,
    resource_type: Option<String>,
    original_file_path: Option<String>,
    overall_score: u8,
    grade: String,
    persona_scores: BTreeMap<String, u8>,
    signals: EntityReadinessSignals,
    recommendations: Vec<ReadinessRecommendation>,
}

#[derive(Debug, Clone, Serialize)]
#[allow(clippy::struct_excessive_bools)]
struct EntityReadinessSignals {
    has_description: bool,
    has_owner: bool,
    has_nova_meta: bool,
    has_primary_key: bool,
    has_tests: bool,
    has_compiled_sql: bool,
    column_count: usize,
    documented_column_count: usize,
    test_count: usize,
    upstream_count: usize,
    downstream_count: usize,
}

#[derive(Debug, Clone, Serialize)]
struct ReadinessRecommendation {
    category: Option<String>,
    priority: Option<String>,
    impact: Option<u8>,
    field: Option<String>,
    message: String,
}

#[derive(Debug, Clone, Serialize)]
struct IndicatorReadinessFinding {
    unique_id: String,
    name: Option<String>,
    resource_type: Option<String>,
    indicator_name: Option<String>,
    indicator_type: String,
    issue: String,
}

#[derive(Debug, Clone, Serialize)]
struct EvalReadinessStatus {
    status: &'static str,
    supplied: bool,
    allowed: Option<bool>,
    blocked: Option<bool>,
    gate_configured: Option<bool>,
    threshold: Option<f64>,
    pass_rate: Option<f64>,
    total_evals: Option<usize>,
    failed_evals: Option<usize>,
    failed_eval_ids: Vec<String>,
    failed_case_ids: Vec<String>,
    telemetry_timestamp: Option<String>,
    suite_name: Option<String>,
    message: String,
}

#[derive(Debug, Clone, Serialize)]
struct ReadinessNextAction {
    priority: u8,
    category: &'static str,
    action: String,
    evidence: String,
}

#[derive(Debug, Clone, Serialize)]
struct AppliedThreshold {
    min_score: Option<u8>,
    min_grade: Option<String>,
    severity: ThresholdSeverity,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
enum ThresholdSeverity {
    Required,
    #[default]
    Advisory,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
struct ThresholdRule {
    min_score: Option<u8>,
    min_grade: Option<String>,
    severity: ThresholdSeverity,
}

impl Default for ThresholdRule {
    fn default() -> Self {
        Self {
            min_score: None,
            min_grade: None,
            severity: ThresholdSeverity::Advisory,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
struct PersonaThresholdConfig {
    default: Option<ThresholdRule>,
    analyst: Option<ThresholdRule>,
    engineer: Option<ThresholdRule>,
    governance: Option<ThresholdRule>,
}

impl PersonaThresholdConfig {
    fn rule_for(&self, persona: &str) -> Option<&ThresholdRule> {
        match persona {
            "analyst" => self.analyst.as_ref().or(self.default.as_ref()),
            "engineer" => self.engineer.as_ref().or(self.default.as_ref()),
            "governance" => self.governance.as_ref().or(self.default.as_ref()),
            _ => self.default.as_ref(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
struct ReadinessThresholdConfig {
    overall: Option<ThresholdRule>,
    persona: PersonaThresholdConfig,
}

#[derive(Debug, Clone)]
struct ReadinessInputs {
    personas: Vec<String>,
    thresholds: ReadinessThresholdConfig,
    eval_status: EvalReadinessStatus,
}

#[derive(Debug, Clone, Default)]
struct EntityScoreAccumulator {
    name: Option<String>,
    resource_type: Option<String>,
    persona_scores: BTreeMap<String, u8>,
}

/// Runs the `audit agent-readiness` CLI command.
///
/// # Errors
/// Returns an error if input validation fails, manifest loading fails, report
/// generation fails, or `--fail-on-blockers` is set and blockers are present.
pub async fn run_agent_readiness_command(args: &AgentReadinessArgs) -> DispatchResult {
    let started = Instant::now();
    let inputs = parse_readiness_inputs(args).map_err(|error| {
        render_or_propagate_error(
            "audit agent-readiness",
            args.json,
            error,
            started.elapsed().as_millis(),
        )
    })?;
    let config = build_agent_readiness_load_config(args).map_err(|error| {
        render_or_propagate_error(
            "audit agent-readiness",
            args.json,
            error,
            started.elapsed().as_millis(),
        )
    })?;
    let load_result = execute_manifest_load(config).await.map_err(|error| {
        render_or_propagate_error(
            "audit agent-readiness",
            args.json,
            error,
            started.elapsed().as_millis(),
        )
    })?;

    let report = build_agent_readiness_report(&load_result.search, &inputs)
        .await
        .map_err(|error| {
            render_or_propagate_error(
                "audit agent-readiness",
                args.json,
                error,
                started.elapsed().as_millis(),
            )
        })?;

    if let Some(path) = args.report_json_path.as_deref() {
        let serialized = serde_json::to_string_pretty(&report).map_err(|error| {
            render_or_propagate_error(
                "audit agent-readiness",
                args.json,
                DbtNovaError::ServerError(error.to_string()),
                started.elapsed().as_millis(),
            )
        })?;
        write_report(path, &serialized).map_err(|error| {
            render_or_propagate_error(
                "audit agent-readiness",
                args.json,
                error,
                started.elapsed().as_millis(),
            )
        })?;
    }
    let markdown = render_markdown_report(&report);
    if let Some(path) = args.report_md_path.as_deref() {
        write_report(path, &markdown).map_err(|error| {
            render_or_propagate_error(
                "audit agent-readiness",
                args.json,
                error,
                started.elapsed().as_millis(),
            )
        })?;
    }

    if args.json {
        let envelope = CliEnvelope::success(
            "audit agent-readiness",
            &report,
            started.elapsed().as_millis(),
        );
        let output = serde_json::to_string_pretty(&envelope).map_err(|error| DispatchError {
            error: DbtNovaError::ServerError(error.to_string()),
            rendered: false,
        })?;
        println!("{output}");
    } else {
        print_human_summary(&report);
        println!();
        println!("{markdown}");
    }

    if args.fail_on_blockers && !report.blocking_findings.is_empty() {
        return Err(DispatchError {
            error: DbtNovaError::ServerError(format!(
                "agent readiness gate found {} blocker(s)",
                report.blocking_findings.len()
            )),
            rendered: true,
        });
    }

    Ok(())
}

fn build_agent_readiness_load_config(
    args: &AgentReadinessArgs,
) -> Result<crate::config::DbtNovaConfig> {
    let load_args = ManifestLoadArgs {
        manifest_path: args.manifest_path.clone(),
        manifest_uri: args.manifest_uri.clone(),
        storage_instance_id: args.storage_instance_id.clone(),
        cleanup_storage_on_start: args.cleanup_storage_on_start,
        read_only: args.read_only,
        json: false,
    };
    let mut config = build_manifest_load_config(&load_args)?;
    config.search.enable_vector_search = false;
    config.search.enable_sparse_search = false;
    config.search.enable_reranker = false;
    Ok(config)
}

fn parse_readiness_inputs(args: &AgentReadinessArgs) -> Result<ReadinessInputs> {
    let mut seen_personas = BTreeSet::new();
    let personas: Vec<String> =
        parse_json_array_input(args.personas_json.as_deref(), None, DEFAULT_PERSONAS_JSON)?
            .into_iter()
            .map(|value| value.trim().to_ascii_lowercase())
            .filter(|value| !value.is_empty())
            .filter(|value| seen_personas.insert(value.clone()))
            .collect();
    if personas.is_empty() {
        return Err(DbtNovaError::InvalidParams(
            "personas_json must contain at least one persona".to_string(),
        ));
    }
    for persona in &personas {
        if SearchPersona::parse(persona) == SearchPersona::Default && persona != "default" {
            return Err(DbtNovaError::InvalidParams(format!(
                "unsupported persona '{persona}'; expected analyst, engineer, or governance"
            )));
        }
    }

    let thresholds: ReadinessThresholdConfig = parse_json_input(
        args.thresholds_json.as_deref(),
        args.thresholds_file.as_deref(),
        DEFAULT_THRESHOLDS_JSON,
    )?;
    let eval_status = parse_eval_status(
        args.eval_gate_json.as_deref(),
        args.eval_gate_file.as_deref(),
    )?;

    Ok(ReadinessInputs {
        personas,
        thresholds,
        eval_status,
    })
}

async fn build_agent_readiness_report(
    search: &ManifestSearch,
    inputs: &ReadinessInputs,
) -> Result<AgentReadinessReport> {
    let resource_types = default_resource_types(search);
    let target_ids = selected_entity_ids(search, &resource_types)?;
    let target_count = target_ids.len();
    let (persona_scores, entity_scores) =
        score_project_personas(search, inputs, &resource_types, target_count).await?;
    let overall_score = average_score(persona_scores.values().map(|score| score.overall_score));
    let grade = grade_from_score(overall_score);

    let mut blocking_findings = Vec::new();
    let mut improvement_findings = Vec::new();
    apply_overall_threshold(
        overall_score,
        grade,
        inputs.thresholds.overall.as_ref(),
        &mut blocking_findings,
        &mut improvement_findings,
    );
    for (persona, score) in &persona_scores {
        apply_persona_threshold(
            persona,
            score,
            &mut blocking_findings,
            &mut improvement_findings,
        );
    }
    apply_eval_status_findings(
        &inputs.eval_status,
        &mut blocking_findings,
        &mut improvement_findings,
    );

    let (entity_findings, entity_improvements) =
        build_entity_findings(search, &target_ids, &entity_scores).await?;
    improvement_findings.extend(entity_improvements);
    let (indicator_findings, indicator_count, ambiguous_indicator_count) =
        build_indicator_findings(search, &target_ids)?;
    apply_indicator_summary_findings(
        indicator_count,
        ambiguous_indicator_count,
        &mut improvement_findings,
    );

    let readiness_band = readiness_band(overall_score, !blocking_findings.is_empty());
    let gate_status = gate_status(
        !blocking_findings.is_empty(),
        !improvement_findings.is_empty(),
    );
    let next_actions = build_next_actions(
        overall_score,
        &inputs.eval_status,
        &blocking_findings,
        &improvement_findings,
        ambiguous_indicator_count,
    );

    Ok(AgentReadinessReport {
        schema_version: "agent_readiness.v1",
        generated_at_ms: current_timestamp_ms(),
        manifest: build_manifest_summary(search),
        config: build_config_summary(search, inputs, &resource_types),
        overall_score,
        grade: grade.to_string(),
        readiness_band,
        gate_status,
        summary: build_readiness_summary(
            target_count,
            entity_scores.len(),
            blocking_findings.len(),
            improvement_findings.len(),
            indicator_count,
            ambiguous_indicator_count,
        ),
        persona_scores,
        blocking_findings,
        improvement_findings,
        entity_findings,
        indicator_findings,
        eval_status: inputs.eval_status.clone(),
        next_actions,
    })
}

fn apply_eval_status_findings(
    eval_status: &EvalReadinessStatus,
    blocking_findings: &mut Vec<ReadinessFinding>,
    improvement_findings: &mut Vec<ReadinessFinding>,
) {
    if eval_status.status == "blocked" {
        blocking_findings.push(ReadinessFinding {
            severity: "blocker",
            category: "eval_gate",
            code: "eval_gate_blocked",
            message: eval_status.message.clone(),
            evidence: serde_json::json!({
                "suite_name": eval_status.suite_name,
                "pass_rate": eval_status.pass_rate,
                "threshold": eval_status.threshold,
                "failed_eval_ids": eval_status.failed_eval_ids,
                "failed_case_ids": eval_status.failed_case_ids
            }),
        });
    } else if eval_status.status == "unavailable" {
        improvement_findings.push(ReadinessFinding {
            severity: "improvement",
            category: "eval_gate",
            code: "eval_gate_unavailable",
            message: eval_status.message.clone(),
            evidence: serde_json::json!({
                "supplied": eval_status.supplied,
                "suite_name": eval_status.suite_name
            }),
        });
    }
}

fn apply_indicator_summary_findings(
    indicator_count: usize,
    ambiguous_indicator_count: usize,
    improvement_findings: &mut Vec<ReadinessFinding>,
) {
    if ambiguous_indicator_count == 0 {
        return;
    }
    improvement_findings.push(ReadinessFinding {
        severity: "improvement",
        category: "indicator_metadata",
        code: "ambiguous_indicators",
        message: format!(
            "{ambiguous_indicator_count} indicator definition(s) need stronger execution metadata"
        ),
        evidence: serde_json::json!({
            "indicator_count": indicator_count,
            "ambiguous_indicator_count": ambiguous_indicator_count
        }),
    });
}

fn build_manifest_summary(search: &ManifestSearch) -> ReadinessManifestSummary {
    let mut resource_counts = BTreeMap::new();
    for (resource_type, count) in &search.entity_counts {
        resource_counts.insert(resource_type.clone(), *count);
    }

    ReadinessManifestSummary {
        source: sanitize_uri(&search.manifest_source_uri),
        hash: search.manifest_hash.clone(),
        version: search.manifest_version.clone(),
        entity_count: search.entity_count(),
        resource_counts,
        search_ready: ReadinessSearchReady {
            vector: search.vector_search_ready(),
            sparse: search.sparse_search_ready(),
            reranker: search.reranker_ready(),
        },
    }
}

fn build_config_summary(
    search: &ManifestSearch,
    inputs: &ReadinessInputs,
    resource_types: &[String],
) -> ReadinessConfigSummary {
    ReadinessConfigSummary {
        personas: inputs.personas.clone(),
        resource_types: resource_types.to_vec(),
        metadata_only: true,
        read_only: search.config().storage_read_only,
        storage_instance_id: search.config().storage_instance_id.clone(),
        thresholds: inputs.thresholds.clone(),
    }
}

fn build_readiness_summary(
    target_count: usize,
    scored_count: usize,
    blocker_count: usize,
    improvement_count: usize,
    indicator_count: usize,
    ambiguous_indicator_count: usize,
) -> ReadinessSummary {
    ReadinessSummary {
        target_count,
        scored_count,
        blocker_count,
        improvement_count,
        indicator_count,
        ambiguous_indicator_count,
    }
}

async fn score_project_personas(
    search: &ManifestSearch,
    inputs: &ReadinessInputs,
    resource_types: &[String],
    target_count: usize,
) -> Result<(
    BTreeMap<String, PersonaReadinessScore>,
    BTreeMap<String, EntityScoreAccumulator>,
)> {
    let mut persona_scores = BTreeMap::new();
    let mut entity_scores: BTreeMap<String, EntityScoreAccumulator> = BTreeMap::new();
    for persona in &inputs.personas {
        let params = GetMetadataScoreParams {
            persona: Some(persona.clone()),
            scope: Some("project".to_string()),
            include_breakdown: false,
            include_recommendations: false,
            resource_types: resource_types.to_vec(),
            limit: Some(target_count),
            offset: Some(0),
            ..GetMetadataScoreParams::default()
        };
        let result = search.get_metadata_score(&params).await?;
        let data = result.get("data").cloned().ok_or_else(|| {
            DbtNovaError::ServerError("metadata score response missing data".to_string())
        })?;
        let overall_score = json_u8(&data, "overall_score")?;
        let grade = json_string(&data, "grade")?;
        let threshold = inputs.thresholds.persona.rule_for(persona).cloned();
        let threshold_gate = evaluate_threshold(overall_score, &grade, threshold.as_ref());
        let scored_count = result
            .get("count")
            .and_then(JsonValue::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .unwrap_or(0);
        let total_available = result
            .get("total_available")
            .and_then(JsonValue::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .unwrap_or(scored_count);
        ingest_project_entity_scores(persona, &data, &mut entity_scores)?;
        persona_scores.insert(
            persona.clone(),
            PersonaReadinessScore {
                overall_score,
                grade,
                gate_status: threshold_gate_to_report_gate(threshold_gate),
                threshold: threshold.as_ref().map(applied_threshold),
                scored_count,
                total_available,
                quality_summary: data
                    .get("quality_summary")
                    .cloned()
                    .unwrap_or(JsonValue::Null),
            },
        );
    }

    Ok((persona_scores, entity_scores))
}

fn ingest_project_entity_scores(
    persona: &str,
    data: &JsonValue,
    entity_scores: &mut BTreeMap<String, EntityScoreAccumulator>,
) -> Result<()> {
    let Some(entities) = data.get("entities").and_then(JsonValue::as_array) else {
        return Ok(());
    };
    for entity in entities {
        let unique_id = entity
            .get("unique_id")
            .and_then(JsonValue::as_str)
            .ok_or_else(|| {
                DbtNovaError::ServerError(
                    "metadata score project response entity missing unique_id".to_string(),
                )
            })?;
        let score = json_u8(entity, "overall_score")?;
        let entry = entity_scores.entry(unique_id.to_string()).or_default();
        entry.name = entity
            .get("name")
            .and_then(JsonValue::as_str)
            .map(ToString::to_string);
        entry.resource_type = entity
            .get("resource_type")
            .and_then(JsonValue::as_str)
            .map(ToString::to_string);
        entry.persona_scores.insert(persona.to_string(), score);
    }
    Ok(())
}

async fn build_entity_findings(
    search: &ManifestSearch,
    target_ids: &[String],
    entity_scores: &BTreeMap<String, EntityScoreAccumulator>,
) -> Result<(Vec<EntityReadinessFinding>, Vec<ReadinessFinding>)> {
    let mut candidates = Vec::new();
    for unique_id in target_ids {
        let Some(score) = entity_scores.get(unique_id) else {
            continue;
        };
        let Some(entity) = search.get_entity_archived(unique_id)? else {
            continue;
        };
        let signals = entity_signals(search, entity, unique_id);
        let average = average_score(score.persona_scores.values().copied());
        let signal_gap_count = signal_gap_count(&signals);
        if average < 70 || signal_gap_count > 0 {
            candidates.push((average, signal_gap_count, unique_id.clone(), signals));
        }
    }
    candidates.sort_by(|left, right| {
        left.0
            .cmp(&right.0)
            .then_with(|| right.1.cmp(&left.1))
            .then_with(|| left.2.cmp(&right.2))
    });

    let mut entity_findings = Vec::new();
    let mut improvements = Vec::new();
    for (average, gap_count, unique_id, signals) in candidates.into_iter().take(MAX_ENTITY_FINDINGS)
    {
        let Some(entity) = search.get_entity_archived(&unique_id)? else {
            continue;
        };
        let score = entity_scores.get(&unique_id).ok_or_else(|| {
            DbtNovaError::ServerError(format!("missing entity score accumulator for {unique_id}"))
        })?;
        let recommendations = entity_recommendations(search, &unique_id, score).await?;
        let grade = grade_from_score(average);
        let message = if average < 70 {
            format!("{unique_id} scored {average} ({grade}) across readiness personas")
        } else {
            format!("{unique_id} has {gap_count} agent-readiness signal gap(s)")
        };
        improvements.push(ReadinessFinding {
            severity: "improvement",
            category: "entity_metadata",
            code: "entity_readiness_gap",
            message,
            evidence: serde_json::json!({
                "unique_id": unique_id,
                "overall_score": average,
                "grade": grade,
                "signal_gap_count": gap_count
            }),
        });
        entity_findings.push(EntityReadinessFinding {
            unique_id: unique_id.clone(),
            name: entity.name_str().map(ToString::to_string),
            resource_type: entity.resource_type_str().map(ToString::to_string),
            original_file_path: entity.original_file_path_str().map(ToString::to_string),
            overall_score: average,
            grade: grade.to_string(),
            persona_scores: score.persona_scores.clone(),
            signals,
            recommendations,
        });
    }

    Ok((entity_findings, improvements))
}

async fn entity_recommendations(
    search: &ManifestSearch,
    unique_id: &str,
    score: &EntityScoreAccumulator,
) -> Result<Vec<ReadinessRecommendation>> {
    let persona = score
        .persona_scores
        .iter()
        .min_by_key(|(_, score)| **score)
        .map_or("engineer", |(persona, _)| persona.as_str());
    let params = GetMetadataScoreParams {
        id_or_name: Some(unique_id.to_string()),
        resource_type: score.resource_type.clone(),
        persona: Some(persona.to_string()),
        scope: Some("entity".to_string()),
        include_breakdown: false,
        include_recommendations: true,
        ..GetMetadataScoreParams::default()
    };
    let result = search.get_metadata_score(&params).await?;
    let data = result.get("data").ok_or_else(|| {
        DbtNovaError::ServerError("metadata score response missing data".to_string())
    })?;
    let Some(recommendations) = data.get("recommendations").and_then(JsonValue::as_array) else {
        return Ok(Vec::new());
    };
    Ok(recommendations
        .iter()
        .take(MAX_ENTITY_RECOMMENDATIONS)
        .map(readiness_recommendation_from_json)
        .collect())
}

fn entity_signals(
    search: &ManifestSearch,
    entity: &ArchivedEntity,
    unique_id: &str,
) -> EntityReadinessSignals {
    let entity_json = entity.to_json_value();
    let column_count = entity.column_names_iter().count();
    EntityReadinessSignals {
        has_description: entity
            .description_str()
            .is_some_and(|description| !description.trim().is_empty()),
        has_owner: has_owner(&entity_json),
        has_nova_meta: entity_nova_meta_json(&entity_json).is_some(),
        has_primary_key: entity_has_primary_key(&entity_json),
        has_tests: search
            .tests_by_entity
            .get(unique_id)
            .is_some_and(|tests| !tests.is_empty()),
        has_compiled_sql: entity.has_compiled_sql() || entity.resource_type_str() != Some("model"),
        column_count,
        documented_column_count: entity.columns_documented_count(),
        test_count: search
            .tests_by_entity
            .get(unique_id)
            .map_or(0, std::vec::Vec::len),
        upstream_count: search
            .parent_map
            .get(unique_id)
            .map_or(0, std::vec::Vec::len),
        downstream_count: search
            .child_map
            .get(unique_id)
            .map_or(0, std::vec::Vec::len),
    }
}

fn signal_gap_count(signals: &EntityReadinessSignals) -> usize {
    [
        signals.has_description,
        signals.has_owner,
        signals.has_nova_meta,
        signals.has_primary_key || signals.column_count == 0,
        signals.has_tests,
        signals.has_compiled_sql,
        signals.documented_column_count >= signals.column_count || signals.column_count == 0,
    ]
    .into_iter()
    .filter(|present| !present)
    .count()
}

fn entity_has_primary_key(entity_json: &JsonValue) -> bool {
    entity_json
        .get("columns")
        .and_then(JsonValue::as_object)
        .is_some_and(|columns| columns.values().any(column_primary_key_bool))
}

fn has_owner(entity_json: &JsonValue) -> bool {
    if entity_json
        .get("meta")
        .and_then(|meta| meta.get("owner"))
        .is_some_and(non_empty_json)
        || entity_json
            .get("config")
            .and_then(|config| config.get("meta"))
            .and_then(|meta| meta.get("owner"))
            .is_some_and(non_empty_json)
    {
        return true;
    }
    entity_json.get("owner").is_some_and(non_empty_json)
}

fn non_empty_json(value: &JsonValue) -> bool {
    match value {
        JsonValue::String(text) => !text.trim().is_empty(),
        JsonValue::Array(values) => !values.is_empty(),
        JsonValue::Object(values) => !values.is_empty(),
        JsonValue::Bool(value) => *value,
        JsonValue::Null => false,
        JsonValue::Number(_) => true,
    }
}

fn build_indicator_findings(
    search: &ManifestSearch,
    target_ids: &[String],
) -> Result<(Vec<IndicatorReadinessFinding>, usize, usize)> {
    let mut findings = Vec::new();
    let mut indicator_count = 0usize;
    for unique_id in target_ids {
        let Some(entity) = search.get_entity_archived(unique_id)? else {
            continue;
        };
        let entity_json = entity.to_json_value();
        let nova = entity_nova_meta_json(&entity_json);
        let nova = nova.as_deref();
        if entity.resource_type_str() == Some("metric") {
            indicator_count += 1;
            if entity
                .description_str()
                .is_none_or(|description| description.trim().is_empty())
            {
                findings.push(indicator_finding(
                    entity,
                    unique_id,
                    None,
                    "metric",
                    "dbt metric is missing a description",
                ));
            }
        }
        if let Some(nova) = nova {
            indicator_count += inspect_indicator_array(
                entity,
                unique_id,
                nova.get("measures").and_then(JsonValue::as_array),
                "measure",
                &mut findings,
            );
            indicator_count += inspect_indicator_array(
                entity,
                unique_id,
                nova.get("metrics").and_then(JsonValue::as_array),
                "metric",
                &mut findings,
            );
            if let Some(metric) = nova.get("metric").and_then(JsonValue::as_object) {
                indicator_count += 1;
                inspect_indicator_object(entity, unique_id, Some(metric), "metric", &mut findings);
            }
        }
    }
    findings.sort_by(|left, right| {
        left.unique_id
            .cmp(&right.unique_id)
            .then_with(|| left.indicator_type.cmp(&right.indicator_type))
            .then_with(|| left.indicator_name.cmp(&right.indicator_name))
            .then_with(|| left.issue.cmp(&right.issue))
    });
    let ambiguous_indicator_count = findings.len();
    findings.truncate(MAX_INDICATOR_FINDINGS);
    Ok((findings, indicator_count, ambiguous_indicator_count))
}

fn inspect_indicator_array(
    entity: &ArchivedEntity,
    unique_id: &str,
    indicators: Option<&Vec<JsonValue>>,
    indicator_type: &str,
    findings: &mut Vec<IndicatorReadinessFinding>,
) -> usize {
    let Some(indicators) = indicators else {
        return 0;
    };
    for indicator in indicators {
        inspect_indicator_object(
            entity,
            unique_id,
            indicator.as_object(),
            indicator_type,
            findings,
        );
    }
    indicators.len()
}

fn inspect_indicator_object(
    entity: &ArchivedEntity,
    unique_id: &str,
    indicator: Option<&serde_json::Map<String, JsonValue>>,
    indicator_type: &str,
    findings: &mut Vec<IndicatorReadinessFinding>,
) {
    let Some(indicator) = indicator else {
        findings.push(indicator_finding(
            entity,
            unique_id,
            None,
            indicator_type,
            "indicator definition is not an object",
        ));
        return;
    };
    let name = indicator
        .get("name")
        .and_then(JsonValue::as_str)
        .map(ToString::to_string);
    let has_expression = string_field_present(indicator, "expression");
    let has_field = string_field_present(indicator, "field");
    if !has_expression && !has_field {
        findings.push(indicator_finding(
            entity,
            unique_id,
            name.clone(),
            indicator_type,
            "indicator is missing expression or field",
        ));
    }
    if indicator_type == "metric"
        && indicator
            .get("grain")
            .and_then(JsonValue::as_object)
            .and_then(|grain| grain.get("time_field"))
            .and_then(JsonValue::as_str)
            .is_none_or(|field| field.trim().is_empty())
    {
        findings.push(indicator_finding(
            entity,
            unique_id,
            name,
            indicator_type,
            "metric is missing grain.time_field",
        ));
    }
}

fn string_field_present(map: &serde_json::Map<String, JsonValue>, field: &str) -> bool {
    map.get(field)
        .and_then(JsonValue::as_str)
        .is_some_and(|value| !value.trim().is_empty())
}

fn indicator_finding(
    entity: &ArchivedEntity,
    unique_id: &str,
    indicator_name: Option<String>,
    indicator_type: &str,
    issue: &str,
) -> IndicatorReadinessFinding {
    IndicatorReadinessFinding {
        unique_id: unique_id.to_string(),
        name: entity.name_str().map(ToString::to_string),
        resource_type: entity.resource_type_str().map(ToString::to_string),
        indicator_name,
        indicator_type: indicator_type.to_string(),
        issue: issue.to_string(),
    }
}

fn default_resource_types(search: &ManifestSearch) -> Vec<String> {
    let mut resource_types = DEFAULT_RESOURCE_TYPE_PRIORITY
        .iter()
        .filter(|resource_type| search.by_resource_type.contains_key(**resource_type))
        .map(|resource_type| (*resource_type).to_string())
        .collect::<Vec<_>>();
    if resource_types.is_empty() {
        resource_types = search.by_resource_type.keys().cloned().collect();
        resource_types.sort();
    }
    resource_types
}

fn selected_entity_ids(search: &ManifestSearch, resource_types: &[String]) -> Result<Vec<String>> {
    let mut selected = BTreeSet::new();
    for resource_type in resource_types {
        let normalized = search.normalize_resource_type_key(resource_type)?;
        if let Some(ids) = search.by_resource_type.get(&normalized) {
            selected.extend(ids.iter().cloned());
        }
    }
    Ok(selected.into_iter().collect())
}

fn apply_overall_threshold(
    overall_score: u8,
    grade: &str,
    threshold: Option<&ThresholdRule>,
    blocking_findings: &mut Vec<ReadinessFinding>,
    improvement_findings: &mut Vec<ReadinessFinding>,
) {
    let gate = evaluate_threshold(overall_score, grade, threshold);
    if gate == "pass" {
        return;
    }
    let finding = ReadinessFinding {
        severity: threshold_gate_to_finding_severity(gate),
        category: "overall_score",
        code: "overall_threshold_missed",
        message: format!("overall readiness score {overall_score} ({grade}) missed its threshold"),
        evidence: serde_json::json!({
            "overall_score": overall_score,
            "grade": grade,
            "threshold": threshold.map(applied_threshold)
        }),
    };
    push_threshold_finding(gate, finding, blocking_findings, improvement_findings);
}

fn apply_persona_threshold(
    persona: &str,
    score: &PersonaReadinessScore,
    blocking_findings: &mut Vec<ReadinessFinding>,
    improvement_findings: &mut Vec<ReadinessFinding>,
) {
    if !matches!(score.gate_status, "fail" | "advisory") {
        return;
    }
    let gate = if score.gate_status == "fail" {
        "required_fail"
    } else {
        "advisory_fail"
    };
    let finding = ReadinessFinding {
        severity: threshold_gate_to_finding_severity(gate),
        category: "persona_score",
        code: "persona_threshold_missed",
        message: format!(
            "{persona} readiness score {} ({}) missed its threshold",
            score.overall_score, score.grade
        ),
        evidence: serde_json::json!({
            "persona": persona,
            "overall_score": score.overall_score,
            "grade": score.grade,
            "threshold": score.threshold
        }),
    };
    push_threshold_finding(gate, finding, blocking_findings, improvement_findings);
}

fn push_threshold_finding(
    gate: &'static str,
    finding: ReadinessFinding,
    blocking_findings: &mut Vec<ReadinessFinding>,
    improvement_findings: &mut Vec<ReadinessFinding>,
) {
    if gate == "required_fail" {
        blocking_findings.push(finding);
    } else {
        improvement_findings.push(finding);
    }
}

fn parse_eval_status(inline: Option<&str>, path: Option<&str>) -> Result<EvalReadinessStatus> {
    let Some(raw) = raw_optional_input(inline, path)? else {
        return Ok(EvalReadinessStatus {
            status: "not_supplied",
            supplied: false,
            allowed: None,
            blocked: None,
            gate_configured: None,
            threshold: None,
            pass_rate: None,
            total_evals: None,
            failed_evals: None,
            failed_eval_ids: Vec::new(),
            failed_case_ids: Vec::new(),
            telemetry_timestamp: None,
            suite_name: None,
            message: "no eval gate report supplied".to_string(),
        });
    };
    let parsed: JsonValue = serde_json::from_str(&raw).map_err(|error| {
        DbtNovaError::InvalidParams(format!("failed to parse eval gate JSON: {error}"))
    })?;
    let report = parsed
        .get("data")
        .filter(|data| !data.is_null())
        .unwrap_or(&parsed);
    let allowed = report.get("allowed").and_then(JsonValue::as_bool);
    let blocked = report.get("blocked").and_then(JsonValue::as_bool);
    let status = match (allowed, blocked) {
        (Some(true), _) => "allowed",
        (_, Some(true)) | (Some(false), _) => "blocked",
        _ => "unavailable",
    };
    let message = report
        .get("message")
        .and_then(JsonValue::as_str)
        .unwrap_or("eval gate report did not include a message")
        .to_string();

    Ok(EvalReadinessStatus {
        status,
        supplied: true,
        allowed,
        blocked,
        gate_configured: report.get("gate_configured").and_then(JsonValue::as_bool),
        threshold: report.get("threshold").and_then(JsonValue::as_f64),
        pass_rate: report.get("pass_rate").and_then(JsonValue::as_f64),
        total_evals: json_usize_optional(report, "total_evals"),
        failed_evals: json_usize_optional(report, "failed_evals"),
        failed_eval_ids: json_string_array(report, "failed_eval_ids"),
        failed_case_ids: json_string_array(report, "failed_case_ids"),
        telemetry_timestamp: report
            .get("telemetry_timestamp")
            .and_then(JsonValue::as_str)
            .map(ToString::to_string),
        suite_name: report
            .get("suite_name")
            .and_then(JsonValue::as_str)
            .map(ToString::to_string),
        message,
    })
}

fn evaluate_threshold(
    overall_score: u8,
    grade: &str,
    threshold: Option<&ThresholdRule>,
) -> &'static str {
    let Some(threshold) = threshold else {
        return "pass";
    };
    let score_pass = threshold.min_score.is_none_or(|min| overall_score >= min);
    let grade_pass = threshold
        .min_grade
        .as_deref()
        .is_none_or(|min| grade_meets_threshold(grade, min));
    if score_pass && grade_pass {
        return "pass";
    }
    match threshold.severity {
        ThresholdSeverity::Required => "required_fail",
        ThresholdSeverity::Advisory => "advisory_fail",
    }
}

fn grade_meets_threshold(actual: &str, minimum: &str) -> bool {
    grade_rank(actual) >= grade_rank(minimum)
}

fn grade_rank(grade: &str) -> i8 {
    match grade.trim().to_ascii_uppercase().as_str() {
        "A" => 4,
        "B" => 3,
        "C" => 2,
        "D" => 1,
        _ => 0,
    }
}

fn threshold_gate_to_report_gate(gate: &'static str) -> &'static str {
    match gate {
        "required_fail" => "fail",
        "advisory_fail" => "advisory",
        _ => "pass",
    }
}

fn threshold_gate_to_finding_severity(gate: &'static str) -> &'static str {
    if gate == "required_fail" {
        "blocker"
    } else {
        "improvement"
    }
}

fn applied_threshold(rule: &ThresholdRule) -> AppliedThreshold {
    AppliedThreshold {
        min_score: rule.min_score,
        min_grade: rule.min_grade.clone(),
        severity: rule.severity,
    }
}

fn readiness_recommendation_from_json(value: &JsonValue) -> ReadinessRecommendation {
    ReadinessRecommendation {
        category: value
            .get("category")
            .and_then(JsonValue::as_str)
            .map(ToString::to_string),
        priority: value
            .get("priority")
            .and_then(JsonValue::as_str)
            .map(ToString::to_string),
        impact: value
            .get("impact")
            .and_then(JsonValue::as_u64)
            .and_then(|impact| u8::try_from(impact).ok()),
        field: value
            .get("field")
            .and_then(JsonValue::as_str)
            .map(ToString::to_string),
        message: value
            .get("message")
            .and_then(JsonValue::as_str)
            .unwrap_or("Improve metadata quality for agent readiness")
            .to_string(),
    }
}

fn build_next_actions(
    overall_score: u8,
    eval_status: &EvalReadinessStatus,
    blocking_findings: &[ReadinessFinding],
    improvement_findings: &[ReadinessFinding],
    ambiguous_indicator_count: usize,
) -> Vec<ReadinessNextAction> {
    let mut actions = Vec::new();
    if !blocking_findings.is_empty() {
        actions.push(ReadinessNextAction {
            priority: 1,
            category: "blockers",
            action:
                "Resolve blocking readiness findings before treating this manifest as launch-ready"
                    .to_string(),
            evidence: format!("{} blocker(s) detected", blocking_findings.len()),
        });
    }
    if eval_status.status == "not_supplied" {
        actions.push(ReadinessNextAction {
            priority: 2,
            category: "eval_gate",
            action: "Run dbt-nova eval gate <SUITE> --json and pass the result with --eval-gate-file or --eval-gate-json".to_string(),
            evidence: "no eval gate report supplied".to_string(),
        });
    } else if eval_status.status == "blocked" {
        actions.push(ReadinessNextAction {
            priority: 1,
            category: "eval_gate",
            action: "Rerun or fix the blocked eval suite before relying on agent workflows"
                .to_string(),
            evidence: eval_status.message.clone(),
        });
    } else if eval_status.status == "unavailable" {
        actions.push(ReadinessNextAction {
            priority: 2,
            category: "eval_gate",
            action: "Provide a valid dbt-nova eval gate JSON report before treating eval evidence as current".to_string(),
            evidence: eval_status.message.clone(),
        });
    }
    if ambiguous_indicator_count > 0 {
        actions.push(ReadinessNextAction {
            priority: 2,
            category: "indicator_metadata",
            action:
                "Add explicit expression, field, and grain metadata to ambiguous Nova indicators"
                    .to_string(),
            evidence: format!("{ambiguous_indicator_count} ambiguous indicator definition(s)"),
        });
    }
    if overall_score < 85 || !improvement_findings.is_empty() {
        actions.push(ReadinessNextAction {
            priority: 3,
            category: "metadata_quality",
            action:
                "Work through the lowest-scoring entity findings, then rerun the readiness report"
                    .to_string(),
            evidence: format!(
                "overall score {overall_score}; {} improvement finding(s)",
                improvement_findings.len()
            ),
        });
    }
    if actions.is_empty() {
        actions.push(ReadinessNextAction {
            priority: 4,
            category: "ship",
            action: "Use this report as agent-readiness evidence for the current manifest"
                .to_string(),
            evidence: "no blockers or material improvements detected".to_string(),
        });
    }
    actions.sort_by_key(|action| action.priority);
    actions
}

fn readiness_band(score: u8, has_blockers: bool) -> &'static str {
    if has_blockers {
        "blocked"
    } else if score >= 85 {
        "high"
    } else if score >= 70 {
        "medium"
    } else {
        "low"
    }
}

fn gate_status(has_blockers: bool, has_improvements: bool) -> &'static str {
    if has_blockers {
        "fail"
    } else if has_improvements {
        "advisory"
    } else {
        "pass"
    }
}

fn render_markdown_report(report: &AgentReadinessReport) -> String {
    let mut out = String::new();
    out.push_str("# Nova Agent Readiness\n\n");
    let _ = write!(
        out,
        "- gate_status: `{}`\n- readiness_band: `{}`\n- overall_score: `{}` ({})\n- target_count: `{}`\n- blockers: `{}`\n- improvements: `{}`\n\n",
        report.gate_status,
        report.readiness_band,
        report.overall_score,
        report.grade,
        report.summary.target_count,
        report.summary.blocker_count,
        report.summary.improvement_count
    );

    out.push_str("## Persona Scores\n\n");
    out.push_str("| Persona | Score | Grade | Gate | Scored |\n");
    out.push_str("|---|---:|---|---|---:|\n");
    for persona in &report.config.personas {
        if let Some(score) = report.persona_scores.get(persona) {
            let _ = writeln!(
                out,
                "| {} | {} | {} | `{}` | {} / {} |",
                title_case(persona),
                score.overall_score,
                score.grade,
                score.gate_status,
                score.scored_count,
                score.total_available
            );
        }
    }
    out.push('\n');

    append_findings_table(&mut out, "Blockers", &report.blocking_findings);
    append_findings_table(&mut out, "Improvements", &report.improvement_findings);

    out.push_str("## Eval Status\n\n");
    let _ = writeln!(
        out,
        "- status: `{}`\n- supplied: `{}`\n- message: {}\n",
        report.eval_status.status, report.eval_status.supplied, report.eval_status.message
    );

    if !report.entity_findings.is_empty() {
        out.push_str("## Entity Findings\n\n");
        out.push_str("| Entity | Type | Score | Signal gaps | Top recommendation |\n");
        out.push_str("|---|---|---:|---:|---|\n");
        for entity in &report.entity_findings {
            let recommendation = entity
                .recommendations
                .first()
                .map_or("-", |recommendation| recommendation.message.as_str());
            let _ = writeln!(
                out,
                "| `{}` | `{}` | {} ({}) | {} | {} |",
                entity.unique_id,
                entity.resource_type.clone().unwrap_or_default(),
                entity.overall_score,
                entity.grade,
                signal_gap_count(&entity.signals),
                escape_markdown_table_cell(recommendation)
            );
        }
        out.push('\n');
    }

    if !report.indicator_findings.is_empty() {
        out.push_str("## Indicator Findings\n\n");
        out.push_str("| Entity | Indicator | Type | Issue |\n");
        out.push_str("|---|---|---|---|\n");
        for finding in &report.indicator_findings {
            let _ = writeln!(
                out,
                "| `{}` | `{}` | `{}` | {} |",
                finding.unique_id,
                finding.indicator_name.as_deref().unwrap_or("-"),
                finding.indicator_type,
                escape_markdown_table_cell(&finding.issue)
            );
        }
        out.push('\n');
    }

    out.push_str("## Next Actions\n\n");
    for (index, action) in report.next_actions.iter().enumerate() {
        let _ = writeln!(
            out,
            "{}. **{}**: {} ({})",
            index + 1,
            title_case(action.category),
            action.action,
            action.evidence
        );
    }

    out
}

fn append_findings_table(out: &mut String, title: &str, findings: &[ReadinessFinding]) {
    if findings.is_empty() {
        return;
    }
    let _ = writeln!(out, "## {title}\n");
    out.push_str("| Category | Code | Message |\n");
    out.push_str("|---|---|---|\n");
    for finding in findings {
        let _ = writeln!(
            out,
            "| `{}` | `{}` | {} |",
            finding.category,
            finding.code,
            escape_markdown_table_cell(&finding.message)
        );
    }
    out.push('\n');
}

fn print_human_summary(report: &AgentReadinessReport) {
    println!("agent readiness audit complete");
    println!("  gate_status: {}", report.gate_status);
    println!("  readiness_band: {}", report.readiness_band);
    println!(
        "  overall_score: {} ({})",
        report.overall_score, report.grade
    );
    println!("  target_count: {}", report.summary.target_count);
    println!("  blockers: {}", report.summary.blocker_count);
    println!("  improvements: {}", report.summary.improvement_count);
    println!("  eval_status: {}", report.eval_status.status);
}

fn write_report(path: &str, contents: &str) -> Result<()> {
    let path = Path::new(path);
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, contents)?;
    Ok(())
}

fn render_or_propagate_error(
    command_name: &str,
    json: bool,
    error: DbtNovaError,
    elapsed_ms: u128,
) -> DispatchError {
    if json {
        let envelope = error_envelope(command_name, &error, elapsed_ms);
        if let Ok(json) = serde_json::to_string_pretty(&envelope) {
            println!("{json}");
            return DispatchError {
                error,
                rendered: true,
            };
        }
    }
    DispatchError {
        error,
        rendered: false,
    }
}

fn parse_json_input<T>(inline: Option<&str>, path: Option<&str>, default_inline: &str) -> Result<T>
where
    T: for<'de> Deserialize<'de>,
{
    let raw = raw_optional_input(inline, path)?.unwrap_or_else(|| default_inline.to_string());
    serde_json::from_str(&raw).map_err(|error| {
        DbtNovaError::InvalidParams(format!("failed to parse JSON input: {error}"))
    })
}

fn parse_json_array_input(
    inline: Option<&str>,
    path: Option<&str>,
    default_inline: &str,
) -> Result<Vec<String>> {
    parse_json_input(inline, path, default_inline)
}

fn raw_optional_input(inline: Option<&str>, path: Option<&str>) -> Result<Option<String>> {
    if let Some(inline) = inline {
        return Ok(Some(inline.to_string()));
    }
    if let Some(path) = path {
        return fs::read_to_string(path).map(Some).map_err(|error| {
            DbtNovaError::InvalidParams(format!("failed to read {path}: {error}"))
        });
    }
    Ok(None)
}

fn json_u8(data: &JsonValue, field: &str) -> Result<u8> {
    let raw = data.get(field).and_then(JsonValue::as_u64).ok_or_else(|| {
        DbtNovaError::ServerError(format!("metadata score response missing {field}"))
    })?;
    u8::try_from(raw).map_err(|_| {
        DbtNovaError::ServerError(format!("metadata score field {field} out of range"))
    })
}

fn json_string(data: &JsonValue, field: &str) -> Result<String> {
    data.get(field)
        .and_then(JsonValue::as_str)
        .map(ToString::to_string)
        .ok_or_else(|| {
            DbtNovaError::ServerError(format!("metadata score response missing {field}"))
        })
}

fn json_usize_optional(data: &JsonValue, field: &str) -> Option<usize> {
    data.get(field)
        .and_then(JsonValue::as_u64)
        .and_then(|value| usize::try_from(value).ok())
}

fn json_string_array(data: &JsonValue, field: &str) -> Vec<String> {
    data.get(field)
        .and_then(JsonValue::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(JsonValue::as_str)
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn average_score(scores: impl IntoIterator<Item = u8>) -> u8 {
    let mut total = 0usize;
    let mut count = 0usize;
    for score in scores {
        total += usize::from(score);
        count += 1;
    }
    if count == 0 {
        return 0;
    }
    let rounded = (total + (count / 2)) / count;
    u8::try_from(rounded.min(100)).unwrap_or(100)
}

fn current_timestamp_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis())
}

fn title_case(value: &str) -> String {
    value
        .split(['_', '-'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn escape_markdown_table_cell(value: &str) -> String {
    value.replace('|', "\\|").replace('\n', " ")
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::Path;

    use serde_json::json;
    use tempfile::TempDir;

    use super::{
        EvalReadinessStatus, ReadinessFinding, ReadinessThresholdConfig, ThresholdRule,
        ThresholdSeverity, build_agent_readiness_load_config, build_agent_readiness_report,
        evaluate_threshold, parse_eval_status, parse_json_input, render_markdown_report,
        write_report,
    };
    use crate::cli::args::AgentReadinessArgs;
    use crate::cli::manifest::execute_manifest_load;
    use crate::tests::common::fixture_manifest_path_string;

    fn fixture_path_string(fixture_name: &str) -> String {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(format!("tests/fixtures/{fixture_name}"))
            .to_string_lossy()
            .to_string()
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
        let markdown = render_markdown_report(&report);
        assert!(markdown.contains("# Nova Agent Readiness"));
        assert!(markdown.contains("## Persona Scores"));
        assert!(markdown.contains("## Next Actions"));
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

    #[test]
    fn markdown_includes_blockers_and_eval_status() {
        let report = super::AgentReadinessReport {
            schema_version: "agent_readiness.v1",
            generated_at_ms: 1,
            manifest: super::ReadinessManifestSummary {
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
            },
            config: super::ReadinessConfigSummary {
                personas: vec!["engineer".to_string()],
                resource_types: vec!["model".to_string()],
                metadata_only: true,
                read_only: false,
                storage_instance_id: "test".to_string(),
                thresholds: ReadinessThresholdConfig::default(),
            },
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
            },
            persona_scores: BTreeMap::from([(
                "engineer".to_string(),
                super::PersonaReadinessScore {
                    overall_score: 60,
                    grade: "D".to_string(),
                    gate_status: "fail",
                    threshold: None,
                    scored_count: 1,
                    total_available: 1,
                    quality_summary: json!({}),
                },
            )]),
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
            eval_status: EvalReadinessStatus {
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
            },
            next_actions: vec![super::ReadinessNextAction {
                priority: 1,
                category: "eval_gate",
                action: "Fix eval gate".to_string(),
                evidence: "blocked".to_string(),
            }],
        };
        let markdown = render_markdown_report(&report);
        assert!(markdown.contains("## Blockers"));
        assert!(markdown.contains("status: `blocked`"));
        assert!(markdown.contains("Fix eval gate"));
    }
}
