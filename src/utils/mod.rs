pub mod circuit_breaker;
pub mod gcp_auth;
pub mod glob;
pub mod id;
pub mod metrics;
pub mod persona;
pub mod sql_structure;
pub mod storage;
pub mod string;
pub mod tool_trace;

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

const TEXT_REDACT_KEYS: [&str; 14] = [
    "token",
    "access_token",
    "apikey",
    "api_key",
    "secret",
    "password",
    "passwd",
    "pwd",
    "credential",
    "signature",
    "authorization",
    "auth",
    "session",
    "jwt",
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

/// Redact sensitive URI, token, and credential-shaped values from free-form text.
#[must_use]
pub fn redact_sensitive_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for (index, line) in text.split('\n').enumerate() {
        if index > 0 {
            out.push('\n');
        }
        out.push_str(&redact_sensitive_line(line));
    }
    out
}

fn redact_sensitive_line(line: &str) -> String {
    let lowercase = line.to_ascii_lowercase();
    if let Some(start) = lowercase.find("authorization")
        && lowercase[start..].starts_with("authorization")
        && let Some(delimiter_offset) = line[start..].find([':', '='])
    {
        let end = start + delimiter_offset + 1;
        return format!("{}[REDACTED]", &line[..end]);
    }

    let trimmed_start = line.len() - line.trim_start().len();
    if lowercase[trimmed_start..].starts_with("bearer ") {
        return format!("{}Bearer [REDACTED]", &line[..trimmed_start]);
    }

    let mut out = String::with_capacity(line.len());
    let mut token_start = None;
    for (index, ch) in line.char_indices() {
        if ch.is_whitespace() {
            if let Some(start) = token_start.take() {
                out.push_str(&redact_sensitive_token(&line[start..index]));
            }
            out.push(ch);
        } else if token_start.is_none() {
            token_start = Some(index);
        }
    }
    if let Some(start) = token_start {
        out.push_str(&redact_sensitive_token(&line[start..]));
    }
    out
}

fn redact_sensitive_token(token: &str) -> String {
    let (prefix, core, suffix) = split_token_wrappers(token);
    if core.is_empty() {
        return token.to_string();
    }

    let redacted = if core.contains("://") || core.contains(":/") || core.contains('/') {
        sanitize_uri(core)
    } else if let Some(redacted) = redact_sensitive_key_value(core) {
        redacted
    } else if core.contains('?') {
        sanitize_uri(core)
    } else {
        core.to_string()
    };
    format!("{prefix}{redacted}{suffix}")
}

fn split_token_wrappers(token: &str) -> (&str, &str, &str) {
    let start = token
        .char_indices()
        .find_map(|(index, ch)| (!is_leading_wrapper(ch)).then_some(index))
        .unwrap_or(token.len());
    let end = token[start..]
        .char_indices()
        .rev()
        .find_map(|(offset, ch)| {
            (!is_trailing_wrapper(ch)).then_some(start + offset + ch.len_utf8())
        })
        .unwrap_or(start);
    (&token[..start], &token[start..end], &token[end..])
}

fn is_leading_wrapper(ch: char) -> bool {
    matches!(ch, '"' | '\'' | '(' | '[' | '{' | '<')
}

fn is_trailing_wrapper(ch: char) -> bool {
    matches!(ch, '"' | '\'' | ')' | ']' | '}' | '>' | ',' | '.' | ';')
}

fn redact_sensitive_key_value(token: &str) -> Option<String> {
    for delimiter in ['=', ':'] {
        if let Some(index) = token.find(delimiter) {
            let key = token[..index]
                .trim_matches(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '-'));
            if is_sensitive_text_key(key) {
                return Some(format!("{}{delimiter}[REDACTED]", &token[..index]));
            }
        }
    }
    None
}

fn is_sensitive_text_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    TEXT_REDACT_KEYS
        .iter()
        .any(|sensitive| key.contains(sensitive))
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
