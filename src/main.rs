use clap::Parser;
use dbt_nova::cli::args::Cli;
use dbt_nova::cli::output::exit_code;
use dbt_nova::error::Result;
use tracing::error;
use tracing_subscriber::fmt::format::FmtSpan;

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    let cli = Cli::parse();
    if let Some(command) = cli.command {
        if let Err(error) = dbt_nova::cli::dispatch(command).await {
            error!(error = %error, "cli command failed");
            eprintln!("dbt-nova CLI error: {error}");
            std::process::exit(exit_code(&error));
        }
        return Ok(());
    }

    dbt_nova::cli::server_cmd::start_from_env().await
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
