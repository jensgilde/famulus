#!/usr/bin/env bash
# Baut Famulus fuers iPad/iPhone und installiert es auf beiden - Pendant zu
# install-mac.sh, gleiche Version (tauri.conf.json), gleicher Code, gleiche
# Signatur-Identitaet.
#
# Voraussetzung: beide Geraete muessen fuer Xcode erreichbar sein - per
# Kabel, oder entsperrt im selben WLAN (Xcode-Funkverbindung, einmalig in
# Xcode selbst gekoppelt). Ist eins nicht erreichbar, wird NUR das
# uebersprungen (kein harter Abbruch) - der Mac-Install soll davon nicht
# abhaengen.
#
# Erster Start auf einem Geraet nach der Installation: iOS verlangt einmalig
# manuelles Vertrauen - Einstellungen > Allgemein > VPN & Geraeteverwaltung >
# Entwicklerzertifikat vertrauen. Das kann kein Skript uebernehmen, das ist
# eine Apple-Sicherheitsvorgabe, kein Famulus-Bug.

set -euo pipefail

cd "$(dirname "$0")/.."

IPAD_UDID="00008122-001470602133801C"
IPHONE_UDID="00008130-000C35323A21001C"

echo "==> Baue Famulus fuers iOS..."
(cd gui && PATH="$HOME/.cargo/bin:$PATH" cargo tauri ios build --export-method debugging --ci)

IPA="gui/gen/apple/build/arm64/Famulus.ipa"
if [ ! -f "$IPA" ]; then
    echo "Fehler: $IPA wurde nicht gebaut." >&2
    exit 1
fi

installiere() {
    local name="$1" udid="$2"
    echo "==> Installiere auf $name..."
    if xcrun devicectl device install app --device "$udid" "$IPA"; then
        echo "==> $name: installiert."
    else
        echo "==> $name: nicht erreichbar oder Installation fehlgeschlagen - uebersprungen." >&2
    fi
}

installiere "iPad" "$IPAD_UDID"
installiere "iPhone" "$IPHONE_UDID"

echo "Fertig."
