use serde::{Deserialize, Serialize};

use crate::error::{DbtNovaError, Result};

/// Supported prebuilt-asset metadata contract version.
pub const PREBUILT_ASSETS_CONTRACT_VERSION: &str = "v1";

/// Metadata contract emitted by the reusable prebuilt-assets workflow.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct PrebuiltAssetsMetadata {
    pub contract_version: String,
    pub manifest_hash: String,
    pub manifest_version: String,
    pub entity_count: u64,
    pub storage_instance_id: String,
    pub dbt_nova_version: String,
    pub build_timestamp: String,
    pub artifact_name_storage: String,
    pub artifact_name_models: String,
}

impl PrebuiltAssetsMetadata {
    /// Parse and validate metadata contract JSON.
    ///
    /// # Errors
    ///
    /// Returns an error when JSON is malformed or contract validation fails.
    pub fn from_json_str(raw: &str) -> Result<Self> {
        let metadata: Self = serde_json::from_str(raw).map_err(|err| {
            DbtNovaError::InvalidParams(format!("invalid prebuilt metadata JSON: {err}"))
        })?;
        metadata.validate()?;
        Ok(metadata)
    }

    /// Validate metadata contract values.
    ///
    /// # Errors
    ///
    /// Returns an error when required contract fields are missing/invalid.
    pub fn validate(&self) -> Result<()> {
        if self.contract_version != PREBUILT_ASSETS_CONTRACT_VERSION {
            return Err(DbtNovaError::InvalidParams(format!(
                "unsupported prebuilt metadata contract_version '{}' (expected '{}')",
                self.contract_version, PREBUILT_ASSETS_CONTRACT_VERSION
            )));
        }

        if self.manifest_hash.trim().is_empty() {
            return Err(DbtNovaError::InvalidParams(
                "prebuilt metadata manifest_hash cannot be empty".to_string(),
            ));
        }
        if self.manifest_version.trim().is_empty() {
            return Err(DbtNovaError::InvalidParams(
                "prebuilt metadata manifest_version cannot be empty".to_string(),
            ));
        }
        if self.storage_instance_id.trim().is_empty() {
            return Err(DbtNovaError::InvalidParams(
                "prebuilt metadata storage_instance_id cannot be empty".to_string(),
            ));
        }
        if self.dbt_nova_version.trim().is_empty() {
            return Err(DbtNovaError::InvalidParams(
                "prebuilt metadata dbt_nova_version cannot be empty".to_string(),
            ));
        }
        if self.artifact_name_storage.trim().is_empty() {
            return Err(DbtNovaError::InvalidParams(
                "prebuilt metadata artifact_name_storage cannot be empty".to_string(),
            ));
        }

        if self.build_timestamp.trim().is_empty() {
            return Err(DbtNovaError::InvalidParams(
                "prebuilt metadata build_timestamp cannot be empty".to_string(),
            ));
        }
        if !is_iso8601_utc_timestamp(self.build_timestamp.trim()) {
            return Err(DbtNovaError::InvalidParams(
                "prebuilt metadata build_timestamp must use UTC ISO-8601 format YYYY-MM-DDTHH:MM:SSZ"
                    .to_string(),
            ));
        }

        Ok(())
    }

    /// Whether this contract includes an optional models archive.
    #[must_use]
    pub fn has_models_artifact(&self) -> bool {
        !self.artifact_name_models.trim().is_empty()
    }
}

fn is_iso8601_utc_timestamp(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 20 {
        return false;
    }
    if bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
        || bytes[19] != b'Z'
    {
        return false;
    }
    if !bytes
        .iter()
        .enumerate()
        .all(|(index, byte)| matches!(index, 4 | 7 | 10 | 13 | 16 | 19) || byte.is_ascii_digit())
    {
        return false;
    }

    let month = parse_u32(&value[5..7]);
    let day = parse_u32(&value[8..10]);
    let hour = parse_u32(&value[11..13]);
    let minute = parse_u32(&value[14..16]);
    let second = parse_u32(&value[17..19]);

    matches!(month, Some(1..=12))
        && matches!(day, Some(1..=31))
        && matches!(hour, Some(0..=23))
        && matches!(minute, Some(0..=59))
        && matches!(second, Some(0..=59))
}

fn parse_u32(segment: &str) -> Option<u32> {
    segment.parse::<u32>().ok()
}

#[cfg(test)]
mod tests {
    use super::{PREBUILT_ASSETS_CONTRACT_VERSION, PrebuiltAssetsMetadata};

    fn valid_contract_json() -> String {
        format!(
            r#"{{
  "contract_version": "{PREBUILT_ASSETS_CONTRACT_VERSION}",
  "manifest_hash": "abcd1234",
  "manifest_version": "v12",
  "entity_count": 42,
  "storage_instance_id": "analytics-prod",
  "dbt_nova_version": "0.0.2",
  "build_timestamp": "2026-01-01T10:00:00Z",
  "artifact_name_storage": "analytics-storage-123",
  "artifact_name_models": "analytics-models-123"
}}"#
        )
    }

    #[test]
    fn from_json_str_accepts_valid_contract() {
        let metadata = PrebuiltAssetsMetadata::from_json_str(&valid_contract_json())
            .expect("valid contract should parse");
        assert_eq!(metadata.contract_version, PREBUILT_ASSETS_CONTRACT_VERSION);
        assert!(metadata.has_models_artifact());
    }

    #[test]
    fn from_json_str_rejects_invalid_contract_version() {
        let invalid = valid_contract_json().replace(
            "\"contract_version\": \"v1\"",
            "\"contract_version\": \"v999\"",
        );
        let error = PrebuiltAssetsMetadata::from_json_str(&invalid)
            .expect_err("invalid contract version should fail");
        assert!(error.to_string().contains("contract_version"));
    }

    #[test]
    fn from_json_str_rejects_missing_required_field() {
        let invalid = valid_contract_json().replace(
            "\"artifact_name_storage\": \"analytics-storage-123\"",
            "\"artifact_name_storage\": \"\"",
        );
        let error = PrebuiltAssetsMetadata::from_json_str(&invalid)
            .expect_err("empty storage name invalid");
        assert!(error.to_string().contains("artifact_name_storage"));
    }

    #[test]
    fn from_json_str_rejects_empty_build_timestamp() {
        let invalid = valid_contract_json().replace(
            "\"build_timestamp\": \"2026-01-01T10:00:00Z\"",
            "\"build_timestamp\": \"\"",
        );
        let error = PrebuiltAssetsMetadata::from_json_str(&invalid).expect_err("invalid timestamp");
        assert!(error.to_string().contains("build_timestamp"));
    }

    #[test]
    fn from_json_str_rejects_non_iso8601_build_timestamp() {
        let invalid = valid_contract_json().replace(
            "\"build_timestamp\": \"2026-01-01T10:00:00Z\"",
            "\"build_timestamp\": \"2026-01-01 10:00:00\"",
        );
        let error = PrebuiltAssetsMetadata::from_json_str(&invalid).expect_err("invalid timestamp");
        assert!(error.to_string().contains("ISO-8601"));
    }
}
