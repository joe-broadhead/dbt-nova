use super::{
    AgentCalledWith, AgentEntityRank, AgentExpected, AgentOrder, AgentSqlStructureExpected,
    AssertionResult, BTreeSet, FinalAnswerExpected, JsonValue, Path, SqlStructureSignature,
    compare_sql_structure, compare_sql_structure_signatures, fs, json, read_tool_trace_file,
    redact_provider_output_text, sql_structure_signature, truncate,
};

pub(super) fn score_agent_expectations(
    expected: &AgentExpected,
    trace: &[JsonValue],
    final_answer_text: &str,
) -> Vec<AssertionResult> {
    let mut assertions = Vec::new();
    for tool in &expected.must_call {
        if first_tool_index(trace, tool).is_some() {
            assertions.push(AssertionResult::pass(
                format!("must_call:{tool}"),
                "required tool was called",
                JsonValue::Null,
            ));
        } else {
            assertions.push(AssertionResult::fail(
                format!("must_call:{tool}"),
                "required tool was not called",
                json!({"observed_tools": observed_tools(trace)}),
            ));
        }
    }
    for tool in &expected.must_not_call {
        if first_tool_index(trace, tool).is_some() {
            assertions.push(AssertionResult::fail(
                format!("must_not_call:{tool}"),
                "forbidden tool was called",
                json!({"observed_tools": observed_tools(trace)}),
            ));
        } else {
            assertions.push(AssertionResult::pass(
                format!("must_not_call:{tool}"),
                "forbidden tool was not called",
                JsonValue::Null,
            ));
        }
    }
    for order in &expected.ordered {
        assertions.push(order_assertion(trace, order));
    }
    for entity in &expected.selected_entities {
        if trace_selected_entity(trace, entity) {
            assertions.push(AssertionResult::pass(
                format!("selected_entity:{entity}"),
                "expected entity appeared in tool evidence",
                JsonValue::Null,
            ));
        } else {
            assertions.push(AssertionResult::fail(
                format!("selected_entity:{entity}"),
                "expected entity did not appear in tool evidence",
                json!({"selected_unique_ids": selected_entities(trace)}),
            ));
        }
    }
    for entity_rank in &expected.selected_entity_ranks {
        assertions.push(selected_entity_rank_assertion(trace, entity_rank));
    }
    for called_with in &expected.called_with {
        assertions.push(called_with_assertion(trace, called_with));
    }
    for sql_structure in &expected.sql_structures {
        assertions.push(agent_sql_structure_assertion(trace, sql_structure));
    }
    if let Some(max_tool_calls) = expected.max_tool_calls {
        assertions.push(max_tool_calls_assertion(trace, max_tool_calls));
    }
    if let Some(max_distinct_tools) = expected.max_distinct_tools {
        assertions.push(max_distinct_tools_assertion(trace, max_distinct_tools));
    }
    if let Some(max_total_response_bytes) = expected.max_total_response_bytes {
        assertions.push(max_total_response_bytes_assertion(
            trace,
            max_total_response_bytes,
        ));
    }
    for (tool, max_bytes) in &expected.max_response_bytes_by_tool {
        assertions.push(max_response_bytes_by_tool_assertion(
            trace, tool, *max_bytes,
        ));
    }
    if let Some(final_answer) = expected.final_answer.as_ref() {
        assertions.extend(score_final_answer(final_answer, final_answer_text));
    }
    assertions
}

pub(super) fn sql_structure_assertion(
    name: impl Into<String>,
    actual_sql: &str,
    expected_sql: &str,
) -> AssertionResult {
    let name = name.into();
    match compare_sql_structure(actual_sql, expected_sql) {
        Ok(comparison) => sql_structure_comparison_assertion(name, &comparison),
        Err(error) => AssertionResult::error(name, error),
    }
}

pub(super) fn sql_structure_comparison_assertion(
    name: impl Into<String>,
    comparison: &crate::utils::sql_structure::SqlStructureComparison,
) -> AssertionResult {
    let clauses = comparison.diff.changed_clauses();
    if comparison.matches {
        AssertionResult::pass(
            name,
            "SQL structure matched expected SELECT, FROM/JOIN, WHERE, and GROUP BY clauses",
            sql_structure_evidence(comparison),
        )
    } else {
        let clause_text = if clauses.is_empty() {
            "unknown".to_string()
        } else {
            clauses.join(", ")
        };
        AssertionResult::fail(
            name,
            format!("SQL structure differed in clauses: {clause_text}"),
            sql_structure_evidence(comparison),
        )
    }
}

