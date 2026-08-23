// WebSocket-Fernbedienung: iPad steuert den Mac über Tailscale.
//
// Architektur:
//   Mac (MagicDNS: macmini.tail29c723.ts.net)  →  WebSocket-Server auf Port 9876
//   iPad                                        →  WebSocket-Client, schickt Aufträge/Anfragen zum Mac
//
// Events fließen vom Mac zurück zum iPad, das sie als Tauri-Events
// ins Frontend durchreicht. Die UI bleibt auf beiden Geräten identisch.

use famulus_core::ui::AgentEvent;
use famulus_core::presets::PresetsConfig;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message as WsMsg;

const SERVER_PORT: u16 = 9876;
const VERBINDUNGS_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(8);

// ── Nachrichten ──────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize)]
#[serde(tag = "typ")]
enum RemoteRequest {
    #[serde(rename = "auftrag")]
    Auftrag {
        auftrag: String,
        verlauf: Vec<RemoteVerlaufEintrag>,
    },
    #[serde(rename = "zustand")]
    Zustand,
    #[serde(rename = "credits")]
    Credits,
    #[serde(rename = "modelle")]
    Modelle { provider: String },
    #[serde(rename = "setze_modell")]
    SetzeModell { provider: String, model: String },
    #[serde(rename = "version")]
    Version,
    #[serde(rename = "presets_liste")]
    PresetsListe,
    #[serde(rename = "presets_aktivieren")]
    PresetsAktivieren { name: String },
    #[serde(rename = "presets_speichern")]
    PresetsSpeichern { name: String, prompt: String },
    #[serde(rename = "presets_loeschen")]
    PresetsLoeschen { name: String },
}

#[derive(Serialize, Deserialize, Clone)]
pub struct RemoteVerlaufEintrag {
    pub rolle: String,
    pub inhalt: String,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "typ")]
enum RemoteResponse {
    #[serde(rename = "event")]
    Event { event: AgentEvent },
    #[serde(rename = "zustand")]
    Zustand { zustand: String },
    #[serde(rename = "credits")]
    Credits { credits: String },
    #[serde(rename = "modelle")]
    Modelle { modelle: serde_json::Value },
    #[serde(rename = "version")]
    Version { version: String },
    #[serde(rename = "presets")]
    Presets { presets: serde_json::Value },
    #[serde(rename = "error")]
    Error { fehler: String },
}

// ── Mac: Server ──────────────────────────────────────────────────────────

use famulus_core::agent::Agent;
use famulus_core::config::Config;
use famulus_core::llm::{self, Message};
use famulus_core::ui::Ui;
use std::sync::Arc;

/// Typ-Alias für die Schreibhälfte einer WebSocket-Verbindung.
type WsTx = futures_util::stream::SplitSink<
    tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
    WsMsg,
>;

/// Startet den WebSocket-Server. Läuft als Hintergrund-Task, solange
/// Famulus auf dem Mac geöffnet ist.
pub async fn server_starten() {
    let addr = format!("0.0.0.0:{SERVER_PORT}");
    let listener = match tokio::net::TcpListener::bind(&addr).await {
        Ok(l) => {
            eprintln!("[remote] Famulus-Fernbedienung bereit auf Port {SERVER_PORT}");
            l
        }
        Err(e) => {
            eprintln!("[remote] Port {SERVER_PORT} nicht verfügbar: {e}");
            return;
        }
    };

    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(c) => c,
            Err(_) => continue,
        };
        eprintln!("[remote] iPad verbunden von {peer}");

        tokio::spawn(async move {
            if let Err(e) = client_betreuen(stream).await {
                eprintln!("[remote] iPad {peer}: {e}");
            }
            eprintln!("[remote] iPad {peer} getrennt");
        });
    }
}

