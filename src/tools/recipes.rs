use std::collections::HashMap;
use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

use serde_json::Value as JsonValue;
use tracing::instrument;

use crate::error::{DbtNovaError, Result};
use crate::manifest::search::ManifestSearch;
use crate::params::{GetRecipeParams, RunRecipeParams, SearchRecipesParams};
use crate::responses::SuccessResponse;

mod jinja_scan;
mod placeholder;

use jinja_scan::{recipe_query_jinja_markers, recipe_query_snippet};
use placeholder::{
    apply_runtime_parameter_substitution, coerce_placeholder_value, contains_query_placeholders,
    normalize_recipe_query_key, parse_placeholder_at, resolve_placeholder_value_type,
};

#[derive(Debug, Clone)]
enum RecipeQuerySource {
    ManifestAnalysis { analysis_id: String },
}

impl RecipeQuerySource {
    fn label(&self) -> &'static str {
        match self {
            Self::ManifestAnalysis { .. } => "manifest_analysis",
        }
    }
}

#[derive(Debug, Clone)]
struct RecipeQuery {
    name: String,
    path: PathBuf,
    order: usize,
    source: RecipeQuerySource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecipeSqlSource {
    CompiledCode,
    RawCode,
}

impl RecipeSqlSource {
    fn label(self) -> &'static str {
        match self {
            Self::CompiledCode => "compiled_code",
            Self::RawCode => "raw_code",
        }
    }
}

#[derive(Debug, Clone)]
struct RecipeRecord {
    id: String,
    path: PathBuf,
    queries: Vec<RecipeQuery>,
}

#[derive(Debug, Clone)]
struct RecipeParameterSpec {
    key: String,
    name: String,
    required: bool,
    placeholder_type: Option<String>,
    default_value: Option<JsonValue>,
    description: Option<String>,
    source: &'static str,
}

impl RecipeParameterSpec {
    fn effective_required(&self) -> bool {
        self.required && self.default_value.is_none()
    }

    fn to_json_value(&self) -> JsonValue {
        let mut obj = serde_json::Map::new();
        obj.insert("name".to_string(), JsonValue::String(self.name.clone()));
        obj.insert(
            "required".to_string(),
            JsonValue::Bool(self.effective_required()),
        );
        obj.insert(
            "source".to_string(),
            JsonValue::String(self.source.to_string()),
        );
        if let Some(placeholder_type) = &self.placeholder_type {
            obj.insert(
                "placeholder_type".to_string(),
                JsonValue::String(placeholder_type.clone()),
            );
        }
        if let Some(description) = &self.description {
            obj.insert(
                "description".to_string(),
                JsonValue::String(description.clone()),
            );
        }
        if let Some(default_value) = &self.default_value {
            obj.insert("default".to_string(), default_value.clone());
        }
        JsonValue::Object(obj)
    }
}

#[derive(Debug, Clone)]
struct PreparedRecipeQuery {
    query: RecipeQuery,
    base_sql: String,
    parameter_specs: Vec<RecipeParameterSpec>,
    analysis_unique_id: String,
    sql_source: RecipeSqlSource,
}

#[derive(Debug)]
struct ResolvedRecipeSql {
    base_sql: String,
    payload: JsonValue,
    analysis_unique_id: String,
    source: RecipeSqlSource,
}

#[derive(Debug, Clone)]
struct RecipeParameterSchema {
    aggregated_specs: Vec<RecipeParameterSpec>,
    query_specs: Vec<(String, Vec<RecipeParameterSpec>)>,
    missing_parameters: Vec<String>,
    unused_parameters: Vec<String>,
    type_mismatches: Vec<JsonValue>,
    effective_parameters: HashMap<String, JsonValue>,
}

