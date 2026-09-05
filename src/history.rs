//! Chat-Verlauf mit Volltextsuche.
//!
//! Alle Chat-Sessions landen in derselben `gedaechtnis.db` wie die
//! Erinnerungen – eine Datenbankdatei statt zwei. Die Tabelle `chats`
//! speichert die Nachrichten als JSON-Blob, damit die Struktur flexibel
//! bleibt und nicht jedes Mal migriert werden muss, wenn das Frontend
//! ein neues Feld hinzufügt.
//!
//! Die Suche läuft über FTS5 – konsistent mit der Erinnerungen-Suche.

use anyhow::{Context, Result};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Mutex;

/// Ein Chat-Eintrag, wie er in der Datenbank liegt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatEintrag {
    pub id: i64,
    pub titel: String,
    /// Die Nachrichten als JSON-String – das Frontend serialisiert/deserialisiert
    /// selbst, der Server behandelt es als opaken Blob.
    pub nachrichten: String,
    pub erstellt: String,
    pub geaendert: String,
    pub archiviert: bool,
}

pub struct History {
    verbindung: Mutex<Connection>,
}

impl History {
    /// Öffnet die History in derselben Datenbank wie das Gedächtnis.
    pub fn oeffnen(pfad: &Path) -> Result<Self> {
        if let Some(ordner) = pfad.parent() {
            std::fs::create_dir_all(ordner).ok();
        }
        let verbindung = Connection::open(pfad)
            .with_context(|| format!("History-Datenbank {} nicht zu öffnen", pfad.display()))?;

        // Dieselbe Datei wie `memory::Gedaechtnis` (`gedaechtnis.db`), und
        // `History::oeffnen` wird pro FFI-Aufruf frisch aufgerufen (siehe
        // ffi.rs::history_db) - ohne busy_timeout liefert eine zeitgleiche
        // Schreib-Connection (Agent-Task, Router-Protokollierung,
        // idle_reflexion) sofort SQLITE_BUSY statt kurz zu warten. Siehe
        // ausführliche Begründung in memory.rs::oeffnen.
        verbindung.busy_timeout(std::time::Duration::from_secs(5))?;

        verbindung.execute_batch(
            "CREATE TABLE IF NOT EXISTS chats (
                id          INTEGER PRIMARY KEY,
                titel       TEXT NOT NULL,
                nachrichten TEXT NOT NULL DEFAULT '[]',
                erstellt    TEXT NOT NULL DEFAULT (datetime('now','localtime')),
                geaendert   TEXT NOT NULL DEFAULT (datetime('now','localtime')),
                archiviert  INTEGER NOT NULL DEFAULT 0
            );",
        )?;

