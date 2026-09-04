#!/bin/bash
# anrufen.sh – öffnet die macOS-/Kontinuitäts-Telefon-App mit vorgefüllter Nummer.
# Wählt NICHT automatisch und wiederholt NICHT – nur einmalige Auslösehilfe.
# Aufruf: anrufen.sh [NUMMER]   (wenn ohne Nummer, wird 017662346258 verwendet)

NUMMER="${1:-017662346258}"

if ! command -v osascript >/dev/null 2>&1; then
  echo "FEHLER: osascript nicht gefunden" >&2
  exit 1
fi

echo "Öffne Telefon-App mit Nummer: $NUMMER"
# FaceTime-App übernimmt auf dem Mac die Kontinuitäts-Anrufe zum gekoppelten iPhone
osascript -e "tell application \"FaceTime\" to open location \"tel:+${NUMMER}\"" 2>/dev/null \
  || osascript -e "open location \"tel:+${NUMMER}\""

echo "Fertig – Nummer ist eingegeben. Anruf bitte manuell in der App starten."