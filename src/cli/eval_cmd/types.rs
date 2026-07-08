use super::*;

#[derive(Debug, Deserialize)]
pub(super) struct EvalSuite {
    pub(super) version: u32,
    #[serde(default)]
    pub(super) name: Option<String>,
    #[serde(default)]
    pub(super) purpose: Option<String>,
    #[serde(default)]
    pub(super) manifest_scope: Option<String>,
    #[serde(default)]
    pub(super) known_gaps: Vec<String>,
    #[serde(default)]
    pub(super) gate: Option<EvalGateConfig>,
    #[serde(flatten)]
    pub(super) date_anchor: DateAnchor,
    #[serde(default)]
    pub(super) defaults: EvalDefaults,
    #[serde(default)]
    pub(super) cases: Vec<EvalCase>,
    #[serde(default)]
    pub(super) agent_cases: Vec<AgentCase>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub(super) struct EvalGateConfig {
    pub(super) threshold: f64,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub(super) struct DateAnchor {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) snapshot_date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) date_range_start: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) date_range_end: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) date_field: Option<String>,
}

impl DateAnchor {
    pub(super) fn normalized(&self) -> Option<Self> {
        let anchor = Self {
            snapshot_date: normalized_string(self.snapshot_date.as_ref()),
            date_range_start: normalized_string(self.date_range_start.as_ref()),
            date_range_end: normalized_string(self.date_range_end.as_ref()),
            date_field: normalized_string(self.date_field.as_ref()),
        };
        (!anchor.is_empty()).then_some(anchor)
    }

    pub(super) fn is_empty(&self) -> bool {
        self.snapshot_date
            .as_deref()
            .is_none_or(|value| value.trim().is_empty())
            && self
                .date_range_start
                .as_deref()
                .is_none_or(|value| value.trim().is_empty())
            && self
                .date_range_end
                .as_deref()
                .is_none_or(|value| value.trim().is_empty())
            && self
                .date_field
                .as_deref()
                .is_none_or(|value| value.trim().is_empty())
    }

    pub(super) fn prompt_lines(&self) -> Vec<String> {
        self.markdown_lines()
            .into_iter()
            .map(|line| line.replace('`', ""))
            .collect()
    }

    pub(super) fn markdown_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        if let Some(value) = self.snapshot_date.as_deref() {
            lines.push(format!("snapshot_date: `{value}`"));
        }
        match (
            self.date_range_start.as_deref(),
            self.date_range_end.as_deref(),
        ) {
            (Some(start), Some(end)) => lines.push(format!("date_range: `{start}` to `{end}`")),
            (Some(start), None) => lines.push(format!("date_range_start: `{start}`")),
            (None, Some(end)) => lines.push(format!("date_range_end: `{end}`")),
            (None, None) => {}
        }
        if let Some(value) = self.date_field.as_deref() {
            lines.push(format!("date_field: `{value}`"));
        }
        lines
    }
}

#[derive(Debug, Default, Deserialize)]
pub(super) struct EvalDefaults {
    #[serde(default)]
    pub(super) persona: Option<String>,
    #[serde(default = "default_top_k")]
    pub(super) top_k: usize,
}

