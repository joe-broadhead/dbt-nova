use std::collections::BTreeMap;

use serde_json::Value as JsonValue;
use tracing::instrument;

use crate::config::MetadataCategoryWeights;
use crate::error::{DbtNovaError, Result};
use crate::manifest::search::ManifestSearch;
use crate::params::{DEFAULT_ENTITY_SCOPE, DEFAULT_METADATA_SCORE_LIMIT, GetMetadataScoreParams};
use crate::responses::SuccessResponse;
use crate::tools::metadata_score::helpers::{average_score, grade_from_score};
use crate::tools::metadata_score::metadata_score_scoring_contract;

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
            "diagnostics": score.diagnostics,
            "scoring_contract": metadata_score_scoring_contract(),
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
        let limit = params.limit.unwrap_or(DEFAULT_METADATA_SCORE_LIMIT);
        let offset = params.offset.unwrap_or(0);
        let mut coverage = CoverageAggregate::default();
        let mut project_summary = ProjectScoreSummaryBuilder::default();
        let mut ordered_entity_ids = Vec::new();

        for resource_type in &resource_types {
            if let Some(entities) = self.by_resource_type.get(resource_type) {
                let mut sorted_entities = entities.clone();
                sorted_entities.sort_unstable();
                ordered_entity_ids.extend(sorted_entities);
            }
        }

        let total_available = ordered_entity_ids.len();
        let mut scored_count = 0usize;

        for (entity_index, entity_id) in ordered_entity_ids.into_iter().enumerate() {
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
                project_summary.ingest(ProjectScoreInput {
                    unique_id: &entity_id,
                    name: entity.name_str(),
                    resource_type: entity.resource_type_str(),
                    overall_score: score.overall_score,
                    grade: &score.grade,
                    categories: &score.categories,
                    recommendations: &score.recommendations,
                });
                if entity_index >= offset && entity_scores.len() < limit {
                    entity_scores.push(serde_json::json!({
                        "unique_id": entity_id,
                        "name": entity.name_str(),
                        "resource_type": entity.resource_type_str(),
                        "overall_score": score.overall_score,
                        "grade": score.grade
                    }));
                }
                scored_count += 1;
            }
        }

        let avg_overall = average_score(total_scores.into_iter().map(Some));
        let count = entity_scores.len();
        let scanned = offset.saturating_add(count);
        let page_truncated = scanned < total_available;
        let quality_summary = coverage.build_summary(scored_count, total_available, false);
        let response = serde_json::json!({
            "scope": "project",
            "persona": persona,
            "overall_score": avg_overall,
            "grade": grade_from_score(avg_overall),
            "limit": limit,
            "offset": offset,
            "entities": entity_scores,
            "quality_summary": quality_summary,
            "summary": project_summary.build(ProjectSummaryContext {
                persona,
                entities: scored_count,
                total_available,
                limit,
                offset,
                page_truncated,
                next_offset: page_truncated.then_some(scanned),
            }),
            "scoring_contract": metadata_score_scoring_contract()
        });

        let mut response = SuccessResponse::new(response, count).with_total(total_available);
        if page_truncated {
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

#[derive(Clone, Copy)]
struct ProjectScoreInput<'a> {
    unique_id: &'a str,
    name: Option<&'a str>,
    resource_type: Option<&'a str>,
    overall_score: u8,
    grade: &'a str,
    categories: &'a JsonValue,
    recommendations: &'a [JsonValue],
}

#[derive(Default)]
struct ProjectScoreSummaryBuilder {
    score_buckets: BTreeMap<String, usize>,
    grade_buckets: BTreeMap<String, usize>,
    worst_entities: Vec<ScoredEntitySummary>,
    categories: BTreeMap<String, CategoryAggregate>,
    recommendations: BTreeMap<String, RecommendationAggregate>,
}

impl ProjectScoreSummaryBuilder {
    fn ingest(&mut self, input: ProjectScoreInput<'_>) {
        *self
            .score_buckets
            .entry(score_bucket(input.overall_score).to_string())
            .or_default() += 1;
        *self
            .grade_buckets
            .entry(input.grade.to_string())
            .or_default() += 1;
        self.worst_entities.push(ScoredEntitySummary {
            unique_id: input.unique_id.to_string(),
            name: input.name.map(ToString::to_string),
            resource_type: input.resource_type.map(ToString::to_string),
            overall_score: input.overall_score,
            grade: input.grade.to_string(),
        });
        ingest_category_scores(&mut self.categories, input.categories);
        ingest_recommendations(&mut self.recommendations, input.recommendations);
    }

