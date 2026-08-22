// Famulus-GUI Bibliothek. Tauri 2 für iOS braucht ein [lib]-Target;
// main.rs ruft nur run() auf, das eigentliche Setup steht hier.

mod remote;

use famulus_core::agent::Agent;
use famulus_core::config::Config;
use famulus_core::llm::{self, BildAnhang, Message};
use famulus_core::ui::{AgentEvent, Ui};
use famulus_core::history::History;
use famulus_core::presets::PresetsConfig;
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

// ── History-Helfer ──────────────────────────────────────────────────────

// Gibt einen String statt anyhow::Error zurueck, weil das der Fehlertyp ist,
// den alle history_*-Commands unten per `?` an die GUI durchreichen.
// Bewusst kein .expect() mehr: das saesse in einem Tauri-Command-Handler,
// und ein Panic dort beantwortet das zugehoerige JS-Promise nie - das
// Frontend haengt dann fuer immer in "Lade...", ohne Fehlermeldung, ohne
// dass der Rest der App merkt, dass etwas schiefging (siehe famulus-gui
// Absturzserie von v0.5.0/v0.6.0: derselbe Fehlerklasse, andere Stelle).
fn history_db() -> Result<History, String> {
    let pfad = dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("KI Agenten")
        .join("famulus")
        .join("gedaechtnis.db");
    History::oeffnen(&pfad).map_err(|e| format!("History-Datenbank nicht zu öffnen: {e:#}"))
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
                let agent = Agent::new(config, provider, Arc::clone(&ui)).await;
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

    // Ollama: lokale Modelle von /api/tags (kein API-Key nötig).
    if provider == "ollama" {
        let body: serde_json::Value = CLIENT
            .get("http://localhost:11434/api/tags")
            .send()
            .await
            .map_err(|e| format!("Ollama-Modell-Abfrage fehlgeschlagen: {e}"))?
            .json()
            .await
            .map_err(|e| format!("Ollama-Modell-Antwort kein JSON: {e}"))?;
        let models: Vec<serde_json::Value> = body["models"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|m| serde_json::json!({"id": m["name"], "objekt": "model"}))
            .collect();
        return Ok(serde_json::Value::Array(models));
    }

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
        // Famulus schickt bei jedem Auftrag Werkzeug-Definitionen mit - ein
        // Modell ohne "tools" in supported_parameters kann damit nichts
        // anfangen und die Anfrage schlaegt fehl. Solche Modelle werden erst
        // gar nicht zur Auswahl angeboten.
        "openrouter" => serde_json::Value::Array(
            body["data"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter(|m| {
                    m["supported_parameters"]
                        .as_array()
                        .is_some_and(|p| p.iter().any(|v| v.as_str() == Some("tools")))
                })
                .collect(),
        ),
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
        table.insert("provider".to_string(), toml::Value::String(provider.clone()));
        table.insert("model".to_string(), toml::Value::String(model.clone()));
    }

    let serialisiert = toml::to_string_pretty(&config)
        .map_err(|e| format!("famulus.toml serialisieren fehlgeschlagen: {e}"))?;
    std::fs::write(&config_path, serialisiert)
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

    let server_ip = remote::mac_tailscale_ip().await;

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
    remote::client_zustand(&remote::mac_tailscale_ip().await).await
}

/// Remote-Version von credits: fragt den Mac über WebSocket.
#[tauri::command]
async fn remote_credits() -> Result<String, String> {
    remote::client_credits(&remote::mac_tailscale_ip().await).await
}

#[tauri::command]
async fn remote_modelle_liste(provider: String) -> Result<serde_json::Value, String> {
    remote::client_modelle(&remote::mac_tailscale_ip().await, &provider).await
}

#[tauri::command]
async fn remote_setze_modell(provider: String, model: String) -> Result<String, String> {
    remote::client_setze_modell(&remote::mac_tailscale_ip().await, &provider, &model).await
}

// ── Presets ───────────────────────────────────────────────────────────

#[tauri::command]
fn presets_liste() -> Result<serde_json::Value, String> {
    let presets = PresetsConfig::load().map_err(|e| format!("{e:#}"))?;
    serde_json::to_value(presets).map_err(|e| format!("{e:#}"))
}

#[tauri::command]
fn presets_aktivieren(name: String) -> Result<serde_json::Value, String> {
    let mut presets = PresetsConfig::load().map_err(|e| format!("{e:#}"))?;
    // Prüf, dass das Preset existiert
    if !presets.presets.iter().any(|p| p.name == name) {
        return Err(format!("Preset '{name}' existiert nicht"));
    }
    presets.active = Some(name);
    presets.save().map_err(|e| format!("{e:#}"))?;
    serde_json::to_value(presets).map_err(|e| format!("{e:#}"))
}

#[tauri::command]
fn presets_speichern(name: String, prompt: String) -> Result<serde_json::Value, String> {
    let mut presets = PresetsConfig::load().map_err(|e| format!("{e:#}"))?;
    if let Some(existing) = presets.presets.iter_mut().find(|p| p.name == name) {
        existing.prompt = prompt;
    } else {
        presets.presets.push(famulus_core::presets::Preset { name, prompt });
    }
    presets.save().map_err(|e| format!("{e:#}"))?;
    serde_json::to_value(presets).map_err(|e| format!("{e:#}"))
}

