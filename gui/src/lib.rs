// Famulus-GUI Bibliothek. Tauri 2 für iOS braucht ein [lib]-Target;
// main.rs ruft nur run() auf, das eigentliche Setup steht hier.

mod remote;

use famulus_core::agent::Agent;
use famulus_core::config::Config;
use famulus_core::llm::{self, BildAnhang, Message};
use famulus_core::ui::{AgentEvent, Ui};
use std::sync::{Arc, LazyLock, Mutex};
use tauri::{AppHandle, Emitter, State};

static CLIENT: LazyLock<reqwest::Client> = LazyLock::new(reqwest::Client::new);

/// Hält den zuletzt gestarteten Auftrag fest, damit der Stopp-Button ihn
/// abbrechen kann. Nur einer gleichzeitig - die Oberfläche lässt gar keinen
/// zweiten zu, solange der erste läuft.
struct LaufenderAuftrag(Mutex<Option<tauri::async_runtime::JoinHandle<()>>>);

struct TauriUi {
    app: AppHandle,
}

impl Ui for TauriUi {
    fn ereignis(&self, ereignis: AgentEvent) {
        let _ = self.app.emit("famulus-ereignis", ereignis);
    }
}

// ── Datei-Upload: Von Frontend kommende Anhänge ──────────────────────────

#[derive(serde::Deserialize)]
struct UploadAnhang {
    name: String,
    medien_typ: String,
    base64: String,
}

#[derive(serde::Deserialize)]
struct VerlaufEintrag {
    rolle: String,
    inhalt: String,
    /// Optionale Anhänge, die das Frontend mitschickt (Bilder, Dateien).
    #[serde(default)]
    anhaenge: Vec<UploadAnhang>,
}

fn verlauf_zu_nachrichten(verlauf: Vec<VerlaufEintrag>) -> Vec<Message> {
    verlauf
        .into_iter()
        .map(|e| {
            if e.rolle == "user" {
                if e.anhaenge.is_empty() {
                    Message::User(e.inhalt)
                } else {
                    let bilder: Vec<BildAnhang> = e.anhaenge
                        .into_iter()
                        .map(|a| BildAnhang {
                            medien_typ: a.medien_typ,
                            base64: a.base64,
                        })
                        .collect();
                    Message::UserMitBild {
                        text: e.inhalt,
                        bilder,
                    }
                }
            } else {
                Message::Assistant {
                    text: e.inhalt,
                    tool_calls: Vec::new(),
                }
            }
        })
        .collect()
}

// ── Tauri Commands ───────────────────────────────────────────────────────

#[tauri::command]
fn starte_auftrag(
    auftrag: String,
    verlauf: Vec<VerlaufEintrag>,
    app: AppHandle,
    laufender: State<'_, LaufenderAuftrag>,
) {
    if let Some(alt) = laufender.0.lock().unwrap().take() {
        alt.abort();
    }

    let handle = tauri::async_runtime::spawn(async move {
        let ui: Arc<dyn Ui> = Arc::new(TauriUi { app: app.clone() });
        let vorherige_nachrichten = verlauf_zu_nachrichten(verlauf);

        let vorbereitung = (|| -> anyhow::Result<(Config, Box<dyn llm::LlmProvider>)> {
            let config = Config::load()?;
            let provider = llm::build_provider(&config)?;
            Ok((config, provider))
        })();

        match vorbereitung {
            Ok((config, provider)) => {
                let agent = Agent::new(config, provider, Arc::clone(&ui));
                if let Err(e) = agent.run_task(&vorherige_nachrichten, &auftrag).await {
                    ui.ereignis(AgentEvent::Abgebrochen {
                        fehler: format!("{e:#}"),
                    });
                }
            }
            Err(e) => ui.ereignis(AgentEvent::Abgebrochen {
                fehler: format!("{e:#}"),
            }),
        }
    });

    *laufender.0.lock().unwrap() = Some(handle);
}

