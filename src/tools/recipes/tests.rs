use super::*;
use std::path::PathBuf;

#[test]
fn test_query_order_parsing() {
    assert_eq!(parse_query_order("query__1.sql"), 1);
    assert_eq!(parse_query_order("query_2.sql"), 2);
    assert_eq!(parse_query_order("analysis__weekly_headline__01.sql"), 1);
    assert_eq!(parse_query_order("query.sql"), usize::MAX);
    assert_eq!(parse_query_order("query_foo.sql"), usize::MAX);
}

#[test]
fn test_select_recipe_queries_by_name_and_index() {
    let recipe = RecipeRecord {
        id: "marketing/retention".to_string(),
        path: PathBuf::from("/tmp/recipes/marketing/retention"),
        queries: vec![
            RecipeQuery {
                name: "query__2.sql".to_string(),
                path: PathBuf::from("/tmp/q2.sql"),
                order: 2,
                source: RecipeQuerySource::ManifestAnalysis {
                    analysis_id: "analysis.test.query_2".to_string(),
                },
            },
            RecipeQuery {
                name: "query__1.sql".to_string(),
                path: PathBuf::from("/tmp/q1.sql"),
                order: 1,
                source: RecipeQuerySource::ManifestAnalysis {
                    analysis_id: "analysis.test.query_1".to_string(),
                },
            },
        ],
    };

    let by_name = select_recipe_queries(
        &recipe,
        &RunRecipeParams {
            recipe_id: "marketing/retention".to_string(),
            query_names: vec!["query__1".to_string()],
            query_indexes: vec![],
            stop_on_failure: true,
            include_sql: false,
            row_limit: None,
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
        },
    )
    .expect("Expected query by name");
    assert_eq!(by_name.len(), 1);
    assert_eq!(by_name[0].name, "query__1.sql");

    let by_index = select_recipe_queries(
        &recipe,
        &RunRecipeParams {
            recipe_id: "marketing/retention".to_string(),
            query_names: vec![],
            query_indexes: vec![2],
            stop_on_failure: false,
            include_sql: false,
            row_limit: None,
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
        },
    )
    .expect("Expected query by index");
    assert_eq!(by_index.len(), 1);
    assert_eq!(by_index[0].name, "query__2.sql");

    let all = select_recipe_queries(
        &recipe,
        &RunRecipeParams {
            recipe_id: "marketing/retention".to_string(),
            query_names: vec![],
            query_indexes: vec![],
            stop_on_failure: false,
            include_sql: false,
            row_limit: None,
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
        },
    )
    .expect("Expected all queries");
    assert_eq!(all[0].order, 1);
    assert_eq!(all[1].order, 2);
}

#[test]
fn test_select_recipe_queries_invalid_index() {
    let recipe = RecipeRecord {
        id: "marketing/retention".to_string(),
        path: PathBuf::from("/tmp/recipes/marketing/retention"),
        queries: vec![RecipeQuery {
            name: "query__1.sql".to_string(),
            path: PathBuf::from("/tmp/q1.sql"),
            order: 1,
            source: RecipeQuerySource::ManifestAnalysis {
                analysis_id: "analysis.test.query_1".to_string(),
            },
        }],
    };

    let err = select_recipe_queries(
        &recipe,
        &RunRecipeParams {
            recipe_id: "marketing/retention".to_string(),
            query_names: vec![],
            query_indexes: vec![2],
            stop_on_failure: false,
            include_sql: false,
            row_limit: None,
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
        },
    )
    .expect_err("Expected invalid index error");
    assert!(err.to_string().contains("out of range"));
}