pub(super) fn sql_structure_evidence(
    comparison: &crate::utils::sql_structure::SqlStructureComparison,
) -> JsonValue {
    json!({
        "grade_mode": "query_structure",
        "changed_clauses": comparison.diff.changed_clauses(),
        "expected": &comparison.expected,
        "actual": &comparison.actual,
        "diff": &comparison.diff,
    })
}

pub(super) fn agent_sql_structure_assertion(
    trace: &[JsonValue],
    expected: &AgentSqlStructureExpected,
) -> AssertionResult {
    let name = format!("sql_structure:{}", expected.tool);
    let expected_signature = match sql_structure_signature(&expected.expected_sql) {
        Ok(signature) => signature,
        Err(error) => return AssertionResult::error(name, error),
    };
    let matching_rows = trace
        .iter()
        .filter(|row| row.get("tool").and_then(JsonValue::as_str) == Some(expected.tool.as_str()))
        .collect::<Vec<_>>();
    let mut first_comparison = None;
    let mut observed_errors = Vec::new();
    let mut observed_structure_count = 0usize;

    for row in &matching_rows {
        let Some(summary) = row.get("params_summary") else {
            continue;
        };
        if let Some(error) = summary
            .get("statement_structure_error")
            .and_then(JsonValue::as_str)
        {
            observed_errors.push(error.to_string());
        }
        let Some(structure) = summary.get("statement_structure") else {
            continue;
        };
        observed_structure_count += 1;
        let actual_signature =
            match serde_json::from_value::<SqlStructureSignature>(structure.clone()) {
                Ok(signature) => signature,
                Err(error) => {
                    observed_errors.push(format!("invalid statement_structure summary: {error}"));
                    continue;
                }
            };
        let comparison =
            compare_sql_structure_signatures(actual_signature, expected_signature.clone());
        if comparison.matches {
            return sql_structure_comparison_assertion(name, &comparison);
        }
        if first_comparison.is_none() {
            first_comparison = Some(comparison);
        }
    }

    if let Some(comparison) = first_comparison {
        return sql_structure_comparison_assertion(name, &comparison);
    }

    AssertionResult::fail(
        name,
        "no observed tool call included a matching SQL structure summary",
        json!({
            "grade_mode": "query_structure",
            "tool": expected.tool,
            "matching_tool_calls": matching_rows.len(),
            "statement_structure_count": observed_structure_count,
            "statement_structure_errors": observed_errors,
            "observed": observed_params_for_tool(trace, &expected.tool),
        }),
    )
}

pub(super) fn max_tool_calls_assertion(
    trace: &[JsonValue],
    max_tool_calls: usize,
) -> AssertionResult {
    let observed = trace.len();
    if observed <= max_tool_calls {
        AssertionResult::pass(
            "max_tool_calls",
            "observed tool call count stayed within budget",
            json!({"observed": observed, "max": max_tool_calls}),
        )
    } else {
        AssertionResult::fail(
            "max_tool_calls",
            "observed tool call count exceeded budget",
            json!({"observed": observed, "max": max_tool_calls, "tools": observed_tools(trace)}),
        )
    }
}

pub(super) fn max_distinct_tools_assertion(
    trace: &[JsonValue],
    max_distinct_tools: usize,
) -> AssertionResult {
    let distinct: BTreeSet<String> = trace
        .iter()
        .filter_map(|row| row.get("tool").and_then(JsonValue::as_str))
        .map(ToString::to_string)
        .collect();
    let observed = distinct.len();
    if observed <= max_distinct_tools {
        AssertionResult::pass(
            "max_distinct_tools",
            "observed distinct tool count stayed within budget",
            json!({"observed": observed, "max": max_distinct_tools}),
        )
    } else {
        AssertionResult::fail(
            "max_distinct_tools",
            "observed distinct tool count exceeded budget",
            json!({"observed": observed, "max": max_distinct_tools, "tools": distinct}),
        )
    }
}

pub(super) fn max_total_response_bytes_assertion(
    trace: &[JsonValue],
    max_total_response_bytes: usize,
) -> AssertionResult {
    let missing_response_bytes = trace_rows_missing_response_bytes(trace);
    if missing_response_bytes > 0 {
        return AssertionResult::fail(
            "max_total_response_bytes",
            "tool trace rows were missing response byte telemetry",
            json!({"missing_response_bytes": missing_response_bytes, "trace_rows": trace.len()}),
        );
    }
    let observed = total_response_bytes(trace);
    if observed <= max_total_response_bytes {
        AssertionResult::pass(
            "max_total_response_bytes",
            "observed total response bytes stayed within budget",
            json!({"observed": observed, "max": max_total_response_bytes}),
        )
    } else {
        AssertionResult::fail(
            "max_total_response_bytes",
            "observed total response bytes exceeded budget",
            json!({"observed": observed, "max": max_total_response_bytes}),
        )
    }
}

