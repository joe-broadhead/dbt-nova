use serde_json::Value as JsonValue;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::manifest::entity::ArchivedEntity;

use super::core::ManifestSearch;

const SOURCE_FRESHNESS_PATHS: &[&str] = &[
    "max_loaded_at",
    "snapshotted_at",
    "freshness.max_loaded_at",
    "freshness.snapshotted_at",
];
const MANIFEST_FRESHNESS_PATHS: &[&str] = &["generated_at"];
const SECONDS_PER_DAY: u64 = 86_400;
const SECONDS_PER_DAY_I64: i64 = 86_400;

impl ManifestSearch {
    #[must_use]
    pub(crate) fn provenance_for_archived(
        &self,
        unique_id: &str,
        entity: &ArchivedEntity,
    ) -> JsonValue {
        let entity_json = entity.to_json_value();
        self.provenance_for_archived_json(unique_id, entity, &entity_json)
    }

    #[must_use]
    pub(crate) fn provenance_for_archived_json(
        &self,
        unique_id: &str,
        entity: &ArchivedEntity,
        entity_json: &JsonValue,
    ) -> JsonValue {
        let owner = owner_value(entity_json).cloned();
        let has_owner = owner.as_ref().is_some_and(value_present);
        let has_description = entity
            .description_str()
            .map(str::trim)
            .is_some_and(|value| !value.is_empty());
        let doc_coverage_pct = doc_coverage_pct(entity);
        let tests_total = self.tests_total(unique_id, entity);
        let has_nova_meta = entity.nova_meta().is_some();
        let source_tier = source_tier(
            entity,
            has_owner,
            has_description,
            doc_coverage_pct,
            tests_total,
        );
        let weights = self.config.metadata_score.persona_weights.governance;
        let score = self.score_entity(unique_id, entity_json, false, false, weights);

        let mut readiness = serde_json::Map::new();
        readiness.insert(
            "metadata_score".to_string(),
            JsonValue::from(score.overall_score),
        );
        readiness.insert("metadata_grade".to_string(), JsonValue::String(score.grade));
        readiness.insert(
            "doc_coverage_pct".to_string(),
            JsonValue::from(doc_coverage_pct),
        );
        readiness.insert("has_owner".to_string(), JsonValue::from(has_owner));
        readiness.insert("has_nova_meta".to_string(), JsonValue::from(has_nova_meta));
        readiness.insert(
            "tests_total".to_string(),
            JsonValue::from(tests_total as u64),
        );

        let mut obj = serde_json::Map::new();
        obj.insert(
            "tier".to_string(),
            JsonValue::String(source_tier.to_string()),
        );
        if let Some(owner) = owner.filter(value_present) {
            obj.insert("owner".to_string(), owner);
        }
        obj.insert("readiness".to_string(), JsonValue::Object(readiness));
        obj.insert(
            "freshness".to_string(),
            self.freshness_provenance(entity_json),
        );

        compact_json_object(&mut obj);
        JsonValue::Object(obj)
    }

    fn tests_total(&self, unique_id: &str, entity: &ArchivedEntity) -> usize {
        let model_tests = self.tests_by_entity.get(unique_id).map_or(0, Vec::len);
        let column_tests = entity
            .column_names_iter()
            .map(|column| {
                let key = format!("{unique_id}:{column}");
                self.tests_by_column.get(&key).map_or(0, Vec::len)
            })
            .sum::<usize>();
        model_tests + column_tests
    }

    fn freshness_provenance(&self, entity_json: &JsonValue) -> JsonValue {
        if let Some(timestamp) = timestamp_from_paths(entity_json, SOURCE_FRESHNESS_PATHS) {
            return freshness_from_timestamp(
                "source_freshness",
                timestamp,
                self.config.provenance_stale_after_days,
            );
        }

        if let Some(timestamp) =
            timestamp_from_paths(&self.manifest_metadata, MANIFEST_FRESHNESS_PATHS)
        {
            return freshness_from_timestamp(
                "manifest_generated_at",
                timestamp,
                self.config.provenance_stale_after_days,
            );
        }

        serde_json::json!({
            "status": "unknown",
            "reason": "no_freshness_timestamp"
        })
    }
}

