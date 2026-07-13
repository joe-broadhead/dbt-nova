use serde::{Deserialize, Serialize};
use tracing_subscriber::fmt::format::FmtSpan;

/// Runtime log output format.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum LogFormat {
    /// Human-readable stderr logs.
    #[default]
    Human,
    /// Newline-delimited JSON logs for hosted collectors.
    Json,
}

impl LogFormat {
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "human" => Some(Self::Human),
            "json" => Some(Self::Json),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Human => "human",
            Self::Json => "json",
        }
    }
}

/// Initialize tracing from environment variables.
///
/// Logging remains opt-in through `DBT_NOVA_LOG` or `RUST_LOG`. When enabled,
/// `DBT_NOVA_LOG_FORMAT=json` switches to collector-friendly JSON output.
pub fn init_tracing_from_env() {
    let Some(filter) = std::env::var("DBT_NOVA_LOG")
        .or_else(|_| std::env::var("RUST_LOG"))
        .ok()
        .filter(|value| !value.trim().is_empty())
    else {
        return;
    };

    let format = std::env::var("DBT_NOVA_LOG_FORMAT")
        .ok()
        .and_then(|value| LogFormat::parse(&value))
        .unwrap_or_default();

    let init_result = match format {
        LogFormat::Human => tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(std::io::stderr)
            .with_target(false)
            .with_span_events(FmtSpan::CLOSE)
            .try_init(),
        LogFormat::Json => tracing_subscriber::fmt()
            .json()
            .flatten_event(true)
            .with_current_span(true)
            .with_span_list(true)
            .with_env_filter(filter)
            .with_writer(std::io::stderr)
            .with_target(false)
            .try_init(),
    };

    if let Err(err) = init_result {
        tracing::warn!(error = %err, "failed to initialize tracing subscriber");
    }
}

#[cfg(test)]
mod tests {
    use super::LogFormat;

    #[test]
    fn log_format_parse_accepts_documented_values() {
        assert_eq!(LogFormat::parse("human"), Some(LogFormat::Human));
        assert_eq!(LogFormat::parse(" HUMAN "), Some(LogFormat::Human));
        assert_eq!(LogFormat::parse("json"), Some(LogFormat::Json));
        assert_eq!(LogFormat::parse("JSON"), Some(LogFormat::Json));
        assert_eq!(LogFormat::parse("xml"), None);
    }
}
