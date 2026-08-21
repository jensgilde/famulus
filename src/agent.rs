use crate::config::Config;
use crate::llm::{LlmProvider, Message, ToolResult};
use crate::memory::{Gedaechtnis, ART_FAKT, ART_LEKTION, ART_PRAEFERENZ};
use crate::presets::PresetsConfig;
use crate::permissions::PermissionManager;
use crate::tools::{all_tools, Tool};
use crate::ui::{AgentEvent, Ui};
use std::collections::HashMap;
use std::sync::Arc;

/// Wie viele Zeichen eines Werkzeug-Ergebnisses in den Kontext dürfen.
///
/// Der Deckel ist wichtiger, als er aussieht: Jedes Ergebnis bleibt bis zum
/// Ende des Auftrags im Verlauf und wird in JEDEM weiteren Zug erneut
/// mitgeschickt. Ein einziges `cargo build` ohne Deckel kostet bei zwanzig
/// Zügen also zwanzig Mal seine eigene Länge - an Geld und an Wartezeit.
const MAX_ERGEBNIS_ZEICHEN: usize = 8_000;

pub struct Agent {
    provider: Box<dyn LlmProvider>,
    tools: HashMap<String, Box<dyn Tool>>,
    permissions: PermissionManager,
    ui: Arc<dyn Ui>,
    /// Optional: Fällt die Datenbank aus, arbeitet Famulus ohne Gedächtnis
    /// weiter. Ein kaputtes Gedächtnis darf den Agenten nicht lahmlegen.
    gedaechtnis: Option<Arc<Gedaechtnis>>,
    max_turns: u32,
    max_erinnerungen: usize,
    reflexion: bool,
    hat_vault: bool,
}

impl Agent {
    pub fn new(config: Config, provider: Box<dyn LlmProvider>, ui: Arc<dyn Ui>) -> Self {
        let tools: HashMap<String, Box<dyn Tool>> = all_tools(&config)
            .into_iter()
            .map(|t| (t.definition().name.clone(), t))
            .collect();

        // Das Gedächtnis wird hier geöffnet, nicht vom Aufrufer - so bekommen
        // Kommandozeile und GUI es ohne doppelten Code, und es gibt nur einen
        // Ort, an dem der Datenbankpfad steht.
        let gedaechtnis = match Gedaechtnis::standard() {
            Ok(g) => Some(Arc::new(g)),
            Err(e) => {
                ui.ereignis(AgentEvent::Abgebrochen {
                    fehler: format!("Gedächtnis nicht verfügbar, arbeite ohne: {e:#}"),
                });
                None
            }
        };

        Self {
            provider,
            tools,
            permissions: PermissionManager::new(&config),
            ui,
            gedaechtnis,
            max_turns: config.max_turns,
            max_erinnerungen: config.max_erinnerungen,
            reflexion: config.reflexion,
            hat_vault: config.vault_pfad().is_some(),
        }
    }

    /// Baut den Vorspann: was Famulus über Jens weiß, und wie er mit dem
    /// Vault umgehen soll.
    ///
    /// Das landet jetzt im System-Teil der Anfrage und nicht mehr als
    /// getarnte Nutzer-Nachricht. Zwei Gründe: Es ist inhaltlich Anweisung
    /// und nicht Gesprächsbeitrag, und es steht damit an der einzigen
    /// Stelle, die sich während eines Auftrags nicht ändert - genau das,
    /// was ein Anbieter zwischenspeichern kann.
    fn systemvorspann(&self, auftrag: &str) -> Option<String> {
        let mut teile = Vec::new();

        // 1. Aktives Preset (vom Nutzer gewähltes System-Prompt) – kommt
        //    als erstes, damit es die Rolle definiert, bevor Gedächtnis und
        //    Vault-Anweisungen folgen.
        if let Ok(presets) = PresetsConfig::load() {
            if let Some(prompt) = presets.aktiver_prompt() {
                teile.push(prompt.to_string());
            }
        }

        // 2. Gedächtnis: was Famulus aus früheren Aufträgen über Jens weiß.
        if let Some(g) = &self.gedaechtnis {
            if let Ok(erinnerungen) = g.relevante(auftrag, self.max_erinnerungen) {
                if !erinnerungen.is_empty() {
                    self.ui.ereignis(AgentEvent::Erinnert {
                        anzahl: erinnerungen.len(),
                    });
                    let liste: Vec<String> = erinnerungen
                        .iter()
                        .map(|e| format!("- ({}) {}", e.art, e.inhalt))
                        .collect();
                    teile.push(format!(
                        "Was du aus früheren Aufträgen weißt:\n{}",
                        liste.join("\n")
                    ));
                }
            }
        }

        // 3. Vault-Anweisungen (nur wenn ein Vault-Pfad konfiguriert ist).
        if self.hat_vault {
            teile.push(
                "Du hast einen Obsidian-Vault als Langzeitgedächtnis über Jens. \
                 Vor nicht-trivialer Hilfe: mit vault_liste schauen, was schon bekannt ist, \
                 und Passendes mit vault_lesen öffnen. Lernst du etwas Dauerhaftes über Jens, \
                 seine Projekte oder Ziele, schreib es mit vault_notiz dorthin - bestehende \
                 Notizen ergänzen statt Dubletten anlegen, Wikilinks [[so]] benutzen, bei \
                 Unsicherheit nach 00-Inbox/. Niemals Geheimnisse in den Vault: keine \
                 Passwörter, API-Keys oder Tokens."
                    .to_string(),
            );
        }

        (!teile.is_empty()).then(|| teile.join("\n\n"))
    }