async fn client_betreuen(stream: tokio::net::TcpStream) -> anyhow::Result<()> {
    let ws = tokio_tungstenite::accept_async(stream).await?;
    let (tx, mut rx) = ws.split();
    let tx = Arc::new(tokio::sync::Mutex::new(tx));

    while let Some(msg) = rx.next().await {
        let msg = msg?;
        if let WsMsg::Text(text) = msg {
            let request: RemoteRequest = match serde_json::from_str(&text) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("[remote] Ungültige Nachricht: {e}");
                    let _ = send_error(&tx, &format!("Ungültige Nachricht: {e}")).await;
                    continue;
                }
            };

            match request {
                RemoteRequest::Zustand => {
                    match zustand_ermitteln() {
                        Ok(z) => {
                            let _ = send_response(&tx, &RemoteResponse::Zustand { zustand: z }).await;
                        }
                        Err(e) => {
                            let _ = send_error(&tx, &e).await;
                        }
                    }
                }
                RemoteRequest::Credits => {
                    match credits_ermitteln().await {
                        Ok(c) => {
                            let _ = send_response(&tx, &RemoteResponse::Credits { credits: c }).await;
                        }
                        Err(e) => {
                            let _ = send_error(&tx, &e).await;
                        }
                    }
                }
                RemoteRequest::Modelle { provider } => {
                    match modelle_ermitteln(&provider).await {
                        Ok(m) => {
                            let _ = send_response(&tx, &RemoteResponse::Modelle { modelle: m }).await;
                        }
                        Err(e) => {
                            let _ = send_error(&tx, &e).await;
                        }
                    }
                }
                RemoteRequest::SetzeModell { provider, model } => {
                    match setze_modell_ermitteln(&provider, &model).await {
                        Ok(z) => {
                            let _ = send_response(&tx, &RemoteResponse::Zustand { zustand: z }).await;
                        }
                        Err(e) => {
                            let _ = send_error(&tx, &e).await;
                        }
                    }
                }
                RemoteRequest::Version => {
                    let v = env!("CARGO_PKG_VERSION").to_string();
                    let _ = send_response(&tx, &RemoteResponse::Version { version: v }).await;
                }
                RemoteRequest::PresetsListe => {
                    match presets_ermitteln() {
                        Ok(p) => {
                            let _ = send_response(&tx, &RemoteResponse::Presets { presets: p }).await;
                        }
                        Err(e) => {
                            let _ = send_error(&tx, &e).await;
                        }
                    }
                }
                RemoteRequest::PresetsAktivieren { name } => {
                    match presets_aktivieren_server(&name) {
                        Ok(p) => {
                            let _ = send_response(&tx, &RemoteResponse::Presets { presets: p }).await;
                        }
                        Err(e) => {
                            let _ = send_error(&tx, &e).await;
                        }
                    }
                }
                RemoteRequest::PresetsSpeichern { name, prompt } => {
                    match presets_speichern_server(&name, &prompt) {
                        Ok(p) => {
                            let _ = send_response(&tx, &RemoteResponse::Presets { presets: p }).await;
                        }
                        Err(e) => {
                            let _ = send_error(&tx, &e).await;
                        }
                    }
                }
                RemoteRequest::PresetsLoeschen { name } => {
                    match presets_loeschen_server(&name) {
                        Ok(p) => {
                            let _ = send_response(&tx, &RemoteResponse::Presets { presets: p }).await;
                        }
                        Err(e) => {
                            let _ = send_error(&tx, &e).await;
                        }
                    }
                }
                RemoteRequest::Auftrag { auftrag, verlauf } => {
                    let verlauf: Vec<Message> = verlauf
                        .into_iter()
                        .map(|e| {
                            if e.rolle == "user" {
                                Message::User(e.inhalt)
                            } else {
                                Message::Assistant {
                                    text: e.inhalt,
                                    tool_calls: Vec::new(),
                                }
                            }
                        })
                        .collect();

                    let tx_for_task = Arc::clone(&tx);

                    tokio::spawn(async move {
                        let ui: Arc<dyn Ui> = Arc::new(RemoteUi { tx: tx_for_task.clone() });

                        let vorbereitung = (|| -> anyhow::Result<(Config, Box<dyn llm::LlmProvider>)> {
                            let config = Config::load()?;
                            let provider = llm::build_provider(&config)?;
                            Ok((config, provider))
                        })();

                        match vorbereitung {
                            Ok((config, provider)) => {
                                let agent = Agent::new(config, provider, ui).await;
                                if let Err(e) = agent.run_task(&verlauf, &auftrag).await {
                                    sende_event(&tx_for_task, &AgentEvent::Abgebrochen {
                                        fehler: format!("{e:#}"),
                                    }).await;
                                }
                            }
                            Err(e) => {
                                sende_event(&tx_for_task, &AgentEvent::Abgebrochen {
                                    fehler: format!("{e:#}"),
                                }).await;
                            }
                        }
                    });
                }
            }
        }
    }

    Ok(())
}

