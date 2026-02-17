use crate::config::DbtNovaConfig;
use crate::error::Result;

use super::{ManifestLocator, ManifestResolution, fetch_s3_manifest};

pub(crate) fn resolve_s3(
    locator: &ManifestLocator,
    config: &DbtNovaConfig,
) -> Result<ManifestResolution> {
    fetch_s3_manifest(&locator.rest, &locator.raw, config)
}
