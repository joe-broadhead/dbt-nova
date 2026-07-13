use std::fmt::Write as _;
use std::path::{Component, Path, PathBuf};
use std::time::Instant;

use crate::cli::args::{NovaMetaAuditArgs, NovaMetaResourceKindArg};
use crate::cli::output::{CliEnvelope, CliMeta};
use crate::error::DbtNovaError;
use crate::nova_meta::{
    NovaMetaFinding, NovaMetaFindingSeverity, NovaMetaTargetSelector, NovaMetaValidationOptions,
    validate_nova_meta,
};
use crate::params::{NovaMetaResourceKindParam, ValidateNovaMetaParams};
use crate::responses::{ApiContract, SuccessResponse, response_api_contract};
use serde::Serialize;

use super::{DispatchError, DispatchResult};

#[derive(Debug, Serialize)]
struct NovaMetaCliEnvelope<'a> {
    api: ApiContract,
    command: &'static str,
    status: &'static str,
    data: &'a crate::nova_meta::NovaMetaValidationReport,
    meta: CliMeta,
    error: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
struct NovaMetaToolSelectorContext {
    project_dir: String,
    paths: Vec<String>,
    resource_kind: Option<&'static str>,
    resource_name: Option<String>,
    column: Option<String>,
}

#[derive(Debug, Serialize)]
struct NovaMetaToolPathPolicy {
    filesystem_root: String,
    project_dir_must_be_under_filesystem_root: bool,
    paths_must_be_relative: bool,
    paths_must_stay_under_project_dir: bool,
}

/// Runs the `audit nova-meta` CLI command.
///
/// # Errors
/// Returns an error when option construction fails or the validator reports one
/// or more errors.
pub fn run_nova_meta_command(args: &NovaMetaAuditArgs) -> DispatchResult {
    let started = Instant::now();
    let options = build_nova_meta_validation_options(args).map_err(Into::<DispatchError>::into)?;
    let report = validate_nova_meta(&options);
    let elapsed_ms = started.elapsed().as_millis();
    let has_errors = report.error_count > 0;

    if args.json {
        let output = if has_errors {
            let envelope = NovaMetaCliEnvelope {
                api: response_api_contract(),
                command: "audit nova-meta",
                status: "error",
                data: &report,
                meta: CliMeta::new(elapsed_ms),
                error: Some(
                    DbtNovaError::ServerError(format!(
                        "nova meta validation failed ({} error(s), {} warning(s))",
                        report.error_count, report.warning_count
                    ))
                    .to_response(),
                ),
            };
            serde_json::to_string_pretty(&envelope)
        } else {
            let envelope = CliEnvelope::success("audit nova-meta", &report, elapsed_ms);
            serde_json::to_string_pretty(&envelope)
        }
        .map_err(|error| DispatchError {
            error: DbtNovaError::ServerError(error.to_string()),
            rendered: false,
        })?;
        println!("{output}");
    } else {
        print_human_report(
            &report.findings,
            report.target_count,
            report.error_count,
            report.warning_count,
        );
    }

    if has_errors {
        return Err(DispatchError {
            error: DbtNovaError::ServerError(format!(
                "nova meta validation failed ({} error(s), {} warning(s))",
                report.error_count, report.warning_count
            )),
            rendered: true,
        });
    }

    Ok(())
}

/// Builds the MCP/CLI-tool response for nova-meta validation.
///
/// # Errors
/// Returns an error when the requested project or path scope is outside the
/// local server filesystem safety policy, or when response serialization fails.
pub fn build_nova_meta_tool_response(
    params: &ValidateNovaMetaParams,
) -> crate::error::Result<serde_json::Value> {
    let filesystem_root = current_filesystem_root()?;
    let options = build_nova_meta_tool_validation_options(params, &filesystem_root)?;
    let report = validate_nova_meta(&options);
    let selector = NovaMetaToolSelectorContext {
        project_dir: options.project_dir.display().to_string(),
        paths: options
            .paths
            .iter()
            .map(|path| display_path(&options.project_dir, path))
            .collect(),
        resource_kind: options
            .selector
            .resource_kind
            .map(crate::nova_meta::NovaMetaResourceKind::as_str),
        resource_name: options.selector.resource_name.clone(),
        column: options.selector.column.clone(),
    };
    let path_policy = NovaMetaToolPathPolicy {
        filesystem_root: filesystem_root.display().to_string(),
        project_dir_must_be_under_filesystem_root: true,
        paths_must_be_relative: true,
        paths_must_stay_under_project_dir: true,
    };
    let mut payload = serde_json::to_value(&report)
        .map_err(|error| DbtNovaError::ServerError(error.to_string()))?;
    let Some(object) = payload.as_object_mut() else {
        return Err(DbtNovaError::ServerError(
            "failed to serialize nova-meta validation report as object".to_string(),
        ));
    };
    object.insert(
        "selector".to_string(),
        serde_json::to_value(selector)
            .map_err(|error| DbtNovaError::ServerError(error.to_string()))?,
    );
    object.insert(
        "path_policy".to_string(),
        serde_json::to_value(path_policy)
            .map_err(|error| DbtNovaError::ServerError(error.to_string()))?,
    );

    serde_json::to_value(SuccessResponse::new(payload, 1))
        .map_err(|error| DbtNovaError::ServerError(error.to_string()))
}