fn zustand_ermitteln() -> Result<String, String> {
    let config = Config::load().map_err(|e| format!("{e:#}"))?;
    Ok(format!(
        "{} · {} · max. {} Schritte",
        config.provider,
        config.model.unwrap_or_else(|| "Standardmodell".to_string()),
        config.max_turns
    ))
}

async fn credits_ermitteln() -> Result<String, String> {
    let config = Config::load().map_err(|e| format!("{e:#}"))?;

    let (url, key_var) = match config.provider.as_str() {
        "hyper" => (
            format!(
                "{}/v1/credits",
                config
                    .base_url
                    .as_deref()
                    .unwrap_or("https://hyper.charm.land")
                    .trim_end_matches('/')
            ),
            config
                .api_key_env
                .clone()
                .unwrap_or_else(|| "HYPER_API_KEY".to_string()),
        ),
        "openrouter" => (
            "https://openrouter.ai/api/v1/credits".to_string(),
            config
                .api_key_env
                .clone()
                .unwrap_or_else(|| "OPENROUTER_API_KEY".to_string()),
        ),
        "ollama" => return Ok("lokal".to_string()),
        other => return Err(format!("Unbekannter Provider '{other}'.")),
    };

    let key = std::env::var(&key_var).map_err(|e| format!("{key_var} nicht gesetzt: {e}"))?;

    let client = reqwest::Client::new();
    let body: serde_json::Value = client
        .get(&url)
        .bearer_auth(&key)
        .send()
        .await
        .map_err(|e| format!("Credits-Anfrage fehlgeschlagen: {e}"))?
        .json()
        .await
        .map_err(|e| format!("Credits-Antwort kein JSON: {e}"))?;

    Ok(match config.provider.as_str() {
        "hyper" => {
            let credits = body["balance"].as_f64().unwrap_or(0.0).floor() as i64;
            credits.to_string()
        }
        "openrouter" => {
            let total = body["data"]["total_credits"].as_f64().unwrap_or(0.0);
            let verbraucht = body["data"]["total_usage"].as_f64().unwrap_or(0.0);
            let rest = total - verbraucht;
            rest.floor().to_string()
        }
        _ => unreachable!(),
    })
}
async fn modelle_ermitteln(provider: &str) -> Result<serde_json::Value, String> {
    let config = Config::load().map_err(|e| format!("{e:#}"))?;
    
    let (url, key_var) = match provider {
        "hyper" => (
            format!(
                "{}/v1/models",
                config.base_url.as_deref().unwrap_or("https://hyper.charm.land").trim_end_matches('/')
            ),
            config.api_key_env.clone().unwrap_or_else(|| "HYPER_API_KEY".to_string()),
        ),
        "openrouter" => (
            "https://openrouter.ai/api/v1/models".to_string(),
            config.api_key_env.clone().unwrap_or_else(|| "OPENROUTER_API_KEY".to_string()),
        ),
        "ollama" => {
            let client = reqwest::Client::new();
            let body: serde_json::Value = client
                .get("http://localhost:11434/api/tags")
                .send()
                .await
                .map_err(|e| format!("Ollama-Modell-Abfrage fehlgeschlagen: {e}"))?
                .json()
                .await
                .map_err(|e| format!("Ollama-Modell-Antwort kein JSON: {e}"))?;
            let models = body["models"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .map(|m| serde_json::json!({"id": m["name"], "objekt": "model"}))
                .collect();
            return Ok(serde_json::Value::Array(models));
        },
        other => return Err(format!("Unbekannter Provider '{other}'.")),
    };

    let key = std::env::var(&key_var).map_err(|e| format!("{key_var} nicht gesetzt: {e}"))?;
    let client = reqwest::Client::new();
    let body: serde_json::Value = client
        .get(&url)
        .bearer_auth(&key)
        .send()
        .await
        .map_err(|e| format!("Modell-Anfrage fehlgeschlagen: {e}"))?
        .json()
        .await
        .map_err(|e| format!("Modell-Antwort kein JSON: {e}"))?;

    if provider == "openrouter" {
        // Famulus schickt bei jedem Auftrag Werkzeug-Definitionen mit - ein
        // Modell ohne "tools" in supported_parameters kann damit nichts
        // anfangen und die Anfrage schlaegt fehl.
        let gefiltert: Vec<serde_json::Value> = body["data"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|m| {
                m["supported_parameters"]
                    .as_array()
                    .is_some_and(|p| p.iter().any(|v| v.as_str() == Some("tools")))
            })
            .collect();
        return Ok(serde_json::Value::Array(gefiltert));
    }

    Ok(body.get("data").cloned().unwrap_or(body))
}

async fn setze_modell_ermitteln(provider: &str, model: &str) -> Result<String, String> {
    let pfad = dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".famulus")
        .join("famulus.toml");

    let mut config: toml::Value = std::fs::read_to_string(&pfad)
        .map_err(|e| format!("famulus.toml nicht lesbar: {e}"))?
        .parse()
        .map_err(|e| format!("famulus.toml kein gültiges TOML: {e}"))?;

    config["provider"] = toml::Value::String(provider.to_string());
    config["model"] = toml::Value::String(model.to_string());

    std::fs::write(&pfad, toml::to_string(&config).map_err(|e| format!("toml schreiben fehlgeschlagen: {e}"))?)
        .map_err(|e| format!("Datei schreiben fehlgeschlagen: {e}"))?;

    Ok(format!("{provider} · {model}"))
}


