#![allow(clippy::doc_markdown)]

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value as JsonValue;
use std::collections::HashMap;

pub const DEFAULT_LIMIT: usize = 50;
pub const DEFAULT_METADATA_SCORE_LIMIT: usize = 1000;
pub const DEFAULT_TEST_COVERAGE_COLUMNS_LIMIT: usize = 50;
pub const DEFAULT_CONFIDENCE: &str = "medium";
pub const DEFAULT_ENTITY_SCOPE: &str = "entity";

/// Default `true` for boolean parameters.
#[must_use]
pub fn default_true() -> bool {
    true
}

#[derive(Debug, Deserialize, JsonSchema, Clone, Copy, Default)]
#[serde(default)]
pub struct PaginationParams {
    /// Maximum results. Omit or pass 0 to use the configured default limit.
    pub limit: Option<usize>,
    /// Pagination offset
    pub offset: usize,
}

#[derive(Debug, Deserialize, JsonSchema, Clone, Copy)]
#[serde(default)]
pub struct ContextLimits {
    /// Maximum depth for lineage traversal (default: 1 for immediate only)
    pub lineage_depth: usize,
    /// Maximum upstream entities to include (default: 10)
    pub upstream_limit: usize,
    /// Maximum downstream entities to include (default: 10)
    pub downstream_limit: usize,
}

impl Default for ContextLimits {
    fn default() -> Self {
        Self {
            lineage_depth: 1,
            upstream_limit: 10,
            downstream_limit: 10,
        }
    }
}

/// Detail level for entity-returning responses.
#[derive(Debug, Deserialize, JsonSchema, Clone, Copy, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DetailLevel {
    Compact,
    #[default]
    Standard,
    Full,
}

/// Parent-group output for indicator search responses.
#[derive(Debug, Deserialize, JsonSchema, Clone, Copy, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ParentGroupMode {
    /// Do not include parent_groups.
    None,
    /// Include only the highest-signal parent group.
    Top,
    /// Include all parent groups up to configured or requested caps.
    #[default]
    All,
}

/// Parameters for the search tool.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Deserialize, JsonSchema, Default)]
pub struct SearchParams {
    /// Search query - matches names, descriptions, SQL code, file paths, column names.
    /// Supports boolean operators, phrases, field-specific queries, and prefix wildcards.
    #[serde(default)]
    pub query: String,
    /// Filter by resource types: model, source, macro, doc, test, seed, snapshot, analysis, exposure, metric, group
    #[serde(default)]
    pub resource_types: Vec<String>,
    /// Optional search persona: "analyst", "engineer", "governance"
    #[serde(default)]
    pub persona: Option<String>,
    /// Response detail level: compact, standard, or full. Omit to use the active result profile.
    #[serde(default)]
    pub detail: Option<DetailLevel>,
    #[serde(default, flatten)]
    pub pagination: PaginationParams,
    /// Minimum relevance score threshold (0.0+, default: no threshold). Higher values return only the most relevant results.
    #[serde(default)]
    pub min_score: Option<f32>,
    /// Enable fuzzy matching for typo tolerance (default: false)
    #[serde(default)]
    pub fuzzy: bool,
    /// Include highlight snippets in search results (default: false)
    #[serde(default)]
    pub include_highlights: bool,
    /// Include raw/compiled SQL in full detail responses (default: false)
    #[serde(default)]
    pub include_sql: bool,
    /// Include deterministic ranking/debug explanations in the response (default: false)
    #[serde(default)]
    pub explain: bool,
}

/// Parameters for the search_indicator tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchIndicatorParams {
    /// Search query - resolves Nova measures and metrics by name, synonym, field, description, or expression.
    #[serde(default)]
    pub query: String,
    /// Filter parent entities by resource types (for example: model, source).
    #[serde(default)]
    pub resource_types: Vec<String>,
    /// Filter indicator types. Supported values: metric, measure.
    #[serde(default)]
    pub indicator_types: Vec<String>,
    /// Optional search persona: "analyst", "engineer", "governance"
    #[serde(default)]
    pub persona: Option<String>,
    #[serde(default, flatten)]
    pub pagination: PaginationParams,
    /// Response detail level: compact, standard, or full. Omit to use the active result profile.
    #[serde(default)]
    pub detail: Option<DetailLevel>,
    /// Parent-group output: none, top, or all. Omit to use the active transport default.
    #[serde(default)]
    pub group_mode: Option<ParentGroupMode>,
    /// Optional cap for parent_groups when group_mode is top or all.
    #[serde(default)]
    pub max_parent_groups: Option<usize>,
    /// Include support signal details in indicator rows and parent groups.
    #[serde(default = "default_true")]
    pub include_support_signals: bool,
    /// Minimum relevance score threshold (0.0+, default: no threshold).
    #[serde(default)]
    pub min_score: Option<f32>,
    /// Include deterministic ranking/debug explanations in the response (default: false)
    #[serde(default)]
    pub explain: bool,
}

