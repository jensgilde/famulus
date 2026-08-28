# Changelog

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