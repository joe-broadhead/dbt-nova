use super::*;

pub(super) fn build_eval_validate_payload(path: &str) -> crate::error::Result<JsonValue> {
    let suite = load_suite(path)?;
    let date_anchor_case_count = suite
        .cases
        .iter()
        .filter(|case| effective_date_anchor(&suite.date_anchor, &case.date_anchor).is_some())
        .count()
        + suite
            .agent_cases
            .iter()
            .filter(|case| effective_date_anchor(&suite.date_anchor, &case.date_anchor).is_some())
            .count();
    Ok(json!({
        "valid": true,
        "path": path,
        "suite_name": suite.name.as_deref().unwrap_or("suite"),
        "version": suite.version,
        "date_anchor": suite.date_anchor.normalized(),
        "date_anchor_case_count": date_anchor_case_count,
        "bridge_case_count": suite.cases.len(),
        "agent_case_count": suite.agent_cases.len(),
    }))
}

pub(super) fn build_eval_gate_report_for_suite(
    suite_name: &str,
) -> crate::error::Result<EvalGateReport> {
    let rows = read_telemetry_rows_for_suite(suite_name)?;
    build_eval_gate_report(suite_name, &rows)
}

pub(super) fn normalized_string(value: Option<&String>) -> Option<String> {
    value
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub(super) fn effective_date_anchor(suite: &DateAnchor, case: &DateAnchor) -> Option<DateAnchor> {
    let anchor = DateAnchor {
        snapshot_date: normalized_string(case.snapshot_date.as_ref())
            .or_else(|| normalized_string(suite.snapshot_date.as_ref())),
        date_range_start: normalized_string(case.date_range_start.as_ref())
            .or_else(|| normalized_string(suite.date_range_start.as_ref())),
        date_range_end: normalized_string(case.date_range_end.as_ref())
            .or_else(|| normalized_string(suite.date_range_end.as_ref())),
        date_field: normalized_string(case.date_field.as_ref())
            .or_else(|| normalized_string(suite.date_field.as_ref())),
    };
    (!anchor.is_empty()).then_some(anchor)
}

pub(super) fn validate_date_anchor(
    anchor: &DateAnchor,
    location: &str,
) -> crate::error::Result<()> {
    validate_optional_date(anchor.snapshot_date.as_deref(), location, "snapshot_date")?;
    validate_optional_date(
        anchor.date_range_start.as_deref(),
        location,
        "date_range_start",
    )?;
    validate_optional_date(anchor.date_range_end.as_deref(), location, "date_range_end")?;
    if let Some(field) = anchor.date_field.as_deref()
        && field.trim().is_empty()
    {
        return Err(DbtNovaError::InvalidParams(format!(
            "{location} date_field must be non-empty when set"
        )));
    }
    Ok(())
}

pub(super) fn validate_complete_date_anchor(
    anchor: &DateAnchor,
    location: &str,
) -> crate::error::Result<()> {
    let snapshot_date =
        validate_optional_date(anchor.snapshot_date.as_deref(), location, "snapshot_date")?;
    let date_range_start = validate_optional_date(
        anchor.date_range_start.as_deref(),
        location,
        "date_range_start",
    )?;
    let date_range_end =
        validate_optional_date(anchor.date_range_end.as_deref(), location, "date_range_end")?;
    if date_range_start.is_some() != date_range_end.is_some() {
        return Err(DbtNovaError::InvalidParams(format!(
            "{location} must include both date_range_start and date_range_end when either date range field is set"
        )));
    }
    if let (Some(start), Some(end)) = (date_range_start, date_range_end)
        && start > end
    {
        return Err(DbtNovaError::InvalidParams(format!(
            "{location} date_range_start must be on or before date_range_end"
        )));
    }
    if snapshot_date.is_none()
        && date_range_start.is_none()
        && anchor
            .date_field
            .as_deref()
            .is_some_and(|field| !field.trim().is_empty())
    {
        return Err(DbtNovaError::InvalidParams(format!(
            "{location} date_field requires snapshot_date or date_range_start/date_range_end"
        )));
    }
    Ok(())
}

pub(super) fn validate_optional_date(
    value: Option<&str>,
    location: &str,
    field: &str,
) -> crate::error::Result<Option<(i32, u32, u32)>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(DbtNovaError::InvalidParams(format!(
            "{location} {field} must be a non-empty YYYY-MM-DD date"
        )));
    }
    parse_iso_date(trimmed).map(Some).ok_or_else(|| {
        DbtNovaError::InvalidParams(format!(
            "{location} {field} must use YYYY-MM-DD with a valid calendar date"
        ))
    })
}

