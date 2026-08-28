// Famulus – Hauptansicht der nativen SwiftUI-Hülle v0.10.0.
// Aufbau wie die Tauri-GUI (ui/index.html): Kopfzeile mit Markenname
// und Modell-Auswahl, links die Chat-Sidebar, Mitte der Chat-Verlauf
// mit Eingabezeile, unten die Statusbar mit Zustand und Credits.

import SwiftUI

struct Hauptansicht: View {
    @State private var store = FamulusStore()
    @State private var linkeSidebarSichtbar = true

    var body: some View {
        VStack(spacing: 0) {
            Kopfzeile(store: store, linkeSidebarSichtbar: $linkeSidebarSichtbar)
            Divider().overlay(Marke.rand)
            HStack(spacing: 0) {
                if linkeSidebarSichtbar {
                    ChatSidebar(store: store)
                        .frame(width: 240)
                    Divider().overlay(Marke.rand)
                }
                ChatBereich(store: store)
            }
        }
        .background(Marke.verlauf)
        .preferredColorScheme(.dark)
        .task { store.laden() }
    }
}

// ── Kopfzeile ────────────────────────────────────────────────────────────

struct Kopfzeile: View {
    @Bindable var store: FamulusStore
    @Binding var linkeSidebarSichtbar: Bool

    var body: some View {
        HStack(spacing: 12) {
            Button {
                withAnimation(.easeInOut(duration: 0.15)) {
                    linkeSidebarSichtbar.toggle()
                }
            } label: {
                Image(systemName: "sidebar.left")
                    .font(.system(size: 13))
                    .foregroundStyle(linkeSidebarSichtbar ? Marke.akzent : Marke.textLeise)
            }
            .buttonStyle(.plain)

            HStack(spacing: 0) {
                Text("FAMULUS ")
                Text("v\(store.version)")
                    .foregroundStyle(Marke.textLeise)
                    .font(.system(size: 11))
            }
            .font(.system(size: 14, weight: .bold, design: .monospaced))
            .foregroundStyle(Marke.text)

            Spacer()

            // Provider-Dropdown
            Picker("", selection: $store.provider) {
                ForEach(["hyper", "openrouter", "ollama"], id: \.self) { p in
                    Text(p).tag(p)
                }
            }
            .pickerStyle(.menu)
            .frame(width: 130)
            .onChange(of: store.provider) { _, _ in
                store.verfuegbareModelle = []
                store.ladeModelle()
            }

            // Modell-Dropdown
            Picker("Modell", selection: $store.ausgewaehltesModell) {
                ForEach(store.verfuegbareModelle, id: \.self) { m in
                    Text(m).tag(m)
                }
            }
            .pickerStyle(.menu)
            .frame(maxWidth: 260)
            .onChange(of: store.ausgewaehltesModell) { _, neu in
                if !neu.isEmpty { store.modellSetzen(neu) }
            }
        }
        .padding(.horizontal, 16).padding(.vertical, 10)
        .background(Marke.kopfFläche)
    }
}

// ── Sidebar: Chat-Liste ──────────────────────────────────────────────────

struct ChatSidebar: View {
    @Bindable var store: FamulusStore

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                Text("Chats")
                    .font(.system(size: 13, weight: .semibold, design: .monospaced))
                    .foregroundStyle(Marke.textSekundär)
                Spacer()
                Button {
                    store.neuerChat()
                } label: {
                    Image(systemName: "plus")
                        .font(.system(size: 12))
                        .foregroundStyle(Marke.akzent)
                }
                .buttonStyle(.plain)
            }
            .padding(12)

            Divider().overlay(Marke.rand)

            ScrollView {
                LazyVStack(spacing: 2) {
                    ForEach(Array(store.chats.enumerated()), id: \.element.id) { index, chat in
                        ChatZeile(
                            chat: chat,
                            aktiv: index == store.aktiverChatIndex,
                            wahl: { store.chatWählen(index) },
                            loeschen: { store.chatLöschen(index) },
                            archivieren: { store.chatArchivieren(index, archiviert: true) })
                    }
                }
                .padding(8)
            }

            if !store.archiv.isEmpty {
                Divider().overlay(Marke.rand)
                Text("Archiv")
                    .font(.system(size: 11, weight: .semibold, design: .monospaced))
                    .foregroundStyle(Marke.textLeise)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, 12).padding(.vertical, 6)
                ScrollView {
                    LazyVStack(spacing: 2) {
                        ForEach(store.archiv, id: \.id) { chat in
                            ArchivZeile(chat: chat)
                        }
                    }
                    .padding(8)
                }
                .frame(maxHeight: 160)
            }
        }
        .background(Marke.seitenLeiste)
    }
}

