# Changelog

## [1.2.0] – 2026-09-05

### Neu
- **Scroll-zu-unten / Chat letzter Eintrag**: Beim Öffnen der App und beim Wechsel in einen bestehenden Chat springt die Ansicht jetzt zuverlässig zum neuesten (untersten) Eintrag – gestaffelte Scroll-Auslösung, damit die Position auch bei asynchron geladenen Chats sicher unten landet.
- **Sichtbarer Aktivitäts-Indikator**: Ein rotierender Cursor/„Denkt…"-Text neben den Chat-Nachrichten zeigt an, wenn der Agent gerade einen Auftrag verarbeitet (gekoppelt an den echten Arbeitszustand des Stores), plus Abbrechen-Schaltfläche laufender Arbeitsläufe.
- **Famulus denkt von selbst**: Wecker-System (launchd) mit drei Ebenen – Kicktipp-Check/-Tippen (Mo+Mi 08:00, täglich 20:00), wöchentliche Mail-Aufräumaktion (So 07:00, auf den Famulus-Hub-Mail-Zweig umgebogen), monatlicher Vault-Cleanup (1. des Monats 06:00) und handlungsfähige Idle-Reflexion, die bei fälligen Aufgaben proaktiv aktiv wird.
- **Anruf-Skript** (`scripts/anrufen.sh`): auf Befehl sichtbar die Telefon-App mit einer Nummer öffnen (nur manuell, keine Automatik).

### Geändert
- **Icons aller Famulus-Apps im Phoenix-Stil**: Motive statt der bisherigen 2-Buchstaben-Monogramme (Famulus=KI-Geist, Games=Gamecontroller, Music=Achtelnote, Downloader=Download-Pfeil, Files=Ordner, Mail=Umschlag) – Anthrazit-Verlauf, runde Ecken, #f97316, Rendering über gemeinsame Icon-Basis.
- **Mail-Wecker auf Famulus Hub umgebogen**: Sonntags-Aufräumauftrag zeigt jetzt auf den Mail-Zweig des Famulus Hub statt auf das gelöschte eigenständige Famulus-Mail-Projekt.

### Behoben
- **IMAP/SMTP-Empfang hing**: `receiveMessage` lieferte bei TLS-Verbindungen keine eingehenden Daten; auf `receive()` umgestellt (per Wegwerf-Test gegen echten Server bewiesen).


## [1.1.0] – 2026-08-28

### Behoben
- **SQLite-Journal auf WAL umgestellt**: `PRAGMA journal_mode = WAL` statt
  `delete`. Bei den vielen parallelen Connections auf `gedaechtnis.db`
  (Router-Logging nach jedem Modellaufruf, History pro FFI-Aufruf,
  `idle_reflexion` im Hintergrund, Agent selbst) blockierte im
  delete-Journal ein schreibender Connection alle anderen komplett -
  bis `busy_timeout` (5s) griff. WAL lässt Leser weiterlesen, während
  ein Schreiber in sein WAL-File schreibt; nur Schreiber untereinander
  sind gesperrt. Zusätzlich: Crash-Sicherheit - im delete-Journal kann
  ein Absturz die Hauptdatei inkonsistent hinterlassen (Rollback-Journal
  liegt noch rum), WAL ist atomar commit-bar.
