use serde_json::{Map as JsonMap, Value as JsonValue, json};

use crate::manifest::entity::{
    column_nova_meta_json, column_primary_key_bool, entity_nova_meta_json,
};
use crate::manifest::search::ManifestSearch;

use super::helpers::{array_tier_score, description_tier_score};

const DESCRIPTION_FULL_CREDIT_CHARS: usize = 100;
const DESCRIPTION_GOOD_ENOUGH_CHARS: usize = 50;
const ARRAY_FULL_CREDIT_COUNT: usize = 3;
const MAX_COLUMN_DIAGNOSTICS: usize = 12;

#[must_use]
pub(crate) fn metadata_score_scoring_contract(version: &str) -> JsonValue {
    let version = normalized_contract_version(version);
    let mut contract = json!({
        "schema_version": format!("metadata_score_contract.{version}"),
        "grade_bands": grade_bands_json(),
        "description_tiers": [
            {"min_chars": 0, "max_chars": 0, "credit_percent": 0},
            {"min_chars": 1, "max_chars": 19, "credit_percent": 20},
            {"min_chars": 20, "max_chars": 49, "credit_percent": 50},
            {"min_chars": 50, "max_chars": 99, "credit_percent": 80},
            {"min_chars": 100, "max_chars": null, "credit_percent": 100}
        ],
        "array_count_tiers": [
            {"count": 0, "credit_percent": 0},
            {"count": 1, "credit_percent": 40},
            {"count": 2, "credit_percent": 70},
            {"count": 3, "credit_percent": 100, "applies_to": "3+"}
        ],
        "grain_shape": {
            "required_type": "object",
            "canonical_fields": {
                "primary_key": "array of primary-key column names",
                "time_field": "time column name",
                "dimensions": "optional array of dimension column names"
            }
        },
        "primary_key_integrity": {
            "required_manifest_tests": ["unique", "not_null"],
            "evidence_source": "dbt manifest test metadata only",
            "warehouse_introspection": false
        }
    });

    if version == "v2" {
        contract["declared_grain"] = json!({
            "full_credit_evidence": [
                "columns.meta.primary_key",
                "meta.nova.grain.primary_key",
                "meta.nova.grain.time_field + meta.nova.grain.dimensions with matching unique/unique_combination_of_columns test"
            ],
            "primary_key_breakdown_compatibility": true
        });
        contract["resource_type_expectations"] = json!({
            "model": {
                "scores_indicators": true,
                "grain_evidence": ["primary_key", "aggregate_grain"]
            },
            "source": {
                "scores_indicators": false,
                "indicator_fields_not_applicable": ["meta.nova.measures", "meta.nova.metrics"]
            },
            "seed": {
                "scores_indicators": true
            },
            "snapshot": {
                "scores_indicators": true
            }
        });
    }

    contract
}

fn normalized_contract_version(version: &str) -> &'static str {
    match version.trim() {
        "v1" | "1" | "metadata_score_contract.v1" => "v1",
        _ => "v2",
    }
}

#[must_use]
pub(crate) fn grade_bands_json() -> JsonValue {
    json!([
        {"grade": "A", "min_score": 90, "max_score": 100},
        {"grade": "B", "min_score": 80, "max_score": 89},
        {"grade": "C", "min_score": 70, "max_score": 79},
        {"grade": "D", "min_score": 60, "max_score": 69},
        {"grade": "F", "min_score": 0, "max_score": 59}
    ])
}

pub(crate) fn build_entity_score_diagnostics(
    search: &ManifestSearch,
    unique_id: &str,
    entity_json: &JsonValue,
) -> JsonValue {
    let mut diagnostics = Vec::new();
    let nova = entity_nova_meta_json(entity_json);
    let nova = nova.as_deref();

    push_description_diagnostic(
        &mut diagnostics,
        "description",
        "documentation",
        entity_json
            .get("description")
            .and_then(JsonValue::as_str)
            .unwrap_or(""),
        30,
        "entity description",
    );

    push_entity_column_description_diagnostics(&mut diagnostics, entity_json);
    push_array_diagnostic(
        &mut diagnostics,
        "meta.nova.synonyms",
        "semantic",
        array_len(nova.and_then(|value| value.get("synonyms"))),
        12,
        "synonyms improve retrieval recall and receive full array-tier credit at three or more items",
    );
    push_array_diagnostic(
        &mut diagnostics,
        "meta.nova.domains",
        "semantic",
        array_len(nova.and_then(|value| value.get("domains"))),
        12,
        "domains improve routing and receive full array-tier credit at three or more items",
    );
    push_array_diagnostic(
        &mut diagnostics,
        "meta.nova.use_cases",
        "semantic",
        array_len(nova.and_then(|value| value.get("use_cases"))),
        12,
        "use cases help agents choose the right asset and receive full array-tier credit at three or more items",
    );
    push_array_diagnostic(
        &mut diagnostics,
        "meta.nova.governance.compliance",
        "governance",
        array_len(
            nova.and_then(|value| value.get("governance"))
                .and_then(|value| value.get("compliance")),
        ),
        20,
        "compliance tags receive partial credit for one or two items and full array-tier credit at three or more items",
    );

    push_grain_shape_diagnostics(&mut diagnostics, nova);
    push_primary_key_integrity_diagnostics(&mut diagnostics, search, unique_id, entity_json);

    JsonValue::Array(diagnostics)
}

