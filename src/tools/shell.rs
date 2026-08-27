use super::Tool;
use crate::llm::ToolDefinition;
use crate::permissions::PermissionManager;
use async_trait::async_trait;
use serde_json::{json, Value};
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
        if !tokens.iter().any(|t| *t == "push") {
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

pub struct ShellTool;

#[async_trait]
impl Tool for ShellTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "run_shell".to_string(),
            description: "Führt einen Shell-Befehl aus und gibt stdout/stderr zurück. Läuft höchstens `timeout_seconds` Sekunden (Standard 300) - dann wird die gesamte Prozessgruppe hart beendet. Für bewusst lange Befehle (z.B. Videokonvertierung) `timeout_seconds` entsprechend hoch setzen.".to_string(),
            parameters_schema: json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "Der auszuführende Shell-Befehl" },
                    "timeout_seconds": { "type": "integer", "description": "Maximale Laufzeit in Sekunden. Standard 300; für lange Befehle (z.B. HandBrake-Konvertierung) höher setzen.", "default": 300 }
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

        // Kein Pfad, keine Prüfung: Shell-Befehle laufen ungefiltert.
        // `deny_paths` greift hier NICHT - wer den Zugriff auf ein
        // Verzeichnis wirklich verhindern will, kommt an run_shell nicht
        // vorbei. Steht so in permissions.rs, ist so gewollt.
        let _ = permissions;

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
        // kein stdout). Explizit Pipes ziehen, wait_with_output() sammelt sie ein.
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

        let kind = cmd.spawn()?;
        let pid = kind.id().expect("frisch gestarteter Prozess hat eine PID");

        let ergebnis = tokio::time::timeout(Duration::from_secs(timeout_sekunden), kind.wait_with_output()).await;

        let output = match ergebnis {
            Ok(ok) => ok?,
            Err(_) => {
                // Zeitüberschreitung. Erst die ganze Prozessgruppe hart
                // beenden (negatives PID-Signal), dann das Kind einziehen,
                // damit kein Zombie übrig bleibt. SIGKILL ist Absicht: Ein
                // Prozess, der so lange hängt, bekommt keine Chance, sich
                // höflich zu verabschieden - er blockiert den gesamten Bot.
                unsafe {
                    libc::kill(-(pid as i32), libc::SIGKILL);
                }
                // `kind` selbst ist in die Timeout-Future gezogen und wird
                // mit ihr fallengelassen - `kill_on_drop(true)` erledigt den
                // Rest, ein eigenes wait() ist hier nicht mehr möglich und
                // nicht mehr nötig. Zombies räumt Tokio beim Drop weg.
                anyhow::bail!(
                    "Zeitüberschreitung: Der Befehl wurde nach {timeout_sekunden}s hart beendet \
                     (komplette Prozessgruppe, PID {pid}). Mögliche Ursachen: der Befehl \
                     wartet auf Eingabe, hängt in einem blockierten Dateisystem oder läuft \
                     einfach zu lange. Für bewusst lange Befehle beim nächsten Versuch \
                     timeout_seconds höher setzen."
                );
            }
        };

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        Ok(format!(
            "exit code: {}\n--- stdout ---\n{}\n--- stderr ---\n{}",
            output.status.code().unwrap_or(-1),
            stdout,
            stderr
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn ausfuehren(command: &str, timeout: Option<u64>) -> anyhow::Result<String> {
        // Minimale, aber echte Config über den TOML-Parser - `deny_paths`
        // bleibt leer, das ist für Shell-Befehle ohnehin ohne Bedeutung.
        let config: crate::config::Config =
            toml::from_str("provider = \"hyper\"").expect("Test-Konfiguration");
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

    /// Der Kernfall: Flag und `push` in anderer Reihenfolge oder mit
    /// zusätzlichen Argumenten dazwischen. Die alte Substring-Prüfung auf
    /// `"push -f"` bzw. `"push --force"` ließ genau das durch.
    #[test]
    fn erkennt_force_push_mit_zwischenliegenden_argumenten() {
        assert!(ist_force_push("git push origin -f"));
        assert!(ist_force_push("git push -f origin main"));
        assert!(ist_force_push("git push --force origin main"));
        assert!(ist_force_push("git  push   -f")); // mehrfache Leerzeichen
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

    // ── Timeout-Tests (brauchen die tokio-Laufzeit) ──────────────────

    #[tokio::test]
    async fn schneller_befehl_liefert_normales_ergebnis() {
        let out = ausfuehren("echo hallo", None).await.unwrap();
        assert!(out.contains("hallo"));
        assert!(out.contains("exit code: 0"));
    }

    /// Der Hänger-Fall vom 2026-08-26, klein nachgestellt: ein Befehl, der
    /// nie fertig wird. Vor dem Fix wartete `execute()` hier für immer.
    #[tokio::test]
    async fn haengender_befehl_wird_abgebrochen() {
        let start = std::time::Instant::now();
        let erg = ausfuehren("sleep 30", Some(1)).await;
        assert!(erg.is_err(), "hängender Befehl muss abbrechen");
        let meldung = format!("{:#}", erg.unwrap_err());
        assert!(meldung.contains("Zeitüberschreitung"));
        assert!(
            start.elapsed() < std::time::Duration::from_secs(10),
            "Abbruch muss kurz nach Ablauf des Timeouts kommen"
        );
    }

    /// Ein Befehl, der knapp unter dem Limit bleibt, muss ganz normal
    /// durchlaufen - der Timeout darf nicht zu früh zuschlagen.
    #[tokio::test]
    async fn befehl_unterhalb_des_limits_laeuft_durch() {
        let erg = ausfuehren("sleep 1; echo fertig", Some(10)).await;
        let out = erg.expect("Befehl innerhalb des Limits darf nicht abbrechen");
        assert!(out.contains("fertig"));
    }

    /// Der eigentliche Zweck der Prozessgruppe: `sh` startet ein Kind, das
    /// ohne Gruppen-Kill als Waise weiterliefe.
    #[tokio::test]
    async fn timeout_toetet_auch_kindprozesse() {
        let erg = ausfuehren("sleep 300", Some(1)).await;
        assert!(erg.is_err());
        tokio::time::sleep(Duration::from_millis(300)).await;
        let suche = std::process::Command::new("sh")
            .arg("-c")
            .arg("ps -ax -o command= | grep '[s]leep 300' | wc -l")
            .output()
            .unwrap();
        let anzahl: usize = String::from_utf8_lossy(&suche.stdout).trim().parse().unwrap_or(99);
        assert_eq!(anzahl, 0, "Der hängende Kindprozess muss mitgetötet werden");
    }
}
