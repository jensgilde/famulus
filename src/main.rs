use clap::Parser;
use colored::Colorize;
use famulus_core::agent::Agent;
use famulus_core::config::Config;
use famulus_core::llm;
use famulus_core::ui::{TerminalUi, Ui};
use std::sync::Arc;

/// Famulus - ein persönlicher KI-Agent mit vollem Rechnerzugriff und
/// austauschbarem LLM-Backend (Charm Hyper oder OpenRouter).
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

    // Die CLI ist Einzelauftrag-Batch, kein interaktives Gespräch nebenher -
    // niemand sendet hier Zwischenfragen. Sender sofort fallen lassen statt
    // offenzuhalten: ein leerer, geschlossener Kanal macht try_recv() in
    // run_task() zu einem reinen No-Op.
    let (_zf_tx, zf_rx) = tokio::sync::mpsc::unbounded_channel();
    let ergebnis = agent.run_task(&[], &cli.task, zf_rx).await;
    zeige_provider_statistik();

    match ergebnis {
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

/// Zeigt die Router-Scorecard, falls schon Daten da sind. Rein informativ -
/// ein Fehler beim Lesen (z.B. Gedächtnis nicht verfügbar) bricht den
/// Auftrag nicht ab, das ist ja schon durchgelaufen.
fn zeige_provider_statistik() {
    let Ok(gedaechtnis) = famulus_core::memory::Gedaechtnis::standard() else {
        return;
    };
    let Ok(statistik) = gedaechtnis.provider_statistik() else {
        return;
    };
    if statistik.is_empty() {
        return;
    }
    println!("\n{}", "Provider-Statistik:".dimmed());
    for s in statistik {
        println!(
            "  {}",
            format!(
                "{}: {} Aufrufe, {:.0}% erfolgreich, ⌀ {:.0}ms",
                s.provider,
                s.anzahl,
                s.erfolgsquote * 100.0,
                s.durchschnitt_ms
            )
            .dimmed()
        );
    }
}