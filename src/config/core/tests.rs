use std::sync::{LazyLock, Mutex};

use super::{
    ArtifactFetchPolicy, CI_AUDIT_TOOL_DENYLIST, DbtNovaConfig, HOSTED_DISCOVERY_TOOL_DENYLIST,
    HOSTED_SQL_TRUSTED_TOOL_DENYLIST, HostedAuthMode, ResultProfile, RuntimePreset,
    ServerTransport,
};
use crate::logging::LogFormat;

static ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

fn base_config() -> DbtNovaConfig {
    DbtNovaConfig {
        manifest_uri: "file:///tmp/manifest.json".to_string(),
        ..DbtNovaConfig::default()
    }
}

fn with_env_vars<R>(vars: &[(&str, Option<&str>)], run: impl FnOnce() -> R) -> R {
    let _guard = ENV_LOCK.lock().expect("env lock");
    let previous = vars
        .iter()
        .map(|(key, _)| (*key, std::env::var(key).ok()))
        .collect::<Vec<_>>();
    for (key, value) in vars {
        match value {
            Some(value) => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::set_var(key, value) };
            }
            None => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::remove_var(key) };
            }
        }
    }

    let result = run();

    for (key, value) in previous {
        match value {
            Some(value) => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::set_var(key, value) };
            }
            None => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::remove_var(key) };
            }
        }
    }

    result
}

#[test]
fn artifact_fetch_policy_parse_is_case_insensitive() {
    assert_eq!(
        ArtifactFetchPolicy::parse("if_missing"),
        Some(ArtifactFetchPolicy::IfMissing)
    );
    assert_eq!(
        ArtifactFetchPolicy::parse("Always"),
        Some(ArtifactFetchPolicy::Always)
    );
    assert_eq!(
        ArtifactFetchPolicy::parse("NEVER"),
        Some(ArtifactFetchPolicy::Never)
    );
    assert_eq!(ArtifactFetchPolicy::parse("unknown"), None);
}

#[test]
fn server_transport_parse_is_case_insensitive() {
    assert_eq!(
        ServerTransport::parse("stdio"),
        Some(ServerTransport::Stdio)
    );
    assert_eq!(
        ServerTransport::parse("streamable_http"),
        Some(ServerTransport::StreamableHttp)
    );
    assert_eq!(
        ServerTransport::parse("streamable-http"),
        Some(ServerTransport::StreamableHttp)
    );
    assert_eq!(
        ServerTransport::parse("http"),
        Some(ServerTransport::StreamableHttp)
    );
    assert_eq!(ServerTransport::parse("unknown"), None);
}

#[test]
fn hosted_auth_mode_parse_accepts_planned_modes() {
    assert_eq!(HostedAuthMode::parse("off"), Some(HostedAuthMode::Off));
    assert_eq!(
        HostedAuthMode::parse("proxy_signed_headers"),
        Some(HostedAuthMode::ProxySignedHeaders)
    );
    assert_eq!(
        HostedAuthMode::parse("proxy-signed-headers"),
        Some(HostedAuthMode::ProxySignedHeaders)
    );
    assert_eq!(HostedAuthMode::parse("jwt"), Some(HostedAuthMode::Jwt));
    assert_eq!(HostedAuthMode::parse("tenant_router"), None);
}

#[test]
fn runtime_preset_parse_accepts_documented_names() {
    assert_eq!(
        RuntimePreset::parse("local-dev"),
        Some(RuntimePreset::LocalDev)
    );
    assert_eq!(
        RuntimePreset::parse("ci_audit"),
        Some(RuntimePreset::CiAudit)
    );
    assert_eq!(
        RuntimePreset::parse("hosted-discovery"),
        Some(RuntimePreset::HostedDiscovery)
    );
    assert_eq!(
        RuntimePreset::parse("hosted_sql_trusted"),
        Some(RuntimePreset::HostedSqlTrusted)
    );
    assert_eq!(RuntimePreset::parse("semantic-layer"), None);
}

#[test]
fn runtime_presets_apply_conservative_metadata_bridge_postures() {
    let mut config = DbtNovaConfig::default();
    config.apply_runtime_preset(RuntimePreset::CiAudit);
    assert_eq!(config.runtime_preset, RuntimePreset::CiAudit);
    assert!(!config.search.enable_vector_search);
    assert!(!config.search.enable_sparse_search);
    assert!(!config.search.enable_reranker);
    assert_eq!(
        config.parsed_tool_denylist(),
        CI_AUDIT_TOOL_DENYLIST
            .iter()
            .map(|tool| (*tool).to_string())
            .collect::<Vec<_>>()
    );

    let mut config = DbtNovaConfig::default();
    config.apply_runtime_preset(RuntimePreset::HostedDiscovery);
    assert_eq!(config.server_transport, ServerTransport::StreamableHttp);
    assert_eq!(config.tool_profile, "agent");
    assert!(
        config
            .parsed_tool_denylist()
            .contains(&"execute_sql".to_string())
    );
    assert!(
        config
            .parsed_tool_denylist()
            .contains(&"validate_config".to_string())
    );
    assert_eq!(
        config.parsed_tool_denylist().len(),
        HOSTED_DISCOVERY_TOOL_DENYLIST.len()
    );

    let mut config = DbtNovaConfig::default();
    config.apply_runtime_preset(RuntimePreset::HostedSqlTrusted);
    let denylist = config.parsed_tool_denylist();
    assert_eq!(config.server_transport, ServerTransport::StreamableHttp);
    assert_eq!(config.tool_profile, "analyst");
    assert!(!denylist.contains(&"execute_sql".to_string()));
    assert!(denylist.contains(&"run_recipe".to_string()));
    assert_eq!(denylist.len(), HOSTED_SQL_TRUSTED_TOOL_DENYLIST.len());
}

