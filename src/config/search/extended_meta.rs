use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

use crate::error::{DbtNovaError, Result};

const EXTENDED_META_DEFAULT_MAX_FIELDS: usize = 32;
const EXTENDED_META_HARD_MAX_FIELDS: usize = 128;
const EXTENDED_META_DEFAULT_MAX_VALUES_PER_FIELD: usize = 64;
const EXTENDED_META_HARD_MAX_VALUES_PER_FIELD: usize = 1024;
const EXTENDED_META_DEFAULT_MAX_BYTES_PER_VALUE: usize = 4096;
const EXTENDED_META_HARD_MAX_BYTES_PER_VALUE: usize = 65_536;

/// Supported value treatment for allowlisted non-Nova dbt metadata fields.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ExtendedMetaFieldMode {
    /// Index scalar metadata as exact/filterable keywords.
    #[default]
    Keyword,
    /// Index scalar metadata as full text.
    Text,
    /// Index string arrays as repeated keyword values.
    StringArray,
    /// Index boolean metadata values.
    Bool,
}

/// One allowlisted non-Nova dbt metadata path for extended-meta search.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct ExtendedMetaFieldConfig {
    /// Logical dbt metadata path, such as `meta.owner` or `columns.*.meta.semantic_group`.
    pub path: String,
    /// Stable search alias used for fielded search and optional summaries.
    pub alias: String,
    /// Value mode for this path.
    pub mode: ExtendedMetaFieldMode,
    /// Optional ranking boost applied to this field in search.
    pub boost: f32,
    /// Whether this field is included in `extended_meta_summary` payloads.
    pub summary: bool,
}

impl Default for ExtendedMetaFieldConfig {
    fn default() -> Self {
        Self {
            path: String::new(),
            alias: String::new(),
            mode: ExtendedMetaFieldMode::default(),
            boost: 1.0,
            summary: false,
        }
    }
}

/// Default-off extended metadata search configuration.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct ExtendedMetaSearchConfig {
    /// Explicit allowlist of non-Nova dbt metadata fields.
    pub fields: Vec<ExtendedMetaFieldConfig>,
    /// Maximum configured fields accepted.
    pub max_fields: usize,
    /// Maximum values indexed per configured field.
    pub max_values_per_field: usize,
    /// Maximum UTF-8 bytes retained per value.
    pub max_bytes_per_value: usize,
}

impl Default for ExtendedMetaSearchConfig {
    fn default() -> Self {
        Self {
            fields: Vec::new(),
            max_fields: EXTENDED_META_DEFAULT_MAX_FIELDS,
            max_values_per_field: EXTENDED_META_DEFAULT_MAX_VALUES_PER_FIELD,
            max_bytes_per_value: EXTENDED_META_DEFAULT_MAX_BYTES_PER_VALUE,
        }
    }
}

impl ExtendedMetaSearchConfig {
    /// Validate configured extended metadata paths and caps.
    ///
    /// # Errors
    ///
    /// Returns an error when the allowlist contains unsafe paths, duplicate
    /// aliases, unsupported caps, or values that cannot be indexed safely.
    pub fn validate(&self) -> Result<()> {
        validate_bounded_usize(
            "search.extended_meta.max_fields",
            self.max_fields,
            EXTENDED_META_HARD_MAX_FIELDS,
        )?;
        validate_bounded_usize(
            "search.extended_meta.max_values_per_field",
            self.max_values_per_field,
            EXTENDED_META_HARD_MAX_VALUES_PER_FIELD,
        )?;
        validate_bounded_usize(
            "search.extended_meta.max_bytes_per_value",
            self.max_bytes_per_value,
            EXTENDED_META_HARD_MAX_BYTES_PER_VALUE,
        )?;

        if self.fields.len() > self.max_fields {
            return Err(DbtNovaError::InvalidParams(format!(
                "search.extended_meta.fields configures {} fields but max_fields is {}",
                self.fields.len(),
                self.max_fields
            )));
        }

        let mut aliases = BTreeSet::new();
        let mut paths = BTreeSet::new();
        for (index, field) in self.fields.iter().enumerate() {
            validate_extended_meta_path(index, &field.path)?;
            let alias = validate_extended_meta_alias(index, &field.alias)?;
            if !aliases.insert(alias.to_string()) {
                return Err(DbtNovaError::InvalidParams(format!(
                    "search.extended_meta.fields[{index}].alias '{alias}' is configured more than once"
                )));
            }

            let path = field.path.trim();
            if !paths.insert(path.to_string()) {
                return Err(DbtNovaError::InvalidParams(format!(
                    "search.extended_meta.fields[{index}].path '{path}' is configured more than once"
                )));
            }

            if !field.boost.is_finite() || field.boost < 0.0 {
                return Err(DbtNovaError::InvalidParams(format!(
                    "search.extended_meta.fields[{index}].boost must be a finite number greater than or equal to 0"
                )));
            }
        }

        Ok(())
    }
}

