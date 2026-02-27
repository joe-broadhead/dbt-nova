use std::future::Future;

use serde_json::{Map as JsonMap, Value as JsonValue};

use crate::error::Result;
use crate::responses::SuccessResponse;

/// Return true when a preflight probe can be considered "present".
///
/// Providers may surface row presence either as concrete rows or as an aggregate
/// `total_row_count` without materialized rows. This helper keeps non-empty
/// probe semantics consistent across providers.
pub(crate) fn preflight_probe_has_rows(rows_len: usize, total_row_count: Option<u64>) -> bool {
    rows_len > 0 || total_row_count.is_some_and(|count| count > 0)
}

/// Standard message used when an object-level preflight probe is empty.
pub(crate) fn empty_preflight_probe_message(check: &str) -> String {
    format!("Preflight {check} probe returned no rows; target may not exist or may be inaccessible")
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum ProbePresence {
    Present,
    Empty,
}

#[derive(Debug)]
pub(crate) struct PreflightReport {
    checks: Vec<JsonValue>,
    ready: bool,
}

impl PreflightReport {
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {
            checks: Vec::new(),
            ready: true,
        }
    }

    pub(crate) fn push_ok(&mut self, name: &'static str, details: JsonMap<String, JsonValue>) {
        self.checks
            .push(build_check(name, true, details, None, None));
    }

    pub(crate) fn push_failure(
        &mut self,
        name: &'static str,
        details: JsonMap<String, JsonValue>,
        message: impl Into<String>,
        action: &'static str,
    ) {
        self.ready = false;
        self.checks.push(build_check(
            name,
            false,
            details,
            Some(message.into()),
            Some(action),
        ));
    }
}

fn build_check(
    name: &'static str,
    ok: bool,
    mut details: JsonMap<String, JsonValue>,
    message: Option<String>,
    action: Option<&'static str>,
) -> JsonValue {
    details.insert("name".to_string(), JsonValue::String(name.to_string()));
    details.insert("ok".to_string(), JsonValue::Bool(ok));
    if let Some(message) = message {
        details.insert("message".to_string(), JsonValue::String(message));
    }
    if let Some(action) = action {
        details.insert("action".to_string(), JsonValue::String(action.to_string()));
    }
    JsonValue::Object(details)
}

pub(crate) async fn run_connectivity_check<F, Fut>(
    report: &mut PreflightReport,
    action: &'static str,
    probe: F,
) where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<()>>,
{
    match probe().await {
        Ok(()) => report.push_ok("connectivity", JsonMap::new()),
        Err(err) => report.push_failure("connectivity", JsonMap::new(), err.to_string(), action),
    }
}