#[test]
fn from_env_applies_preset_before_env_overrides() {
    let config = with_env_vars(
        &[
            ("DBT_NOVA_PRESET", Some("ci-audit")),
            ("DBT_NOVA_SEARCH_ENABLE_VECTOR", Some("true")),
            ("DBT_NOVA_SEARCH_ENABLE_SPARSE", Some("true")),
            ("DBT_NOVA_TOOL_PROFILE", Some("all")),
            ("DBT_NOVA_TOOL_DENYLIST", Some("")),
        ],
        DbtNovaConfig::from_env,
    );

    assert_eq!(config.runtime_preset, RuntimePreset::CiAudit);
    assert!(config.search.enable_vector_search);
    assert!(config.search.enable_sparse_search);
    assert!(!config.search.enable_reranker);
    assert_eq!(config.tool_profile, "all");
    assert!(config.parsed_tool_denylist().is_empty());
}

#[test]
fn from_env_records_invalid_runtime_preset() {
    let config = with_env_vars(
        &[("DBT_NOVA_PRESET", Some("semantic-layer"))],
        DbtNovaConfig::from_env,
    );

    let error = config
        .validate()
        .expect_err("invalid preset should fail validation");
    assert!(error.to_string().contains("DBT_NOVA_PRESET"));
}

#[test]
fn default_result_profiles_keep_cli_standard_and_mcp_compact() {
    let config = DbtNovaConfig::default();

    assert_eq!(config.result_profile, ResultProfile::Standard);
    assert_eq!(config.mcp_result_profile, ResultProfile::Compact);
    assert_eq!(config.mcp_default_limit, 10);
    assert_eq!(config.mcp_max_page_size, 100);
    assert_eq!(config.log_format, LogFormat::Human);
    assert_eq!(config.hosted_auth.mode, HostedAuthMode::Off);
    assert!(!config.hosted_auth.required);
    assert_eq!(config.hosted_auth.identity_subject_claim, "sub");
    assert!(config.hosted_auth.jwt_algorithms.is_empty());
}

#[test]
fn from_env_reads_json_log_format() {
    let config = with_env_vars(
        &[("DBT_NOVA_LOG_FORMAT", Some("json"))],
        DbtNovaConfig::from_env,
    );

    assert_eq!(config.log_format, LogFormat::Json);
    config.validate().expect("json log format should validate");
}

#[test]
fn from_env_records_invalid_log_format() {
    let config = with_env_vars(
        &[("DBT_NOVA_LOG_FORMAT", Some("xml"))],
        DbtNovaConfig::from_env,
    );

    let error = config
        .validate()
        .expect_err("invalid log format should fail validation");
    assert!(error.to_string().contains("DBT_NOVA_LOG_FORMAT"));
}

#[test]
fn from_env_rejects_unknown_hosted_auth_mode() {
    let config = with_env_vars(
        &[("DBT_NOVA_AUTH_MODE", Some("tenant_router"))],
        DbtNovaConfig::from_env,
    );

    let error = config
        .validate()
        .expect_err("unknown hosted auth mode should fail validation");
    assert!(error.to_string().contains("DBT_NOVA_AUTH_MODE"));
}

#[test]
fn from_env_defaults_non_off_hosted_auth_to_required() {
    let config = with_env_vars(
        &[
            ("DBT_NOVA_AUTH_MODE", Some("proxy_signed_headers")),
            ("DBT_NOVA_PROXY_IDENTITY_HEADER", Some("X-Nova-Identity")),
            ("DBT_NOVA_PROXY_SIGNATURE_HEADER", Some("X-Nova-Signature")),
            (
                "DBT_NOVA_PROXY_IDENTITY_SECRET_FILE",
                Some("/run/secrets/nova-proxy-key"),
            ),
        ],
        DbtNovaConfig::from_env,
    );

    assert_eq!(config.hosted_auth.mode, HostedAuthMode::ProxySignedHeaders);
    assert!(config.hosted_auth.required);
    config
        .validate()
        .expect("complete proxy-signed header mode should validate");
}

#[test]
fn validate_rejects_incomplete_proxy_signed_header_mode() {
    let config = with_env_vars(
        &[("DBT_NOVA_AUTH_MODE", Some("proxy_signed_headers"))],
        DbtNovaConfig::from_env,
    );

    let error = config
        .validate()
        .expect_err("incomplete proxy mode should fail validation");
    assert!(
        error.to_string().contains("DBT_NOVA_PROXY_IDENTITY_HEADER"),
        "unexpected error: {error}"
    );
}

#[test]
fn validate_rejects_incomplete_jwt_hosted_auth_skeleton() {
    let config = with_env_vars(
        &[("DBT_NOVA_AUTH_MODE", Some("jwt"))],
        DbtNovaConfig::from_env,
    );

    let error = config
        .validate()
        .expect_err("incomplete jwt mode should fail validation");
    assert!(
        error.to_string().contains("DBT_NOVA_JWT_ISSUER"),
        "unexpected error: {error}"
    );
}

