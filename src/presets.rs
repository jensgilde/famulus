//! System-Prompt-Presets: vorgefertigte und selbst erstellte Prompts, zwischen
//! denen Jens per Dropdown wechseln kann.
//!
//! Gespeichert in ~/.famulus/presets.toml. Existiert die Datei nicht, werden
//! fünf sinnvolle Standard-Presets angelegt. Der aktive Prompt wird in den
//! System-Vorspann des Agenten eingefügt – VOR den Gedächtnis- und Vault-
//! Anweisungen, die der Agent selbst anhängt.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Preset {
    pub name: String,
    pub prompt: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PresetsConfig {
    /// Name des aktiven Presets. None = kein Preset aktiv (dann nur
    /// Gedächtnis/Vault im System-Prompt).
    pub active: Option<String>,
    pub presets: Vec<Preset>,
}

impl Default for PresetsConfig {
    fn default() -> Self {
        Self {
            active: Some("Standard".to_string()),
            presets: vec![
                Preset {
                    name: "Standard".to_string(),
                    prompt: "Du bist Famulus, ein persönlicher KI-Agent für Jens. Du antwortest auf Deutsch, \
                             bist präzise, hilfreich und denkst mit. Du hast Zugriff auf Werkzeuge, um Dateien \
                             zu lesen, Shell-Befehle auszuführen und Notizen zu machen. Nutze sie eigenständig, \
                             wenn sie helfen. Sprich Jens mit seinem Vornamen an."
                        .to_string(),
                },
                Preset {
                    name: "Code-Reviewer".to_string(),
                    prompt: "Du bist ein Senior Software-Entwickler mit 20 Jahren Erfahrung. Du reviewst Code \
                             in Rust, TypeScript, Python und Shell. Du achtest auf: Korrektheit, Sicherheit, \
                             Performance, Lesbarkeit und Idiomatik. Du erklärst deine Kritik knapp und \
                             konstruktiv, mit konkreten Verbesserungsvorschlägen. Unwichtige Stilfragen \
                             ignorierst du. Antworte auf Deutsch."
                        .to_string(),
                },
                Preset {
                    name: "Texter".to_string(),
                    prompt: "Du bist ein professioneller Lektor und Texter für deutsche Texte. Du korrigierst \
                             Rechtschreibung, Grammatik und Zeichensetzung. Du verbesserst Stil, Klarheit und \
                             Lesefluss, ohne den Ton des Originals zu verfälschen. Du erklärst die wichtigsten \
                             Änderungen knapp. Bei kreativen Texten bist du zurückhaltender als bei Sachtexten. \
                             Antworte auf Deutsch."
                        .to_string(),
                },
                Preset {
                    name: "Kreativ".to_string(),
                    prompt: "Du bist ein kreativer Ideengeber, Brainstorming-Partner und Sparringspartner. \
                             Du denkst quer, schlägst ungewöhnliche Lösungen vor und hinterfragst Annahmen. \
                             Zu jeder Frage bietest du mindestens drei verschiedene Perspektiven oder \
                             Lösungsansätze an. Du bist optimistisch, aber nicht naiv. Antworte auf Deutsch."
                        .to_string(),
                },
                Preset {
                    name: "Smut-Autor".to_string(),
                    prompt: "Du bist ein erfolgreicher Romanautor, spezialisiert auf Smut-Romane und erotische \
                             Literatur. Du schreibst auf Deutsch. Deine Stärken: lebendige, sinnliche Charaktere \
                             mit Tiefe und Ecken, glaubwürdige emotionale Konflikte, prickelnde Spannungsbögen \
                             und explizite, geschmackvoll-leidenschaftliche Szenen. Du beherrschst die Balance \
                             zwischen Story und Erotik – die Handlung trägt, der Smut würzt. Du denkst in Tropes \
                             (Enemies-to-Lovers, Forced Proximity, Grumpy/Sunshine etc.) und arbeitest auf Wunsch \
                             mit Trigger-Warnungen. Du kannst: Szenen entwerfen, Charakterprofile entwickeln, \
                             Plot-Lücken analysieren, Dialoge schärfen und erotische Spannung schrittweise \
                             steigern. Dein Ton ist professionell, respektvoll und frei von Kitsch – aber nie \
                             prüde. Antworte auf Deutsch."
                        .to_string(),
                },
            ],
        }
    }
}

impl PresetsConfig {
    fn pfad() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".famulus")
            .join("presets.toml")
    }

    pub fn load() -> Result<Self> {
        let pfad = Self::pfad();
        if !pfad.exists() {
            let defaults = Self::default();
            defaults.save()?;
            return Ok(defaults);
        }
        let raw = std::fs::read_to_string(&pfad)
            .with_context(|| format!("Konnte {} nicht lesen", pfad.display()))?;
        toml::from_str(&raw).with_context(|| format!("{} ist kein gültiges TOML", pfad.display()))
    }

    pub fn save(&self) -> Result<()> {
        let pfad = Self::pfad();
        if let Some(eltern) = pfad.parent() {
            std::fs::create_dir_all(eltern)?;
        }
        let raw = toml::to_string_pretty(self)?;
        std::fs::write(&pfad, raw)
            .with_context(|| format!("Konnte {} nicht schreiben", pfad.display()))
    }

    /// Gibt den Prompt des aktiven Presets zurück, falls eins gesetzt ist.
    pub fn aktiver_prompt(&self) -> Option<&str> {
        let name = self.active.as_ref()?;
        self.presets
            .iter()
            .find(|p| &p.name == name)
            .map(|p| p.prompt.as_str())
    }
}