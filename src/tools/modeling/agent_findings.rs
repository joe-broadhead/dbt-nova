use super::{
    AgentModellingContext, AgentModellingFinding, AgentModellingSeverity, ArchivedEntity,
    ArchivedNovaMeta, ArchivedNovaMetric, ArchivedString, BTreeMap, BTreeSet, ColumnSemanticRef,
    DuplicateIndicatorRow, EntityOverlapProfile, EntityStore, FactLikeParent,
    IndicatorExecutionSurface, JsonValue, MetricSurfaceContext, ModelingIndicatorRef,
    MultiGrainEntityRow, Result, SemanticLabelRef, analyst_candidate_disabled,
    canonical_indicator_refs_for_entity, collect_column_name_drift_findings,
    collect_column_role_conflict_findings, column_governance_present, column_primary_key_names,
    direct_parent_ids, direct_parent_refs, duplicate_indicator_drill_down_hints,
    duplicate_indicator_refs, duplicate_parent_entity_refs, entity_columns_drill_down_hints,
    entity_drill_down_hints, entity_exposes_metric_or_measure, entity_is_analyst_facing,
    entity_layer, execution_surface, fact_like_direct_parents, fact_parent_entity_refs,
    fact_parent_grain_signatures, grain_field_names, grain_time_field,
    has_canonical_metric_or_measure, index_entity_indicator_labels, indicator_refs_for_entity,
    indicator_source_for_entity, is_helper_layer, is_measure_like_data_type, json,
    metric_looks_ratio, metric_missing_time_field_finding, metric_ratio_signals,
    metricflow_measure_references, modeling_entity_ref, modeling_entity_ref_from_entity_ref,
    modeling_metric_ref, normalize_value, normalized_entity_column_names,
    nova_grain_primary_key_names, pii_like_column_signal, semantic_label_drill_down_hints,
    semantic_label_entities, semantic_label_indicators, semantic_model_measure_refs,
    source_direct_parent_refs,
};

pub(super) fn collect_duplicate_indicator_findings(
    duplicate_indicator_rows: &[DuplicateIndicatorRow],
    findings: &mut Vec<AgentModellingFinding>,
) {
    for row in duplicate_indicator_rows {
        if row.canonical_parent_count > 1 {
            let severity = if row.inconsistent_grains {
                AgentModellingSeverity::Blocker
            } else {
                AgentModellingSeverity::High
            };
            findings.push(AgentModellingFinding {
                code: "duplicate_canonical_indicator",
                severity,
                category: "indicator_resolution",
                message: format!(
                    "Indicator `{}` has {} canonical parents.",
                    row.indicator_name, row.canonical_parent_count
                ),
                entities: duplicate_parent_entity_refs(&row.parents),
                indicators: duplicate_indicator_refs(row),
                evidence: json!({
                    "indicator_name": &row.indicator_name,
                    "indicator_type": &row.indicator_type,
                    "parent_count": row.parent_count,
                    "canonical_parent_count": row.canonical_parent_count,
                    "inconsistent_grains": row.inconsistent_grains,
                    "grain_variant_count": row.grain_signatures.len()
                }),
                recommendation: "Choose one canonical execution surface for this business indicator at this grain. If both are legitimate, rename or domain-scope one indicator.".to_string(),
                drill_down_hints: duplicate_indicator_drill_down_hints(row),
            });
        } else if row.parent_count > 1 && row.canonical_parent_count == 0 {
            findings.push(AgentModellingFinding {
                code: "duplicate_indicator_without_canonical_parent",
                severity: AgentModellingSeverity::Medium,
                category: "indicator_resolution",
                message: format!(
                    "Indicator `{}` has multiple parents but no canonical parent.",
                    row.indicator_name
                ),
                entities: duplicate_parent_entity_refs(&row.parents),
                indicators: duplicate_indicator_refs(row),
                evidence: json!({
                    "indicator_name": &row.indicator_name,
                    "indicator_type": &row.indicator_type,
                    "parent_count": row.parent_count,
                    "canonical_parent_count": row.canonical_parent_count,
                    "inconsistent_grains": row.inconsistent_grains,
                    "grain_variant_count": row.grain_signatures.len()
                }),
                recommendation: "Mark one parent indicator canonical or clarify names so agents can choose a preferred definition.".to_string(),
                drill_down_hints: duplicate_indicator_drill_down_hints(row),
            });
        }
    }
}