#[test]
fn validate_rejects_complete_jwt_until_verifier_lands() {
    let config = with_env_vars(
        &[
            ("DBT_NOVA_AUTH_MODE", Some("jwt")),
            ("DBT_NOVA_IDENTITY_SUBJECT_CLAIM", Some("sub")),
            ("DBT_NOVA_JWT_ISSUER", Some("https://issuer.example")),
            ("DBT_NOVA_JWT_AUDIENCE", Some("dbt-nova")),
            (
                "DBT_NOVA_JWT_JWKS_URL",
                Some("https://issuer.example/.well-known/jwks.json"),
            ),
            ("DBT_NOVA_JWT_ALGORITHMS", Some("RS256, ES256")),
        ],
        DbtNovaConfig::from_env,
    );

    assert_eq!(config.hosted_auth.mode, HostedAuthMode::Jwt);
    assert_eq!(
        config.hosted_auth.jwt_algorithms,
        vec!["RS256".to_string(), "ES256".to_string()]
    );
    let error = config
        .validate()
        .expect_err("complete jwt skeleton should still fail closed until implemented");
    assert!(
        error
            .to_string()
            .contains("jwt is parsed but not implemented yet"),
        "unexpected error: {error}"
    );
}

#[test]
fn validate_rejects_non_https_jwt_jwks_url() {
    let config = with_env_vars(
        &[
            ("DBT_NOVA_AUTH_MODE", Some("jwt")),
            ("DBT_NOVA_AUTH_REQUIRED", Some("true")),
            ("DBT_NOVA_JWT_ISSUER", Some("https://issuer.example")),
            ("DBT_NOVA_JWT_AUDIENCE", Some("dbt-nova")),
            (
                "DBT_NOVA_JWT_JWKS_URL",
                Some("http://issuer.example/.well-known/jwks.json"),
            ),
            ("DBT_NOVA_JWT_ALGORITHMS", Some("RS256")),
        ],
        DbtNovaConfig::from_env,
    );

    let error = config
        .validate()
        .expect_err("http JWKS URL should be rejected");
    assert!(
        error
            .to_string()
            .contains("DBT_NOVA_JWT_JWKS_URL must start with https://"),
        "unexpected error: {error}"
    );
}

#[test]
fn validate_rejects_required_auth_when_mode_is_off() {
    let config = with_env_vars(
        &[("DBT_NOVA_AUTH_REQUIRED", Some("true"))],
        DbtNovaConfig::from_env,
    );

    let error = config
        .validate()
        .expect_err("auth_required=true with off mode should fail closed");
    assert!(
        error
            .to_string()
            .contains("DBT_NOVA_AUTH_REQUIRED=true requires DBT_NOVA_AUTH_MODE"),
        "unexpected error: {error}"
    );
}

#[test]
fn default_agent_modelling_audit_config_is_conservative() {
    let config = DbtNovaConfig::default();

    assert!(config.agent_modelling_audit.enabled);
    assert_eq!(config.agent_modelling_audit.max_findings, 100);
    assert_eq!(config.agent_modelling_audit.too_many_parents_threshold, 7);
    assert_eq!(config.agent_modelling_audit.source_fanout_threshold, 20);
    assert!(!config.agent_modelling_audit.enable_sql_shape_checks);
    assert_eq!(config.agent_readiness.modelling.max_blockers, 0);
    assert_eq!(config.agent_readiness.modelling.max_high, 10);
    assert!(config.agent_readiness.modelling.max_blockers_required);
    assert!(!config.agent_readiness.modelling.max_high_required);
}

#[test]
fn from_env_reads_agent_modelling_audit_settings() {
    let _guard = ENV_LOCK.lock().expect("env lock");
    let vars = [
        ("DBT_NOVA_AGENT_MODELLING_AUDIT_ENABLED", Some("false")),
        ("DBT_NOVA_AGENT_MODELLING_MAX_FINDINGS", Some("25")),
        (
            "DBT_NOVA_AGENT_MODELLING_TOO_MANY_PARENTS_THRESHOLD",
            Some("11"),
        ),
        (
            "DBT_NOVA_AGENT_MODELLING_SOURCE_FANOUT_THRESHOLD",
            Some("40"),
        ),
        (
            "DBT_NOVA_AGENT_MODELLING_ENABLE_SQL_SHAPE_CHECKS",
            Some("true"),
        ),
        ("DBT_NOVA_AGENT_READINESS_MODELLING_MAX_BLOCKERS", Some("2")),
        ("DBT_NOVA_AGENT_READINESS_MODELLING_MAX_HIGH", Some("12")),
        (
            "DBT_NOVA_AGENT_READINESS_MODELLING_MAX_BLOCKERS_REQUIRED",
            Some("false"),
        ),
        (
            "DBT_NOVA_AGENT_READINESS_MODELLING_MAX_HIGH_REQUIRED",
            Some("true"),
        ),
    ];
    let previous = vars.map(|(key, _)| (key, std::env::var(key).ok()));
    for (key, value) in vars {
        match value {
            Some(value) => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::set_var(key, value) };
            }
            None => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::remove_var(key) };
            }
        }
    }

    let config = DbtNovaConfig::from_env();

    for (key, value) in previous {
        match value {
            Some(value) => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::set_var(key, value) };
            }
            None => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::remove_var(key) };
            }
        }
    }

    assert!(!config.agent_modelling_audit.enabled);
    assert_eq!(config.agent_modelling_audit.max_findings, 25);
    assert_eq!(config.agent_modelling_audit.too_many_parents_threshold, 11);
    assert_eq!(config.agent_modelling_audit.source_fanout_threshold, 40);
    assert!(config.agent_modelling_audit.enable_sql_shape_checks);
    assert_eq!(config.agent_readiness.modelling.max_blockers, 2);
    assert_eq!(config.agent_readiness.modelling.max_high, 12);
    assert!(!config.agent_readiness.modelling.max_blockers_required);
    assert!(config.agent_readiness.modelling.max_high_required);
}

