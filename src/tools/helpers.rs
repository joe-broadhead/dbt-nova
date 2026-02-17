use serde_json::Value as JsonValue;

pub fn dependency_hints_from_json(entity_json: &JsonValue) -> (JsonValue, usize) {
    let depends_on_nodes = entity_json
        .get("depends_on")
        .and_then(|d| d.get("nodes"))
        .and_then(JsonValue::as_array)
        .map_or(0, std::vec::Vec::len);
    let refs = entity_json
        .get("refs")
        .and_then(JsonValue::as_array)
        .map_or(0, std::vec::Vec::len);
    let sources = entity_json
        .get("sources")
        .and_then(JsonValue::as_array)
        .map_or(0, std::vec::Vec::len);
    let total = depends_on_nodes + refs + sources;
    (
        serde_json::json!({
            "depends_on_nodes": depends_on_nodes,
            "refs": refs,
            "sources": sources
        }),
        total,
    )
}

pub fn primary_key_columns_from_columns(columns: &JsonValue) -> Vec<String> {
    let mut primary_keys = Vec::new();
    let Some(obj) = columns.as_object() else {
        return primary_keys;
    };
    for (name, column) in obj {
        let is_primary = column
            .get("meta")
            .and_then(|meta| meta.get("primary_key"))
            .and_then(JsonValue::as_bool)
            .unwrap_or(false);
        if is_primary {
            primary_keys.push(name.clone());
        }
    }
    primary_keys
}

pub fn primary_key_columns_from_entity(entity_json: &JsonValue) -> Vec<String> {
    entity_json
        .get("columns")
        .map_or_else(Vec::new, primary_key_columns_from_columns)
}

pub fn test_type_from_json(test_json: &JsonValue) -> &str {
    test_json
        .get("test_metadata")
        .and_then(|tm| tm.get("name"))
        .and_then(JsonValue::as_str)
        .unwrap_or("custom")
}

pub fn json_str<'a>(value: &'a JsonValue, field: &str) -> &'a str {
    value.get(field).and_then(JsonValue::as_str).unwrap_or("")
}

pub fn sort_by_json_field(items: &mut [JsonValue], field: &str) {
    items.sort_by(|a, b| json_str(a, field).cmp(json_str(b, field)));
}
