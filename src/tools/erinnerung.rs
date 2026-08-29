//! Gegenstück zu `merken()`: gezieltes Vergessen.
//!
//! Ohne dieses Werkzeug blieb jede falsch gewordene Erinnerung für immer im
//! Gedächtnis - es gab einen Weg, etwas zu lernen, aber keinen, es wieder zu
//! verlernen.

use super::Tool;
use crate::llm::ToolDefinition;
use crate::memory::Gedaechtnis;
use crate::permissions::PermissionManager;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

pub struct ErinnerungVergessenTool {
    pub gedaechtnis: Arc<Gedaechtnis>,
}

#[async_trait]
impl Tool for ErinnerungVergessenTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "erinnerung_vergessen".to_string(),
            description:
                "Löscht eine falsch gewordene Erinnerung aus dem Gedächtnis (Präferenz, Fakt \
                 oder Lektion). Suche zuerst mit einem Stichwort aus dem Inhalt: findet die \
                 Suche genau eine Erinnerung, wird sie gelöscht und der gelöschte Text \
                 zurückgemeldet; findet sie mehrere oder keine, kommt stattdessen eine \
                 Trefferliste zurück, ohne dass etwas gelöscht wird - dann gezielter suchen."
                    .to_string(),
            parameters_schema: json!({
                "type": "object",
                "properties": {
                    "stichwort": {
                        "type": "string",
                        "description": "Ein Textausschnitt, der in genau der zu löschenden \
                                        Erinnerung vorkommt."
                    }
                },
                "required": ["stichwort"]
            }),
        }
    }

    async fn execute(
        &self,
        args: Value,
        _permissions: &PermissionManager,
    ) -> anyhow::Result<String> {
        let stichwort = args["stichwort"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("'stichwort' fehlt"))?
            .trim();
        if stichwort.is_empty() {
            anyhow::bail!("'stichwort' ist leer");
        }

        let treffer = self.gedaechtnis.erinnerung_suchen(stichwort)?;
        match treffer.as_slice() {
            [] => Ok(format!("Keine Erinnerung zu '{stichwort}' gefunden.")),
            [(id, _art, inhalt)] => {
                self.gedaechtnis.vergessen(*id)?;
                Ok(format!("Vergessen: \"{inhalt}\""))
            }
            mehrere => {
                let liste = mehrere
                    .iter()
                    .map(|(id, art, inhalt)| format!("- [{id}] ({art}) {inhalt}"))
                    .collect::<Vec<_>>()
                    .join("\n");
                Ok(format!(
                    "{} Treffer zu '{stichwort}' - zu unspezifisch, nichts gelöscht. \
                     Präzisiere den Suchbegriff:\n{liste}",
                    mehrere.len()
                ))
            }
        }
    }
}