struct RemoteUi {
    tx: Arc<tokio::sync::Mutex<WsTx>>,
}

impl Ui for RemoteUi {
    fn ereignis(&self, ereignis: AgentEvent) {
        let tx = Arc::clone(&self.tx);
        let event = ereignis.clone();
        tokio::task::spawn(async move {
            sende_event(&tx, &event).await;
        });
    }
}

async fn sende_event(tx: &Arc<tokio::sync::Mutex<WsTx>>, event: &AgentEvent) {
    let antwort = RemoteResponse::Event {
        event: event.clone(),
    };
    send_response(tx, &antwort).await;
}

async fn send_response(tx: &Arc<tokio::sync::Mutex<WsTx>>, response: &RemoteResponse) {
    if let Ok(json) = serde_json::to_string(response) {
        let mut guard = tx.lock().await;
        let _ = guard.send(WsMsg::Text(json.into())).await;
    }
}

async fn send_error(tx: &Arc<tokio::sync::Mutex<WsTx>>, fehler: &str) {
    send_response(tx, &RemoteResponse::Error {
        fehler: fehler.to_string(),
    }).await;
}

// ── iPad: Client ─────────────────────────────────────────────────────────

/// Schickt einen Auftrag zum Mac und leitet Events in den Kanal weiter.
// ── Presets (Server-seitig) ──────────────────────────────────────────────

fn presets_ermitteln() -> Result<serde_json::Value, String> {
    let presets = PresetsConfig::load().map_err(|e| format!("{e:#}"))?;
    serde_json::to_value(presets).map_err(|e| format!("{e:#}"))
}

fn presets_aktivieren_server(name: &str) -> Result<serde_json::Value, String> {
    let mut presets = PresetsConfig::load().map_err(|e| format!("{e:#}"))?;
    if !presets.presets.iter().any(|p| p.name == name) {
        return Err(format!("Preset '{name}' existiert nicht"));
    }
    presets.active = Some(name.to_string());
    presets.save().map_err(|e| format!("{e:#}"))?;
    serde_json::to_value(presets).map_err(|e| format!("{e:#}"))
}

fn presets_speichern_server(name: &str, prompt: &str) -> Result<serde_json::Value, String> {
    let mut presets = PresetsConfig::load().map_err(|e| format!("{e:#}"))?;
    if let Some(existing) = presets.presets.iter_mut().find(|p| p.name == name) {
        existing.prompt = prompt.to_string();
    } else {
        presets.presets.push(famulus_core::presets::Preset {
            name: name.to_string(),
            prompt: prompt.to_string(),
        });
    }
    presets.save().map_err(|e| format!("{e:#}"))?;
    serde_json::to_value(presets).map_err(|e| format!("{e:#}"))
}

