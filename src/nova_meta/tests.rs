use std::fs;

use serde_json::Value as JsonValue;
use tempfile::TempDir;

use super::{
    NovaMetaFindingSeverity, NovaMetaResourceKind, NovaMetaTargetSelector,
    NovaMetaValidationOptions, nova_meta_from_mapping, validate_nova_meta,
};
use serde_yaml::Value as YamlValue;

fn write_fixture(temp_dir: &TempDir, relative_path: &str, contents: &str) -> std::path::PathBuf {
    let path = temp_dir.path().join(relative_path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent");
    }
    fs::write(&path, contents).expect("write fixture");
    path
}

#[test]
fn validate_nova_meta_accepts_valid_model_and_column() {
    let temp_dir = TempDir::new().expect("temp dir");
    write_fixture(
        &temp_dir,
        "models/orders.yml",
        r#"
version: 2
models:
  - name: fct_orders
    meta:
      nova:
        canonical: true
        grain:
          primary_key: ["order_id"]
          time_field: order_date
          dimensions: ["country_code"]
        measures:
          - name: orders
            type: count_distinct
            expression: "count(distinct order_id)"
            description: "Orders"
            field: order_id
    columns:
      - name: order_id
        meta:
          nova:
            role: identifier
      - name: order_date
        meta:
          nova:
            role: time
      - name: country_code
        meta:
          nova:
            role: dimension
        "#,
    );

    let report = validate_nova_meta(&NovaMetaValidationOptions {
        project_dir: temp_dir.path().to_path_buf(),
        paths: Vec::new(),
        selector: NovaMetaTargetSelector::default(),
    });

    assert_eq!(report.error_count, 0);
    assert_eq!(report.target_count, 4);
}

#[test]
fn nova_meta_from_mapping_merges_legacy_over_config_meta() {
    let yaml = serde_yaml::from_str::<YamlValue>(
        r#"
name: fct_orders
config:
  meta:
    nova:
      role: measure
      semantic_type: order
      governance:
        pii: false
        compliance: ["soc2"]
meta:
  nova:
    role: dimension
    semantic_type: null
    governance:
      sensitivity: restricted
"#,
    )
    .expect("YAML");
    let mapping = yaml.as_mapping().expect("mapping");

    let nova = nova_meta_from_mapping(mapping).expect("nova meta");

    assert_eq!(nova["role"].as_str(), Some("dimension"));
    assert_eq!(nova["semantic_type"].as_str(), Some("order"));
    assert_eq!(nova["governance"]["pii"].as_bool(), Some(false));
    assert_eq!(
        nova["governance"]["sensitivity"].as_str(),
        Some("restricted")
    );
    assert_eq!(
        nova["governance"]["compliance"]
            .as_array()
            .and_then(|values| values.first())
            .and_then(JsonValue::as_str),
        Some("soc2")
    );
}

#[test]
fn nova_meta_from_mapping_falls_back_when_legacy_nova_is_null() {
    let yaml = serde_yaml::from_str::<YamlValue>(
        r"
name: fct_orders
meta:
  nova: null
config:
  meta:
    nova:
      role: dimension
",
    )
    .expect("YAML");
    let mapping = yaml.as_mapping().expect("mapping");

    let nova = nova_meta_from_mapping(mapping).expect("nova meta");

    assert_eq!(nova["role"].as_str(), Some("dimension"));
}

#[test]
fn validate_nova_meta_accepts_config_meta_only_resources_and_columns() {
    let temp_dir = TempDir::new().expect("temp dir");
    write_fixture(
        &temp_dir,
        "models/config-meta.yml",
        r#"
version: 2
sources:
  - name: raw_app
    config:
      meta:
        nova:
          canonical: true
    tables:
      - name: orders
        config:
          meta:
            nova:
              grain:
                primary_key: ["order_id"]
                time_field: order_date
        columns:
          - name: order_id
            config:
              meta:
                nova:
                  role: identifier
          - name: order_date
            config:
              meta:
                nova:
                  role: time
models:
  - name: fct_customers
    config:
      meta:
        nova:
          grain:
            primary_key: ["customer_id"]
            time_field: signup_date
    columns:
      - name: customer_id
        config:
          meta:
            nova:
              role: identifier
      - name: signup_date
        config:
          meta:
            nova:
              role: time
metrics:
  - name: customer_count
    config:
      meta:
        nova:
          metric:
            name: customer_count
            expression: "count(distinct customer_id)"
"#,
    );

    let report = validate_nova_meta(&NovaMetaValidationOptions {
        project_dir: temp_dir.path().to_path_buf(),
        paths: Vec::new(),
        selector: NovaMetaTargetSelector::default(),
    });

    assert_eq!(report.error_count, 0);
    assert_eq!(report.target_count, 8);
}

