use std::env;
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};

use crate::error::{DbtNovaError, Result};

const DEFAULT_TOKEN_URI: &str = "https://oauth2.googleapis.com/token";
const CLOUD_PLATFORM_SCOPE: &str = "https://www.googleapis.com/auth/cloud-platform";
const TOKEN_TIMEOUT_SECS: u64 = 15;
const GOOGLE_PROJECT_ENV_KEYS: [&str; 3] = [
    "DBT_NOVA_GCP_PROJECT_ID",
    "GOOGLE_CLOUD_PROJECT",
    "GCP_PROJECT_ID",
];
const GOOGLE_TOKEN_ENV_KEYS: [&str; 3] = [
    "DBT_NOVA_GCP_ACCESS_TOKEN",
    "GCP_ACCESS_TOKEN",
    "GOOGLE_OAUTH_ACCESS_TOKEN",
];

/// Resolve a Google project id from env vars.
///
/// `extra_env_keys` are checked first, then shared Google defaults.
#[must_use]
pub fn resolve_gcp_project_id(extra_env_keys: &[&str]) -> Option<String> {
    first_non_empty_env(extra_env_keys).or_else(|| first_non_empty_env(&GOOGLE_PROJECT_ENV_KEYS))
}

/// Resolve a Google OAuth access token from env vars, service-account JSON, or gcloud ADC.
///
/// `extra_env_keys` are checked first, then shared Google defaults.
///
/// # Errors
/// Returns an error when no credential source can produce a valid token.
pub fn resolve_gcp_access_token(extra_env_keys: &[&str]) -> Result<String> {
    // Fast path: explicit tokens provided by caller/provider env.
    if let Some(token) =
        first_non_empty_env(extra_env_keys).or_else(|| first_non_empty_env(&GOOGLE_TOKEN_ENV_KEYS))
    {
        return Ok(token);
    }

    let mut failures = Vec::new();
    let credentials_path = env::var("GOOGLE_APPLICATION_CREDENTIALS")
        .ok()
        .map(|path| path.trim().to_string())
        .filter(|path| !path.is_empty());

    if let Some(path) = credentials_path.as_deref() {
        // Service-account key flow: sign JWT assertion and exchange for OAuth access token.
        match token_from_service_account_file(path) {
            Ok(token) => return Ok(token),
            Err(err) => failures.push(format!("service-account token exchange failed: {err}")),
        }

        // Fallback for environments that rely on gcloud credential file override semantics.
        if let Some(token) = token_from_gcloud_credential_override(path) {
            return Ok(token);
        }
        failures.push(
            "gcloud credential-file override could not produce a token for GOOGLE_APPLICATION_CREDENTIALS"
                .to_string(),
        );
    }

    if let Some(token) = token_from_gcloud_adc() {
        return Ok(token);
    }
    failures.push("gcloud ADC token lookup failed".to_string());

    let mut steps = vec![
        "Set DBT_NOVA_GCP_ACCESS_TOKEN (or provider-specific token env)".to_string(),
        "or set GOOGLE_APPLICATION_CREDENTIALS to a service-account JSON key".to_string(),
        "or run 'gcloud auth application-default login'".to_string(),
    ];
    if !failures.is_empty() {
        steps.push(format!("Details: {}", failures.join("; ")));
    }
    Err(DbtNovaError::GcpAuthError(steps.join(". ")))
}

/// Async wrapper for resolving a Google OAuth access token.
///
/// This offloads blocking credential resolution (service-account exchange and
/// `gcloud` subprocess calls) to the blocking thread pool so async worker threads
/// stay responsive.
///
/// # Errors
/// Returns an error when no credential source can produce a valid token.
pub async fn resolve_gcp_access_token_async(extra_env_keys: &[&str]) -> Result<String> {
    let keys: Vec<String> = extra_env_keys
        .iter()
        .map(|key| (*key).to_string())
        .collect();
    tokio::task::spawn_blocking(move || {
        let refs: Vec<&str> = keys.iter().map(String::as_str).collect();
        resolve_gcp_access_token(&refs)
    })
    .await
    .map_err(|err| {
        DbtNovaError::GcpAuthError(format!(
            "failed to resolve GCP access token in blocking task: {err}"
        ))
    })?
}