pub(super) fn collect_multi_grain_entity_findings(
    entities: &EntityStore,
    multi_grain_entity_rows: &[MultiGrainEntityRow],
    findings: &mut Vec<AgentModellingFinding>,
) -> Result<()> {
    for row in multi_grain_entity_rows {
        let entity = entities.get_archived(&row.entity.unique_id)?;
        let analyst_facing = entity.is_some_and(entity_is_analyst_facing);
        findings.push(AgentModellingFinding {
            code: "entity_multiple_grain_variants",
            severity: if analyst_facing {
                AgentModellingSeverity::High
            } else {
                AgentModellingSeverity::Medium
            },
            category: "grain_safety",
            message: format!(
                "Entity `{}` has {} declared grain variants.",
                row.entity.unique_id, row.grain_variant_count
            ),
            entities: vec![modeling_entity_ref_from_entity_ref(&row.entity)],
            indicators: Vec::new(),
            evidence: json!({
                "grain_variant_count": row.grain_variant_count,
                "analyst_facing": analyst_facing,
                "variant_sources": row.grain_variants.iter().flat_map(|variant| variant.sources.iter()).take(10).collect::<Vec<_>>()
            }),
            recommendation: "Separate base entity grain from metric-specific grain, or split metrics into clearer execution surfaces.".to_string(),
            drill_down_hints: vec![json!({
                "purpose": "inspect_multi_grain_entity",
                "tool": "get_entity",
                "arguments": {
                    "id_or_name": &row.entity.unique_id,
                    "detail": "standard"
                }
            })],
        });
    }
    Ok(())
}

pub(super) fn collect_entity_agent_modelling_findings(
    context: &AgentModellingContext<'_>,
    profiles: &[EntityOverlapProfile],
    findings: &mut Vec<AgentModellingFinding>,
) -> Result<()> {
    for profile in profiles {
        let Some(entity) = context.search.entities.get_archived(&profile.unique_id)? else {
            continue;
        };
        let nova = entity.nova_meta();
        if let Some(nova) = nova {
            collect_indicator_parent_not_queryable_findings(
                &profile.unique_id,
                entity,
                nova,
                findings,
            );
            collect_metric_surface_findings(&profile.unique_id, entity, nova, findings);
            collect_semantic_model_grain_findings(&profile.unique_id, entity, nova, findings);
            collect_canonical_primary_key_finding(&profile.unique_id, entity, nova, findings);
            collect_cross_grain_and_multi_fact_findings(
                context,
                &profile.unique_id,
                entity,
                nova,
                findings,
            )?;
            collect_helper_layer_findings(context, &profile.unique_id, entity, nova, findings);
        }
        collect_parent_lineage_findings(context, &profile.unique_id, entity, findings)?;
        collect_governance_findings(&profile.unique_id, entity, nova, findings);
        collect_catalog_integrity_findings(&profile.unique_id, entity, nova, findings);
        collect_semantic_metric_reference_findings(
            &profile.unique_id,
            entity,
            &context.semantic_model_measure_names,
            findings,
        );
    }
    Ok(())
}

pub(super) fn semantic_model_measure_names(entities: &EntityStore) -> Result<BTreeSet<String>> {
    let mut names = BTreeSet::new();
    for unique_id in entities.ids() {
        let Some(entity) = entities.get_archived(unique_id)? else {
            continue;
        };
        if entity.resource_type_str() != Some("semantic_model") {
            continue;
        }
        let Some(nova) = entity.nova_meta() else {
            continue;
        };
        names.extend(
            nova.measures
                .iter()
                .map(|measure| normalize_value(measure.name.as_str()))
                .filter(|value| !value.is_empty()),
        );
    }
    Ok(names)
}

pub(super) fn semantic_metric_names(entities: &EntityStore) -> Result<BTreeSet<String>> {
    let mut names = BTreeSet::new();
    for unique_id in entities.ids() {
        let Some(entity) = entities.get_archived(unique_id)? else {
            continue;
        };
        if entity.resource_type_str() != Some("metric") {
            continue;
        }
        if let Some(name) = entity.name_str().map(normalize_value)
            && !name.is_empty()
        {
            names.insert(name);
        }
        if let Some(nova) = entity.nova_meta() {
            if let Some(metric) = nova.metric.as_ref() {
                let name = normalize_value(metric.name.as_str());
                if !name.is_empty() {
                    names.insert(name);
                }
            }
            names.extend(
                nova.metrics
                    .iter()
                    .map(|metric| normalize_value(metric.name.as_str()))
                    .filter(|value| !value.is_empty()),
            );
        }
    }
    Ok(names)
}

