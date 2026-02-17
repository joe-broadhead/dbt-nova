#![no_main]

use libfuzzer_sys::fuzz_target;
use serde_json::Value;

fuzz_target!(|data: &[u8]| {
    if let Ok(value) = serde_json::from_slice::<Value>(data) {
        if let Some(obj) = value.as_object() {
            if let Some((key, payload)) = obj.iter().next() {
                let _ = dbt_nova::manifest::entity::Entity::from_json(key, payload);
            }
        }
    }
});
