use std::collections::{HashMap, HashSet, VecDeque};

use crate::config::ConfidenceTier;
use crate::error::{DbtNovaError, Result};
use crate::manifest::lineage_sql::{find_sql_aliases, sql_for_matching};
use crate::manifest::search::ManifestSearch;
use crate::params::GetColumnLineageParams;
use crate::responses::SuccessResponse;
use crate::tools::helpers::dependency_hints_from_json;
use crate::utils::levenshtein_similarity;
use serde::Serialize;
use serde_json::{Value as JsonValue, json};
use tracing::instrument;

#[derive(Debug, Serialize, Clone)]
pub struct MatchExplanation {
    pub strategies_tried: Vec<String>,
    pub successful_strategy: String,
    pub details: JsonValue,
}

#[derive(Debug, Serialize, Clone)]
pub struct ColumnMatch {
    pub column_name: String,
    pub confidence: String,
    pub match_reason: String,
    pub score: f64,
    pub explanation: MatchExplanation,
}

impl ManifestSearch {
    /// Trace column lineage upstream or downstream with configurable matching strategies.
    ///
    /// # Errors
    /// Returns an error if the entity or column cannot be resolved.
    #[instrument(skip(self, params), fields(tool = "get_column_lineage", id_or_name = %params.id_or_name, column = %params.column_name, direction = %params.direction))]
    #[allow(clippy::too_many_lines)]
    pub async fn get_column_lineage(&self, params: &GetColumnLineageParams) -> Result<JsonValue> {
        let resource_type = params.resource_type.as_deref();
        let start_id = self.resolve_single_id(&params.id_or_name, resource_type)?;
        let start_entity = self
            .get_entity(&start_id)
            .await?
            .ok_or_else(|| self.entity_not_found(&start_id, resource_type))?;

        let start_columns = self.get_entity_columns(&start_entity);
        let column_lower = params.column_name.to_lowercase();

        if !start_columns
            .iter()
            .any(|c| c.to_lowercase() == column_lower)
        {
            return Err(DbtNovaError::EntityNotFound {
                query: format!("{}.{}", &start_id, params.column_name),
                resource_type: None,
                available_resource_types: Vec::new(),
            });
        }

        let manifest_map = match params.direction.as_str() {
            "upstream" => &self.parent_map,
            "downstream" => &self.child_map,
            _ => {
                return Err(DbtNovaError::InvalidParams(
                    "direction must be 'upstream' or 'downstream'".to_string(),
                ));
            }
        };

        let configured_max = self.config.column_lineage.max_depth;
        let max_depth = params.depth.unwrap_or(configured_max).min(configured_max);
        if max_depth == 0 {
            return Err(DbtNovaError::InvalidParams(
                "depth must be greater than 0".to_string(),
            ));
        }
        let max_results = self.config.column_lineage.max_results.max(1);
        let max_candidates = self.config.column_lineage.max_match_candidates.max(1);
        let requested_conf = params
            .confidence
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(&self.config.column_lineage.default_confidence)
            .to_lowercase();
        let requested_rank = confidence_rank(
            &requested_conf,
            &self.config.column_lineage.default_confidence,
        );

        let mut results: Vec<JsonValue> = Vec::new();
        let mut visited_entities: HashSet<String> = HashSet::new();
        visited_entities.insert(start_id.clone());
        let mut truncated = false;

        let mut queue: VecDeque<(String, String, usize, Vec<String>)> =
            VecDeque::from([(start_id.clone(), params.column_name.clone(), 0, vec![])]);

        while let Some((current_id, current_column, depth, path)) = queue.pop_front() {
            if truncated {
                break;
            }
            if depth >= max_depth {
                continue;
            }

            let Some(current_entity) = self.get_entity(&current_id).await? else {
                continue;
            };
            let current_json = current_entity.to_json_value();

            let mut related_entities = manifest_map.get(&current_id).cloned().unwrap_or_default();
            related_entities.sort_unstable();
            for related_id in related_entities {
                if truncated {
                    break;
                }
                if visited_entities.contains(&related_id) {
                    continue;
                }

                let Some(related_entity) = self.get_entity(&related_id).await? else {
                    continue;
                };
                let related_json = related_entity.to_json_value();

                let related_columns = self.get_entity_columns(&related_entity);
                let sql_entity = if params.direction == "downstream" {
                    &related_json
                } else {
                    &current_json
                };
                let sql_entity_id = if params.direction == "downstream" {
                    related_id.as_str()
                } else {
                    current_id.as_str()
                };
                let sql_aliases = self.sql_aliases_for(sql_entity_id);
                let matches = match_column(
                    &current_json,
                    &current_column,
                    &related_json,
                    &related_columns,
                    sql_entity,
                    sql_aliases,
                    &self.config,
                );

                for match_info in matches {
                    let rank = confidence_rank(
                        &match_info.confidence,
                        &self.config.column_lineage.default_confidence,
                    );

                    if rank <= requested_rank {
                        let mut new_path = path.clone();
                        new_path.push(current_id.clone());

                        results.push(json!({
                            "unique_id": related_id,
                            "name": related_json.get("name"),
                            "resource_type": related_json.get("resource_type"),
                            "column": match_info.column_name,
                            "confidence": match_info.confidence,
                            "match_reason": match_info.match_reason,
                            "score": match_info.score,
                            "explanation": match_info.explanation,
                            "depth": depth + 1,
                            "path": new_path,
                            "source_entity": current_id,
                            "source_column": current_column,
                        }));

                        if results.len() >= max_results {
                            truncated = true;
                            break;
                        }
                    }
                }

                if !visited_entities.contains(&related_id) {
                    if queue.len() >= max_candidates {
                        tracing::warn!(
                            max_candidates,
                            "Column lineage reached max candidates, stopping early"
                        );
                        truncated = true;
                        break;
                    }
                    queue.push_back((
                        related_id.clone(),
                        current_column.clone(),
                        depth + 1,
                        path.clone(),
                    ));
                    visited_entities.insert(related_id);
                }
            }
        }

        results.sort_by(|a, b| {
            let depth_a = a.get("depth").and_then(JsonValue::as_u64).unwrap_or(0);
            let depth_b = b.get("depth").and_then(JsonValue::as_u64).unwrap_or(0);
            depth_a.cmp(&depth_b).then_with(|| {
                let score_a = a.get("score").and_then(JsonValue::as_f64).unwrap_or(0.0);
                let score_b = b.get("score").and_then(JsonValue::as_f64).unwrap_or(0.0);
                score_b
                    .partial_cmp(&score_a)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| {
                        let id_a = a.get("unique_id").and_then(JsonValue::as_str).unwrap_or("");
                        let id_b = b.get("unique_id").and_then(JsonValue::as_str).unwrap_or("");
                        id_a.cmp(id_b)
                    })
                    .then_with(|| {
                        let col_a = a.get("column").and_then(JsonValue::as_str).unwrap_or("");
                        let col_b = b.get("column").and_then(JsonValue::as_str).unwrap_or("");
                        col_a.cmp(col_b)
                    })
            })
        });