struct ChatZeile: View {
    let chat: Chat
    let aktiv: Bool
    let wahl: () -> Void
    let loeschen: () -> Void
    let archivieren: () -> Void
    @State private var hover = false

    var body: some View {
        Button(action: wahl) {
            Text(chat.titel)
                .font(.system(size: 12, design: .monospaced))
                .foregroundStyle(aktiv ? Marke.akzent : Marke.textSekundär)
                .lineLimit(1)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, 8).padding(.vertical, 6)
                .background(
                    RoundedRectangle(cornerRadius: 4)
                        .fill(aktiv ? Marke.akzentGetönt : (hover ? Marke.hover : .clear)))
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { hover = $0 }
        .contextMenu {
            Button("Archivieren", action: archivieren)
            Button("Löschen", role: .destructive, action: loeschen)
        }
    }
}

struct ArchivZeile: View {
    let chat: Chat
    @State private var hover = false

    var body: some View {
        Text(chat.titel)
            .font(.system(size: 11, design: .monospaced))
            .foregroundStyle(Marke.textLeise)
            .lineLimit(1)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 8).padding(.vertical, 4)
            .background(RoundedRectangle(cornerRadius: 4).fill(hover ? Marke.hover : .clear))
            .onHover { hover = $0 }
    }
}

// ── Chat-Bereich: Nachrichten, Live-Schritte, Eingabe, Statusbar ────────

struct ChatBereich: View {
    @Bindable var store: FamulusStore
    @State private var eingabe = ""
    @FocusState private var eingabeFokus: Bool

    var body: some View {
        VStack(spacing: 0) {
            ScrollViewReader { proxy in
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 14) {
                        ForEach(store.aktiverChat.nachrichten) { nachricht in
                            NachrichtenZeile(nachricht: nachricht)
                                .id(nachricht.id)
                        }
                        if store.beschaeftigt {
                            LiveBlock(store: store)
                                .id("live")
                        }
                    }
                    .padding(20)
                }
                .onChange(of: store.denktText) { _, _ in
                    withAnimation { proxy.scrollTo("live", anchor: .bottom) }
                }
                .onChange(of: store.aktiverChat.nachrichten.count) { _, _ in
                    withAnimation { proxy.scrollTo("live", anchor: .bottom) }
                }
            }

            Divider().overlay(Marke.rand)

            eingabeZeile
            statusBar
        }
    }

    // ── Eingabezeile ──
    private var eingabeZeile: some View {
        HStack(spacing: 8) {
            TextField("Schreib Famulus einen Auftrag…", text: $eingabe, axis: .vertical)
                .textFieldStyle(.plain)
                .font(.system(size: 13, design: .monospaced))
                .foregroundStyle(Marke.text)
                .focused($eingabeFokus)
                .onSubmit { senden() }
                .padding(10)
                .background(RoundedRectangle(cornerRadius: 6).fill(Marke.eingabe))
                .overlay(RoundedRectangle(cornerRadius: 6).stroke(Marke.rand, lineWidth: 1))

            if store.beschaeftigt {
                Button(action: store.stoppen) {
                    Image(systemName: "xmark")
                        .font(.system(size: 12, weight: .bold))
                        .foregroundStyle(Marke.gefahr)
                        .padding(8)
                }
                .buttonStyle(.plain)
            }

            Button(action: senden) {
                Image(systemName: "arrow.up")
                    .font(.system(size: 12, weight: .bold))
                    .foregroundStyle(eingabe.trimmingCharacters(in: .whitespaces).isEmpty || store.beschaeftigt
                        ? Marke.textHauch : Marke.akzent)
                    .padding(8)
            }
            .buttonStyle(.plain)
        }
        .padding(.horizontal, 16).padding(.vertical, 10)
    }

    private func senden() {
        let text = eingabe
        guard !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
        eingabe = ""
        store.senden(text)
    }

    // ── Statusbar ──
    private var statusBar: some View {
        HStack(spacing: 12) {
            Text(store.status)
                .foregroundStyle(Marke.textLeise)
            if store.beschaeftigt {
                ProgressView()
                    .controlSize(.small)
                    .tint(Marke.akzent)
            }
            Spacer()
            Text(store.zustandText)
                .foregroundStyle(Marke.textSekundär)
            Text(store.creditsText)
                .foregroundStyle(Marke.erfolg)
        }
        .font(.system(size: 11, design: .monospaced))
        .padding(.horizontal, 16).padding(.vertical, 7)
        .background(Marke.fußFläche)
    }
}

