use super::Tool;
use crate::llm::ToolDefinition;
use crate::permissions::{Decision, PermissionManager};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::time::Duration;

/// Standard-Deckel für einen einzelnen Shell-Aufruf, in Sekunden.
///
/// Warum dieser Deckel existiert: Ein hängender Systemaufruf reicht, um den
/// gesamten Agenten endlos zu blockieren. Genau das ist am 2026-08-26
/// passiert: Ein `find` über Jens' Home-Verzeichnis blieb in `~/Music` an
/// einem blockierten `readdir` hängen (CPU-Zeit des Prozesses: 0,01 s,
/// Zustand S - er wartete, rechnete nichts), und ohne Timeout wartete
/// `run_shell` einfach weiter. Der Telegram-Bot war daraufhin 50+ Minuten
/// stumm, weil er pro Nachricht seriell arbeitet.
///
/// Der Deckel gilt pro Befehl, nicht pro Auftrag: ein zweiter Werkzeug-
/// Aufruf bekommt sein eigenes Zeitbudget. Braucht ein Befehl länger
/// (z.B. eine DVD-Konvertierung), setzt das Modell den Parameter
/// `timeout_seconds` höher.
const STANDARD_TIMEOUT_SEKUNDEN: u64 = 300;

/// Maximale Ausgabegröße (stdout + stderr) in Bytes, die ein Shell-Befehl
/// liefern darf, bevor er hart beendet wird.
///
/// Warum dieser Deckel existiert: `wait_with_output()` sammelte die komplette
/// Ausgabe gepuffert im RAM - ohne jede Obergrenze. Ein Befehl wie `yes`,
/// `cat /dev/urandom` oder eine fehlerhafte Endlosschleife füllte so den
/// Speicher, bis der System-Timeout (Sekunden!) ihn stoppte; in der Zeit
/// konnten Gigabytes anfallen und den Mac mini in den Swap drücken. Der
/// Agent-Loop kürzt das Ergebnis zwar ohnehin auf MAX_ERGEBNIS_ZEICHEN - aber
/// das half nichts, weil der RAM schon beim Sammeln vollgelaufen war.
///
/// Der Deckel liegt bewusst deutlich über allem, was der Agent-Loop je an
/// das Modell weitergibt (MAX_ERGEBNIS_ZEICHEN = 8000 Zeichen): Wer die
/// volle Ausgabe braucht (z.B. `grep` über viele Dateien), bekommt sie; wer
/// unbeabsichtigt eine Datenflut loslässt, wird bei 1 MB gestoppt statt den
/// Speicher zu sprengen.
const MAX_AUSGABE_BYTES: usize = 1_000_000;

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
        if !tokens.contains(&"push") {
            continue;
        }
        let hat_force_flag = tokens.iter().any(|t| {
            *t == "-f"
                || t.starts_with("--force")
                // Kombinierte Kurzflags wie "-uf".
                || (t.starts_with('-') && !t.starts_with("--") && t.len() > 1 && t[1..].contains('f'))
                // Git-Refspec mit +: "git push origin +main" (force-push, kein normaler Push)
                || t.starts_with('+')
        });
        if hat_force_flag {
            return true;
        }
    }
    false
}