impl Default for SearchIndicatorParams {
    fn default() -> Self {
        Self {
            query: String::new(),
            resource_types: Vec::new(),
            indicator_types: Vec::new(),
            persona: None,
            pagination: PaginationParams::default(),
            detail: None,
            group_mode: None,
            max_parent_groups: None,
            include_support_signals: true,
            min_score: None,
            explain: false,
        }
    }
}

/// Parameters for the indicator_inventory tool.
#[derive(Debug, Deserialize, JsonSchema, Default)]
pub struct IndicatorInventoryParams {
    /// Filter parent entities by resource types (for example: model, source).
    #[serde(default)]
    pub resource_types: Vec<String>,
    /// Filter indicator types. Supported values: metric, measure.
    #[serde(default)]
    pub indicator_types: Vec<String>,
    /// Return only canonical indicators when true.
    #[serde(default)]
    pub canonical_only: bool,
    #[serde(default, flatten)]
    pub pagination: PaginationParams,
}

/// Parameters for the search_columns tool.
#[derive(Debug, Deserialize, JsonSchema, Default)]
pub struct SearchColumnsParams {
    /// Search query - resolves columns by name, synonym, description, role, semantic_type, or example_values.
    #[serde(default)]
    pub query: String,
    /// Filter parent entities by resource types (for example: model, source).
    #[serde(default)]
    pub resource_types: Vec<String>,
    /// Filter columns by role (for example: dimension, time, measure).
    #[serde(default)]
    pub roles: Vec<String>,
    /// Filter columns by semantic_type.
    #[serde(default)]
    pub semantic_types: Vec<String>,
    #[serde(default, flatten)]
    pub pagination: PaginationParams,
    /// Minimum relevance score threshold (0.0+, default: no threshold).
    #[serde(default)]
    pub min_score: Option<f32>,
}

/// Parameters for the column_inventory tool.
#[derive(Debug, Deserialize, JsonSchema, Default)]
pub struct ColumnInventoryParams {
    /// Filter parent entities by resource types (for example: model, source).
    #[serde(default)]
    pub resource_types: Vec<String>,
    /// Filter columns by role (for example: dimension, time, measure).
    #[serde(default)]
    pub roles: Vec<String>,
    /// Filter columns by semantic_type.
    #[serde(default)]
    pub semantic_types: Vec<String>,
    /// Return only columns with Nova annotations (`role`, `semantic_type`, `synonyms`, or `example_values`) when true.
    #[serde(default)]
    pub annotated_only: bool,
    #[serde(default, flatten)]
    pub pagination: PaginationParams,
}

/// Parameters for the compare_grains tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct CompareGrainsParams {
    /// First entity unique ID or name
    pub entity1: String,
    /// Optional resource_type to disambiguate entity1 when using name
    #[serde(default)]
    pub entity1_resource_type: Option<String>,
    /// Second entity unique ID or name
    pub entity2: String,
    /// Optional resource_type to disambiguate entity2 when using name
    #[serde(default)]
    pub entity2_resource_type: Option<String>,
}

/// Parameters for the find_entity_overlap tool.
#[derive(Debug, Deserialize, JsonSchema, Default)]
pub struct FindEntityOverlapParams {
    /// Optional focus entity unique ID or name. When set, only overlap pairs involving this entity are returned.
    #[serde(default)]
    pub id_or_name: Option<String>,
    /// Optional resource_type to disambiguate `id_or_name` when using name.
    #[serde(default)]
    pub resource_type: Option<String>,
    /// Filter candidate entities by resource types (for example: model, source).
    #[serde(default)]
    pub resource_types: Vec<String>,
    #[serde(default, flatten)]
    pub pagination: PaginationParams,
    /// Minimum overlap score threshold (0.0+, default: no threshold).
    #[serde(default)]
    pub min_score: Option<f32>,
}

/// Parameters for the modelling_consistency_report tool.
#[derive(Debug, Deserialize, JsonSchema, Default)]
pub struct ModellingConsistencyReportParams {
    /// Filter candidate entities by resource types (for example: model, source).
    #[serde(default)]
    pub resource_types: Vec<String>,
    /// Maximum results per report section.
    #[serde(default, flatten)]
    pub pagination: PaginationParams,
    /// Minimum overlap score threshold for the overlap section (0.0+, default: no threshold).
    #[serde(default)]
    pub min_score: Option<f32>,
}

