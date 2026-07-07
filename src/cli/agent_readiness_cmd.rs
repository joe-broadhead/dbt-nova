use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::Path;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{Value as JsonValue, json};

use crate::cli::args::{AgentReadinessArgs, ManifestLoadArgs};
use crate::cli::manifest::{build_manifest_load_config, execute_manifest_load};
use crate::cli::output::{CliEnvelope, error_envelope};
use crate::error::{DbtNovaError, Result};
use crate::manifest::entity::{
    ArchivedEntity, column_nova_meta_json, column_primary_key_bool, entity_nova_meta_json,
};
use crate::manifest::search::ManifestSearch;
use crate::params::{
    GetAgentReadinessParams, GetMetadataScoreParams, ModellingConsistencyReportParams,
    PaginationParams,
};
use crate::responses::SuccessResponse;
use crate::tools::metadata_score::{grade_from_score, metadata_score_scoring_contract};
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
const MAX_MODELLING_READINESS_FINDINGS: usize = 12;
const MAX_MODELLING_NEXT_ACTIONS: usize = 3;
const MAX_SUGGESTED_META_PATCHES: usize = 40;
const MAX_GOLDEN_QUESTION_SEEDS: usize = 16;
const MAX_MARKDOWN_META_PATCHES: usize = 10;
const MAX_MARKDOWN_GOLDEN_SEEDS: usize = 8;