pub(super) fn max_response_bytes_by_tool_assertion(
    trace: &[JsonValue],
    tool: &str,
    max_response_bytes: usize,
) -> AssertionResult {
    let matching_rows: Vec<&JsonValue> = trace
        .iter()
        .filter(|row| row.get("tool").and_then(JsonValue::as_str) == Some(tool))
        .collect();
    let missing_response_bytes = matching_rows
        .iter()
        .filter(|row| response_bytes_from_trace_row(row).is_none())
        .count();
    if missing_response_bytes > 0 {
        return AssertionResult::fail(
            format!("max_response_bytes_by_tool:{tool}"),
            "tool trace rows were missing response byte telemetry",
            json!({"missing_response_bytes": missing_response_bytes, "tool": tool}),
        );
    }
    let observed = matching_rows
        .iter()
        .filter_map(|row| response_bytes_from_trace_row(row))
        .max()
        .unwrap_or(0);
    if observed <= max_response_bytes {
        AssertionResult::pass(
            format!("max_response_bytes_by_tool:{tool}"),
            "observed per-tool response bytes stayed within budget",
            json!({"observed": observed, "max": max_response_bytes}),
        )
    } else {
        AssertionResult::fail(
            format!("max_response_bytes_by_tool:{tool}"),
            "observed per-tool response bytes exceeded budget",
            json!({"observed": observed, "max": max_response_bytes}),
        )
    }
}

pub(super) fn total_response_bytes(trace: &[JsonValue]) -> usize {
    trace.iter().filter_map(response_bytes_from_trace_row).sum()
}

pub(super) fn trace_rows_missing_response_bytes(trace: &[JsonValue]) -> usize {
    trace
        .iter()
        .filter(|row| response_bytes_from_trace_row(row).is_none())
        .count()
}

pub(super) fn response_bytes_from_trace_row(row: &JsonValue) -> Option<usize> {
    row.get("response_bytes")
        .and_then(JsonValue::as_u64)
        .and_then(|value| usize::try_from(value).ok())
}

pub(super) fn order_assertion(trace: &[JsonValue], order: &AgentOrder) -> AssertionResult {
    let before_index = first_tool_index(trace, &order.before);
    let missing: Vec<&str> = order
        .must_have_called
        .iter()
        .filter_map(|tool| {
            let index = first_tool_index(trace, tool);
            if index.is_none()
                || before_index.is_none_or(|before| index.unwrap_or(usize::MAX) >= before)
            {
                Some(tool.as_str())
            } else {
                None
            }
        })
        .collect();

    if before_index.is_some() && missing.is_empty() {
        AssertionResult::pass(
            format!("order:{}", order.before),
            "tool order matched",
            JsonValue::Null,
        )
    } else {
        AssertionResult::fail(
            format!("order:{}", order.before),
            "tool order did not match",
            json!({
                "before": order.before,
                "must_have_called": order.must_have_called,
                "observed_tools": observed_tools(trace),
            }),
        )
    }
}

pub(super) fn score_final_answer(
    expected: &FinalAnswerExpected,
    final_answer_text: &str,
) -> Vec<AssertionResult> {
    let haystack = final_answer_text.to_lowercase();
    let mut assertions = Vec::new();
    for needle in &expected.must_contain {
        if haystack.contains(&needle.to_lowercase()) {
            assertions.push(AssertionResult::pass(
                format!("final_answer_contains:{needle}"),
                "final answer contained expected text",
                JsonValue::Null,
            ));
        } else {
            assertions.push(AssertionResult::fail(
                format!("final_answer_contains:{needle}"),
                "final answer did not contain expected text",
                json!({"final_answer": truncate(&redact_provider_output_text(final_answer_text), 4000)}),
            ));
        }
    }
    for needle in &expected.must_not_contain {
        if haystack.contains(&needle.to_lowercase()) {
            assertions.push(AssertionResult::fail(
                format!("final_answer_excludes:{needle}"),
                "final answer contained forbidden text",
                json!({"final_answer": truncate(&redact_provider_output_text(final_answer_text), 4000)}),
            ));
        } else {
            assertions.push(AssertionResult::pass(
                format!("final_answer_excludes:{needle}"),
                "final answer excluded forbidden text",
                JsonValue::Null,
            ));
        }
    }
    assertions
}

pub(super) fn first_tool_index(trace: &[JsonValue], tool: &str) -> Option<usize> {
    trace
        .iter()
        .position(|row| row.get("tool").and_then(JsonValue::as_str) == Some(tool))
}

pub(super) fn observed_tools(trace: &[JsonValue]) -> Vec<String> {
    trace
        .iter()
        .filter_map(|row| row.get("tool").and_then(JsonValue::as_str))
        .map(ToString::to_string)
        .collect()
}

