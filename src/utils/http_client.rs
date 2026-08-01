use std::env;
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::Read;
use std::path::Path;

use reqwest::Certificate;

use crate::error::{DbtNovaError, Result};

const CUSTOM_CA_BUNDLE_ENV_VARS: [&str; 2] = ["REQUESTS_CA_BUNDLE", "CURL_CA_BUNDLE"];
const MAX_CA_BUNDLE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_CA_BUNDLE_READ_BYTES: usize = 8 * 1024 * 1024;

/// Start an asynchronous HTTP client builder with Nova's shared TLS trust policy.
pub(crate) fn async_client_builder() -> Result<reqwest::ClientBuilder> {
    apply_custom_ca_bundle(reqwest::Client::builder())
}

/// Start a blocking HTTP client builder with Nova's shared TLS trust policy.
pub(crate) fn blocking_client_builder() -> Result<reqwest::blocking::ClientBuilder> {
    apply_custom_ca_bundle_blocking(reqwest::blocking::Client::builder())
}

fn apply_custom_ca_bundle(mut builder: reqwest::ClientBuilder) -> Result<reqwest::ClientBuilder> {
    for certificate in custom_ca_certificates()? {
        builder = builder.add_root_certificate(certificate);
    }
    Ok(builder)
}

fn apply_custom_ca_bundle_blocking(
    mut builder: reqwest::blocking::ClientBuilder,
) -> Result<reqwest::blocking::ClientBuilder> {
    for certificate in custom_ca_certificates()? {
        builder = builder.add_root_certificate(certificate);
    }
    Ok(builder)
}

fn custom_ca_certificates() -> Result<Vec<Certificate>> {
    let Some((env_name, value)) = configured_custom_ca_bundle() else {
        return Ok(Vec::new());
    };
    load_ca_bundle(env_name, Path::new(&value))
}

fn configured_custom_ca_bundle() -> Option<(&'static str, OsString)> {
    CUSTOM_CA_BUNDLE_ENV_VARS.iter().find_map(|env_name| {
        env::var_os(env_name)
            .and_then(|value| (!value.as_os_str().is_empty()).then_some((*env_name, value)))
    })
}

fn load_ca_bundle(env_name: &str, path: &Path) -> Result<Vec<Certificate>> {
    let metadata = fs::metadata(path).map_err(|error| {
        tls_config_error(format!(
            "{env_name} points to an unreadable CA bundle '{}': {error}",
            path.display()
        ))
    })?;
    if !metadata.is_file() {
        return Err(tls_config_error(format!(
            "{env_name} must point to a PEM CA bundle file, got '{}'",
            path.display()
        )));
    }
    if metadata.len() > MAX_CA_BUNDLE_BYTES {
        return Err(tls_config_error(format!(
            "{env_name} CA bundle '{}' exceeds the {MAX_CA_BUNDLE_BYTES}-byte limit",
            path.display()
        )));
    }

    let file = File::open(path).map_err(|error| {
        tls_config_error(format!(
            "{env_name} CA bundle '{}' could not be opened: {error}",
            path.display()
        ))
    })?;
    let mut bytes = Vec::with_capacity(
        usize::try_from(metadata.len())
            .unwrap_or(MAX_CA_BUNDLE_READ_BYTES)
            .min(MAX_CA_BUNDLE_READ_BYTES),
    );
    file.take(MAX_CA_BUNDLE_BYTES.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| {
            tls_config_error(format!(
                "{env_name} CA bundle '{}' could not be read: {error}",
                path.display()
            ))
        })?;
    if bytes.len() > MAX_CA_BUNDLE_READ_BYTES {
        return Err(tls_config_error(format!(
            "{env_name} CA bundle '{}' exceeds the {MAX_CA_BUNDLE_BYTES}-byte limit",
            path.display()
        )));
    }

    let certificates = Certificate::from_pem_bundle(&bytes).map_err(|error| {
        tls_config_error(format!(
            "{env_name} CA bundle '{}' is not valid PEM: {error}",
            path.display()
        ))
    })?;
    if certificates.is_empty() {
        return Err(tls_config_error(format!(
            "{env_name} CA bundle '{}' contains no certificates",
            path.display()
        )));
    }
    Ok(certificates)
}

