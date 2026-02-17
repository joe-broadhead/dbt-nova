use crate::config::DbtNovaConfig;
use crate::error::Result;

use super::{ManifestLocator, ManifestResolution, fetch_dbfs_manifest};

pub(crate) fn resolve_dbfs(
    locator: &ManifestLocator,
    config: &DbtNovaConfig,
) -> Result<ManifestResolution> {
    fetch_dbfs_manifest(&locator.rest, &locator.raw, config)
}
