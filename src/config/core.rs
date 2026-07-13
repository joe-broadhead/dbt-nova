use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::collections::{BTreeSet, HashMap};
use std::path::{Component, Path, PathBuf};

use blake3;

use crate::error::{DbtNovaError, Result};
use crate::logging::LogFormat;
use crate::params::DetailLevel;
use crate::tools::catalog::{
    DEFAULT_MCP_TOOL_PROFILE, MCP_TOOL_NAMES, MCP_TOOL_PROFILE_NAMES, mcp_tool_profile_names,
};

use super::column_lineage::ColumnLineageConfig;
use super::hosted_auth::{HostedAuthConfig, HostedAuthMode};
use super::metadata_score::MetadataScoreConfig;
use super::search::SearchConfig;
use super::warehouse::DEFAULT_SQL_PROVIDER;
use super::{env_string, parse_bool, parse_f64, parse_u16, parse_u64, parse_usize, set_string};
use tracing::{info, warn};

/// Rule for mapping entities into logical layers (e.g., staging, mart, core).
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct LayerRule {
    /// Layer name to assign when the rule matches.
    pub layer: String,
    /// Match by file path prefix (e.g., "models/marts/").
    pub path_prefix: Option<String>,
    /// Match by entity name prefix (e.g., "fct_").
    pub name_prefix: Option<String>,
    /// Match by entity name regex (e.g., "^fct_.*").
    pub name_regex: Option<String>,
    /// Match when entity has a specific tag.
    pub tag: Option<String>,
    /// Match only specific resource types (e.g., "model").
    pub resource_type: Option<String>,
}

/// Governance gate policy thresholds used by governance persona payloads.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
#[allow(clippy::struct_excessive_bools)]
pub struct GovernanceGateConfig {
    /// Minimum metadata score required to pass the grade gate.
    pub min_metadata_score: u8,
    /// Minimum documentation coverage percentage required to pass docs gate.
    pub min_documentation_coverage_pct: f64,
    /// Whether test coverage is required for pass.
    pub require_tests: bool,
    /// Whether owner metadata is required for pass.
    pub require_owner: bool,
    /// Whether required Nova fields are enforced.
    pub require_required_fields: bool,
    /// Whether PII requires explicit compliance tags.
    pub require_compliance_for_pii: bool,
    /// Whether gate failures should block (`fail`) instead of advisory-only.
    pub block_on_failure: bool,
}

impl Default for GovernanceGateConfig {
    fn default() -> Self {
        Self {
            min_metadata_score: 90,
            min_documentation_coverage_pct: 80.0,
            require_tests: true,
            require_owner: true,
            require_required_fields: true,
            require_compliance_for_pii: true,
            block_on_failure: true,
        }
    }
}

impl GovernanceGateConfig {
    fn profile(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "strict" => Some(Self::default()),
            "standard" => Some(Self {
                min_metadata_score: 80,
                ..Self::default()
            }),
            "advisory" => Some(Self {
                min_metadata_score: 80,
                block_on_failure: false,
                ..Self::default()
            }),
            _ => None,
        }
    }
}

/// Runtime controls for deterministic agent-modelling audit findings.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct AgentModellingAuditConfig {
    /// Whether deterministic manifest/metadata/catalog agent-modelling findings are emitted.
    pub enabled: bool,
    /// Maximum agent-modelling findings retained in `modelling_consistency_report`.
    pub max_findings: usize,
    /// Direct-parent count at which analyst-facing surfaces are flagged as too wide.
    pub too_many_parents_threshold: usize,
    /// Source fanout threshold reserved for future source-shape checks.
    pub source_fanout_threshold: usize,
    /// Whether optional SQL-shape checks are allowed to run.
    pub enable_sql_shape_checks: bool,
}

impl Default for AgentModellingAuditConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_findings: 100,
            too_many_parents_threshold: 7,
            source_fanout_threshold: 20,
            enable_sql_shape_checks: false,
        }
    }
}

impl AgentModellingAuditConfig {
    fn validate(&self) -> Result<()> {
        if self.max_findings == 0 {
            return Err(DbtNovaError::InvalidParams(
                "agent_modelling_audit.max_findings must be greater than 0".to_string(),
            ));
        }
        if self.too_many_parents_threshold == 0 {
            return Err(DbtNovaError::InvalidParams(
                "agent_modelling_audit.too_many_parents_threshold must be greater than 0"
                    .to_string(),
            ));
        }
        if self.source_fanout_threshold == 0 {
            return Err(DbtNovaError::InvalidParams(
                "agent_modelling_audit.source_fanout_threshold must be greater than 0".to_string(),
            ));
        }
        Ok(())
    }
}

/// Agent-readiness thresholds for modelling findings.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(default)]
pub struct AgentReadinessConfig {
    /// Modelling finding thresholds used by agent-readiness integration.
    pub modelling: AgentReadinessModellingConfig,
}

/// Agent-readiness thresholds for deterministic modelling findings.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct AgentReadinessModellingConfig {
    /// Maximum blocker findings allowed before readiness fails.
    pub max_blockers: usize,
    /// Maximum high-severity findings allowed before advisory/required threshold triggers.
    pub max_high: usize,
    /// Whether exceeding `max_blockers` is required/failing instead of advisory.
    pub max_blockers_required: bool,
    /// Whether exceeding `max_high` is required/failing instead of advisory.
    pub max_high_required: bool,
}

impl Default for AgentReadinessModellingConfig {
    fn default() -> Self {
        Self {
            max_blockers: 0,
            max_high: 10,
            max_blockers_required: true,
            max_high_required: false,
        }
    }
}

/// Fetch policy for remote prebuilt artifact materialization.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactFetchPolicy {
    /// Materialize artifacts when local materialization is missing.
    #[default]
    IfMissing,
    /// Always re-fetch and re-materialize artifacts.
    Always,
    /// Never fetch artifacts; rely on pre-materialized local files.
    Never,
}

impl ArtifactFetchPolicy {
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "if_missing" => Some(Self::IfMissing),
            "always" => Some(Self::Always),
            "never" => Some(Self::Never),
            _ => None,
        }
    }
}

/// MCP server transport mode.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ServerTransport {
    /// Use stdio transport for local MCP clients.
    #[default]
    Stdio,
    /// Use streamable HTTP transport for hosted/server deployments.
    StreamableHttp,
}

impl ServerTransport {
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "stdio" => Some(Self::Stdio),
            "streamable_http" | "streamable-http" | "http" => Some(Self::StreamableHttp),
            _ => None,
        }
    }
}

/// Default result shaping profile for tools that support `detail`.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ResultProfile {
    /// Compact agent-first contract.
    Compact,
    /// Standard summary contract.
    #[default]
    Standard,
    /// Full entity payloads when the tool supports them.
    Full,
}

impl ResultProfile {
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "compact" => Some(Self::Compact),
            "standard" => Some(Self::Standard),
            "full" => Some(Self::Full),
            _ => None,
        }
    }

    #[must_use]
    pub fn detail_level(self) -> DetailLevel {
        match self {
            Self::Compact => DetailLevel::Compact,
            Self::Standard => DetailLevel::Standard,
            Self::Full => DetailLevel::Full,
        }
    }
}

/// Runtime deployment presets applied before environment overrides.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum RuntimePreset {
    /// Current local/default behavior.
    #[default]
    LocalDev,
    /// Offline-friendly audit posture for CI metadata checks.
    CiAudit,
    /// Hosted discovery posture with execution/admin/write tools hidden by default.
    HostedDiscovery,
    /// Hosted trusted analyst posture with direct SQL available and admin/write tools hidden.
    HostedSqlTrusted,
}

impl RuntimePreset {
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "local-dev" | "local_dev" | "local" => Some(Self::LocalDev),
            "ci-audit" | "ci_audit" | "ci" => Some(Self::CiAudit),
            "hosted-discovery" | "hosted_discovery" | "hosted" => Some(Self::HostedDiscovery),
            "hosted-sql-trusted" | "hosted_sql_trusted" | "hosted-sql" | "hosted_sql" => {
                Some(Self::HostedSqlTrusted)
            }
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LocalDev => "local-dev",
            Self::CiAudit => "ci-audit",
            Self::HostedDiscovery => "hosted-discovery",
            Self::HostedSqlTrusted => "hosted-sql-trusted",
        }
    }
}