    /// Führt einen Auftrag vollständig aus - läuft die Beobachten-Denken-
    /// Handeln-Schleife, bis das Modell eine finale Text-Antwort gibt oder
    /// max_turns erreicht ist (Sicherheitsnetz gegen Endlosschleifen).
    ///
    /// `vorherige_nachrichten` ist der bisherige Gesprächsverlauf des Chats,
    /// in dem dieser Auftrag steht - leer für einen neuen Chat oder das CLI
    /// (das kein Chat-Konzept kennt). Famulus selbst hält keinen Verlauf
    /// zwischen Aufrufen fest, das übernimmt die Oberfläche (siehe
    /// `ui/index.html`, `localStorage`) - der Agent bekommt ihn bei jedem
    /// Auftrag frisch mitgegeben.
    pub async fn run_task(
        &self,
        vorherige_nachrichten: &[Message],
        task: &str,
    ) -> anyhow::Result<String> {
        self.ui.ereignis(AgentEvent::Gestartet {
            provider: self.provider.name().to_string(),
            auftrag: task.to_string(),
        });

        let ergebnis = self.auftrag_ausfuehren(vorherige_nachrichten, task).await;

        // Protokoll und Rückblick laufen NACH dem Auftrag und dürfen sein
        // Ergebnis nicht mehr verändern - auch nicht, wenn sie selbst
        // scheitern. Ein misslungener Rückblick ist ärgerlich, aber kein
        // Grund, eine erledigte Arbeit als Fehler zu melden.
        if let Some(g) = &self.gedaechtnis {
            let (text, erfolg) = match &ergebnis {
                Ok(t) => (t.clone(), true),
                Err(e) => (format!("{e:#}"), false),
            };
            if let Err(e) = g.auftrag_protokollieren(task, &text, erfolg) {
                eprintln!("[famulus] Auftrag nicht protokolliert: {e:#}");
            }
            if self.reflexion && erfolg {
                self.rueckblick(task, &text).await;
            }
        }

        ergebnis
    }