#[derive(Debug, Deserialize)]
pub(super) struct EvalCase {
    pub(super) id: String,
    #[serde(default)]
    pub(super) question: Option<String>,
    #[serde(default)]
    pub(super) persona: Option<String>,
    #[serde(flatten)]
    pub(super) date_anchor: DateAnchor,
    pub(super) assertions: Vec<EvalAssertion>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum EvalAssertion {
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
    ToolFieldEquals {
        tool: String,
        #[serde(default = "empty_object")]
        params: JsonValue,
        field: String,
        expected: JsonValue,
    },
    SqlStructure {
        actual_sql: String,
        expected_sql: String,
    },
}

#[derive(Debug, Deserialize)]
pub(super) struct AgentCase {
    pub(super) id: String,
    pub(super) task: String,
    #[serde(flatten)]
    pub(super) date_anchor: DateAnchor,
    #[serde(default)]
    pub(super) expected: AgentExpected,
}

#[derive(Debug, Default, Deserialize)]
pub(super) struct AgentExpected {
    #[serde(default)]
    pub(super) must_call: Vec<String>,
    #[serde(default)]
    pub(super) must_not_call: Vec<String>,
    #[serde(default)]
    pub(super) ordered: Vec<AgentOrder>,
    #[serde(default)]
    pub(super) selected_entities: Vec<String>,
    #[serde(default)]
    pub(super) selected_entity_ranks: Vec<AgentEntityRank>,
    #[serde(default)]
    pub(super) called_with: Vec<AgentCalledWith>,
    #[serde(default)]
    pub(super) sql_structures: Vec<AgentSqlStructureExpected>,
    #[serde(default)]
    pub(super) final_answer: Option<FinalAnswerExpected>,
    #[serde(default)]
    pub(super) max_tool_calls: Option<usize>,
    #[serde(default)]
    pub(super) max_distinct_tools: Option<usize>,
    #[serde(default)]
    pub(super) max_total_response_bytes: Option<usize>,
    #[serde(default)]
    pub(super) max_response_bytes_by_tool: BTreeMap<String, usize>,
}

#[derive(Debug, Deserialize)]
pub(super) struct AgentOrder {
    pub(super) before: String,
    #[serde(default)]
    pub(super) must_have_called: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct AgentEntityRank {
    pub(super) unique_id: String,
    #[serde(default)]
    pub(super) tool: Option<String>,
    #[serde(default)]
    pub(super) max_rank: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub(super) struct AgentCalledWith {
    pub(super) tool: String,
    #[serde(default)]
    pub(super) params: BTreeMap<String, JsonValue>,
    #[serde(default)]
    pub(super) contains: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct AgentSqlStructureExpected {
    #[serde(default = "default_sql_structure_tool")]
    pub(super) tool: String,
    pub(super) expected_sql: String,
}

#[derive(Debug, Default, Deserialize)]
pub(super) struct FinalAnswerExpected {
    #[serde(default)]
    pub(super) must_contain: Vec<String>,
    #[serde(default)]
    pub(super) must_not_contain: Vec<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct EvalReport {
    pub(super) suite_name: String,
    pub(super) version: u32,
    pub(super) mode: &'static str,
    pub(super) output_dir: String,
    pub(super) eval_card: EvalCard,
    pub(super) assertion_count: usize,
    pub(super) pass_count: usize,
    pub(super) fail_count: usize,
    pub(super) error_count: usize,
    pub(super) pass_rate: f64,
    pub(super) fail_under: f64,
    pub(super) gate_status: &'static str,
    pub(super) cases: Vec<EvalCaseReport>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct EvalCard {
    pub(super) schema_version: &'static str,
    pub(super) suite_name: String,
    pub(super) version: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) suite_path: Option<String>,
    pub(super) purpose: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) persona: Option<String>,
    pub(super) manifest_scope: EvalCardManifestScope,
    pub(super) mode: &'static str,
    pub(super) bridge_case_count: usize,
    pub(super) agent_case_count: usize,
    pub(super) run_case_count: usize,
    pub(super) output_dir: String,
    pub(super) assertion_count: usize,
    pub(super) pass_count: usize,
    pub(super) fail_count: usize,
    pub(super) error_count: usize,
    pub(super) pass_rate: f64,
    pub(super) fail_under: f64,
    pub(super) run_status: &'static str,
    pub(super) gate: EvalCardGate,
    pub(super) telemetry: EvalCardTelemetry,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) date_anchor: Option<DateAnchor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) provider: Option<EvalCardProvider>,
    pub(super) known_gaps: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct EvalCardManifestScope {
    pub(super) declared: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) manifest_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) manifest_source: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct EvalCardGate {
    pub(super) status: String,
    pub(super) source: &'static str,
    pub(super) configured: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) threshold: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) pass_rate: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) total_evals: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) failed_evals: Option<usize>,
    pub(super) message: String,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct EvalCardTelemetry {
    pub(super) status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) timestamp: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) run_id: Option<String>,
    pub(super) row_count: usize,
    pub(super) message: String,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct EvalCardProvider {
    pub(super) provider: String,
    pub(super) command_preset: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) model: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct EvalCaseReport {
    pub(super) id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) question: Option<String>,
    pub(super) pass_count: usize,
    pub(super) fail_count: usize,
    pub(super) error_count: usize,
    pub(super) assertions: Vec<AssertionResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) date_anchor: Option<DateAnchor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) artifacts: Option<AgentArtifacts>,
    #[serde(skip)]
    pub(super) telemetry: Option<EvalCaseTelemetry>,
}

#[derive(Debug, Serialize)]
pub(super) struct AssertionResult {
    pub(super) name: String,
    pub(super) status: &'static str,
    pub(super) message: String,
    #[serde(skip_serializing_if = "JsonValue::is_null")]
    pub(super) evidence: JsonValue,
}

#[derive(Debug, Serialize)]
pub(super) struct AgentArtifacts {
    pub(super) stdout: String,
    pub(super) stderr: String,
    pub(super) tool_trace: String,
}

