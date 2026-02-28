use clap::Parser;
use dbt_nova::cli::args::{Cli, Command};
use dbt_nova::cli::output::exit_code;
use dbt_nova::error::Result;
use tracing::error;
use tracing_subscriber::fmt::format::FmtSpan;

enum LaunchTarget {
    Command(Command),
    Server,
}

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    match launch_target(Cli::parse()) {
        LaunchTarget::Command(command) => {
            if let Err(dispatch_error) = dbt_nova::cli::dispatch(command).await {
                error!(error = %dispatch_error.error, "cli command failed");
                if !dispatch_error.rendered {
                    eprintln!("dbt-nova CLI error: {}", dispatch_error.error);
                }
                std::process::exit(exit_code(&dispatch_error.error));
            }
            Ok(())
        }
        LaunchTarget::Server => dbt_nova::cli::server_cmd::start_from_env().await,
    }
}

fn launch_target(cli: Cli) -> LaunchTarget {
    if let Some(command) = cli.command {
        LaunchTarget::Command(command)
    } else {
        LaunchTarget::Server
    }
}

fn init_tracing() {
    let filter = std::env::var("DBT_NOVA_LOG")
        .or_else(|_| std::env::var("RUST_LOG"))
        .ok();
    if let Some(filter) = filter
        && let Err(err) = tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(std::io::stderr)
            .with_target(false)
            .with_span_events(FmtSpan::CLOSE)
            .try_init()
    {
        tracing::warn!(error = %err, "failed to initialize tracing subscriber");
    }
}

#[cfg(test)]
mod tests {
    use super::{LaunchTarget, launch_target};
    use clap::Parser;
    use dbt_nova::cli::args::Cli;

    #[test]
    fn no_subcommand_falls_back_to_server_launch() {
        let cli = Cli::parse_from(["dbt-nova"]);
        assert!(matches!(launch_target(cli), LaunchTarget::Server));
    }

    #[test]
    fn explicit_subcommand_routes_to_cli_dispatch() {
        let cli = Cli::parse_from(["dbt-nova", "health", "check"]);
        assert!(matches!(launch_target(cli), LaunchTarget::Command(_)));
    }
}