pub(super) fn trace_selected_entity(trace: &[JsonValue], entity: &str) -> bool {
    trace.iter().any(|row| {
        row.get("selected_unique_ids")
            .and_then(JsonValue::as_array)
            .is_some_and(|ids| ids.iter().any(|id| id.as_str() == Some(entity)))
    })
}

pub(super) fn selected_entities(trace: &[JsonValue]) -> Vec<String> {
    let mut out = BTreeSet::new();
    for row in trace {
        if let Some(ids) = row.get("selected_unique_ids").and_then(JsonValue::as_array) {
            for id in ids {
                if let Some(id) = id.as_str() {
                    out.insert(id.to_string());
                }
            }
        }
    }
    out.into_iter().collect()
}

pub(super) fn selected_entity_rank_assertion(
    trace: &[JsonValue],
    expected: &AgentEntityRank,
) -> AssertionResult {
    let rank = trace_entity_rank(trace, &expected.unique_id, expected.tool.as_deref());
    match (rank, expected.max_rank) {
        (Some(rank), Some(max_rank)) if rank <= max_rank => AssertionResult::pass(
            format!("selected_entity_rank:{}", expected.unique_id),
            format!("expected entity appeared at rank {rank}"),
            json!({"rank": rank, "max_rank": max_rank, "tool": expected.tool}),
        ),
        (Some(rank), Some(max_rank)) => AssertionResult::fail(
            format!("selected_entity_rank:{}", expected.unique_id),
            format!("expected entity appeared at rank {rank}, above max rank {max_rank}"),
            json!({
                "rank": rank,
                "max_rank": max_rank,
                "tool": expected.tool,
                "top_unique_ids": top_unique_ids(trace, expected.tool.as_deref()),
            }),
        ),
        (Some(rank), None) => AssertionResult::pass(
            format!("selected_entity_rank:{}", expected.unique_id),
            format!("expected entity appeared at rank {rank}"),
            json!({"rank": rank, "tool": expected.tool}),
        ),
        (None, _) => AssertionResult::fail(
            format!("selected_entity_rank:{}", expected.unique_id),
            "expected entity did not appear in ranked tool evidence",
            json!({
                "tool": expected.tool,
                "top_unique_ids": top_unique_ids(trace, expected.tool.as_deref()),
            }),
        ),
    }
}

pub(super) fn trace_entity_rank(
    trace: &[JsonValue],
    entity: &str,
    tool: Option<&str>,
) -> Option<usize> {
    trace
        .iter()
        .filter(|row| {
            tool.is_none_or(|tool| row.get("tool").and_then(JsonValue::as_str) == Some(tool))
        })
        .filter_map(|row| {
            row.get("top_unique_ids")
                .and_then(JsonValue::as_array)
                .and_then(|ids| ids.iter().position(|id| id.as_str() == Some(entity)))
        })
        .map(|index| index + 1)
        .min()
}

pub(super) fn top_unique_ids(trace: &[JsonValue], tool: Option<&str>) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    for row in trace.iter().filter(|row| {
        tool.is_none_or(|tool| row.get("tool").and_then(JsonValue::as_str) == Some(tool))
    }) {
        if let Some(ids) = row.get("top_unique_ids").and_then(JsonValue::as_array) {
            for id in ids {
                if let Some(id) = id.as_str() {
                    let id = id.to_string();
                    if seen.insert(id.clone()) {
                        out.push(id);
                    }
                }
            }
        }
    }
    out
}

pub(super) fn called_with_assertion(
    trace: &[JsonValue],
    expected: &AgentCalledWith,
) -> AssertionResult {
    if trace.iter().any(|row| called_with_matches(row, expected)) {
        return AssertionResult::pass(
            format!("called_with:{}", expected.tool),
            "tool call parameters matched",
            json!({"tool": expected.tool}),
        );
    }
    AssertionResult::fail(
        format!("called_with:{}", expected.tool),
        "no observed tool call matched the expected safe parameters",
        json!({
            "expected": {
                "params": &expected.params,
                "contains": &expected.contains,
            },
            "observed": observed_params_for_tool(trace, &expected.tool),
        }),
    )
}

pub(super) fn called_with_matches(row: &JsonValue, expected: &AgentCalledWith) -> bool {
    if row.get("tool").and_then(JsonValue::as_str) != Some(expected.tool.as_str()) {
        return false;
    }
    let Some(summary) = row.get("params_summary").and_then(JsonValue::as_object) else {
        return expected.params.is_empty() && expected.contains.is_empty();
    };
    expected.params.iter().all(|(key, value)| {
        summary
            .get(key)
            .is_some_and(|actual| param_value_matches(actual, value))
    }) && expected.contains.iter().all(|(key, value)| {
        summary
            .get(key)
            .is_some_and(|actual| json_contains_string(actual, value))
    })
}

