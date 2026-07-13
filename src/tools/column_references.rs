use std::collections::BTreeMap;

use serde_json::{Value as JsonValue, json};

use crate::error::Result;
use crate::manifest::entity::entity_nova_meta_json;
use crate::manifest::lineage_sql::sql_for_matching;
use crate::manifest::search::ManifestSearch;

pub(crate) fn collect_column_references(
    search: &ManifestSearch,
    column_name: &str,
) -> Result<Vec<JsonValue>> {
    let column_name = column_name.trim();
    if column_name.is_empty() {
        return Ok(Vec::new());
    }

    let mut records = Vec::new();
    collect_nova_meta_references(search, column_name, &mut records)?;
    collect_test_references(search, column_name, &mut records)?;
    collect_recipe_sql_references(search, column_name, &mut records)?;
    records.sort_by(reference_sort_key);
    Ok(records)
}

pub(crate) fn reference_counts_by_kind(references: &[JsonValue]) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for reference in references {
        let kind = reference
            .get("kind")
            .and_then(JsonValue::as_str)
            .unwrap_or("unknown");
        *counts.entry(kind.to_string()).or_default() += 1;
    }
    counts
}

pub(crate) fn identifier_match_positions(text: &str, identifier: &str) -> Vec<usize> {
    let needle = identifier.trim().to_ascii_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }

    let haystack = text.to_ascii_lowercase();
    let bytes = haystack.as_bytes();
    let needle_bytes = needle.as_bytes();
    let mut positions = Vec::new();
    let mut cursor = 0usize;
    while let Some(found) = haystack[cursor..].find(&needle) {
        let start = cursor + found;
        let end = start + needle_bytes.len();
        let before_ok = start == 0 || !is_identifier_byte(bytes[start - 1]);
        let after_ok = end >= bytes.len() || !is_identifier_byte(bytes[end]);
        if before_ok && after_ok {
            positions.push(start);
        }
        cursor = end;
    }
    positions
}

fn collect_nova_meta_references(
    search: &ManifestSearch,
    column_name: &str,
    records: &mut Vec<JsonValue>,
) -> Result<()> {
    let mut ids: Vec<String> = search.resource_type_by_id.keys().cloned().collect();
    ids.sort();

    for unique_id in ids {
        let Some(entity) = search.get_entity_archived(&unique_id)? else {
            continue;
        };
        let entity_json = entity.to_json_value();
        let Some(nova) = entity_nova_meta_json(&entity_json) else {
            continue;
        };
        let nova = nova.as_ref();

        collect_grain_dimension_references(
            &unique_id,
            &entity_json,
            nova.get("grain"),
            "meta.nova.grain",
            None,
            column_name,
            records,
        );
        collect_measure_references(&unique_id, &entity_json, nova, column_name, records);
        collect_metric_references(&unique_id, &entity_json, nova, column_name, records);
    }

    Ok(())
}

fn collect_grain_dimension_references(
    unique_id: &str,
    entity_json: &JsonValue,
    grain: Option<&JsonValue>,
    path: &str,
    metric_name: Option<&str>,
    column_name: &str,
    records: &mut Vec<JsonValue>,
) {
    let Some(dimensions) = grain
        .and_then(|grain| grain.get("dimensions"))
        .and_then(JsonValue::as_array)
    else {
        return;
    };

    for (index, dimension) in dimensions.iter().enumerate() {
        let Some(dimension) = dimension.as_str() else {
            continue;
        };
        if !names_equal(dimension, column_name) {
            continue;
        }

        let mut detail = json!({
            "path": format!("{path}.dimensions[{index}]"),
            "field": "dimensions",
            "match": "exact"
        });
        if let Some(metric_name) = metric_name
            && let Some(obj) = detail.as_object_mut()
        {
            obj.insert(
                "metric_name".to_string(),
                JsonValue::String(metric_name.to_string()),
            );
        }
        records.push(reference_record(
            "nova_grain_dimension",
            unique_id,
            entity_json,
            detail,
        ));
    }
}

fn collect_measure_references(
    unique_id: &str,
    entity_json: &JsonValue,
    nova: &JsonValue,
    column_name: &str,
    records: &mut Vec<JsonValue>,
) {
    let Some(measures) = nova.get("measures").and_then(JsonValue::as_array) else {
        return;
    };

    for (index, measure) in measures.iter().enumerate() {
        let field_match = measure
            .get("field")
            .and_then(JsonValue::as_str)
            .is_some_and(|field| names_equal(field, column_name));
        let expression = measure
            .get("expression")
            .or_else(|| measure.get("expr"))
            .and_then(JsonValue::as_str);
        let expression_match = expression
            .is_some_and(|expr| !identifier_match_positions(expr, column_name).is_empty());

        if !field_match && !expression_match {
            continue;
        }

        records.push(reference_record(
            "nova_measure_expression",
            unique_id,
            entity_json,
            json!({
                "path": format!("meta.nova.measures[{index}]"),
                "measure_name": measure.get("name").and_then(JsonValue::as_str),
                "field_match": field_match,
                "expression_match": expression_match,
                "expression": expression,
                "match": if field_match { "exact" } else { "tokenized_expression" }
            }),
        ));
    }
}

