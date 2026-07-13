use serde_json::Value as JsonValue;

use crate::manifest::entity::{
    column_nova_meta_json, column_primary_key_bool, entity_nova_meta_json,
};
use crate::manifest::search::ManifestSearch;

use crate::tools::metadata_score::CategoryBreakdown;
use crate::tools::metadata_score::helpers::{
    array_tier_score, expects_columns, push_recommendation,
};

impl ManifestSearch {
    pub(crate) fn score_quality(
        &self,
        unique_id: &str,
        entity_json: &JsonValue,
        resource_type: Option<&str>,
        include_breakdown: bool,
        include_recommendations: bool,
        recommendations: &mut Vec<JsonValue>,
    ) -> CategoryBreakdown {
        let columns = columns_from_entity(entity_json);
        let expects_columns_flag = expects_columns(resource_type);
        let tests = summarize_tests(self, unique_id, &columns, expects_columns_flag);
        let pk = summarize_primary_keys(self, unique_id, entity_json, &columns);
        let constraints = summarize_constraints(&columns);
        apply_quality_recommendations(
            &tests,
            &pk,
            &constraints,
            expects_columns_flag,
            include_recommendations,
            recommendations,
        );

        let score = total_quality_score(&tests, &pk, &constraints);
        let breakdown = build_quality_breakdown(&tests, &pk, &constraints, include_breakdown);
        let summary = build_quality_summary(&tests);

        CategoryBreakdown {
            score,
            breakdown,
            summary,
        }
    }
}

struct TestSummary {
    test_score: u8,
    model_test_score: u8,
    model_tests: usize,
    column_tests: usize,
    has_tests: bool,
    baseline_score: u8,
    critical_score: u8,
    dimension_score: u8,
    critical_coverage_percent: u8,
    dimension_coverage_percent: u8,
    critical_columns: usize,
    critical_columns_tested: usize,
    dimension_columns: usize,
    dimension_columns_tested: usize,
}

struct PrimaryKeySummary {
    pk_columns: Vec<String>,
    declared_grain_columns: Vec<String>,
    evidence: &'static str,
    pk_score: u8,
    pk_integrity_score: u8,
    has_pk: bool,
    pk_integrity: bool,
}

struct ConstraintSummary {
    score: u8,
    count: usize,
}

fn total_quality_score(
    tests: &TestSummary,
    pk: &PrimaryKeySummary,
    constraints: &ConstraintSummary,
) -> u8 {
    tests.test_score + pk.pk_score + pk.pk_integrity_score + constraints.score
}

fn build_quality_breakdown(
    tests: &TestSummary,
    pk: &PrimaryKeySummary,
    constraints: &ConstraintSummary,
    include_breakdown: bool,
) -> JsonValue {
    if !include_breakdown {
        return JsonValue::Null;
    }

    serde_json::json!({
        "tests": {
            "score": tests.test_score,
            "max": 30,
            "present": tests.has_tests,
            "baseline": tests.baseline_score,
            "critical_coverage_percent": tests.critical_coverage_percent,
            "dimension_coverage_percent": tests.dimension_coverage_percent,
            "critical_columns": tests.critical_columns,
            "critical_columns_tested": tests.critical_columns_tested,
            "dimension_columns": tests.dimension_columns,
            "dimension_columns_tested": tests.dimension_columns_tested
        },
        "model_test_coverage": {
            "score": tests.model_test_score,
            "max": 10,
            "model_tests": tests.model_tests,
            "column_tests": tests.column_tests
        },
        "primary_key": {
            "score": pk.pk_score,
            "max": 20,
            "present": pk.has_pk,
            "columns": pk.pk_columns,
            "evidence": pk.evidence
        },
        "declared_grain": {
            "score": pk.pk_score,
            "max": 20,
            "present": pk.has_pk,
            "columns": pk.declared_grain_columns,
            "evidence": pk.evidence
        },
        "primary_key_integrity": {
            "score": pk.pk_integrity_score,
            "max": 10,
            "ok": pk.pk_integrity,
            "evidence": pk.evidence
        },
        "constraints": {
            "score": constraints.score,
            "max": 10,
            "count": constraints.count
        }
    })
}