pub(super) fn param_value_matches(actual: &JsonValue, expected: &JsonValue) -> bool {
    match (actual, expected) {
        (JsonValue::String(actual), JsonValue::String(expected)) => {
            actual.eq_ignore_ascii_case(expected)
        }
        (JsonValue::Array(actual_items), JsonValue::Array(expected_items)) => {
            expected_items.iter().all(|expected| {
                actual_items
                    .iter()
                    .any(|actual| param_value_matches(actual, expected))
            })
        }
        (JsonValue::Array(actual_items), expected) => actual_items
            .iter()
            .any(|actual| param_value_matches(actual, expected)),
        _ => actual == expected,
    }
}

pub(super) fn observed_params_for_tool(trace: &[JsonValue], tool: &str) -> JsonValue {
    JsonValue::Array(
        trace
            .iter()
            .filter(|row| row.get("tool").and_then(JsonValue::as_str) == Some(tool))
            .filter_map(|row| row.get("params_summary").cloned())
            .collect(),
    )
}

pub(super) struct ToolTraceRead {
    pub(super) rows: Vec<JsonValue>,
    pub(super) errors: Vec<String>,
    pub(super) missing: bool,
}

pub(super) fn reset_trace_file(path: &Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, "")
}

pub(super) fn read_tool_trace(path: &Path) -> ToolTraceRead {
    let read = read_tool_trace_file(path);
    let mut errors = Vec::new();
    if let Some(error) = read.read_error {
        errors.push(error);
    }
    errors.extend(read.parse_warnings.into_iter().map(|warning| {
        format!(
            "failed to parse tool trace line {} in '{}': {}",
            warning.line,
            path.display(),
            warning.message
        )
    }));
    ToolTraceRead {
        rows: read.rows,
        errors,
        missing: read.missing,
    }
}

pub(super) fn rank_assertion(
    name: &str,
    response: &JsonValue,
    expected: &str,
    max_rank: Option<usize>,
    field: &str,
) -> AssertionResult {
    let rows = data_rows(response);
    if let Some(index) = rows
        .iter()
        .position(|row| string_field_equals(row, field, expected))
    {
        let rank = index + 1;
        if max_rank.is_none_or(|max| rank <= max) {
            AssertionResult::pass(
                name,
                format!("expected item ranked {rank}"),
                json!({"rank": rank, "expected": expected}),
            )
        } else {
            AssertionResult::fail(
                name,
                format!(
                    "expected item ranked {rank}, above max rank {}",
                    max_rank.unwrap_or(0)
                ),
                top_evidence(rows, field),
            )
        }
    } else {
        AssertionResult::fail(
            name,
            "expected item was not returned",
            json!({"expected": expected, "top": top_evidence(rows, field)}),
        )
    }
}

pub(super) fn contains_rank_assertion(
    name: &str,
    response: &JsonValue,
    expected: &str,
    max_rank: Option<usize>,
) -> AssertionResult {
    let rows = data_rows(response);
    if let Some(index) = rows
        .iter()
        .position(|row| json_contains_string(row, expected))
    {
        let rank = index + 1;
        if max_rank.is_none_or(|max| rank <= max) {
            AssertionResult::pass(
                name,
                format!("expected value ranked {rank}"),
                json!({"rank": rank, "expected": expected}),
            )
        } else {
            AssertionResult::fail(
                name,
                format!(
                    "expected value ranked {rank}, above max rank {}",
                    max_rank.unwrap_or(0)
                ),
                top_evidence(rows, "unique_id"),
            )
        }
    } else {
        AssertionResult::fail(
            name,
            "expected value was not returned",
            json!({"expected": expected, "top": top_evidence(rows, "unique_id")}),
        )
    }
}

pub(super) fn recipe_rank_assertion(
    response: &JsonValue,
    expected_recipe_id: &str,
    max_rank: Option<usize>,
) -> AssertionResult {
    let rows = data_rows(response);
    if let Some(index) = rows.iter().position(|row| {
        string_field_equals(row, "recipe_id", expected_recipe_id)
            || string_field_equals(row, "id", expected_recipe_id)
    }) {
        let rank = index + 1;
        if max_rank.is_none_or(|max| rank <= max) {
            AssertionResult::pass(
                "recipe_rank",
                format!("expected recipe ranked {rank}"),
                json!({"rank": rank, "expected": expected_recipe_id}),
            )
        } else {
            AssertionResult::fail(
                "recipe_rank",
                format!(
                    "expected recipe ranked {rank}, above max rank {}",
                    max_rank.unwrap_or(0)
                ),
                recipe_top_evidence(rows),
            )
        }
    } else {
        AssertionResult::fail(
            "recipe_rank",
            "expected recipe was not returned",
            json!({"expected": expected_recipe_id, "top": recipe_top_evidence(rows)}),
        )
    }
}

