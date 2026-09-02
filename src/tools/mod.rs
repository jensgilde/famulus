use crate::llm::ToolDefinition;
use crate::permissions::PermissionManager;
use async_trait::async_trait;
use serde_json::Value;

#[async_trait]
pub trait Tool: Send + Sync {
    fn definition(&self) -> ToolDefinition;

    /// Führt das Tool aus. Die Permission-Prüfung passiert INNERHALB der
    /// Implementierung, nicht davor - so kann jedes Tool selbst entscheiden,
    /// was genau geprüft werden muss (Pfad, Befehl, o.ä.), und es gibt
    /// keinen Weg, ein Tool ohne diese Prüfung aufzurufen.
    async fn execute(&self, args: Value, permissions: &PermissionManager)
        -> anyhow::Result<String>;
}

pub mod fs;
pub mod shell;
pub mod vault;
pub mod notizbuch;
pub mod selbstmodell;
pub mod frage_nutzer;
pub mod erinnerung;
pub mod web_request;

/// Stellt die Werkzeuge zusammen, die dem Modell angeboten werden.
///
/// Die Vault-Werkzeuge kommen nur dazu, wenn ein Vault konfiguriert ist. Ein
/// Werkzeug anzubieten, das dann an einem fehlenden Ordner scheitert, wäre
/// schlechter als es wegzulassen: Das Modell versucht es sonst immer wieder.
pub fn all_tools(config: &crate::config::Config) -> Vec<Box<dyn Tool>> {
    let mut werkzeuge: Vec<Box<dyn Tool>> = vec![
        Box::new(fs::ReadFileTool),
        Box::new(fs::WriteFileTool),
        Box::new(shell::ShellTool),
        // Web-Tool: HTTP-GET/POST mit Cookie-Session für externe Dienste.
        Box::new(web_request::WebRequestTool::new(
            dirs::home_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("."))
                .join(".famulus"),
        )),
    ];

    if let Some(wurzel) = config.vault_pfad() {
        werkzeuge.push(Box::new(vault::VaultListeTool {
            wurzel: wurzel.clone(),
        }));
        werkzeuge.push(Box::new(vault::VaultLesenTool {
            wurzel: wurzel.clone(),
        }));
        werkzeuge.push(Box::new(vault::VaultSchreibenTool { wurzel }));
    }

    werkzeuge
}