use clap::{ArgAction, Args, Parser, Subcommand, ValueEnum};

#[derive(Debug, Parser)]
#[command(name = "dbt-nova", version, about = "dbt-nova CLI")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    Server(ServerArgs),
    Manifest(ManifestArgs),
    Tool(ToolArgs),
    Audit(AuditArgs),
    Config(ConfigArgs),
    Storage(StorageArgs),
    Health(HealthArgs),
    Eval(EvalArgs),
}

#[derive(Debug, Args)]
pub struct ServerArgs {
    #[command(subcommand)]
    pub command: ServerCommand,
}

#[derive(Debug, Subcommand)]
pub enum ServerCommand {
    Start(ServerStartArgs),
}

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
pub enum ServerTransportArg {
    Stdio,
    StreamableHttp,
}

#[derive(Debug, Clone, Args, Default)]
pub struct ServerStartArgs {
    #[arg(long, value_enum)]
    pub transport: Option<ServerTransportArg>,
    #[arg(long, value_name = "HOST")]
    pub http_host: Option<String>,
    #[arg(long, value_name = "PORT")]
    pub http_port: Option<u16>,
    #[arg(long, value_name = "PATH")]
    pub http_path: Option<String>,
    #[arg(long, value_name = "BOOL")]
    pub http_stateful_mode: Option<bool>,
}

#[derive(Debug, Args)]
pub struct ManifestArgs {
    #[command(subcommand)]
    pub command: ManifestCommand,
}

#[derive(Debug, Subcommand)]
pub enum ManifestCommand {
    Load(ManifestLoadArgs),
    Reload(ManifestReloadArgs),
    Warm(ManifestWarmArgs),
}