#[test]
fn validate_rejects_invalid_agent_modelling_audit_thresholds() {
    let mut config = base_config();
    config.agent_modelling_audit.max_findings = 0;
    let error = config
        .validate()
        .expect_err("zero max findings should fail validation");
    assert!(
        error
            .to_string()
            .contains("agent_modelling_audit.max_findings")
    );

    let mut config = base_config();
    config.agent_modelling_audit.too_many_parents_threshold = 0;
    let error = config
        .validate()
        .expect_err("zero parent threshold should fail validation");
    assert!(
        error
            .to_string()
            .contains("agent_modelling_audit.too_many_parents_threshold")
    );

    let mut config = base_config();
    config.agent_modelling_audit.source_fanout_threshold = 0;
    let error = config
        .validate()
        .expect_err("zero source fanout threshold should fail validation");
    assert!(
        error
            .to_string()
            .contains("agent_modelling_audit.source_fanout_threshold")
    );
}

#[test]
fn from_env_reads_provenance_stale_after_days() {
    let _guard = ENV_LOCK.lock().expect("env lock");
    let key = "DBT_NOVA_PROVENANCE_STALE_AFTER_DAYS";
    let previous = std::env::var(key).ok();
    // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
    unsafe { std::env::set_var(key, "7") };

    let config = DbtNovaConfig::from_env();

    match previous {
        Some(value) => {
            // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
            unsafe { std::env::set_var(key, value) };
        }
        None => {
            // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
            unsafe { std::env::remove_var(key) };
        }
    }

    assert_eq!(config.provenance_stale_after_days, 7);
}

#[test]
fn validate_rejects_zero_mcp_default_limit() {
    let mut config = base_config();
    config.mcp_default_limit = 0;

    let error = config
        .validate()
        .expect_err("zero MCP default limit should fail validation");
    assert!(error.to_string().contains("mcp_default_limit"));
}

#[test]
fn validate_rejects_incomplete_remote_artifact_configuration() {
    let mut config = base_config();
    config.storage_artifact_uri = "s3://bucket/storage.tar.gz".to_string();

    let error = config
        .validate()
        .expect_err("incomplete remote artifact config should fail");
    let message = error.to_string();
    assert!(message.contains("DBT_NOVA_STORAGE_ARTIFACT_URI"));
    assert!(message.contains("DBT_NOVA_METADATA_ARTIFACT_URI"));
}

#[test]
fn validate_rejects_unsupported_remote_artifact_scheme() {
    let mut config = base_config();
    config.storage_artifact_uri = "ftp://bucket/storage.tar.gz".to_string();
    config.metadata_artifact_uri = "file:///tmp/nova-build-metadata.json".to_string();

    let error = config
        .validate()
        .expect_err("unsupported scheme should fail validation");
    assert!(
        error.to_string().contains("unsupported URI scheme 'ftp'"),
        "error should include unsupported scheme"
    );
}

#[test]
fn validate_rejects_http_artifact_uri_when_disabled() {
    let mut config = base_config();
    config.storage_artifact_uri = "http://example.com/storage.tar.gz".to_string();
    config.metadata_artifact_uri = "https://example.com/nova-build-metadata.json".to_string();

    let error = config
        .validate()
        .expect_err("http artifact URI should fail by default");
    assert!(
        error
            .to_string()
            .contains("DBT_NOVA_ARTIFACT_ALLOW_HTTP=false")
    );
}

#[test]
fn validate_rejects_unsupported_bootstrap_scheme() {
    let mut config = base_config();
    config.bootstrap_uri = "ftp://bucket/nova-bootstrap.json".to_string();

    let error = config
        .validate()
        .expect_err("unsupported bootstrap scheme should fail validation");
    assert!(
        error.to_string().contains("DBT_NOVA_BOOTSTRAP_URI"),
        "error should mention bootstrap var"
    );
}

#[test]
fn validate_rejects_http_bootstrap_uri_when_disabled() {
    let mut config = base_config();
    config.bootstrap_uri = "http://example.com/nova-bootstrap.json".to_string();

    let error = config
        .validate()
        .expect_err("http bootstrap URI should fail by default");
    assert!(
        error
            .to_string()
            .contains("DBT_NOVA_ARTIFACT_ALLOW_HTTP=false")
    );
}

#[test]
fn validate_accepts_remote_artifact_uris_when_http_enabled() {
    let mut config = base_config();
    config.storage_artifact_uri = "http://example.com/storage.tar.gz".to_string();
    config.metadata_artifact_uri = "https://example.com/nova-build-metadata.json".to_string();
    config.models_artifact_uri = "dbfs:/FileStore/models.tar.gz".to_string();
    config.artifact_allow_http = true;
    config.storage_dir = "/tmp/nova-storage".to_string();

    config
        .validate()
        .expect("remote artifact config should validate");
}

#[test]
fn artifacts_cache_dir_defaults_under_storage_root() {
    let config = base_config();
    let path = config
        .artifacts_cache_dir()
        .expect("artifacts cache directory should resolve");
    assert!(
        path.ends_with(".dbt-nova/artifacts"),
        "unexpected artifacts cache path: {}",
        path.display()
    );
}