#[derive(Debug, Clone, Serialize)]
struct AgentReadinessReport {
    schema_version: &'static str,
    generated_at_ms: u128,
    manifest: ReadinessManifestSummary,
    config: ReadinessConfigSummary,
    scoring_contract: JsonValue,
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
    suggested_meta_patches: Vec<SuggestedMetaPatch>,
    golden_question_seeds: Vec<GoldenQuestionSeed>,
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
    suggested_meta_patch_count: usize,
    golden_question_seed_count: usize,
    score_buckets: JsonValue,
    grade_buckets: JsonValue,
    worst_entities_by_persona: JsonValue,
    category_weak_spots: JsonValue,
    top_recommendation_fields: JsonValue,
    drill_down_hints: JsonValue,
    agent_modelling: JsonValue,
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
    metadata_summary: JsonValue,
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
    diagnostics: JsonValue,
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

struct EntityScoreEvidence {
    diagnostics: JsonValue,
    recommendations: Vec<ReadinessRecommendation>,
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
struct SuggestedMetaPatch {
    id: String,
    target_type: &'static str,
    unique_id: String,
    entity_name: Option<String>,
    resource_type: Option<String>,
    original_file_path: Option<String>,
    column_name: Option<String>,
    indicator_name: Option<String>,
    indicator_type: Option<String>,
    field_path: String,
    suggested_value: JsonValue,
    placeholder: bool,
    rationale: String,
    severity: &'static str,
    confidence: f32,
    evidence: JsonValue,
}

#[derive(Debug, Clone, Serialize)]
struct GoldenQuestionSeed {
    id: String,
    seed_type: &'static str,
    priority: u8,
    persona: &'static str,
    question: String,
    expected_entities: Vec<String>,
    expected_indicators: Vec<String>,
    recommended_assertions: Vec<JsonValue>,
    rationale: String,
    review_required: bool,
    date_policy: &'static str,
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

#[derive(Debug, Clone, Serialize)]
struct AppliedCountThreshold {
    value: usize,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
struct CountThresholdRule {
    value: usize,
    severity: ThresholdSeverity,
}

impl Default for CountThresholdRule {
    fn default() -> Self {
        Self {
            value: 0,
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
struct ModellingThresholdConfig {
    max_blockers: Option<CountThresholdRule>,
    max_high: Option<CountThresholdRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
struct ReadinessThresholdConfig {
    overall: Option<ThresholdRule>,
    persona: PersonaThresholdConfig,
    modelling: ModellingThresholdConfig,
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

struct AgentModellingReadinessResult {
    summary: JsonValue,
    next_actions: Vec<ReadinessNextAction>,
}

struct ReadinessSummaryInput {
    target_count: usize,
    scored_count: usize,
    blocker_count: usize,
    improvement_count: usize,
    indicator_count: usize,
    ambiguous_indicator_count: usize,
    suggested_meta_patch_count: usize,
    golden_question_seed_count: usize,
    triage_summary: ReadinessTriageSummary,
    agent_modelling_summary: JsonValue,
}

struct NextActionInput<'a> {
    overall_score: u8,
    eval_status: &'a EvalReadinessStatus,
    blocking_findings: &'a [ReadinessFinding],
    improvement_findings: &'a [ReadinessFinding],
    ambiguous_indicator_count: usize,
    suggested_meta_patch_count: usize,
    golden_question_seed_count: usize,
    modelling_next_actions: &'a [ReadinessNextAction],
}

struct ReadinessFinalSectionInput<'a> {
    overall_score: u8,
    eval_status: &'a EvalReadinessStatus,
    blocking_findings: &'a [ReadinessFinding],
    improvement_findings: &'a [ReadinessFinding],
    target_count: usize,
    scored_count: usize,
    persona_scores: &'a BTreeMap<String, PersonaReadinessScore>,
    indicator_count: usize,
    ambiguous_indicator_count: usize,
    suggested_meta_patch_count: usize,
    golden_question_seed_count: usize,
    agent_modelling: AgentModellingReadinessResult,
}

struct ReadinessFinalSections {
    readiness_band: &'static str,
    gate_status: &'static str,
    summary: ReadinessSummary,
    next_actions: Vec<ReadinessNextAction>,
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

/// Builds the MCP/CLI-tool response for the agent-readiness report.
///
/// # Errors
/// Returns an error when parameter JSON is invalid or report generation fails.
pub(crate) async fn build_agent_readiness_tool_response(
    search: &ManifestSearch,
    params: &GetAgentReadinessParams,
) -> Result<JsonValue> {
    let inputs = parse_readiness_tool_inputs(params)?;
    let report = build_agent_readiness_report(search, &inputs).await?;
    serde_json::to_value(SuccessResponse::new(report, 1))
        .map_err(|error| DbtNovaError::ServerError(error.to_string()))
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
    parse_readiness_inputs_from_sources(
        args.personas_json.as_deref(),
        None,
        args.thresholds_json.as_deref(),
        args.thresholds_file.as_deref(),
        args.eval_gate_json.as_deref(),
        args.eval_gate_file.as_deref(),
    )
}

fn parse_readiness_tool_inputs(params: &GetAgentReadinessParams) -> Result<ReadinessInputs> {
    parse_readiness_inputs_from_sources(
        params.personas_json.as_deref(),
        None,
        params.thresholds_json.as_deref(),
        None,
        params.eval_gate_json.as_deref(),
        None,
    )
}

fn parse_readiness_inputs_from_sources(
    personas_json: Option<&str>,
    personas_file: Option<&str>,
    thresholds_json: Option<&str>,
    thresholds_file: Option<&str>,
    eval_gate_json: Option<&str>,
    eval_gate_file: Option<&str>,
) -> Result<ReadinessInputs> {
    let mut seen_personas = BTreeSet::new();
    let personas: Vec<String> =
        parse_json_array_input(personas_json, personas_file, DEFAULT_PERSONAS_JSON)?
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

    let thresholds: ReadinessThresholdConfig =
        parse_json_input(thresholds_json, thresholds_file, DEFAULT_THRESHOLDS_JSON)?;
    let eval_status = parse_eval_status(eval_gate_json, eval_gate_file)?;

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
    let effective_thresholds = effective_readiness_thresholds(search, &inputs.thresholds);
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
    let agent_modelling = build_agent_modelling_readiness_result(
        search,
        &effective_thresholds.modelling,
        &mut blocking_findings,
        &mut improvement_findings,
    )
    .await?;
    let suggested_meta_patches =
        build_suggested_meta_patches(search, &entity_findings, &indicator_findings)?;
    let golden_question_seeds =
        build_golden_question_seeds(search, &target_ids, &entity_findings, &indicator_findings)?;

    let final_sections = build_readiness_final_sections(ReadinessFinalSectionInput {
        overall_score,
        eval_status: &inputs.eval_status,
        blocking_findings: &blocking_findings,
        improvement_findings: &improvement_findings,
        target_count,
        scored_count: entity_scores.len(),
        persona_scores: &persona_scores,
        indicator_count,
        ambiguous_indicator_count,
        suggested_meta_patch_count: suggested_meta_patches.len(),
        golden_question_seed_count: golden_question_seeds.len(),
        agent_modelling,
    });

    Ok(AgentReadinessReport {
        schema_version: "agent_readiness.v1",
        generated_at_ms: current_timestamp_ms(),
        manifest: build_manifest_summary(search),
        config: build_config_summary(search, inputs, &resource_types, &effective_thresholds),
        scoring_contract: metadata_score_scoring_contract(),
        overall_score,
        grade: grade.to_string(),
        readiness_band: final_sections.readiness_band,
        gate_status: final_sections.gate_status,
        summary: final_sections.summary,
        persona_scores,
        blocking_findings,
        improvement_findings,
        entity_findings,
        indicator_findings,
        suggested_meta_patches,
        golden_question_seeds,
        eval_status: inputs.eval_status.clone(),
        next_actions: final_sections.next_actions,
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
    thresholds: &ReadinessThresholdConfig,
) -> ReadinessConfigSummary {
    ReadinessConfigSummary {
        personas: inputs.personas.clone(),
        resource_types: resource_types.to_vec(),
        metadata_only: true,
        read_only: search.config().storage_read_only,
        storage_instance_id: search.config().storage_instance_id.clone(),
        thresholds: thresholds.clone(),
    }
}

fn build_readiness_final_sections(input: ReadinessFinalSectionInput<'_>) -> ReadinessFinalSections {
    let has_blockers = !input.blocking_findings.is_empty();
    let has_improvements = !input.improvement_findings.is_empty();
    let readiness_band = readiness_band(input.overall_score, has_blockers);
    let gate_status = gate_status(has_blockers, has_improvements);
    let next_actions = build_next_actions(&NextActionInput {
        overall_score: input.overall_score,
        eval_status: input.eval_status,
        blocking_findings: input.blocking_findings,
        improvement_findings: input.improvement_findings,
        ambiguous_indicator_count: input.ambiguous_indicator_count,
        suggested_meta_patch_count: input.suggested_meta_patch_count,
        golden_question_seed_count: input.golden_question_seed_count,
        modelling_next_actions: &input.agent_modelling.next_actions,
    });
    let triage_summary = build_readiness_triage_summary(input.persona_scores);
    let summary = build_readiness_summary(ReadinessSummaryInput {
        target_count: input.target_count,
        scored_count: input.scored_count,
        blocker_count: input.blocking_findings.len(),
        improvement_count: input.improvement_findings.len(),
        indicator_count: input.indicator_count,
        ambiguous_indicator_count: input.ambiguous_indicator_count,
        suggested_meta_patch_count: input.suggested_meta_patch_count,
        golden_question_seed_count: input.golden_question_seed_count,
        triage_summary,
        agent_modelling_summary: input.agent_modelling.summary,
    });

    ReadinessFinalSections {
        readiness_band,
        gate_status,
        summary,
        next_actions,
    }
}

fn build_readiness_summary(input: ReadinessSummaryInput) -> ReadinessSummary {
    ReadinessSummary {
        target_count: input.target_count,
        scored_count: input.scored_count,
        blocker_count: input.blocker_count,
        improvement_count: input.improvement_count,
        indicator_count: input.indicator_count,
        ambiguous_indicator_count: input.ambiguous_indicator_count,
        suggested_meta_patch_count: input.suggested_meta_patch_count,
        golden_question_seed_count: input.golden_question_seed_count,
        score_buckets: input.triage_summary.score_buckets,
        grade_buckets: input.triage_summary.grade_buckets,
        worst_entities_by_persona: input.triage_summary.worst_entities_by_persona,
        category_weak_spots: input.triage_summary.category_weak_spots,
        top_recommendation_fields: input.triage_summary.top_recommendation_fields,
        drill_down_hints: input.triage_summary.drill_down_hints,
        agent_modelling: input.agent_modelling_summary,
    }
}

fn effective_readiness_thresholds(
    search: &ManifestSearch,
    inputs: &ReadinessThresholdConfig,
) -> ReadinessThresholdConfig {
    let mut thresholds = inputs.clone();
    let modelling = &search.config().agent_readiness.modelling;
    thresholds
        .modelling
        .max_blockers
        .get_or_insert_with(|| CountThresholdRule {
            value: modelling.max_blockers,
            severity: count_threshold_severity(modelling.max_blockers_required),
        });
    thresholds
        .modelling
        .max_high
        .get_or_insert_with(|| CountThresholdRule {
            value: modelling.max_high,
            severity: count_threshold_severity(modelling.max_high_required),
        });
    thresholds
}

fn count_threshold_severity(required: bool) -> ThresholdSeverity {
    if required {
        ThresholdSeverity::Required
    } else {
        ThresholdSeverity::Advisory
    }
}

async fn build_agent_modelling_readiness_result(
    search: &ManifestSearch,
    thresholds: &ModellingThresholdConfig,
    blocking_findings: &mut Vec<ReadinessFinding>,
    improvement_findings: &mut Vec<ReadinessFinding>,
) -> Result<AgentModellingReadinessResult> {
    let response = search
        .modelling_consistency_report(&ModellingConsistencyReportParams {
            resource_types: Vec::new(),
            pagination: PaginationParams {
                // Agent modelling findings are bounded independently by
                // `agent_modelling_audit.max_findings`; keep the legacy
                // overlap/duplicate report payload minimal for readiness.
                limit: Some(1),
                offset: 0,
            },
            min_score: None,
        })
        .await?;
    let data = response.get("data").ok_or_else(|| {
        DbtNovaError::ServerError("modelling consistency response missing data".to_string())
    })?;
    let summary = data
        .get("summary")
        .and_then(|summary| summary.get("agent_modelling"))
        .cloned()
        .unwrap_or_else(default_agent_modelling_summary);
    let findings = data
        .get("agent_modelling_findings")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();

    apply_agent_modelling_findings(&findings, blocking_findings, improvement_findings);
    apply_agent_modelling_thresholds(
        &summary,
        thresholds,
        blocking_findings,
        improvement_findings,
    );
    let next_actions = build_agent_modelling_next_actions(&findings);

    Ok(AgentModellingReadinessResult {
        summary,
        next_actions,
    })
}

fn default_agent_modelling_summary() -> JsonValue {
    json!({
        "total": 0,
        "blockers": 0,
        "high": 0,
        "medium": 0,
        "low": 0,
        "truncated": false,
        "top_codes": [],
        "top_categories": []
    })
}

fn apply_agent_modelling_findings(
    findings: &[JsonValue],
    blocking_findings: &mut Vec<ReadinessFinding>,
    improvement_findings: &mut Vec<ReadinessFinding>,
) {
    let mut emitted = 0;
    for finding in findings {
        if emitted >= MAX_MODELLING_READINESS_FINDINGS {
            break;
        }
        match agent_modelling_finding_severity(finding) {
            Some("blocker") => {
                blocking_findings.push(agent_modelling_readiness_finding(
                    finding,
                    "blocker",
                    "agent_modelling_blocker",
                ));
                emitted += 1;
            }
            Some("high") => {
                improvement_findings.push(agent_modelling_readiness_finding(
                    finding,
                    "improvement",
                    "agent_modelling_high",
                ));
                emitted += 1;
            }
            Some("medium") => {
                improvement_findings.push(agent_modelling_readiness_finding(
                    finding,
                    "improvement",
                    "agent_modelling_medium",
                ));
                emitted += 1;
            }
            _ => {}
        }
    }
}

fn agent_modelling_readiness_finding(
    finding: &JsonValue,
    severity: &'static str,
    code: &'static str,
) -> ReadinessFinding {
    let finding_code = agent_modelling_finding_code(finding).unwrap_or("unknown");
    let finding_category = finding
        .get("category")
        .and_then(JsonValue::as_str)
        .unwrap_or("modelling");
    let finding_severity = agent_modelling_finding_severity(finding).unwrap_or("unknown");
    let message = finding
        .get("message")
        .and_then(JsonValue::as_str)
        .map_or_else(
            || format!("Agent modelling finding `{finding_code}` requires review"),
            ToString::to_string,
        );

    ReadinessFinding {
        severity,
        category: "modelling",
        code,
        message,
        evidence: json!({
            "agent_modelling_code": finding_code,
            "agent_modelling_category": finding_category,
            "agent_modelling_severity": finding_severity,
            "entities": finding.get("entities").cloned().unwrap_or_else(|| json!([])),
            "indicators": finding.get("indicators").cloned().unwrap_or_else(|| json!([])),
            "finding_evidence": finding.get("evidence").cloned().unwrap_or(JsonValue::Null),
            "drill_down_hints": finding.get("drill_down_hints").cloned().unwrap_or_else(|| json!([]))
        }),
    }
}

fn apply_agent_modelling_thresholds(
    summary: &JsonValue,
    thresholds: &ModellingThresholdConfig,
    blocking_findings: &mut Vec<ReadinessFinding>,
    improvement_findings: &mut Vec<ReadinessFinding>,
) {
    if let Some(rule) = thresholds.max_blockers.as_ref() {
        apply_agent_modelling_count_threshold(
            "blockers",
            "agent_modelling_blocker_threshold_missed",
            summary_usize(summary, "blockers"),
            rule,
            summary,
            blocking_findings,
            improvement_findings,
        );
    }
    if let Some(rule) = thresholds.max_high.as_ref() {
        apply_agent_modelling_count_threshold(
            "high",
            "agent_modelling_high_threshold_missed",
            summary_usize(summary, "high"),
            rule,
            summary,
            blocking_findings,
            improvement_findings,
        );
    }
}

fn apply_agent_modelling_count_threshold(
    label: &'static str,
    code: &'static str,
    actual: usize,
    threshold: &CountThresholdRule,
    summary: &JsonValue,
    blocking_findings: &mut Vec<ReadinessFinding>,
    improvement_findings: &mut Vec<ReadinessFinding>,
) {
    if actual <= threshold.value {
        return;
    }
    let gate = count_threshold_gate(threshold.severity);
    let finding = ReadinessFinding {
        severity: threshold_gate_to_finding_severity(gate),
        category: "modelling",
        code,
        message: format!(
            "agent modelling {label} count {actual} exceeded threshold {}",
            threshold.value
        ),
        evidence: json!({
            "count": actual,
            "threshold": applied_count_threshold(threshold),
            "agent_modelling_summary": summary
        }),
    };
    push_threshold_finding(gate, finding, blocking_findings, improvement_findings);
}

fn count_threshold_gate(severity: ThresholdSeverity) -> &'static str {
    match severity {
        ThresholdSeverity::Required => "required_fail",
        ThresholdSeverity::Advisory => "advisory_fail",
    }
}

fn applied_count_threshold(rule: &CountThresholdRule) -> AppliedCountThreshold {
    AppliedCountThreshold {
        value: rule.value,
        severity: rule.severity,
    }
}

fn build_agent_modelling_next_actions(findings: &[JsonValue]) -> Vec<ReadinessNextAction> {
    findings
        .iter()
        .filter(|finding| {
            matches!(
                agent_modelling_finding_severity(finding),
                Some("blocker" | "high" | "medium")
            )
        })
        .take(MAX_MODELLING_NEXT_ACTIONS)
        .map(|finding| {
            let severity = agent_modelling_finding_severity(finding).unwrap_or("unknown");
            let code = agent_modelling_finding_code(finding).unwrap_or("unknown");
            let message = finding
                .get("message")
                .and_then(JsonValue::as_str)
                .unwrap_or("Review the modelling finding.");
            ReadinessNextAction {
                priority: if severity == "blocker" { 1 } else { 2 },
                category: "modelling",
                action: format!("Resolve agent modelling finding `{code}`: {message}"),
                evidence: agent_modelling_next_action_evidence(finding),
            }
        })
        .collect()
}

fn agent_modelling_next_action_evidence(finding: &JsonValue) -> String {
    let severity = agent_modelling_finding_severity(finding).unwrap_or("unknown");
    let code = agent_modelling_finding_code(finding).unwrap_or("unknown");
    let entity = finding
        .get("entities")
        .and_then(JsonValue::as_array)
        .and_then(|entities| entities.first())
        .and_then(|entity| entity.get("unique_id"))
        .and_then(JsonValue::as_str)
        .unwrap_or("unknown_entity");
    format!("{severity}:{code}; entity={entity}")
}

fn agent_modelling_finding_severity(finding: &JsonValue) -> Option<&str> {
    finding.get("severity").and_then(JsonValue::as_str)
}

fn agent_modelling_finding_code(finding: &JsonValue) -> Option<&str> {
    finding.get("code").and_then(JsonValue::as_str)
}

fn summary_usize(summary: &JsonValue, key: &str) -> usize {
    summary
        .get(key)
        .and_then(JsonValue::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or(0)
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
            include_recommendations: true,
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
                metadata_summary: data.get("summary").cloned().unwrap_or(JsonValue::Null),
            },
        );
    }

    Ok((persona_scores, entity_scores))
}

struct ReadinessTriageSummary {
    score_buckets: JsonValue,
    grade_buckets: JsonValue,
    worst_entities_by_persona: JsonValue,
    category_weak_spots: JsonValue,
    top_recommendation_fields: JsonValue,
    drill_down_hints: JsonValue,
}

#[derive(Default)]
struct ReadinessRecommendationAggregate {
    categories: BTreeSet<String>,
    count: usize,
    total_impact: u64,
}

fn build_readiness_triage_summary(
    persona_scores: &BTreeMap<String, PersonaReadinessScore>,
) -> ReadinessTriageSummary {
    let mut score_buckets = BTreeMap::new();
    let mut grade_buckets = BTreeMap::new();
    let mut worst_entities = Vec::new();
    let mut category_weak_spots = Vec::new();
    let mut drill_down_hints = Vec::new();
    let mut recommendation_fields = BTreeMap::<String, ReadinessRecommendationAggregate>::new();

    for (persona, score) in persona_scores {
        if let Some(summary) = score.metadata_summary.as_object() {
            score_buckets.insert(
                persona.clone(),
                summary
                    .get("score_buckets")
                    .cloned()
                    .unwrap_or_else(|| json!({})),
            );
            grade_buckets.insert(
                persona.clone(),
                summary
                    .get("grade_buckets")
                    .cloned()
                    .unwrap_or_else(|| json!({})),
            );
            append_persona_summary_rows(
                persona,
                summary.get("worst_entities").and_then(JsonValue::as_array),
                &mut worst_entities,
            );
            append_persona_summary_rows(
                persona,
                summary
                    .get("category_weak_spots")
                    .and_then(JsonValue::as_array),
                &mut category_weak_spots,
            );
            append_persona_summary_rows(
                persona,
                summary
                    .get("drill_down_hints")
                    .and_then(JsonValue::as_array),
                &mut drill_down_hints,
            );
            ingest_readiness_recommendation_summary(
                &mut recommendation_fields,
                summary
                    .get("top_recommendation_fields")
                    .and_then(JsonValue::as_array),
            );
        }
    }

    category_weak_spots.sort_by(|left, right| {
        json_f64(right, "estimated_weighted_point_gap")
            .partial_cmp(&json_f64(left, "estimated_weighted_point_gap"))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    category_weak_spots.truncate(12);

    let mut top_recommendation_fields = recommendation_fields
        .into_iter()
        .map(|(field, aggregate)| {
            let categories = aggregate.categories.into_iter().collect::<Vec<_>>();
            json!({
                "field": field,
                "categories": categories,
                "count": aggregate.count,
                "estimated_point_impact": aggregate.total_impact
            })
        })
        .collect::<Vec<_>>();
    top_recommendation_fields.sort_by(|left, right| {
        right
            .get("count")
            .and_then(JsonValue::as_u64)
            .cmp(&left.get("count").and_then(JsonValue::as_u64))
            .then_with(|| {
                right
                    .get("estimated_point_impact")
                    .and_then(JsonValue::as_u64)
                    .cmp(
                        &left
                            .get("estimated_point_impact")
                            .and_then(JsonValue::as_u64),
                    )
            })
            .then_with(|| {
                left.get("field")
                    .and_then(JsonValue::as_str)
                    .cmp(&right.get("field").and_then(JsonValue::as_str))
            })
    });
    top_recommendation_fields.truncate(10);
    drill_down_hints.truncate(10);

    ReadinessTriageSummary {
        score_buckets: serde_json::to_value(score_buckets).unwrap_or(JsonValue::Null),
        grade_buckets: serde_json::to_value(grade_buckets).unwrap_or(JsonValue::Null),
        worst_entities_by_persona: JsonValue::Array(worst_entities),
        category_weak_spots: JsonValue::Array(category_weak_spots),
        top_recommendation_fields: JsonValue::Array(top_recommendation_fields),
        drill_down_hints: JsonValue::Array(drill_down_hints),
    }
}

fn append_persona_summary_rows(
    persona: &str,
    rows: Option<&Vec<JsonValue>>,
    out: &mut Vec<JsonValue>,
) {
    let Some(rows) = rows else {
        return;
    };
    for row in rows.iter().take(5) {
        let mut row = row.clone();
        if let Some(obj) = row.as_object_mut() {
            obj.entry("persona".to_string())
                .or_insert_with(|| JsonValue::String(persona.to_string()));
        }
        out.push(row);
    }
}

fn ingest_readiness_recommendation_summary(
    aggregate: &mut BTreeMap<String, ReadinessRecommendationAggregate>,
    rows: Option<&Vec<JsonValue>>,
) {
    let Some(rows) = rows else {
        return;
    };
    for row in rows {
        let field = row
            .get("field")
            .and_then(JsonValue::as_str)
            .unwrap_or("metadata")
            .to_string();
        let entry = aggregate.entry(field).or_default();
        entry.count += row
            .get("count")
            .and_then(JsonValue::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .unwrap_or(0);
        entry.total_impact += row
            .get("estimated_point_impact")
            .and_then(JsonValue::as_u64)
            .unwrap_or(0);
        if let Some(category) = row.get("category").and_then(JsonValue::as_str) {
            entry.categories.insert(category.to_string());
        }
    }
}

fn json_f64(value: &JsonValue, field: &str) -> f64 {
    value.get(field).and_then(JsonValue::as_f64).unwrap_or(0.0)
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
        let score_evidence = entity_score_evidence(search, &unique_id, score).await?;
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
            diagnostics: score_evidence.diagnostics,
            recommendations: score_evidence.recommendations,
        });
    }

    Ok((entity_findings, improvements))
}

async fn entity_score_evidence(
    search: &ManifestSearch,
    unique_id: &str,
    score: &EntityScoreAccumulator,
) -> Result<EntityScoreEvidence> {
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
    let recommendations = data
        .get("recommendations")
        .and_then(JsonValue::as_array)
        .map_or_else(Vec::new, |recommendations| {
            recommendations
                .iter()
                .take(MAX_ENTITY_RECOMMENDATIONS)
                .map(readiness_recommendation_from_json)
                .collect()
        });
    Ok(EntityScoreEvidence {
        diagnostics: data
            .get("diagnostics")
            .cloned()
            .unwrap_or_else(|| JsonValue::Array(Vec::new())),
        recommendations,
    })
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

fn build_suggested_meta_patches(
    search: &ManifestSearch,
    entity_findings: &[EntityReadinessFinding],
    indicator_findings: &[IndicatorReadinessFinding],
) -> Result<Vec<SuggestedMetaPatch>> {
    let mut patches = Vec::new();
    let mut seen = BTreeSet::new();

    for finding in entity_findings {
        let Some(entity) = search.get_entity_archived(&finding.unique_id)? else {
            continue;
        };
        append_entity_meta_patches(entity, finding, &mut patches, &mut seen);
        if patches.len() >= MAX_SUGGESTED_META_PATCHES {
            return Ok(patches);
        }
    }

    for finding in indicator_findings {
        let Some(entity) = search.get_entity_archived(&finding.unique_id)? else {
            continue;
        };
        append_indicator_meta_patches(entity, finding, &mut patches, &mut seen);
        if patches.len() >= MAX_SUGGESTED_META_PATCHES {
            break;
        }
    }

    Ok(patches)
}

fn append_entity_meta_patches(
    entity: &ArchivedEntity,
    finding: &EntityReadinessFinding,
    patches: &mut Vec<SuggestedMetaPatch>,
    seen: &mut BTreeSet<String>,
) {
    let entity_json = entity.to_json_value();
    let nova = entity_nova_meta_json(&entity_json);
    let nova = nova.as_deref();

    if !finding.signals.has_owner {
        push_meta_patch(
            patches,
            seen,
            entity_patch(
                entity,
                MetaPatchTarget::Entity,
                MetaPatchContent {
                    field_path: "meta.owner",
                    suggested_value: json!("__OWNER_OR_TEAM__"),
                    placeholder: true,
                    rationale: "Add the owning team or person so agents can route stewardship and review questions.",
                    confidence: 0.95,
                    evidence: json!({"signal": "missing_owner"}),
                },
            ),
        );
    }

    if !json_array_field_non_empty(nova, "domains") {
        push_meta_patch(
            patches,
            seen,
            entity_patch(
                entity,
                MetaPatchTarget::Entity,
                MetaPatchContent {
                    field_path: "meta.nova.domains",
                    suggested_value: json!(["__DOMAIN__"]),
                    placeholder: true,
                    rationale: "Add one or more business domains to improve routing and retrieval.",
                    confidence: 0.86,
                    evidence: json!({"signal": "missing_nova_domains"}),
                },
            ),
        );
    }

    if !json_array_field_non_empty(nova, "use_cases") {
        push_meta_patch(
            patches,
            seen,
            entity_patch(
                entity,
                MetaPatchTarget::Entity,
                MetaPatchContent {
                    field_path: "meta.nova.use_cases",
                    suggested_value: json!(["__USE_CASE__"]),
                    placeholder: true,
                    rationale: "Add common analyst tasks or reporting use cases that this dataset supports.",
                    confidence: 0.84,
                    evidence: json!({"signal": "missing_nova_use_cases"}),
                },
            ),
        );
    }

    if !json_bool_field_present(nova, "canonical") {
        push_meta_patch(
            patches,
            seen,
            entity_patch(
                entity,
                MetaPatchTarget::Entity,
                MetaPatchContent {
                    field_path: "meta.nova.canonical",
                    suggested_value: json!("__TRUE_IF_CANONICAL_DATASET__"),
                    placeholder: true,
                    rationale: "Clarify whether this is the preferred dataset for its concept before agents promote it.",
                    confidence: 0.62,
                    evidence: json!({"signal": "missing_canonical_flag"}),
                },
            ),
        );
    }

    append_grain_meta_patches(entity, &entity_json, nova, finding, patches, seen);
    append_governance_meta_patches(entity, nova, patches, seen);
    append_column_semantic_patches(entity, &entity_json, patches, seen);
    append_indicator_seed_patch(entity, nova, patches, seen);
}

fn append_grain_meta_patches(
    entity: &ArchivedEntity,
    entity_json: &JsonValue,
    nova: Option<&JsonValue>,
    finding: &EntityReadinessFinding,
    patches: &mut Vec<SuggestedMetaPatch>,
    seen: &mut BTreeSet<String>,
) {
    if has_invalid_grain_diagnostic(&finding.diagnostics, "meta.nova.grain") {
        append_invalid_grain_shape_patch(entity, entity_json, patches, seen);
        return;
    }

    let primary_keys = nova
        .and_then(|nova| nova.get("grain"))
        .and_then(|grain| grain.get("primary_key"))
        .and_then(JsonValue::as_array)
        .is_some_and(|keys| !keys.is_empty());
    if !primary_keys && !finding.signals.has_primary_key && finding.signals.column_count > 0 {
        let candidate = infer_primary_key_column(entity_json);
        let suggested_value = candidate.as_ref().map_or_else(
            || json!(["__PRIMARY_KEY_COLUMN__"]),
            |column| json!([column]),
        );
        push_meta_patch(
            patches,
            seen,
            entity_patch(
                entity,
                MetaPatchTarget::Entity,
                MetaPatchContent {
                    field_path: "meta.nova.grain.primary_key",
                    suggested_value,
                    placeholder: candidate.is_none(),
                    rationale: "Add the row-level identifier columns used to establish dataset grain.",
                    confidence: if candidate.is_some() { 0.72 } else { 0.58 },
                    evidence: json!({"signal": "missing_grain_primary_key", "inferred_from_column_name": candidate.is_some()}),
                },
            ),
        );
        if let Some(column) = candidate {
            push_meta_patch(
                patches,
                seen,
                entity_patch(
                    entity,
                    MetaPatchTarget::Column(column.as_str()),
                    MetaPatchContent {
                        field_path: &format!("columns.{column}.meta.primary_key"),
                        suggested_value: json!(true),
                        placeholder: false,
                        rationale: "Mark the likely identifier column as a primary key after review.",
                        confidence: 0.68,
                        evidence: json!({"signal": "missing_column_primary_key", "inferred_from_column_name": true}),
                    },
                ),
            );
        }
    }

    let time_field = nova
        .and_then(|nova| nova.get("grain"))
        .and_then(|grain| grain.get("time_field"))
        .and_then(JsonValue::as_str)
        .is_some_and(|field| !field.trim().is_empty());
    if !time_field {
        let candidate = infer_time_column(entity_json);
        let suggested_value = candidate
            .as_ref()
            .map_or_else(|| json!("__TIME_FIELD__"), |column| json!(column));
        push_meta_patch(
            patches,
            seen,
            entity_patch(
                entity,
                MetaPatchTarget::Entity,
                MetaPatchContent {
                    field_path: "meta.nova.grain.time_field",
                    suggested_value,
                    placeholder: candidate.is_none(),
                    rationale: "Add the default time field used for date-bounded questions, or leave absent if not applicable.",
                    confidence: if candidate.is_some() { 0.70 } else { 0.52 },
                    evidence: json!({"signal": "missing_grain_time_field", "inferred_from_column_name": candidate.is_some()}),
                },
            ),
        );
    }
}

fn append_invalid_grain_shape_patch(
    entity: &ArchivedEntity,
    entity_json: &JsonValue,
    patches: &mut Vec<SuggestedMetaPatch>,
    seen: &mut BTreeSet<String>,
) {
    let primary_key = infer_primary_key_column(entity_json).map_or_else(
        || json!(["__PRIMARY_KEY_COLUMN__"]),
        |column| json!([column]),
    );
    let time_field = infer_time_column(entity_json)
        .map_or_else(|| json!("__TIME_FIELD__"), |column| json!(column));
    push_meta_patch(
        patches,
        seen,
        entity_patch(
            entity,
            MetaPatchTarget::Entity,
            MetaPatchContent {
                field_path: "meta.nova.grain",
                suggested_value: json!({
                    "primary_key": primary_key,
                    "time_field": time_field,
                    "dimensions": ["__DIMENSION_COLUMN__"]
                }),
                placeholder: true,
                rationale: "Replace the invalid grain value with the canonical Nova object shape before adding nested grain fields.",
                confidence: 0.74,
                evidence: json!({"diagnostic": "invalid_grain_shape"}),
            },
        ),
    );
}

fn has_invalid_grain_diagnostic(diagnostics: &JsonValue, field: &str) -> bool {
    diagnostics.as_array().is_some_and(|items| {
        items.iter().any(|item| {
            item.get("code").and_then(JsonValue::as_str) == Some("invalid_grain_shape")
                && item.get("field").and_then(JsonValue::as_str) == Some(field)
        })
    })
}

fn append_governance_meta_patches(
    entity: &ArchivedEntity,
    nova: Option<&JsonValue>,
    patches: &mut Vec<SuggestedMetaPatch>,
    seen: &mut BTreeSet<String>,
) {
    let governance = nova.and_then(|nova| nova.get("governance"));
    if governance
        .and_then(|governance| governance.get("sensitivity"))
        .and_then(JsonValue::as_str)
        .is_none_or(|value| value.trim().is_empty())
    {
        push_meta_patch(
            patches,
            seen,
            entity_patch(
                entity,
                MetaPatchTarget::Entity,
                MetaPatchContent {
                    field_path: "meta.nova.governance.sensitivity",
                    suggested_value: json!("__SENSITIVITY__"),
                    placeholder: true,
                    rationale: "Classify sensitivity so governance agents know how cautiously to handle this dataset.",
                    confidence: 0.82,
                    evidence: json!({"signal": "missing_governance_sensitivity"}),
                },
            ),
        );
    }
    if governance
        .and_then(|governance| governance.get("pii"))
        .and_then(JsonValue::as_str)
        .is_none_or(|value| value.trim().is_empty())
    {
        push_meta_patch(
            patches,
            seen,
            entity_patch(
                entity,
                MetaPatchTarget::Entity,
                MetaPatchContent {
                    field_path: "meta.nova.governance.pii",
                    suggested_value: json!("__PII_CLASSIFICATION__"),
                    placeholder: true,
                    rationale: "Record whether the dataset contains PII so agents can avoid unsafe disclosure.",
                    confidence: 0.82,
                    evidence: json!({"signal": "missing_governance_pii"}),
                },
            ),
        );
    }
    if governance
        .and_then(|governance| governance.get("compliance"))
        .and_then(JsonValue::as_array)
        .is_none_or(Vec::is_empty)
    {
        push_meta_patch(
            patches,
            seen,
            entity_patch(
                entity,
                MetaPatchTarget::Entity,
                MetaPatchContent {
                    field_path: "meta.nova.governance.compliance",
                    suggested_value: json!(["__COMPLIANCE_FRAMEWORK__"]),
                    placeholder: true,
                    rationale: "List applicable compliance frameworks, or replace with an explicit none/empty policy after review.",
                    confidence: 0.72,
                    evidence: json!({"signal": "missing_governance_compliance"}),
                },
            ),
        );
    }
}

fn append_column_semantic_patches(
    entity: &ArchivedEntity,
    entity_json: &JsonValue,
    patches: &mut Vec<SuggestedMetaPatch>,
    seen: &mut BTreeSet<String>,
) {
    let Some(columns) = entity_json.get("columns").and_then(JsonValue::as_object) else {
        return;
    };
    for (column_name, column) in columns.iter().take(3) {
        let nova = column_nova_meta_json(column);
        let nova = nova.as_deref();
        let has_role = nova
            .and_then(|nova| nova.get("role"))
            .and_then(JsonValue::as_str)
            .is_some_and(|value| !value.trim().is_empty());
        let has_semantic_type = nova
            .and_then(|nova| nova.get("semantic_type"))
            .and_then(JsonValue::as_str)
            .is_some_and(|value| !value.trim().is_empty());
        if has_role && has_semantic_type {
            continue;
        }
        let lowered = column_name.to_ascii_lowercase();
        if !has_role && (lowered == "id" || lowered.ends_with("_id")) {
            push_meta_patch(
                patches,
                seen,
                entity_patch(
                    entity,
                    MetaPatchTarget::Column(column_name.as_str()),
                    MetaPatchContent {
                        field_path: &format!("columns.{column_name}.meta.nova.role"),
                        suggested_value: json!("identifier"),
                        placeholder: false,
                        rationale: "Column name suggests an identifier; confirm before applying.",
                        confidence: 0.70,
                        evidence: json!({"signal": "missing_column_role", "inferred_from_column_name": true}),
                    },
                ),
            );
        } else if !has_role && looks_like_time_column(&lowered) {
            push_meta_patch(
                patches,
                seen,
                entity_patch(
                    entity,
                    MetaPatchTarget::Column(column_name.as_str()),
                    MetaPatchContent {
                        field_path: &format!("columns.{column_name}.meta.nova.role"),
                        suggested_value: json!("time"),
                        placeholder: false,
                        rationale: "Column name suggests a time dimension; confirm before applying.",
                        confidence: 0.70,
                        evidence: json!({"signal": "missing_column_role", "inferred_from_column_name": true}),
                    },
                ),
            );
        } else if !has_semantic_type {
            push_meta_patch(
                patches,
                seen,
                entity_patch(
                    entity,
                    MetaPatchTarget::Column(column_name.as_str()),
                    MetaPatchContent {
                        field_path: &format!("columns.{column_name}.meta.nova.semantic_type"),
                        suggested_value: json!("__SEMANTIC_TYPE__"),
                        placeholder: true,
                        rationale: "Add a stable semantic label when this column is used for search, filtering, or governance.",
                        confidence: 0.54,
                        evidence: json!({"signal": "missing_column_semantic_type"}),
                    },
                ),
            );
        }
    }
}

fn append_indicator_seed_patch(
    entity: &ArchivedEntity,
    nova: Option<&JsonValue>,
    patches: &mut Vec<SuggestedMetaPatch>,
    seen: &mut BTreeSet<String>,
) {
    if nova.is_none() || indicator_count_in_nova(nova) > 0 {
        return;
    }
    push_meta_patch(
        patches,
        seen,
        entity_patch(
            entity,
            MetaPatchTarget::Indicator {
                name: None,
                kind: "measure",
            },
            MetaPatchContent {
                field_path: "meta.nova.measures",
                suggested_value: json!([{
                    "name": "__MEASURE_NAME__",
                    "description": "__MEASURE_DESCRIPTION__",
                    "expression": "__AGGREGATION_EXPRESSION__",
                    "field": "__SOURCE_COLUMN__",
                    "canonical": false
                }]),
                placeholder: true,
                rationale: "Add measure definitions when this model owns reusable business quantities.",
                confidence: 0.50,
                evidence: json!({"signal": "missing_nova_indicators"}),
            },
        ),
    );
}

fn append_indicator_meta_patches(
    entity: &ArchivedEntity,
    finding: &IndicatorReadinessFinding,
    patches: &mut Vec<SuggestedMetaPatch>,
    seen: &mut BTreeSet<String>,
) {
    let base_path = indicator_meta_base_path(finding);
    if finding.issue.contains("expression or field") {
        append_indicator_execution_patches(entity, finding, &base_path, patches, seen);
    } else if finding.issue.contains("grain.time_field") {
        append_indicator_time_patch(entity, finding, &base_path, patches, seen);
    } else if finding.issue.contains("missing a description") {
        append_indicator_description_patch(entity, finding, &base_path, patches, seen);
    } else if finding.issue.contains("not an object") {
        append_malformed_indicator_patch(entity, finding, &base_path, patches, seen);
    }
}

fn indicator_patch_target(finding: &IndicatorReadinessFinding) -> MetaPatchTarget<'_> {
    MetaPatchTarget::Indicator {
        name: finding.indicator_name.as_deref(),
        kind: finding.indicator_type.as_str(),
    }
}

fn append_indicator_execution_patches(
    entity: &ArchivedEntity,
    finding: &IndicatorReadinessFinding,
    base_path: &str,
    patches: &mut Vec<SuggestedMetaPatch>,
    seen: &mut BTreeSet<String>,
) {
    push_meta_patch(
        patches,
        seen,
        entity_patch(
            entity,
            indicator_patch_target(finding),
            MetaPatchContent {
                field_path: &format!("{base_path}.expression"),
                suggested_value: json!("__EXPRESSION_OR_FIELD__"),
                placeholder: true,
                rationale: "Add an explicit expression or field before using this indicator as eval ground truth.",
                confidence: 0.90,
                evidence: json!({"indicator_issue": finding.issue}),
            },
        ),
    );
    push_meta_patch(
        patches,
        seen,
        entity_patch(
            entity,
            indicator_patch_target(finding),
            MetaPatchContent {
                field_path: &format!("{base_path}.canonical"),
                suggested_value: json!("__TRUE_IF_CANONICAL_INDICATOR__"),
                placeholder: true,
                rationale: "Clarify canonical indicator ownership instead of letting agents guess between similarly named metrics.",
                confidence: 0.78,
                evidence: json!({"indicator_issue": finding.issue}),
            },
        ),
    );
}

fn append_indicator_time_patch(
    entity: &ArchivedEntity,
    finding: &IndicatorReadinessFinding,
    base_path: &str,
    patches: &mut Vec<SuggestedMetaPatch>,
    seen: &mut BTreeSet<String>,
) {
    push_meta_patch(
        patches,
        seen,
        entity_patch(
            entity,
            indicator_patch_target(finding),
            MetaPatchContent {
                field_path: &format!("{base_path}.grain.time_field"),
                suggested_value: json!("__TIME_FIELD__"),
                placeholder: true,
                rationale: "Add the indicator time grain so date-bounded evals and questions can be anchored safely.",
                confidence: 0.84,
                evidence: json!({"indicator_issue": finding.issue}),
            },
        ),
    );
}

fn append_indicator_description_patch(
    entity: &ArchivedEntity,
    finding: &IndicatorReadinessFinding,
    base_path: &str,
    patches: &mut Vec<SuggestedMetaPatch>,
    seen: &mut BTreeSet<String>,
) {
    let field_path = if base_path == "description" {
        base_path.to_string()
    } else {
        format!("{base_path}.description")
    };
    push_meta_patch(
        patches,
        seen,
        entity_patch(
            entity,
            indicator_patch_target(finding),
            MetaPatchContent {
                field_path: &field_path,
                suggested_value: json!("__INDICATOR_DESCRIPTION__"),
                placeholder: true,
                rationale: "Describe the indicator before asking reviewers or agents to rely on it.",
                confidence: 0.76,
                evidence: json!({"indicator_issue": finding.issue}),
            },
        ),
    );
}

fn append_malformed_indicator_patch(
    entity: &ArchivedEntity,
    finding: &IndicatorReadinessFinding,
    base_path: &str,
    patches: &mut Vec<SuggestedMetaPatch>,
    seen: &mut BTreeSet<String>,
) {
    push_meta_patch(
        patches,
        seen,
        entity_patch(
            entity,
            indicator_patch_target(finding),
            MetaPatchContent {
                field_path: base_path,
                suggested_value: json!({
                    "name": "__INDICATOR_NAME__",
                    "description": "__INDICATOR_DESCRIPTION__",
                    "expression": "__EXPRESSION_OR_FIELD__"
                }),
                placeholder: true,
                rationale: "Replace the malformed indicator with a structured object before generating eval seeds.",
                confidence: 0.88,
                evidence: json!({"indicator_issue": finding.issue}),
            },
        ),
    );
}

fn build_golden_question_seeds(
    search: &ManifestSearch,
    target_ids: &[String],
    entity_findings: &[EntityReadinessFinding],
    indicator_findings: &[IndicatorReadinessFinding],
) -> Result<Vec<GoldenQuestionSeed>> {
    let mut seeds = Vec::new();
    let mut seen = BTreeSet::new();

    for unique_id in target_ids {
        let Some(entity) = search.get_entity_archived(unique_id)? else {
            continue;
        };
        append_canonical_indicator_seeds(entity, unique_id, &mut seeds, &mut seen);
        if seeds.len() >= MAX_GOLDEN_QUESTION_SEEDS {
            return Ok(seeds);
        }
    }

    for finding in indicator_findings {
        append_indicator_review_seed(finding, &mut seeds, &mut seen);
        if seeds.len() >= MAX_GOLDEN_QUESTION_SEEDS {
            return Ok(seeds);
        }
    }

    for finding in entity_findings {
        append_entity_review_seeds(finding, &mut seeds, &mut seen);
        if seeds.len() >= MAX_GOLDEN_QUESTION_SEEDS {
            break;
        }
    }

    seeds.sort_by(|left, right| {
        left.priority
            .cmp(&right.priority)
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(seeds)
}

fn append_canonical_indicator_seeds(
    entity: &ArchivedEntity,
    unique_id: &str,
    seeds: &mut Vec<GoldenQuestionSeed>,
    seen: &mut BTreeSet<String>,
) {
    let entity_json = entity.to_json_value();
    let nova = entity_nova_meta_json(&entity_json);
    let Some(nova) = nova.as_deref() else {
        return;
    };
    let entity_canonical = nova
        .get("canonical")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false);
    append_metric_seed_from_object(
        entity,
        unique_id,
        nova.get("metric"),
        entity_canonical,
        seeds,
        seen,
    );
    if let Some(metrics) = nova.get("metrics").and_then(JsonValue::as_array) {
        for metric in metrics {
            append_metric_seed_from_object(
                entity,
                unique_id,
                Some(metric),
                entity_canonical,
                seeds,
                seen,
            );
        }
    }
    if let Some(measures) = nova.get("measures").and_then(JsonValue::as_array) {
        for measure in measures {
            append_measure_seed_from_object(
                entity,
                unique_id,
                measure,
                entity_canonical,
                seeds,
                seen,
            );
        }
    }
}

fn append_metric_seed_from_object(
    entity: &ArchivedEntity,
    unique_id: &str,
    metric: Option<&JsonValue>,
    entity_canonical: bool,
    seeds: &mut Vec<GoldenQuestionSeed>,
    seen: &mut BTreeSet<String>,
) {
    let Some(metric) = metric.and_then(JsonValue::as_object) else {
        return;
    };
    let Some(name) = metric.get("name").and_then(JsonValue::as_str) else {
        return;
    };
    if !string_value_present(metric.get("expression")) {
        return;
    }
    let canonical = metric
        .get("canonical")
        .and_then(JsonValue::as_bool)
        .unwrap_or(entity_canonical);
    if !canonical {
        return;
    }
    let query = indicator_seed_query(entity, Some(metric), name);
    let resource_type = entity.resource_type_str().unwrap_or("model");
    push_seed(
        seeds,
        seen,
        GoldenQuestionSeed {
            id: stable_seed_id("bridge", unique_id, Some("metric"), Some(name)),
            seed_type: "bridge",
            priority: 1,
            persona: "analyst",
            question: format!("Find the canonical {name} metric for this project."),
            expected_entities: vec![unique_id.to_string()],
            expected_indicators: vec![name.to_string()],
            recommended_assertions: vec![json!({
                "type": "search_indicator_rank",
                "query": query,
                "expected": name,
                "max_rank": 3,
                "resource_types": [resource_type],
                "indicator_types": ["metric"],
                "persona": "analyst"
            })],
            rationale: "Canonical metric has an explicit expression, so it can seed a deterministic bridge eval.".to_string(),
            review_required: true,
            date_policy: "not_date_sensitive",
        },
    );
}

fn append_measure_seed_from_object(
    entity: &ArchivedEntity,
    unique_id: &str,
    measure: &JsonValue,
    entity_canonical: bool,
    seeds: &mut Vec<GoldenQuestionSeed>,
    seen: &mut BTreeSet<String>,
) {
    let Some(measure) = measure.as_object() else {
        return;
    };
    let Some(name) = measure.get("name").and_then(JsonValue::as_str) else {
        return;
    };
    if !string_value_present(measure.get("expression"))
        && !string_value_present(measure.get("field"))
    {
        return;
    }
    let canonical = measure
        .get("canonical")
        .and_then(JsonValue::as_bool)
        .unwrap_or(entity_canonical);
    if !canonical {
        return;
    }
    let query = indicator_seed_query(entity, Some(measure), name);
    let resource_type = entity.resource_type_str().unwrap_or("model");
    push_seed(
        seeds,
        seen,
        GoldenQuestionSeed {
            id: stable_seed_id("bridge", unique_id, Some("measure"), Some(name)),
            seed_type: "bridge",
            priority: 2,
            persona: "analyst",
            question: format!("Find the canonical {name} measure for this project."),
            expected_entities: vec![unique_id.to_string()],
            expected_indicators: vec![name.to_string()],
            recommended_assertions: vec![json!({
                "type": "search_indicator_rank",
                "query": query,
                "expected": name,
                "max_rank": 3,
                "resource_types": [resource_type],
                "indicator_types": ["measure"],
                "persona": "analyst"
            })],
            rationale: "Canonical measure has execution metadata, so it can seed a deterministic bridge eval.".to_string(),
            review_required: true,
            date_policy: "not_date_sensitive",
        },
    );
}

fn append_indicator_review_seed(
    finding: &IndicatorReadinessFinding,
    seeds: &mut Vec<GoldenQuestionSeed>,
    seen: &mut BTreeSet<String>,
) {
    let indicator_label = finding
        .indicator_name
        .as_deref()
        .unwrap_or("unnamed indicator");
    push_seed(
        seeds,
        seen,
        GoldenQuestionSeed {
            id: stable_seed_id(
                "manual_review",
                &finding.unique_id,
                Some(finding.indicator_type.as_str()),
                finding.indicator_name.as_deref().or(Some(finding.issue.as_str())),
            ),
            seed_type: "manual_review",
            priority: 2,
            persona: "analyst",
            question: format!(
                "Review {indicator_label} on {} before turning it into a gated eval.",
                finding.unique_id
            ),
            expected_entities: vec![finding.unique_id.clone()],
            expected_indicators: finding.indicator_name.iter().cloned().collect(),
            recommended_assertions: vec![json!({
                "type": "manual_review",
                "instruction": format!("Resolve indicator readiness issue: {}", finding.issue)
            })],
            rationale: "Indicator metadata is ambiguous, so the first seed should request review rather than assert false ground truth.".to_string(),
            review_required: true,
            date_policy: "not_date_sensitive",
        },
    );
}

fn append_entity_review_seeds(
    finding: &EntityReadinessFinding,
    seeds: &mut Vec<GoldenQuestionSeed>,
    seen: &mut BTreeSet<String>,
) {
    if !finding.signals.has_nova_meta || !finding.signals.has_description {
        push_seed(
            seeds,
            seen,
            GoldenQuestionSeed {
                id: stable_seed_id("manual_review", &finding.unique_id, Some("context"), None),
                seed_type: "manual_review",
                priority: 3,
                persona: "analyst",
                question: format!(
                    "Review whether {} has enough business context for analyst questions.",
                    finding.unique_id
                ),
                expected_entities: vec![finding.unique_id.clone()],
                expected_indicators: Vec::new(),
                recommended_assertions: vec![json!({
                    "type": "metadata_score_min",
                    "id_or_name": finding.unique_id,
                    "threshold": 0.70,
                    "persona": "analyst"
                })],
                rationale: "Entity context is incomplete; make the first eval a review or metadata score gate before answer-correctness checks.".to_string(),
                review_required: true,
                date_policy: "not_date_sensitive",
            },
        );
    }
    if !finding.signals.has_nova_meta {
        push_seed(
            seeds,
            seen,
            GoldenQuestionSeed {
                id: stable_seed_id("manual_review", &finding.unique_id, Some("governance"), None),
                seed_type: "manual_review",
                priority: 3,
                persona: "governance",
                question: format!(
                    "Review governance classification for {} before production agent use.",
                    finding.unique_id
                ),
                expected_entities: vec![finding.unique_id.clone()],
                expected_indicators: Vec::new(),
                recommended_assertions: vec![json!({
                    "type": "metadata_score_min",
                    "id_or_name": finding.unique_id,
                    "threshold": 0.65,
                    "persona": "governance"
                })],
                rationale: "Governance coverage is missing or weak; keep this seed advisory until sensitivity and PII fields are reviewed.".to_string(),
                review_required: true,
                date_policy: "not_date_sensitive",
            },
        );
    }
}

#[derive(Clone, Copy)]
enum MetaPatchTarget<'a> {
    Entity,
    Column(&'a str),
    Indicator {
        name: Option<&'a str>,
        kind: &'a str,
    },
}

struct MetaPatchContent<'a> {
    field_path: &'a str,
    suggested_value: JsonValue,
    placeholder: bool,
    rationale: &'a str,
    confidence: f32,
    evidence: JsonValue,
}

fn entity_patch(
    entity: &ArchivedEntity,
    target: MetaPatchTarget<'_>,
    content: MetaPatchContent<'_>,
) -> SuggestedMetaPatch {
    let (target_type, column_name, indicator_name, indicator_type) = match target {
        MetaPatchTarget::Entity => ("entity", None, None, None),
        MetaPatchTarget::Column(name) => ("column", Some(name), None, None),
        MetaPatchTarget::Indicator { name, kind } => ("indicator", None, name, Some(kind)),
    };
    let unique_id = entity.unique_id.as_str().to_string();
    SuggestedMetaPatch {
        id: stable_meta_patch_id(
            &unique_id,
            column_name,
            indicator_type,
            indicator_name,
            content.field_path,
        ),
        target_type,
        unique_id,
        entity_name: entity.name_str().map(ToString::to_string),
        resource_type: entity.resource_type_str().map(ToString::to_string),
        original_file_path: entity.original_file_path_str().map(ToString::to_string),
        column_name: column_name.map(ToString::to_string),
        indicator_name: indicator_name.map(ToString::to_string),
        indicator_type: indicator_type.map(ToString::to_string),
        field_path: content.field_path.to_string(),
        suggested_value: content.suggested_value,
        placeholder: content.placeholder,
        rationale: content.rationale.to_string(),
        severity: "improvement",
        confidence: content.confidence,
        evidence: content.evidence,
    }
}

fn push_meta_patch(
    patches: &mut Vec<SuggestedMetaPatch>,
    seen: &mut BTreeSet<String>,
    patch: SuggestedMetaPatch,
) {
    if patches.len() >= MAX_SUGGESTED_META_PATCHES || !seen.insert(patch.id.clone()) {
        return;
    }
    patches.push(patch);
}

fn push_seed(
    seeds: &mut Vec<GoldenQuestionSeed>,
    seen: &mut BTreeSet<String>,
    question_seed: GoldenQuestionSeed,
) {
    if seeds.len() >= MAX_GOLDEN_QUESTION_SEEDS || !seen.insert(question_seed.id.clone()) {
        return;
    }
    seeds.push(question_seed);
}

fn json_array_field_non_empty(nova: Option<&JsonValue>, field: &str) -> bool {
    nova.and_then(|nova| nova.get(field))
        .and_then(JsonValue::as_array)
        .is_some_and(|values| !values.is_empty())
}

fn json_bool_field_present(nova: Option<&JsonValue>, field: &str) -> bool {
    nova.and_then(|nova| nova.get(field))
        .and_then(JsonValue::as_bool)
        .is_some()
}

fn infer_primary_key_column(entity_json: &JsonValue) -> Option<String> {
    let columns = entity_json.get("columns")?.as_object()?;
    let entity_name = entity_json
        .get("name")
        .and_then(JsonValue::as_str)
        .unwrap_or_default()
        .trim_start_matches("dim__")
        .trim_start_matches("fct__")
        .trim_start_matches("stg__")
        .trim_end_matches('s')
        .to_ascii_lowercase();
    let mut candidates = columns
        .keys()
        .filter(|column| {
            let lowered = column.to_ascii_lowercase();
            lowered == "id" || lowered == format!("{entity_name}_id") || lowered.ends_with("_id")
        })
        .cloned()
        .collect::<Vec<_>>();
    candidates.sort();
    candidates.first().cloned()
}

fn infer_time_column(entity_json: &JsonValue) -> Option<String> {
    let columns = entity_json.get("columns")?.as_object()?;
    let mut candidates = columns
        .keys()
        .filter(|column| looks_like_time_column(&column.to_ascii_lowercase()))
        .cloned()
        .collect::<Vec<_>>();
    candidates.sort();
    candidates.first().cloned()
}

fn looks_like_time_column(lowered: &str) -> bool {
    lowered == "date"
        || lowered.ends_with("_date")
        || lowered.ends_with("_at")
        || lowered.ends_with("_timestamp")
        || lowered.ends_with("_time")
}

fn indicator_count_in_nova(nova: Option<&JsonValue>) -> usize {
    let Some(nova) = nova else {
        return 0;
    };
    let measures = nova
        .get("measures")
        .and_then(JsonValue::as_array)
        .map_or(0, Vec::len);
    let metrics = nova
        .get("metrics")
        .and_then(JsonValue::as_array)
        .map_or(0, Vec::len);
    let metric = usize::from(nova.get("metric").is_some_and(|value| !value.is_null()));
    measures + metrics + metric
}

fn indicator_meta_base_path(finding: &IndicatorReadinessFinding) -> String {
    match (
        finding.indicator_type.as_str(),
        finding.indicator_name.as_deref(),
    ) {
        ("measure", Some(name)) => format!("meta.nova.measures[name={name}]"),
        ("measure", None) => "meta.nova.measures[]".to_string(),
        ("metric", Some(name)) => format!("meta.nova.metrics[name={name}]"),
        ("metric", None) if finding.resource_type.as_deref() == Some("metric") => {
            "description".to_string()
        }
        ("metric", None) => "meta.nova.metrics[]".to_string(),
        (_, Some(name)) => format!("meta.nova.indicators[name={name}]"),
        _ => "meta.nova.indicators[]".to_string(),
    }
}

fn string_value_present(value: Option<&JsonValue>) -> bool {
    value
        .and_then(JsonValue::as_str)
        .is_some_and(|value| !value.trim().is_empty())
}

fn indicator_seed_query(
    entity: &ArchivedEntity,
    indicator: Option<&serde_json::Map<String, JsonValue>>,
    name: &str,
) -> String {
    let mut tokens = BTreeSet::new();
    insert_query_tokens(&mut tokens, name);
    if let Some(indicator) = indicator {
        insert_query_tokens_from_value(&mut tokens, indicator.get("description"));
        insert_query_tokens_from_array(&mut tokens, indicator.get("synonyms"));
    }
    let entity_json = entity.to_json_value();
    let nova = entity_nova_meta_json(&entity_json);
    let nova = nova.as_deref();
    if let Some(nova) = nova {
        insert_query_tokens_from_array(&mut tokens, nova.get("domains"));
        insert_query_tokens_from_array(&mut tokens, nova.get("use_cases"));
        insert_query_tokens_from_array(&mut tokens, nova.get("synonyms"));
    }
    tokens.into_iter().take(14).collect::<Vec<_>>().join(" ")
}

fn insert_query_tokens(tokens: &mut BTreeSet<String>, value: &str) {
    for token in value
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .map(str::trim)
        .filter(|token| token.len() > 2)
    {
        tokens.insert(token.to_ascii_lowercase());
    }
}

fn insert_query_tokens_from_value(tokens: &mut BTreeSet<String>, value: Option<&JsonValue>) {
    if let Some(value) = value.and_then(JsonValue::as_str) {
        insert_query_tokens(tokens, value);
    }
}

fn insert_query_tokens_from_array(tokens: &mut BTreeSet<String>, value: Option<&JsonValue>) {
    let Some(values) = value.and_then(JsonValue::as_array) else {
        return;
    };
    for value in values.iter().filter_map(JsonValue::as_str) {
        insert_query_tokens(tokens, value);
    }
}

fn stable_meta_patch_id(
    unique_id: &str,
    column_name: Option<&str>,
    indicator_type: Option<&str>,
    indicator_name: Option<&str>,
    field_path: &str,
) -> String {
    [
        "meta_patch",
        unique_id,
        column_name.unwrap_or("-"),
        indicator_type.unwrap_or("-"),
        indicator_name.unwrap_or("-"),
        field_path,
    ]
    .into_iter()
    .map(stable_id_fragment)
    .collect::<Vec<_>>()
    .join("::")
}

fn stable_seed_id(
    seed_type: &str,
    unique_id: &str,
    indicator_type: Option<&str>,
    indicator_name: Option<&str>,
) -> String {
    [
        "golden_seed",
        seed_type,
        unique_id,
        indicator_type.unwrap_or("-"),
        indicator_name.unwrap_or("-"),
    ]
    .into_iter()
    .map(stable_id_fragment)
    .collect::<Vec<_>>()
    .join("::")
}

fn stable_id_fragment(value: &str) -> String {
    let mut out = String::new();
    let mut previous_underscore = false;
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            previous_underscore = false;
        } else if !previous_underscore {
            out.push('_');
            previous_underscore = true;
        }
    }
    let trimmed = out.trim_matches('_');
    if trimmed.is_empty() {
        "none".to_string()
    } else {
        trimmed.to_string()
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

fn build_next_actions(input: &NextActionInput<'_>) -> Vec<ReadinessNextAction> {
    let mut actions = Vec::new();
    if !input.blocking_findings.is_empty() {
        actions.push(ReadinessNextAction {
            priority: 1,
            category: "blockers",
            action:
                "Resolve blocking readiness findings before treating this manifest as launch-ready"
                    .to_string(),
            evidence: format!("{} blocker(s) detected", input.blocking_findings.len()),
        });
    }
    if input.eval_status.status == "not_supplied" {
        actions.push(ReadinessNextAction {
            priority: 2,
            category: "eval_gate",
            action: "Run dbt-nova eval gate <SUITE> --json and pass the result with --eval-gate-file or --eval-gate-json".to_string(),
            evidence: "no eval gate report supplied".to_string(),
        });
    } else if input.eval_status.status == "blocked" {
        actions.push(ReadinessNextAction {
            priority: 1,
            category: "eval_gate",
            action: "Rerun or fix the blocked eval suite before relying on agent workflows"
                .to_string(),
            evidence: input.eval_status.message.clone(),
        });
    } else if input.eval_status.status == "unavailable" {
        actions.push(ReadinessNextAction {
            priority: 2,
            category: "eval_gate",
            action: "Provide a valid dbt-nova eval gate JSON report before treating eval evidence as current".to_string(),
            evidence: input.eval_status.message.clone(),
        });
    }
    if input.ambiguous_indicator_count > 0 {
        actions.push(ReadinessNextAction {
            priority: 2,
            category: "indicator_metadata",
            action:
                "Add explicit expression, field, and grain metadata to ambiguous Nova indicators"
                    .to_string(),
            evidence: format!(
                "{} ambiguous indicator definition(s)",
                input.ambiguous_indicator_count
            ),
        });
    }
    actions.extend(input.modelling_next_actions.iter().cloned());
    if input.suggested_meta_patch_count > 0 {
        actions.push(ReadinessNextAction {
            priority: 2,
            category: "remediation",
            action: "Review suggested_meta_patches and promote safe changes into dbt YAML"
                .to_string(),
            evidence: format!(
                "{} advisory patch suggestion(s)",
                input.suggested_meta_patch_count
            ),
        });
    }
    if input.golden_question_seed_count > 0 {
        actions.push(ReadinessNextAction {
            priority: 3,
            category: "eval_seed",
            action: "Review golden_question_seeds and copy approved cases into an eval suite"
                .to_string(),
            evidence: format!("{} draft eval seed(s)", input.golden_question_seed_count),
        });
    }
    if input.overall_score < 85 || !input.improvement_findings.is_empty() {
        actions.push(ReadinessNextAction {
            priority: 3,
            category: "metadata_quality",
            action:
                "Work through the lowest-scoring entity findings, then rerun the readiness report"
                    .to_string(),
            evidence: format!(
                "overall score {overall_score}; {} improvement finding(s)",
                input.improvement_findings.len(),
                overall_score = input.overall_score
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
    append_markdown_summary(&mut out, report);
    append_scoring_contract_summary(&mut out, report);
    append_readiness_triage_summary(&mut out, report);
    append_persona_score_table(&mut out, report);
    append_findings_table(&mut out, "Blockers", &report.blocking_findings);
    append_findings_table(&mut out, "Improvements", &report.improvement_findings);
    append_eval_status(&mut out, report);
    append_entity_findings_table(&mut out, &report.entity_findings);
    append_indicator_findings_table(&mut out, &report.indicator_findings);
    append_suggested_meta_patches_table(&mut out, &report.suggested_meta_patches);
    append_golden_question_seeds_table(&mut out, &report.golden_question_seeds);
    append_next_actions(&mut out, &report.next_actions);
    out
}

fn append_scoring_contract_summary(out: &mut String, report: &AgentReadinessReport) {
    if report.gate_status == "pass" {
        return;
    }
    out.push_str("## Scoring Contract\n\n");
    out.push_str("- grade_bands: `A >= 90`, `B >= 80`, `C >= 70`, `D >= 60`, `F < 60`\n");
    out.push_str("- description_tiers: `50-99 chars = 80%`, `100+ chars = full credit`\n");
    out.push_str("- array_tiers: `1 item = 40%`, `2 items = 70%`, `3+ items = full credit`\n\n");
}

fn append_readiness_triage_summary(out: &mut String, report: &AgentReadinessReport) {
    let weak_spots = report
        .summary
        .category_weak_spots
        .as_array()
        .map_or(&[][..], Vec::as_slice);
    let repeated_fields = report
        .summary
        .top_recommendation_fields
        .as_array()
        .map_or(&[][..], Vec::as_slice);
    if weak_spots.is_empty() && repeated_fields.is_empty() {
        return;
    }

    out.push_str("## Triage Summary\n\n");
    for weak_spot in weak_spots.iter().take(5) {
        let persona = weak_spot
            .get("persona")
            .and_then(JsonValue::as_str)
            .unwrap_or("project");
        let category = weak_spot
            .get("category")
            .and_then(JsonValue::as_str)
            .unwrap_or("metadata");
        let average_score = weak_spot
            .get("average_score")
            .and_then(JsonValue::as_u64)
            .unwrap_or(0);
        let gap = weak_spot
            .get("estimated_point_gap")
            .and_then(JsonValue::as_u64)
            .unwrap_or(0);
        let _ = writeln!(
            out,
            "- `{persona}` `{category}` average score `{average_score}`, estimated point gap `{gap}`"
        );
    }
    for field in repeated_fields.iter().take(5) {
        let field_name = field
            .get("field")
            .and_then(JsonValue::as_str)
            .unwrap_or("metadata");
        let count = field.get("count").and_then(JsonValue::as_u64).unwrap_or(0);
        let impact = field
            .get("estimated_point_impact")
            .and_then(JsonValue::as_u64)
            .unwrap_or(0);
        let _ = writeln!(
            out,
            "- repeated field `{field_name}` appears `{count}` time(s), estimated point impact `{impact}`"
        );
    }
    out.push('\n');
}

fn append_markdown_summary(out: &mut String, report: &AgentReadinessReport) {
    let _ = write!(
        out,
        "- gate_status: `{}`\n- readiness_band: `{}`\n- overall_score: `{}` ({})\n- target_count: `{}`\n- blockers: `{}`\n- improvements: `{}`\n- suggested_meta_patches: `{}`\n- golden_question_seeds: `{}`\n\n",
        report.gate_status,
        report.readiness_band,
        report.overall_score,
        report.grade,
        report.summary.target_count,
        report.summary.blocker_count,
        report.summary.improvement_count,
        report.summary.suggested_meta_patch_count,
        report.summary.golden_question_seed_count
    );
}

fn append_persona_score_table(out: &mut String, report: &AgentReadinessReport) {
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
}

fn append_eval_status(out: &mut String, report: &AgentReadinessReport) {
    out.push_str("## Eval Status\n\n");
    let _ = writeln!(
        out,
        "- status: `{}`\n- supplied: `{}`\n- message: {}\n",
        report.eval_status.status, report.eval_status.supplied, report.eval_status.message
    );
}

fn append_entity_findings_table(out: &mut String, entity_findings: &[EntityReadinessFinding]) {
    if !entity_findings.is_empty() {
        out.push_str("## Entity Findings\n\n");
        out.push_str("| Entity | Type | Score | Signal gaps | Top recommendation |\n");
        out.push_str("|---|---|---:|---:|---|\n");
        for entity in entity_findings {
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
}

fn append_indicator_findings_table(
    out: &mut String,
    indicator_findings: &[IndicatorReadinessFinding],
) {
    if !indicator_findings.is_empty() {
        out.push_str("## Indicator Findings\n\n");
        out.push_str("| Entity | Indicator | Type | Issue |\n");
        out.push_str("|---|---|---|---|\n");
        for finding in indicator_findings {
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
}

fn append_suggested_meta_patches_table(out: &mut String, patches: &[SuggestedMetaPatch]) {
    if !patches.is_empty() {
        out.push_str("## Suggested Meta Patches\n\n");
        out.push_str("| Target | Field | Suggested value | Rationale |\n");
        out.push_str("|---|---|---|---|\n");
        for patch in patches.iter().take(MAX_MARKDOWN_META_PATCHES) {
            let target = patch
                .column_name
                .as_deref()
                .or(patch.indicator_name.as_deref())
                .unwrap_or(patch.unique_id.as_str());
            let _ = writeln!(
                out,
                "| `{}` | `{}` | `{}` | {} |",
                target,
                patch.field_path,
                escape_markdown_table_cell(&json_inline(&patch.suggested_value)),
                escape_markdown_table_cell(&patch.rationale)
            );
        }
        if patches.len() > MAX_MARKDOWN_META_PATCHES {
            let _ = writeln!(
                out,
                "| `_truncated` | `-` | `-` | {} additional suggestion(s) omitted from Markdown; see JSON report. |",
                patches.len() - MAX_MARKDOWN_META_PATCHES
            );
        }
        out.push('\n');
    }
}

fn append_golden_question_seeds_table(out: &mut String, seeds: &[GoldenQuestionSeed]) {
    if !seeds.is_empty() {
        out.push_str("## Golden Question Seeds\n\n");
        out.push_str("| Type | Persona | Question | Suggested assertion |\n");
        out.push_str("|---|---|---|---|\n");
        for seed in seeds.iter().take(MAX_MARKDOWN_GOLDEN_SEEDS) {
            let assertion = seed
                .recommended_assertions
                .first()
                .map_or_else(|| "-".to_string(), json_inline);
            let _ = writeln!(
                out,
                "| `{}` | `{}` | {} | `{}` |",
                seed.seed_type,
                seed.persona,
                escape_markdown_table_cell(&seed.question),
                escape_markdown_table_cell(&assertion)
            );
        }
        if seeds.len() > MAX_MARKDOWN_GOLDEN_SEEDS {
            let _ = writeln!(
                out,
                "| `_truncated` | `-` | {} additional seed(s) omitted from Markdown; see JSON report. | `-` |",
                seeds.len() - MAX_MARKDOWN_GOLDEN_SEEDS
            );
        }
        out.push('\n');
    }
}

fn append_next_actions(out: &mut String, next_actions: &[ReadinessNextAction]) {
    out.push_str("## Next Actions\n\n");
    for (index, action) in next_actions.iter().enumerate() {
        let _ = writeln!(
            out,
            "{}. **{}**: {} ({})",
            index + 1,
            title_case(action.category),
            action.action,
            action.evidence
        );
    }
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

fn json_inline(value: &JsonValue) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "null".to_string())
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::Path;

    use serde_json::json;
    use tempfile::TempDir;

    use super::{
        EvalReadinessStatus, IndicatorReadinessFinding, ReadinessFinding, ReadinessThresholdConfig,
        ThresholdRule, ThresholdSeverity, append_indicator_meta_patches,
        build_agent_readiness_load_config, build_agent_readiness_report,
        build_agent_readiness_tool_response, evaluate_threshold, parse_eval_status,
        parse_json_input, render_markdown_report, write_report,
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
    async fn agent_readiness_includes_agent_modelling_summary_and_findings() {
        let report = readiness_report_for_fixture(
            "agent_modelling_findings.json",
            "agent-readiness-modeling",
        )
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
            readiness_report_for_fixture("minimal.json", "agent-readiness-suggestions-minimal")
                .await;

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
}