pub(super) fn parse_iso_date(value: &str) -> Option<(i32, u32, u32)> {
    let bytes = value.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return None;
    }
    let year = i32::try_from(parse_date_part(&value[0..4])?).ok()?;
    let month = parse_date_part(&value[5..7])?;
    let day = parse_date_part(&value[8..10])?;
    let max_day = days_in_month(year, month);
    if max_day == 0 || day == 0 || day > max_day {
        return None;
    }
    Some((year, month, day))
}

pub(super) fn parse_date_part(value: &str) -> Option<u32> {
    value
        .as_bytes()
        .iter()
        .all(u8::is_ascii_digit)
        .then(|| value.parse::<u32>().ok())
        .flatten()
}

pub(super) fn eval_history_rows(
    suite_name: &str,
    since: &str,
) -> crate::error::Result<(String, Vec<JsonValue>)> {
    let since_boundary = validate_since_date(since)?;
    let rows = read_telemetry_rows_for_suite(suite_name)?
        .into_iter()
        .filter(|row| telemetry_row_matches_since(row, &since_boundary))
        .collect();
    Ok((since_boundary, rows))
}

pub(super) fn ensure_mcp_latest_telemetry_suite_paths_under_root(
    rows: &[JsonValue],
) -> crate::error::Result<()> {
    ensure_mcp_telemetry_suite_paths_under_root(latest_telemetry_rows(rows))
}

