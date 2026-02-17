use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

use blake3;

use crate::error::{DbtNovaError, Result};

use super::column_lineage::ColumnLineageConfig;
use super::metadata_score::MetadataScoreConfig;
use super::search::SearchConfig;
use super::warehouse::DEFAULT_SQL_PROVIDER;
use super::{env_string, parse_bool, parse_f64, parse_u64, parse_usize, set_string};
use tracing::warn;

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

/// Configuration for the dbt-nova server.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct DbtNovaConfig {
    /// Path to the dbt `manifest.json` file
    pub manifest_path: String,
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
    /// Per-tool rate limits (comma-separated, e.g. "`search=60,execute_sql=30,default=120`")
    pub tool_rate_limits: String,
    /// Rate limit window size in seconds
    pub tool_rate_limit_window_secs: u64,
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
}

impl Default for DbtNovaConfig {
    fn default() -> Self {
        Self {
            manifest_path: "manifest.json".to_string(),
            manifest_uri: String::new(),
            manifest_cache_dir: String::new(),
            recipes_dir: "analyses/recipes".to_string(),
            manifest_refresh_secs: 300,
            manifest_max_bytes: 256 * 1024 * 1024, // 256 MiB
            manifest_http_connect_timeout_secs: 10,
            manifest_http_timeout_secs: 120,
            manifest_fetch_timeout_secs: 300,
            manifest_allow_http: false,
            storage_dir: ".dbt-nova".to_string(),
            storage_instance_id: String::new(),
            cleanup_storage_on_start: false,
            storage_max_instances: 3,
            storage_min_versions: 2,
            storage_max_bytes: 5 * 1024 * 1024 * 1024, // 5 GiB
            storage_build_lock_wait_secs: 300,
            storage_read_only: false,
            tool_rate_limits: "search=60,execute_sql=20,default=120".to_string(),
            tool_rate_limit_window_secs: 60,
            sql_provider: DEFAULT_SQL_PROVIDER.to_string(),
            sql_max_row_limit: 10_000,
            sql_max_byte_limit: 25_000_000,
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
                return;
            }
        }
        if let Ok(storage_root) = self.storage_root_dir() {
            self.search.embedding_cache_dir = storage_root
                .join(".fastembed_cache")
                .to_string_lossy()
                .to_string();
        }
    }

    /// Validate configuration for conflicting or unsafe settings.
    ///
    /// # Errors
    ///
    /// Returns an error when an invalid configuration is detected.
    pub fn validate(&self) -> Result<()> {
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

        Ok(())
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
        config.apply_manifest_env();
        config.apply_storage_env();
        config.apply_runtime_limits_env();
        config.apply_lineage_and_cache_env();
        config.apply_recipe_env();
        config.apply_governance_env();

        config.column_lineage = ColumnLineageConfig::from_env();
        config.search = SearchConfig::from_env();
        config.metadata_score = MetadataScoreConfig::from_env();

        config
    }

    fn apply_manifest_env(&mut self) {
        set_string("DBT_MANIFEST_PATH", &mut self.manifest_path);
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

    fn apply_runtime_limits_env(&mut self) {
        if let Some(value) = env_string("DBT_NOVA_TOOL_RATE_LIMITS") {
            self.tool_rate_limits = value;
        }
        if let Some(v) = parse_u64("DBT_NOVA_TOOL_RATE_LIMIT_WINDOW_SECS")
            && v > 0
        {
            self.tool_rate_limit_window_secs = v;
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
    }
}
