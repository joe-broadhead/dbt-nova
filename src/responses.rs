use serde::Serialize;
use serde_json::Value as JsonValue;

pub const RESPONSE_ENVELOPE_ID: &str = "nova.response.v1";

/// Additive API contract marker for Nova-owned JSON response envelopes.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct ApiContract {
    pub envelope: &'static str,
    pub nova_version: &'static str,
}

#[must_use]
pub const fn response_api_contract() -> ApiContract {
    ApiContract {
        envelope: RESPONSE_ENVELOPE_ID,
        nova_version: env!("CARGO_PKG_VERSION"),
    }
}

pub fn attach_response_api_contract(value: &mut JsonValue) {
    if let Some(obj) = value.as_object_mut() {
        obj.entry("api".to_string())
            .or_insert_with(|| serde_json::json!(response_api_contract()));
    }
}

#[derive(Serialize)]
pub struct PaginationInfo {
    pub success: bool,
    pub count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_available: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncated: Option<bool>,
}

impl PaginationInfo {
    #[must_use]
    pub fn new(count: usize) -> Self {
        Self {
            success: true,
            count,
            total_available: None,
            truncated: None,
        }
    }
}

/// Standard success response wrapper for most tools.
#[derive(Serialize)]
pub struct SuccessResponse<T: Serialize> {
    #[serde(flatten)]
    pub pagination: PaginationInfo,
    pub data: T,
}

impl<T: Serialize> SuccessResponse<T> {
    /// Create a response with required data and count.
    #[must_use]
    pub fn new(data: T, count: usize) -> Self {
        Self {
            pagination: PaginationInfo::new(count),
            data,
        }
    }

    /// Attach the total available item count when known.
    #[must_use]
    pub fn with_total(mut self, total: usize) -> Self {
        self.pagination.total_available = Some(total);
        self
    }

    /// Mark the response as truncated when limits apply.
    #[must_use]
    pub fn with_truncated(mut self, truncated: bool) -> Self {
        self.pagination.truncated = Some(truncated);
        self
    }
}

/// Success response wrapper for search, with persona and suggestions.
#[derive(Serialize)]
pub struct SearchResponse<T: Serialize> {
    #[serde(flatten)]
    pub pagination: PaginationInfo,
    pub persona: String,
    pub suggestions: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub analysis_hints: Option<Vec<String>>,
    pub data: T,
}

impl<T: Serialize> SearchResponse<T> {
    /// Create a search response with required data and count.
    #[must_use]
    pub fn new(data: T, count: usize, persona: String) -> Self {
        Self {
            pagination: PaginationInfo::new(count),
            persona,
            suggestions: Vec::new(),
            analysis_hints: None,
            data,
        }
    }

    /// Attach the total available item count when known.
    #[must_use]
    pub fn with_total(mut self, total: usize) -> Self {
        self.pagination.total_available = Some(total);
        self
    }

    /// Mark the response as truncated when limits apply.
    #[must_use]
    pub fn with_truncated(mut self, truncated: bool) -> Self {
        self.pagination.truncated = Some(truncated);
        self
    }

    /// Attach query suggestions for the search response.
    #[must_use]
    pub fn with_suggestions(mut self, suggestions: Vec<String>) -> Self {
        self.suggestions = suggestions;
        self
    }

    /// Attach analyst guidance hints for ambiguous ranking outcomes.
    #[must_use]
    pub fn with_analysis_hints(mut self, analysis_hints: Vec<String>) -> Self {
        self.analysis_hints = Some(analysis_hints);
        self
    }
}