#[derive(Debug, Clone, Args, Default)]
pub struct ManifestLoadArgs {
    #[arg(long, value_name = "PATH", conflicts_with = "manifest_uri")]
    pub manifest_path: Option<String>,
    #[arg(long, value_name = "URI", conflicts_with = "manifest_path")]
    pub manifest_uri: Option<String>,
    #[arg(long, value_name = "INSTANCE_ID")]
    pub storage_instance_id: Option<String>,
    #[arg(long, default_value_t = false)]
    pub cleanup_storage_on_start: bool,
    #[arg(long, default_value_t = false)]
    pub read_only: bool,
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Debug, Clone, Args, Default)]
pub struct ManifestReloadArgs {
    #[arg(long, value_name = "PATH", conflicts_with = "manifest_uri")]
    pub manifest_path: Option<String>,
    #[arg(long, value_name = "URI", conflicts_with = "manifest_path")]
    pub manifest_uri: Option<String>,
    #[arg(long, value_name = "SECS")]
    pub refresh_secs: Option<u64>,
    #[arg(long, value_name = "INSTANCE_ID")]
    pub storage_instance_id: Option<String>,
    #[arg(long, default_value_t = false)]
    pub cleanup_storage_on_start: bool,
    #[arg(long, default_value_t = false)]
    pub read_only: bool,
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Args, Default)]
pub struct ManifestWarmArgs {
    #[arg(long, value_name = "PATH", conflicts_with = "manifest_uri")]
    pub manifest_path: Option<String>,
    #[arg(long, value_name = "URI", conflicts_with = "manifest_path")]
    pub manifest_uri: Option<String>,
    #[arg(long, value_name = "INSTANCE_ID")]
    pub storage_instance_id: Option<String>,
    #[arg(long, default_value_t = false)]
    pub vector: bool,
    #[arg(long, default_value_t = false)]
    pub sparse: bool,
    #[arg(long, default_value_t = false)]
    pub reranker: bool,
    #[arg(long, default_value_t = false)]
    pub force: bool,
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct ToolArgs {
    #[command(subcommand)]
    pub command: ToolCommand,
}

#[derive(Debug, Subcommand)]
pub enum ToolCommand {
    Call(ToolCallArgs),
}

#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Args, Default)]
pub struct ToolCallArgs {
    #[arg(value_name = "TOOL_NAME")]
    pub tool_name: String,
    #[arg(long, value_name = "JSON", conflicts_with_all = ["params_file", "params_stdin"])]
    pub params_json: Option<String>,
    #[arg(long, value_name = "PATH", conflicts_with_all = ["params_json", "params_stdin"])]
    pub params_file: Option<String>,
    #[arg(long, default_value_t = false, conflicts_with_all = ["params_json", "params_file"])]
    pub params_stdin: bool,
    #[arg(long, value_name = "PATH", conflicts_with = "manifest_uri")]
    pub manifest_path: Option<String>,
    #[arg(long, value_name = "URI", conflicts_with = "manifest_path")]
    pub manifest_uri: Option<String>,
    #[arg(long, value_name = "INSTANCE_ID")]
    pub storage_instance_id: Option<String>,
    #[arg(long, default_value_t = false)]
    pub cleanup_storage_on_start: bool,
    #[arg(long, default_value_t = false)]
    pub read_only: bool,
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct AuditArgs {
    #[command(subcommand)]
    pub command: AuditCommand,
}

#[derive(Debug, Subcommand)]
pub enum AuditCommand {
    MetadataScore(Box<MetadataAuditArgs>),
    NovaMeta(NovaMetaAuditArgs),
}

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
pub enum NovaMetaResourceKindArg {
    Model,
    Source,
    Table,
    Metric,
}

#[derive(Debug, Clone, Args, Default)]
pub struct NovaMetaAuditArgs {
    #[arg(long, value_name = "DIR")]
    pub project_dir: Option<String>,
    #[arg(long, value_name = "PATH", action = ArgAction::Append)]
    pub path: Vec<String>,
    #[arg(long, value_enum)]
    pub resource_kind: Option<NovaMetaResourceKindArg>,
    #[arg(long, value_name = "NAME")]
    pub resource_name: Option<String>,
    #[arg(long, value_name = "COLUMN", requires = "resource_name")]
    pub column: Option<String>,
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq, Default)]
pub enum MetadataAuditSelectionModeArg {
    #[default]
    Project,
    Changed,
    Entities,
}

#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Args, Default)]
pub struct MetadataAuditArgs {
    #[arg(long, value_enum, default_value_t = MetadataAuditSelectionModeArg::Project)]
    pub selection_mode: MetadataAuditSelectionModeArg,
    #[arg(long, value_name = "JSON", conflicts_with = "changed_files_file")]
    pub changed_files_json: Option<String>,
    #[arg(long, value_name = "PATH", conflicts_with = "changed_files_json")]
    pub changed_files_file: Option<String>,
    #[arg(long, value_name = "JSON", conflicts_with = "entity_ids_file")]
    pub entity_ids_json: Option<String>,
    #[arg(long, value_name = "PATH", conflicts_with = "entity_ids_json")]
    pub entity_ids_file: Option<String>,
    #[arg(long, value_name = "JSON")]
    pub resource_types_json: Option<String>,
    #[arg(long, value_name = "JSON")]
    pub personas_json: Option<String>,
    #[arg(long, value_name = "JSON", conflicts_with = "thresholds_file")]
    pub thresholds_json: Option<String>,
    #[arg(long, value_name = "PATH", conflicts_with = "thresholds_json")]
    pub thresholds_file: Option<String>,
    #[arg(long, value_name = "PATH", conflicts_with = "manifest_uri")]
    pub manifest_path: Option<String>,
    #[arg(long, value_name = "URI", conflicts_with = "manifest_path")]
    pub manifest_uri: Option<String>,
    #[arg(long, value_name = "INSTANCE_ID")]
    pub storage_instance_id: Option<String>,
    #[arg(long, default_value_t = false)]
    pub cleanup_storage_on_start: bool,
    #[arg(long, default_value_t = false)]
    pub read_only: bool,
    #[arg(long, value_name = "BOOL")]
    pub include_breakdown: Option<bool>,
    #[arg(long, value_name = "BOOL")]
    pub include_recommendations: Option<bool>,
    #[arg(long, value_name = "PATH")]
    pub report_json_path: Option<String>,
    #[arg(long, value_name = "PATH")]
    pub report_md_path: Option<String>,
    #[arg(long, default_value_t = false)]
    pub fail_on_no_targets: bool,
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct ConfigArgs {
    #[command(subcommand)]
    pub command: ConfigCommand,
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    Show(ConfigShowArgs),
    Validate(ConfigValidateArgs),
}

#[derive(Debug, Clone, Args, Default)]
pub struct ConfigShowArgs {
    #[arg(long, default_value_t = false)]
    pub defaults: bool,
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Debug, Clone, Args, Default)]
pub struct ConfigValidateArgs {
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct StorageArgs {
    #[command(subcommand)]
    pub command: StorageCommand,
}

#[derive(Debug, Subcommand)]
pub enum StorageCommand {
    Inspect(StorageInspectArgs),
    Prune(StoragePruneArgs),
    Cleanup(StorageCleanupArgs),
}

#[derive(Debug, Clone, Args, Default)]
pub struct StorageInspectArgs {
    #[arg(long, value_name = "INSTANCE_ID")]
    pub storage_instance_id: Option<String>,
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Debug, Clone, Args, Default)]
pub struct StoragePruneArgs {
    #[arg(long, value_name = "N")]
    pub max_keep: Option<usize>,
    #[arg(long, value_name = "BYTES")]
    pub max_bytes: Option<u64>,
    #[arg(long, value_name = "INSTANCE_ID")]
    pub storage_instance_id: Option<String>,
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Debug, Clone, Args, Default)]
pub struct StorageCleanupArgs {
    #[arg(long, value_name = "INSTANCE_ID")]
    pub storage_instance_id: Option<String>,
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct HealthArgs {
    #[command(subcommand)]
    pub command: HealthCommand,
}

#[derive(Debug, Subcommand)]
pub enum HealthCommand {
    Check(HealthCheckArgs),
}

#[derive(Debug, Clone, Args, Default)]
pub struct HealthCheckArgs {
    #[arg(long, value_name = "PATH", conflicts_with = "manifest_uri")]
    pub manifest_path: Option<String>,
    #[arg(long, value_name = "URI", conflicts_with = "manifest_path")]
    pub manifest_uri: Option<String>,
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct EvalArgs {
    #[command(subcommand)]
    pub command: EvalCommand,
}

#[derive(Debug, Subcommand)]
pub enum EvalCommand {
    /// Write a starter eval suite.
    Init(EvalInitArgs),
    /// Run deterministic Nova tool assertions against a manifest.
    Run(EvalRunArgs),
    /// Run provider-backed agent evals and score observed Nova tool use.
    Agent(EvalAgentArgs),
    /// Print filtered JSONL eval telemetry history.
    History(EvalHistoryArgs),
    /// Validate an eval suite without loading a manifest or running a provider.
    Validate(EvalValidateArgs),
}

#[derive(Debug, Clone, Args, Default)]
pub struct EvalInitArgs {
    #[arg(
        long,
        value_name = "PERSONA",
        default_value = "analyst",
        help = "Persona to use in the generated starter suite"
    )]
    pub persona: String,
    #[arg(long, value_name = "PATH", help = "Path to write the suite YAML")]
    pub out: String,
    #[arg(
        long,
        default_value_t = false,
        help = "Overwrite an existing suite file"
    )]
    pub force: bool,
}

