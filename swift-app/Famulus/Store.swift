// Famulus – Zustandsmodell der nativen SwiftUI-Hülle v0.12.1.
// Ruft den Rust-Kern über die UniFFI-Bindings (Generated/).
// Dieselbe Ereignis-Logik wie ui/index.html der Tauri-GUI:
// Agent-Ereignisse kommen als JSON über das AuftragsCallback und
// werden hier in den sichtbaren Chat-Zustand übersetzt.

import SwiftUI
import Observation

// ── Datenmodelle ────────────────────────────────────────────────────────

/// Eine gespeicherte Chat-Nachricht. `rolle` ist "user" / "assistant" /
/// "fehler". Entspricht dem `nachrichten`-JSON der Tauri-GUI.
struct ChatNachricht: Codable, Identifiable, Equatable {
    var rolle: String
    var inhalt: String
    var zeitstempel: TimeInterval
    /// Stable ID für ForEach – wird beim Dekodieren vergeben.
    var id = UUID()
}

/// Ein Chat (Titel + Nachrichten), wie er in der History-Datenbank liegt.
struct Chat: Identifiable {
    var sqliteId: Int64?
    var titel: String
    var nachrichten: [ChatNachricht]
    var createdAt: TimeInterval
    var id: String { sqliteId.map(String.init) ?? "neu-\(createdAt)" }
}

/// Ein Werkzeug- oder Gedächtnis-Schritt im "läuft gerade"-Block.
struct Schritt: Identifiable {
    enum Art { case werkzeug, gedaechtnis }
    var art: Art
    var name: String = ""
    var text: String = ""
    var ergebnis: String? = nil
    var id = UUID()
}

/// Ein System-Prompt-Preset (Name + Prompt).
struct Preset: Codable, Identifiable {
    var name: String
    var prompt: String
    var id: String { name }
}

// ── FFI-Callback ────────────────────────────────────────────────────────

/// Brücke vom Rust-Kern zurück in die UI. Wird auf einem Hintergrund-
/// Thread aufgerufen – springt deshalb auf den MainActor, bevor er den
/// Store anfasst. Die Klasse muss `final` sein, damit das Sendable-
/// Callback sie halten darf.
final class AuftragsSenke: AuftragsCallback {
    let weiterleiten: @Sendable (String) -> Void
    init(weiterleiten: @escaping @Sendable (String) -> Void) {
        self.weiterleiten = weiterleiten
    }
    func onEreignis(ereignisJson: String) {
        weiterleiten(ereignisJson)
    }
}

// ── Store ───────────────────────────────────────────────────────────────

@MainActor
@Observable
final class FamulusStore {

    // Chats
    var chats: [Chat] = []
    var archiv: [Chat] = []
    var aktiverChatIndex = 0
    var chatSuche = ""

    // Live-Auftrag
    var beschaeftigt = false
    var denktText = ""
    var agentSchritte: [Schritt] = []

    // Statusbar
    var status = "Bereit"
    var zustandText = ""
    var creditsText = ""

    // Modell-Auswahl
    var provider = "hyper"
    var verfuegbareModelle: [String] = []
    var ausgewaehltesModell = ""

    // Presets
    var presets: [Preset] = []
    var aktivesPreset: String?

    let version = appVersion()

    private var senke: AuftragsSenke?

    // ── Initial laden ────────────────────────────────────────────────

    func laden() {
        chats = ladeChats(archiviert: false)
        archiv = ladeChats(archiviert: true)
        ladePresets()
        aktualisiereStatus()
        // Provider aus Config holen, damit Dropdown und Credits stimmen
        provider = aktiverProvider()
        aktualisiereCredits()
        ladeModelle()
        if chats.isEmpty { neuerChat() }
    }

    var aktiverChat: Chat {
        get {
            if chats.isEmpty { return Chat(titel: "Neuer Chat", nachrichten: [], createdAt: Date.now.timeIntervalSince1970) }
            return chats[min(aktiverChatIndex, chats.count - 1)]
        }
        set {
            guard chats.indices.contains(aktiverChatIndex) else { return }
            chats[aktiverChatIndex] = newValue
        }
    }

    func neuerChat() {
        let neu = Chat(titel: "Neuer Chat", nachrichten: [], createdAt: Date.now.timeIntervalSince1970)
        chats.append(neu)
        aktiverChatIndex = chats.count - 1
        _ = speichern(chat: neu)
    }

    func chatWählen(_ index: Int) {
        guard chats.indices.contains(index) else { return }
        aktiverChatIndex = index
    }