fn build_quality_summary(tests: &TestSummary) -> JsonValue {
    serde_json::json!({
        "test_coverage": {
            "baseline": tests.baseline_score,
            "critical_coverage_percent": tests.critical_coverage_percent,
            "dimension_coverage_percent": tests.dimension_coverage_percent,
            "critical_columns": tests.critical_columns,
            "critical_columns_tested": tests.critical_columns_tested,
            "dimension_columns": tests.dimension_columns,
            "dimension_columns_tested": tests.dimension_columns_tested
        }
    })
}

fn apply_quality_recommendations(
    tests: &TestSummary,
    pk: &PrimaryKeySummary,
    constraints: &ConstraintSummary,
    expects_columns: bool,
    include_recommendations: bool,
    recommendations: &mut Vec<JsonValue>,
) {
    if !include_recommendations {
        return;
    }
    recommend_test_coverage(tests, recommendations);
    recommend_primary_keys(pk, expects_columns, recommendations);
    recommend_constraints(constraints, expects_columns, recommendations);
}

fn recommend_test_coverage(tests: &TestSummary, recommendations: &mut Vec<JsonValue>) {
    if tests.test_score == 0 {
        push_recommendation(
            recommendations,
            "quality",
            30,
            "Add tests for this entity".to_string(),
            "tests",
        );
        return;
    }
    if tests.critical_score < 20 {
        push_recommendation(
            recommendations,
            "quality",
            20 - tests.critical_score,
            "Add tests for critical columns (identifier, measure, time)".to_string(),
            "tests",
        );
    }
    if tests.dimension_columns > 0 && tests.dimension_score < 10 {
        push_recommendation(
            recommendations,
            "quality",
            10 - tests.dimension_score,
            "Add tests for key dimensions used in analysis".to_string(),
            "tests",
        );
    }
    if tests.model_test_score < 10 {
        push_recommendation(
            recommendations,
            "quality",
            10 - tests.model_test_score,
            "Add more model-level data tests".to_string(),
            "tests",
        );
    }
}

fn recommend_primary_keys(
    pk: &PrimaryKeySummary,
    expects_columns: bool,
    recommendations: &mut Vec<JsonValue>,
) {
    if pk.pk_score == 0 && expects_columns {
        push_recommendation(
            recommendations,
            "quality",
            20,
            "Define primary key column(s) using meta.primary_key".to_string(),
            "columns.meta.primary_key",
        );
    }
    if pk.has_pk && !pk.pk_integrity {
        push_recommendation(
            recommendations,
            "quality",
            10,
            "Add unique + not_null tests to primary key columns".to_string(),
            "tests",
        );
    }
}

fn recommend_constraints(
    constraints: &ConstraintSummary,
    expects_columns: bool,
    recommendations: &mut Vec<JsonValue>,
) {
    if constraints.score < 10 && expects_columns {
        push_recommendation(
            recommendations,
            "quality",
            10 - constraints.score,
            "Add constraints (not_null/unique/foreign_key) to columns".to_string(),
            "columns.constraints",
        );
    }
}

fn columns_from_entity(entity_json: &JsonValue) -> serde_json::Map<String, JsonValue> {
    entity_json
        .get("columns")
        .and_then(|c| c.as_object())
        .cloned()
        .unwrap_or_default()
}

