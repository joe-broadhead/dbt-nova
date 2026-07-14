use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value as JsonValue;

use crate::error::Result;
use crate::manifest::entity::{Entity, NovaMeta, normalized_entity_meta_json};
use crate::manifest::search::ManifestSearch;
use crate::params::DiffEntitiesParams;
use crate::responses::SuccessResponse;
use crate::tools::helpers::test_type_from_json;
use tracing::instrument;

const DIFF_ALL_FIELDS: &[&str] = &[
    "columns",
    "tags",
    "grain",
    "indicators",
    "governance",
    "tests",
];

#[derive(Clone)]
struct IndicatorSnapshot {
    name: String,
    indicator_type: &'static str,
    expression: Option<String>,
    canonical: bool,
}

impl ManifestSearch {
    /// Diff two entities across selected fields.
    ///
    /// # Errors
    /// Returns an error if either entity cannot be resolved.
    #[instrument(skip(self, params), fields(tool = "diff_entities", entity1 = %params.entity1, entity2 = %params.entity2))]
    #[allow(clippy::too_many_lines)]
    pub async fn diff_entities(&self, params: &DiffEntitiesParams) -> Result<JsonValue> {
        let resource_type1 = params.entity1_resource_type.as_deref();
        let resource_type2 = params.entity2_resource_type.as_deref();
        let key1 = self.resolve_single_id(&params.entity1, resource_type1)?;
        let key2 = self.resolve_single_id(&params.entity2, resource_type2)?;
        let entity1 = self
            .get_entity(&key1)
            .await?
            .ok_or_else(|| self.entity_not_found(&key1, resource_type1))?;
        let entity2 = self
            .get_entity(&key2)
            .await?
            .ok_or_else(|| self.entity_not_found(&key2, resource_type2))?;

        let entity1_json = entity1.to_json_value();
        let entity2_json = entity2.to_json_value();
        let mut diffs = serde_json::Map::new();

        let mut summary = serde_json::Map::new();
        let compare_fields = expanded_compare_fields(&params.compare_fields);
        for field in &compare_fields {
            match field.as_str() {
                "columns" => {
                    let cols1 = string_set(entity1.column_names());
                    let cols2 = string_set(entity2.column_names());
                    let (diff, counts) = set_diff_json(&cols1, &cols2);

                    diffs.insert("columns".to_string(), diff);
                    summary.insert(
                        "columns".to_string(),
                        serde_json::json!({
                            "only_in_first": counts.only_in_first,
                            "only_in_second": counts.only_in_second,
                            "in_both": counts.in_both
                        }),
                    );
                }
                "tags" => {
                    let tags1 = string_set(entity1.tags.iter().cloned());
                    let tags2 = string_set(entity2.tags.iter().cloned());
                    let (diff, counts) = set_diff_json(&tags1, &tags2);

                    diffs.insert("tags".to_string(), diff);
                    summary.insert(
                        "tags".to_string(),
                        serde_json::json!({
                            "only_in_first": counts.only_in_first,
                            "only_in_second": counts.only_in_second,
                            "in_both": counts.in_both
                        }),
                    );
                }
                "grain" => {
                    let (diff, summary_value) =
                        diff_grain(entity1.nova_meta.as_ref(), entity2.nova_meta.as_ref());
                    diffs.insert("grain".to_string(), diff);
                    summary.insert("grain".to_string(), summary_value);
                }
                "indicators" => {
                    let (diff, summary_value) =
                        diff_indicators(entity1.nova_meta.as_ref(), entity2.nova_meta.as_ref());
                    diffs.insert("indicators".to_string(), diff);
                    summary.insert("indicators".to_string(), summary_value);
                }
                "governance" => {
                    let (diff, summary_value) = diff_governance(
                        entity1.nova_meta.as_ref(),
                        entity2.nova_meta.as_ref(),
                        &entity1_json,
                        &entity2_json,
                    );
                    diffs.insert("governance".to_string(), diff);
                    summary.insert("governance".to_string(), summary_value);
                }
                "tests" => {
                    let (diff, summary_value) =
                        self.diff_tests(&key1, &key2, &entity1, &entity2)?;
                    diffs.insert("tests".to_string(), diff);
                    summary.insert("tests".to_string(), summary_value);
                }
                _ => {
                    let val1 = entity1_json.get(field);
                    let val2 = entity2_json.get(field);
                    diffs.insert(
                        field.clone(),
                        serde_json::json!({
                            "first": val1,
                            "second": val2,
                            "equal": val1 == val2
                        }),
                    );
                }
            }
        }

        Ok(serde_json::to_value(SuccessResponse::new(
            serde_json::json!({
                "entity1": { "unique_id": &key1, "name": entity1.name },
                "entity2": { "unique_id": &key2, "name": entity2.name },
                "summary": summary,
                "differences": diffs
            }),
            1,
        ))?)
    }