pub(super) fn search_columns_assertion(
    response: &JsonValue,
    expected_column: Option<&str>,
    expected_parent_unique_id: Option<&str>,
    max_rank: Option<usize>,
) -> AssertionResult {
    let rows = data_rows(response);
    let position = rows.iter().position(|row| {
        expected_column.is_none_or(|column| json_contains_string(row, column))
            && expected_parent_unique_id
                .is_none_or(|parent| string_field_equals(row, "parent_unique_id", parent))
    });
    if let Some(index) = position {
        let rank = index + 1;
        if max_rank.is_none_or(|max| rank <= max) {
            AssertionResult::pass(
                "search_columns_rank",
                format!("expected column result ranked {rank}"),
                json!({"rank": rank}),
            )
        } else {
            AssertionResult::fail(
                "search_columns_rank",
                format!(
                    "expected column result ranked {rank}, above max rank {}",
                    max_rank.unwrap_or(0)
                ),
                top_evidence(rows, "parent_unique_id"),
            )
        }
    } else {
        AssertionResult::fail(
            "search_columns_rank",
            "expected column result was not returned",
            top_evidence(rows, "parent_unique_id"),
        )
    }
}

pub(super) fn fields_assertion(
    name: &str,
    response: &JsonValue,
    fields: &[String],
) -> AssertionResult {
    let missing: Vec<&str> = fields
        .iter()
        .map(String::as_str)
        .filter(|field| !json_has_field_path(response, field))
        .collect();
    if missing.is_empty() {
        AssertionResult::pass(name, "required fields were present", JsonValue::Null)
    } else {
        AssertionResult::fail(
            name,
            "required fields were missing",
            json!({"missing": missing}),
        )
    }
}

pub(super) fn context_field_equals_assertion(
    response: &JsonValue,
    field: &str,
    expected: &JsonValue,
) -> AssertionResult {
    match json_value_at_path(response, field) {
        Some(actual) if actual == expected => AssertionResult::pass(
            "context_field_equals",
            "context field matched expected value",
            json!({"field": field, "expected": expected}),
        ),
        Some(actual) => AssertionResult::fail(
            "context_field_equals",
            "context field did not match expected value",
            json!({"field": field, "expected": expected, "actual": actual}),
        ),
        None => AssertionResult::fail(
            "context_field_equals",
            "context field was missing",
            json!({"field": field, "expected": expected}),
        ),
    }
}

pub(super) fn context_contains_assertion(
    response: &JsonValue,
    field: Option<&str>,
    expected: &str,
) -> AssertionResult {
    let target = field.and_then(|field| json_value_at_path(response, field));
    let contains = if let Some(value) = target {
        json_contains_string(value, expected)
    } else if field.is_some() {
        false
    } else {
        json_contains_string(response, expected)
    };
    if contains {
        AssertionResult::pass(
            "context_contains",
            "expected value appeared in context",
            json!({"field": field, "expected": expected}),
        )
    } else {
        AssertionResult::fail(
            "context_contains",
            "expected value did not appear in context",
            json!({"field": field, "expected": expected}),
        )
    }
}

pub(super) fn metadata_score_min_assertion(
    response: &JsonValue,
    threshold: f64,
) -> AssertionResult {
    let score = find_score(response);
    match score {
        Some(score) if score >= threshold => AssertionResult::pass(
            "metadata_score_min",
            format!("metadata score {score:.3} met threshold {threshold:.3}"),
            json!({"score": score, "threshold": threshold}),
        ),
        Some(score) => AssertionResult::fail(
            "metadata_score_min",
            format!("metadata score {score:.3} was below threshold {threshold:.3}"),
            json!({"score": score, "threshold": threshold}),
        ),
        None => AssertionResult::fail(
            "metadata_score_min",
            "metadata score response did not contain a numeric score",
            JsonValue::Null,
        ),
    }
}

pub(super) fn metadata_score_max_assertion(
    response: &JsonValue,
    threshold: f64,
) -> AssertionResult {
    let score = find_score(response);
    match score {
        Some(score) if score <= threshold => AssertionResult::pass(
            "metadata_score_max",
            format!("metadata score {score:.3} did not exceed threshold {threshold:.3}"),
            json!({"score": score, "threshold": threshold}),
        ),
        Some(score) => AssertionResult::fail(
            "metadata_score_max",
            format!("metadata score {score:.3} exceeded threshold {threshold:.3}"),
            json!({"score": score, "threshold": threshold}),
        ),
        None => AssertionResult::fail(
            "metadata_score_max",
            "metadata score response did not contain a numeric score",
            JsonValue::Null,
        ),
    }
}

