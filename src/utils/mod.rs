pub mod circuit_breaker;
pub mod gcp_auth;
pub mod glob;
pub mod id;
pub mod metrics;
pub mod persona;
pub mod storage;
pub mod string;

pub use circuit_breaker::{CircuitBreaker, CircuitBreakerState};
pub use gcp_auth::{
    resolve_gcp_access_token, resolve_gcp_access_token_async, resolve_gcp_project_id,
};
pub use glob::{GlobPattern, compile_glob, glob_match, glob_match_compiled, glob_match_simple};
pub use id::unique_suffix;
pub use metrics::{ToolMetricsStore, ToolRateLimiter};
pub use persona::SearchPersona;
pub use storage::{IN_USE_LOCK_FILENAME, dir_in_use, prune_dirs};
pub use string::{
    has_query_syntax, levenshtein_distance, levenshtein_similarity, tokenize_alnum_lowercase,
};

const URI_REDACT_KEYS: [&str; 8] = [
    "token",
    "access_token",
    "apikey",
    "api_key",
    "secret",
    "password",
    "passwd",
    "pwd",
];

/// Sanitize URIs to avoid leaking credentials in logs.
#[must_use]
pub fn sanitize_uri(uri: &str) -> String {
    let trimmed = uri.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let (base, has_query) = match trimmed.find('?') {
        Some(pos) => (&trimmed[..pos], true),
        None => (trimmed, false),
    };
    let base = match base.find('#') {
        Some(pos) => &base[..pos],
        None => base,
    };

    let (scheme, rest) = match base.find("://") {
        Some(pos) => (&base[..(pos + 3)], &base[(pos + 3)..]),
        None => ("", base),
    };

    let (authority, path) = match rest.find('/') {
        Some(pos) => (&rest[..pos], &rest[pos..]),
        None => (rest, ""),
    };

    let mut sanitized_authority = authority.to_string();
    if let Some(at_pos) = authority.rfind('@') {
        sanitized_authority = format!("[REDACTED]@{}", &authority[(at_pos + 1)..]);
    }

    let sanitized_path = sanitize_uri_path(path);

    let mut out = String::new();
    out.push_str(scheme);
    out.push_str(&sanitized_authority);
    out.push_str(&sanitized_path);

    if has_query {
        out.push_str("?[REDACTED]");
    }

    out
}

fn sanitize_uri_path(path: &str) -> String {
    if path.is_empty() {
        return String::new();
    }

    let mut segments: Vec<String> = path.split('/').map(ToString::to_string).collect();
    let mut idx = 0usize;
    while idx < segments.len() {
        let seg = segments[idx].clone();
        let seg_lower = seg.to_lowercase();
        let mut replacement = None;

        for key in &URI_REDACT_KEYS {
            if seg_lower == *key && idx + 1 < segments.len() {
                segments[idx + 1] = "[REDACTED]".to_string();
            }

            if seg_lower.contains(key) {
                if let Some(pos) = seg.find('=') {
                    let (k, _) = seg.split_at(pos + 1);
                    replacement = Some(format!("{k}[REDACTED]"));
                    break;
                }
                if let Some(pos) = seg.find(':') {
                    let (k, _) = seg.split_at(pos + 1);
                    replacement = Some(format!("{k}[REDACTED]"));
                    break;
                }
            }
        }

        if let Some(repl) = replacement {
            segments[idx] = repl;
        }

        idx += 1;
    }

    segments.join("/")
}