pub(super) fn collect_semantic_label_collision_findings(
    context: &AgentModellingContext<'_>,
    profiles: &[EntityOverlapProfile],
    findings: &mut Vec<AgentModellingFinding>,
) -> Result<()> {
    let mut by_label = BTreeMap::<String, BTreeMap<String, SemanticLabelRef>>::new();
    for profile in profiles {
        let Some(entity) = context.search.entities.get_archived(&profile.unique_id)? else {
            continue;
        };
        let Some(nova) = entity.nova_meta() else {
            continue;
        };
        index_entity_indicator_labels(&mut by_label, &profile.unique_id, entity, nova);
    }

    for (label, refs_by_key) in by_label {
        let refs = refs_by_key.into_values().collect::<Vec<_>>();
        if refs.len() <= 1 {
            continue;
        }
        let canonical_count = refs.iter().filter(|entry| entry.canonical).count();
        let severity = if canonical_count > 1 {
            AgentModellingSeverity::High
        } else {
            AgentModellingSeverity::Medium
        };
        findings.push(AgentModellingFinding {
            code: "semantic_label_collision",
            severity,
            category: "indicator_resolution",
            message: format!("Semantic label `{label}` maps to multiple indicators."),
            entities: semantic_label_entities(&refs),
            indicators: semantic_label_indicators(&refs),
            evidence: json!({
                "label": label,
                "refs": refs.iter().map(|entry| entry.ref_key.as_str()).collect::<Vec<_>>(),
                "canonical_count": canonical_count
            }),
            recommendation: "Use domain-scoped names/synonyms such as gross_revenue, net_revenue, web_revenue, or finance_revenue.".to_string(),
            drill_down_hints: semantic_label_drill_down_hints(&label),
        });
    }
    Ok(())
}

pub(super) fn collect_column_semantic_ambiguity_findings(
    context: &AgentModellingContext<'_>,
    profiles: &[EntityOverlapProfile],
    findings: &mut Vec<AgentModellingFinding>,
) -> Result<()> {
    let mut by_semantic_type = BTreeMap::<String, Vec<ColumnSemanticRef>>::new();
    let mut by_column_name = BTreeMap::<String, Vec<ColumnSemanticRef>>::new();
    for profile in profiles {
        let Some(entity) = context.search.entities.get_archived(&profile.unique_id)? else {
            continue;
        };
        let analyst_facing = entity_is_analyst_facing(entity);
        for column in entity.column_meta() {
            let Some(semantic_type) = column
                .semantic_type
                .as_ref()
                .map(|value| normalize_value(value.as_str()))
                .filter(|value| !value.is_empty())
            else {
                continue;
            };
            let column_ref = ColumnSemanticRef {
                entity: modeling_entity_ref(&profile.unique_id, entity),
                column_name: column.name.as_str().to_string(),
                role: column.role.as_ref().map(|role| role.as_str().to_string()),
                semantic_type: semantic_type.clone(),
                analyst_facing,
            };
            by_semantic_type
                .entry(semantic_type)
                .or_default()
                .push(column_ref.clone());
            by_column_name
                .entry(normalize_value(column.name.as_str()))
                .or_default()
                .push(column_ref);
        }
    }

    collect_column_role_conflict_findings(by_semantic_type, findings);
    collect_column_name_drift_findings(by_column_name, findings);
    Ok(())
}

pub(super) fn collect_indicator_parent_not_queryable_findings(
    unique_id: &str,
    entity: &ArchivedEntity,
    nova: &ArchivedNovaMeta,
    findings: &mut Vec<AgentModellingFinding>,
) {
    let execution = execution_surface(entity);
    if execution != IndicatorExecutionSurface::MetadataOnly {
        return;
    }
    for indicator in indicator_refs_for_entity(unique_id, entity, nova) {
        findings.push(AgentModellingFinding {
            code: "indicator_parent_not_queryable",
            severity: AgentModellingSeverity::Blocker,
            category: "queryability",
            message: format!(
                "Indicator `{}` is attached to metadata-only parent `{unique_id}`.",
                indicator.indicator_name
            ),
            entities: vec![modeling_entity_ref(unique_id, entity)],
            indicators: vec![indicator],
            evidence: json!({
                "execution_surface": execution.as_str(),
                "queryable": execution.queryable(),
                "direct_sql_queryable": execution.direct_sql_queryable(),
                "queryable_via": execution.queryable_via()
            }),
            recommendation: "Move the indicator to a queryable dbt model, expose it through dbt Semantic Layer / MetricFlow, or mark the entity as non-analyst-facing.".to_string(),
            drill_down_hints: entity_drill_down_hints(unique_id),
        });
    }
}

pub(super) fn collect_metric_surface_findings(
    unique_id: &str,
    entity: &ArchivedEntity,
    nova: &ArchivedNovaMeta,
    findings: &mut Vec<AgentModellingFinding>,
) {
    let resource_type = entity.resource_type_str();
    let semantic_metric_parent = resource_type == Some("metric");
    let relation_backed = entity.relation_name_str().is_some();
    let column_names = normalized_entity_column_names(entity);
    let context = MetricSurfaceContext {
        unique_id,
        entity,
        nova,
        semantic_metric_parent,
        relation_backed,
        column_names: &column_names,
    };

    if let Some(metric) = nova.metric.as_ref() {
        collect_single_metric_surface_findings(&context, metric, findings);
    }
    for metric in nova.metrics.iter() {
        collect_single_metric_surface_findings(&context, metric, findings);
    }
}

