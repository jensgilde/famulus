#!/bin/bash
# famulus-wecker.sh
# Zentrale Weck-Logik von Famulus: prüft an festen Zeitpunkten, welche
# wiederkehrenden Aufgaben fällig sind, und stößt sie an.
#
# Timer-Typen (in der launchd-plist gesetzt):
#   kicktipp : Montag+Mittwoch + vor jedem Spieltag
#   mail     : wöchentlich
#   vault    : monatlich
#   reflexion: alle 6h (Idle-Reflexion -> aktiv)
#   review    : wöchentlich Sonntag (Task-Observer-Review-Log)
#
# Wird von launchd ohne eigene Umgebung aufgerufen -> PATH hier setzen.

export PATH="/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin:/usr/local/sbin"
LOG="$HOME/Library/Logs/famulus-wecker.log"

log() { echo "$(date '+%Y-%m-%d %H:%M:%S') $*" >> "$LOG"; }

# --- Pfad-Einstellungen -----------------------------------------------------
FAMULUS_BIN="$HOME/KI Agenten/famulus/target/release/famulus"
KICKTIPPDIR="$HOME/KI Agenten/Projekte/kicktipp"
STAMP="$HOME/.famulus/wecker"

mkdir -p "$STAMP"
exec >> "$LOG" 2>&1

typ="${1:-alle}"
log "── Wecker-Zyklus (Typ=$typ) ──"

# Wochentag 0=Sonntag .. 6=Samstag
wochentag=$(date +%w)
tag=$(date +%d)
monat=$(date +%m)

run_famulus() {
    # $1 = Auftragstext
    log "famulus: $1"
    "$FAMULUS_BIN" "$1"
    rc=$?
    log "famulus rc=$rc (${1:0:40})"
    return $rc
}

run_kicktipp() {
    # Spieltag wird an tipp_alle übergeben; dieses skriptiert die komplette Saison
    log "kicktipp: starte tipp_alle"
    cd "$KICKTIPPDIR" || { log "kicktipp: Verzeichnis fehlt"; return 1; }
    node tipp_alle.js 2>&1 >> "$LOG"
    rc=$?
    log "kicktipp rc=$rc"
    return $rc
}

# --- Typ-Aktionen -----------------------------------------------------------
case "$typ" in
  kicktipp)
    # Mo (1) und Mi (3) prüfen & tippen; vor jedem Spieltag zusätzlich (hier täglich als Sicherheit)
    if [ "$wochentag" = "1" ] || [ "$wochentag" = "3" ]; then
        run_kicktipp
        run_famulus "Kicktipp-Spieltag: Prüfe Spielplan, ob heute ein Spieltag ansteht, und ob alle Tipps für die nächsten 2 Spieltage abgegeben sind. Wenn etwas fehlt, tippe es nach."
    fi
    # Täglich nur prüfen (nicht doppelt tippen) vor Spieltagen
    run_famulus "Kicktipp: Prüfe kurz, ob heute ein Spieltag ist. Falls ja, stelle sicher, dass alle Tipps abgegeben sind. Falls nein, tue nichts." 
    ;;

  mail)
    log "mail: wöchentliche IMAP-Aufräumaktion"
    run_famulus "Mail-Aufräumaktion im Famulus Hub: Komprimiere und bereinige die IMAP-Ordner von Famulus Mail (entferne alte/verwaiste Ordner, komprimiere, löse Dubletten auf). Die Mail-Funktion ist Teil des Famulus Hub unter ~/KI Agenten/famulus-hub/swift-app/FamulusHub/Mail. Richte dich nach diesem Hub-Mail-Modul."
    ;;

  vault)
    log "vault: monatlicher Vault-Cleanup"
    run_famulus "Führe den monatlichen Vault-Cleanup durch: Suche im Vault unter ~/KI Agenten/famulus/vault nach Dubletten (zwei Notizen mit gleichem/ähnlichem Inhalt) und veralteten Notizen. Löse Dubletten auf (hält die neuere, füge Verweis auf die ältere) und prüfe auf verwaiste/orphanistische Notizen. Dokumentiere das Ergebnis."
    ;;

  reflexion)
    # Idle-Reflexion -> aktiv werden, wenn etwas fällig ist
    log "reflexion: 6-Stunden-Zyklus"
    run_famulus "Idle-Reflexion in Handeln: Prüfe selbstständig, ob in den nächsten 48 Stunden eine wiederkehrende Aufgabe fällig ist (Kicktipp-Spieltag, wöchentliche Mail-Aufräumaktion, monatlicher Vault-Cleanup). Falls ja, beginne die betreffende Aufgabe jetzt proaktiv. Falls nichts fällig ist, tue nichts weiter."
    ;;

  review)
    # Wöchentlicher Task-Observer-Review (Sonntag): aus Korrekturen/Präferenzen
    # der Woche die wirksamsten Skill-Schärfungen ableiten und als Log sichern.
    log "review: wöchentlicher Task-Observer-Review"
    run_famulus "Führe den wöchentlichen Task-Observer-Review durch. Werte die seit dem letzten Review gesammelten Korrekturen, Präferenzen, Lektionen und wiederkehrenden Muster aus (Notizbuch, Gedächtnis/Erinnerungen, Vault). Schreibe in den Vault unter 03-Wissen/Famulus-Review.md ein knappes Review-Log mit: (1) die 3 bis 5 wirksamsten Skill-Schärfungen für die kommende Woche, konkret und umsetzbar formuliert, (2) was gut lief, (3) welche Dinge du weiterhin korrigieren musstest. Überschreibe die vorherige Review-Notiz, behalte aber unten eine kurze Historie der letzten 4 Einträge."
    ;;

  alle)
    # Kurzer täglicher Durchlauf: nur prüfen, keine schweren Aktionen
    run_famulus "Morgendlicher Check: Prüfe selbstständig, ob heute eine wiederkehrende Aufgabe fällig ist (Kicktipp-Spieltag, Mail-Cleanup, Vault-Cleanup). Falls ja, führe sie aus. Falls nein, nimm zur Kenntnis, dass heute nichts fällig ist, und tue nichts weiter."
    ;;

  *)
    log "Unbekannter Typ: $typ"
    ;;
esac

log "── Wecker-Zyklus beendet ──"
exit 0