/// Parameters for the get_entity tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetEntityParams {
    /// Unique ID (e.g., "model.package.name") or entity name
    #[serde(alias = "unique_id")]
    pub id_or_name: String,
    /// Optional: specify resource_type to disambiguate when using name
    pub resource_type: Option<String>,
    /// Response detail level: compact, standard, or full. Omit to use the active result profile.
    #[serde(default)]
    pub detail: Option<DetailLevel>,
}

/// Parameters for the list_entities tool.
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct ListEntitiesParams {
    /// Resource type: model, source, macro, doc, test, seed, snapshot, analysis, exposure, metric, group
    pub resource_type: String,
    /// Filter by package name
    pub package: Option<String>,
    /// Filter by tags (entities must have ALL specified tags)
    #[serde(default)]
    pub tags: Vec<String>,
    /// Filter by database.schema pattern
    pub database_schema: Option<String>,
    /// Response detail level: compact, standard, or full. Omit to use the active result profile.
    #[serde(default)]
    pub detail: Option<DetailLevel>,
    #[serde(default, flatten)]
    pub pagination: PaginationParams,
}

/// Parameters for searching and discovering recipe templates.
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct SearchRecipesParams {
    /// Optional text filter on recipe id, description, or tags.
    #[serde(default)]
    pub query: String,
    /// Optional topic prefix filter (e.g., `weekly`)
    #[serde(default)]
    pub topic: String,
    /// Include query names in the search response.
    #[serde(default)]
    pub include_queries: bool,
    #[serde(default, flatten)]
    pub pagination: PaginationParams,
}

/// Parameters for loading a recipe definition.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetRecipeParams {
    /// Recipe identifier derived from manifest analysis path (e.g. `weekly_country_kpi_report` or `marketplace/weekly_report`)
    pub recipe_id: String,
    /// Include full SQL text for each query file.
    #[serde(default)]
    pub include_sql: bool,
    /// Include query order and tags metadata.
    #[serde(default)]
    pub include_queries: bool,
    /// Optional execution parameter map for SQL template placeholders.
    #[serde(default)]
    pub parameters: Option<HashMap<String, JsonValue>>,
    /// Optional placeholder type hints (e.g. {"country_code":"string","target_table":"identifier"}).
    #[serde(default)]
    pub placeholder_types: Option<HashMap<String, String>>,
    /// Legacy compatibility alias for `placeholder_types`.
    /// Prefer `placeholder_types` for new clients.
    #[serde(default)]
    pub parameter_types: Option<HashMap<String, String>>,
}

/// Parameters for running a recipe as a reusable, deterministic analysis sequence.
#[derive(Debug, Deserialize, JsonSchema, Clone)]
pub struct RunRecipeParams {
    /// Recipe identifier derived from manifest analysis path (e.g. `weekly_country_kpi_report` or `marketplace/weekly_report`)
    pub recipe_id: String,
    /// Optional explicit query file names to execute.
    #[serde(default)]
    pub query_names: Vec<String>,
    /// Optional explicit 1-based query order indexes to execute.
    #[serde(default)]
    pub query_indexes: Vec<usize>,
    /// Continue executing remaining queries when one fails.
    #[serde(default = "default_true")]
    pub stop_on_failure: bool,
    /// Include SQL text in response payload.
    #[serde(default)]
    pub include_sql: bool,
    /// Optional row limit for each query execution.
    #[serde(default)]
    pub row_limit: Option<u64>,
    /// Optional byte limit for each query execution.
    #[serde(default)]
    pub byte_limit: Option<u64>,
    /// Optional maximum poll timeout in seconds for each query execution.
    #[serde(default)]
    pub max_poll_seconds: Option<u64>,
    /// Optional polling interval in milliseconds for each query execution.
    #[serde(default)]
    pub poll_interval_ms: Option<u64>,
    /// Optional timeout in seconds for each query execution.
    #[serde(default)]
    pub wait_timeout_s: Option<u64>,
    /// Optional execution parameter map for SQL templates.
    #[serde(default)]
    pub parameters: Option<HashMap<String, JsonValue>>,
    /// Optional placeholder type hints for `__TOKEN__` substitution
    /// (e.g., {"country_code":"string","target_table":"identifier"}).
    #[serde(default)]
    pub placeholder_types: Option<HashMap<String, String>>,
    /// Optional SQL bind parameter type hints for warehouse execution
    /// (e.g., {"as_of_date":"DATE"}).
    #[serde(default)]
    pub sql_parameter_types: Option<HashMap<String, String>>,
    /// Legacy compatibility alias fallback for both `placeholder_types` and
    /// `sql_parameter_types`.
    /// Prefer `placeholder_types` for placeholder coercion and
    /// `sql_parameter_types` for warehouse bind parameter hints in new clients.
    #[serde(default)]
    pub parameter_types: Option<HashMap<String, String>>,
    /// Fetch all result chunks for each query (default true in client)
    #[serde(default)]
    pub fetch_all_chunks: Option<bool>,
    /// Max chunks to fetch when fetch_all_chunks=true
    #[serde(default)]
    pub max_chunks: Option<usize>,
}