fn first_non_empty_env(keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Ok(value) = env::var(key) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

fn token_from_gcloud_adc() -> Option<String> {
    token_from_gcloud_command(&["auth", "application-default", "print-access-token"])
}

fn token_from_gcloud_credential_override(path: &str) -> Option<String> {
    token_from_gcloud_command(&[
        &format!("--credential-file-override={path}"),
        "auth",
        "print-access-token",
    ])
}

fn token_from_gcloud_command(args: &[&str]) -> Option<String> {
    let output = Command::new("gcloud").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let token = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if token.is_empty() { None } else { Some(token) }
}

#[derive(Debug, Deserialize)]
struct ServiceAccountKey {
    client_email: String,
    private_key: String,
    token_uri: Option<String>,
}

#[derive(Debug, Serialize)]
struct ServiceAccountClaims<'a> {
    iss: &'a str,
    scope: &'a str,
    aud: &'a str,
    exp: usize,
    iat: usize,
}

#[derive(Debug, Serialize)]
struct TokenExchangeRequest<'a> {
    grant_type: &'a str,
    assertion: &'a str,
}

#[derive(Debug, Deserialize)]
struct TokenExchangeSuccess {
    access_token: String,
}

#[derive(Debug, Deserialize)]
struct TokenExchangeFailure {
    error: Option<String>,
    error_description: Option<String>,
}

fn token_from_service_account_file(path: &str) -> std::result::Result<String, String> {
    let bytes = std::fs::read(path).map_err(|err| format!("cannot read '{path}': {err}"))?;
    let key: ServiceAccountKey = serde_json::from_slice(&bytes)
        .map_err(|err| format!("invalid service-account JSON at '{path}': {err}"))?;

    let token_uri = key
        .token_uri
        .as_deref()
        .map(str::trim)
        .filter(|uri| !uri.is_empty())
        .unwrap_or(DEFAULT_TOKEN_URI);

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::from_secs(0))
        .as_secs();
    let claims = ServiceAccountClaims {
        iss: key.client_email.as_str(),
        scope: CLOUD_PLATFORM_SCOPE,
        aud: token_uri,
        iat: usize::try_from(now.saturating_sub(5)).unwrap_or(0),
        exp: usize::try_from(now.saturating_add(3600)).unwrap_or(usize::MAX),
    };

    let assertion = encode(
        &Header::new(Algorithm::RS256),
        &claims,
        &EncodingKey::from_rsa_pem(key.private_key.as_bytes())
            .map_err(|err| format!("invalid private key: {err}"))?,
    )
    .map_err(|err| format!("failed to sign JWT assertion: {err}"))?;

    let client = Client::builder()
        .timeout(Duration::from_secs(TOKEN_TIMEOUT_SECS))
        .build()
        .map_err(|err| format!("failed to create HTTP client: {err}"))?;

    let response = client
        .post(token_uri)
        // OAuth 2.0 JWT bearer grant as defined by Google service-account flow.
        .form(&TokenExchangeRequest {
            grant_type: "urn:ietf:params:oauth:grant-type:jwt-bearer",
            assertion: assertion.as_str(),
        })
        .send()
        .map_err(|err| format!("token request failed: {err}"))?;

    let status = response.status();
    if status.is_success() {
        let payload: TokenExchangeSuccess = response
            .json()
            .map_err(|err| format!("invalid token success payload: {err}"))?;
        let token = payload.access_token.trim();
        if token.is_empty() {
            return Err("token response contained empty access_token".to_string());
        }
        return Ok(token.to_string());
    }

    let body = response
        .text()
        .unwrap_or_else(|err| format!("<failed to read error body: {err}>"));
    if let Ok(err_payload) = serde_json::from_str::<TokenExchangeFailure>(&body) {
        let error = err_payload
            .error
            .unwrap_or_else(|| "unknown_error".to_string());
        let detail = err_payload
            .error_description
            .unwrap_or_else(|| "no description".to_string());
        return Err(format!("HTTP {} {error}: {detail}", status.as_u16()));
    }

    Err(format!("HTTP {} response: {}", status.as_u16(), body))
}

#[cfg(test)]
mod tests {
    use super::{resolve_gcp_access_token, resolve_gcp_access_token_async, resolve_gcp_project_id};
    use crate::error::DbtNovaError;
    use std::sync::{LazyLock, Mutex};

    static ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    fn with_env<F>(vars: &[(&str, Option<&str>)], f: F)
    where
        F: FnOnce(),
    {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let mut old_values = Vec::with_capacity(vars.len());
        for (key, value) in vars {
            let old = std::env::var(key).ok();
            old_values.push((*key, old));
            unsafe {
                match value {
                    Some(v) => std::env::set_var(key, v),
                    None => std::env::remove_var(key),
                }
            }
        }

        f();

        for (key, old) in old_values {
            unsafe {
                if let Some(v) = old {
                    std::env::set_var(key, v);
                } else {
                    std::env::remove_var(key);
                }
            }
        }
    }

