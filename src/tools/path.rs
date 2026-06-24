use std::collections::HashSet;

use serde_json::Value as JsonValue;

use crate::error::{DbtNovaError, Result};
use crate::manifest::search::ManifestSearch;
use crate::params::{DetailLevel, FindByPathParams};
use crate::responses::SuccessResponse;
use crate::utils::{compile_glob, glob_match_compiled};
use tracing::instrument;

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
        let match_scan_cap = offset.saturating_add(limit);

        let mut result_rows: Vec<JsonValue> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        let mut skipped = 0usize;
        let mut total_matches = 0usize;

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
                total_matches += 1;
                if total_matches > match_scan_cap {
                    break;
                }
                if skipped < offset {
                    skipped += 1;
                    continue;
                }

                if result_rows.len() >= limit {
                    continue;
                }

                if detail == DetailLevel::Full {
                    result_rows.push(ManifestSearch::with_unique_id(entity_json, &unique_id));
                } else {
                    let archived = self.get_entity_archived(&unique_id)?;
                    let mut summary = if detail == DetailLevel::Compact {
                        archived.map_or(JsonValue::Null, |entity| {
                            self.summary_for_compact(&unique_id, entity)
                        })
                    } else {
                        self.entity_summary(&unique_id).unwrap_or(JsonValue::Null)
                    };
                    if let Some(obj) = summary.as_object_mut() {
                        let output_path = if original_path.is_empty() {
                            matched_path.to_string()
                        } else {
                            original_path.to_string()
                        };
                        obj.insert(
                            "original_file_path".to_string(),
                            JsonValue::String(output_path),
                        );
                        if !manifest_path.is_empty() {
                            obj.insert(
                                "path".to_string(),
                                JsonValue::String(manifest_path.to_string()),
                            );
                        }
                    }
                    result_rows.push(summary);
                }
            }
        }

        result_rows.sort_by(|a, b| {
            let path_a = a
                .get("original_file_path")
                .or_else(|| a.get("path"))
                .and_then(|p| p.as_str())
                .unwrap_or("");
            let path_b = b
                .get("original_file_path")
                .or_else(|| b.get("path"))
                .and_then(|p| p.as_str())
                .unwrap_or("");
            path_a.cmp(path_b)
        });

        let count = result_rows.len();
        let mut response = SuccessResponse::new(result_rows, count).with_total(total_matches);
        if total_matches > count + offset {
            response = response.with_truncated(true);
        }
        Ok(serde_json::to_value(response)?)
    }
}
