use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::Path;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::cli::args::{MetadataAuditArgs, MetadataAuditSelectionModeArg};
use crate::cli::manifest::{build_manifest_load_config, execute_manifest_load};
use crate::cli::output::CliEnvelope;
use crate::error::{DbtNovaError, Result};
use crate::manifest::entity::ArchivedEntity;
use crate::manifest::search::ManifestSearch;
use crate::params::GetMetadataScoreParams;
use crate::utils::SearchPersona;

use super::{DispatchError, DispatchResult};

const DEFAULT_RESOURCE_TYPES_JSON: &str = "[\"model\"]";
const DEFAULT_PERSONAS_JSON: &str = "[\"engineer\"]";
const DEFAULT_THRESHOLDS_JSON: &str = r#"{
  "entity": {
    "engineer": { "min_score": 70, "severity": "required" },
    "analyst": { "min_score": 65, "severity": "advisory" },
    "governance": { "min_score": 65, "severity": "advisory" }
  }
}"#;

#[derive(Debug, Clone, Serialize)]
struct MetadataAuditReport {
    selection_mode: String,
    manifest_hash: String,
    manifest_version: String,
    manifest_source: String,
    resource_types: Vec<String>,
    personas: Vec<String>,
    changed_files: Vec<String>,
    selected_entity_ids: Vec<String>,
    target_count: usize,
    scored_count: usize,
    gate_status: &'static str,
    summary: AuditSummary,
    project_summary: Option<BTreeMap<String, ProjectAuditReport>>,
    entities: Vec<EntityAuditReport>,
}

