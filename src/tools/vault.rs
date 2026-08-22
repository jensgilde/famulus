//! Der Obsidian-Vault als zweites Gehirn.
//!
//! Technisch könnte Famulus den Vault auch mit `read_file`/`write_file`
//! anfassen. Eigene Werkzeuge gibt es aus zwei Gründen:
//!
//! 1. **Das Modell benutzt, was es beschrieben bekommt.** Ein Werkzeug namens
//!    `vault_notiz` mit "hier gehört Wissen über Jens hin" wird verwendet;
//!    ein generisches `write_file` nicht.
//! 2. **Eingesperrt.** Jeder Pfad wird gegen die Vault-Wurzel geprüft. Ein
//!    Modell, das `../../.ssh/id_rsa` schickt, kommt hier nicht raus - und
//!    zwar zusätzlich zur normalen Deny-Liste, nicht statt ihr.

use super::Tool;
use crate::config::resolve_for_check;
use crate::llm::ToolDefinition;
use crate::permissions::{Decision, PermissionManager};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// Prüft, dass eine Notiz wirklich im Vault liegt, und liefert den echten
/// Pfad zurück. Nutzt dieselbe Auflösung wie die Deny-Liste, damit `..`,
/// Tilde und Symlinks genauso behandelt werden.
fn im_vault(wurzel: &Path, notiz: &str) -> anyhow::Result<PathBuf> {
    let notiz = notiz.trim();
    if notiz.is_empty() {
        anyhow::bail!("Kein Notizname angegeben.");
    }

    // Absolute Pfade und Tilde nicht stillschweigend zurechtbiegen. Aus
    // "/etc/passwd" würde sonst "Vault/etc/passwd" - drinnen zwar, aber
    // sinnloser Müll, und das Modell merkt nie, dass es sich vertan hat.
    // Eine klare Absage bringt es dazu, es richtig zu machen.
    if notiz.starts_with('/') {
        anyhow::bail!(
            "'{notiz}' ist ein absoluter Pfad. Erwartet wird ein Pfad relativ zur Vault-Wurzel, z.B. '01-Ueber-Jens/Vorlieben.md'."
        );
    }
    if Path::new(notiz)
        .components()
        .any(|c| c.as_os_str() == "~" || c.as_os_str() == "..")
    {
        anyhow::bail!(
            "'{notiz}' enthält '~' oder '..'. Erwartet wird ein einfacher Pfad relativ zur Vault-Wurzel."
        );
    }
    // Obsidian-Notizen sind Markdown; die Endung ergänzen, wenn sie fehlt.
    let mit_endung = if notiz.ends_with(".md") {
        notiz.to_string()
    } else {
        format!("{notiz}.md")
    };

    let wurzel_echt = resolve_for_check(wurzel);
    let ziel = resolve_for_check(&wurzel.join(&mit_endung));

    if !ziel.starts_with(&wurzel_echt) {
        anyhow::bail!("'{notiz}' liegt außerhalb des Vaults. Abgelehnt.");
    }
    Ok(ziel)
}

/// Gemeinsame Berechtigungsprüfung. Der Vault ist normalerweise nicht
/// gesperrt, geprüft wird trotzdem - sonst wäre `deny_paths` hier
/// wirkungslos, falls jemand den Vault mal woandershin legt.
fn erlaubt(permissions: &PermissionManager, ziel: &Path) -> anyhow::Result<()> {
    match permissions.check_path(ziel) {
        Decision::Deny => anyhow::bail!("Zugriff verweigert: '{}' ist gesperrt.", ziel.display()),
        Decision::Allow => Ok(()),
    }
}

pub struct VaultListeTool {
    pub wurzel: PathBuf,
}