fn presets_loeschen_server(name: &str) -> Result<serde_json::Value, String> {
    let mut presets = PresetsConfig::load().map_err(|e| format!("{e:#}"))?;
    if presets.presets.len() <= 1 {
        return Err("Das letzte Preset kann nicht gelöscht werden".to_string());
    }
    presets.presets.retain(|p| p.name != name);
    if presets.active.as_deref() == Some(name) {
        presets.active = presets.presets.first().map(|p| p.name.clone());
    }
    presets.save().map_err(|e| format!("{e:#}"))?;
    serde_json::to_value(presets).map_err(|e| format!("{e:#}"))
}

pub async fn client_auftrag(
    server_ip: &str,
    auftrag: &str,
    verlauf: Vec<RemoteVerlaufEintrag>,
    event_tx: mpsc::UnboundedSender<AgentEvent>,
) -> anyhow::Result<()> {
    let url = format!("ws://{server_ip}:{SERVER_PORT}");

    let (ws, _) = tokio::time::timeout(
        VERBINDUNGS_TIMEOUT,
        tokio_tungstenite::connect_async(&url),
    )
    .await
    .map_err(|_| anyhow::anyhow!("Zeitüberschreitung: Keine Verbindung zu {url}. Läuft Tailscale auf beiden Geräten? Ist Famulus auf dem Mac geöffnet?"))?
    .map_err(|e| anyhow::anyhow!("Keine Verbindung zu {url}: {e}"))?;

    let (mut tx, mut rx) = ws.split();

    // Auftrag senden
    let request = RemoteRequest::Auftrag {
        auftrag: auftrag.to_string(),
        verlauf,
    };
    let json = serde_json::to_string(&request)?;
    tx.send(WsMsg::Text(json.into())).await?;

    // Events empfangen und in den Kanal weiterleiten
    while let Some(msg) = rx.next().await {
        let msg = msg?;
        if let WsMsg::Text(text) = msg {
            match serde_json::from_str::<RemoteResponse>(&text) {
                Ok(RemoteResponse::Event { event }) => {
                    if event_tx.send(event).is_err() {
                        break; // Empfänger hat aufgelegt
                    }
                }
                Ok(RemoteResponse::Error { fehler }) => {
                    let _ = event_tx.send(AgentEvent::Abgebrochen {
                        fehler: format!("Server-Fehler: {fehler}"),
                    });
                    break;
                }
                _ => {} // Ignoriere andere Antworttypen im Auftrag-Kontext
            }
        }
    }

    Ok(())
}

/// Fragt den Zustand (Provider, Modell) vom Mac ab.
pub async fn client_zustand(server_ip: &str) -> Result<String, String> {
    let url = format!("ws://{server_ip}:{SERVER_PORT}");

    let (ws, _) = tokio::time::timeout(VERBINDUNGS_TIMEOUT, tokio_tungstenite::connect_async(&url))
        .await
        .map_err(|_| format!("Zeitüberschreitung: Keine Verbindung zu {url}. Läuft Famulus auf dem Mac?"))?
        .map_err(|e| format!("Keine Verbindung: {e}"))?;

    let (mut tx, mut rx) = ws.split();

    let request = RemoteRequest::Zustand;
    let json = serde_json::to_string(&request).map_err(|e| format!("JSON-Fehler: {e}"))?;
    tx.send(WsMsg::Text(json.into()))
        .await
        .map_err(|e| format!("Sende-Fehler: {e}"))?;

    if let Some(msg) = rx.next().await {
        let msg = msg.map_err(|e| format!("Empfangs-Fehler: {e}"))?;
        if let WsMsg::Text(text) = msg {
            match serde_json::from_str::<RemoteResponse>(&text) {
                Ok(RemoteResponse::Zustand { zustand }) => return Ok(zustand),
                Ok(RemoteResponse::Error { fehler }) => return Err(fehler),
                _ => return Err("Unerwartete Antwort vom Server".to_string()),
            }
        }
    }

    Err("Keine Antwort vom Server".to_string())
}