    /// Die eigentliche Beobachten-Denken-Handeln-Schleife.
    async fn auftrag_ausfuehren(
        &self,
        vorherige_nachrichten: &[Message],
        task: &str,
    ) -> anyhow::Result<String> {
        let system = self.systemvorspann(task);
        let tool_defs: Vec<_> = self.tools.values().map(|t| t.definition()).collect();
        let mut messages = vorherige_nachrichten.to_vec();
        messages.push(Message::User(task.to_string()));

        for turn in 0..self.max_turns {
            let antwort = self
                .provider
                .next(system.as_deref(), &messages, &tool_defs)
                .await?;

            if antwort.ist_fertig() {
                self.ui.ereignis(AgentEvent::Fertig {
                    antwort: antwort.text.clone(),
                });
                return Ok(antwort.text);
            }

            // Sagt das Modell etwas, bevor es zum Werkzeug greift, ist das
            // für Jens interessant - und es geht nicht mehr verloren.
            if !antwort.text.trim().is_empty() {
                self.ui.ereignis(AgentEvent::Denkt {
                    text: antwort.text.clone(),
                });
            }

            // Der geplante Zug kommt als das in den Verlauf, was er ist:
            // eine Assistant-Nachricht mit echten Werkzeug-Aufrufen. Früher
            // stand hier ein nachgebauter Textschnipsel - das Modell konnte
            // seine eigenen Aufrufe damit nicht sauber wiedererkennen.
            messages.push(Message::Assistant {
                text: antwort.text.clone(),
                tool_calls: antwort.tool_calls.clone(),
            });

            let mut ergebnisse = Vec::new();
            for call in &antwort.tool_calls {
                self.ui.ereignis(AgentEvent::WerkzeugAufruf {
                    name: call.name.clone(),
                    argumente: call.arguments.to_string(),
                });

                let (inhalt, fehler) = match self.tools.get(&call.name) {
                    Some(tool) => match tool
                        .execute(call.arguments.clone(), &self.permissions)
                        .await
                    {
                        Ok(text) => (text, false),
                        Err(e) => (format!("FEHLER: {e}"), true),
                    },
                    None => (format!("FEHLER: unbekanntes Tool '{}'", call.name), true),
                };

                self.ui.ereignis(AgentEvent::WerkzeugErgebnis {
                    name: call.name.clone(),
                    ergebnis: inhalt.clone(),
                });

                ergebnisse.push(ToolResult {
                    call_id: call.id.clone(),
                    // Die Oberfläche bekommt oben das volle Ergebnis; nur
                    // was ins Modell wandert, wird gedeckelt.
                    inhalt: kuerzen(&inhalt, MAX_ERGEBNIS_ZEICHEN),
                    fehler,
                });
            }

            // Alle Ergebnisse eines Zuges in EINER Nachricht - siehe die
            // Anmerkung an `Message::ToolResults`.
            messages.push(Message::ToolResults(ergebnisse));

            if turn == self.max_turns - 1 {
                anyhow::bail!(
                    "Maximale Anzahl an Schritten ({}) erreicht, ohne dass das Modell fertig war. Aufgabe evtl. zu groß oder Modell dreht sich im Kreis.",
                    self.max_turns
                );
            }
        }

        unreachable!()
    }

    /// Schaut nach getaner Arbeit zurück und merkt sich, was dauerhaft
    /// nützlich ist.
    ///
    /// Das ist der Unterschied zwischen einem Agenten, der jedes Mal bei null
    /// anfängt, und einem, der besser wird. Bewusst ein eigener Aufruf OHNE
    /// Werkzeuge: Der Rückblick soll nachdenken, nicht nochmal handeln.
    async fn rueckblick(&self, auftrag: &str, ergebnis: &str) {
        let Some(gedaechtnis) = &self.gedaechtnis else {
            return;
        };

        // Lange Ergebnisse kürzen - der Rückblick soll nicht teurer werden
        // als der Auftrag selbst.
        let gekuerzt = kuerzen(ergebnis, 2_000);

        let frage = format!(
            "Du hast gerade einen Auftrag erledigt. Blick kurz zurück.\n\n\
             AUFTRAG:\n{auftrag}\n\nERGEBNIS:\n{gekuerzt}\n\n\
             Gibt es daraus etwas, das dauerhaft wert ist, gemerkt zu werden? Kategorien:\n\
             - \"{ART_PRAEFERENZ}\": wie Jens Dinge haben will\n\
             - \"{ART_FAKT}\": wie sein System oder seine Projekte beschaffen sind\n\
             - \"{ART_LEKTION}\": was schiefging und wie man es künftig vermeidet\n\n\
             Strenge Regeln:\n\
             - Höchstens 3 Einträge, lieber keinen als einen belanglosen.\n\
             - Nur Dauerhaftes. Nichts, was nur für diesen einen Auftrag galt.\n\
             - Niemals Geheimnisse: keine Passwörter, API-Keys, Tokens.\n\
             - Jeder Eintrag ein vollständiger, für sich verständlicher Satz.\n\n\
             Antworte AUSSCHLIESSLICH mit JSON, ohne Erklärung drumherum:\n\
             {{\"erinnerungen\":[{{\"art\":\"fakt\",\"inhalt\":\"...\"}}]}}\n\
             Nichts Merkenswertes? Dann {{\"erinnerungen\":[]}}"
        );

        // Keine Werkzeuge anbieten - siehe oben.
        let antwort = match self
            .provider
            .next(None, &[Message::User(frage)], &[])
            .await
        {
            Ok(a) if a.ist_fertig() => a.text,
            Ok(_) => return, // wollte handeln statt denken
            Err(e) => {
                eprintln!("[famulus] Rückblick fehlgeschlagen: {e:#}");
                return;
            }
        };

        let Some(json) = json_herausschneiden(&antwort) else {
            return;
        };
        let Ok(geparst) = serde_json::from_str::<serde_json::Value>(&json) else {
            return;
        };
        let Some(eintraege) = geparst["erinnerungen"].as_array() else {
            return;
        };

        let mut neu = 0usize;
        for eintrag in eintraege.iter().take(3) {
            let art = match eintrag["art"].as_str() {
                Some(a) if [ART_PRAEFERENZ, ART_FAKT, ART_LEKTION].contains(&a) => a,
                // Unbekannte Kategorie: lieber als Fakt ablegen als wegwerfen.
                _ => ART_FAKT,
            };
            let Some(inhalt) = eintrag["inhalt"].as_str() else {
                continue;
            };
            match gedaechtnis.merken(art, inhalt, auftrag) {
                Ok(true) => neu += 1,
                Ok(false) => {} // kannte er schon
                Err(e) => eprintln!("[famulus] Erinnerung nicht gespeichert: {e:#}"),
            }
        }

        if neu > 0 {
            self.ui.ereignis(AgentEvent::Gelernt { anzahl: neu });
        }
    }
}

