// Kein Konsolenfenster unter Windows. Auf Linux wirkungslos, schadet nicht.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Ohne diese Pruefung ignoriert das Programm jedes Argument und startet
    // trotzdem die volle GUI samt Fenster und WebSocket-Server - auch bei
    // "--version". Das fuehrt bei jedem Aufruf zur Versionspruefung (z.B.
    // waehrend eines Rebuilds) zu einer weiteren, ungewollten Instanz.
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!("famulus-gui {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    famulus_gui::run();
}