- **Stille Kontextkürzung sichtbar gemacht**: Wenn `nachrichten_kuerzen`
  ältere Nachrichten aus dem Kontext wirft, wird jetzt eine Hinweis-
  Nachricht in den Verlauf eingefügt ("X frühere Nachrichten wurden
  entfernt"). Vorher referenzierte das Modell stolz Dinge, die es nicht
  mehr sehen konnte - Jens bekam "Stimmt, das hatten wir schon"-Antworten
  auf nie gesagte Inhalte.
- **Notizbuch-Verlust bei fehlgeschlagener Konsolidierung**: Das
  Notizbuch wurde immer geleert, auch wenn der Konsolidierungs-LLM-Call
  fehlschlug oder unparsebares JSON lieferte. Jetzt wird erst geleert,
  wenn mindestens eine Erinnerung tatsächlich übernommen wurde.
- **Präferenzen: Relevanz statt reine Recency**: Bei 326 gespeicherten
  Präferenzen und einem Budget von 12 (Sub-Deckel 6) bekam Jens nur die
  6 jüngsten Präferenzen - egal ob sie zum Auftrag passten. Jetzt werden
  Präferenzen per FTS5-Match auf den Auftragstext vorsortiert (relevante
  zuerst), Recency als Tiebreaker. In `relevante()` UND `relevante_semantisch()`.
- **Reflexion bei Trivial-Aufträgen übersprungen**: "Wie spät ist es?"
  kostete zwei zusätzliche LLM-Calls (Notizbuch-Konsolidierung +
  Rückblick) für nichts Merkbares. Jetzt greift `ist_einfacher_auftrag`
  auch hier.

## [1.0.0] – 2026-08-28

### Neu
- **`frage_nutzer`-Werkzeug (Telegram)**: Multiple-Choice-Rückfragen mit
  2-4 anklickbaren Buttons (Inline-Keyboard) statt Fließtext. Bewusst
  nicht-blockierend gebaut - die Antwort kommt als eigene Nachricht
  herein, kein Warten in der Poll-Schleife.
- **`idle_reflexion` verdrahtet**: War geschrieben, aber nie aufgerufen
  (kein UDL-Eintrag, keine Swift-Seite). Jetzt per Timer alle 6 Stunden
  aus der Swift-Hülle aufgerufen, wie ursprünglich dokumentiert.
- **Chat-Zeilen**: Archivieren/Löschen jetzt über Hover-Icons direkt
  sichtbar (vorher nur per unentdeckbarem Rechtsklick-Menü), Löschen
  fragt jetzt vorher nach.
- **Modell-Dropdown**: alphabetisch sortiert.

### Behoben
- **Gedächtnis fraß sich selbst**: `relevante_semantisch()` ignorierte
  `max_erinnerungen` komplett, sobald mehr Präferenzen gespeichert waren
  als der Deckel erlaubt (live beobachtet: 309 Präferenzen gegen einen
  Deckel von 12 - alle 12 Plätze gingen an Präferenzen, kein Fakt/keine
  Lektion kam mehr durch). Jetzt zweifach gefixt: harter Gesamt-Deckel
  (`truncate`) plus Sub-Deckel, der Präferenzen auf höchstens die Hälfte
  des Budgets begrenzt.
- **Rückblick überverallgemeinerte einmalige Anweisungen**: "Für diesen
  einen Audit nichts ändern" wurde wiederholt als dauerhafte Präferenz
  gespeichert und überschwemmte spätere, ausdrücklich gegenteilige
  Aufträge. Rückblick-Prompt jetzt mit expliziter Warnung dagegen.
- **`art`-Tippfehler unsichtbar**: Varianten wie "präferenz" (Umlaut)
  landeten unnormiert in der DB und fielen aus jeder Präferenz-Suche
  raus. Jetzt normalisiert vor dem Speichern.
- **Mutex-Poison in `memory.rs`**: ein einzelner Panic während eines
  gehaltenen Locks hätte das Gedächtnis für den Rest des Prozesses
  unbenutzbar gemacht. Jetzt wie in `ffi.rs` mit `into_inner()`
  behandelt statt mit `.expect()`.
- **`check_ask` verankert jetzt an `$HOME`** statt loser Substring-Suche
  (`contains("/.ssh/")`) - übersah vorher den Ordner `~/.ssh` selbst und
  griff fälschlich bei harmlosen Pfaden wie `/tmp/projekt/.ssh/config`.
- **OpenRouter-Modellliste**: 39 nur per Batch-API erreichbare
  `:batch`-Varianten rausgefiltert - schlugen über den normalen
  chat/completions-Endpunkt immer fehl.

## [0.13.0] – 2026-08-28

### Entfernt
- **Tauri-GUI endgültig aus dem Repo**: Der `gui/`-Ordner ist archiviert
  (Google Drive) und gelöscht; der `[workspace]`-Abschnitt in `Cargo.toml`
  entfällt, `famulus-core` ist jetzt ein eigenständiges Package. Die
  Swift-Hülle (swift-app/) ist die einzige grafische Hülle.

### Neu
- **Datei-Upload in der Swift-Hülle**: Die Swift-Hülle hat jetzt wieder
  den 📎-Upload, der seit der Tauri-Migration fehlte. Ein Klick auf die
  Büroklammer öffnet den Dateidialog (nur Bilder, Mehrfachauswahl), die
  Anhänge erscheinen als Vorschau-Chips über der Eingabe und lassen sich
  einzeln entfernen. Angehängte Bilder gehen zusammen mit der Nachricht
  über die FFI-Grenze an den Kern (`verlauf_zu_nachrichten` →
  `Message::UserMitBild`), der sie an das Modell schickt. Bilder ohne
  Text werden als „Beschreibe diese Datei(en)." gesendet – exakt die
  Feature-Parität zum alten Tauri-GUI. Auch in der Chat-Historie werden
  Anhänge jetzt als Thumbnails angezeigt und in der History-Datenbank
  mitgespeichert.

### Behoben
- **Doppelte aktuelle Nachricht**: Beim Senden wurde die gerade
  angehängte Nachricht zusätzlich im Verlauf-JSON mitgeschickt und
  dadurch doppelt ans Modell übergeben. Jetzt wird sie wie in der
  Tauri-GUI (`slice(0,-1)`) aus dem Verlauf herausgenommen.

### Behoben (Kern, aus Vorarbeiten übernommen)
- **Renn-Fenster beim Stoppen**: `stoppe_auftrag()` prüfte früher mit
  `is_finished()`, ob der Task sein Abschluss-Ereignis schon selbst
  gesendet hatte – zwischen Check und Abort konnte der Task fertig
  werden, und die Hülle bekam „Fertig" UND „Abgebrochen" (leere Doppel-
  Nachricht). Jetzt entscheidet ein `terminiert`-Atomic in der neuen
  `TerminaleUi`, wer das terminale Ereignis senden darf – ohne
  Renn-Fenster.
- **Poisoned-Mutex-Panics**: Alle Zugriffe auf die FFI-Statics
  (`LAUFENDER_AUFTRAG`, `ZWISCHENFRAGE_KANAL`) nutzen jetzt
  `unwrap_or_else(|e| e.into_inner())` statt nacktem `unwrap()`.

### Geändert
- **Stabile Codesignatur für die App**: `build-app.sh` signiert jetzt
  mit dem echten Entwickler-Zertifikat statt Ad-hoc, damit macOS-
  Ordnerfreigaben (TCC) über Builds hinweg erhalten bleiben – derselbe
  Fix wie 64bd20d für die CLI-Binaries.
- Version-Bump auf 0.13.0 (neues Feature = Minor nach SemVer).
- `project.yml` und Header-Versionen auf 0.13.0 angeglichen.


## [0.12.1] – 2026-08-28

### Behoben
- **Stop-Button war tot**: `stoppe_auftrag()` in der FFI-Brücke hat nur
  `handle.abort()` gerufen – ein hart beendeter Tokio-Task kann aber
  selbst kein Abschluss-Ereignis mehr senden. Die Swift-Hülle blieb
  dadurch für immer im Beschäftigt-Zustand. Jetzt emittiert
  `stoppe_auftrag()` das `Abgebrochen`-Ereignis nach dem Abort selbst
  (dasselbe Muster wie das Tauri-GUI in `gui/src/lib.rs`); die
  Ereignis-Senke wird dafür im `LAUFENDER_AUFTRAG`-Static mit abgelegt.
- **Keine Zwischenfragen während eines laufenden Auftrags**: Der
  Senden-Button war bei `beschaeftigt` deaktiviert und der Store hat
  jeden Text abgelehnt – obwohl die FFI-Funktion `zwischenfrage(text)`
  längst existierte. Jetzt verhält sich die Hülle wie die
  Tauri-Referenz (`ui/index.html::sendeZwischenfrage`): Bei laufendem
  Auftrag sendet der Pfeil-Button eine Zwischenfrage an den laufenden
  Zug, der Platzhalter wechselt zu „Zwischenfrage stellen…".

### Geändert
- **Native Schriften statt Monospace**: Alle `design: .monospaced`-
  Definitionen der Swift-Hülle (17 Stellen) sind jetzt SF Pro – die
  native macOS-Systemschrift, modern und edel statt Terminal-Look.
  Auch das Tauri-Frontend (`ui/index.html`) nutzt jetzt den System-
  Font-Stack (`-apple-system`) statt "SF Mono".

## [0.12.0] – 2026-08-28


## [0.12.0] – 2026-08-28

### Hinzugefügt
- **Preset-Panel in der rechten Seitenleiste** (wie in der Tauri-GUI):
  Ein Preset-Dropdown über dem Archiv, darunter eine editierbare
  Prompt-Textarea und die beiden Aktionen Speichern (✓) und Löschen
  (✕). Das letzte verbleibende Preset lässt sich nicht löschen –
  der Kern verweigert das bereits, der Löschen-Button ist dann
  deaktiviert.
- **Store: `presetSpeichern(_:)` und `presetLoeschen()`**: Die beiden
  FFI-Funktionen `presetsSpeichern`/`presetsLoeschen` sind jetzt
  angebunden (Aktivieren und Laden existierten schon).

## [0.11.0] – 2026-08-28

### Geändert
- **Swift-Hülle im Phoenix-Style**: Die native macOS-Oberfläche
  (`swift-app/`) nutzt jetzt dieselbe Design-DNA wie Tankmonitor und
  die Webseite: dunkle Grundfläche #1e1e1e, weicher Orange-Fade oben,
  Akzent #f97316 (statt Braun/Schwarz-Verlauf).
- **Chat-Übersicht links/rechts**: Was Jens schreibt, steht rechts
  (orange getönt, Rand orange), was Famulus antwortet, links (graue
  Fläche). Fehler rot markiert. Entspricht der Tauri-Referenz-Logik.
- **Archiv-Sidebar rechts, ein-/ausklappbar**: Über den rechten
  Sidebar-Button in der Kopfzeile auf-/zuklappbar. Zeigt alle
  archivierten Chats; ein Klick holt den Chat zurück in die aktive
  Liste und öffnet ihn.

### Hinzugefügt
- **FFI: `credits_fuer_provider(provider)`**: Guthaben für einen
  explizit gewählten Provider. Nötig, weil `credits()` immer den in
  der Config gespeicherten Provider liest – der Dropdown-Wechsel
  allein ändert die Config nicht.
- **FFI: `aktiver_provider()`**: Liefert den in famulus.toml
  gespeicherten Provider. Braucht die Swift-Hülle, um beim Start das
  Provider-Dropdown und die Credits korrekt zu initialisieren.

### Behoben
- **Credits zeigten immer den Config-Provider**: Das Credits-Feld in
  der Swift-Hülle rief nur `credits()` auf – wer im Dropdown den
  Provider wechselte (z. B. OpenRouter), sah weiter die Credits des
  in der Config gespeicherten Providers. Jetzt fragt die Hülle
  provider-spezifisch (`creditsFuerProvider`) und aktualisiert beim
  Dropdown-Wechsel sofort.

### Geändert (Version)
- Version-Bump auf 0.11.0 (Minor: neue FFI-Funktionen + UI-Features).

## [0.10.0] – 2026-08-28


## [0.10.0] – 2026-08-28

### Hinzugefügt
- **Native SwiftUI-Hülle für macOS** (`swift-app/`): Famulus hat jetzt
  eine native Swift-Oberfläche nach dem Muster von Famulus Games. Der
  Rust-Kern bleibt derselbe; angebunden über eine UniFFI-Brücke
  (`src/ffi.rs` + `src/ffi.udl`, staticlib). Die Tauri-GUI bleibt
  parallel bestehen, bis die Swift-Hülle gleichwertig ist.
- **FFI-Schicht**: `starte_auftrag` mit Callback-Streaming, Modell-/
  Provider-Wahl, Credits, Presets, History und TOML-Schalter – alles,
  was vorher nur die Tauri-GUI konnte, liegt jetzt im Kern und steht
  jeder Hülle zur Verfügung.
- Build-Pipeline: `scripts/build-ffi.sh` (staticlib + Bindings),
  `scripts/build-app.sh` (Xcode-Build + atomarer Swap nach
  /Applications), `swift-app/project.yml` (XcodeGen), App-Icon aus
  dem Tauri-Icon generiert.

### Behoben
- **History-Duplikate in der Swift-Hülle**: `speichern()` rief immer
  `historySpeichern` (INSERT) auf – bei jeder Agent-Antwort wäre eine
  neue DB-Zeile entstanden. Die FFI hat jetzt `history_aktualisieren`
  (UPDATE), und der Store trennt: bestehender Chat → UPDATE, neuer →
  INSERT. Keine Duplikate mehr in `gedaechtnis.db`.
- **`erstellt`-Feld falsch geparst**: Die DB liefert das Datum als
  SQLite-Text (`"2026-08-28 09:35:00"`), der Store las es als `Int64`
  → immer 0. Jetzt `DateFormatter` wie `new Date()` im Tauri-Frontend.

### Geändert
- Version-Bump auf 0.10.0 (Minor: neues Feature). Kern + FFI-Brücke.

## [0.9.6] – 2026-08-27

### Geändert
- **Famulus-App-Icon auf den Stil von Famulus Games gebracht**: Das
  Icon nutzt jetzt dieselbe Marken-DNA wie das FG-Icon – Braun→Schwarz-
  Verlauf (1:1 aus dem FG-Icon gesampelt, Abweichung 0,0), abgerundete
  Ecken mit transparenten Rändern, orangenes „F" in Menlo Bold (#F86E27)
  mit identischer Buchstabenhöhe. Gelb/schwarzes Eck-Icon aus 0.9.4 ist
  abgelöst. Alle Famulus-Produkte haben damit denselben visuellen Stil.
- **Neues Skript `scripts/generate-icon.py`**: Deterministischer
  Icon-Generator (PIL + Menlo.ttc + FG-Referenz), erzeugt macOS
  (.png/.icns/.ico + Store-Kacheln), iOS-AppIcon-Set und Android-
  Adaptive-Icons. Kein externes Grafik-Werkzeug nötig.
- iOS-AppIcon-Set (iPad/iPhone) und Android-Varianten auf das neue
  Icon umgestellt.

## [0.9.5] – 2026-08-27

### Geändert
- **Durchgängiger Farbverlauf Braun → Schwarz** in der gesamten GUI
  (Mac + iPad + iPhone): Die App zeichnet jetzt einen vertikalen
  Verlauf vom warmen Braun oben (`#211B16`) bis reinem Schwarz unten.
  Die Flächen darüber sind teiltransparent eingefärbt, damit der
  Verlauf als Linie durch die ganze App läuft: Toolbar oben bleibt
  braun, Sidebars halbtransparent, Statusbar unten fast schwarz.
  Jens' Wunsch: „oben das Braun sehr gut, nach unten bis auf Schwarz".
- **Alle Famulus-Produkte auf denselben Style gebracht**: Famulus
  Games (SwiftUI) zieht auf die warme Palette um (Braun/Creme/
  Orange-Akzent statt Schwarz/Gelb) – beide Apps nutzen jetzt
  identische Design-Tokens inkl. desselben Verlaufs.
- **App-Icon für Famulus Games**: „FG" in Menlo Bold und Orange auf
  Braun-Schwarz-Verlauf, gleiche DNA wie das Famulus-Icon („F",
  Gelb). Asset-Catalog neu angelegt, project.yml zieht mit.

## [0.9.4] – 2026-08-27

### Geändert
- **Markenfarbe finalisiert, dann Warm-Theme**: Zuerst Magenta→Gelb
  endgültig abgeschlossen (die installierte App lief noch mit #ff69b4
  aus einem alten Build; App-Icon per Hue-Shift umgefärbt, drei
  veraltete `.bak`-Dateien entfernt). Anschließend neues Aussehen nach
  Screenshot-Vorgabe von Jens: warmes Dunkelbraun statt Schwarz
  (`#211B16` bis `#1D1712`), Creme-Text (`#EDE3D5`) und Orange-Akzent
  `#F86E27` (Hover `#FF8A4A`) statt Gelb im GUI. Gelb `#FFC53D` bleibt
  die Markenfarbe für Icons und künftige Produkte – das Famulus-GUI
  trägt jetzt den Warm-Look. Gebaut und installiert auf Mac, iPad und
  iPhone.


## [0.9.3] – 2026-08-26

### Behoben
- **Force-Push-Erkennung ließ `+`-Syntax durch**: `git push origin
  +main` (Git-Refspec-Syntax für Force-Push) wurde nicht erkannt,
  weil `ist_force_push()` nur `-f`, `--force*` und kombinierte
  Kurzflags prüfte. Jetzt erkennt die Funktion auch Token, die mit
  `+` beginnen (z. B. `+main`, `+v0.9.2:refs/tags/v0.9.2`,
  `+HEAD:main`). Neuer Test `erkennt_plus_syntax_force_push` mit
  drei Varianten.
- **Unused-Import-Warnung in `router.rs`**: `use crate::memory::Gedaechtnis`
  war nur im `#[cfg(not(test))]`-Pfad tatsächlich genutzt, erzeugte
  aber im Test-Build eine Compiler-Warnung. Jetzt korrekt hinter
  `#[cfg(not(test))]` importiert.

### Neu getestet
- 1 neuer Test für `+`-Syntax-Force-Push (3 Assertions: einfacher
  Branch, Tag-Refspec, HEAD-Refspec). Gesamt: 60 Tests, 0 Warnungen.
## [0.9.2] – 2026-08-26

### Geändert
- **System-Prompt: uneingeschränkter Zugriff zweifach verankert**.
  Der Theory-of-Mind-Block („Falls du dir unsicher bist, frag
  Jens") wurde ersetzt durch eine klare Anweisung: „Jens hat dir
  uneingeschränkten Zugriff erteilt. Frage nie nach Berechtigungen."
  Die drei festen Regeln bekamen einen Vorspann mit derselben
  Anweisung und die Ausnahmen-Liste wurde um ~/.gnupg, ~/.aws,
  ~/.password-store erweitert. Grund: Der Vault-Eintrag existierte
  seit 13 Monaten, aber das aktive Bewusstsein (System-Prompt)
  kannte ihn nicht – jetzt ist er an der einzigen Stelle, die
  garantiert bei jedem Auftrag gelesen wird.

Alle nennenswerten Änderungen an Famulus werden in dieser Datei dokumentiert.

Das Format basiert auf [Keep a Changelog](https://keepachangelog.com/de/1.1.0/),
und dieses Projekt hält sich an [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.9.1] – 2026-08-26

### Behoben
- **`run_shell` hing ohne Timeout endlos**: Ein einzelner hängender
  Shell-Befehl blockierte den gesamten Agenten (und damit den
  Telegram-Bot) für immer. Konkreter Auslöser am selben Tag: ein
  `find` über das Home-Verzeichnis, das in einem blockierten
  `readdir` (Ordner `~/Music`) hing – der Bot war 50+ Minuten stumm.
  Ab jetzt gilt pro Befehl ein Zeitdeckel (Standard 300 s, per
  `timeout_seconds` je Aufruf anpassbar); bei Überschreitung wird
  die **gesamte Prozessgruppe** hart beendet (eigene Gruppe via
  `setpgid`, SIGKILL an `-PID`), damit keine verwaisten Kindprozesse
  übrig bleiben.
- **Leere Ausgabe bei `run_shell`**: `tokio::process::Command` erbt
  ohne Angabe standardmäßig die stdio-Handles des Elternprozesses –
  `wait_with_output()` hätte dann nichts zu sammeln. Stdout/Stderr
  werden jetzt explizit als Pipe gezogen.

### Geändert
- `run_shell` kennt den neuen Parameter `timeout_seconds`
  (optional, Standard 300). Für bewusst lange Befehle – z. B.
  HandBrake-/DVD-Konvertierung – setzt der Agent ihn höher.
- Version-Bump auf 0.9.1 (Patch: Bugfixes). `gui/tauri.conf.json`
  gleichgezogen (0.8.1 → 0.9.1).

### Neu getestet
- 4 Unit-Tests für das Shell-Timeout: schneller Befehl läuft durch,
  hängender Befehl bricht rechtzeitig ab, Befehl unterhalb des Limits
  bleibt unangetastet, Kindprozesse werden beim Timeout mitgetötet.

## [0.9.0] – 2026-08-26

### Hinzugefügt
- **`/status`-Befehl im Telegram-Bot**: Schickt man dem Bot `/status`,
  antwortet er sofort mit dem aktuell eingestellten Modell und dem
  Guthaben beim aktiven Provider - ohne LLM-Lauf, kostet also nichts
  und beantwortet die Frage auch dann, wenn der Provider gerade
  gestört ist. Motiviert von der iPad-Nutzung: Dort gibt es keine
  Statusleiste wie in der GUI, Modell und Credits waren unsichtbar.
- Neues Kern-Modul `src/credits.rs`: die Guthaben-Abfrage (Hyper,
  OpenRouter, Ollama → "lokal") lebt jetzt einmal im Kern statt
  doppelt in GUI und Bot. Die GUI nutzt ihre eigene Kopie noch -
  Konsolidierung ist ein Kandidat für eine spätere Version.

### Geändert
- Version-Bump auf 0.9.0 (neues Feature = Minor nach SemVer).

## [0.8.1] – 2026-08-24

### Geändert
- **Streaming für den Anthropic-/Hyper-Provider**: Antworten werden jetzt
  Stück für Stück gelesen statt als Ganzes abgewartet. Der 300-Sekunden-
  Timeout gilt nicht mehr für die gesamte Antwort, sondern als
  Inaktivitätsgrenze: Solange Daten fließen, darf die Antwort beliebig
  lange dauern. Damit stirbt kein Auftrag mehr mit
  „operation timed out“ an langen, aber gesunden Antworten.
- Eigener HTTP-Client ohne Gesamt-Timeout für den Streaming-Pfad
  (`http_client_ohne_gesamttimeout`); der Verbindungsaufbau bleibt über
  `connect_timeout` (20 s) begrenzt.

### Neu getestet
- 6 Unit-Tests für den SSE-Parser: Text-Deltas, thinking-Blöcke (werden
  ignoriert), Ereignis zerlegt über Paketgrenzen, Tool-Aufrufe aus
  JSON-Fragmenten, Tool ohne Argumente, Abbruch bei fehlendem
  `message_stop` und bei stockendem Strom.

## [0.8.0] – 2026-08-23

### Hinzugefügt
- **Sprachausgabe & -erkennung (TTS + STT)** in der GUI (macOS
  Speech-Framework); Mikrofon- und Spracherkennungs-Berechtigung
  werden vom Install-Skript in die Info.plist gepatcht.
- **Zwischenfragen** während eines laufenden Auftrags: Senden wird
  zum Zwischenfrage-Button, Abbrechen hat einen eigenen Button.
- **Datum/Uhrzeit** neben den Rollen-Labels (Du/Famulus/Fehler) im Chat.
- **iPad/iPhone-Install-Skript** (`scripts/install-ios.sh`), wird vom
  Mac-Install automatisch mitgezogen.
- **Echte Löschen-/Archivieren-Buttons** pro Chat-Zeile: 🗄 archiviert
  reversibel, ✕ löscht endgültig mit Bestätigungsdialog.

### Behoben
- **Chat-Löschen per ✕ wirkungslos** auf macOS: `window.confirm()` wird
  von WKWebView ohne Dialog still mit `false` beantwortet (kein
  `runJavaScriptConfirmPanel` im UI-Delegate von wry). Ersatz durch
  einen eigenen Bestätigungsdialog (`frageBestaetigung()`), auch für
  das Preset-Löschen.
- Löschen des letzten Chats ließ `renderMessages()` an `chats[0] is
  undefined` sterben – es wird jetzt automatisch ein neuer Chat
  angelegt.
- Veralteter Tailscale-Hostname blockierte jede iPad/iPhone-Fernbedienung.
- Doppeltes „Archiv“-Sidebar-Header-Div entfernt.

### Geändert
- Regelbasierte automatische Modellwahl; Credits-Anzeige beim
  Umschalten auf die automatische Modellwahl korrigiert.


## [0.7.2] – 2026-08-22

### Hinzugefügt
- **Selbstmodell-Tool** (`src/tools/selbstmodell.rs`): schreibt
  `Wer-ist-Famulus.md` in den Vault - Fähigkeiten, Geschichte (aus
  CHANGELOG.md), Selbstkenntnis (Scorecard, Erinnerungszahl), Grenzen.
  Fließt über `systemvorspann()` als Selbstbild in jeden Auftrag ein.
- **Idle-Loop**: alle 6 Stunden, nur während die GUI läuft (kein
  Hintergrund-Daemon, stirbt mit dem Prozess). Bewusst konservativ:
  kein LLM-Aufruf, nur ein DB-Statistik-Schnappschuss ins Notizbuch -
  keine laufenden Kosten, kein Konflikt mit dem Ein-Task-Slot der GUI.
- **`max_turns = 997`** in `~/.famulus/famulus.toml` (war 101).

### Behoben
- ToM-Textbaustein in `agent.rs` hatte massives Leerzeichen-Wirrwarr im
  Prompt-String - unnötige Tokens bei jedem Auftrag.
- Ungenutzte `chrono`-Abhängigkeit entfernt (nirgends importiert).
- Zwei beim Anfügen von `selbstmodell` versehentlich gelöschte
  Doc-Kommentare in `tools/mod.rs` wiederhergestellt.
- Absturz (`SIGKILL Code Signature Invalid`) durch Rebuild während der
  Entwicklung dieser Features - kein Zusammenhang mit `max_turns`
  selbst. Frischer Build + atomarer Install (siehe 0.6.2-Fix) hat es
  behoben.

### Versions-Nachtrag
- Die vorherigen zwei Commits ("v0.7.1") hatten die Versionsnummer nie
  tatsächlich auf 0.7.1 gesetzt - direkt auf 0.7.2 gesprungen, um keine
  doppelt vergebene Version zu erzeugen.

## [0.7.0] – 2026-08-22

### Hinzugefügt
- **Provider-Router mit Fallback** (`src/llm/router.rs`): `RouterProvider`
  implementiert `LlmProvider` genauso wie jeder echte Anbieter (Decorator-
  Muster) - `agent.rs` musste dafür nicht angefasst werden. Ohne
  konfigurierten Fallback (Normalfall) liegt genau ein Provider in der
  Liste, Verhalten ist identisch zu vorher.
- **`fallback_providers` in `famulus.toml`** (optional, leer = altes
  Verhalten): Reihe von Ausweich-Providern, die der Reihe nach probiert
  werden, wenn der Hauptprovider fehlschlägt (Netzwerk, Rate-Limit,
  Timeout), statt den Auftrag abzubrechen.
- **Scorecard** (`provider_statistik`-Tabelle in `gedaechtnis.db`):
  protokolliert jeden Modellaufruf (Provider, Erfolg, Dauer). CLI zeigt am
  Ende eines Auftrags eine Zusammenfassung (Aufrufe, Erfolgsquote,
  Durchschnittslatenz je Provider).

### Bewusst nicht gemacht
- Kein dynamisches Umsortieren nach Erfolgsquote oder kostenbasiertes
  Routing - `LlmAntwort` liefert keine Token-/Kosten-Daten, dafür müssten
  erst alle Provider-Implementierungen erweitert werden. Die Statistik wird
  trotzdem von Anfang an mitgeschrieben.
- Kein Docker-Sandboxing für Tool-Ausführung (Idee kam ursprünglich aus
  einem KI-OS-Vergleich) - widerspricht Famulus' bewusster
  Sicherheitsphilosophie (voller Rechnerzugriff, siehe README). Bleibt so.

## [0.6.1] – 2026-08-22

### Behoben
- **Stufe 2 (Embeddings) war inaktiv, weil das falsche Modell benutzt wurde.**
  `qwen3:14b` (Famulus' Chat-Modell) hat laut Ollama keine "embedding"-
  Fähigkeit; `/api/embeddings` lehnte jede Anfrage ab mit "This server does
  not support embeddings. Start it with `--embeddings`" - eine Fehlermeldung,
  die einen Server-Start-Flag suggeriert, den es in dieser Ollama-Version
  (0.32.15) gar nicht gibt (`ollama serve --help` kennt kein `--embeddings`).
  Tatsächlich ist es eine Modell-Eigenschaft, kein Server-Flag.
- **Fix**: dediziertes Embedding-Modell `nomic-embed-text` gepullt (274 MB)
  und in `embedding_berechnen` fest verdrahtet (`EMBEDDING_MODELL`-Konstante).
  Nebeneffekt: deutlich schneller und leichter als ein 14B-Chat-Modell für
  Embeddings zu zweckentfremden.
- Verifiziert: 125 Erinnerungen mit korrekten 768-dimensionalen Vektoren
  in der DB, `embeddings_nachholen` meldet jetzt einen echten Erfolgs-
  Zähler statt der vorherigen Fehlmeldung "114 nachgeholt" bei 0
  tatsächlich gespeicherten Embeddings.

## [0.6.0] – 2026-08-22

### Hinzugefügt
- **Gedächtnis-Ausbau in drei Stufen** (Famulus' eigener Vorschlag, von Claude
  fertiggestellt, weil der ursprüngliche Stand nicht mehr lief):
  - **Stufe 1 – FTS5**: `erinnerungen` und der Vault haben jetzt einen
    SQLite-FTS5-Index mit BM25-Ranking statt reinem Wortüberlapp. Neu:
    Tool `vault_suche` – bisher gab es zwar den Index, aber kein Werkzeug,
    das ihn benutzt hätte.
  - **Stufe 2 – Embeddings**: Semantische Suche via Ollama-Embeddings
    (`qwen3:14b`) + Cosine Similarity, mit automatischem Rückfall auf
    FTS5, wenn Ollama keine Embeddings ausliefert (`ollama serve
    --embeddings` nötig, sonst inaktiv - kein Fehler, nur leiser Rückfall).
  - **Stufe 3 – Notizbuch**: Tool `notizbuch`, mit dem der Agent sich
    während der Arbeit Notizen macht; der Rückblick konsolidiert sie am
    Ende in dauerhafte Erinnerungen.

### Behoben
- **Absturz bei jedem einzelnen Auftrag**: Die Embedding-Anbindung nutzte
  `reqwest::blocking` innerhalb der Tokio-Runtime, unter der CLI wie GUI
  laufen. Das crasht sofort mit "Cannot drop a runtime in a context where
  blocking is not allowed" - Famulus konnte dadurch keinen einzigen
  Auftrag mehr fertig bearbeiten. Fix: die komplette Embedding-Kette
  (`embedding_berechnen`, `embedding_speichern`, `embeddings_nachholen`,
  `relevante_semantisch`, `Agent::new`, `systemvorspann`) auf async
  umgestellt, mit dem normalen `reqwest::Client` statt `reqwest::blocking`.
- **Falscher Erfolgs-Zähler beim Embedding-Nachholen**: "114 Embeddings
  nachgeholt" wurde geloggt, obwohl jedes einzelne fehlgeschlagen war -
  der Zähler prüfte nur "kein Rust-Fehler", nicht "wirklich gespeichert".
  Zusätzlich lief bei jedem Start ein Versuch pro fehlender Erinnerung,
  auch wenn Ollama gar keine Embeddings unterstützt. Fix: einmaliger
  Verfügbarkeits-Check vorab (`embeddings_verfuegbar`), `embedding_speichern`
  meldet jetzt ehrlich zurück, ob wirklich eins gespeichert wurde.
- **Frontend zeigte bei neuen Erinnerungen die falsche Kategorie**: Das
  `AgentEvent`-Enum ist per `#[serde(tag = "art")]` getaggt - `ev.art` im
  Frontend war deshalb immer `"gemmerkt"` (der Tag selbst), nie die
  tatsächliche Kategorie. Musste `ev.kategorie` heißen.
- Diverse Kompilierfehler/Warnungen behoben (Test-Feldname nach
  `AgentEvent`-Umbenennung, Borrow-Checker-Probleme durch `?` als
  Block-Tail-Ausdruck, unnötiges `mut`).

## [0.5.0] – 2026-08-22

### Hinzugefügt
- **Chat-Verlauf mit Volltextsuche**: `src/history.rs` legt die Chats in
  derselben `gedaechtnis.db` ab wie die Erinnerungen. Alte
  `localStorage`-Chats werden beim ersten Start automatisch übernommen.
  Neue Tauri-Commands: `history_liste`, `history_suche`,
  `history_speichern`, `history_loeschen`, `history_archiv_liste`,
  `history_archivieren`.

### Behoben
- **Wiederholte Crashes durch kaputtes App-Bundle**: `/Applications/Famulus.app`
  enthielt ein verschachteltes `Famulus.app/Famulus.app`, weil ein
  Neu-Install per `cp -a Famulus.app /Applications/Famulus.app` in ein
  bereits bestehendes Zielverzeichnis kopiert hat, statt es zu ersetzen.
  `codesign --verify` schlug dadurch mit "unsealed contents present in the
  bundle root" fehl, und macOS killte den Prozess mit
  `SIGKILL (Code Signature Invalid)` - sowohl beim Start ("Taskgated
  Invalid Signature") als auch mitten in der Arbeit, wenn eine laufende
  Instanz Code-Seiten von der inzwischen veränderten Datei nachlud. Das
  erklärt auch die mehrfachen Dock-Icons bei wiederholten
  Start-Versuchen.
- **Fix**: `/Applications/Famulus.app` bereinigt und `scripts/install-mac.sh`
  ergänzt - baut mit `cargo tauri build --bundles app`, beendet eine
  laufende Instanz zuerst, entfernt die alte Installation komplett
  (`rm -rf`) statt hineinzukopieren, und startet danach genau einmal neu.
- **Versions-Drift**: `gui/Cargo.toml` stand noch auf `0.3.0`, während
  `Cargo.toml` und `gui/tauri.conf.json` schon `0.4.0` waren. Alle drei
  jetzt synchron auf `0.5.0`.

### Geändert
- README.md: `ollama` in die Provider-Tabelle ergänzt, Gedächtnis-Pfad
  korrigiert (`~/KI Agenten/famulus/gedaechtnis.db`, nicht
  `~/.famulus/gedaechtnis.db`), macOS-Installationshinweis auf
  `scripts/install-mac.sh` umgestellt.
- MAC-SETUP.md: Warnung vor manuellem `cp` über ein bestehendes Bundle.

## [0.4.0] – 2026-08-21

### Hinzugefügt
- **System-Prompt-Editor mit Presets**: In der rechten Sidebar kann jetzt zwischen verschiedenen
  System-Prompts gewechselt werden. Vier Presets sind vorinstalliert:
  - **Standard**: Famulus als hilfsbereiter Assistent
  - **Code-Reviewer**: Senior-Entwickler für Rust, TypeScript, Python, Shell
  - **Texter**: Professioneller Lektor für deutsche Texte
  - **Kreativ**: Ideengeber und Brainstorming-Partner
- Presets werden in `~/.famulus/presets.toml` gespeichert und können bearbeitet, gelöscht und
  neu angelegt werden.
- Der aktive System-Prompt wird vom Agenten in den System-Vorspann übernommen – vor den
  Gedächtnis- und Vault-Anweisungen.
- Remote-Unterstützung für iPad: Presets werden über WebSocket synchronisiert.

### Geändert
- `agent.rs`: `systemvorspann()` lädt jetzt den aktiven Prompt aus den Presets.
- `gui/src/lib.rs`: 4 neue Tauri-Commands (`presets_liste`, `presets_aktivieren`,
  `presets_speichern`, `presets_loeschen`) + 4 Remote-Commands.
- `gui/src/remote.rs`: Neue Request/Response-Varianten für Presets.
- `ui/index.html`: Rechte Sidebar enthält jetzt Presets-Bereich mit Dropdown + Textarea + Buttons.

## [0.3.1] – 2026-08-21

### Behoben
- **Code-Signing-Crash**: `bundle.active` in `tauri.conf.json` war auf `false` gesetzt, wodurch `cargo build --release` das Binary un-signiert (ad-hoc) ins `/Applications/Famulus.app` kopierte. macOS killte den Prozess mit `SIGKILL (Code Signature Invalid)`, sobald es Code-Seiten von Platte nachlud. Zusätzlich gingen TCC-Freigaben bei jedem Rebuild verloren, weil ad-hoc-Signaturen sich bei jedem Build ändern.
- **Fix**: `bundle.active` auf `true` gesetzt. Der offizielle Weg ist jetzt `cargo tauri build --bundles app` – das signiert mit dem konfigurierten Dev-Zertifikat (`Apple Development: jens.gilde@gmail.com`), versiegelt das Bundle und erzeugt eine stabile Signatur-Identität.

### Geändert
- `tauri.conf.json`: `"bundle": { "active": true }` (war `false`)
- Build-Prozess: `cargo tauri build --bundles app` statt manuellem Kopieren des Binarys

## [0.3.0] – 2026-08-21

### Hinzugefügt
- **Ollama-Provider**: Lokale Modelle (deepseek-r1:14b) als dritter Provider in der Dropdown-Liste.
  - Provider-Auswahl jetzt: Hyper · OpenRouter · Ollama
  - Ollama spricht das OpenAI-Protokoll auf `localhost:11434`, kein API-Key nötig.
  - Modell-Liste wird direkt von Ollama `/api/tags` geladen.
  - Credits-Anzeige zeigt "lokal" für Ollama (kostenlos).
  - `OpenAiProvider::neu_ohne_key()` für Provider ohne Authentifizierung.

### Geändert
- `OpenAiProvider` sendet nur dann `Bearer`-Auth-Header, wenn ein API-Key gesetzt ist.
- `build_provider()` akzeptiert jetzt "ollama" als Provider.
- `credits()` und `modelle_liste()` (Mac + Remote) erkennen Ollama als Sonderfall.

## [0.2.0] – 2025-07-15

### Hinzugefügt
- **Linke Seitenleiste (Chats)** mit `☰`-Toggle-Button in der Toolbar. Startet geschlossen.
- **Rechte Seitenleiste (Archiv)** mit `◫`-Toggle-Button in der Toolbar. Archivierte Chats können wiederhergestellt werden.
- Sidebar-Zustand (offen/geschlossen) wird in `localStorage` gespeichert und bleibt über Neustarts erhalten.
- **Datei-Upload-Backend**: `BildAnhang` und `UserMitBild`-Strukturen für Screenshots und Dateianhänge an LLMs.
- **Modell-Liste pro Provider**: Modell-Dropdown zeigt jetzt nur die Modelle des aktuell gewählten Providers an, statt einer globalen Liste.

### Geändert
- **Credits** von der Toolbar in die Statusbar (unten rechts) verschoben.
- Fehler-Banner für bessere Sichtbarkeit von Fehlermeldungen überarbeitet.
- UI: Toolbar-Layout neu strukturiert (links: Sidebar-Toggle, Mitte: Titel, rechts: Provider/Modell).

### Behoben
- Linke Seitenleiste konnte nicht geschlossen werden (kein Toggle vorhanden).
- Rechte Seitenleiste fehlte komplett (Archiv-Funktion war nicht zugänglich).

## [0.1.0] – 2025-07-14

### Erster Release
- Grundlegende Chat-Funktionalität mit Claude (Anthropic) und OpenAI als Provider.
- Tauri 2 GUI mit macOS-App.
- CLI-Modus (`famulus`).
- Shell-Zugriff für den Agenten.
- SQLite-basiertes Gedächtnis.
- Versionsnummer im Header.