pub const CI_AUDIT_TOOL_DENYLIST: &[&str] = &["execute_sql", "run_recipe"];

pub const HOSTED_DISCOVERY_TOOL_DENYLIST: &[&str] = &[
    "execute_sql",
    "run_recipe",
    "reload_manifest",
    "warm_manifest",
    "show_config",
    "validate_config",
    "inspect_storage",
    "prune_storage",
    "cleanup_storage",
    "run_eval",
    "init_eval_suite",
    "run_agent_eval",
    "summarize_tool_trace",
    "redact_tool_trace",
    "replay_tool_trace",
];

pub const HOSTED_SQL_TRUSTED_TOOL_DENYLIST: &[&str] = &[
    "run_recipe",
    "reload_manifest",
    "warm_manifest",
    "show_config",
    "validate_config",
    "inspect_storage",
    "prune_storage",
    "cleanup_storage",
    "run_eval",
    "init_eval_suite",
    "run_agent_eval",
    "summarize_tool_trace",
    "redact_tool_trace",
    "replay_tool_trace",
];

fn tool_name_csv(tools: &[&str]) -> String {
    tools.join(",")
}

/// Configuration for the dbt-nova server.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
#[allow(clippy::struct_excessive_bools)]
pub struct DbtNovaConfig {
    /// Runtime preset applied before environment overrides.
    pub runtime_preset: RuntimePreset,
    /// Path to the dbt `manifest.json` file
    pub manifest_path: String,
    /// True when `manifest_path` was set explicitly by env/CLI/user config.
    #[serde(skip)]
    pub manifest_path_explicit: bool,
    /// Optional dbt `catalog.json` path/URI for warehouse column metadata.
    pub catalog_path: String,
    /// Optional manifest URI (file://, http(s)://, dbfs://, s3://, gs://)
    pub manifest_uri: String,
    /// Optional manifest cache directory (defaults under storage root)
    pub manifest_cache_dir: String,
    /// Default recipe directory (relative to manifest directory when relative path is provided)
    pub recipes_dir: String,
    /// Seconds to keep cached manifest before refreshing (0 = never refresh)
    pub manifest_refresh_secs: u64,
    /// Maximum number of bytes allowed for remote manifest fetches (0 = unlimited)
    pub manifest_max_bytes: u64,
    /// HTTP connect timeout (seconds) for remote manifests (0 = no timeout)
    pub manifest_http_connect_timeout_secs: u64,
    /// HTTP request timeout (seconds) for remote manifests (0 = no timeout)
    pub manifest_http_timeout_secs: u64,
    /// Total fetch deadline (seconds) for remote manifests (0 = no deadline)
    pub manifest_fetch_timeout_secs: u64,
    /// Allow non-TLS manifest URIs (http://). Disable to enforce HTTPS-only.
    pub manifest_allow_http: bool,
    /// Optional allowlist of dbt `unique_id` patterns to retain during manifest parse.
    pub manifest_prune_allow_ids: Vec<String>,
    /// Optional denylist of dbt `unique_id` patterns to remove during manifest parse.
    pub manifest_prune_deny_ids: Vec<String>,
    /// Base directory for on-disk storage (instances live under `storage_dir/instances`)
    pub storage_dir: String,
    /// Storage instance id (generated from manifest when empty)
    pub storage_instance_id: String,
    /// Whether to clean storage directory on startup
    pub cleanup_storage_on_start: bool,
    /// Max number of storage instances to retain (0 = unlimited)
    pub storage_max_instances: usize,
    /// Minimum number of manifest versions to retain per instance
    pub storage_min_versions: usize,
    /// Max total storage bytes to retain across instances (0 = unlimited)
    pub storage_max_bytes: u64,
    /// Max seconds to wait for an existing build lock before failing
    pub storage_build_lock_wait_secs: u64,
    /// Disallow building indexes (fail if instance not present)
    pub storage_read_only: bool,
    /// Remote URI to the prebuilt storage archive (`file://`, `s3://`, `gs://`, `dbfs://`, `http(s)://`)
    pub storage_artifact_uri: String,
    /// Remote URI to the prebuilt metadata contract JSON
    pub metadata_artifact_uri: String,
    /// Optional remote URI to the prebuilt models archive
    pub models_artifact_uri: String,
    /// Optional local cache directory for downloaded prebuilt artifacts
    pub artifacts_cache_dir: String,
    /// Optional remote URI to a bootstrap contract JSON document.
    pub bootstrap_uri: String,
    /// Remote artifact fetch policy
    pub artifact_fetch_policy: ArtifactFetchPolicy,
    /// Timeout in seconds for artifact fetch operations (0 = no timeout)
    pub artifact_timeout_secs: u64,
    /// Maximum compressed bytes allowed for remote prebuilt artifact downloads (0 = unlimited)
    pub artifact_max_bytes: u64,
    /// Maximum entries allowed while extracting a prebuilt artifact archive (0 = unlimited)
    pub artifact_archive_max_entries: usize,
    /// Maximum decompressed bytes allowed while extracting a prebuilt artifact archive (0 = unlimited)
    pub artifact_archive_max_uncompressed_bytes: u64,
    /// Allow non-TLS remote artifact URIs (`http://`)
    pub artifact_allow_http: bool,
    /// Server transport mode (`stdio` or `streamable_http`)
    pub server_transport: ServerTransport,
    /// Log output format when `DBT_NOVA_LOG` or `RUST_LOG` enables logs.
    pub log_format: LogFormat,
    /// Bind host for hosted HTTP mode
    pub http_host: String,
    /// Bind port for hosted HTTP mode
    pub http_port: u16,
    /// HTTP mount path for MCP requests
    pub http_path: String,
    /// Explicit acknowledgement that hosted HTTP is protected by an authenticating reverse proxy.
    pub http_expect_auth_proxy: bool,
    /// Additional inbound Host header values allowed by the Streamable HTTP transport.
    pub http_allowed_hosts: String,
    /// Whether hosted HTTP mode should use stateful sessions
    pub http_stateful_mode: bool,
    /// SSE keepalive interval in seconds for hosted HTTP mode (0 = disable)
    pub http_sse_keep_alive_secs: u64,
    /// SSE retry hint in seconds for hosted HTTP mode (0 = disable)
    pub http_sse_retry_secs: u64,
    /// Maximum inbound HTTP request body bytes for hosted mode (0 = unlimited)
    pub http_max_body_bytes: usize,
    /// Expose the Prometheus-compatible `/metrics` endpoint in hosted HTTP mode.
    pub metrics_enabled: bool,
    /// Default-off hosted HTTP identity/authentication skeleton.
    pub hosted_auth: HostedAuthConfig,
    /// Per-tool rate limits (comma-separated, e.g. "`search=60,execute_sql=30,default=120`")
    pub tool_rate_limits: String,
    /// Rate limit window size in seconds
    pub tool_rate_limit_window_secs: u64,
    /// Optional tool allowlist (comma-separated exact MCP tool names)
    pub tool_allowlist: String,
    /// Optional tool denylist (comma-separated exact MCP tool names)
    pub tool_denylist: String,
    /// MCP tool profile (`agent`, `analyst`, `engineer`, `governance`, `ops`, or `all`)
    pub tool_profile: String,
    /// Default result profile for non-MCP tool calls when detail is omitted
    pub result_profile: ResultProfile,
    /// Default result profile for MCP tool calls when detail is omitted
    pub mcp_result_profile: ResultProfile,
    /// Default MCP result limit when limit is omitted or 0
    pub mcp_default_limit: usize,
    /// Maximum MCP result page size before core search caps are applied (0 disables MCP-specific cap)
    pub mcp_max_page_size: usize,
    /// Maximum serialized MCP tool response bytes (0 disables central response budgeting)
    pub mcp_max_response_bytes: usize,
    /// Maximum characters retained for long strings when MCP response budgeting truncates
    pub mcp_max_string_chars: usize,
    /// Include `_nova_result_meta` on MCP responses when the central budget pass runs
    pub mcp_include_truncation_meta: bool,
    /// SQL provider for `execute_sql` (default: "databricks")
    pub sql_provider: String,
    /// Max rows allowed for `execute_sql` requests (0 = unlimited)
    pub sql_max_row_limit: u64,
    /// Max bytes allowed for `execute_sql` requests (0 = unlimited)
    pub sql_max_byte_limit: u64,
    /// Max result chunks allowed for `execute_sql` requests (0 = unlimited)
    pub sql_max_chunks: usize,
    /// Max poll timeout seconds allowed for `execute_sql` requests (0 = unlimited)
    pub sql_max_poll_seconds: u64,
    /// Minimum poll interval in ms for `execute_sql` requests (0 disables floor)
    pub sql_min_poll_interval_ms: u64,
    /// Max concurrent `execute_sql`/`run_recipe` requests (0 = unlimited)
    pub sql_max_concurrent: usize,
    /// Max queued `execute_sql`/`run_recipe` requests while saturated
    pub sql_max_queue: usize,
    /// Max milliseconds to wait for a SQL execution slot (0 disables timeout)
    pub sql_queue_timeout_ms: u64,
    /// Max number of entities to cache in memory (0 disables cache)
    pub entity_cache_size: usize,
    /// Max number of entities allowed in a single `batch_get_entities` call (0 disables limit)
    pub batch_get_max_items: usize,
    /// Maximum traversal depth for entity lineage
    pub lineage_max_depth: usize,
    /// Maximum results returned for entity lineage
    pub lineage_max_results: usize,
    /// Column lineage matching configuration
    pub column_lineage: ColumnLineageConfig,
    /// Search configuration
    pub search: SearchConfig,
    /// Metadata scoring configuration
    pub metadata_score: MetadataScoreConfig,
    /// Optional layer mapping rules for persona summaries
    pub layer_rules: Vec<LayerRule>,
    /// Governance-required Nova fields by resource type
    pub governance_required_fields: HashMap<String, Vec<String>>,
    /// Governance gate threshold policy for persona payloads
    pub governance_gate: GovernanceGateConfig,
    /// Runtime controls for deterministic agent-modelling audit findings.
    pub agent_modelling_audit: AgentModellingAuditConfig,
    /// Agent readiness thresholds for deterministic modelling findings.
    pub agent_readiness: AgentReadinessConfig,
    /// Days after which provenance timestamps are marked stale.
    pub provenance_stale_after_days: u64,
    /// Runtime bootstrap resolution status (not user-configurable).
    #[serde(skip)]
    pub bootstrap_status: Option<JsonValue>,
    /// Configuration errors captured while reading fallible environment values.
    #[serde(skip)]
    pub env_errors: Vec<String>,
}