fn print_human_report(
    findings: &[NovaMetaFinding],
    target_count: usize,
    error_count: usize,
    warning_count: usize,
) {
    println!("nova meta validation complete");
    println!("targets: {target_count}");
    println!("errors: {error_count}");
    println!("warnings: {warning_count}");

    for finding in findings {
        let mut line = String::new();
        let severity = match finding.severity {
            NovaMetaFindingSeverity::Error => "error",
            NovaMetaFindingSeverity::Warning => "warning",
        };
        let _ = write!(line, "- [{severity}] {}", finding.file_path);
        if let Some(resource_kind) = finding.resource_kind.as_deref()
            && let Some(resource_name) = finding.resource_name.as_deref()
        {
            let _ = write!(line, " ({resource_kind} {resource_name}");
            if let Some(column_name) = finding.column_name.as_deref() {
                let _ = write!(line, " column {column_name}");
            }
            line.push(')');
        }
        if let Some(line_number) = finding.line {
            let _ = write!(line, ":{line_number}");
        }
        let _ = write!(line, " {}: {}", finding.code, finding.message);
        if let Some(field_path) = finding.field_path.as_deref() {
            let _ = write!(line, " [{field_path}]");
        }
        println!("{line}");
    }
}

/// Builds validator options for the `audit nova-meta` command.
///
/// # Errors
/// Returns an error if the CLI flags cannot be converted into a valid validator
/// configuration.
pub fn build_nova_meta_validation_options(
    args: &NovaMetaAuditArgs,
) -> crate::error::Result<NovaMetaValidationOptions> {
    let project_dir = args
        .project_dir
        .as_deref()
        .map_or_else(|| PathBuf::from("."), PathBuf::from);
    let paths = args.path.iter().map(PathBuf::from).collect();

    Ok(NovaMetaValidationOptions {
        project_dir,
        paths,
        selector: NovaMetaTargetSelector {
            resource_kind: args.resource_kind.map(map_resource_kind),
            resource_name: args.resource_name.clone(),
            column: args.column.clone(),
        },
    })
}

fn build_nova_meta_tool_validation_options(
    params: &ValidateNovaMetaParams,
    filesystem_root: &Path,
) -> crate::error::Result<NovaMetaValidationOptions> {
    let project_dir = resolve_safe_project_dir(params.project_dir.as_deref(), filesystem_root)?;
    let paths = params
        .paths
        .iter()
        .map(|path| resolve_safe_project_path(&project_dir, path))
        .collect::<crate::error::Result<Vec<_>>>()?;

    if params
        .column
        .as_deref()
        .is_some_and(|value| value.trim().is_empty())
    {
        return Err(DbtNovaError::InvalidParams(
            "column must not be empty".to_string(),
        ));
    }
    if params
        .resource_name
        .as_deref()
        .is_some_and(|value| value.trim().is_empty())
    {
        return Err(DbtNovaError::InvalidParams(
            "resource_name must not be empty".to_string(),
        ));
    }
    if params.column.is_some() && params.resource_name.is_none() {
        return Err(DbtNovaError::InvalidParams(
            "column requires resource_name".to_string(),
        ));
    }

    Ok(NovaMetaValidationOptions {
        project_dir,
        paths,
        selector: NovaMetaTargetSelector {
            resource_kind: params.resource_kind.map(map_resource_kind_param),
            resource_name: params.resource_name.clone(),
            column: params.column.clone(),
        },
    })
}

fn current_filesystem_root() -> crate::error::Result<PathBuf> {
    std::env::current_dir()
        .and_then(|path| path.canonicalize())
        .map_err(|error| {
            DbtNovaError::InvalidParams(format!(
                "failed to resolve server filesystem root: {error}"
            ))
        })
}