pub(super) fn collect_single_metric_surface_findings(
    context: &MetricSurfaceContext<'_>,
    metric: &ArchivedNovaMetric,
    findings: &mut Vec<AgentModellingFinding>,
) {
    let effective_grain = metric.grain.as_ref().or(context.nova.grain.as_ref());
    if !context.semantic_metric_parent && effective_grain.and_then(grain_time_field).is_none() {
        findings.push(metric_missing_time_field_finding(
            context.unique_id,
            context.entity,
            metric,
        ));
    }

    if !context.relation_backed {
        return;
    }

    let metric_name = metric.name.as_str();
    if !metric.template && !context.column_names.contains(&normalize_value(metric_name)) {
        findings.push(AgentModellingFinding {
            code: "metric_output_column_missing",
            severity: AgentModellingSeverity::Medium,
            category: "queryability",
            message: format!(
                "Relation-backed metric `{metric_name}` is not exposed as an output column."
            ),
            entities: vec![modeling_entity_ref(context.unique_id, context.entity)],
            indicators: vec![modeling_metric_ref(
                context.unique_id,
                context.entity,
                metric_name,
            )],
            evidence: json!({
                "metric_name": metric_name,
                "relation_name": context.entity.relation_name_str(),
                "template": metric.template,
                "column_present": false
            }),
            recommendation: "Expose the metric value as a column named after the metric, or mark the metric as a template if it is not a direct output.".to_string(),
            drill_down_hints: entity_columns_drill_down_hints(context.unique_id),
        });
    }

    if let Some(grain) = effective_grain {
        let missing_fields = grain_field_names(grain)
            .into_iter()
            .filter(|field| !context.column_names.contains(&normalize_value(field)))
            .collect::<Vec<_>>();
        if !missing_fields.is_empty() {
            findings.push(AgentModellingFinding {
                code: "metric_grain_field_not_in_output",
                severity: AgentModellingSeverity::High,
                category: "grain_safety",
                message: format!(
                    "Relation-backed metric `{metric_name}` declares grain fields missing from output columns."
                ),
                entities: vec![modeling_entity_ref(context.unique_id, context.entity)],
                indicators: vec![modeling_metric_ref(
                    context.unique_id,
                    context.entity,
                    metric_name,
                )],
                evidence: json!({
                    "metric_name": metric_name,
                    "missing_fields": missing_fields,
                    "relation_name": context.entity.relation_name_str()
                }),
                recommendation: "Ensure every declared grain field is present on the relation-backed metric model.".to_string(),
                drill_down_hints: entity_columns_drill_down_hints(context.unique_id),
            });
        }
    }
}

pub(super) fn collect_semantic_model_grain_findings(
    unique_id: &str,
    entity: &ArchivedEntity,
    nova: &ArchivedNovaMeta,
    findings: &mut Vec<AgentModellingFinding>,
) {
    if entity.resource_type_str() != Some("semantic_model") || nova.measures.is_empty() {
        return;
    }
    let primary_key = nova
        .grain
        .as_ref()
        .is_some_and(|grain| !grain.primary_key.is_empty());
    let time_field = nova.grain.as_ref().and_then(grain_time_field).is_some();
    if !primary_key {
        findings.push(AgentModellingFinding {
            code: "semantic_model_missing_primary_entity",
            severity: AgentModellingSeverity::Medium,
            category: "grain_safety",
            message: format!("Semantic model `{unique_id}` has measures but no primary entity."),
            entities: vec![modeling_entity_ref(unique_id, entity)],
            indicators: semantic_model_measure_refs(unique_id, entity, nova),
            evidence: json!({
                "measure_count": nova.measures.len(),
                "primary_entity_present": false
            }),
            recommendation: "Add a primary entity to the semantic model so agents can reason about row identity and joinability.".to_string(),
            drill_down_hints: entity_drill_down_hints(unique_id),
        });
    }
    if !time_field {
        findings.push(AgentModellingFinding {
            code: "semantic_model_missing_time_dimension",
            severity: AgentModellingSeverity::High,
            category: "grain_safety",
            message: format!("Semantic model `{unique_id}` has measures but no time dimension."),
            entities: vec![modeling_entity_ref(unique_id, entity)],
            indicators: semantic_model_measure_refs(unique_id, entity, nova),
            evidence: json!({
                "measure_count": nova.measures.len(),
                "time_dimension_present": false
            }),
            recommendation: "Add a time dimension to the semantic model, or mark the measures as non-temporal if they cannot support period analysis.".to_string(),
            drill_down_hints: entity_drill_down_hints(unique_id),
        });
    }
}

