#![no_main]

use dbt_nova::nova_meta::{
    NovaMetaTargetSelector, NovaMetaValidationOptions, validate_nova_meta,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(temp_dir) = tempfile::tempdir() else {
        return;
    };
    let file_path = temp_dir.path().join("models.yml");
    if std::fs::write(&file_path, data).is_err() {
        return;
    }

    let _report = validate_nova_meta(&NovaMetaValidationOptions {
        project_dir: temp_dir.path().to_path_buf(),
        paths: vec![file_path],
        selector: NovaMetaTargetSelector::default(),
    });
});
