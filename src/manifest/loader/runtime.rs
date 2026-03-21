use std::collections::{HashMap, HashSet};

use serde_json::Value as JsonValue;
use tracing::warn;

use crate::config::DbtNovaConfig;
use crate::error::{DbtNovaError, Result};
use crate::manifest::lineage_sql::{extract_ref_calls, find_sql_aliases, sql_for_matching};
use crate::manifest::store::EntityStore;
use crate::manifest::vector_search::SearchComponentBuild;

pub(super) fn panic_message(err: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = err.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = err.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic".to_string()
    }
}

pub(super) fn combine_index_build_results<T, V, S>(
    tantivy_result: Result<T>,
    vector_result: Result<SearchComponentBuild<V>>,
    sparse_result: Result<SearchComponentBuild<S>>,
) -> Result<(T, SearchComponentBuild<V>, SearchComponentBuild<S>)> {
    let mut failures = Vec::new();
    if let Err(err) = &tantivy_result {
        failures.push(format!("tantivy: {err}"));
    }
    if let Err(err) = &vector_result {
        failures.push(format!("vector: {err}"));
    }
    if let Err(err) = &sparse_result {
        failures.push(format!("sparse: {err}"));
    }

    if !failures.is_empty() {
        return Err(DbtNovaError::ServerError(format!(
            "Manifest index build failed: {}",
            failures.join("; ")
        )));
    }

    let Ok(tantivy) = tantivy_result else {
        return Err(DbtNovaError::ServerError(
            "Manifest index build failed (tantivy result missing)".to_string(),
        ));
    };
    let Ok(vector) = vector_result else {
        return Err(DbtNovaError::ServerError(
            "Manifest index build failed (vector result missing)".to_string(),
        ));
    };
    let Ok(sparse) = sparse_result else {
        return Err(DbtNovaError::ServerError(
            "Manifest index build failed (sparse result missing)".to_string(),
        ));
    };

    Ok((tantivy, vector, sparse))
}

pub(super) fn build_column_lineage_aliases(
    entities: &EntityStore,
    config: &DbtNovaConfig,
) -> Result<HashMap<String, HashMap<String, String>>> {
    if !config.search.column_lineage_precompute {
        return Ok(HashMap::new());
    }

    let mut aliases = HashMap::new();
    for unique_id in entities.ids().cloned().collect::<Vec<_>>() {
        let Some(entity) = entities.get_blocking(&unique_id)? else {
            continue;
        };
        let payload: JsonValue = match serde_json::from_str(&entity.payload_json) {
            Ok(payload) => payload,
            Err(err) => {
                warn!(
                    unique_id = %unique_id,
                    error = %err,
                    "failed to parse entity payload_json during column lineage alias precompute; skipping entity"
                );
                continue;
            }
        };
        if let Some(sql) = sql_for_matching(&payload) {
            let map = find_sql_aliases(sql);
            if !map.is_empty() {
                aliases.insert(unique_id, map);
            }
        }
    }

    Ok(aliases)
}

pub(super) fn build_manifest_health(
    entities: &EntityStore,
    parent_map: &HashMap<String, HashSet<String>>,
    resource_type_by_id: &HashMap<String, String>,
) -> Result<JsonValue> {
    const SAMPLE_LIMIT: usize = 25;

    let mut models_total = 0usize;
    let mut models_with_dependencies = 0usize;
    let mut models_without_dependencies = 0usize;
    let mut models_with_ref_calls = 0usize;
    let mut models_ref_calls_without_dependencies = 0usize;
    let mut malformed_ref_candidate_sample = Vec::new();
    let mut ref_without_dependencies_sample = Vec::new();

    for unique_id in entities.ids() {
        if resource_type_by_id.get(unique_id).map(String::as_str) != Some("model") {
            continue;
        }
        models_total += 1;
        let dependency_count = parent_map.get(unique_id).map_or(0, HashSet::len);
        if dependency_count > 0 {
            models_with_dependencies += 1;
        } else {
            models_without_dependencies += 1;
        }

        let Some(entity) = entities.get_blocking(unique_id)? else {
            continue;
        };
        let payload = entity.to_json_value();
        let Some(sql) = sql_for_matching(&payload) else {
            continue;
        };
        let ref_calls = extract_ref_calls(sql);
        if ref_calls.is_empty() {
            continue;
        }
        models_with_ref_calls += 1;

        if dependency_count == 0 {
            models_ref_calls_without_dependencies += 1;
            if ref_without_dependencies_sample.len() < SAMPLE_LIMIT {
                ref_without_dependencies_sample.push(unique_id.clone());
            }
            if has_apparent_malformed_ref(sql)
                && malformed_ref_candidate_sample.len() < SAMPLE_LIMIT
            {
                malformed_ref_candidate_sample.push(unique_id.clone());
            }
        }
    }

    Ok(serde_json::json!({
        "is_healthy": models_ref_calls_without_dependencies == 0,
        "models_total": models_total,
        "models_with_dependencies": models_with_dependencies,
        "models_without_dependencies": models_without_dependencies,
        "models_with_ref_calls": models_with_ref_calls,
        "models_ref_calls_without_dependencies": models_ref_calls_without_dependencies,
        "malformed_ref_candidate_count": malformed_ref_candidate_sample.len(),
        "malformed_ref_candidate_sample": malformed_ref_candidate_sample,
        "ref_calls_without_dependencies_sample": ref_without_dependencies_sample
    }))
}

pub(super) fn has_apparent_malformed_ref(sql: &str) -> bool {
    let lower = sql.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let mut cursor = 0usize;

    while let Some(found) = lower[cursor..].find("ref(") {
        let ref_idx = cursor + found;
        let mut left = ref_idx;
        while left > 0 && bytes[left - 1].is_ascii_whitespace() {
            left -= 1;
        }
        if left > 0 && bytes[left - 1] == b'{' {
            let mut second = left - 1;
            while second > 0 && bytes[second - 1].is_ascii_whitespace() {
                second -= 1;
            }
            if second == 0 || bytes[second - 1] != b'{' {
                return true;
            }
        }
        cursor = ref_idx + 4;
    }

    false
}
