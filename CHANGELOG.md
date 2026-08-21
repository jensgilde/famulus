# Changelog

Alle nennenswerten Änderungen an Famulus werden in dieser Datei dokumentiert.

Das Format basiert auf [Keep a Changelog](https://keepachangelog.com/de/1.1.0/),
und dieses Projekt hält sich an [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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