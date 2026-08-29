// Famulus – UniFFI-Brücke v0.13.0.
// Dünne FFI-Schicht für die native Swift-Hülle (swift-app/), seit dem
// Entfernen des Tauri-GUI (2026-08-28, archiviert auf Google Drive) die
// einzige grafische Hülle. Enthält auch die Logik, die vorher nur im
// Tauri-GUI wohnte (Modell-Liste, TOML-Umschaltung, History-Zugriff).
// Muster: Famulus Games bridge.rs (dort bewährt seit v0.2.0).

use crate::agent::Agent;
use crate::config::Config;
use crate::history::History;
use crate::llm::{self, BildAnhang, Message};
use crate::presets::PresetsConfig;
use crate::ui::{AgentEvent, Ui};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

// ---------------------------------------------------------------- Runtime

/// Eigene Tokio-Runtime für die FFI-Grenze: Swift ruft synchrone
/// Funktionen auf, die async Arbeit intern hier abwickeln. Die Tauri-GUI
/// nutzt ihre eigene Runtime; CLI/Telegram bleiben unberührt.
static RUNTIME: LazyLock<tokio::runtime::Runtime> = LazyLock::new(|| {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("famulus-ffi")
        .build()
        .expect("Tokio-Runtime für die FFI-Brücke konnte nicht gestartet werden")
});

/// Hält den zuletzt gestarteten Auftrag fest, damit stoppe_auftrag ihn
/// abbrechen kann. Nur einer gleichzeitig - dieselbe Regel wie in der GUI.
/// Die Ereignis-Senke wird mit abgelegt: Ein hart abgebrochener Tokio-Task
/// kann selbst kein `Abgebrochen` mehr senden - stoppe_auftrag emittiert
/// das Ereignis deshalb nach dem abort() selbst (Muster aus der Tauri-GUI,
/// heute archiviert). Das `terminiert`-Atomic entscheidet ohne Renn-Fenster,
/// wer das terminale Ereignis (Fertig/Abgebrochen) senden darf.
struct LaufenderAuftrag {
    handle: tokio::task::JoinHandle<()>,
    ui: Arc<dyn Ui>,
}
static LAUFENDER_AUFTRAG: LazyLock<Mutex<Option<LaufenderAuftrag>>> =
    LazyLock::new(|| Mutex::new(None));

/// Sendehälfte für Zwischenfragen an den gerade laufenden Auftrag - siehe
/// `agent.rs::run_task`. `None` heißt: kein Auftrag läuft gerade.
static ZWISCHENFRAGE_KANAL: LazyLock<Mutex<Option<tokio::sync::mpsc::UnboundedSender<String>>>> =
    LazyLock::new(|| Mutex::new(None));

static CLIENT: LazyLock<reqwest::Client> = LazyLock::new(reqwest::Client::new);

// ---------------------------------------------------------------- Fehler

/// Fehlertyp für die FFI-Grenze: einfache Meldung, kein anyhow.
#[derive(Debug)]
pub enum Fehler {
    Nachricht { meldung: String },
}

impl std::fmt::Display for Fehler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Fehler::Nachricht { meldung } => write!(f, "{meldung}"),
        }
    }
}

impl std::error::Error for Fehler {}

fn fehler(meldung: impl Into<String>) -> Fehler {
    Fehler::Nachricht {
        meldung: meldung.into(),
    }
}

/// Wandelt String-Fehler (wie sie die GUI-Befehle liefern) in den
/// FFI-Fehlertyp um.
fn fehler_s(e: String) -> Fehler {
    fehler(e)
}

// ---------------------------------------------------------------- Callback

/// Ereignis-Senke für die Swift-Hülle. Entspricht dem "famulus-ereignis"-
/// Kanal der Tauri-GUI: Jedes AgentEvent kommt als JSON-String (Feld "art"
/// steuert die Anzeige – derselbe Vertrag wie ui/index.html).
pub trait AuftragsCallback: Send + Sync {
    fn on_ereignis(&self, ereignis_json: String);
}