/// Kürzt lange Ausgaben auf ein erträgliches Maß und behält dabei Anfang und
/// Ende.
///
/// Anfang und Ende statt nur Anfang, weil bei Befehlsausgaben beides zählt:
/// vorne steht, was gemacht wurde, hinten die Fehlermeldung und der Exit-Code.
/// Wer nur vorne abschneidet, wirft ausgerechnet das Ergebnis weg.
fn kuerzen(text: &str, hoechstens: usize) -> String {
    // Schneller Ausweg für den Normalfall: Ein Zeichen belegt in UTF-8 nie
    // weniger als ein Byte, also sind wenige Bytes garantiert wenige Zeichen.
    if text.len() <= hoechstens {
        return text.to_string();
    }
    let gesamt = text.chars().count();
    if gesamt <= hoechstens {
        return text.to_string();
    }

    let kopf_zeichen = hoechstens * 2 / 3;
    let fuss_zeichen = hoechstens - kopf_zeichen;
    let kopf_ende = text
        .char_indices()
        .nth(kopf_zeichen)
        .map_or(text.len(), |(i, _)| i);
    let fuss_start = text
        .char_indices()
        .nth(gesamt - fuss_zeichen)
        .map_or(text.len(), |(i, _)| i);

    format!(
        "{}\n\n[... {} Zeichen gekürzt ...]\n\n{}",
        &text[..kopf_ende],
        gesamt - hoechstens,
        &text[fuss_start..]
    )
}

/// Holt das JSON-Objekt aus einer Modellantwort.
///
/// Modelle umrahmen JSON gern mit ```json-Blöcken oder einem freundlichen
/// Satz davor. Statt darauf zu vertrauen, dass sie es diesmal lassen,
/// schneiden wir von der ersten `{` bis zur letzten `}`.
fn json_herausschneiden(text: &str) -> Option<String> {
    let start = text.find('{')?;
    let ende = text.rfind('}')?;
    (ende > start).then(|| text[start..=ende].to_string())
}

#[cfg(test)]
mod tests {
    use super::{json_herausschneiden, kuerzen};

    #[test]
    fn findet_json_trotz_geplauder() {
        let faelle = [
            r#"{"erinnerungen":[]}"#,
            "Klar, hier:\n```json\n{\"erinnerungen\":[]}\n```\nPasst so?",
            "```\n{\"erinnerungen\":[{\"art\":\"fakt\",\"inhalt\":\"x\"}]}\n```",
        ];
        for fall in faelle {
            let raus = json_herausschneiden(fall).expect("muss JSON finden");
            assert!(
                serde_json::from_str::<serde_json::Value>(&raus).is_ok(),
                "unbrauchbares JSON aus: {fall}"
            );
        }
    }

    #[test]
    fn ohne_json_kein_treffer() {
        assert!(json_herausschneiden("Nichts Merkenswertes.").is_none());
    }

    #[test]
    fn kurzes_bleibt_unveraendert() {
        assert_eq!(kuerzen("hallo", 100), "hallo");
    }

    #[test]
    fn langes_wird_vorne_und_hinten_behalten() {
        let text = format!("ANFANG{}ENDE", "x".repeat(5_000));
        let raus = kuerzen(&text, 100);
        assert!(raus.starts_with("ANFANG"), "Anfang fehlt: {raus}");
        assert!(raus.ends_with("ENDE"), "Ende fehlt: {raus}");
        assert!(raus.contains("gekürzt"), "Hinweis fehlt");
        assert!(raus.chars().count() < 300, "immer noch zu lang");
    }

    /// Umlaute belegen mehrere Bytes. Würde an Byte-Grenzen geschnitten,
    /// bräche das Programm hier mit einer Panik ab.
    #[test]
    fn kuerzen_verkraftet_umlaute() {
        let text = "ä".repeat(5_000);
        let raus = kuerzen(&text, 100);
        assert!(raus.contains("gekürzt"));
    }
}
