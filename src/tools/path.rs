use std::collections::HashSet;

use serde_json::Value as JsonValue;

use crate::error::{DbtNovaError, Result};
use crate::manifest::search::ManifestSearch;
use crate::params::{DetailLevel, FindByPathParams};
use crate::responses::SuccessResponse;
use crate::utils::{compile_glob, glob_match_compiled};
use tracing::instrument;

struct PathMatch {
    unique_id: String,
    sort_path: String,
    original_path: String,
    manifest_path: String,
}

impl ManifestSearch {
    /// Find entities by file path pattern.
    ///
    /// # Errors
    /// Returns an error if the path pattern is invalid or entities cannot be resolved.
    #[instrument(skip(self, params), fields(tool = "find_by_path", pattern_len = params.path_pattern.len(), limit = ?params.pagination.limit, offset = params.pagination.offset))]
    #[allow(clippy::too_many_lines)]
    pub async fn find_by_path(&self, params: &FindByPathParams) -> Result<JsonValue> {
        if params.path_pattern.chars().count() > self.config.search.max_path_pattern_length {
            return Err(DbtNovaError::InvalidParams(format!(
                "Path pattern exceeds maximum length of {} characters",
                self.config.search.max_path_pattern_length
            )));
        }
        if params.pagination.offset > self.config.search.max_offset {
            return Err(DbtNovaError::InvalidParams(format!(
                "Offset exceeds maximum of {}",
                self.config.search.max_offset
            )));
        }

        let pattern = params.path_pattern.trim_start_matches("./");

        let candidates = self.get_path_candidates(pattern);

        let matcher = compile_glob(pattern, true);
        let is_match = |path: &str| -> bool { glob_match_compiled(&matcher, path) };

        let detail = self.detail_level(params.detail);
        let limit = self.page_limit(params.pagination.limit);
        let offset = params.pagination.offset;

        let mut path_matches: Vec<PathMatch> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();

        for unique_id in candidates {
            if !seen.insert(unique_id.clone()) {
                continue;
            }

            let Some(entity) = self.get_entity(&unique_id).await? else {
                continue;
            };

            if !params.resource_types.is_empty() {
                let rt = entity.resource_type.as_deref().unwrap_or("");
                if !params.resource_types.iter().any(|r| r == rt) {
                    continue;
                }
            }

            let entity_json = entity.to_json_value();
            let original_path = entity
                .original_file_path
                .as_deref()
                .unwrap_or("")
                .trim_start_matches("./");
            let manifest_path = entity_json
                .get("path")
                .and_then(|p| p.as_str())
                .unwrap_or("")
                .trim_start_matches("./");

            let matched_path = if !original_path.is_empty() && is_match(original_path) {
                Some(original_path)
            } else if !manifest_path.is_empty() && is_match(manifest_path) {
                Some(manifest_path)
            } else {
                None
            };

            if let Some(matched_path) = matched_path {
                let output_path = if original_path.is_empty() {
                    matched_path.to_string()
                } else {
                    original_path.to_string()
                };
                path_matches.push(PathMatch {
                    unique_id,
                    sort_path: output_path,
                    original_path: original_path.to_string(),
                    manifest_path: manifest_path.to_string(),
                });
            }
        }

        path_matches.sort_by(|left, right| {
            left.sort_path
                .cmp(&right.sort_path)
                .then_with(|| left.unique_id.cmp(&right.unique_id))
        });

        let total_matches = path_matches.len();
        let mut result_rows: Vec<JsonValue> = Vec::new();
        for path_match in path_matches.into_iter().skip(offset).take(limit) {
            if detail == DetailLevel::Full {
                let Some(entity) = self.get_entity(&path_match.unique_id).await? else {
                    continue;
                };
                result_rows.push(ManifestSearch::with_unique_id(
                    entity.to_json_value(),
                    &path_match.unique_id,
                ));
            } else {
                let archived = self.get_entity_archived(&path_match.unique_id)?;
                let mut summary = if detail == DetailLevel::Compact {
                    archived.map_or(JsonValue::Null, |entity| {
                        self.summary_for_compact(&path_match.unique_id, entity)
                    })
                } else {
                    self.entity_summary(&path_match.unique_id)
                        .unwrap_or(JsonValue::Null)
                };
                if let Some(obj) = summary.as_object_mut() {
                    let output_path = if path_match.original_path.is_empty() {
                        path_match.sort_path
                    } else {
                        path_match.original_path
                    };
                    obj.insert(
                        "original_file_path".to_string(),
                        JsonValue::String(output_path),
                    );
                    if !path_match.manifest_path.is_empty() {
                        obj.insert(
                            "path".to_string(),
                            JsonValue::String(path_match.manifest_path),
                        );
                    }
                }
                result_rows.push(summary);
            }
        }

        let count = result_rows.len();
        let mut response = SuccessResponse::new(result_rows, count).with_total(total_matches);
        if total_matches > count + offset {
            response = response.with_truncated(true);
        }
        Ok(serde_json::to_value(response)?)
    }
}
