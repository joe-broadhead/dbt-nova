use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::Instant;

use crate::cli::args::{NovaMetaAuditArgs, NovaMetaResourceKindArg};
use crate::cli::output::{CliEnvelope, CliMeta};
use crate::error::DbtNovaError;
use crate::nova_meta::{
    NovaMetaFinding, NovaMetaFindingSeverity, NovaMetaTargetSelector, NovaMetaValidationOptions,
    validate_nova_meta,
};
use serde::Serialize;

use super::{DispatchError, DispatchResult};

#[derive(Debug, Serialize)]
struct NovaMetaCliEnvelope<'a> {
    command: &'static str,
    status: &'static str,
    data: &'a crate::nova_meta::NovaMetaValidationReport,
    meta: CliMeta,
    error: Option<serde_json::Value>,
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
                command: "audit nova-meta",
                status: "error",
                data: &report,
                meta: CliMeta {
                    elapsed_ms,
                    timestamp_ms: timestamp_ms(),
                    version: env!("CARGO_PKG_VERSION"),
                },
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

fn timestamp_ms() -> u128 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis())
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

fn map_resource_kind(kind: NovaMetaResourceKindArg) -> crate::nova_meta::NovaMetaResourceKind {
    match kind {
        NovaMetaResourceKindArg::Model => crate::nova_meta::NovaMetaResourceKind::Model,
        NovaMetaResourceKindArg::Source => crate::nova_meta::NovaMetaResourceKind::Source,
        NovaMetaResourceKindArg::Table => crate::nova_meta::NovaMetaResourceKind::Table,
        NovaMetaResourceKindArg::Metric => crate::nova_meta::NovaMetaResourceKind::Metric,
    }
}

#[cfg(test)]
mod tests {
    use super::build_nova_meta_validation_options;
    use crate::cli::args::{NovaMetaAuditArgs, NovaMetaResourceKindArg};
    use crate::nova_meta::NovaMetaResourceKind;

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
}