        let start_entity_json = start_entity.to_json_value();
        let (lineage_hints, hints_total) = dependency_hints_from_json(&start_entity_json);
        let lineage_status = if results.is_empty() {
            if hints_total == 0 {
                "no_dependencies_recorded"
            } else {
                "dependencies_unresolved_or_filtered"
            }
        } else {
            "ok"
        };

        let count = results.len();
        let payload = json!({
            "start_entity": &start_id,
            "start_column": params.column_name,
            "direction": params.direction,
            "lineage": results,
            "lineage_status": lineage_status,
            "lineage_hints": lineage_hints
        });
        let response = SuccessResponse::new(payload, count);
        let response = if truncated {
            response.with_truncated(true)
        } else {
            response
        };
        Ok(serde_json::to_value(response)?)
    }
}

fn confidence_rank(conf: &str, default_conf: &str) -> u8 {
    let conf = if conf.is_empty() { default_conf } else { conf };
    match conf {
        "high" => 0,
        "low" => 2,
        _ => 1,
    }
}

fn record_match(
    best: &mut HashMap<String, ColumnMatch>,
    column: String,
    reason: &str,
    tier: ConfidenceTier,
    weight: f64,
    score: f64,
    details: JsonValue,
) {
    let confidence = tier.as_str().to_string();
    let entry = ColumnMatch {
        column_name: column.clone(),
        confidence: confidence.clone(),
        match_reason: reason.to_string(),
        score: score * weight,
        explanation: MatchExplanation {
            strategies_tried: vec![reason.to_string()],
            successful_strategy: reason.to_string(),
            details,
        },
    };

    match best.get(&column) {
        Some(existing) => {
            if entry.score > existing.score {
                best.insert(column, entry);
            }
        }
        None => {
            best.insert(column, entry);
        }
    }
}