pub(super) fn ensure_mcp_telemetry_suite_paths_under_root<'a>(
    rows: impl IntoIterator<Item = &'a JsonValue>,
) -> crate::error::Result<()> {
    for row in rows {
        let Some(path) = row
            .get("suite_path")
            .and_then(JsonValue::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        let (_root, candidate) = mcp_eval_candidate_path(path, "suite_path")?;
        if candidate.exists() {
            let canonical = candidate.canonicalize().map_err(|error| {
                DbtNovaError::InvalidParams(format!(
                    "failed to resolve suite_path '{}': {error}",
                    candidate.display()
                ))
            })?;
            let root = mcp_eval_filesystem_root()?;
            ensure_mcp_eval_path_under_root(&canonical, &root, "suite_path")?;
            if !canonical.is_file() {
                return Err(DbtNovaError::InvalidParams(format!(
                    "suite_path '{}' is not a file",
                    canonical.display()
                )));
            }
        }
    }
    Ok(())
}

pub(super) fn provider_output_for_artifact(output: &str) -> String {
    if raw_provider_logs_enabled() {
        output.to_string()
    } else {
        redact_provider_output_text(output)
    }
}

pub(super) fn provider_failure_evidence(
    invocation: &provider::ProviderInvocation,
    stderr: &str,
) -> JsonValue {
    let mut evidence = provider_invocation_evidence(invocation);
    if let JsonValue::Object(object) = &mut evidence {
        object.insert(
            "stderr".to_string(),
            JsonValue::String(truncate(&redact_provider_output_text(stderr), 4000)),
        );
    }
    evidence
}

pub(super) fn provider_invocation_evidence(invocation: &provider::ProviderInvocation) -> JsonValue {
    json!({
        "command": redact_provider_output_text(&invocation.command),
        "args": redact_provider_args(&invocation.args),
    })
}

pub(super) fn redact_provider_args(args: &[String]) -> Vec<String> {
    let mut redacted = Vec::with_capacity(args.len());
    let mut redact_next = false;
    for arg in args {
        if redact_next {
            redacted.push("[REDACTED]".to_string());
            redact_next = false;
            continue;
        }
        redacted.push(redact_provider_output_text(arg));
        redact_next = provider_arg_expects_sensitive_value(arg);
    }
    redacted
}

pub(super) fn provider_arg_expects_sensitive_value(arg: &str) -> bool {
    let trimmed = arg.trim();
    if trimmed.is_empty()
        || trimmed.contains('=')
        || trimmed.contains(':')
        || trimmed.contains('/')
        || !trimmed.starts_with('-')
    {
        return false;
    }
    let key = trimmed.trim_start_matches('-').to_ascii_lowercase();
    [
        "token",
        "access-token",
        "access_token",
        "api-token",
        "api_token",
        "apikey",
        "api-key",
        "api_key",
        "secret",
        "password",
        "passwd",
        "pwd",
        "credential",
        "authorization",
        "auth",
        "session",
        "jwt",
    ]
    .iter()
    .any(|sensitive| key.contains(sensitive))
}

pub(super) fn redact_provider_output_text(output: &str) -> String {
    redact_sensitive_text(output)
}

pub(super) fn raw_provider_logs_enabled() -> bool {
    std::env::var(RAW_PROVIDER_LOGS_ENV).is_ok_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

pub(super) fn success_value<T: Serialize>(
    payload: T,
    count: usize,
) -> crate::error::Result<JsonValue> {
    serde_json::to_value(SuccessResponse::new(payload, count))
        .map_err(|error| DbtNovaError::ServerError(error.to_string()))
}

pub(super) fn with_eval_safety_policy(mut payload: JsonValue) -> crate::error::Result<JsonValue> {
    let policy = serde_json::to_value(eval_mcp_safety_policy()?)
        .map_err(|error| DbtNovaError::ServerError(error.to_string()))?;
    let Some(object) = payload.as_object_mut() else {
        return Err(DbtNovaError::ServerError(
            "failed to serialize eval response payload as object".to_string(),
        ));
    };
    object.insert("safety_policy".to_string(), policy);
    Ok(payload)
}

pub(super) fn eval_mcp_safety_policy() -> crate::error::Result<EvalMcpSafetyPolicy> {
    Ok(EvalMcpSafetyPolicy {
        filesystem_root: mcp_eval_filesystem_root()?.display().to_string(),
        eval_run_enabled_env: MCP_ENABLE_EVAL_RUN_ENV,
        eval_writes_enabled_env: MCP_ENABLE_EVAL_WRITES_ENV,
        agent_eval_enabled_env: MCP_ENABLE_AGENT_EVAL_ENV,
        custom_agent_provider_enabled_env: MCP_ENABLE_CUSTOM_AGENT_PROVIDER_ENV,
        raw_provider_logs_enabled_env: RAW_PROVIDER_LOGS_ENV,
        provider_logs_redacted_by_default: true,
        local_paths_must_stay_under_filesystem_root: true,
    })
}

pub(super) fn require_mcp_eval_flag(
    env_name: &'static str,
    tool_name: &str,
) -> crate::error::Result<()> {
    if mcp_eval_flag_enabled(env_name) {
        return Ok(());
    }
    Err(DbtNovaError::InvalidParams(format!(
        "{tool_name} is disabled for MCP/tool-call use; set {env_name}=1 to enable this local execution capability"
    )))
}

pub(super) fn mcp_eval_flag_enabled(env_name: &str) -> bool {
    std::env::var(env_name)
        .ok()
        .map(|value| value.trim().to_ascii_lowercase())
        .is_some_and(|value| matches!(value.as_str(), "1" | "true" | "yes" | "on"))
}

pub(super) fn resolve_mcp_existing_file(
    raw_path: &str,
    label: &str,
) -> crate::error::Result<PathBuf> {
    let (_root, candidate) = mcp_eval_candidate_path(raw_path, label)?;
    let canonical = candidate.canonicalize().map_err(|error| {
        DbtNovaError::InvalidParams(format!(
            "failed to resolve {label} '{}': {error}",
            candidate.display()
        ))
    })?;
    let root = mcp_eval_filesystem_root()?;
    ensure_mcp_eval_path_under_root(&canonical, &root, label)?;
    if !canonical.is_file() {
        return Err(DbtNovaError::InvalidParams(format!(
            "{label} '{}' is not a file",
            canonical.display()
        )));
    }
    Ok(canonical)
}

pub(super) fn resolve_mcp_writable_path(
    raw_path: &str,
    label: &str,
) -> crate::error::Result<PathBuf> {
    let (root, candidate) = mcp_eval_candidate_path(raw_path, label)?;
    if candidate.exists() {
        let canonical = candidate.canonicalize().map_err(|error| {
            DbtNovaError::InvalidParams(format!(
                "failed to resolve {label} '{}': {error}",
                candidate.display()
            ))
        })?;
        ensure_mcp_eval_path_under_root(&canonical, &root, label)?;
        return Ok(canonical);
    }
    ensure_existing_ancestor_under_root(&candidate, &root, label)?;
    Ok(candidate)
}

pub(super) fn mcp_eval_candidate_path(
    raw_path: &str,
    label: &str,
) -> crate::error::Result<(PathBuf, PathBuf)> {
    let trimmed = raw_path.trim();
    if trimmed.is_empty() {
        return Err(DbtNovaError::InvalidParams(format!(
            "{label} must not be empty"
        )));
    }
    let root = mcp_eval_filesystem_root()?;
    let path = PathBuf::from(trimmed);
    reject_mcp_eval_parent_dirs(&path, label)?;
    if !path.is_absolute() {
        reject_mcp_eval_relative_traversal(&path, label)?;
    }
    let candidate = if path.is_absolute() {
        path
    } else {
        root.join(path)
    };
    ensure_mcp_eval_path_under_root(&candidate, &root, label)?;
    Ok((root, candidate))
}

pub(super) fn reject_mcp_eval_parent_dirs(path: &Path, label: &str) -> crate::error::Result<()> {
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(DbtNovaError::InvalidParams(format!(
            "{label} must stay under the server working directory"
        )));
    }
    Ok(())
}

pub(super) fn reject_mcp_eval_relative_traversal(
    path: &Path,
    label: &str,
) -> crate::error::Result<()> {
    for component in path.components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            Component::ParentDir => {
                return Err(DbtNovaError::InvalidParams(format!(
                    "{label} must stay under the server working directory"
                )));
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(DbtNovaError::InvalidParams(format!(
                    "{label} must be relative or resolve under the server working directory"
                )));
            }
        }
    }
    Ok(())
}

