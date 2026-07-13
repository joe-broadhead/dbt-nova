use serde_json::Value as JsonValue;

use crate::config::MetadataCategoryWeights;
use crate::error::{DbtNovaError, Result};
use crate::manifest::search::ManifestSearch;
use crate::params::GetMetadataScoreParams;
use crate::responses::SuccessResponse;

use crate::tools::metadata_score::helpers::{average_score, grade_from_score};
use crate::tools::metadata_score::metadata_score_scoring_contract;

impl ManifestSearch {
    #[allow(clippy::unused_async)]
    pub(crate) async fn score_column_scope(
        &self,
        params: &GetMetadataScoreParams,
        weights: MetadataCategoryWeights,
        persona: &str,
    ) -> Result<JsonValue> {
        let id = params.id_or_name.as_deref().ok_or_else(|| {
            DbtNovaError::InvalidParams("id_or_name required for column scope".into())
        })?;
        let unique_id = self.resolve_single_id(id, params.resource_type.as_deref())?;
        let entity = self
            .get_entity_archived(&unique_id)?
            .ok_or_else(|| self.entity_not_found(&unique_id, params.resource_type.as_deref()))?;
        let entity_json = entity.to_json_value();
        let columns = entity_json
            .get("columns")
            .and_then(|c| c.as_object())
            .cloned()
            .unwrap_or_default();

        let mut scored_columns = Vec::with_capacity(columns.len());
        for (name, info) in columns {
            let column_score = self.score_column(
                &unique_id,
                &name,
                &info,
                params.include_breakdown,
                params.include_recommendations,
                weights,
            );
            scored_columns.push(column_score);
        }

        let avg_overall = average_score(scored_columns.iter().map(|c| c["overall_score"].as_u64()));
        let column_count = scored_columns.len();
        let response = serde_json::json!({
            "unique_id": unique_id,
            "name": entity.name_str(),
            "resource_type": entity.resource_type_str(),
            "scope": "column",
            "persona": persona,
            "overall_score": avg_overall,
            "grade": grade_from_score(avg_overall),
            "scoring_contract": metadata_score_scoring_contract(
                &self.config().metadata_score.scoring_contract_version
            ),
            "columns": scored_columns
        });

        Ok(serde_json::to_value(SuccessResponse::new(
            response,
            column_count,
        ))?)
    }
}