#[test]
fn validate_nova_meta_selected_column_matches_config_meta_only_target() {
    let temp_dir = TempDir::new().expect("temp dir");
    let path = write_fixture(
        &temp_dir,
        "models/orders.yml",
        r"
version: 2
models:
  - name: fct_orders
    columns:
      - name: order_id
        config:
          meta:
            nova:
              role: identifier
",
    );

    let report = validate_nova_meta(&NovaMetaValidationOptions {
        project_dir: temp_dir.path().to_path_buf(),
        paths: vec![path],
        selector: NovaMetaTargetSelector {
            resource_kind: Some(NovaMetaResourceKind::Model),
            resource_name: Some("fct_orders".to_string()),
            column: Some("order_id".to_string()),
        },
    });

    assert_eq!(report.target_count, 1);
    assert_eq!(report.error_count, 0);
}

#[test]
fn validate_nova_meta_runs_semantic_checks_for_config_meta_only_target() {
    let temp_dir = TempDir::new().expect("temp dir");
    write_fixture(
        &temp_dir,
        "models/orders.yml",
        r#"
version: 2
models:
  - name: fct_orders
    config:
      meta:
        nova:
          grain:
            primary_key: ["missing_id"]
    columns:
      - name: order_id
"#,
    );

    let report = validate_nova_meta(&NovaMetaValidationOptions {
        project_dir: temp_dir.path().to_path_buf(),
        paths: Vec::new(),
        selector: NovaMetaTargetSelector::default(),
    });

    assert_eq!(report.target_count, 1);
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.code == "missing_referenced_field")
    );
}

#[test]
fn validate_nova_meta_reports_schema_and_semantic_errors() {
    let temp_dir = TempDir::new().expect("temp dir");
    write_fixture(
        &temp_dir,
        "models/orders.yml",
        r#"
version: 2
models:
  - name: fct_orders
    meta:
      nova:
        metric:
          name: conversion_rate
        metrics:
          - name: conversion_rate
            recommended_filters:
              - field: missing_field
                operator: between
                values: ["web"]
    columns:
      - name: order_id
        "#,
    );

    let report = validate_nova_meta(&NovaMetaValidationOptions {
        project_dir: temp_dir.path().to_path_buf(),
        paths: Vec::new(),
        selector: NovaMetaTargetSelector::default(),
    });

    assert!(report.error_count >= 3);
    assert!(report.findings.iter().any(|finding| {
        finding.code == "metric_and_metrics_conflict"
            && finding.severity == NovaMetaFindingSeverity::Error
    }));
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.code == "missing_referenced_field")
    );
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.code == "invalid_filter_values")
    );
}

#[test]
fn validate_nova_meta_rejects_unsupported_filter_operator() {
    let temp_dir = TempDir::new().expect("temp dir");
    write_fixture(
        &temp_dir,
        "models/orders.yml",
        r#"
version: 2
models:
  - name: fct_orders
    meta:
      nova:
        metric:
          name: conversion_rate
          template: true
          expression: "sum(orders)"
          grain:
            dimensions: ["order_id"]
          recommended_filters:
            - field: order_id
              operator: gte
              values: ["100"]
    columns:
      - name: order_id
        "#,
    );

    let report = validate_nova_meta(&NovaMetaValidationOptions {
        project_dir: temp_dir.path().to_path_buf(),
        paths: Vec::new(),
        selector: NovaMetaTargetSelector::default(),
    });

    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.code == "unsupported_filter_operator")
    );
}

