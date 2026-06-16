#![allow(clippy::too_many_lines)]

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write as IoWrite};
use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{Value as JsonValue, json};

use crate::cli::args::{
    EvalAgentRunArgs, EvalHistoryArgs, EvalInitArgs, EvalRunArgs, EvalValidateArgs,
    ManifestLoadArgs,
};
use crate::cli::manifest::{build_manifest_load_config, execute_manifest_load};
use crate::cli::output::{CliEnvelope, error_envelope};
use crate::cli::{DispatchError, DispatchResult};
use crate::error::DbtNovaError;
use crate::manifest::ManifestSearch;

const DEFAULT_TOP_K: usize = 5;
const DEFAULT_FAIL_UNDER: f64 = 1.0;
const MAX_SAFE_PATH_SEGMENT_CHARS: usize = 120;
const DEFAULT_TELEMETRY_DIR: &str = ".nova/eval-runs/telemetry";

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
    ContextFieldEquals {
        id_or_name: String,
        field: String,
        expected: JsonValue,
    },
    ContextContains {
        id_or_name: String,
        expected: String,
        #[serde(default)]
        field: Option<String>,
    },
    MetadataScoreMin {
        #[serde(default)]
        id_or_name: Option<String>,
        threshold: f64,
        #[serde(default)]
        persona: Option<String>,
    },
    MetadataScoreMax {
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
    ToolResponseBudget {
        tool: String,
        #[serde(default = "empty_object")]
        params: JsonValue,
        max_response_bytes: usize,
        #[serde(default)]
        must_contain_paths: Vec<String>,
        #[serde(default)]
        must_not_contain_paths: Vec<String>,
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
    selected_entity_ranks: Vec<AgentEntityRank>,
    #[serde(default)]
    called_with: Vec<AgentCalledWith>,
    #[serde(default)]
    final_answer: Option<FinalAnswerExpected>,
    #[serde(default)]
    max_tool_calls: Option<usize>,
    #[serde(default)]
    max_distinct_tools: Option<usize>,
    #[serde(default)]
    max_total_response_bytes: Option<usize>,
    #[serde(default)]
    max_response_bytes_by_tool: BTreeMap<String, usize>,
}

#[derive(Debug, Deserialize)]
struct AgentOrder {
    before: String,
    #[serde(default)]
    must_have_called: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct AgentEntityRank {
    unique_id: String,
    #[serde(default)]
    tool: Option<String>,
    #[serde(default)]
    max_rank: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct AgentCalledWith {
    tool: String,
    #[serde(default)]
    params: BTreeMap<String, JsonValue>,
    #[serde(default)]
    contains: BTreeMap<String, String>,
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
    #[serde(skip)]
    telemetry: Option<EvalCaseTelemetry>,
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

#[derive(Debug, Clone)]
struct EvalCaseTelemetry {
    tool_call_count: usize,
    distinct_tool_count: usize,
    total_response_bytes: Option<u64>,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    total_tokens: Option<u64>,
}

#[derive(Debug, Clone, Copy)]
struct AgentTelemetryContext<'a> {
    provider: &'a str,
    provider_command_preset: &'a str,
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

/// Validates an eval suite without loading a manifest or running a provider.
///
/// # Errors
/// Returns an error when the suite cannot be read or fails schema validation.
pub fn run_validate_command(args: &EvalValidateArgs) -> DispatchResult {
    let started = Instant::now();
    let suite = load_suite(&args.suite).map_err(|error| {
        if args.json {
            let envelope = error_envelope("eval validate", &error, started.elapsed().as_millis());
            if let Ok(out) = serde_json::to_string_pretty(&envelope) {
                println!("{out}");
                return DispatchError {
                    error,
                    rendered: true,
                };
            }
        }
        DispatchError {
            error,
            rendered: false,
        }
    })?;
    let payload = json!({
        "valid": true,
        "path": args.suite,
        "suite_name": suite.name.as_deref().unwrap_or("suite"),
        "version": suite.version,
        "bridge_case_count": suite.cases.len(),
        "agent_case_count": suite.agent_cases.len(),
    });
    if args.json {
        let envelope =
            CliEnvelope::success("eval validate", payload, started.elapsed().as_millis());
        let out = serde_json::to_string_pretty(&envelope)
            .map_err(|error| server_error(error.to_string()))?;
        println!("{out}");
    } else {
        println!("eval suite is valid");
        println!("  suite: {}", suite.name.as_deref().unwrap_or("suite"));
        println!("  version: {}", suite.version);
        println!("  bridge cases: {}", suite.cases.len());
        println!("  agent cases: {}", suite.agent_cases.len());
    }
    Ok(())
}

/// Prints filtered JSONL eval telemetry history.
///
/// # Errors
/// Returns an error when `--since` is invalid or existing telemetry cannot be read.
pub fn run_history_command(args: &EvalHistoryArgs) -> DispatchResult {
    let since_boundary = validate_since_date(&args.since)?;
    let path = telemetry_path_for_suite(&args.suite);
    if !path.exists() {
        return Ok(());
    }
    let file = fs::File::open(&path).map_err(|error| server_error(error.to_string()))?;
    let reader = BufReader::new(file);
    for (index, line) in reader.lines().enumerate() {
        let line = line.map_err(|error| server_error(error.to_string()))?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let row = serde_json::from_str::<JsonValue>(trimmed).map_err(|error| {
            DbtNovaError::InvalidParams(format!(
                "failed to parse telemetry line {} in '{}': {error}",
                index + 1,
                path.display()
            ))
        })?;
        if telemetry_row_matches_since(&row, &since_boundary) {
            let out =
                serde_json::to_string(&row).map_err(|error| server_error(error.to_string()))?;
            println!("{out}");
        }
    }
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
    validate_telemetry_retention(args.telemetry_retention)?;
    if suite.cases.is_empty() {
        return Err(
            DbtNovaError::InvalidParams("eval suite contains no bridge cases".to_string()).into(),
        );
    }
    let selected_cases = selected_bridge_cases(&suite.cases, &args.case_ids)?;
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

    let mut cases = Vec::with_capacity(selected_cases.len());
    for case in selected_cases {
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
    let elapsed = started.elapsed();
    let elapsed_ms = elapsed.as_millis();
    if args.telemetry {
        write_eval_telemetry(
            &report,
            &args.suite,
            Some(&load_result.search.manifest_hash),
            elapsed_ms_to_u64(elapsed),
            args.telemetry_retention,
            None,
        )?;
    }
    finish_report("eval run", &report, args.json, elapsed_ms)
}

/// Runs agent-provider evals and scores the tool-use trace.
///
/// # Errors
/// Returns an error when the suite is invalid, a provider command cannot be run, or the score gate fails.
pub async fn run_agent_eval_command(args: &EvalAgentRunArgs) -> DispatchResult {
    let started = Instant::now();
    let suite = load_suite(&args.suite)?;
    validate_fail_under(args.fail_under)?;
    validate_telemetry_retention(args.telemetry_retention)?;
    if suite.agent_cases.is_empty() {
        return Err(
            DbtNovaError::InvalidParams("eval suite contains no agent_cases".to_string()).into(),
        );
    }
    let selected_cases = selected_agent_cases(&suite.agent_cases, &args.case_ids)?;
    let output_dir = resolve_output_dir(args.output_dir.as_deref(), &suite, "agent");
    fs::create_dir_all(output_dir.join("tool-calls"))
        .map_err(|error| server_error(error.to_string()))?;

    let mut cases = Vec::with_capacity(selected_cases.len());
    for case in selected_cases {
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
    let elapsed = started.elapsed();
    let elapsed_ms = elapsed.as_millis();
    if args.telemetry {
        write_eval_telemetry(
            &report,
            &args.suite,
            None,
            elapsed_ms_to_u64(elapsed),
            args.telemetry_retention,
            Some(AgentTelemetryContext {
                provider: &args.provider,
                provider_command_preset: agent_provider_command_preset(args),
            }),
        )?;
    }
    finish_report("eval agent run", &report, args.json, elapsed_ms)
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
        EvalAssertion::ContextFieldEquals {
            id_or_name,
            field,
            expected,
        } => {
            let params = json!({"id_or_name": id_or_name});
            match call_tool(search, "get_context", params).await {
                Ok(response) => context_field_equals_assertion(&response, field, expected),
                Err(error) => AssertionResult::error("context_field_equals", error.to_string()),
            }
        }
        EvalAssertion::ContextContains {
            id_or_name,
            expected,
            field,
        } => {
            let params = json!({"id_or_name": id_or_name});
            match call_tool(search, "get_context", params).await {
                Ok(response) => context_contains_assertion(&response, field.as_deref(), expected),
                Err(error) => AssertionResult::error("context_contains", error.to_string()),
            }
        }
        EvalAssertion::MetadataScoreMin {
            id_or_name,
            threshold,
            persona,
        }
        | EvalAssertion::MetadataScoreMax {
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
                Ok(response) => match assertion {
                    EvalAssertion::MetadataScoreMin { .. } => {
                        metadata_score_min_assertion(&response, *threshold)
                    }
                    EvalAssertion::MetadataScoreMax { .. } => {
                        metadata_score_max_assertion(&response, *threshold)
                    }
                    _ => unreachable!("matched metadata score assertion"),
                },
                Err(error) => AssertionResult::error(
                    match assertion {
                        EvalAssertion::MetadataScoreMin { .. } => "metadata_score_min",
                        EvalAssertion::MetadataScoreMax { .. } => "metadata_score_max",
                        _ => unreachable!("matched metadata score assertion"),
                    },
                    error.to_string(),
                ),
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
                Ok(response) => recipe_rank_assertion(&response, expected_recipe_id, *max_rank),
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
                Ok(response) => tool_success_assertion(tool, &response),
                Err(error) => {
                    AssertionResult::error(format!("tool_success:{tool}"), error.to_string())
                }
            }
        }
        EvalAssertion::ToolResponseBudget {
            tool,
            params,
            max_response_bytes,
            must_contain_paths,
            must_not_contain_paths,
        } => match call_tool(search, tool, params.clone()).await {
            Ok(response) => tool_response_budget_assertion(
                tool,
                &response,
                *max_response_bytes,
                must_contain_paths,
                must_not_contain_paths,
            ),
            Err(error) => {
                AssertionResult::error(format!("tool_response_budget:{tool}"), error.to_string())
            }
        },
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
    normalize_tool_trace_indices(&mut trace.rows);
    if trace.rows.is_empty() && case.expected.requires_trace() {
        assertions.push(AssertionResult::error(
            "tool_trace_missing",
            "tool trace rows were not observed; verify the provider launches a local dbt-nova process, emits MCP tool events, or writes trace rows",
        ));
    }
    for error in &trace.errors {
        assertions.push(AssertionResult::error("tool_trace_parse", error.clone()));
    }
    let final_answer_text =
        provider::read_provider_final_answer(&stdout).unwrap_or_else(|| stdout.clone());
    assertions.extend(score_agent_expectations(
        &case.expected,
        &trace.rows,
        &final_answer_text,
    ));
    let telemetry = eval_case_telemetry_from_trace(&trace.rows);
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
    .with_telemetry(telemetry)
}

fn score_agent_expectations(
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

fn max_tool_calls_assertion(trace: &[JsonValue], max_tool_calls: usize) -> AssertionResult {
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

fn max_distinct_tools_assertion(trace: &[JsonValue], max_distinct_tools: usize) -> AssertionResult {
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

fn max_total_response_bytes_assertion(
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

fn max_response_bytes_by_tool_assertion(
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

fn total_response_bytes(trace: &[JsonValue]) -> usize {
    trace.iter().filter_map(response_bytes_from_trace_row).sum()
}

fn trace_rows_missing_response_bytes(trace: &[JsonValue]) -> usize {
    trace
        .iter()
        .filter(|row| response_bytes_from_trace_row(row).is_none())
        .count()
}

fn response_bytes_from_trace_row(row: &JsonValue) -> Option<usize> {
    row.get("response_bytes")
        .and_then(JsonValue::as_u64)
        .and_then(|value| usize::try_from(value).ok())
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

fn score_final_answer(
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
                json!({"final_answer": truncate(final_answer_text, 4000)}),
            ));
        }
    }
    for needle in &expected.must_not_contain {
        if haystack.contains(&needle.to_lowercase()) {
            assertions.push(AssertionResult::fail(
                format!("final_answer_excludes:{needle}"),
                "final answer contained forbidden text",
                json!({"final_answer": truncate(final_answer_text, 4000)}),
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

fn selected_entity_rank_assertion(
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

fn trace_entity_rank(trace: &[JsonValue], entity: &str, tool: Option<&str>) -> Option<usize> {
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

fn top_unique_ids(trace: &[JsonValue], tool: Option<&str>) -> Vec<String> {
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

fn called_with_assertion(trace: &[JsonValue], expected: &AgentCalledWith) -> AssertionResult {
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

fn called_with_matches(row: &JsonValue, expected: &AgentCalledWith) -> bool {
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

fn param_value_matches(actual: &JsonValue, expected: &JsonValue) -> bool {
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

fn observed_params_for_tool(trace: &[JsonValue], tool: &str) -> JsonValue {
    JsonValue::Array(
        trace
            .iter()
            .filter(|row| row.get("tool").and_then(JsonValue::as_str) == Some(tool))
            .filter_map(|row| row.get("params_summary").cloned())
            .collect(),
    )
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
    normalize_tool_trace_indices(&mut rows);
    ToolTraceRead {
        rows,
        errors,
        missing: false,
    }
}

fn normalize_tool_trace_indices(rows: &mut [JsonValue]) {
    for (index, row) in rows.iter_mut().enumerate() {
        if let Some(obj) = row.as_object_mut() {
            obj.insert("tool_call_index".to_string(), JsonValue::from(index as u64));
        }
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

fn recipe_rank_assertion(
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

fn context_field_equals_assertion(
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

fn context_contains_assertion(
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

fn metadata_score_min_assertion(response: &JsonValue, threshold: f64) -> AssertionResult {
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

fn metadata_score_max_assertion(response: &JsonValue, threshold: f64) -> AssertionResult {
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

fn tool_success_assertion(tool: &str, response: &JsonValue) -> AssertionResult {
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

fn tool_response_budget_assertion(
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

fn tool_failure_evidence(response: &JsonValue) -> JsonValue {
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

fn safe_evidence_scalar(value: &JsonValue) -> Option<JsonValue> {
    match value {
        JsonValue::Null | JsonValue::Bool(_) | JsonValue::Number(_) => Some(value.clone()),
        JsonValue::String(value) => Some(JsonValue::String(truncate(value, 1000))),
        JsonValue::Array(_) | JsonValue::Object(_) => None,
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
    json_value_at_path(value, field_path).is_some_and(|value| !value.is_null())
}

fn json_value_at_path<'a>(value: &'a JsonValue, field_path: &str) -> Option<&'a JsonValue> {
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

fn recipe_top_evidence(rows: &[JsonValue]) -> JsonValue {
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

fn write_eval_telemetry(
    report: &EvalReport,
    suite_path: &str,
    manifest_hash: Option<&str>,
    duration_ms: u64,
    retention: Option<usize>,
    agent: Option<AgentTelemetryContext<'_>>,
) -> DispatchResult {
    let telemetry_path = telemetry_path_for_suite(&report.suite_name);
    if let Some(parent) = telemetry_path.parent() {
        fs::create_dir_all(parent).map_err(|error| server_error(error.to_string()))?;
    }
    let timestamp_ms = timestamp_millis();
    let timestamp = format_utc_timestamp_millis(timestamp_ms);
    let git_sha = current_git_sha();
    let manifest_hash = manifest_hash
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&telemetry_path)
        .map_err(|error| server_error(error.to_string()))?;
    for case in &report.cases {
        for assertion in &case.assertions {
            let mut row = serde_json::Map::new();
            row.insert("timestamp".to_string(), json!(&timestamp));
            row.insert("timestamp_ms".to_string(), json!(timestamp_ms));
            row.insert("suite_name".to_string(), json!(&report.suite_name));
            row.insert("suite_path".to_string(), json!(suite_path));
            row.insert("mode".to_string(), json!(report.mode));
            row.insert("case_id".to_string(), json!(&case.id));
            row.insert("assertion_name".to_string(), json!(&assertion.name));
            row.insert(
                "assertion_type".to_string(),
                json!(assertion_type(&assertion.name)),
            );
            row.insert("status".to_string(), json!(assertion.status));
            row.insert(
                "grade_mode".to_string(),
                json!(telemetry_grade_mode(report.mode)),
            );
            row.insert("duration_ms".to_string(), json!(duration_ms));
            row.insert("output_dir".to_string(), json!(&report.output_dir));
            if let Some(manifest_hash) = manifest_hash.as_ref() {
                row.insert("manifest_hash".to_string(), json!(manifest_hash));
            }
            if let Some(git_sha) = git_sha.as_ref() {
                row.insert("git_sha".to_string(), json!(git_sha));
            }
            if let Some(agent) = agent {
                row.insert("provider".to_string(), json!(agent.provider));
                row.insert(
                    "provider_command_preset".to_string(),
                    json!(agent.provider_command_preset),
                );
                if let Some(telemetry) = case.telemetry.as_ref() {
                    row.insert(
                        "tool_call_count".to_string(),
                        json!(telemetry.tool_call_count),
                    );
                    row.insert(
                        "distinct_tool_count".to_string(),
                        json!(telemetry.distinct_tool_count),
                    );
                    if let Some(value) = telemetry.total_response_bytes {
                        row.insert("total_response_bytes".to_string(), json!(value));
                    }
                    if let Some(value) = telemetry.input_tokens {
                        row.insert("input_tokens".to_string(), json!(value));
                    }
                    if let Some(value) = telemetry.output_tokens {
                        row.insert("output_tokens".to_string(), json!(value));
                    }
                    if let Some(value) = telemetry.total_tokens {
                        row.insert("total_tokens".to_string(), json!(value));
                    }
                }
            }
            let line = serde_json::to_string(&JsonValue::Object(row))
                .map_err(|error| server_error(error.to_string()))?;
            file.write_all(line.as_bytes())
                .map_err(|error| server_error(error.to_string()))?;
            file.write_all(b"\n")
                .map_err(|error| server_error(error.to_string()))?;
        }
    }
    drop(file);

    if let Some(max_rows) = retention {
        apply_telemetry_retention(&telemetry_path, max_rows)?;
    }
    Ok(())
}

fn apply_telemetry_retention(path: &Path, max_rows: usize) -> DispatchResult {
    let raw = fs::read_to_string(path).map_err(|error| server_error(error.to_string()))?;
    let rows: Vec<&str> = raw.lines().filter(|line| !line.trim().is_empty()).collect();
    if rows.len() <= max_rows {
        return Ok(());
    }
    let keep_from = rows.len().saturating_sub(max_rows);
    let mut out = rows[keep_from..].join("\n");
    out.push('\n');
    fs::write(path, out).map_err(|error| server_error(error.to_string()))?;
    Ok(())
}

fn telemetry_path_for_suite(suite_name: &str) -> PathBuf {
    PathBuf::from(DEFAULT_TELEMETRY_DIR).join(format!(
        "{}-{:016x}.jsonl",
        safe_path_segment(suite_name),
        stable_telemetry_hash(suite_name)
    ))
}

fn stable_telemetry_hash(value: &str) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0001_0000_01b3);
    }
    hash
}

fn telemetry_row_matches_since(row: &JsonValue, since_boundary: &str) -> bool {
    row.get("timestamp")
        .and_then(JsonValue::as_str)
        .is_some_and(|timestamp| timestamp >= since_boundary)
}

fn validate_since_date(value: &str) -> crate::error::Result<String> {
    let valid_shape = value.len() == 10
        && value.as_bytes()[4] == b'-'
        && value.as_bytes()[7] == b'-'
        && value
            .chars()
            .enumerate()
            .all(|(index, ch)| matches!(index, 4 | 7) || ch.is_ascii_digit());
    if !valid_shape {
        return Err(DbtNovaError::InvalidParams(
            "--since must use YYYY-MM-DD".to_string(),
        ));
    }
    let year = value[0..4]
        .parse::<i32>()
        .map_err(|_| DbtNovaError::InvalidParams("--since must use YYYY-MM-DD".to_string()))?;
    let month = value[5..7]
        .parse::<u32>()
        .map_err(|_| DbtNovaError::InvalidParams("--since must use YYYY-MM-DD".to_string()))?;
    let day = value[8..10]
        .parse::<u32>()
        .map_err(|_| DbtNovaError::InvalidParams("--since must use YYYY-MM-DD".to_string()))?;
    if !(1..=12).contains(&month) || day == 0 || day > days_in_month(year, month) {
        return Err(DbtNovaError::InvalidParams(
            "--since must use a valid YYYY-MM-DD date".to_string(),
        ));
    }
    Ok(format!("{value}T00:00:00.000Z"))
}

fn validate_telemetry_retention(value: Option<usize>) -> crate::error::Result<()> {
    if value == Some(0) {
        return Err(DbtNovaError::InvalidParams(
            "--telemetry-retention must be greater than zero".to_string(),
        ));
    }
    Ok(())
}

fn telemetry_grade_mode(mode: &str) -> &'static str {
    match mode {
        "agent" => "provider_trace",
        _ => "deterministic",
    }
}

fn assertion_type(name: &str) -> &str {
    name.split_once(':').map_or(name, |(prefix, _)| prefix)
}

fn agent_provider_command_preset(args: &EvalAgentRunArgs) -> &str {
    if args.provider_command.is_some() || args.provider_args_json.is_some() {
        "custom"
    } else {
        args.provider.as_str()
    }
}

fn eval_case_telemetry_from_trace(trace: &[JsonValue]) -> EvalCaseTelemetry {
    let mut distinct_tools = BTreeSet::new();
    let mut response_bytes_seen = false;
    let mut total_response_bytes = 0_u64;
    for row in trace {
        if let Some(tool) = row.get("tool").and_then(JsonValue::as_str) {
            distinct_tools.insert(tool.to_string());
        }
        if let Some(bytes) = row.get("response_bytes").and_then(JsonValue::as_u64) {
            response_bytes_seen = true;
            total_response_bytes = total_response_bytes.saturating_add(bytes);
        }
    }
    EvalCaseTelemetry {
        tool_call_count: trace.len(),
        distinct_tool_count: distinct_tools.len(),
        total_response_bytes: response_bytes_seen.then_some(total_response_bytes),
        input_tokens: sum_first_available_u64(
            trace,
            &[
                &["input_tokens"],
                &["usage", "input_tokens"],
                &["usage", "prompt_tokens"],
            ],
        ),
        output_tokens: sum_first_available_u64(
            trace,
            &[
                &["output_tokens"],
                &["usage", "output_tokens"],
                &["usage", "completion_tokens"],
            ],
        ),
        total_tokens: sum_first_available_u64(
            trace,
            &[&["total_tokens"], &["usage", "total_tokens"]],
        ),
    }
}

fn sum_first_available_u64(trace: &[JsonValue], paths: &[&[&str]]) -> Option<u64> {
    let mut seen = false;
    let mut total = 0_u64;
    for row in trace {
        for path in paths {
            if let Some(value) = json_path_u64(row, path) {
                seen = true;
                total = total.saturating_add(value);
                break;
            }
        }
    }
    seen.then_some(total)
}

fn json_path_u64(value: &JsonValue, path: &[&str]) -> Option<u64> {
    let mut cursor = value;
    for part in path {
        cursor = cursor.get(*part)?;
    }
    cursor.as_u64()
}

fn current_git_sha() -> Option<String> {
    let output = StdCommand::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let sha = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!sha.is_empty()).then_some(sha)
}

fn timestamp_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        })
}

fn format_utc_timestamp_millis(timestamp_ms: u64) -> String {
    let secs = timestamp_ms / 1000;
    let millis = timestamp_ms % 1000;
    let days = i64::try_from(secs / 86_400).unwrap_or(i64::MAX);
    let seconds_of_day = secs % 86_400;
    let (year, month, day) = civil_from_days(days);
    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;
    let second = seconds_of_day % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z")
}

fn civil_from_days(days_since_epoch: i64) -> (i32, u32, u32) {
    let z = days_since_epoch.saturating_add(719_468);
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    if month <= 2 {
        year += 1;
    }
    (
        i32::try_from(year).unwrap_or(i32::MAX),
        u32::try_from(month).unwrap_or(12),
        u32::try_from(day).unwrap_or(31),
    )
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
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
            telemetry: None,
        }
    }

    fn with_telemetry(mut self, telemetry: EvalCaseTelemetry) -> Self {
        self.telemetry = Some(telemetry);
        self
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
            || !self.selected_entity_ranks.is_empty()
            || !self.called_with.is_empty()
            || self.max_tool_calls.is_some()
            || self.max_distinct_tools.is_some()
            || self.max_total_response_bytes.is_some()
            || !self.max_response_bytes_by_tool.is_empty()
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
        for assertion in &case.assertions {
            validate_assertion(assertion, &case.id)?;
        }
    }
    for case in &suite.agent_cases {
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

fn validate_assertion(assertion: &EvalAssertion, case_id: &str) -> crate::error::Result<()> {
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
        _ => {}
    }
    Ok(())
}

fn validate_agent_expected(expected: &AgentExpected, case_id: &str) -> crate::error::Result<()> {
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

fn is_safe_expected_param_value(value: &JsonValue) -> bool {
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

fn selected_bridge_cases<'a>(
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

fn selected_agent_cases<'a>(
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

fn normalized_case_filter(case_ids: &[String]) -> crate::error::Result<BTreeSet<&str>> {
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

fn validate_selected_cases<'a>(
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

fn agent_prompt(case: &AgentCase) -> String {
    format!(
        "You are running a dbt-nova analytics-agent eval.\n\nTask:\n{}\n\nRules:\n- Use Nova discovery and execution tools directly. Do not inspect repository files, source code, fixtures, or Rust params unless a Nova command fails and you cannot recover from the error message.\n- For KPI, metric, conversion, funnel, checkout, or business-concept questions, start with search_indicator using compact results: detail=\"compact\", group_mode=\"top\", include_support_signals=true, limit=3, persona=\"analyst\".\n- For rate, conversion, or funnel questions, include the requested metric names literally in the query and set indicator_types=[\"metric\"] unless you are explicitly searching for raw measures.\n- When a metric row returns an expression, copy that expression exactly into SQL; do not substitute similarly named measures or invent a numerator/denominator.\n- Use support_signals, grain dimensions, and relation_name from search_indicator to apply every requested filter before SQL. Do not aggregate across a grain dimension named in the task, such as country, market, channel, segment, or device.\n- Treat relation_name, grain, and expression fields returned by search_indicator as the execution contract. Do not run schema inspection SQL such as DESCRIBE or information_schema when those fields are present.\n- Use execute_sql only after Nova discovery identifies the canonical execution entity or relation. Use one aggregate SQL statement for current and comparison periods when possible. Skip get_entity when search_indicator already returns the relation, grain, measures, and metric expressions you need; otherwise use get_entity with id_or_name and detail=\"compact\".\n- Keep Nova calls to the minimum needed: usually search_indicator plus one execute_sql for calculations, and only search_indicator for model or metric lookup tasks. Avoid get_context, get_lineage, get_sql, and full-detail responses unless blocked.\n- If using the CLI, assume $DBT_NOVA_EVAL_BIN is set. For search/get calls, use --params-json. For execute_sql with quotes or newlines, write a JSON params file like {{\"statement\":\"select ...\",\"row_limit\":50}} and call $DBT_NOVA_EVAL_BIN tool call execute_sql --params-file <file> --json; do not inline multiline SQL in --params-json. Parameter reminders: get_entity uses id_or_name; execute_sql uses statement. Do not run echo, grep, read, or source inspection for normal tool usage.\n\nFinish with a concise answer that cites the Nova evidence, the SQL result, and the explicit filter values used.",
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
