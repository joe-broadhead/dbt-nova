use serde_json::Value as JsonValue;

use crate::error::{DbtNovaError, Result};
use crate::manifest::search::ManifestSearch;
use crate::params::{BatchGetParams, DetailLevel, GetColumnsParams, GetEntityParams, GetSqlParams};
use crate::responses::SuccessResponse;
use crate::tools::helpers::primary_key_columns_from_columns;
use rayon::prelude::*;
use tracing::instrument;

impl ManifestSearch {
    /// Get complete entity data by `unique_id` or name. Returns ALL fields from the manifest.
    ///
    /// # Errors
    /// Returns an error if the entity cannot be resolved, is ambiguous, or the manifest is unavailable.
    #[instrument(skip(self, params), fields(tool = "get_entity", id_or_name = %params.id_or_name, resource_type = ?params.resource_type))]
    pub async fn get_entity_data(&self, params: &GetEntityParams) -> Result<JsonValue> {
        let keys = self.resolve_id_or_name(&params.id_or_name, params.resource_type.as_deref());
        let detail = params.detail;

        if keys.is_empty() {
            return Err(self.entity_not_found(&params.id_or_name, params.resource_type.as_deref()));
        }

        if keys.len() > 1 {
            let mut matches: Vec<JsonValue> = Vec::new();
            for k in &keys {
                if let Some(entity) = self.get_entity_archived(k)? {
                    matches.push(self.summary_for_standard(k, entity));
                }
            }
            return Ok(serde_json::to_value(SuccessResponse::new(
                matches.clone(),
                matches.len(),
            ))?);
        }

        let entity = self.get_entity_archived(&keys[0])?.map_or_else(
            || JsonValue::Null,
            |entity| {
                if detail == DetailLevel::Full {
                    entity.to_json_value()
                } else {
                    self.summary_for_standard(&keys[0], entity)
                }
            },
        );
        Ok(serde_json::to_value(SuccessResponse::new(entity, 1))?)
    }

    /// Get raw or compiled SQL for a model.
    ///
    /// # Errors
    /// Returns an error if the model cannot be resolved or has no SQL available.
    #[instrument(skip(self, params), fields(tool = "get_sql", id_or_name = %params.id_or_name, compiled = params.compiled))]
    pub async fn get_sql(&self, params: &GetSqlParams) -> Result<JsonValue> {
        let unique_id = self.resolve_single_id(&params.id_or_name, Some("model"))?;
        let entity = self
            .get_entity_archived(&unique_id)?
            .ok_or_else(|| self.entity_not_found(&unique_id, Some("model")))?;
        let entity_json = entity.to_json_value();
        let sql_field = if params.compiled {
            "compiled_code"
        } else {
            "raw_code"
        };

        let sql = entity_json.get(sql_field).and_then(|s| s.as_str());

        match sql {
            Some(code) => {
                let contains_templating = sql_contains_templating(code);
                let mut response = serde_json::json!({
                    "unique_id": &unique_id,
                    "name": entity_json.get("name"),
                    "sql_type": if params.compiled { "compiled" } else { "raw" },
                    "sql": code,
                    "contains_templating": contains_templating
                });
                if params.compiled
                    && let Some(obj) = response.as_object_mut()
                {
                    obj.insert(
                        "compiled_available".to_string(),
                        serde_json::json!(!contains_templating),
                    );
                }
                Ok(serde_json::to_value(SuccessResponse::new(response, 1))?)
            }
            None => Err(DbtNovaError::InvalidParams(format!(
                "Entity '{unique_id}' does not include {sql_field}; try compiled=false for raw SQL or check whether the model was compiled in the manifest"
            ))),
        }
    }

