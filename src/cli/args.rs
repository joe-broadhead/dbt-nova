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
    Call { tool_name: String },
}

#[derive(Debug, Args)]
pub struct ConfigArgs {
    #[command(subcommand)]
    pub command: ConfigCommand,
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    Show,
    Validate,
}

#[derive(Debug, Args)]
pub struct StorageArgs {
    #[command(subcommand)]
    pub command: StorageCommand,
}

#[derive(Debug, Subcommand)]
pub enum StorageCommand {
    Inspect,
    Prune,
    Cleanup,
}

#[derive(Debug, Args)]
pub struct HealthArgs {
    #[command(subcommand)]
    pub command: HealthCommand,
}

#[derive(Debug, Subcommand)]
pub enum HealthCommand {
    Check,
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::{Cli, Command, ManifestCommand, ServerCommand};

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
}
