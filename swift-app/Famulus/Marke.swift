// Famulus – Design-Tokens der nativen SwiftUI-Hülle v0.11.0.
// Phoenix-inspirierter Style, identisch mit Tankmonitor und der
// Webseite: dunkle Grundfläche #1e1e1e, weicher Orange-Fade oben,
// Akzent #f97316. Magenta ist seit 2026-08-27 dauerhaft raus.

import SwiftUI

enum Marke {
    // ── Phoenix Dark-Tokens (1:1 aus tankmonitor/style.css) ──
    static let grundFläche  = Color(red: 0.118, green: 0.118, blue: 0.118) // #1e1e1e
    static let fläche       = Color(red: 0.149, green: 0.149, blue: 0.149) // #262626
    static let rand         = Color(red: 0.227, green: 0.227, blue: 0.227) // #3a3a3a
    static let randStark    = Color(red: 0.290, green: 0.290, blue: 0.290) // #4a4a4a
    static let text         = Color(red: 0.976, green: 0.980, blue: 0.984) // #f9fafb
    static let textSekundär = Color(red: 0.612, green: 0.635, blue: 0.686) // #9ca3af
    static let textLeise    = Color(red: 0.424, green: 0.447, blue: 0.502) // #6b7280
    static let textHauch    = Color(red: 0.350, green: 0.370, blue: 0.420) // gedämpft

    // Akzent: Phoenix-Orange
    static let akzent       = Color(red: 0.976, green: 0.451, blue: 0.086) // #f97316
    static let akzentHover  = Color(red: 0.914, green: 0.345, blue: 0.047) // #ea580c
    static let akzentGetönt = akzent.opacity(0.12)

    // Signale
    static let erfolg  = Color(red: 0.290, green: 0.871, blue: 0.502) // #4ade80
    static let warnung = Color(red: 0.984, green: 0.749, blue: 0.141) // #fbbf24
    static let gefahr  = Color(red: 0.973, green: 0.443, blue: 0.443) // #f87171

    // ── Hintergrundverlauf: Orange-Fade oben (wie Phoenix) ──
    static let fadeOben = LinearGradient(
        colors: [
            Color(red: 0.486, green: 0.176, blue: 0.071).opacity(0.55), // #7c2d12 gedämpft
            Color.clear
        ],
        startPoint: .top, endPoint: .bottom)

    static var hintergrund: some View {
        ZStack {
            grundFläche
            fadeOben
        }
        .ignoresSafeArea()
    }

    // ── Flächen für Kopf, Fuß, Sidebar ──
    static let kopfFläche   = fläche.opacity(0.92)
    static let fußFläche    = grundFläche.opacity(0.95)
    static let seitenLeiste = fläche.opacity(0.60)
    static let eingabe      = Color(red: 0.125, green: 0.125, blue: 0.125) // #202020
    static let hover        = Color(red: 0.180, green: 0.180, blue: 0.180) // #2e2e2e

    // Chat-Nachrichten
    static let userNachricht    = akzentGetönt
    static let userRand         = akzent.opacity(0.35)
    static let assistantNachricht = fläche
    static let assistantRand    = rand
}
