use serde_json::Value as JsonValue;

use crate::manifest::entity::entity_meta_field_json;

#[must_use]
#[allow(clippy::cast_precision_loss)]
pub(crate) fn average_score<I>(scores: I) -> u8
where
    I: Iterator<Item = Option<u64>>,
{
    let mut total = 0u64;
    let mut count = 0u64;
    for score in scores.flatten() {
        total += score;
        count += 1;
    }
    if count == 0 {
        return 0;
    }
    clamp_to_u8((total as f32) / (count as f32), 100)
}

/// Convert a numeric score (0-100) into a letter grade.
///
/// Grades:
/// - A: 90-100
/// - B: 80-89
/// - C: 70-79
/// - D: 60-69
/// - F: 0-59
#[must_use]
pub fn grade_from_score(score: u8) -> &'static str {
    match score {
        90..=100 => "A",
        80..=89 => "B",
        70..=79 => "C",
        60..=69 => "D",
        _ => "F",
    }
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]
pub(crate) fn clamp_to_u8(value: f32, max: u8) -> u8 {
    value.clamp(0.0, f32::from(max)).round() as u8
}

#[must_use]
pub fn description_tier_score(text: &str, max_points: u8) -> u8 {
    let len = text.trim().len();
    let tier = match len {
        0 => 0.0,
        1..=19 => 0.2,
        20..=49 => 0.5,
        50..=99 => 0.8,
        _ => 1.0,
    };
    clamp_to_u8(tier * f32::from(max_points), max_points)
}

#[must_use]
pub fn array_tier_score(count: usize, max_points: u8) -> u8 {
    let tier = match count {
        0 => 0.0,
        1 => 0.4,
        2 => 0.7,
        _ => 1.0,
    };
    clamp_to_u8(tier * f32::from(max_points), max_points)
}

#[must_use]
#[allow(clippy::cast_precision_loss)]
pub(crate) fn percent_with_expectation(part: usize, total: usize, expects_columns: bool) -> u8 {
    if total == 0 {
        if expects_columns { 0 } else { 100 }
    } else {
        clamp_to_u8((part as f32 / total as f32) * 100.0, 100)
    }
}

#[must_use]
pub(crate) fn percent_score(percent: u8, max_points: u8) -> u8 {
    clamp_to_u8(
        (f32::from(percent) / 100.0) * f32::from(max_points),
        max_points,
    )
}

#[must_use]
pub(crate) fn expects_columns(resource_type: Option<&str>) -> bool {
    matches!(
        resource_type.unwrap_or(""),
        "model" | "source" | "seed" | "snapshot"
    )
}

#[must_use]
pub(crate) fn array_len(nova: Option<&JsonValue>, key: &str) -> usize {
    nova.and_then(|n| n.get(key))
        .and_then(JsonValue::as_array)
        .map_or(0, Vec::len)
}

#[must_use]
pub(crate) fn has_owner(entity_json: &JsonValue) -> bool {
    let meta_owner = entity_meta_field_json(entity_json, "owner");

    if let Some(owner) = meta_owner {
        match owner {
            JsonValue::String(s) => return !s.trim().is_empty(),
            JsonValue::Object(map) => return !map.is_empty(),
            JsonValue::Array(arr) => return !arr.is_empty(),
            JsonValue::Bool(true) => return true,
            value => return !value.is_null(),
        }
    }

    match entity_json.get("owner") {
        Some(JsonValue::String(s)) => !s.trim().is_empty(),
        Some(JsonValue::Object(map)) => !map.is_empty(),
        Some(JsonValue::Array(arr)) => !arr.is_empty(),
        Some(JsonValue::Bool(true)) => true,
        Some(value) => !value.is_null(),
        None => false,
    }
}

#[allow(clippy::needless_pass_by_value)]
pub(crate) fn push_recommendation(
    recommendations: &mut Vec<JsonValue>,
    category: &str,
    impact: u8,
    message: String,
    field: &str,
) {
    let priority = if impact >= 15 {
        "high"
    } else if impact >= 8 {
        "medium"
    } else {
        "low"
    };
    recommendations.push(serde_json::json!({
        "category": category,
        "priority": priority,
        "impact": impact,
        "message": message,
        "field": field
    }));
}
