use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use jsonschema::{Validator, draft202012};
use serde::Serialize;
use serde_json::{Map as JsonMap, Value as JsonValue, json};
use serde_yaml::Value as YamlValue;

use crate::error::{DbtNovaError, Result};

const NOVA_META_SCHEMA_VERSION: &str = "v0";
const ENTITY_ONLY_FIELDS: [&str; 8] = [
    "canonical",
    "domains",
    "use_cases",
    "grain",
    "measures",
    "metric",
    "metrics",
    "search",
];
const ROOT_ARRAY_WARNING_FIELDS: [&str; 4] = ["synonyms", "domains", "use_cases", "example_values"];
const DEFAULT_IGNORED_DIR_NAMES: [&str; 9] = [
    ".git",
    ".venv",
    "venv",
    "target",
    "dbt_packages",
    "node_modules",
    "dist",
    "build",
    "__pycache__",
];

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum NovaMetaResourceKind {
    Model,
    Source,
    Table,
    Metric,
}

impl NovaMetaResourceKind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Model => "model",
            Self::Source => "source",
            Self::Table => "table",
            Self::Metric => "metric",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NovaMetaTargetScope {
    Entity,
    Column,
}

#[derive(Debug, Clone)]
pub struct NovaMetaValidationOptions {
    pub project_dir: PathBuf,
    pub paths: Vec<PathBuf>,
    pub selector: NovaMetaTargetSelector,
}

#[derive(Debug, Clone, Default)]
pub struct NovaMetaTargetSelector {
    pub resource_kind: Option<NovaMetaResourceKind>,
    pub resource_name: Option<String>,
    pub column: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NovaMetaValidationReport {
    pub schema_version: &'static str,
    pub project_dir: String,
    pub scanned_files: usize,
    pub target_count: usize,
    pub error_count: usize,
    pub warning_count: usize,
    pub findings: Vec<NovaMetaFinding>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum NovaMetaFindingSeverity {
    Error,
    Warning,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum NovaMetaFindingKind {
    Parse,
    Schema,
    Semantic,
}

#[derive(Debug, Clone, Serialize)]
pub struct NovaMetaFinding {
    pub severity: NovaMetaFindingSeverity,
    pub kind: NovaMetaFindingKind,
    pub code: String,
    pub file_path: String,
    pub resource_kind: Option<String>,
    pub resource_name: Option<String>,
    pub parent_name: Option<String>,
    pub column_name: Option<String>,
    pub line: Option<usize>,
    pub field_path: Option<String>,
    pub schema_path: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone)]
struct ResourceDefinition {
    file_path: String,
    resource_kind: NovaMetaResourceKind,
    resource_name: String,
    parent_name: Option<String>,
    column_name: Option<String>,
    line: Option<usize>,
}

#[derive(Debug, Clone)]
struct NovaMetaTarget {
    definition: ResourceDefinition,
    scope: NovaMetaTargetScope,
    nova: JsonValue,
    declared_columns: BTreeSet<String>,
    column_roles: BTreeMap<String, String>,
}

#[derive(Debug, Default)]
struct FileScanOutcome {
    definitions: Vec<ResourceDefinition>,
    targets: Vec<NovaMetaTarget>,
    findings: Vec<NovaMetaFinding>,
}

#[must_use]
pub fn validate_nova_meta(options: &NovaMetaValidationOptions) -> NovaMetaValidationReport {
    let mut findings = Vec::new();
    let mut definitions = Vec::new();
    let mut targets = Vec::new();

    let scanned_files = match collect_yaml_files(&options.project_dir, &options.paths) {
        Ok(files) => files,
        Err(error) => {
            findings.push(file_error_finding(
                &options.project_dir.display().to_string(),
                "scan_error",
                error.to_string(),
            ));
            return finalize_report(&options.project_dir, 0, 0, findings);
        }
    };

    for file in &scanned_files {
        let outcome = scan_file(file, &options.project_dir);
        definitions.extend(outcome.definitions);
        targets.extend(outcome.targets);
        findings.extend(outcome.findings);
    }

    let selected_targets = select_targets(
        &definitions,
        &targets,
        &options.selector,
        &options.project_dir,
    );
    match selected_targets {
        Ok(selected) => {
            let selected_count = selected.len();
            for target in selected {
                findings.extend(validate_target_schema(target));
                findings.extend(validate_target_semantics(target));
            }
            finalize_report(
                &options.project_dir,
                scanned_files.len(),
                selected_count,
                findings,
            )
        }
        Err(error) => {
            findings.push(file_error_finding(
                &options.project_dir.display().to_string(),
                "selection_error",
                error.to_string(),
            ));
            finalize_report(&options.project_dir, scanned_files.len(), 0, findings)
        }
    }
}

fn finalize_report(
    project_dir: &Path,
    scanned_files: usize,
    target_count: usize,
    mut findings: Vec<NovaMetaFinding>,
) -> NovaMetaValidationReport {
    findings.sort_by(compare_findings);
    let error_count = findings
        .iter()
        .filter(|finding| finding.severity == NovaMetaFindingSeverity::Error)
        .count();
    let warning_count = findings
        .iter()
        .filter(|finding| finding.severity == NovaMetaFindingSeverity::Warning)
        .count();

    NovaMetaValidationReport {
        schema_version: NOVA_META_SCHEMA_VERSION,
        project_dir: project_dir.display().to_string(),
        scanned_files,
        target_count,
        error_count,
        warning_count,
        findings,
    }
}

fn collect_yaml_files(project_dir: &Path, explicit_paths: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut files = BTreeSet::new();
    if explicit_paths.is_empty() {
        collect_yaml_files_from_path(project_dir, &mut files, true)?;
    } else {
        for path in explicit_paths {
            let resolved = resolve_path(project_dir, path);
            collect_yaml_files_from_path(&resolved, &mut files, true)?;
        }
        if files.is_empty() {
            return Err(DbtNovaError::InvalidParams(
                "no YAML files were found in the supplied --path values".to_string(),
            ));
        }
    }
    Ok(files.into_iter().collect())
}

fn collect_yaml_files_from_path(
    path: &Path,
    files: &mut BTreeSet<PathBuf>,
    root_override: bool,
) -> Result<()> {
    if !path.exists() {
        return Err(DbtNovaError::InvalidParams(format!(
            "path '{}' does not exist",
            path.display()
        )));
    }

    if path.is_file() {
        if is_yaml_file(path) {
            files.insert(path.to_path_buf());
        }
        return Ok(());
    }

    if !root_override && should_skip_dir(path) {
        return Ok(());
    }

    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let child = entry.path();
        if child.is_dir() {
            collect_yaml_files_from_path(&child, files, false)?;
        } else if is_yaml_file(&child) {
            files.insert(child);
        }
    }
    Ok(())
}

fn should_skip_dir(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| DEFAULT_IGNORED_DIR_NAMES.contains(&name))
}

