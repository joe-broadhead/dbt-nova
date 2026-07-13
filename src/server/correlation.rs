use std::time::{Duration, Instant};

use axum::{
    extract::Request,
    http::{HeaderMap, HeaderName, HeaderValue, Method, Uri},
    middleware::Next,
    response::Response,
};
use tokio::task_local;
use tracing::{Instrument, info, warn};

const REQUEST_ID_HEADER_NAME: &str = "x-request-id";
const CORRELATION_ID_HEADER_NAME: &str = "x-correlation-id";
const MAX_REQUEST_ID_LEN: usize = 128;

task_local! {
    static CURRENT_REQUEST_ID: RequestId;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RequestId(String);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RequestIdSource {
    Proxy,
    Generated,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RequestLogFields {
    pub request_id: String,
    pub request_id_source: &'static str,
    pub method: String,
    pub path: String,
}

impl RequestId {
    fn generated() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }

    #[must_use]
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl RequestIdSource {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Proxy => "proxy",
            Self::Generated => "generated",
        }
    }
}

pub(crate) fn current_request_id() -> Option<String> {
    CURRENT_REQUEST_ID
        .try_with(|request_id| request_id.as_str().to_string())
        .ok()
}

pub(crate) async fn correlate_http_request(mut request: Request, next: Next) -> Response {
    let started = Instant::now();
    let (request_id, source) = resolve_request_id(request.headers());
    let fields = RequestLogFields::new(request.method(), request.uri(), &request_id, source);
    request.extensions_mut().insert(request_id.clone());

    let span = tracing::info_span!(
        "hosted_http_request",
        request_id = %request_id.as_str(),
        request_id_source = fields.request_id_source,
        method = %fields.method,
        path = %fields.path,
    );

    let mut response = CURRENT_REQUEST_ID
        .scope(request_id.clone(), async move {
            next.run(request).instrument(span).await
        })
        .await;
    insert_response_request_id(response.headers_mut(), &request_id);
    log_request_completed(&fields, response.status().as_u16(), started.elapsed());
    response
}

pub(crate) fn resolve_request_id(headers: &HeaderMap) -> (RequestId, RequestIdSource) {
    for header_name in [REQUEST_ID_HEADER_NAME, CORRELATION_ID_HEADER_NAME] {
        if let Some(value) = headers.get(header_name)
            && let Ok(value) = value.to_str()
            && valid_request_id(value)
        {
            return (RequestId(value.trim().to_string()), RequestIdSource::Proxy);
        }
    }
    (RequestId::generated(), RequestIdSource::Generated)
}

pub(crate) fn insert_response_request_id(headers: &mut HeaderMap, request_id: &RequestId) {
    if let Ok(value) = HeaderValue::from_str(request_id.as_str()) {
        headers.insert(HeaderName::from_static(REQUEST_ID_HEADER_NAME), value);
    }
}

impl RequestLogFields {
    #[must_use]
    fn new(method: &Method, uri: &Uri, request_id: &RequestId, source: RequestIdSource) -> Self {
        Self {
            request_id: request_id.as_str().to_string(),
            request_id_source: source.as_str(),
            method: method.as_str().to_string(),
            path: uri.path().to_string(),
        }
    }
}

fn log_request_completed(fields: &RequestLogFields, status: u16, elapsed: Duration) {
    let duration_ms = elapsed_ms_to_u64(elapsed);
    if status >= 500 {
        warn!(
            request_id = %fields.request_id,
            request_id_source = fields.request_id_source,
            method = %fields.method,
            path = %fields.path,
            status,
            duration_ms,
            "hosted HTTP request completed with server error"
        );
    } else {
        info!(
            request_id = %fields.request_id,
            request_id_source = fields.request_id_source,
            method = %fields.method,
            path = %fields.path,
            status,
            duration_ms,
            "hosted HTTP request completed"
        );
    }
}

fn valid_request_id(value: &str) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty()
        && trimmed.len() <= MAX_REQUEST_ID_LEN
        && trimmed
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
}

fn elapsed_ms_to_u64(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use axum::http::{HeaderMap, Method, Uri};

    use super::{
        CORRELATION_ID_HEADER_NAME, REQUEST_ID_HEADER_NAME, RequestId, RequestIdSource,
        RequestLogFields, insert_response_request_id, resolve_request_id,
    };

    #[test]
    fn request_id_resolution_prefers_safe_proxy_header() {
        let mut headers = HeaderMap::new();
        headers.insert(REQUEST_ID_HEADER_NAME, "req-123.trace".parse().unwrap());
        headers.insert(CORRELATION_ID_HEADER_NAME, "corr-456".parse().unwrap());

        let (request_id, source) = resolve_request_id(&headers);

        assert_eq!(request_id.as_str(), "req-123.trace");
        assert_eq!(source, RequestIdSource::Proxy);
    }

    #[test]
    fn request_id_resolution_falls_back_to_correlation_header() {
        let mut headers = HeaderMap::new();
        headers.insert(CORRELATION_ID_HEADER_NAME, "corr-456".parse().unwrap());

        let (request_id, source) = resolve_request_id(&headers);

        assert_eq!(request_id.as_str(), "corr-456");
        assert_eq!(source, RequestIdSource::Proxy);
    }

    #[test]
    fn request_id_resolution_rejects_sensitive_or_unsafe_header_values() {
        let mut headers = HeaderMap::new();
        headers.insert(REQUEST_ID_HEADER_NAME, "Bearer raw-token".parse().unwrap());

        let (request_id, source) = resolve_request_id(&headers);

        assert_ne!(request_id.as_str(), "Bearer raw-token");
        assert_eq!(source, RequestIdSource::Generated);
        assert_eq!(request_id.as_str().len(), 36);
    }

    #[test]
    fn response_request_id_header_uses_resolved_id() {
        let mut headers = HeaderMap::new();
        let request_id = RequestId("req-123".to_string());

        insert_response_request_id(&mut headers, &request_id);

        assert_eq!(
            headers
                .get(REQUEST_ID_HEADER_NAME)
                .and_then(|v| v.to_str().ok()),
            Some("req-123")
        );
    }

    #[test]
    fn request_log_fields_drop_query_strings() {
        let request_id = RequestId("req-123".to_string());
        let uri: Uri = "/mcp?token=raw-token&query=select%20secret"
            .parse()
            .unwrap();

        let fields =
            RequestLogFields::new(&Method::POST, &uri, &request_id, RequestIdSource::Proxy);

        assert_eq!(fields.path, "/mcp");
        assert!(!fields.path.contains("raw-token"));
        assert!(!fields.path.contains("select"));
    }
}