/// Findet das erste Token eines Shell-Befehls, das in einem gesperrten
/// (Deny) oder sensiblen (Ask) Verzeichnis liegt.
///
/// Die Lücke, die das schließt: `deny_paths` ist die einzige echte Sperre
/// von Famulus - aber `run_shell` lief komplett an ihr vorbei. Ein Modell
/// konnte also jedes Verbot über einen Shell-Befehl umgehen: `rm -rf
/// ~/wichtiges` nach dem Motto "das ist nur read_file, das gilt für mich
/// nicht". Seit diesem Check gilt `deny_paths` auch für Shell-Kommandos -
/// ist ein Pfad als Token im Befehl erkennbar, wird vor der Ausführung
/// geblockt. Die Sensible-Pfad-Prüfung (~/.ssh etc., check_ask) läuft
/// natürlich auch - die gilt unabhängig von deny_paths immer.
///
/// Bewusst NICHT wasserdicht: die Shell ist Turing-vollständig, jede
/// Pfadprüfung auf Token-Ebene lässt sich durch Expansion, Variablenspiel
/// oder `eval` umgehen. Gedeckt sind aber die naheliegenden Formen: direkte
/// Pfade, `~` (löst `resolve_for_check` selbst auf) und `$HOME/...` (hier
/// manuell expandiert - `$HOME` ist der eine Pfad, der wirklich wehtut).
///
/// Rückgabe: `(token, verboten)` - `verboten = true` ist ein Deny (harter
/// Abbruch), `false` ein Ask (Rückfrage vor Ausführung), analog zur
/// Unterscheidung in `permissions.rs`.
fn gesperrter_pfad_token(command: &str, permissions: &PermissionManager) -> Option<(String, bool)> {
    let gesperrt = permissions.gesperrte_pfade();

    for token in command.split_whitespace() {
        // Options-Flags und reine Zuweisungen (FOO=bar) sind keine Pfade.
        if token.starts_with('-') || (token.contains('=') && !token.contains('/')) {
            continue;
        }
        let bereinigt = token.trim_matches(|c| matches!(c, '"' | '\'' | '`' | '(' | ')' | ';'));
        if bereinigt.is_empty() {
            continue;
        }

        // `$HOME/...` kommt aus der Shell-Expansion als ein Token an -
        // ohne manuelle Auflösung liefe es an der Pfadprüfung vorbei.
        let pfad: PathBuf = if let Some(rest) = bereinigt.strip_prefix("$HOME") {
            match dirs::home_dir() {
                Some(home) => {
                    let mut p = home;
                    if !rest.is_empty() {
                        p.push(rest.trim_start_matches('/'));
                    }
                    p
                }
                None => PathBuf::from(bereinigt),
            }
        } else {
            PathBuf::from(bereinigt)
        };

        // Deny-Check nur, wenn überhaupt etwas gesperrt ist; der Ask-Check
        // gilt immer.
        if !gesperrt.is_empty() && permissions.check_path(&pfad) == Decision::Deny {
            return Some((bereinigt.to_string(), true));
        }
        if permissions.check_ask(&pfad) == Decision::Ask {
            return Some((bereinigt.to_string(), false));
        }
    }
    None
}

pub struct ShellTool;

