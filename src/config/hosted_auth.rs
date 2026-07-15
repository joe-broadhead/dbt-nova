use std::str::FromStr;

use jsonwebtoken::Algorithm;
use serde::{Deserialize, Serialize};

use crate::error::{DbtNovaError, Result};

/// Hosted authentication modes for streamable HTTP deployments.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum HostedAuthMode {
    /// Current behavior: rely on the external proxy/platform boundary.
    #[default]
    Off,
    /// Signed identity envelope from a trusted reverse proxy.
    ProxySignedHeaders,
    /// Bearer JWT validation at the Nova HTTP boundary.
    Jwt,
}

impl HostedAuthMode {
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "off" => Some(Self::Off),
            "proxy_signed_headers" | "proxy-signed-headers" | "proxy" => {
                Some(Self::ProxySignedHeaders)
            }
            "jwt" => Some(Self::Jwt),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::ProxySignedHeaders => "proxy_signed_headers",
            Self::Jwt => "jwt",
        }
    }
}

/// Default-off hosted HTTP identity validation config.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct HostedAuthConfig {
    /// Hosted auth mode. Proxy and JWT modes are implemented.
    pub mode: HostedAuthMode,
    /// Whether authentication must be present for hosted requests.
    pub required: bool,
    /// Claim/field used as stable request subject.
    pub identity_subject_claim: String,
    /// Optional email claim/field.
    pub identity_email_claim: String,
    /// Optional display-name claim/field.
    pub identity_name_claim: String,
    /// Optional groups claim/field reserved for future policy hooks.
    pub identity_groups_claim: String,
    /// Proxy-mode base64url JSON identity envelope header.
    pub proxy_identity_header: String,
    /// Proxy-mode HMAC-SHA256 signature header.
    pub proxy_signature_header: String,
    /// Proxy-mode local secret file used for envelope verification.
    pub proxy_identity_secret_file: String,
    /// Proxy-mode timestamp freshness window.
    pub proxy_identity_max_age_secs: u64,
    /// JWT issuer allowlist entry.
    pub jwt_issuer: String,
    /// JWT audience allowlist entry.
    pub jwt_audience: String,
    /// HTTPS JWKS endpoint for JWT signature verification.
    pub jwt_jwks_url: String,
    /// Explicit JWT algorithm allowlist. `none` is never accepted.
    pub jwt_algorithms: Vec<String>,
    /// Clock skew leeway for JWT `exp` and `nbf` checks.
    pub jwt_clock_skew_secs: u64,
}

impl Default for HostedAuthConfig {
    fn default() -> Self {
        Self {
            mode: HostedAuthMode::Off,
            required: false,
            identity_subject_claim: "sub".to_string(),
            identity_email_claim: "email".to_string(),
            identity_name_claim: "name".to_string(),
            identity_groups_claim: "groups".to_string(),
            proxy_identity_header: String::new(),
            proxy_signature_header: String::new(),
            proxy_identity_secret_file: String::new(),
            proxy_identity_max_age_secs: 300,
            jwt_issuer: String::new(),
            jwt_audience: String::new(),
            jwt_jwks_url: String::new(),
            jwt_algorithms: Vec::new(),
            jwt_clock_skew_secs: 60,
        }
    }
}

impl HostedAuthConfig {
    /// Validate hosted auth configuration.
    ///
    /// # Errors
    /// Returns an error for unknown/incomplete/misleading auth configuration.
    pub fn validate(&self) -> Result<()> {
        match self.mode {
            HostedAuthMode::Off => {
                if self.required {
                    return Err(DbtNovaError::InvalidParams(
                        "DBT_NOVA_AUTH_REQUIRED=true requires DBT_NOVA_AUTH_MODE=proxy_signed_headers or jwt"
                            .to_string(),
                    ));
                }
                Ok(())
            }
            HostedAuthMode::ProxySignedHeaders => {
                self.validate_proxy_signed_headers()?;
                Ok(())
            }
            HostedAuthMode::Jwt => {
                self.validate_jwt()?;
                Ok(())
            }
        }
    }

    fn validate_proxy_signed_headers(&self) -> Result<()> {
        self.validate_non_off_common()?;
        require_non_empty(
            "DBT_NOVA_PROXY_IDENTITY_HEADER",
            &self.proxy_identity_header,
        )?;
        require_non_empty(
            "DBT_NOVA_PROXY_SIGNATURE_HEADER",
            &self.proxy_signature_header,
        )?;
        require_non_empty(
            "DBT_NOVA_PROXY_IDENTITY_SECRET_FILE",
            &self.proxy_identity_secret_file,
        )?;
        if self.proxy_identity_max_age_secs == 0 {
            return Err(DbtNovaError::InvalidParams(
                "DBT_NOVA_PROXY_IDENTITY_MAX_AGE_SECS must be greater than 0".to_string(),
            ));
        }
        Ok(())
    }

    fn validate_jwt(&self) -> Result<()> {
        self.validate_non_off_common()?;
        require_non_empty("DBT_NOVA_JWT_ISSUER", &self.jwt_issuer)?;
        require_non_empty("DBT_NOVA_JWT_AUDIENCE", &self.jwt_audience)?;
        require_non_empty("DBT_NOVA_JWT_JWKS_URL", &self.jwt_jwks_url)?;
        require_https_url("DBT_NOVA_JWT_JWKS_URL", &self.jwt_jwks_url)?;
        parse_jwt_algorithms(&self.jwt_algorithms)?;
        Ok(())
    }

    fn validate_non_off_common(&self) -> Result<()> {
        if !self.required {
            return Err(DbtNovaError::InvalidParams(
                "non-off DBT_NOVA_AUTH_MODE requires DBT_NOVA_AUTH_REQUIRED=true".to_string(),
            ));
        }
        require_non_empty(
            "DBT_NOVA_IDENTITY_SUBJECT_CLAIM",
            &self.identity_subject_claim,
        )
    }
}

fn require_non_empty(name: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        return Err(DbtNovaError::InvalidParams(format!(
            "{name} is required for the selected hosted auth mode"
        )));
    }
    Ok(())
}

pub(crate) fn parse_jwt_algorithms(values: &[String]) -> Result<Vec<Algorithm>> {
    let mut algorithms = Vec::with_capacity(values.len());
    for value in values {
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        let algorithm = Algorithm::from_str(value).map_err(|_| {
            DbtNovaError::InvalidParams(format!(
                "DBT_NOVA_JWT_ALGORITHMS contains unsupported algorithm `{value}`"
            ))
        })?;
        if is_hmac_algorithm(algorithm) {
            return Err(DbtNovaError::InvalidParams(
                "DBT_NOVA_JWT_ALGORITHMS must use asymmetric algorithms; HS256, HS384, and HS512 are not accepted for hosted JWT mode"
                    .to_string(),
            ));
        }
        if !algorithms.contains(&algorithm) {
            algorithms.push(algorithm);
        }
    }
    if algorithms.is_empty() {
        return Err(DbtNovaError::InvalidParams(
            "DBT_NOVA_JWT_ALGORITHMS must include at least one accepted algorithm for JWT mode"
                .to_string(),
        ));
    }
    Ok(algorithms)
}

const fn is_hmac_algorithm(algorithm: Algorithm) -> bool {
    matches!(
        algorithm,
        Algorithm::HS256 | Algorithm::HS384 | Algorithm::HS512
    )
}

fn require_https_url(name: &str, value: &str) -> Result<()> {
    if !value.trim().to_ascii_lowercase().starts_with("https://") {
        return Err(DbtNovaError::InvalidParams(format!(
            "{name} must start with https://"
        )));
    }
    Ok(())
}
