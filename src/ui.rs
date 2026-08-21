use serde::{Deserialize, Serialize};

/// Alles, was der Agent während eines Auftrags zu berichten hat. Statt direkt
/// zu drucken schickt er diese Ereignisse an die Oberfläche - im Terminal
/// werden daraus Textzeilen, in der GUI Einträge im Fenster.
///
/// `Serialize`, damit die GUI sie unverändert ans Frontend durchreichen kann.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "art", rename_all = "snake_case")]
pub enum AgentEvent {
    /// Auftrag angenommen, Schleife startet.
    Gestartet { provider: String, auftrag: String },
    /// Das Modell hat etwas gesagt, bevor es zum Werkzeug greift - meist,
    /// was es vorhat. Früher fiel dieser Text unter den Tisch.
    Denkt { text: String },
    /// Das Modell will ein Werkzeug benutzen.
    WerkzeugAufruf { name: String, argumente: String },
    /// Das Werkzeug ist durchgelaufen (oder gescheitert - dann steht der
    /// Fehler im Ergebnis).
    WerkzeugErgebnis { name: String, ergebnis: String },
    /// Erinnerungen aus früheren Aufträgen wurden in den Kontext gelegt.
    Erinnert { anzahl: usize },
    /// Der Rückblick hat neue Erkenntnisse ins Gedächtnis geschrieben.
    Gelernt { anzahl: usize },
    /// Das Modell ist fertig und hat eine finale Antwort.
    Fertig { antwort: String },
    /// Der Auftrag ist abgebrochen (Fehler oder Limit erreicht).
    Abgebrochen { fehler: String },
}

/// Die Oberfläche, mit der Famulus spricht. Terminal und GUI implementieren
/// beide dieses Trait - der Agent kennt nur diese eine Methode und weiß
/// nicht, wo seine Ausgabe landet.
///
/// Hier stand früher zusätzlich eine `frage`-Methode für Berechtigungs-
/// Rückfragen. Die ist weg: Famulus fragt nicht mehr, er macht. Wer das
/// zurückhaben will, braucht wieder ein `Ask` in `permissions::Decision` -
/// ohne das hätte eine Rückfrage niemanden, der sie auslöst.
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
            AgentEvent::Gestartet { provider, auftrag } => {
                println!("{}", format!("→ Auftrag an {provider}: {auftrag}").cyan());
            }
            AgentEvent::Denkt { text } => {
                println!("{}", format!("  … {text}").dimmed());
            }
            AgentEvent::WerkzeugAufruf { name, argumente } => {
                println!("{}", format!("  ⚙ {name}({argumente})").yellow());
            }
            AgentEvent::Erinnert { anzahl } => {
                println!(
                    "{}",
                    format!("  ⌾ {anzahl} Erinnerungen im Kontext").dimmed()
                );
            }
            AgentEvent::Gelernt { anzahl } => {
                println!("{}", format!("  ✎ {anzahl} neu gemerkt").dimmed());
            }
            AgentEvent::WerkzeugErgebnis { .. } => {
                // Im Terminal absichtlich still: die Ergebnisse sind oft
                // seitenlang und stehen ohnehin in der finalen Antwort.
                // Die GUI kann sie ausklappbar anzeigen.
            }
            AgentEvent::Fertig { .. } => {
                println!("{}", "✔ Fertig".green().bold());
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
                AgentEvent::Gestartet {
                    provider: "hyper".into(),
                    auftrag: "tu was".into(),
                },
                "gestartet",
            ),
            (
                AgentEvent::Denkt {
                    text: "ich schau mal nach".into(),
                },
                "denkt",
            ),
            (
                AgentEvent::WerkzeugAufruf {
                    name: "run_shell".into(),
                    argumente: "{}".into(),
                },
                "werkzeug_aufruf",
            ),
            (
                AgentEvent::WerkzeugErgebnis {
                    name: "run_shell".into(),
                    ergebnis: "ok".into(),
                },
                "werkzeug_ergebnis",
            ),
            (AgentEvent::Erinnert { anzahl: 3 }, "erinnert"),
            (AgentEvent::Gelernt { anzahl: 1 }, "gelernt"),
            (
                AgentEvent::Fertig {
                    antwort: "fertig".into(),
                },
                "fertig",
            ),
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
        let json = serde_json::to_value(AgentEvent::WerkzeugAufruf {
            name: "read_file".into(),
            argumente: "{\"path\":\"x\"}".into(),
        })
        .unwrap();
        assert_eq!(json["name"], "read_file");
        assert_eq!(json["argumente"], "{\"path\":\"x\"}");

        let json = serde_json::to_value(AgentEvent::Fertig {
            antwort: "hallo".into(),
        })
        .unwrap();
        assert_eq!(json["antwort"], "hallo");
    }
}