#[derive(Debug, Clone, Args, Default)]
#[allow(clippy::struct_excessive_bools)]
pub struct EvalRunArgs {
    #[arg(long, value_name = "PATH", help = "YAML or JSON eval suite path")]
    pub suite: String,
    #[arg(
        long,
        value_name = "PATH",
        conflicts_with = "manifest_uri",
        help = "Local dbt manifest.json path"
    )]
    pub manifest_path: Option<String>,
    #[arg(
        long,
        value_name = "URI",
        conflicts_with = "manifest_path",
        help = "Remote manifest or prebuilt artifact URI"
    )]
    pub manifest_uri: Option<String>,
    #[arg(
        long,
        value_name = "INSTANCE_ID",
        help = "Storage instance id for cached Nova assets"
    )]
    pub storage_instance_id: Option<String>,
    #[arg(long, value_name = "DIR", help = "Directory for eval result artifacts")]
    pub output_dir: Option<String>,
    #[arg(
        long,
        default_value_t = false,
        help = "Append per-assertion JSONL telemetry for this eval run"
    )]
    pub telemetry: bool,
    #[arg(
        long = "telemetry-retention",
        value_name = "ROWS",
        help = "After writing telemetry, keep only the newest ROWS rows for this suite"
    )]
    pub telemetry_retention: Option<usize>,
    #[arg(
        long = "case-id",
        value_name = "ID",
        action = ArgAction::Append,
        help = "Only run the named bridge case; repeat for multiple cases"
    )]
    pub case_ids: Vec<String>,
    #[arg(
        long,
        value_name = "FLOAT",
        help = "Required pass rate between 0.0 and 1.0"
    )]
    pub fail_under: Option<f64>,
    #[arg(
        long,
        default_value_t = false,
        help = "Clear the selected storage instance before loading"
    )]
    pub cleanup_storage_on_start: bool,
    #[arg(long, default_value_t = false, help = "Open storage in read-only mode")]
    pub read_only: bool,
    #[arg(long, default_value_t = false, help = "Emit a JSON CLI envelope")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct EvalAgentArgs {
    #[command(subcommand)]
    pub command: EvalAgentCommand,
}

