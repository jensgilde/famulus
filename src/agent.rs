use crate::config::Config;
use crate::llm::{LlmProvider, Message, ToolResult};
use crate::memory::{Gedaechtnis, ART_FAKT};
use crate::presets::PresetsConfig;
use crate::permissions::PermissionManager;
use crate::tools::notizbuch::NotizbuchTool;
use crate::tools::{all_tools, Tool};
use crate::ui::{AgentEvent, Ui};
use std::collections::HashMap;
use std::sync::Arc;

/// Wie viele Zeichen eines Werkzeug-Ergebnisses in den Kontext dürfen.
const MAX_ERGEBNIS_ZEICHEN: usize = 8_000;

/// Maximale Zeichen im gesamten Nachrichten-Kontext, bevor gekürzt wird.
/// Schützt vor Provider-Kontextlimits bei langen Sessions mit max_turns=997.
const MAX_KONTEXT_ZEICHEN: usize = 128_000;

pub struct Agent {
    provider: Box<dyn LlmProvider>,
    tools: HashMap<String, Box<dyn Tool>>,
    permissions: PermissionManager,
    ui: Arc<dyn Ui>,
    gedaechtnis: Option<Arc<Gedaechtnis>>,
    max_turns: u32,
    max_erinnerungen: usize,
    reflexion: bool,
    hat_vault: bool,
    /// Vault-Pfad für Selbstmodell und ToM
    vault_pfad: Option<std::path::PathBuf>,
    /// Stufe 2: Embeddings sind verfügbar (Ollama läuft mit --embeddings).
    embeddings_aktiv: bool,
}

impl Agent {
    pub async fn new(config: Config, provider: Box<dyn LlmProvider>, ui: Arc<dyn Ui>) -> Self {
        let gedaechtnis = match Gedaechtnis::standard() {
            Ok(g) => Some(Arc::new(g)),
            Err(e) => {
                ui.ereignis(AgentEvent::Abgebrochen {
                    fehler: format!("Gedächtnis nicht verfügbar, arbeite ohne: {e:#}"),
                });
                None
            }
        };

        let hat_vault = config.vault_pfad().is_some();

        // ── Stufe 1+2: Vault-Index und Embeddings beim Start aktualisieren ─
        if let Some(ref g) = gedaechtnis {
            // Vault-Indizierung (Stufe 1).
            if let Some(vault_pfad) = config.vault_pfad() {
                match g.vault_index_aktualisieren(&vault_pfad) {
                    Ok(n) => {
                        ui.ereignis(AgentEvent::Erinnert { anzahl: n });
                        eprintln!("[memory] Vault-Index: {n} Notizen indiziert");
                    }
                    Err(e) => {
                        eprintln!("[memory] Vault-Index fehlgeschlagen: {e:#}");
                    }
                }
            }

            // Embeddings nachholen (Stufe 2).
            match g.embeddings_nachholen().await {
                Ok(n) if n > 0 => {
                    eprintln!("[memory] {n} Embeddings nachgeholt");
                }
                Ok(_) => {}
                Err(e) => {
                    eprintln!("[memory] Embeddings nicht verfügbar: {e:#}");
                }
            }
        }

        // Prüfen, ob Embeddings funktionieren (einmalig - nicht bei jedem
        // Auftrag neu, siehe embeddings_verfuegbar()).
        let embeddings_aktiv = gedaechtnis.is_some() && Gedaechtnis::embeddings_verfuegbar().await;

        let mut tools: HashMap<String, Box<dyn Tool>> = all_tools(&config)
            .into_iter()
            .map(|t| (t.definition().name.clone(), t))
            .collect();

        // ── Stufe 3: Notizbuch-Tool hinzufügen ─
        if let Some(ref g) = gedaechtnis {
            let notizbuch = Box::new(NotizbuchTool {
                gedaechtnis: Arc::clone(g),
            });
            tools.insert(notizbuch.definition().name.clone(), notizbuch);
        }

        // ── Stufe 1: Vault-Suche-Tool (braucht den Gedächtnis-Index, kann
        // deshalb nicht wie die anderen Vault-Tools über all_tools() aus
        // der Config allein gebaut werden) ─
        if hat_vault {
            if let Some(ref g) = gedaechtnis {
                let vault_suche = Box::new(crate::tools::vault::VaultSucheTool {
                    gedaechtnis: Arc::clone(g),
                });
                tools.insert(vault_suche.definition().name.clone(), vault_suche);
            }
        }

        // ── Selbstmodell-Tool: schreibt Wer-ist-Famulus.md in den Vault ──
        let vault_pfad = config.vault_pfad();
        if let (Some(ref g), Some(ref vp)) = (&gedaechtnis, &vault_pfad) {
            let selbstmodell = Box::new(crate::tools::selbstmodell::SelbstmodellTool {
                gedaechtnis: Arc::clone(g),
                vault_pfad: vp.clone(),
            });
            tools.insert(selbstmodell.definition().name.clone(), selbstmodell);
        }

        Self {
            provider,
            tools,
            permissions: PermissionManager::new(&config),
            ui,
            gedaechtnis,
            max_turns: config.max_turns,
            max_erinnerungen: config.max_erinnerungen,
            reflexion: config.reflexion,
            hat_vault,
            vault_pfad,
            embeddings_aktiv,
        }
    }

