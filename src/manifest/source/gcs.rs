use crate::config::DbtNovaConfig;
use crate::error::Result;

use super::{ManifestLocator, ManifestResolution, fetch_gcs_manifest};

pub(crate) fn resolve_gcs(
    locator: &ManifestLocator,
    config: &DbtNovaConfig,
) -> Result<ManifestResolution> {
    fetch_gcs_manifest(&locator.rest, &locator.raw, config)
}