#[tauri::command]
fn stoppe_auftrag(app: AppHandle, laufender: State<'_, LaufenderAuftrag>) {
    if let Some(handle) = laufender.0.lock().unwrap().take() {
        handle.abort();
        let _ = app.emit(
            "famulus-ereignis",
            AgentEvent::Abgebrochen {
                fehler: "Abgebrochen.".to_string(),
            },
        );
    }
}

#[tauri::command]
fn pruefe_kanal(app: AppHandle) {
    let _ = app.emit("famulus-kanaltest", ());
}

#[tauri::command]
fn kanal_steht() {
    eprintln!("[famulus] Ereigniskanal steht - das Fenster empfängt Ereignisse.");
}

#[tauri::command]
fn zustand() -> Result<String, String> {
    let config = Config::load().map_err(|e| format!("{e:#}"))?;
    Ok(format!(
        "{} · {} · max. {} Schritte",
        config.provider,
        config.model.unwrap_or_else(|| "Standardmodell".to_string()),
        config.max_turns
    ))
}

#[tauri::command]
async fn credits() -> Result<String, String> {
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
        other => return Err(format!("Unbekannter Provider '{other}'.")),
    };

    let key = std::env::var(&key_var).map_err(|e| format!("{key_var} nicht gesetzt: {e}"))?;

    let body: serde_json::Value = CLIENT
        .get(&url)
        .bearer_auth(&key)
        .send()
        .await
        .map_err(|e| format!("Credits-Anfrage fehlgeschlagen: {e}"))?
        .json()
        .await
        .map_err(|e| format!("Credits-Antwort kein JSON: {e}"))?;

    Ok(format!(
        "{}",
        match config.provider.as_str() {
            "hyper" => {
                let credits = body["balance"].as_f64().unwrap_or(0.0).floor() as i64;
                credits.to_string()
            }
            "openrouter" => {
                let total = body["data"]["total_credits"].as_f64().unwrap_or(0.0);
                let verbraucht = body["data"]["total_usage"].as_f64().unwrap_or(0.0);
                let rest = total - verbraucht;
                (rest.floor() as i64).to_string()
            }
            _ => "?".to_string(),
        }
    ))
}

#[tauri::command]
async fn modelle_liste(provider: String) -> Result<serde_json::Value, String> {
    let config = Config::load().map_err(|e| format!("{e:#}"))?;

    let (url, key_var) = match provider.as_str() {
        "hyper" => (
            format!(
                "{}/v1/models",
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
            "https://openrouter.ai/api/v1/models".to_string(),
            config
                .api_key_env
                .clone()
                .unwrap_or_else(|| "OPENROUTER_API_KEY".to_string()),
        ),
        _ => return Ok(serde_json::json!([])),
    };

    let key = std::env::var(&key_var).map_err(|e| format!("{key_var} nicht gesetzt: {e}"))?;

    let body: serde_json::Value = CLIENT
        .get(&url)
        .bearer_auth(&key)
        .send()
        .await
        .map_err(|e| format!("Modell-Abfrage fehlgeschlagen: {e}"))?
        .json()
        .await
        .map_err(|e| format!("Modell-Antwort kein JSON: {e}"))?;

    let raw = match provider.as_str() {
        "hyper" => body["data"].clone(),
        "openrouter" => body["data"].clone(),
        _ => serde_json::json!([]),
    };

    Ok(raw)
}

#[tauri::command]
async fn setze_modell(provider: String, model: String) -> Result<String, String> {
    let config_path = dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".famulus")
        .join("famulus.toml");

    let mut config: toml::Value = std::fs::read_to_string(&config_path)
        .map_err(|e| format!("famulus.toml lesen fehlgeschlagen: {e}"))?
        .parse()
        .map_err(|e| format!("famulus.toml kein gültiges TOML: {e}"))?;

    if let Some(table) = config.as_table_mut() {
        table.insert("model".to_string(), toml::Value::String(model.clone()));
    }

    std::fs::write(&config_path, toml::to_string_pretty(&config).unwrap())
        .map_err(|e| format!("famulus.toml schreiben fehlgeschlagen: {e}"))?;

    // Das Frontend soll sofort sehen, was jetzt gilt.
    let config = Config::load().map_err(|e| format!("{e:#}"))?;
    Ok(format!(
        "{} · {} · max. {} Schritte",
        config.provider,
        config.model.unwrap_or_else(|| "Standardmodell".to_string()),
        config.max_turns
    ))
}