    fn diff_tests(
        &self,
        key1: &str,
        key2: &str,
        entity1: &Entity,
        entity2: &Entity,
    ) -> Result<(JsonValue, JsonValue)> {
        let counts1 = self.test_counts_by_type(key1)?;
        let counts2 = self.test_counts_by_type(key2)?;
        let total1 = counts1.values().sum::<usize>();
        let total2 = counts2.values().sum::<usize>();
        let shared_columns = string_set(entity1.column_names())
            .intersection(&string_set(entity2.column_names()))
            .cloned()
            .collect::<Vec<_>>();
        let mut shared_column_differences = Vec::new();

        for column in shared_columns {
            let tests1 = self.test_types_for_column(key1, &column)?;
            let tests2 = self.test_types_for_column(key2, &column)?;
            let only_in_first = sorted_difference(&tests1, &tests2);
            let only_in_second = sorted_difference(&tests2, &tests1);
            if !only_in_first.is_empty() || !only_in_second.is_empty() {
                shared_column_differences.push(serde_json::json!({
                    "column": column,
                    "only_in_first": only_in_first,
                    "only_in_second": only_in_second
                }));
            }
        }

        let diff = serde_json::json!({
            "entity1_total": total1,
            "entity2_total": total2,
            "by_type": {
                "entity1": counts1,
                "entity2": counts2
            },
            "shared_column_differences": shared_column_differences
        });
        let summary = serde_json::json!({
            "entity1_total": total1,
            "entity2_total": total2,
            "shared_column_differences": shared_column_differences.len()
        });

        Ok((diff, summary))
    }

    fn test_counts_by_type(&self, entity_id: &str) -> Result<BTreeMap<String, usize>> {
        let mut counts = BTreeMap::new();
        for test_id in self.tests_by_entity.get(entity_id).into_iter().flatten() {
            if let Some(test) = self.get_entity_archived(test_id)? {
                let test_json = test.to_json_value();
                *counts
                    .entry(test_type_from_json(&test_json).to_string())
                    .or_default() += 1;
            }
        }
        Ok(counts)
    }

    fn test_types_for_column(&self, entity_id: &str, column: &str) -> Result<BTreeSet<String>> {
        let mut types = BTreeSet::new();
        let key = format!("{entity_id}:{column}");
        for test_id in self.tests_by_column.get(&key).into_iter().flatten() {
            if let Some(test) = self.get_entity_archived(test_id)? {
                let test_json = test.to_json_value();
                types.insert(test_type_from_json(&test_json).to_string());
            }
        }
        Ok(types)
    }
}

#[derive(Clone, Copy)]
struct SetDiffCounts {
    only_in_first: usize,
    only_in_second: usize,
    in_both: usize,
}

fn expanded_compare_fields(fields: &[String]) -> Vec<String> {
    if fields.is_empty() {
        return vec!["columns".to_string()];
    }

    let mut expanded = Vec::new();
    let mut seen = BTreeSet::new();
    for field in fields {
        if field == "all" {
            for built_in in DIFF_ALL_FIELDS {
                push_unique(&mut expanded, &mut seen, built_in);
            }
        } else {
            push_unique(&mut expanded, &mut seen, field);
        }
    }
    expanded
}