/// Parameters for the get_lineage tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetLineageParams {
    /// Unique ID or name of the entity
    pub id_or_name: String,
    /// Direction: "upstream" (what this depends on) or "downstream" (what depends on this)
    pub direction: String,
    /// Maximum depth (1 = direct only). Values above the config max are clamped.
    pub depth: Option<usize>,
    /// Filter results by resource types
    #[serde(default)]
    pub resource_types: Vec<String>,
    /// Response detail level: compact, standard, or full. Omit to use the active result profile.
    #[serde(default)]
    pub detail: Option<DetailLevel>,
}

/// Parameters for the get_sql tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetSqlParams {
    /// Unique ID or name of the model
    pub id_or_name: String,
    /// Return compiled SQL if true, raw SQL if false (default: false)
    #[serde(default)]
    pub compiled: bool,
}

/// Parameters for the get_columns tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetColumnsParams {
    /// Unique ID or name of the model/source
    pub id_or_name: String,
}

/// Parameters for the diff_entities tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct DiffEntitiesParams {
    /// First entity unique ID or name
    pub entity1: String,
    /// Optional resource_type to disambiguate entity1 when using name
    #[serde(default)]
    pub entity1_resource_type: Option<String>,
    /// Second entity unique ID or name
    pub entity2: String,
    /// Optional resource_type to disambiguate entity2 when using name
    #[serde(default)]
    pub entity2_resource_type: Option<String>,
    /// Fields to compare (default: columns)
    #[serde(default)]
    pub compare_fields: Vec<String>,
}

/// Parameters for the get_impact tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetImpactParams {
    /// Unique ID or name of the entity to assess
    pub id_or_name: String,
}

/// Parameters for the get_column_lineage tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetColumnLineageParams {
    /// Unique ID or name of the model/source
    pub id_or_name: String,
    /// Optional resource_type to disambiguate when using name
    #[serde(default)]
    pub resource_type: Option<String>,
    /// Column name to trace
    pub column_name: String,
    /// Direction: "upstream" (find column origins) or "downstream" (find column usage)
    pub direction: String,
    /// Maximum depth to traverse. Values above the config max are clamped.
    pub depth: Option<usize>,
    /// Confidence threshold: "high", "medium", or "low"
    #[serde(default)]
    pub confidence: Option<String>,
}

/// Parameters for the get_test_coverage tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetTestCoverageParams {
    /// Unique ID or name of the model/source to analyze
    pub id_or_name: String,
    /// Optional resource_type to disambiguate when using name
    #[serde(default)]
    pub resource_type: Option<String>,
    /// Include full test details (default: true)
    #[serde(default = "default_true")]
    pub include_full: bool,
    /// Maximum columns to return in columns_without_tests (default: 50)
    #[serde(default)]
    pub columns_limit: Option<usize>,
}

/// Parameters for the batch_get_entities tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct BatchGetParams {
    /// List of unique IDs to retrieve
    pub unique_ids: Vec<String>,
    /// Response detail level: compact, standard, or full. Omit to use the active result profile.
    #[serde(default)]
    pub detail: Option<DetailLevel>,
}

/// Parameters for the find_by_path tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct FindByPathParams {
    /// File path pattern to match (e.g., "models/staging/**", "models/*.sql")
    pub path_pattern: String,
    /// Filter by resource types (empty = all types)
    #[serde(default)]
    pub resource_types: Vec<String>,
    /// Response detail level: compact, standard, or full. Omit to use the active result profile.
    #[serde(default)]
    pub detail: Option<DetailLevel>,
    #[serde(default, flatten)]
    pub pagination: PaginationParams,
}

/// Parameters for the get_undocumented tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetUndocumentedParams {
    /// Resource type to check: model, source, macro, etc.
    pub resource_type: String,
    /// Optional entity identifier (unique_id or name) to scope results
    pub id_or_name: Option<String>,
    /// Optional package name to scope results
    pub package: Option<String>,
    /// Optional file path prefix to scope results (e.g. "models/staging/")
    pub path_prefix: Option<String>,
    /// Also check for undocumented columns (default: true)
    #[serde(default = "default_true")]
    pub include_columns: bool,
    /// Include full entity data (default: false for performance)
    #[serde(default)]
    pub include_full: bool,
    #[serde(default, flatten)]
    pub pagination: PaginationParams,
}

