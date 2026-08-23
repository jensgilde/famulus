use super::Tool;
use crate::config::expand_home;
use crate::llm::ToolDefinition;
use crate::permissions::{Decision, PermissionManager};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::Path;
use tokio::io::AsyncReadExt;

/// Höchstens so viele Bytes werden aus einer Datei gelesen.
///
/// Der Deckel schützt den Arbeitsspeicher, nicht den Kontext: `read_to_string`
/// auf eine 2-GB-Datei zieht die komplett in den RAM, bevor irgendjemand
/// kürzen könnte. Was davon am Ende ins Modell wandert, kürzt die
/// Agent-Schleife nochmal deutlich schärfer.
const MAX_DATEI_BYTES: u64 = 4 * 1024 * 1024;

/// Gemeinsame Prüfung für beide Datei-Werkzeuge.
fn pruefen(permissions: &PermissionManager, path: &Path, path_str: &str) -> anyhow::Result<()> {
    match permissions.check_path(path) {
        Decision::Deny => {
            anyhow::bail!("Zugriff verweigert: '{path_str}' liegt in einem gesperrten Verzeichnis.")
        }
        Decision::Ask => {
            anyhow::bail!("RÜCKFRAGE ERFORDERLICH: Der Pfad '{path_str}' betrifft sensible Daten. Frage den Nutzer vor dem Zugriff um Erlaubnis.")
        }
        Decision::Allow => {
            // Zusätzliche Ask-Prüfung für sensible Pfade.
            match permissions.check_ask(path) {
                Decision::Ask => {
                    anyhow::bail!("RÜCKFRAGE ERFORDERLICH: Der Pfad '{path_str}' betrifft sensible Daten (~/.ssh, ~/.gnupg, ~/.aws, ~/.password-store). Frage den Nutzer vor dem Zugriff um Erlaubnis.")
                }
                _ => Ok(()),
            }
        }
    }
}

pub struct ReadFileTool;

#[async_trait]
impl Tool for ReadFileTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "read_file".to_string(),
            description: "Liest den Inhalt einer Textdatei von der Festplatte.".to_string(),
            parameters_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Absoluter oder relativer Pfad zur Datei" }
                },
                "required": ["path"]
            }),
        }
    }

    async fn execute(
        &self,
        args: Value,
        permissions: &PermissionManager,
    ) -> anyhow::Result<String> {
        let path_str = args["path"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("'path' fehlt"))?;
        // Tilde hier EINMAL auflösen und danach überall denselben Pfad
        // benutzen - geprüft werden muss exakt das, was auch geöffnet wird.
        let path = expand_home(path_str);
        pruefen(permissions, &path, path_str)?;

        let datei = tokio::fs::File::open(&path).await?;
        let mut puffer = Vec::new();
        // Ein Byte mehr als der Deckel lesen: nur so lässt sich unterscheiden,
        // ob die Datei genau MAX_DATEI_BYTES groß ist (nichts fehlt) oder
        // größer (wirklich abgeschnitten). Mit `take(MAX_DATEI_BYTES)` allein
        // war `puffer.len() == MAX_DATEI_BYTES` bei einer exakt passenden
        // Datei fälschlich als "abgeschnitten" markiert, obwohl der komplette
        // Inhalt gelesen wurde.
        datei
            .take(MAX_DATEI_BYTES + 1)
            .read_to_end(&mut puffer)
            .await?;

        let abgeschnitten = puffer.len() as u64 > MAX_DATEI_BYTES;
        if abgeschnitten {
            puffer.truncate(MAX_DATEI_BYTES as usize);
        }
        let mut inhalt = String::from_utf8_lossy(&puffer).into_owned();
        if abgeschnitten {
            inhalt.push_str("\n\n[... Datei ist größer als 4 MB, hier abgeschnitten ...]");
        }
        Ok(inhalt)
    }
}

pub struct WriteFileTool;

#[async_trait]
impl Tool for WriteFileTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "write_file".to_string(),
            description: "Schreibt Text in eine Datei (überschreibt vorhandenen Inhalt)."
                .to_string(),
            parameters_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "content": { "type": "string" }
                },
                "required": ["path", "content"]
            }),
        }
    }

    async fn execute(
        &self,
        args: Value,
        permissions: &PermissionManager,
    ) -> anyhow::Result<String> {
        let path_str = args["path"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("'path' fehlt"))?;
        let content = args["content"].as_str().unwrap_or_default();
        let path = expand_home(path_str);
        pruefen(permissions, &path, path_str)?;

        if let Some(eltern) = path.parent() {
            tokio::fs::create_dir_all(eltern).await?;
        }
        tokio::fs::write(&path, content).await?;
        Ok(format!(
            "Geschrieben: {path_str} ({} Zeichen)",
            content.len()
        ))
    }
}
