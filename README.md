# Famulus

Phase 0: ein Rust-CLI-Agent mit voller Datei-/Shell-Kontrolle, austauschbarem
LLM-Backend (Charm Hyper, Claude oder Grok) und einer hart einprogrammierten
Deny-Liste, die deine Trading-Bot-Verzeichnisse schützt - unabhängig davon,
was du in der Config einträgst oder vergisst.

## Provider

| `provider` | Endpunkt | Key aus | Protokoll |
|---|---|---|---|
| `hyper` | `https://hyper.charm.land` | `HYPER_API_KEY` | Anthropic Messages |
| `openrouter` | `https://openrouter.ai/api` | `OPENROUTER_API_KEY` | OpenAI Chat Completions |
| `ollama` | `http://localhost:11434` | kein Key noetig | OpenAI Chat Completions |

Charm Hyper spricht das Anthropic-Protokoll, `openrouter` das OpenAI-Protokoll
- deshalb reichen zwei Implementierungen (`llm/anthropic.rs`, `llm/openai.rs`).
Mit `base_url` und `api_key_env` in der `famulus.toml` lässt sich bei Bedarf
ein anderer kompatibler Dienst einhängen, ohne Code anzufassen.

Modellliste: `curl -s https://hyper.charm.land/v1/models`

## Setup

```bash
# Rust-Toolchain, falls noch nicht vorhanden
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Config anlegen
mkdir -p ~/.famulus
cp famulus.toml.example ~/.famulus/famulus.toml
# ~/.famulus/famulus.toml öffnen, provider wählen (hyper/openrouter),
# ggf. eigene deny_paths ergänzen

# API-Key hinterlegen
cp .env.example .env
# .env öffnen, HYPER_API_KEY oder OPENROUTER_API_KEY eintragen
```

## Bauen und starten

```bash
cargo build
cargo run -- "Liste alle Dateien im aktuellen Verzeichnis auf"
```

Beim ersten `cargo build` wird es vermutlich ein paar Kompilierfehler geben -
das ist normal bei frisch getipptem Code, den noch niemand kompiliert hat.
Schick mir einfach die Fehlermeldung, dann fixen wir das zusammen. Das ist
auch eine gute Gelegenheit, ein Gefühl für Rusts Fehlermeldungen zu
bekommen, die anfangs kryptisch wirken, aber meistens sehr genau sagen, was
zu tun ist.

## Wie die Sicherheit funktioniert

**Kurz: gar nicht mehr viel.** Famulus läuft mit vollem Rechnerzugriff und
fragt vor keiner Aktion nach. Das ist eine bewusste Entscheidung, keine
Lücke - sie steht hier, damit sich niemand auf einen Schutz verlässt, den es
nicht gibt.

Zwei Entscheidungsstufen pro Datei-Aktion:

- **Deny**: Der Pfad liegt unter einem Eintrag aus `deny_paths` in der
  `famulus.toml` → Aktion wird verweigert.
- **Allow**: alles andere.

Der Vergleich läuft auf aufgelösten Pfaden, greift also auch bei `..`, bei
Tilde und bei Symlinks - und auch dann, wenn die Datei noch gar nicht
existiert (`config::resolve_for_check`). Ist `deny_paths` leer, ist alles
erlaubt.

**`run_shell` läuft ungefiltert.** Shell-Befehle haben keinen einzelnen Pfad,
gegen den man prüfen könnte, also greift `deny_paths` dort nicht. Wer ein
Verzeichnis wirklich dichtmachen will, braucht Dateisystem-Rechte, keinen
Eintrag in einer TOML-Datei.

Früher gab es eine dritte Stufe **Ask** mit Rückfrage im Terminal bzw. als
GUI-Dialog. Die ist entfernt, weil sie faktisch nie ausgelöst wurde. Wer sie
zurückwill, braucht wieder ein `Ask` in `permissions::Decision` *und* eine
`frage`-Methode im `Ui`-Trait - eine Rückfrage ohne beides hat niemanden,
der sie stellt.

## Gedächtnis (Phase 1)

Zwei getrennte Speicher, mit Absicht:

| Wo | Was | Wann gelesen |
|---|---|---|
| `~/KI Agenten/famulus/gedaechtnis.db` | kurze Fakten, Präferenzen, Lektionen, Chat-Verlauf | bei **jedem** Auftrag |
| Obsidian-Vault | ausführliches Wissen in Prosa | wenn das Modell es holt |