pub(super) fn collect_canonical_primary_key_finding(
    unique_id: &str,
    entity: &ArchivedEntity,
    nova: &ArchivedNovaMeta,
    findings: &mut Vec<AgentModellingFinding>,
) {
    if !nova.canonical || entity.resource_type_str() != Some("model") {
        return;
    }
    let column_primary_keys = column_primary_key_names(entity);
    let grain_primary_keys = nova_grain_primary_key_names(nova);
    if !column_primary_keys.is_empty() || !grain_primary_keys.is_empty() {
        return;
    }
    findings.push(AgentModellingFinding {
        code: "canonical_entity_missing_primary_key",
        severity: AgentModellingSeverity::High,
        category: "grain_safety",
        message: format!("Canonical model `{unique_id}` has no declared primary key."),
        entities: vec![modeling_entity_ref(unique_id, entity)],
        indicators: indicator_refs_for_entity(unique_id, entity, nova),
        evidence: json!({
            "canonical": true,
            "column_primary_keys": column_primary_keys,
            "grain_primary_key": grain_primary_keys,
            "primary_key_present": false
        }),
        recommendation: "Declare primary key columns via `meta.nova.grain.primary_key` or column-level `meta.primary_key`, then add `unique` and `not_null` tests.".to_string(),
        drill_down_hints: entity_columns_drill_down_hints(unique_id),
    });
}

pub(super) fn collect_cross_grain_and_multi_fact_findings(
    context: &AgentModellingContext<'_>,
    unique_id: &str,
    entity: &ArchivedEntity,
    nova: &ArchivedNovaMeta,
    findings: &mut Vec<AgentModellingFinding>,
) -> Result<()> {
    let fact_parents = fact_like_direct_parents(context, unique_id)?;
    if entity_exposes_metric_or_measure(nova) && fact_parents.len() >= 2 {
        let grain_signatures = fact_parent_grain_signatures(&fact_parents);
        let severity = if grain_signatures.len() > 1 {
            AgentModellingSeverity::High
        } else {
            AgentModellingSeverity::Medium
        };
        findings.push(AgentModellingFinding {
            code: "multi_fact_metric_model",
            severity,
            category: "cross_grain_risk",
            message: format!(
                "Entity `{unique_id}` exposes indicators while joining multiple fact-like parents."
            ),
            entities: vec![modeling_entity_ref(unique_id, entity)],
            indicators: indicator_refs_for_entity(unique_id, entity, nova),
            evidence: json!({
                "fact_like_parent_count": fact_parents.len(),
                "fact_like_parents": fact_parent_entity_refs(&fact_parents),
                "grain_signatures": grain_signatures
            }),
            recommendation: "Verify this model aggregates each fact input to the output grain before joining. If this is a canonical KPI, document the output grain and tests or expose it through dbt Semantic Layer / MetricFlow.".to_string(),
            drill_down_hints: entity_drill_down_hints(unique_id),
        });
    }

    if let Some(metric) = nova.metric.as_ref() {
        collect_single_metric_cross_grain_findings(
            context,
            unique_id,
            entity,
            metric,
            &fact_parents,
            findings,
        );
    }
    for metric in nova.metrics.iter() {
        collect_single_metric_cross_grain_findings(
            context,
            unique_id,
            entity,
            metric,
            &fact_parents,
            findings,
        );
    }
    Ok(())
}

pub(super) fn collect_single_metric_cross_grain_findings(
    context: &AgentModellingContext<'_>,
    unique_id: &str,
    entity: &ArchivedEntity,
    metric: &ArchivedNovaMetric,
    fact_parents: &[FactLikeParent],
    findings: &mut Vec<AgentModellingFinding>,
) {
    if !metric_looks_ratio(metric) {
        return;
    }
    let metric_name = metric.name.as_str();
    let execution = execution_surface(entity);
    let ratio_signals = metric_ratio_signals(metric);
    if execution == IndicatorExecutionSurface::MetadataOnly {
        findings.push(AgentModellingFinding {
            code: "ratio_like_metric_without_deterministic_surface",
            severity: AgentModellingSeverity::Blocker,
            category: "cross_grain_risk",
            message: format!(
                "Ratio-like metric `{metric_name}` has no deterministic execution surface."
            ),
            entities: vec![modeling_entity_ref(unique_id, entity)],
            indicators: vec![modeling_metric_ref(unique_id, entity, metric_name)],
            evidence: json!({
                "metric_name": metric_name,
                "ratio_signals": ratio_signals,
                "execution_surface": execution.as_str(),
                "queryable": execution.queryable(),
                "direct_sql_queryable": execution.direct_sql_queryable(),
                "queryable_via": execution.queryable_via()
            }),
            recommendation: "Do not leave a ratio/cross-grain KPI as metadata-only. Expose a dbt model, MetricFlow metric, OSI-derived semantic artifact, recipe, or saved query.".to_string(),
            drill_down_hints: entity_drill_down_hints(unique_id),
        });
    }

    let metric_label = normalize_value(metric_name);
    if fact_parents.len() >= 2 && !context.semantic_metric_names.contains(&metric_label) {
        findings.push(AgentModellingFinding {
            code: "cross_grain_kpi_without_semantic_artifact",
            severity: AgentModellingSeverity::High,
            category: "cross_grain_risk",
            message: format!(
                "Ratio-like KPI `{metric_name}` combines fact-like parents without a matching semantic metric."
            ),
            entities: vec![modeling_entity_ref(unique_id, entity)],
            indicators: vec![modeling_metric_ref(unique_id, entity, metric_name)],
            evidence: json!({
                "metric_name": metric_name,
                "fact_like_parent_count": fact_parents.len(),
                "fact_like_parents": fact_parent_entity_refs(fact_parents),
                "semantic_metric_with_same_label": false
            }),
            recommendation: "Model this KPI as a deterministic dbt model or dbt Semantic Layer metric; do not require agents to infer cross-fact joins.".to_string(),
            drill_down_hints: entity_drill_down_hints(unique_id),
        });
    }
}

