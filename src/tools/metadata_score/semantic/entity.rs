use serde_json::Value as JsonValue;

use crate::manifest::entity::{column_nova_meta_json, entity_nova_meta_json};
use crate::manifest::search::ManifestSearch;

use crate::tools::metadata_score::CategoryBreakdown;
use crate::tools::metadata_score::helpers::{
    array_len, array_tier_score, clamp_to_u8, expects_columns, percent_score,
    percent_with_expectation, push_recommendation,
};

impl ManifestSearch {
    #[allow(clippy::unused_self)]
    pub(crate) fn score_semantic(
        &self,
        entity_json: &JsonValue,
        resource_type: Option<&str>,
        include_breakdown: bool,
        include_recommendations: bool,
        recommendations: &mut Vec<JsonValue>,
    ) -> CategoryBreakdown {
        let nova = entity_nova_meta_json(entity_json);
        let columns = entity_json
            .get("columns")
            .and_then(|c| c.as_object())
            .cloned()
            .unwrap_or_default();
        let expects_columns = expects_columns(resource_type);
        let scores = collect_semantic_scores(
            nova.as_deref(),
            &columns,
            expects_columns,
            resource_type,
            include_recommendations,
            recommendations,
        );

        let score = scores.total_score();
        let breakdown = build_semantic_breakdown(&scores, include_breakdown);

        CategoryBreakdown {
            score,
            breakdown,
            summary: JsonValue::Null,
        }
    }
}

struct ScoreCount {
    score: u8,
    count: usize,
}

struct ScoreDetail {
    score: u8,
    breakdown: JsonValue,
}

struct ColumnSemanticSummary {
    score: u8,
    with_semantic: usize,
    total_columns: usize,
    coverage_percent: f64,
}

struct SemanticScores {
    synonyms: ScoreCount,
    domains: ScoreCount,
    use_cases: ScoreCount,
    column_semantics: ColumnSemanticSummary,
    grain: u8,
    measures: ScoreDetail,
    metrics: ScoreDetail,
}

impl SemanticScores {
    fn total_score(&self) -> u8 {
        self.synonyms.score
            + self.domains.score
            + self.use_cases.score
            + self.column_semantics.score
            + self.grain
            + self.measures.score
            + self.metrics.score
    }
}

fn collect_semantic_scores(
    nova: Option<&JsonValue>,
    columns: &serde_json::Map<String, JsonValue>,
    expects_columns: bool,
    resource_type: Option<&str>,
    include_recommendations: bool,
    recommendations: &mut Vec<JsonValue>,
) -> SemanticScores {
    let synonyms = {
        let (score, count) = score_synonyms(nova, include_recommendations, recommendations);
        ScoreCount { score, count }
    };
    let domains = {
        let (score, count) = score_domains(nova, include_recommendations, recommendations);
        ScoreCount { score, count }
    };
    let use_cases = {
        let (score, count) = score_use_cases(nova, include_recommendations, recommendations);
        ScoreCount { score, count }
    };
    let column_semantics = score_column_semantics(
        columns,
        expects_columns,
        include_recommendations,
        recommendations,
    );
    let grain = score_grain(nova, include_recommendations, recommendations);
    let score_indicator_metadata = scores_indicator_metadata(resource_type);
    let measures = {
        let (score, breakdown) = if score_indicator_metadata {
            score_measures(nova, include_recommendations, recommendations)
        } else {
            not_applicable_indicator_score(nova, "measures")
        };
        ScoreDetail { score, breakdown }
    };
    let metrics = {
        let (score, breakdown) = if score_indicator_metadata {
            score_metrics(nova, include_recommendations, recommendations)
        } else {
            not_applicable_indicator_score(nova, "metrics")
        };
        ScoreDetail { score, breakdown }
    };

    SemanticScores {
        synonyms,
        domains,
        use_cases,
        column_semantics,
        grain,
        measures,
        metrics,
    }
}