fn collect_metric_references(
    unique_id: &str,
    entity_json: &JsonValue,
    nova: &JsonValue,
    column_name: &str,
    records: &mut Vec<JsonValue>,
) {
    if let Some(metric) = nova.get("metric") {
        collect_single_metric_reference(
            unique_id,
            entity_json,
            metric,
            "meta.nova.metric",
            column_name,
            records,
        );
    }

    let Some(metrics) = nova.get("metrics").and_then(JsonValue::as_array) else {
        return;
    };
    for (index, metric) in metrics.iter().enumerate() {
        collect_single_metric_reference(
            unique_id,
            entity_json,
            metric,
            &format!("meta.nova.metrics[{index}]"),
            column_name,
            records,
        );
    }
}

fn collect_single_metric_reference(
    unique_id: &str,
    entity_json: &JsonValue,
    metric: &JsonValue,
    path: &str,
    column_name: &str,
    records: &mut Vec<JsonValue>,
) {
    let metric_name = metric.get("name").and_then(JsonValue::as_str);
    let expression = metric
        .get("expression")
        .or_else(|| metric.get("expr"))
        .and_then(JsonValue::as_str);
    if expression.is_some_and(|expr| !identifier_match_positions(expr, column_name).is_empty()) {
        records.push(reference_record(
            "nova_metric_expression",
            unique_id,
            entity_json,
            json!({
                "path": path,
                "metric_name": metric_name,
                "expression": expression,
                "match": "tokenized_expression"
            }),
        ));
    }

    collect_grain_dimension_references(
        unique_id,
        entity_json,
        metric.get("grain"),
        &format!("{path}.grain"),
        metric_name,
        column_name,
        records,
    );
}

fn collect_test_references(
    search: &ManifestSearch,
    column_name: &str,
    records: &mut Vec<JsonValue>,
) -> Result<()> {
    let Some(test_ids) = search.by_resource_type.get("test") else {
        return Ok(());
    };
    let mut test_ids = test_ids.clone();
    test_ids.sort();

    for unique_id in test_ids {
        let Some(test) = search.get_entity_archived(&unique_id)? else {
            continue;
        };
        let test_json = test.to_json_value();
        let mut matches = Vec::new();
        if test_json
            .get("column_name")
            .and_then(JsonValue::as_str)
            .is_some_and(|column| names_equal(column, column_name))
        {
            matches.push(json!({"path": "column_name", "match": "exact"}));
        }
        if let Some(kwargs) = test_json
            .get("test_metadata")
            .and_then(|metadata| metadata.get("kwargs"))
        {
            collect_json_exact_matches(kwargs, "$.test_metadata.kwargs", column_name, &mut matches);
        }

        if matches.is_empty() {
            continue;
        }

        records.push(reference_record(
            "test",
            &unique_id,
            &test_json,
            json!({
                "test_name": test_json
                    .get("test_metadata")
                    .and_then(|metadata| metadata.get("name"))
                    .and_then(JsonValue::as_str),
                "matches": matches,
                "depends_on": dependency_nodes(&test_json),
                "match": "exact"
            }),
        ));
    }

    Ok(())
}

fn collect_recipe_sql_references(
    search: &ManifestSearch,
    column_name: &str,
    records: &mut Vec<JsonValue>,
) -> Result<()> {
    let Some(analysis_ids) = search.by_resource_type.get("analysis") else {
        return Ok(());
    };
    let mut analysis_ids = analysis_ids.clone();
    analysis_ids.sort();

    for unique_id in analysis_ids {
        let Some(analysis) = search.get_entity_archived(&unique_id)? else {
            continue;
        };
        let analysis_json = analysis.to_json_value();
        if !is_recipe_analysis(search, &analysis_json) {
            continue;
        }
        let Some(sql) = sql_for_matching(&analysis_json) else {
            continue;
        };
        let positions = identifier_match_positions(sql, column_name);
        if positions.is_empty() {
            continue;
        }

        records.push(reference_record(
            "recipe_sql",
            &unique_id,
            &analysis_json,
            json!({
                "match": "textual",
                "occurrences": positions.len(),
                "snippet": snippet_around(sql, positions[0], column_name.len()),
                "depends_on": dependency_nodes(&analysis_json)
            }),
        ));
    }

    Ok(())
}

fn collect_json_exact_matches(
    value: &JsonValue,
    path: &str,
    column_name: &str,
    matches: &mut Vec<JsonValue>,
) {
    match value {
        JsonValue::String(value) if names_equal(value, column_name) => {
            matches.push(json!({"path": path, "match": "exact"}));
        }
        JsonValue::Array(values) => {
            for (index, item) in values.iter().enumerate() {
                collect_json_exact_matches(item, &format!("{path}[{index}]"), column_name, matches);
            }
        }
        JsonValue::Object(map) => {
            for (key, item) in map {
                collect_json_exact_matches(item, &format!("{path}.{key}"), column_name, matches);
            }
        }
        _ => {}
    }
}