/// Matches output columns to upstream source columns using cascading strategies.
///
/// # Strategies (in order of priority)
///
/// 1. **Exact Match** (`confidence: high`)
///    - Output column name equals source column name exactly
///    - Example: `customer_id` → `customer_id`
///
/// 2. **SQL Alias** (`confidence: high`)
///    - Detects `source_col AS output_col` patterns in SQL
///    - Example: `user_id AS customer_id` → matches `user_id`
///
/// 3. **SQL Proximity** (`confidence: medium`)
///    - Source column mentioned within N tokens of output in SQL
///    - Catches transformations like `COALESCE(a, b) AS result`
///
/// 4. **Suffix Match** (`confidence: medium`)
///    - Source column is suffix of output: `table_column` matches `column`
///    - Example: `users_customer_id` → `customer_id`
///
/// 5. **Prefix Match** (`confidence: medium`)
///    - Source column is prefix of output: `column_suffix` matches `column`
///    - Example: `customer_id_v2` → `customer_id`
///
/// 6. **Normalized Match** (`confidence: low`)
///    - Case-insensitive, underscore-normalized comparison
///    - Example: `CustomerID` → `customer_id`
///
/// 7. **Levenshtein Distance** (`confidence: low`)
///    - Fuzzy match with edit distance ≤ threshold (default: 0.75 similarity)
///    - Catches typos and minor variations
///
/// # Arguments
/// * `output_column` - The column to find upstream sources for
/// * `upstream_columns` - Available columns from upstream models
/// * `sql` - Optional SQL for alias/proximity detection
/// * `config` - Column lineage configuration
///
/// # Returns
/// `Vec<ColumnMatch>` ordered by confidence (high → low)
#[allow(clippy::too_many_lines)]
fn match_column(
    source_entity: &JsonValue,
    source_column: &str,
    target_entity: &JsonValue,
    target_columns: &[String],
    sql_entity: &JsonValue,
    sql_aliases: Option<&HashMap<String, String>>,
    config: &crate::config::DbtNovaConfig,
) -> Vec<ColumnMatch> {
    let mut best: HashMap<String, ColumnMatch> = HashMap::new();

    let source_columns_map = source_entity.get("columns").and_then(|c| c.as_object());
    let target_columns_map = target_entity.get("columns").and_then(|c| c.as_object());

    let source_col_value = source_columns_map.and_then(|m| m.get(source_column));
    let source_lower = source_column.to_lowercase();

    if config.column_lineage.matching.exact_match.enabled {
        let strat = &config.column_lineage.matching.exact_match;
        for col in target_columns {
            if col.eq_ignore_ascii_case(source_column)
                && columns_compatible(
                    source_col_value,
                    target_columns_map.and_then(|m| m.get(col)),
                )
            {
                record_match(
                    &mut best,
                    col.clone(),
                    "exact_match",
                    strat.confidence_tier,
                    strat.weight,
                    1.0,
                    json!({"column": col}),
                );
            }
        }
    }

    if config.column_lineage.matching.sql_alias.enabled {
        let strat = &config.column_lineage.matching.sql_alias;
        let mut handle_aliases = |aliases: &HashMap<String, String>| {
            for (src, alias) in aliases {
                let src_lower = src.to_lowercase();
                let alias_lower = alias.to_lowercase();
                if source_lower == src_lower {
                    for col in target_columns {
                        if col.eq_ignore_ascii_case(alias)
                            && columns_compatible(
                                source_col_value,
                                target_columns_map.and_then(|m| m.get(col)),
                            )
                        {
                            record_match(
                                &mut best,
                                col.clone(),
                                "sql_alias",
                                strat.confidence_tier,
                                strat.weight,
                                0.9,
                                json!({"alias": alias}),
                            );
                        }
                    }
                } else if source_lower == alias_lower {
                    for col in target_columns {
                        if col.eq_ignore_ascii_case(src)
                            && columns_compatible(
                                source_col_value,
                                target_columns_map.and_then(|m| m.get(col)),
                            )
                        {
                            record_match(
                                &mut best,
                                col.clone(),
                                "sql_alias",
                                strat.confidence_tier,
                                strat.weight,
                                0.9,
                                json!({"alias": alias, "source": src}),
                            );
                        }
                    }
                }
            }
        };

        if let Some(aliases) = sql_aliases {
            handle_aliases(aliases);
        } else if let Some(sql) = sql_for_matching(sql_entity) {
            let aliases = find_sql_aliases(sql);
            handle_aliases(&aliases);
        }
    }

    if config.column_lineage.matching.sql_proximity.enabled {
        let strat = &config.column_lineage.matching.sql_proximity;
        if let Some(sql) = sql_for_matching(sql_entity) {
            let sql_lower = sql.to_lowercase();
            let source_positions = find_all_positions(&sql_lower, &source_lower);
            if !source_positions.is_empty() {
                for col in target_columns {
                    let col_lower = col.to_lowercase();
                    if !is_name_similar(&source_lower, &col_lower, config) {
                        continue;
                    }
                    let col_positions = find_all_positions(&sql_lower, &col_lower);
                    if col_positions.is_empty() {
                        continue;
                    }
                    if !columns_compatible(
                        source_col_value,
                        target_columns_map.and_then(|m| m.get(col)),
                    ) {
                        continue;
                    }

                    let mut best_distance: Option<usize> = None;
                    let mut best_pair: Option<(usize, usize)> = None;
                    for sp in &source_positions {
                        for cp in &col_positions {
                            let dist = sp.abs_diff(*cp);
                            if dist <= config.column_lineage.sql_proximity_max_distance
                                && best_distance.is_none_or(|best| dist < best)
                            {
                                best_distance = Some(dist);
                                best_pair = Some((*sp, *cp));
                            }
                        }
                    }

                    if let Some(distance) = best_distance {
                        let (source_pos, target_pos) = best_pair.unwrap_or((0, 0));
                        record_match(
                            &mut best,
                            col.clone(),
                            "sql_proximity",
                            strat.confidence_tier,
                            strat.weight,
                            0.7,
                            json!({
                                "source_position": source_pos,
                                "target_position": target_pos,
                                "distance": distance
                            }),
                        );
                    }
                }
            }
        }
    }

    if let Some(sql) = sql_for_matching(sql_entity) {
        let sql_lower = sql.to_lowercase();
        for col in target_columns {
            let Some(target_meta) = target_columns_map.and_then(|m| m.get(col)) else {
                continue;
            };
            let data_type = target_meta
                .get("data_type")
                .and_then(JsonValue::as_str)
                .unwrap_or("")
                .to_lowercase();
            if !data_type.contains("struct") {
                continue;
            }
            let col_lower = col.to_lowercase();
            let needle = format!("{col_lower}.{source_lower}");
            let underscored = format!("{col_lower}._{source_lower}");
            if sql_lower.contains(&needle) || sql_lower.contains(&underscored) {
                let matched_field = if sql_lower.contains(&underscored) {
                    format!("_{source_column}")
                } else {
                    source_column.to_string()
                };
                let strat = &config.column_lineage.matching.sql_alias;
                let field_path = format!("{col}.{matched_field}");
                record_match(
                    &mut best,
                    col.clone(),
                    "sql_nested_field",
                    strat.confidence_tier,
                    strat.weight,
                    0.65,
                    json!({"struct_column": col, "field": matched_field, "field_path": field_path}),
                );
            }
        }

        let source_type = source_col_value
            .and_then(|c| c.get("data_type"))
            .and_then(JsonValue::as_str)
            .unwrap_or("")
            .to_lowercase();
        if source_type.contains("struct") {
            let strat = &config.column_lineage.matching.sql_alias;
            for col in target_columns {
                let col_lower = col.to_lowercase();
                let needle = format!("{source_lower}.{col_lower}");
                let underscored = format!("{source_lower}._{col_lower}");
                if sql_lower.contains(&needle) || sql_lower.contains(&underscored) {
                    let matched_field = if sql_lower.contains(&underscored) {
                        format!("_{col}")
                    } else {
                        col.clone()
                    };
                    let field_path = format!("{source_column}.{matched_field}");
                    record_match(
                        &mut best,
                        col.clone(),
                        "sql_nested_field",
                        strat.confidence_tier,
                        strat.weight,
                        0.65,
                        json!({"struct_column": source_column, "field": matched_field, "field_path": field_path}),
                    );
                }
            }
        }
    }

    if config.column_lineage.matching.suffix_match.enabled {
        let strat = &config.column_lineage.matching.suffix_match;
        for col in target_columns {
            let col_lower = col.to_lowercase();
            if (col_lower.ends_with(&source_lower) || source_lower.ends_with(&col_lower))
                && col_lower.len() >= config.column_lineage.min_prefix_suffix_length
                && source_lower.len() >= config.column_lineage.min_prefix_suffix_length
                && columns_compatible(
                    source_col_value,
                    target_columns_map.and_then(|m| m.get(col)),
                )
            {
                record_match(
                    &mut best,
                    col.clone(),
                    "suffix_match",
                    strat.confidence_tier,
                    strat.weight,
                    0.6,
                    json!({"col": col_lower, "source": source_lower}),
                );
            }
        }
    }

    if config.column_lineage.matching.prefix_match.enabled {
        let strat = &config.column_lineage.matching.prefix_match;
        for col in target_columns {
            let col_lower = col.to_lowercase();
            if (col_lower.starts_with(&source_lower) || source_lower.starts_with(&col_lower))
                && col_lower.len() >= config.column_lineage.min_prefix_suffix_length
                && source_lower.len() >= config.column_lineage.min_prefix_suffix_length
                && columns_compatible(
                    source_col_value,
                    target_columns_map.and_then(|m| m.get(col)),
                )
            {
                record_match(
                    &mut best,
                    col.clone(),
                    "prefix_match",
                    strat.confidence_tier,
                    strat.weight,
                    0.6,
                    json!({"col": col_lower, "source": source_lower}),
                );
            }
        }
    }

    if config.column_lineage.matching.normalized_match.enabled {
        let strat = &config.column_lineage.matching.normalized_match;
        let source_normalized: String = source_lower
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect();
        for col in target_columns {
            let col_normalized: String = col
                .to_lowercase()
                .chars()
                .filter(|c| c.is_alphanumeric())
                .collect();
            if !source_normalized.is_empty()
                && source_normalized == col_normalized
                && columns_compatible(
                    source_col_value,
                    target_columns_map.and_then(|m| m.get(col)),
                )
            {
                record_match(
                    &mut best,
                    col.clone(),
                    "normalized_match",
                    strat.confidence_tier,
                    strat.weight,
                    0.55,
                    json!({"normalized": source_normalized}),
                );
            }
        }
    }

    if config.column_lineage.matching.levenshtein_match.enabled {
        let strat = &config.column_lineage.matching.levenshtein_match;
        for col in target_columns {
            let col_lower = col.to_lowercase();
            if source_lower.len() >= config.column_lineage.min_levenshtein_length
                && col_lower.len() >= config.column_lineage.min_levenshtein_length
            {
                let similarity = levenshtein_similarity(&source_lower, &col_lower);
                if similarity >= config.column_lineage.levenshtein_threshold
                    && columns_compatible(
                        source_col_value,
                        target_columns_map.and_then(|m| m.get(col)),
                    )
                {
                    record_match(
                        &mut best,
                        col.clone(),
                        "levenshtein_match",
                        strat.confidence_tier,
                        strat.weight,
                        similarity,
                        json!({"similarity": similarity}),
                    );
                }
            }
        }
    }

    if best.is_empty() && target_columns.is_empty() {
        let is_source = target_entity
            .get("resource_type")
            .and_then(JsonValue::as_str)
            .is_some_and(|rt| rt == "source");
        if is_source && let Some(sql) = sql_for_matching(sql_entity) {
            let sql_lower = sql.to_lowercase();
            if !source_lower.is_empty() && sql_lower.contains(&source_lower) {
                let strat = &config.column_lineage.matching.exact_match;
                record_match(
                    &mut best,
                    source_column.to_string(),
                    "direct_select",
                    strat.confidence_tier,
                    strat.weight,
                    0.85,
                    json!({"column": source_column}),
                );
            }
        }
    }

    best.into_values().collect()
}

