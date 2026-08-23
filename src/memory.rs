//! Das Gedächtnis: was Famulus über Aufträge hinweg behält.
//!
//! Drei Stufen, die aufeinander aufbauen:
//!
//! - **Stufe 1 (FTS5):** Volltextsuche für Erinnerungen und Vault. Ersetzt
//!   das alte Keyword-Matching durch BM25-Ranking von SQLite.
//! - **Stufe 2 (Embeddings):** Semantische Vektor-Suche via Ollama.
//!   `nomic-embed-text` generiert Embedding-Vektoren, Cosine Similarity in
//!   Rust. Erkennt, dass "Code-Signing-Fix" und "Zertifikat" zusammengehören.
//!   Bewusst ein dediziertes Embedding-Modell statt qwen3:14b (dem Chat-
//!   Modell, das Famulus sonst benutzt): qwen3:14b hat laut Ollama keine
//!   "embedding"-Fähigkeit und `/api/embeddings` lehnt es rundweg ab - ein
//!   14B-Chat-Modell für Embeddings zu zweckentfremden wäre ohnehin
//!   Ressourcenverschwendung gegenüber einem 274-MB-Modell, das genau
//!   dafür gebaut ist.
//! - **Stufe 3 (Notizbuch):** Der Agent kann sich während der Arbeit selbst
//!   Notizen machen und diese später konsolidieren – echtes In-Task-Lernen.
//!
//! Die Trennung zum Obsidian-Vault (`tools/vault.rs`) bleibt bestehen:
//! Die Datenbank hält kurze Fakten, der Vault ausführliches Wissen.

use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use serde_json::Value;
use std::path::Path;
use std::sync::Mutex;

/// Ein gemerkter Satz.
#[derive(Debug, Clone)]
pub struct Erinnerung {
    pub art: String,
    pub inhalt: String,
}

/// Aggregierte Bilanz eines Providers über alle bisherigen Aufrufe.
#[derive(Debug, Clone)]
pub struct ProviderStatistik {
    pub provider: String,
    pub anzahl: i64,
    pub erfolgsquote: f64,
    pub durchschnitt_ms: f64,
}

/// Womit man es zu tun hat. Bewusst wenige Kategorien - je mehr Schubladen,
/// desto öfter landet etwas in der falschen.
pub const ART_PRAEFERENZ: &str = "praeferenz"; // wie Jens Dinge haben will
pub const ART_FAKT: &str = "fakt"; // wie das System/Projekt beschaffen ist
pub const ART_LEKTION: &str = "lektion"; // was beim Arbeiten schiefging

/// Dediziertes Embedding-Modell für Stufe 2. Muss vorher gepullt sein:
/// `ollama pull nomic-embed-text` (274 MB, kein extra Server-Flag nötig).
const EMBEDDING_MODELL: &str = "nomic-embed-text";

pub struct Gedaechtnis {
    verbindung: Mutex<Connection>,
}

impl Gedaechtnis {
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

