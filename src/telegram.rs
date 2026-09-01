//! Telegram-Anbindung: Famulus über einen Telegram-Bot ansprechbar machen.
//!
//! Läuft als eigener Prozess (Binary `famulus-telegram`), pollt Telegrams
//! `getUpdates` und fährt pro eingehender Nachricht denselben Agent-Lauf wie
//! GUI/CLI - siehe `gui/src/lib.rs::starte_auftrag` fürs Vorbild: der Agent
//! wird pro Auftrag neu gebaut, der Gesprächsverlauf lebt hier im Prozess
//! (pro Chat-ID) statt im Frontend.
//!
//! **Zwei Tasks seit v1.0.1:** Ein Dauer-Poll-Task ist der EINZIGE
//! `getUpdates`-Aufrufer und sortiert die Updates (Allowlist). Der
//! Hauptloop arbeitet seriell (ein Agent-Lauf pro Nachricht, wie die GUI ihn
//! auch fährt). Während ein Auftrag läuft, hört der Hauptloop über einen
//! dritten `select`-Zweig weiter auf eingehende Nachrichten und leitet sie
//! als Zwischenfrage an den laufenden Agenten weiter statt sie stumm zu
//! schlucken - vorher war der Bot während eines Auftrags komplett taub
//! (Nachrichten wurden erst nach dem Auftrag als eigener Auftrag gelesen).
//! `/status` antwortet auch während eines Laufs sofort, ohne LLM.
//!
//! **Sicherheit:** Famulus hat vollen, ungefragten Rechnerzugriff - siehe
//! `permissions.rs`. Ein Bot-Token allein schützt nichts: Wer die Chat-ID
//! kennt, kann mit dem Bot reden. Deshalb antwortet dieser Bot NUR auf Chats
//! aus `TELEGRAM_ALLOWED_CHAT_IDS` - jede andere Chat-ID wird abgewiesen und
//! geloggt, bekommt aber nie einen Agent-Lauf zu Gesicht.

use crate::agent::Agent;
use crate::config::Config;
use crate::llm::{self, Message};
use crate::ui::{AgentEvent, Ui};
use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

const API_BASE: &str = "https://api.telegram.org";
// Telegrams Grenze liegt bei 4096 Zeichen - Puffer für Sicherheit gegen
// Zählweisen-Differenzen (Telegram zählt UTF-16-Einheiten, nicht Zeichen).
const TELEGRAM_MAX_LEN: usize = 3500;

pub struct TelegramConfig {
    pub token: String,
    pub allowed_chat_ids: Vec<i64>,
}

impl TelegramConfig {
    /// Liest Token und Allowlist aus der Umgebung. Erwartet, dass `.env`
    /// vorher geladen wurde (siehe `src/bin/telegram.rs`) - `Config::load()`
    /// selbst macht das erst später, aber diese Werte werden schon davor
    /// gebraucht.
    pub fn from_env() -> Result<Self> {
        let token = std::env::var("TELEGRAM_BOT_TOKEN")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .context("TELEGRAM_BOT_TOKEN fehlt oder ist leer. Trag ihn in ~/.famulus/.env ein.")?;

        let allowed_chat_ids: Vec<i64> = std::env::var("TELEGRAM_ALLOWED_CHAT_IDS")
            .ok()
            .map(|s| {
                s.split(',')
                    .filter_map(|p| p.trim().parse::<i64>().ok())
                    .collect()
            })
            .unwrap_or_default();

        if allowed_chat_ids.is_empty() {
            anyhow::bail!(
                "TELEGRAM_ALLOWED_CHAT_IDS fehlt oder ist leer. Ohne Allowlist würde \
                 jeder, der die Chat-ID kennt, vollen Rechnerzugriff über den Bot bekommen - \
                 trag mindestens deine eigene Chat-ID (kommagetrennt für mehrere) in \
                 ~/.famulus/.env ein."
            );
        }

        Ok(Self {
            token: token.trim().to_string(),
            allowed_chat_ids,
        })
    }
}

