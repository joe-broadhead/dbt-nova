use std::collections::{HashMap, HashSet, VecDeque};

use serde_json::Value as JsonValue;

use crate::error::{DbtNovaError, Result};
use crate::manifest::search::ManifestSearch;
use crate::params::{DetailLevel, GetImpactParams, GetLineageParams};
use crate::responses::SuccessResponse;
use crate::tools::helpers::dependency_hints_from_json;
use tracing::{debug, instrument};

impl ManifestSearch {
    /// Get upstream or downstream lineage for an entity.
    ///
    /// # Errors
    /// Returns an error if the entity cannot be resolved or lineage traversal fails.
    #[instrument(
        skip(self, params),
        fields(
            tool = "get_lineage",
            id_or_name = %params.id_or_name,
            direction = %params.direction,
            detail = ?params.detail
        )
    )]
    #[allow(clippy::too_many_lines)]
    pub async fn get_lineage(&self, params: &GetLineageParams) -> Result<JsonValue> {
        let (root_id, entity) = self.resolve_entity(&params.id_or_name, None).await?;
        debug!(
            unique_id = %root_id,
            direction = %params.direction,
            depth = params.depth.unwrap_or(self.config.lineage_max_depth),
            resource_types = ?params.resource_types,
            detail = ?params.detail,
            "lineage request"
        );
        let map = match params.direction.as_str() {
            "upstream" => &self.parent_map,
            "downstream" => &self.child_map,
            _ => {
                return Err(DbtNovaError::InvalidParams(
                    "direction must be 'upstream' or 'downstream'".to_string(),
                ));
            }
        };

        let configured_max = self.config.lineage_max_depth;
        let max_depth = params.depth.unwrap_or(configured_max).min(configured_max);
        if max_depth == 0 {
            return Err(DbtNovaError::InvalidParams(
                "depth must be greater than 0".to_string(),
            ));
        }
        let max_results = self.config.lineage_max_results.max(1);
        let detail = self.detail_level(params.detail);
        let cache_key = if self.config.search.lineage_cache_size > 0 {
            Some(lineage_cache_key(
                &root_id,
                params,
                detail,
                max_depth,
                max_results,
            ))
        } else {
            None
        };
        if let Some(key) = cache_key.as_ref()
            && let Some(cached) = self.lineage_cache_get(key)
        {
            return self.refresh_lineage_response_provenance(cached);
        }
        let mut visited_depth: HashMap<String, usize> = HashMap::new();
        let mut result_set: HashSet<String> = HashSet::new();
        let mut result: Vec<String> = Vec::new();
        let mut edges: Vec<JsonValue> = Vec::new();
        let mut edge_set: HashSet<(String, String)> = HashSet::new();
        let mut queue: VecDeque<(String, usize)> = VecDeque::new();
        let mut depth_reached = 0usize;
        let mut truncated = false;

        visited_depth.insert(root_id.clone(), 0);
        queue.push_back((root_id.clone(), 0));

        while let Some((id, depth_used)) = queue.pop_front() {
            if truncated {
                break;
            }
            depth_reached = depth_reached.max(depth_used);
            if let Some(related) = map.get(&id) {
                let mut related_ids: Vec<&String> = related.iter().collect();
                related_ids.sort_unstable();
                for rel_id in related_ids {
                    if truncated {
                        break;
                    }

                    let include = lineage_type_allowed(
                        self.resource_type_by_id.get(rel_id).map(String::as_str),
                        &params.resource_types,
                    );
                    let next_depth = depth_used + usize::from(include);
                    if next_depth > max_depth {
                        continue;
                    }

                    let should_visit = match visited_depth.get(rel_id) {
                        Some(previous_depth) => next_depth < *previous_depth,
                        None => true,
                    };
                    if !should_visit {
                        continue;
                    }

                    visited_depth.insert(rel_id.clone(), next_depth);
                    queue.push_back((rel_id.clone(), next_depth));

                    if include && result_set.insert(rel_id.clone()) {
                        result.push(rel_id.clone());
                        let (from, to) = if params.direction == "upstream" {
                            (rel_id.clone(), id.clone())
                        } else {
                            (id.clone(), rel_id.clone())
                        };
                        if edge_set.insert((from.clone(), to.clone())) {
                            edges.push(serde_json::json!({
                                "from": from,
                                "to": to
                            }));
                        }
                        if result.len() >= max_results {
                            truncated = true;
                            break;
                        }
                    }
                }
            }
        }

        if result.len() > max_results {
            result.truncate(max_results);
            truncated = true;
        }

        let mut results: Vec<JsonValue> = Vec::with_capacity(result.len());
        for id in &result {
            match detail {
                DetailLevel::Full => {
                    if let Some(archived) = self.get_entity_archived(id)? {
                        let mut entity = archived.to_json_value();
                        ManifestSearch::insert_unique_id(&mut entity, id);
                        let provenance = self.provenance_for_archived_json(id, archived, &entity);
                        if let Some(obj) = entity.as_object_mut() {
                            obj.insert("provenance".to_string(), provenance);
                        }
                        results.push(entity);
                    } else {
                        results.push(JsonValue::Null);
                    }
                }
                DetailLevel::Compact => {
                    if let Some(entity) = self.get_entity_archived(id)? {
                        let mut summary = self.summary_for_compact(id, entity);
                        let entity_json = entity.to_json_value();
                        let provenance =
                            self.provenance_for_archived_json(id, entity, &entity_json);
                        if let Some(obj) = summary.as_object_mut() {
                            obj.insert("provenance".to_string(), provenance);
                        }
                        results.push(summary);
                    }
                }
                DetailLevel::Standard => {
                    if let Ok(mut summary) = self.entity_summary(id) {
                        if let Some(entity) = self.get_entity_archived(id)? {
                            let entity_json = entity.to_json_value();
                            let provenance =
                                self.provenance_for_archived_json(id, entity, &entity_json);
                            if let Some(obj) = summary.as_object_mut() {
                                obj.insert("provenance".to_string(), provenance);
                            }
                        }
                        results.push(summary);
                    } else {
                        results.push(serde_json::json!({
                            "unique_id": id,
                            "name": null,
                            "resource_type": null
                        }));
                    }
                }
            }
        }

        let count = results.len();
        let entity_json = entity.to_json_value();
        let (lineage_hints, hints_total) = dependency_hints_from_json(&entity_json);
        let lineage_status = if results.is_empty() && edges.is_empty() {
            if hints_total == 0 {
                "no_dependencies_recorded"
            } else {
                "dependencies_unresolved_or_filtered"
            }
        } else {
            "ok"
        };

        let mut data = serde_json::json!({
            "root_id": root_id,
            "direction": params.direction,
            "depth": max_depth,
            "depth_reached": depth_reached,
            "entities": results,
            "edges": edges
        });
        if let Some(obj) = data.as_object_mut() {
            obj.insert(
                "lineage_status".to_string(),
                JsonValue::String(lineage_status.to_string()),
            );
            obj.insert("lineage_hints".to_string(), lineage_hints);
        }
        let response = SuccessResponse::new(data, count);
        let response = if truncated {
            response.with_truncated(true)
        } else {
            response
        };
        let response_value = serde_json::to_value(response)?;
        if let Some(key) = cache_key {
            self.lineage_cache_insert(key, response_value.clone());
        }
        Ok(response_value)
    }

    fn refresh_lineage_response_provenance(&self, mut response: JsonValue) -> Result<JsonValue> {
        let Some(entities) = response
            .get_mut("data")
            .and_then(|data| data.get_mut("entities"))
            .and_then(JsonValue::as_array_mut)
        else {
            return Ok(response);
        };

        for entity_value in entities {
            let Some(unique_id) = entity_value
                .get("unique_id")
                .and_then(JsonValue::as_str)
                .map(str::to_string)
            else {
                continue;
            };
            let Some(entity) = self.get_entity_archived(&unique_id)? else {
                continue;
            };
            let entity_json = entity.to_json_value();
            let provenance = self.provenance_for_archived_json(&unique_id, entity, &entity_json);
            if let Some(obj) = entity_value.as_object_mut() {
                obj.insert("provenance".to_string(), provenance);
            }
        }

        Ok(response)
    }

    /// Analyze the downstream impact of changing an entity.
    ///
    /// # Errors
    /// Returns an error if the entity or lineage cannot be resolved.
    #[instrument(skip(self, params), fields(tool = "get_impact", id_or_name = %params.id_or_name))]
    pub async fn get_impact(&self, params: &GetImpactParams) -> Result<JsonValue> {
        let (unique_id, entity) = self.resolve_entity(&params.id_or_name, None).await?;
        let column_count = entity.column_names().len();

        let lineage_params = GetLineageParams {
            id_or_name: unique_id.clone(),
            direction: "downstream".to_string(),
            depth: None,
            resource_types: vec![],
            detail: Some(DetailLevel::Standard),
        };

        let downstream_result = self.get_lineage(&lineage_params).await?;
        let downstream_entities = downstream_result
            .get("data")
            .and_then(|d| d.get("entities"))
            .and_then(|d| d.as_array())
            .cloned()
            .unwrap_or_default();
        let downstream_count = downstream_entities.len();

        let mut by_type: HashMap<String, usize> = HashMap::new();
        for item in downstream_entities {
            let rt = item
                .get("resource_type")
                .and_then(|r| r.as_str())
                .unwrap_or("unknown");
            *by_type.entry(rt.to_string()).or_default() += 1;
        }

        #[allow(clippy::cast_precision_loss)]
        let impact_score = (downstream_count as f64) * (column_count as f64);

        Ok(serde_json::to_value(SuccessResponse::new(
            serde_json::json!({
                "unique_id": unique_id,
                "downstream_count": downstream_count,
                "column_count": column_count,
                "by_type": by_type,
                "impact_score": impact_score
            }),
            1,
        ))?)
    }
}