impl Default for DbtNovaConfig {
    fn default() -> Self {
        Self {
            runtime_preset: RuntimePreset::LocalDev,
            manifest_path: "manifest.json".to_string(),
            manifest_path_explicit: false,
            catalog_path: String::new(),
            manifest_uri: String::new(),
            manifest_cache_dir: String::new(),
            recipes_dir: "analyses/recipes".to_string(),
            manifest_refresh_secs: 300,
            manifest_max_bytes: 256 * 1024 * 1024, // 256 MiB
            manifest_http_connect_timeout_secs: 10,
            manifest_http_timeout_secs: 120,
            manifest_fetch_timeout_secs: 300,
            manifest_allow_http: false,
            manifest_prune_allow_ids: Vec::new(),
            manifest_prune_deny_ids: Vec::new(),
            storage_dir: ".dbt-nova".to_string(),
            storage_instance_id: String::new(),
            cleanup_storage_on_start: false,
            storage_max_instances: 3,
            storage_min_versions: 2,
            storage_max_bytes: 5 * 1024 * 1024 * 1024, // 5 GiB
            storage_build_lock_wait_secs: 300,
            storage_read_only: false,
            storage_artifact_uri: String::new(),
            metadata_artifact_uri: String::new(),
            models_artifact_uri: String::new(),
            artifacts_cache_dir: String::new(),
            bootstrap_uri: String::new(),
            artifact_fetch_policy: ArtifactFetchPolicy::IfMissing,
            artifact_timeout_secs: 300,
            artifact_max_bytes: 3 * 1024 * 1024 * 1024, // 3 GiB
            artifact_archive_max_entries: 200_000,
            artifact_archive_max_uncompressed_bytes: 10 * 1024 * 1024 * 1024, // 10 GiB
            artifact_allow_http: false,
            server_transport: ServerTransport::Stdio,
            log_format: LogFormat::Human,
            http_host: "127.0.0.1".to_string(),
            http_port: 8000,
            http_path: "/mcp".to_string(),
            http_expect_auth_proxy: false,
            http_allowed_hosts: String::new(),
            http_stateful_mode: true,
            http_sse_keep_alive_secs: 15,
            http_sse_retry_secs: 3,
            http_max_body_bytes: 16 * 1024 * 1024,
            metrics_enabled: true,
            hosted_auth: HostedAuthConfig::default(),
            tool_rate_limits: "search=60,execute_sql=20,default=120".to_string(),
            tool_rate_limit_window_secs: 60,
            tool_allowlist: String::new(),
            tool_denylist: String::new(),
            tool_profile: DEFAULT_MCP_TOOL_PROFILE.to_string(),
            result_profile: ResultProfile::Standard,
            mcp_result_profile: ResultProfile::Compact,
            mcp_default_limit: 10,
            mcp_max_page_size: 100,
            mcp_max_response_bytes: 65_536,
            mcp_max_string_chars: 4_096,
            mcp_include_truncation_meta: true,
            sql_provider: DEFAULT_SQL_PROVIDER.to_string(),
            sql_max_row_limit: 10_000,
            sql_max_byte_limit: 100_000_000,
            sql_max_chunks: 100,
            sql_max_poll_seconds: 900,
            sql_min_poll_interval_ms: 200,
            sql_max_concurrent: 10,
            sql_max_queue: 20,
            sql_queue_timeout_ms: 30_000,
            entity_cache_size: 1000,
            batch_get_max_items: 5_000,
            lineage_max_depth: 200,
            lineage_max_results: 10_000,
            column_lineage: ColumnLineageConfig::default(),
            search: SearchConfig::default(),
            metadata_score: MetadataScoreConfig::default(),
            layer_rules: Vec::new(),
            governance_required_fields: default_governance_required_fields(),
            governance_gate: GovernanceGateConfig::default(),
            agent_modelling_audit: AgentModellingAuditConfig::default(),
            agent_readiness: AgentReadinessConfig::default(),
            provenance_stale_after_days: 30,
            bootstrap_status: None,
            env_errors: Vec::new(),
        }
    }
}

fn default_governance_required_fields() -> HashMap<String, Vec<String>> {
    HashMap::from([
        (
            "model".to_string(),
            vec![
                "nova.domains".to_string(),
                "nova.use_cases".to_string(),
                "nova.synonyms".to_string(),
                "nova.tier".to_string(),
                "nova.governance.sensitivity".to_string(),
                "nova.governance.pii".to_string(),
                "nova.governance.compliance".to_string(),
            ],
        ),
        (
            "source".to_string(),
            vec![
                "nova.domains".to_string(),
                "nova.governance.sensitivity".to_string(),
                "nova.governance.pii".to_string(),
                "nova.governance.compliance".to_string(),
            ],
        ),
    ])
}

impl DbtNovaConfig {
    /// Apply a runtime preset before environment or CLI overrides.
    pub fn apply_runtime_preset(&mut self, preset: RuntimePreset) {
        self.runtime_preset = preset;
        match preset {
            RuntimePreset::LocalDev => {}
            RuntimePreset::CiAudit => {
                self.search.enable_vector_search = false;
                self.search.enable_sparse_search = false;
                self.search.enable_reranker = false;
                self.tool_denylist = tool_name_csv(CI_AUDIT_TOOL_DENYLIST);
            }
            RuntimePreset::HostedDiscovery => {
                self.server_transport = ServerTransport::StreamableHttp;
                self.tool_profile = DEFAULT_MCP_TOOL_PROFILE.to_string();
                self.tool_denylist = tool_name_csv(HOSTED_DISCOVERY_TOOL_DENYLIST);
            }
            RuntimePreset::HostedSqlTrusted => {
                self.server_transport = ServerTransport::StreamableHttp;
                self.tool_profile = "analyst".to_string();
                self.tool_denylist = tool_name_csv(HOSTED_SQL_TRUSTED_TOOL_DENYLIST);
            }
        }
    }

    /// Whether manifest pruning is enabled.
    #[must_use]
    pub fn manifest_pruning_enabled(&self) -> bool {
        !canonical_prune_patterns(&self.manifest_prune_allow_ids).is_empty()
            || !canonical_prune_patterns(&self.manifest_prune_deny_ids).is_empty()
    }