#[test]
fn test_apply_runtime_parameter_substitution() {
    let mut parameters = HashMap::new();
    parameters.insert("COUNTRY".to_string(), JsonValue::String("us".to_string()));
    parameters.insert("IS_ACTIVE".to_string(), JsonValue::Bool(true));
    parameters.insert(
        "TARGET_TABLE".to_string(),
        JsonValue::String("analytics__events".to_string()),
    );
    let mut parameter_types = HashMap::new();
    parameter_types.insert("TARGET_TABLE".to_string(), "identifier".to_string());

    let rendered = apply_runtime_parameter_substitution(
            "select * from __TARGET_TABLE__ where country = '__COUNTRY__' and is_active = __IS_ACTIVE__",
            &parameters,
            Some(&parameter_types),
        )
        .expect("render");

    assert_eq!(
        rendered,
        "select * from analytics__events where country = 'us' and is_active = true"
    );
}

#[test]
fn test_apply_runtime_parameter_substitution_missing_param() {
    let mut parameters = HashMap::new();
    parameters.insert("OTHER".to_string(), JsonValue::String("foo".to_string()));
    let err =
        apply_runtime_parameter_substitution("select * from __TARGET_TABLE__", &parameters, None)
            .expect_err("expected missing param");
    assert!(
        err.to_string()
            .contains("Missing runtime parameter for placeholder '__TARGET_TABLE__'")
    );
}

#[test]
fn test_resolve_recipe_placeholder_types_legacy_fallback_normalizes_keys() {
    let mut legacy = HashMap::new();
    legacy.insert("country_code".to_string(), "string".to_string());

    let resolved = resolve_recipe_placeholder_types(None, Some(&legacy), "get_recipe")
        .expect("expected successful fallback resolution")
        .expect("expected merged fallback map");

    assert_eq!(resolved.get("country_code"), Some(&"string".to_string()));
}

#[test]
fn test_resolve_recipe_placeholder_types_rejects_conflicting_hints() {
    let mut primary = HashMap::new();
    primary.insert("COUNTRY_CODE".to_string(), "identifier".to_string());
    let mut legacy = HashMap::new();
    legacy.insert("country_code".to_string(), "string".to_string());

    let err = resolve_recipe_placeholder_types(Some(&primary), Some(&legacy), "run_recipe")
        .expect_err("expected conflicting hint error");
    let message = err.to_string();
    assert!(message.contains("conflicting type hints"));
    assert!(message.contains("placeholder_types"));
    assert!(message.contains("parameter_types"));
}

#[test]
fn test_recipe_query_jinja_markers_detects_comment_blocks() {
    let markers = recipe_query_jinja_markers("{# comment #}\nselect 1");
    assert_eq!(markers, vec!["{#"]);
}

#[test]
fn test_recipe_query_jinja_markers_ignores_sql_literals() {
    let markers = recipe_query_jinja_markers(
        "select '{{' as open_token, '{%' as block_token, '{#' as comment_token",
    );
    assert!(markers.is_empty());
}

#[test]
fn test_recipe_query_jinja_markers_ignores_sql_comments() {
    let markers = recipe_query_jinja_markers(
        "-- {{ in line comment }}\nselect 1 /* {% in block comment %} */",
    );
    assert!(markers.is_empty());
}

#[test]
fn test_recipe_query_jinja_markers_ignores_backslash_escaped_quote_literals() {
    let markers =
        recipe_query_jinja_markers("select E'It\\'s {{ok}} and {% raw %} and {# note #}' as msg");
    assert!(markers.is_empty());
}

#[test]
fn test_recipe_query_jinja_markers_detects_after_standard_sql_backslash_literal() {
    let markers = recipe_query_jinja_markers("select 'C:\\' as path; {{ ref('model') }}");
    assert_eq!(markers, vec!["{{"]);
}

#[test]
fn test_recipe_query_jinja_markers_ignores_dollar_quoted_literals() {
    let markers = recipe_query_jinja_markers("select $$ {{ok}} {% block %} {# note #} $$ as body");
    assert!(markers.is_empty());
}

#[test]
fn test_recipe_query_jinja_markers_ignores_tagged_dollar_quoted_literals() {
    let markers =
        recipe_query_jinja_markers("select $tag$ {{ok}} {% block %} {# note #} $tag$ as body");
    assert!(markers.is_empty());
}
