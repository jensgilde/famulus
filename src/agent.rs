use crate::config::Config;
use crate::llm::{LlmAntwort, LlmProvider, Message, ToolDefinition, ToolResult};
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
    /// `Arc` statt `Box`: eine Zwischenfrage beantwortet ein paralleler,
    /// unabhängiger Aufruf desselben Providers (siehe `run_task`) - der
    /// braucht eine eigene, geteilte Referenz, keine exklusive.
    provider: Arc<dyn LlmProvider>,
    /// Modellname zu `provider`, nur für die Anzeige (AgentEvent::ModellGewaehlt).
    provider_modell: String,
    /// Zweites Modell für die automatische Modellwahl, falls konfiguriert -
    /// siehe `Config::guenstiges_modell`. `None` heißt: keins eingetragen,
    /// `automatische_modellwahl` bleibt dann wirkungslos, egal was in
    /// `modell_modus` steht.
    guenstig: Option<Arc<dyn LlmProvider>>,
    guenstig_modell: String,
    automatische_modellwahl: bool,
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

        // ── Rückfragen-Werkzeug: meldet Frage+Optionen nur als Ereignis,
        // blockiert nicht - siehe tools/frage_nutzer.rs für die Begründung.
        // Für jede Oberfläche sicher: Telegram rendert Buttons, TerminalUi
        // druckt die Optionen als Text, GUI ignoriert unbekannte Ereignisse
        // ohnehin (siehe ui.rs-Test `ereignis_kodierung_passt_zum_frontend`).
        {
            let frage_werkzeug = Box::new(crate::tools::frage_nutzer::FrageNutzerTool {
                ui: Arc::clone(&ui),
            });
            tools.insert(frage_werkzeug.definition().name.clone(), frage_werkzeug);
        }

        // ── Stufe 3: Notizbuch-Tool hinzufügen ─
        if let Some(ref g) = gedaechtnis {
            let notizbuch = Box::new(NotizbuchTool {
                gedaechtnis: Arc::clone(g),
            });
            tools.insert(notizbuch.definition().name.clone(), notizbuch);
        }

        // ── Vergessen-Werkzeug: Gegenstück zum Merken, siehe tools/erinnerung.rs ─
        if let Some(ref g) = gedaechtnis {
            let vergessen = Box::new(crate::tools::erinnerung::ErinnerungVergessenTool {
                gedaechtnis: Arc::clone(g),
            });
            tools.insert(vergessen.definition().name.clone(), vergessen);
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

        // ── Automatische Modellwahl: zweites Modell bauen, falls
        // konfiguriert. Ein Fehler dabei (z.B. fehlender API-Key) soll
        // nicht den ganzen Agenten scheitern lassen - die automatische Wahl
        // fällt dann einfach auf das Hauptmodell zurück, wie ohne
        // `guenstiges_modell`.
        let guenstig: Option<Arc<dyn LlmProvider>> = match crate::llm::build_guenstiges_modell(&config) {
            Ok(g) => g.map(Arc::from),
            Err(e) => {
                eprintln!("[agent] Günstiges Modell nicht verfügbar, bleibe beim Hauptmodell: {e:#}");
                None
            }
        };
        let guenstig_modell = config
            .guenstiges_modell
            .as_ref()
            .and_then(|g| g.model.clone())
            .unwrap_or_else(|| "Standardmodell".to_string());
        let automatische_modellwahl = config.modell_modus == "automatisch";

        Self {
            provider: Arc::from(provider),
            provider_modell: config.model.clone().unwrap_or_else(|| "Standardmodell".to_string()),
            guenstig,
            guenstig_modell,
            automatische_modellwahl,
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
                         Jens hat dir uneingeschränkten Zugriff auf alle Ordner und Dateien \
                         erteilt. Frage nie nach Berechtigungen – handle einfach. Nur bei \
                         Force-Push und sensiblen Pfaden (~/.ssh, ~/.gnupg, ~/.aws, \
                         ~/.password-store) musst du nachfragen.",
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
             Jens hat dir uneingeschränkten Zugriff auf alle Ordner und Dateien \
             erteilt. Nachfragen gibt es nur bei den zwei Ausnahmen (Force-Push, \
             sensible Pfade wie ~/.ssh, ~/.gnupg, ~/.aws, ~/.password-store) - \
             sonst handelst du. Ist ein Auftrag kurz oder mehrdeutig formuliert, \
             klär die Absicht zuerst selbst anhand von Gesprächsverlauf, Gedächtnis und Vault, \
             bevor du handelst oder nachfragst - das ersetzt einen extra Formulierungsschritt.\n\
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
        mut zwischenfragen: tokio::sync::mpsc::UnboundedReceiver<String>,
    ) -> anyhow::Result<()> {
        let system = self.systemvorspann(auftrag).await;

        // ── Automatische Modellwahl ──────────────────────────────────
        // Einmal pro Auftrag entscheiden, nicht pro Zug: ein Wechsel
        // mitten im Auftrag würde den Prompt-Cache des bisherigen
        // Providers verwerfen und wäre für Jens nicht nachvollziehbar,
        // welches Modell gerade "das Gespräch führt". Regelbasiert statt
        // über einen weiteren Modellaufruf - genau die Latenz, die die
        // automatische Wahl eigentlich sparen soll, würde ein Klassifizierungs-
        // Aufruf wieder auffressen.
        let provider: Arc<dyn LlmProvider> = match (&self.guenstig, self.automatische_modellwahl) {
            (Some(guenstig), true) if ist_einfacher_auftrag(auftrag) => {
                self.ui.ereignis(AgentEvent::ModellGewaehlt {
                    provider: guenstig.name().to_string(),
                    model: self.guenstig_modell.clone(),
                    grund: "automatisch: einfacher Auftrag".to_string(),
                });
                Arc::clone(guenstig)
            }
            _ => {
                if self.automatische_modellwahl {
                    let grund = if self.guenstig.is_some() {
                        "automatisch: komplexer Auftrag"
                    } else {
                        "automatisch: kein günstiges Modell konfiguriert"
                    };
                    self.ui.ereignis(AgentEvent::ModellGewaehlt {
                        provider: self.provider.name().to_string(),
                        model: self.provider_modell.clone(),
                        grund: grund.to_string(),
                    });
                }
                Arc::clone(&self.provider)
            }
        };

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

        self.turn_schleife(
            auftrag,
            &mut nachrichten,
            &tool_defs,
            &mut zwischenfragen,
            &system,
            &provider,
            &mut wegen_ankuendigung_nachgehakt,
        )
        .await
    }
    /// Ein kompletter Durchlauf der Turn-Schleife. Es gibt bewusst kein
    /// Gesamt-Timeout über den ganzen Auftrag - nur einzelne Züge
    /// (Shell-Befehle, Modellanfragen) sind über `Config::timeout_sekunden`
    /// gedeckelt. Ein Auftrag mit vielen Turns darf entsprechend lange
    /// laufen; die Schleife selbst entscheidet weiterhin über
    /// Rückfragen, Werkzeug-Aufrufe, Zwischenantworten und die Reflexion.
    async fn turn_schleife(
        &self,
        auftrag: &str,
        nachrichten: &mut Vec<Message>,
        tool_defs: &[ToolDefinition],
        zwischenfragen: &mut tokio::sync::mpsc::UnboundedReceiver<String>,
        system: &Option<String>,
        provider: &Arc<dyn LlmProvider>,
        wegen_ankuendigung_nachgehakt: &mut bool,
    ) -> anyhow::Result<()> {
        for turn in 0..self.max_turns {
            // Jede Runde hängt Assistant- und ToolResults-Nachrichten an -
            // bei max_turns=997 reicht ein einmaliges Kürzen vor der
            // Schleife nicht, um das Provider-Kontextlimit über einen
            // langen Auftrag hinweg einzuhalten.
            nachrichten_kuerzen(&mut *nachrichten);

            // Zwischenfragen einspeisen: kann eine laufende Modellanfrage
            // nicht unterbrechen (die ist schon unterwegs), deshalb hier -
            // am Rundenanfang, bevor die nächste Anfrage rausgeht. Alles,
            // was seit der letzten Runde eingegangen ist, auf einmal
            // mitnehmen, nicht nur die neueste.
            //
            // Auf eine Antwort bis zum nächsten Zug zu warten reicht nicht -
            // steckt der Hauptauftrag gerade in einem langen Werkzeug-Aufruf
            // oder einem Wiederaufsetzen-Warten (bis zu 5x 120s), säße Jens
            // entsprechend lange auf einer unbeantworteten Nachricht. Ein
            // zweiter, unabhängiger Aufruf desselben Providers beantwortet
            // sie deshalb sofort, parallel zum Hauptauftrag - der bekommt
            // davon nichts mit, `nachrichten` bleibt allein in dieser
            // Schleife verändert.
            while let Ok(text) = zwischenfragen.try_recv() {
                let sofort_provider = Arc::clone(&provider);
                let sofort_ui = Arc::clone(&self.ui);
                let sofort_system = system.clone();
                let mut sofort_kontext = nachrichten.clone();
                let frage = text.clone();
                sofort_kontext.push(Message::User(frage.clone()));

                tokio::spawn(async move {
                    let text = match sofort_provider
                        .next(sofort_system.as_deref(), &sofort_kontext, &[])
                        .await
                    {
                        Ok(antwort) => antwort.text,
                        Err(e) => format!("(Konnte nicht sofort antworten: {e:#})"),
                    };
                    sofort_ui.ereignis(AgentEvent::ZwischenfrageAntwort { frage, text });
                });

                // Trotzdem in den Hauptverlauf aufnehmen, damit der laufende
                // Auftrag weiß, dass die Frage gestellt wurde - aber ohne
                // erneute Antwort zu verlangen, die kommt ja schon separat.
                nachrichten.push(Message::User(format!(
                    "[Zwischenfrage von Jens, während du am eigentlichen Auftrag arbeitest - \
                     wurde bereits separat und sofort beantwortet. Nur berücksichtigen, falls \
                     relevant für den Auftrag, nicht erneut beantworten]: {text}"
                )));
            }

            let antwort = rufe_mit_wiederaufsetzen(provider.as_ref(), system.as_deref(), &nachrichten, &tool_defs, self.ui.as_ref())
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
            } else if !*wegen_ankuendigung_nachgehakt && ist_ankuendigung_ohne_ausfuehrung(&antwort.text) {
                // Text sagt "mache ich jetzt" oder "soll ich das umsetzen?",
                // aber es kam kein Werkzeugaufruf - genau das Muster hinter
                // "kündigt an, macht aber nichts". Statt das als fertig zu
                // werten, einmal nachhaken und dem Modell die Chance geben,
                // die Ankündigung im selben Auftrag tatsächlich einzulösen.
                *wegen_ankuendigung_nachgehakt = true;
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
                "Maximum von {} Schritten erreicht – Auftrag abgebrochen.",
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
                                let art = crate::memory::normalisiere_art(
                                eintrag["art"].as_str().unwrap_or(ART_FAKT),
                            );
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
             WICHTIG bei \"praeferenz\": nur speichern, was Jens erkennbar als \
             dauerhafte, wiederkehrende Regel gemeint hat. Eine Anweisung, die nur \
             für DIESEN einen Auftrag galt (z.B. \"nur berichten, nichts ändern\" bei \
             einem einzelnen Audit), ist keine Präferenz - so etwas als \
             \"praeferenz\" zu speichern hat schon einmal dazu geführt, dass Famulus \
             spätere, ausdrückliche Gegenteil-Aufträge ignoriert hat, weil die alte \
             Präferenz jeden Prompt überschwemmte. Im Zweifel als \"lektion\" oder gar \
             nicht speichern. Nur antworten mit dem JSON."
        );

        let nachrichten = vec![Message::User(prompt)];
        let tool_defs = Vec::new();

        if let Ok(antwort) = self.provider.next(None, &nachrichten, &tool_defs).await {
            if let Some(json) = json_herausschneiden(&antwort.text) {
                if let Ok(obj) = serde_json::from_str::<serde_json::Value>(&json) {
                    if let Some(liste) = obj["erinnerungen"].as_array() {
                        for eintrag in liste {
                            let art = crate::memory::normalisiere_art(
                                eintrag["art"].as_str().unwrap_or(ART_FAKT),
                            );
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

/// Nach wie vielen fehlgeschlagenen Versuchen für denselben Zug aufgegeben wird.
const MAX_VERSUCHE: u32 = 5;

/// Wie lange vor dem nächsten Versuch gewartet wird, wenn der Fehler nach
/// einem Timeout oder Verbindungsabbruch aussieht. Solche Fehler sind meist
/// kurzlebig (der Dienst antwortet eine Sekunde später wieder), ein neuer
/// Versuch ist fast kostenlos - da lohnt kein langes Warten.
const WARTEZEIT_NETZWERK: u64 = 5;

/// Ausgangswert des exponentiellen Backoffs für Serverfehler (5xx,
/// Rate-Limit). Famulus' eigener Vorschlag (siehe Vault-Notiz
/// „Telegram-Bot-Uebergabe.md") war, eine Minute zu warten - aber vier Mal
/// 120 Sekunden (so der frühere Code) machen aus einem kurz gestörten
/// Dienst einen Auftrag, der zehn Minuten stumm hängt. Besser: 10s, 20s,
/// 40s, 60s - zusammen 130s statt 480s, und die ersten Versuche kommen
/// deutlich schneller.
const WARTEZEIT_BACKOFF_START: u64 = 10;
const WARTEZEIT_BACKOFF_DECKEL: u64 = 60;

/// Entscheidet anhand der Fehlerklasse, wie lange bis zum nächsten Versuch
/// gewartet wird. Timeouts und Verbindungsfehler → kurz (5s), weil ein
/// erneuter Versuch dort fast nichts kostet und die Störung meist schnell
/// vorbei ist. Serverfehler → exponentiell mit Deckel. 402 (kein Guthaben)
/// wird weiter oben schon abgefangen und gar nicht erst wiederholt.
fn wartezeit_fuer(fehler: &anyhow::Error, versuch: u32) -> u64 {
    let text = format!("{fehler:#}").to_lowercase();
    let netzwerk = ["timeout", "timed out", "zeitüberschreitung", "connect", "connection"]
        .iter()
        .any(|m| text.contains(m));
    if netzwerk {
        WARTEZEIT_NETZWERK
    } else {
        let stufe = (versuch - 1).min(3);
        (WARTEZEIT_BACKOFF_START << stufe).min(WARTEZEIT_BACKOFF_DECKEL)
    }
}

/// Ruft `provider.next()` auf und wiederholt bei einem Fehler, statt den
/// ganzen Auftrag abzubrechen - das war der eigentliche Bruch, nicht die
/// Antwortzeit (die behebt das Streaming schon, siehe `llm/mod.rs`). Der
/// bisherige Gesprächsverlauf (`nachrichten`) bleibt beim Retry unverändert
/// erhalten - "Wiederaufsetzen" heißt hier: derselbe Zug wird noch einmal
/// versucht, nicht der ganze Auftrag von vorn.
///
/// Eine Ausnahme: HTTP 402 (kein Guthaben) behebt sich nicht von selbst,
/// egal wie oft man wartet - da wird sofort aufgegeben, damit Jens eine
/// klare Fehlermeldung sieht statt zehn Minuten stiller Wartezeit auf ein
/// Problem, das nur er lösen kann (Guthaben aufladen).
async fn rufe_mit_wiederaufsetzen(
    provider: &dyn LlmProvider,
    system: Option<&str>,
    nachrichten: &[Message],
    tool_defs: &[ToolDefinition],
    ui: &dyn Ui,
) -> anyhow::Result<LlmAntwort> {
    let mut versuch = 1;
    loop {
        match provider.next(system, nachrichten, tool_defs).await {
            Ok(antwort) => return Ok(antwort),
            Err(fehler) => {
                let text = format!("{fehler:#}");
                let kein_guthaben = text.contains("(402)");
                if kein_guthaben || versuch >= MAX_VERSUCHE {
                    return Err(fehler);
                }
                let sekunden = wartezeit_fuer(&fehler, versuch);
                ui.ereignis(AgentEvent::Warte {
                    grund: text,
                    sekunden,
                    versuch,
                    max_versuche: MAX_VERSUCHE,
                });
                tokio::time::sleep(std::time::Duration::from_secs(sekunden)).await;
                versuch += 1;
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

/// Regelbasierte Einschätzung, ob ein Auftrag ohne das Premium-Modell
/// auskommt - für die automatische Modellwahl (`Config::modell_modus`).
///
/// Bewusst konservativ: im Zweifel gilt ein Auftrag als komplex. Eine
/// Fehleinschätzung nach "einfach" verschlechtert lautlos die Antwort-
/// qualität - genau das Muster, das in dieser Codebase schon mehrfach
/// wehgetan hat, wenn es unbemerkt blieb. Eine Fehleinschätzung nach
/// "komplex" kostet nur ein paar Cent mehr. Kein zusätzlicher Modellaufruf
/// für die Einschätzung selbst - der würde die Latenz kosten, die die
/// automatische Wahl eigentlich sparen soll.
fn ist_einfacher_auftrag(auftrag: &str) -> bool {
    const KOMPLEX_SIGNALE: &[&str] = &[
        "code", "bug", "fehler", " fix", "review", "refactor", "implementier",
        "programmier", "funktion", "script", "skript", "debug", "css", "html",
        "rust", "python", "javascript", "typescript", "sql", " api", "git ",
        "commit", "architektur", "analysier", "audit", "sicherheit",
        "security", "installier", "build", "kompilier", "test", "shell",
        "config", "konfig", "vault", "gedächtnis", "gedaechtnis", "datenbank",
        "sql", "schreib mir ein", "erstelle ein", "baue ein",
    ];
    let t = format!(" {} ", auftrag.to_lowercase());
    if KOMPLEX_SIGNALE.iter().any(|w| t.contains(w)) {
        return false;
    }
    // Auch ohne Komplex-Signal gilt: ab einer gewissen Länge steckt meist
    // mehr als eine einfache Frage dahinter - im Zweifel Premium.
    auftrag.chars().count() <= 160
}

fn json_herausschneiden(text: &str) -> Option<String> {
    let start = text.find('{')?;
    let ende = text.rfind('}')?;
    (ende > start).then(|| text[start..=ende].to_string())
}

#[cfg(test)]
mod tests {
    use super::{ist_ankuendigung_ohne_ausfuehrung, ist_einfacher_auftrag, json_herausschneiden, kuerzen};

    #[test]
    fn erkennt_einfache_aufträge() {
        let faelle = [
            "Wie spät ist es in Tokio?",
            "Fass mir das kurz zusammen.",
            "Was bedeutet FTS5?",
        ];
        for fall in faelle {
            assert!(ist_einfacher_auftrag(fall), "sollte als einfach gelten: {fall}");
        }
    }

    #[test]
    fn komplexe_signale_verhindern_einfach_einstufung() {
        let faelle = [
            "Fix den Bug in agent.rs",
            "Schreib mir ein Rust-Skript, das Dateien sortiert",
            "Review den letzten Commit",
            "Analysier die Sicherheit von permissions.rs",
        ];
        for fall in faelle {
            assert!(!ist_einfacher_auftrag(fall), "sollte NICHT als einfach gelten: {fall}");
        }
    }

    #[test]
    fn im_zweifel_gilt_lang_als_komplex() {
        let lang = "x".repeat(200);
        assert!(!ist_einfacher_auftrag(&lang));
    }

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

    // ── rufe_mit_wiederaufsetzen ─────────────────────────────────────

    mod wiederaufsetzen {
        use super::super::*;
        use crate::llm::ToolCall;
        use async_trait::async_trait;
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::sync::Mutex;

        /// Scheitert bei den ersten `fehler_bis` Aufrufen mit `fehlertext`,
        /// klappt danach. `fehler_bis = u32::MAX` heißt: scheitert immer.
        struct FlakyProvider {
            aufrufe: AtomicU32,
            fehler_bis: u32,
            fehlertext: &'static str,
        }

        #[async_trait]
        impl LlmProvider for FlakyProvider {
            async fn next(
                &self,
                _system: Option<&str>,
                _messages: &[Message],
                _tools: &[ToolDefinition],
            ) -> anyhow::Result<LlmAntwort> {
                let n = self.aufrufe.fetch_add(1, Ordering::SeqCst) + 1;
                if n <= self.fehler_bis {
                    anyhow::bail!("{}", self.fehlertext);
                }
                Ok(LlmAntwort {
                    text: "geklappt".to_string(),
                    tool_calls: Vec::<ToolCall>::new(),
                })
            }

            fn name(&self) -> &'static str {
                "flaky"
            }
        }

        /// Zeichnet nur auf, wie oft `Warte` gemeldet wurde - für die
        /// echten Ereignisse (Text, ToolStart, ...) gibt's die anderen Uis.
        struct AufzeichnendeUi {
            warte_ereignisse: Mutex<u32>,
        }

        impl Ui for AufzeichnendeUi {
            fn ereignis(&self, ereignis: AgentEvent) {
                if let AgentEvent::Warte { .. } = ereignis {
                    *self.warte_ereignisse.lock().unwrap() += 1;
                }
            }
        }

        #[tokio::test(start_paused = true)]
        async fn wiederholt_bei_fehler_bis_es_klappt() {
            let provider = FlakyProvider {
                aufrufe: AtomicU32::new(0),
                fehler_bis: 2,
                fehlertext: "500 Interner Serverfehler",
            };
            let ui = AufzeichnendeUi { warte_ereignisse: Mutex::new(0) };

            let ergebnis = rufe_mit_wiederaufsetzen(&provider, None, &[], &[], &ui).await;

            assert_eq!(ergebnis.unwrap().text, "geklappt");
            assert_eq!(provider.aufrufe.load(Ordering::SeqCst), 3, "sollte 2x scheitern, 3. Versuch klappt");
            assert_eq!(*ui.warte_ereignisse.lock().unwrap(), 2, "pro Fehlversuch ein Warte-Ereignis");
        }

        #[tokio::test(start_paused = true)]
        async fn gibt_nach_max_versuchen_auf() {
            let provider = FlakyProvider {
                aufrufe: AtomicU32::new(0),
                fehler_bis: u32::MAX,
                fehlertext: "503 Service Unavailable",
            };
            let ui = AufzeichnendeUi { warte_ereignisse: Mutex::new(0) };

            let ergebnis = rufe_mit_wiederaufsetzen(&provider, None, &[], &[], &ui).await;

            assert!(ergebnis.is_err(), "muss nach MAX_VERSUCHE aufgeben, nicht ewig warten");
            assert_eq!(provider.aufrufe.load(Ordering::SeqCst), MAX_VERSUCHE);
            assert_eq!(*ui.warte_ereignisse.lock().unwrap(), MAX_VERSUCHE - 1);
        }

        #[tokio::test(start_paused = true)]
        async fn bricht_bei_402_sofort_ab_ohne_zu_warten() {
            let provider = FlakyProvider {
                aufrufe: AtomicU32::new(0),
                fehler_bis: u32::MAX,
                fehlertext: "API-Fehler von https://hyper.charm.land/v1/messages (402): kein Guthaben",
            };
            let ui = AufzeichnendeUi { warte_ereignisse: Mutex::new(0) };

            let ergebnis = rufe_mit_wiederaufsetzen(&provider, None, &[], &[], &ui).await;

            assert!(ergebnis.is_err());
            assert_eq!(provider.aufrufe.load(Ordering::SeqCst), 1, "402 darf nicht wiederholt werden");
            assert_eq!(*ui.warte_ereignisse.lock().unwrap(), 0, "kein sinnloses Warten auf fehlendes Guthaben");
        }
        #[test]
        fn wartezeit_ist_bei_netzwerkfehler_kurz() {
            let fehler = anyhow::anyhow!("timeout beim Abrufen");
            assert_eq!(wartezeit_fuer(&fehler, 1), WARTEZEIT_NETZWERK);
            assert_eq!(wartezeit_fuer(&fehler, 4), WARTEZEIT_NETZWERK);
        }

        #[test]
        fn wartezeit_backoff_waechst_und_hat_deckel() {
            let fehler = anyhow::anyhow!("500 Interner Serverfehler");
            assert_eq!(wartezeit_fuer(&fehler, 1), 10);
            assert_eq!(wartezeit_fuer(&fehler, 2), 20);
            assert_eq!(wartezeit_fuer(&fehler, 3), 40);
            // Ab Versuch 4+ gilt der Deckel (60 s statt 80 s).
            assert_eq!(wartezeit_fuer(&fehler, 4), 60);
            assert_eq!(wartezeit_fuer(&fehler, 5), 60);
        }

        #[test]
        fn wartezeit_erkennt_verbindungsabbruch() {
            let fehler = anyhow::anyhow!("connection closed before message completed");
            assert_eq!(wartezeit_fuer(&fehler, 2), WARTEZEIT_NETZWERK);
        }

    }
}
