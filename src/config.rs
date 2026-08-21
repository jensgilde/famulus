use anyhow::{Context, Result};
use serde::Deserialize;
use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub provider: String, // "hyper" oder "openrouter"
    #[serde(default)]
    pub model: Option<String>,
    /// Abweichende API-Adresse. Leer lassen für den echten Anbieter; gesetzt
    /// für kompatible Dienste, die dasselbe Protokoll sprechen (z.B. Charm
    /// Hyper unter https://hyper.charm.land).
    #[serde(default)]
    pub base_url: Option<String>,
    /// Name der Umgebungsvariable, aus der der API-Key gelesen wird. Leer
    /// lassen für den Standard des jeweiligen Providers. Nützlich, damit ein
    /// Hyper-Key nicht ausgerechnet in ANTHROPIC_API_KEY landen muss.
    #[serde(default)]
    pub api_key_env: Option<String>,
    #[serde(default = "default_max_turns")]
    pub max_turns: u32,
    /// Wie lang eine einzelne Modellantwort höchstens werden darf. Zu klein
    /// heißt: lange Antworten brechen mittendrin ab, ohne dass es auffällt.
    #[serde(default = "default_antwort_tokens")]
    pub max_antwort_tokens: u32,
    /// Geduld mit dem Anbieter, in Sekunden. Ohne Deckel wartet Famulus
    /// endlos auf einen Dienst, der nie antwortet.
    #[serde(default = "default_timeout")]
    pub timeout_sekunden: u64,
    /// Verzeichnisse, die Famulus nicht anfassen darf - weder lesen noch
    /// schreiben. Leer heißt: volle Freiheit auf dem ganzen Rechner.
    #[serde(default)]
    pub deny_paths: Vec<String>,

    // ---- Gedächtnis (Phase 1) ----
    /// Obsidian-Vault als Langzeitgedächtnis. Leer = Vault-Werkzeuge werden
    /// gar nicht erst angeboten.
    #[serde(default)]
    pub vault: Option<String>,
    /// Wie viele gemerkte Sätze höchstens in den Prompt gelegt werden.
    /// Deckel gegen einen Kontext, der mit den Monaten immer teurer wird.
    #[serde(default = "default_erinnerungen")]
    pub max_erinnerungen: usize,
    /// Nach jedem Auftrag einen Rückblick machen und Erkenntnisse merken.
    #[serde(default = "default_wahr")]
    pub reflexion: bool,
}

fn default_antwort_tokens() -> u32 {
    16_000
}

fn default_timeout() -> u64 {
    300
}

fn default_erinnerungen() -> usize {
    12
}

fn default_wahr() -> bool {
    true
}

impl Config {
    /// Der Vault-Pfad, falls konfiguriert UND vorhanden.
    ///
    /// Die Existenzprüfung ist Absicht: Ein falsch geschriebener Pfad soll
    /// dazu führen, dass die Werkzeuge fehlen (auffällig), statt dass jeder
    /// Schreibversuch einzeln scheitert (verwirrend).
    pub fn vault_pfad(&self) -> Option<PathBuf> {
        let pfad = expand_home(self.vault.as_deref()?);
        pfad.is_dir().then_some(pfad)
    }
}

fn default_max_turns() -> u32 {
    101
}

impl Config {
    pub fn load() -> Result<Self> {
        // .env für API-Keys laden (liegt idealerweise NICHT im Git-Repo).
        //
        // Zuerst das aktuelle Verzeichnis - so arbeitet man beim Entwickeln.
        // Danach ~/.famulus/.env als Rückfalloption: Ein Fenster, das per
        // Doppelklick startet, hat kein sinnvolles Arbeitsverzeichnis, und
        // ohne diesen zweiten Versuch fände die GUI den Key schlicht nicht.
        // `from_path` überschreibt nichts bereits Gesetztes, die Reihenfolge
        // ist also eine echte Rangfolge.
        let _ = dotenvy::dotenv();
        if let Some(home) = dirs::home_dir() {
            let _ = dotenvy::from_path(home.join(".famulus").join(".env"));
        }

        let config_path = config_file_path();
        let raw = std::fs::read_to_string(&config_path).with_context(|| {
            format!(
                "Konnte {} nicht lesen. Kopier famulus.toml.example nach {} und pass es an.",
                config_path.display(),
                config_path.display()
            )
        })?;

        let config: Config = toml::from_str(&raw).with_context(|| {
            format!(
                "famulus.toml ({}) ist kein gültiges TOML",
                config_path.display()
            )
        })?;

        Ok(config)
    }
}