fn validate_bounded_usize(name: &str, value: usize, hard_max: usize) -> Result<()> {
    if value == 0 || value > hard_max {
        return Err(DbtNovaError::InvalidParams(format!(
            "{name} must be between 1 and {hard_max}"
        )));
    }
    Ok(())
}

fn validate_extended_meta_alias(index: usize, alias: &str) -> Result<&str> {
    let alias = alias.trim();
    if alias.is_empty() {
        return Err(DbtNovaError::InvalidParams(format!(
            "search.extended_meta.fields[{index}].alias is required"
        )));
    }
    let mut chars = alias.chars();
    let Some(first) = chars.next() else {
        return Err(DbtNovaError::InvalidParams(format!(
            "search.extended_meta.fields[{index}].alias is required"
        )));
    };
    if !first.is_ascii_lowercase() {
        return Err(DbtNovaError::InvalidParams(format!(
            "search.extended_meta.fields[{index}].alias '{alias}' must start with a lowercase ASCII letter"
        )));
    }
    if !chars.all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_') {
        return Err(DbtNovaError::InvalidParams(format!(
            "search.extended_meta.fields[{index}].alias '{alias}' must contain only lowercase ASCII letters, digits, and underscores"
        )));
    }

    Ok(alias)
}

fn validate_extended_meta_path(index: usize, path: &str) -> Result<()> {
    let path = path.trim();
    if path.is_empty() {
        return Err(DbtNovaError::InvalidParams(format!(
            "search.extended_meta.fields[{index}].path is required"
        )));
    }

    let segments = path.split('.').collect::<Vec<_>>();
    if segments.iter().any(|segment| segment.trim().is_empty()) {
        return Err(DbtNovaError::InvalidParams(format!(
            "search.extended_meta.fields[{index}].path '{path}' contains an empty segment"
        )));
    }
    for (segment_index, segment) in segments.iter().enumerate() {
        let valid_segment = *segment == "*"
            || segment
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-');
        if !valid_segment {
            return Err(DbtNovaError::InvalidParams(format!(
                "search.extended_meta.fields[{index}].path '{path}' must use dot-separated ASCII key names"
            )));
        }
        if *segment == "*" && !(segment_index == 1 && segments.first() == Some(&"columns")) {
            return Err(DbtNovaError::InvalidParams(format!(
                "search.extended_meta.fields[{index}].path '{path}' may only use '*' in 'columns.*.meta.' paths"
            )));
        }
        if let Some(sensitive) = contains_sensitive_segment(segment) {
            return Err(DbtNovaError::InvalidParams(format!(
                "search.extended_meta.fields[{index}].path '{path}' is not allowed because segment '{segment}' matches sensitive key '{sensitive}'"
            )));
        }
    }

    if !is_supported_extended_meta_path(&segments) {
        return Err(DbtNovaError::InvalidParams(format!(
            "search.extended_meta.fields[{index}].path '{path}' must start with 'meta.' or 'columns.*.meta.'"
        )));
    }
    if is_nova_meta_path(&segments) {
        return Err(DbtNovaError::InvalidParams(format!(
            "search.extended_meta.fields[{index}].path '{path}' targets meta.nova, which is already indexed by Nova"
        )));
    }

    Ok(())
}

fn is_supported_extended_meta_path(segments: &[&str]) -> bool {
    if segments.len() >= 2 && segments[0] == "meta" {
        return true;
    }
    segments.len() >= 4 && segments[0] == "columns" && segments[1] == "*" && segments[2] == "meta"
}

fn is_nova_meta_path(segments: &[&str]) -> bool {
    (segments.len() >= 2 && segments[0] == "meta" && segments[1] == "nova")
        || (segments.len() >= 4
            && segments[0] == "columns"
            && segments[1] == "*"
            && segments[2] == "meta"
            && segments[3] == "nova")
}

fn contains_sensitive_segment(segment: &str) -> Option<&'static str> {
    let normalized = normalize_sensitive_segment(segment);
    if normalized.contains("private_key") || normalized.contains("privatekey") {
        return Some("private_key");
    }
    if normalized.contains("api_key") || normalized.contains("apikey") {
        return Some("api_key");
    }

    for token in normalized.split('_') {
        match token {
            "token" => return Some("token"),
            "secret" => return Some("secret"),
            "password" => return Some("password"),
            "credential" | "credentials" => return Some("credential"),
            _ => {}
        }
    }
    None
}

fn normalize_sensitive_segment(segment: &str) -> String {
    let mut normalized = String::with_capacity(segment.len());
    let mut previous_was_separator = true;
    for ch in segment.chars() {
        if ch.is_ascii_uppercase() {
            if !previous_was_separator {
                normalized.push('_');
            }
            normalized.push(ch.to_ascii_lowercase());
            previous_was_separator = false;
        } else if ch.is_ascii_alphanumeric() {
            normalized.push(ch.to_ascii_lowercase());
            previous_was_separator = false;
        } else if !previous_was_separator {
            normalized.push('_');
            previous_was_separator = true;
        }
    }
    normalized.trim_matches('_').to_string()
}
