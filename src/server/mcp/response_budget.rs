use std::collections::{BTreeMap, BTreeSet};

use crate::config::DbtNovaConfig;
use crate::params::PaginationParams;
use crate::tools::catalog::MCP_BUDGETABLE_DATA_ARRAY_FIELDS;

pub(super) fn apply_mcp_response_budget(
    mut value: serde_json::Value,
    config: &DbtNovaConfig,
) -> serde_json::Value {
    let budget = config.mcp_max_response_bytes;
    if budget == 0 {
        return value;
    }
    let initial_bytes = serialized_len(&value);
    if initial_bytes <= budget {
        return value;
    }
    let original_shape = ResponseShapeSnapshot::capture(&value);

    let mut omitted_paths = Vec::new();
    truncate_json_for_budget(
        &mut value,
        "$".to_string(),
        config.mcp_max_string_chars.max(1),
        50,
        50,
        &mut omitted_paths,
    );
    if serialized_len(&value) > budget {
        truncate_json_for_budget(
            &mut value,
            "$".to_string(),
            1024,
            20,
            20,
            &mut omitted_paths,
        );
    }
    if serialized_len(&value) > budget {
        truncate_json_for_budget(&mut value, "$".to_string(), 512, 5, 5, &mut omitted_paths);
    }
    normalize_budgeted_response_shape(&mut value, &original_shape, !omitted_paths.is_empty());
    finalize_mcp_budgeted_response(
        value,
        budget,
        config.mcp_include_truncation_meta,
        omitted_paths,
        original_shape.total_count_hint,
    )
}

pub(super) fn serialized_len(value: &serde_json::Value) -> usize {
    serde_json::to_string(value).map_or(0, |serialized| serialized.len())
}

pub(super) fn attach_result_meta(
    value: &mut serde_json::Value,
    response_bytes: usize,
    budget_bytes: usize,
    truncated: bool,
    mut omitted_paths: Vec<String>,
    original_count: Option<u64>,
) {
    omitted_paths.sort();
    omitted_paths.dedup();
    let mut meta = serde_json::json!({
        "response_bytes": response_bytes,
        "budget_bytes": budget_bytes,
        "truncated": truncated,
        "omitted_paths": omitted_paths
    });
    if let Some(original_count) = original_count
        && let Some(obj) = meta.as_object_mut()
    {
        obj.insert(
            "original_count".to_string(),
            serde_json::Value::from(original_count),
        );
    }
    if let Some(obj) = value.as_object_mut() {
        obj.insert("_nova_result_meta".to_string(), meta);
    }
}

pub(super) fn update_result_meta_response_bytes(value: &mut serde_json::Value) {
    let response_bytes = serialized_len(value);
    if let Some(obj) = value.as_object_mut()
        && let Some(meta) = obj
            .get_mut("_nova_result_meta")
            .and_then(serde_json::Value::as_object_mut)
    {
        meta.insert(
            "response_bytes".to_string(),
            serde_json::Value::from(response_bytes),
        );
    }
}

pub(super) fn apply_mcp_next_offset_meta(
    value: &mut serde_json::Value,
    config: &DbtNovaConfig,
    pagination: Option<&PaginationParams>,
) {
    let Some(pagination) = pagination else {
        return;
    };
    let total_available = value
        .get("total_available")
        .and_then(serde_json::Value::as_u64);
    let count = value.get("count").and_then(serde_json::Value::as_u64);
    let (Some(total_available), Some(count)) = (total_available, count) else {
        return;
    };
    if count == 0 {
        return;
    }
    let Some(offset) = u64::try_from(pagination.offset).ok() else {
        return;
    };
    let next_offset = offset.saturating_add(count);
    if next_offset >= total_available {
        return;
    }
    let truncated = response_is_truncated(value);

    let Some(obj) = value.as_object_mut() else {
        return;
    };
    let had_result_meta = obj.contains_key("_nova_result_meta");
    let meta = obj
        .entry("_nova_result_meta".to_string())
        .or_insert_with(|| serde_json::json!({}));
    let Some(meta) = meta.as_object_mut() else {
        return;
    };
    meta.insert(
        "next_offset".to_string(),
        serde_json::Value::from(next_offset),
    );
    if config.mcp_include_truncation_meta || had_result_meta {
        meta.entry("truncated".to_string())
            .or_insert_with(|| serde_json::Value::from(truncated));
    }
    if had_result_meta {
        update_result_meta_response_bytes(value);
    }
    let mut refresh_response_bytes = false;
    if config.mcp_max_response_bytes > 0
        && serialized_len(value) > config.mcp_max_response_bytes
        && let Some(obj) = value.as_object_mut()
    {
        if had_result_meta {
            if let Some(meta) = obj
                .get_mut("_nova_result_meta")
                .and_then(serde_json::Value::as_object_mut)
            {
                meta.remove("next_offset");
            }
            refresh_response_bytes = true;
        } else {
            obj.remove("_nova_result_meta");
        }
    }
    if refresh_response_bytes {
        update_result_meta_response_bytes(value);
    }
}