fn lineage_cache_key(
    unique_id: &str,
    params: &GetLineageParams,
    detail: DetailLevel,
    depth: usize,
    max_results: usize,
) -> String {
    let mut resource_types = params.resource_types.clone();
    resource_types.sort();
    let detail = match detail {
        DetailLevel::Compact => "compact",
        DetailLevel::Standard => "standard",
        DetailLevel::Full => "full",
    };
    format!(
        "{id}|{direction}|{depth}|{max_results}|{detail}|{types}",
        id = unique_id,
        direction = params.direction,
        depth = depth,
        max_results = max_results,
        detail = detail,
        types = resource_types.join(","),
    )
}

fn lineage_type_allowed(resource_type: Option<&str>, allowed: &[String]) -> bool {
    if allowed.is_empty() {
        return true;
    }
    let Some(resource_type) = resource_type else {
        return false;
    };
    allowed
        .iter()
        .any(|candidate| candidate.eq_ignore_ascii_case(resource_type))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::common::get_searcher;

    #[test]
    fn cached_lineage_response_refreshes_provenance() {
        let searcher = get_searcher();
        let response = serde_json::json!({
            "success": true,
            "data": {
                "entities": [{
                    "unique_id": "model.nova_test.dim__customers",
                    "provenance": {
                        "tier": "raw",
                        "freshness": {
                            "status": "fresh",
                            "age_days": 999
                        }
                    }
                }]
            }
        });

        let refreshed = searcher
            .refresh_lineage_response_provenance(response)
            .expect("refresh provenance");
        let entity = refreshed
            .pointer("/data/entities/0")
            .expect("lineage entity");

        assert_ne!(
            entity.pointer("/provenance/tier"),
            Some(&JsonValue::String("raw".to_string()))
        );
        assert_ne!(
            entity.pointer("/provenance/freshness/age_days"),
            Some(&JsonValue::from(999))
        );
    }
}