#[test]
fn uses_home_storage_root_fallback_for_manifest_uri_without_cache_dir() {
    let config = base_config();
    assert!(config.uses_home_storage_root_fallback());
}

#[test]
fn does_not_use_home_storage_root_fallback_when_manifest_cache_dir_is_set() {
    let config = DbtNovaConfig {
        manifest_uri: "file:///tmp/manifest.json".to_string(),
        manifest_cache_dir: "/tmp/nova-manifests".to_string(),
        ..DbtNovaConfig::default()
    };
    assert!(!config.uses_home_storage_root_fallback());
}

#[test]
fn does_not_use_home_storage_root_fallback_when_storage_dir_is_absolute() {
    let config = DbtNovaConfig {
        manifest_uri: "file:///tmp/manifest.json".to_string(),
        storage_dir: "/tmp/nova-storage".to_string(),
        ..DbtNovaConfig::default()
    };
    assert!(!config.uses_home_storage_root_fallback());
}

#[test]
fn validate_rejects_home_storage_root_fallback_for_remote_artifact_mode() {
    let mut config = base_config();
    config.storage_artifact_uri = "file:///tmp/storage.tar.gz".to_string();
    config.metadata_artifact_uri = "file:///tmp/metadata.json".to_string();

    let error = config
        .validate()
        .expect_err("implicit home storage fallback should be rejected");
    assert!(error.to_string().contains("DBT_NOVA_MANIFEST_CACHE_DIR"));
    assert!(error.to_string().contains("DBT_NOVA_STORAGE_DIR"));
}

#[test]
fn validate_rejects_http_transport_without_rooted_path() {
    let mut config = base_config();
    config.server_transport = ServerTransport::StreamableHttp;
    config.http_path = "mcp".to_string();

    let error = config
        .validate()
        .expect_err("non-rooted HTTP path should fail validation");
    assert!(error.to_string().contains("http_path"));
}

#[test]
fn validate_rejects_http_transport_with_wildcard_path() {
    let mut config = base_config();
    config.server_transport = ServerTransport::StreamableHttp;
    config.http_path = "/{*rest}".to_string();

    let error = config
        .validate()
        .expect_err("wildcard HTTP path should fail validation");
    assert!(error.to_string().contains("literal path segments"));
}

#[test]
fn validate_rejects_http_transport_with_probe_path_collision() {
    for path in ["/healthz", "/readyz", "/metrics"] {
        let mut config = base_config();
        config.server_transport = ServerTransport::StreamableHttp;
        config.http_path = path.to_string();

        let error = config
            .validate()
            .expect_err("probe path collisions should fail validation");
        assert!(
            error
                .to_string()
                .contains("reserves /healthz, /readyz, and /metrics"),
            "unexpected error for {path}: {error}"
        );
    }
}

#[test]
fn validate_rejects_exposed_http_transport_without_auth_proxy_ack() {
    let mut config = base_config();
    config.server_transport = ServerTransport::StreamableHttp;
    config.http_host = "0.0.0.0".to_string();

    let error = config
        .validate()
        .expect_err("public HTTP bind without auth proxy acknowledgement should fail");
    assert!(
        error
            .to_string()
            .contains("DBT_NOVA_HTTP_EXPECT_AUTH_PROXY=true")
    );
}

#[test]
fn validate_rejects_exposed_proxy_identity_mode_without_auth_proxy_ack() {
    let mut config = base_config();
    config.server_transport = ServerTransport::StreamableHttp;
    config.http_host = "0.0.0.0".to_string();
    config.hosted_auth.mode = HostedAuthMode::ProxySignedHeaders;
    config.hosted_auth.required = true;
    config.hosted_auth.proxy_identity_header = "X-Nova-Identity".to_string();
    config.hosted_auth.proxy_signature_header = "X-Nova-Signature".to_string();
    config.hosted_auth.proxy_identity_secret_file = "/run/secrets/nova-proxy-key".to_string();

    let error = config
        .validate()
        .expect_err("proxy identity mode still needs auth proxy acknowledgement");
    assert!(
        error
            .to_string()
            .contains("DBT_NOVA_HTTP_EXPECT_AUTH_PROXY=true")
    );
}

#[test]
fn validate_accepts_exposed_http_transport_with_auth_proxy_ack() {
    let mut config = base_config();
    config.server_transport = ServerTransport::StreamableHttp;
    config.http_host = "0.0.0.0".to_string();
    config.http_expect_auth_proxy = true;

    config
        .validate()
        .expect("public HTTP bind should validate when auth proxy is acknowledged");
}

#[test]
fn from_env_trims_http_path() {
    let _guard = ENV_LOCK.lock().expect("env lock");
    let vars = [("DBT_NOVA_HTTP_PATH", Some(" /mcp "))];
    let previous = vars.map(|(key, _)| (key, std::env::var(key).ok()));
    for (key, value) in vars {
        match value {
            Some(value) => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::set_var(key, value) };
            }
            None => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::remove_var(key) };
            }
        }
    }

    let config = DbtNovaConfig::from_env();

    for (key, value) in previous {
        match value {
            Some(value) => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::set_var(key, value) };
            }
            None => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::remove_var(key) };
            }
        }
    }

    assert_eq!(config.http_path, "/mcp");
}