#[tauri::command]
fn presets_loeschen(name: String) -> Result<serde_json::Value, String> {
    let mut presets = PresetsConfig::load().map_err(|e| format!("{e:#}"))?;
    // Verhindern, dass das letzte Preset gelöscht wird
    if presets.presets.len() <= 1 {
        return Err("Das letzte Preset kann nicht gelöscht werden".to_string());
    }
    presets.presets.retain(|p| p.name != name);
    // Falls das aktive Preset gelöscht wurde, auf das erste umschalten
    if presets.active.as_deref() == Some(&name) {
        presets.active = presets.presets.first().map(|p| p.name.clone());
    }
    presets.save().map_err(|e| format!("{e:#}"))?;
    serde_json::to_value(presets).map_err(|e| format!("{e:#}"))
}

#[tauri::command]
fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

// ── History (Chat-Verlauf mit Suche) ─────────────────────────────────────

#[tauri::command]
fn history_liste() -> Result<Vec<serde_json::Value>, String> {
    let db = history_db()?;
    let eintraege = db.liste().map_err(|e| format!("{e:#}"))?;
    Ok(eintraege.iter().map(|e| {
        serde_json::json!({
            "id": e.id,
            "titel": e.titel,
            "nachrichten": e.nachrichten,
            "erstellt": e.erstellt,
            "geaendert": e.geaendert,
            "archiviert": e.archiviert,
        })
    }).collect())
}

#[tauri::command]
fn history_suche(begriff: String) -> Result<Vec<serde_json::Value>, String> {
    let db = history_db()?;
    let eintraege = db.suche(&begriff).map_err(|e| format!("{e:#}"))?;
    Ok(eintraege.iter().map(|e| {
        serde_json::json!({
            "id": e.id,
            "titel": e.titel,
            "nachrichten": e.nachrichten,
            "erstellt": e.erstellt,
            "geaendert": e.geaendert,
            "archiviert": e.archiviert,
        })
    }).collect())
}

#[tauri::command]
fn history_speichern(id: Option<i64>, titel: String, nachrichten: String) -> Result<i64, String> {
    let db = history_db()?;
    if let Some(id) = id {
        db.aktualisieren(id, &titel, &nachrichten).map_err(|e| format!("{e:#}"))?;
        Ok(id)
    } else {
        db.speichern(&titel, &nachrichten).map_err(|e| format!("{e:#}"))
    }
}

#[tauri::command]
fn history_loeschen(id: i64) -> Result<(), String> {
    let db = history_db()?;
    db.loeschen(id).map_err(|e| format!("{e:#}"))
}

#[tauri::command]
fn history_archiv_liste() -> Result<Vec<serde_json::Value>, String> {
    let db = history_db()?;
    let eintraege = db.archiv_liste().map_err(|e| format!("{e:#}"))?;
    Ok(eintraege.iter().map(|e| {
        serde_json::json!({
            "id": e.id,
            "titel": e.titel,
            "nachrichten": e.nachrichten,
            "erstellt": e.erstellt,
            "geaendert": e.geaendert,
            "archiviert": e.archiviert,
        })
    }).collect())
}

#[tauri::command]
fn history_archivieren(id: i64, archiviert: bool) -> Result<(), String> {
    let db = history_db()?;
    db.archivieren(id, archiviert).map_err(|e| format!("{e:#}"))
}

/// Remote-Presets: fragt den Mac über WebSocket.
#[tauri::command]
async fn remote_presets_liste() -> Result<serde_json::Value, String> {
    remote::client_presets_liste(&remote::mac_tailscale_ip().await).await
}

#[tauri::command]
async fn remote_presets_aktivieren(name: String) -> Result<serde_json::Value, String> {
    remote::client_presets_aktivieren(&remote::mac_tailscale_ip().await, &name).await
}

#[tauri::command]
async fn remote_presets_speichern(name: String, prompt: String) -> Result<serde_json::Value, String> {
    remote::client_presets_speichern(&remote::mac_tailscale_ip().await, &name, &prompt).await
}

#[tauri::command]
async fn remote_presets_loeschen(name: String) -> Result<serde_json::Value, String> {
    remote::client_presets_loeschen(&remote::mac_tailscale_ip().await, &name).await
}

#[tauri::command]
async fn remote_version() -> Result<String, String> {
    remote::client_version(&remote::mac_tailscale_ip().await).await
}

#[tauri::command]
fn ist_ios() -> bool {
    cfg!(target_os = "ios")
}

pub fn run() {
    // Ohne das verschwindet ein Panic in einem Tauri-Command oder einer
    // spawn()-Task spurlos - die betroffene Aktion haengt oder tut
    // scheinbar nichts, und niemand erfaehrt je, warum (siehe die
    // reqwest::blocking- und history_db()-Faelle: beide waren echte
    // Panics, keiner hat sich von selbst gemeldet). Der Hook ersetzt
    // nicht das Beheben der eigentlichen Ursache, macht sie aber sofort
    // sichtbar statt erst nach langem Suchen.
    std::panic::set_hook(Box::new(|info| {
        eprintln!("[famulus] PANIC: {info}");
    }));

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
            presets_liste,
            presets_aktivieren,
            presets_speichern,
            presets_loeschen,
            version,
            remote_presets_liste,
            remote_presets_aktivieren,
            remote_presets_speichern,
            remote_presets_loeschen,
            remote_version,
            ist_ios,
            history_liste,
            history_suche,
            history_speichern,
            history_loeschen,
            history_archiv_liste,
            history_archivieren,
        ])
        .run(tauri::generate_context!())
        .expect("Famulus-Fenster konnte nicht gestartet werden");
}

#[cfg(target_os = "ios")]
#[tauri::mobile_entry_point]
fn main() {
    run();
}