#[derive(Debug, Subcommand)]
pub enum EvalAgentCommand {
    /// Run provider-backed agent evals and score observed Nova tool use.
    Run(EvalAgentRunArgs),
}

#[derive(Debug, Clone, Args, Default)]
#[allow(clippy::struct_excessive_bools)]
pub struct EvalAgentRunArgs {
    #[arg(long, value_name = "PATH", help = "YAML or JSON eval suite path")]
    pub suite: String,
    #[arg(
        long,
        value_name = "PROVIDER",
        default_value = "opencode",
        help = "Provider preset: opencode, codex, claude, goose, or custom"
    )]
    pub provider: String,
    #[arg(
        long,
        value_name = "MODEL",
        help = "Model id to pass to provider presets that support --model, for example opencode/deepseek-v4-flash-free"
    )]
    pub provider_model: Option<String>,
    #[arg(
        long,
        value_name = "COMMAND",
        help = "Custom provider command to execute"
    )]
    pub provider_command: Option<String>,
    #[arg(
        long,
        value_name = "JSON_ARRAY",
        help = "Custom provider arguments as a JSON string array with placeholders"
    )]
    pub provider_args_json: Option<String>,
    #[arg(
        long,
        value_name = "PATH",
        conflicts_with = "manifest_uri",
        help = "Local dbt manifest.json path"
    )]
    pub manifest_path: Option<String>,
    #[arg(
        long,
        value_name = "URI",
        conflicts_with = "manifest_path",
        help = "Remote manifest or prebuilt artifact URI"
    )]
    pub manifest_uri: Option<String>,
    #[arg(
        long,
        value_name = "INSTANCE_ID",
        help = "Storage instance id for cached Nova assets"
    )]
    pub storage_instance_id: Option<String>,
    #[arg(long, value_name = "DIR", help = "Directory for eval result artifacts")]
    pub output_dir: Option<String>,
    #[arg(
        long,
        default_value_t = false,
        help = "Append per-assertion JSONL telemetry for this eval run"
    )]
    pub telemetry: bool,
    #[arg(
        long = "telemetry-retention",
        value_name = "ROWS",
        help = "After writing telemetry, keep only the newest ROWS rows for this suite"
    )]
    pub telemetry_retention: Option<usize>,
    #[arg(
        long = "case-id",
        value_name = "ID",
        action = ArgAction::Append,
        help = "Only run the named agent case; repeat for multiple cases"
    )]
    pub case_ids: Vec<String>,
    #[arg(
        long,
        value_name = "SECS",
        default_value_t = 600,
        help = "Provider command timeout in seconds"
    )]
    pub timeout_secs: u64,
    #[arg(
        long,
        value_name = "FLOAT",
        help = "Required pass rate between 0.0 and 1.0"
    )]
    pub fail_under: Option<f64>,
    #[arg(
        long,
        default_value_t = false,
        help = "Clear the selected storage instance before running the provider"
    )]
    pub cleanup_storage_on_start: bool,
    #[arg(long, default_value_t = false, help = "Open storage in read-only mode")]
    pub read_only: bool,
    #[arg(long, default_value_t = false, help = "Emit a JSON CLI envelope")]
    pub json: bool,
}

