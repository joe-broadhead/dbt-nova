#![allow(clippy::too_many_lines)]

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{Value as JsonValue, json};

use crate::cli::args::{EvalAgentRunArgs, EvalInitArgs, EvalRunArgs, ManifestLoadArgs};
use crate::cli::manifest::{build_manifest_load_config, execute_manifest_load};
use crate::cli::output::CliEnvelope;
use crate::cli::{DispatchError, DispatchResult};
use crate::error::DbtNovaError;
use crate::manifest::ManifestSearch;

const DEFAULT_TOP_K: usize = 5;
const DEFAULT_FAIL_UNDER: f64 = 1.0;
const MAX_SAFE_PATH_SEGMENT_CHARS: usize = 120;

mod provider;

#[derive(Debug, Deserialize)]
struct EvalSuite {
    version: u32,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    defaults: EvalDefaults,
    #[serde(default)]
    cases: Vec<EvalCase>,
    #[serde(default)]
    agent_cases: Vec<AgentCase>,
}

#[derive(Debug, Default, Deserialize)]
struct EvalDefaults {
    #[serde(default)]
    persona: Option<String>,
    #[serde(default = "default_top_k")]
    top_k: usize,
}

#[derive(Debug, Deserialize)]
struct EvalCase {
    id: String,
    #[serde(default)]
    question: Option<String>,
    #[serde(default)]
    persona: Option<String>,
    assertions: Vec<EvalAssertion>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum EvalAssertion {
    SearchRank {
        query: String,
        expected_unique_id: String,
        #[serde(default)]
        max_rank: Option<usize>,
        #[serde(default)]
        resource_types: Vec<String>,
        #[serde(default)]
        persona: Option<String>,
    },
    SearchIndicatorRank {
        query: String,
        expected: String,
        #[serde(default)]
        max_rank: Option<usize>,
        #[serde(default)]
        resource_types: Vec<String>,
        #[serde(default)]
        indicator_types: Vec<String>,
        #[serde(default)]
        persona: Option<String>,
    },
    SearchColumnsRank {
        query: String,
        #[serde(default)]
        expected_column: Option<String>,
        #[serde(default)]
        expected_parent_unique_id: Option<String>,
        #[serde(default)]
        max_rank: Option<usize>,
    },
    ContextHas {
        id_or_name: String,
        fields: Vec<String>,
    },
    MetadataScoreMin {
        #[serde(default)]
        id_or_name: Option<String>,
        threshold: f64,
        #[serde(default)]
        persona: Option<String>,
    },
    RecipeRank {
        query: String,
        expected_recipe_id: String,
        #[serde(default)]
        max_rank: Option<usize>,
    },
    RecipeHasQueries {
        recipe_id: String,
        #[serde(default)]
        min_queries: Option<usize>,
    },
    LineageContains {
        id_or_name: String,
        direction: String,
        expected_unique_id: String,
        #[serde(default)]
        depth: Option<usize>,
    },
    ToolSuccess {
        tool: String,
        #[serde(default = "empty_object")]
        params: JsonValue,
    },
}

#[derive(Debug, Deserialize)]
struct AgentCase {
    id: String,
    task: String,
    #[serde(default)]
    expected: AgentExpected,
}

#[derive(Debug, Default, Deserialize)]
struct AgentExpected {
    #[serde(default)]
    must_call: Vec<String>,
    #[serde(default)]
    must_not_call: Vec<String>,
    #[serde(default)]
    ordered: Vec<AgentOrder>,
    #[serde(default)]
    selected_entities: Vec<String>,
    #[serde(default)]
    final_answer: Option<FinalAnswerExpected>,
}

#[derive(Debug, Deserialize)]
struct AgentOrder {
    before: String,
    #[serde(default)]
    must_have_called: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
struct FinalAnswerExpected {
    #[serde(default)]
    must_contain: Vec<String>,
    #[serde(default)]
    must_not_contain: Vec<String>,
}

#[derive(Debug, Serialize)]
struct EvalReport {
    suite_name: String,
    version: u32,
    mode: &'static str,
    output_dir: String,
    assertion_count: usize,
    pass_count: usize,
    fail_count: usize,
    error_count: usize,
    pass_rate: f64,
    fail_under: f64,
    gate_status: &'static str,
    cases: Vec<EvalCaseReport>,
}

#[derive(Debug, Serialize)]
struct EvalCaseReport {
    id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    question: Option<String>,
    pass_count: usize,
    fail_count: usize,
    error_count: usize,
    assertions: Vec<AssertionResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    artifacts: Option<AgentArtifacts>,
}

#[derive(Debug, Serialize)]
struct AssertionResult {
    name: String,
    status: &'static str,
    message: String,
    #[serde(skip_serializing_if = "JsonValue::is_null")]
    evidence: JsonValue,
}

#[derive(Debug, Serialize)]
struct AgentArtifacts {
    stdout: String,
    stderr: String,
    tool_trace: String,
}

/// Writes a starter eval suite.
///
/// # Errors
/// Returns an error when the target exists without `--force` or cannot be written.
pub fn run_init_command(args: &EvalInitArgs) -> DispatchResult {
    let started = Instant::now();
    let path = PathBuf::from(&args.out);
    if path.exists() && !args.force {
        return Err(DbtNovaError::InvalidParams(format!(
            "eval suite '{}' already exists; pass --force to overwrite",
            path.display()
        ))
        .into());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| server_error(error.to_string()))?;
    }