pub(crate) fn build_column_score_diagnostics(
    column_name: &str,
    column_info: &JsonValue,
) -> JsonValue {
    let mut diagnostics = Vec::new();
    let nova = column_nova_meta_json(column_info);
    let nova = nova.as_deref();

    push_description_diagnostic(
        &mut diagnostics,
        &format!("columns.{column_name}.description"),
        "documentation",
        column_info
            .get("description")
            .and_then(JsonValue::as_str)
            .unwrap_or(""),
        50,
        "column description",
    );
    push_array_diagnostic(
        &mut diagnostics,
        &format!("columns.{column_name}.meta.nova.synonyms"),
        "semantic",
        array_len(nova.and_then(|value| value.get("synonyms"))),
        40,
        "column synonyms receive full array-tier credit at three or more items",
    );
    push_array_diagnostic(
        &mut diagnostics,
        &format!("columns.{column_name}.meta.nova.governance.compliance"),
        "governance",
        array_len(
            nova.and_then(|value| value.get("governance"))
                .and_then(|value| value.get("compliance")),
        ),
        20,
        "column compliance tags receive full array-tier credit at three or more items",
    );
    push_array_diagnostic(
        &mut diagnostics,
        &format!("columns.{column_name}.constraints"),
        "quality",
        constraint_count(column_info),
        20,
        "constraints receive full array-tier credit at three or more not_null, unique, or foreign_key entries",
    );

    JsonValue::Array(diagnostics)
}

fn push_entity_column_description_diagnostics(
    diagnostics: &mut Vec<JsonValue>,
    entity_json: &JsonValue,
) {
    let Some(columns) = entity_json.get("columns").and_then(JsonValue::as_object) else {
        return;
    };

    for (column_name, column) in columns.iter().take(MAX_COLUMN_DIAGNOSTICS) {
        push_description_diagnostic(
            diagnostics,
            &format!("columns.{column_name}.description"),
            "documentation",
            column
                .get("description")
                .and_then(JsonValue::as_str)
                .unwrap_or(""),
            100,
            "column description",
        );
    }
}

fn push_description_diagnostic(
    diagnostics: &mut Vec<JsonValue>,
    field: &str,
    category: &str,
    text: &str,
    max_points: u8,
    label: &str,
) {
    let chars = text.trim().len();
    let score = description_tier_score(text, max_points);
    if score >= max_points {
        return;
    }
    diagnostics.push(json!({
        "code": "description_tier_progress",
        "category": category,
        "field": field,
        "label": label,
        "observed_chars": chars,
        "score": score,
        "max": max_points,
        "good_enough_chars": DESCRIPTION_GOOD_ENOUGH_CHARS,
        "full_credit_chars": DESCRIPTION_FULL_CREDIT_CHARS,
        "next_threshold_chars": next_description_threshold(chars),
        "message": format!("{label} has {chars} character(s); 50-99 chars receive 80% description-tier credit and 100+ chars receive full credit")
    }));
}

fn next_description_threshold(chars: usize) -> Option<usize> {
    match chars {
        0 => Some(1),
        1..=19 => Some(20),
        20..=49 => Some(DESCRIPTION_GOOD_ENOUGH_CHARS),
        50..=99 => Some(DESCRIPTION_FULL_CREDIT_CHARS),
        _ => None,
    }
}

fn push_array_diagnostic(
    diagnostics: &mut Vec<JsonValue>,
    field: &str,
    category: &str,
    count: usize,
    max_points: u8,
    message: &str,
) {
    if count >= ARRAY_FULL_CREDIT_COUNT {
        return;
    }
    diagnostics.push(json!({
        "code": "array_tier_progress",
        "category": category,
        "field": field,
        "count": count,
        "score": array_tier_score(count, max_points),
        "max": max_points,
        "next_useful_count": next_array_count(count),
        "full_credit_count": ARRAY_FULL_CREDIT_COUNT,
        "message": message
    }));
}

fn next_array_count(count: usize) -> usize {
    match count {
        0 => 1,
        1 => 2,
        _ => ARRAY_FULL_CREDIT_COUNT,
    }
}

