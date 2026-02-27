//! Tests for SQL tools and statement validation.
use super::common::*;
use crate::tools::sql::validate_sql_statement;

// Get SQL Tests
#[tokio::test(flavor = "multi_thread")]
async fn test_get_sql_raw() {
    let searcher = get_searcher();
    let params = GetSqlParams {
        id_or_name: "int__campaign_features".to_string(),
        compiled: false,
    };
    let result = searcher.get_sql(&params).await.json();
    // May or may not have raw_code depending on manifest
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

    let data = result.get("data").expect("response missing data");
    assert_eq!(
        data.get("unique_id").and_then(|value| value.as_str()),
        Some("model.nova_test.int__campaign_features")
    );
    assert_eq!(
        data.get("sql_type").and_then(|value| value.as_str()),
        Some("raw")
    );
    let sql = data
        .get("sql")
        .and_then(|value| value.as_str())
        .expect("data.sql should be a string");
    assert!(!sql.trim().is_empty(), "raw SQL should not be empty");
    assert!(
        sql.to_ascii_lowercase().contains("select"),
        "raw SQL should contain a SELECT statement"
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn test_get_sql_not_found() {
    let searcher = get_searcher();
    let params = GetSqlParams {
        id_or_name: "nonexistent_model".to_string(),
        compiled: false,
    };
    let result = searcher.get_sql(&params).await.json();
    let success = result
        .get("success")
        .expect("response missing 'success' field")
        .as_bool()
        .expect("'success' field should be boolean");
    assert!(!success, "Should fail for nonexistent model");
    let error_code = result.get("error_code").and_then(|e| e.as_str());
    assert_eq!(error_code, Some("NOT_FOUND"));
}

#[test]
fn test_sql_validation_allows_select() {
    assert!(validate_sql_statement("SELECT * FROM some_table").is_ok());
    assert!(validate_sql_statement("SELECT a, b FROM t WHERE x = 1").is_ok());
    assert!(validate_sql_statement("EXPLAIN SELECT * FROM t").is_ok());
}

#[test]
fn test_sql_validation_blocks_dangerous() {
    let dangerous = [
        "DROP TABLE users",
        "DELETE FROM users",
        "TRUNCATE TABLE users",
        "INSERT INTO users VALUES (1)",
        "UPDATE users SET x = 1",
        "CREATE TABLE evil (id INT)",
        "ALTER TABLE users ADD COLUMN x INT",
        "; DROP TABLE users; --",
    ];

    for stmt in dangerous {
        assert!(
            validate_sql_statement(stmt).is_err(),
            "Should block: {stmt}"
        );
    }
}