    fn build(mut self, context: ProjectSummaryContext<'_>) -> JsonValue {
        self.worst_entities.sort_by(|left, right| {
            left.overall_score
                .cmp(&right.overall_score)
                .then_with(|| left.unique_id.cmp(&right.unique_id))
        });
        self.worst_entities.truncate(10);

        let mut category_weak_spots = self
            .categories
            .into_iter()
            .filter(|(_, aggregate)| aggregate.count > 0)
            .map(|(category, aggregate)| {
                let average = aggregate.average_score();
                serde_json::json!({
                    "category": category,
                    "average_score": average,
                    "entity_count": aggregate.count,
                    "estimated_point_gap": aggregate.point_gap,
                    "estimated_weighted_point_gap": aggregate.weighted_point_gap
                })
            })
            .collect::<Vec<_>>();
        category_weak_spots.sort_by(|left, right| {
            json_f64(right, "estimated_weighted_point_gap")
                .partial_cmp(&json_f64(left, "estimated_weighted_point_gap"))
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let mut recommendation_fields = self
            .recommendations
            .into_iter()
            .map(|(field, aggregate)| {
                serde_json::json!({
                    "field": field,
                    "category": aggregate.category,
                    "count": aggregate.count,
                    "estimated_point_impact": aggregate.total_impact
                })
            })
            .collect::<Vec<_>>();
        recommendation_fields.sort_by(|left, right| {
            right
                .get("count")
                .and_then(JsonValue::as_u64)
                .cmp(&left.get("count").and_then(JsonValue::as_u64))
                .then_with(|| {
                    right
                        .get("estimated_point_impact")
                        .and_then(JsonValue::as_u64)
                        .cmp(
                            &left
                                .get("estimated_point_impact")
                                .and_then(JsonValue::as_u64),
                        )
                })
                .then_with(|| {
                    left.get("field")
                        .and_then(JsonValue::as_str)
                        .cmp(&right.get("field").and_then(JsonValue::as_str))
                })
        });
        recommendation_fields.truncate(10);

        let drill_down_hints = self
            .worst_entities
            .iter()
            .take(5)
            .map(|entity| {
                serde_json::json!({
                    "purpose": "inspect_low_scoring_entity",
                    "tool": "get_metadata_score",
                    "arguments": {
                        "scope": "entity",
                        "id_or_name": entity.unique_id,
                        "resource_type": entity.resource_type,
                        "persona": context.persona,
                        "include_breakdown": true,
                        "include_recommendations": true
                    }
                })
            })
            .collect::<Vec<_>>();

        serde_json::json!({
            "scope": "all_entities",
            "entities": context.entities,
            "entities_total": context.total_available,
            "truncated": false,
            "sample_truncated": context.page_truncated,
            "page": {
                "limit": context.limit,
                "offset": context.offset,
                "next_offset": context.next_offset
            },
            "score_buckets": self.score_buckets,
            "grade_buckets": self.grade_buckets,
            "worst_entities": self.worst_entities,
            "category_weak_spots": category_weak_spots,
            "top_recommendation_fields": recommendation_fields,
            "drill_down_hints": drill_down_hints
        })
    }
}

#[derive(Clone, Copy)]
struct ProjectSummaryContext<'a> {
    persona: &'a str,
    entities: usize,
    total_available: usize,
    limit: usize,
    offset: usize,
    page_truncated: bool,
    next_offset: Option<usize>,
}

#[derive(serde::Serialize)]
struct ScoredEntitySummary {
    unique_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    resource_type: Option<String>,
    overall_score: u8,
    grade: String,
}

#[derive(Default)]
struct CategoryAggregate {
    total_score: u64,
    count: u64,
    point_gap: u64,
    weighted_point_gap: f64,
}

impl CategoryAggregate {
    fn average_score(&self) -> u8 {
        if self.count == 0 {
            return 0;
        }
        average_score(std::iter::once(Some(self.total_score / self.count)))
    }
}

#[derive(Default)]
struct RecommendationAggregate {
    category: Option<String>,
    count: usize,
    total_impact: u64,
}

fn score_bucket(score: u8) -> &'static str {
    match score {
        90..=100 => "90-100",
        80..=89 => "80-89",
        70..=79 => "70-79",
        60..=69 => "60-69",
        _ => "0-59",
    }
}

fn ingest_category_scores(categories: &mut BTreeMap<String, CategoryAggregate>, value: &JsonValue) {
    let Some(map) = value.as_object() else {
        return;
    };
    for (category, details) in map {
        let Some(score) = details.get("score").and_then(JsonValue::as_u64) else {
            continue;
        };
        let weight = details
            .get("weight")
            .and_then(JsonValue::as_f64)
            .unwrap_or(0.0);
        let entry = categories.entry(category.clone()).or_default();
        entry.total_score += score;
        entry.count += 1;
        let score_gap = 100_u64.saturating_sub(score);
        entry.point_gap += score_gap;
        entry.weighted_point_gap += f64::from(u32::try_from(score_gap).unwrap_or(100)) * weight;
    }
}

fn ingest_recommendations(
    recommendations: &mut BTreeMap<String, RecommendationAggregate>,
    values: &[JsonValue],
) {
    for recommendation in values {
        let field = recommendation
            .get("field")
            .and_then(JsonValue::as_str)
            .unwrap_or("metadata")
            .to_string();
        let entry = recommendations.entry(field).or_default();
        entry.count += 1;
        entry.total_impact += recommendation
            .get("impact")
            .and_then(JsonValue::as_u64)
            .unwrap_or(0);
        if entry.category.is_none() {
            entry.category = recommendation
                .get("category")
                .and_then(JsonValue::as_str)
                .map(ToString::to_string);
        }
    }
}

fn json_f64(value: &JsonValue, field: &str) -> f64 {
    value.get(field).and_then(JsonValue::as_f64).unwrap_or(0.0)
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