        // ── Basis-Tabellen ────────────────────────────────────────────
        verbindung.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS erinnerungen (
                id       INTEGER PRIMARY KEY,
                art      TEXT NOT NULL,
                inhalt   TEXT NOT NULL UNIQUE,
                quelle   TEXT,
                erstellt TEXT NOT NULL DEFAULT (datetime('now','localtime'))
            );
            ",
        )?;

        // ── Stufe 1: FTS5-Volltextsuche ───────────────────────────────
        // content='erinnerungen' heißt: die Spaltenwerte kommen aus der
        // erinnerungen-Tabelle. Der Suchindex selbst (die invertierte
        // Liste) wird dadurch aber NICHT automatisch mitgepflegt - SQLite
        // synchronisiert externe Content-Tabellen nur über Trigger. Ohne
        // sie bleibt der Index leer und jede MATCH-Suche liefert null
        // Treffer, obwohl `erinnerungen` selbst befüllt ist.
        verbindung.execute_batch(
            "CREATE VIRTUAL TABLE IF NOT EXISTS erinnerungen_fts USING fts5(
                inhalt,
                content='erinnerungen',
                content_rowid='id'
            );
            CREATE TRIGGER IF NOT EXISTS erinnerungen_fts_ai AFTER INSERT ON erinnerungen BEGIN
                INSERT INTO erinnerungen_fts(rowid, inhalt) VALUES (new.id, new.inhalt);
            END;
            CREATE TRIGGER IF NOT EXISTS erinnerungen_fts_ad AFTER DELETE ON erinnerungen BEGIN
                INSERT INTO erinnerungen_fts(erinnerungen_fts, rowid, inhalt) VALUES ('delete', old.id, old.inhalt);
            END;
            CREATE TRIGGER IF NOT EXISTS erinnerungen_fts_au AFTER UPDATE ON erinnerungen BEGIN
                INSERT INTO erinnerungen_fts(erinnerungen_fts, rowid, inhalt) VALUES ('delete', old.id, old.inhalt);
                INSERT INTO erinnerungen_fts(rowid, inhalt) VALUES (new.id, new.inhalt);
            END;",
        )?;

        // Migration für bestehende gedaechtnis.db: Zeilen, die vor den
        // obigen Triggern eingefügt wurden, fehlen im Suchindex. Einmalig
        // nachziehen, ohne die erinnerungen-Tabelle selbst anzufassen.
        {
            let anzahl: i64 =
                verbindung.query_row("SELECT count(*) FROM erinnerungen", [], |r| r.get(0))?;
            let indiziert: i64 = verbindung
                .query_row("SELECT count(*) FROM erinnerungen_fts_docsize", [], |r| {
                    r.get(0)
                })
                .unwrap_or(0);
            if indiziert < anzahl {
                verbindung
                    .execute_batch("INSERT INTO erinnerungen_fts(erinnerungen_fts) VALUES('rebuild');")?;
            }
        }

        // Vault-Index: wird bei Bedarf neu aufgebaut.
        verbindung.execute_batch(
            "CREATE VIRTUAL TABLE IF NOT EXISTS vault_idx USING fts5(
                pfad,
                inhalt,
                tokenize='unicode61'
            );",
        )?;

        // ── Stufe 2: Embeddings ───────────────────────────────────────
        verbindung.execute_batch(
            "CREATE TABLE IF NOT EXISTS erinnerungen_embedding (
                id        INTEGER PRIMARY KEY REFERENCES erinnerungen(id) ON DELETE CASCADE,
                embedding BLOB NOT NULL
            );",
        )?;

        // ── Stufe 3: Notizbuch ────────────────────────────────────────
        verbindung.execute_batch(
            "CREATE TABLE IF NOT EXISTS notizbuch (
                id       INTEGER PRIMARY KEY,
                inhalt   TEXT NOT NULL,
                erstellt TEXT NOT NULL DEFAULT (datetime('now','localtime'))
            );",
        )?;

        // ── Provider-Statistik (Router-Scorecard) ──────────────────────
        verbindung.execute_batch(
            "CREATE TABLE IF NOT EXISTS provider_statistik (
                id        INTEGER PRIMARY KEY,
                provider  TEXT NOT NULL,
                erfolg    INTEGER NOT NULL,
                dauer_ms  INTEGER NOT NULL,
                zeitpunkt TEXT NOT NULL DEFAULT (datetime('now','localtime'))
            );",
        )?;

        Ok(Self {
            verbindung: Mutex::new(verbindung),
        })
    }

    // ── Basis: Merken ─────────────────────────────────────────────────

    /// Merkt sich einen Satz. Gibt `false` zurück, wenn er schon bekannt war.
    pub fn merken(&self, art: &str, inhalt: &str, quelle: &str) -> Result<bool> {
        let inhalt = inhalt.trim();
        if inhalt.is_empty() {
            return Ok(false);
        }
        let verbindung = self.verbindung.lock().expect("Gedächtnis vergiftet");
        let geaendert = verbindung.execute(
            "INSERT OR IGNORE INTO erinnerungen (art, inhalt, quelle) VALUES (?1, ?2, ?3)",
            (art, inhalt, quelle),
        )?;
        Ok(geaendert > 0)
    }

    /// Wie `merken`, berechnet bei einer neuen Erinnerung aber sofort ihr
    /// Embedding (Stufe 2) – ohne das wäre frisch Gelerntes erst nach dem
    /// nächsten Start über `embeddings_nachholen` semantisch auffindbar.
    pub async fn merken_und_einbetten(&self, art: &str, inhalt: &str, quelle: &str) -> Result<bool> {
        let neu = self.merken(art, inhalt, quelle)?;
        if neu {
            let id: Option<i64> = {
                let verbindung = self.verbindung.lock().expect("Gedächtnis vergiftet");
                verbindung
                    .query_row(
                        "SELECT id FROM erinnerungen WHERE inhalt = ?1",
                        params![inhalt.trim()],
                        |r| r.get(0),
                    )
                    .ok()
            };
            if let Some(id) = id {
                let _ = self.embedding_speichern(id).await;
            }
        }
        Ok(neu)
    }

    // ── Stufe 1: FTS5-Suche ───────────────────────────────────────────

    /// Die Erinnerungen, die zu diesem Auftrag am ehesten passen.
    ///
    /// Nutzt FTS5 mit BM25-Ranking für die Relevanz. Präferenzen kommen
    /// immer mit – egal ob sie zum Auftrag passen oder nicht.
    /// Fällt FTS5 aus (z.B. weil die Tabelle noch nicht existiert), greift
    /// die alte Keyword-Matching-Methode als Rückfall.
    pub fn relevante(&self, auftrag: &str, hoechstens: usize) -> Result<Vec<Erinnerung>> {
        let verbindung = self.verbindung.lock().expect("Gedächtnis vergiftet");

        // 1. Alle Präferenzen – immer relevant, unabhängig vom Auftrag.
        let praefs = verbindung
            .prepare("SELECT art, inhalt FROM erinnerungen WHERE art = ?1 ORDER BY id DESC")?
            .query_map(params![ART_PRAEFERENZ], |zeile| {
                Ok(Erinnerung {
                    art: zeile.get(0)?,
                    inhalt: zeile.get(1)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect::<Vec<_>>();

        let mut gesehen: std::collections::HashSet<String> = praefs
            .iter()
            .map(|e| e.inhalt.clone())
            .collect();
        let mut ergebnis = praefs;

        // 2. FTS5-Suche für den Rest (falls die Tabelle existiert).
        let fts_ok = verbindung
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='erinnerungen_fts'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap_or(0) > 0;

        if fts_ok {
            // FTS5-MATCH mit dem Auftragstext. SQLite-FTS5 erwartet eine
            // saubere Query – wir escapen Sonderzeichen und nehmen nur
            // Wörter mit >= 4 Zeichen (wie das alte worte()).
            let suchbegriffe: Vec<String> = worte(auftrag)
                .into_iter()
                .map(|w| format!("\"{}\"", w.replace('"', "")))
                .collect();

            if !suchbegriffe.is_empty() {
                let query = suchbegriffe.join(" OR ");
                let mut stmt = verbindung.prepare(
                    "SELECT e.art, e.inhalt
                     FROM erinnerungen e
                     JOIN erinnerungen_fts f ON e.id = f.rowid
                     WHERE erinnerungen_fts MATCH ?1 AND e.art != ?2
                     ORDER BY bm25(erinnerungen_fts)
                     LIMIT ?3",
                )?;

                let rest = stmt
                    .query_map(params![query, ART_PRAEFERENZ, hoechstens as i64], |zeile| {
                        Ok(Erinnerung {
                            art: zeile.get(0)?,
                            inhalt: zeile.get(1)?,
                        })
                    })?
                    .filter_map(|r| r.ok())
                    .filter(|e| gesehen.insert(e.inhalt.clone()))
                    .collect::<Vec<_>>();

                ergebnis.extend(rest);
            }
        }

        // 3. Falls FTS5 nicht genug liefert: alte Keyword-Methode als Fallback.
        if ergebnis.len() < hoechstens {
            let mut alle: Vec<Erinnerung> = verbindung
                .prepare("SELECT art, inhalt FROM erinnerungen WHERE art != ?1 ORDER BY id DESC")?
                .query_map(params![ART_PRAEFERENZ], |zeile| {
                    Ok(Erinnerung {
                        art: zeile.get(0)?,
                        inhalt: zeile.get(1)?,
                    })
                })?
                .filter_map(|r| r.ok())
                .filter(|e| gesehen.insert(e.inhalt.clone()))
                .collect();

            let stichworte: std::collections::HashSet<String> =
                worte(auftrag).into_iter().collect();
            alle.sort_by_key(|e| {
                let treffer = worte(&e.inhalt)
                    .iter()
                    .filter(|w| stichworte.contains(*w))
                    .count();
                std::cmp::Reverse(treffer)
            });

            for e in alle {
                if ergebnis.len() >= hoechstens {
                    break;
                }
                ergebnis.push(e);
            }
        }

        ergebnis.truncate(hoechstens);
        Ok(ergebnis)
    }

    // ── Stufe 1: Vault-Suche ──────────────────────────────────────────

    /// Baut den FTS5-Index für den Vault neu auf.
    pub fn vault_index_aktualisieren(&self, vault_wurzel: &Path) -> Result<usize> {
        let verbindung = self.verbindung.lock().expect("Gedächtnis vergiftet");

        // Alten Index löschen.
        verbindung.execute("DELETE FROM vault_idx", [])?;

        let mut anzahl = 0;
        let mut notizen = Vec::new();
        sammle_markdown(vault_wurzel, vault_wurzel, &mut notizen)?;

        for (pfad, inhalt) in &notizen {
            verbindung.execute(
                "INSERT INTO vault_idx (pfad, inhalt) VALUES (?1, ?2)",
                params![pfad, inhalt],
            )?;
            anzahl += 1;
        }

        Ok(anzahl)
    }

    /// Durchsucht den Vault-Index nach einem Begriff.
    pub fn vault_suche(
        &self,
        begriff: &str,
        hoechstens: usize,
    ) -> Result<Vec<(String, String)>> {
        let verbindung = self.verbindung.lock().expect("Gedächtnis vergiftet");

        // Prüfen, ob der Index existiert und nicht leer ist.
        let count: i64 = verbindung.query_row(
            "SELECT count(*) FROM vault_idx",
            [],
            |r| r.get(0),
        )?;
        if count == 0 {
            return Ok(Vec::new());
        }

        let mut stmt = verbindung.prepare(
            "SELECT pfad, snippet(vault_idx, 1, '<mark>', '</mark>', '...', 40)
             FROM vault_idx
             WHERE vault_idx MATCH ?1
             ORDER BY bm25(vault_idx)
             LIMIT ?2",
        )?;

        // `begriff` kommt vom Agenten (letztlich einem LLM) und darf keine
        // rohe FTS5-Query sein: ein Streuzeichen wie `"` lässt den
        // MATCH-Ausdruck mit "unterminated string" scheitern, und ein
        // Wort wie "NOT"/"AND"/"OR" würde als FTS5-Operator statt als
        // Suchwort interpretiert. Anführungszeichen escapen und als
        // Phrase einfassen, so wie in history::suche().
        let fts_muster = format!("\"{}\"", begriff.replace('"', "\"\""));

        let ergebnisse = stmt
            .query_map(params![fts_muster, hoechstens as i64], |zeile| {
                Ok((zeile.get::<_, String>(0)?, zeile.get::<_, String>(1)?))
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(ergebnisse)
    }

    // ── Stufe 2: Embeddings ───────────────────────────────────────────

    /// Berechnet das Embedding für einen Text via Ollama.
    ///
    /// Async mit dem normalen `reqwest::Client` – nicht `reqwest::blocking`.
    /// Famulus läuft komplett auf Tokio (CLI wie GUI); ein blockierender
    /// HTTP-Client reißt dort sofort den Prozess mit einem Panic runter
    /// ("Cannot drop a runtime in a context where blocking is not allowed"),
    /// weil er versucht, innerhalb eines laufenden Runtime-Threads eine
    /// zweite Runtime aufzumachen.
    /// Prüft, ob Ollama gerade erreichbar ist und `nomic-embed-text` Embeddings
    /// ausliefert. Wird einmal beim Start aufgerufen, damit `relevante_semantisch`
    /// nicht bei jedem einzelnen Auftrag erneut einen zum Scheitern
    /// verurteilten Netzwerk-Aufruf macht, wenn Ollama das eh nicht kann.
    pub async fn embeddings_verfuegbar() -> bool {
        Self::embedding_berechnen("Verfügbarkeitsprüfung").await.is_ok()
    }

    async fn embedding_berechnen(text: &str) -> Result<Vec<f32>> {
        let resp = reqwest::Client::new()
            .post("http://localhost:11434/api/embeddings")
            .json(&serde_json::json!({
                "model": EMBEDDING_MODELL,
                "prompt": text
            }))
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
            .context("Ollama-Embedding-API nicht erreichbar")?;

        let body: Value = resp.json().await.context("Ungültige Embedding-Antwort")?;
        let embedding: Vec<f32> = body["embedding"]
            .as_array()
            .context("Kein 'embedding'-Feld in Antwort")?
            .iter()
            .map(|v| v.as_f64().unwrap_or(0.0) as f32)
            .collect();

        if embedding.is_empty() {
            anyhow::bail!("Leeres Embedding von Ollama");
        }
        Ok(embedding)
    }

    /// Speichert das Embedding für eine Erinnerung. Gibt zurück, ob danach
    /// wirklich eins vorliegt (`false` heißt: Ollama war nicht erreichbar
    /// oder lieferte keins - kein Fehler, nur "hat nicht geklappt").
    /// Wird nach dem Merken aufgerufen, scheitert aber nicht – ein fehlendes
    /// Embedding ist kein Grund, den Auftrag abzubrechen.
    ///
    /// Die DB-Sperre wird bewusst vor dem `.await` wieder freigegeben -
    /// ein `std::sync::MutexGuard` über einen Await-Punkt hinweg zu halten,
    /// blockiert für die Dauer des Netzwerk-Aufrufs jeden anderen Zugriff
    /// aufs Gedächtnis (z.B. die GUI, die parallel den Chatverlauf liest).
    pub async fn embedding_speichern(&self, id: i64) -> Result<bool> {
        let inhalt = {
            let verbindung = self.verbindung.lock().expect("Gedächtnis vergiftet");
            let hat: bool = verbindung.query_row(
                "SELECT count(*) > 0 FROM erinnerungen_embedding WHERE id = ?1",
                params![id],
                |r| r.get(0),
            )?;
            if hat {
                return Ok(true);
            }
            verbindung.query_row(
                "SELECT inhalt FROM erinnerungen WHERE id = ?1",
                params![id],
                |r| r.get::<_, String>(0),
            )?
        };

        match Self::embedding_berechnen(&inhalt).await {
            Ok(embedding) => {
                let bytes: Vec<u8> = embedding
                    .iter()
                    .flat_map(|f| f.to_le_bytes())
                    .collect();
                let verbindung = self.verbindung.lock().expect("Gedächtnis vergiftet");
                verbindung.execute(
                    "INSERT OR REPLACE INTO erinnerungen_embedding (id, embedding) VALUES (?1, ?2)",
                    params![id, bytes],
                )?;
                Ok(true)
            }
            Err(e) => {
                // Leise fehlschlagen – Embeddings sind optional.
                eprintln!("[memory] Embedding für id={id} fehlgeschlagen: {e:#}");
                Ok(false)
            }
        }
    }

    /// Semantische Suche via Cosine Similarity auf Embedding-Vektoren.
    pub async fn relevante_semantisch(
        &self,
        auftrag: &str,
        hoechstens: usize,
    ) -> Result<Vec<Erinnerung>> {
        // 1. Alle Präferenzen (immer relevant). Sperre wird vor dem
        // Netzwerk-Aufruf unten wieder freigegeben, siehe embedding_speichern.
        let mut gesehen: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut ergebnis: Vec<Erinnerung> = {
            let verbindung = self.verbindung.lock().expect("Gedächtnis vergiftet");
            let gefunden: Vec<Erinnerung> = verbindung
                .prepare("SELECT art, inhalt FROM erinnerungen WHERE art = ?1 ORDER BY id DESC")?
                .query_map(params![ART_PRAEFERENZ], |zeile| {
                    Ok(Erinnerung {
                        art: zeile.get(0)?,
                        inhalt: zeile.get(1)?,
                    })
                })?
                .filter_map(|r| {
                    if let Ok(ref e) = r {
                        gesehen.insert(e.inhalt.clone());
                    }
                    r.ok()
                })
                .collect();
            gefunden
        };

        // 2. Query-Embedding berechnen (kein Lock gehalten während des Awaits).
        let query_emb = match Self::embedding_berechnen(auftrag).await {
            Ok(e) => e,
            Err(_) => {
                // Fallback: FTS5-Suche.
                return self.relevante(auftrag, hoechstens);
            }
        };

        // 3. Alle gespeicherten Embeddings laden und vergleichen.
        let mut bewertet: Vec<(f32, Erinnerung)> = {
            let verbindung = self.verbindung.lock().expect("Gedächtnis vergiftet");
            let mut stmt = verbindung.prepare(
                "SELECT e.id, e.art, e.inhalt, em.embedding
                 FROM erinnerungen e
                 JOIN erinnerungen_embedding em ON e.id = em.id
                 WHERE e.art != ?1",
            )?;

            let gefunden: Vec<(f32, Erinnerung)> = stmt
                .query_map(params![ART_PRAEFERENZ], |zeile| {
                    let id: i64 = zeile.get(0)?;
                    let art: String = zeile.get(1)?;
                    let inhalt: String = zeile.get(2)?;
                    let bytes: Vec<u8> = zeile.get(3)?;
                    Ok((id, art, inhalt, bytes))
                })?
                .filter_map(|r| r.ok())
                .filter(|(_, _, inhalt, _)| gesehen.insert(inhalt.clone()))
                .map(|(_, art, inhalt, bytes)| {
                    let emb: Vec<f32> = bytes
                        .chunks_exact(4)
                        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                        .collect();
                    let similarity = cosine_similarity(&query_emb, &emb);
                    (similarity, Erinnerung { art, inhalt })
                })
                .collect();
            gefunden
        };

        // 4. Nach Ähnlichkeit sortieren (höchste zuerst).
        bewertet.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

        for (_, e) in bewertet {
            if ergebnis.len() >= hoechstens {
                break;
            }
            ergebnis.push(e);
        }

        Ok(ergebnis)
    }

    /// Stellt sicher, dass alle Erinnerungen Embeddings haben.
    /// Wird beim Start aufgerufen – läuft einmal durch und füllt Lücken.
    ///
    /// Prüft zuerst einmalig, ob Ollama Embeddings überhaupt ausliefert -
    /// sonst würden hier bei jedem Start hundert-plus zum Scheitern
    /// verurteilte Anfragen rausgehen, eine pro fehlender Erinnerung.
    pub async fn embeddings_nachholen(&self) -> Result<usize> {
        if !Self::embeddings_verfuegbar().await {
            return Ok(0);
        }

        let fehlende: Vec<(i64, String)> = {
            let verbindung = self.verbindung.lock().expect("Gedächtnis vergiftet");
            let gefunden: Vec<(i64, String)> = verbindung
                .prepare(
                    "SELECT e.id, e.inhalt
                     FROM erinnerungen e
                     LEFT JOIN erinnerungen_embedding em ON e.id = em.id
                     WHERE em.id IS NULL",
                )?
                .query_map([], |zeile| Ok((zeile.get(0)?, zeile.get(1)?)))?
                .filter_map(|r| r.ok())
                .collect();
            gefunden
        };

        let mut anzahl = 0;
        for (id, _inhalt) in &fehlende {
            if self.embedding_speichern(*id).await.unwrap_or(false) {
                anzahl += 1;
            }
        }

        Ok(anzahl)
    }

    // ── Stufe 3: Notizbuch ────────────────────────────────────────────

    /// Schreibt eine Notiz während der Arbeit.
    pub fn notizbuch_schreiben(&self, inhalt: &str) -> Result<()> {
        let inhalt = inhalt.trim();
        if inhalt.is_empty() {
            return Ok(());
        }
        let verbindung = self.verbindung.lock().expect("Gedächtnis vergiftet");
        verbindung.execute(
            "INSERT INTO notizbuch (inhalt) VALUES (?1)",
            params![inhalt],
        )?;
        Ok(())
    }

    /// Liest alle Notizen aus dem aktuellen Auftrag.
    pub fn notizbuch_lesen(&self) -> Result<Vec<String>> {
        let verbindung = self.verbindung.lock().expect("Gedächtnis vergiftet");
        let notizen = verbindung
            .prepare("SELECT inhalt FROM notizbuch ORDER BY id ASC")?
            .query_map([], |zeile| zeile.get(0))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(notizen)
    }

    /// Löscht alle Notizen (nachdem sie konsolidiert wurden).
    pub fn notizbuch_leeren(&self) -> Result<()> {
        let verbindung = self.verbindung.lock().expect("Gedächtnis vergiftet");
        verbindung.execute("DELETE FROM notizbuch", [])?;
        Ok(())
    }

    // ── Provider-Statistik (Router-Scorecard) ──────────────────────────

    /// Protokolliert einen Modellaufruf. Wird vom Router nach jedem Versuch
    /// aufgerufen - auch nach fehlgeschlagenen, damit die Erfolgsquote
    /// stimmt. Rein additiv: kein Aufrufer wertet den Rückgabewert aus,
    /// ein Fehler hier darf nie die eigentliche Modellantwort verhindern.
    pub fn provider_aufruf_protokollieren(
        &self,
        provider: &str,
        erfolg: bool,
        dauer_ms: u64,
    ) -> Result<()> {
        let verbindung = self.verbindung.lock().expect("Gedächtnis vergiftet");
        verbindung.execute(
            "INSERT INTO provider_statistik (provider, erfolg, dauer_ms) VALUES (?1, ?2, ?3)",
            params![provider, erfolg as i64, dauer_ms as i64],
        )?;
        Ok(())
    }

    /// Erfolgsquote und Durchschnittslatenz je Provider, absteigend nach
    /// Aufrufzahl. `erfolgsquote` ist 0.0-1.0 (SQLite AVG über 0/1-Werte).
    pub fn provider_statistik(&self) -> Result<Vec<ProviderStatistik>> {
        let verbindung = self.verbindung.lock().expect("Gedächtnis vergiftet");
        let mut stmt = verbindung.prepare(
            "SELECT provider, count(*), avg(erfolg), avg(dauer_ms)
             FROM provider_statistik
             GROUP BY provider
             ORDER BY count(*) DESC",
        )?;
        let ergebnisse = stmt
            .query_map([], |zeile| {
                Ok(ProviderStatistik {
                    provider: zeile.get(0)?,
                    anzahl: zeile.get(1)?,
                    erfolgsquote: zeile.get(2)?,
                    durchschnitt_ms: zeile.get(3)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(ergebnisse)
    }

    pub fn anzahl(&self) -> Result<usize> {
        let verbindung = self.verbindung.lock().expect("Gedächtnis vergiftet");
        let n: i64 = verbindung.query_row("SELECT count(*) FROM erinnerungen", [], |r| r.get(0))?;
        Ok(n as usize)
    }
}

// ── Hilfsfunktionen ──────────────────────────────────────────────────

/// Cosine Similarity zwischen zwei Vektoren.
fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    dot / (norm_a * norm_b)
}

/// Zieht Wörter mit >= 4 Zeichen aus einem Text. Dient als Fallback und
/// für die FTS5-Query-Konstruktion.
fn worte(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() >= 4)
        .map(|w| w.to_string())
        .collect()
}

/// Sammelt Markdown-Dateien rekursiv aus einem Ordner.
fn sammle_markdown(wurzel: &Path, ordner: &Path, raus: &mut Vec<(String, String)>) -> Result<()> {
    let Ok(eintraege) = std::fs::read_dir(ordner) else {
        return Ok(());
    };
    for eintrag in eintraege.flatten() {
        let pfad = eintrag.path();
        let name = eintrag.file_name();
        if name.to_string_lossy().starts_with('.') {
            continue;
        }
        if pfad.is_dir() {
            sammle_markdown(wurzel, &pfad, raus)?;
        } else if pfad.extension().is_some_and(|e| e == "md") {
            if let Ok(relativ) = pfad.strip_prefix(wurzel) {
                let inhalt = std::fs::read_to_string(&pfad).unwrap_or_default();
                raus.push((relativ.to_string_lossy().into_owned(), inhalt));
            }
        }
    }
    Ok(())
}

/// Tägliche Selbstreflexion: aktualisiert das Selbstmodell und die
/// Provider-Statistik. Wird von der GUI als Hintergrund-Task aufgerufen,
/// alle 6 Stunden. Öffnet die DB bewusst frisch – kein Agent nötig.
pub fn idle_reflexion() {
    eprintln!("[idle] Selbstreflexion gestartet...");

    let gedaechtnis = match crate::memory::Gedaechtnis::standard() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("[idle] Gedächtnis nicht verfügbar: {e:#}");
            return;
        }
    };

    // 1. Provider-Statistik lesen und als Notiz speichern.
    if let Ok(statistik) = gedaechtnis.provider_statistik() {
        if !statistik.is_empty() {
            let mut notiz = String::from("Idle-Reflexion: Provider-Statistik:\n");
            for s in &statistik {
                notiz.push_str(&format!(
                    "{}: {:.0}% Erfolg bei {} Aufrufen (\u{00d8} {:.0}ms)\n",
                    s.provider, s.erfolgsquote * 100.0, s.anzahl, s.durchschnitt_ms
                ));
            }
            let _ = gedaechtnis.notizbuch_schreiben(&notiz);
        }
    }

    // 2. Gedächtnis-Statistik.
    if let Ok(anzahl) = gedaechtnis.anzahl() {
        let _ = gedaechtnis.notizbuch_schreiben(&format!(
            "Idle-Reflexion: {} Erinnerungen im Gedächtnis.", anzahl
        ));
    }

    eprintln!("[idle] Selbstreflexion abgeschlossen.");
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
    fn cosine_gleiche_vektoren_sind_1() {
        let v = vec![1.0, 2.0, 3.0];
        assert!((cosine_similarity(&v, &v) - 1.0).abs() < 0.001);
    }

    #[test]
    fn cosine_orthogonale_vektoren_sind_0() {
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        assert!((cosine_similarity(&a, &b) - 0.0).abs() < 0.001);
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

    /// Prüft den FTS5-Index direkt (nicht über `relevante()`, das bei
    /// leerem Index still auf die Keyword-Methode zurückfällt und damit
    /// eine defekte Trigger-Synchronisation verschleiern würde).
    #[test]
    fn fts_index_bleibt_mit_erinnerungen_synchron() {
        let pfad = temp();
        let g = Gedaechtnis::oeffnen(&pfad).unwrap();
        g.merken(ART_FAKT, "Cargo braucht pkgconfig zum Bauen", "test")
            .unwrap();
        {
            let verbindung = g.verbindung.lock().unwrap();
            let treffer: i64 = verbindung
                .query_row(
                    "SELECT count(*) FROM erinnerungen_fts WHERE erinnerungen_fts MATCH '\"pkgconfig\"'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(treffer, 1, "Neue Erinnerung muss sofort im FTS5-Index stehen");
        }
        // Löschen muss den Indexeintrag ebenfalls entfernen.
        {
            let verbindung = g.verbindung.lock().unwrap();
            verbindung.execute("DELETE FROM erinnerungen", []).unwrap();
            let treffer: i64 = verbindung
                .query_row(
                    "SELECT count(*) FROM erinnerungen_fts WHERE erinnerungen_fts MATCH '\"pkgconfig\"'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(treffer, 0, "Gelöschte Erinnerung darf nicht mehr auffindbar sein");
        }
        std::fs::remove_file(pfad).ok();
    }

    /// Eine bestehende gedaechtnis.db kann Zeilen enthalten, die vor der
    /// Trigger-Synchronisation eingefügt wurden. `oeffnen()` muss den
    /// Index dafür einmalig nachziehen ("rebuild"), ohne die Erinnerungen
    /// selbst anzufassen.
    #[test]
    fn bestehende_db_ohne_index_wird_beim_oeffnen_nachindiziert() {
        let pfad = temp();
        {
            // Simuliert eine alte DB: Tabelle direkt befüllt, am FTS5-Index vorbei.
            let verbindung = Connection::open(&pfad).unwrap();
            verbindung
                .execute_batch(
                    "CREATE TABLE erinnerungen (
                        id       INTEGER PRIMARY KEY,
                        art      TEXT NOT NULL,
                        inhalt   TEXT NOT NULL UNIQUE,
                        quelle   TEXT,
                        erstellt TEXT NOT NULL DEFAULT (datetime('now','localtime'))
                    );",
                )
                .unwrap();
            verbindung
                .execute(
                    "INSERT INTO erinnerungen (art, inhalt) VALUES (?1, ?2)",
                    params![ART_FAKT, "Alte Erinnerung ohne FTS5-Indexeintrag"],
                )
                .unwrap();
        }
        let g = Gedaechtnis::oeffnen(&pfad).unwrap();
        assert_eq!(g.anzahl().unwrap(), 1, "oeffnen() darf keine Zeilen verlieren");
        let treffer = g.relevante("Alte Erinnerung", 5).unwrap();
        assert!(
            treffer.iter().any(|e| e.inhalt.contains("Alte Erinnerung")),
            "Nachträglich indizierte Alt-Erinnerung muss auffindbar sein"
        );
        std::fs::remove_file(pfad).ok();
    }

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

    #[test]
    fn notizbuch_schreiben_und_lesen() {
        let pfad = temp();
        let g = Gedaechtnis::oeffnen(&pfad).unwrap();
        g.notizbuch_schreiben("Notiz 1").unwrap();
        g.notizbuch_schreiben("Notiz 2").unwrap();
        let notizen = g.notizbuch_lesen().unwrap();
        assert_eq!(notizen.len(), 2);
        g.notizbuch_leeren().unwrap();
        assert!(g.notizbuch_lesen().unwrap().is_empty());
        std::fs::remove_file(pfad).ok();
    }

    #[test]
    fn provider_statistik_rechnet_erfolgsquote_und_durchschnitt() {
        let pfad = temp();
        let g = Gedaechtnis::oeffnen(&pfad).unwrap();
        g.provider_aufruf_protokollieren("hyper", true, 1000)
            .unwrap();
        g.provider_aufruf_protokollieren("hyper", true, 2000)
            .unwrap();
        g.provider_aufruf_protokollieren("hyper", false, 3000)
            .unwrap();
        g.provider_aufruf_protokollieren("openrouter", true, 500)
            .unwrap();

        let statistik = g.provider_statistik().unwrap();
        let hyper = statistik.iter().find(|s| s.provider == "hyper").unwrap();
        assert_eq!(hyper.anzahl, 3);
        assert!((hyper.erfolgsquote - (2.0 / 3.0)).abs() < 0.001);
        assert!((hyper.durchschnitt_ms - 2000.0).abs() < 0.001);

        let openrouter = statistik
            .iter()
            .find(|s| s.provider == "openrouter")
            .unwrap();
        assert_eq!(openrouter.anzahl, 1);
        assert!((openrouter.erfolgsquote - 1.0).abs() < 0.001);

        std::fs::remove_file(pfad).ok();
    }
}