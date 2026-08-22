use serde::{Deserialize, Serialize};

/// Alles, was der Agent während eines Auftrags zu berichten hat. Statt direkt
/// zu drucken schickt er diese Ereignisse an die Oberfläche - im Terminal
/// werden daraus Textzeilen, in der GUI Einträge im Fenster.
///
/// `Serialize`, damit die GUI sie unverändert ans Frontend durchreichen kann.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "art", rename_all = "snake_case")]
pub enum AgentEvent {
    /// Ein Stück Text vom Modell (Streaming oder Antwort).
    Text { chunk: String },
    /// Ein Werkzeug-Aufruf beginnt.
    ToolStart { name: String, args: serde_json::Value },
    /// Ein Werkzeug-Aufruf ist beendet (Ergebnis oder Fehler).
    ToolEnd { name: String, inhalt: String },
    /// Erinnerungen aus früheren Aufträgen wurden in den Kontext gelegt.
    Erinnert { anzahl: usize },
    /// Eine neue Erkenntnis wurde ins Gedächtnis geschrieben.
    Gemmerkt { kategorie: String, inhalt: String },
    /// Der Rückblick läuft (Notizbuch-Konsolidierung).
    Reflektiere,
    /// Der Auftrag ist fertig (finale Antwort wurde schon via Text gesendet).
    Fertig,
    /// Der Auftrag ist abgebrochen (Fehler oder Limit erreicht).
    Abgebrochen { fehler: String },
}

/// Die Oberfläche, mit der Famulus spricht. Terminal und GUI implementieren
/// beide dieses Trait - der Agent kennt nur diese eine Methode und weiß
/// nicht, wo seine Ausgabe landet.
pub trait Ui: Send + Sync {
    /// Meldet ein Ereignis. Darf nicht blockieren.
    fn ereignis(&self, ereignis: AgentEvent);
}

/// Oberfläche für das Kommandozeilen-Famulus: Ausgabe nach stdout.
pub struct TerminalUi;

impl Ui for TerminalUi {
    fn ereignis(&self, ereignis: AgentEvent) {
        use colored::Colorize;

        match ereignis {
            AgentEvent::Text { chunk } => {
                print!("{}", chunk);
            }
            AgentEvent::ToolStart { name, args } => {
                println!("{}", format!("  ⚙ {name}({args})").yellow());
            }
            AgentEvent::ToolEnd { name, inhalt: _ } => {
                // Im Terminal absichtlich leise – Ergebnisse sind oft
                // seitenlang und stehen ohnehin in der finalen Antwort.
                let _ = name;
            }
            AgentEvent::Erinnert { anzahl } => {
                println!(
                    "{}",
                    format!("  ⌾ {anzahl} Erinnerungen im Kontext").dimmed()
                );
            }
            AgentEvent::Gemmerkt { kategorie, inhalt } => {
                println!("{}", format!("  ✎ ({kategorie}) {inhalt}").dimmed());
            }
            AgentEvent::Reflektiere => {
                println!("{}", "  ⟳ Rückblick...".dimmed());
            }
            AgentEvent::Fertig => {
                println!("\n{}", "✔ Fertig".green().bold());
            }
            AgentEvent::Abgebrochen { fehler } => {
                eprintln!("\n{} {fehler}", "✗ Fehler:".red().bold());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Das Frontend (`ui/index.html`) verzweigt über `ereignis.art`. Weicht
    /// die Kodierung hier ab, kommt zwar alles im Fenster an, aber kein
    /// einziger Zweig greift - und das Fenster bleibt still, ohne dass
    /// irgendwo ein Fehler auftaucht. Genau deshalb steht der Vertrag hier
    /// als Test und nicht nur als Kommentar.
    #[test]
    fn ereignis_kodierung_passt_zum_frontend() {
        let faelle = vec![
            (
                AgentEvent::Text {
                    chunk: "hallo".into(),
                },
                "text",
            ),
            (
                AgentEvent::ToolStart {
                    name: "run_shell".into(),
                    args: serde_json::json!({}),
                },
                "tool_start",
            ),
            (
                AgentEvent::ToolEnd {
                    name: "run_shell".into(),
                    inhalt: "ok".into(),
                },
                "tool_end",
            ),
            (AgentEvent::Erinnert { anzahl: 3 }, "erinnert"),
            (
                AgentEvent::Gemmerkt {
                    kategorie: "fakt".into(),
                    inhalt: "x".into(),
                },
                "gemmerkt",
            ),
            (AgentEvent::Reflektiere, "reflektiere"),
            (AgentEvent::Fertig, "fertig"),
            (
                AgentEvent::Abgebrochen {
                    fehler: "kaputt".into(),
                },
                "abgebrochen",
            ),
        ];

        for (ereignis, erwartet) in faelle {
            let json = serde_json::to_value(&ereignis).expect("muss serialisierbar sein");
            assert_eq!(
                json["art"], erwartet,
                "Frontend erwartet art=\"{erwartet}\", bekam {json}"
            );
        }
    }

    /// Die Feldnamen, die das Fenster ausliest, müssen ebenfalls stimmen.
    #[test]
    fn ereignis_felder_heissen_wie_im_frontend() {
        let json = serde_json::to_value(AgentEvent::ToolStart {
            name: "read_file".into(),
            args: serde_json::json!({"path": "x"}),
        })
        .unwrap();
        assert_eq!(json["name"], "read_file");

        let json = serde_json::to_value(AgentEvent::Text {
            chunk: "hallo".into(),
        })
        .unwrap();
        assert_eq!(json["chunk"], "hallo");
    }
}