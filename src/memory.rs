//! Das Gedächtnis: was Famulus über Aufträge hinweg behält.
//!
//! Zwei getrennte Dinge, die man leicht verwechselt:
//!
//! - **Hier (SQLite):** kurze, harte Fakten und Lektionen. Klein, schnell
//!   abfragbar, wird bei *jedem* Auftrag in den Prompt gelegt. Das ist das
//!   Arbeitsgedächtnis.
//! - **Der Obsidian-Vault (`tools/vault.rs`):** ausführliches Wissen in
//!   Prosa - Biografie, Projekte, Entscheidungen. Wird gelesen, wenn es
//!   gebraucht wird, nicht bei jedem Auftrag.
//!
//! Die Trennung ist Absicht: Käme alles in jeden Prompt, wäre der Kontext
//! nach zwei Wochen voll und teuer. Die Datenbank hält das Kurze, der Vault
//! das Lange.

use anyhow::{Context, Result};
use rusqlite::Connection;
use std::path::Path;
use std::sync::Mutex;

/// Ein gemerkter Satz.
#[derive(Debug, Clone)]
pub struct Erinnerung {
    pub art: String,
    pub inhalt: String,
}

/// Womit man es zu tun hat. Bewusst wenige Kategorien - je mehr Schubladen,
/// desto öfter landet etwas in der falschen.
pub const ART_PRAEFERENZ: &str = "praeferenz"; // wie Jens Dinge haben will
pub const ART_FAKT: &str = "fakt"; // wie das System/Projekt beschaffen ist
pub const ART_LEKTION: &str = "lektion"; // was beim Arbeiten schiefging

pub struct Gedaechtnis {
    // rusqlite-Verbindungen sind nicht `Sync`. Der Agent wird aber zwischen
    // Tasks geteilt, also einmal sauber einsperren statt die Verbindung
    // durchzureichen.
    verbindung: Mutex<Connection>,
}

impl Gedaechtnis {
    /// Der Standardort: `~/KI Agenten/famulus/gedaechtnis.db`, direkt im
    /// Projektordner statt versteckt unter `~/.famulus` - auf Jens' Wunsch
    /// liegt das Gedächtnis (wie der Vault) sichtbar bei Famulus selbst.
    pub fn standard() -> Result<Self> {
        let ordner = dirs::home_dir()
            .context("Kein Home-Verzeichnis gefunden")?
            .join("KI Agenten")
            .join("famulus");
        Self::oeffnen(&ordner.join("gedaechtnis.db"))
    }

    pub fn oeffnen(pfad: &Path) -> Result<Self> {
        if let Some(ordner) = pfad.parent() {
            std::fs::create_dir_all(ordner).ok();
        }
        let verbindung = Connection::open(pfad)
            .with_context(|| format!("Gedächtnis-Datenbank {} nicht zu öffnen", pfad.display()))?;

        verbindung.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS erinnerungen (
                id       INTEGER PRIMARY KEY,
                art      TEXT NOT NULL,
                inhalt   TEXT NOT NULL UNIQUE,
                quelle   TEXT,
                erstellt TEXT NOT NULL DEFAULT (datetime('now','localtime'))
            );
            CREATE TABLE IF NOT EXISTS auftraege (
                id        INTEGER PRIMARY KEY,
                auftrag   TEXT NOT NULL,
                ergebnis  TEXT,
                erfolg    INTEGER NOT NULL,
                zeitpunkt TEXT NOT NULL DEFAULT (datetime('now','localtime'))
            );
            ",
        )?;