#[derive(Debug, Clone, Args, Default)]
pub struct EvalHistoryArgs {
    #[arg(
        long,
        value_name = "NAME",
        help = "Eval suite name to read history for"
    )]
    pub suite: String,
    #[arg(
        long,
        value_name = "YYYY-MM-DD",
        help = "Only print telemetry rows on or after this UTC date"
    )]
    pub since: String,
}

#[derive(Debug, Clone, Args, Default)]
pub struct EvalValidateArgs {
    #[arg(long, value_name = "PATH", help = "YAML or JSON eval suite path")]
    pub suite: String,
    #[arg(long, default_value_t = false, help = "Emit a JSON CLI envelope")]
    pub json: bool,
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::{
        AuditCommand, Cli, Command, ConfigCommand, EvalAgentCommand, EvalCommand, HealthCommand,
        ManifestCommand, MetadataAuditSelectionModeArg, NovaMetaResourceKindArg, ServerCommand,
        ServerTransportArg, StorageCommand, ToolCommand,
    };

    #[test]
    fn cli_parses_no_subcommand() {
        let cli = Cli::parse_from(["dbt-nova"]);
        assert!(cli.command.is_none());
    }

    #[test]
    fn cli_parses_server_start() {
        let cli = Cli::parse_from([
            "dbt-nova",
            "server",
            "start",
            "--transport",
            "streamable-http",
            "--http-host",
            "0.0.0.0",
            "--http-port",
            "8080",
            "--http-path",
            "/mcp",
            "--http-stateful-mode",
            "false",
        ]);
        let command = cli.command.expect("command");
        match command {
            Command::Server(server) => {
                let ServerCommand::Start(args) = server.command;
                assert_eq!(args.transport, Some(ServerTransportArg::StreamableHttp));
                assert_eq!(args.http_host.as_deref(), Some("0.0.0.0"));
                assert_eq!(args.http_port, Some(8080));
                assert_eq!(args.http_path.as_deref(), Some("/mcp"));
                assert_eq!(args.http_stateful_mode, Some(false));
            }
            _ => panic!("expected server command"),
        }
    }

    #[test]
    fn cli_parses_all_top_level_groups() {
        let groups: [&[&str]; 8] = [
            &["dbt-nova", "manifest", "load"],
            &["dbt-nova", "tool", "call", "search"],
            &["dbt-nova", "audit", "metadata-score"],
            &["dbt-nova", "audit", "nova-meta"],
            &["dbt-nova", "config", "show"],
            &["dbt-nova", "storage", "inspect"],
            &["dbt-nova", "health", "check"],
            &["dbt-nova", "eval", "run", "--suite", "suite.yml"],
        ];
        for args in groups {
            let cli = Cli::parse_from(args);
            assert!(cli.command.is_some());
        }
    }

    #[test]
    fn eval_validate_parses_suite_path() {
        let cli = Cli::parse_from(["dbt-nova", "eval", "validate", "--suite", "suite.yml"]);
        match cli.command.expect("command") {
            Command::Eval(eval) => match eval.command {
                EvalCommand::Validate(args) => assert_eq!(args.suite, "suite.yml"),
                _ => panic!("expected eval validate"),
            },
            _ => panic!("expected eval command"),
        }
    }

    #[test]
    fn eval_run_parses_repeated_case_ids() {
        let cli = Cli::parse_from([
            "dbt-nova",
            "eval",
            "run",
            "--suite",
            "suite.yml",
            "--case-id",
            "one",
            "--case-id",
            "two",
        ]);
        match cli.command.expect("command") {
            Command::Eval(eval) => match eval.command {
                EvalCommand::Run(args) => assert_eq!(args.case_ids, vec!["one", "two"]),
                _ => panic!("expected eval run"),
            },
            _ => panic!("expected eval command"),
        }
    }