fn build_semantic_breakdown(scores: &SemanticScores, include_breakdown: bool) -> JsonValue {
    if !include_breakdown {
        return JsonValue::Null;
    }

    serde_json::json!({
        "synonyms": {"score": scores.synonyms.score, "max": 12, "count": scores.synonyms.count},
        "domains": {"score": scores.domains.score, "max": 12, "count": scores.domains.count},
        "use_cases": {"score": scores.use_cases.score, "max": 12, "count": scores.use_cases.count},
        "column_semantics": {
            "score": scores.column_semantics.score,
            "max": 30,
            "columns_with_semantics": scores.column_semantics.with_semantic,
            "columns_total": scores.column_semantics.total_columns,
            "coverage_percent": scores.column_semantics.coverage_percent
        },
        "grain": {"score": scores.grain, "max": 20, "present": scores.grain > 0},
        "measures": scores.measures.breakdown,
        "metrics": scores.metrics.breakdown
    })
}

fn score_synonyms(
    nova: Option<&JsonValue>,
    include_recommendations: bool,
    recommendations: &mut Vec<JsonValue>,
) -> (u8, usize) {
    let count = array_len(nova, "synonyms");
    let score = array_tier_score(count, 12);
    if include_recommendations && score < 12 {
        push_recommendation(
            recommendations,
            "semantic",
            12 - score,
            array_progress_recommendation(
                "meta.nova.synonyms",
                count,
                score,
                12,
                "Add meta.nova.synonyms (e.g. one common search phrase agents should map here).",
                "2-3 agent-search aliases score full credit.",
            ),
            "meta.nova.synonyms",
        );
    }
    (score, count)
}

fn score_domains(
    nova: Option<&JsonValue>,
    include_recommendations: bool,
    recommendations: &mut Vec<JsonValue>,
) -> (u8, usize) {
    let count = array_len(nova, "domains");
    let score = array_tier_score(count, 12);
    if include_recommendations && score < 12 {
        push_recommendation(
            recommendations,
            "semantic",
            12 - score,
            array_progress_recommendation(
                "meta.nova.domains",
                count,
                score,
                12,
                "Add meta.nova.domains (e.g. one primary business domain).",
                "2-3 well-chosen domains score full credit.",
            ),
            "meta.nova.domains",
        );
    }
    (score, count)
}

fn score_use_cases(
    nova: Option<&JsonValue>,
    include_recommendations: bool,
    recommendations: &mut Vec<JsonValue>,
) -> (u8, usize) {
    let count = array_len(nova, "use_cases");
    let score = array_tier_score(count, 12);
    if include_recommendations && score < 12 {
        push_recommendation(
            recommendations,
            "semantic",
            12 - score,
            array_progress_recommendation(
                "meta.nova.use_cases",
                count,
                score,
                12,
                "Add meta.nova.use_cases (e.g. one concrete analyst or agent task).",
                "2-3 concrete use cases score full credit.",
            ),
            "meta.nova.use_cases",
        );
    }
    (score, count)
}

fn score_column_semantics(
    columns: &serde_json::Map<String, JsonValue>,
    expects_columns: bool,
    include_recommendations: bool,
    recommendations: &mut Vec<JsonValue>,
) -> ColumnSemanticSummary {
    let (with_semantic, total_columns) = column_semantic_coverage(columns, expects_columns);
    let coverage_percent = percent_with_expectation(with_semantic, total_columns, expects_columns);
    let score = percent_score(coverage_percent, 30);
    if include_recommendations && score < 30 && expects_columns {
        push_recommendation(
            recommendations,
            "semantic",
            30 - score,
            "Add nova semantic metadata (role/semantic_type) to more columns".to_string(),
            "columns.meta.nova",
        );
    }

    ColumnSemanticSummary {
        score,
        with_semantic,
        total_columns,
        coverage_percent: f64::from(coverage_percent),
    }
}

fn score_grain(
    nova: Option<&JsonValue>,
    include_recommendations: bool,
    recommendations: &mut Vec<JsonValue>,
) -> u8 {
    let score = if grain_present(nova) { 20 } else { 0 };
    if include_recommendations && score == 0 {
        let message = if nova.and_then(|n| n.get("grain")).is_some() {
            "Replace meta.nova.grain with an object that declares primary_key, time_field, or metric grain.".to_string()
        } else {
            "Define meta.nova.grain for this entity or metric.".to_string()
        };
        push_recommendation(recommendations, "semantic", 20, message, "meta.nova.grain");
    }
    score
}

