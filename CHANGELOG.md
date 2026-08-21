# Changelog

Alle nennenswerten Änderungen an Famulus werden in dieser Datei dokumentiert.

Das Format basiert auf [Keep a Changelog](https://keepachangelog.com/de/1.1.0/),
und dieses Projekt hält sich an [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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