    async fn systemvorspann(&self, auftrag: &str) -> Option<String> {
        let mut teile = Vec::new();

        // 1. Aktives Preset.
        if let Ok(presets) = PresetsConfig::load() {
            if let Some(prompt) = presets.aktiver_prompt() {
                teile.push(prompt.to_string());
            }
        }

        // 2. Selbstbild: Wer-ist-Famulus.md aus dem Vault, falls vorhanden.
        if let Some(ref vp) = self.vault_pfad {
            let selbstbild_pfad = vp.join("Wer-ist-Famulus.md");
            if let Ok(inhalt) = std::fs::read_to_string(&selbstbild_pfad) {
                if !inhalt.is_empty() {
                    teile.push(format!("Dein Selbstbild (aus dem Vault):\n{}", inhalt));
                }
            }
        }

        // 3. Metakognition: Provider-Statistik als Selbstkenntnis.
        if let Some(ref g) = self.gedaechtnis {
            if let Ok(statistik) = g.provider_statistik() {
                if !statistik.is_empty() {
                    let mut zeilen = vec!["Aktuelle Provider-Statistik (deine Selbstkenntnis):".to_string()];
                    for s in &statistik {
                        zeilen.push(format!(
                            "  - {}: {:.0}% Erfolg bei {} Aufrufen, \u{00d8} {:.0}ms",
                            s.provider, s.erfolgsquote * 100.0, s.anzahl, s.durchschnitt_ms
                        ));
                    }
                    zeilen.push("Nutze diese Daten, um bei Unsicherheit den zuverlässigsten Provider zu wählen.".to_string());
                    teile.push(zeilen.join("\n"));
                }
            }
        }

        // 4. Theory of Mind über Jens: was wissen wir über seinen aktuellen Zustand?
        if let Some(ref g) = self.gedaechtnis {
            if let Ok(treffer) = g.relevante("Jens Status Stimmung aktuell", 3) {
                let jens_infos: Vec<String> = treffer.iter()
                    .filter(|e| e.art == crate::memory::ART_PRAEFERENZ || e.art == crate::memory::ART_FAKT)
                    .map(|e| format!("- {}", e.inhalt))
                    .collect();
                if !jens_infos.is_empty() {
                    teile.push(format!(
                        "Was du über Jens' aktuellen Zustand weißt:\n{}\n\n\
                         Falls du dir bei etwas unsicher bist, frag Jens EINMAL pro Auftrag. \
                         Aber nur dann, wenn es wirklich nötig ist – nicht öfter.",
                        jens_infos.join("\n")
                    ));
                }
            }
        }

        // 5. Gedächtnis: semantische Suche (Stufe 2) oder FTS5 (Stufe 1).
        if let Some(g) = &self.gedaechtnis {
            let erinnerungen = if self.embeddings_aktiv {
                g.relevante_semantisch(auftrag, self.max_erinnerungen)
                    .await
                    .unwrap_or_else(|_| g.relevante(auftrag, self.max_erinnerungen).unwrap_or_default())
            } else {
                g.relevante(auftrag, self.max_erinnerungen).unwrap_or_default()
            };

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

            // Notizbuch-Inhalt vom letzten Auftrag (Stufe 3).
            if let Ok(notizen) = g.notizbuch_lesen() {
                if !notizen.is_empty() {
                    teile.push(format!(
                        "Notizen aus deiner letzten Arbeitssitzung:\n{}",
                        notizen.iter().map(|n| format!("- {n}")).collect::<Vec<_>>().join("\n")
                    ));
                }
            }
        }

        // 3. Vault-Anweisungen.
        if self.hat_vault {
            teile.push(
                "Du hast einen Obsidian-Vault als Langzeitgedächtnis über Jens. \
                 Hast du ein konkretes Stichwort, nutze vault_suche - das ist schneller als \
                 sich mit vault_liste durch alle Notizen zu tasten. Für einen groben Überblick \
                 stattdessen mit vault_liste schauen, was schon bekannt ist, \
                 und Passendes mit vault_lesen öffnen. Lernst du etwas Dauerhaftes über Jens, \
                 seine Projekte oder Ziele, schreib es mit vault_notiz dorthin - bestehende \
                 Notizen ergänzen statt Dubletten anlegen, Wikilinks [[so]] benutzen, bei \
                 Unsicherheit nach 00-Inbox/. Niemals Geheimnisse in den Vault: keine \
                 Passwörter, API-Keys oder Tokens."
                    .to_string(),
            );
        }

        // 4. Notizbuch-Anweisung (Stufe 3).
        if self.gedaechtnis.is_some() {
            teile.push(
                "Du hast ein Notizbuch (Tool: notizbuch). Nutze es, um dir während der Arbeit \
                 wichtige Erkenntnisse zu merken – Fakten über Jens, seine Präferenzen, \
                 technische Details oder Lektionen. Schreibe es knapp und präzise. \
                 Am Ende des Auftrags wird das Notizbuch automatisch ins Langzeitgedächtnis \
                 übernommen."
                    .to_string(),
            );
        }

        // 5. Arbeitsdisziplin: nicht ankündigen, sondern tun; nicht
        // ausdenken, sondern belegen; nicht falsch deuten, sondern
        // durchspielen. Steht hier als eigener Absatz statt nur im
        // Selbstbild, weil das Selbstbild von SelbstmodellTool regelmäßig
        // neu geschrieben wird - diese Regel soll unabhängig davon immer
        // gelten.
        //
        // Regel 3 kam dazu, nachdem ein Selbst-Audit zwei Zeilen korrekt
        // zitierte (agent.rs, nachrichten_kuerzen) und trotzdem beide Male
        // das Gegenteil dessen behauptete, was der Code tatsächlich tut -
        // der Schutz-Code, der genau das verhindert hätte, wurde nicht
        // mitgelesen. Beleg-Pflicht (Regel 2) verhindert Erfundenes, nicht
        // Fehlinterpretiertes-mit-echtem-Zitat - dafür ist Regel 3 da.
        teile.push(
            "Drei feste Regeln für deine Arbeitsweise:\n\
             1. Kündige eine Aktion nie als letzte Nachricht eines Zuges an, ohne sie im \
             selben Zug per Werkzeugaufruf auszuführen. \"Mache ich jetzt\" oder \"Soll ich \
             das umsetzen?\" ohne begleitenden Werkzeugaufruf ist kein gültiger Abschluss - \
             ein Auftrag ist nicht fertig, nur weil du sagst, dass er fertig ist. \
             Nachfragen gibt es nur bei den zwei Ausnahmen (Force-Push, sensible Pfade wie \
             ~/.ssh) - sonst handelst du.\n\
             2. Jede Tatsachenbehauptung über Code, Dateien oder Konfiguration - besonders \
             bei Audits, Fehlersuche oder Reviews - muss auf einem Werkzeugaufruf beruhen, \
             den du in diesem Auftrag tatsächlich gemacht hast, nicht auf einer Vermutung, \
             wie es in so einem Projekt vermutlich aussieht. Stammt eine Angabe aus deinem \
             Gedächtnis statt aus einer frischen Prüfung, sag das explizit dazu (\"laut \
             Gedächtnis vom ...\") statt sie als aktuellen Befund auszugeben. Lieber wenige \
             belegte Funde als eine vollständig aussehende Liste.\n\
             3. Ein Zitat ist kein Beweis für deine Deutung davon, nur dafür, dass du die \
             Zeile gesehen hast. Bevor du einen Bug als kritisch oder mittelschwer einstufst: \
             spiel ihn mit konkreten Beispielwerten Zeile für Zeile durch die zitierte \
             Funktion durch, inklusive der Stellen, die den Fehler verhindern könnten (Guards, \
             frühe Returns, Schutz-Code) - nicht nur der Stelle, die ihn zu belegen scheint. \
             Wo möglich, führ die Behauptung tatsächlich aus (Wegwerf-Test, kleines \
             Reproduktionsskript) statt sie nur zu lesen und zu schlussfolgern - Ausführen \
             beweist, Lesen interpretiert nur. Ohne einen durchgespielten oder ausgeführten \
             Nachweis: der Fund ist \"möglich, unverifiziert\", niemals kritisch oder \
             mittelschwer eingestuft."
                .to_string(),
        );

        (!teile.is_empty()).then(|| teile.join("\n\n"))
    }