fn find_all_positions(haystack: &str, needle: &str) -> Vec<usize> {
    if needle.is_empty() {
        return Vec::new();
    }
    let mut positions = Vec::new();
    let mut start = 0usize;
    while let Some(pos) = haystack[start..].find(needle) {
        let idx = start + pos;
        positions.push(idx);
        start = idx + needle.len();
    }
    positions
}

fn is_name_similar(source: &str, target: &str, config: &crate::config::DbtNovaConfig) -> bool {
    if source == target {
        return true;
    }
    if source.is_empty() || target.is_empty() {
        return false;
    }
    if source.contains(target) || target.contains(source) {
        return true;
    }

    let source_norm: String = source.chars().filter(|c| c.is_alphanumeric()).collect();
    let target_norm: String = target.chars().filter(|c| c.is_alphanumeric()).collect();
    if !source_norm.is_empty() && source_norm == target_norm {
        return true;
    }

    if source.len() >= config.column_lineage.min_levenshtein_length
        && target.len() >= config.column_lineage.min_levenshtein_length
    {
        let similarity = levenshtein_similarity(source, target);
        if similarity >= config.column_lineage.levenshtein_threshold {
            return true;
        }
    }

    false
}

fn columns_compatible(source_col: Option<&JsonValue>, target_col: Option<&JsonValue>) -> bool {
    let source_type = source_col
        .and_then(|c| c.get("data_type"))
        .and_then(|t| t.as_str());
    let target_type = target_col
        .and_then(|c| c.get("data_type"))
        .and_then(|t| t.as_str());

    match (source_type, target_type) {
        (Some(s), Some(t)) => types_compatible(s, t),
        _ => true,
    }
}

fn types_compatible(a: &str, b: &str) -> bool {
    let numeric = [
        "int", "integer", "bigint", "float", "double", "decimal", "numeric",
    ];
    let string = ["varchar", "text", "string", "char"];
    let temporal = ["date", "datetime", "timestamp", "time"];

    let a = a.to_lowercase();
    let b = b.to_lowercase();

    if numeric.iter().any(|t| a.contains(t)) && numeric.iter().any(|t| b.contains(t)) {
        return true;
    }
    if string.iter().any(|t| a.contains(t)) && string.iter().any(|t| b.contains(t)) {
        return true;
    }
    if temporal.iter().any(|t| a.contains(t)) && temporal.iter().any(|t| b.contains(t)) {
        return true;
    }

    a == b
}
