//! Tests for `find_by_path` tool responses.
use super::common::*;

// Find By Path Tests
#[tokio::test(flavor = "multi_thread")]
async fn test_find_by_path_exact_match() {
    let searcher = get_searcher();
    // Find an actual path from the manifest
    let models = searcher.by_resource_type.get("model").unwrap();
    let mut test_path = None;
    for model_id in models {
        if let Some(entity) = searcher.get_entity(model_id).await.unwrap()
            && let Some(path) = entity.original_file_path.as_deref()
            && !path.is_empty()
        {
            test_path = Some(path.to_string());
            break;
        }
    }
    if test_path.is_none() {
        println!("Skipping test: no model with path found");
        return;
    }
    let params = FindByPathParams {
        path_pattern: test_path.unwrap(),
        resource_types: vec![],
        detail: DetailLevel::Standard,
        pagination: PaginationParams {
            limit: 50,
            offset: 0,
        },
    };
    let result = searcher.find_by_path(&params).await.json();
    let success = result
        .get("success")
        .expect("response missing 'success' field")
        .as_bool()
        .expect("'success' field should be boolean");
    assert!(
        success,
        "Expected success=true but got error: {:?}",
        result.get("error")
    );
    let count = result
        .get("count")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    assert!(count >= 1, "Exact path should match at least one entity");
}
#[tokio::test(flavor = "multi_thread")]
async fn test_find_by_path_glob_star() {
    let searcher = get_searcher();
    // Find common path prefix
    let models = searcher.by_resource_type.get("model").unwrap();
    let mut common_prefix = None;
    for model_id in models {
        if let Some(entity) = searcher.get_entity(model_id).await.unwrap()
            && let Some(path) = entity.original_file_path.as_deref()
            && path.contains('/')
        {
            let parts: Vec<&str> = path.split('/').collect();
            if parts.len() >= 2 {
                let prefix = parts[0];
                common_prefix = Some(format!("{prefix}/*"));
                break;
            }
        }
    }
    if common_prefix.is_none() {
        println!("Skipping test: no suitable path found");
        return;
    }
    let params = FindByPathParams {
        path_pattern: common_prefix.unwrap(),
        resource_types: vec![],
        detail: DetailLevel::Standard,
        pagination: PaginationParams {
            limit: 100,
            offset: 0,
        },
    };
    let result = searcher.find_by_path(&params).await.json();
    let success = result
        .get("success")
        .expect("response missing 'success' field")
        .as_bool()
        .expect("'success' field should be boolean");
    assert!(
        success,
        "Expected success=true but got error: {:?}",
        result.get("error")
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn test_find_by_path_double_star() {
    let searcher = get_searcher();
    // Use ** to match any depth
    let params = FindByPathParams {
        path_pattern: "models/**".to_string(),
        resource_types: vec!["model".to_string()],
        detail: DetailLevel::Standard,
        pagination: PaginationParams {
            limit: 50,
            offset: 0,
        },
    };
    let result = searcher.find_by_path(&params).await.json();
    let success = result
        .get("success")
        .expect("response missing 'success' field")
        .as_bool()
        .expect("'success' field should be boolean");
    assert!(
        success,
        "Expected success=true but got error: {:?}",
        result.get("error")
    );
    // All results should have paths starting with "models/"
    if let Some(data) = result.get("data").and_then(|d| d.as_array()) {
        for item in data {
            let path = item
                .get("original_file_path")
                .and_then(|p| p.as_str())
                .unwrap_or("");
            assert!(
                path.starts_with("models/") || path.starts_with("models\\"),
                "Path '{path}' should start with 'models/'"
            );
        }
    }
}
#[tokio::test(flavor = "multi_thread")]
async fn test_find_by_path_with_resource_type_filter() {
    let searcher = get_searcher();
    let params = FindByPathParams {
        path_pattern: "**".to_string(), // Match everything
        resource_types: vec!["model".to_string()],
        detail: DetailLevel::Standard,
        pagination: PaginationParams {
            limit: 10,
            offset: 0,
        },
    };
    let result = searcher.find_by_path(&params).await.json();
    let success = result
        .get("success")
        .expect("response missing 'success' field")
        .as_bool()
        .expect("'success' field should be boolean");
    assert!(
        success,
        "Expected success=true but got error: {:?}",
        result.get("error")
    );
    // All results should be models
    if let Some(data) = result.get("data").and_then(|d| d.as_array()) {
        for item in data {
            let rt = item.get("resource_type").and_then(|r| r.as_str());
            assert_eq!(rt, Some("model"));
        }
    }
}
#[tokio::test(flavor = "multi_thread")]
async fn test_find_by_path_respects_limit() {
    let searcher = get_searcher();
    let params = FindByPathParams {
        path_pattern: "**".to_string(),
        resource_types: vec![],
        detail: DetailLevel::Standard,
        pagination: PaginationParams {
            limit: 5,
            offset: 0,
        },
    };
    let result = searcher.find_by_path(&params).await.json();
    let count = result
        .get("count")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    assert!(count <= 5, "Should respect limit");
}
#[tokio::test(flavor = "multi_thread")]
async fn test_find_by_path_no_matches() {
    let searcher = get_searcher();
    let params = FindByPathParams {
        path_pattern: "definitely/not/a/real/path/**".to_string(),
        resource_types: vec![],
        detail: DetailLevel::Standard,
        pagination: PaginationParams {
            limit: 50,
            offset: 0,
        },
    };
    let result = searcher.find_by_path(&params).await.json();
    let success = result
        .get("success")
        .expect("response missing 'success' field")
        .as_bool()
        .expect("'success' field should be boolean");
    assert!(
        success,
        "Expected success=true but got error: {:?}",
        result.get("error")
    );
    assert_eq!(
        result
            .get("count")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(1),
        0
    );
}
