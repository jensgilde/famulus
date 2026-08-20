use super::Tool;
use crate::llm::ToolDefinition;
use crate::permissions::PermissionManager;
use async_trait::async_trait;
use serde_json::{json, Value};

pub struct ShellTool;

#[async_trait]
impl Tool for ShellTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "run_shell".to_string(),
            description: "Führt einen Shell-Befehl aus und gibt stdout/stderr zurück.".to_string(),
            parameters_schema: json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "Der auszuführende Shell-Befehl" }
                },
                "required": ["command"]
            }),
        }
    }

    async fn execute(
        &self,
        args: Value,
        permissions: &PermissionManager,
    ) -> anyhow::Result<String> {
        let command = args["command"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("'command' fehlt"))?;

        // Kein Pfad, keine Prüfung: Shell-Befehle laufen ungefiltert.
        // `deny_paths` greift hier NICHT - wer den Zugriff auf ein
        // Verzeichnis wirklich verhindern will, kommt an run_shell nicht
        // vorbei. Steht so in permissions.rs, ist so gewollt.
        let _ = permissions;

        let output = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(command)
            .output()
            .await?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        Ok(format!(
            "exit code: {}\n--- stdout ---\n{}\n--- stderr ---\n{}",
            output.status.code().unwrap_or(-1),
            stdout,
            stderr
        ))
    }
}
