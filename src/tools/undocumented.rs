use serde_json::Value as JsonValue;

use crate::error::Result;
use crate::manifest::search::ManifestSearch;
use crate::params::GetUndocumentedParams;
use crate::responses::SuccessResponse;
use tracing::instrument;

impl ManifestSearch {
    /// Find undocumented entities and optionally columns.
    ///
    /// # Errors
    /// Returns an error if manifest access fails.
    #[instrument(skip(self, params), fields(tool = "get_undocumented", resource_type = %params.resource_type, limit = params.pagination.limit, offset = params.pagination.offset, include_columns = params.include_columns))]
    #[allow(clippy::too_many_lines)]
    pub async fn get_undocumented(&self, params: &GetUndocumentedParams) -> Result<JsonValue> {
        let resource_type = self.normalize_resource_type_key(&params.resource_type)?;
        let resource_candidates = self.by_resource_type.get(&resource_type).ok_or_else(|| {
            crate::error::DbtNovaError::ServerError(format!(
                "resource_type '{resource_type}' resolved but was not indexed"
            ))
        })?;

        let candidates: Vec<String> = if let Some(id_or_name) = params.id_or_name.as_deref() {
            vec![self.resolve_single_id(id_or_name, Some(&resource_type))?]
        } else {
            resource_candidates.clone()
        };

        let limit = self.page_limit(params.pagination.limit);
        let offset = params.pagination.offset;

        let mut undocumented_entities: Vec<JsonValue> = Vec::new();
        let mut undocumented_columns: Vec<JsonValue> = Vec::new();
        let mut skipped = 0usize;
        let mut total_missing_entities = 0usize;
        let mut total_missing_columns = 0usize;
        let mut entities_truncated = false;
        let mut columns_truncated = false;

        for unique_id in candidates {
            if undocumented_entities.len() >= limit {
                break;
            }

            let Some(entity) = self.get_entity(&unique_id).await? else {
                continue;
            };
            let entity_json = entity.to_json_value();

            if let Some(package) = params.package.as_deref()
                && entity.package_name.as_deref() != Some(package)
            {
                continue;
            }

            if let Some(path_prefix) = params.path_prefix.as_deref() {
                let path = entity
                    .original_file_path
                    .as_deref()
                    .or_else(|| entity_json.get("path").and_then(|v| v.as_str()))
                    .unwrap_or("");
                if !path.starts_with(path_prefix) {
                    continue;
                }
            }

            let desc = entity_json
                .get("description")
                .and_then(|d| d.as_str())
                .unwrap_or("");
            let mut include_entity = false;
            if desc.trim().is_empty() {
                total_missing_entities += 1;
                if skipped < offset {
                    skipped += 1;
                } else if undocumented_entities.len() < limit {
                    include_entity = true;
                } else {
                    entities_truncated = true;
                }
            }

            if params.include_columns
                && let Some(cols) = entity_json.get("columns").and_then(|c| c.as_object())
            {
                for (col_name, col) in cols {
                    let col_desc = col
                        .get("description")
                        .and_then(|d| d.as_str())
                        .unwrap_or("");
                    if col_desc.trim().is_empty() {
                        total_missing_columns += 1;
                        if undocumented_columns.len() < limit {
                            undocumented_columns.push(serde_json::json!({
                                "entity_unique_id": unique_id.as_str(),
                                "column_name": col_name,
                            }));
                        } else {
                            columns_truncated = true;
                        }
                    }
                }
            }

            if include_entity {
                if params.include_full {
                    undocumented_entities
                        .push(ManifestSearch::with_unique_id(entity_json, &unique_id));
                } else {
                    undocumented_entities.push(serde_json::json!({
                        "unique_id": unique_id.as_str(),
                        "name": entity.name,
                        "resource_type": entity.resource_type,
                    }));
                }
            }
        }

        let returned_entities = undocumented_entities.len();
        let returned_columns = undocumented_columns.len();
        let returned_total = returned_entities.saturating_add(returned_columns);

        let response = serde_json::json!({
            "entities": undocumented_entities,
            "summary": {
                "entities_missing_docs": total_missing_entities,
                "columns_missing_docs": total_missing_columns,
                "entities_returned": returned_entities,
                "columns_returned": returned_columns,
                "items_returned": returned_total
            },
            "columns_truncated": columns_truncated,
            "undocumented_columns": undocumented_columns,
        });

        let mut response = SuccessResponse::new(response, returned_total);
        if entities_truncated
            || columns_truncated
            || (undocumented_entities.len() >= limit
                && total_missing_entities > undocumented_entities.len() + offset)
        {
            response = response.with_truncated(true);
        }
        Ok(serde_json::to_value(response)?)
    }
}