#[derive(Debug, Clone, Serialize)]
struct AuditSummary {
    required_fail_count: usize,
    advisory_fail_count: usize,
    pass_count: usize,
    no_target: bool,
    no_target_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct EntityAuditReport {
    unique_id: String,
    name: Option<String>,
    resource_type: Option<String>,
    original_file_path: Option<String>,
    patch_path: Option<String>,
    gate_status: &'static str,
    personas: BTreeMap<String, PersonaAuditReport>,
}

#[derive(Debug, Clone, Serialize)]
struct PersonaAuditReport {
    overall_score: u8,
    grade: String,
    categories: JsonValue,
    breakdown: JsonValue,
    recommendations: JsonValue,
    threshold: Option<AppliedThreshold>,
    gate_status: &'static str,
}

#[derive(Debug, Clone, Serialize)]
struct ProjectAuditReport {
    overall_score: u8,
    grade: String,
    quality_summary: JsonValue,
    threshold: Option<AppliedThreshold>,
    gate_status: &'static str,
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
    #[default]
    Required,
    Advisory,
}

#[derive(Debug, Clone, Deserialize)]
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
            severity: ThresholdSeverity::Required,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
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

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
struct AuditThresholdConfig {
    entity: PersonaThresholdConfig,
    project: PersonaThresholdConfig,
}

#[derive(Debug, Clone)]
struct AuditInputs {
    selection_mode: MetadataAuditSelectionModeArg,
    resource_types: Vec<String>,
    personas: Vec<String>,
    changed_files: Vec<String>,
    explicit_entities: Vec<String>,
    thresholds: AuditThresholdConfig,
}

/// Runs the `audit metadata-score` CLI command.
///
/// # Errors
/// Returns an error if input validation fails, manifest loading fails, or report serialization fails.
pub async fn run_metadata_score_command(args: &MetadataAuditArgs) -> DispatchResult {
    let started = Instant::now();
    let audit_inputs = parse_audit_inputs(args).map_err(|error| DispatchError {
        error,
        rendered: false,
    })?;
    let load_args = crate::cli::args::ManifestLoadArgs {
        manifest_path: args.manifest_path.clone(),
        manifest_uri: args.manifest_uri.clone(),
        storage_instance_id: args.storage_instance_id.clone(),
        cleanup_storage_on_start: args.cleanup_storage_on_start,
        read_only: args.read_only,
        json: false,
    };
    let config = build_manifest_load_config(&load_args).map_err(Into::<DispatchError>::into)?;
    let load_result = execute_manifest_load(config)
        .await
        .map_err(Into::<DispatchError>::into)?;

    let report = build_metadata_audit_report(&load_result.search, &audit_inputs, args).await?;

    if let Some(path) = args.report_json_path.as_deref() {
        let serialized = serde_json::to_string_pretty(&report)
            .map_err(|error| DbtNovaError::ServerError(error.to_string()))?;
        write_report(path, &serialized)?;
    }
    let markdown = render_markdown_report(&report);
    if let Some(path) = args.report_md_path.as_deref() {
        write_report(path, &markdown)?;
    }

    if args.json {
        let envelope = CliEnvelope::success(
            "audit metadata-score",
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

    if report.gate_status == "fail" {
        return Err(DispatchError {
            error: DbtNovaError::ServerError(format!(
                "metadata audit gate failed ({} required failure(s))",
                report.summary.required_fail_count
            )),
            rendered: true,
        });
    }

    Ok(())
}

fn parse_audit_inputs(args: &MetadataAuditArgs) -> Result<AuditInputs> {
    let resource_types: Vec<String> = parse_json_array_input(
        args.resource_types_json.as_deref(),
        None,
        DEFAULT_RESOURCE_TYPES_JSON,
    )?
    .into_iter()
    .map(|value| value.trim().to_lowercase())
    .filter(|value| !value.is_empty())
    .collect();
    if resource_types.is_empty() {
        return Err(DbtNovaError::InvalidParams(
            "resource_types_json must contain at least one resource type".to_string(),
        ));
    }

    let personas: Vec<String> =
        parse_json_array_input(args.personas_json.as_deref(), None, DEFAULT_PERSONAS_JSON)?
            .into_iter()
            .map(|value| value.trim().to_lowercase())
            .filter(|value| !value.is_empty())
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

    let changed_files = parse_json_array_input(
        args.changed_files_json.as_deref(),
        args.changed_files_file.as_deref(),
        "[]",
    )?
    .into_iter()
    .map(|path| normalize_path(&path))
    .filter(|path| !path.is_empty())
    .collect::<Vec<_>>();

    let explicit_entities = parse_json_array_input(
        args.entity_ids_json.as_deref(),
        args.entity_ids_file.as_deref(),
        "[]",
    )?;

    match args.selection_mode {
        MetadataAuditSelectionModeArg::Changed if changed_files.is_empty() => {
            return Err(DbtNovaError::InvalidParams(
                "selection_mode=changed requires changed_files_json or changed_files_file"
                    .to_string(),
            ));
        }
        MetadataAuditSelectionModeArg::Entities if explicit_entities.is_empty() => {
            return Err(DbtNovaError::InvalidParams(
                "selection_mode=entities requires entity_ids_json or entity_ids_file".to_string(),
            ));
        }
        _ => {}
    }

    let thresholds: AuditThresholdConfig = parse_json_input(
        args.thresholds_json.as_deref(),
        args.thresholds_file.as_deref(),
        DEFAULT_THRESHOLDS_JSON,
    )?;

    Ok(AuditInputs {
        selection_mode: args.selection_mode,
        resource_types,
        personas,
        changed_files,
        explicit_entities,
        thresholds,
    })
}

async fn build_metadata_audit_report(
    search: &ManifestSearch,
    inputs: &AuditInputs,
    args: &MetadataAuditArgs,
) -> Result<MetadataAuditReport> {
    let selected_entity_ids = select_entity_ids(search, inputs)?;
    let mut entities = Vec::with_capacity(selected_entity_ids.len());
    let mut required_fail_count = 0usize;
    let mut advisory_fail_count = 0usize;
    let mut pass_count = 0usize;
    let no_target_reason = no_target_reason(inputs, args.fail_on_no_targets, &selected_entity_ids);

    for unique_id in &selected_entity_ids {
        let entity = search
            .get_entity_archived(unique_id)?
            .ok_or_else(|| search.entity_not_found(unique_id, None))?;
        let report = score_entity(search, entity, unique_id, inputs, args).await?;
        match report.gate_status {
            "fail" => required_fail_count += 1,
            "advisory" => advisory_fail_count += 1,
            _ => pass_count += 1,
        }
        entities.push(report);
    }

    let project_summary = if inputs.selection_mode == MetadataAuditSelectionModeArg::Project {
        let project_summary =
            score_project_summary(search, inputs, args, selected_entity_ids.len()).await?;
        required_fail_count = project_summary
            .values()
            .filter(|report| report.gate_status == "required_fail")
            .count();
        advisory_fail_count = project_summary
            .values()
            .filter(|report| report.gate_status == "advisory_fail")
            .count();
        pass_count = project_summary
            .values()
            .filter(|report| report.gate_status == "pass")
            .count();
        Some(project_summary)
    } else {
        None
    };

    if selected_entity_ids.is_empty() {
        if args.fail_on_no_targets {
            required_fail_count = 1;
        } else if changed_files_include_dbt_model_or_schema_paths(&inputs.changed_files) {
            advisory_fail_count = 1;
        }
    }

    let gate_status = match (required_fail_count, advisory_fail_count) {
        (required, _) if required > 0 => "fail",
        (0, advisory) if advisory > 0 => "advisory",
        _ => "pass",
    };

    Ok(MetadataAuditReport {
        selection_mode: selection_mode_label(inputs.selection_mode).to_string(),
        manifest_hash: search.manifest_hash.clone(),
        manifest_version: search.manifest_version.clone(),
        manifest_source: search.manifest_source_uri.clone(),
        resource_types: inputs.resource_types.clone(),
        personas: inputs.personas.clone(),
        changed_files: inputs.changed_files.clone(),
        selected_entity_ids: selected_entity_ids.clone(),
        target_count: selected_entity_ids.len(),
        scored_count: entities.len(),
        gate_status,
        summary: AuditSummary {
            required_fail_count,
            advisory_fail_count,
            pass_count,
            no_target: selected_entity_ids.is_empty(),
            no_target_reason,
        },
        project_summary,
        entities,
    })
}

async fn score_entity(
    search: &ManifestSearch,
    entity: &ArchivedEntity,
    unique_id: &str,
    inputs: &AuditInputs,
    args: &MetadataAuditArgs,
) -> Result<EntityAuditReport> {
    let mut personas = BTreeMap::new();
    let mut required_fail = false;
    let mut advisory_fail = false;
    let resource_type = entity.resource_type_str().map(ToString::to_string);
    let threshold_profile = inputs.threshold_profile();

    for persona in &inputs.personas {
        let params = GetMetadataScoreParams {
            id_or_name: Some(unique_id.to_string()),
            resource_type: resource_type.clone(),
            persona: Some(persona.clone()),
            scope: Some("entity".to_string()),
            include_breakdown: args.include_breakdown.unwrap_or(true),
            include_recommendations: args.include_recommendations.unwrap_or(true),
            ..GetMetadataScoreParams::default()
        };
        let result = search.get_metadata_score(&params).await?;
        let data = result.get("data").cloned().ok_or_else(|| {
            DbtNovaError::ServerError("metadata score response missing data".to_string())
        })?;
        let overall_score = json_u8(&data, "overall_score")?;
        let grade = json_string(&data, "grade")?;
        let threshold = if inputs.selection_mode == MetadataAuditSelectionModeArg::Project {
            None
        } else {
            threshold_profile.rule_for(persona).cloned()
        };
        let persona_gate = evaluate_threshold(overall_score, &grade, threshold.as_ref());
        match persona_gate {
            "required_fail" => required_fail = true,
            "advisory_fail" => advisory_fail = true,
            _ => {}
        }
        personas.insert(
            persona.clone(),
            PersonaAuditReport {
                overall_score,
                grade,
                categories: data.get("categories").cloned().unwrap_or(JsonValue::Null),
                breakdown: data.get("breakdown").cloned().unwrap_or(JsonValue::Null),
                recommendations: data
                    .get("recommendations")
                    .cloned()
                    .unwrap_or(JsonValue::Array(Vec::new())),
                threshold: threshold.map(|rule| AppliedThreshold {
                    min_score: rule.min_score,
                    min_grade: rule.min_grade,
                    severity: rule.severity,
                }),
                gate_status: match persona_gate {
                    "required_fail" | "advisory_fail" => persona_gate,
                    _ => "pass",
                },
            },
        );
    }

    let gate_status = if required_fail {
        "fail"
    } else if advisory_fail {
        "advisory"
    } else {
        "pass"
    };

    Ok(EntityAuditReport {
        unique_id: unique_id.to_string(),
        name: entity.name_str().map(ToString::to_string),
        resource_type,
        original_file_path: entity.original_file_path_str().map(ToString::to_string),
        patch_path: patch_path_from_entity(entity),
        gate_status,
        personas,
    })
}

async fn score_project_summary(
    search: &ManifestSearch,
    inputs: &AuditInputs,
    args: &MetadataAuditArgs,
    total_targets: usize,
) -> Result<BTreeMap<String, ProjectAuditReport>> {
    let mut summary = BTreeMap::new();
    for persona in &inputs.personas {
        let params = GetMetadataScoreParams {
            persona: Some(persona.clone()),
            scope: Some("project".to_string()),
            include_breakdown: args.include_breakdown.unwrap_or(true),
            include_recommendations: args.include_recommendations.unwrap_or(true),
            resource_types: inputs.resource_types.clone(),
            limit: Some(total_targets),
            offset: Some(0),
            ..GetMetadataScoreParams::default()
        };
        let result = search.get_metadata_score(&params).await?;
        let data = result.get("data").cloned().ok_or_else(|| {
            DbtNovaError::ServerError("metadata score response missing data".to_string())
        })?;
        let overall_score = json_u8(&data, "overall_score")?;
        let grade = json_string(&data, "grade")?;
        let threshold = inputs.thresholds.project.rule_for(persona).cloned();
        let gate_status = evaluate_threshold(overall_score, &grade, threshold.as_ref());
        summary.insert(
            persona.clone(),
            ProjectAuditReport {
                overall_score,
                grade,
                quality_summary: data
                    .get("quality_summary")
                    .cloned()
                    .unwrap_or(JsonValue::Null),
                threshold: threshold.map(|rule| AppliedThreshold {
                    min_score: rule.min_score,
                    min_grade: rule.min_grade,
                    severity: rule.severity,
                }),
                gate_status,
            },
        );
    }
    Ok(summary)
}

fn select_entity_ids(search: &ManifestSearch, inputs: &AuditInputs) -> Result<Vec<String>> {
    let mut selected = BTreeSet::new();
    match inputs.selection_mode {
        MetadataAuditSelectionModeArg::Project => {
            for resource_type in normalized_resource_types(search, &inputs.resource_types)? {
                if let Some(ids) = search.by_resource_type.get(&resource_type) {
                    for id in ids {
                        selected.insert(id.clone());
                    }
                }
            }
        }
        MetadataAuditSelectionModeArg::Changed => {
            let changed: BTreeSet<String> = inputs.changed_files.iter().cloned().collect();
            for resource_type in normalized_resource_types(search, &inputs.resource_types)? {
                if let Some(ids) = search.by_resource_type.get(&resource_type) {
                    for id in ids {
                        if let Some(entity) = search.get_entity_archived(id)?
                            && entity_matches_changed_files(entity, &changed)
                        {
                            selected.insert(id.clone());
                        }
                    }
                }
            }
        }
        MetadataAuditSelectionModeArg::Entities => {
            for id_or_name in &inputs.explicit_entities {
                selected.insert(search.resolve_single_id(id_or_name, None)?);
            }
        }
    }
    Ok(selected.into_iter().collect())
}

fn normalized_resource_types(
    search: &ManifestSearch,
    resource_types: &[String],
) -> Result<Vec<String>> {
    let mut normalized = Vec::new();
    for resource_type in resource_types {
        normalized.push(search.normalize_resource_type_key(resource_type)?);
    }
    normalized.sort();
    normalized.dedup();
    Ok(normalized)
}

fn entity_matches_changed_files(entity: &ArchivedEntity, changed_files: &BTreeSet<String>) -> bool {
    entity
        .original_file_path_str()
        .map(normalize_path)
        .is_some_and(|path| changed_files.contains(&path))
        || patch_path_from_entity(entity)
            .as_deref()
            .map(normalize_path)
            .is_some_and(|path| changed_files.contains(&path))
}

fn no_target_reason(
    inputs: &AuditInputs,
    fail_on_no_targets: bool,
    selected_entity_ids: &[String],
) -> Option<String> {
    if !selected_entity_ids.is_empty() {
        return None;
    }
    if inputs.selection_mode == MetadataAuditSelectionModeArg::Changed
        && changed_files_include_dbt_model_or_schema_paths(&inputs.changed_files)
    {
        return Some(
            "selection_mode=changed found dbt model/schema-looking changed files, but none matched manifest original_file_path or patch_path"
                .to_string(),
        );
    }
    fail_on_no_targets.then(|| "metadata audit selected no entities".to_string())
}

fn changed_files_include_dbt_model_or_schema_paths(changed_files: &[String]) -> bool {
    changed_files
        .iter()
        .map(|path| normalize_path(path))
        .any(|path| {
            let extension = Path::new(&path)
                .extension()
                .and_then(|extension| extension.to_str())
                .map(str::to_ascii_lowercase);
            let is_model_path = path.starts_with("models/");
            let is_sql_model = is_model_path && extension.as_deref() == Some("sql");
            let is_yaml_patch =
                is_model_path && matches!(extension.as_deref(), Some("yml" | "yaml"));
            is_sql_model || is_yaml_patch
        })
}

fn patch_path_from_entity(entity: &ArchivedEntity) -> Option<String> {
    entity
        .to_json_value()
        .get("patch_path")
        .and_then(JsonValue::as_str)
        .map(ToString::to_string)
}

fn normalize_path(path: &str) -> String {
    path.trim().trim_start_matches("./").replace('\\', "/")
}

fn selection_mode_label(mode: MetadataAuditSelectionModeArg) -> &'static str {
    match mode {
        MetadataAuditSelectionModeArg::Project => "project",
        MetadataAuditSelectionModeArg::Changed => "changed",
        MetadataAuditSelectionModeArg::Entities => "entities",
    }
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

fn render_markdown_report(report: &MetadataAuditReport) -> String {
    let mut out = String::new();
    out.push_str("# Nova Metadata Audit\n\n");
    let _ = write!(
        out,
        "- gate_status: `{}`\n- selection_mode: `{}`\n- target_count: `{}`\n- required_fail_count: `{}`\n- advisory_fail_count: `{}`\n\n",
        report.gate_status,
        report.selection_mode,
        report.target_count,
        report.summary.required_fail_count,
        report.summary.advisory_fail_count
    );

    if report.summary.no_target {
        append_no_target_markdown(&mut out, report);
        return out;
    }

    if let Some(project_summary) = &report.project_summary {
        out.push_str("## Project Summary\n\n");
        out.push_str("| Persona | Score | Grade | Gate |\n");
        out.push_str("|---|---:|---|---|\n");
        for persona in &report.personas {
            if let Some(score) = project_summary.get(persona) {
                let _ = writeln!(
                    out,
                    "| {} | {} | {} | `{}` |",
                    title_case(persona),
                    score.overall_score,
                    score.grade,
                    score.gate_status
                );
            }
        }
        out.push('\n');
    }

    out.push_str("| Entity | Type | Gate |");
    for persona in &report.personas {
        let _ = write!(out, " {} |", title_case(persona));
    }
    out.push('\n');
    out.push_str("|---|---|---|");
    for _ in &report.personas {
        out.push_str("---|");
    }
    out.push('\n');

    for entity in &report.entities {
        let _ = write!(
            out,
            "| `{}` | `{}` | `{}` |",
            entity.unique_id,
            entity.resource_type.clone().unwrap_or_default(),
            entity.gate_status
        );
        for persona in &report.personas {
            let cell = entity.personas.get(persona).map_or_else(
                || "-".to_string(),
                |score| format!("{} ({})", score.overall_score, score.grade),
            );
            let _ = write!(out, " {cell} |");
        }
        out.push('\n');
    }

    let failing = report
        .entities
        .iter()
        .filter(|entity| entity.gate_status != "pass")
        .collect::<Vec<_>>();
    if !failing.is_empty() {
        out.push_str("\n## Findings\n\n");
        for entity in failing {
            let _ = writeln!(out, "- `{}` `{}`", entity.unique_id, entity.gate_status);
            for persona in &report.personas {
                let Some(score) = entity.personas.get(persona) else {
                    continue;
                };
                if score.gate_status == "pass" {
                    continue;
                }
                let _ = writeln!(
                    out,
                    "  - {}: {} ({})",
                    title_case(persona),
                    score.overall_score,
                    score.grade
                );
                if let Some(recommendations) = score.recommendations.as_array() {
                    for recommendation in recommendations.iter().take(3) {
                        if let Some(field) = recommendation.get("field").and_then(JsonValue::as_str)
                        {
                            let category = recommendation
                                .get("category")
                                .and_then(JsonValue::as_str)
                                .unwrap_or("metadata");
                            let _ = writeln!(out, "    - {category}: `{field}`");
                        }
                    }
                }
            }
        }
    }

    out
}

fn append_no_target_markdown(out: &mut String, report: &MetadataAuditReport) {
    out.push_str("No entities matched the selection.\n");
    if let Some(reason) = &report.summary.no_target_reason {
        let _ = writeln!(out, "Reason: {reason}");
    }
}

fn title_case(value: &str) -> String {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

fn print_human_summary(report: &MetadataAuditReport) {
    println!("metadata audit complete");
    println!("  gate_status: {}", report.gate_status);
    println!("  selection_mode: {}", report.selection_mode);
    println!("  target_count: {}", report.target_count);
    println!(
        "  required_fail_count: {}",
        report.summary.required_fail_count
    );
    println!(
        "  advisory_fail_count: {}",
        report.summary.advisory_fail_count
    );
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

fn parse_json_input<T>(inline: Option<&str>, path: Option<&str>, default_inline: &str) -> Result<T>
where
    T: for<'de> Deserialize<'de>,
{
    let raw = if let Some(inline) = inline {
        inline.to_string()
    } else if let Some(path) = path {
        fs::read_to_string(path).map_err(|error| {
            DbtNovaError::InvalidParams(format!("failed to read {path}: {error}"))
        })?
    } else {
        default_inline.to_string()
    };

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

impl AuditInputs {
    fn threshold_profile(&self) -> &PersonaThresholdConfig {
        match self.selection_mode {
            MetadataAuditSelectionModeArg::Project => &self.thresholds.project,
            MetadataAuditSelectionModeArg::Changed | MetadataAuditSelectionModeArg::Entities => {
                &self.thresholds.entity
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AuditThresholdConfig, MetadataAuditReport, ThresholdSeverity, entity_matches_changed_files,
        evaluate_threshold, normalize_path, parse_json_input, render_markdown_report,
    };
    use crate::cli::args::{MetadataAuditArgs, MetadataAuditSelectionModeArg};
    use crate::cli::manifest::{build_manifest_load_config, execute_manifest_load};
    use crate::tests::common::fixture_manifest_path_string;
    use serde_json::Value as JsonValue;
    use serde_json::json;
    use tempfile::TempDir;

    #[test]
    fn normalize_path_strips_dot_prefix_and_backslashes() {
        assert_eq!(
            normalize_path("./models\\staging\\orders.sql"),
            "models/staging/orders.sql"
        );
    }

    #[test]
    fn parse_threshold_defaults_loads_expected_rule() {
        let thresholds: AuditThresholdConfig =
            parse_json_input(None, None, super::DEFAULT_THRESHOLDS_JSON).expect("thresholds");
        let engineer = thresholds.entity.engineer.expect("engineer threshold");
        assert_eq!(engineer.min_score, Some(70));
        assert_eq!(engineer.severity, ThresholdSeverity::Required);
    }

    #[test]
    fn evaluate_threshold_distinguishes_required_and_advisory_failures() {
        let required = super::ThresholdRule {
            min_score: Some(80),
            min_grade: Some("B".to_string()),
            severity: ThresholdSeverity::Required,
        };
        let advisory = super::ThresholdRule {
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
        assert_eq!(evaluate_threshold(90, "A", Some(&required)), "pass");
    }

    #[tokio::test]
    async fn changed_selection_matches_original_file_path() {
        let temp_dir = TempDir::new().expect("temp dir");
        let args = MetadataAuditArgs {
            selection_mode: MetadataAuditSelectionModeArg::Changed,
            changed_files_json: Some(
                "[\"models/staging/traffic/stg__traffic_sessions.sql\"]".to_string(),
            ),
            manifest_path: Some(fixture_manifest_path_string()),
            storage_instance_id: Some("audit-changed-selection-test".to_string()),
            cleanup_storage_on_start: true,
            ..MetadataAuditArgs::default()
        };
        let inputs = super::parse_audit_inputs(&args).expect("audit inputs");
        let load_args = crate::cli::args::ManifestLoadArgs {
            manifest_path: args.manifest_path.clone(),
            storage_instance_id: args.storage_instance_id.clone(),
            cleanup_storage_on_start: args.cleanup_storage_on_start,
            ..crate::cli::args::ManifestLoadArgs::default()
        };
        let mut config = build_manifest_load_config(&load_args).expect("config");
        config.storage_dir = temp_dir.path().to_string_lossy().to_string();
        let loaded = execute_manifest_load(config).await.expect("load");
        let selected = super::select_entity_ids(&loaded.search, &inputs).expect("selected");
        assert!(
            selected
                .iter()
                .any(|id| id == "model.nova_test.stg__traffic_sessions")
        );
    }

    #[tokio::test]
    async fn entity_matches_changed_files_uses_patch_path_when_present() {
        let entity = crate::manifest::entity::Entity::from_json(
            "model.pkg.orders",
            &json!({
                "resource_type": "model",
                "name": "orders",
                "original_file_path": "models/marts/orders.sql",
                "patch_path": "models/marts/orders.yml"
            }),
        );
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&entity).expect("serialize");
        let archived =
            rkyv::access::<crate::manifest::entity::ArchivedEntity, rkyv::rancor::Error>(&bytes)
                .expect("archive");
        let changed = std::collections::BTreeSet::from(["models/marts/orders.yml".to_string()]);
        assert!(entity_matches_changed_files(archived, &changed));
    }

    #[tokio::test]
    async fn changed_selection_no_target_for_model_yaml_is_advisory() {
        let temp_dir = TempDir::new().expect("temp dir");
        let args = MetadataAuditArgs {
            selection_mode: MetadataAuditSelectionModeArg::Changed,
            changed_files_json: Some("[\"models/marts/missing_model.yml\"]".to_string()),
            manifest_path: Some(fixture_manifest_path_string()),
            storage_instance_id: Some("audit-changed-no-target-advisory-test".to_string()),
            cleanup_storage_on_start: true,
            ..MetadataAuditArgs::default()
        };
        let inputs = super::parse_audit_inputs(&args).expect("audit inputs");
        let load_args = crate::cli::args::ManifestLoadArgs {
            manifest_path: args.manifest_path.clone(),
            storage_instance_id: args.storage_instance_id.clone(),
            cleanup_storage_on_start: args.cleanup_storage_on_start,
            ..crate::cli::args::ManifestLoadArgs::default()
        };
        let mut config = build_manifest_load_config(&load_args).expect("config");
        config.storage_dir = temp_dir.path().to_string_lossy().to_string();
        let loaded = execute_manifest_load(config).await.expect("load");
        let report = super::build_metadata_audit_report(&loaded.search, &inputs, &args)
            .await
            .expect("report");
        assert_eq!(report.target_count, 0);
        assert_eq!(report.gate_status, "advisory");
        assert_eq!(report.summary.required_fail_count, 0);
        assert_eq!(report.summary.advisory_fail_count, 1);
        assert!(
            report
                .summary
                .no_target_reason
                .as_deref()
                .unwrap_or_default()
                .contains("dbt model/schema-looking changed files")
        );
        assert!(render_markdown_report(&report).contains("Reason:"));
    }

    #[tokio::test]
    async fn changed_selection_no_target_respects_fail_on_no_targets() {
        let temp_dir = TempDir::new().expect("temp dir");
        let args = MetadataAuditArgs {
            selection_mode: MetadataAuditSelectionModeArg::Changed,
            changed_files_json: Some("[\"models/marts/missing_model.sql\"]".to_string()),
            manifest_path: Some(fixture_manifest_path_string()),
            storage_instance_id: Some("audit-changed-no-target-fail-test".to_string()),
            cleanup_storage_on_start: true,
            fail_on_no_targets: true,
            ..MetadataAuditArgs::default()
        };
        let inputs = super::parse_audit_inputs(&args).expect("audit inputs");
        let load_args = crate::cli::args::ManifestLoadArgs {
            manifest_path: args.manifest_path.clone(),
            storage_instance_id: args.storage_instance_id.clone(),
            cleanup_storage_on_start: args.cleanup_storage_on_start,
            ..crate::cli::args::ManifestLoadArgs::default()
        };
        let mut config = build_manifest_load_config(&load_args).expect("config");
        config.storage_dir = temp_dir.path().to_string_lossy().to_string();
        let loaded = execute_manifest_load(config).await.expect("load");
        let report = super::build_metadata_audit_report(&loaded.search, &inputs, &args)
            .await
            .expect("report");
        assert_eq!(report.target_count, 0);
        assert_eq!(report.gate_status, "fail");
        assert_eq!(report.summary.required_fail_count, 1);
        assert_eq!(report.summary.advisory_fail_count, 0);
    }

    #[test]
    fn markdown_report_includes_gate_status() {
        let report = MetadataAuditReport {
            selection_mode: "changed".to_string(),
            manifest_hash: "abc".to_string(),
            manifest_version: "abc".to_string(),
            manifest_source: "file:///tmp/manifest.json".to_string(),
            resource_types: vec!["model".to_string()],
            personas: vec!["engineer".to_string()],
            changed_files: vec!["models/marts/orders.sql".to_string()],
            selected_entity_ids: vec!["model.pkg.orders".to_string()],
            target_count: 1,
            scored_count: 1,
            gate_status: "fail",
            summary: super::AuditSummary {
                required_fail_count: 1,
                advisory_fail_count: 0,
                pass_count: 0,
                no_target: false,
                no_target_reason: None,
            },
            project_summary: None,
            entities: vec![super::EntityAuditReport {
                unique_id: "model.pkg.orders".to_string(),
                name: Some("orders".to_string()),
                resource_type: Some("model".to_string()),
                original_file_path: Some("models/marts/orders.sql".to_string()),
                patch_path: None,
                gate_status: "fail",
                personas: std::collections::BTreeMap::from([(
                    "engineer".to_string(),
                    super::PersonaAuditReport {
                        overall_score: 60,
                        grade: "D".to_string(),
                        categories: JsonValue::Null,
                        breakdown: JsonValue::Null,
                        recommendations: JsonValue::Array(Vec::new()),
                        threshold: None,
                        gate_status: "required_fail",
                    },
                )]),
            }],
        };
        let markdown = render_markdown_report(&report);
        assert!(markdown.contains("gate_status: `fail`"));
        assert!(markdown.contains("model.pkg.orders"));
    }

    #[tokio::test]
    async fn project_selection_uses_project_thresholds_for_gate_status() {
        let temp_dir = TempDir::new().expect("temp dir");
        let args = MetadataAuditArgs {
            selection_mode: MetadataAuditSelectionModeArg::Project,
            resource_types_json: Some("[\"model\"]".to_string()),
            personas_json: Some("[\"engineer\"]".to_string()),
            thresholds_json: Some(
                "{\"project\":{\"engineer\":{\"min_score\":101,\"severity\":\"required\"}}}"
                    .to_string(),
            ),
            manifest_path: Some(fixture_manifest_path_string()),
            storage_instance_id: Some("audit-project-selection-test".to_string()),
            cleanup_storage_on_start: true,
            ..MetadataAuditArgs::default()
        };
        let inputs = super::parse_audit_inputs(&args).expect("audit inputs");
        let load_args = crate::cli::args::ManifestLoadArgs {
            manifest_path: args.manifest_path.clone(),
            storage_instance_id: args.storage_instance_id.clone(),
            cleanup_storage_on_start: args.cleanup_storage_on_start,
            ..crate::cli::args::ManifestLoadArgs::default()
        };
        let mut config = build_manifest_load_config(&load_args).expect("config");
        config.storage_dir = temp_dir.path().to_string_lossy().to_string();
        let loaded = execute_manifest_load(config).await.expect("load");
        let report = super::build_metadata_audit_report(&loaded.search, &inputs, &args)
            .await
            .expect("report");
        assert_eq!(report.gate_status, "fail");
        assert_eq!(report.summary.required_fail_count, 1);
        assert!(report.project_summary.is_some());
        assert!(
            report
                .entities
                .iter()
                .all(|entity| entity.gate_status == "pass")
        );
    }
}
