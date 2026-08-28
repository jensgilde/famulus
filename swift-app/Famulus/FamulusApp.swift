// Famulus – Einstiegspunkt der nativen SwiftUI-Hülle v0.12.1.
// Dieselbe Marken-DNA wie der Tauri-Kern (ui/index.html) und
// Famulus Games: dunkles Terminal-Design, Orange-Akzent (#F86E27),
// Monospace. Die Logik liegt im Rust-Kern (libfamulus_core.a via UniFFI).

import SwiftUI

@main
struct FamulusApp: App {
    var body: some Scene {
        WindowGroup("Famulus") {
            Hauptansicht()
                .frame(minWidth: 820, minHeight: 560)
        }
        .defaultSize(width: 1180, height: 760)
    }
}