fn summarize_tests(
    search: &ManifestSearch,
    unique_id: &str,
    columns: &serde_json::Map<String, JsonValue>,
    expects_columns: bool,
) -> TestSummary {
    let tests = search
        .tests_by_entity
        .get(unique_id)
        .cloned()
        .unwrap_or_default();
    let (model_level_tests, column_tests) = classify_tests(search, &tests);
    let has_tests = !(model_level_tests.is_empty() && column_tests.is_empty());
    let baseline_score = if has_tests { 10 } else { 0 };
    let model_test_score = array_tier_score(model_level_tests.len(), 10);
    let coverage = summarize_test_coverage(search, unique_id, columns, expects_columns);
    let test_score = (baseline_score + coverage.critical_score + coverage.dimension_score).min(30);

    TestSummary {
        test_score,
        model_test_score,
        model_tests: model_level_tests.len(),
        column_tests: column_tests.len(),
        has_tests,
        baseline_score,
        critical_score: coverage.critical_score,
        dimension_score: coverage.dimension_score,
        critical_coverage_percent: coverage.critical_coverage_percent,
        dimension_coverage_percent: coverage.dimension_coverage_percent,
        critical_columns: coverage.critical_columns,
        critical_columns_tested: coverage.critical_columns_tested,
        dimension_columns: coverage.dimension_columns,
        dimension_columns_tested: coverage.dimension_columns_tested,
    }
}

struct CoverageSummary {
    critical_score: u8,
    dimension_score: u8,
    critical_coverage_percent: u8,
    dimension_coverage_percent: u8,
    critical_columns: usize,
    critical_columns_tested: usize,
    dimension_columns: usize,
    dimension_columns_tested: usize,
}

#[allow(clippy::cast_precision_loss)]
fn summarize_test_coverage(
    search: &ManifestSearch,
    unique_id: &str,
    columns: &serde_json::Map<String, JsonValue>,
    expects_columns: bool,
) -> CoverageSummary {
    let mut critical_weight_total = 0.0f32;
    let mut critical_weight_tested = 0.0f32;
    let mut critical_columns = 0usize;
    let mut critical_columns_tested = 0usize;
    let mut dimension_columns = 0usize;
    let mut dimension_columns_tested = 0usize;

    for (name, col) in columns {
        let role = column_role(col);
        let has_tests = column_has_tests(search, unique_id, name);
        match role.as_deref() {
            Some("identifier" | "measure") => {
                critical_weight_total += 3.0;
                critical_columns += 1;
                if has_tests {
                    critical_weight_tested += 3.0;
                    critical_columns_tested += 1;
                }
            }
            Some("time") => {
                critical_weight_total += 2.0;
                critical_columns += 1;
                if has_tests {
                    critical_weight_tested += 2.0;
                    critical_columns_tested += 1;
                }
            }
            Some("dimension") => {
                dimension_columns += 1;
                if has_tests {
                    dimension_columns_tested += 1;
                }
            }
            _ => {}
        }
    }

    let critical_coverage_percent = if critical_weight_total > 0.0 {
        crate::tools::metadata_score::helpers::clamp_to_u8(
            (critical_weight_tested / critical_weight_total) * 100.0,
            100,
        )
    } else if expects_columns {
        0
    } else {
        100
    };

    let dimension_coverage_percent = if dimension_columns > 0 {
        crate::tools::metadata_score::helpers::clamp_to_u8(
            (dimension_columns_tested as f32 / dimension_columns as f32) * 100.0,
            100,
        )
    } else if expects_columns {
        0
    } else {
        100
    };

    let critical_score =
        crate::tools::metadata_score::helpers::percent_score(critical_coverage_percent, 20);
    let dimension_score =
        crate::tools::metadata_score::helpers::percent_score(dimension_coverage_percent, 10);

    CoverageSummary {
        critical_score,
        dimension_score,
        critical_coverage_percent,
        dimension_coverage_percent,
        critical_columns,
        critical_columns_tested,
        dimension_columns,
        dimension_columns_tested,
    }
}

fn column_has_tests(search: &ManifestSearch, unique_id: &str, column_name: &str) -> bool {
    let key = format!("{unique_id}:{column_name}");
    search
        .tests_by_column
        .get(&key)
        .is_some_and(|tests| !tests.is_empty())
}

fn column_role(col: &JsonValue) -> Option<String> {
    let is_pk = column_primary_key_bool(col);
    let role = column_nova_meta_json(col)
        .as_deref()
        .and_then(|n| n.get("role"))
        .and_then(JsonValue::as_str)
        .map(str::to_string);
    if role.is_none() && is_pk {
        return Some("identifier".to_string());
    }
    role
}