pub(super) fn collect_parent_lineage_findings(
    context: &AgentModellingContext<'_>,
    unique_id: &str,
    entity: &ArchivedEntity,
    findings: &mut Vec<AgentModellingFinding>,
) -> Result<()> {
    if !entity_is_analyst_facing(entity) {
        return Ok(());
    }
    let source_parents = source_direct_parent_refs(context, unique_id)?;
    if !source_parents.is_empty() {
        findings.push(AgentModellingFinding {
            code: "analyst_facing_model_depends_on_source",
            severity: AgentModellingSeverity::High,
            category: "layering",
            message: format!("Analyst-facing entity `{unique_id}` depends directly on a source."),
            entities: vec![modeling_entity_ref(unique_id, entity)],
            indicators: entity
                .nova_meta()
                .map(|nova| indicator_refs_for_entity(unique_id, entity, nova))
                .unwrap_or_default(),
            evidence: json!({
                "source_parent_count": source_parents.len(),
                "source_parents": source_parents
            }),
            recommendation: "Route raw source access through staging/base models before exposing analyst-facing metrics or marts.".to_string(),
            drill_down_hints: entity_drill_down_hints(unique_id),
        });
    }

    let parent_count = direct_parent_ids(context.search, unique_id).len();
    let threshold = context
        .search
        .config
        .agent_modelling_audit
        .too_many_parents_threshold;
    if parent_count >= threshold {
        findings.push(AgentModellingFinding {
            code: "agent_surface_too_many_parents",
            severity: AgentModellingSeverity::Medium,
            category: "layering",
            message: format!("Analyst-facing entity `{unique_id}` has {parent_count} direct parents."),
            entities: vec![modeling_entity_ref(unique_id, entity)],
            indicators: entity
                .nova_meta()
                .map(|nova| indicator_refs_for_entity(unique_id, entity, nova))
                .unwrap_or_default(),
            evidence: json!({
                "direct_parent_count": parent_count,
                "threshold": threshold,
                "direct_parents": direct_parent_refs(context, unique_id)?
            }),
            recommendation: "Split the model into clearer intermediate concepts, or document why this wide analyst surface is intentionally curated.".to_string(),
            drill_down_hints: entity_drill_down_hints(unique_id),
        });
    }
    Ok(())
}

pub(super) fn collect_helper_layer_findings(
    context: &AgentModellingContext<'_>,
    unique_id: &str,
    entity: &ArchivedEntity,
    nova: &ArchivedNovaMeta,
    findings: &mut Vec<AgentModellingFinding>,
) {
    let Some(layer) = entity_layer(context.search, entity) else {
        return;
    };
    if !is_helper_layer(&layer) {
        return;
    }
    if has_canonical_metric_or_measure(nova) {
        findings.push(AgentModellingFinding {
            code: "non_mart_model_exposes_canonical_indicator",
            severity: AgentModellingSeverity::Medium,
            category: "layering",
            message: format!(
                "Helper-layer entity `{unique_id}` exposes a canonical indicator."
            ),
            entities: vec![modeling_entity_ref(unique_id, entity)],
            indicators: canonical_indicator_refs_for_entity(unique_id, entity, nova),
            evidence: json!({
                "layer": layer,
                "canonical_indicator_present": true
            }),
            recommendation: "Move canonical indicators to the analyst-facing mart, or de-rank the helper with `search.candidates.analyst: false`.".to_string(),
            drill_down_hints: entity_drill_down_hints(unique_id),
        });
    }
    if !analyst_candidate_disabled(nova) {
        findings.push(AgentModellingFinding {
            code: "helper_ranked_as_analyst_candidate",
            severity: AgentModellingSeverity::Low,
            category: "layering",
            message: format!("Helper-layer entity `{unique_id}` is still an analyst candidate."),
            entities: vec![modeling_entity_ref(unique_id, entity)],
            indicators: indicator_refs_for_entity(unique_id, entity, nova),
            evidence: json!({
                "layer": layer,
                "analyst_candidate": true
            }),
            recommendation: "Set `meta.nova.search.candidates.analyst: false` for helper models that should remain searchable but not rank first.".to_string(),
            drill_down_hints: entity_drill_down_hints(unique_id),
        });
    }
}

