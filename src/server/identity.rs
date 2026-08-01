use std::str::FromStr;
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::{
    extract::{Request, State},
    http::{HeaderMap, HeaderName, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use base64::Engine as _;
use hmac::{Hmac, Mac};
use jsonwebtoken::{
    Algorithm, DecodingKey, Validation, decode, decode_header,
    errors::ErrorKind as JwtErrorKind,
    jwk::{AlgorithmParameters, Jwk, JwkSet, KeyOperations, PublicKeyUse},
};
use serde_json::Value as JsonValue;
use sha2::{Digest, Sha256};
use tracing::warn;

use crate::config::{HostedAuthConfig, HostedAuthMode, parse_jwt_algorithms};
use crate::error::{DbtNovaError, Result};
use crate::utils::http_client::{async_client_builder, blocking_client_builder};

const SIGNATURE_PREFIX: &str = "sha256=";
const MAX_IDENTITY_HEADER_BYTES: usize = 8 * 1024;
const MAX_SIGNATURE_HEADER_BYTES: usize = 256;
const MAX_SECRET_FILE_BYTES: usize = 64 * 1024;
const MIN_SECRET_BYTES: usize = 32;
const FUTURE_IAT_SKEW_SECS: u64 = 60;
const MAX_AUTHORIZATION_HEADER_BYTES: usize = 16 * 1024;
const MAX_JWKS_RESPONSE_BYTES: usize = 256 * 1024;
const JWKS_FETCH_TIMEOUT_SECS: u64 = 10;

type HmacSha256 = Hmac<Sha256>;

/// Sanitized request identity verified from trusted proxy headers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RequestIdentity {
    pub mode: HostedAuthMode,
    pub subject_hash: String,
}

#[derive(Debug)]
pub(crate) enum HostedIdentityVerifier {
    Proxy(ProxyIdentityVerifier),
    Jwt(Box<JwtIdentityVerifier>),
}

#[derive(Debug, Clone)]
pub(crate) struct ProxyIdentityVerifier {
    identity_header: HeaderName,
    signature_header: HeaderName,
    subject_claim: String,
    max_age: Duration,
    secret: Arc<Vec<u8>>,
}

#[derive(Debug)]
pub(crate) struct JwtIdentityVerifier {
    subject_claim: String,
    jwks_url: String,
    algorithms: Vec<Algorithm>,
    validation: Validation,
    jwks: RwLock<JwkSet>,
    client: reqwest::Client,
}

#[derive(Debug)]
struct JwtIdentityVerifierParts {
    issuer: String,
    audience: String,
    subject_claim: String,
    jwks_url: String,
    algorithms: Vec<Algorithm>,
    clock_skew_secs: u64,
    jwks: JwkSet,
    client: reqwest::Client,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VerificationError {
    MissingIdentity,
    MissingSignature,
    MissingAuthorization,
    MalformedAuthorization,
    MalformedIdentity,
    MissingKeyId,
    UnknownKeyId,
    UnsupportedAlgorithm,
    InvalidSignature,
    StaleIdentity,
    InvalidToken,
    JwksUnavailable,
}

impl HostedIdentityVerifier {
    /// Build the active hosted identity verifier for the configured mode.
    ///
    /// # Errors
    /// Returns an error when verifier config or key material cannot be loaded.
    pub(crate) fn from_config(config: &HostedAuthConfig) -> Result<Option<Self>> {
        match config.mode {
            HostedAuthMode::Off => Ok(None),
            HostedAuthMode::ProxySignedHeaders => {
                ProxyIdentityVerifier::from_config(config).map(|verifier| verifier.map(Self::Proxy))
            }
            HostedAuthMode::Jwt => JwtIdentityVerifier::from_config(config)
                .map(|verifier| verifier.map(|verifier| Self::Jwt(Box::new(verifier)))),
        }
    }

    const fn mode(&self) -> HostedAuthMode {
        match self {
            Self::Proxy(_) => HostedAuthMode::ProxySignedHeaders,
            Self::Jwt(_) => HostedAuthMode::Jwt,
        }
    }

    async fn verify_headers(
        &self,
        headers: &HeaderMap,
    ) -> std::result::Result<RequestIdentity, VerificationError> {
        match self {
            Self::Proxy(verifier) => verifier.verify_headers(headers),
            Self::Jwt(verifier) => verifier.verify_headers(headers).await,
        }
    }
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

impl JwtIdentityVerifier {
    /// Build a JWT verifier from validated hosted auth config.
    ///
    /// # Errors
    /// Returns an error when JWT algorithms are unsupported or JWKS cannot be loaded.
    pub(crate) fn from_config(config: &HostedAuthConfig) -> Result<Option<Self>> {
        if config.mode != HostedAuthMode::Jwt {
            return Ok(None);
        }
        let algorithms = parse_jwt_algorithms(&config.jwt_algorithms)?;
        let startup_client = build_blocking_jwks_client()?;
        let jwks_url = config.jwt_jwks_url.trim().to_string();
        let jwks = fetch_jwks_blocking(&startup_client, &jwks_url)?;
        validate_jwks_has_asymmetric_key(&jwks)?;
        let client = build_jwks_client()?;
        Ok(Some(Self::from_parts(JwtIdentityVerifierParts {
            issuer: config.jwt_issuer.trim().to_string(),
            audience: config.jwt_audience.trim().to_string(),
            subject_claim: config.identity_subject_claim.trim().to_string(),
            jwks_url,
            algorithms,
            clock_skew_secs: config.jwt_clock_skew_secs,
            jwks,
            client,
        })))
    }

    fn from_parts(parts: JwtIdentityVerifierParts) -> Self {
        let JwtIdentityVerifierParts {
            issuer,
            audience,
            subject_claim,
            jwks_url,
            algorithms,
            clock_skew_secs,
            jwks,
            client,
        } = parts;
        let mut validation = Validation::new(algorithms[0]);
        validation.algorithms.clone_from(&algorithms);
        validation.leeway = clock_skew_secs;
        validation.validate_exp = true;
        validation.validate_nbf = true;
        validation.set_audience(&[audience.as_str()]);
        validation.set_issuer(&[issuer.as_str()]);
        validation.set_required_spec_claims(&["exp", "nbf", "aud", "iss"]);
        Self {
            subject_claim,
            jwks_url,
            algorithms,
            validation,
            jwks: RwLock::new(jwks),
            client,
        }
    }

    async fn verify_headers(
        &self,
        headers: &HeaderMap,
    ) -> std::result::Result<RequestIdentity, VerificationError> {
        let token = bearer_token(headers)?;
        match self.verify_token_with_cache(token) {
            Ok(identity) => Ok(identity),
            Err(VerificationError::UnknownKeyId | VerificationError::InvalidSignature) => {
                self.refresh_jwks().await?;
                self.verify_token_with_cache(token)
            }
            Err(error) => Err(error),
        }
    }

    fn verify_token_with_cache(
        &self,
        token: &str,
    ) -> std::result::Result<RequestIdentity, VerificationError> {
        let header = decode_header(token).map_err(|_| VerificationError::MalformedAuthorization)?;
        if !self.algorithms.contains(&header.alg) {
            return Err(VerificationError::UnsupportedAlgorithm);
        }
        let kid = header
            .kid
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or(VerificationError::MissingKeyId)?;
        let jwk = self.cached_jwk(kid)?;
        validate_jwk_for_header(&jwk, header.alg)?;
        let decoding_key =
            DecodingKey::from_jwk(&jwk).map_err(|_| VerificationError::InvalidToken)?;
        let token_data = decode::<JsonValue>(token, &decoding_key, &self.validation)
            .map_err(|error| jwt_error_to_verification_error(&error))?;
        let subject = token_data
            .claims
            .get(&self.subject_claim)
            .and_then(JsonValue::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or(VerificationError::MalformedIdentity)?;
        Ok(RequestIdentity {
            mode: HostedAuthMode::Jwt,
            subject_hash: sha256_hex(subject.as_bytes()),
        })
    }

    fn cached_jwk(&self, kid: &str) -> std::result::Result<Jwk, VerificationError> {
        let jwks = self
            .jwks
            .read()
            .map_err(|_| VerificationError::JwksUnavailable)?;
        jwks.find(kid)
            .cloned()
            .ok_or(VerificationError::UnknownKeyId)
    }

    async fn refresh_jwks(&self) -> std::result::Result<(), VerificationError> {
        let refreshed = fetch_jwks(&self.client, &self.jwks_url)
            .await
            .map_err(|_| VerificationError::JwksUnavailable)?;
        validate_jwks_has_asymmetric_key(&refreshed)
            .map_err(|_| VerificationError::JwksUnavailable)?;
        let mut jwks = self
            .jwks
            .write()
            .map_err(|_| VerificationError::JwksUnavailable)?;
        *jwks = refreshed;
        Ok(())
    }
}

pub(crate) async fn verify_hosted_identity_request(
    State(verifier): State<Arc<HostedIdentityVerifier>>,
    mut request: Request,
    next: Next,
) -> Response {
    match verifier.verify_headers(request.headers()).await {
        Ok(identity) => {
            tracing::info!(
                auth_mode = identity.mode.as_str(),
                subject_hash = %identity.subject_hash,
                "hosted identity verified"
            );
            request.extensions_mut().insert(identity);
            next.run(request).await
        }
        Err(error) => {
            warn!(
                auth_mode = verifier.mode().as_str(),
                reason = error.reason(),
                "hosted identity verification failed"
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
            Self::MissingAuthorization => "missing_authorization_header",
            Self::MalformedAuthorization => "malformed_authorization_header",
            Self::MalformedIdentity => "malformed_identity_envelope",
            Self::MissingKeyId => "missing_jwt_kid",
            Self::UnknownKeyId => "unknown_jwt_kid",
            Self::UnsupportedAlgorithm => "unsupported_jwt_algorithm",
            Self::InvalidSignature => "invalid_signature",
            Self::StaleIdentity => "stale_identity",
            Self::InvalidToken => "invalid_jwt",
            Self::JwksUnavailable => "jwks_unavailable",
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

fn bearer_token(headers: &HeaderMap) -> std::result::Result<&str, VerificationError> {
    let value = header_value(
        headers,
        &header::AUTHORIZATION,
        MAX_AUTHORIZATION_HEADER_BYTES,
    )
    .ok_or(VerificationError::MissingAuthorization)?;
    let (scheme, token) = value
        .split_once(' ')
        .ok_or(VerificationError::MalformedAuthorization)?;
    if !scheme.eq_ignore_ascii_case("Bearer") {
        return Err(VerificationError::MalformedAuthorization);
    }
    let token = token.trim();
    if token.is_empty() || token.contains(char::is_whitespace) {
        return Err(VerificationError::MalformedAuthorization);
    }
    Ok(token)
}

fn build_jwks_client() -> Result<reqwest::Client> {
    async_client_builder()?
        .timeout(Duration::from_secs(JWKS_FETCH_TIMEOUT_SECS))
        .build()
        .map_err(|error| {
            DbtNovaError::InvalidParams(format!(
                "DBT_NOVA_JWT_JWKS_URL client could not be created: {error}"
            ))
        })
}

fn build_blocking_jwks_client() -> Result<reqwest::blocking::Client> {
    blocking_client_builder()?
        .timeout(Duration::from_secs(JWKS_FETCH_TIMEOUT_SECS))
        .build()
        .map_err(|error| {
            DbtNovaError::InvalidParams(format!(
                "DBT_NOVA_JWT_JWKS_URL client could not be created: {error}"
            ))
        })
}

fn fetch_jwks_blocking(client: &reqwest::blocking::Client, url: &str) -> Result<JwkSet> {
    let response = client
        .get(url.trim())
        .header(header::ACCEPT.as_str(), "application/json")
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(|error| {
            DbtNovaError::InvalidParams(format!(
                "DBT_NOVA_JWT_JWKS_URL could not be fetched: {error}"
            ))
        })?;
    if response
        .content_length()
        .is_some_and(|length| length > MAX_JWKS_RESPONSE_BYTES as u64)
    {
        return Err(DbtNovaError::InvalidParams(format!(
            "DBT_NOVA_JWT_JWKS_URL response must be at most {MAX_JWKS_RESPONSE_BYTES} bytes"
        )));
    }
    let bytes = response.bytes().map_err(|error| {
        DbtNovaError::InvalidParams(format!(
            "DBT_NOVA_JWT_JWKS_URL response could not be read: {error}"
        ))
    })?;
    if bytes.len() > MAX_JWKS_RESPONSE_BYTES {
        return Err(DbtNovaError::InvalidParams(format!(
            "DBT_NOVA_JWT_JWKS_URL response must be at most {MAX_JWKS_RESPONSE_BYTES} bytes"
        )));
    }
    serde_json::from_slice::<JwkSet>(&bytes).map_err(|error| {
        DbtNovaError::InvalidParams(format!(
            "DBT_NOVA_JWT_JWKS_URL response was not a valid JWKS: {error}"
        ))
    })
}

async fn fetch_jwks(client: &reqwest::Client, url: &str) -> Result<JwkSet> {
    let response = client
        .get(url.trim())
        .header(header::ACCEPT.as_str(), "application/json")
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|error| {
            DbtNovaError::InvalidParams(format!(
                "DBT_NOVA_JWT_JWKS_URL could not be fetched: {error}"
            ))
        })?;
    if response
        .content_length()
        .is_some_and(|length| length > MAX_JWKS_RESPONSE_BYTES as u64)
    {
        return Err(DbtNovaError::InvalidParams(format!(
            "DBT_NOVA_JWT_JWKS_URL response must be at most {MAX_JWKS_RESPONSE_BYTES} bytes"
        )));
    }
    let bytes = response.bytes().await.map_err(|error| {
        DbtNovaError::InvalidParams(format!(
            "DBT_NOVA_JWT_JWKS_URL response could not be read: {error}"
        ))
    })?;
    if bytes.len() > MAX_JWKS_RESPONSE_BYTES {
        return Err(DbtNovaError::InvalidParams(format!(
            "DBT_NOVA_JWT_JWKS_URL response must be at most {MAX_JWKS_RESPONSE_BYTES} bytes"
        )));
    }
    serde_json::from_slice::<JwkSet>(&bytes).map_err(|error| {
        DbtNovaError::InvalidParams(format!(
            "DBT_NOVA_JWT_JWKS_URL response was not a valid JWKS: {error}"
        ))
    })
}

fn validate_jwks_has_asymmetric_key(jwks: &JwkSet) -> Result<()> {
    if jwks.keys.iter().any(is_asymmetric_jwk) {
        return Ok(());
    }
    Err(DbtNovaError::InvalidParams(
        "DBT_NOVA_JWT_JWKS_URL must contain at least one asymmetric signature key".to_string(),
    ))
}

fn is_asymmetric_jwk(jwk: &Jwk) -> bool {
    matches!(
        jwk.algorithm,
        AlgorithmParameters::RSA(_)
            | AlgorithmParameters::EllipticCurve(_)
            | AlgorithmParameters::OctetKeyPair(_)
    )
}

fn validate_jwk_for_header(
    jwk: &Jwk,
    header_alg: Algorithm,
) -> std::result::Result<(), VerificationError> {
    if !is_asymmetric_jwk(jwk) {
        return Err(VerificationError::UnsupportedAlgorithm);
    }
    if let Some(use_) = &jwk.common.public_key_use
        && use_ != &PublicKeyUse::Signature
    {
        return Err(VerificationError::InvalidToken);
    }
    if let Some(key_ops) = &jwk.common.key_operations
        && !key_ops
            .iter()
            .any(|operation| operation == &KeyOperations::Verify)
    {
        return Err(VerificationError::InvalidToken);
    }
    if let Some(key_algorithm) = jwk.common.key_algorithm {
        let key_algorithm = Algorithm::from_str(&key_algorithm.to_string())
            .map_err(|_| VerificationError::UnsupportedAlgorithm)?;
        if key_algorithm != header_alg {
            return Err(VerificationError::UnsupportedAlgorithm);
        }
    }
    Ok(())
}

fn jwt_error_to_verification_error(error: &jsonwebtoken::errors::Error) -> VerificationError {
    match error.kind() {
        JwtErrorKind::ExpiredSignature | JwtErrorKind::ImmatureSignature => {
            VerificationError::StaleIdentity
        }
        JwtErrorKind::InvalidSignature => VerificationError::InvalidSignature,
        JwtErrorKind::InvalidAlgorithm | JwtErrorKind::MissingAlgorithm => {
            VerificationError::UnsupportedAlgorithm
        }
        _ => VerificationError::InvalidToken,
    }
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
    use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::{
        HostedAuthMode, JwtIdentityVerifier, JwtIdentityVerifierParts, ProxyIdentityVerifier,
        VerificationError, build_blocking_jwks_client, build_jwks_client,
        encode_proxy_identity_for_tests, fetch_jwks_blocking, parse_jwt_algorithms,
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

    const PRIVATE_ED25519_KEY_PEM: &str = "\
-----BEGIN PRIVATE KEY-----\n\
MC4CAQAwBQYDK2VwBCIEIGrD/e7uKYqSY4twDEsRfMMuLSrODf14dpTiTK6K1YI0\n\
-----END PRIVATE KEY-----\n";

    const OTHER_PRIVATE_ED25519_KEY_PEM: &str = "\
-----BEGIN PRIVATE KEY-----\n\
MC4CAQAwBQYDK2VwBCIEIEKk6M0gfjMZ4Fd3gv9+78epAFf/OIMWfcospPeL6oyH\n\
-----END PRIVATE KEY-----\n";

    fn jwt_jwks(kid: &str) -> jsonwebtoken::jwk::JwkSet {
        serde_json::from_value(json!({
            "keys": [{
                "kty": "OKP",
                "use": "sig",
                "crv": "Ed25519",
                "x": "2-Jj2UvNCvQiUPNYRgSi0cJSPiJI6Rs6D0UTeEpQVj8",
                "kid": kid,
                "alg": "EdDSA"
            }]
        }))
        .expect("test JWKS")
    }

    fn jwt_verifier(jwks: jsonwebtoken::jwk::JwkSet) -> JwtIdentityVerifier {
        JwtIdentityVerifier::from_parts(JwtIdentityVerifierParts {
            issuer: "https://issuer.example".to_string(),
            audience: "dbt-nova".to_string(),
            subject_claim: "sub".to_string(),
            jwks_url: "https://issuer.example/.well-known/jwks.json".to_string(),
            algorithms: vec![Algorithm::EdDSA],
            clock_skew_secs: 0,
            jwks,
            client: build_jwks_client().expect("JWKS client"),
        })
    }

    fn jwt_claims(
        sub: &str,
        issuer: &str,
        audience: &str,
        nbf: u64,
        exp: u64,
    ) -> serde_json::Value {
        json!({
            "sub": sub,
            "iss": issuer,
            "aud": audience,
            "nbf": nbf,
            "exp": exp,
        })
    }

    fn signed_jwt(kid: &str, claims: &serde_json::Value) -> String {
        signed_jwt_with_key(kid, claims, PRIVATE_ED25519_KEY_PEM)
    }

    fn signed_jwt_with_key(kid: &str, claims: &serde_json::Value, private_key_pem: &str) -> String {
        let mut header = Header::new(Algorithm::EdDSA);
        header.kid = Some(kid.to_string());
        encode(
            &header,
            claims,
            &EncodingKey::from_ed_pem(private_key_pem.as_bytes()).expect("test Ed25519 key"),
        )
        .expect("test JWT")
    }

    fn authorization_headers(token: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {token}").parse().unwrap(),
        );
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

    #[tokio::test]
    async fn jwt_identity_rejects_missing_bearer_token() {
        let verifier = jwt_verifier(jwt_jwks("ed01"));
        let error = verifier
            .verify_headers(&HeaderMap::new())
            .await
            .expect_err("JWT mode requires bearer authorization");

        assert_eq!(error, VerificationError::MissingAuthorization);
    }

    #[test]
    fn jwt_identity_accepts_valid_eddsa_jwks_token() {
        let verifier = jwt_verifier(jwt_jwks("ed01"));
        let claims = jwt_claims(
            "user@example.com",
            "https://issuer.example",
            "dbt-nova",
            super::unix_now_secs().saturating_sub(10),
            super::unix_now_secs() + 3600,
        );
        let token = signed_jwt("ed01", &claims);

        let identity = verifier
            .verify_token_with_cache(&token)
            .expect("JWT should verify");

        assert_eq!(identity.mode, HostedAuthMode::Jwt);
        assert_eq!(identity.subject_hash.len(), 64);
        assert_ne!(identity.subject_hash, "user@example.com");
    }

    #[test]
    fn jwt_identity_rejects_wrong_audience() {
        let verifier = jwt_verifier(jwt_jwks("ed01"));
        let claims = jwt_claims(
            "user@example.com",
            "https://issuer.example",
            "other-service",
            super::unix_now_secs().saturating_sub(10),
            super::unix_now_secs() + 3600,
        );
        let token = signed_jwt("ed01", &claims);

        let error = verifier
            .verify_token_with_cache(&token)
            .expect_err("wrong audience should fail");

        assert_eq!(error, VerificationError::InvalidToken);
    }

    #[test]
    fn jwt_identity_rejects_wrong_issuer() {
        let verifier = jwt_verifier(jwt_jwks("ed01"));
        let claims = jwt_claims(
            "user@example.com",
            "https://issuer.invalid",
            "dbt-nova",
            super::unix_now_secs().saturating_sub(10),
            super::unix_now_secs() + 3600,
        );
        let token = signed_jwt("ed01", &claims);

        let error = verifier
            .verify_token_with_cache(&token)
            .expect_err("wrong issuer should fail");

        assert_eq!(error, VerificationError::InvalidToken);
    }

    #[test]
    fn jwt_identity_rejects_expired_token() {
        let verifier = jwt_verifier(jwt_jwks("ed01"));
        let claims = jwt_claims(
            "user@example.com",
            "https://issuer.example",
            "dbt-nova",
            super::unix_now_secs().saturating_sub(120),
            super::unix_now_secs().saturating_sub(60),
        );
        let token = signed_jwt("ed01", &claims);

        let error = verifier
            .verify_token_with_cache(&token)
            .expect_err("expired token should fail");

        assert_eq!(error, VerificationError::StaleIdentity);
    }

    #[test]
    fn jwt_identity_rejects_not_yet_valid_token() {
        let verifier = jwt_verifier(jwt_jwks("ed01"));
        let claims = jwt_claims(
            "user@example.com",
            "https://issuer.example",
            "dbt-nova",
            super::unix_now_secs() + 60,
            super::unix_now_secs() + 3600,
        );
        let token = signed_jwt("ed01", &claims);

        let error = verifier
            .verify_token_with_cache(&token)
            .expect_err("future nbf token should fail");

        assert_eq!(error, VerificationError::StaleIdentity);
    }

    #[test]
    fn jwt_identity_rejects_unknown_kid() {
        let verifier = jwt_verifier(jwt_jwks("other"));
        let claims = jwt_claims(
            "user@example.com",
            "https://issuer.example",
            "dbt-nova",
            super::unix_now_secs().saturating_sub(10),
            super::unix_now_secs() + 3600,
        );
        let token = signed_jwt("ed01", &claims);

        let error = verifier
            .verify_token_with_cache(&token)
            .expect_err("unknown kid should fail");

        assert_eq!(error, VerificationError::UnknownKeyId);
    }

    #[test]
    fn jwt_identity_rejects_bad_signature() {
        let verifier = jwt_verifier(jwt_jwks("ed01"));
        let claims = jwt_claims(
            "user@example.com",
            "https://issuer.example",
            "dbt-nova",
            super::unix_now_secs().saturating_sub(10),
            super::unix_now_secs() + 3600,
        );
        let token = signed_jwt_with_key("ed01", &claims, OTHER_PRIVATE_ED25519_KEY_PEM);

        let error = verifier
            .verify_token_with_cache(&token)
            .expect_err("bad signature should fail");

        assert_eq!(error, VerificationError::InvalidSignature);
    }

    #[test]
    fn jwt_identity_rejects_hmac_algorithm_config() {
        let error = parse_jwt_algorithms(&["HS256".to_string()])
            .expect_err("HMAC JWT algorithms should be rejected");

        assert!(error.to_string().contains("HS256"));
    }

    #[test]
    fn jwt_identity_reports_jwks_fetch_outage() {
        let client = build_blocking_jwks_client().expect("JWKS client");
        let error = fetch_jwks_blocking(&client, "http://127.0.0.1:9/jwks")
            .expect_err("outage should fail");

        assert!(error.to_string().contains("DBT_NOVA_JWT_JWKS_URL"));
    }

    #[tokio::test]
    async fn jwt_identity_refreshes_jwks_on_unknown_kid() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/jwks"))
            .respond_with(ResponseTemplate::new(200).set_body_json(jwt_jwks("ed01")))
            .expect(1)
            .mount(&server)
            .await;
        let verifier = JwtIdentityVerifier::from_parts(JwtIdentityVerifierParts {
            issuer: "https://issuer.example".to_string(),
            audience: "dbt-nova".to_string(),
            subject_claim: "sub".to_string(),
            jwks_url: format!("{}/jwks", server.uri()),
            algorithms: vec![Algorithm::EdDSA],
            clock_skew_secs: 0,
            jwks: jwt_jwks("old"),
            client: build_jwks_client().expect("JWKS client"),
        });
        let claims = jwt_claims(
            "user@example.com",
            "https://issuer.example",
            "dbt-nova",
            super::unix_now_secs().saturating_sub(10),
            super::unix_now_secs() + 3600,
        );
        let token = signed_jwt("ed01", &claims);
        let headers = authorization_headers(&token);

        let identity = verifier
            .verify_headers(&headers)
            .await
            .expect("refresh should load matching key");

        assert_eq!(identity.mode, HostedAuthMode::Jwt);
    }
}