    /// Deterministic fingerprint for pruning inputs used in cache/reuse identity.
    #[must_use]
    pub fn manifest_prune_fingerprint(&self) -> String {
        if !self.manifest_pruning_enabled() {
            return String::new();
        }

        let allow = canonical_prune_patterns(&self.manifest_prune_allow_ids);
        let deny = canonical_prune_patterns(&self.manifest_prune_deny_ids);
        let payload = serde_json::json!({
            "allow": allow,
            "deny": deny,
        });
        blake3::hash(payload.to_string().as_bytes())
            .to_hex()
            .to_string()
    }

    /// Ensure a deterministic storage instance id is set.
    pub fn ensure_storage_instance_id(&mut self) {
        if !self.storage_instance_id.trim().is_empty() {
            return;
        }
        let mut seed = if self.manifest_uri.trim().is_empty() {
            self.manifest_path.clone()
        } else {
            self.manifest_uri.clone()
        };
        if self.manifest_uri.trim().is_empty()
            && let Ok(path) = std::fs::canonicalize(&self.manifest_path)
        {
            seed = path.to_string_lossy().to_string();
        }
        let hash = blake3::hash(seed.as_bytes());
        let hex = hash.to_hex();
        let short = &hex.as_str()[..12.min(hex.as_str().len())];
        self.storage_instance_id = format!("manifest-{short}");
    }

    /// Resolve the manifest cache directory, creating a default if unset.
    ///
    /// # Errors
    ///
    /// Returns an error if the storage root cannot be resolved.
    pub fn manifest_cache_dir(&self) -> Result<PathBuf> {
        if !self.manifest_cache_dir.trim().is_empty() {
            return Ok(PathBuf::from(&self.manifest_cache_dir));
        }
        Ok(self.storage_root_dir()?.join("manifests"))
    }

    /// Resolve the artifacts cache directory, creating a default if unset.
    ///
    /// # Errors
    ///
    /// Returns an error if the storage root cannot be resolved.
    pub fn artifacts_cache_dir(&self) -> Result<PathBuf> {
        if !self.artifacts_cache_dir.trim().is_empty() {
            return Ok(PathBuf::from(&self.artifacts_cache_dir));
        }
        Ok(self.storage_root_dir()?.join("artifacts"))
    }

    /// Whether remote prebuilt artifact mode is configured.
    #[must_use]
    pub fn remote_artifact_mode_enabled(&self) -> bool {
        !self.storage_artifact_uri.trim().is_empty()
            || !self.metadata_artifact_uri.trim().is_empty()
            || !self.models_artifact_uri.trim().is_empty()
    }

    #[must_use]
    pub fn uses_home_storage_root_fallback(&self) -> bool {
        !self.manifest_uri.trim().is_empty()
            && self.manifest_cache_dir.trim().is_empty()
            && !Path::new(&self.storage_dir).is_absolute()
    }

    /// Ensure the embedding cache directory is set under the cache root.
    pub fn ensure_embedding_cache_dir(&mut self) {
        if !self.search.embedding_cache_dir.trim().is_empty() {
            return;
        }
        if let Ok(exe_path) = std::env::current_exe()
            && let Some(parent) = exe_path.parent()
        {
            let bundled = parent.join("models");
            if bundled.is_dir() {
                self.search.embedding_cache_dir = bundled.to_string_lossy().to_string();
                info!(
                    embedding_cache_dir = %self.search.embedding_cache_dir,
                    source = "adjacent_executable_models_dir",
                    "selected embedding cache dir"
                );
                return;
            }
        }
        if let Ok(home) = std::env::var("HOME") {
            let home_path = PathBuf::from(&home);
            let bundled = home_path.join(".local").join("bin").join("models");
            if bundled.is_dir() {
                self.search.embedding_cache_dir = bundled.to_string_lossy().to_string();
                info!(
                    embedding_cache_dir = %self.search.embedding_cache_dir,
                    source = "local_bin_models_dir",
                    "selected embedding cache dir"
                );
                return;
            }
            self.search.embedding_cache_dir = home_path
                .join(".dbt-nova")
                .join(".fastembed_cache")
                .to_string_lossy()
                .to_string();
            info!(
                embedding_cache_dir = %self.search.embedding_cache_dir,
                source = "home_default",
                "selected embedding cache dir"
            );
            return;
        }
        self.search.embedding_cache_dir = PathBuf::from(".dbt-nova")
            .join(".fastembed_cache")
            .to_string_lossy()
            .to_string();
        info!(
            embedding_cache_dir = %self.search.embedding_cache_dir,
            source = "relative_default",
            "selected embedding cache dir"
        );
    }

    #[must_use]
    pub fn parsed_tool_allowlist(&self) -> Option<Vec<String>> {
        let allowlist = parse_tool_name_csv(&self.tool_allowlist);
        if allowlist.is_empty() {
            None
        } else {
            Some(allowlist)
        }
    }

    #[must_use]
    pub fn parsed_tool_denylist(&self) -> Vec<String> {
        parse_tool_name_csv(&self.tool_denylist)
    }

    #[must_use]
    pub fn resolved_mcp_tool_names(&self) -> BTreeSet<String> {
        let mut eligible = if let Some(allowlist) = self.parsed_tool_allowlist() {
            allowlist.into_iter().collect::<BTreeSet<_>>()
        } else {
            mcp_tool_profile_names(&self.tool_profile)
                .unwrap_or(&MCP_TOOL_NAMES)
                .iter()
                .map(|name| (*name).to_string())
                .collect()
        };
        for denied in self.parsed_tool_denylist() {
            eligible.remove(&denied);
        }
        eligible
    }

    fn validate_tool_filters(&self) -> Result<()> {
        let valid_names = MCP_TOOL_NAMES.iter().copied().collect::<BTreeSet<_>>();
        let allowlist = self.parsed_tool_allowlist().unwrap_or_default();
        let denylist = self.parsed_tool_denylist();

        let invalid_allowlist = allowlist
            .into_iter()
            .filter(|name| !valid_names.contains(name.as_str()))
            .collect::<BTreeSet<_>>();
        let invalid_denylist = denylist
            .into_iter()
            .filter(|name| !valid_names.contains(name.as_str()))
            .collect::<BTreeSet<_>>();

        let valid_profile = mcp_tool_profile_names(&self.tool_profile).is_some();

        if invalid_allowlist.is_empty() && invalid_denylist.is_empty() && valid_profile {
            return Ok(());
        }

        let mut invalid_sections = Vec::new();
        if !valid_profile {
            invalid_sections.push(format!(
                "DBT_NOVA_TOOL_PROFILE: {} (expected one of: {})",
                self.tool_profile,
                MCP_TOOL_PROFILE_NAMES.join(", ")
            ));
        }
        if !invalid_allowlist.is_empty() {
            invalid_sections.push(format!(
                "DBT_NOVA_TOOL_ALLOWLIST: {}",
                invalid_allowlist.into_iter().collect::<Vec<_>>().join(", ")
            ));
        }
        if !invalid_denylist.is_empty() {
            invalid_sections.push(format!(
                "DBT_NOVA_TOOL_DENYLIST: {}",
                invalid_denylist.into_iter().collect::<Vec<_>>().join(", ")
            ));
        }

        let valid_tools = MCP_TOOL_NAMES.join(", ");
        Err(DbtNovaError::InvalidParams(format!(
            "invalid MCP tool names: {}. Tool names must be case-sensitive exact MCP tool names. Check valid tool names: {}",
            invalid_sections.join("; "),
            valid_tools
        )))
    }