pub(super) fn collect_governance_findings(
    unique_id: &str,
    entity: &ArchivedEntity,
    nova: Option<&ArchivedNovaMeta>,
    findings: &mut Vec<AgentModellingFinding>,
) {
    if !entity_is_analyst_facing(entity) {
        return;
    }
    let entity_governance_present = nova.is_some_and(|nova| nova.governance.is_some());
    if !entity_governance_present {
        findings.push(AgentModellingFinding {
            code: "analyst_surface_missing_governance",
            severity: AgentModellingSeverity::Medium,
            category: "governance",
            message: format!("Analyst-facing entity `{unique_id}` has no Nova governance block."),
            entities: vec![modeling_entity_ref(unique_id, entity)],
            indicators: nova
                .map(|nova| indicator_refs_for_entity(unique_id, entity, nova))
                .unwrap_or_default(),
            evidence: json!({
                "governance_present": false,
                "analyst_facing": true
            }),
            recommendation: "Add `meta.nova.governance.sensitivity`, `pii`, and compliance fields for analyst-facing surfaces.".to_string(),
            drill_down_hints: entity_drill_down_hints(unique_id),
        });
    }

    let entity_json = entity.to_json_value();
    let Some(columns) = entity_json.get("columns").and_then(JsonValue::as_object) else {
        return;
    };
    for (column_name, column) in columns {
        let Some(pii_signal) = pii_like_column_signal(column_name) else {
            continue;
        };
        let column_governance_present = column_governance_present(column);
        if entity_governance_present || column_governance_present {
            continue;
        }
        findings.push(AgentModellingFinding {
            code: "pii_like_column_without_governance",
            severity: AgentModellingSeverity::Medium,
            category: "governance",
            message: format!(
                "PII-like column `{column_name}` appears on analyst-facing entity `{unique_id}` without governance classification."
            ),
            entities: vec![modeling_entity_ref(unique_id, entity)],
            indicators: nova
                .map(|nova| indicator_refs_for_entity(unique_id, entity, nova))
                .unwrap_or_default(),
            evidence: json!({
                "column_name": column_name,
                "pii_signal": pii_signal,
                "entity_governance_present": entity_governance_present,
                "column_governance_present": column_governance_present
            }),
            recommendation: "Classify PII at entity or column level so agents can apply governance caveats.".to_string(),
            drill_down_hints: entity_columns_drill_down_hints(unique_id),
        });
    }
}

pub(super) fn collect_catalog_integrity_findings(
    unique_id: &str,
    entity: &ArchivedEntity,
    nova: Option<&ArchivedNovaMeta>,
    findings: &mut Vec<AgentModellingFinding>,
) {
    let entity_json = entity.to_json_value();
    let Some(columns) = entity_json.get("columns").and_then(JsonValue::as_object) else {
        return;
    };
    if let Some(nova) = nova {
        collect_catalog_indicator_field_findings(unique_id, entity, nova, columns, findings);
    }
    collect_catalog_only_candidate_measure_columns(unique_id, entity, columns, findings);
}