    /// Führt einen Auftrag vollständig aus.
    pub async fn run_task(
        &self,
        vorherige_nachrichten: &[Message],
        auftrag: &str,
    ) -> anyhow::Result<()> {
        let system = self.systemvorspann(auftrag).await;

        let mut nachrichten: Vec<Message> = vorherige_nachrichten.to_vec();
        nachrichten.push(Message::User(auftrag.to_string()));

                // Kontext kürzen, wenn nötig (Schutz vor Provider-Limit).
        nachrichten_kuerzen(&mut nachrichten);
        let tool_defs: Vec<_> = self.tools.values().map(|t| t.definition()).collect();

        // Schon einmal nachgehakt, weil eine Antwort nur eine Ankündigung
        // oder Rückfrage ohne Werkzeugaufruf war? Höchstens einmal pro
        // Auftrag - sonst riskiert ein Modell, das die Regel partout nicht
        // befolgt, eine Endlosschleife statt eines klaren Abbruchs.
        let mut wegen_ankuendigung_nachgehakt = false;

        for turn in 0..self.max_turns {
            // Jede Runde hängt Assistant- und ToolResults-Nachrichten an -
            // bei max_turns=997 reicht ein einmaliges Kürzen vor der
            // Schleife nicht, um das Provider-Kontextlimit über einen
            // langen Auftrag hinweg einzuhalten.
            nachrichten_kuerzen(&mut nachrichten);

            let antwort = self
                .provider
                .next(system.as_deref(), &nachrichten, &tool_defs)
                .await?;

            // Text ausgeben.
            if !antwort.text.is_empty() {
                self.ui.ereignis(AgentEvent::Text {
                    chunk: antwort.text.clone(),
                });
            }

            // Werkzeug-Aufrufe ausführen.
            if !antwort.tool_calls.is_empty() {
                let mut ergebnisse = Vec::new();
                for tc in &antwort.tool_calls {
                    self.ui.ereignis(AgentEvent::ToolStart {
                        name: tc.name.clone(),
                        args: tc.arguments.clone(),
                    });

                    let ergebnis = match self.tools.get(&tc.name) {
                        Some(tool) => match tool
                            .execute(tc.arguments.clone(), &self.permissions)
                            .await
                        {
                            Ok(inhalte) => {
                                let gekuerzt = kuerzen(&inhalte, MAX_ERGEBNIS_ZEICHEN);
                                self.ui.ereignis(AgentEvent::ToolEnd {
                                    name: tc.name.clone(),
                                    inhalt: gekuerzt.clone(),
                                });
                                ToolResult {
                                    call_id: tc.id.clone(),
                                    inhalt: gekuerzt,
                                    fehler: false,
                                }
                            }
                            Err(e) => {
                                let fehler_text = format!("{e:#}");
                                self.ui.ereignis(AgentEvent::ToolEnd {
                                    name: tc.name.clone(),
                                    inhalt: fehler_text.clone(),
                                });
                                ToolResult {
                                    call_id: tc.id.clone(),
                                    inhalt: fehler_text,
                                    fehler: true,
                                }
                            }
                        },
                        None => {
                            let fehler_text = format!("Unbekanntes Werkzeug: {}", tc.name);
                            self.ui.ereignis(AgentEvent::ToolEnd {
                                name: tc.name.clone(),
                                inhalt: fehler_text.clone(),
                            });
                            ToolResult {
                                call_id: tc.id.clone(),
                                inhalt: fehler_text,
                                fehler: true,
                            }
                        }
                    };
                    ergebnisse.push(ergebnis);
                }

                // Assistant-Nachricht mit Tool-Calls.
                nachrichten.push(Message::Assistant {
                    text: antwort.text,
                    tool_calls: antwort.tool_calls,
                });
                nachrichten.push(Message::ToolResults(ergebnisse));
            } else if !wegen_ankuendigung_nachgehakt && ist_ankuendigung_ohne_ausfuehrung(&antwort.text) {
                // Text sagt "mache ich jetzt" oder "soll ich das umsetzen?",
                // aber es kam kein Werkzeugaufruf - genau das Muster hinter
                // "kündigt an, macht aber nichts". Statt das als fertig zu
                // werten, einmal nachhaken und dem Modell die Chance geben,
                // die Ankündigung im selben Auftrag tatsächlich einzulösen.
                wegen_ankuendigung_nachgehakt = true;
                nachrichten.push(Message::Assistant {
                    text: antwort.text,
                    tool_calls: Vec::new(),
                });
                nachrichten.push(Message::User(
                    "Du hast angekündigt, etwas zu tun, oder gefragt, ob du es tun sollst - \
                     aber keinen Werkzeugaufruf gemacht. Führ es jetzt in diesem Zug \
                     tatsächlich mit den passenden Werkzeugen aus, oder erklär konkret, \
                     woran es hakt. Frag nicht erneut nach, außer es geht um Force-Push \
                     oder einen sensiblen Pfad."
                        .to_string(),
                ));
            } else {
                // Keine Werkzeuge mehr: das war die finale Antwort.
                nachrichten.push(Message::Assistant {
                    text: antwort.text,
                    tool_calls: Vec::new(),
                });

                self.ui.ereignis(AgentEvent::Fertig);

                // ── Rückblick + Notizbuch-Konsolidierung (Stufe 3) ──
                if self.reflexion {
                    self.reflektieren(auftrag, turn + 1).await;
                }
                return Ok(());
            }
        }

        // Max Turns erreicht.
        self.ui.ereignis(AgentEvent::Abgebrochen {
            fehler: format!(
                "Maximale Anzahl von {} Schritten erreicht – Auftrag abgebrochen.",
                self.max_turns
            ),
        });
        Ok(())
    }