pub(super) fn recipe_queries_assertion(
    response: &JsonValue,
    min_queries: usize,
) -> AssertionResult {
    let query_count = count_recipe_queries(response);
    if query_count >= min_queries {
        AssertionResult::pass(
            "recipe_has_queries",
            format!("recipe contained {query_count} queries"),
            json!({"query_count": query_count, "min_queries": min_queries}),
        )
    } else {
        AssertionResult::fail(
            "recipe_has_queries",
            format!("recipe contained {query_count} queries, below minimum {min_queries}"),
            json!({"query_count": query_count, "min_queries": min_queries}),
        )
    }
}

pub(super) fn contains_string_assertion(
    name: &str,
    response: &JsonValue,
    expected: &str,
) -> AssertionResult {
    if json_contains_string(response, expected) {
        AssertionResult::pass(
            name,
            "expected value appeared in response",
            json!({"expected": expected}),
        )
    } else {
        AssertionResult::fail(
            name,
            "expected value did not appear in response",
            json!({"expected": expected}),
        )
    }
}

pub(super) fn tool_success_assertion(tool: &str, response: &JsonValue) -> AssertionResult {
    if response.get("success").and_then(JsonValue::as_bool) == Some(false) {
        return AssertionResult::fail(
            format!("tool_success:{tool}"),
            "tool returned an explicit success=false response",
            tool_failure_evidence(response),
        );
    }
    AssertionResult::pass(
        format!("tool_success:{tool}"),
        "tool returned success",
        json!({"count": response.get("count").cloned().unwrap_or(JsonValue::Null)}),
    )
}

pub(super) fn tool_response_budget_assertion(
    tool: &str,
    response: &JsonValue,
    max_response_bytes: usize,
    must_contain_paths: &[String],
    must_not_contain_paths: &[String],
) -> AssertionResult {
    let response_bytes = serde_json::to_string(response).map_or(usize::MAX, |value| value.len());
    let missing_paths: Vec<&str> = must_contain_paths
        .iter()
        .map(String::as_str)
        .filter(|path| !json_has_field_path(response, path))
        .collect();
    let present_forbidden_paths: Vec<&str> = must_not_contain_paths
        .iter()
        .map(String::as_str)
        .filter(|path| json_has_field_path(response, path))
        .collect();
    if response_bytes <= max_response_bytes
        && missing_paths.is_empty()
        && present_forbidden_paths.is_empty()
    {
        AssertionResult::pass(
            format!("tool_response_budget:{tool}"),
            "tool response stayed within budget and shape constraints",
            json!({"response_bytes": response_bytes, "max_response_bytes": max_response_bytes}),
        )
    } else {
        AssertionResult::fail(
            format!("tool_response_budget:{tool}"),
            "tool response exceeded budget or shape constraints",
            json!({
                "response_bytes": response_bytes,
                "max_response_bytes": max_response_bytes,
                "missing_paths": missing_paths,
                "present_forbidden_paths": present_forbidden_paths,
            }),
        )
    }
}

pub(super) fn tool_field_equals_assertion(
    tool: &str,
    response: &JsonValue,
    field: &str,
    expected: &JsonValue,
) -> AssertionResult {
    match json_value_at_path(response, field) {
        Some(actual) if actual == expected => AssertionResult::pass(
            format!("tool_field_equals:{tool}:{field}"),
            "tool field matched expected value",
            json!({"field": field, "expected": expected}),
        ),
        Some(actual) => AssertionResult::fail(
            format!("tool_field_equals:{tool}:{field}"),
            "tool field did not match expected value",
            json!({"field": field, "expected": expected, "actual": actual}),
        ),
        None => AssertionResult::fail(
            format!("tool_field_equals:{tool}:{field}"),
            "tool field was missing",
            json!({"field": field, "expected": expected}),
        ),
    }
}

pub(super) fn tool_failure_evidence(response: &JsonValue) -> JsonValue {
    let mut evidence = serde_json::Map::new();
    for key in ["success", "error_code", "code", "message", "count"] {
        if let Some(value) = response.get(key).and_then(safe_evidence_scalar) {
            evidence.insert(key.to_string(), value);
        }
    }
    if let Some(error) = response.get("error").and_then(JsonValue::as_object) {
        for key in ["error_code", "code", "message"] {
            if let Some(value) = error.get(key).and_then(safe_evidence_scalar) {
                evidence.insert(format!("error.{key}"), value);
            }
        }
    }
    if evidence.is_empty()
        && let Some(map) = response.as_object()
    {
        evidence.insert(
            "keys".to_string(),
            JsonValue::Array(
                map.keys()
                    .take(20)
                    .cloned()
                    .map(JsonValue::String)
                    .collect(),
            ),
        );
    }
    JsonValue::Object(evidence)
}