fn owner_value(entity_json: &JsonValue) -> Option<&JsonValue> {
    entity_json
        .get("meta")
        .and_then(|meta| meta.get("owner"))
        .filter(|value| value_present(value))
        .or_else(|| {
            entity_json
                .get("config")
                .and_then(|config| config.get("meta"))
                .and_then(|meta| meta.get("owner"))
                .filter(|value| value_present(value))
        })
        .or_else(|| {
            entity_json
                .get("owner")
                .filter(|value| value_present(value))
        })
}

fn source_tier(
    entity: &ArchivedEntity,
    has_owner: bool,
    has_description: bool,
    doc_coverage_pct: f64,
    tests_total: usize,
) -> &'static str {
    if has_canonical_indicator(entity) {
        return "semantic_layer";
    }
    if has_owner
        || has_description
        || entity.doc_blocks_present()
        || entity.nova_meta().is_some()
        || doc_coverage_pct > 0.0
        || tests_total > 0
    {
        return "curated";
    }
    "raw"
}

fn has_canonical_indicator(entity: &ArchivedEntity) -> bool {
    let Some(nova) = entity.nova_meta() else {
        return false;
    };
    let has_measure = !nova.measures.is_empty();
    let has_metric = nova.metric.is_some() || !nova.metrics.is_empty();
    if nova.canonical && (has_measure || has_metric) {
        return true;
    }
    nova.measures.iter().any(|measure| measure.canonical)
        || nova.metric.as_ref().is_some_and(|metric| metric.canonical)
        || nova.metrics.iter().any(|metric| metric.canonical)
}

fn doc_coverage_pct(entity: &ArchivedEntity) -> f64 {
    let columns_total = entity.column_names_iter().count();
    if columns_total == 0 {
        return 0.0;
    }
    #[allow(clippy::cast_precision_loss)]
    let documented = entity.columns_documented_count() as f64;
    #[allow(clippy::cast_precision_loss)]
    let total = columns_total as f64;
    ((documented / total) * 10_000.0).round() / 100.0
}

fn timestamp_from_paths<'a>(root: &'a JsonValue, paths: &[&str]) -> Option<&'a str> {
    paths.iter().find_map(|path| string_at_path(root, path))
}

fn string_at_path<'a>(root: &'a JsonValue, path: &str) -> Option<&'a str> {
    let mut current = root;
    for part in path.split('.') {
        current = current.get(part)?;
    }
    current
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn freshness_from_timestamp(source: &str, timestamp: &str, stale_after_days: u64) -> JsonValue {
    let Some(observed_at_secs) = parse_rfc3339_timestamp_secs(timestamp) else {
        return serde_json::json!({
            "status": "unknown",
            "source": source,
            "timestamp": timestamp,
            "reason": "unparseable_timestamp"
        });
    };
    let now_secs = now_utc_secs();
    let age_secs = now_secs.saturating_sub(observed_at_secs);
    let stale_after_secs = stale_after_days.saturating_mul(SECONDS_PER_DAY);
    let status = if age_secs > stale_after_secs {
        "stale"
    } else {
        "fresh"
    };

    serde_json::json!({
        "status": status,
        "source": source,
        "timestamp": timestamp,
        "age_days": age_secs / SECONDS_PER_DAY,
        "stale_after_days": stale_after_days
    })
}

fn now_utc_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

fn parse_rfc3339_timestamp_secs(value: &str) -> Option<u64> {
    let value = value.trim();
    if value.len() < 20 {
        return None;
    }
    let year = value.get(0..4)?.parse::<i32>().ok()?;
    let month = value.get(5..7)?.parse::<u32>().ok()?;
    let day = value.get(8..10)?.parse::<u32>().ok()?;
    let hour = value.get(11..13)?.parse::<u32>().ok()?;
    let minute = value.get(14..16)?.parse::<u32>().ok()?;
    let second = value.get(17..19)?.parse::<u32>().ok()?;
    if value.get(4..5)? != "-"
        || value.get(7..8)? != "-"
        || !matches!(value.get(10..11)?, "T" | " ")
        || value.get(13..14)? != ":"
        || value.get(16..17)? != ":"
        || !valid_ymdhms(year, month, day, hour, minute, second)
    {
        return None;
    }

    let offset = parse_offset_seconds(value.get(19..)?)?;
    let days = days_from_civil(year, month, day);
    let local_secs = days
        .checked_mul(SECONDS_PER_DAY_I64)?
        .checked_add(i64::from(hour * 3600 + minute * 60 + second))?;
    let utc_secs = local_secs.checked_sub(offset)?;
    u64::try_from(utc_secs).ok()
}

