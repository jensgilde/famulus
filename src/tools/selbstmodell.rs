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

        aktualisieren(&self.gedaechtnis, &self.vault_pfad, permissions, neue_erkenntnis).await
    }
}

/// Baut die Selbstbeschreibung neu und schreibt sie in den Vault. Geteilt
/// zwischen dem `selbstmodell`-Werkzeug (aufgerufen vom Modell, ggf. mit
/// `neue_erkenntnis`) und `memory::idle_reflexion()` (alle 6 Stunden im
/// Hintergrund, ohne neue Erkenntnis) - EINE Stelle, die den Text
/// zusammensetzt, damit die beiden Wege nicht auseinanderlaufen und der
/// Docstring von `idle_reflexion()` ("aktualisiert das Selbstmodell") auch
/// wirklich stimmt.
pub async fn aktualisieren(
    gedaechtnis: &Gedaechtnis,
    vault_pfad: &std::path::Path,
    permissions: &PermissionManager,
    neue_erkenntnis: &str,
) -> anyhow::Result<String> {
    let pfad = vault_pfad.join("Wer-ist-Famulus.md");

    // Ohne übergebene Erkenntnis (Werkzeugaufruf ohne Parameter, oder die
    // stille `idle_reflexion()` alle 6 Stunden) die zuletzt festgehaltene
    // "Aktuelle Erkenntnis" aus der bestehenden Datei übernehmen - sonst
    // würde jede Regeneration sie kommentarlos wegwerfen, obwohl niemand
    // etwas Neues gemeldet hat. Das war genau die Lücke, bevor es diese
    // Funktion gab: jeder Aufruf baute den Text komplett neu und verlor
    // dabei, was ein früherer Aufruf unter "Aktuelle Erkenntnis" notiert hatte.
    let neue_erkenntnis = if !neue_erkenntnis.is_empty() {
        neue_erkenntnis.to_string()
    } else {
        letzte_erkenntnis_aus_datei(&pfad).await.unwrap_or_default()
    };
    let neue_erkenntnis = neue_erkenntnis.as_str();

    // ── Daten fürs Selbstmodell sammeln ─────────────────────────
    let statistik = gedaechtnis.provider_statistik().unwrap_or_default();
    let anzahl_erinnerungen = gedaechtnis.anzahl().unwrap_or(0);
    let embeddings_aktiv = if Gedaechtnis::embeddings_verfuegbar().await {
        "aktiv"
    } else {
        "inaktiv (fällt auf FTS5 zurück)"
    };
    // Presets kommen aus presets.toml, nicht als fester Text - ein
    // sechstes Preset soll hier automatisch mitgezählt werden, statt die
    // Selbstbeschreibung lügen zu lassen (real vorgefallen: der Text
    // behauptete "fünf", `PresetsConfig::default()` legt vier an).
    let preset_anzahl = crate::presets::PresetsConfig::load()
        .map(|p| p.presets.len())
        .unwrap_or(0);

    // CHANGELOG lesen (liegt neben der Binary, also im Projektordner)
    let changelog = lese_changelog().await;

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
    text.push_str("- Ich kann zwischen zwei LLM-Providern wählen: Hyper, OpenRouter.\n");
    text.push_str("- Für Chat nutze ich ausschließlich Remote-Modelle (Standard: Hyper/deepseek-v4-flash); für die semantische Gedächtnissuche läuft lokal ein Embedding-Modell über Ollama.\n");
    text.push_str("- Ich habe ein dreistufiges Gedächtnis: FTS5-Volltextsuche, ");
    text.push_str("Embedding-basierte semantische Suche und ein Notizbuch für In-Task-Lernen.\n");
    text.push_str("- Ich kann Chat-Verläufe durchsuchen und archivieren.\n");
    text.push_str(&format!(
        "- Ich habe {preset_anzahl} System-Prompt-Presets, zwischen denen Jens per Dropdown wechseln kann.\n\n"
    ));

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

/// Liest den Abschnitt "## Aktuelle Erkenntnis" aus einer bestehenden
/// Selbstbeschreibung, falls vorhanden - die Vorstufe für den
/// "nichts wegwerfen"-Fallback in `aktualisieren()`. `None` bei fehlender
/// Datei, fehlendem Abschnitt oder leerem Inhalt.
async fn letzte_erkenntnis_aus_datei(pfad: &std::path::Path) -> Option<String> {
    const MARKER: &str = "## Aktuelle Erkenntnis\n\n";
    let inhalt = tokio::fs::read_to_string(pfad).await.ok()?;
    let start = inhalt.find(MARKER)? + MARKER.len();
    let rest = &inhalt[start..];
    let ende = rest.find("\n## ").unwrap_or(rest.len());
    let text = rest[..ende].trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// Das CHANGELOG liegt im Projektordner, nicht im Vault. Wir suchen an den
/// typischen Stellen.
async fn lese_changelog() -> String {
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
            // Nur die letzten 2 Versionen nehmen: dieser Text landet über
            // das Selbstbild in JEDEM System-Prompt (systemvorspann(),
            // Schritt 2) - bei Ollama ohne Prompt-Caching kostet jede
            // zusätzliche Version echte Tokens bei jedem einzelnen
            // Aufruf, nicht nur einmal. War vorher 6 Versionen (bei
            // Wer-ist-Famulus.md zuletzt >200 Zeilen); die vollständige
            // Historie steht ohnehin unverändert in CHANGELOG.md, ein
            // Duplikat davon im Vault bringt nichts außer Kosten.
            const JUENGSTE_VERSIONEN: usize = 2;
            let versionen: Vec<String> = versionen.into_iter().take(JUENGSTE_VERSIONEN).collect();
            if versionen.is_empty() {
                return String::new();
            }
            // Jeder Block endet schon mit der Leerzeile, die im
            // CHANGELOG vor der nächsten Überschrift steht - mit "\n"
            // statt "\n\n" verbinden, sonst entstehen doppelte
            // Leerzeilen zwischen den Versionen.
            let ausschnitt = versionen.join("\n").trim_end().to_string();
            return format!(
                "{ausschnitt}\n\n(Nur die {JUENGSTE_VERSIONEN} jüngsten Versionen - \
                 vollständige Historie in CHANGELOG.md im Projektordner.)"
            );
        }
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        std::env::temp_dir().join(format!(
            "famulus_selbstmodell_{}_{}.md",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ))
    }

    #[tokio::test]
    async fn fehlende_datei_liefert_none() {
        let pfad = temp();
        assert!(letzte_erkenntnis_aus_datei(&pfad).await.is_none());
    }

    #[tokio::test]
    async fn extrahiert_den_abschnitt_bis_zur_naechsten_ueberschrift() {
        let pfad = temp();
        tokio::fs::write(
            &pfad,
            "# Wer ist Famulus?\n\n## Aktuelle Erkenntnis\n\nJens mag kurze Antworten.\n\n## Meine Grenzen\n\n- ...\n",
        )
        .await
        .unwrap();
        assert_eq!(
            letzte_erkenntnis_aus_datei(&pfad).await.as_deref(),
            Some("Jens mag kurze Antworten.")
        );
        std::fs::remove_file(pfad).ok();
    }

    #[tokio::test]
    async fn fehlender_abschnitt_liefert_none() {
        let pfad = temp();
        tokio::fs::write(&pfad, "# Wer ist Famulus?\n\n## Meine Grenzen\n\n- ...\n")
            .await
            .unwrap();
        assert!(letzte_erkenntnis_aus_datei(&pfad).await.is_none());
        std::fs::remove_file(pfad).ok();
    }
}