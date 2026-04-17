use std::collections::HashMap;

use serde::Serialize;
use serde_json::Value as JsonValue;

use crate::error::{DbtNovaError, Result};
use crate::manifest::entity::{
    Entity, NovaMeta, column_primary_key_bool, normalized_column_meta_json,
};
use crate::manifest::search::ManifestSearch;
use crate::params::{ContextMode, GetContextParams};
use crate::responses::SuccessResponse;
use crate::tools::helpers::{
    dependency_hints_from_json, primary_key_columns_from_entity, test_type_from_json,
};
use tracing::instrument;

const TEST_CONTEXT_COLUMNS_WITHOUT_TESTS_LIMIT: usize = 20;

#[derive(Debug, Serialize)]
pub struct ContextResponse {
    pub unique_id: String,
    pub entity: EntityContext,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upstream: Option<LineageContext>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub downstream: Option<LineageContext>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tests: Option<TestContext>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub docs: Option<Vec<DocContext>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub analysis_hints: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct EntityContext {
    pub name: String,
    pub resource_type: String,
    pub package_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub database: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_file_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_path: Option<String>,
    pub tags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compiled_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary_key_columns: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grain_summary: Option<JsonValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nova_summary: Option<JsonValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub columns: Option<Vec<ColumnContext>>,
}

#[derive(Debug, Serialize)]
pub struct ColumnContext {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<JsonValue>,
    pub tests: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct LineageContext {
    pub count: usize,
    pub truncated: bool,
    pub by_type: HashMap<String, usize>,
    pub entities: Vec<LineageEntity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lineage_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lineage_hints: Option<JsonValue>,
}

#[derive(Debug, Serialize)]
pub struct LineageEntity {
    pub unique_id: String,
    pub name: String,
    pub resource_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct TestContext {
    pub total_tests: usize,
    pub schema_tests: usize,
    pub data_tests: usize,
    pub coverage_percentage: usize,
    pub columns_tested: usize,
    pub columns_total: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub columns_without_tests: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub columns_without_tests_truncated: Option<bool>,
    pub by_type: HashMap<String, usize>,
    pub gaps: Vec<CoverageGap>,
    pub tests: Vec<TestSummary>,
}

#[derive(Debug, Serialize)]
pub struct CoverageGap {
    pub gap_type: String,
    pub severity: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<String>,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct TestSummary {
    pub unique_id: String,
    pub name: String,
    pub test_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column_name: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct DocContext {
    pub unique_id: String,
    pub name: String,
    pub block_contents: String,
    pub relation: String,
}

impl ManifestSearch {
    /// Get comprehensive context for a dbt entity including lineage, tests, and docs.
    ///
    /// # Errors
    /// Returns an error if the entity cannot be resolved or related data fails to load.
    #[instrument(skip(self, params), fields(tool = "get_context", id_or_name = %params.id_or_name, resource_type = ?params.resource_type))]
    pub async fn get_context(&self, params: &GetContextParams) -> Result<JsonValue> {
        // Resolve entity
        let (entity_id, entity) = self
            .resolve_entity(&params.id_or_name, params.resource_type.as_deref())
            .await?;

        // Build entity context
        let entity_context =
            self.build_entity_context(&entity_id, &entity, params.context_mode, params)?;

        // Build optional sections
        let entity_json = entity.to_json_value();
        let upstream = if params.include_upstream {
            Some(self.build_lineage_context(
                &entity_id,
                &entity_json,
                "upstream",
                params.limits.lineage_depth,
                params.limits.upstream_limit,
                params.context_mode,
            )?)
        } else {
            None
        };

        let downstream = if params.include_downstream {
            Some(self.build_lineage_context(
                &entity_id,
                &entity_json,
                "downstream",
                params.limits.lineage_depth,
                params.limits.downstream_limit,
                params.context_mode,
            )?)
        } else {
            None
        };

        let tests = if params.include_tests {
            Some(self.build_test_context(&entity_id)?)
        } else {
            None
        };

        let docs = if params.include_docs {
            Some(self.build_docs_context(&entity_id, &entity)?)
        } else {
            None
        };

        let mut analysis_hints: Vec<String> = Vec::new();
        if let Some(lineage) = &upstream
            && lineage
                .lineage_status
                .as_deref()
                .is_some_and(|s| s == "no_dependencies_recorded")
        {
            analysis_hints.push(
                "Upstream lineage is empty because the manifest has no dependency metadata. Consider rebuilding the manifest or using get_sql to trace refs.".to_string(),
            );
        }
        if let Some(lineage) = &downstream
            && lineage
                .lineage_status
                .as_deref()
                .is_some_and(|s| s == "no_dependencies_recorded")
        {
            analysis_hints.push(
                "Downstream lineage is empty because the manifest has no dependency metadata. Consider rebuilding the manifest or searching for dependent models.".to_string(),
            );
        }

        let response = ContextResponse {
            unique_id: entity_id.clone(),
            entity: entity_context,
            upstream,
            downstream,
            tests,
            docs,
            analysis_hints: if analysis_hints.is_empty() {
                None
            } else {
                Some(analysis_hints)
            },
        };

        Ok(serde_json::to_value(SuccessResponse::new(response, 1))?)
    }

    fn build_entity_context(
        &self,
        entity_id: &str,
        entity: &Entity,
        context_mode: ContextMode,
        params: &GetContextParams,
    ) -> Result<EntityContext> {
        let columns = if params.include_columns {
            Some(self.build_columns_context(entity_id, entity)?)
        } else {
            None
        };

        let entity_json = entity.to_json_value();
        let primary_key_columns = {
            let keys = primary_key_columns_from_entity(&entity_json);
            if keys.is_empty() { None } else { Some(keys) }
        };
        let nova_summary = entity.nova_meta.as_ref().and_then(build_nova_summary);
        let grain_summary =
            build_grain_summary(entity.nova_meta.as_ref(), primary_key_columns.as_deref());
        let (raw_code, compiled_code) = if params.include_sql {
            (
                entity_json
                    .get("raw_code")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                entity_json
                    .get("compiled_code")
                    .and_then(|v| v.as_str())
                    .map(String::from),
            )
        } else {
            (None, None)
        };

        let description = if context_mode == ContextMode::Engineer {
            None
        } else {
            entity.description.clone().filter(|s| !s.is_empty())
        };

        Ok(EntityContext {
            name: entity.name.clone().unwrap_or_default(),
            resource_type: entity.resource_type.clone().unwrap_or_default(),
            package_name: entity.package_name.clone().unwrap_or_default(),
            description,
            database: entity.database.clone(),
            schema: entity.schema.clone(),
            original_file_path: entity.original_file_path.clone(),
            file_path: entity_json
                .get("path")
                .and_then(|v| v.as_str())
                .map(String::from)
                .or_else(|| entity.original_file_path.clone()),
            tags: entity.tags.clone(),
            raw_code,
            compiled_code,
            primary_key_columns,
            grain_summary,
            nova_summary,
            columns,
        })
    }

    fn build_columns_context(
        &self,
        entity_id: &str,
        entity: &Entity,
    ) -> Result<Vec<ColumnContext>> {
        let entity_json = entity.to_json_value();
        let Some(columns_obj) = entity_json.get("columns").and_then(|c| c.as_object()) else {
            return Ok(vec![]);
        };

        let mut columns: Vec<ColumnContext> = Vec::new();

        for (col_name, col_info) in columns_obj {
            // Get tests for this column
            let key = format!("{entity_id}:{col_name}");
            let mut col_tests: Vec<String> = Vec::new();
            if let Some(test_ids) = self.tests_by_column.get(&key) {
                for tid in test_ids {
                    if let Some(test) = self.get_entity_archived(tid)? {
                        let test_json = test.to_json_value();
                        if let Some(name) = test_json
                            .get("test_metadata")
                            .and_then(|tm| tm.get("name"))
                            .and_then(|n| n.as_str())
                        {
                            col_tests.push(name.to_string());
                        }
                    }
                }
            }

            columns.push(ColumnContext {
                name: col_name.clone(),
                description: col_info
                    .get("description")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(String::from),
                data_type: col_info
                    .get("data_type")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                meta: normalized_column_meta_json(col_info),
                tests: col_tests,
            });
        }

        // Sort columns by name for consistent output
        columns.sort_by(|a, b| a.name.cmp(&b.name));

        Ok(columns)
    }

    fn build_lineage_context(
        &self,
        entity_id: &str,
        entity_json: &JsonValue,
        direction: &str,
        depth: usize,
        limit: usize,
        context_mode: ContextMode,
    ) -> Result<LineageContext> {
        let map = match direction {
            "upstream" => &self.parent_map,
            "downstream" => &self.child_map,
            _ => return Err(DbtNovaError::InvalidParams("Invalid direction".into())),
        };

        let mut visited = std::collections::HashSet::new();
        let mut result: Vec<String> = Vec::new();
        let mut current_level: Vec<String> = vec![entity_id.to_string()];
        let mut current_depth = 0;

        visited.insert(entity_id.to_string());

        while !current_level.is_empty() && current_depth < depth && result.len() < limit {
            let mut next_level: Vec<String> = Vec::new();

            for id in &current_level {
                if result.len() >= limit {
                    break;
                }
                if let Some(related) = map.get(id) {
                    for rel_id in related {
                        if result.len() >= limit {
                            break;
                        }
                        if !visited.contains(rel_id) {
                            visited.insert(rel_id.clone());
                            result.push(rel_id.clone());
                            next_level.push(rel_id.clone());
                        }
                    }
                }
            }

            current_level = next_level;
            current_depth += 1;
        }

        let truncated = result.len() >= limit;

        let (lineage_hints, hints_total) = dependency_hints_from_json(entity_json);
        let lineage_status = if result.is_empty() {
            if hints_total == 0 {
                Some("no_dependencies_recorded".to_string())
            } else {
                Some("dependencies_unresolved_or_filtered".to_string())
            }
        } else {
            Some("ok".to_string())
        };

        // Build by_type summary
        let mut by_type: HashMap<String, usize> = HashMap::new();
        let mut entities: Vec<LineageEntity> = Vec::new();

        for id in &result {
            if let Some(entity) = self.get_entity_archived(id)? {
                let rt = entity.resource_type_str().unwrap_or("unknown");
                *by_type.entry(rt.to_string()).or_default() += 1;

                let description = if context_mode == ContextMode::Engineer {
                    None
                } else {
                    entity
                        .description_str()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                };

                entities.push(LineageEntity {
                    unique_id: id.clone(),
                    name: entity.name_str().unwrap_or("").to_string(),
                    resource_type: rt.to_string(),
                    description,
                });
            }
        }

        Ok(LineageContext {
            count: entities.len(),
            truncated,
            by_type,
            entities,
            lineage_status,
            lineage_hints: Some(lineage_hints),
        })
    }

    #[allow(clippy::too_many_lines)]
    fn build_test_context(&self, entity_id: &str) -> Result<TestContext> {
        let test_ids = self
            .tests_by_entity
            .get(entity_id)
            .cloned()
            .unwrap_or_default();

        let mut schema_tests_count = 0;
        let mut data_tests_count = 0;
        let mut by_type: HashMap<String, usize> = HashMap::new();
        let mut tests: Vec<TestSummary> = Vec::new();
        let mut columns_tested = std::collections::HashSet::new();

        for test_id in &test_ids {
            if let Some(test) = self.get_entity_archived(test_id)? {
                let test_json = test.to_json_value();
                let test_type = test_type_from_json(&test_json);

                *by_type.entry(test_type.to_string()).or_default() += 1;

                let is_schema_test = test_json
                    .get("test_metadata")
                    .is_some_and(|meta| !meta.is_null());

                if is_schema_test {
                    schema_tests_count += 1;
                    if let Some(col) = test_json.get("column_name").and_then(|c| c.as_str()) {
                        columns_tested.insert(col.to_string());
                    }
                } else {
                    data_tests_count += 1;
                }

                tests.push(TestSummary {
                    unique_id: test_id.clone(),
                    name: test.name_str().unwrap_or("").to_string(),
                    test_type: test_type.to_string(),
                    column_name: test_json
                        .get("column_name")
                        .and_then(|c| c.as_str())
                        .map(String::from),
                });
            }
        }

        // Calculate coverage
        let entity = self.get_entity_archived(entity_id)?;
        let total_columns = entity.as_ref().map_or(0, |e| e.column_names_iter().count());

        let columns_tested_count = columns_tested.len();
        let coverage_percentage = (columns_tested_count.saturating_mul(100) + total_columns / 2)
            .checked_div(total_columns)
            .unwrap_or(100);

        let mut columns_without_tests: Vec<String> = Vec::new();
        if let Some(entity) = entity.as_ref() {
            for column in entity.column_names_iter() {
                if !columns_tested.contains(column) {
                    columns_without_tests.push(column.to_string());
                }
            }
        }
        let columns_without_tests_truncated =
            columns_without_tests.len() > TEST_CONTEXT_COLUMNS_WITHOUT_TESTS_LIMIT;
        if columns_without_tests_truncated {
            columns_without_tests.truncate(TEST_CONTEXT_COLUMNS_WITHOUT_TESTS_LIMIT);
        }

        // Build gaps
        let mut gaps: Vec<CoverageGap> = Vec::new();
        let mut pk_columns: std::collections::HashSet<String> = std::collections::HashSet::new();

        if let Some(entity) = entity.as_ref() {
            let entity_json = entity.to_json_value();
            if let Some(cols) = entity_json.get("columns").and_then(|c| c.as_object()) {
                for (col_name, col_info) in cols {
                    let is_pk = column_primary_key_bool(col_info);

                    if is_pk {
                        pk_columns.insert(col_name.clone());
                    }

                    if is_pk && !columns_tested.contains(col_name) {
                        gaps.push(CoverageGap {
                            gap_type: "missing_pk_test".to_string(),
                            severity: "high".to_string(),
                            column: Some(col_name.clone()),
                            message: format!("Primary key column '{col_name}' lacks tests"),
                        });
                    }
                }
            }
        }

        for col_name in &columns_without_tests {
            if pk_columns.contains(col_name) {
                continue;
            }
            gaps.push(CoverageGap {
                gap_type: "untested_column".to_string(),
                severity: "low".to_string(),
                column: Some(col_name.clone()),
                message: format!("Column '{col_name}' has no tests"),
            });
        }

        Ok(TestContext {
            total_tests: test_ids.len(),
            schema_tests: schema_tests_count,
            data_tests: data_tests_count,
            coverage_percentage,
            columns_tested: columns_tested_count,
            columns_total: total_columns,
            columns_without_tests,
            columns_without_tests_truncated: columns_without_tests_truncated.then_some(true),
            by_type,
            gaps,
            tests,
        })
    }

    fn build_docs_context(&self, _entity_id: &str, entity: &Entity) -> Result<Vec<DocContext>> {
        let mut docs: Vec<DocContext> = Vec::new();

        let package = entity.package_name.as_deref().unwrap_or("");
        let entity_name = entity.name.as_deref().unwrap_or("");

        if let Some(doc_ids) = self.by_resource_type.get("doc") {
            for doc_id in doc_ids {
                if let Some(doc) = self.get_entity_archived(doc_id)? {
                    let doc_json = doc.to_json_value();
                    let doc_package = doc_json
                        .get("package_name")
                        .and_then(|p| p.as_str())
                        .unwrap_or("");
                    let doc_name = doc_json.get("name").and_then(|n| n.as_str()).unwrap_or("");
                    let block_contents = doc_json
                        .get("block_contents")
                        .and_then(|b| b.as_str())
                        .unwrap_or("");

                    // Check if doc is related to this entity
                    let relation = if doc_package == package && doc_name == entity_name {
                        Some("same_name".to_string())
                    } else if !entity_name.is_empty()
                        && block_contents
                            .to_lowercase()
                            .contains(&entity_name.to_lowercase())
                    {
                        Some("references_entity".to_string())
                    } else {
                        None
                    };

                    if let Some(relation) = relation {
                        docs.push(DocContext {
                            unique_id: doc_id.clone(),
                            name: doc_name.to_string(),
                            block_contents: block_contents.to_string(),
                            relation,
                        });
                    }
                }
            }
        }

        Ok(docs)
    }
}

fn build_grain_object(
    primary_key: Option<&[String]>,
    time_field: Option<&str>,
    dimensions: Option<&[String]>,
) -> Option<JsonValue> {
    let mut obj = serde_json::Map::new();
    if let Some(keys) = primary_key
        && !keys.is_empty()
    {
        obj.insert("primary_key".to_string(), serde_json::json!(keys));
    }
    if let Some(time_field) = time_field {
        obj.insert(
            "time_field".to_string(),
            JsonValue::String(time_field.to_string()),
        );
    }
    if let Some(dimensions) = dimensions
        && !dimensions.is_empty()
    {
        obj.insert("dimensions".to_string(), serde_json::json!(dimensions));
    }

    if obj.is_empty() {
        None
    } else {
        Some(JsonValue::Object(obj))
    }
}

fn build_grain_summary(
    nova: Option<&NovaMeta>,
    primary_key_columns: Option<&[String]>,
) -> Option<JsonValue> {
    let grain = nova.and_then(|n| n.grain.as_ref());
    let primary_key = grain
        .map(|g| g.primary_key.as_slice())
        .filter(|keys| !keys.is_empty())
        .or(primary_key_columns);
    let time_field = grain.and_then(|g| g.time_field.as_deref());
    let dimensions = grain.map(|g| g.dimensions.as_slice());

    build_grain_object(primary_key, time_field, dimensions)
}

#[allow(clippy::too_many_lines)]
fn build_nova_summary(nova: &NovaMeta) -> Option<JsonValue> {
    let mut obj = serde_json::Map::new();

    if let Some(role) = nova.role.as_ref() {
        obj.insert("role".to_string(), JsonValue::String(role.clone()));
    }
    if let Some(semantic_type) = nova.semantic_type.as_ref() {
        obj.insert(
            "semantic_type".to_string(),
            JsonValue::String(semantic_type.clone()),
        );
    }
    if !nova.synonyms.is_empty() {
        obj.insert("synonyms".to_string(), serde_json::json!(nova.synonyms));
    }
    if !nova.domains.is_empty() {
        obj.insert("domains".to_string(), serde_json::json!(nova.domains));
    }
    if !nova.use_cases.is_empty() {
        obj.insert("use_cases".to_string(), serde_json::json!(nova.use_cases));
    }
    if let Some(tier) = nova.tier.as_ref() {
        obj.insert("tier".to_string(), JsonValue::String(tier.clone()));
    }
    if nova.canonical {
        obj.insert("canonical".to_string(), JsonValue::from(true));
    }
    if let Some(search) = nova.search.as_ref()
        && let Some(candidates) = search.candidates.as_ref()
        && candidates.has_non_default_flags()
    {
        let mut candidate_obj = serde_json::Map::new();
        if !candidates.analyst {
            candidate_obj.insert("analyst".to_string(), JsonValue::from(false));
        }
        if !candidates.engineer {
            candidate_obj.insert("engineer".to_string(), JsonValue::from(false));
        }
        if !candidates.governance {
            candidate_obj.insert("governance".to_string(), JsonValue::from(false));
        }
        if !candidate_obj.is_empty() {
            obj.insert(
                "search_candidates".to_string(),
                JsonValue::Object(candidate_obj),
            );
        }
    }

    if let Some(grain) = nova.grain.as_ref()
        && let Some(grain_obj) = build_grain_object(
            Some(grain.primary_key.as_slice()),
            grain.time_field.as_deref(),
            Some(grain.dimensions.as_slice()),
        )
    {
        obj.insert("grain".to_string(), grain_obj);
    }

    if !nova.measures.is_empty() {
        let measures: Vec<JsonValue> = nova
            .measures
            .iter()
            .map(|measure| {
                let mut measure_obj = serde_json::Map::new();
                measure_obj.insert("name".to_string(), JsonValue::String(measure.name.clone()));
                if let Some(measure_type) = measure.measure_type.as_ref() {
                    measure_obj.insert("type".to_string(), JsonValue::String(measure_type.clone()));
                }
                if let Some(description) = measure.description.as_ref()
                    && !description.trim().is_empty()
                {
                    measure_obj.insert(
                        "description".to_string(),
                        JsonValue::String(description.clone()),
                    );
                }
                if let Some(field) = measure.field.as_ref() {
                    measure_obj.insert("field".to_string(), JsonValue::String(field.clone()));
                }
                if let Some(expression) = measure.expression.as_ref() {
                    measure_obj.insert(
                        "expression".to_string(),
                        JsonValue::String(expression.clone()),
                    );
                }
                if !measure.synonyms.is_empty() {
                    measure_obj.insert("synonyms".to_string(), serde_json::json!(measure.synonyms));
                }
                if nova.canonical || measure.canonical {
                    measure_obj.insert("canonical".to_string(), JsonValue::from(true));
                }
                JsonValue::Object(measure_obj)
            })
            .collect();
        if !measures.is_empty() {
            obj.insert("measures".to_string(), JsonValue::Array(measures));
        }
    }

    if nova.metric.is_some() || !nova.metrics.is_empty() {
        let mut metrics: Vec<JsonValue> = Vec::new();
        if let Some(metric) = nova.metric.as_ref() {
            metrics.push(build_metric_summary(metric, nova.canonical));
        }
        for metric in &nova.metrics {
            metrics.push(build_metric_summary(metric, nova.canonical));
        }
        if !metrics.is_empty() {
            obj.insert("metrics".to_string(), JsonValue::Array(metrics));
        }
    }

    if let Some(governance) = nova.governance.as_ref() {
        let mut gov_obj = serde_json::Map::new();
        if let Some(sensitivity) = governance.sensitivity.as_ref() {
            gov_obj.insert(
                "sensitivity".to_string(),
                JsonValue::String(sensitivity.clone()),
            );
        }
        if let Some(pii) = governance.pii.as_ref() {
            gov_obj.insert("pii".to_string(), JsonValue::String(pii.clone()));
        }
        if !governance.compliance.is_empty() {
            gov_obj.insert(
                "compliance".to_string(),
                serde_json::json!(governance.compliance),
            );
        }
        if !gov_obj.is_empty() {
            obj.insert("governance".to_string(), JsonValue::Object(gov_obj));
        }
    }

    if obj.is_empty() {
        None
    } else {
        Some(JsonValue::Object(obj))
    }
}

fn build_metric_summary(
    metric: &crate::manifest::entity::NovaMetric,
    entity_canonical: bool,
) -> JsonValue {
    let mut metric_obj = serde_json::Map::new();
    metric_obj.insert("name".to_string(), JsonValue::String(metric.name.clone()));
    if let Some(description) = metric.description.as_ref()
        && !description.trim().is_empty()
    {
        metric_obj.insert(
            "description".to_string(),
            JsonValue::String(description.clone()),
        );
    }
    if let Some(expression) = metric.expression.as_ref() {
        metric_obj.insert(
            "expression".to_string(),
            JsonValue::String(expression.clone()),
        );
    }
    if metric.template {
        metric_obj.insert("template".to_string(), JsonValue::from(true));
    }
    if entity_canonical || metric.canonical {
        metric_obj.insert("canonical".to_string(), JsonValue::from(true));
    }
    if let Some(grain) = metric.grain.as_ref()
        && let Some(grain_obj) = build_grain_object(
            Some(grain.primary_key.as_slice()),
            grain.time_field.as_deref(),
            Some(grain.dimensions.as_slice()),
        )
    {
        metric_obj.insert("grain".to_string(), grain_obj);
    }
    if !metric.synonyms.is_empty() {
        metric_obj.insert("synonyms".to_string(), serde_json::json!(metric.synonyms));
    }
    if !metric.recommended_filters.is_empty() {
        let filters: Vec<JsonValue> = metric
            .recommended_filters
            .iter()
            .map(|filter| {
                let mut filter_obj = serde_json::Map::new();
                filter_obj.insert("field".to_string(), JsonValue::String(filter.field.clone()));
                if let Some(operator) = filter.operator.as_ref() {
                    filter_obj.insert("operator".to_string(), JsonValue::String(operator.clone()));
                }
                if let Some(label) = filter.label.as_ref() {
                    filter_obj.insert("label".to_string(), JsonValue::String(label.clone()));
                }
                if !filter.values.is_empty() {
                    filter_obj.insert("values".to_string(), serde_json::json!(filter.values));
                }
                JsonValue::Object(filter_obj)
            })
            .collect();
        if !filters.is_empty() {
            metric_obj.insert("recommended_filters".to_string(), JsonValue::Array(filters));
        }
    }
    JsonValue::Object(metric_obj)
}
