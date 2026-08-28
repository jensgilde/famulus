#!/usr/bin/env bash
# Famulus – CLI-Binaries stabil signieren (famulus + famulus-telegram).
#
# WARUM das nötig ist (TCC / Ordnerfreigaben):
#   macOS merkt sich Ordner-Freigaben (Documents, Downloads, Desktop, ...)
#   anhand der Codesignatur-Identität eines Programms.
#   - Bei AD-HOC-Signatur ist die Identität der reine Code-Hash (cdhash) –
#     und der ändert sich mit JEDEM Build. macOS hält jede neue Version für
#     ein "anderes Programm" und vergisst alle erteilten Freigaben.
#   - Abhilfe: mit dem echten Entwickler-Zertifikat + stabilem Identifier
#     signieren. Dann ist die Identität (Zertifikat + Identifier) über
#     Builds hinweg gleich und die Freigaben bleiben dauerhaft erhalten.
#
# Dieses Skript ist der "einzig richtige Weg", die CLI-Binaries zu signieren.
# Es wird von scripts/build-cli.sh nach `cargo build --release` aufgerufen.
#
# SICHERHEIT: Ein Binary, das GERADE LÄUFT, darf nicht überschrieben werden –
# der Kernel killt den Prozess sonst mit SIGKILL (Code Signature Invalid),
# sobald sich die Datei auf der Platte unter dem laufenden Prozess ändert
# (siehe vault/03-Wissen/Wiederkehrender-Absturz-Code-Signature.md).
# Deshalb überspringt das Skript Binaries, deren Prozess gerade läuft.
# Den famulus-telegram-Bot NICHT hierüber neu starten – der wird per launchd
# verwaltet (com.jensgilde.famulus-telegram) und darf während laufender
# Telegram-Aufträge nicht angefasst werden.

set -euo pipefail
cd "$(dirname "$0")/.."

# Zertifikat: erstes gültiges "Apple Development"-Zertifikat im Schlüsselbund.
SIGN_IDENTITY=$(security find-identity -v -p codesigning 2>/dev/null \
    | grep -o '"Apple Development: [^"]*"' | head -1 | tr -d '"')

if [ -z "$SIGN_IDENTITY" ]; then
    echo "FEHLER: Kein Apple-Development-Zertifikat gefunden." >&2
    echo "Ohne Zertifikat gehen Ordner-Freigaben bei jedem Build verloren." >&2
    exit 1
fi
echo "Signatur-Identität: $SIGN_IDENTITY"

# Stabile Identifier pro Binary – passend zu den bereits in der TCC-DB
# gespeicherten Einträgen, damit bestehende Freigaben weiter gelten.
sign_one() {
    local bin="$1" ident="$2"

    if [ ! -f "$bin" ]; then
        echo "  – $bin nicht vorhanden, übersprungen."
        return
    fi

    # Läuft dieses Binary gerade? Dann NICHT antasten (SIGKILL-Gefahr).
    # pgrep -x matcht exakt den Prozessnamen (comm) – ein bloßes -f auf den
    # Pfad würde z.B. für "target/release/famulus" auch den laufenden
    # "famulus-telegram"-Prozess treffen (Substring) und fälschlich sperren.
    if pgrep -x "$(basename "$bin")" > /dev/null 2>&1; then
        echo "  ! $bin läuft gerade – übersprungen (kein Überschreiben unter"
        echo "    einem laufenden Prozess). Erst stoppen, dann erneut signieren."
        return
    fi

    codesign --force --sign "$SIGN_IDENTITY" --identifier "$ident" "$bin"
    codesign --verify --strict --verbose=2 "$bin" 2>/dev/null
    echo "  ✓ $bin signiert als '$ident'"
}

echo "==> Signiere CLI-Binaries in target/release ..."
sign_one "target/release/famulus"         "one.gilde.famulus-cli"
sign_one "target/release/famulus-telegram" "one.gilde.famulus-telegram"

echo "Fertig."