    /// Validate configuration for conflicting or unsafe settings.
    ///
    /// # Errors
    ///
    /// Returns an error when an invalid configuration is detected.
    pub fn validate(&self) -> Result<()> {
        if !self.env_errors.is_empty() {
            return Err(DbtNovaError::InvalidParams(self.env_errors.join("; ")));
        }
        self.search.validate()?;

        if !self.manifest_allow_http && self.manifest_uri.trim().starts_with("http://") {
            return Err(DbtNovaError::InvalidParams(
                "manifest_allow_http=false but manifest_uri uses http://".to_string(),
            ));
        }

        if self.search.enable_vector_search && self.search.embedding_model.trim().is_empty() {
            return Err(DbtNovaError::InvalidParams(
                "vector search enabled but embedding_model is empty".to_string(),
            ));
        }
        if self.search.enable_reranker && self.search.reranker_model.trim().is_empty() {
            return Err(DbtNovaError::InvalidParams(
                "reranker enabled but reranker_model is empty".to_string(),
            ));
        }
        self.validate_result_profile_config()?;
        self.agent_modelling_audit.validate()?;
        self.hosted_auth.validate()?;

        if self.entity_cache_size == 0 {
            warn!("entity cache disabled (entity_cache_size=0)");
        }
        if self.search.lineage_cache_size == 0 {
            warn!("lineage cache disabled (search.lineage_cache_size=0)");
        } else if self.search.lineage_cache_size < 128 {
            warn!(
                lineage_cache_size = self.search.lineage_cache_size,
                "lineage cache size is very small; expect low hit rates"
            );
        }

        if self.remote_artifact_mode_enabled() {
            let has_storage = !self.storage_artifact_uri.trim().is_empty();
            let has_metadata = !self.metadata_artifact_uri.trim().is_empty();
            if !has_storage || !has_metadata {
                return Err(DbtNovaError::InvalidParams(
                    "remote artifact mode requires both DBT_NOVA_STORAGE_ARTIFACT_URI and DBT_NOVA_METADATA_ARTIFACT_URI"
                        .to_string(),
                ));
            }

            validate_artifact_uri(
                "DBT_NOVA_STORAGE_ARTIFACT_URI",
                &self.storage_artifact_uri,
                self.artifact_allow_http,
            )?;
            validate_artifact_uri(
                "DBT_NOVA_METADATA_ARTIFACT_URI",
                &self.metadata_artifact_uri,
                self.artifact_allow_http,
            )?;
            if !self.models_artifact_uri.trim().is_empty() {
                validate_artifact_uri(
                    "DBT_NOVA_MODELS_ARTIFACT_URI",
                    &self.models_artifact_uri,
                    self.artifact_allow_http,
                )?;
            }
        }

        if !self.bootstrap_uri.trim().is_empty() {
            validate_artifact_uri(
                "DBT_NOVA_BOOTSTRAP_URI",
                &self.bootstrap_uri,
                self.artifact_allow_http,
            )?;
        }

        if self.uses_home_storage_root_fallback()
            && (self.remote_artifact_mode_enabled() || !self.bootstrap_uri.trim().is_empty())
        {
            return Err(DbtNovaError::InvalidParams(
                "manifest_uri with bootstrap/remote-artifact mode requires an explicit non-HOME storage anchor; set DBT_NOVA_MANIFEST_CACHE_DIR or an absolute DBT_NOVA_STORAGE_DIR"
                    .to_string(),
            ));
        }

        if self.server_transport == ServerTransport::StreamableHttp {
            if self.http_host.trim().is_empty() {
                return Err(DbtNovaError::InvalidParams(
                    "streamable HTTP transport requires a non-empty http_host".to_string(),
                ));
            }
            let http_path = self.http_path.trim();
            if !http_path_is_literal_mount(http_path) {
                return Err(DbtNovaError::InvalidParams(
                    "streamable HTTP transport requires http_path to start with '/' and contain only literal path segments".to_string(),
                ));
            }
            if http_path_conflicts_with_probe_route(http_path) {
                return Err(DbtNovaError::InvalidParams(
                    "streamable HTTP transport reserves /healthz, /readyz, and /metrics for probe and metrics endpoints; choose a different http_path".to_string(),
                ));
            }
            if self.http_transport_binds_non_loopback() && !self.http_expect_auth_proxy {
                return Err(DbtNovaError::InvalidParams(
                    "streamable HTTP transport has no built-in authentication and is configured to listen on a non-loopback host. Bind to 127.0.0.1/::1 for local-only use, or set DBT_NOVA_HTTP_EXPECT_AUTH_PROXY=true only when an authenticating reverse proxy is enforcing access in front of dbt-nova; published container images do not set this acknowledgement by default.".to_string(),
                ));
            }
        }

        self.validate_tool_filters()?;

        Ok(())
    }

    fn validate_result_profile_config(&self) -> Result<()> {
        if self.mcp_default_limit == 0 {
            return Err(DbtNovaError::InvalidParams(
                "mcp_default_limit must be greater than 0".to_string(),
            ));
        }
        Ok(())
    }

    #[must_use]
    pub(crate) fn http_transport_binds_non_loopback(&self) -> bool {
        self.server_transport == ServerTransport::StreamableHttp
            && http_host_is_non_loopback(&self.http_host)
    }

    /// Resolve the base storage root directory.
    ///
    /// # Errors
    ///
    /// Returns an error when the storage directory is unsafe or invalid.
    pub fn storage_root_dir(&self) -> Result<PathBuf> {
        if self.storage_dir.trim().is_empty()
            || self.storage_dir == "."
            || self.storage_dir == "./"
            || self.storage_dir == "/"
        {
            return Err(DbtNovaError::ServerError(
                "Refusing to use storage directory; DBT_NOVA_STORAGE_DIR is unsafe".to_string(),
            ));
        }

        let storage_dir_path = Path::new(&self.storage_dir);
        if storage_dir_path
            .components()
            .any(|c| matches!(c, Component::ParentDir))
        {
            return Err(DbtNovaError::ServerError(
                "Refusing to use storage directory; DBT_NOVA_STORAGE_DIR contains '..'".to_string(),
            ));
        }

        let manifest_dir = {
            let manifest_path = PathBuf::from(&self.manifest_path);
            manifest_path
                .parent()
                .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
        };

        let storage_root = if self.manifest_uri.trim().is_empty() {
            manifest_dir.join(&self.storage_dir)
        } else if !self.manifest_cache_dir.trim().is_empty() {
            PathBuf::from(&self.manifest_cache_dir).join(&self.storage_dir)
        } else {
            std::env::var("HOME")
                .map_or_else(|_| PathBuf::from("."), PathBuf::from)
                .join(&self.storage_dir)
        };

        if self.manifest_uri.trim().is_empty() && storage_root == manifest_dir {
            return Err(DbtNovaError::ServerError(
                "Refusing to use storage directory; resolved to manifest directory".to_string(),
            ));
        }

        Ok(storage_root)
    }

    /// Path to the directory that stores indexed instances.
    ///
    /// # Errors
    ///
    /// Returns an error if the storage root cannot be resolved.
    pub fn storage_instances_dir(&self) -> Result<PathBuf> {
        Ok(self.storage_root_dir()?.join("instances"))
    }

    /// Path to the active storage instance root directory.
    ///
    /// # Errors
    ///
    /// Returns an error if the storage root cannot be resolved.
    pub fn storage_instance_root_dir(&self) -> Result<PathBuf> {
        Ok(self
            .storage_instances_dir()?
            .join(&self.storage_instance_id))
    }

    /// Path to the base storage directory for this instance/version.
    ///
    /// # Errors
    ///
    /// Returns an error if the storage instance id is unsafe.
    pub fn storage_base_dir(&self) -> Result<PathBuf> {
        let storage_root = self.storage_instances_dir()?;
        let instance_id = self.storage_instance_id.trim();
        if instance_id.is_empty()
            || instance_id.contains('/')
            || instance_id.contains('\\')
            || matches!(instance_id, "." | "..")
        {
            return Err(DbtNovaError::ServerError(
                "Refusing to use storage directory; storage instance id is unsafe".to_string(),
            ));
        }

        let mut components = Path::new(instance_id).components();
        match (components.next(), components.next()) {
            (Some(Component::Normal(_)), None) => {}
            _ => {
                return Err(DbtNovaError::ServerError(
                    "Refusing to use storage directory; storage instance id is unsafe".to_string(),
                ));
            }
        }

        Ok(storage_root.join(&self.storage_instance_id))
    }

    /// Build configuration from environment variables.
    #[must_use]
    pub fn from_env() -> Self {
        let mut config = Self::default();
        config.apply_runtime_preset_env();
        config.apply_manifest_env();
        config.apply_artifact_env();
        config.apply_storage_env();
        config.apply_server_env();
        config.apply_runtime_limits_env();
        config.apply_lineage_and_cache_env();
        config.apply_recipe_env();
        config.apply_governance_env();
        config.apply_agent_modelling_env();

        config.column_lineage = ColumnLineageConfig::from_env();
        config.search.apply_env();
        config.metadata_score = MetadataScoreConfig::from_env();

        config
    }

