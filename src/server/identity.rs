use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::{
    extract::{Request, State},
    http::{HeaderMap, HeaderName, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use base64::Engine as _;
use hmac::{Hmac, Mac};
use serde_json::Value as JsonValue;
use sha2::{Digest, Sha256};
use tracing::warn;

use crate::config::{HostedAuthConfig, HostedAuthMode};
use crate::error::{DbtNovaError, Result};

const SIGNATURE_PREFIX: &str = "sha256=";
const MAX_IDENTITY_HEADER_BYTES: usize = 8 * 1024;
const MAX_SIGNATURE_HEADER_BYTES: usize = 256;
const MAX_SECRET_FILE_BYTES: usize = 64 * 1024;
const MIN_SECRET_BYTES: usize = 32;
const FUTURE_IAT_SKEW_SECS: u64 = 60;

type HmacSha256 = Hmac<Sha256>;

/// Sanitized request identity verified from trusted proxy headers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RequestIdentity {
    pub mode: HostedAuthMode,
    pub subject_hash: String,
}

#[derive(Debug, Clone)]
pub(crate) struct ProxyIdentityVerifier {
    identity_header: HeaderName,
    signature_header: HeaderName,
    subject_claim: String,
    max_age: Duration,
    secret: Arc<Vec<u8>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VerificationError {
    MissingIdentity,
    MissingSignature,
    MalformedIdentity,
    InvalidSignature,
    StaleIdentity,
}

impl ProxyIdentityVerifier {
    /// Build a proxy identity verifier from validated hosted auth config.
    ///
    /// # Errors
    /// Returns an error when header names are invalid or the secret file cannot
    /// be loaded safely.
    pub(crate) fn from_config(config: &HostedAuthConfig) -> Result<Option<Self>> {
        if config.mode != HostedAuthMode::ProxySignedHeaders {
            return Ok(None);
        }
        let identity_header = parse_header_name(
            "DBT_NOVA_PROXY_IDENTITY_HEADER",
            &config.proxy_identity_header,
        )?;
        let signature_header = parse_header_name(
            "DBT_NOVA_PROXY_SIGNATURE_HEADER",
            &config.proxy_signature_header,
        )?;
        if identity_header == signature_header {
            return Err(DbtNovaError::InvalidParams(
                "DBT_NOVA_PROXY_IDENTITY_HEADER and DBT_NOVA_PROXY_SIGNATURE_HEADER must be different"
                    .to_string(),
            ));
        }
        let secret = read_proxy_identity_secret(&config.proxy_identity_secret_file)?;
        Ok(Some(Self {
            identity_header,
            signature_header,
            subject_claim: config.identity_subject_claim.trim().to_string(),
            max_age: Duration::from_secs(config.proxy_identity_max_age_secs),
            secret: Arc::new(secret),
        }))
    }

    fn verify_headers(
        &self,
        headers: &HeaderMap,
    ) -> std::result::Result<RequestIdentity, VerificationError> {
        self.verify_headers_at(headers, unix_now_secs())
    }

    fn verify_headers_at(
        &self,
        headers: &HeaderMap,
        now_secs: u64,
    ) -> std::result::Result<RequestIdentity, VerificationError> {
        let identity = header_value(headers, &self.identity_header, MAX_IDENTITY_HEADER_BYTES)
            .ok_or(VerificationError::MissingIdentity)?;
        let signature = header_value(headers, &self.signature_header, MAX_SIGNATURE_HEADER_BYTES)
            .ok_or(VerificationError::MissingSignature)?;
        self.verify_header_values_at(identity, signature, now_secs)
    }

    fn verify_header_values_at(
        &self,
        identity_header_value: &str,
        signature_header_value: &str,
        now_secs: u64,
    ) -> std::result::Result<RequestIdentity, VerificationError> {
        verify_signature(
            identity_header_value.as_bytes(),
            signature_header_value,
            &self.secret,
        )?;
        let envelope = decode_identity_envelope(identity_header_value)?;
        let iat = envelope_iat(&envelope)?;
        if iat > now_secs.saturating_add(FUTURE_IAT_SKEW_SECS) {
            return Err(VerificationError::StaleIdentity);
        }
        if now_secs.saturating_sub(iat) > self.max_age.as_secs() {
            return Err(VerificationError::StaleIdentity);
        }
        let subject = envelope
            .get(&self.subject_claim)
            .and_then(JsonValue::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or(VerificationError::MalformedIdentity)?;
        Ok(RequestIdentity {
            mode: HostedAuthMode::ProxySignedHeaders,
            subject_hash: sha256_hex(subject.as_bytes()),
        })
    }
}

pub(crate) async fn verify_proxy_identity_request(
    State(verifier): State<Arc<ProxyIdentityVerifier>>,
    mut request: Request,
    next: Next,
) -> Response {
    match verifier.verify_headers(request.headers()) {
        Ok(identity) => {
            tracing::info!(
                auth_mode = identity.mode.as_str(),
                subject_hash = %identity.subject_hash,
                "hosted proxy identity verified"
            );
            request.extensions_mut().insert(identity);
            next.run(request).await
        }
        Err(error) => {
            warn!(
                auth_mode = HostedAuthMode::ProxySignedHeaders.as_str(),
                reason = error.reason(),
                "hosted proxy identity verification failed"
            );
            unauthorized_identity_response()
        }
    }
}

impl VerificationError {
    const fn reason(self) -> &'static str {
        match self {
            Self::MissingIdentity => "missing_identity_header",
            Self::MissingSignature => "missing_signature_header",
            Self::MalformedIdentity => "malformed_identity_envelope",
            Self::InvalidSignature => "invalid_signature",
            Self::StaleIdentity => "stale_identity_envelope",
        }
    }
}

fn unauthorized_identity_response() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [(header::CONTENT_TYPE, "application/json")],
        serde_json::json!({
            "success": false,
            "error": "hosted identity verification failed",
            "error_code": "UNAUTHORIZED"
        })
        .to_string(),
    )
        .into_response()
}

