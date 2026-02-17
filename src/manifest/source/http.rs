use crate::config::DbtNovaConfig;
use crate::error::{DbtNovaError, Result};

use super::{ManifestLocator, ManifestResolution, fetch_http_manifest};

pub(crate) fn resolve_http(
    locator: &ManifestLocator,
    config: &DbtNovaConfig,
) -> Result<ManifestResolution> {
    if locator.scheme == "http" && !config.manifest_allow_http {
        return Err(DbtNovaError::InvalidParams(
            "http manifest URIs are disabled; use https:// or set DBT_NOVA_MANIFEST_ALLOW_HTTP=true"
                .to_string(),
        ));
    }
    fetch_http_manifest(&locator.raw, config)
}