// ── Nachrichten ──────────────────────────────────────────────────────────

struct NachrichtenZeile: View {
    let nachricht: ChatNachricht

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 8) {
                Text(rolle)
                    .font(.system(size: 10, weight: .semibold, design: .monospaced))
                    .foregroundStyle(farbe)
                Text(formatZeit(nachricht.zeitstempel))
                    .font(.system(size: 9, design: .monospaced))
                    .foregroundStyle(Marke.textHauch)
            }
            Text(nachricht.inhalt)
                .font(.system(size: 13, design: .monospaced))
                .foregroundStyle(nachricht.rolle == "fehler" ? Marke.gefahr : Marke.text)
                .textSelection(.enabled)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(12)
        .background(
            RoundedRectangle(cornerRadius: 8)
                .fill(nachricht.rolle == "user" ? Marke.fläche : Marke.eingabe))
        .overlay(
            RoundedRectangle(cornerRadius: 8)
                .stroke(nachricht.rolle == "fehler" ? Marke.gefahr.opacity(0.4) : Marke.rand, lineWidth: 1))
    }

    private var rolle: String {
        switch nachricht.rolle {
        case "user": return "Du"
        case "fehler": return "Fehler"
        default: return "Famulus"
        }
    }

    private var farbe: Color {
        switch nachricht.rolle {
        case "user": return Marke.akzent
        case "fehler": return Marke.gefahr
        default: return Marke.erfolg
        }
    }

    private func formatZeit(_ ts: TimeInterval) -> String {
        guard ts > 0 else { return "" }
        let datum = Date(timeIntervalSince1970: ts / 1000)
        let f = DateFormatter()
        f.dateFormat = "HH:mm"
        return f.string(from: datum)
    }
}

// ── Live-Block (während ein Auftrag läuft) ───────────────────────────────

struct LiveBlock: View {
    @Bindable var store: FamulusStore

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            if !store.denktText.isEmpty {
                Text("… " + store.denktText)
                    .font(.system(size: 13, design: .monospaced))
                    .foregroundStyle(Marke.textSekundär)
                    .textSelection(.enabled)
            }
            ForEach(store.agentSchritte) { schritt in
                SchrittZeile(schritt: schritt)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(12)
        .background(RoundedRectangle(cornerRadius: 8).fill(Marke.eingabe.opacity(0.6)))
        .overlay(RoundedRectangle(cornerRadius: 8).stroke(Marke.rand, lineWidth: 1))
    }
}

struct SchrittZeile: View {
    let schritt: Schritt
    @State private var offen = false

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Button {
                offen.toggle()
            } label: {
                HStack(spacing: 6) {
                    Image(systemName: schritt.art == .werkzeug ? "gearshape" : "bookmark")
                        .font(.system(size: 10))
                    Text(schritt.art == .werkzeug ? schritt.name : schritt.text)
                        .lineLimit(offen ? nil : 1)
                }
                .font(.system(size: 11, design: .monospaced))
                .foregroundStyle(Marke.textLeise)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .buttonStyle(.plain)

            if offen, let ergebnis = schritt.ergebnis {
                Text(String(ergebnis.prefix(4000)))
                    .font(.system(size: 10, design: .monospaced))
                    .foregroundStyle(Marke.textHauch)
                    .textSelection(.enabled)
                    .padding(6)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(RoundedRectangle(cornerRadius: 4).fill(Marke.verlaufOben))
            }
        }
    }
}