impl ManifestSearch {
    /// Search available recipe directories and return a paginated list.
    ///
    /// # Errors
    /// Returns an error if recipe metadata cannot be read.
    #[instrument(
        skip(self, params),
        fields(
            tool = "search_recipes",
            query_len = params.query.len(),
            topic = %params.topic,
            limit = params.pagination.limit,
            offset = params.pagination.offset
        )
    )]
    pub async fn search_recipes(&self, params: &SearchRecipesParams) -> Result<JsonValue> {
        let mut recipe_records = self.discover_recipes()?;
        apply_recipe_query_filter(&mut recipe_records, params);

        recipe_records.sort_by(|a, b| a.id.cmp(&b.id));

        let total = recipe_records.len();
        let offset = params.pagination.offset.min(total);
        let limit = self.page_limit(params.pagination.limit);
        let end = (offset + limit).min(total);

        let mut data = Vec::new();
        for recipe in recipe_records
            .iter()
            .skip(offset)
            .take(end.saturating_sub(offset))
        {
            let schema = self.recipe_parameter_schema(recipe, None, None)?;
            let mut item = serde_json::json!({
                "id": recipe.id,
                "topic": recipe.id,
                "path": display_path(&recipe.path),
                "query_count": recipe.queries.len(),
                "required_parameters": parameter_names_by_required(&schema.aggregated_specs, true),
                "optional_parameters": parameter_names_by_required(&schema.aggregated_specs, false),
                "parameter_defaults": parameter_defaults_map(&schema.aggregated_specs),
                "query_parameters": query_parameter_map(&schema.query_specs),
            });
            if params.include_queries
                && let Some(obj) = item.as_object_mut()
            {
                let query_names: Vec<JsonValue> = recipe
                    .queries
                    .iter()
                    .map(|query| JsonValue::String(query.name.clone()))
                    .collect();
                obj.insert("queries".to_string(), JsonValue::Array(query_names));
            }
            data.push(item);
        }

        let count = data.len();
        let truncated = total > count + offset;
        let mut response = SuccessResponse::new(data, count).with_total(total);
        if truncated {
            response = response.with_truncated(true);
        }
        Ok(serde_json::to_value(response)?)
    }

    /// Fetch recipe metadata and optional SQL.
    ///
    /// # Errors
    /// Returns an error if the recipe is not found or SQL files cannot be read.
    #[instrument(skip(self, params), fields(tool = "get_recipe", recipe_id = %params.recipe_id))]
    pub async fn get_recipe(&self, params: &GetRecipeParams) -> Result<JsonValue> {
        if params.recipe_id.trim().is_empty() {
            return Err(DbtNovaError::InvalidParams(
                "recipe_id cannot be empty".to_string(),
            ));
        }

        let recipe = self.find_recipe(&params.recipe_id)?;
        let placeholder_types = resolve_recipe_placeholder_types(
            params.placeholder_types.as_ref(),
            params.parameter_types.as_ref(),
            "get_recipe",
        )?;
        if recipe.queries.is_empty() {
            return Err(DbtNovaError::InvalidParams(format!(
                "Recipe '{}' contains no SQL queries",
                recipe.id
            )));
        }

        let schema = self.recipe_parameter_schema(
            &recipe,
            params.parameters.as_ref(),
            placeholder_types.as_ref(),
        )?;
        validate_recipe_renderable_for_get(&recipe.id, params, &schema)?;

        let query_names: Vec<String> = recipe
            .queries
            .iter()
            .map(|query| query.name.clone())
            .collect();
        let query_payloads = if params.include_queries {
            let prepared_queries = self.prepare_recipe_queries(&recipe.queries)?;
            Some(build_recipe_query_payloads(
                &recipe.id,
                &prepared_queries,
                &schema,
                params.include_sql,
                placeholder_types.as_ref(),
            )?)
        } else {
            None
        };
        let response = build_get_recipe_response(&recipe, &query_names, &schema, query_payloads);

        Ok(serde_json::to_value(SuccessResponse::new(response, 1))?)
    }

    /// Run one or more recipe queries in deterministic order.
    ///
    /// # Errors
    /// Returns an error if the recipe cannot be resolved, query execution fails, or query selection is invalid.
    #[allow(clippy::too_many_lines)]
    #[instrument(
        skip(self, params),
        fields(
            tool = "run_recipe",
            recipe_id = %params.recipe_id,
            query_count = params.query_names.len() + params.query_indexes.len()
        )
    )]
    pub async fn run_recipe(&self, params: &RunRecipeParams) -> Result<JsonValue> {
        if params.recipe_id.trim().is_empty() {
            return Err(DbtNovaError::InvalidParams(
                "recipe_id cannot be empty".to_string(),
            ));
        }

        let recipe = self.find_recipe(&params.recipe_id)?;
        let placeholder_types = resolve_recipe_placeholder_types(
            params.placeholder_types.as_ref(),
            params.parameter_types.as_ref(),
            "run_recipe",
        )?;
        let sql_parameter_types = resolve_recipe_sql_parameter_types(
            params.sql_parameter_types.as_ref(),
            params.parameter_types.as_ref(),
            "run_recipe",
        )?;
        let selected = select_recipe_queries(&recipe, params)?;
        if selected.is_empty() {
            return Err(DbtNovaError::InvalidParams(format!(
                "No query selectors matched recipe '{}'",
                recipe.id
            )));
        }

        let selected_queries: Vec<RecipeQuery> =
            selected.iter().map(|query| (*query).clone()).collect();
        let prepared_queries = self.prepare_recipe_queries(&selected_queries)?;
        let schema = self.recipe_parameter_schema(
            &RecipeRecord {
                id: recipe.id.clone(),
                path: recipe.path.clone(),
                queries: selected_queries.clone(),
            },
            params.parameters.as_ref(),
            placeholder_types.as_ref(),
        )?;
        if !schema.missing_parameters.is_empty() || !schema.type_mismatches.is_empty() {
            let details = serde_json::json!({
                "recipe_id": recipe.id,
                "required_parameters": parameter_names_by_required(&schema.aggregated_specs, true),
                "optional_parameters": parameter_names_by_required(&schema.aggregated_specs, false),
                "parameter_defaults": parameter_defaults_map(&schema.aggregated_specs),
                "query_parameters": query_parameter_map(&schema.query_specs),
                "missing_parameters": schema.missing_parameters.clone(),
                "unused_parameters": schema.unused_parameters.clone(),
                "type_mismatches": schema.type_mismatches.clone(),
                "by_query": query_validation_payload(&prepared_queries, &schema),
            });
            return Err(DbtNovaError::InvalidParamsDetailed {
                message: "Recipe parameter preflight validation failed".to_string(),
                details,
            });
        }

        let mut steps: Vec<JsonValue> = Vec::new();
        let mut executed = 0usize;
        let mut failed = 0usize;
        let selected_query_names: Vec<String> = prepared_queries
            .iter()
            .map(|prepared| prepared.query.name.clone())
            .collect();
        let mut rendered_statements: Vec<(&PreparedRecipeQuery, String)> =
            Vec::with_capacity(prepared_queries.len());
        for prepared in &prepared_queries {
            rendered_statements.push((
                prepared,
                render_recipe_query_sql(
                    &recipe.id,
                    prepared,
                    &schema.effective_parameters,
                    placeholder_types.as_ref(),
                )?,
            ));
        }

        for (prepared, statement) in rendered_statements {
            let query = &prepared.query;
            let exec_params = crate::params::ExecuteSqlParams {
                statement: statement.clone(),
                warehouse_id: None,
                preflight_only: false,
                preflight_catalog: None,
                preflight_schema: None,
                preflight_relation: None,
                row_limit: params.row_limit,
                byte_limit: params.byte_limit,
                wait_timeout_s: params.wait_timeout_s,
                poll_interval_ms: params.poll_interval_ms,
                max_poll_seconds: params.max_poll_seconds,
                parameters: params.parameters.clone(),
                parameter_types: sql_parameter_types.clone(),
                fetch_all_chunks: params.fetch_all_chunks,
                max_chunks: params.max_chunks,
            };

            executed = executed.saturating_add(1);
            let step = match self.execute_sql(&exec_params).await {
                Ok(result) => {
                    let mut step = serde_json::Map::new();
                    step.insert(
                        "query_name".to_string(),
                        JsonValue::String(query.name.clone()),
                    );
                    step.insert("order".to_string(), JsonValue::from(query.order));
                    step.insert("status".to_string(), JsonValue::String("ok".to_string()));
                    step.insert("result".to_string(), result);
                    if params.include_sql {
                        step.insert("sql".to_string(), JsonValue::String(statement.clone()));
                    }
                    JsonValue::Object(step)
                }
                Err(err) => {
                    failed = failed.saturating_add(1);
                    let mut step = serde_json::Map::new();
                    step.insert(
                        "query_name".to_string(),
                        JsonValue::String(query.name.clone()),
                    );
                    step.insert("order".to_string(), JsonValue::from(query.order));
                    step.insert("status".to_string(), JsonValue::String("error".to_string()));
                    step.insert("error".to_string(), JsonValue::String(err.to_string()));
                    if params.include_sql {
                        step.insert("sql".to_string(), JsonValue::String(statement.clone()));
                    }
                    if params.stop_on_failure {
                        return Err(err);
                    }
                    JsonValue::Object(step)
                }
            };
            steps.push(step);
        }

        let stopped_on_error = false;
        let mut response = serde_json::json!({
            "recipe_id": recipe.id,
            "executed_queries": executed,
            "failed_queries": failed,
            "stopped_on_error": stopped_on_error,
            "required_parameters": parameter_names_by_required(&schema.aggregated_specs, true),
            "optional_parameters": parameter_names_by_required(&schema.aggregated_specs, false),
            "parameter_defaults": parameter_defaults_map(&schema.aggregated_specs),
            "query_parameters": query_parameter_map(&schema.query_specs),
            "missing_parameters": schema.missing_parameters.clone(),
            "unused_parameters": schema.unused_parameters.clone(),
            "type_mismatches": schema.type_mismatches.clone(),
            "steps": JsonValue::Array(steps),
        });
        if let Some(obj) = response.as_object_mut() {
            obj.insert(
                "query_names".to_string(),
                JsonValue::Array(
                    selected_query_names
                        .iter()
                        .map(|query_name| JsonValue::String(query_name.clone()))
                        .collect(),
                ),
            );
        }
        Ok(serde_json::to_value(SuccessResponse::new(
            response, executed,
        ))?)
    }

    fn discover_recipes(&self) -> Result<Vec<RecipeRecord>> {
        let mut recipe_records = self.list_manifest_recipes()?;
        recipe_records.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(recipe_records)
    }

    fn find_recipe(&self, recipe_id: &str) -> Result<RecipeRecord> {
        let normalized = normalize_recipe_id(recipe_id);
        if normalized.is_empty() {
            return Err(DbtNovaError::InvalidParams(
                "recipe_id cannot be empty".to_string(),
            ));
        }
        let relative = Path::new(&normalized);
        if relative
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::RootDir))
        {
            return Err(DbtNovaError::InvalidParams(format!(
                "Invalid recipe_id '{recipe_id}': path traversal is not allowed"
            )));
        }
        let candidates = self.discover_recipes()?;
        for candidate in candidates {
            if recipe_id_matches(&candidate.id.to_lowercase(), &normalized) {
                return Ok(candidate);
            }
        }

        Err(DbtNovaError::InvalidParams(format!(
            "Recipe not found: {recipe_id}"
        )))
    }

    fn list_manifest_recipes(&self) -> Result<Vec<RecipeRecord>> {
        let Some(analysis_ids) = self.by_resource_type.get("analysis") else {
            return Ok(Vec::new());
        };

        let mut grouped: HashMap<String, Vec<RecipeQuery>> = HashMap::new();
        let prefix = manifest_recipe_prefix(self);

        for unique_id in analysis_ids {
            let Some(entity) = self.get_entity_archived(unique_id)? else {
                continue;
            };
            let Some(path) = entity.original_file_path_str() else {
                continue;
            };

            let normalized_path = normalize_recipe_path(path);
            let recipe_rel = if normalized_path.starts_with(&prefix) {
                normalized_path[prefix.len()..].trim_start_matches('/')
            } else {
                continue;
            };
            if recipe_rel.is_empty() {
                continue;
            }

            let query_name = Path::new(recipe_rel)
                .file_name()
                .and_then(|name| name.to_str())
                .map(str::to_string);
            let Some(query_name) = query_name else {
                continue;
            };
            let Some(query_name) = query_file_name(&query_name) else {
                continue;
            };
            let query_order = parse_query_order(&query_name);
            let Some(recipe_id) = Path::new(recipe_rel).parent() else {
                continue;
            };
            let Some(recipe_id) = recipe_id.to_str() else {
                continue;
            };
            let recipe_id = normalize_path_part(recipe_id);
            if recipe_id.is_empty() {
                continue;
            }

            grouped.entry(recipe_id).or_default().push(RecipeQuery {
                name: query_name,
                path: Path::new(&normalized_path).to_path_buf(),
                order: query_order,
                source: RecipeQuerySource::ManifestAnalysis {
                    analysis_id: unique_id.clone(),
                },
            });
        }

        let mut records = Vec::with_capacity(grouped.len());
        for (id, mut queries) in grouped {
            if queries.is_empty() {
                continue;
            }
            queries.sort_by(|a, b| (a.order, &a.name).cmp(&(b.order, &b.name)));
            records.push(RecipeRecord {
                id: id.clone(),
                path: Path::new(&prefix).join(&id),
                queries,
            });
        }

        records.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(records)
    }

    fn prepare_recipe_queries(&self, queries: &[RecipeQuery]) -> Result<Vec<PreparedRecipeQuery>> {
        let mut prepared_queries = Vec::with_capacity(queries.len());
        for query in queries {
            let resolved = self.load_query_base_sql_and_payload(query)?;
            let parameter_specs =
                build_query_parameter_specs(&resolved.base_sql, &resolved.payload);
            prepared_queries.push(PreparedRecipeQuery {
                query: query.clone(),
                base_sql: resolved.base_sql,
                parameter_specs,
                analysis_unique_id: resolved.analysis_unique_id,
                sql_source: resolved.source,
            });
        }
        Ok(prepared_queries)
    }

    fn recipe_parameter_schema(
        &self,
        recipe: &RecipeRecord,
        provided_parameters: Option<&HashMap<String, JsonValue>>,
        placeholder_types: Option<&HashMap<String, String>>,
    ) -> Result<RecipeParameterSchema> {
        let prepared_queries = self.prepare_recipe_queries(&recipe.queries)?;
        let aggregated_specs = aggregate_parameter_specs(
            prepared_queries
                .iter()
                .map(|prepared| prepared.parameter_specs.as_slice()),
        );

        let effective_parameters =
            build_effective_parameter_map(provided_parameters, &aggregated_specs);
        let used_keys: HashSet<String> = aggregated_specs
            .iter()
            .map(|spec| spec.key.clone())
            .collect();

        let missing_parameters = parameter_names_by_required_and_presence(
            &aggregated_specs,
            &effective_parameters,
            true,
        );

        let unused_parameters = provided_parameters
            .map(|parameters| {
                let mut unused = Vec::new();
                for key in parameters.keys() {
                    let normalized = normalize_recipe_query_key(key);
                    if !used_keys.contains(&normalized) {
                        unused.push(key.clone());
                    }
                }
                unused.sort();
                unused.dedup();
                unused
            })
            .unwrap_or_default();

        let mut type_mismatches = Vec::new();
        for prepared in &prepared_queries {
            for spec in &prepared.parameter_specs {
                let Some(expected_type) =
                    resolve_placeholder_type_for_spec(spec, placeholder_types)
                else {
                    continue;
                };
                let Some(value) = effective_parameters.get(&spec.key) else {
                    continue;
                };
                if let Err(err) = coerce_placeholder_value(value, &expected_type, false) {
                    type_mismatches.push(serde_json::json!({
                        "query_name": prepared.query.name,
                        "parameter": spec.name,
                        "placeholder_type": expected_type,
                        "error": err.to_string(),
                    }));
                }
            }
        }

        let query_specs = prepared_queries
            .into_iter()
            .map(|prepared| (prepared.query.name, prepared.parameter_specs))
            .collect();

        Ok(RecipeParameterSchema {
            aggregated_specs,
            query_specs,
            missing_parameters,
            unused_parameters,
            type_mismatches,
            effective_parameters,
        })
    }

    fn load_query_base_sql_and_payload(&self, query: &RecipeQuery) -> Result<ResolvedRecipeSql> {
        let RecipeQuerySource::ManifestAnalysis { analysis_id } = &query.source;
        let analysis_unique_id = self
            .resolve_single_id(analysis_id, Some("analysis"))
            .map_err(|err| {
                DbtNovaError::InvalidParams(format!(
                    "Failed to resolve analysis reference '{analysis_id}' in query '{}': {err}",
                    query.name
                ))
            })?;
        let entity = self
            .get_entity_archived(&analysis_unique_id)?
            .ok_or_else(|| self.entity_not_found(&analysis_unique_id, Some("analysis")))?;
        let payload = entity.to_json_value();
        let compiled_code = payload
            .get("compiled_code")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let raw_code = payload
            .get("raw_code")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty());

        let (base_sql, source) = if let Some(compiled) = compiled_code {
            (compiled.to_string(), RecipeSqlSource::CompiledCode)
        } else if let Some(raw) = raw_code {
            (raw.to_string(), RecipeSqlSource::RawCode)
        } else {
            return Err(DbtNovaError::InvalidParams(format!(
                "Analysis '{analysis_unique_id}' does not expose compiled_code or raw_code"
            )));
        };

        Ok(ResolvedRecipeSql {
            base_sql,
            payload,
            analysis_unique_id,
            source,
        })
    }
}