pub(super) fn response_is_truncated(value: &serde_json::Value) -> bool {
    value
        .get("truncated")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
        || value
            .get("_nova_result_meta")
            .and_then(|meta| meta.get("truncated"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
}

pub(super) fn finalize_mcp_budgeted_response(
    value: serde_json::Value,
    budget: usize,
    include_meta: bool,
    mut omitted_paths: Vec<String>,
    original_count: Option<u64>,
) -> serde_json::Value {
    if serialized_len(&value) <= budget {
        if !include_meta {
            return value;
        }
        let mut with_meta = value.clone();
        let response_bytes = serialized_len(&with_meta);
        attach_result_meta(
            &mut with_meta,
            response_bytes,
            budget,
            true,
            omitted_paths.clone(),
            original_count,
        );
        trim_result_meta_paths_for_budget(&mut with_meta, budget);
        update_result_meta_response_bytes(&mut with_meta);
        if serialized_len(&with_meta) <= budget {
            return with_meta;
        }
        return value;
    }

    omitted_paths.push("$".to_string());
    let mut fallback = compact_truncated_response(&value);
    if include_meta {
        let response_bytes = serialized_len(&fallback);
        attach_result_meta(
            &mut fallback,
            response_bytes,
            budget,
            true,
            omitted_paths,
            original_count,
        );
        trim_result_meta_for_budget(&mut fallback, budget);
        update_result_meta_response_bytes(&mut fallback);
    }
    fallback
}

pub(super) fn trim_result_meta_paths_for_budget(value: &mut serde_json::Value, budget: usize) {
    if serialized_len(value) <= budget {
        return;
    }
    if let Some(obj) = value.as_object_mut()
        && let Some(meta) = obj
            .get_mut("_nova_result_meta")
            .and_then(serde_json::Value::as_object_mut)
    {
        meta.insert("omitted_paths".to_string(), serde_json::json!(["$"]));
    }
}

pub(super) fn compact_truncated_response(value: &serde_json::Value) -> serde_json::Value {
    let mut obj = serde_json::Map::new();
    if let Some(source) = value.as_object() {
        for key in ["api", "success", "total_available", "persona"] {
            if let Some(child) = source.get(key) {
                obj.insert(key.to_string(), child.clone());
            }
        }
        match source.get("data") {
            Some(serde_json::Value::Array(_)) => {
                obj.insert("count".to_string(), serde_json::Value::from(0));
                obj.insert("data".to_string(), serde_json::Value::Array(Vec::new()));
            }
            Some(serde_json::Value::Object(data)) if data.contains_key("rows") => {
                obj.insert("count".to_string(), serde_json::Value::from(0));
                obj.insert(
                    "data".to_string(),
                    serde_json::json!({
                        "rows": [],
                        "truncated": true
                    }),
                );
            }
            Some(serde_json::Value::Object(data)) if data_contains_collection_payload(data) => {
                compact_collection_payload(data, &mut obj);
            }
            Some(serde_json::Value::Object(_)) => {
                if let Some(count) = source.get("count") {
                    obj.insert("count".to_string(), count.clone());
                }
                obj.insert("data".to_string(), serde_json::json!({"_truncated": true}));
            }
            Some(serde_json::Value::String(_)) => {
                if let Some(count) = source.get("count") {
                    obj.insert("count".to_string(), count.clone());
                }
                obj.insert(
                    "data".to_string(),
                    serde_json::Value::String("[truncated]".to_string()),
                );
            }
            Some(
                serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_),
            )
            | None => {
                if let Some(count) = source.get("count") {
                    obj.insert("count".to_string(), count.clone());
                }
                obj.insert("data".to_string(), serde_json::Value::Null);
            }
        }
    } else {
        obj.insert("data".to_string(), serde_json::Value::Null);
    }
    obj.insert("truncated".to_string(), serde_json::Value::from(true));
    serde_json::Value::Object(obj)
}

pub(super) fn data_contains_collection_payload(
    data: &serde_json::Map<String, serde_json::Value>,
) -> bool {
    MCP_BUDGETABLE_DATA_ARRAY_FIELDS.iter().any(|field| {
        data.get(field.field)
            .is_some_and(serde_json::Value::is_array)
    })
}

pub(super) fn compact_collection_payload(
    data: &serde_json::Map<String, serde_json::Value>,
    obj: &mut serde_json::Map<String, serde_json::Value>,
) {
    let mut compact = serde_json::Map::new();

    for field in MCP_BUDGETABLE_DATA_ARRAY_FIELDS {
        if data
            .get(field.field)
            .is_some_and(serde_json::Value::is_array)
        {
            compact.insert(
                field.field.to_string(),
                serde_json::Value::Array(Vec::new()),
            );
        }
        if let Some(count_field) = field.returned_count_field
            && data.contains_key(count_field)
        {
            compact.insert(count_field.to_string(), serde_json::Value::from(0));
        }
    }
    if let Some(summary) = data
        .get("summary")
        .and_then(serde_json::Value::as_object)
        .filter(|summary| {
            ["entities_returned", "columns_returned", "items_returned"]
                .iter()
                .any(|key| summary.contains_key(*key))
        })
    {
        let mut compact_summary = summary.clone();
        for key in ["entities_returned", "columns_returned", "items_returned"] {
            if compact_summary.contains_key(key) {
                compact_summary.insert(key.to_string(), serde_json::Value::from(0));
            }
        }
        compact.insert(
            "summary".to_string(),
            serde_json::Value::Object(compact_summary),
        );
    }

    compact.insert("truncated".to_string(), serde_json::Value::from(true));
    obj.insert("count".to_string(), serde_json::Value::from(0));
    obj.insert("data".to_string(), serde_json::Value::Object(compact));
}

pub(super) fn trim_result_meta_for_budget(value: &mut serde_json::Value, budget: usize) {
    trim_result_meta_paths_for_budget(value, budget);
    if serialized_len(value) <= budget {
        return;
    }
    if let Some(obj) = value.as_object_mut() {
        obj.retain(|key, _| key == "_nova_result_meta");
    }
}

pub(super) fn result_count_hint(value: &serde_json::Value) -> Option<u64> {
    value
        .get("total_available")
        .or_else(|| value.get("count"))
        .and_then(serde_json::Value::as_u64)
}

pub(super) struct ResponseShapeSnapshot {
    count: Option<u64>,
    total_count_hint: Option<u64>,
    data_rows_len: Option<u64>,
    data_object_array_lens: BTreeMap<String, u64>,
}

impl ResponseShapeSnapshot {
    fn capture(value: &serde_json::Value) -> Self {
        let mut data_object_array_lens = BTreeMap::new();
        let data_rows_len = value
            .get("data")
            .and_then(serde_json::Value::as_object)
            .and_then(|data| {
                for (key, child) in data {
                    if let Some(len) = array_len(Some(child)) {
                        data_object_array_lens.insert(key.clone(), len);
                    }
                }
                data.get("rows")
            })
            .and_then(|rows| array_len(Some(rows)));

        Self {
            count: value.get("count").and_then(serde_json::Value::as_u64),
            total_count_hint: result_count_hint(value),
            data_rows_len,
            data_object_array_lens,
        }
    }

    fn original_data_array_len(&self, key: &str) -> Option<u64> {
        self.data_object_array_lens.get(key).copied()
    }
}

pub(super) fn array_len(value: Option<&serde_json::Value>) -> Option<u64> {
    value
        .and_then(serde_json::Value::as_array)
        .and_then(|items| u64::try_from(items.len()).ok())
}

pub(super) fn normalize_budgeted_response_shape(
    value: &mut serde_json::Value,
    original_shape: &ResponseShapeSnapshot,
    budget_truncated: bool,
) {
    let Some(obj) = value.as_object_mut() else {
        return;
    };
    if budget_truncated {
        obj.insert("truncated".to_string(), serde_json::Value::from(true));
    }
    if let Some(returned_count) = array_len(obj.get("data")) {
        normalize_budgeted_count(obj, returned_count, original_shape.total_count_hint);
        return;
    }

    let Some(data) = obj
        .get_mut("data")
        .and_then(serde_json::Value::as_object_mut)
    else {
        return;
    };

    let mut top_count = None;
    let mut data_truncated = false;

    if let Some(returned_count) = array_len(data.get("rows")) {
        top_count = Some(returned_count);
        data_truncated |= original_shape
            .data_rows_len
            .is_some_and(|original_len| original_len > returned_count);
        data.insert("truncated".to_string(), serde_json::Value::from(true));
    }

    let entities_returned = array_len(data.get("entities"));
    if let Some(returned_count) = entities_returned {
        update_data_returned_count(data, "found_count", returned_count);
        data_truncated |= original_shape
            .original_data_array_len("entities")
            .is_some_and(|original_len| original_len > returned_count);
        if original_shape
            .original_data_array_len("entities")
            .is_some_and(|original_len| original_shape.count == Some(original_len))
        {
            top_count = Some(returned_count);
        }
    }

    if let Some(returned_count) = array_len(data.get("lineage")) {
        data_truncated |= original_shape
            .original_data_array_len("lineage")
            .is_some_and(|original_len| original_len > returned_count);
        if original_shape
            .original_data_array_len("lineage")
            .is_some_and(|original_len| original_shape.count == Some(original_len))
        {
            top_count = Some(returned_count);
        }
    }

    if let Some(returned_count) = array_len(data.get("not_found")) {
        update_data_returned_count(data, "not_found_count", returned_count);
        data_truncated |= original_shape
            .original_data_array_len("not_found")
            .is_some_and(|original_len| original_len > returned_count);
    }

    if let Some(returned_count) = array_len(data.get("edges")) {
        data_truncated |= original_shape
            .original_data_array_len("edges")
            .is_some_and(|original_len| original_len > returned_count);
    }

    if let Some(returned_count) = array_len(data.get("references")) {
        update_data_returned_count(data, "reference_count", returned_count);
        data_truncated |= original_shape
            .original_data_array_len("references")
            .is_some_and(|original_len| original_len > returned_count);
    }

    if !data.contains_key("rows")
        && let Some(returned_count) = array_len(data.get("columns"))
    {
        data_truncated |= original_shape
            .original_data_array_len("columns")
            .is_some_and(|original_len| original_len > returned_count);
        if original_shape
            .original_data_array_len("columns")
            .is_some_and(|original_len| original_shape.count == Some(original_len))
        {
            top_count = Some(returned_count);
        }
    }

    if normalize_undocumented_summary(data) {
        top_count = data
            .get("summary")
            .and_then(|summary| summary.get("items_returned"))
            .and_then(serde_json::Value::as_u64);
    }

    if data_truncated || budget_truncated {
        data.insert("truncated".to_string(), serde_json::Value::from(true));
    }

    let Some(returned_count) = top_count else {
        return;
    };
    normalize_budgeted_count(obj, returned_count, original_shape.total_count_hint);
}

pub(super) fn update_data_returned_count(
    data: &mut serde_json::Map<String, serde_json::Value>,
    key: &str,
    returned_count: u64,
) {
    if data.contains_key(key) {
        data.insert(key.to_string(), serde_json::Value::from(returned_count));
    }
}

pub(super) fn normalize_undocumented_summary(
    data: &mut serde_json::Map<String, serde_json::Value>,
) -> bool {
    let entities_returned = array_len(data.get("entities"));
    let columns_returned = array_len(data.get("undocumented_columns"));
    let Some(summary) = data
        .get_mut("summary")
        .and_then(serde_json::Value::as_object_mut)
    else {
        return false;
    };
    if !["entities_returned", "columns_returned", "items_returned"]
        .iter()
        .any(|key| summary.contains_key(*key))
    {
        return false;
    }

    if let Some(entities_returned) = entities_returned
        && summary.contains_key("entities_returned")
    {
        summary.insert(
            "entities_returned".to_string(),
            serde_json::Value::from(entities_returned),
        );
    }
    if let Some(columns_returned) = columns_returned
        && summary.contains_key("columns_returned")
    {
        summary.insert(
            "columns_returned".to_string(),
            serde_json::Value::from(columns_returned),
        );
    }
    let items_returned = entities_returned
        .unwrap_or_else(|| {
            summary
                .get("entities_returned")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0)
        })
        .saturating_add(columns_returned.unwrap_or_else(|| {
            summary
                .get("columns_returned")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0)
        }));
    if summary.contains_key("items_returned") {
        summary.insert(
            "items_returned".to_string(),
            serde_json::Value::from(items_returned),
        );
    }
    true
}

