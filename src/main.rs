use clap::Parser;
use colored::Colorize;
use famulus_core::agent::Agent;
use famulus_core::config::Config;
use famulus_core::llm;
use famulus_core::ui::{TerminalUi, Ui};
use std::sync::Arc;

/// Famulus - ein persönlicher KI-Agent mit vollem Rechnerzugriff und
/// austauschbarem LLM-Backend (Charm Hyper, Anthropic Claude oder xAI Grok).
#[derive(Parser)]
#[command(name = "famulus", version)]
struct Cli {
    /// Der Auftrag, den Famulus erledigen soll
    task: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let config = Config::load()?;
    println!(
        "{}",
        format!(
            "Famulus startet mit Provider '{}' (max. {} Schritte)",
            config.provider, config.max_turns
        )
        .dimmed()
    );

    let ui: Arc<dyn Ui> = Arc::new(TerminalUi);
    let provider = llm::build_provider(&config)?;
    let agent = Agent::new(config, provider, Arc::clone(&ui)).await;

    match agent.run_task(&[], &cli.task).await {
        Ok(()) => {
            println!("\n{}", "── Fertig ──".bold());
        }
        Err(e) => {
            eprintln!("\n{} {e}", "✗ Fehler:".red().bold());
            std::process::exit(1);
        }
    }

    Ok(())
}