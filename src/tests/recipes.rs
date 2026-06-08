//! Tests for recipe discovery and retrieval tools.
use super::common::*;
use std::collections::HashMap;

#[tokio::test(flavor = "multi_thread")]
async fn test_search_recipes_discovers_fixture_recipes() {
    let searcher = get_searcher_with_fixture("recipes_by_analysis.json");
    let params = SearchRecipesParams {
        query: String::new(),
        topic: String::new(),
        include_queries: true,
        pagination: PaginationParams {
            limit: Some(20),
            offset: 0,
        },
    };
    let result = searcher.search_recipes(&params).await.json();
    let success = result
        .get("success")
        .expect("response missing 'success'")
        .as_bool()
        .expect("'success' should be boolean");
    assert!(
        success,
        "Expected search_recipes to succeed: {:?}",
        result.get("error")
    );
    let data = result
        .get("data")
        .and_then(serde_json::Value::as_array)
        .expect("search_recipes should return array data");
    let weekly = data
        .iter()
        .find(|item| item.get("id").and_then(|id| id.as_str()) == Some("marketplace/weekly_report"))
        .expect("marketplace/weekly_report recipe missing");
    assert_eq!(data.len(), 1);
    assert_eq!(
        weekly
            .get("query_count")
            .and_then(serde_json::Value::as_u64),
        Some(2)
    );
    assert_eq!(
        weekly
            .get("queries")
            .and_then(serde_json::Value::as_array)
            .expect("queries should be present when include_queries=true")
            .len(),
        2
    );
    let required_parameters = weekly
        .get("required_parameters")
        .and_then(serde_json::Value::as_array)
        .expect("required_parameters should be present");
    assert!(
        required_parameters
            .iter()
            .any(|entry| entry.as_str() == Some("COUNTRY_CODE"))
    );
    let optional_parameters = weekly
        .get("optional_parameters")
        .and_then(serde_json::Value::as_array)
        .expect("optional_parameters should be present");
    assert!(
        optional_parameters
            .iter()
            .any(|entry| entry.as_str() == Some("TOP_N"))
    );
    let defaults = weekly
        .get("parameter_defaults")
        .and_then(serde_json::Value::as_object)
        .expect("parameter_defaults should be present");
    assert_eq!(
        defaults.get("TOP_N").and_then(serde_json::Value::as_i64),
        Some(10)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_search_recipes_query_filter_applies_to_query_names() {
    let searcher = get_searcher_with_fixture("recipes_by_analysis.json");
    let params = SearchRecipesParams {
        query: "channel_mix".to_string(),
        topic: String::new(),
        include_queries: false,
        pagination: PaginationParams {
            limit: Some(20),
            offset: 0,
        },
    };
    let result = searcher.search_recipes(&params).await.json();
    let success = result
        .get("success")
        .expect("response missing 'success'")
        .as_bool()
        .expect("'success' should be boolean");
    assert!(
        success,
        "Expected filtered search_recipes to succeed: {:?}",
        result.get("error")
    );
    let data = result
        .get("data")
        .and_then(serde_json::Value::as_array)
        .expect("search_recipes should return array data");
    assert_eq!(data.len(), 1);
    assert_eq!(
        data[0].get("id").and_then(|id| id.as_str()),
        Some("marketplace/weekly_report")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_search_recipes_query_filter_normalizes_spaces_and_separators() {
    let searcher = get_searcher_with_fixture("recipes_by_analysis.json");
    let params = SearchRecipesParams {
        query: "weekly report".to_string(),
        topic: String::new(),
        include_queries: false,
        pagination: PaginationParams {
            limit: Some(20),
            offset: 0,
        },
    };
    let result = searcher.search_recipes(&params).await.json();
    let success = result
        .get("success")
        .expect("response missing 'success'")
        .as_bool()
        .expect("'success' should be boolean");
    assert!(
        success,
        "Expected normalized search_recipes to succeed: {:?}",
        result.get("error")
    );
    let data = result
        .get("data")
        .and_then(serde_json::Value::as_array)
        .expect("search_recipes should return array data");
    assert_eq!(data.len(), 1);
    assert_eq!(
        data[0].get("id").and_then(|id| id.as_str()),
        Some("marketplace/weekly_report")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_search_recipes_topic_filter_normalizes_spaces_and_separators() {
    let searcher = get_searcher_with_fixture("recipes_by_analysis.json");
    let params = SearchRecipesParams {
        query: String::new(),
        topic: "marketplace weekly".to_string(),
        include_queries: false,
        pagination: PaginationParams {
            limit: Some(20),
            offset: 0,
        },
    };
    let result = searcher.search_recipes(&params).await.json();
    let success = result
        .get("success")
        .expect("response missing 'success'")
        .as_bool()
        .expect("'success' should be boolean");
    assert!(
        success,
        "Expected normalized topic filter to succeed: {:?}",
        result.get("error")
    );
    let data = result
        .get("data")
        .and_then(serde_json::Value::as_array)
        .expect("search_recipes should return array data");
    assert_eq!(data.len(), 1);
    assert_eq!(
        data[0].get("id").and_then(|id| id.as_str()),
        Some("marketplace/weekly_report")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_get_recipe_with_basename_and_sql_payload() {
    let searcher = get_searcher_with_fixture("recipes_by_analysis.json");
    let mut parameters = HashMap::new();
    parameters.insert(
        "COUNTRY_CODE".to_string(),
        JsonValue::String("FR".to_string()),
    );
    let params = GetRecipeParams {
        recipe_id: "weekly_report".to_string(),
        include_sql: true,
        include_queries: true,
        parameters: Some(parameters),
        placeholder_types: None,
        parameter_types: None,
    };
    let result = searcher.get_recipe(&params).await.json();
    let success = result
        .get("success")
        .expect("response missing 'success'")
        .as_bool()
        .expect("'success' should be boolean");
    assert!(
        success,
        "Expected get_recipe to succeed: {:?}",
        result.get("error")
    );
    let payload = result
        .get("data")
        .and_then(|d| d.as_object())
        .expect("get_recipe should return object payload");
    assert_eq!(
        payload
            .get("query_count")
            .and_then(serde_json::Value::as_u64),
        Some(2)
    );
    let queries = payload
        .get("queries")
        .and_then(serde_json::Value::as_array)
        .expect("queries should be present");
    assert_eq!(queries.len(), 2);
    assert!(queries[0].get("sql").and_then(|s| s.as_str()).is_some());
    let missing = payload
        .get("missing_parameters")
        .and_then(serde_json::Value::as_array)
        .expect("missing_parameters should be present");
    assert!(missing.is_empty(), "Expected no missing parameters");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_get_recipe_not_found_returns_error() {
    let searcher = get_searcher_with_fixture("recipes_by_analysis.json");
    let params = GetRecipeParams {
        recipe_id: "does_not_exist".to_string(),
        include_sql: false,
        include_queries: false,
        parameters: None,
        placeholder_types: None,
        parameter_types: None,
    };
    let result = searcher.get_recipe(&params).await.json();
    let success = result
        .get("success")
        .expect("response missing 'success'")
        .as_bool()
        .expect("'success' should be boolean");
    assert!(!success, "Expected invalid recipe_id to fail");
    assert!(
        result
            .get("error")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .contains("Recipe not found"),
        "Expected recipe not found error"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_search_recipes_discovers_manifest_driven_recipes() {
    let searcher = get_searcher_with_fixture("recipes_by_analysis.json");
    let params = SearchRecipesParams {
        query: String::new(),
        topic: String::from("marketplace"),
        include_queries: true,
        pagination: PaginationParams {
            limit: Some(20),
            offset: 0,
        },
    };
    let result = searcher.search_recipes(&params).await.json();
    let success = result
        .get("success")
        .expect("response missing 'success'")
        .as_bool()
        .expect("'success' should be boolean");
    assert!(
        success,
        "Expected manifest recipe search to succeed: {:?}",
        result.get("error")
    );

    let data = result
        .get("data")
        .and_then(serde_json::Value::as_array)
        .expect("search_recipes should return array data");
    let weekly = data
        .iter()
        .find(|item| {
            item.get("id").and_then(serde_json::Value::as_str) == Some("marketplace/weekly_report")
        })
        .expect("weekly_report recipe missing from manifest discovery");
    assert_eq!(
        weekly
            .get("query_count")
            .and_then(serde_json::Value::as_u64),
        Some(2)
    );
    let queries = weekly
        .get("queries")
        .and_then(serde_json::Value::as_array)
        .expect("queries should be present when include_queries=true");
    assert_eq!(queries.len(), 2);
    let query_names: Vec<&str> = queries.iter().filter_map(|query| query.as_str()).collect();
    assert!(
        query_names.contains(&"analysis__weekly_headline__01.sql")
            && query_names.contains(&"analysis__weekly_channel_mix__02.sql"),
        "Expected manifest discovery to include descriptive ordered SQL names"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_get_recipe_from_manifest_analysis_contains_compiled_sql() {
    let searcher = get_searcher_with_fixture("recipes_by_analysis.json");
    let mut parameters = HashMap::new();
    parameters.insert(
        "COUNTRY_CODE".to_string(),
        JsonValue::String("GB".to_string()),
    );
    let params = GetRecipeParams {
        recipe_id: "marketplace/weekly_report".to_string(),
        include_sql: true,
        include_queries: true,
        parameters: Some(parameters),
        placeholder_types: None,
        parameter_types: None,
    };
    let result = searcher.get_recipe(&params).await.json();
    let success = result
        .get("success")
        .expect("response missing 'success'")
        .as_bool()
        .expect("'success' should be boolean");
    assert!(
        success,
        "Expected get_recipe on manifest-backed recipe to succeed: {:?}",
        result.get("error")
    );

    let payload = result
        .get("data")
        .and_then(serde_json::Value::as_object)
        .expect("get_recipe should return object payload");
    let queries = payload
        .get("queries")
        .and_then(serde_json::Value::as_array)
        .expect("queries should be present");
    assert_eq!(queries.len(), 2);

    let first = &queries[0];
    assert_eq!(
        first.get("source").and_then(serde_json::Value::as_str),
        Some("manifest_analysis")
    );
    assert!(
        first
            .get("analysis_id")
            .and_then(serde_json::Value::as_str)
            .is_some(),
        "Expected analysis_id in manifest-backed query metadata"
    );
    let sql = first
        .get("sql")
        .and_then(serde_json::Value::as_str)
        .expect("sql should be present when include_sql=true");
    assert!(
        sql.contains("select 'weekly' as report_scope"),
        "Expected compiled SQL to be resolved from manifest analysis"
    );
    assert!(
        sql.contains("'GB' as country_code"),
        "Expected placeholders to be substituted using provided parameters"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_get_recipe_reports_missing_parameters_without_sql_rendering() {
    let searcher = get_searcher_with_fixture("recipes_by_analysis.json");
    let params = GetRecipeParams {
        recipe_id: "marketplace/weekly_report".to_string(),
        include_sql: false,
        include_queries: true,
        parameters: None,
        placeholder_types: None,
        parameter_types: None,
    };
    let result = searcher.get_recipe(&params).await.json();
    let success = result
        .get("success")
        .and_then(serde_json::Value::as_bool)
        .expect("response missing success flag");
    assert!(
        success,
        "Expected get_recipe without SQL rendering to succeed"
    );
    let missing = result
        .get("data")
        .and_then(|value| value.get("missing_parameters"))
        .and_then(serde_json::Value::as_array)
        .expect("missing_parameters should be present");
    assert!(
        missing
            .iter()
            .any(|entry| entry.as_str() == Some("COUNTRY_CODE")),
        "Expected COUNTRY_CODE to be reported as missing"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_run_recipe_preflight_returns_structured_missing_parameters() {
    let searcher = get_searcher_with_fixture("recipes_by_analysis.json");
    let params = RunRecipeParams {
        recipe_id: "marketplace/weekly_report".to_string(),
        query_names: vec!["analysis__weekly_headline__01.sql".to_string()],
        query_indexes: vec![],
        stop_on_failure: true,
        include_sql: false,
        row_limit: Some(10),
        byte_limit: None,
        max_poll_seconds: None,
        poll_interval_ms: None,
        wait_timeout_s: None,
        parameters: None,
        placeholder_types: None,
        sql_parameter_types: None,
        parameter_types: None,
        fetch_all_chunks: None,
        max_chunks: None,
    };
    let result = searcher.run_recipe(&params).await.json();
    let success = result
        .get("success")
        .and_then(serde_json::Value::as_bool)
        .expect("response missing success flag");
    assert!(
        !success,
        "Expected run_recipe preflight to fail without required parameters"
    );
    let details = result
        .get("details")
        .and_then(serde_json::Value::as_object)
        .expect("structured details should be present");
    let missing = details
        .get("missing_parameters")
        .and_then(serde_json::Value::as_array)
        .expect("missing_parameters should be present");
    assert!(
        missing
            .iter()
            .any(|entry| entry.as_str() == Some("COUNTRY_CODE")),
        "Expected COUNTRY_CODE in missing_parameters"
    );
    assert!(
        details
            .get("by_query")
            .and_then(serde_json::Value::as_array)
            .is_some(),
        "Expected per-query validation details"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_run_recipe_preflight_returns_type_mismatch_details() {
    let searcher = get_searcher_with_fixture("recipes_by_analysis.json");
    let mut parameters = HashMap::new();
    parameters.insert(
        "COUNTRY_CODE".to_string(),
        JsonValue::String("FR".to_string()),
    );
    parameters.insert(
        "TOP_N".to_string(),
        JsonValue::String("not_a_number".to_string()),
    );
    let params = RunRecipeParams {
        recipe_id: "marketplace/weekly_report".to_string(),
        query_names: vec!["analysis__weekly_headline__01.sql".to_string()],
        query_indexes: vec![],
        stop_on_failure: true,
        include_sql: false,
        row_limit: Some(10),
        byte_limit: None,
        max_poll_seconds: None,
        poll_interval_ms: None,
        wait_timeout_s: None,
        parameters: Some(parameters),
        placeholder_types: None,
        sql_parameter_types: None,
        parameter_types: None,
        fetch_all_chunks: None,
        max_chunks: None,
    };
    let result = searcher.run_recipe(&params).await.json();
    let success = result
        .get("success")
        .and_then(serde_json::Value::as_bool)
        .expect("response missing success flag");
    assert!(
        !success,
        "Expected run_recipe preflight to fail on type mismatch"
    );
    let type_mismatches = result
        .get("details")
        .and_then(|value| value.get("type_mismatches"))
        .and_then(serde_json::Value::as_array)
        .expect("type_mismatches should be present");
    assert!(
        type_mismatches
            .iter()
            .any(
                |entry| entry.get("parameter").and_then(serde_json::Value::as_str) == Some("TOP_N")
            ),
        "Expected TOP_N type mismatch"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_get_recipe_include_sql_rejects_templated_raw_fallback() {
    let searcher = get_searcher_with_fixture("recipes_raw_fallback.json");
    let params = GetRecipeParams {
        recipe_id: "templated_report".to_string(),
        include_sql: true,
        include_queries: true,
        parameters: None,
        placeholder_types: None,
        parameter_types: None,
    };

    let result = searcher.get_recipe(&params).await.json();
    let success = result
        .get("success")
        .and_then(serde_json::Value::as_bool)
        .expect("response missing success flag");
    assert!(!success, "Expected templated raw fallback to fail");
    assert_eq!(
        result.get("error_code").and_then(serde_json::Value::as_str),
        Some("INVALID_PARAMS")
    );
    assert!(
        result
            .get("error")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .contains("compiled_code is unavailable and raw_code contains dbt/Jinja templating"),
        "Expected explicit jinja fallback error"
    );
    let details = result
        .get("details")
        .and_then(serde_json::Value::as_object)
        .expect("Expected structured details");
    assert_eq!(
        details.get("recipe_id").and_then(serde_json::Value::as_str),
        Some("marketplace/templated_report")
    );
    assert_eq!(
        details
            .get("sql_source")
            .and_then(serde_json::Value::as_str),
        Some("raw_code")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_get_recipe_include_sql_allows_plain_raw_fallback() {
    let searcher = get_searcher_with_fixture("recipes_raw_fallback.json");
    let params = GetRecipeParams {
        recipe_id: "plain_raw_report".to_string(),
        include_sql: true,
        include_queries: true,
        parameters: None,
        placeholder_types: None,
        parameter_types: None,
    };

    let result = searcher.get_recipe(&params).await.json();
    let success = result
        .get("success")
        .and_then(serde_json::Value::as_bool)
        .expect("response missing success flag");
    assert!(success, "Expected plain raw fallback to succeed");
    let queries = result
        .get("data")
        .and_then(|value| value.get("queries"))
        .and_then(serde_json::Value::as_array)
        .expect("queries should be present");
    let sql = queries[0]
        .get("sql")
        .and_then(serde_json::Value::as_str)
        .expect("sql should be rendered");
    assert_eq!(sql, "select 2 as plain_raw_value");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_get_recipe_include_sql_prefers_compiled_code_when_present() {
    let searcher = get_searcher_with_fixture("recipes_raw_fallback.json");
    let params = GetRecipeParams {
        recipe_id: "compiled_report".to_string(),
        include_sql: true,
        include_queries: true,
        parameters: None,
        placeholder_types: None,
        parameter_types: None,
    };

    let result = searcher.get_recipe(&params).await.json();
    let success = result
        .get("success")
        .and_then(serde_json::Value::as_bool)
        .expect("response missing success flag");
    assert!(success, "Expected compiled SQL to be preferred");
    let queries = result
        .get("data")
        .and_then(|value| value.get("queries"))
        .and_then(serde_json::Value::as_array)
        .expect("queries should be present");
    let sql = queries[0]
        .get("sql")
        .and_then(serde_json::Value::as_str)
        .expect("sql should be rendered");
    assert_eq!(sql, "select 3 as compiled_value");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_run_recipe_rejects_templated_raw_fallback_before_sql_execution() {
    let searcher = get_searcher_with_fixture("recipes_raw_fallback.json");
    let params = RunRecipeParams {
        recipe_id: "templated_report".to_string(),
        query_names: vec![],
        query_indexes: vec![],
        stop_on_failure: true,
        include_sql: false,
        row_limit: Some(10),
        byte_limit: None,
        max_poll_seconds: None,
        poll_interval_ms: None,
        wait_timeout_s: None,
        parameters: None,
        placeholder_types: None,
        sql_parameter_types: None,
        parameter_types: None,
        fetch_all_chunks: None,
        max_chunks: None,
    };

    let result = searcher.run_recipe(&params).await.json();
    let success = result
        .get("success")
        .and_then(serde_json::Value::as_bool)
        .expect("response missing success flag");
    assert!(
        !success,
        "Expected templated raw fallback to fail before warehouse execution"
    );
    assert_eq!(
        result.get("error_code").and_then(serde_json::Value::as_str),
        Some("INVALID_PARAMS")
    );
    assert!(
        result
            .get("error")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .contains("compiled_code is unavailable and raw_code contains dbt/Jinja templating"),
        "Expected explicit jinja fallback error"
    );
}
