//! In-Task-Notizbuch: der Agent kann sich während der Arbeit selbst
//! Notizen machen und diese nach dem Auftrag konsolidieren.
//!
//! Das ist Stufe 3 des Gedächtnis-Ausbaus: Zwischen "gar nichts merken" und
//! "Rückblick nach dem Auftrag" liegt das Arbeitsgedächtnis. Der Agent
//! schreibt wichtige Erkenntnisse sofort ins Notizbuch, der Rückblick
//! konsolidiert sie am Ende in dauerhafte Erinnerungen.

use super::Tool;
use crate::llm::ToolDefinition;
use crate::memory::Gedaechtnis;
use crate::permissions::PermissionManager;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

pub struct NotizbuchTool {
    pub gedaechtnis: Arc<Gedaechtnis>,
}

#[async_trait]
impl Tool for NotizbuchTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "notizbuch".to_string(),
            description:
                "Schreibt eine Erkenntnis, die du während der Arbeit gewinnst, ins Notizbuch. \
                 Nutze das für alles, was du dir merken willst, bevor der Auftrag zu Ende ist – \
                 Fakten, Präferenzen, Lektionen. Der Inhalt wird nach dem Auftrag automatisch \
                 ins Langzeitgedächtnis übernommen. Kurz und präzise formulieren."
                    .to_string(),
            parameters_schema: json!({
                "type": "object",
                "properties": {
                    "inhalt": {
                        "type": "string",
                        "description": "Die Erkenntnis, die du festhalten willst. Ein Satz, prägnant."
                    }
                },
                "required": ["inhalt"]
            }),
        }
    }

    async fn execute(
        &self,
        args: Value,
        _permissions: &PermissionManager,
    ) -> anyhow::Result<String> {
        let inhalt = args["inhalt"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("'inhalt' fehlt"))?
            .trim();
        // `notizbuch_schreiben` speichert bei leerem/nur-Leerzeichen-Inhalt
        // still gar nichts (siehe memory.rs) - ohne diesen Check hätte das
        // Tool trotzdem "Notiz gespeichert" gemeldet, obwohl nichts im
        // Notizbuch landete.
        if inhalt.is_empty() {
            anyhow::bail!("'inhalt' ist leer");
        }

        self.gedaechtnis.notizbuch_schreiben(inhalt)?;
        Ok(format!("Notiz gespeichert: {inhalt}"))
    }
}