fn resolve_safe_project_dir(
    project_dir: Option<&str>,
    filesystem_root: &Path,
) -> crate::error::Result<PathBuf> {
    let raw = project_dir.unwrap_or(".").trim();
    if raw.is_empty() {
        return Err(DbtNovaError::InvalidParams(
            "project_dir must not be empty".to_string(),
        ));
    }
    let requested = PathBuf::from(raw);
    let resolved = if requested.is_absolute() {
        requested
    } else {
        filesystem_root.join(requested)
    };
    let canonical = resolved.canonicalize().map_err(|error| {
        DbtNovaError::InvalidParams(format!(
            "failed to resolve project_dir '{}': {error}",
            resolved.display()
        ))
    })?;
    if !canonical.is_dir() {
        return Err(DbtNovaError::InvalidParams(format!(
            "project_dir '{}' is not a directory",
            canonical.display()
        )));
    }
    ensure_under_root(
        &canonical,
        filesystem_root,
        "project_dir must stay under the server working directory",
    )?;
    Ok(canonical)
}

fn resolve_safe_project_path(project_dir: &Path, raw_path: &str) -> crate::error::Result<PathBuf> {
    let trimmed = raw_path.trim();
    if trimmed.is_empty() {
        return Err(DbtNovaError::InvalidParams(
            "paths must not contain empty values".to_string(),
        ));
    }
    let relative = PathBuf::from(trimmed);
    for component in relative.components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            Component::ParentDir => {
                return Err(DbtNovaError::InvalidParams(format!(
                    "path '{trimmed}' must stay inside project_dir"
                )));
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(DbtNovaError::InvalidParams(format!(
                    "path '{trimmed}' must be relative to project_dir"
                )));
            }
        }
    }

    let candidate = project_dir.join(&relative);
    if candidate.exists() {
        let canonical = candidate.canonicalize().map_err(|error| {
            DbtNovaError::InvalidParams(format!(
                "failed to resolve path '{}': {error}",
                candidate.display()
            ))
        })?;
        ensure_under_root(&canonical, project_dir, "path must stay inside project_dir")?;
        return Ok(canonical);
    }

    Ok(relative)
}

fn ensure_under_root(path: &Path, root: &Path, message: &str) -> crate::error::Result<()> {
    if path.starts_with(root) {
        Ok(())
    } else {
        Err(DbtNovaError::InvalidParams(format!(
            "{message}: '{}' is outside '{}'",
            path.display(),
            root.display()
        )))
    }
}

fn display_path(project_dir: &Path, path: &Path) -> String {
    path.strip_prefix(project_dir)
        .unwrap_or(path)
        .display()
        .to_string()
}

fn map_resource_kind(kind: NovaMetaResourceKindArg) -> crate::nova_meta::NovaMetaResourceKind {
    match kind {
        NovaMetaResourceKindArg::Model => crate::nova_meta::NovaMetaResourceKind::Model,
        NovaMetaResourceKindArg::Source => crate::nova_meta::NovaMetaResourceKind::Source,
        NovaMetaResourceKindArg::Table => crate::nova_meta::NovaMetaResourceKind::Table,
        NovaMetaResourceKindArg::Metric => crate::nova_meta::NovaMetaResourceKind::Metric,
    }
}

fn map_resource_kind_param(
    kind: NovaMetaResourceKindParam,
) -> crate::nova_meta::NovaMetaResourceKind {
    match kind {
        NovaMetaResourceKindParam::Model => crate::nova_meta::NovaMetaResourceKind::Model,
        NovaMetaResourceKindParam::Source => crate::nova_meta::NovaMetaResourceKind::Source,
        NovaMetaResourceKindParam::Table => crate::nova_meta::NovaMetaResourceKind::Table,
        NovaMetaResourceKindParam::Metric => crate::nova_meta::NovaMetaResourceKind::Metric,
    }
}

#[cfg(test)]
mod tests {
    use super::{build_nova_meta_tool_response, build_nova_meta_validation_options};
    use crate::cli::args::{NovaMetaAuditArgs, NovaMetaResourceKindArg};
    use crate::nova_meta::{NovaMetaResourceKind, validate_nova_meta};
    use crate::params::{NovaMetaResourceKindParam, ValidateNovaMetaParams};

    #[test]
    fn build_nova_meta_validation_options_defaults_project_dir() {
        let options = build_nova_meta_validation_options(&NovaMetaAuditArgs::default())
            .expect("build options");
        assert_eq!(options.project_dir, std::path::PathBuf::from("."));
        assert!(options.paths.is_empty());
    }

