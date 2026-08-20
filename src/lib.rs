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
pub mod llm;
pub mod memory;
pub mod permissions;
pub mod tools;
pub mod ui;