pub(super) fn ensure_existing_ancestor_under_root(
    candidate: &Path,
    root: &Path,
    label: &str,
) -> crate::error::Result<()> {
    let mut ancestor = candidate.parent();
    while let Some(path) = ancestor {
        if path.exists() {
            let canonical = path.canonicalize().map_err(|error| {
                DbtNovaError::InvalidParams(format!(
                    "failed to resolve parent for {label} '{}': {error}",
                    path.display()
                ))
            })?;
            return ensure_mcp_eval_path_under_root(&canonical, root, label);
        }
        ancestor = path.parent();
    }
    Err(DbtNovaError::InvalidParams(format!(
        "{label} has no existing parent under '{}'",
        root.display()
    )))
}

pub(super) fn ensure_mcp_eval_path_under_root(
    path: &Path,
    root: &Path,
    label: &str,
) -> crate::error::Result<()> {
    if path.starts_with(root) {
        return Ok(());
    }
    Err(DbtNovaError::InvalidParams(format!(
        "{label} '{}' is outside server working directory '{}'",
        path.display(),
        root.display()
    )))
}

pub(super) fn mcp_eval_filesystem_root() -> crate::error::Result<PathBuf> {
    std::env::current_dir()
        .and_then(|path| path.canonicalize())
        .map_err(|error| {
            DbtNovaError::InvalidParams(format!(
                "failed to resolve server working directory: {error}"
            ))
        })
}

pub(super) fn load_suite(path: &str) -> crate::error::Result<EvalSuite> {
    load_suite_with_hash(path).map(|(suite, _hash)| suite)
}

pub(super) fn load_suite_with_hash(path: &str) -> crate::error::Result<(EvalSuite, String)> {
    let raw = fs::read(path).map_err(|error| {
        DbtNovaError::InvalidParams(format!("failed to read eval suite '{path}': {error}"))
    })?;
    let suite_hash = blake3::hash(&raw).to_hex().to_string();
    let raw = std::str::from_utf8(&raw).map_err(|error| {
        DbtNovaError::InvalidParams(format!(
            "failed to read eval suite '{path}' as UTF-8: {error}"
        ))
    })?;
    let suite: EvalSuite = if Path::new(path)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
    {
        serde_json::from_str(raw).map_err(|error| {
            DbtNovaError::InvalidParams(format!("failed to parse eval suite JSON: {error}"))
        })?
    } else {
        serde_yaml::from_str(raw).map_err(|error| {
            DbtNovaError::InvalidParams(format!("failed to parse eval suite YAML: {error}"))
        })?
    };
    validate_suite(&suite)?;
    Ok((suite, suite_hash))
}

pub(super) fn validate_suite(suite: &EvalSuite) -> crate::error::Result<()> {
    if suite.version == 0 {
        return Err(DbtNovaError::InvalidParams(
            "eval suite version must be greater than zero".to_string(),
        ));
    }
    if let Some(gate) = suite.gate
        && !(0.0..=1.0).contains(&gate.threshold)
    {
        return Err(DbtNovaError::InvalidParams(
            "eval suite gate.threshold must be between 0.0 and 1.0".to_string(),
        ));
    }
    validate_date_anchor(&suite.date_anchor, "eval suite")?;
    if suite.cases.is_empty()
        && suite.agent_cases.is_empty()
        && let Some(date_anchor) = suite.date_anchor.normalized()
    {
        validate_complete_date_anchor(&date_anchor, "eval suite")?;
    }
    validate_case_ids(suite.cases.iter().map(|case| case.id.as_str()), "cases")?;
    validate_case_ids(
        suite.agent_cases.iter().map(|case| case.id.as_str()),
        "agent_cases",
    )?;
    for case in &suite.cases {
        validate_date_anchor(&case.date_anchor, &format!("eval case '{}'", case.id))?;
        if let Some(date_anchor) = effective_date_anchor(&suite.date_anchor, &case.date_anchor) {
            validate_complete_date_anchor(
                &date_anchor,
                &format!("effective date anchor for eval case '{}'", case.id),
            )?;
        }
        if case.assertions.is_empty() {
            return Err(DbtNovaError::InvalidParams(format!(
                "eval case '{}' must include at least one assertion",
                case.id
            )));
        }
        for assertion in &case.assertions {
            validate_assertion(assertion, &case.id)?;
        }
    }
    for case in &suite.agent_cases {
        validate_date_anchor(&case.date_anchor, &format!("agent case '{}'", case.id))?;
        if let Some(date_anchor) = effective_date_anchor(&suite.date_anchor, &case.date_anchor) {
            validate_complete_date_anchor(
                &date_anchor,
                &format!("effective date anchor for agent case '{}'", case.id),
            )?;
        }
        if case.task.trim().is_empty() {
            return Err(DbtNovaError::InvalidParams(format!(
                "agent case '{}' must include a non-empty task",
                case.id
            )));
        }
        validate_agent_expected(&case.expected, &case.id)?;
    }
    Ok(())
}