fn recipe_parameter_contract_details(recipe_id: &str, schema: &RecipeParameterSchema) -> JsonValue {
    serde_json::json!({
        "recipe_id": recipe_id,
        "required_parameters": parameter_names_by_required(&schema.aggregated_specs, true),
        "optional_parameters": parameter_names_by_required(&schema.aggregated_specs, false),
        "parameter_defaults": parameter_defaults_map(&schema.aggregated_specs),
        "query_parameters": query_parameter_map(&schema.query_specs),
        "missing_parameters": schema.missing_parameters.clone(),
        "type_mismatches": schema.type_mismatches.clone(),
        "unused_parameters": schema.unused_parameters.clone(),
    })
}

fn validate_recipe_renderable_for_get(
    recipe_id: &str,
    params: &GetRecipeParams,
    schema: &RecipeParameterSchema,
) -> Result<()> {
    if params.include_sql
        && params.include_queries
        && (!schema.missing_parameters.is_empty() || !schema.type_mismatches.is_empty())
    {
        return Err(DbtNovaError::InvalidParamsDetailed {
            message: "Recipe parameters are incomplete or invalid for SQL rendering".to_string(),
            details: recipe_parameter_contract_details(recipe_id, schema),
        });
    }
    Ok(())
}

fn build_recipe_query_payloads(
    recipe_id: &str,
    prepared_queries: &[PreparedRecipeQuery],
    schema: &RecipeParameterSchema,
    include_sql: bool,
    placeholder_types: Option<&HashMap<String, String>>,
) -> Result<Vec<JsonValue>> {
    let mut query_payloads = Vec::with_capacity(prepared_queries.len());
    for prepared in prepared_queries {
        let query = &prepared.query;
        let mut obj = serde_json::Map::new();
        obj.insert("name".to_string(), JsonValue::String(query.name.clone()));
        obj.insert(
            "path".to_string(),
            JsonValue::String(display_path(&query.path)),
        );
        obj.insert("order".to_string(), JsonValue::from(query.order));
        obj.insert(
            "source".to_string(),
            JsonValue::String(query.source.label().to_string()),
        );
        obj.insert(
            "analysis_id".to_string(),
            JsonValue::String(prepared.analysis_unique_id.clone()),
        );
        obj.insert(
            "parameters".to_string(),
            JsonValue::Array(
                prepared
                    .parameter_specs
                    .iter()
                    .map(RecipeParameterSpec::to_json_value)
                    .collect(),
            ),
        );
        obj.insert(
            "required_parameters".to_string(),
            JsonValue::Array(
                prepared
                    .parameter_specs
                    .iter()
                    .filter(|spec| spec.effective_required())
                    .map(|spec| JsonValue::String(spec.name.clone()))
                    .collect(),
            ),
        );
        obj.insert(
            "optional_parameters".to_string(),
            JsonValue::Array(
                prepared
                    .parameter_specs
                    .iter()
                    .filter(|spec| !spec.effective_required())
                    .map(|spec| JsonValue::String(spec.name.clone()))
                    .collect(),
            ),
        );

        let query_missing_parameters: Vec<JsonValue> = prepared
            .parameter_specs
            .iter()
            .filter(|spec| {
                spec.effective_required() && !schema.effective_parameters.contains_key(&spec.key)
            })
            .map(|spec| JsonValue::String(spec.name.clone()))
            .collect();
        obj.insert(
            "missing_parameters".to_string(),
            JsonValue::Array(query_missing_parameters),
        );
        if include_sql {
            obj.insert(
                "sql".to_string(),
                JsonValue::String(render_recipe_query_sql(
                    recipe_id,
                    prepared,
                    &schema.effective_parameters,
                    placeholder_types,
                )?),
            );
        }
        query_payloads.push(JsonValue::Object(obj));
    }
    Ok(query_payloads)
}