/// Fragt Credits vom Mac ab.
pub async fn client_credits(server_ip: &str) -> Result<String, String> {
    let url = format!("ws://{server_ip}:{SERVER_PORT}");

    let (ws, _) = tokio::time::timeout(VERBINDUNGS_TIMEOUT, tokio_tungstenite::connect_async(&url))
        .await
        .map_err(|_| format!("Zeitüberschreitung: Keine Verbindung zu {url}."))?
        .map_err(|e| format!("Keine Verbindung: {e}"))?;

    let (mut tx, mut rx) = ws.split();

    let request = RemoteRequest::Credits;
    let json = serde_json::to_string(&request).map_err(|e| format!("JSON-Fehler: {e}"))?;
    tx.send(WsMsg::Text(json.into()))
        .await
        .map_err(|e| format!("Sende-Fehler: {e}"))?;

    if let Some(msg) = rx.next().await {
        let msg = msg.map_err(|e| format!("Empfangs-Fehler: {e}"))?;
        if let WsMsg::Text(text) = msg {
            match serde_json::from_str::<RemoteResponse>(&text) {
                Ok(RemoteResponse::Credits { credits }) => return Ok(credits),
                Ok(RemoteResponse::Error { fehler }) => return Err(fehler),
                _ => return Err("Unerwartete Antwort vom Server".to_string()),
            }
        }
    }

    Err("Keine Antwort vom Server".to_string())
}

/// Löst die Tailscale-Adresse des Macs über MagicDNS auf.

/// Fragt die Modellliste vom Mac ab.
pub async fn client_modelle(server_ip: &str, provider: &str) -> Result<serde_json::Value, String> {
    let url = format!("ws://{server_ip}:{SERVER_PORT}");

    let (ws, _) = tokio::time::timeout(VERBINDUNGS_TIMEOUT, tokio_tungstenite::connect_async(&url))
        .await
        .map_err(|_| format!("Zeitüberschreitung: Keine Verbindung zu {url}."))?
        .map_err(|e| format!("Keine Verbindung: {e}"))?;

    let (mut tx, mut rx) = ws.split();

    let request = RemoteRequest::Modelle { provider: provider.to_string() };
    let json = serde_json::to_string(&request).map_err(|e| format!("JSON-Fehler: {e}"))?;
    tx.send(WsMsg::Text(json.into()))
        .await
        .map_err(|e| format!("Sende-Fehler: {e}"))?;

    if let Some(msg) = rx.next().await {
        let msg = msg.map_err(|e| format!("Empfangs-Fehler: {e}"))?;
        if let WsMsg::Text(text) = msg {
            match serde_json::from_str::<RemoteResponse>(&text) {
                Ok(RemoteResponse::Modelle { modelle }) => return Ok(modelle),
                Ok(RemoteResponse::Error { fehler }) => return Err(fehler),
                _ => return Err("Unerwartete Antwort vom Server".to_string()),
            }
        }
    }

    Err("Keine Antwort vom Server".to_string())
}

/// Setzt das Modell auf dem Mac.
pub async fn client_setze_modell(server_ip: &str, provider: &str, model: &str) -> Result<String, String> {
    let url = format!("ws://{server_ip}:{SERVER_PORT}");

    let (ws, _) = tokio::time::timeout(VERBINDUNGS_TIMEOUT, tokio_tungstenite::connect_async(&url))
        .await
        .map_err(|_| format!("Zeitüberschreitung: Keine Verbindung zu {url}."))?
        .map_err(|e| format!("Keine Verbindung: {e}"))?;

    let (mut tx, mut rx) = ws.split();

    let request = RemoteRequest::SetzeModell { provider: provider.to_string(), model: model.to_string() };
    let json = serde_json::to_string(&request).map_err(|e| format!("JSON-Fehler: {e}"))?;
    tx.send(WsMsg::Text(json.into()))
        .await
        .map_err(|e| format!("Sende-Fehler: {e}"))?;

    if let Some(msg) = rx.next().await {
        let msg = msg.map_err(|e| format!("Empfangs-Fehler: {e}"))?;
        if let WsMsg::Text(text) = msg {
            match serde_json::from_str::<RemoteResponse>(&text) {
                Ok(RemoteResponse::Zustand { zustand }) => return Ok(zustand),
                Ok(RemoteResponse::Error { fehler }) => return Err(fehler),
                _ => return Err("Unerwartete Antwort vom Server".to_string()),
            }
        }
    }

    Err("Keine Antwort vom Server".to_string())
}