/// Detail level for validate_dag responses.
#[derive(Debug, Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ValidateDagDetail {
    /// Return full issues and orphaned lists (default).
    #[default]
    Full,
    /// Return only summary counts and orphaned types.
    Summary,
}

/// Parameters for the validate_dag tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ValidateDagParams {
    /// Detail level: full or summary (default: full)
    #[serde(default)]
    pub detail: ValidateDagDetail,
}

/// Parameters for the get_context tool.
#[derive(Debug, Deserialize, JsonSchema)]
#[allow(clippy::struct_excessive_bools)]
pub struct GetContextParams {
    /// Unique ID or name of the entity to get context for
    pub id_or_name: String,
    /// Optional resource_type to disambiguate when using name
    pub resource_type: Option<String>,
    /// Include column details with types and descriptions (default: true)
    #[serde(default = "default_true")]
    pub include_columns: bool,
    /// Include upstream lineage (default: true)
    #[serde(default = "default_true")]
    pub include_upstream: bool,
    /// Include downstream lineage (default: true)
    #[serde(default = "default_true")]
    pub include_downstream: bool,
    /// Include test coverage analysis (default: true)
    #[serde(default = "default_true")]
    pub include_tests: bool,
    /// Include related documentation (default: true)
    #[serde(default = "default_true")]
    pub include_docs: bool,
    /// Include raw and compiled SQL in the entity context (default: false)
    #[serde(default)]
    pub include_sql: bool,
    /// Context mode: standard (default) or engineer
    #[serde(default)]
    pub context_mode: ContextMode,
    #[serde(default, flatten)]
    pub limits: ContextLimits,
}

/// Output shaping for get_context responses.
#[derive(Debug, Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ContextMode {
    /// Standard context output (default).
    #[default]
    Standard,
    /// Engineer-focused output (suppresses long descriptions).
    Engineer,
}

/// Parameters for the get_metadata_score tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetMetadataScoreParams {
    /// Entity to score (optional for project scope)
    #[serde(default)]
    pub id_or_name: Option<String>,
    /// Optional resource_type to disambiguate when using name
    #[serde(default)]
    pub resource_type: Option<String>,
    /// Optional persona: "analyst", "engineer", "governance"
    #[serde(default)]
    pub persona: Option<String>,
    /// Scope: "entity", "column", or "project"
    #[serde(default)]
    pub scope: Option<String>,
    /// Include per-check details (default: true)
    #[serde(default = "default_true")]
    pub include_breakdown: bool,
    /// Include improvement suggestions (default: true)
    #[serde(default = "default_true")]
    pub include_recommendations: bool,
    /// Filter resource types for project scope
    #[serde(default)]
    pub resource_types: Vec<String>,
    /// Maximum entities to score for project scope
    #[serde(default)]
    pub limit: Option<usize>,
    /// Pagination offset for project scope
    #[serde(default)]
    pub offset: Option<usize>,
}

impl Default for GetMetadataScoreParams {
    fn default() -> Self {
        Self {
            id_or_name: None,
            resource_type: None,
            persona: None,
            scope: None,
            include_breakdown: true,
            include_recommendations: true,
            resource_types: Vec::new(),
            limit: None,
            offset: None,
        }
    }
}

/// Parameters for the get_agent_readiness tool.
#[derive(Debug, Deserialize, JsonSchema, Clone, Default)]
pub struct GetAgentReadinessParams {
    /// JSON array of personas to score. Defaults to `["engineer","analyst","governance"]`.
    #[serde(default)]
    pub personas_json: Option<String>,
    /// JSON threshold configuration. Defaults to advisory readiness thresholds.
    #[serde(default)]
    pub thresholds_json: Option<String>,
    /// Inline JSON from `dbt-nova eval gate <SUITE> --json` or the raw gate report.
    #[serde(default)]
    pub eval_gate_json: Option<String>,
}

/// Selection mode for the get_metadata_audit tool.
#[derive(Debug, Deserialize, JsonSchema, Clone, Copy, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MetadataAuditSelectionModeParam {
    /// Score all selected resource types.
    #[default]
    Project,
    /// Score manifest entities whose original_file_path or patch_path matches changed files.
    Changed,
    /// Score explicit entity ids or unique names.
    Entities,
}