fn tls_config_error(message: impl Into<String>) -> DbtNovaError {
    DbtNovaError::InvalidParams(format!("TLS trust configuration error: {}", message.into()))
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::sync::{LazyLock, Mutex};

    use tempfile::TempDir;

    use super::{MAX_CA_BUNDLE_BYTES, async_client_builder, blocking_client_builder};

    static ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    const TEST_CA_PEM: &str = r"-----BEGIN CERTIFICATE-----
MIIBnTCCAUOgAwIBAgIUUJ3mcRyhBlr7CqofxGEv8dLb6nYwCgYIKoZIzj0EAwIw
GzEZMBcGA1UEAwwQZGJ0LW5vdmEtdGVzdC1jYTAgFw0yNjA4MDEyMzE5NThaGA8y
MTI2MDcwODIzMTk1OFowGzEZMBcGA1UEAwwQZGJ0LW5vdmEtdGVzdC1jYTBZMBMG
ByqGSM49AgEGCCqGSM49AwEHA0IABCiNGLNRCmUF4vR8owt8brU0p1Z4RhytBy1k
SuAS+cvextVJ+11E4C5msqSh+QGmGold6/jBtKGYriCjItdCBvSjYzBhMB0GA1Ud
DgQWBBSQJcLpU5JQ8IPR1ffMGCMkPArFkDAfBgNVHSMEGDAWgBSQJcLpU5JQ8IPR
1ffMGCMkPArFkDAPBgNVHRMBAf8EBTADAQH/MA4GA1UdDwEB/wQEAwIBBjAKBggq
hkjOPQQDAgNIADBFAiBOF+qta4kDwAs6QvU2bxR0iyI90moQneaO3WJFItDA4QIh
AIyN7kTJeH0IenKUpeN+Sxm7D21JKDPu+mCcCMK9sLp6
-----END CERTIFICATE-----
";

    struct EnvGuard {
        values: Vec<(&'static str, Option<OsString>)>,
    }

    impl EnvGuard {
        fn clear() -> Self {
            let values = ["REQUESTS_CA_BUNDLE", "CURL_CA_BUNDLE"]
                .into_iter()
                .map(|key| (key, std::env::var_os(key)))
                .collect();
            // SAFETY: tests serialize environment mutation with ENV_LOCK.
            unsafe {
                std::env::remove_var("REQUESTS_CA_BUNDLE");
                std::env::remove_var("CURL_CA_BUNDLE");
            }
            Self { values }
        }

        fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) {
            // SAFETY: tests serialize environment mutation with ENV_LOCK.
            unsafe { std::env::set_var(key, value) };
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            // SAFETY: tests serialize environment mutation with ENV_LOCK.
            unsafe {
                for (key, value) in &self.values {
                    match value {
                        Some(value) => std::env::set_var(key, value),
                        None => std::env::remove_var(key),
                    }
                }
            }
        }
    }

    #[test]
    fn requests_ca_bundle_configures_async_and_blocking_clients() {
        let _lock = ENV_LOCK.lock().expect("environment lock");
        let _env = EnvGuard::clear();
        let temp = TempDir::new().expect("temporary CA directory");
        let path = temp.path().join("corporate-ca.pem");
        std::fs::write(&path, TEST_CA_PEM).expect("write test CA bundle");
        EnvGuard::set("REQUESTS_CA_BUNDLE", &path);

        async_client_builder()
            .expect("valid async TLS configuration")
            .build()
            .expect("async client");
        blocking_client_builder()
            .expect("valid blocking TLS configuration")
            .build()
            .expect("blocking client");
    }

    #[test]
    fn requests_ca_bundle_takes_precedence_over_curl_fallback() {
        let _lock = ENV_LOCK.lock().expect("environment lock");
        let _env = EnvGuard::clear();
        let temp = TempDir::new().expect("temporary CA directory");
        let path = temp.path().join("requests-ca.pem");
        std::fs::write(&path, TEST_CA_PEM).expect("write test CA bundle");
        EnvGuard::set("REQUESTS_CA_BUNDLE", &path);
        EnvGuard::set("CURL_CA_BUNDLE", temp.path().join("missing.pem"));

        let _builder = async_client_builder().expect("REQUESTS_CA_BUNDLE should win");
    }

    #[test]
    fn explicit_custom_ca_bundle_errors_fail_closed() {
        let _lock = ENV_LOCK.lock().expect("environment lock");
        let _env = EnvGuard::clear();
        let temp = TempDir::new().expect("temporary CA directory");
        let path = temp.path().join("invalid.pem");
        std::fs::write(&path, "not a certificate").expect("write invalid bundle");
        EnvGuard::set("REQUESTS_CA_BUNDLE", &path);

        let error = async_client_builder()
            .expect_err("invalid explicit bundle must fail")
            .to_string();
        assert!(error.contains("REQUESTS_CA_BUNDLE"));
        assert!(error.contains("contains no certificates") || error.contains("not valid PEM"));
    }

    #[test]
    fn custom_ca_bundle_size_is_bounded() {
        let _lock = ENV_LOCK.lock().expect("environment lock");
        let _env = EnvGuard::clear();
        let temp = TempDir::new().expect("temporary CA directory");
        let path = temp.path().join("oversized.pem");
        let file = std::fs::File::create(&path).expect("create sparse bundle");
        file.set_len(MAX_CA_BUNDLE_BYTES + 1)
            .expect("size sparse bundle");
        EnvGuard::set("CURL_CA_BUNDLE", &path);

        let error = blocking_client_builder()
            .expect_err("oversized explicit bundle must fail")
            .to_string();
        assert!(error.contains("CURL_CA_BUNDLE"));
        assert!(error.contains("exceeds"));
    }
}
