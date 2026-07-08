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

mod patches;
mod render;
mod seeds;
mod types;

use patches::build_suggested_meta_patches;
use render::{
    append_entity_findings_table, append_eval_status, append_findings_table,
    append_golden_question_seeds_table, append_indicator_findings_table, append_markdown_summary,
    append_next_actions, append_persona_score_table, append_readiness_triage_summary,
    append_suggested_meta_patches_table, average_score, current_timestamp_ms, json_string,
    json_string_array, json_u8, json_usize_optional, parse_json_array_input, parse_json_input,
    print_human_summary, raw_optional_input, render_or_propagate_error, write_report,
};
use seeds::{
    MetaPatchContent, MetaPatchTarget, build_golden_question_seeds, entity_patch,
    indicator_count_in_nova, indicator_meta_base_path, infer_primary_key_column, infer_time_column,
    json_array_field_non_empty, json_bool_field_present, looks_like_time_column, push_meta_patch,
};
use types::{
    AgentModellingReadinessResult, AgentReadinessReport, AppliedCountThreshold, AppliedThreshold,
    CountThresholdRule, EntityReadinessFinding, EntityReadinessSignals, EntityScoreAccumulator,
    EntityScoreEvidence, EvalReadinessStatus, GoldenQuestionSeed, IndicatorReadinessFinding,
    ModellingThresholdConfig, NextActionInput, PersonaReadinessScore, ReadinessConfigSummary,
    ReadinessFinalSectionInput, ReadinessFinalSections, ReadinessFinding, ReadinessInputs,
    ReadinessManifestSummary, ReadinessNextAction, ReadinessRecommendation, ReadinessSearchReady,
    ReadinessSummary, ReadinessSummaryInput, ReadinessThresholdConfig, SuggestedMetaPatch,
    ThresholdRule, ThresholdSeverity,
};

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

#[cfg(test)]
mod tests;