fn build_get_recipe_response(
    recipe: &RecipeRecord,
    query_names: &[String],
    schema: &RecipeParameterSchema,
    query_payloads: Option<Vec<JsonValue>>,
) -> JsonValue {
    let mut response = serde_json::json!({
        "id": recipe.id,
        "topic": recipe.id,
        "path": display_path(&recipe.path),
        "query_count": recipe.queries.len(),
        "query_names": query_names,
        "required_parameters": parameter_names_by_required(&schema.aggregated_specs, true),
        "optional_parameters": parameter_names_by_required(&schema.aggregated_specs, false),
        "parameter_defaults": parameter_defaults_map(&schema.aggregated_specs),
        "query_parameters": query_parameter_map(&schema.query_specs),
        "missing_parameters": schema.missing_parameters.clone(),
        "unused_parameters": schema.unused_parameters.clone(),
        "type_mismatches": schema.type_mismatches.clone(),
    });
    if let Some(query_payloads) = query_payloads
        && let Some(obj) = response.as_object_mut()
    {
        obj.insert("queries".to_string(), JsonValue::Array(query_payloads));
    }
    response
}

fn apply_recipe_query_filter(records: &mut Vec<RecipeRecord>, params: &SearchRecipesParams) {
    if !params.topic.trim().is_empty() {
        let topic = params.topic.to_lowercase();
        records.retain(|recipe| recipe.id.to_lowercase().contains(&topic));
    }

    if params.query.trim().is_empty() {
        return;
    }

    let query = params.query.to_lowercase();
    records.retain(|recipe| {
        recipe.id.to_lowercase().contains(&query)
            || recipe
                .queries
                .iter()
                .any(|q| q.name.to_lowercase().contains(&query))
    });
}