fn is_yaml_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("yml" | "yaml")
    )
}

fn resolve_path(project_dir: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        project_dir.join(path)
    }
}

fn scan_file(path: &Path, project_dir: &Path) -> FileScanOutcome {
    let file_path = display_path(project_dir, path);
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) => {
            return FileScanOutcome {
                findings: vec![file_error_finding(
                    &file_path,
                    "read_error",
                    format!("failed to read YAML file: {error}"),
                )],
                ..Default::default()
            };
        }
    };

    let yaml = match serde_yaml::from_str::<YamlValue>(&contents) {
        Ok(yaml) => yaml,
        Err(error) => {
            return FileScanOutcome {
                findings: vec![NovaMetaFinding {
                    severity: NovaMetaFindingSeverity::Error,
                    kind: NovaMetaFindingKind::Parse,
                    code: "yaml_parse_error".to_string(),
                    file_path,
                    resource_kind: None,
                    resource_name: None,
                    parent_name: None,
                    column_name: None,
                    line: error.location().map(|location| location.line()),
                    field_path: None,
                    schema_path: None,
                    message: format!("failed to parse YAML: {error}"),
                }],
                ..Default::default()
            };
        }
    };

    let mut outcome = FileScanOutcome::default();
    let Some(root) = yaml.as_mapping() else {
        return outcome;
    };

    let lines: Vec<&str> = contents.lines().collect();
    scan_entities(
        root.get(YamlValue::from("models"))
            .and_then(YamlValue::as_sequence),
        NovaMetaResourceKind::Model,
        None,
        &file_path,
        &lines,
        &mut outcome,
    );
    scan_sources(
        root.get(YamlValue::from("sources"))
            .and_then(YamlValue::as_sequence),
        &file_path,
        &lines,
        &mut outcome,
    );
    scan_entities(
        root.get(YamlValue::from("metrics"))
            .and_then(YamlValue::as_sequence),
        NovaMetaResourceKind::Metric,
        None,
        &file_path,
        &lines,
        &mut outcome,
    );

    outcome
}

fn scan_sources(
    sources: Option<&Vec<YamlValue>>,
    file_path: &str,
    lines: &[&str],
    outcome: &mut FileScanOutcome,
) {
    let Some(sources) = sources else {
        return;
    };
    for source in sources {
        let Some(mapping) = source.as_mapping() else {
            continue;
        };
        let Some(source_name) = yaml_string(mapping.get(YamlValue::from("name"))) else {
            continue;
        };
        let source_line = find_named_line(lines, None, &source_name);
        let source_meta = nova_meta_from_mapping(mapping.get(YamlValue::from("meta")));
        let source_definition = ResourceDefinition {
            file_path: file_path.to_string(),
            resource_kind: NovaMetaResourceKind::Source,
            resource_name: source_name.clone(),
            parent_name: None,
            column_name: None,
            line: source_line,
        };
        outcome.definitions.push(source_definition.clone());
        if let Some(nova) = source_meta {
            outcome.targets.push(NovaMetaTarget {
                definition: source_definition,
                scope: NovaMetaTargetScope::Entity,
                nova,
                declared_columns: BTreeSet::new(),
                column_roles: BTreeMap::new(),
            });
        }

        scan_entities(
            mapping
                .get(YamlValue::from("tables"))
                .and_then(YamlValue::as_sequence),
            NovaMetaResourceKind::Table,
            Some(source_name.as_str()),
            file_path,
            lines,
            outcome,
        );
    }
}