#[async_trait]
impl Tool for VaultListeTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "vault_liste".to_string(),
            description:
                "Listet die Notizen im Obsidian-Vault (dem Langzeitgedächtnis über Jens). \
                 Damit anfangen, wenn du wissen willst, was bereits bekannt ist."
                    .to_string(),
            parameters_schema: json!({
                "type": "object",
                "properties": {
                    "ordner": {
                        "type": "string",
                        "description": "Optionaler Unterordner, z.B. '01-Ueber-Jens'. Leer = ganzer Vault."
                    }
                }
            }),
        }
    }

    async fn execute(
        &self,
        args: Value,
        permissions: &PermissionManager,
    ) -> anyhow::Result<String> {
        let ordner = args["ordner"].as_str().unwrap_or("").trim().to_string();
        let ziel = if ordner.is_empty() {
            resolve_for_check(&self.wurzel)
        } else {
            let z = resolve_for_check(&self.wurzel.join(&ordner));
            if !z.starts_with(resolve_for_check(&self.wurzel)) {
                anyhow::bail!("'{ordner}' liegt außerhalb des Vaults.");
            }
            z
        };

        erlaubt(permissions, &ziel)?;

        let mut notizen = Vec::new();
        sammle(&ziel, &ziel, &mut notizen)?;
        notizen.sort();

        if notizen.is_empty() {
            return Ok("Keine Notizen gefunden.".to_string());
        }
        Ok(format!(
            "{} Notizen:\n{}",
            notizen.len(),
            notizen.join("\n")
        ))
    }
}

fn sammle(wurzel: &Path, ordner: &Path, raus: &mut Vec<String>) -> anyhow::Result<()> {
    let Ok(eintraege) = std::fs::read_dir(ordner) else {
        return Ok(());
    };
    for eintrag in eintraege.flatten() {
        let pfad = eintrag.path();
        let name = eintrag.file_name();
        // Obsidians eigene Konfiguration ist für uns Rauschen.
        if name.to_string_lossy().starts_with('.') {
            continue;
        }
        if pfad.is_dir() {
            sammle(wurzel, &pfad, raus)?;
        } else if pfad.extension().is_some_and(|e| e == "md") {
            if let Ok(relativ) = pfad.strip_prefix(wurzel) {
                raus.push(relativ.to_string_lossy().into_owned());
            }
        }
    }
    Ok(())
}

pub struct VaultLesenTool {
    pub wurzel: PathBuf,
}

#[async_trait]
impl Tool for VaultLesenTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "vault_lesen".to_string(),
            description: "Liest eine Notiz aus dem Obsidian-Vault. Pfad relativ zur Vault-Wurzel, \
                          z.B. '01-Ueber-Jens/Vorlieben.md'."
                .to_string(),
            parameters_schema: json!({
                "type": "object",
                "properties": {
                    "notiz": { "type": "string", "description": "Pfad relativ zum Vault" }
                },
                "required": ["notiz"]
            }),
        }
    }

    async fn execute(
        &self,
        args: Value,
        permissions: &PermissionManager,
    ) -> anyhow::Result<String> {
        let notiz = args["notiz"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("'notiz' fehlt"))?;
        let ziel = im_vault(&self.wurzel, notiz)?;
        erlaubt(permissions, &ziel)?;
        Ok(tokio::fs::read_to_string(&ziel).await?)
    }
}

pub struct VaultSchreibenTool {
    pub wurzel: PathBuf,
}

#[async_trait]
impl Tool for VaultSchreibenTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "vault_notiz".to_string(),
            description:
                "Schreibt Wissen über Jens dauerhaft in den Obsidian-Vault. Hängt standardmäßig \
                 unten an, statt zu überschreiben. Regeln: keine Geheimnisse (Passwörter, Keys, \
                 Tokens), bestehende Notizen ergänzen statt Dubletten anlegen, Wikilinks [[so]] \
                 verwenden, bei Unsicherheit nach '00-Inbox/'."
                    .to_string(),
            parameters_schema: json!({
                "type": "object",
                "properties": {
                    "notiz":  { "type": "string", "description": "Pfad relativ zum Vault, z.B. '02-Projekte/Famulus.md'" },
                    "inhalt": { "type": "string", "description": "Der Text, der geschrieben wird (Markdown)" },
                    "ersetzen": {
                        "type": "boolean",
                        "description": "true überschreibt die ganze Notiz. Standard false = anhängen."
                    }
                },
                "required": ["notiz", "inhalt"]
            }),
        }
    }

    async fn execute(
        &self,
        args: Value,
        permissions: &PermissionManager,
    ) -> anyhow::Result<String> {
        let notiz = args["notiz"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("'notiz' fehlt"))?;
        let inhalt = args["inhalt"].as_str().unwrap_or_default();
        let ersetzen = args["ersetzen"].as_bool().unwrap_or(false);

        let ziel = im_vault(&self.wurzel, notiz)?;
        erlaubt(permissions, &ziel)?;

        if let Some(ordner) = ziel.parent() {
            tokio::fs::create_dir_all(ordner).await.ok();
        }

        if ersetzen {
            tokio::fs::write(&ziel, inhalt).await?;
            Ok(format!("Notiz '{notiz}' ersetzt."))
        } else {
            use tokio::io::AsyncWriteExt;
            let vorhanden = ziel.exists();
            let mut datei = tokio::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&ziel)
                .await?;
            // Beim Anhängen eine Leerzeile davor, damit Markdown nicht
            // zusammenklebt.
            let text = if vorhanden {
                format!("\n{inhalt}\n")
            } else {
                format!("{inhalt}\n")
            };
            datei.write_all(text.as_bytes()).await?;
            Ok(format!(
                "In '{notiz}' {} ({} Zeichen).",
                if vorhanden {
                    "ergänzt"
                } else {
                    "neu angelegt"
                },
                inhalt.len()
            ))
        }
    }
}