pub(super) fn validate_assertion(
    assertion: &EvalAssertion,
    case_id: &str,
) -> crate::error::Result<()> {
    match assertion {
        EvalAssertion::SearchColumnsRank {
            expected_column,
            expected_parent_unique_id,
            ..
        } => {
            let has_expected_column = expected_column
                .as_ref()
                .is_some_and(|value| !value.trim().is_empty());
            let has_expected_parent = expected_parent_unique_id
                .as_ref()
                .is_some_and(|value| !value.trim().is_empty());
            if !has_expected_column && !has_expected_parent {
                return Err(DbtNovaError::InvalidParams(format!(
                    "search_columns_rank assertion in case '{case_id}' must include expected_column or expected_parent_unique_id"
                )));
            }
        }
        EvalAssertion::ContextFieldEquals { field, .. } if field.trim().is_empty() => {
            return Err(DbtNovaError::InvalidParams(format!(
                "context_field_equals assertion in case '{case_id}' must include a non-empty field"
            )));
        }
        EvalAssertion::ContextContains {
            expected, field, ..
        } => {
            if expected.trim().is_empty() {
                return Err(DbtNovaError::InvalidParams(format!(
                    "context_contains assertion in case '{case_id}' must include non-empty expected text"
                )));
            }
            if field.as_ref().is_some_and(|field| field.trim().is_empty()) {
                return Err(DbtNovaError::InvalidParams(format!(
                    "context_contains assertion in case '{case_id}' must include a non-empty field when field is set"
                )));
            }
        }
        EvalAssertion::ToolResponseBudget {
            tool,
            max_response_bytes,
            must_contain_paths,
            must_not_contain_paths,
            ..
        } => {
            if tool.trim().is_empty() {
                return Err(DbtNovaError::InvalidParams(format!(
                    "tool_response_budget assertion in case '{case_id}' must include a non-empty tool"
                )));
            }
            if *max_response_bytes == 0 {
                return Err(DbtNovaError::InvalidParams(format!(
                    "tool_response_budget assertion in case '{case_id}' must use max_response_bytes greater than zero"
                )));
            }
            if must_contain_paths
                .iter()
                .chain(must_not_contain_paths)
                .any(|path| path.trim().is_empty())
            {
                return Err(DbtNovaError::InvalidParams(format!(
                    "tool_response_budget assertion in case '{case_id}' must use non-empty field paths"
                )));
            }
        }
        EvalAssertion::ToolFieldEquals { tool, field, .. } => {
            if tool.trim().is_empty() {
                return Err(DbtNovaError::InvalidParams(format!(
                    "tool_field_equals assertion in case '{case_id}' must include a non-empty tool"
                )));
            }
            if field.trim().is_empty() {
                return Err(DbtNovaError::InvalidParams(format!(
                    "tool_field_equals assertion in case '{case_id}' must include a non-empty field"
                )));
            }
        }
        EvalAssertion::SqlStructure {
            actual_sql,
            expected_sql,
        } => {
            validate_sql_structure_field(actual_sql, case_id, "actual_sql")?;
            validate_sql_structure_field(expected_sql, case_id, "expected_sql")?;
        }
        _ => {}
    }
    Ok(())
}

pub(super) fn validate_sql_structure_field(
    sql: &str,
    case_id: &str,
    field: &str,
) -> crate::error::Result<()> {
    if sql.trim().is_empty() {
        return Err(DbtNovaError::InvalidParams(format!(
            "sql_structure assertion in case '{case_id}' must include non-empty {field}"
        )));
    }
    sql_structure_signature(sql).map(|_| ()).map_err(|error| {
        DbtNovaError::InvalidParams(format!(
            "sql_structure assertion in case '{case_id}' has invalid {field}: {error}"
        ))
    })
}