fn scan_entities(
    entities: Option<&Vec<YamlValue>>,
    kind: NovaMetaResourceKind,
    parent_name: Option<&str>,
    file_path: &str,
    lines: &[&str],
    outcome: &mut FileScanOutcome,
) {
    let Some(entities) = entities else {
        return;
    };

    for entity in entities {
        let Some(mapping) = entity.as_mapping() else {
            continue;
        };
        let Some(resource_name) = yaml_string(mapping.get(YamlValue::from("name"))) else {
            continue;
        };
        let entity_line = find_named_line(lines, parent_name, &resource_name);
        let declared_columns = collect_declared_columns(mapping.get(YamlValue::from("columns")));
        let column_roles = collect_column_roles(mapping.get(YamlValue::from("columns")));
        let entity_meta = nova_meta_from_mapping(mapping.get(YamlValue::from("meta")));

        let definition = ResourceDefinition {
            file_path: file_path.to_string(),
            resource_kind: kind,
            resource_name: resource_name.clone(),
            parent_name: parent_name.map(str::to_string),
            column_name: None,
            line: entity_line,
        };
        outcome.definitions.push(definition.clone());
        if let Some(nova) = entity_meta {
            outcome.targets.push(NovaMetaTarget {
                definition,
                scope: NovaMetaTargetScope::Entity,
                nova,
                declared_columns: declared_columns.clone(),
                column_roles: column_roles.clone(),
            });
        }

        scan_columns(
            mapping.get(YamlValue::from("columns")),
            kind,
            &resource_name,
            parent_name,
            file_path,
            lines,
            &declared_columns,
            &column_roles,
            outcome,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn scan_columns(
    columns: Option<&YamlValue>,
    kind: NovaMetaResourceKind,
    resource_name: &str,
    parent_name: Option<&str>,
    file_path: &str,
    lines: &[&str],
    declared_columns: &BTreeSet<String>,
    column_roles: &BTreeMap<String, String>,
    outcome: &mut FileScanOutcome,
) {
    let Some(columns) = columns.and_then(YamlValue::as_sequence) else {
        return;
    };

    let resource_line = find_named_line(lines, parent_name, resource_name);
    for column in columns {
        let Some(mapping) = column.as_mapping() else {
            continue;
        };
        let Some(column_name) = yaml_string(mapping.get(YamlValue::from("name"))) else {
            continue;
        };
        let column_line = find_named_line_after(lines, resource_line, &column_name);
        let column_meta = nova_meta_from_mapping(mapping.get(YamlValue::from("meta")));
        let definition = ResourceDefinition {
            file_path: file_path.to_string(),
            resource_kind: kind,
            resource_name: resource_name.to_string(),
            parent_name: parent_name.map(str::to_string),
            column_name: Some(column_name.clone()),
            line: column_line,
        };
        outcome.definitions.push(definition.clone());
        if let Some(nova) = column_meta {
            outcome.targets.push(NovaMetaTarget {
                definition,
                scope: NovaMetaTargetScope::Column,
                nova,
                declared_columns: declared_columns.clone(),
                column_roles: column_roles.clone(),
            });
        }
    }
}

fn collect_declared_columns(columns: Option<&YamlValue>) -> BTreeSet<String> {
    columns
        .and_then(YamlValue::as_sequence)
        .into_iter()
        .flatten()
        .filter_map(YamlValue::as_mapping)
        .filter_map(|mapping| yaml_string(mapping.get(YamlValue::from("name"))))
        .collect()
}

fn collect_column_roles(columns: Option<&YamlValue>) -> BTreeMap<String, String> {
    let mut roles = BTreeMap::new();
    let Some(columns) = columns.and_then(YamlValue::as_sequence) else {
        return roles;
    };

    for column in columns {
        let Some(mapping) = column.as_mapping() else {
            continue;
        };
        let Some(column_name) = yaml_string(mapping.get(YamlValue::from("name"))) else {
            continue;
        };
        let Some(meta) = mapping
            .get(YamlValue::from("meta"))
            .and_then(YamlValue::as_mapping)
        else {
            continue;
        };
        let Some(nova) = meta
            .get(YamlValue::from("nova"))
            .and_then(YamlValue::as_mapping)
        else {
            continue;
        };
        if let Some(role) = yaml_string(nova.get(YamlValue::from("role"))) {
            roles.insert(column_name, role);
        }
    }

    roles
}

fn nova_meta_from_mapping(meta: Option<&YamlValue>) -> Option<JsonValue> {
    let meta = meta.and_then(YamlValue::as_mapping)?;
    let nova = meta.get(YamlValue::from("nova"))?;
    serde_json::to_value(nova).ok()
}

fn yaml_string(value: Option<&YamlValue>) -> Option<String> {
    match value {
        Some(YamlValue::String(value)) => Some(value.trim().to_string()),
        Some(YamlValue::Number(value)) => Some(value.to_string()),
        _ => None,
    }
}

fn select_targets<'a>(
    definitions: &'a [ResourceDefinition],
    targets: &'a [NovaMetaTarget],
    selector: &NovaMetaTargetSelector,
    project_dir: &Path,
) -> Result<Vec<&'a NovaMetaTarget>> {
    if selector.resource_name.is_none()
        && selector.column.is_none()
        && selector.resource_kind.is_none()
    {
        return Ok(targets.iter().collect());
    }

    let matches_definition = |definition: &ResourceDefinition| -> bool {
        if let Some(kind) = selector.resource_kind
            && definition.resource_kind != kind
        {
            return false;
        }
        if let Some(resource_name) = selector.resource_name.as_deref()
            && definition.resource_name != resource_name
        {
            return false;
        }
        if let Some(column) = selector.column.as_deref()
            && definition.column_name.as_deref() != Some(column)
        {
            return false;
        }
        true
    };

    let matching_definitions: Vec<&ResourceDefinition> = definitions
        .iter()
        .filter(|definition| matches_definition(definition))
        .collect();
    if matching_definitions.is_empty() {
        let mut message = String::from("target did not match any dbt YAML resource");
        if let Some(kind) = selector.resource_kind {
            let _ = write!(message, " (resource_kind={})", kind.as_str());
        }
        if let Some(name) = selector.resource_name.as_deref() {
            let _ = write!(message, " (resource_name={name})");
        }
        if let Some(column) = selector.column.as_deref() {
            let _ = write!(message, " (column={column})");
        }
        return Err(DbtNovaError::InvalidParams(message));
    }

    let matching_targets: Vec<&NovaMetaTarget> = targets
        .iter()
        .filter(|target| matches_definition(&target.definition))
        .collect();
    if !matching_targets.is_empty() {
        return Ok(matching_targets);
    }

    let first = matching_definitions[0];
    let location = display_path(project_dir, Path::new(&first.file_path));
    if let Some(column_name) = first.column_name.as_deref() {
        Err(DbtNovaError::InvalidParams(format!(
            "column '{}' on {} '{}' in '{}' does not define meta.nova",
            column_name,
            first.resource_kind.as_str(),
            first.resource_name,
            location
        )))
    } else {
        Err(DbtNovaError::InvalidParams(format!(
            "{} '{}' in '{}' does not define meta.nova",
            first.resource_kind.as_str(),
            first.resource_name,
            location
        )))
    }
}

fn validate_target_schema(target: &NovaMetaTarget) -> Vec<NovaMetaFinding> {
    let mut findings = Vec::new();
    let wrapped = json!({ "nova": target.nova });
    let validator = schema_validator();
    for error in validator.iter_errors(&wrapped) {
        findings.push(NovaMetaFinding {
            severity: NovaMetaFindingSeverity::Error,
            kind: NovaMetaFindingKind::Schema,
            code: "schema_violation".to_string(),
            file_path: target.definition.file_path.clone(),
            resource_kind: Some(target.definition.resource_kind.as_str().to_string()),
            resource_name: Some(target.definition.resource_name.clone()),
            parent_name: target.definition.parent_name.clone(),
            column_name: target.definition.column_name.clone(),
            line: target.definition.line,
            field_path: Some(error.instance_path().as_str().to_string()),
            schema_path: Some(error.schema_path().as_str().to_string()),
            message: error.to_string(),
        });
    }
    findings
}

fn validate_target_semantics(target: &NovaMetaTarget) -> Vec<NovaMetaFinding> {
    let mut findings = Vec::new();
    let Some(nova) = target.nova.as_object() else {
        return findings;
    };

    if target.scope == NovaMetaTargetScope::Column {
        findings.extend(validate_column_scope_rules(target, nova));
    }

    if target.scope == NovaMetaTargetScope::Entity {
        findings.extend(validate_entity_scope_rules(target, nova));
    }

    findings.extend(validate_blank_string_rules(target, nova));
    findings.extend(validate_root_array_rules(target, nova));
    findings.extend(validate_governance_rules(target, nova));

    findings
}

fn validate_column_scope_rules(
    target: &NovaMetaTarget,
    nova: &JsonMap<String, JsonValue>,
) -> Vec<NovaMetaFinding> {
    let mut findings = Vec::new();
    for field in ENTITY_ONLY_FIELDS {
        if nova.contains_key(field) {
            findings.push(semantic_error(
                target,
                "column_entity_only_field",
                format!("meta.nova.{field} is only allowed at the entity level"),
                Some(format!("meta.nova.{field}")),
            ));
        }
    }
    findings
}

fn validate_entity_scope_rules(
    target: &NovaMetaTarget,
    nova: &JsonMap<String, JsonValue>,
) -> Vec<NovaMetaFinding> {
    let mut findings = Vec::new();
    let columns = validates_field_references(target).then_some(&target.declared_columns);

    if nova.contains_key("metric") && nova.contains_key("metrics") {
        findings.push(semantic_error(
            target,
            "metric_and_metrics_conflict",
            "meta.nova cannot define both metric and metrics on the same entity".to_string(),
            Some("meta.nova".to_string()),
        ));
    }

    if let Some(grain) = nova.get("grain").and_then(JsonValue::as_object) {
        findings.extend(validate_grain(target, "meta.nova.grain", grain, columns));
    }
    if let Some(measures) = nova.get("measures").and_then(JsonValue::as_array) {
        findings.extend(validate_measures(target, measures, columns));
    }
    if let Some(metric) = nova.get("metric").and_then(JsonValue::as_object) {
        findings.extend(validate_metric(target, "meta.nova.metric", metric, columns));
    }
    if let Some(metrics) = nova.get("metrics").and_then(JsonValue::as_array) {
        for (index, metric) in metrics.iter().enumerate() {
            if let Some(metric) = metric.as_object() {
                findings.extend(validate_metric(
                    target,
                    &format!("meta.nova.metrics[{index}]"),
                    metric,
                    columns,
                ));
            }
        }
    }

    findings.extend(validate_duplicate_names(target, nova));
    findings.extend(validate_canonical_conflicts(target, nova));
    findings.extend(validate_column_role_hints(target, nova));

    findings
}

fn validate_grain(
    target: &NovaMetaTarget,
    base_path: &str,
    grain: &JsonMap<String, JsonValue>,
    columns: Option<&BTreeSet<String>>,
) -> Vec<NovaMetaFinding> {
    let mut findings = Vec::new();
    let primary_keys = string_array(grain.get("primary_key"));
    let dimensions = string_array(grain.get("dimensions"));
    let time_field = grain.get("time_field").and_then(JsonValue::as_str);

    findings.extend(validate_field_exists(
        target,
        columns,
        time_field,
        &format!("{base_path}.time_field"),
    ));

    for primary_key in &primary_keys {
        findings.extend(validate_field_exists(
            target,
            columns,
            Some(primary_key.as_str()),
            &format!("{base_path}.primary_key"),
        ));
    }
    for dimension in &dimensions {
        findings.extend(validate_field_exists(
            target,
            columns,
            Some(dimension.as_str()),
            &format!("{base_path}.dimensions"),
        ));
    }

    findings.extend(validate_duplicate_array_entries(
        target,
        &primary_keys,
        "duplicate_primary_key",
        &format!("{base_path}.primary_key"),
    ));
    findings.extend(validate_duplicate_array_entries(
        target,
        &dimensions,
        "duplicate_dimension",
        &format!("{base_path}.dimensions"),
    ));

    let dimension_set: BTreeSet<&str> = dimensions.iter().map(String::as_str).collect();
    let primary_key_set: BTreeSet<&str> = primary_keys.iter().map(String::as_str).collect();
    for overlap in primary_key_set.intersection(&dimension_set) {
        findings.push(semantic_error(
            target,
            "grain_field_overlap",
            format!(
                "field '{overlap}' cannot appear in both grain.primary_key and grain.dimensions"
            ),
            Some(base_path.to_string()),
        ));
    }
    if let Some(time_field) = time_field
        && (primary_key_set.contains(time_field) || dimension_set.contains(time_field))
    {
        findings.push(semantic_error(
            target,
            "grain_field_overlap",
            format!("time field '{time_field}' cannot also appear in grain.primary_key or grain.dimensions"),
            Some(base_path.to_string()),
        ));
    }

    findings
}

fn validate_measures(
    target: &NovaMetaTarget,
    measures: &[JsonValue],
    columns: Option<&BTreeSet<String>>,
) -> Vec<NovaMetaFinding> {
    let mut findings = Vec::new();
    for (index, measure) in measures.iter().enumerate() {
        let Some(measure) = measure.as_object() else {
            continue;
        };
        let path = format!("meta.nova.measures[{index}]");
        findings.extend(validate_required_non_blank_string(
            target,
            measure.get("name").and_then(JsonValue::as_str),
            &format!("{path}.name"),
        ));
        let field = measure.get("field").and_then(JsonValue::as_str);
        findings.extend(validate_optional_non_blank_string(
            target,
            field,
            &format!("{path}.field"),
        ));
        findings.extend(validate_field_exists(
            target,
            columns,
            field,
            &format!("{path}.field"),
        ));
        if field.is_some() && measure.get("expression").is_none() {
            findings.push(semantic_warning(
                target,
                "measure_missing_expression",
                "measure should normally include an expression".to_string(),
                Some(path.clone()),
            ));
        }
        if measure.get("description").is_none() {
            findings.push(semantic_warning(
                target,
                "measure_missing_description",
                "measure should normally include a description".to_string(),
                Some(path),
            ));
        }
    }
    findings
}

fn validate_metric(
    target: &NovaMetaTarget,
    base_path: &str,
    metric: &JsonMap<String, JsonValue>,
    columns: Option<&BTreeSet<String>>,
) -> Vec<NovaMetaFinding> {
    let mut findings = Vec::new();
    findings.extend(validate_required_non_blank_string(
        target,
        metric.get("name").and_then(JsonValue::as_str),
        &format!("{base_path}.name"),
    ));
    if let Some(grain) = metric.get("grain").and_then(JsonValue::as_object) {
        findings.extend(validate_grain(
            target,
            &format!("{base_path}.grain"),
            grain,
            columns,
        ));
    } else {
        findings.push(semantic_warning(
            target,
            "metric_missing_grain",
            "metric template should normally include a grain block".to_string(),
            Some(base_path.to_string()),
        ));
    }
    if metric.get("template").and_then(JsonValue::as_bool) != Some(true) {
        findings.push(semantic_warning(
            target,
            "metric_missing_template",
            "metric template should normally set template: true".to_string(),
            Some(base_path.to_string()),
        ));
    }
    if metric.get("expression").is_none() {
        findings.push(semantic_warning(
            target,
            "metric_missing_expression",
            "metric template should normally include an expression".to_string(),
            Some(base_path.to_string()),
        ));
    }

    if let Some(filters) = metric
        .get("recommended_filters")
        .and_then(JsonValue::as_array)
    {
        if filters.is_empty() {
            findings.push(semantic_warning(
                target,
                "empty_recommended_filters",
                "recommended_filters is present but empty".to_string(),
                Some(format!("{base_path}.recommended_filters")),
            ));
        }
        for (index, filter) in filters.iter().enumerate() {
            let Some(filter) = filter.as_object() else {
                continue;
            };
            let path = format!("{base_path}.recommended_filters[{index}]");
            let field = filter.get("field").and_then(JsonValue::as_str);
            findings.extend(validate_required_non_blank_string(
                target,
                field,
                &format!("{path}.field"),
            ));
            findings.extend(validate_field_exists(
                target,
                columns,
                field,
                &format!("{path}.field"),
            ));
            findings.extend(validate_filter_operator(target, filter, &path));
        }
    }

    findings
}

fn validate_duplicate_names(
    target: &NovaMetaTarget,
    nova: &JsonMap<String, JsonValue>,
) -> Vec<NovaMetaFinding> {
    let mut findings = Vec::new();
    let mut seen_measures = BTreeSet::new();
    for name in nova
        .get("measures")
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
        .filter_map(|measure| measure.get("name"))
        .filter_map(JsonValue::as_str)
    {
        let normalized = normalize_name(name);
        if !seen_measures.insert(normalized.clone()) {
            findings.push(semantic_error(
                target,
                "duplicate_measure_name",
                format!("duplicate measure name '{name}' on the same entity"),
                Some("meta.nova.measures".to_string()),
            ));
        }
    }

    let mut seen_metrics = BTreeSet::new();
    if let Some(name) = nova
        .get("metric")
        .and_then(JsonValue::as_object)
        .and_then(|metric| metric.get("name"))
        .and_then(JsonValue::as_str)
    {
        seen_metrics.insert(normalize_name(name));
    }
    for name in nova
        .get("metrics")
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
        .filter_map(|metric| metric.get("name"))
        .filter_map(JsonValue::as_str)
    {
        let normalized = normalize_name(name);
        if !seen_metrics.insert(normalized.clone()) {
            findings.push(semantic_error(
                target,
                "duplicate_metric_name",
                format!("duplicate metric name '{name}' on the same entity"),
                Some("meta.nova.metrics".to_string()),
            ));
        }
    }

    for metric_name in seen_metrics {
        if seen_measures.contains(&metric_name) {
            findings.push(semantic_error(
                target,
                "duplicate_semantic_name",
                format!(
                    "name '{metric_name}' is used by both a measure and a metric on the same entity"
                ),
                Some("meta.nova".to_string()),
            ));
        }
    }

    findings
}

fn validate_canonical_conflicts(
    target: &NovaMetaTarget,
    nova: &JsonMap<String, JsonValue>,
) -> Vec<NovaMetaFinding> {
    let mut findings = Vec::new();
    if nova.get("canonical").and_then(JsonValue::as_bool) == Some(true)
        && nova
            .get("search")
            .and_then(JsonValue::as_object)
            .and_then(|search| search.get("candidates"))
            .and_then(JsonValue::as_object)
            .and_then(|candidates| candidates.get("analyst"))
            .and_then(JsonValue::as_bool)
            == Some(false)
    {
        findings.push(semantic_warning(
            target,
            "canonical_analyst_deboost_conflict",
            "canonical entities should not usually set search.candidates.analyst to false"
                .to_string(),
            Some("meta.nova.search.candidates.analyst".to_string()),
        ));
    }

    if let Some(candidates) = nova
        .get("search")
        .and_then(JsonValue::as_object)
        .and_then(|search| search.get("candidates"))
        .and_then(JsonValue::as_object)
        && candidates
            .values()
            .all(|value| value.as_bool() == Some(false))
    {
        findings.push(semantic_warning(
            target,
            "search_candidates_all_false",
            "search.candidates marks every persona as false; leave the block absent unless this is intentional"
                .to_string(),
            Some("meta.nova.search.candidates".to_string()),
        ));
    }

    findings
}

fn validate_column_role_hints(
    target: &NovaMetaTarget,
    nova: &JsonMap<String, JsonValue>,
) -> Vec<NovaMetaFinding> {
    let mut findings = Vec::new();

    if let Some(time_field) = nova
        .get("grain")
        .and_then(JsonValue::as_object)
        .and_then(|grain| grain.get("time_field"))
        .and_then(JsonValue::as_str)
        && let Some(role) = target.column_roles.get(time_field)
        && role != "time"
    {
        findings.push(semantic_warning(
            target,
            "time_field_role_mismatch",
            format!(
                "grain.time_field '{time_field}' is annotated with role '{role}' instead of 'time'"
            ),
            Some("meta.nova.grain.time_field".to_string()),
        ));
    }

    for primary_key in nova
        .get("grain")
        .and_then(JsonValue::as_object)
        .and_then(|grain| grain.get("primary_key"))
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
        .filter_map(JsonValue::as_str)
    {
        if let Some(role) = target.column_roles.get(primary_key)
            && role != "identifier"
        {
            findings.push(semantic_warning(
                target,
                "primary_key_role_mismatch",
                format!("grain.primary_key '{primary_key}' is annotated with role '{role}' instead of 'identifier'"),
                Some("meta.nova.grain.primary_key".to_string()),
            ));
        }
    }

    findings
}

fn validate_blank_string_rules(
    target: &NovaMetaTarget,
    nova: &JsonMap<String, JsonValue>,
) -> Vec<NovaMetaFinding> {
    let mut findings = Vec::new();
    for key in ["role", "semantic_type", "tier"] {
        if let Some(value) = nova.get(key).and_then(JsonValue::as_str)
            && value.trim().is_empty()
        {
            findings.push(semantic_error(
                target,
                "blank_string",
                format!("meta.nova.{key} cannot be blank"),
                Some(format!("meta.nova.{key}")),
            ));
        }
    }

    for key in ["synonyms", "domains", "use_cases", "example_values"] {
        if let Some(values) = nova.get(key).and_then(JsonValue::as_array) {
            for (index, value) in values.iter().enumerate() {
                if value.as_str().is_some_and(|value| value.trim().is_empty()) {
                    findings.push(semantic_error(
                        target,
                        "blank_array_value",
                        format!("meta.nova.{key}[{index}] cannot be blank"),
                        Some(format!("meta.nova.{key}[{index}]")),
                    ));
                }
            }
        }
    }

    findings
}

fn validate_root_array_rules(
    target: &NovaMetaTarget,
    nova: &JsonMap<String, JsonValue>,
) -> Vec<NovaMetaFinding> {
    let mut findings = Vec::new();
    for key in ROOT_ARRAY_WARNING_FIELDS {
        if let Some(values) = nova.get(key).and_then(JsonValue::as_array) {
            if values.is_empty() {
                findings.push(semantic_warning(
                    target,
                    "empty_array",
                    format!("meta.nova.{key} is present but empty"),
                    Some(format!("meta.nova.{key}")),
                ));
            }
            let strings = values
                .iter()
                .filter_map(JsonValue::as_str)
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>();
            findings.extend(validate_duplicate_array_entries(
                target,
                &strings,
                "duplicate_array_value",
                &format!("meta.nova.{key}"),
            ));
        }
    }
    findings
}

fn validate_governance_rules(
    target: &NovaMetaTarget,
    nova: &JsonMap<String, JsonValue>,
) -> Vec<NovaMetaFinding> {
    let mut findings = Vec::new();
    let Some(governance) = nova.get("governance").and_then(JsonValue::as_object) else {
        return findings;
    };
    if let Some(compliance) = governance.get("compliance").and_then(JsonValue::as_array) {
        if compliance.is_empty() {
            findings.push(semantic_warning(
                target,
                "empty_array",
                "meta.nova.governance.compliance is present but empty".to_string(),
                Some("meta.nova.governance.compliance".to_string()),
            ));
        }
        let values = compliance
            .iter()
            .filter_map(JsonValue::as_str)
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();
        findings.extend(validate_duplicate_array_entries(
            target,
            &values,
            "duplicate_array_value",
            "meta.nova.governance.compliance",
        ));
    }
    let Some(sensitivity) = governance.get("sensitivity").and_then(JsonValue::as_str) else {
        return findings;
    };
    let Some(pii) = governance.get("pii") else {
        return findings;
    };

    let pii_present = match pii {
        JsonValue::Bool(value) => *value,
        JsonValue::String(value) => value.eq_ignore_ascii_case("confirmed"),
        JsonValue::Array(values) => !values.is_empty(),
        _ => false,
    };
    if pii_present && matches!(sensitivity, "none" | "public") {
        findings.push(semantic_warning(
            target,
            "governance_pii_sensitivity_conflict",
            format!("governance.pii indicates sensitive data but governance.sensitivity is '{sensitivity}'"),
            Some("meta.nova.governance".to_string()),
        ));
    }

    findings
}

fn validate_field_exists(
    target: &NovaMetaTarget,
    columns: Option<&BTreeSet<String>>,
    field: Option<&str>,
    path: &str,
) -> Vec<NovaMetaFinding> {
    let Some(field) = field else {
        return Vec::new();
    };
    if field.trim().is_empty() {
        return vec![semantic_error(
            target,
            "blank_string",
            format!("{path} cannot be blank"),
            Some(path.to_string()),
        )];
    }
    let Some(columns) = columns else {
        return Vec::new();
    };
    if columns.contains(field) {
        return Vec::new();
    }
    vec![semantic_error(
        target,
        "missing_referenced_field",
        format!("referenced field '{field}' does not exist on the entity"),
        Some(path.to_string()),
    )]
}

fn validate_duplicate_array_entries(
    target: &NovaMetaTarget,
    values: &[String],
    code: &str,
    path: &str,
) -> Vec<NovaMetaFinding> {
    let mut findings = Vec::new();
    let mut seen = BTreeSet::new();
    for value in values {
        let normalized = normalize_name(value);
        if normalized.is_empty() {
            continue;
        }
        if !seen.insert(normalized) {
            findings.push(semantic_warning(
                target,
                code,
                format!("'{value}' appears more than once"),
                Some(path.to_string()),
            ));
        }
    }
    findings
}

fn validate_required_non_blank_string(
    target: &NovaMetaTarget,
    value: Option<&str>,
    path: &str,
) -> Vec<NovaMetaFinding> {
    match value {
        Some(value) if !value.trim().is_empty() => Vec::new(),
        _ => vec![semantic_error(
            target,
            "blank_string",
            format!("{path} cannot be blank"),
            Some(path.to_string()),
        )],
    }
}

fn validate_optional_non_blank_string(
    target: &NovaMetaTarget,
    value: Option<&str>,
    path: &str,
) -> Vec<NovaMetaFinding> {
    match value {
        Some(value) if value.trim().is_empty() => vec![semantic_error(
            target,
            "blank_string",
            format!("{path} cannot be blank"),
            Some(path.to_string()),
        )],
        _ => Vec::new(),
    }
}

fn validate_filter_operator(
    target: &NovaMetaTarget,
    filter: &JsonMap<String, JsonValue>,
    path: &str,
) -> Vec<NovaMetaFinding> {
    let mut findings = Vec::new();
    let operator = filter.get("operator").and_then(JsonValue::as_str);
    let values = filter
        .get("values")
        .and_then(JsonValue::as_array)
        .map_or(0, Vec::len);
    let field_path = format!("{path}.operator");

    match operator {
        Some("between") => {
            if values != 2 {
                findings.push(semantic_error(
                    target,
                    "invalid_filter_values",
                    "operator 'between' requires exactly 2 values".to_string(),
                    Some(field_path),
                ));
            }
        }
        Some("is_null" | "is_not_null") => {
            if values != 0 {
                findings.push(semantic_error(
                    target,
                    "invalid_filter_values",
                    "operator 'is_null' and 'is_not_null' cannot include values".to_string(),
                    Some(field_path),
                ));
            }
        }
        Some("=" | "!=" | ">" | ">=" | "<" | "<=") => {
            if values != 1 {
                findings.push(semantic_error(
                    target,
                    "invalid_filter_values",
                    "comparison operators require exactly 1 value".to_string(),
                    Some(field_path),
                ));
            }
        }
        Some("in" | "not_in") => {
            if values == 0 {
                findings.push(semantic_error(
                    target,
                    "invalid_filter_values",
                    "operator 'in' and 'not_in' require at least 1 value".to_string(),
                    Some(field_path),
                ));
            }
        }
        Some(operator) if operator.trim().is_empty() => findings.push(semantic_error(
            target,
            "blank_operator",
            "recommended filter operator cannot be blank".to_string(),
            Some(field_path),
        )),
        Some(operator) => findings.push(semantic_error(
            target,
            "unsupported_filter_operator",
            format!("unsupported recommended filter operator '{operator}'"),
            Some(field_path),
        )),
        _ => {}
    }

    findings
}

fn validates_field_references(target: &NovaMetaTarget) -> bool {
    target.definition.resource_kind != NovaMetaResourceKind::Metric
}

fn semantic_error(
    target: &NovaMetaTarget,
    code: &str,
    message: String,
    field_path: Option<String>,
) -> NovaMetaFinding {
    base_target_finding(
        target,
        NovaMetaFindingSeverity::Error,
        code,
        message,
        field_path,
    )
}

fn semantic_warning(
    target: &NovaMetaTarget,
    code: &str,
    message: String,
    field_path: Option<String>,
) -> NovaMetaFinding {
    base_target_finding(
        target,
        NovaMetaFindingSeverity::Warning,
        code,
        message,
        field_path,
    )
}

fn base_target_finding(
    target: &NovaMetaTarget,
    severity: NovaMetaFindingSeverity,
    code: &str,
    message: String,
    field_path: Option<String>,
) -> NovaMetaFinding {
    NovaMetaFinding {
        severity,
        kind: NovaMetaFindingKind::Semantic,
        code: code.to_string(),
        file_path: target.definition.file_path.clone(),
        resource_kind: Some(target.definition.resource_kind.as_str().to_string()),
        resource_name: Some(target.definition.resource_name.clone()),
        parent_name: target.definition.parent_name.clone(),
        column_name: target.definition.column_name.clone(),
        line: target.definition.line,
        field_path,
        schema_path: None,
        message,
    }
}

fn file_error_finding(file_path: &str, code: &str, message: String) -> NovaMetaFinding {
    NovaMetaFinding {
        severity: NovaMetaFindingSeverity::Error,
        kind: NovaMetaFindingKind::Parse,
        code: code.to_string(),
        file_path: file_path.to_string(),
        resource_kind: None,
        resource_name: None,
        parent_name: None,
        column_name: None,
        line: None,
        field_path: None,
        schema_path: None,
        message,
    }
}

fn schema_validator() -> &'static Validator {
    static VALIDATOR: OnceLock<Validator> = OnceLock::new();
    VALIDATOR.get_or_init(|| {
        let schema: JsonValue = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/schemas/nova/v0.json"
        )))
        .expect("valid bundled nova schema");
        draft202012::options()
            .build(&schema)
            .expect("compile bundled nova schema")
    })
}