fn score_measures(
    nova: Option<&JsonValue>,
    include_recommendations: bool,
    recommendations: &mut Vec<JsonValue>,
) -> (u8, JsonValue) {
    let score = measures_quality_score(nova, 12);
    if include_recommendations && score.0 < 12 {
        push_recommendation(
            recommendations,
            "semantic",
            12 - score.0,
            indicator_progress_recommendation(
                "meta.nova.measures",
                &score.1,
                "Add meta.nova.measures when this model owns reusable business quantities.",
                true,
            ),
            "meta.nova.measures",
        );
    }
    score
}

fn score_metrics(
    nova: Option<&JsonValue>,
    include_recommendations: bool,
    recommendations: &mut Vec<JsonValue>,
) -> (u8, JsonValue) {
    let score = metrics_quality_score(nova, 12);
    if include_recommendations && score.0 < 12 {
        push_recommendation(
            recommendations,
            "semantic",
            12 - score.0,
            indicator_progress_recommendation(
                "meta.nova.metrics",
                &score.1,
                "Add meta.nova.metrics when this entity owns reusable derived metrics.",
                false,
            ),
            "meta.nova.metrics",
        );
    }
    score
}

fn array_progress_recommendation(
    field_path: &str,
    count: usize,
    score: u8,
    max: u8,
    absent_message: &str,
    full_credit_guidance: &str,
) -> String {
    if count == 0 {
        return absent_message.to_string();
    }
    let entry = if count == 1 { "entry" } else { "entries" };
    format!("{field_path} has {count} {entry} ({score}/{max}); {full_credit_guidance}")
}

fn indicator_progress_recommendation(
    field_path: &str,
    breakdown: &JsonValue,
    absent_message: &str,
    supports_field: bool,
) -> String {
    let count = breakdown
        .get("count")
        .and_then(JsonValue::as_u64)
        .unwrap_or(0);
    if count == 0 {
        return absent_message.to_string();
    }
    let score = breakdown
        .get("score")
        .and_then(JsonValue::as_u64)
        .unwrap_or(0);
    let max = breakdown
        .get("max")
        .and_then(JsonValue::as_u64)
        .unwrap_or(0);
    let has_expression = breakdown
        .get("has_expression")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false);
    let has_synonyms = breakdown
        .get("has_synonyms")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false);
    let entry = if count == 1 { "entry" } else { "entries" };
    let execution_metadata = if supports_field {
        "expressions or fields"
    } else {
        "expressions"
    };
    let guidance = match (has_expression, has_synonyms) {
        (false, false) => format!("add {execution_metadata} plus synonyms to score full credit."),
        (false, true) => format!("add {execution_metadata} to score full credit."),
        (true, false) => "add synonyms to score full credit.".to_string(),
        (true, true) => "review metadata quality to score full credit.".to_string(),
    };
    format!("{field_path} has {count} {entry} ({score}/{max}); {guidance}")
}

fn scores_indicator_metadata(resource_type: Option<&str>) -> bool {
    !matches!(resource_type, Some("source"))
}

fn not_applicable_indicator_score(nova: Option<&JsonValue>, key: &str) -> (u8, JsonValue) {
    let count = match key {
        "measures" => array_len(nova, "measures"),
        "metrics" => metric_count(nova),
        _ => 0,
    };
    (
        0,
        serde_json::json!({
            "score": 0,
            "max": 0,
            "count": count,
            "applicable": false,
            "reason": "not_scored_for_resource_type"
        }),
    )
}

fn metric_count(nova: Option<&JsonValue>) -> usize {
    nova.and_then(|n| n.get("metrics").or_else(|| n.get("metric")))
        .map_or(0, |metrics| {
            if let Some(values) = metrics.as_array() {
                values.len()
            } else {
                usize::from(metrics.is_object())
            }
        })
}