#[derive(Debug, Deserialize)]
struct TgResponse<T> {
    ok: bool,
    result: Option<T>,
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Update {
    update_id: i64,
    message: Option<TgMessage>,
    /// Button-Klick auf ein `frage_nutzer`-Inline-Keyboard, siehe
    /// `AgentEvent::FrageAnNutzer` und `verarbeite_callback`.
    callback_query: Option<TgCallbackQuery>,
}

#[derive(Debug, Deserialize)]
struct TgMessage {
    chat: TgChat,
    text: Option<String>,
    #[serde(default)]
    from: Option<TgUser>,
    #[serde(default)]
    message_id: i64,
    /// Nur bei Nachrichten mit Buttons gesetzt - liefert die Beschriftung
    /// zu einem `callback_data`-Index zurück, ohne dass wir uns die Frage
    /// selbst irgendwo merken müssten.
    #[serde(default)]
    reply_markup: Option<TgReplyMarkup>,
}

#[derive(Debug, Deserialize)]
struct TgReplyMarkup {
    inline_keyboard: Vec<Vec<TgInlineButton>>,
}

#[derive(Debug, Deserialize)]
struct TgInlineButton {
    text: String,
}

#[derive(Debug, Deserialize)]
struct TgCallbackQuery {
    id: String,
    /// Der Index der geklickten Option als String, siehe
    /// `send_message_mit_optionen` (callback_data = Options-Index).
    data: Option<String>,
    message: Option<TgMessage>,
}

#[derive(Debug, Deserialize)]
struct TgChat {
    id: i64,
}

#[derive(Debug, Deserialize)]
struct TgUser {
    username: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Me {
    username: Option<String>,
}

async fn get_me(client: &reqwest::Client, token: &str) -> Result<String> {
    let url = format!("{API_BASE}/bot{token}/getMe");
    let resp: TgResponse<Me> = client.get(&url).send().await?.json().await?;
    if !resp.ok {
        anyhow::bail!("getMe fehlgeschlagen: {}", resp.description.unwrap_or_default());
    }
    Ok(resp
        .result
        .and_then(|m| m.username)
        .unwrap_or_else(|| "?".to_string()))
}

/// Long-Polling: Telegram hält die Verbindung bis zu `timeout` Sekunden
/// offen und antwortet sofort, sobald eine Nachricht da ist - spart das
/// Kurz-Intervall-Pollen, das bei jedem Bot unnötig Last erzeugt.
async fn get_updates(client: &reqwest::Client, token: &str, offset: i64) -> Result<Vec<Update>> {
    let url = format!("{API_BASE}/bot{token}/getUpdates");
    let resp: TgResponse<Vec<Update>> = client
        .get(&url)
        .query(&[("offset", offset.to_string()), ("timeout", "30".to_string())])
        .timeout(Duration::from_secs(40))
        .send()
        .await?
        .json()
        .await
        .context("Antwort von getUpdates war kein gültiges JSON")?;
    if !resp.ok {
        anyhow::bail!("getUpdates fehlgeschlagen: {}", resp.description.unwrap_or_default());
    }
    Ok(resp.result.unwrap_or_default())
}

async fn send_message(client: &reqwest::Client, token: &str, chat_id: i64, text: &str) -> Result<()> {
    let text = if text.trim().is_empty() { "(keine Antwort)" } else { text };
    for teil in in_stuecke(text, TELEGRAM_MAX_LEN) {
        let url = format!("{API_BASE}/bot{token}/sendMessage");
        let resp = client
            .post(&url)
            .json(&serde_json::json!({ "chat_id": chat_id, "text": teil }))
            .send()
            .await
            .context("sendMessage fehlgeschlagen")?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("sendMessage HTTP {status}: {body}");
        }
    }
    Ok(())
}

/// Schickt eine Frage mit anklickbaren Optionen (Inline-Keyboard, eine
/// Option pro Zeile). `callback_data` ist bewusst nur der Index als String,
/// nicht der Options-Text selbst: Telegram deckelt `callback_data` auf 64
/// Byte, und die Beschriftung lässt sich beim Klick ohnehin verlustfrei aus
/// `callback_query.message.reply_markup` zurücklesen (siehe
/// `verarbeite_callback`) - kein Extra-Zustand nötig.
async fn send_message_mit_optionen(
    client: &reqwest::Client,
    token: &str,
    chat_id: i64,
    text: &str,
    optionen: &[String],
) -> Result<()> {
    let inline_keyboard: Vec<Vec<serde_json::Value>> = optionen
        .iter()
        .enumerate()
        .map(|(i, opt)| vec![serde_json::json!({ "text": opt, "callback_data": i.to_string() })])
        .collect();
    let url = format!("{API_BASE}/bot{token}/sendMessage");
    let resp = client
        .post(&url)
        .json(&serde_json::json!({
            "chat_id": chat_id,
            "text": text,
            "reply_markup": { "inline_keyboard": inline_keyboard },
        }))
        .send()
        .await
        .context("sendMessage (mit Optionen) fehlgeschlagen")?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("sendMessage (mit Optionen) HTTP {status}: {body}");
    }
    Ok(())
}

