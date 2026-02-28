use clap::{Args, Parser, Subcommand};

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
    Config(ConfigArgs),
    Storage(StorageArgs),
    Health(HealthArgs),
}

#[derive(Debug, Args)]
pub struct ServerArgs {
    #[command(subcommand)]
    pub command: ServerCommand,
}

#[derive(Debug, Subcommand)]
pub enum ServerCommand {
    Start,
}

#[derive(Debug, Args)]
pub struct ManifestArgs {
    #[command(subcommand)]
    pub command: ManifestCommand,
}

#[derive(Debug, Subcommand)]
pub enum ManifestCommand {
    Load(ManifestLoadArgs),
    Reload,
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

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::{
        Cli, Command, ConfigCommand, HealthCommand, ManifestCommand, ServerCommand, StorageCommand,
        ToolCommand,
    };

    #[test]
    fn cli_parses_no_subcommand() {
        let cli = Cli::parse_from(["dbt-nova"]);
        assert!(cli.command.is_none());
    }

    #[test]
    fn cli_parses_server_start() {
        let cli = Cli::parse_from(["dbt-nova", "server", "start"]);
        let command = cli.command.expect("command");
        match command {
            Command::Server(server) => {
                assert!(matches!(server.command, ServerCommand::Start));
            }
            _ => panic!("expected server command"),
        }
    }

    #[test]
    fn cli_parses_all_top_level_groups() {
        let groups: [&[&str]; 5] = [
            &["dbt-nova", "manifest", "load"],
            &["dbt-nova", "tool", "call", "search"],
            &["dbt-nova", "config", "show"],
            &["dbt-nova", "storage", "inspect"],
            &["dbt-nova", "health", "check"],
        ];
        for args in groups {
            let cli = Cli::parse_from(args);
            assert!(cli.command.is_some());
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