fn push_unique(expanded: &mut Vec<String>, seen: &mut BTreeSet<String>, field: &str) {
    if seen.insert(field.to_string()) {
        expanded.push(field.to_string());
    }
}

fn string_set<I>(values: I) -> BTreeSet<String>
where
    I: IntoIterator<Item = String>,
{
    values.into_iter().collect()
}

fn sorted_difference(left: &BTreeSet<String>, right: &BTreeSet<String>) -> Vec<String> {
    left.difference(right).cloned().collect()
}

fn sorted_intersection(left: &BTreeSet<String>, right: &BTreeSet<String>) -> Vec<String> {
    left.intersection(right).cloned().collect()
}

fn set_diff_json(left: &BTreeSet<String>, right: &BTreeSet<String>) -> (JsonValue, SetDiffCounts) {
    let only_in_first = sorted_difference(left, right);
    let only_in_second = sorted_difference(right, left);
    let in_both = sorted_intersection(left, right);
    let counts = SetDiffCounts {
        only_in_first: only_in_first.len(),
        only_in_second: only_in_second.len(),
        in_both: in_both.len(),
    };

    (
        serde_json::json!({
            "only_in_first": only_in_first,
            "only_in_second": only_in_second,
            "in_both": in_both,
            "counts": {
                "only_in_first": counts.only_in_first,
                "only_in_second": counts.only_in_second,
                "in_both": counts.in_both
            }
        }),
        counts,
    )
}

fn diff_grain(nova1: Option<&NovaMeta>, nova2: Option<&NovaMeta>) -> (JsonValue, JsonValue) {
    let grain1 = nova1.and_then(|nova| nova.grain.as_ref());
    let grain2 = nova2.and_then(|nova| nova.grain.as_ref());
    let time1 = grain1.and_then(|grain| grain.time_field.as_deref());
    let time2 = grain2.and_then(|grain| grain.time_field.as_deref());
    let dimensions1 = string_set(
        grain1
            .map(|grain| grain.dimensions.clone())
            .unwrap_or_default(),
    );
    let dimensions2 = string_set(
        grain2
            .map(|grain| grain.dimensions.clone())
            .unwrap_or_default(),
    );
    let primary_key1 = string_set(
        grain1
            .map(|grain| grain.primary_key.clone())
            .unwrap_or_default(),
    );
    let primary_key2 = string_set(
        grain2
            .map(|grain| grain.primary_key.clone())
            .unwrap_or_default(),
    );
    let (dimensions_diff, dimensions_counts) = set_diff_json(&dimensions1, &dimensions2);
    let (primary_key_diff, primary_key_counts) = set_diff_json(&primary_key1, &primary_key2);
    let same_time_field = time1 == time2;

    (
        serde_json::json!({
            "same_time_field": same_time_field,
            "entity1_time_field": time1,
            "entity2_time_field": time2,
            "entity1_dimensions": sorted_strings(&dimensions1),
            "entity2_dimensions": sorted_strings(&dimensions2),
            "shared": sorted_intersection(&dimensions1, &dimensions2),
            "only_in_first": sorted_difference(&dimensions1, &dimensions2),
            "only_in_second": sorted_difference(&dimensions2, &dimensions1),
            "dimensions": dimensions_diff,
            "primary_key": primary_key_diff
        }),
        serde_json::json!({
            "entity1_has_grain": grain1.is_some(),
            "entity2_has_grain": grain2.is_some(),
            "same_time_field": same_time_field,
            "shared_dimensions": dimensions_counts.in_both,
            "only_in_first_dimensions": dimensions_counts.only_in_first,
            "only_in_second_dimensions": dimensions_counts.only_in_second,
            "shared_primary_key": primary_key_counts.in_both,
            "only_in_first_primary_key": primary_key_counts.only_in_first,
            "only_in_second_primary_key": primary_key_counts.only_in_second
        }),
    )
}

fn sorted_strings(values: &BTreeSet<String>) -> Vec<String> {
    values.iter().cloned().collect()
}