/// Fragt die Version vom Mac ab.
pub async fn client_version(server_ip: &str) -> Result<String, String> {
    let url = format!("ws://{server_ip}:{SERVER_PORT}");

    let (ws, _) = tokio::time::timeout(VERBINDUNGS_TIMEOUT, tokio_tungstenite::connect_async(&url))
        .await
        .map_err(|_| format!("Zeitüberschreitung: Keine Verbindung zu {url}."))?
        .map_err(|e| format!("Keine Verbindung: {e}"))?;

    let (mut tx, mut rx) = ws.split();

    let request = RemoteRequest::Version;
    let json = serde_json::to_string(&request).map_err(|e| format!("JSON-Fehler: {e}"))?;
    tx.send(WsMsg::Text(json.into()))
        .await
        .map_err(|e| format!("Sende-Fehler: {e}"))?;

    if let Some(msg) = rx.next().await {
        let msg = msg.map_err(|e| format!("Empfangs-Fehler: {e}"))?;
        if let WsMsg::Text(text) = msg {
            match serde_json::from_str::<RemoteResponse>(&text) {
                Ok(RemoteResponse::Version { version }) => return Ok(version),
                Ok(RemoteResponse::Error { fehler }) => return Err(fehler),
                _ => return Err("Unerwartete Antwort vom Server".to_string()),
            }
        }
    }

    Err("Keine Antwort vom Server".to_string())
}

// ── Client: Presets ─────────────────────────────────────────────────────

/// Holt die Presets vom Mac.
pub async fn client_presets_liste(server_ip: &str) -> Result<serde_json::Value, String> {
    let url = format!("ws://{server_ip}:{SERVER_PORT}");

    let (ws, _) = tokio::time::timeout(VERBINDUNGS_TIMEOUT, tokio_tungstenite::connect_async(&url))
        .await
        .map_err(|_| format!("Zeitüberschreitung: Keine Verbindung zu {url}."))?
        .map_err(|e| format!("Keine Verbindung: {e}"))?;

    let (mut tx, mut rx) = ws.split();

    let request = RemoteRequest::PresetsListe;
    let json = serde_json::to_string(&request).map_err(|e| format!("JSON-Fehler: {e}"))?;
    tx.send(WsMsg::Text(json.into()))
        .await
        .map_err(|e| format!("Sende-Fehler: {e}"))?;

    if let Some(msg) = rx.next().await {
        let msg = msg.map_err(|e| format!("Empfangs-Fehler: {e}"))?;
        if let WsMsg::Text(text) = msg {
            match serde_json::from_str::<RemoteResponse>(&text) {
                Ok(RemoteResponse::Presets { presets }) => return Ok(presets),
                Ok(RemoteResponse::Error { fehler }) => return Err(fehler),
                _ => return Err("Unerwartete Antwort vom Server".to_string()),
            }
        }
    }

    Err("Keine Antwort vom Server".to_string())
}

/// Aktiviert ein Preset auf dem Mac.
pub async fn client_presets_aktivieren(server_ip: &str, name: &str) -> Result<serde_json::Value, String> {
    let url = format!("ws://{server_ip}:{SERVER_PORT}");

    let (ws, _) = tokio::time::timeout(VERBINDUNGS_TIMEOUT, tokio_tungstenite::connect_async(&url))
        .await
        .map_err(|_| format!("Zeitüberschreitung: Keine Verbindung zu {url}."))?
        .map_err(|e| format!("Keine Verbindung: {e}"))?;

    let (mut tx, mut rx) = ws.split();

    let request = RemoteRequest::PresetsAktivieren { name: name.to_string() };
    let json = serde_json::to_string(&request).map_err(|e| format!("JSON-Fehler: {e}"))?;
    tx.send(WsMsg::Text(json.into()))
        .await
        .map_err(|e| format!("Sende-Fehler: {e}"))?;

    if let Some(msg) = rx.next().await {
        let msg = msg.map_err(|e| format!("Empfangs-Fehler: {e}"))?;
        if let WsMsg::Text(text) = msg {
            match serde_json::from_str::<RemoteResponse>(&text) {
                Ok(RemoteResponse::Presets { presets }) => return Ok(presets),
                Ok(RemoteResponse::Error { fehler }) => return Err(fehler),
                _ => return Err("Unerwartete Antwort vom Server".to_string()),
            }
        }
    }

    Err("Keine Antwort vom Server".to_string())
}