pub(super) fn normalize_budgeted_count(
    obj: &mut serde_json::Map<String, serde_json::Value>,
    returned_count: u64,
    original_count: Option<u64>,
) -> bool {
    let count_changed = obj
        .get("count")
        .and_then(serde_json::Value::as_u64)
        .is_some_and(|count| count != returned_count);
    if count_changed {
        obj.insert("count".to_string(), serde_json::Value::from(returned_count));
    }
    if original_count.is_some_and(|count| count > returned_count) || count_changed {
        obj.insert("truncated".to_string(), serde_json::Value::from(true));
        true
    } else {
        false
    }
}

pub(super) fn truncate_json_for_budget(
    value: &mut serde_json::Value,
    path: String,
    max_string_chars: usize,
    max_array_items: usize,
    max_object_entries: usize,
    omitted_paths: &mut Vec<String>,
) {
    match value {
        serde_json::Value::String(text) => {
            if text.chars().count() > max_string_chars {
                let truncated: String = text.chars().take(max_string_chars).collect();
                *text = format!("{truncated}...[truncated]");
                omitted_paths.push(path);
            }
        }
        serde_json::Value::Array(items) => {
            if items.len() > max_array_items {
                items.truncate(max_array_items);
                omitted_paths.push(path.clone());
            }
            for (idx, item) in items.iter_mut().enumerate() {
                truncate_json_for_budget(
                    item,
                    format!("{path}.{idx}"),
                    max_string_chars,
                    max_array_items,
                    max_object_entries,
                    omitted_paths,
                );
            }
        }
        serde_json::Value::Object(obj) => {
            if path != "$" && obj.len() > max_object_entries {
                prune_object_entries(obj, &path, max_object_entries, omitted_paths);
            }
            for noisy_key in ["block_contents", "compiled_code", "raw_code", "sql"] {
                if let Some(child) = obj.get_mut(noisy_key) {
                    truncate_json_for_budget(
                        child,
                        format!("{path}.{noisy_key}"),
                        max_string_chars.min(2048),
                        max_array_items,
                        max_object_entries,
                        omitted_paths,
                    );
                }
            }
            for (key, child) in obj {
                if matches!(
                    key.as_str(),
                    "block_contents" | "compiled_code" | "raw_code" | "sql"
                ) {
                    continue;
                }
                truncate_json_for_budget(
                    child,
                    format!("{path}.{key}"),
                    max_string_chars,
                    max_array_items,
                    max_object_entries,
                    omitted_paths,
                );
            }
        }
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => {}
    }
}

