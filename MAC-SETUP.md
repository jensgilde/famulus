# Famulus auf dem Mac bauen

Diese Anleitung geht davon aus, dass auf dem Mac noch gar nichts eingerichtet
ist. Reihenfolge einhalten, dann läuft es durch.

## 1. Xcode Command Line Tools

```bash
xcode-select --install
```

Das öffnet ein Fenster, Installation bestätigen und warten (ein paar hundert
MB). Prüfen, ob es geklappt hat:

```bash
cc --version    # muss "Apple clang" ausgeben
```

**Warum das nötig ist:** Famulus benutzt `rusqlite` mit der Einstellung
`bundled` — das heißt, die SQLite-Datenbank wird aus ihrem C-Quelltext
mitkompiliert, statt die vom System zu benutzen. Dafür braucht es einen
C-Compiler. Tauri braucht die Tools ebenfalls.

## 2. Rust

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Vorschlag 1 (Standard-Installation) bestätigen. Danach entweder ein neues
Terminal öffnen oder:

```bash
source "$HOME/.cargo/env"
rustc --version   # sollte 1.9x oder neuer sein
```

`rustup` erkennt selbst, ob der Mac einen Apple-Silicon- oder einen
Intel-Prozessor hat. Da musst du nichts einstellen.

## 3. Was auf dem Mac NICHT nötig ist

Das ist der große Unterschied zu Fedora — spar dir die Sucherei:

| Auf Fedora gebraucht | Auf dem Mac |
|---|---|
| `webkit2gtk4.1-devel`, `gtk3-devel`, `libappindicator` … | **nichts.** Tauri benutzt das systemeigene WKWebView |
| `openssl-devel` | **nichts.** `reqwest` benutzt hier Apples Security-Framework |
| `sqlite-devel` | **nichts.** Wird mitkompiliert (siehe Schritt 1) |
| `pkg-config` | **nichts** für dieses Projekt |

Kein Homebrew nötig. Kein `npm` — das Frontend ist eine einzelne
HTML-Datei ohne Build-Schritt.

## 4. Quelltext auspacken

```bash
unzip famulus-mac.zip -d ~/
cd ~/famulus
```

## 5. API-Key hinterlegen

Der Key ist **absichtlich nicht im Zip**. Leg die Datei von Hand an:

```bash
cp .env.example .env
```

Dann `.env` in einem Editor öffnen und beim passenden Eintrag den Key
eintragen. Für Charm Hyper:

```
HYPER_API_KEY=sk-hyper-...
```

Den Key findest du auf der Fedora-Kiste in `~/famulus/.env`.

## 6. Konfiguration anlegen

Famulus liest seine Einstellungen aus `~/.famulus/famulus.toml` — also aus
dem Home-Verzeichnis, nicht aus dem Projektordner.

```bash
mkdir -p ~/.famulus
cp famulus.toml.example ~/.famulus/famulus.toml
```

Dein aktueller Stand auf Fedora sieht so aus — kannst du wörtlich übernehmen,
außer beim Vault-Pfad (siehe unten):

```toml
provider = "hyper"
model = "deepseek-v4-pro"
max_turns = 20
max_erinnerungen = 12
reflexion = true
deny_paths = []
vault = "~/Documents/Hermes-Vault"
```

**Zum Vault:** Wenn der Hermes-Vault auf dem Mac nicht unter
`~/Documents/Hermes-Vault` liegt, entweder den Pfad anpassen oder die Zeile
weglassen. Ein falscher Pfad führt dazu, dass die Vault-Werkzeuge gar nicht
erst angeboten werden — das ist Absicht, damit du es merkst.

**Zu `deny_paths = []`:** Das heißt volle Freiheit auf dem ganzen Rechner,
keine Rückfragen. So ist es gewollt (siehe README, Abschnitt „Wie die
Sicherheit funktioniert").

Das Gedächtnis (`~/.famulus/gedaechtnis.db`) legt Famulus beim ersten Start
selbst an. Wenn du die Erinnerungen von der Fedora-Kiste mitnehmen willst,
kopier die Datei einfach mit rüber.

## 7. Bauen

```bash
cd ~/famulus

cargo test                        # 23 Tests, sollten alle grün sein
cargo run -- "Sag Hallo"          # Kommandozeile
cargo run -p famulus-gui          # Fenster
```

Der erste Build dauert ein paar Minuten, weil alle Abhängigkeiten frisch
übersetzt werden — die GUI länger als das CLI. Danach geht es schnell.

Für die fertigen Binaries:

```bash
cargo build --release             # nur das CLI
cargo build --release -p famulus-gui
```

Achtung, `cargo build --release` allein baut im Wurzelverzeichnis **nur das
CLI**, nicht die GUI — die GUI ist ein eigenes Mitglied im Cargo-Workspace
und muss mit `-p famulus-gui` angestoßen werden. Der Release-Build ist
bewusst auf kleine Binaries getrimmt (`lto = "fat"`, `codegen-units = 1`)
und dauert deshalb deutlich länger als der normale: auf der Fedora-Kiste
rund 4 Minuten fürs CLI und 8 für die GUI.

Die fertigen Programme liegen dann in `target/release/famulus` und
`target/release/famulus-gui`.

**Fuer die GUI als installierte `/Applications/Famulus.app`:** nicht von
Hand `cp` benutzen, sondern `./scripts/install-mac.sh`. Ein rohes
`cp -a Famulus.app /Applications/Famulus.app` nistet sich selbst, wenn das
Ziel schon existiert, statt es zu ersetzen - das Ergebnis ist ein Bundle,
dessen Signaturpruefung fehlschlaegt ("unsealed contents present in the
bundle root") und das macOS mit `SIGKILL (Code Signature Invalid)` killt,
auch waehrend die App laeuft. Das Skript baut mit `cargo tauri build
--bundles app` (signiert automatisch mit dem in `gui/tauri.conf.json`
hinterlegten Zertifikat), beendet eine laufende Instanz zuerst und ersetzt
das Bundle komplett statt hineinzukopieren.

## 8. Wenn etwas hakt

**`linker 'cc' not found`** → Schritt 1 wurde übersprungen oder ist
durchgefallen.

**Fehler beim Übersetzen von `libsqlite3-sys`** → ebenfalls Schritt 1; der
C-Compiler fehlt.

**Das GUI-Fenster bleibt leer** → Famulus schickt beim Laden einen
Selbsttest durchs Ereignissystem. Steht in der Terminal-Ausgabe die Zeile
`[famulus] Ereigniskanal steht`, ist die Verbindung zwischen Kern und Fenster
in Ordnung und das Problem liegt woanders. Fehlt sie, stimmt etwas mit
`gui/capabilities/default.json` nicht.

**`HYPER_API_KEY fehlt oder ist leer`** → Schritt 5. Famulus sucht die `.env`
zuerst im aktuellen Verzeichnis, dann in `~/.famulus/.env`. Wer die GUI per
Doppelklick startet statt aus dem Terminal, hat kein sinnvolles
Arbeitsverzeichnis und braucht die zweite Variante.

## 9. Was hier ungetestet ist

Ehrlichkeitshalber: Dieses Projekt ist bisher **nur auf Fedora Linux gebaut
und gelaufen**. Der Quelltext enthält nichts offensichtlich Linux-Eigenes —
die einzige plattformabhängige Stelle ist `std::os::unix::fs::symlink` in
einem Test, und macOS ist ein Unix. Aber „sollte gehen" ist nicht „ist
gelaufen". Wenn beim ersten Build etwas quer kommt, ist das kein Grund zur
Sorge, sondern normal.