fn push_grain_shape_diagnostics(diagnostics: &mut Vec<JsonValue>, nova: Option<&JsonValue>) {
    let Some(nova) = nova else {
        return;
    };
    push_grain_shape_diagnostic(diagnostics, "meta.nova.grain", nova.get("grain"));
    push_grain_shape_diagnostic(
        diagnostics,
        "meta.nova.metric.grain",
        nova.get("metric").and_then(|metric| metric.get("grain")),
    );
    if let Some(metrics) = nova.get("metrics").and_then(JsonValue::as_array) {
        for (index, metric) in metrics.iter().enumerate() {
            let label = metric
                .get("name")
                .and_then(JsonValue::as_str)
                .map_or_else(|| index.to_string(), ToString::to_string);
            push_grain_shape_diagnostic(
                diagnostics,
                &format!("meta.nova.metrics[{label}].grain"),
                metric.get("grain"),
            );
        }
    }
}

fn push_grain_shape_diagnostic(
    diagnostics: &mut Vec<JsonValue>,
    field: &str,
    grain: Option<&JsonValue>,
) {
    let Some(grain) = grain else {
        return;
    };
    let invalid_reason = match grain {
        JsonValue::Object(map) if map.is_empty() => Some("empty object"),
        JsonValue::Object(map) if !has_known_grain_field(map) => {
            Some("object without canonical grain fields")
        }
        JsonValue::Object(_) => None,
        JsonValue::String(_) => Some("string"),
        JsonValue::Array(_) => Some("array"),
        JsonValue::Bool(_) => Some("boolean"),
        JsonValue::Number(_) => Some("number"),
        JsonValue::Null => Some("null"),
    };
    let Some(invalid_reason) = invalid_reason else {
        return;
    };
    diagnostics.push(json!({
        "code": "invalid_grain_shape",
        "category": "semantic",
        "field": field,
        "observed_type": json_type_label(grain),
        "invalid_reason": invalid_reason,
        "expected_shape": {
            "primary_key": ["order_id"],
            "time_field": "order_date",
            "dimensions": ["country_code"]
        },
        "message": format!("{field} must be a non-empty object with canonical grain fields; {invalid_reason} values score as missing")
    }));
}

fn has_known_grain_field(map: &JsonMap<String, JsonValue>) -> bool {
    ["primary_key", "time_field", "dimensions"]
        .iter()
        .any(|field| map.contains_key(*field))
}

fn json_type_label(value: &JsonValue) -> &'static str {
    match value {
        JsonValue::Null => "null",
        JsonValue::Bool(_) => "boolean",
        JsonValue::Number(_) => "number",
        JsonValue::String(_) => "string",
        JsonValue::Array(_) => "array",
        JsonValue::Object(_) => "object",
    }
}

fn push_primary_key_integrity_diagnostics(
    diagnostics: &mut Vec<JsonValue>,
    search: &ManifestSearch,
    unique_id: &str,
    entity_json: &JsonValue,
) {
    let Some(columns) = entity_json.get("columns").and_then(JsonValue::as_object) else {
        return;
    };
    for (column_name, column) in columns {
        if !column_primary_key_bool(column) {
            continue;
        }
        let missing_tests = missing_primary_key_tests(search, unique_id, column_name);
        if missing_tests.is_empty() {
            continue;
        }
        diagnostics.push(json!({
            "code": "primary_key_integrity_missing_tests",
            "category": "quality",
            "field": format!("columns.{column_name}"),
            "column": column_name,
            "missing_tests": missing_tests,
            "required_tests": ["unique", "not_null"],
            "evidence_source": "dbt manifest test metadata only",
            "warehouse_introspection": false,
            "message": format!("primary key column '{column_name}' is missing required dbt manifest test evidence; Nova does not infer uniqueness from compiled SQL or warehouse introspection")
        }));
    }
}

fn missing_primary_key_tests(
    search: &ManifestSearch,
    unique_id: &str,
    column_name: &str,
) -> Vec<&'static str> {
    let key = format!("{unique_id}:{column_name}");
    let tests = search
        .tests_by_column
        .get(&key)
        .cloned()
        .unwrap_or_default();
    let mut has_unique = false;
    let mut has_not_null = false;
    for test_id in &tests {
        if let Ok(Some(test)) = search.get_entity_archived(test_id) {
            let test_json = test.to_json_value();
            let name = test_json
                .get("test_metadata")
                .and_then(|metadata| metadata.get("name"))
                .and_then(JsonValue::as_str);
            if name == Some("unique") {
                has_unique = true;
            } else if name == Some("not_null") {
                has_not_null = true;
            }
        }
    }

    let mut missing = Vec::new();
    if !has_unique {
        missing.push("unique");
    }
    if !has_not_null {
        missing.push("not_null");
    }
    missing
}

fn array_len(value: Option<&JsonValue>) -> usize {
    value
        .and_then(JsonValue::as_array)
        .map_or(0, std::vec::Vec::len)
}

fn constraint_count(column: &JsonValue) -> usize {
    column
        .get("constraints")
        .and_then(JsonValue::as_array)
        .map_or(0, |constraints| {
            constraints
                .iter()
                .filter(|constraint| {
                    constraint
                        .get("type")
                        .and_then(JsonValue::as_str)
                        .is_some_and(|kind| matches!(kind, "not_null" | "unique" | "foreign_key"))
                })
                .count()
        })
}