    fn apply_runtime_preset_env(&mut self) {
        let Some(value) = env_string("DBT_NOVA_PRESET") else {
            return;
        };
        if value.trim().is_empty() {
            return;
        }
        if let Some(preset) = RuntimePreset::parse(&value) {
            self.apply_runtime_preset(preset);
        } else {
            self.env_errors.push(format!(
                "Invalid DBT_NOVA_PRESET value '{value}'; expected local-dev|ci-audit|hosted-discovery|hosted-sql-trusted"
            ));
        }
    }

    fn apply_manifest_env(&mut self) {
        set_string("DBT_MANIFEST_PATH", &mut self.manifest_path);
        if let Some(value) = env_string("DBT_MANIFEST_PATH")
            && !value.trim().is_empty()
        {
            self.manifest_path_explicit = true;
        }
        set_string("DBT_NOVA_CATALOG_PATH", &mut self.catalog_path);
        set_string("DBT_NOVA_MANIFEST_URI", &mut self.manifest_uri);
        set_string("DBT_NOVA_MANIFEST_CACHE_DIR", &mut self.manifest_cache_dir);
        if let Some(v) = parse_u64("DBT_NOVA_MANIFEST_REFRESH_SECS") {
            self.manifest_refresh_secs = v;
        }
        if let Some(v) = parse_u64("DBT_NOVA_MANIFEST_MAX_BYTES") {
            self.manifest_max_bytes = v;
        }
        if let Some(v) = parse_u64("DBT_NOVA_MANIFEST_HTTP_CONNECT_TIMEOUT_SECS") {
            self.manifest_http_connect_timeout_secs = v;
        }
        if let Some(v) = parse_u64("DBT_NOVA_MANIFEST_HTTP_TIMEOUT_SECS") {
            self.manifest_http_timeout_secs = v;
        }
        if let Some(v) = parse_u64("DBT_NOVA_MANIFEST_FETCH_TIMEOUT_SECS") {
            self.manifest_fetch_timeout_secs = v;
        }
        if let Some(value) = parse_bool("DBT_NOVA_MANIFEST_ALLOW_HTTP") {
            self.manifest_allow_http = value;
        }
        if let Some(value) = env_string("DBT_NOVA_PRUNE_ALLOW_IDS") {
            match serde_json::from_str::<Vec<String>>(&value) {
                Ok(patterns) => self.manifest_prune_allow_ids = patterns,
                Err(err) => {
                    self.env_errors.push(format!(
                        "Invalid DBT_NOVA_PRUNE_ALLOW_IDS JSON; expected a JSON array of strings (error: {err})"
                    ));
                }
            }
        }
        if let Some(value) = env_string("DBT_NOVA_PRUNE_DENY_IDS") {
            match serde_json::from_str::<Vec<String>>(&value) {
                Ok(patterns) => self.manifest_prune_deny_ids = patterns,
                Err(err) => {
                    self.env_errors.push(format!(
                        "Invalid DBT_NOVA_PRUNE_DENY_IDS JSON; expected a JSON array of strings (error: {err})"
                    ));
                }
            }
        }
    }

    fn apply_artifact_env(&mut self) {
        set_string(
            "DBT_NOVA_STORAGE_ARTIFACT_URI",
            &mut self.storage_artifact_uri,
        );
        set_string(
            "DBT_NOVA_METADATA_ARTIFACT_URI",
            &mut self.metadata_artifact_uri,
        );
        set_string(
            "DBT_NOVA_MODELS_ARTIFACT_URI",
            &mut self.models_artifact_uri,
        );
        set_string(
            "DBT_NOVA_ARTIFACTS_CACHE_DIR",
            &mut self.artifacts_cache_dir,
        );
        set_string("DBT_NOVA_BOOTSTRAP_URI", &mut self.bootstrap_uri);
        if let Some(value) = env_string("DBT_NOVA_ARTIFACT_FETCH_POLICY") {
            if let Some(policy) = ArtifactFetchPolicy::parse(&value) {
                self.artifact_fetch_policy = policy;
            } else {
                warn!(
                    "Invalid DBT_NOVA_ARTIFACT_FETCH_POLICY value '{}'; expected if_missing|always|never",
                    value
                );
            }
        }
        if let Some(v) = parse_u64("DBT_NOVA_ARTIFACT_TIMEOUT_SECS") {
            self.artifact_timeout_secs = v;
        }
        if let Some(v) = parse_u64("DBT_NOVA_ARTIFACT_MAX_BYTES") {
            self.artifact_max_bytes = v;
        }
        if let Some(v) = parse_usize("DBT_NOVA_ARTIFACT_ARCHIVE_MAX_ENTRIES") {
            self.artifact_archive_max_entries = v;
        }
        if let Some(v) = parse_u64("DBT_NOVA_ARTIFACT_ARCHIVE_MAX_UNCOMPRESSED_BYTES") {
            self.artifact_archive_max_uncompressed_bytes = v;
        }
        if let Some(value) = parse_bool("DBT_NOVA_ARTIFACT_ALLOW_HTTP") {
            self.artifact_allow_http = value;
        }
    }

    fn apply_storage_env(&mut self) {
        set_string("DBT_NOVA_STORAGE_DIR", &mut self.storage_dir);
        set_string(
            "DBT_NOVA_STORAGE_INSTANCE_ID",
            &mut self.storage_instance_id,
        );
        if let Some(value) = parse_bool("DBT_NOVA_CLEANUP_STORAGE_ON_START") {
            self.cleanup_storage_on_start = value;
        }
        if let Some(v) = parse_usize("DBT_NOVA_STORAGE_MAX_INSTANCES") {
            self.storage_max_instances = v;
        }
        if let Some(v) = parse_usize("DBT_NOVA_STORAGE_MIN_VERSIONS") {
            self.storage_min_versions = v;
        }
        if let Some(v) = parse_u64("DBT_NOVA_STORAGE_MAX_BYTES") {
            self.storage_max_bytes = v;
        }
        if let Some(v) = parse_u64("DBT_NOVA_STORAGE_BUILD_LOCK_WAIT_SECS") {
            self.storage_build_lock_wait_secs = v;
        }
        if let Some(value) = parse_bool("DBT_NOVA_STORAGE_READ_ONLY") {
            self.storage_read_only = value;
        }
    }

    fn apply_server_env(&mut self) {
        if let Some(value) = env_string("DBT_NOVA_SERVER_TRANSPORT") {
            if let Some(transport) = ServerTransport::parse(&value) {
                self.server_transport = transport;
            } else {
                warn!(
                    "Invalid DBT_NOVA_SERVER_TRANSPORT value '{}'; expected stdio|streamable_http",
                    value
                );
            }
        }
        if let Some(value) = env_string("DBT_NOVA_LOG_FORMAT")
            && !value.trim().is_empty()
        {
            if let Some(log_format) = LogFormat::parse(&value) {
                self.log_format = log_format;
            } else {
                self.env_errors.push(format!(
                    "Invalid DBT_NOVA_LOG_FORMAT value '{value}'; expected human|json"
                ));
            }
        }

        let explicit_http_host =
            env_string("DBT_NOVA_HTTP_HOST").filter(|value| !value.trim().is_empty());
        if let Some(host) = explicit_http_host.as_ref() {
            self.http_host.clone_from(host);
        }
        let explicit_http_port = env_string("DBT_NOVA_HTTP_PORT");
        let parsed_http_port = explicit_http_port
            .as_ref()
            .and_then(|value| value.parse().ok());
        if let Some(port) = parsed_http_port {
            self.http_port = port;
        }
        if let Some(path) = env_string("DBT_NOVA_HTTP_PATH") {
            let trimmed = path.trim();
            if !trimmed.is_empty() {
                self.http_path = trimmed.to_string();
            }
        }
        if let Some(value) = parse_bool("DBT_NOVA_HTTP_EXPECT_AUTH_PROXY") {
            self.http_expect_auth_proxy = value;
        }
        if let Some(value) = env_string("DBT_NOVA_HTTP_ALLOWED_HOSTS") {
            self.http_allowed_hosts = value;
        }
        if let Some(value) = parse_bool("DBT_NOVA_HTTP_STATEFUL_MODE") {
            self.http_stateful_mode = value;
        }
        if let Some(value) = parse_u64("DBT_NOVA_HTTP_SSE_KEEP_ALIVE_SECS") {
            self.http_sse_keep_alive_secs = value;
        }
        if let Some(value) = parse_u64("DBT_NOVA_HTTP_SSE_RETRY_SECS") {
            self.http_sse_retry_secs = value;
        }
        if let Some(value) = parse_usize("DBT_NOVA_HTTP_MAX_BODY_BYTES") {
            self.http_max_body_bytes = value;
        }
        if let Some(value) = parse_bool("DBT_NOVA_METRICS_ENABLED") {
            self.metrics_enabled = value;
        }
        self.apply_hosted_auth_env();

        self.apply_http_platform_port_fallback(
            explicit_http_host.is_some(),
            parsed_http_port.is_some(),
            parse_u16("PORT"),
        );
    }