    #[test]
    fn project_id_prefers_provider_specific_key() {
        with_env(
            &[
                ("DBT_NOVA_BIGQUERY_PROJECT_ID", Some("bq-project")),
                ("DBT_NOVA_GCP_PROJECT_ID", Some("shared-project")),
                ("GOOGLE_CLOUD_PROJECT", Some("google-project")),
                ("GCP_PROJECT_ID", Some("gcp-project")),
            ],
            || {
                let project = resolve_gcp_project_id(&["DBT_NOVA_BIGQUERY_PROJECT_ID"]);
                assert_eq!(project.as_deref(), Some("bq-project"));
            },
        );
    }

    #[test]
    fn project_id_falls_back_to_shared_key() {
        with_env(
            &[
                ("DBT_NOVA_BIGQUERY_PROJECT_ID", None),
                ("DBT_NOVA_GCP_PROJECT_ID", Some("shared-project")),
                ("GOOGLE_CLOUD_PROJECT", Some("google-project")),
                ("GCP_PROJECT_ID", Some("gcp-project")),
            ],
            || {
                let project = resolve_gcp_project_id(&["DBT_NOVA_BIGQUERY_PROJECT_ID"]);
                assert_eq!(project.as_deref(), Some("shared-project"));
            },
        );
    }

    #[test]
    fn access_token_prefers_provider_specific_key() {
        with_env(
            &[
                ("DBT_NOVA_BIGQUERY_ACCESS_TOKEN", Some("bq-token")),
                ("DBT_NOVA_GCP_ACCESS_TOKEN", Some("shared-token")),
                ("GCP_ACCESS_TOKEN", Some("legacy-token")),
                ("GOOGLE_OAUTH_ACCESS_TOKEN", Some("oauth-token")),
                ("GOOGLE_APPLICATION_CREDENTIALS", None),
            ],
            || {
                let token = resolve_gcp_access_token(&["DBT_NOVA_BIGQUERY_ACCESS_TOKEN"])
                    .expect("token should resolve");
                assert_eq!(token, "bq-token");
            },
        );
    }

    #[test]
    fn access_token_falls_back_to_shared_key() {
        with_env(
            &[
                ("DBT_NOVA_BIGQUERY_ACCESS_TOKEN", None),
                ("DBT_NOVA_GCP_ACCESS_TOKEN", Some("shared-token")),
                ("GCP_ACCESS_TOKEN", Some("legacy-token")),
                ("GOOGLE_OAUTH_ACCESS_TOKEN", Some("oauth-token")),
                ("GOOGLE_APPLICATION_CREDENTIALS", None),
            ],
            || {
                let token = resolve_gcp_access_token(&["DBT_NOVA_BIGQUERY_ACCESS_TOKEN"])
                    .expect("token should resolve");
                assert_eq!(token, "shared-token");
            },
        );
    }

    #[test]
    fn access_token_async_prefers_provider_specific_key() {
        with_env(
            &[
                ("DBT_NOVA_BIGQUERY_ACCESS_TOKEN", Some("bq-token-async")),
                ("DBT_NOVA_GCP_ACCESS_TOKEN", Some("shared-token-async")),
                ("GOOGLE_APPLICATION_CREDENTIALS", None),
            ],
            || {
                let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
                let token = runtime
                    .block_on(resolve_gcp_access_token_async(&[
                        "DBT_NOVA_BIGQUERY_ACCESS_TOKEN",
                    ]))
                    .expect("token should resolve");
                assert_eq!(token, "bq-token-async");
            },
        );
    }

    #[test]
    fn access_token_missing_sources_returns_structured_error() {
        with_env(
            &[
                ("DBT_NOVA_BIGQUERY_ACCESS_TOKEN", None),
                ("DBT_NOVA_GCP_ACCESS_TOKEN", None),
                ("GCP_ACCESS_TOKEN", None),
                ("GOOGLE_OAUTH_ACCESS_TOKEN", None),
                ("GOOGLE_APPLICATION_CREDENTIALS", None),
            ],
            || {
                let err = resolve_gcp_access_token(&["DBT_NOVA_BIGQUERY_ACCESS_TOKEN"])
                    .expect_err("missing token sources should fail");
                assert!(matches!(err, DbtNovaError::GcpAuthError(_)));
            },
        );
    }
}
