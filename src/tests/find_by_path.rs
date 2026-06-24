//! Tests for `find_by_path` tool responses.
use super::common::*;

// Find By Path Tests
#[tokio::test(flavor = "multi_thread")]
async fn test_find_by_path_exact_match() {
    let searcher = get_searcher();
    let test_path = "models/staging/traffic/stg__traffic_sessions.sql".to_string();
    let params = FindByPathParams {
        path_pattern: test_path,
        resource_types: vec![],
        detail: Some(DetailLevel::Standard),
        pagination: PaginationParams {
            limit: Some(50),
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
    let params = FindByPathParams {
        path_pattern: "models/staging/*".to_string(),
        resource_types: vec![],
        detail: Some(DetailLevel::Standard),
        pagination: PaginationParams {
            limit: Some(100),
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
        detail: Some(DetailLevel::Standard),
        pagination: PaginationParams {
            limit: Some(50),
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
        detail: Some(DetailLevel::Standard),
        pagination: PaginationParams {
            limit: Some(10),
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
        detail: Some(DetailLevel::Standard),
        pagination: PaginationParams {
            limit: Some(5),
            offset: 0,
        },
    };
    let result = searcher.find_by_path(&params).await.json();
    let count = result
        .get("count")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    assert!(count <= 5, "Should respect limit");
    assert_eq!(result.get("truncated"), Some(&serde_json::json!(true)));
    assert_eq!(
        result
            .get("total_available")
            .and_then(serde_json::Value::as_u64),
        Some(6)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_find_by_path_rejects_excessive_offset() {
    let searcher = get_searcher();
    let params = FindByPathParams {
        path_pattern: "**".to_string(),
        resource_types: vec![],
        detail: Some(DetailLevel::Standard),
        pagination: PaginationParams {
            limit: Some(5),
            offset: searcher.config().search.max_offset + 1,
        },
    };
    let err = searcher
        .find_by_path(&params)
        .await
        .expect_err("offset should be rejected");

    assert!(
        err.to_string().contains("Offset exceeds maximum"),
        "unexpected error: {err}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_find_by_path_no_matches() {
    let searcher = get_searcher();
    let params = FindByPathParams {
        path_pattern: "definitely/not/a/real/path/**".to_string(),
        resource_types: vec![],
        detail: Some(DetailLevel::Standard),
        pagination: PaginationParams {
            limit: Some(50),
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