pub fn expand_home(path: &str) -> PathBuf {
    expand_tilde(Path::new(path))
}

/// Ersetzt ein führendes `~` durch das Home-Verzeichnis.
///
/// Die Tilde ist ein Shell-Zeichen, kein Betriebssystem-Zeichen: `open()`
/// kennt sie nicht, normalerweise löst die Shell sie vorher auf. Ein Modell
/// schickt aber gern mal `~/trading/x` als Tool-Argument - und dann kommt der
/// Pfad ungefiltert hier an. Ohne diese Auflösung liefe er an der Deny-Liste
/// vorbei und scheiterte erst beim Öffnen mit einem irreführenden
/// "file not found" statt mit einer klaren Verweigerung.
pub fn expand_tilde(path: &Path) -> PathBuf {
    let mut components = path.components();
    match components.next() {
        Some(Component::Normal(first)) if first == "~" => match dirs::home_dir() {
            Some(home) => home.join(components.as_path()),
            None => path.to_path_buf(),
        },
        _ => path.to_path_buf(),
    }
}

/// Bringt einen beliebigen Pfad in eine verlässliche absolute Form, die man
/// gegen die Deny-Liste vergleichen kann - auch wenn die Datei noch gar nicht
/// existiert.
///
/// `canonicalize()` allein reicht dafür NICHT: es schlägt fehl, sobald der
/// Pfad noch nicht auf der Platte liegt. Genau das ist aber der Normalfall
/// beim Schreiben neuer Dateien, also ausgerechnet der gefährlichen Richtung.
/// Deshalb in drei Schritten:
///
/// 1. relativ -> absolut (sonst greift kein Präfix-Vergleich)
/// 2. `.` und `..` selbst rausrechnen, damit `~/famulus/../trading` nicht
///    als "liegt unter ~/famulus" durchgeht
/// 3. Symlinks auflösen, indem der tiefste *existierende* Elternordner
///    canonicalisiert und der noch nicht existierende Rest wieder
///    angehängt wird
pub fn resolve_for_check(path: &Path) -> PathBuf {
    let path = expand_tilde(path);

    let absolute = if path.is_absolute() {
        path.clone()
    } else {
        match std::env::current_dir() {
            Ok(cwd) => cwd.join(&path),
            Err(_) => path.clone(),
        }
    };

    let normalized = lexical_normalize(&absolute);

    // Vom vollen Pfad aus nach oben laufen, bis ein Stück existiert, das sich
    // canonicalisieren laesst. Der abgeschnittene Rest kann keine Symlinks
    // enthalten - was nicht existiert, kann kein Link sein.
    let mut prefix = normalized.as_path();
    let mut tail: Vec<&OsStr> = Vec::new();
    loop {
        if let Ok(real) = prefix.canonicalize() {
            let mut resolved = real;
            for part in tail.iter().rev() {
                resolved.push(part);
            }
            return resolved;
        }
        match (prefix.file_name(), prefix.parent()) {
            (Some(name), Some(parent)) => {
                tail.push(name);
                prefix = parent;
            }
            // Nichts auf dem Weg existiert (oder wir sind an der Wurzel):
            // dann ist die lexikalische Form das Beste, was wir haben.
            _ => return normalized,
        }
    }
}

/// Rechnet `.` und `..` rein textuell raus. `..` an der Wurzel bleibt an der
/// Wurzel - man kann sich damit nicht aus dem Dateisystem rausschieben.
fn lexical_normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn config_file_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".famulus")
        .join("famulus.toml")
}