/// Parameters for the get_metadata_audit tool.
#[derive(Debug, Deserialize, JsonSchema, Clone, Default)]
pub struct GetMetadataAuditParams {
    /// Audit selection mode. Defaults to project.
    #[serde(default)]
    pub selection_mode: MetadataAuditSelectionModeParam,
    /// JSON array of changed file paths for changed selection mode.
    #[serde(default)]
    pub changed_files_json: Option<String>,
    /// JSON array of entity ids or names for entities selection mode.
    #[serde(default)]
    pub entity_ids_json: Option<String>,
    /// JSON array of resource types. Defaults to `["model"]`.
    #[serde(default)]
    pub resource_types_json: Option<String>,
    /// JSON array of personas. Defaults to `["engineer"]`.
    #[serde(default)]
    pub personas_json: Option<String>,
    /// JSON threshold configuration for required/advisory gates.
    #[serde(default)]
    pub thresholds_json: Option<String>,
    /// Include per-check metadata score breakdowns. Defaults to true.
    #[serde(default)]
    pub include_breakdown: Option<bool>,
    /// Include metadata score recommendations. Defaults to true.
    #[serde(default)]
    pub include_recommendations: Option<bool>,
    /// Mark no-target selections as required failures.
    #[serde(default)]
    pub fail_on_no_targets: bool,
}

/// Resource kind selector for the validate_nova_meta tool.
#[derive(Debug, Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NovaMetaResourceKindParam {
    /// dbt model resource.
    Model,
    /// dbt source resource.
    Source,
    /// dbt source table resource.
    Table,
    /// dbt metric resource.
    Metric,
}

/// Parameters for the validate_nova_meta tool.
#[derive(Debug, Deserialize, JsonSchema, Clone, Default)]
pub struct ValidateNovaMetaParams {
    /// dbt project directory to scan. Relative paths are resolved under the server working directory.
    #[serde(default)]
    pub project_dir: Option<String>,
    /// Relative YAML file or directory paths under project_dir. Omit for project-wide validation.
    #[serde(default, alias = "path")]
    pub paths: Vec<String>,
    /// Optional resource kind selector.
    #[serde(default)]
    pub resource_kind: Option<NovaMetaResourceKindParam>,
    /// Optional resource name selector.
    #[serde(default)]
    pub resource_name: Option<String>,
    /// Optional column selector. Requires resource_name.
    #[serde(default)]
    pub column: Option<String>,
}

/// Parameters for the validate_eval_suite tool.
#[derive(Debug, Deserialize, JsonSchema, Clone, Default)]
pub struct ValidateEvalSuiteParams {
    /// YAML or JSON eval suite path under the server working directory.
    pub suite: String,
}

/// Parameters for the get_eval_gate tool.
#[derive(Debug, Deserialize, JsonSchema, Clone, Default)]
pub struct GetEvalGateParams {
    /// Eval suite name to check in telemetry.
    pub suite: String,
}

/// Parameters for the get_eval_history tool.
#[derive(Debug, Deserialize, JsonSchema, Clone, Default)]
pub struct GetEvalHistoryParams {
    /// Eval suite name to read history for.
    pub suite: String,
    /// Only return telemetry rows on or after this UTC date (YYYY-MM-DD).
    pub since: String,
}

/// Parameters for the compare_eval_runs tool.
#[derive(Debug, Deserialize, JsonSchema, Clone, Default)]
pub struct CompareEvalRunsParams {
    /// Before eval result directory or results.json path under the server working directory.
    pub before: String,
    /// After eval result directory or results.json path under the server working directory.
    pub after: String,
}

/// Parameters for the run_eval tool.
#[derive(Debug, Deserialize, JsonSchema, Clone, Default)]
pub struct RunEvalParams {
    /// YAML or JSON eval suite path under the server working directory.
    pub suite: String,
    /// Directory for eval result artifacts. Defaults to `.nova/eval-runs/...`.
    #[serde(default)]
    pub output_dir: Option<String>,
    /// Append per-assertion JSONL telemetry for this eval run.
    #[serde(default)]
    pub telemetry: bool,
    /// After writing telemetry, keep only the newest ROWS rows for this suite.
    #[serde(default)]
    pub telemetry_retention: Option<usize>,
    /// Only run the named bridge cases.
    #[serde(default)]
    pub case_ids: Vec<String>,
    /// Required pass rate between 0.0 and 1.0.
    #[serde(default)]
    pub fail_under: Option<f64>,
}

/// Parameters for the init_eval_suite tool.
#[derive(Debug, Deserialize, JsonSchema, Clone, Default)]
pub struct InitEvalSuiteParams {
    /// Persona to use in the generated starter suite. Defaults to analyst.
    #[serde(default)]
    pub persona: Option<String>,
    /// Path to write the suite YAML under the server working directory.
    pub out: String,
    /// Overwrite an existing suite file.
    #[serde(default)]
    pub force: bool,
}