pub(super) fn validate_agent_expected(
    expected: &AgentExpected,
    case_id: &str,
) -> crate::error::Result<()> {
    for rank in &expected.selected_entity_ranks {
        if rank.unique_id.trim().is_empty() {
            return Err(DbtNovaError::InvalidParams(format!(
                "selected_entity_ranks in agent case '{case_id}' must include non-empty unique_id values"
            )));
        }
        if rank
            .tool
            .as_ref()
            .is_some_and(|tool| tool.trim().is_empty())
        {
            return Err(DbtNovaError::InvalidParams(format!(
                "selected_entity_ranks in agent case '{case_id}' must include non-empty tool values when tool is set"
            )));
        }
        if rank.max_rank == Some(0) {
            return Err(DbtNovaError::InvalidParams(format!(
                "selected_entity_ranks in agent case '{case_id}' must use max_rank greater than zero"
            )));
        }
    }
    for called_with in &expected.called_with {
        if called_with.tool.trim().is_empty() {
            return Err(DbtNovaError::InvalidParams(format!(
                "called_with expectations in agent case '{case_id}' must include non-empty tool values"
            )));
        }
        if called_with.params.is_empty() && called_with.contains.is_empty() {
            return Err(DbtNovaError::InvalidParams(format!(
                "called_with expectations in agent case '{case_id}' must include params or contains constraints"
            )));
        }
        if called_with
            .contains
            .iter()
            .any(|(key, value)| key.trim().is_empty() || value.trim().is_empty())
        {
            return Err(DbtNovaError::InvalidParams(format!(
                "called_with contains expectations in agent case '{case_id}' must include non-empty keys and values"
            )));
        }
        if called_with.params.keys().any(|key| key.trim().is_empty()) {
            return Err(DbtNovaError::InvalidParams(format!(
                "called_with params expectations in agent case '{case_id}' must include non-empty keys"
            )));
        }
        if called_with
            .params
            .values()
            .any(|value| !is_safe_expected_param_value(value))
        {
            return Err(DbtNovaError::InvalidParams(format!(
                "called_with params expectations in agent case '{case_id}' must use scalar values or arrays of scalar values"
            )));
        }
    }
    for sql_structure in &expected.sql_structures {
        if sql_structure.tool.trim().is_empty() {
            return Err(DbtNovaError::InvalidParams(format!(
                "sql_structures expectations in agent case '{case_id}' must include non-empty tool values"
            )));
        }
        if sql_structure.expected_sql.trim().is_empty() {
            return Err(DbtNovaError::InvalidParams(format!(
                "sql_structures expectations in agent case '{case_id}' must include non-empty expected_sql"
            )));
        }
        sql_structure_signature(&sql_structure.expected_sql).map_err(|error| {
            DbtNovaError::InvalidParams(format!(
                "sql_structures expectation in agent case '{case_id}' has invalid expected_sql: {error}"
            ))
        })?;
    }
    if expected.max_tool_calls == Some(0)
        || expected.max_distinct_tools == Some(0)
        || expected.max_total_response_bytes == Some(0)
        || expected
            .max_response_bytes_by_tool
            .values()
            .any(|max| *max == 0)
    {
        return Err(DbtNovaError::InvalidParams(format!(
            "agent case '{case_id}' budget expectations must use positive thresholds"
        )));
    }
    if expected
        .max_response_bytes_by_tool
        .keys()
        .any(|tool| tool.trim().is_empty())
    {
        return Err(DbtNovaError::InvalidParams(format!(
            "agent case '{case_id}' max_response_bytes_by_tool keys must be non-empty tool names"
        )));
    }
    Ok(())
}

pub(super) fn is_safe_expected_param_value(value: &JsonValue) -> bool {
    match value {
        JsonValue::Null | JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::String(_) => true,
        JsonValue::Array(items) => items.iter().all(|item| {
            matches!(
                item,
                JsonValue::Null | JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::String(_)
            )
        }),
        JsonValue::Object(_) => false,
    }
}

pub(super) fn validate_case_ids<'a>(
    ids: impl Iterator<Item = &'a str>,
    section: &str,
) -> crate::error::Result<()> {
    let mut seen = BTreeSet::new();
    let mut seen_artifact_segments = BTreeSet::new();
    for id in ids {
        let trimmed = id.trim();
        if trimmed.is_empty() {
            return Err(DbtNovaError::InvalidParams(format!(
                "{section} entries must include non-empty ids"
            )));
        }
        if !seen.insert(trimmed.to_string()) {
            return Err(DbtNovaError::InvalidParams(format!(
                "duplicate eval case id '{trimmed}' in {section}"
            )));
        }
        let artifact_segment = safe_path_segment(trimmed);
        let artifact_segment_key = artifact_segment.to_ascii_lowercase();
        if !seen_artifact_segments.insert(artifact_segment_key) {
            return Err(DbtNovaError::InvalidParams(format!(
                "eval case ids in {section} must map to unique artifact paths case-insensitively; duplicate segment '{artifact_segment}'"
            )));
        }
    }
    Ok(())
}