#[test]
fn from_env_uses_platform_port_for_http_transport() {
    let _guard = ENV_LOCK.lock().expect("env lock");
    let vars = [
        ("DBT_NOVA_SERVER_TRANSPORT", Some("streamable_http")),
        ("DBT_NOVA_HTTP_HOST", None),
        ("DBT_NOVA_HTTP_PORT", None),
        ("DBT_NOVA_HTTP_EXPECT_AUTH_PROXY", None),
        ("PORT", Some("9090")),
    ];
    let previous = vars.map(|(key, _)| (key, std::env::var(key).ok()));
    for (key, value) in vars {
        match value {
            Some(value) => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::set_var(key, value) };
            }
            None => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::remove_var(key) };
            }
        }
    }

    let config = DbtNovaConfig::from_env();

    for (key, value) in previous {
        match value {
            Some(value) => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::set_var(key, value) };
            }
            None => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::remove_var(key) };
            }
        }
    }

    assert_eq!(config.server_transport, ServerTransport::StreamableHttp);
    assert_eq!(config.http_port, 9090);
    assert_eq!(config.http_host, "0.0.0.0");
    let error = config
        .validate()
        .expect_err("platform PORT fallback exposes HTTP and should need auth proxy ack");
    assert!(
        error
            .to_string()
            .contains("DBT_NOVA_HTTP_EXPECT_AUTH_PROXY=true")
    );
    assert!(
        error
            .to_string()
            .contains("published container images do not set this acknowledgement by default")
    );
}

#[test]
fn from_env_platform_port_validates_with_auth_proxy_ack() {
    let _guard = ENV_LOCK.lock().expect("env lock");
    let vars = [
        ("DBT_NOVA_SERVER_TRANSPORT", Some("streamable_http")),
        ("DBT_NOVA_HTTP_HOST", None),
        ("DBT_NOVA_HTTP_PORT", None),
        ("DBT_NOVA_HTTP_EXPECT_AUTH_PROXY", Some("true")),
        ("PORT", Some("9090")),
    ];
    let previous = vars.map(|(key, _)| (key, std::env::var(key).ok()));
    for (key, value) in vars {
        match value {
            Some(value) => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::set_var(key, value) };
            }
            None => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::remove_var(key) };
            }
        }
    }

    let config = DbtNovaConfig::from_env();

    for (key, value) in previous {
        match value {
            Some(value) => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::set_var(key, value) };
            }
            None => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::remove_var(key) };
            }
        }
    }

    assert_eq!(config.server_transport, ServerTransport::StreamableHttp);
    assert_eq!(config.http_port, 9090);
    assert_eq!(config.http_host, "0.0.0.0");
    assert!(config.http_expect_auth_proxy);
    config
        .validate()
        .expect("platform PORT fallback should validate with explicit auth proxy ack");
}

#[test]
fn from_env_uses_platform_host_fallback_with_explicit_http_port() {
    let _guard = ENV_LOCK.lock().expect("env lock");
    let vars = [
        ("DBT_NOVA_SERVER_TRANSPORT", Some("streamable_http")),
        ("DBT_NOVA_HTTP_HOST", None),
        ("DBT_NOVA_HTTP_PORT", Some("8080")),
        ("PORT", Some("9090")),
    ];
    let previous = vars.map(|(key, _)| (key, std::env::var(key).ok()));
    for (key, value) in vars {
        match value {
            Some(value) => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::set_var(key, value) };
            }
            None => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::remove_var(key) };
            }
        }
    }

    let config = DbtNovaConfig::from_env();

    for (key, value) in previous {
        match value {
            Some(value) => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::set_var(key, value) };
            }
            None => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::remove_var(key) };
            }
        }
    }

    assert_eq!(config.server_transport, ServerTransport::StreamableHttp);
    assert_eq!(config.http_port, 8080);
    assert_eq!(config.http_host, "0.0.0.0");
}

#[test]
fn from_env_ignores_invalid_http_port_for_platform_fallback() {
    let _guard = ENV_LOCK.lock().expect("env lock");
    let vars = [
        ("DBT_NOVA_SERVER_TRANSPORT", Some("streamable_http")),
        ("DBT_NOVA_HTTP_HOST", None),
        ("DBT_NOVA_HTTP_PORT", Some("not-a-port")),
        ("PORT", Some("9090")),
    ];
    let previous = vars.map(|(key, _)| (key, std::env::var(key).ok()));
    for (key, value) in vars {
        match value {
            Some(value) => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::set_var(key, value) };
            }
            None => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::remove_var(key) };
            }
        }
    }

    let config = DbtNovaConfig::from_env();

    for (key, value) in previous {
        match value {
            Some(value) => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::set_var(key, value) };
            }
            None => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::remove_var(key) };
            }
        }
    }

    assert_eq!(config.server_transport, ServerTransport::StreamableHttp);
    assert_eq!(config.http_port, 9090);
    assert_eq!(config.http_host, "0.0.0.0");
}

#[test]
fn from_env_reads_http_expect_auth_proxy_flag() {
    let _guard = ENV_LOCK.lock().expect("env lock");
    let vars = [("DBT_NOVA_HTTP_EXPECT_AUTH_PROXY", Some("true"))];
    let previous = vars.map(|(key, _)| (key, std::env::var(key).ok()));
    for (key, value) in vars {
        match value {
            Some(value) => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::set_var(key, value) };
            }
            None => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::remove_var(key) };
            }
        }
    }

    let config = DbtNovaConfig::from_env();

    for (key, value) in previous {
        match value {
            Some(value) => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::set_var(key, value) };
            }
            None => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::remove_var(key) };
            }
        }
    }

    assert!(config.http_expect_auth_proxy);
}

