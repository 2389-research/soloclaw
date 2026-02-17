// ABOUTME: Entry point for simpleclaw — a TUI agent with layered tool approval.
// ABOUTME: Parses CLI args, loads config, and launches the app.

use clap::Parser;
use simpleclaw::{app, config};

/// TUI agent with layered tool approval.
#[derive(Parser)]
#[command(name = "simpleclaw", about = "TUI agent with layered tool approval")]
struct Cli {
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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let mut config = config::Config::load()?;

    // Apply CLI overrides.
    if let Some(provider) = cli.provider {
        config.llm.provider = provider;
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
