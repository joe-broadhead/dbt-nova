use std::collections::HashSet;

use serde_json::Value as JsonValue;

use crate::error::Result;
use crate::manifest::search::ManifestSearch;
use crate::params::DiffEntitiesParams;
use crate::responses::SuccessResponse;
use tracing::instrument;

impl ManifestSearch {
    /// Diff two entities across selected fields.
    ///
    /// # Errors
    /// Returns an error if either entity cannot be resolved.
    #[instrument(skip(self, params), fields(tool = "diff_entities", entity1 = %params.entity1, entity2 = %params.entity2))]
    #[allow(clippy::too_many_lines)]
    pub async fn diff_entities(&self, params: &DiffEntitiesParams) -> Result<JsonValue> {
        let resource_type1 = params.entity1_resource_type.as_deref();
        let resource_type2 = params.entity2_resource_type.as_deref();
        let key1 = self.resolve_single_id(&params.entity1, resource_type1)?;
        let key2 = self.resolve_single_id(&params.entity2, resource_type2)?;
        let entity1 = self
            .get_entity(&key1)
            .await?
            .ok_or_else(|| self.entity_not_found(&key1, resource_type1))?;
        let entity2 = self
            .get_entity(&key2)
            .await?
            .ok_or_else(|| self.entity_not_found(&key2, resource_type2))?;

        let entity1_json = entity1.to_json_value();
        let entity2_json = entity2.to_json_value();
        let mut diffs = serde_json::Map::new();

        let mut summary = serde_json::Map::new();
        let compare_fields = if params.compare_fields.is_empty() {
            vec!["columns".to_string()]
        } else {
            params.compare_fields.clone()
        };
        for field in &compare_fields {
            match field.as_str() {
                "columns" => {
                    let cols1: HashSet<String> = entity1.column_names().into_iter().collect();
                    let cols2: HashSet<String> = entity2.column_names().into_iter().collect();
                    let only_in_first: Vec<_> = cols1.difference(&cols2).collect();
                    let only_in_second: Vec<_> = cols2.difference(&cols1).collect();
                    let in_both: Vec<_> = cols1.intersection(&cols2).collect();
                    let counts = (only_in_first.len(), only_in_second.len(), in_both.len());

                    diffs.insert(
                        "columns".to_string(),
                        serde_json::json!({
                            "only_in_first": only_in_first,
                            "only_in_second": only_in_second,
                            "in_both": in_both,
                            "counts": {
                                "only_in_first": counts.0,
                                "only_in_second": counts.1,
                                "in_both": counts.2
                            }
                        }),
                    );
                    summary.insert(
                        "columns".to_string(),
                        serde_json::json!({
                            "only_in_first": counts.0,
                            "only_in_second": counts.1,
                            "in_both": counts.2
                        }),
                    );
                }
                "tags" => {
                    let tags1: HashSet<String> = entity1.tags.iter().cloned().collect();
                    let tags2: HashSet<String> = entity2.tags.iter().cloned().collect();
                    let only_in_first: Vec<_> = tags1.difference(&tags2).collect();
                    let only_in_second: Vec<_> = tags2.difference(&tags1).collect();
                    let in_both: Vec<_> = tags1.intersection(&tags2).collect();
                    let counts = (only_in_first.len(), only_in_second.len(), in_both.len());

                    diffs.insert(
                        "tags".to_string(),
                        serde_json::json!({
                            "only_in_first": only_in_first,
                            "only_in_second": only_in_second,
                            "in_both": in_both,
                            "counts": {
                                "only_in_first": counts.0,
                                "only_in_second": counts.1,
                                "in_both": counts.2
                            }
                        }),
                    );
                    summary.insert(
                        "tags".to_string(),
                        serde_json::json!({
                            "only_in_first": counts.0,
                            "only_in_second": counts.1,
                            "in_both": counts.2
                        }),
                    );
                }
                _ => {
                    let val1 = entity1_json.get(field);
                    let val2 = entity2_json.get(field);
                    diffs.insert(
                        field.clone(),
                        serde_json::json!({
                            "first": val1,
                            "second": val2,
                            "equal": val1 == val2
                        }),
                    );
                }
            }
        }

        Ok(serde_json::to_value(SuccessResponse::new(
            serde_json::json!({
                "entity1": { "unique_id": &key1, "name": entity1.name },
                "entity2": { "unique_id": &key2, "name": entity2.name },
                "summary": summary,
                "differences": diffs
            }),
            1,
        ))?)
    }
}