fn display_path(project_dir: &Path, path: &Path) -> String {
    path.strip_prefix(project_dir)
        .unwrap_or(path)
        .display()
        .to_string()
}

fn compare_findings(left: &NovaMetaFinding, right: &NovaMetaFinding) -> std::cmp::Ordering {
    left.file_path
        .cmp(&right.file_path)
        .then_with(|| left.line.cmp(&right.line))
        .then_with(|| left.resource_kind.cmp(&right.resource_kind))
        .then_with(|| left.resource_name.cmp(&right.resource_name))
        .then_with(|| left.column_name.cmp(&right.column_name))
        .then_with(|| left.code.cmp(&right.code))
}

fn normalize_name(value: &str) -> String {
    value.trim().to_lowercase()
}

fn string_array(value: Option<&JsonValue>) -> Vec<String> {
    value
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
        .filter_map(JsonValue::as_str)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect()
}

fn find_named_line(lines: &[&str], parent_name: Option<&str>, name: &str) -> Option<usize> {
    let start = parent_name
        .and_then(|parent_name| find_named_line_after(lines, None, parent_name))
        .map(|line| line.saturating_sub(1));
    find_named_line_after(lines, start, name)
}

fn find_named_line_after(lines: &[&str], start_line: Option<usize>, name: &str) -> Option<usize> {
    let start_index = start_line.unwrap_or(0);
    for (index, line) in lines.iter().enumerate().skip(start_index) {
        if line_name(line).as_deref() == Some(name) {
            return Some(index + 1);
        }
    }
    None
}