#[tauri::command]
async fn remote_auftrag(
    auftrag: String,
    verlauf: Vec<VerlaufEintrag>,
    app: AppHandle,
    laufender: State<'_, LaufenderAuftrag>,
) -> Result<(), String> {
    if let Some(alt) = laufender.0.lock().unwrap().take() {
        alt.abort();
    }

    let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel::<AgentEvent>();

    // Leite eingehende Events ans Frontend weiter
    let app_clone = app.clone();
    tauri::async_runtime::spawn(async move {
        while let Some(event) = event_rx.recv().await {
            let _ = app_clone.emit("famulus-ereignis", event);
        }
    });

    let server_ip = remote::mac_tailscale_ip().to_string();

    let verlauf_remote: Vec<remote::RemoteVerlaufEintrag> = verlauf
        .into_iter()
        .map(|e| remote::RemoteVerlaufEintrag {
            rolle: e.rolle,
            inhalt: e.inhalt,
        })
        .collect();

    let handle = tauri::async_runtime::spawn(async move {
        match remote::client_auftrag(&server_ip, &auftrag, verlauf_remote, event_tx).await {
            Ok(()) => {}
            Err(e) => {
                let _ = app.emit(
                    "famulus-ereignis",
                    AgentEvent::Abgebrochen {
                        fehler: format!("{e:#}"),
                    },
                );
            }
        }
    });

    *laufender.0.lock().unwrap() = Some(handle);

    Ok(())
}

/// Remote-Version von zustand: fragt den Mac über WebSocket.
#[tauri::command]
async fn remote_zustand() -> Result<String, String> {
    remote::client_zustand(remote::mac_tailscale_ip()).await
}

/// Remote-Version von credits: fragt den Mac über WebSocket.
#[tauri::command]
async fn remote_credits() -> Result<String, String> {
    remote::client_credits(remote::mac_tailscale_ip()).await
}

#[tauri::command]
async fn remote_modelle_liste(provider: String) -> Result<serde_json::Value, String> {
    remote::client_modelle(remote::mac_tailscale_ip(), &provider).await
}

#[tauri::command]
async fn remote_setze_modell(provider: String, model: String) -> Result<String, String> {
    remote::client_setze_modell(remote::mac_tailscale_ip(), &provider, &model).await
}

#[tauri::command]
fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[tauri::command]
async fn remote_version() -> Result<String, String> {
    remote::client_version(remote::mac_tailscale_ip()).await
}

#[tauri::command]
fn ist_ios() -> bool {
    cfg!(target_os = "ios")
}

pub fn run() {
    tauri::Builder::default()
        .manage(LaufenderAuftrag(Mutex::new(None)))
        .setup(|_app| {
            #[cfg(not(target_os = "ios"))]
            {
                tauri::async_runtime::spawn(async {
                    remote::server_starten().await;
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            starte_auftrag,
            stoppe_auftrag,
            zustand,
            pruefe_kanal,
            kanal_steht,
            credits,
            modelle_liste,
            setze_modell,
            remote_auftrag,
            remote_zustand,
            remote_credits,
            remote_modelle_liste,
            remote_setze_modell,
            version,
            remote_version,
            ist_ios,
        ])
        .run(tauri::generate_context!())
        .expect("Famulus-Fenster konnte nicht gestartet werden");
}

#[cfg(target_os = "ios")]
#[tauri::mobile_entry_point]
fn main() {
    run();
}