    fn apply_hosted_auth_env(&mut self) {
        if let Some(value) = env_string("DBT_NOVA_AUTH_MODE") {
            if let Some(mode) = HostedAuthMode::parse(&value) {
                self.hosted_auth.mode = mode;
                if mode != HostedAuthMode::Off && env_string("DBT_NOVA_AUTH_REQUIRED").is_none() {
                    self.hosted_auth.required = true;
                }
            } else {
                self.env_errors.push(format!(
                    "Invalid DBT_NOVA_AUTH_MODE value '{value}'; expected off|proxy_signed_headers|jwt"
                ));
            }
        }
        if let Some(value) = parse_bool("DBT_NOVA_AUTH_REQUIRED") {
            self.hosted_auth.required = value;
        }
        set_string(
            "DBT_NOVA_IDENTITY_SUBJECT_CLAIM",
            &mut self.hosted_auth.identity_subject_claim,
        );
        set_string(
            "DBT_NOVA_IDENTITY_EMAIL_CLAIM",
            &mut self.hosted_auth.identity_email_claim,
        );
        set_string(
            "DBT_NOVA_IDENTITY_NAME_CLAIM",
            &mut self.hosted_auth.identity_name_claim,
        );
        set_string(
            "DBT_NOVA_IDENTITY_GROUPS_CLAIM",
            &mut self.hosted_auth.identity_groups_claim,
        );
        set_string(
            "DBT_NOVA_PROXY_IDENTITY_HEADER",
            &mut self.hosted_auth.proxy_identity_header,
        );
        set_string(
            "DBT_NOVA_PROXY_SIGNATURE_HEADER",
            &mut self.hosted_auth.proxy_signature_header,
        );
        set_string(
            "DBT_NOVA_PROXY_IDENTITY_SECRET_FILE",
            &mut self.hosted_auth.proxy_identity_secret_file,
        );
        if let Some(value) = parse_u64("DBT_NOVA_PROXY_IDENTITY_MAX_AGE_SECS") {
            self.hosted_auth.proxy_identity_max_age_secs = value;
        }
        set_string("DBT_NOVA_JWT_ISSUER", &mut self.hosted_auth.jwt_issuer);
        set_string("DBT_NOVA_JWT_AUDIENCE", &mut self.hosted_auth.jwt_audience);
        set_string("DBT_NOVA_JWT_JWKS_URL", &mut self.hosted_auth.jwt_jwks_url);
        if let Some(value) = env_string("DBT_NOVA_JWT_ALGORITHMS") {
            self.hosted_auth.jwt_algorithms = parse_csv_values(&value);
        }
        if let Some(value) = parse_u64("DBT_NOVA_JWT_CLOCK_SKEW_SECS") {
            self.hosted_auth.jwt_clock_skew_secs = value;
        }
    }

    pub(crate) fn apply_http_platform_port_fallback(
        &mut self,
        explicit_http_host: bool,
        explicit_http_port: bool,
        platform_port: Option<u16>,
    ) {
        if self.server_transport != ServerTransport::StreamableHttp {
            return;
        }
        if platform_port.is_some() && !explicit_http_host {
            self.http_host = "0.0.0.0".to_string();
        }
        if explicit_http_port {
            return;
        }
        if let Some(port) = platform_port {
            self.http_port = port;
        }
    }

    fn apply_runtime_limits_env(&mut self) {
        if let Some(value) = env_string("DBT_NOVA_TOOL_RATE_LIMITS") {
            self.tool_rate_limits = value;
        }
        if let Some(v) = parse_u64("DBT_NOVA_TOOL_RATE_LIMIT_WINDOW_SECS")
            && v > 0
        {
            self.tool_rate_limit_window_secs = v;
        }
        if let Some(value) = env_string("DBT_NOVA_TOOL_ALLOWLIST") {
            self.tool_allowlist = value;
        }
        if let Some(value) = env_string("DBT_NOVA_TOOL_DENYLIST") {
            self.tool_denylist = value;
        }
        if let Some(value) = env_string("DBT_NOVA_TOOL_PROFILE") {
            self.tool_profile = value;
        }
        if let Some(value) = env_string("DBT_NOVA_RESULT_PROFILE") {
            if let Some(profile) = ResultProfile::parse(&value) {
                self.result_profile = profile;
            } else {
                self.env_errors.push(format!(
                    "Invalid DBT_NOVA_RESULT_PROFILE value '{value}'; expected compact|standard|full"
                ));
            }
        }
        if let Some(value) = env_string("DBT_NOVA_MCP_RESULT_PROFILE") {
            if let Some(profile) = ResultProfile::parse(&value) {
                self.mcp_result_profile = profile;
            } else {
                self.env_errors.push(format!(
                    "Invalid DBT_NOVA_MCP_RESULT_PROFILE value '{value}'; expected compact|standard|full"
                ));
            }
        }
        if let Some(v) = parse_usize("DBT_NOVA_MCP_DEFAULT_LIMIT")
            && v > 0
        {
            self.mcp_default_limit = v;
        }
        if let Some(v) = parse_usize("DBT_NOVA_MCP_MAX_PAGE_SIZE") {
            self.mcp_max_page_size = v;
        }
        if let Some(v) = parse_usize("DBT_NOVA_MCP_MAX_RESPONSE_BYTES") {
            self.mcp_max_response_bytes = v;
        }
        if let Some(v) = parse_usize("DBT_NOVA_MCP_MAX_STRING_CHARS")
            && v > 0
        {
            self.mcp_max_string_chars = v;
        }
        if let Some(v) = parse_bool("DBT_NOVA_MCP_INCLUDE_TRUNCATION_META") {
            self.mcp_include_truncation_meta = v;
        }
        set_string("DBT_NOVA_SQL_PROVIDER", &mut self.sql_provider);
        if let Some(v) = parse_u64("DBT_NOVA_SQL_MAX_ROW_LIMIT") {
            self.sql_max_row_limit = v;
        }
        if let Some(v) = parse_u64("DBT_NOVA_SQL_MAX_BYTE_LIMIT") {
            self.sql_max_byte_limit = v;
        }
        if let Some(v) = parse_usize("DBT_NOVA_SQL_MAX_CHUNKS") {
            self.sql_max_chunks = v;
        }
        if let Some(v) = parse_u64("DBT_NOVA_SQL_MAX_POLL_SECONDS") {
            self.sql_max_poll_seconds = v;
        }
        if let Some(v) = parse_u64("DBT_NOVA_SQL_MIN_POLL_INTERVAL_MS") {
            self.sql_min_poll_interval_ms = v;
        }
        if let Some(v) = parse_usize("DBT_NOVA_SQL_MAX_CONCURRENT") {
            self.sql_max_concurrent = v;
        }
        if let Some(v) = parse_usize("DBT_NOVA_SQL_MAX_QUEUE") {
            self.sql_max_queue = v;
        }
        if let Some(v) = parse_u64("DBT_NOVA_SQL_QUEUE_TIMEOUT_MS") {
            self.sql_queue_timeout_ms = v;
        }
    }

    fn apply_recipe_env(&mut self) {
        set_string("DBT_NOVA_RECIPES_DIR", &mut self.recipes_dir);
    }