/// Beendet den "lädt..."-Zustand des geklickten Buttons. Rein kosmetisch -
/// ein Fehlschlag hier darf die eigentliche Verarbeitung nicht aufhalten,
/// deshalb wird der Fehler nur geloggt statt weitergereicht.
async fn beantworte_callback(client: &reqwest::Client, token: &str, callback_id: &str) {
    let url = format!("{API_BASE}/bot{token}/answerCallbackQuery");
    if let Err(e) = client
        .post(&url)
        .json(&serde_json::json!({ "callback_query_id": callback_id }))
        .send()
        .await
    {
        eprintln!("[telegram] answerCallbackQuery fehlgeschlagen: {e:#}");
    }
}

/// Entfernt die Buttons einer beantworteten Frage, damit derselbe Klick
/// nicht zweimal einen Auftrag auslösen kann. Best-effort wie oben.
async fn entferne_buttons(client: &reqwest::Client, token: &str, chat_id: i64, message_id: i64) {
    if message_id == 0 {
        return;
    }
    let url = format!("{API_BASE}/bot{token}/editMessageReplyMarkup");
    if let Err(e) = client
        .post(&url)
        .json(&serde_json::json!({
            "chat_id": chat_id,
            "message_id": message_id,
            "reply_markup": { "inline_keyboard": [] },
        }))
        .send()
        .await
    {
        eprintln!("[telegram] editMessageReplyMarkup fehlgeschlagen: {e:#}");
    }
}

/// Teilt Text an Zeilengrenzen in Telegram-taugliche Stücke. Reine
/// Zeichen-Deckelung würde mitten im Wort/Emoji abschneiden; das hier
/// bricht bevorzugt an `\n`, nur ein zu langer Einzelabschnitt wird hart
/// geschnitten.
fn in_stuecke(text: &str, max: usize) -> Vec<String> {
    if text.chars().count() <= max {
        return vec![text.to_string()];
    }
    let mut out = Vec::new();
    let mut aktuell = String::new();
    for zeile in text.split_inclusive('\n') {
        if !aktuell.is_empty() && aktuell.chars().count() + zeile.chars().count() > max {
            out.push(std::mem::take(&mut aktuell));
        }
        aktuell.push_str(zeile);
        while aktuell.chars().count() > max {
            let cut: String = aktuell.chars().take(max).collect();
            let rest_len = cut.chars().count();
            out.push(cut);
            aktuell = aktuell.chars().skip(rest_len).collect();
        }
    }
    if !aktuell.is_empty() {
        out.push(aktuell);
    }
    out
}

/// Reicht AgentEvents aus einem laufenden Auftrag an den Poll-Loop durch.
/// `ereignis()` darf laut `Ui`-Vertrag nicht blockieren - das unbounded
/// `send()` ist synchron und passt genau deshalb.
struct TelegramUi {
    tx: tokio::sync::mpsc::UnboundedSender<AgentEvent>,
}