pub(super) fn resolve_output_dir(explicit: Option<&str>, suite: &EvalSuite, mode: &str) -> PathBuf {
    if let Some(path) = explicit {
        return PathBuf::from(path);
    }
    let suite_name = suite.name.as_deref().unwrap_or("suite");
    PathBuf::from(".nova").join("eval-runs").join(format!(
        "{}-{}-{}",
        timestamp_secs(),
        safe_path_segment(suite_name),
        mode
    ))
}

pub(super) fn validate_fail_under(value: Option<f64>) -> crate::error::Result<()> {
    let Some(value) = value else {
        return Ok(());
    };
    if !(0.0..=1.0).contains(&value) {
        return Err(DbtNovaError::InvalidParams(
            "--fail-under must be between 0.0 and 1.0".to_string(),
        ));
    }
    Ok(())
}

pub(super) fn selected_bridge_cases<'a>(
    cases: &'a [EvalCase],
    case_ids: &[String],
) -> crate::error::Result<Vec<&'a EvalCase>> {
    if case_ids.is_empty() {
        return Ok(cases.iter().collect());
    }
    let wanted = normalized_case_filter(case_ids)?;
    let selected: Vec<&EvalCase> = cases
        .iter()
        .filter(|case| wanted.contains(case.id.as_str()))
        .collect();
    validate_selected_cases(
        &wanted,
        selected.iter().map(|case| case.id.as_str()),
        "bridge",
    )?;
    Ok(selected)
}

pub(super) fn selected_agent_cases<'a>(
    cases: &'a [AgentCase],
    case_ids: &[String],
) -> crate::error::Result<Vec<&'a AgentCase>> {
    if case_ids.is_empty() {
        return Ok(cases.iter().collect());
    }
    let wanted = normalized_case_filter(case_ids)?;
    let selected: Vec<&AgentCase> = cases
        .iter()
        .filter(|case| wanted.contains(case.id.as_str()))
        .collect();
    validate_selected_cases(
        &wanted,
        selected.iter().map(|case| case.id.as_str()),
        "agent",
    )?;
    Ok(selected)
}

pub(super) fn normalized_case_filter(case_ids: &[String]) -> crate::error::Result<BTreeSet<&str>> {
    let mut wanted = BTreeSet::new();
    for id in case_ids {
        let trimmed = id.trim();
        if trimmed.is_empty() {
            return Err(DbtNovaError::InvalidParams(
                "--case-id values must be non-empty".to_string(),
            ));
        }
        wanted.insert(trimmed);
    }
    Ok(wanted)
}

pub(super) fn validate_selected_cases<'a>(
    wanted: &BTreeSet<&'a str>,
    selected: impl Iterator<Item = &'a str>,
    mode: &str,
) -> crate::error::Result<()> {
    let found: BTreeSet<&str> = selected.collect();
    let missing: Vec<&str> = wanted.difference(&found).copied().collect();
    if missing.is_empty() {
        return Ok(());
    }
    Err(DbtNovaError::InvalidParams(format!(
        "requested {mode} eval case id(s) not found: {}",
        missing.join(", ")
    )))
}

