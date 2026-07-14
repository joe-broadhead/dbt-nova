use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map as JsonMap, Value as JsonValue};

use super::{
    NovaMetaFinding, NovaMetaTarget, NovaMetaTargetScope, ResourceDefinition, normalize_name,
    semantic_warning, string_array,
};

struct CanonicalIndicator<'a> {
    target: &'a NovaMetaTarget,
    name: String,
    indicator_type: &'static str,
    field_path: String,
    grain_signature: String,
    scoped_to_grain: bool,
}

pub(super) fn validate_project_canonical_indicators(
    targets: &[&NovaMetaTarget],
) -> Vec<NovaMetaFinding> {
    let mut by_name = BTreeMap::<String, Vec<CanonicalIndicator<'_>>>::new();
    for target in targets {
        if target.scope != NovaMetaTargetScope::Entity {
            continue;
        }
        let Some(nova) = target.nova.as_object() else {
            continue;
        };
        for indicator in canonical_indicators_for_target(target, nova) {
            by_name
                .entry(normalize_name(&indicator.name))
                .or_default()
                .push(indicator);
        }
    }

    let mut findings = Vec::new();
    for indicators in by_name.values() {
        if indicators.len() <= 1 {
            continue;
        }
        findings.extend(duplicate_canonical_surface_findings(indicators));
        if let Some(finding) = canonical_indicator_conflict_finding(indicators) {
            findings.push(finding);
        }
    }
    findings
}

fn canonical_indicators_for_target<'a>(
    target: &'a NovaMetaTarget,
    nova: &'a JsonMap<String, JsonValue>,
) -> Vec<CanonicalIndicator<'a>> {
    let mut indicators = Vec::new();
    let entity_grain = nova.get("grain").and_then(JsonValue::as_object);

    for (index, measure) in nova
        .get("measures")
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
        .enumerate()
    {
        let Some(measure) = measure.as_object() else {
            continue;
        };
        if measure.get("canonical").and_then(JsonValue::as_bool) != Some(true) {
            continue;
        }
        if let Some(name) = indicator_name(measure) {
            indicators.push(CanonicalIndicator {
                target,
                name,
                indicator_type: "measure",
                field_path: format!("meta.nova.measures[{index}].canonical"),
                grain_signature: grain_signature(entity_grain),
                scoped_to_grain: canonical_scope_is_grain(measure),
            });
        }
    }

    if let Some(metric) = nova.get("metric").and_then(JsonValue::as_object)
        && metric.get("canonical").and_then(JsonValue::as_bool) == Some(true)
        && let Some(name) = indicator_name(metric)
    {
        indicators.push(CanonicalIndicator {
            target,
            name,
            indicator_type: "metric",
            field_path: "meta.nova.metric.canonical".to_string(),
            grain_signature: grain_signature(
                metric
                    .get("grain")
                    .and_then(JsonValue::as_object)
                    .or(entity_grain),
            ),
            scoped_to_grain: canonical_scope_is_grain(metric),
        });
    }

    for (index, metric) in nova
        .get("metrics")
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
        .enumerate()
    {
        let Some(metric) = metric.as_object() else {
            continue;
        };
        if metric.get("canonical").and_then(JsonValue::as_bool) != Some(true) {
            continue;
        }
        if let Some(name) = indicator_name(metric) {
            indicators.push(CanonicalIndicator {
                target,
                name,
                indicator_type: "metric",
                field_path: format!("meta.nova.metrics[{index}].canonical"),
                grain_signature: grain_signature(
                    metric
                        .get("grain")
                        .and_then(JsonValue::as_object)
                        .or(entity_grain),
                ),
                scoped_to_grain: canonical_scope_is_grain(metric),
            });
        }
    }

    indicators
}

fn indicator_name(indicator: &JsonMap<String, JsonValue>) -> Option<String> {
    indicator
        .get("name")
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn duplicate_canonical_surface_findings(
    indicators: &[CanonicalIndicator<'_>],
) -> Vec<NovaMetaFinding> {
    let mut by_grain = BTreeMap::<&str, Vec<&CanonicalIndicator<'_>>>::new();
    for indicator in indicators {
        by_grain
            .entry(indicator.grain_signature.as_str())
            .or_default()
            .push(indicator);
    }

    let mut findings = Vec::new();
    for matching_grain in by_grain.values().filter(|values| values.len() > 1) {
        let first = matching_grain[0];
        findings.push(semantic_warning(
            first.target,
            "duplicate_canonical_surface",
            format!(
                "canonical indicator '{}' is declared on multiple parents with the same grain ({}): {}",
                first.name,
                first.grain_signature,
                canonical_indicator_locations(matching_grain)
            ),
            Some(first.field_path.clone()),
        ));
    }
    findings
}

fn canonical_indicator_conflict_finding(
    indicators: &[CanonicalIndicator<'_>],
) -> Option<NovaMetaFinding> {
    let unscoped = indicators
        .iter()
        .filter(|indicator| !indicator.scoped_to_grain)
        .collect::<Vec<_>>();
    if unscoped.len() <= 1 {
        return None;
    }

    let unique_grains = unscoped
        .iter()
        .map(|indicator| indicator.grain_signature.as_str())
        .collect::<BTreeSet<_>>();
    if unique_grains.len() <= 1 {
        return None;
    }

    let first = unscoped[0];
    Some(semantic_warning(
        first.target,
        "canonical_indicator_conflict",
        format!(
            "canonical indicator '{}' is declared on multiple parents with different grains: {}. Use canonical_scope: grain only for definitions that are intentionally canonical within their own grain.",
            first.name,
            canonical_indicator_locations(&unscoped)
        ),
        Some(first.field_path.clone()),
    ))
}

fn canonical_indicator_locations(indicators: &[&CanonicalIndicator<'_>]) -> String {
    indicators
        .iter()
        .map(|indicator| {
            format!(
                "{} {} ({}, grain: {})",
                indicator.target.definition.resource_kind.as_str(),
                display_resource_name(&indicator.target.definition),
                indicator.indicator_type,
                indicator.grain_signature
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn display_resource_name(definition: &ResourceDefinition) -> String {
    if let Some(parent) = definition.parent_name.as_deref() {
        format!("{parent}.{}", definition.resource_name)
    } else {
        definition.resource_name.clone()
    }
}

fn canonical_scope_is_grain(indicator: &JsonMap<String, JsonValue>) -> bool {
    indicator.get("canonical_scope").and_then(JsonValue::as_str) == Some("grain")
}

fn grain_signature(grain: Option<&JsonMap<String, JsonValue>>) -> String {
    let Some(grain) = grain else {
        return "unspecified".to_string();
    };
    let primary_key = sorted_string_array(grain.get("primary_key"));
    let dimensions = sorted_string_array(grain.get("dimensions"));
    let time_field = grain
        .get("time_field")
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("none");
    format!(
        "time={time_field}; dimensions=[{}]; primary_key=[{}]",
        dimensions.join(","),
        primary_key.join(",")
    )
}

fn sorted_string_array(value: Option<&JsonValue>) -> Vec<String> {
    let mut values = string_array(value);
    values.sort();
    values.dedup();
    values
}
