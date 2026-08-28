//! Famulus-Kern.
//!
//! Bewusst als Bibliothek und nicht nur als Programm: Kommandozeile und GUI
//! benutzen exakt denselben Agenten, dieselbe Berechtigungslogik und dieselbe
//! Deny-Liste. Ohne das entstünden zwei Kopien der Sicherheitsregeln, die
//! irgendwann auseinanderlaufen - und die GUI-Kopie wäre die, an die niemand
//! denkt, wenn eine Lücke gefixt wird.
//!
//! Was die Oberfläche austauscht, ist einzig das `Ui`-Trait: wohin die
//! Ereignisse gehen. Was erlaubt ist, entscheidet immer der Kern - siehe
//! `permissions.rs`, wo auch steht, wie wenig das inzwischen ist.

pub mod agent;
pub mod config;
/// Guthaben-Abfrage (Hyper/OpenRouter), von GUI und Telegram-Bot genutzt
pub mod credits;
/// UniFFI-Brücke für die native Swift-Hülle (swift-app/). Enthält auch die
/// Logik, die vorher nur im Tauri-GUI wohnte (Modell-Liste, TOML-Schalter,
/// History-Zugriff), damit der Kern sie allen Hüllen anbieten kann.
pub mod ffi;
/// Chat-Verlauf mit Volltextsuche (SQLite)
pub mod history;
pub mod llm;
pub mod memory;
/// System-Prompt-Presets mit Dropdown-Umschaltung
pub mod presets;
pub mod permissions;
/// Famulus als Telegram-Bot ansprechbar machen (Binary `famulus-telegram`).
pub mod telegram;
pub mod tools;
pub mod ui;

// UniFFI-Scaffolding muss im Crate-Root expandiert werden (erzeugt
// crate::UniFfiTag). Die UDL-Funktionen finden sich über die Re-Exporte
// unten; `zustand`, `credits` usw. liegen in ffi.rs.
uniffi::include_scaffolding!("ffi");
pub use ffi::{
    app_version, credits, history_archiv_liste, history_archivieren, history_liste,
    history_aktualisieren, history_loeschen, history_speichern, history_suche, modelle_liste, presets_aktivieren,
    presets_liste, presets_loeschen, presets_speichern, setze_modell, setze_modell_modus,
    starte_auftrag, stoppe_auftrag, zwischenfrage, zustand, AuftragsCallback, Fehler,
};