#[test]
fn from_env_reads_http_allowed_hosts() {
    let _guard = ENV_LOCK.lock().expect("env lock");
    let vars = [(
        "DBT_NOVA_HTTP_ALLOWED_HOSTS",
        Some("nova.example.com,nova.example.com:443"),
    )];
    let previous = vars.map(|(key, _)| (key, std::env::var(key).ok()));
    for (key, value) in vars {
        match value {
            Some(value) => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::set_var(key, value) };
            }
            None => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::remove_var(key) };
            }
        }
    }

    let config = DbtNovaConfig::from_env();

    for (key, value) in previous {
        match value {
            Some(value) => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::set_var(key, value) };
            }
            None => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::remove_var(key) };
            }
        }
    }

    assert_eq!(
        config.http_allowed_hosts,
        "nova.example.com,nova.example.com:443"
    );
}

#[test]
fn from_env_reads_http_max_body_bytes() {
    let _guard = ENV_LOCK.lock().expect("env lock");
    let vars = [("DBT_NOVA_HTTP_MAX_BODY_BYTES", Some("4096"))];
    let previous = vars.map(|(key, _)| (key, std::env::var(key).ok()));
    for (key, value) in vars {
        match value {
            Some(value) => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::set_var(key, value) };
            }
            None => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::remove_var(key) };
            }
        }
    }

    let config = DbtNovaConfig::from_env();

    for (key, value) in previous {
        match value {
            Some(value) => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::set_var(key, value) };
            }
            None => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::remove_var(key) };
            }
        }
    }

    assert_eq!(config.http_max_body_bytes, 4096);
}

#[test]
fn from_env_reads_tool_allowlist_and_denylist() {
    let _guard = ENV_LOCK.lock().expect("env lock");
    let vars = [
        ("DBT_NOVA_TOOL_ALLOWLIST", Some("search, get_entity")),
        ("DBT_NOVA_TOOL_DENYLIST", Some("execute_sql")),
        ("DBT_NOVA_TOOL_PROFILE", Some("engineer")),
    ];
    let previous = vars.map(|(key, _)| (key, std::env::var(key).ok()));
    for (key, value) in vars {
        match value {
            Some(value) => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::set_var(key, value) };
            }
            None => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::remove_var(key) };
            }
        }
    }

    let config = DbtNovaConfig::from_env();

    for (key, value) in previous {
        match value {
            Some(value) => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::set_var(key, value) };
            }
            None => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::remove_var(key) };
            }
        }
    }

    assert_eq!(
        config.parsed_tool_allowlist(),
        Some(vec!["search".to_string(), "get_entity".to_string()])
    );
    assert_eq!(
        config.parsed_tool_denylist(),
        vec!["execute_sql".to_string()]
    );
    assert_eq!(config.tool_profile, "engineer");
}

#[test]
fn from_env_reads_result_profile_settings() {
    let _guard = ENV_LOCK.lock().expect("env lock");
    let vars = [
        ("DBT_NOVA_RESULT_PROFILE", Some("full")),
        ("DBT_NOVA_MCP_RESULT_PROFILE", Some("standard")),
        ("DBT_NOVA_MCP_DEFAULT_LIMIT", Some("7")),
        ("DBT_NOVA_MCP_MAX_PAGE_SIZE", Some("25")),
    ];
    let previous = vars.map(|(key, _)| (key, std::env::var(key).ok()));
    for (key, value) in vars {
        match value {
            Some(value) => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::set_var(key, value) };
            }
            None => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::remove_var(key) };
            }
        }
    }

    let config = DbtNovaConfig::from_env();

    for (key, value) in previous {
        match value {
            Some(value) => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::set_var(key, value) };
            }
            None => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::remove_var(key) };
            }
        }
    }

    assert_eq!(config.result_profile, ResultProfile::Full);
    assert_eq!(config.mcp_result_profile, ResultProfile::Standard);
    assert_eq!(config.mcp_default_limit, 7);
    assert_eq!(config.mcp_max_page_size, 25);
}

#[test]
fn from_env_records_invalid_result_profile() {
    let _guard = ENV_LOCK.lock().expect("env lock");
    let vars = [("DBT_NOVA_RESULT_PROFILE", Some("verbose"))];
    let previous = vars.map(|(key, _)| (key, std::env::var(key).ok()));
    for (key, value) in vars {
        match value {
            Some(value) => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::set_var(key, value) };
            }
            None => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::remove_var(key) };
            }
        }
    }

    let config = DbtNovaConfig::from_env();

    for (key, value) in previous {
        match value {
            Some(value) => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::set_var(key, value) };
            }
            None => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::remove_var(key) };
            }
        }
    }

    let error = config
        .validate()
        .expect_err("invalid result profile should fail validation");
    assert!(error.to_string().contains("DBT_NOVA_RESULT_PROFILE"));
}

#[test]
fn from_env_parses_manifest_prune_ids() {
    let _guard = ENV_LOCK.lock().expect("env lock");
    let vars = [
        (
            "DBT_NOVA_PRUNE_ALLOW_IDS",
            Some("[\"model.pkg.orders\",\"analysis.pkg.*\"]"),
        ),
        ("DBT_NOVA_PRUNE_DENY_IDS", Some("[\"model.pkg.stg_*\"]")),
    ];
    let previous = vars.map(|(key, _)| (key, std::env::var(key).ok()));
    for (key, value) in vars {
        match value {
            Some(value) => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::set_var(key, value) };
            }
            None => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::remove_var(key) };
            }
        }
    }

    let config = DbtNovaConfig::from_env();

    for (key, value) in previous {
        match value {
            Some(value) => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::set_var(key, value) };
            }
            None => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::remove_var(key) };
            }
        }
    }

    assert_eq!(
        config.manifest_prune_allow_ids,
        vec!["model.pkg.orders".to_string(), "analysis.pkg.*".to_string()]
    );
    assert_eq!(
        config.manifest_prune_deny_ids,
        vec!["model.pkg.stg_*".to_string()]
    );
}