In derselben Datenbank liegt seit v0.5.0 auch der durchsuchbare Chat-Verlauf
der GUI (`src/history.rs`) - eine Datei statt zwei, damit nichts
auseinanderlaufen kann.

Käme alles in jeden Prompt, wäre der Kontext nach ein paar Wochen voll und
teuer. Die Datenbank hält das Kurze, der Vault das Lange.

**Selbstlernend** heißt hier konkret: Nach jedem erfolgreichen Auftrag läuft
ein zweiter, werkzeugloser Modellaufruf ("Rückblick"), der höchstens drei
dauerhaft nützliche Sätze herausdestilliert und ins Gedächtnis legt. Beim
nächsten Auftrag stehen sie im Kontext. Dubletten fängt ein `UNIQUE` auf dem
Inhalt ab, damit dieselbe Erkenntnis nicht zwanzigmal im Prompt landet.

Abschalten: `reflexion = false` in der `famulus.toml`. Vault ganz weglassen:
`vault` leer lassen, dann werden die Vault-Werkzeuge gar nicht erst angeboten.

Die Vault-Werkzeuge (`vault_liste`, `vault_lesen`, `vault_notiz`) sind auf die
Vault-Wurzel eingesperrt: Pfade mit `..`, `~` oder führendem `/` werden
abgelehnt - zusätzlich zur normalen Deny-Liste, nicht statt ihr.

## Installation ins System

```bash
cargo build --release
install -m 755 target/release/famulus     ~/.local/bin/famulus
install -m 755 target/release/famulus-gui ~/.local/bin/famulus-gui
```

Starter und Icon liegen in `~/.local/share/applications/famulus.desktop`
bzw. `~/.local/share/icons/hicolor/*/apps/famulus.png`.

**macOS:** `scripts/install-mac.sh` benutzen, nicht von Hand kopieren. Ein
blosses `cp -a .../Famulus.app /Applications/Famulus.app` nistet sich
selbst, wenn das Ziel schon existiert (`cp` legt dann
`/Applications/Famulus.app/Famulus.app` an statt zu ersetzen) - das kaputte
Bundle besteht die Signaturpruefung nicht mehr und macOS killt den Prozess
mit `SIGKILL (Code Signature Invalid)`, auch mitten in der Arbeit, wenn eine
laufende Instanz gerade Code-Seiten von der veraenderten Datei nachlaedt.
Das Skript beendet eine laufende Instanz zuerst, ersetzt das Bundle
komplett und startet genau einmal neu:

```bash
./scripts/install-mac.sh
```

## Was als Nächstes kommt (Phase 3)

- iPad/iPhone-Fernsteuerung. Das `Ui`-Trait ist die Stelle, an der die
  Ereignisse übers Netz umgeleitet werden.

## Grafische Oberfläche (Phase 2)

```bash
cargo run -p famulus-gui        # Fenster starten
cargo run -- "Auftrag"          # Kommandozeile, unverändert
```

Beide benutzen **denselben Kern**: derselbe Agent, dieselbe Deny-Liste,
dieselbe Berechtigungslogik. Unterschiedlich ist nur, was das `Ui`-Trait
(`src/ui.rs`) einhängt - Terminal-Ein-/Ausgabe oder Fenster-Ereignisse. Es
gibt bewusst keine zweite Kopie der Sicherheitsregeln, die auseinanderlaufen
könnte.

Die GUI sucht `.env` zusätzlich in `~/.famulus/.env`, weil ein per Doppelklick
gestartetes Fenster kein sinnvolles Arbeitsverzeichnis hat.

## Projektstruktur

- `src/main.rs` – CLI-Einstiegspunkt
- `src/config.rs` – lädt `famulus.toml` und `.env`, Pfad-Auflösung
- `src/permissions.rs` – Allow/Deny-Logik gegen `deny_paths`
- `src/llm/` – `LlmProvider`-Trait, dazu zwei Protokolle: `anthropic.rs`
  (Charm Hyper) und `openai.rs` (OpenRouter)
- `src/tools/` – `Tool`-Trait + Datei-Lesen/Schreiben, Shell-Ausführung
- `src/agent.rs` – die Beobachten-Denken-Handeln-Schleife
- `src/ui.rs` – `Ui`-Trait (Ereignisse) und die Terminal-Variante
- `src/lib.rs` – der Kern als Bibliothek, damit CLI und GUI ihn teilen
- `gui/` – Tauri-2-Anwendung; `ui/index.html` ist das Fenster
