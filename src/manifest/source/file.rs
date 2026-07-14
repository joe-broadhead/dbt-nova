use crate::config::DbtNovaConfig;
use crate::error::{DbtNovaError, Result};

use super::{ManifestLocator, ManifestResolution};

pub(crate) fn resolve_file(
    locator: &ManifestLocator,
    config: &DbtNovaConfig,
) -> Result<ManifestResolution> {
    let local_path = std::path::PathBuf::from(&locator.rest);
    enforce_local_size_limit(&local_path, config.manifest_max_bytes)?;
    Ok(ManifestResolution {
        local_path,
        source_uri: locator.raw.clone(),
        cached: false,
    })
}

fn enforce_local_size_limit(path: &std::path::Path, max_bytes: u64) -> Result<()> {
    if max_bytes == 0 {
        return Ok(());
    }
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => {
            return Err(DbtNovaError::ManifestError(format!(
                "Failed to inspect local manifest file {}: {err}",
                path.display()
            )));
        }
    };
    let len = metadata.len();
    if len > max_bytes {
        return Err(DbtNovaError::ManifestError(format!(
            "Local manifest file {} exceeded size limit ({len} > {max_bytes})",
            path.display()
        )));
    }
    Ok(())
}
