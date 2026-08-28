// Famulus – Design-Tokens der nativen SwiftUI-Hülle v0.10.0.
// Identische Marken-DNA wie der Tauri-Kern (ui/index.html) und
// Famulus Games (dort bewährt seit v0.2.3): warmes Braun oben,
// durchgängiger Verlauf bis Schwarz unten, Creme-Text,
// Orange-Akzent #F86E27. Magenta ist seit 2026-08-27 dauerhaft raus.

import SwiftUI

enum Marke {
    // Akzent: Famulus-Orange
    static let akzent      = Color(red: 0.973, green: 0.431, blue: 0.153) // #F86E27
    static let akzentHover = Color(red: 1.000, green: 0.541, blue: 0.290) // #FF8A4A
    static let akzentGetönt = akzent.opacity(0.13)

    // Verlauf: Braun oben -> Schwarz unten (wie im Kern)
    static let verlaufOben  = Color(red: 0.129, green: 0.106, blue: 0.086) // #211B16
    static let verlauf = LinearGradient(
        colors: [verlaufOben, Color(red: 0.106, green: 0.082, blue: 0.067),
                 Color(red: 0.043, green: 0.031, blue: 0.024), .black],
        startPoint: .top, endPoint: .bottom)

    // Flächen (teiltransparent angelehnt an den Kern)
    static let kopfFläche   = Color(red: 0.106, green: 0.082, blue: 0.067).opacity(0.88)
    static let fußFläche    = Color.black.opacity(0.55)
    static let fläche       = Color(red: 0.157, green: 0.125, blue: 0.098) // #282019
    static let eingabe      = Color(red: 0.145, green: 0.114, blue: 0.082) // #251D15
    static let hover        = Color(red: 0.220, green: 0.165, blue: 0.118) // #382A1E
    static let seitenLeiste = Color(red: 0.114, green: 0.090, blue: 0.071) // #1D1712

    // Text (Creme-Töne wie im Kern)
    static let text          = Color(red: 0.929, green: 0.890, blue: 0.835) // #EDE3D5
    static let textSekundär  = Color(red: 0.753, green: 0.698, blue: 0.620) // #C0B29E
    static let textLeise     = Color(red: 0.522, green: 0.467, blue: 0.388) // #857763
    static let textHauch     = Color(red: 0.333, green: 0.286, blue: 0.227) // #55493A

    // Ränder & Signale
    static let rand      = Color(red: 0.227, green: 0.173, blue: 0.122)     // #3A2C1F
    static let randHover = Color(red: 0.306, green: 0.231, blue: 0.157)     // #4E3B28
    static let gefahr    = Color(red: 1.000, green: 0.267, blue: 0.267)     // #ff4444
    static let erfolg    = Color(red: 0.000, green: 0.800, blue: 0.400)     // #00cc66
    static let warnung   = Color(red: 1.000, green: 0.667, blue: 0.000)     // #ffaa00
}