fn reference_record(
    kind: &str,
    unique_id: &str,
    entity_json: &JsonValue,
    detail: JsonValue,
) -> JsonValue {
    let mut record = serde_json::Map::new();
    record.insert("kind".to_string(), JsonValue::String(kind.to_string()));
    record.insert(
        "unique_id".to_string(),
        JsonValue::String(unique_id.to_string()),
    );
    record.insert(
        "name".to_string(),
        entity_json
            .get("name")
            .and_then(JsonValue::as_str)
            .map_or(JsonValue::Null, |name| JsonValue::String(name.to_string())),
    );
    record.insert(
        "resource_type".to_string(),
        entity_json
            .get("resource_type")
            .and_then(JsonValue::as_str)
            .map_or(JsonValue::Null, |resource_type| {
                JsonValue::String(resource_type.to_string())
            }),
    );
    record.insert(
        "original_file_path".to_string(),
        entity_json
            .get("original_file_path")
            .or_else(|| entity_json.get("path"))
            .and_then(JsonValue::as_str)
            .map_or(JsonValue::Null, |path| JsonValue::String(path.to_string())),
    );
    record.insert("detail".to_string(), detail);
    JsonValue::Object(record)
}

fn reference_sort_key(left: &JsonValue, right: &JsonValue) -> std::cmp::Ordering {
    let left_key = (
        left.get("kind").and_then(JsonValue::as_str).unwrap_or(""),
        left.get("unique_id")
            .and_then(JsonValue::as_str)
            .unwrap_or(""),
        left.get("original_file_path")
            .and_then(JsonValue::as_str)
            .unwrap_or(""),
    );
    let right_key = (
        right.get("kind").and_then(JsonValue::as_str).unwrap_or(""),
        right
            .get("unique_id")
            .and_then(JsonValue::as_str)
            .unwrap_or(""),
        right
            .get("original_file_path")
            .and_then(JsonValue::as_str)
            .unwrap_or(""),
    );
    left_key.cmp(&right_key)
}

fn dependency_nodes(entity_json: &JsonValue) -> Vec<String> {
    entity_json
        .get("depends_on")
        .and_then(|depends_on| depends_on.get("nodes"))
        .and_then(JsonValue::as_array)
        .map(|nodes| {
            nodes
                .iter()
                .filter_map(JsonValue::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn is_recipe_analysis(search: &ManifestSearch, entity_json: &JsonValue) -> bool {
    let Some(path) = entity_json
        .get("original_file_path")
        .or_else(|| entity_json.get("path"))
        .and_then(JsonValue::as_str)
    else {
        return false;
    };

    let configured = search.config.recipes_dir.trim();
    let prefix = if configured.is_empty() {
        "analyses/recipes"
    } else {
        configured
    };
    let mut prefix = normalize_path(prefix);
    if !prefix.ends_with('/') {
        prefix.push('/');
    }

    normalize_path(path).starts_with(&prefix)
}

fn normalize_path(path: &str) -> String {
    path.replace('\\', "/").trim_start_matches("./").to_string()
}

fn snippet_around(sql: &str, start: usize, len: usize) -> String {
    let snippet_start = char_boundary_before(sql, start.saturating_sub(60));
    let snippet_end = char_boundary_after(sql, (start + len + 60).min(sql.len()));
    sql[snippet_start..snippet_end]
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn char_boundary_before(value: &str, mut index: usize) -> usize {
    while index > 0 && !value.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn char_boundary_after(value: &str, mut index: usize) -> usize {
    while index < value.len() && !value.is_char_boundary(index) {
        index += 1;
    }
    index
}

fn names_equal(left: &str, right: &str) -> bool {
    normalize_name(left).eq_ignore_ascii_case(&normalize_name(right))
}

fn normalize_name(value: &str) -> String {
    value
        .trim()
        .trim_matches(|c: char| matches!(c, '`' | '"' | '[' | ']'))
        .rsplit('.')
        .next()
        .unwrap_or(value)
        .to_string()
}

fn is_identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

#[cfg(test)]
mod tests {
    use super::identifier_match_positions;

    #[test]
    fn identifier_match_positions_respects_boundaries() {
        assert_eq!(
            identifier_match_positions("type_of_channel", "type_of_channel"),
            vec![0]
        );
        assert!(identifier_match_positions("new_type_of_channel", "type_of_channel").is_empty());
        assert!(identifier_match_positions("type_of_channel_v2", "type_of_channel").is_empty());
        assert_eq!(
            identifier_match_positions("select t.type_of_channel from table t", "type_of_channel"),
            vec![9]
        );
    }
}