    /// Get column information for a model or source.
    ///
    /// # Errors
    /// Returns an error if the entity cannot be resolved or the manifest is unavailable.
    #[instrument(skip(self, params), fields(tool = "get_columns", id_or_name = %params.id_or_name))]
    pub async fn get_columns(&self, params: &GetColumnsParams) -> Result<JsonValue> {
        let unique_id = self.resolve_single_id(&params.id_or_name, None)?;
        let entity = self
            .get_entity_archived(&unique_id)?
            .ok_or_else(|| self.entity_not_found(&unique_id, None))?;
        let entity_json = entity.to_json_value();
        let columns = entity_json
            .get("columns")
            .cloned()
            .unwrap_or(JsonValue::Null);

        let column_list: Vec<JsonValue> = if let Some(obj) = columns.as_object() {
            let mut ordered: Vec<(&String, &JsonValue)> = obj.iter().collect();
            ordered.sort_by(|(left_name, _), (right_name, _)| left_name.cmp(right_name));
            ordered
                .into_iter()
                .map(|(_, value)| value.clone())
                .collect()
        } else {
            vec![]
        };
        let primary_key_columns = primary_key_columns_from_columns(&columns);

        let mut response = serde_json::json!({
            "unique_id": &unique_id,
            "name": entity_json.get("name"),
            "columns": column_list
        });
        if !primary_key_columns.is_empty()
            && let Some(obj) = response.as_object_mut()
        {
            obj.insert(
                "primary_key_columns".to_string(),
                serde_json::json!(primary_key_columns),
            );
        }

        Ok(serde_json::to_value(SuccessResponse::new(
            response,
            column_list.len(),
        ))?)
    }

    /// Get multiple entities in one call.
    ///
    /// # Errors
    /// Returns an error if the request exceeds configured limits or the manifest is unavailable.
    #[instrument(skip(self, params), fields(tool = "batch_get_entities", count = params.unique_ids.len(), detail = ?params.detail))]
    pub async fn batch_get_entities(&self, params: &BatchGetParams) -> Result<JsonValue> {
        let max_items = self.config.batch_get_max_items;
        if max_items > 0 && params.unique_ids.len() > max_items {
            return Err(DbtNovaError::InvalidParams(format!(
                "batch_get_entities supports up to {} ids per call (received {})",
                max_items,
                params.unique_ids.len()
            )));
        }
        let ids: Vec<String> = params.unique_ids.clone();

        let detail = params.detail;
        let store = self.entities.clone();
        let results: Vec<(String, Option<JsonValue>)> = tokio::task::spawn_blocking(move || {
            ids.par_iter()
                .map(|id| {
                    let payload = match store.get_archived(id)? {
                        Some(entity) => {
                            let json = serde_json::from_str(entity.payload_json())
                                .unwrap_or(JsonValue::Null);
                            Some(json)
                        }
                        None => None,
                    };
                    Ok((id.clone(), payload))
                })
                .collect::<Result<Vec<_>>>()
        })
        .await
        .map_err(|err| DbtNovaError::ServerError(err.to_string()))??;

        let mut found: Vec<JsonValue> = Vec::new();
        let mut not_found: Vec<String> = Vec::new();

        for (id, entity_json) in results {
            if let Some(entity_json) = entity_json {
                if detail == DetailLevel::Full {
                    found.push(ManifestSearch::with_unique_id(entity_json, &id));
                } else if let Some(entity) = self.get_entity_archived(&id)? {
                    found.push(self.summary_for_standard(&id, entity));
                } else {
                    not_found.push(id);
                }
            } else {
                not_found.push(id);
            }
        }

        Ok(serde_json::to_value(SuccessResponse::new(
            serde_json::json!({
                "entities": found,
                "not_found": not_found,
                "found_count": found.len(),
                "not_found_count": not_found.len()
            }),
            found.len(),
        ))?)
    }
}

fn sql_contains_templating(sql: &str) -> bool {
    sql.contains("{{")
        || sql.contains("{%")
        || sql.contains("{#")
        || sql.contains("{ ref(")
        || sql.contains("{{ ref(")
}
