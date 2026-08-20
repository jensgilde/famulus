//! Was Famulus anfassen darf.
//!
//! **Stand der Dinge:** Famulus läuft mit vollem Rechnerzugriff. Es gibt
//! keine Rückfrage vor einer Aktion und keine eingebaute Sperrliste - der
//! einzige Hebel ist `deny_paths` in der `famulus.toml`. Steht dort nichts,
//! darf er überall lesen und schreiben und beliebige Shell-Befehle ausführen.
//! Das ist so gewollt; es steht hier, damit niemand beim Lesen des Codes an
//! einen Schutz glaubt, den es nicht gibt.
//!
//! Was `deny_paths` leistet, wenn es gefüllt ist: Der Vergleich passiert auf
//! aufgelösten Pfaden (siehe `config::resolve_for_check`), greift also auch
//! bei `..`, bei Tilde und bei Symlinks - und auch dann, wenn die Datei noch
//! gar nicht existiert.

use crate::config::{expand_home, resolve_for_check, Config};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Decision {
    Allow,
    Deny,
}

pub struct PermissionManager {
    /// Fertig aufgelöst, einmal beim Start. Vorher lief für jeden einzelnen
    /// Werkzeug-Aufruf `canonicalize()` über jeden Eintrag der Liste neu -
    /// Dateisystem-Zugriffe für eine Antwort, die sich nie ändert.
    gesperrt: Vec<PathBuf>,
}

impl PermissionManager {
    pub fn new(config: &Config) -> Self {
        Self {
            gesperrt: config
                .deny_paths
                .iter()
                .map(|p| resolve_for_check(&expand_home(p)))
                .collect(),
        }
    }