fn column_semantic_coverage(
    columns: &serde_json::Map<String, JsonValue>,
    expects_columns: bool,
) -> (usize, usize) {
    if columns.is_empty() {
        return if expects_columns { (0, 1) } else { (1, 1) };
    }
    let mut with_semantic = 0usize;
    for column in columns.values() {
        let nova = column_nova_meta_json(column);
        if nova.as_deref().and_then(|n| n.get("role")).is_some()
            || nova
                .as_deref()
                .and_then(|n| n.get("semantic_type"))
                .is_some()
        {
            with_semantic += 1;
        }
    }
    (with_semantic, columns.len())
}

fn grain_present(nova: Option<&JsonValue>) -> bool {
    let has_grain = nova
        .and_then(|n| n.get("grain"))
        .and_then(|g| g.as_object())
        .is_some_and(|g| !g.is_empty());
    if has_grain {
        return true;
    }

    let has_metric_grain = nova
        .and_then(|n| n.get("metric"))
        .and_then(|m| m.get("grain"))
        .and_then(|g| g.as_object())
        .is_some_and(|g| !g.is_empty());
    if has_metric_grain {
        return true;
    }

    nova.and_then(|n| n.get("metrics"))
        .and_then(|m| m.as_array())
        .is_some_and(|arr| {
            arr.iter()
                .filter_map(|metric| metric.get("grain"))
                .filter_map(|g| g.as_object())
                .any(|g| !g.is_empty())
        })
}

fn scale_score(base_score: u8, base_max: u8, max_points: u8) -> u8 {
    if base_max == 0 || max_points == 0 {
        return 0;
    }
    clamp_to_u8(
        (f32::from(base_score) / f32::from(base_max)) * f32::from(max_points),
        max_points,
    )
}

fn measures_quality_score(nova: Option<&JsonValue>, max_points: u8) -> (u8, JsonValue) {
    let measures = nova
        .and_then(|n| n.get("measures"))
        .and_then(|v| v.as_array());
    let (count, has_expr, has_syn) = match measures {
        Some(arr) if !arr.is_empty() => (
            arr.len(),
            arr.iter().any(|m| m.get("expression").is_some()),
            arr.iter().any(|m| {
                m.get("synonyms")
                    .and_then(JsonValue::as_array)
                    .is_some_and(|a| !a.is_empty())
            }),
        ),
        _ => (0, false, false),
    };
    let base_score = match (count, has_expr, has_syn) {
        (0, _, _) => 0,
        (_, false, _) => 6,
        (_, true, false) => 10,
        (_, true, true) => 15,
    };
    let score = scale_score(base_score, 15, max_points);
    (
        score,
        serde_json::json!({
            "score": score,
            "max": max_points,
            "count": count,
            "has_expression": has_expr,
            "has_synonyms": has_syn
        }),
    )
}

fn metrics_quality_score(nova: Option<&JsonValue>, max_points: u8) -> (u8, JsonValue) {
    let metrics = nova
        .and_then(|n| n.get("metrics").or_else(|| n.get("metric")))
        .and_then(|v| {
            if v.is_array() {
                v.as_array().cloned()
            } else if v.is_object() {
                Some(vec![v.clone()])
            } else {
                None
            }
        });

    let (count, has_expr, has_syn) = match metrics {
        Some(arr) if !arr.is_empty() => (
            arr.len(),
            arr.iter().any(|m| m.get("expression").is_some()),
            arr.iter().any(|m| {
                m.get("synonyms")
                    .and_then(JsonValue::as_array)
                    .is_some_and(|a| !a.is_empty())
            }),
        ),
        _ => (0, false, false),
    };

    let base_score = match (count, has_expr, has_syn) {
        (0, _, _) => 0,
        (_, false, _) => 5,
        (_, true, false) => 10,
        (_, true, true) => 15,
    };
    let score = scale_score(base_score, 15, max_points);

    (
        score,
        serde_json::json!({
            "score": score,
            "max": max_points,
            "count": count,
            "has_expression": has_expr,
            "has_synonyms": has_syn
        }),
    )
}
