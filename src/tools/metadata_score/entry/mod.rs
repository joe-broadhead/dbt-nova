use serde_json::Value as JsonValue;
use tracing::instrument;

use crate::config::MetadataCategoryWeights;
use crate::error::{DbtNovaError, Result};
use crate::manifest::search::ManifestSearch;
use crate::params::{DEFAULT_ENTITY_SCOPE, DEFAULT_METADATA_SCORE_LIMIT, GetMetadataScoreParams};
use crate::responses::SuccessResponse;
use crate::tools::metadata_score::helpers::{average_score, grade_from_score};

impl ManifestSearch {
    /// Compute metadata scores for an entity, column, or project scope.
    ///
    /// # Errors
    /// Returns an error if inputs are invalid or entities cannot be resolved.
    #[instrument(skip(self, params), fields(tool = "get_metadata_score"))]
    pub async fn get_metadata_score(&self, params: &GetMetadataScoreParams) -> Result<JsonValue> {
        let scope = params
            .scope
            .as_deref()
            .unwrap_or(DEFAULT_ENTITY_SCOPE)
            .trim()
            .to_lowercase();
        let (weights, persona) = self.resolve_metadata_weights(params.persona.as_deref());

        match scope.as_str() {
            "entity" => self.score_entity_scope(params, weights, &persona).await,
            "column" => self.score_column_scope(params, weights, &persona).await,
            "project" => self.score_project_scope(params, weights, &persona).await,
            _ => Err(DbtNovaError::InvalidParams(
                "scope must be entity, column, or project".to_string(),
            )),
        }
    }

    #[allow(clippy::unused_async)]
    async fn score_entity_scope(
        &self,
        params: &GetMetadataScoreParams,
        weights: MetadataCategoryWeights,
        persona: &str,
    ) -> Result<JsonValue> {
        let id = params.id_or_name.as_deref().ok_or_else(|| {
            DbtNovaError::InvalidParams("id_or_name required for entity scope".into())
        })?;
        let unique_id = self.resolve_single_id(id, params.resource_type.as_deref())?;
        let entity = self
            .get_entity_archived(&unique_id)?
            .ok_or_else(|| self.entity_not_found(&unique_id, params.resource_type.as_deref()))?;
        let entity_json = entity.to_json_value();

        let score = self.score_entity(
            &unique_id,
            &entity_json,
            params.include_breakdown,
            params.include_recommendations,
            weights,
        );

        let response = serde_json::json!({
            "unique_id": unique_id,
            "name": entity.name_str(),
            "resource_type": entity.resource_type_str(),
            "scope": "entity",
            "persona": persona,
            "overall_score": score.overall_score,
            "grade": score.grade,
            "categories": score.categories,
            "breakdown": score.breakdown,
            "recommendations": score.recommendations
        });

        Ok(serde_json::to_value(SuccessResponse::new(response, 1))?)
    }

    #[allow(clippy::unused_async)]
    async fn score_project_scope(
        &self,
        params: &GetMetadataScoreParams,
        weights: MetadataCategoryWeights,
        persona: &str,
    ) -> Result<JsonValue> {
        let mut resource_types = params.resource_types.clone();
        if resource_types.is_empty() {
            resource_types = vec!["model".to_string()];
        }
        resource_types.sort_unstable();
        resource_types.dedup();

        let mut entity_scores = Vec::new();
        let mut total_scores = Vec::new();
        let mut count = 0usize;
        let limit = params.limit.unwrap_or(DEFAULT_METADATA_SCORE_LIMIT);
        let offset = params.offset.unwrap_or(0);
        let mut total_available = 0usize;
        let mut coverage = CoverageAggregate::default();
        let mut ordered_entity_ids = Vec::new();

        for resource_type in &resource_types {
            if let Some(entities) = self.by_resource_type.get(resource_type) {
                total_available += entities.len();
                let mut sorted_entities = entities.clone();
                sorted_entities.sort_unstable();
                ordered_entity_ids.extend(sorted_entities);
            }
        }

        for entity_id in ordered_entity_ids.into_iter().skip(offset).take(limit) {
            if let Some(entity) = self.get_entity_archived(&entity_id)? {
                let entity_json = entity.to_json_value();
                let score = self.score_entity(
                    &entity_id,
                    &entity_json,
                    params.include_breakdown,
                    params.include_recommendations,
                    weights,
                );

                total_scores.push(u64::from(score.overall_score));
                coverage.ingest(&score.categories);
                entity_scores.push(serde_json::json!({
                    "unique_id": entity_id,
                    "name": entity.name_str(),
                    "resource_type": entity.resource_type_str(),
                    "overall_score": score.overall_score,
                    "grade": score.grade
                }));
                count += 1;
            }
        }

        let avg_overall = average_score(total_scores.into_iter().map(Some));
        let scanned = offset.saturating_add(count);
        let truncated = scanned < total_available;
        let quality_summary = coverage.build_summary(count, total_available, truncated);
        let response = serde_json::json!({
            "scope": "project",
            "persona": persona,
            "overall_score": avg_overall,
            "grade": grade_from_score(avg_overall),
            "limit": limit,
            "offset": offset,
            "entities": entity_scores,
            "quality_summary": quality_summary
        });

        let mut response = SuccessResponse::new(response, count).with_total(total_available);
        if truncated {
            response = response.with_truncated(true);
        }
        Ok(serde_json::to_value(response)?)
    }

