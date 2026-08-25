//! Telegram-Anbindung: Famulus über einen Telegram-Bot ansprechbar machen.
//!
//! Läuft als eigener Prozess (Binary `famulus-telegram`), pollt Telegrams
//! `getUpdates` und fährt pro eingehender Nachricht denselben Agent-Lauf wie
//! GUI/CLI - siehe `gui/src/lib.rs::starte_auftrag` fürs Vorbild: der Agent
//! wird pro Auftrag neu gebaut, der Gesprächsverlauf lebt hier im Prozess
//! (pro Chat-ID) statt im Frontend.
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
use std::sync::Arc;
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
}

#[derive(Debug, Deserialize)]
struct TgMessage {
    chat: TgChat,
    text: Option<String>,
    #[serde(default)]
    from: Option<TgUser>,
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
async fn bearbeite_nachricht(
    client: &reqwest::Client,
    token: &str,
    chat_id: i64,
    text: String,
    verlauf: &mut Vec<Message>,
) {
    // Schnellbefehl ohne LLM-Lauf (kostet nichts, antwortet sofort):
    // /status zeigt Modell und Credits. Bewusst vor der ganzen
    // Agent-Vorbereitung abgefangen, damit es auch dann funktioniert,
    // wenn gerade kein Provider erreichbar ist.
    if text.trim() == "/status" {
        let _ = send_message(client, token, chat_id, &status_text().await).await;
        return;
    }

    let vorbereitung = (|| -> Result<(Config, Box<dyn llm::LlmProvider>)> {
        let config = Config::load()?;
        let provider = llm::build_provider(&config)?;
        Ok((config, provider))
    })();

    let (config, provider) = match vorbereitung {
        Ok(x) => x,
        Err(e) => {
            let _ = send_message(client, token, chat_id, &format!("✗ Konnte nicht starten: {e:#}")).await;
            return;
        }
    };

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let ui: Arc<dyn Ui> = Arc::new(TelegramUi { tx });
    let agent = Agent::new(config, provider, Arc::clone(&ui)).await;

    // Keine Zwischenfragen-Unterstützung in v1: eine zweite Telegram-
    // Nachricht, während der Agent noch läuft, wartet einfach auf den
    // nächsten Poll-Durchlauf und wird als eigener Auftrag behandelt -
    // genau wie main.rs es für die CLI schon macht.
    let (_zf_tx, zf_rx) = tokio::sync::mpsc::unbounded_channel();

    let vorherige = verlauf.clone();
    let lauf = agent.run_task(&vorherige, &text, zf_rx);
    tokio::pin!(lauf);

    let mut assembled = String::new();
    let mut abbruch: Option<String> = None;

    let lauf_ergebnis = loop {
        tokio::select! {
            ereignis = rx.recv() => {
                match ereignis {
                    Some(AgentEvent::Text { chunk }) => assembled.push_str(&chunk),
                    Some(AgentEvent::ToolStart { name, args }) => {
                        let vorschau: String = args.to_string().chars().take(200).collect();
                        let _ = send_message(client, token, chat_id, &format!("⚙ {name}({vorschau})")).await;
                    }
                    Some(AgentEvent::Abgebrochen { fehler }) => abbruch = Some(fehler),
                    Some(AgentEvent::ZwischenfrageAntwort { frage, text }) => {
                        let _ = send_message(client, token, chat_id, &format!("↩ Zu \"{frage}\": {text}")).await;
                    }
                    Some(AgentEvent::Warte { grund, sekunden, versuch, max_versuche }) => {
                        let grund: String = grund.chars().take(300).collect();
                        let _ = send_message(
                            client,
                            token,
                            chat_id,
                            &format!("⏳ Fehlgeschlagen ({versuch}/{max_versuche}), versuche in {sekunden}s erneut: {grund}"),
                        )
                        .await;
                    }
                    Some(_) => {}
                    None => {}
                }
            }
            ergebnis = &mut lauf => break ergebnis,
        }
    };

    let mut antwort = assembled;
    if let Some(f) = abbruch {
        if !antwort.is_empty() {
            antwort.push_str("

");
        }
        antwort.push_str(&format!("✗ {f}"));
    }
    if let Err(e) = lauf_ergebnis {
        if !antwort.is_empty() {
            antwort.push_str("

");
        }
        antwort.push_str(&format!("✗ {e:#}"));
    }

    if let Err(e) = send_message(client, token, chat_id, &antwort).await {
        eprintln!("[telegram] Antwort senden fehlgeschlagen: {e:#}");
    }

    verlauf.push(Message::User(text.clone()));
    verlauf.push(Message::Assistant {
        text: antwort,
        tool_calls: Vec::new(),
    });
}

pub async fn run(cfg: TelegramConfig) -> Result<()> {
    let client = reqwest::Client::new();
    let username = get_me(&client, &cfg.token).await?;
    println!(
        "[telegram] Verbunden als @{username}, erlaubte Chats: {:?}",
        cfg.allowed_chat_ids
    );

    let mut offset: i64 = 0;
    // Verlauf pro Chat, nur im Prozessspeicher - überlebt einen Neustart
    // absichtlich nicht, genau wie die GUI ihn nicht in eine Datei schreibt.
    let mut verlaeufe: HashMap<i64, Vec<Message>> = HashMap::new();

    loop {
        let updates = match get_updates(&client, &cfg.token, offset).await {
            Ok(u) => u,
            Err(e) => {
                eprintln!("[telegram] getUpdates fehlgeschlagen: {e:#} - warte 5s");
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
        };

        for update in updates {
            offset = offset.max(update.update_id + 1);

            let Some(msg) = update.message else { continue };
            let Some(text) = msg.text else { continue };
            let chat_id = msg.chat.id;

            if !cfg.allowed_chat_ids.contains(&chat_id) {
                let wer = msg
                    .from
                    .and_then(|f| f.username)
                    .unwrap_or_else(|| "unbekannt".to_string());
                eprintln!(
                    "[telegram] Abgewiesen: chat_id={chat_id} (@{wer}) ist nicht auf der Allowlist"
                );
                let _ = send_message(&client, &cfg.token, chat_id, "Nicht autorisiert.").await;
                continue;
            }

            println!("[telegram] Nachricht von chat_id={chat_id}: {text}");
            let verlauf = verlaeufe.entry(chat_id).or_default();
            bearbeite_nachricht(&client, &cfg.token, chat_id, text, verlauf).await;
        }
    }
}