/// Speichert ein Preset auf dem Mac.
pub async fn client_presets_speichern(server_ip: &str, name: &str, prompt: &str) -> Result<serde_json::Value, String> {
    let url = format!("ws://{server_ip}:{SERVER_PORT}");

    let (ws, _) = tokio::time::timeout(VERBINDUNGS_TIMEOUT, tokio_tungstenite::connect_async(&url))
        .await
        .map_err(|_| format!("Zeitüberschreitung: Keine Verbindung zu {url}."))?
        .map_err(|e| format!("Keine Verbindung: {e}"))?;

    let (mut tx, mut rx) = ws.split();

    let request = RemoteRequest::PresetsSpeichern { name: name.to_string(), prompt: prompt.to_string() };
    let json = serde_json::to_string(&request).map_err(|e| format!("JSON-Fehler: {e}"))?;
    tx.send(WsMsg::Text(json.into()))
        .await
        .map_err(|e| format!("Sende-Fehler: {e}"))?;

    if let Some(msg) = rx.next().await {
        let msg = msg.map_err(|e| format!("Empfangs-Fehler: {e}"))?;
        if let WsMsg::Text(text) = msg {
            match serde_json::from_str::<RemoteResponse>(&text) {
                Ok(RemoteResponse::Presets { presets }) => return Ok(presets),
                Ok(RemoteResponse::Error { fehler }) => return Err(fehler),
                _ => return Err("Unerwartete Antwort vom Server".to_string()),
            }
        }
    }

    Err("Keine Antwort vom Server".to_string())
}

/// Löscht ein Preset auf dem Mac.
pub async fn client_presets_loeschen(server_ip: &str, name: &str) -> Result<serde_json::Value, String> {
    let url = format!("ws://{server_ip}:{SERVER_PORT}");

    let (ws, _) = tokio::time::timeout(VERBINDUNGS_TIMEOUT, tokio_tungstenite::connect_async(&url))
        .await
        .map_err(|_| format!("Zeitüberschreitung: Keine Verbindung zu {url}."))?
        .map_err(|e| format!("Keine Verbindung: {e}"))?;

    let (mut tx, mut rx) = ws.split();

    let request = RemoteRequest::PresetsLoeschen { name: name.to_string() };
    let json = serde_json::to_string(&request).map_err(|e| format!("JSON-Fehler: {e}"))?;
    tx.send(WsMsg::Text(json.into()))
        .await
        .map_err(|e| format!("Sende-Fehler: {e}"))?;

    if let Some(msg) = rx.next().await {
        let msg = msg.map_err(|e| format!("Empfangs-Fehler: {e}"))?;
        if let WsMsg::Text(text) = msg {
            match serde_json::from_str::<RemoteResponse>(&text) {
                Ok(RemoteResponse::Presets { presets }) => return Ok(presets),
                Ok(RemoteResponse::Error { fehler }) => return Err(fehler),
                _ => return Err("Unerwartete Antwort vom Server".to_string()),
            }
        }
    }

    Err("Keine Antwort vom Server".to_string())
}

/// Löst die Tailscale-Adresse des Macs über MagicDNS auf.
pub async fn mac_tailscale_ip() -> String {
    // MagicDNS-Hostname – stabiler als die IP, die sich ändern kann.
    // Wird von Tailscale auf jedem Gerät im Netzwerk aufgelöst.
    const HOSTNAME: &str = "macmini.tail29c723.ts.net";

    // Erst DNS-Auflösung, Fallback auf hartcodierte IP.
    if let Ok(addr) = tokio::net::lookup_host(format!("{HOSTNAME}:{SERVER_PORT}")).await {
        let mut v4: Option<String> = None;
        let mut v6: Option<String> = None;
        for a in addr {
            if a.is_ipv4() {
                v4 = Some(a.ip().to_string());
            } else if v6.is_none() {
                v6 = Some(a.ip().to_string());
            }
        }
        // IPv4 zuerst, fällt nur auf IPv6 zurück, wenn kein IPv4 da ist.
        if let Some(ip) = v4.or(v6) {
            return ip;
        }
    }
    "100.70.211.30".to_string()
}