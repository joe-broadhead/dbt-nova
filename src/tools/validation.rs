use serde_json::Value as JsonValue;

use crate::error::Result;
use crate::manifest::search::ManifestSearch;
use crate::params::{ValidateDagDetail, ValidateDagParams};
use crate::responses::SuccessResponse;
use tracing::instrument;

impl ManifestSearch {
    /// Validate the DAG for cycles and issues.
    ///
    /// # Errors
    /// Returns an error if manifest access fails.
    #[instrument(skip(self), fields(tool = "validate_dag"))]
    #[allow(clippy::too_many_lines)]
    pub async fn validate_dag(&self, params: &ValidateDagParams) -> Result<JsonValue> {
        let mut issues: Vec<JsonValue> = Vec::new();
        let mut orphaned: Vec<String> = Vec::new();

        // Check for orphaned nodes (no parents and no children)
        for node in self.entities.ids() {
            let has_parent = self.parent_map.contains_key(node);
            let has_child = self.child_map.contains_key(node);
            if !has_parent && !has_child {
                orphaned.push(node.clone());
            }
        }

        // Cycle detection (DFS with recursion stack)
        let mut visited: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut on_stack: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut stack: Vec<String> = Vec::new();
        let mut seen_cycles: std::collections::HashSet<String> = std::collections::HashSet::new();

        fn canonical_cycle_key(nodes: &[String]) -> String {
            if nodes.is_empty() {
                return String::new();
            }
            let len = nodes.len();
            let mut best: Option<String> = None;
            for start in 0..len {
                let mut key = String::new();
                for i in 0..len {
                    if i > 0 {
                        key.push_str("->");
                    }
                    key.push_str(&nodes[(start + i) % len]);
                }
                if best.as_ref().is_none_or(|current| key < *current) {
                    best = Some(key);
                }
            }
            best.unwrap_or_default()
        }

        fn dfs(
            node: &str,
            parent_map: &std::collections::HashMap<String, Vec<String>>,
            visited: &mut std::collections::HashSet<String>,
            on_stack: &mut std::collections::HashSet<String>,
            stack: &mut Vec<String>,
            issues: &mut Vec<JsonValue>,
            seen_cycles: &mut std::collections::HashSet<String>,
        ) {
            if on_stack.contains(node) {
                let cycle_nodes = if let Some(pos) = stack.iter().position(|n| n == node) {
                    stack[pos..].to_vec()
                } else {
                    vec![node.to_string()]
                };
                let key = canonical_cycle_key(&cycle_nodes);
                if seen_cycles.insert(key) {
                    issues.push(serde_json::json!({
                        "type": "cycle",
                        "nodes": cycle_nodes,
                        "message": format!("Cycle detected involving {}", node),
                    }));
                }
                return;
            }
            if visited.contains(node) {
                return;
            }

            visited.insert(node.to_string());
            on_stack.insert(node.to_string());
            stack.push(node.to_string());

            if let Some(parents) = parent_map.get(node) {
                for parent in parents {
                    dfs(
                        parent,
                        parent_map,
                        visited,
                        on_stack,
                        stack,
                        issues,
                        seen_cycles,
                    );
                }
            }

            stack.pop();
            on_stack.remove(node);
        }

        for node in self.parent_map.keys() {
            dfs(
                node,
                &self.parent_map,
                &mut visited,
                &mut on_stack,
                &mut stack,
                &mut issues,
                &mut seen_cycles,
            );
        }

        let issue_count = issues.len();
        let orphan_total = orphaned.len();
        let orphan_limit = self.config.search.max_page_size.max(1);
        let orphaned_truncated = orphan_total > orphan_limit;
        if orphaned_truncated {
            orphaned.truncate(orphan_limit);
        }

        let response = if params.detail == ValidateDagDetail::Summary {
            let mut by_type: std::collections::HashMap<String, usize> =
                std::collections::HashMap::new();
            for id in &orphaned {
                let resource_type = id.split('.').next().unwrap_or("unknown");
                *by_type.entry(resource_type.to_string()).or_default() += 1;
            }
            let mut orphaned_by_type: Vec<JsonValue> = by_type
                .into_iter()
                .map(|(resource_type, count)| {
                    serde_json::json!({
                        "resource_type": resource_type,
                        "count": count
                    })
                })
                .collect();
            orphaned_by_type.sort_by(|a, b| {
                let a_count = a.get("count").and_then(JsonValue::as_u64).unwrap_or(0);
                let b_count = b.get("count").and_then(JsonValue::as_u64).unwrap_or(0);
                b_count.cmp(&a_count)
            });
            serde_json::json!({
                "valid": issue_count == 0,
                "issue_count": issue_count,
                "orphaned_total": orphan_total,
                "orphaned_by_type": orphaned_by_type,
            })
        } else {
            serde_json::json!({
                "valid": issue_count == 0,
                "issue_count": issue_count,
                "issues": issues,
                "orphaned": orphaned,
                "orphaned_total": orphan_total,
                "orphaned_truncated": orphaned_truncated,
            })
        };

        let response = SuccessResponse::new(response, issue_count);
        let response = if orphaned_truncated {
            response.with_truncated(true)
        } else {
            response
        };
        Ok(serde_json::to_value(response)?)
    }
}