/// Parameters for the run_agent_eval tool.
#[derive(Debug, Deserialize, JsonSchema, Clone, Default)]
pub struct RunAgentEvalParams {
    /// YAML or JSON eval suite path under the server working directory.
    pub suite: String,
    /// Provider preset: opencode, codex, claude, goose, or custom.
    #[serde(default)]
    pub provider: Option<String>,
    /// Model id to pass to provider presets that support --model.
    #[serde(default)]
    pub provider_model: Option<String>,
    /// Custom provider command to execute. Requires explicit custom-provider opt-in.
    #[serde(default)]
    pub provider_command: Option<String>,
    /// Custom provider arguments as a JSON string array with placeholders. Requires explicit custom-provider opt-in.
    #[serde(default)]
    pub provider_args_json: Option<String>,
    /// Local dbt manifest.json path under the server working directory.
    #[serde(default)]
    pub manifest_path: Option<String>,
    /// Remote manifest or prebuilt artifact URI.
    #[serde(default)]
    pub manifest_uri: Option<String>,
    /// Storage instance id for cached Nova assets.
    #[serde(default)]
    pub storage_instance_id: Option<String>,
    /// Directory for eval result artifacts. Defaults to `.nova/eval-runs/...`.
    #[serde(default)]
    pub output_dir: Option<String>,
    /// Append per-assertion JSONL telemetry for this eval run.
    #[serde(default)]
    pub telemetry: bool,
    /// After writing telemetry, keep only the newest ROWS rows for this suite.
    #[serde(default)]
    pub telemetry_retention: Option<usize>,
    /// Only run the named agent cases.
    #[serde(default)]
    pub case_ids: Vec<String>,
    /// Provider command timeout in seconds. Defaults to 600.
    #[serde(default)]
    pub timeout_secs: Option<u64>,
    /// Required pass rate between 0.0 and 1.0.
    #[serde(default)]
    pub fail_under: Option<f64>,
    /// Clear the selected storage instance before running the provider.
    #[serde(default)]
    pub cleanup_storage_on_start: bool,
    /// Open storage in read-only mode.
    #[serde(default)]
    pub read_only: bool,
}

/// Parameters for the inspect_tool_trace tool.
#[derive(Debug, Deserialize, JsonSchema, Clone, Default)]
pub struct TraceInspectParams {
    /// JSONL tool trace path under the server working directory.
    pub path: String,
}

/// Parameters for the summarize_tool_trace tool.
#[derive(Debug, Deserialize, JsonSchema, Clone, Default)]
pub struct TraceSummarizeParams {
    /// JSONL tool trace path under the server working directory.
    pub path: String,
    /// Optional Markdown report path under the server working directory. Requires trace write opt-in.
    #[serde(default)]
    pub report_md_path: Option<String>,
}

/// Parameters for the redact_tool_trace tool.
#[derive(Debug, Deserialize, JsonSchema, Clone, Default)]
pub struct TraceRedactParams {
    /// JSONL tool trace path under the server working directory.
    pub path: String,
    /// Output JSONL path under the server working directory. Requires trace write opt-in.
    pub out: String,
}

/// Parameters for the replay_tool_trace tool.
#[derive(Debug, Deserialize, JsonSchema, Clone, Default)]
pub struct TraceReplayParams {
    /// JSONL tool trace path under the server working directory.
    pub path: String,
}

/// Parameters for the show_config tool.
#[derive(Debug, Deserialize, JsonSchema, Clone, Default)]
pub struct ConfigShowParams {
    /// Return default configuration instead of the active runtime configuration.
    #[serde(default)]
    pub defaults: bool,
}

/// Parameters for the validate_config tool.
#[derive(Debug, Deserialize, JsonSchema, Clone, Default)]
pub struct ConfigValidateParams {}

/// Parameters for the reload_manifest tool.
#[derive(Debug, Deserialize, JsonSchema, Clone, Default)]
pub struct ReloadManifestParams {
    /// Manifest URI to load (e.g. http(s)://, s3://, gs://, dbfs://)
    pub manifest_uri: Option<String>,
    /// Local manifest path to load (clears manifest_uri if provided)
    pub manifest_path: Option<String>,
    /// Optional refresh interval override (seconds)
    pub refresh_secs: Option<u64>,
    /// Optional explicit storage instance id for index storage
    pub storage_instance_id: Option<String>,
}

impl ReloadManifestParams {
    /// Returns true when the request changes the live MCP server source,
    /// scheduler, or storage identity rather than reloading the current source.
    #[must_use]
    pub fn changes_runtime_source_or_storage(&self) -> bool {
        non_empty_param(self.manifest_uri.as_deref())
            || non_empty_param(self.manifest_path.as_deref())
            || non_empty_param(self.storage_instance_id.as_deref())
            || self.refresh_secs.is_some()
    }
}

