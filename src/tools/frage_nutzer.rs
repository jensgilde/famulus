//! Werkzeug für Multiple-Choice-Rückfragen an den Nutzer.
//!
//! Blockiert bewusst NICHT auf eine Antwort - das Werkzeug meldet die
//! Frage nur als `AgentEvent::FrageAnNutzer` an die Oberfläche (Telegram
//! rendert daraus anklickbare Buttons, siehe `telegram.rs`) und kehrt
//! sofort zurück. Die eigentliche Antwort kommt als ganz normale, neue
//! Nachricht herein und setzt den Gesprächsverlauf fort - genau wie eine
//! getippte Antwort das schon immer tut. Ein blockierendes Design (auf
//! einen Button-Klick warten, während der Auftrag "hängt") würde die
//! Telegram-Poll-Schleife selbst lahmlegen: die läuft seriell, ein
//! wartender Auftrag würde nie zum nächsten `getUpdates()` kommen, das
//! den Klick überhaupt erst brächte - ein Deadlock mit sich selbst.

use super::Tool;
use crate::llm::ToolDefinition;
use crate::permissions::PermissionManager;
use crate::ui::{AgentEvent, Ui};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

pub struct FrageNutzerTool {
    pub ui: Arc<dyn Ui>,
}

#[async_trait]
impl Tool for FrageNutzerTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "frage_nutzer".to_string(),
            description: "Stellt eine kurze Multiple-Choice-Frage mit 2-4 anklickbaren \
                Antwortoptionen (auf Telegram: Buttons unter der Nachricht). Nutzen, wenn eine \
                klare Entscheidung ansteht, die sich in wenigen kurzen Optionen fassen lässt - \
                nicht für offene Fragen, dafür reicht normaler Text. Die Antwort kommt als \
                eigene, neue Nachricht herein, NICHT als Rückgabewert dieses Aufrufs - nach dem \
                Aufruf nichts mehr behaupten oder tun, den Zug einfach beenden."
                .to_string(),
            parameters_schema: json!({
                "type": "object",
                "properties": {
                    "frage": {
                        "type": "string",
                        "description": "Die Frage, kurz und konkret formuliert"
                    },
                    "optionen": {
                        "type": "array",
                        "items": { "type": "string" },
                        "minItems": 2,
                        "maxItems": 4,
                        "description": "2 bis 4 kurze Antwort-Optionen (Button-Beschriftung, \
                            möglichst unter 20 Zeichen)"
                    }
                },
                "required": ["frage", "optionen"]
            }),
        }
    }

    async fn execute(&self, args: Value, _permissions: &PermissionManager) -> anyhow::Result<String> {
        let frage = args["frage"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("'frage' fehlt"))?
            .trim()
            .to_string();
        if frage.is_empty() {
            anyhow::bail!("'frage' ist leer");
        }

        let optionen: Vec<String> = args["optionen"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("'optionen' fehlt"))?
            .iter()
            .filter_map(|v| v.as_str().map(str::trim).filter(|s| !s.is_empty()))
            .map(str::to_string)
            .collect();
        if !(2..=4).contains(&optionen.len()) {
            anyhow::bail!(
                "'optionen' braucht 2 bis 4 nicht-leere Einträge, hatte {}",
                optionen.len()
            );
        }

        self.ui.ereignis(AgentEvent::FrageAnNutzer {
            frage,
            optionen,
        });

        Ok("Frage mit Optionen wurde an den Nutzer geschickt (als anklickbare Buttons, falls \
            die Oberfläche das kann, sonst als Text mit nummerierten Optionen). Die Antwort \
            kommt als eigene, neue Nachricht - jetzt nichts mehr tun oder behaupten, einfach \
            den Zug beenden."
            .to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct AufzeichnendeUi {
        ereignisse: Mutex<Vec<AgentEvent>>,
    }

    impl Ui for AufzeichnendeUi {
        fn ereignis(&self, ereignis: AgentEvent) {
            self.ereignisse.lock().unwrap().push(ereignis);
        }
    }

    fn permissions() -> PermissionManager {
        let config: crate::config::Config =
            toml::from_str("provider = \"hyper\"").expect("Test-Konfiguration");
        PermissionManager::new(&config)
    }

    #[tokio::test]
    async fn meldet_frage_und_optionen_als_ereignis() {
        let ui = Arc::new(AufzeichnendeUi { ereignisse: Mutex::new(Vec::new()) });
        let werkzeug = FrageNutzerTool { ui: Arc::clone(&ui) as Arc<dyn Ui> };
        let args = json!({ "frage": "Welche Größe?", "optionen": ["Klein", "Groß"] });

        let ergebnis = werkzeug.execute(args, &permissions()).await.unwrap();
        assert!(ergebnis.contains("geschickt"));

        let ereignisse = ui.ereignisse.lock().unwrap();
        assert_eq!(ereignisse.len(), 1);
        match &ereignisse[0] {
            AgentEvent::FrageAnNutzer { frage, optionen } => {
                assert_eq!(frage, "Welche Größe?");
                assert_eq!(optionen, &vec!["Klein".to_string(), "Groß".to_string()]);
            }
            other => panic!("falsches Ereignis: {other:?}"),
        }
    }

    #[tokio::test]
    async fn lehnt_zu_wenige_optionen_ab() {
        let ui = Arc::new(AufzeichnendeUi { ereignisse: Mutex::new(Vec::new()) });
        let werkzeug = FrageNutzerTool { ui };
        let args = json!({ "frage": "Ja oder?", "optionen": ["Nur eine"] });
        assert!(werkzeug.execute(args, &permissions()).await.is_err());
    }

    #[tokio::test]
    async fn lehnt_zu_viele_optionen_ab() {
        let ui = Arc::new(AufzeichnendeUi { ereignisse: Mutex::new(Vec::new()) });
        let werkzeug = FrageNutzerTool { ui };
        let args = json!({ "frage": "Welche?", "optionen": ["a", "b", "c", "d", "e"] });
        assert!(werkzeug.execute(args, &permissions()).await.is_err());
    }
}
