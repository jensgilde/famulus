use super::Tool;
use crate::llm::ToolDefinition;
use crate::permissions::PermissionManager;
use async_trait::async_trait;
use serde_json::{json, Value};

/// Erkennt einen Force-Push, auch wenn `-f`/`--force` nicht direkt hinter
/// `push` steht.
///
/// Die frühere Prüfung war ein reiner Substring-Test auf `"push --force"`
/// bzw. `"push -f"` und griff deshalb nicht bei `git push origin -f`,
/// `git push -f origin main` oder mehrfachen Leerzeichen - alles Formen, die
/// ein Modell (oder ein Mensch) ganz natürlich schreibt. Jetzt wird pro
/// Befehls-Segment (getrennt durch `;`, `&`, `|`, Zeilenumbruch) auf Token-
/// Ebene geprüft: enthält das Segment `push` UND irgendwo darin ein
/// Force-Flag, zählt das als Force-Push - unabhängig von der Reihenfolge.
fn ist_force_push(command: &str) -> bool {
    for segment in command.split(['\n', ';', '&', '|']) {
        let tokens: Vec<&str> = segment.split_whitespace().collect();
        if !tokens.iter().any(|t| *t == "push") {
            continue;
        }
        let hat_force_flag = tokens.iter().any(|t| {
            *t == "-f"
                || t.starts_with("--force")
                // Kombinierte Kurzflags wie "-uf".
                || (t.starts_with('-') && !t.starts_with("--") && t.len() > 1 && t[1..].contains('f'))
        });
        if hat_force_flag {
            return true;
        }
    }
    false
}

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

        // Force-Push erkennen: IMMER Rückfrage, bevor der Befehl ausgeführt wird.
        if ist_force_push(command) {
            return Ok("RÜCKFRAGE ERFORDERLICH: Dieser Befehl enthält einen Force-Push. Frage den Nutzer vor der Ausführung um Erlaubnis. Führe den Befehl erst aus, wenn der Nutzer ausdrücklich zustimmt.".to_string());
        }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn erkennt_klassisches_force_push() {
        assert!(ist_force_push("git push --force"));
        assert!(ist_force_push("git push -f"));
        assert!(ist_force_push("git push --force-with-lease"));
    }

    /// Der Kernfall: Flag und `push` in anderer Reihenfolge oder mit
    /// zusätzlichen Argumenten dazwischen. Die alte Substring-Prüfung auf
    /// `"push -f"` bzw. `"push --force"` ließ genau das durch.
    #[test]
    fn erkennt_force_push_mit_zwischenliegenden_argumenten() {
        assert!(ist_force_push("git push origin -f"));
        assert!(ist_force_push("git push -f origin main"));
        assert!(ist_force_push("git push --force origin main"));
        assert!(ist_force_push("git  push   -f")); // mehrfache Leerzeichen
    }

    #[test]
    fn erkennt_kombinierte_kurzflags() {
        assert!(ist_force_push("git push -uf origin main"));
    }

    #[test]
    fn normaler_push_bleibt_erlaubt() {
        assert!(!ist_force_push("git push"));
        assert!(!ist_force_push("git push origin main"));
    }

    #[test]
    fn force_flag_ohne_push_loest_nichts_aus() {
        assert!(!ist_force_push("rm -f irgendwas.txt"));
    }

    #[test]
    fn force_flag_in_anderem_befehls_segment_loest_nichts_aus() {
        assert!(!ist_force_push("rm -f foo.txt && git push origin main"));
    }
}