pub(super) fn agent_prompt(
    case: &AgentCase,
    date_anchor: Option<&DateAnchor>,
    persona: Option<&str>,
) -> String {
    let date_anchor_section = date_anchor.map_or_else(String::new, |anchor| {
        let mut section = String::from("\nDate anchor:\n");
        for line in anchor.prompt_lines() {
            let _ = writeln!(section, "- {line}");
        }
        section.push_str("- Treat these dates as ground truth for relative time phrases in the task. Do not reinterpret them using today's date.\n");
        section
    });
    if persona
        .map(str::trim)
        .is_some_and(|value| value.eq_ignore_ascii_case("reviewer"))
    {
        return format!(
            "You are running a dbt-nova reviewer-agent eval.\n\nTask:\n{}\n{}\nRules:\n- Review the supplied draft answer and evidence packet. Do not execute SQL, mutate files, inspect repository source, or invent missing evidence.\n- Use only evidence from the task packet or read-only Nova provenance/search/context evidence explicitly requested by the task.\n- If the packet lacks evidence needed for the review, return verdict `needs_evidence` and name the missing evidence.\n- Challenge semantic-layer bypass: if a governed Nova metric, measure, or semantic parent is available but the draft relies on raw/source table evidence without an evidence-backed fallback reason, require a fix.\n- Challenge stale or unknown freshness: if the draft uses stale or unknown-freshness evidence without an explicit caveat, require a fix.\n- Output a concise review with `verdict`, `findings`, `severity`, `evidence`, and `suggested_fix`.\n- Valid verdicts are `pass`, `fix_required`, and `needs_evidence`. Use `fix_required` for any semantic-layer bypass or missing freshness caveat that could change the answer.\n\nFinish with the review only; do not answer the original analytics question.",
            case.task, date_anchor_section
        );
    }
    format!(
        "You are running a dbt-nova analytics-agent eval.\n\nTask:\n{}\n{}\nRules:\n- Use Nova discovery and execution tools directly. Do not inspect repository files, source code, fixtures, or Rust params unless a Nova command fails and you cannot recover from the error message.\n- For KPI, metric, conversion, funnel, checkout, or business-concept questions, start with search_indicator using compact results: detail=\"compact\", group_mode=\"top\", include_support_signals=true, limit=3, persona=\"analyst\".\n- For rate, conversion, or funnel questions, include the requested metric names literally in the query and set indicator_types=[\"metric\"] unless you are explicitly searching for raw measures.\n- When a metric row returns an expression, copy that expression exactly into SQL; do not substitute similarly named measures or invent a numerator/denominator.\n- Use support_signals, grain dimensions, and relation_name from search_indicator to apply every requested filter before SQL. Do not aggregate across a grain dimension named in the task, such as country, market, channel, segment, or device.\n- Treat relation_name, grain, and expression fields returned by search_indicator as the execution contract. Do not run schema inspection SQL such as DESCRIBE or information_schema when those fields are present.\n- Use execute_sql only after Nova discovery identifies the canonical execution entity or relation. Use one aggregate SQL statement for current and comparison periods when possible. Skip get_entity when search_indicator already returns the relation, grain, measures, and metric expressions you need; otherwise use get_entity with id_or_name and detail=\"compact\".\n- Keep Nova calls to the minimum needed: usually search_indicator plus one execute_sql for calculations, and only search_indicator for model or metric lookup tasks. Avoid get_context, get_lineage, get_sql, and full-detail responses unless blocked.\n- If using the CLI, assume $DBT_NOVA_EVAL_BIN is set. For search/get calls, use --params-json. For execute_sql with quotes or newlines, write a JSON params file like {{\"statement\":\"select ...\",\"row_limit\":50}} and call $DBT_NOVA_EVAL_BIN tool call execute_sql --params-file <file> --json; do not inline multiline SQL in --params-json. Parameter reminders: get_entity uses id_or_name; execute_sql uses statement. Do not run echo, grep, read, or source inspection for normal tool usage.\n\nFinish with a concise answer that cites the Nova evidence, the SQL result, and the explicit filter values used.",
        case.task, date_anchor_section
    )
}

pub(super) fn starter_suite(persona: &str) -> String {
    let safe_persona = safe_path_segment(persona);
    let persona_yaml = serde_json::to_string(persona).unwrap_or_else(|_| "\"analyst\"".to_string());
    format!(
        r"version: 1
name: nova-{safe_persona}-smoke
defaults:
  persona: {persona_yaml}
  top_k: 5
cases:
  - id: canonical_entity_search
    question: Find the canonical entity for a business concept.
    assertions:
      - type: search_rank
        query: orders
        expected_unique_id: model.pkg.orders
        max_rank: 5
      - type: context_has
        id_or_name: model.pkg.orders
        fields:
          - data.unique_id
          - data.entity.name
agent_cases:
  - id: analyst_metric_lookup
    task: Which canonical model and indicator should be used to analyze gross merchandise value?
    expected:
      must_call:
        - search_indicator
        - get_context
      selected_entities:
        - model.pkg.orders
      final_answer:
        must_contain:
          - gross merchandise value
"
    )
}

pub(super) fn empty_object() -> JsonValue {
    json!({})
}

pub(super) fn default_sql_structure_tool() -> String {
    "execute_sql".to_string()
}

pub(super) fn default_top_k() -> usize {
    DEFAULT_TOP_K
}

pub(super) fn safe_path_segment(value: &str) -> String {
    let sanitized: String = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = sanitized.trim_matches(['-', '.']).trim();
    let capped: String = trimmed.chars().take(MAX_SAFE_PATH_SEGMENT_CHARS).collect();
    if capped.is_empty() || matches!(capped.as_str(), "." | "..") {
        "eval".to_string()
    } else {
        capped
    }
}

pub(super) fn truncate(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

pub(super) fn timestamp_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

pub(super) fn elapsed_ms_to_u64(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

pub(super) fn ratio(numerator: usize, denominator: usize) -> f64 {
    let numerator = u32::try_from(numerator).unwrap_or(u32::MAX);
    let denominator = u32::try_from(denominator).unwrap_or(u32::MAX);
    f64::from(numerator) / f64::from(denominator)
}

pub(super) fn server_error(message: String) -> DbtNovaError {
    DbtNovaError::ServerError(message)
}