fn parse_header_name(name: &str, value: &str) -> Result<HeaderName> {
    HeaderName::from_bytes(value.trim().as_bytes()).map_err(|_| {
        DbtNovaError::InvalidParams(format!(
            "{name} must be a valid HTTP header name for proxy identity mode"
        ))
    })
}

fn read_proxy_identity_secret(path: &str) -> Result<Vec<u8>> {
    let path = path.trim();
    let metadata = std::fs::metadata(path).map_err(|error| {
        DbtNovaError::InvalidParams(format!(
            "DBT_NOVA_PROXY_IDENTITY_SECRET_FILE could not be read: {error}"
        ))
    })?;
    if !metadata.is_file() {
        return Err(DbtNovaError::InvalidParams(
            "DBT_NOVA_PROXY_IDENTITY_SECRET_FILE must point to a regular file".to_string(),
        ));
    }
    if metadata.len() > MAX_SECRET_FILE_BYTES as u64 {
        return Err(DbtNovaError::InvalidParams(format!(
            "DBT_NOVA_PROXY_IDENTITY_SECRET_FILE must be at most {MAX_SECRET_FILE_BYTES} bytes"
        )));
    }
    let mut secret = std::fs::read(path).map_err(|error| {
        DbtNovaError::InvalidParams(format!(
            "DBT_NOVA_PROXY_IDENTITY_SECRET_FILE could not be read: {error}"
        ))
    })?;
    trim_ascii_whitespace(&mut secret);
    if secret.len() < MIN_SECRET_BYTES {
        return Err(DbtNovaError::InvalidParams(format!(
            "DBT_NOVA_PROXY_IDENTITY_SECRET_FILE must contain at least {MIN_SECRET_BYTES} bytes of secret material"
        )));
    }
    Ok(secret)
}

fn header_value<'a>(
    headers: &'a HeaderMap,
    name: &HeaderName,
    max_bytes: usize,
) -> Option<&'a str> {
    let value = headers.get(name)?.to_str().ok()?;
    if value.trim().is_empty() || value.len() > max_bytes {
        return None;
    }
    Some(value)
}

fn decode_identity_envelope(value: &str) -> std::result::Result<JsonValue, VerificationError> {
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|_| VerificationError::MalformedIdentity)?;
    if decoded.len() > MAX_IDENTITY_HEADER_BYTES {
        return Err(VerificationError::MalformedIdentity);
    }
    let envelope: JsonValue =
        serde_json::from_slice(&decoded).map_err(|_| VerificationError::MalformedIdentity)?;
    if !envelope.is_object() {
        return Err(VerificationError::MalformedIdentity);
    }
    Ok(envelope)
}

fn envelope_iat(envelope: &JsonValue) -> std::result::Result<u64, VerificationError> {
    match envelope.get("iat") {
        Some(JsonValue::Number(number)) => {
            number.as_u64().ok_or(VerificationError::MalformedIdentity)
        }
        Some(JsonValue::String(value)) => value
            .parse::<u64>()
            .map_err(|_| VerificationError::MalformedIdentity),
        _ => Err(VerificationError::MalformedIdentity),
    }
}

