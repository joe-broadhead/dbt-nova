use crate::cli::args::{MetadataAuditArgs, MetadataAuditSelectionModeArg};
use crate::error::{DbtNovaError, Result};
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
            params.changed_files_json.as_ref(),
        )?,
        entity_ids_json: typed_array_or_json(
            "entity_ids",
            &params.entity_ids,
            "entity_ids_json",
            params.entity_ids_json.as_ref(),
        )?,
        resource_types_json: typed_array_or_json(
            "resource_types",
            &params.resource_types,
            "resource_types_json",
            params.resource_types_json.as_ref(),
        )?,
        personas_json: typed_array_or_json(
            "personas",
            &params.personas,
            "personas_json",
            params.personas_json.as_ref(),
        )?,
        thresholds_json: params.thresholds_json.clone(),
        include_breakdown: params.include_breakdown,
        include_recommendations: params.include_recommendations,
        fail_on_no_targets: params.fail_on_no_targets,
        ..MetadataAuditArgs::default()
    })
}

fn typed_array_or_json(
    typed_name: &str,
    typed_values: &[String],
    json_name: &str,
    json_value: Option<&String>,
) -> Result<Option<String>> {
    if typed_values.is_empty() {
        return Ok(json_value.cloned());
    }
    if json_value.is_some() {
        return Err(DbtNovaError::InvalidParams(format!(
            "use only one of {typed_name} or {json_name}"
        )));
    }
    serde_json::to_string(typed_values)
        .map(Some)
        .map_err(|error| DbtNovaError::InvalidParams(format!("invalid {typed_name}: {error}")))
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