    #[test]
    fn eval_run_parses_telemetry_flags() {
        let cli = Cli::parse_from([
            "dbt-nova",
            "eval",
            "run",
            "--suite",
            "suite.yml",
            "--telemetry",
            "--telemetry-retention",
            "100",
        ]);
        match cli.command.expect("command") {
            Command::Eval(eval) => match eval.command {
                EvalCommand::Run(args) => {
                    assert!(args.telemetry);
                    assert_eq!(args.telemetry_retention, Some(100));
                }
                _ => panic!("expected eval run"),
            },
            _ => panic!("expected eval command"),
        }
    }

    #[test]
    fn eval_agent_run_parses_repeated_case_ids() {
        let cli = Cli::parse_from([
            "dbt-nova",
            "eval",
            "agent",
            "run",
            "--suite",
            "suite.yml",
            "--case-id",
            "agent-one",
            "--case-id",
            "agent-two",
        ]);
        match cli.command.expect("command") {
            Command::Eval(eval) => match eval.command {
                EvalCommand::Agent(agent) => match agent.command {
                    EvalAgentCommand::Run(args) => {
                        assert_eq!(args.case_ids, vec!["agent-one", "agent-two"]);
                    }
                },
                _ => panic!("expected eval agent run"),
            },
            _ => panic!("expected eval command"),
        }
    }

    #[test]
    fn eval_history_parses_suite_and_since() {
        let cli = Cli::parse_from([
            "dbt-nova",
            "eval",
            "history",
            "--suite",
            "starter",
            "--since",
            "2026-06-01",
        ]);
        match cli.command.expect("command") {
            Command::Eval(eval) => match eval.command {
                EvalCommand::History(args) => {
                    assert_eq!(args.suite, "starter");
                    assert_eq!(args.since, "2026-06-01");
                }
                _ => panic!("expected eval history"),
            },
            _ => panic!("expected eval command"),
        }
    }

    #[test]
    fn manifest_load_rejects_conflicting_source_flags() {
        let parsed = Cli::try_parse_from([
            "dbt-nova",
            "manifest",
            "load",
            "--manifest-path",
            "target/manifest.json",
            "--manifest-uri",
            "https://example.com/manifest.json",
        ]);
        assert!(parsed.is_err());
    }

    #[test]
    fn manifest_load_parses_json_flag() {
        let cli = Cli::parse_from(["dbt-nova", "manifest", "load", "--json"]);
        let command = cli.command.expect("command");
        match command {
            Command::Manifest(manifest) => {
                assert!(matches!(manifest.command, ManifestCommand::Load(_)));
            }
            _ => panic!("expected manifest command"),
        }
    }

    #[test]
    fn manifest_reload_rejects_conflicting_source_flags() {
        let parsed = Cli::try_parse_from([
            "dbt-nova",
            "manifest",
            "reload",
            "--manifest-path",
            "target/manifest.json",
            "--manifest-uri",
            "https://example.com/manifest.json",
        ]);
        assert!(parsed.is_err());
    }

    #[test]
    fn manifest_reload_parses_flags() {
        let cli = Cli::parse_from([
            "dbt-nova",
            "manifest",
            "reload",
            "--manifest-path",
            "target/manifest.json",
            "--refresh-secs",
            "120",
            "--json",
        ]);
        let command = cli.command.expect("command");
        match command {
            Command::Manifest(manifest) => match manifest.command {
                ManifestCommand::Reload(args) => {
                    assert_eq!(args.manifest_path.as_deref(), Some("target/manifest.json"));
                    assert_eq!(args.refresh_secs, Some(120));
                    assert!(args.json);
                }
                ManifestCommand::Load(_) | ManifestCommand::Warm(_) => {
                    panic!("expected manifest reload command")
                }
            },
            _ => panic!("expected manifest command"),
        }
    }

    #[test]
    fn manifest_warm_parses_flags() {
        let cli = Cli::parse_from([
            "dbt-nova",
            "manifest",
            "warm",
            "--manifest-path",
            "target/manifest.json",
            "--vector",
            "--force",
            "--json",
        ]);
        let command = cli.command.expect("command");
        match command {
            Command::Manifest(manifest) => match manifest.command {
                ManifestCommand::Warm(args) => {
                    assert_eq!(args.manifest_path.as_deref(), Some("target/manifest.json"));
                    assert!(args.vector);
                    assert!(!args.sparse);
                    assert!(args.force);
                    assert!(args.json);
                }
                _ => panic!("expected manifest warm command"),
            },
            _ => panic!("expected manifest command"),
        }
    }

