use std::collections::{HashMap, HashSet};

use serde_json::Value as JsonValue;

use crate::error::Result;
use crate::manifest::entity::column_primary_key_bool;
use crate::manifest::search::ManifestSearch;
use crate::params::{DEFAULT_TEST_COVERAGE_COLUMNS_LIMIT, GetTestCoverageParams};
use crate::responses::SuccessResponse;
use crate::tools::helpers::test_type_from_json;
use tracing::instrument;

impl ManifestSearch {
    /// Calculate test coverage for a dbt entity.
    ///
    /// # Errors
    /// Returns an error if the entity or related tests cannot be resolved.
    #[instrument(skip(self, params), fields(tool = "get_test_coverage", id_or_name = %params.id_or_name, include_full = params.include_full))]
    #[allow(clippy::too_many_lines)]
    pub async fn get_test_coverage(&self, params: &GetTestCoverageParams) -> Result<JsonValue> {
        let resource_type = params.resource_type.as_deref();
        let entity_id = self.resolve_single_id(&params.id_or_name, resource_type)?;
        let Some(entity) = self.get_entity(&entity_id).await? else {
            return Err(self.entity_not_found(&entity_id, resource_type));
        };
        let entity_json = entity.to_json_value();

        let test_ids = self
            .tests_by_entity
            .get(&entity_id)
            .cloned()
            .unwrap_or_default();

        let mut schema_tests: Vec<JsonValue> = Vec::new();
        let mut data_tests: Vec<JsonValue> = Vec::new();
        let mut test_types: HashMap<String, usize> = HashMap::new();
        let mut columns_tested: HashSet<String> = HashSet::new();

        for test_id in &test_ids {
            if let Some(test) = self.get_entity(test_id).await? {
                let test_json = test.to_json_value();
                let test_type = test_type_from_json(&test_json);

                *test_types.entry(test_type.to_string()).or_default() += 1;

                let is_schema_test = test_json
                    .get("test_metadata")
                    .is_some_and(|meta| !meta.is_null());

                let test_info = if params.include_full {
                    serde_json::json!({
                        "unique_id": test_id,
                        "name": test_json.get("name"),
                        "test_type": test_type,
                        "column_name": test_json.get("column_name"),
                        "severity": test_json.get("config").and_then(|c| c.get("severity")),
                        "tags": test_json.get("tags"),
                        "test_metadata": test_json.get("test_metadata")
                    })
                } else {
                    serde_json::json!({
                        "unique_id": test_id,
                        "name": test_json.get("name"),
                        "test_type": test_type,
                        "column_name": test_json.get("column_name")
                    })
                };

                if is_schema_test {
                    if let Some(col) = test_json.get("column_name").and_then(|c| c.as_str()) {
                        columns_tested.insert(col.to_string());
                    }
                    schema_tests.push(test_info);
                } else {
                    data_tests.push(test_info);
                }
            }
        }

        let entity_columns: HashSet<String> = entity.column_names().into_iter().collect();

        let mut columns_without_tests: Vec<String> = entity_columns
            .difference(&columns_tested)
            .cloned()
            .collect();
        let columns_total_without_tests = columns_without_tests.len();
        let columns_limit = params
            .columns_limit
            .unwrap_or(DEFAULT_TEST_COVERAGE_COLUMNS_LIMIT);
        let columns_truncated = columns_limit > 0 && columns_total_without_tests > columns_limit;
        if columns_truncated {
            columns_without_tests.truncate(columns_limit);
        }

        let mut coverage_gaps: Vec<JsonValue> = Vec::new();

        if let Some(cols) = entity_json.get("columns").and_then(|c| c.as_object()) {
            for (col_name, col_info) in cols {
                let is_pk = column_primary_key_bool(col_info);

                if is_pk {
                    let key = format!("{}:{col_name}", &entity_id);
                    let col_tests = self.tests_by_column.get(&key).cloned().unwrap_or_default();

                    let has_unique = self.test_exists(&col_tests, "unique").await?;
                    let has_not_null = self.test_exists(&col_tests, "not_null").await?;

                    if !has_unique {
                        coverage_gaps.push(serde_json::json!({
                            "type": "missing_unique_test",
                            "severity": "high",
                            "column": col_name,
                            "message": format!("Primary key column '{}' lacks a unique test", col_name)
                        }));
                    }
                    if !has_not_null {
                        coverage_gaps.push(serde_json::json!({
                            "type": "missing_not_null_test",
                            "severity": "high",
                            "column": col_name,
                            "message": format!("Primary key column '{}' lacks a not_null test", col_name)
                        }));
                    }
                }
            }
        }

        if columns_total_without_tests > 0 && columns_total_without_tests <= 5 {
            for col in &columns_without_tests {
                coverage_gaps.push(serde_json::json!({
                    "type": "untested_column",
                    "severity": "low",
                    "column": col,
                    "message": format!("Column '{col}' has no tests")
                }));
            }
        } else if columns_total_without_tests > 5 {
            let column_count = columns_total_without_tests;
            coverage_gaps.push(serde_json::json!({
                "type": "many_untested_columns",
                "severity": "medium",
                "column_count": column_count,
                "message": format!("{column_count} columns have no tests")
            }));
        }

        let total_columns = entity_columns.len();
        let tested_columns = columns_tested.len();
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_precision_loss,
            clippy::cast_sign_loss
        )]
        let coverage_pct = if total_columns > 0 {
            (tested_columns as f64 / total_columns as f64 * 100.0).round() as usize
        } else {
            0
        };

        Ok(serde_json::to_value(SuccessResponse::new(
            serde_json::json!({
                "unique_id": &entity_id,
                "name": entity.name,
                "resource_type": entity.resource_type,
                "summary": {
                    "total_tests": test_ids.len(),
                    "schema_tests": schema_tests.len(),
                    "data_tests": data_tests.len(),
                    "test_types": test_types,
                    "columns_tested": tested_columns,
                    "columns_total": total_columns,
                    "coverage_percentage": coverage_pct
                },
                "schema_tests": schema_tests,
                "data_tests": data_tests,
                "columns_without_tests": columns_without_tests,
                "columns_without_tests_truncated": columns_truncated,
                "columns_without_tests_total": columns_total_without_tests,
                "coverage_gaps": coverage_gaps
            }),
            test_ids.len(),
        ))?)
    }

    async fn test_exists(&self, test_ids: &[String], test_name: &str) -> Result<bool> {
        for tid in test_ids {
            if let Some(test) = self.get_entity(tid).await? {
                let test_json = test.to_json_value();
                if test_json
                    .get("test_metadata")
                    .and_then(|tm| tm.get("name"))
                    .and_then(|n| n.as_str())
                    == Some(test_name)
                {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }
}