#[async_trait]
impl Tool for ShellTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "run_shell".into(),
            description: "Führt einen Shell-Befehl aus und gibt stdout/stderr zurück. Läuft höchstens `timeout_seconds` Sekunden (Standard 300) - dann wird die gesamte Prozessgruppe hart beendet. Für bewusst lange Befehle (z.B. Videokonvertierung) `timeout_seconds` entsprechend hoch setzen.".into(),
            parameters_schema: json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "Der auszuführende Shell-Befehl"
                    },
                    "timeout_seconds": {
                        "type": "integer",
                        "description": "Maximale Laufzeit in Sekunden. Standard 300; für lange Befehle (z.B. HandBrake-Konvertierung) höher setzen.",
                        "default": STANDARD_TIMEOUT_SEKUNDEN
                    }
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

        // `deny_paths` gilt auch für Shell-Kommandos - vorher lief jeder
        // Befehl ungefiltert und hob jede Sperre auf. Deny = harter Abbruch
        // (Fehler, das Modell kann nicht "nachverhandeln"), Ask = Rückfrage
        // als Text, damit das Modell frage_nutzer aufrufen kann.
        if let Some((token, verboten)) = gesperrter_pfad_token(command, permissions) {
            if verboten {
                anyhow::bail!(
                    "ZUGRIFF VERWEIGERT: Der Befehl enthält den gesperrten Pfad '{token}'. \
                     Der Pfad steht in deny_paths und darf weder direkt noch über \
                     Shell-Befehle angefasst werden."
                );
            }
            return Ok(format!(
                "RÜCKFRAGE ERFORDERLICH: Der Befehl enthält den sensiblen Pfad '{token}' \
                 (~/.ssh, ~/.gnupg, ~/.aws oder ~/.password-store). Frage den Nutzer vor \
                 der Ausführung um Erlaubnis und führe den Befehl erst aus, wenn er \
                 ausdrücklich zustimmt."
            ));
        }

        // Ungültige oder fehlende Angabe = Standard. Ein Modell, das hier
        // Unsinn einträgt, soll nicht den ganzen Lauf sprengen.
        let timeout_sekunden = args["timeout_seconds"]
            .as_u64()
            .filter(|s| *s > 0)
            .unwrap_or(STANDARD_TIMEOUT_SEKUNDEN);

        // `process_group(0)` + `pre_exec`: Der Befehl bekommt seine eigene
        // Prozessgruppe. Das ist die Voraussetzung dafür, dass der Timeout
        // auch die Enkel trifft: `sh -c find ...` startet `find` als Kind
        // von `sh` - nur ein Kill der ganzen Gruppe (-PID) erwischt beide.
        let mut cmd = tokio::process::Command::new("sh");
        cmd.arg("-c").arg(command);
        // Tokio erbt standardmäßig die stdio-Handles des Elternprozesses -
        // dann wäre die Ausgabe hier leer (beobachtet im Test: exit 0, aber
        // kein stdout). Explizit Pipes ziehen, wir lesen sie gestreamt und
        // gedeckelt ein (siehe unten).
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());
        cmd.kill_on_drop(true);
        cmd.process_group(0);
        #[cfg(unix)]
        unsafe {
            cmd.pre_exec(|| {
                // Eigene Prozessgruppe: setpgid(0,0) macht den Kind-Prozess
                // zum Gruppenleiter. Fehlschlagen lassen wäre fatal - dann
                // liefe der Befehl in Famulus' eigener Gruppe, und der
                // Timeout-Kill würde Famulus selbst erschießen.
                if libc::setpgid(0, 0) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }

        let mut kind = cmd.spawn()?;
        let pid = kind.id().expect("frisch gestarteter Prozess hat eine PID");

        // Gestreamtes, gedeckeltes Einsammeln statt wait_with_output():
        // wait_with_output() pufferte stdout+stderr unbegrenzt im RAM (siehe
        // Kommentar an MAX_AUSGABE_BYTES). Hier lesen wir beide Pipes
        // parallel in je einen gedeckelten Puffer; sobald die Gesamtmenge
        // das Limit überschreitet, killen wir die Prozessgruppe sofort
        // (bail!) statt weiter Speicher zu füllen. Parallel lesen ist
        // nötig: füllt der Befehl stderr, während wir nur stdout lesen,
        // blockiert er am vollen Pipe-Puffer (Deadlock).
        let mut stdout_pipe = kind.stdout.take().expect("stdout wurde gepiped");
        let mut stderr_pipe = kind.stderr.take().expect("stderr wurde gepiped");

        // Die Zukunft, die stdout+stderr gedeckelt einliest. Läuft unter
        // dem Zeit-Timeout: es beendet sich entweder normal (Prozess fertig)
        // oder mit dem Ausgabe-Fehler (Limit überschritten).
        let einsammeln = async {
            use tokio::io::AsyncReadExt;
            let mut stdout_buf: Vec<u8> = Vec::new();
            let mut stderr_buf: Vec<u8> = Vec::new();
            let mut gesamt = 0usize;
            let mut stdout_fertig = false;
            let mut stderr_fertig = false;
            // Zwei getrennte Lese-Puffer: select! leiht beide Zweige gleichzeitig,
            // ein gemeinsamer Puffer wäre eine doppelte &mut-Leihe (E0499).
            let mut stueck_out = [0u8; 8192];
            let mut stueck_err = [0u8; 8192];

            // Beide Pipes abwechselnd lesen, bis beide geschlossen sind
            // (Prozess beendet) oder das Limit reißt.
            while !stdout_fertig || !stderr_fertig {
                let gelesen = tokio::select! {
                    r = stdout_pipe.read(&mut stueck_out), if !stdout_fertig => {
                        let n = r?;
                        if n == 0 { stdout_fertig = true; continue; }
                        stdout_buf.extend_from_slice(&stueck_out[..n]);
                        n
                    }
                    r = stderr_pipe.read(&mut stueck_err), if !stderr_fertig => {
                        let n = r?;
                        if n == 0 { stderr_fertig = true; continue; }
                        stderr_buf.extend_from_slice(&stueck_err[..n]);
                        n
                    }
                };
                gesamt += gelesen;
                if gesamt > MAX_AUSGABE_BYTES {
                    // Limit gerissen: Prozessgruppe hart beenden (gleiche
                    // Begründung wie beim Zeit-Timeout unten) und mit einem
                    // klaren Fehler aussteigen - KEIN Weiterlesen, sonst
                    // läuft der RAM trotz Deckel weiter voll.
                    unsafe {
                        libc::kill(-(pid as i32), libc::SIGKILL);
                    }
                    anyhow::bail!(
                        "AUSGABE-LIMIT ÜBERSCHRITTEN: Der Befehl hat mehr als {MAX_AUSGABE_BYTES} \
                         Bytes auf stdout/stderr geschrieben und wurde hart beendet \
                         (komplette Prozessgruppe, PID {pid}). Das deutet auf eine Datenflut \
                         hin (Endlosschleife, `yes`, sehr großer Dump). Die Ausgabe wurde \
                         verworfen, um den Arbeitsspeicher zu schützen. Wer die volle Ausgabe \
                         braucht, soll sie in eine Datei umleiten und gezielt lesen."
                    );
                }
            }
            Ok((stdout_buf, stderr_buf))
        };

        let ergebnis = tokio::time::timeout(Duration::from_secs(timeout_sekunden), einsammeln).await;

        let (stdout_bytes, stderr_bytes) = match ergebnis {
            Ok(Ok(puffer)) => puffer,
            Ok(Err(e)) => {
                // Entweder ein echter I/O-Fehler beim Lesen oder unser
                // Ausgabe-Limit (dann wurde die Gruppe bereits gekillt).
                // Das Kind aufräumen, damit kein Zombie bleibt.
                let _ = kind.kill().await;
                return Err(e);
            }
            Err(_) => {
                // Zeitüberschreitung. Erst die ganze Prozessgruppe hart
                // beenden (negatives PID-Signal), dann das Kind einziehen,
                // damit kein Zombie übrig bleibt. SIGKILL ist Absicht: Ein
                // Prozess, der so lange hängt, bekommt keine Chance, sich
                // höflich zu verabschieden - er blockiert den gesamten Bot.
                unsafe {
                    libc::kill(-(pid as i32), libc::SIGKILL);
                }
                // `kind` wird fallengelassen - `kill_on_drop(true)` erledigt
                // den Rest, ein eigenes wait() ist hier nicht mehr möglich
                // und nicht mehr nötig. Zombies räumt Tokio beim Drop weg.
                anyhow::bail!(
                    "Zeitüberschreitung: Der Befehl wurde nach {timeout_sekunden}s hart beendet \
                     (komplette Prozessgruppe, PID {pid}). Mögliche Ursachen: der Befehl \
                     wartet auf Eingabe, hängt in einem blockierten Dateisystem oder läuft \
                     einfach zu lange. Für bewusst lange Befehle beim nächsten Versuch \
                     timeout_seconds höher setzen."
                );
            }
        };

        // Exit-Status einholen: die Pipes sind geschlossen (Prozess
        // beendet), wait() liefert sofort.
        let status = kind.wait().await?;

        let stdout = String::from_utf8_lossy(&stdout_bytes);
        let stderr = String::from_utf8_lossy(&stderr_bytes);

        Ok(format!(
            "exit code: {}\n--- stdout ---\n{}\n--- stderr ---\n{}",
            status.code().unwrap_or(-1),
            stdout,
            stderr
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn config_mit(deny: Vec<String>) -> Config {
        Config {
            provider: "hyper".to_string(),
            model: None,
            base_url: None,
            api_key_env: None,
            temperature: None,
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

    #[tokio::test]
    async fn execute_blockt_deny_pfad_ohne_auszufuehren() {
        let root = std::env::temp_dir().join(format!("famulus_test_deny_{}", std::process::id()));
        std::fs::create_dir_all(root.join("gesperrt")).unwrap();
        std::fs::create_dir_all(root.join("projekt")).unwrap();
        std::fs::write(root.join("gesperrt/geheim.txt"), b"geheim").unwrap();
        let ziel = root.join("projekt/kopie.txt");

        let manager = PermissionManager::new(&config_mit(vec![
            root.join("gesperrt").to_string_lossy().to_string()
        ]));
        let ergebnis = ShellTool
            .execute(
                json!({ "command": format!("cp '{}' '{}'", root.join("gesperrt/geheim.txt").display(), ziel.display()) }),
                &manager,
            )
            .await;
        assert!(ergebnis.is_err());
        assert!(ergebnis.unwrap_err().to_string().contains("ZUGRIFF VERWEIGERT"));
        assert!(!ziel.exists(), "Befehl darf im Deny-Fall nie ausgeführt werden");
    }

    #[tokio::test]
    async fn execute_fragt_bei_sensiblem_pfad_zurueck() {
        let manager = PermissionManager::new(&config_mit(vec![]));
        let ergebnis = ShellTool
            .execute(json!({ "command": "cat ~/.ssh/id_ed25519" }), &manager)
            .await
            .unwrap();
        assert!(ergebnis.contains("RÜCKFRAGE ERFORDERLICH"));
    }

    // ── Timeout-Tests (brauchen die tokio-Laufzeit) ──────────────────

    #[tokio::test]
    async fn schneller_befehl_liefert_normales_ergebnis() {
        let out = ausfuehren("echo hallo", None).await.unwrap();
        assert!(out.contains("hallo"));
        assert!(out.contains("exit code: 0"));
    }

    #[tokio::test]
    async fn ausgabe_ueber_limit_wird_abgebrochen() {
        // `yes` produziert endlos "y"-Zeilen - ohne Deckel würde das den
        // RAM füllen bis zum Zeit-Timeout. Mit MAX_AUSGABE_BYTES muss der
        // Befehl deutlich vorher mit dem Limit-Fehler abbrechen.
        let ergebnis = ausfuehren("yes", Some(60)).await;
        assert!(ergebnis.is_err());
        let msg = ergebnis.unwrap_err().to_string();
        assert!(
            msg.contains("AUSGABE-LIMIT"),
            "Erwartete Ausgabe-Limit-Fehler, bekam: {msg}"
        );
    }

    #[tokio::test]
    async fn normale_ausgabe_bleibt_unberuehrt() {
        // Eine Ausgabe weit unter dem Limit darf nicht gekürzt werden.
        let out = ausfuehren("for i in 1 2 3; do echo zeile$i; done", None).await.unwrap();
        assert!(out.contains("zeile1"));
        assert!(out.contains("zeile3"));
        assert!(!out.contains("AUSGABE-LIMIT"));
    }

    async fn ausfuehren(command: &str, timeout: Option<u64>) -> anyhow::Result<String> {
        let config = config_mit(vec![]);
        let permissions = PermissionManager::new(&config);
        let mut args = json!({ "command": command });
        if let Some(t) = timeout {
            args["timeout_seconds"] = json!(t);
        }
        ShellTool.execute(args, &permissions).await
    }

    #[test]
    fn erkennt_klassisches_force_push() {
        assert!(ist_force_push("git push --force"));
        assert!(ist_force_push("git push -f"));
        assert!(ist_force_push("git push --force-with-lease"));
    }

    #[test]
    fn erkennt_force_push_mit_zwischenliegenden_argumenten() {
        assert!(ist_force_push("git push origin -f"));
        assert!(ist_force_push("git push -f origin main"));
        assert!(ist_force_push("git push --force origin main"));
        assert!(ist_force_push("git  push   -f"));
    }

    #[test]
    fn erkennt_kombinierte_kurzflags() {
        assert!(ist_force_push("git push -uf origin main"));
    }

    #[test]
    fn erkennt_plus_syntax_force_push() {
        assert!(ist_force_push("git push origin +main"));
        assert!(ist_force_push("git push origin +v0.9.2:refs/tags/v0.9.2"));
        assert!(ist_force_push("git push origin +HEAD:main"));
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
