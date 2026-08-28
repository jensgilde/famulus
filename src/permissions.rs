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
    /// Dürfte nur mit Rückfrage – das Modell MUSS vorher bestätigen.
    Ask,
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

    /// Prüft, ob ein Pfad eine Rückfrage erfordert (Force-Push, Secret-Pfade).
    /// Diese Check geschieht zusätzlich zu check_path – auch erlaubte Pfade
    /// können eine Ask-Entscheidung auslösen.
    ///
    /// Verankert an `$HOME`, nicht als roher Substring-Test: eine reine
    /// `contains("/.ssh/")`-Prüfung übersah den Ordner `~/.ssh` selbst (kein
    /// `/` danach, wenn kein Dateiname folgt) und griff andererseits auch bei
    /// harmlosen Pfaden wie `/tmp/projekt/.ssh/config`, die nichts mit den
    /// echten Zugangsdaten zu tun haben. `Path::starts_with` vergleicht
    /// Pfad-Komponenten statt Zeichenketten - `~/.ssh-backup` triggert damit
    /// auch nicht mehr fälschlich als Präfix-Treffer auf `~/.ssh`.
    pub fn check_ask(&self, path: &Path) -> Decision {
        let candidate = resolve_for_check(path);
        const SENSIBLE_ORDNER: &[&str] = &[".ssh", ".gnupg", ".aws", ".password-store"];
        if let Some(home) = dirs::home_dir().map(|h| resolve_for_check(&h)) {
            for name in SENSIBLE_ORDNER {
                let sensibel = home.join(name);
                if candidate == sensibel || candidate.starts_with(&sensibel) {
                    return Decision::Ask;
                }
            }
        }
        Decision::Allow
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
            fallback_providers: Vec::new(),
            max_turns: 20,
            max_antwort_tokens: 16_000,
            timeout_sekunden: 300,
            vault: None,
            max_erinnerungen: 12,
            reflexion: false,
            deny_paths: deny,
            modell_modus: "manuell".to_string(),
            guenstiges_modell: None,
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

    // ── check_ask: sensible Pfade unter $HOME ──────────────────────────

    #[test]
    fn datei_in_ssh_ordner_erfordert_rueckfrage() {
        let manager = PermissionManager::new(&config_mit(vec![]));
        assert_eq!(
            manager.check_ask(Path::new("~/.ssh/id_ed25519")),
            Decision::Ask
        );
    }

    /// Der Ordner selbst, ohne Datei dahinter - die alte Substring-Prüfung
    /// (`contains("/.ssh/")`) übersah genau diesen Fall, weil kein `/` mehr
    /// folgt.
    #[test]
    fn ssh_ordner_selbst_erfordert_rueckfrage() {
        let manager = PermissionManager::new(&config_mit(vec![]));
        assert_eq!(manager.check_ask(Path::new("~/.ssh")), Decision::Ask);
    }

    /// Ein Ordner, der nur zufällig mit demselben Namen beginnt, darf nicht
    /// mitgetriggert werden - genau das Gegenstück zu
    /// `namensaehnliches_verzeichnis_wird_erlaubt` oben, nur für check_ask.
    #[test]
    fn aehnlich_benannter_ordner_loest_keine_rueckfrage_aus() {
        let manager = PermissionManager::new(&config_mit(vec![]));
        assert_eq!(
            manager.check_ask(Path::new("~/.ssh-backup/irgendwas.txt")),
            Decision::Allow
        );
    }

    /// Ein `.ssh`-Unterordner außerhalb von `$HOME` (z.B. in einem
    /// Projektverzeichnis) hat nichts mit den echten Zugangsdaten zu tun -
    /// die alte reine Substring-Prüfung hätte hier fälschlich Ask ausgelöst.
    #[test]
    fn ssh_ordner_ausserhalb_von_home_loest_keine_rueckfrage_aus() {
        let manager = PermissionManager::new(&config_mit(vec![]));
        assert_eq!(
            manager.check_ask(Path::new("/tmp/projekt/.ssh/config")),
            Decision::Allow
        );
    }

    #[test]
    fn normale_datei_loest_keine_rueckfrage_aus() {
        let manager = PermissionManager::new(&config_mit(vec![]));
        assert_eq!(
            manager.check_ask(Path::new("~/Dokumente/notizen.txt")),
            Decision::Allow
        );
    }
}