fn verify_signature(
    identity_header_value: &[u8],
    signature_header_value: &str,
    secret: &[u8],
) -> std::result::Result<(), VerificationError> {
    let signature = signature_header_value
        .strip_prefix(SIGNATURE_PREFIX)
        .ok_or(VerificationError::InvalidSignature)?;
    let signature = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(signature)
        .map_err(|_| VerificationError::InvalidSignature)?;
    let mut mac =
        HmacSha256::new_from_slice(secret).map_err(|_| VerificationError::InvalidSignature)?;
    mac.update(identity_header_value);
    mac.verify_slice(&signature)
        .map_err(|_| VerificationError::InvalidSignature)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

fn trim_ascii_whitespace(bytes: &mut Vec<u8>) {
    while bytes.last().is_some_and(u8::is_ascii_whitespace) {
        bytes.pop();
    }
    let first_non_ws = bytes
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(bytes.len());
    if first_non_ws > 0 {
        bytes.drain(..first_non_ws);
    }
}

fn unix_now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

#[cfg(test)]
pub(crate) fn encode_proxy_identity_for_tests(value: &JsonValue) -> String {
    let bytes = serde_json::to_vec(value).expect("test identity JSON");
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

#[cfg(test)]
pub(crate) fn sign_proxy_identity_for_tests(secret: &[u8], identity_header_value: &str) -> String {
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts test keys");
    mac.update(identity_header_value.as_bytes());
    let signature = mac.finalize().into_bytes();
    format!(
        "{SIGNATURE_PREFIX}{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(signature)
    )
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderMap;
    use serde_json::json;

    use super::{
        HostedAuthMode, ProxyIdentityVerifier, VerificationError, encode_proxy_identity_for_tests,
        sign_proxy_identity_for_tests,
    };

    const SECRET: &[u8] = b"0123456789abcdef0123456789abcdef";

    fn verifier(max_age_secs: u64) -> ProxyIdentityVerifier {
        ProxyIdentityVerifier {
            identity_header: "x-nova-identity".parse().unwrap(),
            signature_header: "x-nova-signature".parse().unwrap(),
            subject_claim: "sub".to_string(),
            max_age: std::time::Duration::from_secs(max_age_secs),
            secret: std::sync::Arc::new(SECRET.to_vec()),
        }
    }

    fn signed_headers(identity: &str, signature: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert("x-nova-identity", identity.parse().unwrap());
        headers.insert("x-nova-signature", signature.parse().unwrap());
        headers
    }

    #[test]
    fn proxy_identity_accepts_valid_hmac_envelope() {
        let identity = encode_proxy_identity_for_tests(&json!({
            "sub": "user@example.com",
            "iat": 100,
        }));
        let signature = sign_proxy_identity_for_tests(SECRET, &identity);

        let verified = verifier(300)
            .verify_header_values_at(&identity, &signature, 120)
            .expect("identity should verify");

        assert_eq!(verified.mode, HostedAuthMode::ProxySignedHeaders);
        assert_eq!(verified.subject_hash.len(), 64);
        assert_ne!(verified.subject_hash, "user@example.com");
    }

    #[test]
    fn proxy_identity_rejects_invalid_signature() {
        let identity = encode_proxy_identity_for_tests(&json!({
            "sub": "user@example.com",
            "iat": 100,
        }));
        let signature =
            sign_proxy_identity_for_tests(b"abcdef0123456789abcdef0123456789", &identity);

        let error = verifier(300)
            .verify_header_values_at(&identity, &signature, 120)
            .expect_err("wrong secret should fail");

        assert_eq!(error, VerificationError::InvalidSignature);
    }

    #[test]
    fn proxy_identity_rejects_tampered_header_whitespace() {
        let identity = encode_proxy_identity_for_tests(&json!({
            "sub": "user@example.com",
            "iat": 100,
        }));
        let signature = sign_proxy_identity_for_tests(SECRET, &identity);
        let tampered_identity = format!(" {identity}");

        let error = verifier(300)
            .verify_header_values_at(&tampered_identity, &signature, 120)
            .expect_err("signature should cover exact header value");

        assert_eq!(error, VerificationError::InvalidSignature);
    }

    #[test]
    fn proxy_identity_rejects_stale_envelope() {
        let identity = encode_proxy_identity_for_tests(&json!({
            "sub": "user@example.com",
            "iat": 100,
        }));
        let signature = sign_proxy_identity_for_tests(SECRET, &identity);

        let error = verifier(30)
            .verify_header_values_at(&identity, &signature, 200)
            .expect_err("stale identity should fail");

        assert_eq!(error, VerificationError::StaleIdentity);
    }

    #[test]
    fn proxy_identity_rejects_missing_subject() {
        let identity = encode_proxy_identity_for_tests(&json!({
            "iat": 100,
        }));
        let signature = sign_proxy_identity_for_tests(SECRET, &identity);

        let error = verifier(300)
            .verify_header_values_at(&identity, &signature, 120)
            .expect_err("subject is required");

        assert_eq!(error, VerificationError::MalformedIdentity);
    }

    #[test]
    fn proxy_identity_rejects_oversized_header_as_missing() {
        let identity = "a".repeat(super::MAX_IDENTITY_HEADER_BYTES + 1);
        let signature = sign_proxy_identity_for_tests(SECRET, &identity);
        let headers = signed_headers(&identity, &signature);

        let error = verifier(300)
            .verify_headers_at(&headers, 120)
            .expect_err("oversized identity header should fail");

        assert_eq!(error, VerificationError::MissingIdentity);
    }
}
