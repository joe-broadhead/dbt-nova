use super::{
    ArchivedEntity, BTreeSet, EntityReadinessFinding, IndicatorReadinessFinding, JsonValue,
    MAX_SUGGESTED_META_PATCHES, ManifestSearch, MetaPatchContent, MetaPatchTarget, Result,
    SuggestedMetaPatch, column_nova_meta_json, entity_nova_meta_json, entity_patch,
    indicator_count_in_nova, indicator_meta_base_path, infer_primary_key_column, infer_time_column,
    json, json_array_field_non_empty, json_bool_field_present, looks_like_time_column,
    push_meta_patch,
};

pub(super) fn build_suggested_meta_patches(
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

pub(super) fn append_entity_meta_patches(
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

pub(super) fn append_grain_meta_patches(
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

pub(super) fn append_invalid_grain_shape_patch(
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

pub(super) fn has_invalid_grain_diagnostic(diagnostics: &JsonValue, field: &str) -> bool {
    diagnostics.as_array().is_some_and(|items| {
        items.iter().any(|item| {
            item.get("code").and_then(JsonValue::as_str) == Some("invalid_grain_shape")
                && item.get("field").and_then(JsonValue::as_str) == Some(field)
        })
    })
}

pub(super) fn append_governance_meta_patches(
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

pub(super) fn append_column_semantic_patches(
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

pub(super) fn append_indicator_seed_patch(
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

pub(super) fn append_indicator_meta_patches(
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

pub(super) fn indicator_patch_target(finding: &IndicatorReadinessFinding) -> MetaPatchTarget<'_> {
    MetaPatchTarget::Indicator {
        name: finding.indicator_name.as_deref(),
        kind: finding.indicator_type.as_str(),
    }
}

pub(super) fn append_indicator_execution_patches(
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

pub(super) fn append_indicator_time_patch(
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

pub(super) fn append_indicator_description_patch(
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

pub(super) fn append_malformed_indicator_patch(
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