struct FfiUi {
    cb: Arc<dyn AuftragsCallback>,
}

impl Ui for FfiUi {
    fn ereignis(&self, ereignis: AgentEvent) {
        // Serialize kann bei diesem Enum praktisch nicht scheitern; falls
        // doch, ist Schweigen besser als ein Panic über die FFI-Grenze.
        if let Ok(json) = serde_json::to_string(&ereignis) {
            self.cb.on_ereignis(json);
        }
    }
}

/// Wickelt eine Ereignis-Senke so, dass terminale Ereignisse
/// (Fertig/Abgebrochen) höchstens einmal durchkommen. Der Atomic-Swap ist
/// der Schiedsrichter zwischen dem Task selbst und stoppe_auftrag(): Wird
/// der Auftrag exakt im Moment des Stops fertig, sendet genau eine Seite -
/// früher konnte hier ein `is_finished()`-Check im Renn-Fenster liegen und
/// die Hülle bekam erst „Fertig" und dann „Abgebrochen" (leere Doppel-
/// Nachricht). Normale Ereignisse (Text, ToolStart, ...) laufen unverändert
/// durch.
struct TerminaleUi {
    inner: Arc<dyn Ui>,
    terminiert: Arc<AtomicBool>,
}

impl Ui for TerminaleUi {
    fn ereignis(&self, ereignis: AgentEvent) {
        let terminal = matches!(ereignis, AgentEvent::Fertig | AgentEvent::Abgebrochen { .. });
        if terminal && self.terminiert.swap(true, Ordering::SeqCst) {
            return; // jemand anderes hat das terminale Ereignis schon gesendet
        }
        self.inner.ereignis(ereignis);
    }
}

// ---------------------------------------------------------------- Auftrag