    /// Rückblick: nach jedem Auftrag Erkenntnisse ziehen und merken.
    async fn reflektieren(&self, auftrag: &str, zuege: u32) {
        let Some(g) = &self.gedaechtnis else {
            return;
        };

        self.ui.ereignis(AgentEvent::Reflektiere);

        // ── Stufe 3: Notizbuch auslesen und konsolidieren ──
        let notizen = g.notizbuch_lesen().unwrap_or_default();
        if !notizen.is_empty() {
            let notiz_text = notizen
                .iter()
                .enumerate()
                .map(|(i, n)| format!("{}. {n}", i + 1))
                .collect::<Vec<_>>()
                .join("\n");

            let prompt = format!(
                "Du hast dir während der Arbeit folgende Notizen gemacht:\n\n{notiz_text}\n\n\
                 Überführe jede Notiz in eine dauerhafte Erinnerung. Gib ein JSON-Objekt zurück:\n\
                 {{\"erinnerungen\": [{{\"art\": \"praeferenz|fakt|lektion\", \"inhalt\": \"...\"}}]}}\n\
                 Doppelte Erinnerungen weglassen. Nur antworten mit dem JSON."
            );

            let tool_defs = Vec::new();
            let nachrichten = vec![Message::User(prompt)];

            if let Ok(antwort) = self.provider.next(None, &nachrichten, &tool_defs).await {
                if let Some(json) = json_herausschneiden(&antwort.text) {
                    if let Ok(obj) = serde_json::from_str::<serde_json::Value>(&json) {
                        if let Some(liste) = obj["erinnerungen"].as_array() {
                            for eintrag in liste {
                                let art = eintrag["art"].as_str().unwrap_or(ART_FAKT);
                                let inhalt = eintrag["inhalt"].as_str().unwrap_or("");
                                if g.merken_und_einbetten(art, inhalt, "notizbuch").await.unwrap_or(false) {
                                    self.ui.ereignis(AgentEvent::Gemmerkt {
                                        kategorie: art.to_string(),
                                        inhalt: inhalt.to_string(),
                                    });
                                }
                            }
                        }
                    }
                }
            }

            // Notizbuch leeren nach erfolgreicher Konsolidierung.
            let _ = g.notizbuch_leeren();
        }

        // ── Klassischer Rückblick auf den Auftrag selbst ──
        let prompt = format!(
            "Du hast gerade diesen Auftrag in {zuege} Zügen bearbeitet:\n\n{auftrag}\n\n\
             Was hast du daraus über Jens, das System oder die Arbeitsweise gelernt? \
             Gib ein JSON-Objekt zurück:\n\
             {{\"erinnerungen\": [{{\"art\": \"praeferenz|fakt|lektion\", \"inhalt\": \"...\"}}]}}\n\
             Nur neue Erkenntnisse, die du noch nicht wusstest. Maximal 3. \
             Nur antworten mit dem JSON."
        );

        let nachrichten = vec![Message::User(prompt)];
        let tool_defs = Vec::new();

        if let Ok(antwort) = self.provider.next(None, &nachrichten, &tool_defs).await {
            if let Some(json) = json_herausschneiden(&antwort.text) {
                if let Ok(obj) = serde_json::from_str::<serde_json::Value>(&json) {
                    if let Some(liste) = obj["erinnerungen"].as_array() {
                        for eintrag in liste {
                            let art = eintrag["art"].as_str().unwrap_or(ART_FAKT);
                            let inhalt = eintrag["inhalt"].as_str().unwrap_or("");
                            if g.merken_und_einbetten(art, inhalt, "rueckblick").await.unwrap_or(false) {
                                self.ui.ereignis(AgentEvent::Gemmerkt {
                                    kategorie: art.to_string(),
                                    inhalt: inhalt.to_string(),
                                });
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Kürzt einen Text auf eine maximale Zeichenzahl.
fn kuerzen(text: &str, hoechstens: usize) -> String {
    if text.is_empty() {
        return String::new();
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

/// Kürzt Nachrichten, wenn sie das Kontextlimit überschreiten.
/// Behält den System-Prompt (erste Nachricht) und die letzten Nachrichten.
fn nachrichten_kuerzen(nachrichten: &mut Vec<Message>) {
    let gesamt: usize = nachrichten.iter().map(|m| format!("{m:?}").len()).sum();
    if gesamt <= MAX_KONTEXT_ZEICHEN {
        return;
    }
    let system = nachrichten.first().cloned();
    let system_len = system.as_ref().map(|m| format!("{m:?}").len()).unwrap_or(0);
    let budget = MAX_KONTEXT_ZEICHEN.saturating_sub(system_len);
    let mut hinten_len = 0;
    let mut keep_idx = nachrichten.len();
    for (i, m) in nachrichten.iter().enumerate().rev() {
        if i == 0 && system.is_some() { continue; }
        let len = format!("{m:?}").len();
        if hinten_len + len > budget { break; }
        hinten_len += len;
        keep_idx = i;
    }
    let gekuerzt = nachrichten.len() - keep_idx;
    if gekuerzt > 0 && system.is_some() {
        *nachrichten = {
            let mut v = vec![system.expect("System-Prompt muss da sein, wenn wir kürzen")];
            v.extend(nachrichten.drain(keep_idx..));
            v
        };
        eprintln!("[agent] Kontext gekürzt: {gekuerzt} ältere Nachrichten entfernt, {hinten_len} Zeichen behalten");
    }
}

/// Erkennt, ob eine text-only-Antwort eine Ankündigung oder Rückfrage ist,
/// ohne dass im selben Zug tatsächlich etwas ausgeführt wurde.
///
/// Bewusst konservativ (wenige, eindeutige Formulierungen): eine
/// Falscherkennung würde eine echte fertige Antwort um einen Zug verlängern,
/// das ist der billigere Fehler als das eigentliche Problem - eine
/// Ankündigung, die als Abschluss durchgeht - ungefangen zu lassen.
fn ist_ankuendigung_ohne_ausfuehrung(text: &str) -> bool {
    let t = text.to_lowercase();
    const MUSTER: &[&str] = &[
        "setze ich um",
        "setze ich das um",
        "setze ich jetzt um",
        "mache ich jetzt",
        "mache ich gleich",
        "mache ich direkt",
        "werde ich jetzt",
        "werde ich gleich",
        "werde ich direkt",
        "kümmere ich mich jetzt",
        "kümmere ich mich gleich",
        "kümmere mich jetzt darum",
        "kümmere mich darum",
        "soll ich das umsetzen",
        "soll ich das machen",
        "soll ich das tun",
        "soll ich es umsetzen",
        "soll ich es machen",
        "darf ich das umsetzen",
        "darf ich das machen",
    ];
    MUSTER.iter().any(|m| t.contains(m))
}

fn json_herausschneiden(text: &str) -> Option<String> {
    let start = text.find('{')?;
    let ende = text.rfind('}')?;
    (ende > start).then(|| text[start..=ende].to_string())
}

#[cfg(test)]
mod tests {
    use super::{ist_ankuendigung_ohne_ausfuehrung, json_herausschneiden, kuerzen};

    #[test]
    fn erkennt_ankuendigung_ohne_ausfuehrung() {
        let faelle = [
            "Setze ich um.",
            "Mache ich jetzt.",
            "Soll ich das umsetzen?",
            "Klar, kümmere mich jetzt darum.",
        ];
        for fall in faelle {
            assert!(ist_ankuendigung_ohne_ausfuehrung(fall), "sollte erkannt werden: {fall}");
        }
    }

    #[test]
    fn echte_abschluss_antwort_wird_nicht_faelschlich_erkannt() {
        let faelle = [
            "Fertig - die Datei ist angelegt und der Test läuft grün.",
            "Ich habe den Bug in Zeile 42 gefixt.",
            "Das kann ich nicht automatisch prüfen, ohne einen Netzwerkaufruf zu machen.",
        ];
        for fall in faelle {
            assert!(!ist_ankuendigung_ohne_ausfuehrung(fall), "sollte NICHT erkannt werden: {fall}");
        }
    }

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

    #[test]
    fn kuerzen_verkraftet_umlaute() {
        let text = "ä".repeat(5_000);
        let raus = kuerzen(&text, 100);
        assert!(raus.contains("gekürzt"));
    }
}