    /// Prüft, ob ein Dateipfad angefasst werden darf.
    pub fn check_path(&self, path: &Path) -> Decision {
        let candidate = resolve_for_check(path);

        if self.gesperrt.iter().any(|d| candidate.starts_with(d)) {
            return Decision::Deny;
        }

        // Alles, was nicht ausdrücklich gesperrt ist, ist erlaubt.
        Decision::Allow
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    fn config_mit(deny: Vec<String>) -> Config {
        Config {
            provider: "hyper".to_string(),
            model: None,
            base_url: None,
            api_key_env: None,
            max_turns: 20,
            max_antwort_tokens: 16_000,
            timeout_sekunden: 300,
            vault: None,
            max_erinnerungen: 12,
            reflexion: false,
            deny_paths: deny,
        }
    }

    /// Legt eine Spielwiese an: ein gesperrtes Verzeichnis, ein erlaubtes und
    /// einen Symlink, der ins gesperrte zeigt.
    struct Sandbox {
        root: PathBuf,
    }

    impl Sandbox {
        fn new() -> Self {
            let id = COUNTER.fetch_add(1, Ordering::SeqCst);
            let root = std::env::temp_dir().join(format!(
                "famulus_perm_test_{}_{}",
                std::process::id(),
                id
            ));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(root.join("gesperrt")).unwrap();
            std::fs::create_dir_all(root.join("projekt")).unwrap();
            std::fs::write(root.join("gesperrt/vorhanden.txt"), b"geheim").unwrap();
            std::os::unix::fs::symlink(root.join("gesperrt"), root.join("link")).unwrap();
            Self { root }
        }

        fn manager(&self) -> PermissionManager {
            PermissionManager::new(&config_mit(vec![self
                .root
                .join("gesperrt")
                .to_string_lossy()
                .into_owned()]))
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn absoluter_pfad_auf_vorhandene_datei_wird_geblockt() {
        let sb = Sandbox::new();
        let p = sb.root.join("gesperrt/vorhanden.txt");
        assert_eq!(sb.manager().check_path(&p), Decision::Deny);
    }

    #[test]
    fn absoluter_pfad_auf_neue_datei_wird_geblockt() {
        let sb = Sandbox::new();
        let p = sb.root.join("gesperrt/neu.txt");
        assert_eq!(sb.manager().check_path(&p), Decision::Deny);
    }

    /// Der Kernfall: aus einem erlaubten Verzeichnis per `..` in ein
    /// gesperrtes springen. Vor dem Fix landete das auf Allow, wurde also
    /// ohne jede Rückfrage geschrieben.
    #[test]
    fn traversal_aus_erlaubtem_verzeichnis_wird_geblockt() {
        let sb = Sandbox::new();
        let p = sb.root.join("projekt/../gesperrt/neu.txt");
        assert_eq!(sb.manager().check_path(&p), Decision::Deny);
    }

    /// Symlink, der ins gesperrte Verzeichnis zeigt, mit noch nicht
    /// existierender Zieldatei.
    #[test]
    fn symlink_auf_gesperrtes_verzeichnis_wird_geblockt() {
        let sb = Sandbox::new();
        let p = sb.root.join("link/neu.txt");
        assert_eq!(sb.manager().check_path(&p), Decision::Deny);
    }

    /// Relative Pfade müssen gegen das Arbeitsverzeichnis aufgelöst werden,
    /// sonst greift der Präfix-Vergleich gar nicht erst.
    #[test]
    fn relativer_pfad_auf_neue_datei_wird_geblockt() {
        let sb = Sandbox::new();
        let vorher = std::env::current_dir().unwrap();
        std::env::set_current_dir(&sb.root).unwrap();
        let entscheidung = sb.manager().check_path(Path::new("gesperrt/neu.txt"));
        std::env::set_current_dir(vorher).unwrap();
        assert_eq!(entscheidung, Decision::Deny);
    }

    /// Gegenprobe: das erlaubte Verzeichnis muss erlaubt bleiben, sonst
    /// hätten wir die Sperre nur durch Lähmung erkauft.
    #[test]
    fn erlaubtes_verzeichnis_bleibt_erlaubt() {
        let sb = Sandbox::new();
        let p = sb.root.join("projekt/notizen.txt");
        assert_eq!(sb.manager().check_path(&p), Decision::Allow);
    }

    /// Ohne Eintrag in deny_paths ist alles erlaubt - das ist der Normalfall
    /// bei Jens.
    #[test]
    fn ohne_sperrliste_ist_alles_erlaubt() {
        let manager = PermissionManager::new(&config_mit(vec![]));
        assert_eq!(manager.check_path(Path::new("/etc/passwd")), Decision::Allow);
    }

    /// Modelle schicken Pfade oft mit Tilde. Die muss aufgelöst werden,
    /// sonst greift die Sperrliste nicht und man bekommt statt einer klaren
    /// Verweigerung nur ein irreführendes "file not found".
    #[test]
    fn tilde_pfad_in_gesperrtem_verzeichnis_wird_geblockt() {
        let manager = PermissionManager::new(&config_mit(vec!["~/trading".to_string()]));
        // Reine Pfad-Arithmetik - hier wird nichts gelesen oder angelegt.
        assert_eq!(
            manager.check_path(Path::new("~/trading/gibtsnicht.txt")),
            Decision::Deny
        );
    }

    /// Auch die Kombination aus Tilde und `..` muss halten.
    #[test]
    fn tilde_mit_traversal_wird_geblockt() {
        let manager = PermissionManager::new(&config_mit(vec!["~/trading".to_string()]));
        assert_eq!(
            manager.check_path(Path::new("~/famulus/../trading/gibtsnicht.txt")),
            Decision::Deny
        );
    }

    /// Ein Verzeichnis, das nur zufällig mit demselben Text anfängt, wird
    /// nicht mitgesperrt und ist erlaubt.
    #[test]
    fn namensaehnliches_verzeichnis_wird_erlaubt() {
        let sb = Sandbox::new();
        std::fs::create_dir_all(sb.root.join("gesperrt_backup")).unwrap();
        let p = sb.root.join("gesperrt_backup/datei.txt");
        assert_eq!(sb.manager().check_path(&p), Decision::Allow);
    }
}