impl Ui for TelegramUi {
    fn ereignis(&self, ereignis: AgentEvent) {
        let _ = self.tx.send(ereignis);
    }
}

/// Baut die Antwort für `/status`: aktuelles Modell aus der Config und
/// das Guthaben beim aktiven Provider. Läuft ohne LLM-Aufruf.
async fn status_text() -> String {
    let config = match Config::load() {
        Ok(c) => c,
        Err(e) => return format!("✗ Konnte Konfiguration nicht lesen: {e:#}"),
    };

    let modell = if config.modell_modus == "automatisch" {
        "automatisch (Famulus wählt)".to_string()
    } else {
        format!(
            "{} · {}",
            config.provider,
            config.model.clone().unwrap_or_else(|| "Standardmodell".to_string())
        )
    };

    let credits = crate::credits::anzeigen(&config)
        .await
        .unwrap_or_else(|e| format!("Fehler ({})", e));

    format!("🤖 Modell: {modell}\n💰 Credits: {credits}")
}

/// Baut einen frischen Agenten (wie die GUI pro Auftrag) und fährt ihn mit
/// dem bisherigen Verlauf dieses Chats. Wichtig: `run_task` kann mit einem
/// Fehler zurückkommen, OHNE vorher `Fertig`/`Abgebrochen` gesendet zu haben
/// (z.B. genau der reqwest-Timeout gegen hyper.charm.land, der Jens neulich
/// begegnet ist) - deshalb hier `select!` auf das Task-Future selbst statt
/// nur auf ein Abschluss-Ereignis zu warten. Wer nur auf `Fertig` wartet,
/// hängt bei jedem Fehler, der vor dem ersten Ereignis auftritt, für immer.
/// Vorsortiertes Update für den Hauptloop: Textnachricht oder aufgelöster
/// Button-Klick, jeweils nur aus autorisierten Chats.
struct Eingehend {
    chat_id: i64,
    text: String,
}

/// Sender für Zwischenfragen an den gerade laufenden Auftrag - das Gegenstück
/// zu `ZWISCHENFRAGE_KANAL` aus ffi.rs, hier im Telegram-Prozess. Gesetzt,
/// solange ein Auftrag läuft; der Hauptloop legt eingehende Nachrichten hier
/// ab, damit der Agent sie sofort separat beantwortet (agent.rs,
/// „Zwischenfragen einspeisen“).
static ZWISCHENFRAGE_TELEGRAM: LazyLock<Mutex<Option<tokio::sync::mpsc::UnboundedSender<String>>>> =
    LazyLock::new(|| Mutex::new(None));

/// 1, solange der Hauptloop in einem Agent-Lauf steckt. Darüber entscheidet
/// der Hauptloop, ob eine eingehende Nachricht als Zwischenfrage in den
/// laufenden Auftrag geht (1) oder als eigener Auftrag behandelt wird (0).
static AUFTRAG_AKTIV: AtomicBool = AtomicBool::new(false);