/// Volltextsuche über den Vault-Index (Stufe 1 des Gedächtnis-Ausbaus).
///
/// Schneller Weg zu einer Stelle im Vault, wenn man ein Stichwort hat -
/// die Alternative wäre, sich mit `vault_liste` + `vault_lesen` durch
/// potenziell viele Dateien zu tasten. Braucht den Gedächtnis-Index
/// (`Gedaechtnis::vault_index_aktualisieren`, läuft beim Start), nicht das
/// Dateisystem direkt - deshalb kein Pfad-Check wie bei den anderen
/// Vault-Werkzeugen: es werden nur bereits indizierte Schnipsel
/// zurückgegeben, nichts wird geöffnet oder geschrieben.
pub struct VaultSucheTool {
    pub gedaechtnis: std::sync::Arc<crate::memory::Gedaechtnis>,
}

#[async_trait]
impl Tool for VaultSucheTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "vault_suche".to_string(),
            description:
                "Durchsucht den Obsidian-Vault per Volltextsuche nach einem Stichwort - schneller \
                 als vault_liste + vault_lesen, wenn du weißt, wonach du suchst. Gibt pro Treffer \
                 den Dateipfad und einen kurzen Textausschnitt mit dem Fundort zurück."
                    .to_string(),
            parameters_schema: json!({
                "type": "object",
                "properties": {
                    "begriff": {
                        "type": "string",
                        "description": "Suchbegriff oder kurze Phrase."
                    }
                },
                "required": ["begriff"]
            }),
        }
    }

    async fn execute(
        &self,
        args: Value,
        _permissions: &PermissionManager,
    ) -> anyhow::Result<String> {
        let begriff = args["begriff"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("'begriff' fehlt"))?;

        let treffer = self.gedaechtnis.vault_suche(begriff, 10)?;
        if treffer.is_empty() {
            return Ok("Keine Treffer im Vault.".to_string());
        }
        Ok(treffer
            .iter()
            .map(|(pfad, schnipsel)| format!("- {pfad}: {schnipsel}"))
            .collect::<Vec<_>>()
            .join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Die wichtigste Eigenschaft dieser Werkzeuge: man kommt aus dem Vault
    /// nicht raus, egal wie der Pfad aussieht.
    #[test]
    fn ausbruch_aus_dem_vault_wird_abgelehnt() {
        let wurzel = std::env::temp_dir().join("famulus_vault_test");
        std::fs::create_dir_all(&wurzel).unwrap();

        for boesartig in [
            "../../.ssh/id_rsa",
            "../trading/strategie",
            "~/trading/strategie",
            "/etc/passwd",
        ] {
            assert!(
                im_vault(&wurzel, boesartig).is_err(),
                "'{boesartig}' hätte abgelehnt werden müssen"
            );
        }

        // Gegenprobe: normale Notizen gehen durch.
        assert!(im_vault(&wurzel, "01-Ueber-Jens/Vorlieben.md").is_ok());
        assert!(im_vault(&wurzel, "00-Inbox/Neu").is_ok());

        std::fs::remove_dir_all(&wurzel).ok();
    }

    #[test]
    fn endung_wird_ergaenzt() {
        let wurzel = std::env::temp_dir().join("famulus_vault_test2");
        std::fs::create_dir_all(&wurzel).unwrap();
        let p = im_vault(&wurzel, "00-Inbox/Notiz").unwrap();
        assert!(p.to_string_lossy().ends_with("Notiz.md"));
        std::fs::remove_dir_all(&wurzel).ok();
    }
}
