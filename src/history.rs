//! Chat-Verlauf mit Volltextsuche.
//!
//! Alle Chat-Sessions landen in derselben `gedaechtnis.db` wie die
//! Erinnerungen – eine Datenbankdatei statt zwei. Die Tabelle `chats`
//! speichert die Nachrichten als JSON-Blob, damit die Struktur flexibel
//! bleibt und nicht jedes Mal migriert werden muss, wenn das Frontend
//! ein neues Feld hinzufügt.
//!
//! Die Suche läuft über SQLite LIKE – für ein paar hundert Chat-Sessions
//! ist das schnell genug. FTS5 wäre erst sinnvoll, wenn die Datenbank auf
//! tausende Einträge wächst.

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

        Ok(Self {
            verbindung: Mutex::new(verbindung),
        })
    }

    /// Alle nicht-archivierten Chats, neueste zuerst.
    pub fn liste(&self) -> Result<Vec<ChatEintrag>> {
        let verbindung = self.verbindung.lock().expect("History vergiftet");
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
        let verbindung = self.verbindung.lock().expect("History vergiftet");
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

    /// Durchsucht nicht-archivierte Chats nach einem Begriff.
    /// Sucht in Titel und Nachrichten-Inhalt (LIKE).
    pub fn suche(&self, begriff: &str) -> Result<Vec<ChatEintrag>> {
        let verbindung = self.verbindung.lock().expect("History vergiftet");
        let muster = format!("%{}%", begriff.replace('%', "\\%").replace('_', "\\_"));
        let mut abfrage = verbindung.prepare(
            "SELECT id, titel, nachrichten, erstellt, geaendert, archiviert
             FROM chats
             WHERE archiviert = 0 AND (titel LIKE ?1 OR nachrichten LIKE ?1)
             ORDER BY geaendert DESC",
        )?;
        let eintraege = abfrage
            .query_map([&muster], |zeile| {
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
        let verbindung = self.verbindung.lock().expect("History vergiftet");
        verbindung.execute(
            "INSERT INTO chats (titel, nachrichten) VALUES (?1, ?2)",
            (titel, nachrichten),
        )?;
        Ok(verbindung.last_insert_rowid())
    }

    /// Aktualisiert einen bestehenden Chat (Titel und Nachrichten).
    pub fn aktualisieren(&self, id: i64, titel: &str, nachrichten: &str) -> Result<()> {
        let verbindung = self.verbindung.lock().expect("History vergiftet");
        verbindung.execute(
            "UPDATE chats SET titel = ?1, nachrichten = ?2, geaendert = datetime('now','localtime') WHERE id = ?3",
            (titel, nachrichten, id),
        )?;
        Ok(())
    }

    /// Löscht einen Chat endgültig aus der Datenbank.
    pub fn loeschen(&self, id: i64) -> Result<()> {
        let verbindung = self.verbindung.lock().expect("History vergiftet");
        verbindung.execute("DELETE FROM chats WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Verschiebt einen Chat ins Archiv (oder holt ihn zurück).
    pub fn archivieren(&self, id: i64, archiviert: bool) -> Result<()> {
        let verbindung = self.verbindung.lock().expect("History vergiftet");
        let wert = if archiviert { 1 } else { 0 };
        verbindung.execute(
            "UPDATE chats SET archiviert = ?1, geaendert = datetime('now','localtime') WHERE id = ?2",
            (wert, id),
        )?;
        Ok(())
    }
}