pub(super) fn safe_evidence_scalar(value: &JsonValue) -> Option<JsonValue> {
    match value {
        JsonValue::Null | JsonValue::Bool(_) | JsonValue::Number(_) => Some(value.clone()),
        JsonValue::String(value) => Some(JsonValue::String(truncate(value, 1000))),
        JsonValue::Array(_) | JsonValue::Object(_) => None,
    }
}

pub(super) fn data_rows(response: &JsonValue) -> &[JsonValue] {
    response
        .get("data")
        .and_then(JsonValue::as_array)
        .map_or(&[], Vec::as_slice)
}

pub(super) fn string_field_equals(row: &JsonValue, field: &str, expected: &str) -> bool {
    row.get(field)
        .and_then(JsonValue::as_str)
        .is_some_and(|value| value.eq_ignore_ascii_case(expected))
}

pub(super) fn json_contains_string(value: &JsonValue, expected: &str) -> bool {
    let expected = expected.to_lowercase();
    json_contains_string_lower(value, &expected)
}

pub(super) fn json_contains_string_lower(value: &JsonValue, expected: &str) -> bool {
    match value {
        JsonValue::String(value) => value.to_lowercase().contains(expected),
        JsonValue::Array(items) => items
            .iter()
            .any(|item| json_contains_string_lower(item, expected)),
        JsonValue::Object(map) => map
            .values()
            .any(|child| json_contains_string_lower(child, expected)),
        JsonValue::Null | JsonValue::Bool(_) | JsonValue::Number(_) => false,
    }
}

pub(super) fn json_has_field_path(value: &JsonValue, field_path: &str) -> bool {
    json_value_at_path(value, field_path).is_some_and(|value| !value.is_null())
}

pub(super) fn json_value_at_path<'a>(
    value: &'a JsonValue,
    field_path: &str,
) -> Option<&'a JsonValue> {
    let mut current = value;
    for part in field_path.split('.') {
        current = match current {
            JsonValue::Array(items) => items.get(part.parse::<usize>().ok()?)?,
            JsonValue::Object(_) => current.get(part)?,
            _ => return None,
        };
    }
    Some(current)
}

pub(super) fn find_score(value: &JsonValue) -> Option<f64> {
    for key in ["overall_score", "score", "metadata_score"] {
        if let Some(score) = find_number_by_key(value, key) {
            return Some(score);
        }
    }
    None
}

pub(super) fn find_number_by_key(value: &JsonValue, key: &str) -> Option<f64> {
    match value {
        JsonValue::Object(map) => {
            if let Some(score) = map.get(key).and_then(JsonValue::as_f64) {
                return Some(score);
            }
            map.values()
                .find_map(|child| find_number_by_key(child, key))
        }
        JsonValue::Array(items) => items.iter().find_map(|item| find_number_by_key(item, key)),
        JsonValue::Null | JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::String(_) => None,
    }
}

pub(super) fn count_recipe_queries(value: &JsonValue) -> usize {
    match value {
        JsonValue::Object(map) => {
            for key in ["queries", "query_names"] {
                if let Some(count) = map.get(key).and_then(JsonValue::as_array).map(Vec::len) {
                    return count;
                }
            }
            map.values().map(count_recipe_queries).max().unwrap_or(0)
        }
        JsonValue::Array(items) => items.iter().map(count_recipe_queries).max().unwrap_or(0),
        JsonValue::Null | JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::String(_) => 0,
    }
}

pub(super) fn top_evidence(rows: &[JsonValue], field: &str) -> JsonValue {
    JsonValue::Array(
        rows.iter()
            .take(10)
            .map(|row| {
                json!({
                    field: row.get(field).cloned().unwrap_or(JsonValue::Null),
                    "name": row.get("name").cloned().unwrap_or(JsonValue::Null),
                    "unique_id": row.get("unique_id").cloned().unwrap_or(JsonValue::Null),
                })
            })
            .collect(),
    )
}

pub(super) fn recipe_top_evidence(rows: &[JsonValue]) -> JsonValue {
    JsonValue::Array(
        rows.iter()
            .take(10)
            .map(|row| {
                json!({
                    "id": row.get("id").cloned().unwrap_or(JsonValue::Null),
                    "recipe_id": row.get("recipe_id").cloned().unwrap_or(JsonValue::Null),
                    "topic": row.get("topic").cloned().unwrap_or(JsonValue::Null),
                })
            })
            .collect(),
    )
}

pub(super) fn effective_limit(max_rank: Option<usize>, default_top_k: usize) -> usize {
    max_rank.unwrap_or(default_top_k).max(1)
}
