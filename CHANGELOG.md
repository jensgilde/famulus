# Changelog

Alle nennenswerten Änderungen an Famulus werden in dieser Datei dokumentiert.

Das Format basiert auf [Keep a Changelog](https://keepachangelog.com/de/1.1.0/),
und dieses Projekt hält sich an [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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