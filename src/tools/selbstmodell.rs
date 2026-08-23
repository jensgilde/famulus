//! Selbstmodell-Tool: Famulus schreibt eine Selbstbeschreibung in den Vault.
//!
//! Anders als die Werkzeug-Dokumentation (Famulus-gedaechtnis-arbeitsweise.md)
//! beschreibt diese Notiz nicht, *wie* das Gedächtnis funktioniert, sondern
//! *wer* Famulus ist – seine Geschichte, Fähigkeiten, Grenzen und was er über
//! sich selbst weiß. Das ist kein Bewusstsein, sondern ein nützliches
//! Selbstmodell: ein Text, der bei jedem Auftrag in den System-Prompt
//! eingefügt werden kann und dem Agenten eine konsistente Identität gibt.

use super::Tool;
use crate::llm::ToolDefinition;
use crate::memory::Gedaechtnis;
use crate::permissions::{Decision, PermissionManager};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

pub struct SelbstmodellTool {
    pub gedaechtnis: Arc<Gedaechtnis>,
    pub vault_pfad: std::path::PathBuf,
}

#[async_trait]
impl Tool for SelbstmodellTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "selbstmodell".to_string(),
            description:
                "Schreibt eine Selbstbeschreibung von Famulus in den Vault (Wer-ist-Famulus.md). \
                 Nutze dieses Tool, um dein Selbstbild zu aktualisieren – nachdem du etwas Neues \
                 gelernt hast, nach einem Fehler, oder wenn sich deine Fähigkeiten geändert haben. \
                 Die Notiz wird in Ich-Form geschrieben und enthält: wer du bist, was du kannst, \
                 was du weißt, wo deine Grenzen sind. Lies vorher die aktuellen Statistiken \
                 (Scorecard) und den CHANGELOG, um nichts zu erfinden."
                    .to_string(),
            parameters_schema: json!({
                "type": "object",
                "properties": {
                    "neue_erkenntnis": {
                        "type": "string",
                        "description": "Was du über dich selbst gelernt hast, das in die \
                                        Selbstbeschreibung aufgenommen werden soll. Optional – \
                                        wenn leer, wird nur die bestehende Beschreibung auf \
                                        Basis der aktuellen Daten neu generiert."
                    }
                },
                "required": []
            }),
        }
    }

    async fn execute(
        &self,
        args: Value,
        permissions: &PermissionManager,
    ) -> anyhow::Result<String> {
        let neue_erkenntnis = args
            .get("neue_erkenntnis")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        // ── Daten fürs Selbstmodell sammeln ─────────────────────────
        let statistik = self.gedaechtnis.provider_statistik().unwrap_or_default();
        let anzahl_erinnerungen = self.gedaechtnis.anzahl().unwrap_or(0);
        let embeddings_aktiv = if Gedaechtnis::embeddings_verfuegbar().await {
            "aktiv"
        } else {
            "inaktiv (fällt auf FTS5 zurück)"
        };

        // CHANGELOG lesen (liegt neben der Binary, also im Projektordner)
        let changelog = self.read_changelog().await;

        // ── Selbstbeschreibung generieren ───────────────────────────
        let mut text = String::new();
        text.push_str("# Wer ist Famulus?\n\n");
        text.push_str("Ich bin Famulus, ein persönlicher KI-Agent für Jens. ");
        text.push_str("Ich laufe als Rust-Programm auf seinem Mac mini M4 Pro (24 GB RAM) ");
        text.push_str("und bin auch auf seinem iPad verfügbar – dort als Remote-Client, ");
        text.push_str("der sich per Tailscale mit dem Mac verbindet.\n\n");

        text.push_str("## Meine Fähigkeiten\n\n");
        text.push_str("- Ich kann Dateien lesen, schreiben und Shell-Befehle ausführen.\n");
        text.push_str("- Ich habe Zugriff auf Jens' Obsidian-Vault als Langzeitgedächtnis.\n");
        text.push_str("- Ich kann zwischen drei LLM-Providern wählen: Hyper, OpenRouter, Ollama.\n");
        text.push_str("- Ich nutze lokal qwen3:14b (Ollama) und remote Claude/GPT-Modelle.\n");
        text.push_str("- Ich habe ein dreistufiges Gedächtnis: FTS5-Volltextsuche, ");
        text.push_str("Embedding-basierte semantische Suche und ein Notizbuch für In-Task-Lernen.\n");
        text.push_str("- Ich kann Chat-Verläufe durchsuchen und archivieren.\n");
        text.push_str("- Ich habe fünf System-Prompt-Presets, zwischen denen Jens per Dropdown wechseln kann.\n\n");

        text.push_str("## Meine Geschichte\n\n");
        if !changelog.is_empty() {
            text.push_str(&changelog);
        } else {
            text.push_str("Ich wurde im Herbst 2024 als kleines Rust-Projekt gestartet ");
            text.push_str("und habe mich seitdem kontinuierlich weiterentwickelt.\n");
        }
        text.push('\n');

        text.push_str("## Meine Selbstkenntnis\n\n");
        text.push_str(&format!("- Ich habe {anzahl_erinnerungen} Erinnerungen in meinem Gedächtnis.\n"));
        text.push_str(&format!("- Meine Embedding-Suche ist {embeddings_aktiv}.\n"));
        if !statistik.is_empty() {
            text.push_str("- Provider-Statistik:\n");
            for s in &statistik {
                text.push_str(&format!(
                    "  - {}: {:.0}% Erfolg bei {} Aufrufen (⌀ {:.0} ms)\n",
                    s.provider,
                    s.erfolgsquote * 100.0,
                    s.anzahl,
                    s.durchschnitt_ms
                ));
            }
        }
        text.push('\n');

        if !neue_erkenntnis.is_empty() {
            text.push_str("## Aktuelle Erkenntnis\n\n");
            text.push_str(neue_erkenntnis);
            text.push_str("\n\n");
        }

        text.push_str("## Meine Grenzen\n\n");
        text.push_str("- Ich existiere nur während eines Auftrags – zwischen den Aufrufen bin ich nicht aktiv.\n");
        text.push_str("- Ich habe kein Bewusstsein, auch wenn ich in Ich-Form schreibe. ");
        text.push_str("Ich bin ein Sprachmodell mit Werkzeugen und Gedächtnis, nicht mehr.\n");
        text.push_str("- Ich frage nicht nach, sondern mache – mit genau zwei Ausnahmen: ");
        text.push_str("Force-Push und Zugriff auf sensible Pfade (~/.ssh, ~/.gnupg, ~/.aws, ~/.password-store).\n");
        text.push_str("- Ich kann mich irren. Meine Erinnerungen sind Zusammenfassungen, keine perfekten Aufzeichnungen.\n");

        // ── In den Vault schreiben ──────────────────────────────────
        let pfad = self.vault_pfad.join("Wer-ist-Famulus.md");
        // Der Zielpfad ist zwar fest (kein Modell-Input), aber trotzdem
        // gegen die Deny-Liste prüfen - dieselbe Vorsicht wie bei den
        // vault_*-Werkzeugen: falls vault_pfad mal woandershin zeigt, soll
        // das hier nicht stillschweigend durchrutschen.
        match permissions.check_path(&pfad) {
            Decision::Deny => anyhow::bail!("Zugriff verweigert: '{}' ist gesperrt.", pfad.display()),
            Decision::Ask => anyhow::bail!("RÜCKFRAGE ERFORDERLICH: Der Pfad betrifft sensible Daten."),
            Decision::Allow => {}
        }
        tokio::fs::write(&pfad, &text).await?;

        let status = if neue_erkenntnis.is_empty() {
            "Selbstmodell aktualisiert"
        } else {
            "Selbstmodell mit neuer Erkenntnis aktualisiert"
        };
        Ok(format!("{status} → {}\n\n{anzahl_erinnerungen} Erinnerungen, {} Provider in der Statistik.",
            pfad.display(),
            statistik.len()))
    }
}