#[test]
fn from_env_invalid_manifest_prune_json_fails_validation() {
    let _guard = ENV_LOCK.lock().expect("env lock");
    let vars = [
        ("DBT_NOVA_PRUNE_ALLOW_IDS", Some("model.pkg.*")),
        ("DBT_NOVA_PRUNE_DENY_IDS", Some("[\"model.pkg.stg_*\"]")),
    ];
    let previous = vars.map(|(key, _)| (key, std::env::var(key).ok()));
    for (key, value) in vars {
        match value {
            Some(value) => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::set_var(key, value) };
            }
            None => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::remove_var(key) };
            }
        }
    }

    let config = DbtNovaConfig::from_env();

    for (key, value) in previous {
        match value {
            Some(value) => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::set_var(key, value) };
            }
            None => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::remove_var(key) };
            }
        }
    }

    let error = config
        .validate()
        .expect_err("invalid prune JSON should fail");
    assert!(
        error
            .to_string()
            .contains("Invalid DBT_NOVA_PRUNE_ALLOW_IDS JSON")
    );
}

#[test]
fn manifest_prune_fingerprint_is_order_independent() {
    let config_a = DbtNovaConfig {
        manifest_prune_allow_ids: vec![
            "model.pkg.a".to_string(),
            "model.pkg.b".to_string(),
            String::new(),
        ],
        manifest_prune_deny_ids: vec![
            "model.pkg.c".to_string(),
            "model.pkg.d".to_string(),
            "   ".to_string(),
        ],
        ..DbtNovaConfig::default()
    };
    let config_b = DbtNovaConfig {
        manifest_prune_allow_ids: vec![" model.pkg.b ".to_string(), "model.pkg.a".to_string()],
        manifest_prune_deny_ids: vec!["model.pkg.d".to_string(), " model.pkg.c ".to_string()],
        ..DbtNovaConfig::default()
    };
    assert_eq!(
        config_a.manifest_prune_fingerprint(),
        config_b.manifest_prune_fingerprint()
    );
    assert!(config_a.manifest_pruning_enabled());
}

#[test]
fn validate_rejects_invalid_tool_names_in_filters() {
    let config = DbtNovaConfig {
        tool_allowlist: "search,unknown_tool".to_string(),
        tool_denylist: "execute_sql,Search".to_string(),
        ..base_config()
    };

    let error = config
        .validate()
        .expect_err("invalid tool names should fail validation");
    let message = error.to_string();
    assert!(message.contains("DBT_NOVA_TOOL_ALLOWLIST: unknown_tool"));
    assert!(message.contains("DBT_NOVA_TOOL_DENYLIST: Search"));
    assert!(message.contains("case-sensitive exact MCP tool names"));
    assert!(message.contains("search"));
    assert!(message.contains("execute_sql"));
}

#[test]
fn validate_rejects_invalid_tool_profile() {
    let config = DbtNovaConfig {
        tool_profile: "everything".to_string(),
        ..base_config()
    };

    let error = config
        .validate()
        .expect_err("invalid tool profile should fail validation");
    let message = error.to_string();
    assert!(message.contains("DBT_NOVA_TOOL_PROFILE: everything"));
    assert!(message.contains("agent, analyst, engineer, governance, ops, all"));
}

#[test]
fn empty_allowlist_is_treated_as_unset() {
    let config = DbtNovaConfig {
        tool_allowlist: "   ".to_string(),
        tool_denylist: "execute_sql".to_string(),
        ..base_config()
    };

    let resolved = config.resolved_mcp_tool_names();
    assert!(resolved.contains("search"));
    assert!(!resolved.contains("execute_sql"));
}

#[test]
fn default_tool_profile_is_lean_agent_catalog() {
    let config = base_config();
    let resolved = config.resolved_mcp_tool_names();

    assert!(resolved.contains("search"));
    assert!(resolved.contains("get_context"));
    assert!(resolved.contains("search_recipes"));
    assert!(!resolved.contains("execute_sql"));
    assert!(!resolved.contains("run_eval"));
    assert!(!resolved.contains("inspect_storage"));
    assert!(resolved.len() < crate::tools::catalog::MCP_TOOL_COUNT);
}

#[test]
fn all_tool_profile_restores_full_catalog() {
    let config = DbtNovaConfig {
        tool_profile: "all".to_string(),
        ..base_config()
    };
    let resolved = config.resolved_mcp_tool_names();

    assert_eq!(resolved.len(), crate::tools::catalog::MCP_TOOL_COUNT);
    assert!(resolved.contains("execute_sql"));
    assert!(resolved.contains("run_eval"));
}

#[test]
fn denylist_takes_precedence_over_allowlist() {
    let config = DbtNovaConfig {
        tool_allowlist: "search,execute_sql".to_string(),
        tool_denylist: "execute_sql".to_string(),
        ..base_config()
    };

    let resolved = config.resolved_mcp_tool_names();
    assert!(resolved.contains("search"));
    assert!(!resolved.contains("execute_sql"));
    assert_eq!(resolved.len(), 1);
}