pub(super) fn collect_catalog_indicator_field_findings(
    unique_id: &str,
    entity: &ArchivedEntity,
    nova: &ArchivedNovaMeta,
    columns: &serde_json::Map<String, JsonValue>,
    findings: &mut Vec<AgentModellingFinding>,
) {
    for measure in nova.measures.iter() {
        let Some(field) = measure.field.as_ref().map(ArchivedString::as_str) else {
            continue;
        };
        let Some(column) = columns.get(field) else {
            continue;
        };
        let Some(drift) = column.get("catalog_drift").and_then(JsonValue::as_object) else {
            continue;
        };
        if drift
            .get("type_mismatch")
            .and_then(JsonValue::as_bool)
            .unwrap_or(false)
        {
            findings.push(AgentModellingFinding {
                code: "catalog_type_drift_on_indicator_field",
                severity: AgentModellingSeverity::Medium,
                category: "catalog_reality",
                message: format!(
                    "Measure `{}` uses field `{field}` with manifest/catalog type drift.",
                    measure.name.as_str()
                ),
                entities: vec![modeling_entity_ref(unique_id, entity)],
                indicators: vec![ModelingIndicatorRef {
                    indicator_name: measure.name.as_str().to_string(),
                    indicator_type: "measure".to_string(),
                    parent_unique_id: unique_id.to_string(),
                    source: Some(indicator_source_for_entity(entity).to_string()),
                }],
                evidence: json!({
                    "field": field,
                    "manifest_data_type": drift.get("manifest_data_type"),
                    "catalog_data_type": drift.get("catalog_data_type")
                }),
                recommendation: "Update dbt column metadata or investigate warehouse schema drift before relying on this measure.".to_string(),
                drill_down_hints: entity_columns_drill_down_hints(unique_id),
            });
        }
        if drift
            .get("missing_in_catalog")
            .and_then(JsonValue::as_bool)
            .unwrap_or(false)
        {
            findings.push(AgentModellingFinding {
                code: "catalog_missing_indicator_field",
                severity: AgentModellingSeverity::High,
                category: "catalog_reality",
                message: format!(
                    "Measure `{}` uses field `{field}` that is missing from catalog reality.",
                    measure.name.as_str()
                ),
                entities: vec![modeling_entity_ref(unique_id, entity)],
                indicators: vec![ModelingIndicatorRef {
                    indicator_name: measure.name.as_str().to_string(),
                    indicator_type: "measure".to_string(),
                    parent_unique_id: unique_id.to_string(),
                    source: Some(indicator_source_for_entity(entity).to_string()),
                }],
                evidence: json!({
                    "field": field,
                    "manifest_data_type": drift.get("manifest_data_type"),
                    "missing_in_catalog": true
                }),
                recommendation: "The dbt manifest declares an indicator field that is absent from catalog reality. Refresh or repair dbt docs or the warehouse schema.".to_string(),
                drill_down_hints: entity_columns_drill_down_hints(unique_id),
            });
        }
    }
}

pub(super) fn collect_catalog_only_candidate_measure_columns(
    unique_id: &str,
    entity: &ArchivedEntity,
    columns: &serde_json::Map<String, JsonValue>,
    findings: &mut Vec<AgentModellingFinding>,
) {
    if !entity_is_analyst_facing(entity) {
        return;
    }
    for (column_name, column) in columns {
        let catalog_only = column
            .get("catalog_drift")
            .and_then(|drift| drift.get("catalog_only"))
            .and_then(JsonValue::as_bool)
            .unwrap_or(false);
        if !catalog_only {
            continue;
        }
        let data_type = column
            .get("data_type")
            .and_then(JsonValue::as_str)
            .or_else(|| column.get("catalog_data_type").and_then(JsonValue::as_str));
        if !data_type.is_some_and(is_measure_like_data_type) {
            continue;
        }
        findings.push(AgentModellingFinding {
            code: "catalog_only_candidate_measure_column",
            severity: AgentModellingSeverity::Low,
            category: "catalog_reality",
            message: format!(
                "Catalog-only column `{column_name}` looks measure-like on an analyst-facing entity."
            ),
            entities: vec![modeling_entity_ref(unique_id, entity)],
            indicators: Vec::new(),
            evidence: json!({
                "column_name": column_name,
                "catalog_data_type": data_type,
                "catalog_only": true
            }),
            recommendation: "Consider documenting this warehouse-only measure-like column in dbt if analysts should use it.".to_string(),
            drill_down_hints: entity_columns_drill_down_hints(unique_id),
        });
    }
}

pub(super) fn collect_semantic_metric_reference_findings(
    unique_id: &str,
    entity: &ArchivedEntity,
    semantic_model_measure_names: &BTreeSet<String>,
    findings: &mut Vec<AgentModellingFinding>,
) {
    if entity.resource_type_str() != Some("metric") {
        return;
    }
    let entity_json = entity.to_json_value();
    let measure_refs = metricflow_measure_references(&entity_json);
    let missing = measure_refs
        .into_iter()
        .filter(|name| !semantic_model_measure_names.contains(&normalize_value(name)))
        .collect::<Vec<_>>();
    if missing.is_empty() {
        return;
    }
    let metric_name = entity.name_str().unwrap_or(unique_id);
    findings.push(AgentModellingFinding {
        code: "semantic_metric_unresolved_measure_ref",
        severity: AgentModellingSeverity::High,
        category: "semantic_artifact_integrity",
        message: format!(
            "dbt metric `{metric_name}` references measure(s) that are absent from semantic models."
        ),
        entities: vec![modeling_entity_ref(unique_id, entity)],
        indicators: entity
            .nova_meta()
            .and_then(|nova| nova.metric.as_ref())
            .map(|metric| vec![modeling_metric_ref(unique_id, entity, metric.name.as_str())])
            .unwrap_or_default(),
        evidence: json!({
            "missing_measure_refs": missing,
            "semantic_model_measure_count": semantic_model_measure_names.len()
        }),
        recommendation: "Fix the dbt semantic metric reference or ensure the referenced semantic model measure is present in the manifest.".to_string(),
        drill_down_hints: entity_drill_down_hints(unique_id),
    });
}