fn recipe_contract_roots(payload: &JsonValue) -> Vec<&JsonValue> {
    let mut roots = Vec::new();
    if let Some(root) = payload.pointer("/meta/nova/recipe") {
        roots.push(root);
    }
    if let Some(root) = payload.pointer("/config/meta/nova/recipe") {
        roots.push(root);
    }
    roots
}

fn parse_metadata_parameter_object(
    name_hint: Option<&str>,
    value: &JsonValue,
) -> Option<RecipeParameterSpec> {
    let (name, required, placeholder_type, default_value, description) = match value {
        JsonValue::Object(obj) => {
            let name = obj
                .get("name")
                .and_then(JsonValue::as_str)
                .or(name_hint)
                .map(str::trim)
                .filter(|name| !name.is_empty())?;
            let default_value = obj.get("default").cloned();
            let required = obj
                .get("required")
                .and_then(JsonValue::as_bool)
                .unwrap_or(default_value.is_none());
            let placeholder_type = obj
                .get("type")
                .and_then(JsonValue::as_str)
                .or_else(|| obj.get("placeholder_type").and_then(JsonValue::as_str))
                .or_else(|| obj.get("parameter_type").and_then(JsonValue::as_str))
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_ascii_lowercase);
            let description = obj
                .get("description")
                .and_then(JsonValue::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string);
            (
                name.to_string(),
                required,
                placeholder_type,
                default_value,
                description,
            )
        }
        JsonValue::String(value) => {
            let name = name_hint?.trim().to_string();
            if name.is_empty() {
                return None;
            }
            (
                name,
                true,
                Some(value.trim().to_ascii_lowercase()),
                None,
                None,
            )
        }
        _ => return None,
    };

    let key = normalize_recipe_query_key(&name);
    if key.is_empty() {
        return None;
    }

    Some(RecipeParameterSpec {
        key,
        name,
        required,
        placeholder_type,
        default_value,
        description,
        source: "metadata",
    })
}