pub async fn run(cfg: TelegramConfig) -> Result<()> {
    let client = reqwest::Client::new();
    let username = get_me(&client, &cfg.token).await?;
    println!(
        "[telegram] Verbunden als @{username}, erlaubte Chats: {:?}",
        cfg.allowed_chat_ids
    );

    // Verlauf pro Chat, nur im Prozessspeicher - überlebt einen Neustart
    // absichtlich nicht, genau wie die GUI ihn nicht in eine Datei schreibt.
    let verlaeufe: Arc<Mutex<HashMap<i64, Vec<Message>>>> =
        Arc::new(Mutex::new(HashMap::new()));

    // Der Poll-Task ist der EINZIGE getUpdates-Aufrufer und läuft dauerhaft.
    // Eingehende Nachrichten gehen in einen unbegrenzten Kanal; solange der
    // Hauptloop beschäftigt ist, puffern sie dort (bzw. werden während eines
    // Agent-Laufs sofort als Zwischenfrage weitergeleitet, siehe unten).
    // Wichtig: Weil es genau einen getUpdates-Aufrufer gibt, kann es keine
    // offset-Konflikte geben - ein zweiter Poll während eines Agent-Laufs
    // wäre genau der Fehler des alten seriellen Designs gewesen.
    let (eingehend_tx, mut eingehend_rx) =
        tokio::sync::mpsc::unbounded_channel::<Eingehend>();

    let poll_client = client.clone();
    let poll_token = cfg.token.clone();
    let poll_chats = cfg.allowed_chat_ids.clone();

    let _poll_task = tokio::spawn(async move {
        let mut offset: i64 = 0;
        loop {
            let updates = match get_updates(&poll_client, &poll_token, offset).await {
                Ok(u) => u,
                Err(e) => {
                    eprintln!("[telegram] getUpdates fehlgeschlagen: {e:#} - warte 5s");
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    continue;
                }
            };

            if updates.is_empty() {
                // Telegram long-poll: Leere Antwort = nichts Neues. Kurz
                // schlafen, damit der Loop nicht heiß läuft.
                tokio::time::sleep(Duration::from_millis(200)).await;
                continue;
            }

            for update in updates {
                offset = offset.max(update.update_id + 1);

                if let Some(cb) = update.callback_query {
                    beantworte_callback(&poll_client, &poll_token, &cb.id).await;
                    let Some(msg) = cb.message else { continue };
                    let chat_id = msg.chat.id;
                    if !poll_chats.contains(&chat_id) {
                        continue;
                    }
                    let gewaehlt = cb
                        .data
                        .as_deref()
                        .and_then(|d| d.parse::<usize>().ok())
                        .and_then(|i| msg.reply_markup.as_ref().map(|m| (i, m)))
                        .and_then(|(i, markup)| markup.inline_keyboard.get(i))
                        .and_then(|zeile| zeile.first())
                        .map(|knopf| knopf.text.clone());
                    let Some(text) = gewaehlt else {
                        eprintln!("[telegram] Button-Klick ohne auflösbare Option (chat_id={chat_id})");
                        continue;
                    };
                    entferne_buttons(&poll_client, &poll_token, chat_id, msg.message_id).await;
                    println!("[telegram] Button-Antwort von chat_id={chat_id}: {text}");
                    let _ = eingehend_tx.send(Eingehend { chat_id, text });
                    continue;
                }

                let Some(msg) = update.message else { continue };
                let Some(text) = msg.text else { continue };
                let chat_id = msg.chat.id;
                if !poll_chats.contains(&chat_id) {
                    let wer = msg
                        .from
                        .and_then(|f| f.username)
                        .unwrap_or_else(|| "unbekannt".to_string());
                    eprintln!(
                        "[telegram] Abgewiesen: chat_id={chat_id} (@{wer}) ist nicht auf der Allowlist"
                    );
                    let _ = send_message(&poll_client, &poll_token, chat_id, "Nicht autorisiert.").await;
                    continue;
                }
                println!("[telegram] Nachricht von chat_id={chat_id}: {text}");
                let _ = eingehend_tx.send(Eingehend { chat_id, text });
            }
        }
    });

    // Hauptloop: verarbeitet die eingehenden Nachrichten seriell.
    while let Some(eingehend) = eingehend_rx.recv().await {
        let chat_id = eingehend.chat_id;

        // ── Schnellbefehl /status: kostet nichts, antwortet sofort ──
        if eingehend.text.trim() == "/status" {
            let _ = send_message(&client, &cfg.token, chat_id, &status_text().await).await;
            continue;
        }

        let mut verlauf_guard = verlaeufe.lock().unwrap_or_else(|e| e.into_inner());
        let verlauf_vec: &mut Vec<Message> =
            verlauf_guard.entry(chat_id).or_default();

        // ── /reset: Verlauf sichern und löschen (eigener Code, kein Agent) ──
        if eingehend.text.trim() == "/reset" {
            if verlauf_vec.is_empty() {
                let _ = send_message(&client, &cfg.token, chat_id, "Kein Chatverlauf zum Zurücksetzen.").await;
                continue;
            }
            let _ = send_message(&client, &cfg.token, chat_id, "🧠 Ich sichere kurz das Wichtigste aus unserem Chat, dann räume ich auf …").await;
            let vorbereitung = (|| -> Result<(Config, Box<dyn llm::LlmProvider>)> {
                let config = Config::load()?;
                let provider = llm::build_provider(&config)?;
                Ok((config, provider))
            })();
            let (config, provider) = match vorbereitung {
                Ok(x) => x,
                Err(e) => {
                    let _ = send_message(&client, &cfg.token, chat_id, &format!("✗ Kann /reset nicht ausführen: {e:#}")).await;
                    continue;
                }
            };
            let (tx, mut reset_rx) = tokio::sync::mpsc::unbounded_channel();
            let ui: Arc<dyn Ui> = Arc::new(TelegramUi { tx });
            let agent = Agent::new(config, provider, Arc::clone(&ui)).await;
            let (_zf_tx, zf_rx) = tokio::sync::mpsc::unbounded_channel();
            let vorherige = verlauf_vec.clone();
            let n = vorherige.len();
            let reset_prompt = format!(
                "Der Chatverlauf ({n} Nachrichten) wird gleich gelöscht.              Fasse zuerst das Wichtigste zusammen, das ich mir merken sollte:              Fakten über Jens, seine Präferenzen, laufende Projekte, offene              Aufgaben und Lektionen. Nutze das notizbuch-Tool, um die wichtigsten              Erkenntnisse (maximal 10) zu speichern. Gib dann eine kurze              Zusammenfassung, was du gespeichert hast."
            );
            let lauf = agent.run_task(&vorherige, &reset_prompt, zf_rx);
            tokio::pin!(lauf);
            let mut assembled = String::new();
            let mut abbruch: Option<String> = None;
            let lauf_ergebnis = loop {
                tokio::select! {
                    biased;
                    ergebnis = &mut lauf => {
                        // `lauf` kann seine letzten Events (z.B. den finalen
                        // Text) und die Fertigstellung im selben Poll
                        // auslösen - durch `biased` gewinnt dieser Zweig dann,
                        // OHNE dass `reset_rx.recv()` noch drankäme, und die
                        // bereits im Kanal wartenden Events gingen sonst beim
                        // Verlassen der Schleife verloren (sichtbar als leere
                        // Antwort / "(keine Antwort)"). Deshalb hier vor dem
                        // Verlassen noch alles nicht-blockierend abholen.
                        while let Ok(ereignis) = reset_rx.try_recv() {
                            match ereignis {
                                AgentEvent::Text { chunk } => assembled.push_str(&chunk),
                                AgentEvent::Abgebrochen { fehler } => abbruch = Some(fehler),
                                _ => {}
                            }
                        }
                        break ergebnis;
                    }
                    ereignis = reset_rx.recv() => {
                        match ereignis {
                            Some(AgentEvent::Text { chunk }) => assembled.push_str(&chunk),
                            Some(AgentEvent::Abgebrochen { fehler }) => abbruch = Some(fehler),
                            Some(_) => {}
                            None => {}
                        }
                    }
                }
            };
            let mut antwort = assembled;
            if let Some(f) = abbruch {
                if !antwort.is_empty() { antwort.push_str("

"); }
                antwort.push_str(&format!("✗ {f}"));
            }
            if let Err(e) = lauf_ergebnis {
                if !antwort.is_empty() { antwort.push_str("

"); }
                antwort.push_str(&format!("✗ {e:#}"));
            }
            verlauf_vec.clear();
            let bestaetigung = format!("✅ Chatverlauf gelöscht.

{}", antwort);
            let _ = send_message(&client, &cfg.token, chat_id, &bestaetigung).await;
            continue;
        }

        // ── Sofort antworten statt einreihen: Nachricht, die während eines
        // laufenden Auftrags reinkommt, wird NICHT als eigener Auftrag
        // behandelt (das öffnete einen zweiten parallelen Agent-Lauf mit
        // demselben Verlauf), sondern als Zwischenfrage in den laufenden
        // Auftrag eingespeist. Siehe agent.rs.
        if AUFTRAG_AKTIV.load(Ordering::SeqCst) {
            if let Some(tx) = ZWISCHENFRAGE_TELEGRAM
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_ref()
            {
                let _ = tx.send(eingehend.text);
            }
            continue;
        }

        let vorbereitung = (|| -> Result<(Config, Box<dyn llm::LlmProvider>)> {
            let config = Config::load()?;
            let provider = llm::build_provider(&config)?;
            Ok((config, provider))
        })();

        let (config, provider) = match vorbereitung {
            Ok(x) => x,
            Err(e) => {
                let _ = send_message(&client, &cfg.token, chat_id, &format!("✗ Konnte nicht starten: {e:#}")).await;
                continue;
            }
        };

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let ui: Arc<dyn Ui> = Arc::new(TelegramUi { tx });
        let agent = Agent::new(config, provider, Arc::clone(&ui)).await;

        let text = eingehend.text.clone();
        let vorherige = verlauf_vec.clone();
        let (zf_tx, zf_rx) = tokio::sync::mpsc::unbounded_channel();
        *ZWISCHENFRAGE_TELEGRAM.lock().unwrap_or_else(|e| e.into_inner()) = Some(zf_tx);

        AUFTRAG_AKTIV.store(true, Ordering::SeqCst);

        let lauf = agent.run_task(&vorherige, &text, zf_rx);
        tokio::pin!(lauf);

        let mut assembled = String::new();
        let mut abbruch: Option<String> = None;

        let lauf_ergebnis = loop {
            tokio::select! {
                biased;
                // `lauf` zuerst: Ist er fertig, gewinnt er - so wird eine
                // Nachricht, die im selben Moment eintrifft, nicht noch als
                // Zwischenfrage an einen schon beendeten Agenten geschickt
                // (die wäre verloren), sondern nach dem Auftrag normal
                // verarbeitet.
                ergebnis = &mut lauf => {
                    // `lauf` kann seine letzten Events (insbesondere den
                    // finalen Text) und die Fertigstellung im selben Poll
                    // auslösen - durch `biased` gewinnt dieser Zweig dann,
                    // OHNE dass `rx.recv()` noch drankäme, und der bereits im
                    // Kanal wartende Text ginge beim Verlassen der Schleife
                    // verloren (sichtbar als "(keine Antwort)" in Telegram,
                    // obwohl das Modell längst geantwortet hatte). Deshalb
                    // hier vor dem Verlassen noch alles nicht-blockierend
                    // abholen.
                    while let Ok(ereignis) = rx.try_recv() {
                        match ereignis {
                            AgentEvent::Text { chunk } => assembled.push_str(&chunk),
                            AgentEvent::ToolStart { name, args } => {
                                let vorschau: String = args.to_string().chars().take(200).collect();
                                let _ = send_message(&client, &cfg.token, chat_id, &format!("⚙ {name}({vorschau})")).await;
                            }
                            AgentEvent::Abgebrochen { fehler } => abbruch = Some(fehler),
                            AgentEvent::ZwischenfrageAntwort { frage, text } => {
                                let _ = send_message(&client, &cfg.token, chat_id, &format!("↩ Zu \"{frage}\": {text}")).await;
                            }
                            AgentEvent::Warte { grund, sekunden, versuch, max_versuche } => {
                                let grund: String = grund.chars().take(300).collect();
                                let _ = send_message(&client, &cfg.token, chat_id, &format!("⏳ Fehlgeschlagen ({versuch}/{max_versuche}), versuche in {sekunden}s erneut: {grund}")).await;
                            }
                            AgentEvent::FrageAnNutzer { frage, optionen } => {
                                if let Err(e) = send_message_mit_optionen(&client, &cfg.token, chat_id, &frage, &optionen).await {
                                    eprintln!("[telegram] Rückfrage senden fehlgeschlagen: {e:#}");
                                }
                            }
                            AgentEvent::Zwischenstand { text } => {
                                let _ = send_message(&client, &cfg.token, chat_id, &format!("◷ Zwischenstand: {text}")).await;
                            }
                            _ => {}
                        }
                    }
                    break ergebnis;
                }
                ereignis = rx.recv() => {
                    match ereignis {
                        Some(AgentEvent::Text { chunk }) => assembled.push_str(&chunk),
                        Some(AgentEvent::ToolStart { name, args }) => {
                            let vorschau: String = args.to_string().chars().take(200).collect();
                            let _ = send_message(&client, &cfg.token, chat_id, &format!("⚙ {name}({vorschau})")).await;
                        }
                        Some(AgentEvent::Abgebrochen { fehler }) => abbruch = Some(fehler),
                        Some(AgentEvent::ZwischenfrageAntwort { frage, text }) => {
                            let _ = send_message(&client, &cfg.token, chat_id, &format!("↩ Zu \"{frage}\": {text}")).await;
                        }
                        Some(AgentEvent::Warte { grund, sekunden, versuch, max_versuche }) => {
                            let grund: String = grund.chars().take(300).collect();
                            let _ = send_message(&client, &cfg.token, chat_id, &format!("⏳ Fehlgeschlagen ({versuch}/{max_versuche}), versuche in {sekunden}s erneut: {grund}")).await;
                        }
                        Some(AgentEvent::FrageAnNutzer { frage, optionen }) => {
                            if let Err(e) = send_message_mit_optionen(&client, &cfg.token, chat_id, &frage, &optionen).await {
                                eprintln!("[telegram] Rückfrage senden fehlgeschlagen: {e:#}");
                            }
                        }
                        Some(AgentEvent::Zwischenstand { text }) => {
                            let _ = send_message(&client, &cfg.token, chat_id, &format!("◷ Zwischenstand: {text}")).await;
                        }
                        Some(_) => {}
                        None => {}
                    }
                }
                // Während der Agent läuft, eingehende Nachrichten als
                // Zwischenfragen weiterreichen - das war die Kern-Stille:
                // alte Version las erst nach dem Auftrag wieder.
                nachricht = eingehend_rx.recv() => {
                    match nachricht {
                        Some(n) => {
                            if n.text.trim() == "/status" {
                                let _ = send_message(&client, &cfg.token, n.chat_id, &status_text().await).await;
                                continue;
                            }
                            if let Some(tx) = ZWISCHENFRAGE_TELEGRAM
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .as_ref()
                            {
                                let _ = tx.send(n.text);
                            }
                        }
                        None => {}
                    }
                }
            }
        };

        AUFTRAG_AKTIV.store(false, Ordering::SeqCst);
        *ZWISCHENFRAGE_TELEGRAM.lock().unwrap_or_else(|e| e.into_inner()) = None;

        let mut antwort = assembled;
        if let Some(f) = abbruch {
            if !antwort.is_empty() { antwort.push_str("

"); }
            antwort.push_str(&format!("✗ {f}"));
        }
        if let Err(e) = lauf_ergebnis {
            if !antwort.is_empty() { antwort.push_str("

"); }
            antwort.push_str(&format!("✗ {e:#}"));
        }

        if let Err(e) = send_message(&client, &cfg.token, chat_id, &antwort).await {
            eprintln!("[telegram] Antwort senden fehlgeschlagen: {e:#}");
        }

        verlauf_vec.push(Message::User(text.clone()));
        verlauf_vec.push(Message::Assistant {
            text: antwort,
            tool_calls: Vec::new(),
        });
    }
    Ok(())
}