#[test]
fn validate_nova_meta_skips_field_existence_checks_for_metric_resources() {
    let temp_dir = TempDir::new().expect("temp dir");
    write_fixture(
        &temp_dir,
        "models/metrics.yml",
        r#"
version: 2
metrics:
  - name: orders_conversion
    meta:
      nova:
        metric:
          name: conversion_rate
          template: true
          expression: "sum(orders) / nullif(sum(sessions), 0)"
          grain:
            time_field: metric_date
            dimensions: ["country_code"]
          recommended_filters:
            - field: channel
              operator: "="
              values: ["web"]
        "#,
    );

    let report = validate_nova_meta(&NovaMetaValidationOptions {
        project_dir: temp_dir.path().to_path_buf(),
        paths: Vec::new(),
        selector: NovaMetaTargetSelector {
            resource_kind: Some(NovaMetaResourceKind::Metric),
            resource_name: Some("orders_conversion".to_string()),
            column: None,
        },
    });

    assert_eq!(report.error_count, 0);
    assert!(
        !report
            .findings
            .iter()
            .any(|finding| finding.code == "missing_referenced_field")
    );
}

#[test]
fn validate_nova_meta_can_target_single_resource_and_column() {
    let temp_dir = TempDir::new().expect("temp dir");
    let path = write_fixture(
        &temp_dir,
        "models/orders.yml",
        r"
version: 2
models:
  - name: fct_orders
    meta:
      nova:
        canonical: true
    columns:
      - name: order_id
        meta:
          nova:
            role: identifier
  - name: fct_sessions
    meta:
      nova:
        canonical: true
        ",
    );

    let report = validate_nova_meta(&NovaMetaValidationOptions {
        project_dir: temp_dir.path().to_path_buf(),
        paths: vec![path],
        selector: NovaMetaTargetSelector {
            resource_kind: Some(NovaMetaResourceKind::Model),
            resource_name: Some("fct_orders".to_string()),
            column: Some("order_id".to_string()),
        },
    });

    assert_eq!(report.target_count, 1);
    assert_eq!(report.error_count, 0);
}

#[test]
fn validate_nova_meta_reports_missing_meta_for_selected_resource() {
    let temp_dir = TempDir::new().expect("temp dir");
    write_fixture(
        &temp_dir,
        "models/orders.yml",
        r"
version: 2
models:
  - name: fct_orders
    columns:
      - name: order_id
",
    );

    let report = validate_nova_meta(&NovaMetaValidationOptions {
        project_dir: temp_dir.path().to_path_buf(),
        paths: Vec::new(),
        selector: NovaMetaTargetSelector {
            resource_kind: Some(NovaMetaResourceKind::Model),
            resource_name: Some("fct_orders".to_string()),
            column: None,
        },
    });

    assert_eq!(report.target_count, 0);
    assert_eq!(report.error_count, 1);
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.code == "selection_error")
    );
}

#[test]
fn validate_nova_meta_skips_default_ignored_directories() {
    let temp_dir = TempDir::new().expect("temp dir");
    write_fixture(
        &temp_dir,
        "models/orders.yml",
        r"
version: 2
models:
  - name: fct_orders
    meta:
      nova:
        canonical: true
",
    );
    write_fixture(
        &temp_dir,
        ".venv/ignored.yml",
        r"
version: 2
models:
  - name: ignored_model
    meta:
      nova:
        role: fact
",
    );
    write_fixture(
        &temp_dir,
        "target/ignored.yml",
        r"
version: 2
models:
  - name: ignored_target_model
    meta:
      nova:
        role: fact
",
    );

    let report = validate_nova_meta(&NovaMetaValidationOptions {
        project_dir: temp_dir.path().to_path_buf(),
        paths: Vec::new(),
        selector: NovaMetaTargetSelector::default(),
    });

    assert_eq!(report.target_count, 1);
    assert_eq!(report.error_count, 0);
    assert!(
        report
            .findings
            .iter()
            .all(|finding| !finding.file_path.starts_with(".venv/"))
    );
    assert!(
        report
            .findings
            .iter()
            .all(|finding| !finding.file_path.starts_with("target/"))
    );
}

#[test]
fn validate_nova_meta_allows_explicit_path_inside_ignored_directory() {
    let temp_dir = TempDir::new().expect("temp dir");
    write_fixture(
        &temp_dir,
        ".venv/ignored.yml",
        r"
version: 2
models:
  - name: ignored_model
    meta:
      nova:
        canonical: true
",
    );

    let report = validate_nova_meta(&NovaMetaValidationOptions {
        project_dir: temp_dir.path().to_path_buf(),
        paths: vec![std::path::PathBuf::from(".venv")],
        selector: NovaMetaTargetSelector::default(),
    });

    assert_eq!(report.target_count, 1);
    assert_eq!(report.error_count, 0);
}
