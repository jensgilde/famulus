// Kein Konsolenfenster unter Windows. Auf Linux wirkungslos, schadet nicht.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    famulus_gui::run();
}