fn non_empty_param(value: Option<&str>) -> bool {
    value.is_some_and(|value| !value.trim().is_empty())
}

/// Parameters for the warm_manifest tool.
#[derive(Debug, Deserialize, JsonSchema, Clone, Default)]
#[allow(clippy::struct_excessive_bools)]
pub struct WarmManifestParams {
    /// Warm vector embedding query/model caches.
    #[serde(default)]
    pub vector: bool,
    /// Warm sparse embedding query/model caches.
    #[serde(default)]
    pub sparse: bool,
    /// Warm reranker query/model caches.
    #[serde(default)]
    pub reranker: bool,
    /// Require freshly rebuilt manifest-scoped cache files.
    #[serde(default)]
    pub force: bool,
}

/// Parameters for the inspect_storage tool.
#[derive(Debug, Deserialize, JsonSchema, Clone, Default)]
pub struct StorageInspectParams {
    /// Optional storage instance id to inspect as the configured instance.
    #[serde(default)]
    pub storage_instance_id: Option<String>,
}

/// Parameters for the prune_storage tool.
#[derive(Debug, Deserialize, JsonSchema, Clone, Default)]
pub struct StoragePruneParams {
    /// Number of stale storage instances to retain. Defaults to storage policy.
    #[serde(default)]
    pub max_keep: Option<usize>,
    /// Maximum total storage bytes to retain. Defaults to storage policy.
    #[serde(default)]
    pub max_bytes: Option<u64>,
    /// Optional storage instance id to protect from pruning.
    #[serde(default)]
    pub storage_instance_id: Option<String>,
}

/// Parameters for the cleanup_storage tool.
#[derive(Debug, Deserialize, JsonSchema, Clone, Default)]
pub struct StorageCleanupParams {
    /// Optional storage instance id to clean up.
    #[serde(default)]
    pub storage_instance_id: Option<String>,
}

/// Parameters for the execute_sql tool.
#[derive(Debug, Deserialize, JsonSchema, Clone)]
pub struct ExecuteSqlParams {
    /// SQL statement to execute.
    /// Optional when `preflight_only=true`.
    #[serde(default, alias = "sql")]
    pub statement: String,
    /// Optional override for Databricks warehouse id or http path
    pub warehouse_id: Option<String>,
    /// Run provider diagnostics without executing the main statement.
    #[serde(default)]
    pub preflight_only: bool,
    /// Optional catalog to check during SQL preflight.
    #[serde(default)]
    pub preflight_catalog: Option<String>,
    /// Optional schema to check during SQL preflight.
    #[serde(default)]
    pub preflight_schema: Option<String>,
    /// Optional relation (table/view) to check during SQL preflight.
    #[serde(default)]
    pub preflight_relation: Option<String>,
    /// Optional row limit for result payload
    #[serde(default)]
    pub row_limit: Option<u64>,
    /// Optional byte limit for result payload
    #[serde(default)]
    pub byte_limit: Option<u64>,
    /// Optional wait timeout in seconds (0 or 5-50 for Databricks)
    #[serde(default)]
    pub wait_timeout_s: Option<u64>,
    /// Optional polling interval in milliseconds
    #[serde(default)]
    pub poll_interval_ms: Option<u64>,
    /// Optional max poll time in seconds
    #[serde(default)]
    pub max_poll_seconds: Option<u64>,
    /// Optional named parameters for the statement
    #[serde(default)]
    pub parameters: Option<HashMap<String, JsonValue>>,
    /// Optional SQL types for parameters (e.g. {"d":"DATE"})
    #[serde(default)]
    pub parameter_types: Option<HashMap<String, String>>,
    /// Fetch all result chunks (default true in client)
    #[serde(default)]
    pub fetch_all_chunks: Option<bool>,
    /// Max chunks to fetch when fetch_all_chunks is true
    #[serde(default)]
    pub max_chunks: Option<usize>,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{ExecuteSqlParams, GetEntityParams};

    #[test]
    fn get_entity_accepts_unique_id_alias() {
        let params: GetEntityParams =
            serde_json::from_value(json!({"unique_id": "model.pkg.orders"})).expect("params");

        assert_eq!(params.id_or_name, "model.pkg.orders");
    }

    #[test]
    fn execute_sql_accepts_sql_alias() {
        let params: ExecuteSqlParams =
            serde_json::from_value(json!({"sql": "select 1"})).expect("params");

        assert_eq!(params.statement, "select 1");
    }
}