fn parse_offset_seconds(value: &str) -> Option<i64> {
    let trimmed = value.trim();
    let zone = if let Some(rest) = trimmed.strip_prefix('.') {
        let zone_start = rest.find(['Z', '+', '-'])?;
        &rest[zone_start..]
    } else {
        trimmed
    };
    if zone == "Z" {
        return Some(0);
    }

    let sign = match zone.get(0..1)? {
        "+" => 1_i64,
        "-" => -1_i64,
        _ => return None,
    };
    let hours = zone.get(1..3)?.parse::<i64>().ok()?;
    let minutes = zone.get(4..6)?.parse::<i64>().ok()?;
    if zone.get(3..4)? != ":" || hours > 23 || minutes > 59 {
        return None;
    }
    Some(sign * (hours * 3600 + minutes * 60))
}

fn valid_ymdhms(year: i32, month: u32, day: u32, hour: u32, minute: u32, second: u32) -> bool {
    (1..=12).contains(&month)
        && (1..=days_in_month(year, month)).contains(&day)
        && hour < 24
        && minute < 60
        && second < 60
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let month = i32::try_from(month).unwrap_or_default();
    let day = i32::try_from(day).unwrap_or_default();
    let year = year - i32::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let shifted_month = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * shifted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    i64::from(era) * 146_097 + i64::from(day_of_era) - 719_468
}

fn compact_json_value(value: &mut JsonValue) {
    match value {
        JsonValue::Object(obj) => compact_json_object(obj),
        JsonValue::Array(arr) => {
            for item in arr.iter_mut() {
                compact_json_value(item);
            }
            arr.retain(value_present);
        }
        _ => {}
    }
}

fn compact_json_object(obj: &mut serde_json::Map<String, JsonValue>) {
    for value in obj.values_mut() {
        compact_json_value(value);
    }
    obj.retain(|_, value| value_present(value));
}

fn value_present(value: &JsonValue) -> bool {
    match value {
        JsonValue::Null => false,
        JsonValue::Bool(_) | JsonValue::Number(_) => true,
        JsonValue::String(value) => !value.trim().is_empty(),
        JsonValue::Array(values) => !values.is_empty(),
        JsonValue::Object(values) => !values.is_empty(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_provenance_timestamp_parser_accepts_utc_and_offsets() {
        let utc = parse_rfc3339_timestamp_secs("1970-01-01T00:00:00Z").unwrap();
        let offset = parse_rfc3339_timestamp_secs("1970-01-01T01:30:00+01:30").unwrap();
        let fractional = parse_rfc3339_timestamp_secs("1970-01-01T00:00:00.000Z").unwrap();

        assert_eq!(utc, 0);
        assert_eq!(offset, 0);
        assert_eq!(fractional, 0);
    }

    #[test]
    fn search_provenance_timestamp_parser_rejects_invalid_dates() {
        assert!(parse_rfc3339_timestamp_secs("2025-02-29T00:00:00Z").is_none());
        assert!(parse_rfc3339_timestamp_secs("2025-01-01T24:00:00Z").is_none());
        assert!(parse_rfc3339_timestamp_secs("not-a-timestamp").is_none());
    }

    #[test]
    fn search_provenance_owner_falls_back_when_legacy_owner_blank() {
        let entity = serde_json::json!({
            "meta": {
                "owner": " "
            },
            "config": {
                "meta": {
                    "owner": "analytics"
                }
            }
        });

        assert_eq!(
            owner_value(&entity).and_then(JsonValue::as_str),
            Some("analytics")
        );
    }
}
