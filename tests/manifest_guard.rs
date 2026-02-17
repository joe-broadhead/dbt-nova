//! Guard test to prevent hardcoded manifest.json paths in tests.
use std::fs;
use std::path::PathBuf;

#[test]
fn test_no_manifest_json_hardcoding_in_tests() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut offenders = Vec::new();
    let test_dirs = [root.join("tests"), root.join("src/tests")];

    for dir in test_dirs {
        if let Ok(entries) = fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                    continue;
                }
                if let Ok(contents) = fs::read_to_string(&path)
                    && contents.contains("manifest.json")
                    && !contents.contains("nova_manifest.json")
                {
                    offenders.push(path);
                }
            }
        }
    }

    if !offenders.is_empty() {
        panic!(
            "Hardcoded manifest.json references found in tests: {:?}",
            offenders
        );
    }
}