#[derive(Debug, Clone)]
pub(super) struct EvalCaseTelemetry {
    pub(super) tool_call_count: usize,
    pub(super) distinct_tool_count: usize,
    pub(super) total_response_bytes: Option<u64>,
    pub(super) input_tokens: Option<u64>,
    pub(super) output_tokens: Option<u64>,
    pub(super) total_tokens: Option<u64>,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct AgentTelemetryContext<'a> {
    pub(super) provider: &'a str,
    pub(super) provider_command_preset: &'a str,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct EvalTelemetryRunContext<'a> {
    pub(super) suite_path: &'a str,
    pub(super) suite_hash: &'a str,
    pub(super) suite_case_count: usize,
    pub(super) manifest_hash: Option<&'a str>,
    pub(super) duration_ms: u64,
    pub(super) retention: Option<usize>,
    pub(super) agent: Option<AgentTelemetryContext<'a>>,
}

#[derive(Debug, Serialize)]
pub(super) struct EvalGateReport {
    pub(super) suite_name: String,
    pub(super) allowed: bool,
    pub(super) blocked: bool,
    pub(super) gate_configured: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) threshold: Option<f64>,
    pub(super) pass_rate: f64,
    pub(super) total_evals: usize,
    pub(super) failed_evals: usize,
    pub(super) failed_eval_ids: Vec<String>,
    pub(super) failed_case_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) telemetry_timestamp: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) output_dir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) suite_path: Option<String>,
    pub(super) message: String,
}

pub(super) enum GateConfigStatus {
    Configured {
        gate: EvalGateConfig,
        suite_hash: String,
    },
    Unconfigured,
    Unavailable(String),
}

#[derive(Debug, Serialize)]
pub(super) struct EvalMcpSafetyPolicy {
    pub(super) filesystem_root: String,
    pub(super) eval_run_enabled_env: &'static str,
    pub(super) eval_writes_enabled_env: &'static str,
    pub(super) agent_eval_enabled_env: &'static str,
    pub(super) custom_agent_provider_enabled_env: &'static str,
    pub(super) raw_provider_logs_enabled_env: &'static str,
    pub(super) provider_logs_redacted_by_default: bool,
    pub(super) local_paths_must_stay_under_filesystem_root: bool,
}

#[derive(Debug, Serialize)]
pub(super) struct EvalHistoryPayload {
    pub(super) suite_name: String,
    pub(super) since: String,
    pub(super) row_count: usize,
    pub(super) rows: Vec<JsonValue>,
    pub(super) safety_policy: EvalMcpSafetyPolicy,
}

#[derive(Debug, Serialize)]
pub(super) struct EvalComparisonReport {
    pub(super) schema_version: &'static str,
    pub(super) before: EvalComparisonRunSummary,
    pub(super) after: EvalComparisonRunSummary,
    pub(super) delta: EvalComparisonDelta,
    pub(super) markdown: String,
}

#[derive(Debug, Serialize)]
pub(super) struct EvalComparisonRunSummary {
    pub(super) results_path: String,
    pub(super) suite_name: String,
    pub(super) mode: String,
    pub(super) output_dir: String,
    pub(super) gate_status: String,
    pub(super) case_count: usize,
    pub(super) assertion_count: usize,
    pub(super) pass_count: usize,
    pub(super) fail_count: usize,
    pub(super) error_count: usize,
    pub(super) pass_rate: f64,
    pub(super) passing_cases: Vec<String>,
    pub(super) failing_cases: Vec<String>,
    pub(super) case_statuses: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "EvalComparisonMetrics::is_empty")]
    pub(super) metrics: EvalComparisonMetrics,
    pub(super) trace_case_count: usize,
    pub(super) warnings: Vec<String>,
}

#[derive(Debug, Default, Clone, Serialize)]
pub(super) struct EvalComparisonMetrics {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) tool_call_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) distinct_tool_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) total_response_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) duration_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) input_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) output_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) total_tokens: Option<u64>,
}

#[derive(Debug, Serialize)]
pub(super) struct EvalComparisonDelta {
    pub(super) suite_name_changed: bool,
    pub(super) mode_changed: bool,
    pub(super) pass_rate: f64,
    pub(super) pass_rate_percentage_points: f64,
    pub(super) case_count: i64,
    pub(super) assertion_count: i64,
    pub(super) pass_count: i64,
    pub(super) fail_count: i64,
    pub(super) error_count: i64,
    #[serde(skip_serializing_if = "EvalComparisonMetricDeltas::is_empty")]
    pub(super) metrics: EvalComparisonMetricDeltas,
    pub(super) newly_passing_cases: Vec<String>,
    pub(super) newly_failing_cases: Vec<String>,
    pub(super) unchanged_failing_cases: Vec<String>,
    pub(super) added_cases: Vec<String>,
    pub(super) removed_cases: Vec<String>,
    pub(super) changed_cases: Vec<EvalCaseStatusDelta>,
}

#[derive(Debug, Default, Serialize)]
pub(super) struct EvalComparisonMetricDeltas {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) tool_call_count: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) distinct_tool_count: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) total_response_bytes: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) duration_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) input_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) output_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) total_tokens: Option<i64>,
}

#[derive(Debug, Serialize)]
pub(super) struct EvalCaseStatusDelta {
    pub(super) id: String,
    pub(super) before: Option<String>,
    pub(super) after: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct EvalCardRunContext {
    pub(super) suite_path: Option<String>,
    pub(super) manifest_hash: Option<String>,
    pub(super) manifest_source: Option<String>,
    pub(super) telemetry_requested: bool,
    pub(super) provider: Option<EvalCardProvider>,
}