fn parse_metadata_parameter_list(entry: &JsonValue) -> Option<RecipeParameterSpec> {
    let JsonValue::Object(obj) = entry else {
        return None;
    };
    let name = obj
        .get("name")
        .or_else(|| obj.get("parameter"))
        .or_else(|| obj.get("id"))
        .and_then(JsonValue::as_str)?;
    parse_metadata_parameter_object(Some(name), entry)
}

fn metadata_parameter_specs(payload: &JsonValue) -> HashMap<String, RecipeParameterSpec> {
    let mut specs = HashMap::new();
    for root in recipe_contract_roots(payload) {
        let Some(parameters) = root.get("parameters") else {
            continue;
        };
        match parameters {
            JsonValue::Object(map) => {
                for (name, value) in map {
                    if let Some(spec) = parse_metadata_parameter_object(Some(name), value) {
                        specs.insert(spec.key.clone(), spec);
                    }
                }
            }
            JsonValue::Array(entries) => {
                for entry in entries {
                    if let Some(spec) = parse_metadata_parameter_list(entry) {
                        specs.insert(spec.key.clone(), spec);
                    }
                }
            }
            _ => {}
        }
    }
    specs
}

fn extract_query_placeholders(sql: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut seen = HashSet::new();
    let bytes = sql.as_bytes();
    let mut i = 0usize;

    while i < bytes.len() {
        if let Some((_, end, name)) = parse_placeholder_at(sql, i) {
            let key = normalize_recipe_query_key(&name);
            if seen.insert(key) {
                names.push(name);
            }
            i = end;
            continue;
        }
        let Some(next_char) = sql[i..].chars().next() else {
            break;
        };
        i += next_char.len_utf8();
    }

    names
}

fn build_query_parameter_specs(sql: &str, payload: &JsonValue) -> Vec<RecipeParameterSpec> {
    let mut specs = metadata_parameter_specs(payload);
    for token in extract_query_placeholders(sql) {
        let key = normalize_recipe_query_key(&token);
        if key.is_empty() {
            continue;
        }
        specs
            .entry(key.clone())
            .or_insert_with(|| RecipeParameterSpec {
                key,
                name: token,
                required: true,
                placeholder_type: None,
                default_value: None,
                description: None,
                source: "placeholder",
            });
    }
    let mut values: Vec<RecipeParameterSpec> = specs.into_values().collect();
    values.sort_by(|left, right| left.name.cmp(&right.name));
    values
}

fn merge_parameter_specs(existing: &mut RecipeParameterSpec, incoming: &RecipeParameterSpec) {
    existing.required = existing.required || incoming.required;
    if existing.placeholder_type.is_none() {
        existing
            .placeholder_type
            .clone_from(&incoming.placeholder_type);
    }
    if existing.default_value.is_none() {
        existing.default_value.clone_from(&incoming.default_value);
    }
    if existing.description.is_none() {
        existing.description.clone_from(&incoming.description);
    }
    if existing.source == "placeholder" && incoming.source == "metadata" {
        existing.source = "metadata";
    }
    if existing.name != incoming.name
        && existing.name.to_ascii_lowercase() == existing.name
        && incoming.name.to_ascii_uppercase() == incoming.name
    {
        existing.name.clone_from(&incoming.name);
    }
}

fn aggregate_parameter_specs<'a>(
    specs: impl Iterator<Item = &'a [RecipeParameterSpec]>,
) -> Vec<RecipeParameterSpec> {
    let mut merged: HashMap<String, RecipeParameterSpec> = HashMap::new();
    for query_specs in specs {
        for spec in query_specs {
            if let Some(existing) = merged.get_mut(&spec.key) {
                merge_parameter_specs(existing, spec);
            } else {
                merged.insert(spec.key.clone(), spec.clone());
            }
        }
    }
    let mut values: Vec<RecipeParameterSpec> = merged.into_values().collect();
    values.sort_by(|left, right| left.name.cmp(&right.name));
    values
}

fn build_effective_parameter_map(
    parameters: Option<&HashMap<String, JsonValue>>,
    specs: &[RecipeParameterSpec],
) -> HashMap<String, JsonValue> {
    let mut effective = HashMap::new();
    if let Some(parameters) = parameters {
        for (key, value) in parameters {
            effective.insert(normalize_recipe_query_key(key), value.clone());
        }
    }
    for spec in specs {
        if !effective.contains_key(&spec.key)
            && let Some(default_value) = &spec.default_value
        {
            effective.insert(spec.key.clone(), default_value.clone());
        }
    }
    effective
}

fn parameter_names_by_required(specs: &[RecipeParameterSpec], required: bool) -> Vec<JsonValue> {
    specs
        .iter()
        .filter(|spec| spec.effective_required() == required)
        .map(|spec| JsonValue::String(spec.name.clone()))
        .collect()
}

fn parameter_names_by_required_and_presence(
    specs: &[RecipeParameterSpec],
    effective_parameters: &HashMap<String, JsonValue>,
    required: bool,
) -> Vec<String> {
    let mut names: Vec<String> = specs
        .iter()
        .filter(|spec| {
            spec.effective_required() == required && !effective_parameters.contains_key(&spec.key)
        })
        .map(|spec| spec.name.clone())
        .collect();
    names.sort();
    names
}

fn parameter_defaults_map(specs: &[RecipeParameterSpec]) -> JsonValue {
    let mut map = serde_json::Map::new();
    for spec in specs {
        if let Some(default_value) = &spec.default_value {
            map.insert(spec.name.clone(), default_value.clone());
        }
    }
    JsonValue::Object(map)
}

fn query_parameter_map(query_specs: &[(String, Vec<RecipeParameterSpec>)]) -> JsonValue {
    let mut map = serde_json::Map::new();
    for (query_name, specs) in query_specs {
        map.insert(
            query_name.clone(),
            JsonValue::Array(
                specs
                    .iter()
                    .map(RecipeParameterSpec::to_json_value)
                    .collect(),
            ),
        );
    }
    JsonValue::Object(map)
}