    #[test]
    fn tool_call_rejects_conflicting_param_sources() {
        let parsed = Cli::try_parse_from([
            "dbt-nova",
            "tool",
            "call",
            "search",
            "--params-json",
            "{}",
            "--params-file",
            "params.json",
        ]);
        assert!(parsed.is_err());
    }

    #[test]
    fn tool_call_rejects_conflicting_json_and_stdin_sources() {
        let parsed = Cli::try_parse_from([
            "dbt-nova",
            "tool",
            "call",
            "search",
            "--params-json",
            "{}",
            "--params-stdin",
        ]);
        assert!(parsed.is_err());
    }

    #[test]
    fn tool_call_rejects_conflicting_file_and_stdin_sources() {
        let parsed = Cli::try_parse_from([
            "dbt-nova",
            "tool",
            "call",
            "search",
            "--params-file",
            "params.json",
            "--params-stdin",
        ]);
        assert!(parsed.is_err());
    }

    #[test]
    fn tool_call_parses_json_flag() {
        let cli = Cli::parse_from(["dbt-nova", "tool", "call", "search", "--json"]);
        let command = cli.command.expect("command");
        match command {
            Command::Tool(tool) => {
                assert!(matches!(tool.command, ToolCommand::Call(_)));
            }
            _ => panic!("expected tool command"),
        }
    }

    #[test]
    fn audit_metadata_score_parses_flags() {
        let cli = Cli::parse_from([
            "dbt-nova",
            "audit",
            "metadata-score",
            "--selection-mode",
            "changed",
            "--changed-files-json",
            "[\"models/staging/orders.sql\"]",
            "--resource-types-json",
            "[\"model\"]",
            "--personas-json",
            "[\"engineer\",\"analyst\"]",
            "--thresholds-json",
            "{\"entity\":{\"engineer\":{\"min_score\":70,\"severity\":\"required\"}}}",
            "--manifest-path",
            "target/manifest.json",
            "--report-json-path",
            "out/report.json",
            "--report-md-path",
            "out/report.md",
            "--json",
        ]);
        let command = cli.command.expect("command");
        match command {
            Command::Audit(audit) => {
                let AuditCommand::MetadataScore(args) = audit.command else {
                    panic!("expected metadata-score command");
                };
                assert_eq!(args.selection_mode, MetadataAuditSelectionModeArg::Changed);
                assert_eq!(
                    args.changed_files_json.as_deref(),
                    Some("[\"models/staging/orders.sql\"]")
                );
                assert_eq!(args.manifest_path.as_deref(), Some("target/manifest.json"));
                assert_eq!(args.report_json_path.as_deref(), Some("out/report.json"));
                assert_eq!(args.report_md_path.as_deref(), Some("out/report.md"));
                assert!(args.json);
            }
            _ => panic!("expected audit command"),
        }
    }

    #[test]
    fn audit_metadata_score_rejects_conflicting_changed_sources() {
        let parsed = Cli::try_parse_from([
            "dbt-nova",
            "audit",
            "metadata-score",
            "--changed-files-json",
            "[]",
            "--changed-files-file",
            "changed.json",
        ]);
        assert!(parsed.is_err());
    }

    #[test]
    fn audit_metadata_score_parses_explicit_bool_overrides() {
        let cli = Cli::parse_from([
            "dbt-nova",
            "audit",
            "metadata-score",
            "--include-breakdown",
            "false",
            "--include-recommendations",
            "true",
        ]);
        let command = cli.command.expect("command");
        match command {
            Command::Audit(audit) => {
                let AuditCommand::MetadataScore(args) = audit.command else {
                    panic!("expected metadata-score command");
                };
                assert_eq!(args.include_breakdown, Some(false));
                assert_eq!(args.include_recommendations, Some(true));
            }
            _ => panic!("expected audit command"),
        }
    }

