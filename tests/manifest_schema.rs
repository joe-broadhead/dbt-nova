//! Schema validation tests for fixture manifests.
use std::fs;

use jsonschema::draft202012;
use serde_json::Value;

fn load_json(path: &str) -> Value {
    let contents = fs::read_to_string(path).expect("Failed to read JSON file");
    serde_json::from_str(&contents).expect("Failed to parse JSON file")
}

#[test]
fn fixture_manifest_matches_dbt_schema_v12() {
    let schema = load_json("schemas/dbt/manifest_v12.json");
    let instance = load_json("tests/fixtures/nova_manifest.json");
    let compiled = draft202012::options()
        .build(&schema)
        .expect("Failed to compile dbt schema");

    if !compiled.is_valid(&instance) {
        let messages: Vec<String> = compiled
            .iter_errors(&instance)
            .take(5)
            .map(|e| e.to_string())
            .collect();
        panic!(
            "Fixture manifest does not match dbt v12 schema. Sample errors: {:?}",
            messages
        );
    }
}