/// Verlauf (JSON-Array) in Kern-Nachrichten wandeln. Gleicher Vertrag wie
/// früher gui/src/lib.rs::verlauf_zu_nachrichten (Tauri-GUI, archiviert).
fn verlauf_zu_nachrichten(json: &str) -> Vec<Message> {
    #[derive(serde::Deserialize)]
    struct Anhang {
        medien_typ: String,
        base64: String,
    }
    #[derive(serde::Deserialize)]
    struct Eintrag {
        rolle: String,
        inhalt: String,
        #[serde(default)]
        anhaenge: Vec<Anhang>,
    }
    let eintraege: Vec<Eintrag> = serde_json::from_str(json).unwrap_or_default();
    eintraege
        .into_iter()
        .map(|e| {
            if e.rolle == "user" {
                if e.anhaenge.is_empty() {
                    Message::User(e.inhalt)
                } else {
                    let bilder: Vec<BildAnhang> = e
                        .anhaenge
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

/// Startet einen Auftrag. Kehrt sofort zurück; die Ereignisse kommen
/// asynchron über das Callback. Ein bereits laufender Auftrag wird vorher
/// abgebrochen (wie in der GUI).
pub fn starte_auftrag(auftrag: String, verlauf_json: String, cb: Box<dyn AuftragsCallback>) {
    if let Some(alt) = LAUFENDER_AUFTRAG.lock().unwrap_or_else(|e| e.into_inner()).take() {
        alt.handle.abort();
        // Wie in stoppe_auftrag(): ein hart abgebrochener Task sendet selbst
        // kein Abschluss-Ereignis mehr. Ohne dieses `Abgebrochen` hier bliebe
        // die Hülle des VORHERIGEN Auftrags für immer im Beschäftigt-Zustand
        // (dieselbe "toter Stop-Button"-Klasse, die stoppe_auftrag löst) -
        // der `terminiert`-Swap in TerminaleUi sorgt dafür, dass das nicht
        // doppelt kommt, falls der alte Task sein eigenes Ereignis im
        // selben Moment noch abgesetzt hat.
        alt.ui.ereignis(AgentEvent::Abgebrochen {
            fehler: "Abgebrochen (neuer Auftrag gestartet).".to_string(),
        });
    }

    let (zf_tx, zf_rx) = tokio::sync::mpsc::unbounded_channel();
    *ZWISCHENFRAGE_KANAL.lock().unwrap_or_else(|e| e.into_inner()) = Some(zf_tx);

    let cb: Arc<dyn AuftragsCallback> = cb.into();
    let terminiert = Arc::new(AtomicBool::new(false));
    let ui: Arc<dyn Ui> = Arc::new(TerminaleUi {
        inner: Arc::new(FfiUi { cb }),
        terminiert: Arc::clone(&terminiert),
    });

    let task_ui = Arc::clone(&ui);
    let handle = RUNTIME.spawn(async move {
        let vorherige_nachrichten = verlauf_zu_nachrichten(&verlauf_json);

        let vorbereitung = (|| -> anyhow::Result<(Config, Box<dyn llm::LlmProvider>)> {
            let config = Config::load()?;
            let provider = llm::build_provider(&config)?;
            Ok((config, provider))
        })();

        match vorbereitung {
            Ok((config, provider)) => {
                let agent = Agent::new(config, provider, Arc::clone(&task_ui)).await;
                if let Err(e) = agent.run_task(&vorherige_nachrichten, &auftrag, zf_rx).await {
                    task_ui.ereignis(AgentEvent::Abgebrochen {
                        fehler: format!("{e:#}"),
                    });
                }
            }
            Err(e) => task_ui.ereignis(AgentEvent::Abgebrochen {
                fehler: format!("{e:#}"),
            }),
        }
    });

    *LAUFENDER_AUFTRAG.lock().unwrap_or_else(|e| e.into_inner()) = Some(LaufenderAuftrag { handle, ui });
}

/// Bricht den laufenden Auftrag ab und meldet der Hülle `Abgebrochen`.
/// Der abort() allein reicht nicht: Ein hart beendeter Tokio-Task sendet
/// selbst kein Abschluss-Ereignis mehr, die Hülle bliebe für immer im
/// Beschäftigt-Zustand (Bug: "toter" Stop-Button). Das Ereignis wird
/// deshalb hier emittiert - exakt das Muster aus der Tauri-GUI (archiviert).
/// Der `terminiert`-Swap ersetzt den früheren `is_finished()`-Check, der
/// ein Renn-Fenster hatte (Task wird zwischen Check und abort fertig).
pub fn stoppe_auftrag() {
    if let Some(auftrag) = LAUFENDER_AUFTRAG.lock().unwrap_or_else(|e| e.into_inner()).take() {
        auftrag.handle.abort();
        // Ob der Task sein Abschluss-Ereignis (Fertig) nicht vielleicht
        // schon selbst gesendet hat, entscheidet der `terminiert`-Swap in
        // TerminaleUi: Hat er, wird dieses `Abgebrochen` verworfen; hat er
        // nicht, kommt es durch und beendet den Beschäftigt-Zustand der
        // Hülle (Bug "toter Stop-Button"). Nicht hier selbst swapen -
        // sonst würde TerminaleUi genau dieses Ereignis wieder filtern.
        auftrag.ui.ereignis(AgentEvent::Abgebrochen {
            fehler: "Abgebrochen.".to_string(),
        });
    }
    *ZWISCHENFRAGE_KANAL.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// Schickt eine Zwischenfrage an den laufenden Auftrag, ohne ihn
/// abzubrechen - siehe agent.rs::run_task.
pub fn zwischenfrage(text: String) {
    if let Some(tx) = ZWISCHENFRAGE_KANAL.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        let _ = tx.send(text);
    }
}

// ---------------------------------------------------------------- Status

/// App-Version, kommt aus Cargo.toml (env!) – Präferenz Jens.
pub fn app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Baut die kurze Statuszeile ("hyper · deepseek-v4-pro · max. 997
/// Schritte") - gleiche Logik wie früher gui/src/lib.rs::zustand_text.
fn zustand_text(config: &Config) -> String {
    let modell_teil = if config.modell_modus == "automatisch" {
        "automatisch (Famulus wählt)".to_string()
    } else {
        format!(
            "{} · {}",
            config.provider,
            config.model.clone().unwrap_or_else(|| "Standardmodell".to_string())
        )
    };
    format!("{modell_teil} · max. {} Schritte", config.max_turns)
}

/// Kurze Statuszeile: Provider · Modell · max. Schritte.
pub fn zustand() -> Result<String, Fehler> {
    let config = Config::load().map_err(|e| fehler(format!("{e:#}")))?;
    Ok(zustand_text(&config))
}

/// Guthaben beim aktiven Provider. Nutzt die Kern-Implementierung aus
/// credits.rs - dieselbe Logik wie der Telegram-Bot.
pub fn credits() -> Result<String, Fehler> {
    let config = Config::load().map_err(|e| fehler(format!("{e:#}")))?;
    RUNTIME
        .block_on(crate::credits::anzeigen(&config))
        .map_err(fehler_s)
}

/// Wie credits(), aber für einen explizit gewählten Provider.
/// Wird gebraucht, wenn der Nutzer im Dropdown den Provider
/// wechselt, ohne dass die Config (famulus.toml) geändert wird -
/// sonst würde credits() immer den in der Config gespeicherten
/// Provider abfragen.
pub fn credits_fuer_provider(provider: String) -> Result<String, Fehler> {
    let mut config = Config::load().map_err(|e| fehler(format!("{e:#}")))?;
    config.provider = provider;
    RUNTIME
        .block_on(crate::credits::anzeigen(&config))
        .map_err(fehler_s)
}

/// Der in der Config (famulus.toml) gespeicherte Provider.
/// Braucht die Swift-Hülle, um beim Start das Dropdown korrekt zu
/// initialisieren und die passenden Credits anzuzeigen.
pub fn aktiver_provider() -> String {
    Config::load()
        .map(|c| c.provider)
        .unwrap_or_else(|_| "hyper".to_string())
}

/// Verfügbare Modelle eines Providers als JSON-Array (id-Feld), gleiche
/// Logik und Filter wie früher gui/src/lib.rs::modelle_liste (archiviert).
pub fn modelle_liste(provider: String) -> Result<String, Fehler> {
    let config = Config::load().map_err(|e| fehler(format!("{e:#}")))?;

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
        other => return Err(fehler(format!("Unbekannter Provider '{other}'."))),
    };

    let key = std::env::var(&key_var)
        .map_err(|e| fehler(format!("{key_var} nicht gesetzt: {e}")))?;

    let body: serde_json::Value = RUNTIME
        .block_on(async {
            CLIENT
                .get(&url)
                .bearer_auth(&key)
                .send()
                .await
                .map_err(|e| format!("Modell-Abfrage fehlgeschlagen: {e}"))?
                .json()
                .await
                .map_err(|e| format!("Modell-Antwort kein JSON: {e}"))
        })
        .map_err(fehler_s)?;

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
                // ":batch"-Varianten laufen nur über OpenRouns separate
                // Batch-API (asynchron, Stunden Latenz) - über den normalen
                // chat/completions-Endpunkt, den Famulus benutzt, liefern
                // sie immer "404: only available through the Batch API".
                // Live geprüft (2026-08-28): 39 von 330 Modellen betroffen.
                .filter(|m| {
                    m["id"]
                        .as_str()
                        .is_some_and(|id| !id.ends_with(":batch"))
                })
                .collect(),
        ),
        _ => serde_json::json!([]),
    };

    serde_json::to_string(&raw).map_err(|e| fehler(format!("{e}")))
}

fn config_toml_pfad() -> std::path::PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".famulus")
        .join("famulus.toml")
}

fn toml_lesen() -> Result<toml::Value, Fehler> {
    let pfad = config_toml_pfad();
    std::fs::read_to_string(&pfad)
        .map_err(|e| fehler(format!("famulus.toml lesen fehlgeschlagen: {e}")))?
        .parse()
        .map_err(|e| fehler(format!("famulus.toml kein gültiges TOML: {e}")))
}

fn toml_schreiben(config: &toml::Value) -> Result<(), Fehler> {
    let serialisiert = toml::to_string_pretty(config)
        .map_err(|e| fehler(format!("famulus.toml serialisieren fehlgeschlagen: {e}")))?;
    std::fs::write(config_toml_pfad(), serialisiert)
        .map_err(|e| fehler(format!("famulus.toml schreiben fehlgeschlagen: {e}")))
}

/// Stellt Provider + Modell um (und schaltet auf "manuell", wie die GUI).
/// Liefert die neue Statuszeile.
pub fn setze_modell(provider: String, model: String) -> Result<String, Fehler> {
    let mut config = toml_lesen()?;
    if let Some(table) = config.as_table_mut() {
        table.insert("provider".to_string(), toml::Value::String(provider));
        table.insert("model".to_string(), toml::Value::String(model));
        // Ein konkretes Modell von Hand zu wählen ist ein expliziter
        // Wunsch - überschreibt "automatisch", sonst würde die eigene Wahl
        // von der automatischen Modellwahl beim nächsten Auftrag wieder
        // verworfen.
        table.insert(
            "modell_modus".to_string(),
            toml::Value::String("manuell".to_string()),
        );
    }
    toml_schreiben(&config)?;
    let config = Config::load().map_err(|e| fehler(format!("{e:#}")))?;
    Ok(zustand_text(&config))
}

/// Schaltet zwischen manueller und automatischer Modellwahl um.
pub fn setze_modell_modus(modus: String) -> Result<String, Fehler> {
    if modus != "manuell" && modus != "automatisch" {
        return Err(fehler(format!("Unbekannter Modell-Modus: '{modus}'")));
    }
    let mut config = toml_lesen()?;
    if let Some(table) = config.as_table_mut() {
        table.insert("modell_modus".to_string(), toml::Value::String(modus));
    }
    toml_schreiben(&config)?;
    let config = Config::load().map_err(|e| fehler(format!("{e:#}")))?;
    Ok(zustand_text(&config))
}

// ---------------------------------------------------------------- Presets

/// Alle Presets + aktives als JSON (gleiche Form wie die Tauri-GUI).
pub fn presets_liste() -> Result<String, Fehler> {
    let presets = PresetsConfig::load().map_err(|e| fehler(format!("{e:#}")))?;
    serde_json::to_string(&presets).map_err(|e| fehler(format!("{e}")))
}

pub fn presets_aktivieren(name: String) -> Result<String, Fehler> {
    let mut presets = PresetsConfig::load().map_err(|e| fehler(format!("{e:#}")))?;
    if !presets.presets.iter().any(|p| p.name == name) {
        return Err(fehler(format!("Preset '{name}' existiert nicht")));
    }
    presets.active = Some(name);
    presets.save().map_err(|e| fehler(format!("{e:#}")))?;
    serde_json::to_string(&presets).map_err(|e| fehler(format!("{e}")))
}

pub fn presets_speichern(name: String, prompt: String) -> Result<String, Fehler> {
    let mut presets = PresetsConfig::load().map_err(|e| fehler(format!("{e:#}")))?;
    if let Some(vorhanden) = presets.presets.iter_mut().find(|p| p.name == name) {
        vorhanden.prompt = prompt;
    } else {
        presets
            .presets
            .push(crate::presets::Preset { name, prompt });
    }
    presets.save().map_err(|e| fehler(format!("{e:#}")))?;
    serde_json::to_string(&presets).map_err(|e| fehler(format!("{e}")))
}

pub fn presets_loeschen(name: String) -> Result<String, Fehler> {
    let mut presets = PresetsConfig::load().map_err(|e| fehler(format!("{e:#}")))?;
    // Verhindern, dass das letzte Preset gelöscht wird
    if presets.presets.len() <= 1 {
        return Err(fehler("Das letzte Preset kann nicht gelöscht werden"));
    }
    presets.presets.retain(|p| p.name != name);
    // Falls das aktive Preset gelöscht wurde, auf das erste umschalten
    if presets.active.as_deref() == Some(name.as_str()) {
        presets.active = presets.presets.first().map(|p| p.name.clone());
    }
    presets.save().map_err(|e| fehler(format!("{e:#}")))?;
    serde_json::to_string(&presets).map_err(|e| fehler(format!("{e}")))
}

// ---------------------------------------------------------------- History

fn history_db() -> Result<History, Fehler> {
    let pfad = dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("KI Agenten")
        .join("famulus")
        .join("gedaechtnis.db");
    History::oeffnen(&pfad).map_err(|e| fehler(format!("History-Datenbank nicht zu öffnen: {e:#}")))
}

fn eintrag_zu_json(e: &crate::history::ChatEintrag) -> serde_json::Value {
    serde_json::json!({
        "id": e.id,
        "titel": e.titel,
        "nachrichten": e.nachrichten,
        "erstellt": e.erstellt,
        "geaendert": e.geaendert,
        "archiviert": e.archiviert,
    })
}

pub fn history_liste() -> Result<String, Fehler> {
    let db = history_db()?;
    let eintraege = db.liste().map_err(|e| fehler(format!("{e:#}")))?;
    let wert = serde_json::Value::Array(eintraege.iter().map(eintrag_zu_json).collect());
    serde_json::to_string(&wert).map_err(|e| fehler(format!("{e}")))
}

pub fn history_suche(begriff: String) -> Result<String, Fehler> {
    let db = history_db()?;
    let eintraege = db.suche(&begriff).map_err(|e| fehler(format!("{e:#}")))?;
    let wert = serde_json::Value::Array(eintraege.iter().map(eintrag_zu_json).collect());
    serde_json::to_string(&wert).map_err(|e| fehler(format!("{e}")))
}

/// Speichert einen Chat, liefert die neue/aktualisierte ID als JSON: {"id": n}.
pub fn history_speichern(titel: String, nachrichten: String) -> Result<String, Fehler> {
    let db = history_db()?;
    let id = db
        .speichern(&titel, &nachrichten)
        .map_err(|e| fehler(format!("{e:#}")))?;
    Ok(format!("{{\"id\": {id}}}"))
}

/// Aktualisiert einen bestehenden Chat (Titel und Nachrichten) in der
/// History-Datenbank. Trennung von speichern/aktualisieren wie in der
/// Tauri-GUI, damit wiederholtes Speichern keine Duplikate erzeugt.
pub fn history_aktualisieren(id: i64, titel: String, nachrichten: String) -> Result<(), Fehler> {
    let db = history_db()?;
    db.aktualisieren(id, &titel, &nachrichten)
        .map_err(|e| fehler(format!("{e:#}")))
}

pub fn history_loeschen(id: i64) -> Result<(), Fehler> {
    let db = history_db()?;
    db.loeschen(id).map_err(|e| fehler(format!("{e:#}")))
}

pub fn history_archiv_liste() -> Result<String, Fehler> {
    let db = history_db()?;
    let eintraege = db.archiv_liste().map_err(|e| fehler(format!("{e:#}")))?;
    let wert = serde_json::Value::Array(eintraege.iter().map(eintrag_zu_json).collect());
    serde_json::to_string(&wert).map_err(|e| fehler(format!("{e}")))
}

pub fn history_archivieren(id: i64, archiviert: bool) -> Result<(), Fehler> {
    let db = history_db()?;
    db.archivieren(id, archiviert)
        .map_err(|e| fehler(format!("{e:#}")))
}

/// Dünner FFI-Durchgriff auf `memory::idle_reflexion` - war bisher
/// geschrieben, aber nie von irgendwoher aufgerufen (kein UDL-Eintrag,
/// keine Swift-Seite). Siehe `FamulusStore.starteIdleReflexion`.
pub fn idle_reflexion() {
    crate::memory::idle_reflexion();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verlauf_parsen_text_und_bilder() {
        let json = r#"[
            {"rolle":"user","inhalt":"Hallo"},
            {"rolle":"assistant","inhalt":"Hi!"},
            {"rolle":"user","inhalt":"Schau mal","anhaenge":[{"medien_typ":"image/png","base64":"QUJD"}]}
        ]"#;
        let nachrichten = verlauf_zu_nachrichten(json);
        assert_eq!(nachrichten.len(), 3);
        assert!(matches!(&nachrichten[0], Message::User(t) if t == "Hallo"));
        assert!(matches!(&nachrichten[2], Message::UserMitBild { bilder, .. } if bilder.len() == 1));
    }

    #[test]
    fn verlauf_parsen_akzeptiert_leeres_json() {
        assert!(verlauf_zu_nachrichten("[]").is_empty());
        assert!(verlauf_zu_nachrichten("kaputt").is_empty());
    }

    #[test]
    fn zustand_und_version_sind_lesbar() {
        assert!(!app_version().is_empty());
        // zustand kann scheitern, wenn keine Config existiert - aber es
        // darf nicht paniken.
        let _ = zustand();
    }

    #[test]
    fn setze_modell_modus_lehnt_unbekannten_modus_ab() {
        assert!(setze_modell_modus("quatsch".into()).is_err());
    }

    // ── TerminaleUi: genau ein terminales Ereignis ────────────────────

    /// Zählt alle Ereignisse - zum Prüfen, was an der Hülle ankommt.
    struct SammelUi {
        ereignisse: Mutex<Vec<AgentEvent>>,
    }

    impl Ui for SammelUi {
        fn ereignis(&self, ereignis: AgentEvent) {
            self.ereignisse.lock().unwrap().push(ereignis);
        }
    }

    #[test]
    fn terminale_ui_laesst_nur_ein_terminales_ereignis_durch() {
        // Das konkrete SammelUi behalten wir als Griff, bevor es hinter
        // `dyn Ui` verschwindet - so können wir das Ergebnis ohne
        // Downcast ablesen. (Direkte Zuweisung statt Arc::clone, weil der
        // Unsize-Coerce nur an der Zuweisungsstelle greift.)
        let sammel = Arc::new(SammelUi {
            ereignisse: Mutex::new(Vec::new()),
        });
        let inner: Arc<dyn Ui> = sammel.clone();
        let terminiert = Arc::new(AtomicBool::new(false));
        let ui = TerminaleUi {
            inner,
            terminiert: Arc::clone(&terminiert),
        };

        // Normales Ereignis kommt immer durch.
        ui.ereignis(AgentEvent::Erinnert { anzahl: 3 });
        // Erstes terminales Ereignis kommt durch.
        ui.ereignis(AgentEvent::Fertig);
        // Zweites terminales (z.B. der parallele abort-Pfad) wird gefiltert.
        ui.ereignis(AgentEvent::Abgebrochen {
            fehler: "Abgebrochen.".to_string(),
        });

        let liste = sammel.ereignisse.lock().unwrap();
        assert_eq!(liste.len(), 2, "Text-Ereignis + genau ein terminales Ereignis");
        assert!(matches!(&liste[0], AgentEvent::Erinnert { anzahl: 3 }));
        assert!(matches!(&liste[1], AgentEvent::Fertig));
    }
}