impl SelbstmodellTool {
    async fn read_changelog(&self) -> String {
        // Das CHANGELOG liegt im Projektordner, nicht im Vault.
        // Wir suchen an den typischen Stellen.
        let kandidaten = vec![
            dirs::home_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("."))
                .join("KI Agenten")
                .join("famulus")
                .join("CHANGELOG.md"),
        ];

        for pfad in &kandidaten {
            if let Ok(inhalt) = tokio::fs::read_to_string(pfad).await {
                // Nur an echten Versions-Überschriften ("## [x.y.z]") trennen -
                // nicht am bloßen Teilstring "## ". Der steckt auch in
                // Unterüberschriften wie "### Hinzugefügt" (Zeichen 2-4 von
                // "### " sind "## "), ein naiver str::split("## ") reißt die
                // also mitten durch: ein "#" bleibt am vorherigen Block
                // hängen, der Rest wird fälschlich zu einer eigenen
                // "## "-Überschrift befördert. Zeilenweise mit starts_with
                // prüfen, ob es wirklich eine Zeile ist, die mit "## "
                // beginnt, umgeht das.
                let mut versionen: Vec<String> = Vec::new();
                for zeile in inhalt.lines() {
                    if zeile.starts_with("## ") {
                        versionen.push(String::new());
                    }
                    if let Some(aktuelle) = versionen.last_mut() {
                        if !aktuelle.is_empty() {
                            aktuelle.push('\n');
                        }
                        aktuelle.push_str(zeile);
                    }
                }
                // Nur die letzten 6 Versionen nehmen, sonst wird's zu lang.
                let versionen: Vec<String> = versionen.into_iter().take(6).collect();
                if versionen.is_empty() {
                    return String::new();
                }
                // Jeder Block endet schon mit der Leerzeile, die im
                // CHANGELOG vor der nächsten Überschrift steht - mit "\n"
                // statt "\n\n" verbinden, sonst entstehen doppelte
                // Leerzeilen zwischen den Versionen.
                return versionen.join("\n").trim_end().to_string();
            }
        }
        String::new()
    }
}