        Ok(Self {
            verbindung: Mutex::new(verbindung),
        })
    }

    /// Merkt sich einen Satz. Gibt `false` zurück, wenn er schon bekannt war -
    /// `UNIQUE` auf dem Inhalt verhindert, dass dieselbe Erkenntnis nach
    /// zwanzig Aufträgen zwanzigmal im Prompt steht.
    pub fn merken(&self, art: &str, inhalt: &str, quelle: &str) -> Result<bool> {
        let inhalt = inhalt.trim();
        if inhalt.is_empty() {
            return Ok(false);
        }
        let geaendert = self
            .verbindung
            .lock()
            .expect("Gedächtnis vergiftet")
            .execute(
                "INSERT OR IGNORE INTO erinnerungen (art, inhalt, quelle) VALUES (?1, ?2, ?3)",
                (art, inhalt, quelle),
            )?;
        Ok(geaendert > 0)
    }

    /// Die Erinnerungen, die zu diesem Auftrag am ehesten passen.
    ///
    /// Bewusst simpel: Wortüberschneidung zwischen Auftrag und Erinnerung,
    /// bei Gleichstand das Neuere zuerst. Kein Vektor-Index, keine
    /// Ähnlichkeitssuche - bei ein paar hundert kurzen Sätzen wäre das
    /// Maschinerie ohne Gegenwert. Wenn die Datenbank mal groß wird, ist
    /// genau diese Funktion die Stelle, die man austauscht.
    pub fn relevante(&self, auftrag: &str, hoechstens: usize) -> Result<Vec<Erinnerung>> {
        let verbindung = self.verbindung.lock().expect("Gedächtnis vergiftet");
        let mut abfrage =
            verbindung.prepare("SELECT art, inhalt FROM erinnerungen ORDER BY id DESC")?;
        let alle: Vec<Erinnerung> = abfrage
            .query_map([], |zeile| {
                Ok(Erinnerung {
                    art: zeile.get(0)?,
                    inhalt: zeile.get(1)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        // Als Menge statt als Liste: das Nachschlagen unten läuft sonst
        // für jedes Wort einmal durch den ganzen Auftrag.
        let stichworte: std::collections::HashSet<String> = worte(auftrag).into_iter().collect();
        let mut bewertet: Vec<(usize, Erinnerung)> = alle
            .into_iter()
            .map(|e| {
                let treffer = worte(&e.inhalt)
                    .iter()
                    .filter(|w| stichworte.contains(*w))
                    .count();
                // Präferenzen sind fast immer relevant, auch ohne Worttreffer -
                // "antworte auf Deutsch" gilt bei jedem Auftrag.
                let bonus = if e.art == ART_PRAEFERENZ { 5 } else { 0 };
                (treffer * 10 + bonus, e)
            })
            .collect();

        // Absteigend, aber stabil: Bei Punktgleichstand bleibt die
        // Reihenfolge aus der Abfrage (id DESC) erhalten, neuer schlägt also
        // älter von allein. Hier stand mal ein eigener Neuheits-Bonus - der
        // war durch Ganzzahl-Division für alles außer dem allerneuesten
        // Eintrag immer 0 und damit wirkungslos.
        bewertet.sort_by_key(|(punkte, _)| std::cmp::Reverse(*punkte));
        Ok(bewertet
            .into_iter()
            .take(hoechstens)
            .map(|(_, e)| e)
            .collect())
    }

    pub fn auftrag_protokollieren(
        &self,
        auftrag: &str,
        ergebnis: &str,
        erfolg: bool,
    ) -> Result<()> {
        self.verbindung
            .lock()
            .expect("Gedächtnis vergiftet")
            .execute(
                "INSERT INTO auftraege (auftrag, ergebnis, erfolg) VALUES (?1, ?2, ?3)",
                (auftrag, ergebnis, if erfolg { 1 } else { 0 }),
            )?;
        Ok(())
    }

    pub fn anzahl(&self) -> Result<i64> {
        Ok(self
            .verbindung
            .lock()
            .expect("Gedächtnis vergiftet")
            .query_row("SELECT COUNT(*) FROM erinnerungen", [], |z| z.get(0))?)
    }
}

/// Zerlegt Text in kleingeschriebene Wörter ab vier Buchstaben. Die kurzen
/// ("und", "der", "die") tragen nichts zur Ähnlichkeit bei und würden jede
/// Erinnerung mit jedem Auftrag verbinden.
fn worte(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() >= 4)
        .map(|w| w.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        std::env::temp_dir().join(format!(
            "famulus_gedaechtnis_{}_{}.db",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ))
    }

    #[test]
    fn merkt_und_findet_wieder() {
        let pfad = temp();
        let g = Gedaechtnis::oeffnen(&pfad).unwrap();
        g.merken(
            ART_FAKT,
            "Der Obsidian-Vault liegt unter Documents/Hermes-Vault",
            "test",
        )
        .unwrap();
        let treffer = g.relevante("Wo liegt der Obsidian-Vault?", 5).unwrap();
        assert!(treffer.iter().any(|e| e.inhalt.contains("Hermes-Vault")));
        std::fs::remove_file(pfad).ok();
    }

    /// Ohne das steht dieselbe Erkenntnis nach zwanzig Aufträgen zwanzigmal
    /// im Prompt und verdrängt alles andere.
    #[test]
    fn merkt_dubletten_nicht_doppelt() {
        let pfad = temp();
        let g = Gedaechtnis::oeffnen(&pfad).unwrap();
        assert!(g
            .merken(ART_LEKTION, "Cargo braucht pkg-config", "a")
            .unwrap());
        assert!(!g
            .merken(ART_LEKTION, "Cargo braucht pkg-config", "b")
            .unwrap());
        assert_eq!(g.anzahl().unwrap(), 1);
        std::fs::remove_file(pfad).ok();
    }

    #[test]
    fn praeferenzen_kommen_auch_ohne_worttreffer_mit() {
        let pfad = temp();
        let g = Gedaechtnis::oeffnen(&pfad).unwrap();
        g.merken(ART_PRAEFERENZ, "Jens will geduzt werden", "test")
            .unwrap();
        g.merken(
            ART_FAKT,
            "Home Assistant läuft auf derselben Maschine",
            "test",
        )
        .unwrap();
        // Auftrag hat mit beidem nichts zu tun.
        let treffer = g.relevante("Kompiliere das Projekt neu", 5).unwrap();
        assert!(
            treffer.iter().any(|e| e.art == ART_PRAEFERENZ),
            "Präferenzen müssen immer mitkommen"
        );
        std::fs::remove_file(pfad).ok();
    }

    #[test]
    fn leeres_wird_nicht_gemerkt() {
        let pfad = temp();
        let g = Gedaechtnis::oeffnen(&pfad).unwrap();
        assert!(!g.merken(ART_FAKT, "   ", "test").unwrap());
        assert_eq!(g.anzahl().unwrap(), 0);
        std::fs::remove_file(pfad).ok();
    }
}
