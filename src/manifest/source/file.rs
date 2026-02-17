use crate::config::DbtNovaConfig;
use crate::error::Result;

use super::{ManifestLocator, ManifestResolution};

#[allow(clippy::unnecessary_wraps)]
pub(crate) fn resolve_file(
    locator: &ManifestLocator,
    _config: &DbtNovaConfig,
) -> Result<ManifestResolution> {
    Ok(ManifestResolution {
        local_path: std::path::PathBuf::from(&locator.rest),
        source_uri: locator.raw.clone(),
        cached: false,
    })
}