pub(super) fn prune_object_entries(
    obj: &mut serde_json::Map<String, serde_json::Value>,
    path: &str,
    max_object_entries: usize,
    omitted_paths: &mut Vec<String>,
) {
    if max_object_entries == 0 {
        obj.clear();
        omitted_paths.push(format!("{path}.*"));
        return;
    }
    let important_keys = [
        "rows",
        "columns",
        "column_types",
        "truncated",
        "stats",
        "state",
        "provider",
        "success",
        "count",
        "total_available",
        "data",
        "parent_groups",
        "unique_id",
        "parent_unique_id",
        "name",
        "resource_type",
        "relation_name",
        "grain",
        "expression",
        "indicator_name",
        "indicator_type",
    ];
    let mut keep: BTreeSet<String> = important_keys
        .iter()
        .filter(|key| obj.contains_key(**key))
        .take(max_object_entries)
        .map(|key| (*key).to_string())
        .collect();
    for key in obj.keys() {
        if keep.len() >= max_object_entries {
            break;
        }
        keep.insert(key.clone());
    }
    if keep.len() < obj.len() {
        let remove_keys: Vec<String> = obj
            .keys()
            .filter(|key| !keep.contains(*key))
            .cloned()
            .collect();
        for key in remove_keys {
            obj.remove(&key);
        }
        omitted_paths.push(format!("{path}.*"));
    }
}