fn summarize_primary_keys(
    search: &ManifestSearch,
    unique_id: &str,
    entity_json: &JsonValue,
    columns: &serde_json::Map<String, JsonValue>,
) -> PrimaryKeySummary {
    let (column_pk_columns, has_column_pk) = primary_keys(columns);
    if has_column_pk {
        return primary_key_summary(
            search,
            unique_id,
            column_pk_columns,
            "columns.meta.primary_key",
        );
    }

    if let Some(grain_pk_columns) = nova_grain_primary_keys(entity_json) {
        return primary_key_summary(
            search,
            unique_id,
            grain_pk_columns,
            "meta.nova.grain.primary_key",
        );
    }

    if let Some(grain_columns) = aggregate_grain_columns(entity_json)
        && unique_test_covers_columns(search, unique_id, &grain_columns)
    {
        return PrimaryKeySummary {
            pk_columns: grain_columns.clone(),
            declared_grain_columns: grain_columns,
            evidence: "meta.nova.grain.time_field_dimensions_unique_test",
            pk_score: 20,
            pk_integrity_score: 10,
            has_pk: true,
            pk_integrity: true,
        };
    }

    PrimaryKeySummary {
        pk_columns: Vec::new(),
        declared_grain_columns: Vec::new(),
        evidence: "missing",
        pk_score: 0,
        pk_integrity_score: 0,
        has_pk: false,
        pk_integrity: false,
    }
}

fn primary_key_summary(
    search: &ManifestSearch,
    unique_id: &str,
    pk_columns: Vec<String>,
    evidence: &'static str,
) -> PrimaryKeySummary {
    let pk_integrity = pk_integrity_ok(search, unique_id, &pk_columns);
    let pk_integrity_score = if pk_integrity { 10 } else { 0 };

    PrimaryKeySummary {
        declared_grain_columns: pk_columns.clone(),
        pk_columns,
        evidence,
        pk_score: 20,
        pk_integrity_score,
        has_pk: true,
        pk_integrity,
    }
}

fn summarize_constraints(columns: &serde_json::Map<String, JsonValue>) -> ConstraintSummary {
    let count = constraint_count(columns);
    let score = array_tier_score(count, 10);
    ConstraintSummary { score, count }
}

fn primary_keys(columns: &serde_json::Map<String, JsonValue>) -> (Vec<String>, bool) {
    let mut pk_cols = Vec::new();
    for (name, col) in columns {
        let is_pk = column_primary_key_bool(col);
        if is_pk {
            pk_cols.push(name.clone());
        }
    }
    let has_pk = !pk_cols.is_empty();
    (pk_cols, has_pk)
}

fn pk_integrity_ok(search: &ManifestSearch, unique_id: &str, pk_columns: &[String]) -> bool {
    for col in pk_columns {
        let key = format!("{unique_id}:{col}");
        let tests = search
            .tests_by_column
            .get(&key)
            .cloned()
            .unwrap_or_default();
        if tests.is_empty() {
            return false;
        }
        let mut has_unique = false;
        let mut has_not_null = false;
        for tid in &tests {
            if let Ok(Some(test)) = search.get_entity_archived(tid) {
                let test_json = test.to_json_value();
                let name = test_json
                    .get("test_metadata")
                    .and_then(|tm| tm.get("name"))
                    .and_then(|n| n.as_str());
                if name == Some("unique") {
                    has_unique = true;
                } else if name == Some("not_null") {
                    has_not_null = true;
                }
            }
        }
        if !(has_unique && has_not_null) {
            return false;
        }
    }
    true
}

