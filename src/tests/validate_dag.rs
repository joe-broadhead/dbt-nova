//! Tests for `validate_dag` tool responses.
use super::common::*;
use crate::params::{ValidateDagDetail, ValidateDagParams};

// Validate DAG Tests
#[tokio::test(flavor = "multi_thread")]
async fn test_validate_dag() {
    let searcher = get_searcher();
    let result = searcher
        .validate_dag(&ValidateDagParams {
            detail: ValidateDagDetail::Full,
        })
        .await
        .json();
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
    if let Some(data) = result.get("data") {
        assert!(data.get("valid").is_some());
        assert!(data.get("issue_count").is_some());
        assert!(data.get("issues").is_some());
    }
}