fn diff_indicators(nova1: Option<&NovaMeta>, nova2: Option<&NovaMeta>) -> (JsonValue, JsonValue) {
    let left_map = collect_indicators(nova1);
    let right_map = collect_indicators(nova2);
    let names1 = left_map.keys().cloned().collect::<BTreeSet<_>>();
    let names2 = right_map.keys().cloned().collect::<BTreeSet<_>>();
    let in_both_keys = sorted_intersection(&names1, &names2);
    let only_in_first = sorted_difference(&names1, &names2)
        .into_iter()
        .filter_map(|key| left_map.get(&key).map(|snapshot| snapshot.name.clone()))
        .collect::<Vec<_>>();
    let only_in_second = sorted_difference(&names2, &names1)
        .into_iter()
        .filter_map(|key| right_map.get(&key).map(|snapshot| snapshot.name.clone()))
        .collect::<Vec<_>>();
    let mut in_both = Vec::new();
    let mut canonical_in_both = Vec::new();
    let mut expression_conflicts = Vec::new();

    for key in &in_both_keys {
        let Some(left_snapshot) = left_map.get(key) else {
            continue;
        };
        let Some(right_snapshot) = right_map.get(key) else {
            continue;
        };
        in_both.push(left_snapshot.name.clone());
        if left_snapshot.canonical && right_snapshot.canonical {
            canonical_in_both.push(left_snapshot.name.clone());
        }
        if let (Some(expr1), Some(expr2)) = (&left_snapshot.expression, &right_snapshot.expression)
            && normalize_indicator_expression(expr1) != normalize_indicator_expression(expr2)
        {
            expression_conflicts.push(serde_json::json!({
                "name": left_snapshot.name,
                "type1": left_snapshot.indicator_type,
                "type2": right_snapshot.indicator_type,
                "expr1": expr1,
                "expr2": expr2
            }));
        }
    }

    let near_duplicates = identical_expression_near_duplicates(&left_map, &right_map);

    (
        serde_json::json!({
            "in_both": in_both,
            "only_in_first": only_in_first,
            "only_in_second": only_in_second,
            "canonical_in_both": canonical_in_both,
            "expression_conflicts": expression_conflicts,
            "near_duplicates": near_duplicates,
            "counts": {
                "in_both": in_both_keys.len(),
                "only_in_first": only_in_first.len(),
                "only_in_second": only_in_second.len(),
                "canonical_in_both": canonical_in_both.len(),
                "expression_conflicts": expression_conflicts.len(),
                "near_duplicates": near_duplicates.len()
            }
        }),
        serde_json::json!({
            "in_both": in_both_keys.len(),
            "only_in_first": only_in_first.len(),
            "only_in_second": only_in_second.len(),
            "canonical_in_both": canonical_in_both.len(),
            "expression_conflicts": expression_conflicts.len(),
            "near_duplicates": near_duplicates.len()
        }),
    )
}

fn collect_indicators(nova: Option<&NovaMeta>) -> BTreeMap<String, IndicatorSnapshot> {
    let mut indicators = BTreeMap::new();
    let Some(nova) = nova else {
        return indicators;
    };

    for measure in &nova.measures {
        indicators.insert(
            normalize_indicator_name(&measure.name),
            IndicatorSnapshot {
                name: measure.name.clone(),
                indicator_type: "measure",
                expression: measure.expression.clone().or_else(|| measure.field.clone()),
                canonical: measure.canonical,
            },
        );
    }
    if let Some(metric) = nova.metric.as_ref() {
        indicators.insert(
            normalize_indicator_name(&metric.name),
            IndicatorSnapshot {
                name: metric.name.clone(),
                indicator_type: "metric",
                expression: metric.expression.clone(),
                canonical: metric.canonical,
            },
        );
    }
    for metric in &nova.metrics {
        indicators.insert(
            normalize_indicator_name(&metric.name),
            IndicatorSnapshot {
                name: metric.name.clone(),
                indicator_type: "metric",
                expression: metric.expression.clone(),
                canonical: metric.canonical,
            },
        );
    }

    indicators
}