    fn apply_lineage_and_cache_env(&mut self) {
        if let Some(v) = parse_usize("DBT_NOVA_ENTITY_CACHE_SIZE") {
            self.entity_cache_size = v;
        }

        if let Some(v) = parse_usize("DBT_NOVA_BATCH_GET_MAX_ITEMS") {
            self.batch_get_max_items = v;
        }

        if let Some(v) = parse_usize("DBT_NOVA_MAX_ENTITY_LINEAGE_RESULTS")
            && v > 0
        {
            self.lineage_max_results = v;
        }

        if let Some(v) = parse_usize("DBT_NOVA_MAX_LINEAGE_DEPTH")
            && v > 0
        {
            self.lineage_max_depth = v;
        }
    }

    fn apply_governance_env(&mut self) {
        if let Some(value) = env_string("DBT_NOVA_LAYER_RULES") {
            match serde_json::from_str::<Vec<LayerRule>>(&value) {
                Ok(rules) => self.layer_rules = rules,
                Err(err) => warn!("Invalid DBT_NOVA_LAYER_RULES JSON; ignoring (error: {err})"),
            }
        }

        if let Some(value) = env_string("DBT_NOVA_GOV_REQUIRED_FIELDS") {
            match serde_json::from_str::<HashMap<String, Vec<String>>>(&value) {
                Ok(policy) => self.governance_required_fields = policy,
                Err(err) => {
                    warn!("Invalid DBT_NOVA_GOV_REQUIRED_FIELDS JSON; ignoring (error: {err})");
                }
            }
        }

        if let Some(value) = env_string("DBT_NOVA_GOV_GATE_PROFILE") {
            if let Some(profile) = GovernanceGateConfig::profile(&value) {
                self.governance_gate = profile;
            } else {
                warn!(
                    "Invalid DBT_NOVA_GOV_GATE_PROFILE value '{}'; expected strict|standard|advisory",
                    value
                );
            }
        }

        if let Some(value) = env_string("DBT_NOVA_GOV_GATE_POLICY") {
            match serde_json::from_str::<GovernanceGateConfig>(&value) {
                Ok(policy) => self.governance_gate = policy,
                Err(err) => {
                    warn!("Invalid DBT_NOVA_GOV_GATE_POLICY JSON; ignoring (error: {err})");
                }
            }
        }

        if let Some(value) = parse_usize("DBT_NOVA_GOV_GATE_MIN_METADATA_SCORE") {
            if value <= 100 {
                #[allow(clippy::cast_possible_truncation)]
                {
                    self.governance_gate.min_metadata_score = value as u8;
                }
            } else {
                warn!(
                    "Invalid DBT_NOVA_GOV_GATE_MIN_METADATA_SCORE value {}; expected 0..=100",
                    value
                );
            }
        }

        if let Some(value) = parse_f64("DBT_NOVA_GOV_GATE_MIN_DOC_COVERAGE_PCT") {
            if (0.0..=100.0).contains(&value) {
                self.governance_gate.min_documentation_coverage_pct = value;
            } else {
                warn!(
                    "Invalid DBT_NOVA_GOV_GATE_MIN_DOC_COVERAGE_PCT value {}; expected 0..=100",
                    value
                );
            }
        }

        if let Some(value) = parse_bool("DBT_NOVA_GOV_GATE_REQUIRE_TESTS") {
            self.governance_gate.require_tests = value;
        }
        if let Some(value) = parse_bool("DBT_NOVA_GOV_GATE_REQUIRE_OWNER") {
            self.governance_gate.require_owner = value;
        }
        if let Some(value) = parse_bool("DBT_NOVA_GOV_GATE_REQUIRE_REQUIRED_FIELDS") {
            self.governance_gate.require_required_fields = value;
        }
        if let Some(value) = parse_bool("DBT_NOVA_GOV_GATE_REQUIRE_COMPLIANCE_FOR_PII") {
            self.governance_gate.require_compliance_for_pii = value;
        }
        if let Some(value) = parse_bool("DBT_NOVA_GOV_GATE_BLOCK_ON_FAILURE") {
            self.governance_gate.block_on_failure = value;
        }

        if let Some(value) = parse_u64("DBT_NOVA_PROVENANCE_STALE_AFTER_DAYS") {
            self.provenance_stale_after_days = value;
        }
    }

    fn apply_agent_modelling_env(&mut self) {
        if let Some(value) = parse_bool("DBT_NOVA_AGENT_MODELLING_AUDIT_ENABLED") {
            self.agent_modelling_audit.enabled = value;
        }
        if let Some(value) = parse_usize("DBT_NOVA_AGENT_MODELLING_MAX_FINDINGS") {
            self.agent_modelling_audit.max_findings = value;
        }
        if let Some(value) = parse_usize("DBT_NOVA_AGENT_MODELLING_TOO_MANY_PARENTS_THRESHOLD") {
            self.agent_modelling_audit.too_many_parents_threshold = value;
        }
        if let Some(value) = parse_usize("DBT_NOVA_AGENT_MODELLING_SOURCE_FANOUT_THRESHOLD") {
            self.agent_modelling_audit.source_fanout_threshold = value;
        }
        if let Some(value) = parse_bool("DBT_NOVA_AGENT_MODELLING_ENABLE_SQL_SHAPE_CHECKS") {
            self.agent_modelling_audit.enable_sql_shape_checks = value;
        }
        if let Some(value) = parse_usize("DBT_NOVA_AGENT_READINESS_MODELLING_MAX_BLOCKERS") {
            self.agent_readiness.modelling.max_blockers = value;
        }
        if let Some(value) = parse_usize("DBT_NOVA_AGENT_READINESS_MODELLING_MAX_HIGH") {
            self.agent_readiness.modelling.max_high = value;
        }
        if let Some(value) = parse_bool("DBT_NOVA_AGENT_READINESS_MODELLING_MAX_BLOCKERS_REQUIRED")
        {
            self.agent_readiness.modelling.max_blockers_required = value;
        }
        if let Some(value) = parse_bool("DBT_NOVA_AGENT_READINESS_MODELLING_MAX_HIGH_REQUIRED") {
            self.agent_readiness.modelling.max_high_required = value;
        }
    }
}

fn canonical_prune_patterns(patterns: &[String]) -> Vec<String> {
    let mut values: Vec<String> = patterns
        .iter()
        .filter_map(|pattern| {
            let trimmed = pattern.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        })
        .collect();
    values.sort();
    values
}

fn parse_tool_name_csv(raw: &str) -> Vec<String> {
    parse_csv_values(raw)
}

fn parse_csv_values(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn artifact_uri_scheme(uri: &str) -> String {
    let lower = uri.trim().to_ascii_lowercase();
    if lower.starts_with("dbfs:/") && !lower.starts_with("dbfs://") {
        return "dbfs".to_string();
    }
    if let Some((scheme, _)) = lower.split_once("://") {
        return scheme.to_string();
    }
    "file".to_string()
}

fn validate_artifact_uri(name: &str, uri: &str, allow_http: bool) -> Result<()> {
    let trimmed = uri.trim();
    if trimmed.is_empty() {
        return Err(DbtNovaError::InvalidParams(format!(
            "{name} cannot be empty"
        )));
    }

    let scheme = artifact_uri_scheme(trimmed);
    match scheme.as_str() {
        "file" | "https" | "http" | "dbfs" | "s3" | "gs" => {}
        _ => {
            return Err(DbtNovaError::InvalidParams(format!(
                "{name} has unsupported URI scheme '{scheme}' (supported: file,https,http,dbfs,s3,gs)"
            )));
        }
    }

    if scheme == "http" && !allow_http {
        return Err(DbtNovaError::InvalidParams(format!(
            "{name} uses http:// but DBT_NOVA_ARTIFACT_ALLOW_HTTP=false"
        )));
    }

    Ok(())
}

fn http_path_is_literal_mount(path: &str) -> bool {
    if path.is_empty() || !path.starts_with('/') {
        return false;
    }
    if path == "/" {
        return true;
    }
    !path.contains('{') && !path.contains('}') && !path.contains('*') && !path.contains(':')
}

fn http_path_conflicts_with_probe_route(path: &str) -> bool {
    matches!(path, "/healthz" | "/readyz" | "/metrics")
}

fn http_host_is_non_loopback(host: &str) -> bool {
    let normalized = host.trim().trim_start_matches('[').trim_end_matches(']');
    if normalized.is_empty() {
        return false;
    }
    if normalized.eq_ignore_ascii_case("localhost") {
        return false;
    }
    normalized
        .parse::<std::net::IpAddr>()
        .map_or(true, |ip| !ip.is_loopback())
}

#[cfg(test)]
mod tests;