fn resolve_placeholder_type_for_spec(
    spec: &RecipeParameterSpec,
    placeholder_types: Option<&HashMap<String, String>>,
) -> Option<String> {
    if let Some(runtime_override) = placeholder_types.and_then(|types| {
        resolve_placeholder_value_type(&spec.name, Some(types))
            .or_else(|| resolve_placeholder_value_type(&spec.key, Some(types)))
    }) {
        return Some(runtime_override.to_ascii_lowercase());
    }
    spec.placeholder_type.clone()
}

fn merge_optional_type_maps(
    primary: Option<&HashMap<String, String>>,
    fallback: Option<&HashMap<String, String>>,
    context: &str,
    primary_name: &str,
    fallback_name: &str,
    normalize_keys: bool,
) -> Result<Option<HashMap<String, String>>> {
    let mut merged = HashMap::new();
    if let Some(fallback) = fallback {
        for (key, value) in fallback {
            let merged_key = if normalize_keys {
                normalize_recipe_query_key(key)
            } else {
                key.clone()
            };
            if merged_key.is_empty() {
                continue;
            }
            merged.insert(merged_key, value.clone());
        }
    }
    if let Some(primary) = primary {
        for (key, value) in primary {
            let merged_key = if normalize_keys {
                normalize_recipe_query_key(key)
            } else {
                key.clone()
            };
            if merged_key.is_empty() {
                continue;
            }
            if let Some(existing) = merged.get(&merged_key)
                && existing != value
            {
                return Err(DbtNovaError::InvalidParams(format!(
                    "{context}: conflicting type hints for '{key}' between {primary_name} and {fallback_name}"
                )));
            }
            merged.insert(merged_key, value.clone());
        }
    }
    if merged.is_empty() {
        Ok(None)
    } else {
        Ok(Some(merged))
    }
}

fn resolve_recipe_placeholder_types(
    placeholder_types: Option<&HashMap<String, String>>,
    legacy_parameter_types: Option<&HashMap<String, String>>,
    context: &str,
) -> Result<Option<HashMap<String, String>>> {
    merge_optional_type_maps(
        placeholder_types,
        legacy_parameter_types,
        context,
        "placeholder_types",
        "parameter_types",
        true,
    )
}

fn resolve_recipe_sql_parameter_types(
    sql_parameter_types: Option<&HashMap<String, String>>,
    legacy_parameter_types: Option<&HashMap<String, String>>,
    context: &str,
) -> Result<Option<HashMap<String, String>>> {
    merge_optional_type_maps(
        sql_parameter_types,
        legacy_parameter_types,
        context,
        "sql_parameter_types",
        "parameter_types",
        false,
    )
}

fn query_validation_payload(
    prepared_queries: &[PreparedRecipeQuery],
    schema: &RecipeParameterSchema,
) -> JsonValue {
    let mut items = Vec::with_capacity(prepared_queries.len());
    for prepared in prepared_queries {
        let missing: Vec<JsonValue> = prepared
            .parameter_specs
            .iter()
            .filter(|spec| {
                spec.effective_required() && !schema.effective_parameters.contains_key(&spec.key)
            })
            .map(|spec| JsonValue::String(spec.name.clone()))
            .collect();
        let mut obj = serde_json::Map::new();
        obj.insert(
            "query_name".to_string(),
            JsonValue::String(prepared.query.name.clone()),
        );
        obj.insert(
            "required_parameters".to_string(),
            JsonValue::Array(
                prepared
                    .parameter_specs
                    .iter()
                    .filter(|spec| spec.effective_required())
                    .map(|spec| JsonValue::String(spec.name.clone()))
                    .collect(),
            ),
        );
        obj.insert(
            "optional_parameters".to_string(),
            JsonValue::Array(
                prepared
                    .parameter_specs
                    .iter()
                    .filter(|spec| !spec.effective_required())
                    .map(|spec| JsonValue::String(spec.name.clone()))
                    .collect(),
            ),
        );
        obj.insert("missing_parameters".to_string(), JsonValue::Array(missing));
        obj.insert(
            "parameters".to_string(),
            JsonValue::Array(
                prepared
                    .parameter_specs
                    .iter()
                    .map(RecipeParameterSpec::to_json_value)
                    .collect(),
            ),
        );
        items.push(JsonValue::Object(obj));
    }
    JsonValue::Array(items)
}

fn display_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn parse_query_order(name: &str) -> usize {
    let no_ext = name.strip_suffix(".sql").unwrap_or(name);

    let mut suffixes = no_ext.rsplitn(2, "__");
    if let (Some(raw_order), Some(_)) = (suffixes.next(), suffixes.next())
        && let Ok(order) = raw_order.parse::<usize>()
    {
        return order;
    }

    let mut parts = no_ext.rsplitn(2, '_');
    if let (Some(raw_order), Some(_)) = (parts.next(), parts.next())
        && let Ok(order) = raw_order.parse::<usize>()
    {
        return order;
    }

    usize::MAX
}

fn query_file_name(entry_name: &str) -> Option<String> {
    if entry_name.to_lowercase().ends_with(".sql") {
        Some(entry_name.to_string())
    } else {
        None
    }
}

fn recipe_id_matches(candidate_id: &str, search_id: &str) -> bool {
    if search_id.is_empty() {
        return false;
    }
    candidate_id == search_id || candidate_id.ends_with(&format!("/{search_id}"))
}

fn manifest_recipe_prefix(searcher: &ManifestSearch) -> String {
    let configured = searcher.config.recipes_dir.trim();
    let fallback = "analyses/recipes";
    let normalized = normalize_recipe_path(if configured.is_empty() {
        fallback
    } else {
        configured
    });
    let mut prefix = normalized;
    if !prefix.ends_with('/') {
        prefix.push('/');
    }
    prefix
}

