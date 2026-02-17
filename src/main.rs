// ABOUTME: Entry point for soloclaw — a TUI agent with layered tool approval.
// ABOUTME: Parses CLI args, loads config, and launches the app.

use clap::Parser;
use soloclaw::{app, config};

/// TUI agent with layered tool approval.
#[derive(Parser)]
#[command(name = "soloclaw", about = "TUI agent with layered tool approval")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// LLM provider (anthropic, openai, gemini, openrouter, ollama).
    #[arg(long)]
    provider: Option<String>,

    /// Model name to use.
    #[arg(long)]
    model: Option<String>,

    /// Default security level (deny, allowlist, full).
    #[arg(long)]
    security: Option<String>,
}

#[derive(clap::Subcommand)]
enum Command {
    /// Initialize XDG config and secrets for soloclaw.
    Setup,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    if matches!(cli.command, Some(Command::Setup)) {
        return config::run_setup();
    }

    let mut config = config::Config::load()?;

    // Apply CLI overrides.
    if let Some(provider) = cli.provider {
        config.llm.provider = provider;
        if cli.model.is_none() {
            config.llm.model = config::default_model_for_provider(&config.llm.provider).to_string();
        }
    }
    if let Some(model) = cli.model {
        config.llm.model = model;
    }
    if let Some(security) = cli.security {
        config.approval.security = security;
    }

    let app = app::App::new(config);
    app.run().await
}