    fn resolve_metadata_weights(&self, persona: Option<&str>) -> (MetadataCategoryWeights, String) {
        let persona = persona.unwrap_or("default").trim().to_lowercase();

        let weights = match persona.as_str() {
            "analyst" => self.config().metadata_score.persona_weights.analyst,
            "engineer" => self.config().metadata_score.persona_weights.engineer,
            "governance" => self.config().metadata_score.persona_weights.governance,
            _ => self.config().metadata_score.persona_weights.default,
        };

        (weights, persona)
    }
}

#[derive(Default)]
struct CoverageAggregate {
    entities_with_tests: u64,
    critical_columns: u64,
    critical_columns_tested: u64,
    dimension_columns: u64,
    dimension_columns_tested: u64,
}

impl CoverageAggregate {
    fn ingest(&mut self, categories: &JsonValue) {
        let summary = categories
            .get("quality")
            .and_then(|q| q.get("summary"))
            .and_then(|s| s.get("test_coverage"));
        let Some(summary) = summary else {
            return;
        };
        let baseline = summary
            .get("baseline")
            .and_then(JsonValue::as_u64)
            .unwrap_or(0);
        if baseline > 0 {
            self.entities_with_tests += 1;
        }
        self.critical_columns += summary
            .get("critical_columns")
            .and_then(JsonValue::as_u64)
            .unwrap_or(0);
        self.critical_columns_tested += summary
            .get("critical_columns_tested")
            .and_then(JsonValue::as_u64)
            .unwrap_or(0);
        self.dimension_columns += summary
            .get("dimension_columns")
            .and_then(JsonValue::as_u64)
            .unwrap_or(0);
        self.dimension_columns_tested += summary
            .get("dimension_columns_tested")
            .and_then(JsonValue::as_u64)
            .unwrap_or(0);
    }

    fn build_summary(&self, entities: usize, total: usize, truncated: bool) -> JsonValue {
        let critical_percent = percent_or_zero(self.critical_columns_tested, self.critical_columns);
        let dimension_percent =
            percent_or_zero(self.dimension_columns_tested, self.dimension_columns);

        serde_json::json!({
            "scope": if truncated { "included_entities" } else { "all_entities" },
            "entities": entities,
            "entities_total": total,
            "entities_with_tests": self.entities_with_tests,
            "test_coverage": {
                "critical_columns": self.critical_columns,
                "critical_columns_tested": self.critical_columns_tested,
                "critical_coverage_percent": critical_percent,
                "dimension_columns": self.dimension_columns,
                "dimension_columns_tested": self.dimension_columns_tested,
                "dimension_coverage_percent": dimension_percent
            }
        })
    }
}

#[allow(clippy::cast_precision_loss)]
fn percent_or_zero(numerator: u64, denominator: u64) -> u8 {
    if denominator == 0 {
        return 0;
    }
    crate::tools::metadata_score::helpers::clamp_to_u8(
        (numerator as f32 / denominator as f32) * 100.0,
        100,
    )
}