fn normalize_recipe_path(value: &str) -> String {
    value
        .replace('\\', "/")
        .trim_start_matches("./")
        .trim_start_matches('/')
        .trim_end_matches('/')
        .to_string()
}

fn normalize_path_part(value: &str) -> String {
    value
        .replace('\\', "/")
        .trim_start_matches("./")
        .trim_start_matches('/')
        .trim_end_matches('/')
        .to_string()
}

fn normalize_recipe_id(recipe_id: &str) -> String {
    recipe_id
        .trim()
        .trim_start_matches("./")
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_lowercase()
}

fn query_selector_key(name: &str) -> String {
    name.trim()
        .replace('\\', "/")
        .trim_start_matches("./")
        .trim_end_matches('/')
        .strip_suffix(".sql")
        .unwrap_or(name.trim())
        .to_lowercase()
}

fn non_executable_recipe_query_error(
    recipe_id: &str,
    prepared: &PreparedRecipeQuery,
) -> DbtNovaError {
    let message = format!(
        "Recipe query '{}' cannot execute: compiled_code is unavailable and raw_code contains dbt/Jinja templating (`{{{{` / `{{%` / `{{#`). Rebuild manifest with compiled analysis SQL.",
        prepared.query.name
    );
    let details = serde_json::json!({
        "recipe_id": recipe_id,
        "query_name": prepared.query.name,
        "analysis_id": prepared.analysis_unique_id,
        "path": display_path(&prepared.query.path),
        "sql_source": prepared.sql_source.label(),
        "jinja_markers": recipe_query_jinja_markers(&prepared.base_sql),
        "raw_snippet": recipe_query_snippet(&prepared.base_sql, 220),
        "action": "Provide compiled analysis SQL in the manifest, or remove dbt/Jinja templating from raw_code fallback.",
    });
    DbtNovaError::InvalidParamsDetailed { message, details }
}

fn ensure_recipe_query_executable(recipe_id: &str, prepared: &PreparedRecipeQuery) -> Result<()> {
    if prepared.sql_source == RecipeSqlSource::RawCode
        && !recipe_query_jinja_markers(&prepared.base_sql).is_empty()
    {
        return Err(non_executable_recipe_query_error(recipe_id, prepared));
    }
    Ok(())
}

fn render_recipe_query_sql(
    recipe_id: &str,
    prepared: &PreparedRecipeQuery,
    parameters: &HashMap<String, JsonValue>,
    parameter_types: Option<&HashMap<String, String>>,
) -> Result<String> {
    ensure_recipe_query_executable(recipe_id, prepared)?;

    if parameters.is_empty() && contains_query_placeholders(&prepared.base_sql) {
        return Err(DbtNovaError::InvalidParams(format!(
            "Recipe query '{}' requires runtime parameters",
            prepared.query.name
        )));
    }

    if parameter_types.is_some() || !parameters.is_empty() {
        apply_runtime_parameter_substitution(&prepared.base_sql, parameters, parameter_types)
    } else {
        Ok(prepared.base_sql.clone())
    }
}

fn select_recipe_queries<'a>(
    recipe: &'a RecipeRecord,
    params: &RunRecipeParams,
) -> Result<Vec<&'a RecipeQuery>> {
    let mut selected: Vec<&'a RecipeQuery> = Vec::new();
    let mut ordered_queries: Vec<&'a RecipeQuery> = recipe.queries.iter().collect();
    ordered_queries.sort_by(|a, b| (a.order, &a.name).cmp(&(b.order, &b.name)));

    if !params.query_indexes.is_empty() {
        for index in &params.query_indexes {
            if index == &0 || *index > recipe.queries.len() {
                return Err(DbtNovaError::InvalidParams(format!(
                    "Query index {} is out of range for recipe {}",
                    index, recipe.id
                )));
            }
            selected.push(ordered_queries[index - 1]);
        }
    }

    if !params.query_names.is_empty() {
        for request in &params.query_names {
            let query_key = query_selector_key(request);
            let matched = recipe
                .queries
                .iter()
                .find(|query| query_selector_key(&query.name) == query_key)
                .or_else(|| {
                    recipe.queries.iter().find(|query| {
                        query.name == *request || query_selector_key(&query.name) == query_key
                    })
                });
            if let Some(query) = matched {
                selected.push(query);
            } else {
                return Err(DbtNovaError::InvalidParams(format!(
                    "Query '{}' not found in recipe {}",
                    request, recipe.id
                )));
            }
        }
    }

    if selected.is_empty() {
        selected.extend(ordered_queries);
    }

    let mut unique = Vec::new();
    let mut seen = HashSet::new();
    for query in selected {
        if seen.insert(query.name.clone()) {
            unique.push(query);
        }
    }
    unique.sort_by(|a, b| (a.order, &a.name).cmp(&(b.order, &b.name)));
    Ok(unique)
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let err = apply_runtime_parameter_substitution(
            "select * from __TARGET_TABLE__",
            &parameters,
            None,
        )
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
        let markers = recipe_query_jinja_markers(
            "select E'It\\'s {{ok}} and {% raw %} and {# note #}' as msg",
        );
        assert!(markers.is_empty());
    }

    #[test]
    fn test_recipe_query_jinja_markers_detects_after_standard_sql_backslash_literal() {
        let markers = recipe_query_jinja_markers("select 'C:\\' as path; {{ ref('model') }}");
        assert_eq!(markers, vec!["{{"]);
    }

    #[test]
    fn test_recipe_query_jinja_markers_ignores_dollar_quoted_literals() {
        let markers =
            recipe_query_jinja_markers("select $$ {{ok}} {% block %} {# note #} $$ as body");
        assert!(markers.is_empty());
    }

    #[test]
    fn test_recipe_query_jinja_markers_ignores_tagged_dollar_quoted_literals() {
        let markers =
            recipe_query_jinja_markers("select $tag$ {{ok}} {% block %} {# note #} $tag$ as body");
        assert!(markers.is_empty());
    }
}