    fs::write(&path, starter_suite(&args.persona))
        .map_err(|error| server_error(error.to_string()))?;
    let payload = json!({
        "path": path.display().to_string(),
        "persona": args.persona,
    });
    let envelope = CliEnvelope::success("eval init", payload, started.elapsed().as_millis());
    let out =
        serde_json::to_string_pretty(&envelope).map_err(|error| server_error(error.to_string()))?;
    println!("{out}");
    Ok(())
}

/// Runs deterministic Nova bridge assertions against a manifest.
///
/// # Errors
/// Returns an error when the suite is invalid, manifest loading fails, assertions error, or the score gate fails.
pub async fn run_eval_command(args: &EvalRunArgs) -> DispatchResult {
    let started = Instant::now();
    let suite = load_suite(&args.suite)?;
    validate_fail_under(args.fail_under)?;
    if suite.cases.is_empty() {
        return Err(
            DbtNovaError::InvalidParams("eval suite contains no bridge cases".to_string()).into(),
        );
    }
    let output_dir = resolve_output_dir(args.output_dir.as_deref(), &suite, "bridge");
    fs::create_dir_all(&output_dir).map_err(|error| server_error(error.to_string()))?;

    let config = build_manifest_load_config(&ManifestLoadArgs {
        manifest_path: args.manifest_path.clone(),
        manifest_uri: args.manifest_uri.clone(),
        storage_instance_id: args.storage_instance_id.clone(),
        cleanup_storage_on_start: args.cleanup_storage_on_start,
        read_only: args.read_only,
        json: false,
    })?;
    let load_result = execute_manifest_load(config).await?;

    let mut cases = Vec::with_capacity(suite.cases.len());
    for case in &suite.cases {
        cases.push(evaluate_bridge_case(case, &suite, &load_result.search).await);
    }
    let report = build_report(
        &suite,
        "bridge",
        output_dir.display().to_string(),
        args.fail_under.unwrap_or(DEFAULT_FAIL_UNDER),
        cases,
    );
    write_report_artifacts(&output_dir, &report, &args.suite)?;
    finish_report(
        "eval run",
        &report,
        args.json,
        started.elapsed().as_millis(),
    )
}

/// Runs agent-provider evals and scores the tool-use trace.
///
/// # Errors
/// Returns an error when the suite is invalid, a provider command cannot be run, or the score gate fails.
pub async fn run_agent_eval_command(args: &EvalAgentRunArgs) -> DispatchResult {
    let started = Instant::now();
    let suite = load_suite(&args.suite)?;
    validate_fail_under(args.fail_under)?;
    if suite.agent_cases.is_empty() {
        return Err(
            DbtNovaError::InvalidParams("eval suite contains no agent_cases".to_string()).into(),
        );
    }
    let output_dir = resolve_output_dir(args.output_dir.as_deref(), &suite, "agent");
    fs::create_dir_all(output_dir.join("tool-calls"))
        .map_err(|error| server_error(error.to_string()))?;

    let mut cases = Vec::with_capacity(suite.agent_cases.len());
    for case in &suite.agent_cases {
        cases.push(run_agent_case(case, args, &output_dir).await);
    }
    let report = build_report(
        &suite,
        "agent",
        output_dir.display().to_string(),
        args.fail_under.unwrap_or(DEFAULT_FAIL_UNDER),
        cases,
    );
    write_report_artifacts(&output_dir, &report, &args.suite)?;
    finish_report(
        "eval agent run",
        &report,
        args.json,
        started.elapsed().as_millis(),
    )
}

async fn evaluate_bridge_case(
    case: &EvalCase,
    suite: &EvalSuite,
    search: &ManifestSearch,
) -> EvalCaseReport {
    let mut assertions = Vec::with_capacity(case.assertions.len());
    for assertion in &case.assertions {
        assertions.push(evaluate_bridge_assertion(assertion, case, suite, search).await);
    }
    EvalCaseReport::new(case.id.clone(), case.question.clone(), assertions, None)
}

async fn evaluate_bridge_assertion(
    assertion: &EvalAssertion,
    case: &EvalCase,
    suite: &EvalSuite,
    search: &ManifestSearch,
) -> AssertionResult {
    match assertion {
        EvalAssertion::SearchRank {
            query,
            expected_unique_id,
            max_rank,
            resource_types,
            persona,
        } => {
            let limit = effective_limit(*max_rank, suite.defaults.top_k);
            let params = json!({
                "query": query,
                "resource_types": resource_types,
                "persona": effective_persona(persona.as_ref(), case, suite),
                "limit": limit,
            });
            match call_tool(search, "search", params).await {
                Ok(response) => rank_assertion(
                    "search_rank",
                    &response,
                    expected_unique_id,
                    *max_rank,
                    "unique_id",
                ),
                Err(error) => AssertionResult::error("search_rank", error.to_string()),
            }
        }
        EvalAssertion::SearchIndicatorRank {
            query,
            expected,
            max_rank,
            resource_types,
            indicator_types,
            persona,
        } => {
            let limit = effective_limit(*max_rank, suite.defaults.top_k);
            let params = json!({
                "query": query,
                "resource_types": resource_types,
                "indicator_types": indicator_types,
                "persona": effective_persona(persona.as_ref(), case, suite),
                "limit": limit,
            });
            match call_tool(search, "search_indicator", params).await {
                Ok(response) => {
                    contains_rank_assertion("search_indicator_rank", &response, expected, *max_rank)
                }
                Err(error) => AssertionResult::error("search_indicator_rank", error.to_string()),
            }
        }
        EvalAssertion::SearchColumnsRank {
            query,
            expected_column,
            expected_parent_unique_id,
            max_rank,
        } => {
            let limit = effective_limit(*max_rank, suite.defaults.top_k);
            let params = json!({"query": query, "limit": limit});
            match call_tool(search, "search_columns", params).await {
                Ok(response) => search_columns_assertion(
                    &response,
                    expected_column.as_deref(),
                    expected_parent_unique_id.as_deref(),
                    *max_rank,
                ),
                Err(error) => AssertionResult::error("search_columns_rank", error.to_string()),
            }
        }
        EvalAssertion::ContextHas { id_or_name, fields } => {
            let params = json!({"id_or_name": id_or_name});
            match call_tool(search, "get_context", params).await {
                Ok(response) => fields_assertion("context_has", &response, fields),
                Err(error) => AssertionResult::error("context_has", error.to_string()),
            }
        }
        EvalAssertion::MetadataScoreMin {
            id_or_name,
            threshold,
            persona,
        } => {
            let params = json!({
                "id_or_name": id_or_name,
                "persona": effective_persona(persona.as_ref(), case, suite),
                "limit": 20,
            });
            match call_tool(search, "get_metadata_score", params).await {
                Ok(response) => metadata_score_assertion(&response, *threshold),
                Err(error) => AssertionResult::error("metadata_score_min", error.to_string()),
            }
        }
        EvalAssertion::RecipeRank {
            query,
            expected_recipe_id,
            max_rank,
        } => {
            let limit = effective_limit(*max_rank, suite.defaults.top_k);
            let params = json!({"query": query, "include_queries": false, "limit": limit});
            match call_tool(search, "search_recipes", params).await {
                Ok(response) => rank_assertion(
                    "recipe_rank",
                    &response,
                    expected_recipe_id,
                    *max_rank,
                    "recipe_id",
                ),
                Err(error) => AssertionResult::error("recipe_rank", error.to_string()),
            }
        }
        EvalAssertion::RecipeHasQueries {
            recipe_id,
            min_queries,
        } => {
            let params = json!({"recipe_id": recipe_id, "include_queries": true});
            match call_tool(search, "get_recipe", params).await {
                Ok(response) => recipe_queries_assertion(&response, min_queries.unwrap_or(1)),
                Err(error) => AssertionResult::error("recipe_has_queries", error.to_string()),
            }
        }
        EvalAssertion::LineageContains {
            id_or_name,
            direction,
            expected_unique_id,
            depth,
        } => {
            let params = json!({
                "id_or_name": id_or_name,
                "direction": direction,
                "depth": depth.unwrap_or(2),
            });
            match call_tool(search, "get_lineage", params).await {
                Ok(response) => {
                    contains_string_assertion("lineage_contains", &response, expected_unique_id)
                }
                Err(error) => AssertionResult::error("lineage_contains", error.to_string()),
            }
        }
        EvalAssertion::ToolSuccess { tool, params } => {
            match call_tool(search, tool, params.clone()).await {
                Ok(response) => AssertionResult::pass(
                    format!("tool_success:{tool}"),
                    "tool returned success",
                    json!({"count": response.get("count").cloned().unwrap_or(JsonValue::Null)}),
                ),
                Err(error) => {
                    AssertionResult::error(format!("tool_success:{tool}"), error.to_string())
                }
            }
        }
    }
}

async fn call_tool(
    search: &ManifestSearch,
    tool: &str,
    params: JsonValue,
) -> crate::error::Result<JsonValue> {
    let started = Instant::now();
    let result = crate::cli::tool::dispatch_tool(search, tool, params.clone()).await;
    match &result {
        Ok(response) => crate::utils::tool_trace::record_tool_call(
            "eval",
            tool,
            Some(&params),
            Some(response),
            true,
            elapsed_ms_to_u64(started.elapsed()),
        ),
        Err(error) => crate::utils::tool_trace::record_tool_call(
            "eval",
            tool,
            Some(&params),
            Some(&json!({"error_code": error.error_code(), "message": error.to_string()})),
            false,
            elapsed_ms_to_u64(started.elapsed()),
        ),
    }
    result
}

async fn run_agent_case(
    case: &AgentCase,
    args: &EvalAgentRunArgs,
    output_dir: &Path,
) -> EvalCaseReport {
    let case_dir = output_dir.join(safe_path_segment(&case.id));
    if let Err(error) = fs::create_dir_all(&case_dir) {
        return EvalCaseReport::new(
            case.id.clone(),
            Some(case.task.clone()),
            vec![AssertionResult::error(
                "provider_exit_success",
                format!("failed to create case artifact directory: {error}"),
            )],
            None,
        );
    }
    let trace_path = output_dir
        .join("tool-calls")
        .join(format!("{}.jsonl", safe_path_segment(&case.id)));
    if let Err(error) = reset_trace_file(&trace_path) {
        return EvalCaseReport::new(
            case.id.clone(),
            Some(case.task.clone()),
            vec![AssertionResult::error(
                "tool_trace_reset",
                format!("failed to reset tool trace file: {error}"),
            )],
            None,
        );
    }
    let prompt = agent_prompt(case);
    let invocation = match provider::provider_invocation(args, &prompt, &trace_path) {
        Ok(invocation) => invocation,
        Err(error) => {
            return EvalCaseReport::new(
                case.id.clone(),
                Some(case.task.clone()),
                vec![AssertionResult::error(
                    "provider_exit_success",
                    error.to_string(),
                )],
                None,
            );
        }
    };

    let output = provider::run_provider_command(&invocation, args, &trace_path, &case_dir).await;
    let (stdout, _stderr, assertions) = match output {
        Ok(output) => {
            if let Err(error) = fs::write(case_dir.join("stdout.log"), &output.stdout) {
                tracing::warn!(error = %error, case_id = %case.id, "failed to write eval stdout");
            }
            if let Err(error) = fs::write(case_dir.join("stderr.log"), &output.stderr) {
                tracing::warn!(error = %error, case_id = %case.id, "failed to write eval stderr");
            }
            let mut assertions = Vec::new();
            if output.status_success {
                assertions.push(AssertionResult::pass(
                    "provider_exit_success",
                    "provider command exited successfully",
                    json!({"command": invocation.command, "args": invocation.args}),
                ));
            } else {
                assertions.push(AssertionResult::fail(
                    "provider_exit_success",
                    "provider command exited with a non-zero status",
                    json!({
                        "command": invocation.command,
                        "args": invocation.args,
                        "stderr": truncate(&output.stderr, 4000),
                    }),
                ));
            }
            (output.stdout, output.stderr, assertions)
        }
        Err(error) => (
            String::new(),
            String::new(),
            vec![AssertionResult::error(
                "provider_exit_success",
                error.to_string(),
            )],
        ),
    };

    let mut assertions = assertions;
    let mut trace = read_tool_trace(&trace_path);
    if trace.rows.is_empty() {
        let provider_trace = provider::read_provider_tool_trace(&stdout);
        if !provider_trace.rows.is_empty() {
            trace.rows = provider_trace.rows;
            trace.errors.extend(provider_trace.errors);
            trace.missing = false;
        }
    }
    if trace.rows.is_empty() && case.expected.requires_trace() {
        assertions.push(AssertionResult::error(
            "tool_trace_missing",
            "tool trace rows were not observed; verify the provider launches a local dbt-nova process, emits MCP tool events, or writes trace rows",
        ));
    }
    for error in &trace.errors {
        assertions.push(AssertionResult::error("tool_trace_parse", error.clone()));
    }
    assertions.extend(score_agent_expectations(
        &case.expected,
        &trace.rows,
        &stdout,
    ));
    EvalCaseReport::new(
        case.id.clone(),
        Some(case.task.clone()),
        assertions,
        Some(AgentArtifacts {
            stdout: case_dir.join("stdout.log").display().to_string(),
            stderr: case_dir.join("stderr.log").display().to_string(),
            tool_trace: trace_path.display().to_string(),
        }),
    )
}

fn score_agent_expectations(
    expected: &AgentExpected,
    trace: &[JsonValue],
    stdout: &str,
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
    if let Some(final_answer) = expected.final_answer.as_ref() {
        assertions.extend(score_final_answer(final_answer, stdout));
    }
    assertions
}

fn order_assertion(trace: &[JsonValue], order: &AgentOrder) -> AssertionResult {
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

fn score_final_answer(expected: &FinalAnswerExpected, stdout: &str) -> Vec<AssertionResult> {
    let haystack = stdout.to_lowercase();
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
                json!({"stdout": truncate(stdout, 4000)}),
            ));
        }
    }
    for needle in &expected.must_not_contain {
        if haystack.contains(&needle.to_lowercase()) {
            assertions.push(AssertionResult::fail(
                format!("final_answer_excludes:{needle}"),
                "final answer contained forbidden text",
                json!({"stdout": truncate(stdout, 4000)}),
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

fn first_tool_index(trace: &[JsonValue], tool: &str) -> Option<usize> {
    trace
        .iter()
        .position(|row| row.get("tool").and_then(JsonValue::as_str) == Some(tool))
}

fn observed_tools(trace: &[JsonValue]) -> Vec<String> {
    trace
        .iter()
        .filter_map(|row| row.get("tool").and_then(JsonValue::as_str))
        .map(ToString::to_string)
        .collect()
}

fn trace_selected_entity(trace: &[JsonValue], entity: &str) -> bool {
    trace.iter().any(|row| {
        row.get("selected_unique_ids")
            .and_then(JsonValue::as_array)
            .is_some_and(|ids| ids.iter().any(|id| id.as_str() == Some(entity)))
    })
}

fn selected_entities(trace: &[JsonValue]) -> Vec<String> {
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

struct ToolTraceRead {
    rows: Vec<JsonValue>,
    errors: Vec<String>,
    missing: bool,
}

fn reset_trace_file(path: &Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, "")
}

fn read_tool_trace(path: &Path) -> ToolTraceRead {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return ToolTraceRead {
                rows: Vec::new(),
                errors: Vec::new(),
                missing: true,
            };
        }
        Err(error) => {
            return ToolTraceRead {
                rows: Vec::new(),
                errors: vec![format!(
                    "failed to read tool trace '{}': {error}",
                    path.display()
                )],
                missing: false,
            };
        }
    };

    let mut rows = Vec::new();
    let mut errors = Vec::new();
    for (index, line) in raw.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<JsonValue>(line) {
            Ok(row) => rows.push(row),
            Err(error) => errors.push(format!(
                "failed to parse tool trace line {} in '{}': {error}",
                index + 1,
                path.display()
            )),
        }
    }
    ToolTraceRead {
        rows,
        errors,
        missing: false,
    }
}

fn rank_assertion(
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

fn contains_rank_assertion(
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

fn search_columns_assertion(
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

fn fields_assertion(name: &str, response: &JsonValue, fields: &[String]) -> AssertionResult {
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

fn metadata_score_assertion(response: &JsonValue, threshold: f64) -> AssertionResult {
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

fn recipe_queries_assertion(response: &JsonValue, min_queries: usize) -> AssertionResult {
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

fn contains_string_assertion(name: &str, response: &JsonValue, expected: &str) -> AssertionResult {
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

fn data_rows(response: &JsonValue) -> &[JsonValue] {
    response
        .get("data")
        .and_then(JsonValue::as_array)
        .map_or(&[], Vec::as_slice)
}

fn string_field_equals(row: &JsonValue, field: &str, expected: &str) -> bool {
    row.get(field)
        .and_then(JsonValue::as_str)
        .is_some_and(|value| value.eq_ignore_ascii_case(expected))
}

fn json_contains_string(value: &JsonValue, expected: &str) -> bool {
    let expected = expected.to_lowercase();
    json_contains_string_lower(value, &expected)
}

fn json_contains_string_lower(value: &JsonValue, expected: &str) -> bool {
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

fn json_has_field_path(value: &JsonValue, field_path: &str) -> bool {
    let mut current = value;
    for part in field_path.split('.') {
        let Some(next) = current.get(part) else {
            return false;
        };
        current = next;
    }
    !current.is_null()
}

fn find_score(value: &JsonValue) -> Option<f64> {
    for key in ["overall_score", "score", "metadata_score"] {
        if let Some(score) = find_number_by_key(value, key) {
            return Some(score);
        }
    }
    None
}

fn find_number_by_key(value: &JsonValue, key: &str) -> Option<f64> {
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

fn count_recipe_queries(value: &JsonValue) -> usize {
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

fn top_evidence(rows: &[JsonValue], field: &str) -> JsonValue {
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

fn effective_limit(max_rank: Option<usize>, default_top_k: usize) -> usize {
    max_rank.unwrap_or(default_top_k).max(1)
}

fn effective_persona(
    assertion_persona: Option<&String>,
    case: &EvalCase,
    suite: &EvalSuite,
) -> Option<String> {
    assertion_persona
        .cloned()
        .or_else(|| case.persona.clone())
        .or_else(|| suite.defaults.persona.clone())
}

fn build_report(
    suite: &EvalSuite,
    mode: &'static str,
    output_dir: String,
    fail_under: f64,
    cases: Vec<EvalCaseReport>,
) -> EvalReport {
    let pass_count = cases.iter().map(|case| case.pass_count).sum();
    let fail_count = cases.iter().map(|case| case.fail_count).sum();
    let error_count = cases.iter().map(|case| case.error_count).sum();
    let assertion_count = pass_count + fail_count + error_count;
    let pass_rate = if assertion_count == 0 {
        0.0
    } else {
        ratio(pass_count, assertion_count)
    };
    let gate_status = if pass_rate >= fail_under {
        "pass"
    } else {
        "fail"
    };
    EvalReport {
        suite_name: suite.name.clone().unwrap_or_else(|| "unnamed".to_string()),
        version: suite.version,
        mode,
        output_dir,
        assertion_count,
        pass_count,
        fail_count,
        error_count,
        pass_rate,
        fail_under,
        gate_status,
        cases,
    }
}

fn write_report_artifacts(
    output_dir: &Path,
    report: &EvalReport,
    suite_path: &str,
) -> DispatchResult {
    fs::create_dir_all(output_dir).map_err(|error| server_error(error.to_string()))?;
    let report_json =
        serde_json::to_string_pretty(report).map_err(|error| server_error(error.to_string()))?;
    fs::write(output_dir.join("results.json"), report_json)
        .map_err(|error| server_error(error.to_string()))?;
    fs::write(output_dir.join("results.tsv"), render_tsv(report))
        .map_err(|error| server_error(error.to_string()))?;
    fs::write(output_dir.join("report.md"), render_markdown(report))
        .map_err(|error| server_error(error.to_string()))?;
    if let Err(error) = fs::copy(suite_path, output_dir.join("suite.yml")) {
        tracing::warn!(error = %error, suite_path, "failed to copy eval suite");
    }
    Ok(())
}

fn render_tsv(report: &EvalReport) -> String {
    let mut out = String::from("case_id\tassertion\tstatus\tmessage\n");
    for case in &report.cases {
        for assertion in &case.assertions {
            let _ = writeln!(
                out,
                "{}\t{}\t{}\t{}",
                tsv_escape(&case.id),
                tsv_escape(&assertion.name),
                assertion.status,
                tsv_escape(&assertion.message)
            );
        }
    }
    out
}

fn render_markdown(report: &EvalReport) -> String {
    let mut out = format!(
        "# Nova Eval Report\n\n- Suite: `{}`\n- Mode: `{}`\n- Gate: `{}`\n- Pass rate: `{:.1}%`\n- Assertions: {} pass, {} fail, {} error\n\n",
        report.suite_name,
        report.mode,
        report.gate_status,
        report.pass_rate * 100.0,
        report.pass_count,
        report.fail_count,
        report.error_count
    );
    for case in &report.cases {
        let _ = writeln!(out, "## {}\n", case.id);
        for assertion in &case.assertions {
            let _ = writeln!(
                out,
                "- `{}` `{}`: {}",
                assertion.status, assertion.name, assertion.message
            );
        }
        out.push('\n');
    }
    out
}

fn tsv_escape(value: &str) -> String {
    value.replace(['\t', '\n', '\r'], " ")
}

fn finish_report(
    command: &str,
    report: &EvalReport,
    json_output: bool,
    elapsed_ms: u128,
) -> DispatchResult {
    let gate_failed = report.gate_status != "pass";
    if json_output {
        let envelope = CliEnvelope::success(command, &report, elapsed_ms);
        let out = serde_json::to_string_pretty(&envelope)
            .map_err(|error| server_error(error.to_string()))?;
        println!("{out}");
        if gate_failed {
            return Err(DispatchError {
                error: DbtNovaError::ServerError(format!(
                    "eval gate failed: pass rate {:.3} below threshold {:.3}",
                    report.pass_rate, report.fail_under
                )),
                rendered: true,
            });
        }
        return Ok(());
    }

    println!(
        "Nova eval {}: {} ({}/{} passed, {:.1}%). Artifacts: {}",
        report.mode,
        report.gate_status,
        report.pass_count,
        report.assertion_count,
        report.pass_rate * 100.0,
        report.output_dir
    );
    if gate_failed {
        return Err(DbtNovaError::ServerError(format!(
            "eval gate failed: pass rate {:.3} below threshold {:.3}",
            report.pass_rate, report.fail_under
        ))
        .into());
    }
    Ok(())
}

impl EvalCaseReport {
    fn new(
        id: String,
        question: Option<String>,
        assertions: Vec<AssertionResult>,
        artifacts: Option<AgentArtifacts>,
    ) -> Self {
        let pass_count = assertions
            .iter()
            .filter(|assertion| assertion.status == "pass")
            .count();
        let fail_count = assertions
            .iter()
            .filter(|assertion| assertion.status == "fail")
            .count();
        let error_count = assertions
            .iter()
            .filter(|assertion| assertion.status == "error")
            .count();
        Self {
            id,
            question,
            pass_count,
            fail_count,
            error_count,
            assertions,
            artifacts,
        }
    }
}

impl AssertionResult {
    fn pass(name: impl Into<String>, message: impl Into<String>, evidence: JsonValue) -> Self {
        Self {
            name: name.into(),
            status: "pass",
            message: message.into(),
            evidence,
        }
    }

    fn fail(name: impl Into<String>, message: impl Into<String>, evidence: JsonValue) -> Self {
        Self {
            name: name.into(),
            status: "fail",
            message: message.into(),
            evidence,
        }
    }

    fn error(name: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            status: "error",
            message: message.into(),
            evidence: JsonValue::Null,
        }
    }
}

impl AgentExpected {
    fn requires_trace(&self) -> bool {
        !self.must_call.is_empty()
            || !self.must_not_call.is_empty()
            || !self.ordered.is_empty()
            || !self.selected_entities.is_empty()
    }
}

fn load_suite(path: &str) -> crate::error::Result<EvalSuite> {
    let raw = fs::read_to_string(path).map_err(|error| {
        DbtNovaError::InvalidParams(format!("failed to read eval suite '{path}': {error}"))
    })?;
    let suite: EvalSuite = if Path::new(path)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
    {
        serde_json::from_str(&raw).map_err(|error| {
            DbtNovaError::InvalidParams(format!("failed to parse eval suite JSON: {error}"))
        })?
    } else {
        serde_yaml::from_str(&raw).map_err(|error| {
            DbtNovaError::InvalidParams(format!("failed to parse eval suite YAML: {error}"))
        })?
    };
    validate_suite(&suite)?;
    Ok(suite)
}

fn validate_suite(suite: &EvalSuite) -> crate::error::Result<()> {
    if suite.version == 0 {
        return Err(DbtNovaError::InvalidParams(
            "eval suite version must be greater than zero".to_string(),
        ));
    }
    validate_case_ids(suite.cases.iter().map(|case| case.id.as_str()), "cases")?;
    validate_case_ids(
        suite.agent_cases.iter().map(|case| case.id.as_str()),
        "agent_cases",
    )?;
    for case in &suite.cases {
        if case.assertions.is_empty() {
            return Err(DbtNovaError::InvalidParams(format!(
                "eval case '{}' must include at least one assertion",
                case.id
            )));
        }
    }
    for case in &suite.agent_cases {
        if case.task.trim().is_empty() {
            return Err(DbtNovaError::InvalidParams(format!(
                "agent case '{}' must include a non-empty task",
                case.id
            )));
        }
    }
    Ok(())
}

fn validate_case_ids<'a>(
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

fn resolve_output_dir(explicit: Option<&str>, suite: &EvalSuite, mode: &str) -> PathBuf {
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

fn validate_fail_under(value: Option<f64>) -> crate::error::Result<()> {
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

fn agent_prompt(case: &AgentCase) -> String {
    format!(
        "You are running a dbt-nova eval case.\n\nTask:\n{}\n\nUse Nova tools when they are needed. Do not mutate repository files. Finish with a concise answer that cites the evidence you used.",
        case.task
    )
}

fn starter_suite(persona: &str) -> String {
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

fn empty_object() -> JsonValue {
    json!({})
}

fn default_top_k() -> usize {
    DEFAULT_TOP_K
}

fn safe_path_segment(value: &str) -> String {
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

fn truncate(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

fn timestamp_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

fn elapsed_ms_to_u64(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn ratio(numerator: usize, denominator: usize) -> f64 {
    let numerator = u32::try_from(numerator).unwrap_or(u32::MAX);
    let denominator = u32::try_from(denominator).unwrap_or(u32::MAX);
    f64::from(numerator) / f64::from(denominator)
}

fn server_error(message: String) -> DbtNovaError {
    DbtNovaError::ServerError(message)
}

#[cfg(test)]
mod tests;
