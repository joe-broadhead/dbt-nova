use crate::cli::args::{MetadataAuditArgs, MetadataAuditSelectionModeArg};
use crate::cli::tool_param_aliases::{typed_array_or_json, typed_json_or_json};
use crate::error::Result;
use crate::params::{GetMetadataAuditParams, MetadataAuditSelectionModeParam};

pub(super) fn metadata_audit_args_from_tool_params(
    params: &GetMetadataAuditParams,
) -> Result<MetadataAuditArgs> {
    Ok(MetadataAuditArgs {
        selection_mode: metadata_audit_selection_mode_from_param(params.selection_mode),
        changed_files_json: typed_array_or_json(
            "changed_files",
            &params.changed_files,
            "changed_files_json",
            params.changed_files_json.as_deref(),
        )?,
        entity_ids_json: typed_array_or_json(
            "entity_ids",
            &params.entity_ids,
            "entity_ids_json",
            params.entity_ids_json.as_deref(),
        )?,
        resource_types_json: typed_array_or_json(
            "resource_types",
            &params.resource_types,
            "resource_types_json",
            params.resource_types_json.as_deref(),
        )?,
        personas_json: typed_array_or_json(
            "personas",
            &params.personas,
            "personas_json",
            params.personas_json.as_deref(),
        )?,
        thresholds_json: typed_json_or_json(
            "thresholds",
            params.thresholds.as_ref(),
            "thresholds_json",
            params.thresholds_json.as_deref(),
        )?,
        include_breakdown: params.include_breakdown,
        include_recommendations: params.include_recommendations,
        fail_on_no_targets: params.fail_on_no_targets,
        ..MetadataAuditArgs::default()
    })
}

fn metadata_audit_selection_mode_from_param(
    mode: MetadataAuditSelectionModeParam,
) -> MetadataAuditSelectionModeArg {
    match mode {
        MetadataAuditSelectionModeParam::Project => MetadataAuditSelectionModeArg::Project,
        MetadataAuditSelectionModeParam::Changed => MetadataAuditSelectionModeArg::Changed,
        MetadataAuditSelectionModeParam::Entities => MetadataAuditSelectionModeArg::Entities,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value as JsonValue, json};

    #[test]
    fn metadata_audit_tool_params_accept_typed_aliases() {
        let args = metadata_audit_args_from_tool_params(&GetMetadataAuditParams {
            selection_mode: MetadataAuditSelectionModeParam::Changed,
            changed_files: vec!["models/marts/orders.sql".to_string()],
            changed_files_json: Some(r#"["models/marts/orders.sql"]"#.to_string()),
            resource_types: vec!["model".to_string()],
            personas: vec!["governance".to_string()],
            thresholds: Some(json!({
                "entity": {
                    "governance": {"min_score": 80, "severity": "advisory"}
                }
            })),
            include_recommendations: Some(false),
            ..GetMetadataAuditParams::default()
        })
        .expect("typed aliases");

        assert_eq!(args.selection_mode, MetadataAuditSelectionModeArg::Changed);
        assert_eq!(
            args.changed_files_json.as_deref(),
            Some(r#"["models/marts/orders.sql"]"#)
        );
        assert_eq!(args.resource_types_json.as_deref(), Some(r#"["model"]"#));
        assert_eq!(args.personas_json.as_deref(), Some(r#"["governance"]"#));
        assert_eq!(args.include_recommendations, Some(false));
        assert_eq!(
            serde_json::from_str::<JsonValue>(args.thresholds_json.as_deref().expect("thresholds"))
                .expect("thresholds json"),
            json!({
                "entity": {
                    "governance": {"min_score": 80, "severity": "advisory"}
                }
            })
        );
    }

    #[test]
    fn metadata_audit_tool_params_reject_conflicting_aliases() {
        let error = metadata_audit_args_from_tool_params(&GetMetadataAuditParams {
            thresholds_json: Some(
                r#"{"entity":{"engineer":{"min_score":70,"severity":"required"}}}"#.to_string(),
            ),
            thresholds: Some(json!({
                "entity": {
                    "engineer": {"min_score": 80, "severity": "required"}
                }
            })),
            ..GetMetadataAuditParams::default()
        })
        .expect_err("conflicting aliases should fail");

        assert!(
            error
                .to_string()
                .contains("thresholds and thresholds_json differ"),
            "unexpected error: {error}"
        );
    }
}