        // FTS5-Index für Chat-Suche – konsistent mit der Erinnerungen-Suche.
        // content='chats' liest die Spaltenwerte aus der chats-Tabelle,
        // hält den Suchindex selbst aber NICHT automatisch synchron -
        // dafür braucht es Trigger. Ohne sie bleibt der Index leer und
        // `suche()` findet nie etwas, egal wie viele Chats gespeichert sind.
        verbindung.execute_batch(
            "CREATE VIRTUAL TABLE IF NOT EXISTS chats_fts USING fts5(
                titel,
                nachrichten,
                content='chats',
                content_rowid='id'
            );
            CREATE TRIGGER IF NOT EXISTS chats_fts_ai AFTER INSERT ON chats BEGIN
                INSERT INTO chats_fts(rowid, titel, nachrichten) VALUES (new.id, new.titel, new.nachrichten);
            END;
            CREATE TRIGGER IF NOT EXISTS chats_fts_ad AFTER DELETE ON chats BEGIN
                INSERT INTO chats_fts(chats_fts, rowid, titel, nachrichten) VALUES ('delete', old.id, old.titel, old.nachrichten);
            END;
            CREATE TRIGGER IF NOT EXISTS chats_fts_au AFTER UPDATE ON chats BEGIN
                INSERT INTO chats_fts(chats_fts, rowid, titel, nachrichten) VALUES ('delete', old.id, old.titel, old.nachrichten);
                INSERT INTO chats_fts(rowid, titel, nachrichten) VALUES (new.id, new.titel, new.nachrichten);
            END;",
        )?;

        // Migration für eine bestehende gedaechtnis.db: Chats, die vor
        // den obigen Triggern gespeichert wurden, fehlen im Suchindex.
        // Einmalig nachziehen, ohne die chats-Tabelle selbst anzufassen.
        {
            let anzahl: i64 = verbindung.query_row("SELECT count(*) FROM chats", [], |r| r.get(0))?;
            let indiziert: i64 = verbindung
                .query_row("SELECT count(*) FROM chats_fts_docsize", [], |r| r.get(0))
                .unwrap_or(0);
            if indiziert < anzahl {
                verbindung.execute_batch("INSERT INTO chats_fts(chats_fts) VALUES('rebuild');")?;
            }
        }

        Ok(Self {
            verbindung: Mutex::new(verbindung),
        })
    }

    /// Alle nicht-archivierten Chats, neueste zuerst.
    pub fn liste(&self) -> Result<Vec<ChatEintrag>> {
        // Wie memory.rs::Gedaechtnis: eine vergiftete Sperre (nach einem
        // Panic, während sie gehalten wurde) macht History nicht dauerhaft
        // unbenutzbar - jeder weitere Aufruf über die FFI-Grenze würde
        // sonst mit `.expect()` erneut paniken, statt den Fehler einmalig
        // durchzureichen.
        let verbindung = self.verbindung.lock().unwrap_or_else(|e| e.into_inner());
        let mut abfrage = verbindung.prepare(
            "SELECT id, titel, nachrichten, erstellt, geaendert, archiviert
             FROM chats WHERE archiviert = 0 ORDER BY geaendert DESC",
        )?;
        let eintraege = abfrage
            .query_map([], |zeile| {
                Ok(ChatEintrag {
                    id: zeile.get(0)?,
                    titel: zeile.get(1)?,
                    nachrichten: zeile.get(2)?,
                    erstellt: zeile.get(3)?,
                    geaendert: zeile.get(4)?,
                    archiviert: zeile.get::<_, i32>(5)? != 0,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(eintraege)
    }

    /// Alle archivierten Chats, älteste zuerst (Archiv ist chronologisch).
    pub fn archiv_liste(&self) -> Result<Vec<ChatEintrag>> {
        // Wie memory.rs::Gedaechtnis: eine vergiftete Sperre (nach einem
        // Panic, während sie gehalten wurde) macht History nicht dauerhaft
        // unbenutzbar - jeder weitere Aufruf über die FFI-Grenze würde
        // sonst mit `.expect()` erneut paniken, statt den Fehler einmalig
        // durchzureichen.
        let verbindung = self.verbindung.lock().unwrap_or_else(|e| e.into_inner());
        let mut abfrage = verbindung.prepare(
            "SELECT id, titel, nachrichten, erstellt, geaendert, archiviert
             FROM chats WHERE archiviert = 1 ORDER BY erstellt ASC",
        )?;
        let eintraege = abfrage
            .query_map([], |zeile| {
                Ok(ChatEintrag {
                    id: zeile.get(0)?,
                    titel: zeile.get(1)?,
                    nachrichten: zeile.get(2)?,
                    erstellt: zeile.get(3)?,
                    geaendert: zeile.get(4)?,
                    archiviert: zeile.get::<_, i32>(5)? != 0,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(eintraege)
    }

    /// Durchsucht nicht-archivierte Chats nach einem Begriff (FTS5).
    pub fn suche(&self, begriff: &str) -> Result<Vec<ChatEintrag>> {
        // Wie memory.rs::Gedaechtnis: eine vergiftete Sperre (nach einem
        // Panic, während sie gehalten wurde) macht History nicht dauerhaft
        // unbenutzbar - jeder weitere Aufruf über die FFI-Grenze würde
        // sonst mit `.expect()` erneut paniken, statt den Fehler einmalig
        // durchzureichen.
        let verbindung = self.verbindung.lock().unwrap_or_else(|e| e.into_inner());
        // FTS5: Anführungszeichen escapen, dann in doppelte Anführungszeichen.
        let escaped = begriff.replace('"', "\"\"");
        let fts_muster = format!("\"{}\"", escaped);
        let mut abfrage = verbindung.prepare(
            "SELECT c.id, c.titel, c.nachrichten, c.erstellt, c.geaendert, c.archiviert
             FROM chats c
             JOIN chats_fts fts ON c.id = fts.rowid
             WHERE c.archiviert = 0 AND chats_fts MATCH ?1
             ORDER BY rank",
        )?;
        let eintraege = abfrage
            .query_map([&fts_muster], |zeile| {
                Ok(ChatEintrag {
                    id: zeile.get(0)?,
                    titel: zeile.get(1)?,
                    nachrichten: zeile.get(2)?,
                    erstellt: zeile.get(3)?,
                    geaendert: zeile.get(4)?,
                    archiviert: zeile.get::<_, i32>(5)? != 0,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(eintraege)
    }

    /// Speichert einen neuen Chat und gibt seine ID zurück.
    pub fn speichern(&self, titel: &str, nachrichten: &str) -> Result<i64> {
        // Wie memory.rs::Gedaechtnis: eine vergiftete Sperre (nach einem
        // Panic, während sie gehalten wurde) macht History nicht dauerhaft
        // unbenutzbar - jeder weitere Aufruf über die FFI-Grenze würde
        // sonst mit `.expect()` erneut paniken, statt den Fehler einmalig
        // durchzureichen.
        let verbindung = self.verbindung.lock().unwrap_or_else(|e| e.into_inner());
        verbindung.execute(
            "INSERT INTO chats (titel, nachrichten) VALUES (?1, ?2)",
            (titel, nachrichten),
        )?;
        Ok(verbindung.last_insert_rowid())
    }

    /// Aktualisiert einen bestehenden Chat (Titel und Nachrichten).
    pub fn aktualisieren(&self, id: i64, titel: &str, nachrichten: &str) -> Result<()> {
        // Wie memory.rs::Gedaechtnis: eine vergiftete Sperre (nach einem
        // Panic, während sie gehalten wurde) macht History nicht dauerhaft
        // unbenutzbar - jeder weitere Aufruf über die FFI-Grenze würde
        // sonst mit `.expect()` erneut paniken, statt den Fehler einmalig
        // durchzureichen.
        let verbindung = self.verbindung.lock().unwrap_or_else(|e| e.into_inner());
        verbindung.execute(
            "UPDATE chats SET titel = ?1, nachrichten = ?2, geaendert = datetime('now','localtime') WHERE id = ?3",
            (titel, nachrichten, id),
        )?;
        Ok(())
    }

    /// Löscht einen Chat endgültig aus der Datenbank.
    pub fn loeschen(&self, id: i64) -> Result<()> {
        // Wie memory.rs::Gedaechtnis: eine vergiftete Sperre (nach einem
        // Panic, während sie gehalten wurde) macht History nicht dauerhaft
        // unbenutzbar - jeder weitere Aufruf über die FFI-Grenze würde
        // sonst mit `.expect()` erneut paniken, statt den Fehler einmalig
        // durchzureichen.
        let verbindung = self.verbindung.lock().unwrap_or_else(|e| e.into_inner());
        verbindung.execute("DELETE FROM chats WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Verschiebt einen Chat ins Archiv (oder holt ihn zurück).
    pub fn archivieren(&self, id: i64, archiviert: bool) -> Result<()> {
        // Wie memory.rs::Gedaechtnis: eine vergiftete Sperre (nach einem
        // Panic, während sie gehalten wurde) macht History nicht dauerhaft
        // unbenutzbar - jeder weitere Aufruf über die FFI-Grenze würde
        // sonst mit `.expect()` erneut paniken, statt den Fehler einmalig
        // durchzureichen.
        let verbindung = self.verbindung.lock().unwrap_or_else(|e| e.into_inner());
        let wert = if archiviert { 1 } else { 0 };
        verbindung.execute(
            "UPDATE chats SET archiviert = ?1, geaendert = datetime('now','localtime') WHERE id = ?2",
            (wert, id),
        )?;
        Ok(())
    }

    /// Räumt alte Chat-Verläufe auf: behält die letzten `max` nicht-archivierten
    /// und alle archivierten Chats, löscht alles andere. Verhindert, dass die
    /// `chats`/`chats_fts`-Tabellen unbegrenzt wachsen. Die FTS5-Trigger sorgen
    /// dafür, dass gelöschte Chats automatisch aus dem Suchindex verschwinden.
    /// Gibt die Anzahl gelöschter Chats zurück.
    pub fn bereinigen(&self, max_nicht_archiviert: usize) -> Result<usize> {
        let verbindung = self.verbindung.lock().unwrap_or_else(|e| e.into_inner());
        let anzahl: i64 = verbindung.query_row(
            "SELECT count(*) FROM chats WHERE archiviert = 0", [], |r| r.get(0))?;
        if anzahl <= max_nicht_archiviert as i64 {
            return Ok(0);
        }
        let geloescht = verbindung.execute(
            "DELETE FROM chats WHERE archiviert = 0 AND id NOT IN (
                SELECT id FROM chats WHERE archiviert = 0
                ORDER BY id DESC LIMIT ?1
            )", [max_nicht_archiviert as i64])?;
        Ok(geloescht)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        std::env::temp_dir().join(format!(
            "famulus_history_{}_{}.db",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ))
    }

    /// Ohne Sync-Trigger auf `chats_fts` bleibt der FTS5-Index leer und
    /// `suche()` findet nie etwas, egal wie viele Chats gespeichert sind.
    #[test]
    fn suche_findet_gespeicherten_chat() {
        let pfad = temp();
        let h = History::oeffnen(&pfad).unwrap();
        h.speichern("Codesignierung", "Fehler beim Signieren der App").unwrap();
        let treffer = h.suche("Signieren").unwrap();
        assert_eq!(treffer.len(), 1);
        assert_eq!(treffer[0].titel, "Codesignierung");
        std::fs::remove_file(pfad).ok();
    }

    /// Aktualisierte Titel/Nachrichten müssen im Index nachgezogen werden,
    /// gelöschte Chats dürfen nicht mehr auffindbar sein.
    #[test]
    fn suche_folgt_aktualisierung_und_loeschung() {
        let pfad = temp();
        let h = History::oeffnen(&pfad).unwrap();
        let id = h.speichern("Altes Thema", "Ursprünglicher Inhalt").unwrap();

        h.aktualisieren(id, "Neues Thema", "Geänderter Inhalt").unwrap();
        assert!(h.suche("Geänderter").unwrap().iter().any(|c| c.id == id));
        assert!(h.suche("Ursprünglicher").unwrap().is_empty());

        h.loeschen(id).unwrap();
        assert!(h.suche("Geänderter").unwrap().is_empty());
        std::fs::remove_file(pfad).ok();
    }

    /// Eine bestehende gedaechtnis.db kann Chats enthalten, die vor der
    /// Trigger-Synchronisation gespeichert wurden. `oeffnen()` muss den
    /// Index dafür einmalig nachziehen, ohne die Chats selbst anzufassen.
    #[test]
    fn bestehende_db_ohne_index_wird_beim_oeffnen_nachindiziert() {
        let pfad = temp();
        {
            let verbindung = Connection::open(&pfad).unwrap();
            verbindung
                .execute_batch(
                    "CREATE TABLE chats (
                        id          INTEGER PRIMARY KEY,
                        titel       TEXT NOT NULL,
                        nachrichten TEXT NOT NULL DEFAULT '[]',
                        erstellt    TEXT NOT NULL DEFAULT (datetime('now','localtime')),
                        geaendert   TEXT NOT NULL DEFAULT (datetime('now','localtime')),
                        archiviert  INTEGER NOT NULL DEFAULT 0
                    );",
                )
                .unwrap();
            verbindung
                .execute(
                    "INSERT INTO chats (titel, nachrichten) VALUES (?1, ?2)",
                    ("Alter Chat", "[\"Netzwerkfehler beim Verbindungsaufbau\"]"),
                )
                .unwrap();
        }
        let h = History::oeffnen(&pfad).unwrap();
        assert_eq!(h.liste().unwrap().len(), 1, "oeffnen() darf keine Zeilen verlieren");
        let treffer = h.suche("Netzwerkfehler").unwrap();
        assert_eq!(treffer.len(), 1, "Nachträglich indizierter Alt-Chat muss auffindbar sein");
        std::fs::remove_file(pfad).ok();
    }

    #[test]
    fn bereinigen_entfernt_alte_chats() {
        let pfad = temp();
        let h = History::oeffnen(&pfad).unwrap();
        h.speichern("Chat 1", "test").unwrap();
        h.speichern("Chat 2", "test").unwrap();
        h.speichern("Chat 3", "test").unwrap();
        h.speichern("Chat 4", "test").unwrap();
        h.speichern("Chat 5", "test").unwrap();
        let geloescht = h.bereinigen(3).unwrap();
        assert_eq!(geloescht, 2);
        let uebrig = h.liste().unwrap();
        assert_eq!(uebrig.len(), 3);
        let titel: Vec<String> = uebrig.iter().map(|c| c.titel.clone()).collect();
        assert!(titel.contains(&"Chat 5".to_string()));
        assert!(titel.contains(&"Chat 4".to_string()));
        assert!(titel.contains(&"Chat 3".to_string()));
        std::fs::remove_file(pfad).ok();
    }

    #[test]
    fn bereinigen_laesst_kleine_historie_in_ruhe() {
        let pfad = temp();
        let h = History::oeffnen(&pfad).unwrap();
        h.speichern("Chat 1", "test").unwrap();
        h.speichern("Chat 2", "test").unwrap();
        let geloescht = h.bereinigen(5).unwrap();
        assert_eq!(geloescht, 0);
        assert_eq!(h.liste().unwrap().len(), 2);
        std::fs::remove_file(pfad).ok();
    }
}