fn nova_grain_primary_keys(entity_json: &JsonValue) -> Option<Vec<String>> {
    let keys = entity_nova_meta_json(entity_json)
        .as_deref()
        .and_then(|nova| nova.get("grain"))
        .and_then(|grain| grain.get("primary_key"))
        .and_then(JsonValue::as_array)?
        .iter()
        .filter_map(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    (!keys.is_empty()).then_some(keys)
}

fn aggregate_grain_columns(entity_json: &JsonValue) -> Option<Vec<String>> {
    let nova = entity_nova_meta_json(entity_json);
    let grain = nova.as_deref()?.get("grain")?;
    let time_field = grain
        .get("time_field")
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let dimensions = grain
        .get("dimensions")
        .and_then(JsonValue::as_array)?
        .iter()
        .filter_map(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    if dimensions.is_empty() {
        return None;
    }
    let mut columns = Vec::with_capacity(dimensions.len() + 1);
    columns.push(time_field.to_string());
    columns.extend(dimensions);
    Some(columns)
}

fn unique_test_covers_columns(
    search: &ManifestSearch,
    unique_id: &str,
    expected_columns: &[String],
) -> bool {
    let Some(test_ids) = search.tests_by_entity.get(unique_id) else {
        return false;
    };
    let expected = normalized_column_set(expected_columns);
    for test_id in test_ids {
        let Ok(Some(test)) = search.get_entity_archived(test_id) else {
            continue;
        };
        let test_json = test.to_json_value();
        let Some(test_columns) = unique_test_columns(&test_json) else {
            continue;
        };
        if normalized_column_set(&test_columns) == expected {
            return true;
        }
    }
    false
}

fn unique_test_columns(test_json: &JsonValue) -> Option<Vec<String>> {
    let test_name = test_json
        .get("test_metadata")
        .and_then(|metadata| metadata.get("name"))
        .and_then(JsonValue::as_str)?;
    match test_name {
        "unique" => test_json
            .get("column_name")
            .and_then(JsonValue::as_str)
            .map(|column| vec![column.to_string()]),
        "unique_combination_of_columns" | "dbt_utils.unique_combination_of_columns" => {
            test_argument_columns(test_json, "combination_of_columns")
        }
        _ => None,
    }
}

fn test_argument_columns(test_json: &JsonValue, key: &str) -> Option<Vec<String>> {
    for pointer in [
        format!("/test_metadata/kwargs/{key}"),
        format!("/test_metadata/arguments/{key}"),
        format!("/kwargs/{key}"),
        format!("/arguments/{key}"),
    ] {
        if let Some(columns) = test_json.pointer(&pointer).and_then(json_string_array) {
            return Some(columns);
        }
    }
    None
}

fn json_string_array(value: &JsonValue) -> Option<Vec<String>> {
    match value {
        JsonValue::Array(values) => {
            let columns = values
                .iter()
                .filter_map(JsonValue::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string)
                .collect::<Vec<_>>();
            (!columns.is_empty()).then_some(columns)
        }
        JsonValue::String(value) => {
            let columns = value
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string)
                .collect::<Vec<_>>();
            (!columns.is_empty()).then_some(columns)
        }
        _ => None,
    }
}

fn normalized_column_set(columns: &[String]) -> Vec<String> {
    let mut normalized = columns
        .iter()
        .map(|column| column.trim().to_lowercase())
        .filter(|column| !column.is_empty())
        .collect::<Vec<_>>();
    normalized.sort_unstable();
    normalized.dedup();
    normalized
}

fn constraint_count(columns: &serde_json::Map<String, JsonValue>) -> usize {
    let mut count = 0usize;
    for col in columns.values() {
        let Some(constraints) = col.get("constraints").and_then(|c| c.as_array()) else {
            continue;
        };
        for constraint in constraints {
            let constraint_type = constraint.get("type").and_then(|v| v.as_str());
            if matches!(constraint_type, Some("not_null" | "unique" | "foreign_key")) {
                count += 1;
            }
        }
    }
    count
}

fn classify_tests(search: &ManifestSearch, tests: &[String]) -> (Vec<String>, Vec<String>) {
    let mut model_level = Vec::new();
    let mut column_level = Vec::new();
    for test_id in tests {
        if let Ok(Some(test)) = search.get_entity_archived(test_id) {
            let test_json = test.to_json_value();
            if test_json
                .get("column_name")
                .and_then(|c| c.as_str())
                .is_some()
            {
                column_level.push(test_id.clone());
            } else {
                model_level.push(test_id.clone());
            }
        }
    }
    (model_level, column_level)
}