    func chatLöschen(_ index: Int) {
        guard chats.indices.contains(index) else { return }
        let c = chats[index]
        if let id = c.sqliteId { try? historyLoeschen(id: id) }
        chats.remove(at: index)
        if aktiverChatIndex >= chats.count { aktiverChatIndex = max(0, chats.count - 1) }
        if chats.isEmpty { neuerChat() }
    }

    func chatArchivieren(_ index: Int, archiviert: Bool) {
        guard chats.indices.contains(index) else { return }
        let c = chats[index]
        if let id = c.sqliteId {
            try? historyArchivieren(id: id, archiviert: archiviert)
            chats.remove(at: index)
            if archiviert { archiv.insert(c, at: 0) }
            if aktiverChatIndex >= chats.count { aktiverChatIndex = max(0, chats.count - 1) }
        }
    }

    /// Holt einen archivierten Chat zurück in die aktive Liste und
    /// öffnet ihn. Wird aus der rechten Archiv-Sidebar aufgerufen.
    func archivOeffnen(_ chat: Chat) {
        guard let id = chat.sqliteId else { return }
        try? historyArchivieren(id: id, archiviert: false)
        archiv.removeAll { $0.sqliteId == id }
        var c = chat
        // sqliteId bleibt identisch – der Chat wird künftig aktualisiert.
        chats.append(c)
        aktiverChatIndex = chats.count - 1
    }

    // ── Auftrag ──────────────────────────────────────────────────────

    func senden(_ text: String) {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }

        // Beschäftigt: als Zwischenfrage senden – wie die Tauri-Referenz
        // (ui/index.html::sendeZwischenfrage). Kein neuer Auftrag, der
        // laufende läuft weiter; die Antwort kommt als `zwischenfrage_antwort`.
        if beschaeftigt {
            var chat = aktiverChat
            chat.nachrichten.append(ChatNachricht(
                rolle: "user", inhalt: trimmed, zeitstempel: Date.now.timeIntervalSince1970 * 1000))
            aktiverChat = chat
            _ = speichern(chat: chat)
            zwischenfrage(text: trimmed)
            return
        }

        var chat = aktiverChat
        chat.nachrichten.append(ChatNachricht(
            rolle: "user", inhalt: trimmed, zeitstempel: Date.now.timeIntervalSince1970 * 1000))
        if chat.titel == "Neuer Chat" {
            chat.titel = String(trimmed.prefix(40))
        }
        aktiverChat = chat
        _ = speichern(chat: chat)

        // Live-Zustand zurücksetzen und beschäftigen.
        beschaeftigt = true
        denktText = ""
        agentSchritte = []
        status = "Denke…"

        // Verlauf als JSON für den Kern zusammenstellen.
        let verlauf = chat.nachrichten
            .filter { $0.rolle == "user" || $0.rolle == "assistant" }
            .map { ["rolle": $0.rolle, "inhalt": $0.inhalt] }
        let verlaufDaten = (try? JSONSerialization.data(withJSONObject: verlauf)) ?? Data()
        let verlaufJson = String(data: verlaufDaten, encoding: .utf8) ?? "[]"