fn line_name(line: &str) -> Option<String> {
    let trimmed = line.trim();
    let rest = trimmed
        .strip_prefix("- name:")
        .or_else(|| trimmed.strip_prefix("name:"))?;
    let value = rest.split('#').next().unwrap_or(rest).trim();
    let value = value.trim_matches('"').trim_matches('\'').trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::{
        NovaMetaFindingSeverity, NovaMetaResourceKind, NovaMetaTargetSelector,
        NovaMetaValidationOptions, validate_nova_meta,
    };

    fn write_fixture(
        temp_dir: &TempDir,
        relative_path: &str,
        contents: &str,
    ) -> std::path::PathBuf {
        let path = temp_dir.path().join(relative_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent");
        }
        fs::write(&path, contents).expect("write fixture");
        path
    }

    #[test]
    fn validate_nova_meta_accepts_valid_model_and_column() {
        let temp_dir = TempDir::new().expect("temp dir");
        write_fixture(
            &temp_dir,
            "models/orders.yml",
            r#"
version: 2
models:
  - name: fct_orders
    meta:
      nova:
        canonical: true
        grain:
          primary_key: ["order_id"]
          time_field: order_date
          dimensions: ["country_code"]
        measures:
          - name: orders
            type: count_distinct
            expression: "count(distinct order_id)"
            description: "Orders"
            field: order_id
    columns:
      - name: order_id
        meta:
          nova:
            role: identifier
      - name: order_date
        meta:
          nova:
            role: time
      - name: country_code
        meta:
          nova:
            role: dimension
        "#,
        );

        let report = validate_nova_meta(&NovaMetaValidationOptions {
            project_dir: temp_dir.path().to_path_buf(),
            paths: Vec::new(),
            selector: NovaMetaTargetSelector::default(),
        });

        assert_eq!(report.error_count, 0);
        assert_eq!(report.target_count, 4);
    }

    #[test]
    fn validate_nova_meta_reports_schema_and_semantic_errors() {
        let temp_dir = TempDir::new().expect("temp dir");
        write_fixture(
            &temp_dir,
            "models/orders.yml",
            r#"
version: 2
models:
  - name: fct_orders
    meta:
      nova:
        metric:
          name: conversion_rate
        metrics:
          - name: conversion_rate
            recommended_filters:
              - field: missing_field
                operator: between
                values: ["web"]
    columns:
      - name: order_id
        "#,
        );

        let report = validate_nova_meta(&NovaMetaValidationOptions {
            project_dir: temp_dir.path().to_path_buf(),
            paths: Vec::new(),
            selector: NovaMetaTargetSelector::default(),
        });

        assert!(report.error_count >= 3);
        assert!(report.findings.iter().any(|finding| {
            finding.code == "metric_and_metrics_conflict"
                && finding.severity == NovaMetaFindingSeverity::Error
        }));
        assert!(
            report
                .findings
                .iter()
                .any(|finding| finding.code == "missing_referenced_field")
        );
        assert!(
            report
                .findings
                .iter()
                .any(|finding| finding.code == "invalid_filter_values")
        );
    }

    #[test]
    fn validate_nova_meta_rejects_unsupported_filter_operator() {
        let temp_dir = TempDir::new().expect("temp dir");
        write_fixture(
            &temp_dir,
            "models/orders.yml",
            r#"
version: 2
models:
  - name: fct_orders
    meta:
      nova:
        metric:
          name: conversion_rate
          template: true
          expression: "sum(orders)"
          grain:
            dimensions: ["order_id"]
          recommended_filters:
            - field: order_id
              operator: gte
              values: ["100"]
    columns:
      - name: order_id
        "#,
        );

        let report = validate_nova_meta(&NovaMetaValidationOptions {
            project_dir: temp_dir.path().to_path_buf(),
            paths: Vec::new(),
            selector: NovaMetaTargetSelector::default(),
        });

        assert!(
            report
                .findings
                .iter()
                .any(|finding| finding.code == "unsupported_filter_operator")
        );
    }

    #[test]
    fn validate_nova_meta_skips_field_existence_checks_for_metric_resources() {
        let temp_dir = TempDir::new().expect("temp dir");
        write_fixture(
            &temp_dir,
            "models/metrics.yml",
            r#"
version: 2
metrics:
  - name: orders_conversion
    meta:
      nova:
        metric:
          name: conversion_rate
          template: true
          expression: "sum(orders) / nullif(sum(sessions), 0)"
          grain:
            time_field: metric_date
            dimensions: ["country_code"]
          recommended_filters:
            - field: channel
              operator: "="
              values: ["web"]
        "#,
        );

        let report = validate_nova_meta(&NovaMetaValidationOptions {
            project_dir: temp_dir.path().to_path_buf(),
            paths: Vec::new(),
            selector: NovaMetaTargetSelector {
                resource_kind: Some(NovaMetaResourceKind::Metric),
                resource_name: Some("orders_conversion".to_string()),
                column: None,
            },
        });

        assert_eq!(report.error_count, 0);
        assert!(
            !report
                .findings
                .iter()
                .any(|finding| finding.code == "missing_referenced_field")
        );
    }

    #[test]
    fn validate_nova_meta_can_target_single_resource_and_column() {
        let temp_dir = TempDir::new().expect("temp dir");
        let path = write_fixture(
            &temp_dir,
            "models/orders.yml",
            r"
version: 2
models:
  - name: fct_orders
    meta:
      nova:
        canonical: true
    columns:
      - name: order_id
        meta:
          nova:
            role: identifier
  - name: fct_sessions
    meta:
      nova:
        canonical: true
        ",
        );

        let report = validate_nova_meta(&NovaMetaValidationOptions {
            project_dir: temp_dir.path().to_path_buf(),
            paths: vec![path],
            selector: NovaMetaTargetSelector {
                resource_kind: Some(NovaMetaResourceKind::Model),
                resource_name: Some("fct_orders".to_string()),
                column: Some("order_id".to_string()),
            },
        });

        assert_eq!(report.target_count, 1);
        assert_eq!(report.error_count, 0);
    }

    #[test]
    fn validate_nova_meta_reports_missing_meta_for_selected_resource() {
        let temp_dir = TempDir::new().expect("temp dir");
        write_fixture(
            &temp_dir,
            "models/orders.yml",
            r"
version: 2
models:
  - name: fct_orders
    columns:
      - name: order_id
",
        );

        let report = validate_nova_meta(&NovaMetaValidationOptions {
            project_dir: temp_dir.path().to_path_buf(),
            paths: Vec::new(),
            selector: NovaMetaTargetSelector {
                resource_kind: Some(NovaMetaResourceKind::Model),
                resource_name: Some("fct_orders".to_string()),
                column: None,
            },
        });

        assert_eq!(report.target_count, 0);
        assert_eq!(report.error_count, 1);
        assert!(
            report
                .findings
                .iter()
                .any(|finding| finding.code == "selection_error")
        );
    }

    #[test]
    fn validate_nova_meta_skips_default_ignored_directories() {
        let temp_dir = TempDir::new().expect("temp dir");
        write_fixture(
            &temp_dir,
            "models/orders.yml",
            r"
version: 2
models:
  - name: fct_orders
    meta:
      nova:
        canonical: true
",
        );
        write_fixture(
            &temp_dir,
            ".venv/ignored.yml",
            r"
version: 2
models:
  - name: ignored_model
    meta:
      nova:
        role: fact
",
        );
        write_fixture(
            &temp_dir,
            "target/ignored.yml",
            r"
version: 2
models:
  - name: ignored_target_model
    meta:
      nova:
        role: fact
",
        );

        let report = validate_nova_meta(&NovaMetaValidationOptions {
            project_dir: temp_dir.path().to_path_buf(),
            paths: Vec::new(),
            selector: NovaMetaTargetSelector::default(),
        });

        assert_eq!(report.target_count, 1);
        assert_eq!(report.error_count, 0);
        assert!(
            report
                .findings
                .iter()
                .all(|finding| !finding.file_path.starts_with(".venv/"))
        );
        assert!(
            report
                .findings
                .iter()
                .all(|finding| !finding.file_path.starts_with("target/"))
        );
    }

    #[test]
    fn validate_nova_meta_allows_explicit_path_inside_ignored_directory() {
        let temp_dir = TempDir::new().expect("temp dir");
        write_fixture(
            &temp_dir,
            ".venv/ignored.yml",
            r"
version: 2
models:
  - name: ignored_model
    meta:
      nova:
        canonical: true
",
        );

        let report = validate_nova_meta(&NovaMetaValidationOptions {
            project_dir: temp_dir.path().to_path_buf(),
            paths: vec![std::path::PathBuf::from(".venv")],
            selector: NovaMetaTargetSelector::default(),
        });

        assert_eq!(report.target_count, 1);
        assert_eq!(report.error_count, 0);
    }
}