    #[test]
    fn audit_nova_meta_parses_targeting_flags() {
        let cli = Cli::parse_from([
            "dbt-nova",
            "audit",
            "nova-meta",
            "--project-dir",
            ".",
            "--path",
            "models/marts/orders.yml",
            "--resource-kind",
            "model",
            "--resource-name",
            "fct_orders",
            "--column",
            "order_date",
            "--json",
        ]);
        let command = cli.command.expect("command");
        match command {
            Command::Audit(audit) => {
                let AuditCommand::NovaMeta(args) = audit.command else {
                    panic!("expected nova-meta command");
                };
                assert_eq!(args.project_dir.as_deref(), Some("."));
                assert_eq!(args.path, vec!["models/marts/orders.yml"]);
                assert_eq!(args.resource_kind, Some(NovaMetaResourceKindArg::Model));
                assert_eq!(args.resource_name.as_deref(), Some("fct_orders"));
                assert_eq!(args.column.as_deref(), Some("order_date"));
                assert!(args.json);
            }
            _ => panic!("expected audit command"),
        }
    }

    #[test]
    fn config_show_parses_defaults_and_json_flags() {
        let cli = Cli::parse_from(["dbt-nova", "config", "show", "--defaults", "--json"]);
        let command = cli.command.expect("command");
        match command {
            Command::Config(config) => {
                let ConfigCommand::Show(show_args) = config.command else {
                    panic!("expected config show command");
                };
                assert!(show_args.defaults);
                assert!(show_args.json);
            }
            _ => panic!("expected config command"),
        }
    }

    #[test]
    fn config_validate_parses_json_flag() {
        let cli = Cli::parse_from(["dbt-nova", "config", "validate", "--json"]);
        let command = cli.command.expect("command");
        match command {
            Command::Config(config) => {
                assert!(matches!(config.command, ConfigCommand::Validate(_)));
            }
            _ => panic!("expected config command"),
        }
    }

    #[test]
    fn storage_prune_parses_limit_flags() {
        let cli = Cli::parse_from([
            "dbt-nova",
            "storage",
            "prune",
            "--max-keep",
            "2",
            "--max-bytes",
            "1000",
            "--json",
        ]);
        let command = cli.command.expect("command");
        match command {
            Command::Storage(storage) => {
                let StorageCommand::Prune(prune_args) = storage.command else {
                    panic!("expected storage prune command");
                };
                assert_eq!(prune_args.max_keep, Some(2));
                assert_eq!(prune_args.max_bytes, Some(1000));
                assert!(prune_args.json);
            }
            _ => panic!("expected storage command"),
        }
    }

    #[test]
    fn storage_cleanup_parses_storage_instance_id_flag() {
        let cli = Cli::parse_from([
            "dbt-nova",
            "storage",
            "cleanup",
            "--storage-instance-id",
            "manifest-abc123",
        ]);
        let command = cli.command.expect("command");
        match command {
            Command::Storage(storage) => {
                let StorageCommand::Cleanup(cleanup_args) = storage.command else {
                    panic!("expected storage cleanup command");
                };
                assert_eq!(
                    cleanup_args.storage_instance_id.as_deref(),
                    Some("manifest-abc123")
                );
            }
            _ => panic!("expected storage command"),
        }
    }

    #[test]
    fn health_check_parses_manifest_override_flags() {
        let cli = Cli::parse_from([
            "dbt-nova",
            "health",
            "check",
            "--manifest-path",
            "tests/fixtures/nova_manifest.json",
            "--json",
        ]);
        let command = cli.command.expect("command");
        match command {
            Command::Health(health) => {
                let HealthCommand::Check(check_args) = health.command;
                assert_eq!(
                    check_args.manifest_path.as_deref(),
                    Some("tests/fixtures/nova_manifest.json")
                );
                assert!(check_args.json);
            }
            _ => panic!("expected health command"),
        }
    }

    #[test]
    fn health_check_rejects_conflicting_manifest_source_flags() {
        let parsed = Cli::try_parse_from([
            "dbt-nova",
            "health",
            "check",
            "--manifest-path",
            "tests/fixtures/nova_manifest.json",
            "--manifest-uri",
            "https://example.com/manifest.json",
        ]);
        assert!(parsed.is_err());
    }
}