        let senke = AuftragsSenke { [weak self] json in
            // Der Rust-Kern ruft auf einem Hintergrund-Thread auf.
            Task { @MainActor [weak self] in
                self?.verarbeite(ereignis: json)
            }
        }
        self.senke = senke
        starteAuftrag(auftrag: trimmed, verlaufJson: verlaufJson, cb: senke)
    }

    func stoppen() {
        stoppeAuftrag()
    }

    /// Ein einzelnes Agent-Ereignis in den Chat-Zustand übersetzen.
    /// Dieselbe Verzweigung wie der `switch (ev.art)` in ui/index.html.
    private func verarbeite(ereignis json: String) {
        guard let daten = json.data(using: .utf8),
              let obj = try? JSONSerialization.jsonObject(with: daten) as? [String: Any],
              let art = obj["art"] as? String else { return }

        switch art {
        case "text":
            denktText += (obj["chunk"] as? String) ?? ""
            status = "Schreibe…"

        case "tool_start":
            agentSchritte.append(Schritt(
                art: .werkzeug,
                name: obj["name"] as? String ?? ""))
            denktText = ""
            status = "Werkzeug: \(obj["name"] as? String ?? "")"

        case "tool_end":
            let name = obj["name"] as? String ?? ""
            let inhalt = obj["inhalt"] as? String ?? ""
            if let idx = agentSchritte.lastIndex(where: {
                $0.art == .werkzeug && $0.name == name && $0.ergebnis == nil
            }) {
                agentSchritte[idx].ergebnis = inhalt
            }

        case "erinnert":
            let anzahl = obj["anzahl"] as? Int ?? 0
            agentSchritte.append(Schritt(art: .gedaechtnis, text: "⌾ \(anzahl) Erinnerungen im Kontext"))

        case "gemmerkt":
            let kat = obj["kategorie"] as? String ?? ""
            let inhalt = obj["inhalt"] as? String ?? ""
            agentSchritte.append(Schritt(art: .gedaechtnis, text: "✎ (\(kat)) \(inhalt)"))

        case "reflektiere":
            agentSchritte.append(Schritt(art: .gedaechtnis, text: "⟳ Rückblick…"))

        case "warte":
            let versuch = obj["versuch"] as? Int ?? 0
            let max = obj["max_versuche"] as? Int ?? 0
            let sek = obj["sekunden"] as? Int ?? 0
            let grund = obj["grund"] as? String ?? ""
            agentSchritte.append(Schritt(
                art: .gedaechtnis,
                text: "⏳ Fehlgeschlagen (\(versuch)/\(max)), erneut in \(sek)s: \(grund)"))
            status = "Warte \(sek)s…"

        case "modell_gewaehlt":
            let p = obj["provider"] as? String ?? provider
            let m = obj["model"] as? String ?? ""
            zustandText = "\(p) · \(m)"
            aktualisiereCredits()

        case "zwischenfrage_antwort":
            let frage = obj["frage"] as? String ?? ""
            let text = obj["text"] as? String ?? ""
            var chat = aktiverChat
            chat.nachrichten.append(ChatNachricht(
                rolle: "assistant",
                inhalt: "↩ Zu „\(frage)“:\n\n\(text)",
                zeitstempel: Date.now.timeIntervalSince1970 * 1000))
            aktiverChat = chat
            _ = speichern(chat: chat)

        case "fertig":
            var chat = aktiverChat
            chat.nachrichten.append(ChatNachricht(
                rolle: "assistant", inhalt: denktText,
                zeitstempel: Date.now.timeIntervalSince1970 * 1000))
            aktiverChat = chat
            _ = speichern(chat: chat)
            agentSchritte = []
            denktText = ""
            beschaeftigt = false
            status = "Bereit"
            aktualisiereCredits()

        case "abgebrochen":
            let fehler = obj["fehler"] as? String ?? "Abgebrochen"
            var chat = aktiverChat
            chat.nachrichten.append(ChatNachricht(
                rolle: "fehler", inhalt: fehler,
                zeitstempel: Date.now.timeIntervalSince1970 * 1000))
            aktiverChat = chat
            _ = speichern(chat: chat)
            agentSchritte = []
            denktText = ""
            beschaeftigt = false
            status = "Abgebrochen"
            aktualisiereCredits()

        default:
            break
        }
    }

    // ── Persistenz (History-Datenbank) ───────────────────────────────

    /// SQLite liefert `erstellt` als Text im Format "yyyy-MM-dd HH:mm:ss"
    /// (localtime). Der Formatter parst das wie `new Date()` im Tauri-Frontend.
    private static let datumFormat: DateFormatter = {
        let f = DateFormatter()
        f.dateFormat = "yyyy-MM-dd HH:mm:ss"
        f.locale = Locale(identifier: "en_US_POSIX")
        return f
    }()

    private static func parseDatum(_ text: String) -> Date {
        datumFormat.date(from: text) ?? .distantPast
    }


    private func ladeChats(archiviert: Bool) -> [Chat] {
        let json = (archiviert ? (try? historyArchivListe()) : (try? historyListe())) ?? "[]"
        guard let daten = json.data(using: .utf8),
              let roh = try? JSONSerialization.jsonObject(with: daten) as? [[String: Any]] else { return [] }
        return roh.compactMap { eintrag in
            let titel = eintrag["titel"] as? String ?? "Chat"
            let id = eintrag["id"] as? Int64
            let nachrichtenJson = eintrag["nachrichten"] as? String ?? "[]"
            let erstellt = Self.parseDatum(eintrag["erstellt"] as? String ?? "")
            var nachrichten: [ChatNachricht] = []
            if let nd = nachrichtenJson.data(using: .utf8),
               let rohN = try? JSONSerialization.jsonObject(with: nd) as? [[String: Any]] {
                nachrichten = rohN.compactMap { n in
                    guard let rolle = n["rolle"] as? String,
                          let inhalt = n["inhalt"] as? String else { return nil }
                    return ChatNachricht(
                        rolle: rolle, inhalt: inhalt,
                        zeitstempel: n["zeitstempel"] as? TimeInterval ?? 0)
                }
            }
            return Chat(sqliteId: id, titel: titel, nachrichten: nachrichten, createdAt: erstellt.timeIntervalSince1970)
        }
    }

    @discardableResult
    private func speichern(chat: Chat) -> Int64? {
        let nachrichten = chat.nachrichten.map { m -> [String: Any] in
            var dict: [String: Any] = ["rolle": m.rolle, "inhalt": m.inhalt, "zeitstempel": m.zeitstempel]
            return dict
        }
        let daten = (try? JSONSerialization.data(withJSONObject: nachrichten)) ?? Data()
        let json = String(data: daten, encoding: .utf8) ?? "[]"
        do {
            // Bestehender Chat -> UPDATE statt INSERT, sonst entstehen
            // bei jedem Speichern Duplikate in der Datenbank.
            if let id = chat.sqliteId {
                try historyAktualisieren(id: id, titel: chat.titel, nachrichten: json)
                return id
            }
            let neu = try historySpeichern(titel: chat.titel, nachrichten: json)
            if let idDaten = neu.data(using: .utf8),
               let obj = try? JSONSerialization.jsonObject(with: idDaten) as? [String: Int64],
               let id = obj["id"] {
                // ID im Chat verankern (indexbasiert, weil structs Werttypen sind).
                if let idx = chats.firstIndex(where: { $0.sqliteId == chat.sqliteId && $0.createdAt == chat.createdAt }) {
                    chats[idx].sqliteId = id
                }
                return id
            }
        } catch { /* History-Fehler blockieren den Chat nicht. */ }
        return nil
    }

    // ── Presets ──────────────────────────────────────────────────────

    private func ladePresets() {
        guard let json = try? presetsListe(),
              let daten = json.data(using: .utf8),
              let obj = try? JSONSerialization.jsonObject(with: daten) as? [String: Any] else { return }
        let rohPresets = obj["presets"] as? [[String: String]] ?? []
        presets = rohPresets.compactMap { p in
            guard let name = p["name"], let prompt = p["prompt"] else { return nil }
            return Preset(name: name, prompt: prompt)
        }
        aktivesPreset = obj["active"] as? String
    }

    func presetAktivieren(_ name: String) {
        _ = try? presetsAktivieren(name: name)
        ladePresets()
    }

    /// Speichert den Prompt unter dem aktiven Preset-Namen (wie die Tauri-GUI).
    func presetSpeichern(_ prompt: String) {
        let bereinigt = prompt.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !bereinigt.isEmpty, let name = aktivesPreset else { return }
        _ = try? presetsSpeichern(name: name, prompt: bereinigt)
        ladePresets()
    }

    /// Löscht das aktive Preset. Das letzte verbleibende Preset kann nicht
    /// gelöscht werden – der Kern verweigert das bereits, wir prüfen vor.
    func presetLoeschen() {
        guard let name = aktivesPreset, presets.count > 1 else { return }
        _ = try? presetsLoeschen(name: name)
        ladePresets()
    }

    // ── Modell ──────────────────────────────────────────────────────

    func ladeModelle() {
        Task {
            let modelle = await Task.detached { [provider = self.provider] in
                (try? modelleListe(provider: provider)) ?? "[]"
            }.value
            if let daten = modelle.data(using: .utf8),
               let roh = try? JSONSerialization.jsonObject(with: daten) as? [[String: Any]] {
                verfuegbareModelle = roh.compactMap { $0["id"] as? String }
            }
        }
    }

    func modellSetzen(_ modell: String) {
        Task {
            let neu = await Task.detached { [provider = self.provider, modell] in
                (try? setzeModell(provider: provider, model: modell)) ?? ""
            }.value
            zustandText = neu
            ausgewaehltesModell = modell
        }
    }

    // ── Status & Credits ─────────────────────────────────────────────

    private func aktualisiereStatus() {
        zustandText = (try? zustand()) ?? ""
    }

    private func aktualisiereCredits() {
        let p = provider
        Task {
            let neu = await Task.detached { (try? creditsFuerProvider(provider: p)) ?? "" }.value
            creditsText = neu
        }
    }

    /// Öffentliche Variante: wird beim Provider-Wechsel in der
    /// Kopfzeile aufgerufen, damit die Credits sofort den neuen
    /// Provider widerspiegeln (z. B. OpenRouter).
    func creditsAktualisieren(provider: String) {
        creditsText = "…"
        Task {
            let neu = await Task.detached { (try? creditsFuerProvider(provider: provider)) ?? "" }.value
            creditsText = neu
        }
    }
}