    #[test]
    fn build_nova_meta_validation_options_maps_selector() {
        let options = build_nova_meta_validation_options(&NovaMetaAuditArgs {
            project_dir: Some("/tmp/project".to_string()),
            path: vec!["models/orders.yml".to_string()],
            resource_kind: Some(NovaMetaResourceKindArg::Model),
            resource_name: Some("fct_orders".to_string()),
            column: Some("order_date".to_string()),
            json: true,
        })
        .expect("build options");

        assert_eq!(
            options.project_dir,
            std::path::PathBuf::from("/tmp/project")
        );
        assert_eq!(
            options.paths,
            vec![std::path::PathBuf::from("models/orders.yml")]
        );
        assert_eq!(
            options.selector.resource_kind,
            Some(NovaMetaResourceKind::Model)
        );
        assert_eq!(
            options.selector.resource_name.as_deref(),
            Some("fct_orders")
        );
        assert_eq!(options.selector.column.as_deref(), Some("order_date"));
    }

    #[test]
    fn build_nova_meta_tool_response_matches_cli_report_data() {
        let root = std::env::current_dir()
            .expect("cwd")
            .canonicalize()
            .expect("canonical cwd");
        let temp_dir = tempfile::TempDir::new_in(&root).expect("temp project");
        let project_relative = temp_dir
            .path()
            .strip_prefix(&root)
            .expect("relative temp project")
            .display()
            .to_string();
        let models_dir = temp_dir.path().join("models");
        std::fs::create_dir_all(&models_dir).expect("models dir");
        std::fs::write(
            models_dir.join("orders.yml"),
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
",
        )
        .expect("fixture");

        let cli_options = build_nova_meta_validation_options(&NovaMetaAuditArgs {
            project_dir: Some(project_relative.clone()),
            path: vec!["models/orders.yml".to_string()],
            resource_kind: Some(NovaMetaResourceKindArg::Model),
            resource_name: Some("fct_orders".to_string()),
            column: None,
            json: true,
        })
        .expect("cli options");
        let cli_report = validate_nova_meta(&cli_options);
        let tool_response = build_nova_meta_tool_response(&ValidateNovaMetaParams {
            project_dir: Some(project_relative),
            paths: vec!["models/orders.yml".to_string()],
            resource_kind: Some(NovaMetaResourceKindParam::Model),
            resource_name: Some("fct_orders".to_string()),
            column: None,
        })
        .expect("tool response");

        assert_eq!(tool_response["success"], serde_json::json!(true));
        assert_eq!(
            tool_response["data"]["target_count"],
            serde_json::json!(cli_report.target_count)
        );
        assert_eq!(
            tool_response["data"]["error_count"],
            serde_json::json!(cli_report.error_count)
        );
        assert_eq!(
            tool_response["data"]["warning_count"],
            serde_json::json!(cli_report.warning_count)
        );
        assert_eq!(
            tool_response["data"]["findings"],
            serde_json::to_value(&cli_report.findings).expect("findings json")
        );
        assert_eq!(
            tool_response["data"]["selector"]["resource_kind"],
            serde_json::json!("model")
        );
        assert_eq!(
            tool_response["data"]["selector"]["paths"],
            serde_json::json!(["models/orders.yml"])
        );
    }

    #[test]
    fn build_nova_meta_tool_response_rejects_paths_outside_project_dir() {
        let root = std::env::current_dir()
            .expect("cwd")
            .canonicalize()
            .expect("canonical cwd");
        let temp_dir = tempfile::TempDir::new_in(&root).expect("temp project");
        let project_relative = temp_dir
            .path()
            .strip_prefix(&root)
            .expect("relative temp project")
            .display()
            .to_string();

        let err = build_nova_meta_tool_response(&ValidateNovaMetaParams {
            project_dir: Some(project_relative.clone()),
            paths: vec!["../Cargo.toml".to_string()],
            ..ValidateNovaMetaParams::default()
        })
        .expect_err("unsafe path");

        assert!(err.to_string().contains("must stay inside project_dir"));

        let err = build_nova_meta_tool_response(&ValidateNovaMetaParams {
            project_dir: Some(project_relative),
            paths: vec![root.join("Cargo.toml").display().to_string()],
            ..ValidateNovaMetaParams::default()
        })
        .expect_err("absolute path");

        assert!(err.to_string().contains("must be relative to project_dir"));
    }
}