fn identical_expression_near_duplicates(
    indicators1: &BTreeMap<String, IndicatorSnapshot>,
    indicators2: &BTreeMap<String, IndicatorSnapshot>,
) -> Vec<JsonValue> {
    let mut rows = Vec::new();
    let mut seen = BTreeSet::new();
    for (key1, indicator1) in indicators1 {
        let Some(expr1) = indicator1.expression.as_ref() else {
            continue;
        };
        let normalized1 = normalize_indicator_expression(expr1);
        if normalized1.is_empty() {
            continue;
        }
        for (key2, indicator2) in indicators2 {
            if key1 == key2 {
                continue;
            }
            let Some(expr2) = indicator2.expression.as_ref() else {
                continue;
            };
            if normalized1 != normalize_indicator_expression(expr2) {
                continue;
            }
            let pair_key = format!("{key1}\u{0}{key2}");
            if !seen.insert(pair_key) {
                continue;
            }
            rows.push(serde_json::json!({
                "name1": indicator1.name,
                "name2": indicator2.name,
                "type1": indicator1.indicator_type,
                "type2": indicator2.indicator_type,
                "reason": "identical_expression",
                "expression": expr1
            }));
        }
    }
    rows
}

fn normalize_indicator_name(name: &str) -> String {
    name.trim().to_ascii_lowercase()
}

fn normalize_indicator_expression(expression: &str) -> String {
    expression
        .chars()
        .filter(|ch| !ch.is_whitespace() && !matches!(ch, '`' | '"' | '\''))
        .flat_map(char::to_lowercase)
        .collect()
}

fn diff_governance(
    nova1: Option<&NovaMeta>,
    nova2: Option<&NovaMeta>,
    entity1_json: &JsonValue,
    entity2_json: &JsonValue,
) -> (JsonValue, JsonValue) {
    let sensitivity1 = nova1
        .and_then(|nova| nova.governance.as_ref())
        .and_then(|governance| governance.sensitivity.as_deref());
    let sensitivity2 = nova2
        .and_then(|nova| nova.governance.as_ref())
        .and_then(|governance| governance.sensitivity.as_deref());
    let pii1 = nova1
        .and_then(|nova| nova.governance.as_ref())
        .and_then(|governance| governance.pii.as_deref());
    let pii2 = nova2
        .and_then(|nova| nova.governance.as_ref())
        .and_then(|governance| governance.pii.as_deref());
    let compliance1 = string_set(
        nova1
            .and_then(|nova| nova.governance.as_ref())
            .map(|governance| governance.compliance.clone())
            .unwrap_or_default(),
    );
    let compliance2 = string_set(
        nova2
            .and_then(|nova| nova.governance.as_ref())
            .map(|governance| governance.compliance.clone())
            .unwrap_or_default(),
    );
    let owner1 =
        normalized_entity_meta_json(entity1_json).and_then(|meta| meta.get("owner").cloned());
    let owner2 =
        normalized_entity_meta_json(entity2_json).and_then(|meta| meta.get("owner").cloned());
    let (compliance_diff, compliance_counts) = set_diff_json(&compliance1, &compliance2);
    let changed_fields = usize::from(sensitivity1 != sensitivity2)
        + usize::from(pii1 != pii2)
        + usize::from(owner1 != owner2)
        + usize::from(compliance_counts.only_in_first > 0 || compliance_counts.only_in_second > 0);

    (
        serde_json::json!({
            "sensitivity": {
                "first": sensitivity1,
                "second": sensitivity2,
                "equal": sensitivity1 == sensitivity2
            },
            "pii": {
                "first": pii1,
                "second": pii2,
                "equal": pii1 == pii2
            },
            "owner": {
                "first": owner1,
                "second": owner2,
                "equal": owner1 == owner2
            },
            "compliance": compliance_diff
        }),
        serde_json::json!({
            "changed_fields": changed_fields,
            "same_sensitivity": sensitivity1 == sensitivity2,
            "same_pii": pii1 == pii2,
            "same_owner": owner1 == owner2,
            "shared_compliance": compliance_counts.in_both,
            "only_in_first_compliance": compliance_counts.only_in_first,
            "only_in_second_compliance": compliance_counts.only_in_second
        }),
    )
}