pub(crate) fn run_connectivity_check_sync<F>(
    report: &mut PreflightReport,
    action: &'static str,
    probe: F,
) where
    F: FnOnce() -> Result<()>,
{
    match probe() {
        Ok(()) => report.push_ok("connectivity", JsonMap::new()),
        Err(err) => report.push_failure("connectivity", JsonMap::new(), err.to_string(), action),
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_optional_object_check<T, Normalize, Probe, ProbeFut, RawDetails, Details>(
    report: &mut PreflightReport,
    raw_input: Option<&str>,
    check_name: &'static str,
    normalize: Normalize,
    probe: Probe,
    raw_details: RawDetails,
    normalized_details: Details,
    invalid_action: &'static str,
    probe_action: &'static str,
    empty_message: &str,
) where
    Normalize: Fn(&str) -> Result<T>,
    Probe: Fn(&T) -> ProbeFut,
    ProbeFut: Future<Output = Result<ProbePresence>>,
    RawDetails: Fn(&str) -> JsonMap<String, JsonValue>,
    Details: Fn(&T) -> JsonMap<String, JsonValue>,
{
    let Some(raw_input) = raw_input else {
        return;
    };

    match normalize(raw_input) {
        Ok(normalized) => {
            let details = normalized_details(&normalized);
            match probe(&normalized).await {
                Ok(ProbePresence::Present) => report.push_ok(check_name, details),
                Ok(ProbePresence::Empty) => report.push_failure(
                    check_name,
                    details,
                    empty_message.to_string(),
                    probe_action,
                ),
                Err(err) => report.push_failure(check_name, details, err.to_string(), probe_action),
            }
        }
        Err(err) => {
            report.push_failure(
                check_name,
                raw_details(raw_input),
                err.to_string(),
                invalid_action,
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn run_optional_object_check_sync<T, Normalize, Probe, RawDetails, Details>(
    report: &mut PreflightReport,
    raw_input: Option<&str>,
    check_name: &'static str,
    normalize: Normalize,
    probe: Probe,
    raw_details: RawDetails,
    normalized_details: Details,
    invalid_action: &'static str,
    probe_action: &'static str,
    empty_message: &str,
) where
    Normalize: Fn(&str) -> Result<T>,
    Probe: Fn(&T) -> Result<ProbePresence>,
    RawDetails: Fn(&str) -> JsonMap<String, JsonValue>,
    Details: Fn(&T) -> JsonMap<String, JsonValue>,
{
    let Some(raw_input) = raw_input else {
        return;
    };

    match normalize(raw_input) {
        Ok(normalized) => {
            let details = normalized_details(&normalized);
            match probe(&normalized) {
                Ok(ProbePresence::Present) => report.push_ok(check_name, details),
                Ok(ProbePresence::Empty) => report.push_failure(
                    check_name,
                    details,
                    empty_message.to_string(),
                    probe_action,
                ),
                Err(err) => report.push_failure(check_name, details, err.to_string(), probe_action),
            }
        }
        Err(err) => {
            report.push_failure(
                check_name,
                raw_details(raw_input),
                err.to_string(),
                invalid_action,
            );
        }
    }
}

pub(crate) fn build_preflight_response(
    provider: &'static str,
    mut metadata: JsonMap<String, JsonValue>,
    report: PreflightReport,
) -> Result<JsonValue> {
    metadata.insert(
        "provider".to_string(),
        JsonValue::String(provider.to_string()),
    );
    metadata.insert("ready".to_string(), JsonValue::Bool(report.ready));
    metadata.insert("checks".to_string(), JsonValue::Array(report.checks));

    serde_json::to_value(SuccessResponse::new(JsonValue::Object(metadata), 1)).map_err(Into::into)
}

pub(crate) fn build_configuration_failure_response(
    provider: &'static str,
    metadata: JsonMap<String, JsonValue>,
    message: impl Into<String>,
    action: &'static str,
) -> Result<JsonValue> {
    let mut report = PreflightReport::new();
    report.push_failure("configuration", JsonMap::new(), message, action);
    build_preflight_response(provider, metadata, report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn optional_check_marks_ready_false_on_invalid_identifier() {
        let mut report = PreflightReport::new();

        run_optional_object_check(
            &mut report,
            Some("bad identifier"),
            "catalog_access",
            |_input| {
                Err(crate::error::DbtNovaError::InvalidParams(
                    "bad identifier".to_string(),
                ))
            },
            |_normalized: &String| async { Ok(ProbePresence::Present) },
            |raw| JsonMap::from_iter([(String::from("catalog"), json!(raw))]),
            |_normalized| JsonMap::new(),
            "Use a valid catalog",
            "Verify access",
            "empty probe",
        )
        .await;

        let payload = build_preflight_response("test", JsonMap::new(), report).expect("response");
        assert_eq!(payload["data"]["ready"], json!(false));
        assert_eq!(
            payload["data"]["checks"][0]["name"],
            json!("catalog_access")
        );
        assert_eq!(payload["data"]["checks"][0]["ok"], json!(false));
    }

    #[tokio::test]
    async fn optional_check_treats_empty_probe_as_failure() {
        let mut report = PreflightReport::new();

        run_optional_object_check(
            &mut report,
            Some("analytics"),
            "schema_access",
            |input| Ok(input.to_string()),
            |_normalized: &String| async { Ok(ProbePresence::Empty) },
            |raw| JsonMap::from_iter([(String::from("schema"), json!(raw))]),
            |normalized| JsonMap::from_iter([(String::from("schema"), json!(normalized))]),
            "Use valid schema",
            "Verify schema access",
            "empty schema probe",
        )
        .await;

        let payload = build_preflight_response("test", JsonMap::new(), report).expect("response");
        assert_eq!(payload["data"]["ready"], json!(false));
        assert_eq!(
            payload["data"]["checks"][0]["message"],
            json!("empty schema probe")
        );
    }

    #[test]
    fn optional_check_sync_treats_empty_probe_as_failure() {
        let mut report = PreflightReport::new();

        run_optional_object_check_sync(
            &mut report,
            Some("analytics"),
            "schema_access",
            |input| Ok(input.to_string()),
            |_normalized: &String| Ok(ProbePresence::Empty),
            |raw| JsonMap::from_iter([(String::from("schema"), json!(raw))]),
            |normalized| JsonMap::from_iter([(String::from("schema"), json!(normalized))]),
            "Use valid schema",
            "Verify schema access",
            "empty schema probe",
        );

        let payload = build_preflight_response("test", JsonMap::new(), report).expect("response");
        assert_eq!(payload["data"]["ready"], json!(false));
        assert_eq!(
            payload["data"]["checks"][0]["message"],
            json!("empty schema probe")
        );
    }
}
