use std::fs;
use std::path::Path;

use schemars_0_8::schema::RootSchema;
use typify::{TypeSpace, TypeSpaceSettings};

fn main() {
    // Re-run the build script when schemas change.
    println!("cargo:rerun-if-changed=schemas/");

    let strict = strict_schema();

    // Load dbt + Nova JSON Schemas used for the public `dbt_nova::dbt_types`
    // compatibility module. Runtime manifest loading uses the streaming parser
    // instead of these generated types.
    let dbt_schema = read_schema("schemas/dbt/manifest_v12.json", strict);
    let nova_schema = read_schema("schemas/nova/v0.json", strict);

    // Generate Rust types from the schemas with typify.
    let settings = TypeSpaceSettings::default();

    let mut type_space = TypeSpace::new(&settings);
    match serde_json::from_str::<RootSchema>(&dbt_schema) {
        Ok(root) => {
            let _ = type_space.add_root_schema(root);
        }
        Err(err) => {
            if strict {
                panic!("Failed to parse dbt schema: {err}");
            }
            println!("cargo:warning=Failed to parse dbt schema: {err}");
        }
    }
    match serde_json::from_str::<RootSchema>(&nova_schema) {
        Ok(root) => {
            let _ = type_space.add_root_schema(root);
        }
        Err(err) => {
            if strict {
                panic!("Failed to parse nova schema: {err}");
            }
            println!("cargo:warning=Failed to parse nova schema: {err}");
        }
    }

    // Emit generated types into Cargo's build output directory.
    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR missing");
    let generated = type_space.to_stream().to_string();
    let out_path = Path::new(&out_dir).join("dbt_types.rs");
    let _ = fs::write(out_path, generated);
}

fn read_schema(path: &str, strict: bool) -> String {
    match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(err) => {
            // Warn but continue; missing schemas are handled gracefully to avoid blocking builds.
            if strict {
                panic!("Schema file not found: {path} ({err})");
            }
            println!("cargo:warning=Schema file not found: {path} ({err})");
            "{}".to_string()
        }
    }
}

fn strict_schema() -> bool {
    let strict_env = std::env::var("DBT_NOVA_STRICT_SCHEMA")
        .ok()
        .map(|v| {
            matches!(
                v.trim().to_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false);
    if strict_env {
        return true;
    }
    std::env::var("CI")
        .ok()
        .map(|v| {
            matches!(
                v.trim().to_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}
