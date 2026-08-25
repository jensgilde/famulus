// Fernbedienungs-Protokoll: die Nachrichten, die über den WebSocket auf
// Port 9876 laufen.
//
// Dieser Typensatz ist die EINZIGE Quelle der Wahrheit für das Protokoll.
// Früher lebten die Definitionen nur in der GUI (gui/src/remote.rs) - seit
// der Telegram-Bot (eigenes Binary, eigenes Crate) dieselbe Fernbedienung
// nutzt, liegen sie hier im Kern. So kann keine zweite Kopie entstehen,
// die still auseinanderläuft (dasselbe Prinzip wie beim Kern selbst, siehe
// lib.rs-Kopfkommentar).

use crate::ui::AgentEvent;
use serde::{Deserialize, Serialize};

/// Der Port der Fernbedienung auf dem Mac.
pub const SERVER_PORT: u16 = 9876;

#[derive(Serialize, Deserialize)]
#[serde(tag = "typ")]
pub enum RemoteRequest {
    #[serde(rename = "auftrag")]
    Auftrag {
        auftrag: String,
        verlauf: Vec<RemoteVerlaufEintrag>,
    },
    #[serde(rename = "zustand")]
    Zustand,
    #[serde(rename = "credits")]
    Credits,
    #[serde(rename = "modelle")]
    Modelle { provider: String },
    #[serde(rename = "setze_modell")]
    SetzeModell { provider: String, model: String },
    #[serde(rename = "setze_modell_modus")]
    SetzeModellModus { modus: String },
    /// Schickt eine Zwischenfrage an den gerade auf dem Mac laufenden
    /// Auftrag, ohne ihn abzubrechen.
    #[serde(rename = "zwischenfrage")]
    Zwischenfrage { text: String },
    #[serde(rename = "version")]
    Version,
    #[serde(rename = "presets_liste")]
    PresetsListe,
    #[serde(rename = "presets_aktivieren")]
    PresetsAktivieren { name: String },
    #[serde(rename = "presets_speichern")]
    PresetsSpeichern { name: String, prompt: String },
    #[serde(rename = "presets_loeschen")]
    PresetsLoeschen { name: String },
    /// Bricht den aktuell auf dem Mac laufenden Auftrag ab.
    #[serde(rename = "abbrechen")]
    Abbrechen,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct RemoteVerlaufEintrag {
    pub rolle: String,
    pub inhalt: String,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "typ")]
pub enum RemoteResponse {
    #[serde(rename = "event")]
    Event { event: AgentEvent },
    #[serde(rename = "zustand")]
    Zustand { zustand: String },
    #[serde(rename = "credits")]
    Credits { credits: String },
    #[serde(rename = "modelle")]
    Modelle { modelle: serde_json::Value },
    #[serde(rename = "version")]
    Version { version: String },
    #[serde(rename = "presets")]
    Presets { presets: serde_json::Value },
    #[serde(rename = "ok")]
    Ok,
    #[serde(rename = "error")]
    Error { fehler: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Der Bot und die GUI müssen dasselbe JSON sprechen. Dieser Test
    /// friert die Drahtformate ein: weicht je ein `rename` oder das
    /// `tag`-Feld ab, verstehen sich die Geräte nicht mehr - lautlos,
    /// denn eine fehlgeschlagene Deserialisierung fällt meist nur als
    /// "unerwartete Antwort" auf.
    #[test]
    fn drahtformat_anfrage_stabil() {
        let json = serde_json::to_value(RemoteRequest::Auftrag {
            auftrag: "mach was".into(),
            verlauf: vec![RemoteVerlaufEintrag {
                rolle: "user".into(),
                inhalt: "früher".into(),
            }],
        })
        .unwrap();
        assert_eq!(json["typ"], "auftrag");
        assert_eq!(json["auftrag"], "mach was");
        assert_eq!(json["verlauf"][0]["rolle"], "user");

        let json = serde_json::to_value(RemoteRequest::Zwischenfrage {
            text: "wie weit?".into(),
        })
        .unwrap();
        assert_eq!(json["typ"], "zwischenfrage");

        let json = serde_json::to_value(RemoteRequest::Abbrechen).unwrap();
        assert_eq!(json["typ"], "abbrechen");
    }

    #[test]
    fn drahtformat_antwort_stabil() {
        let json = serde_json::to_value(RemoteResponse::Event {
            event: AgentEvent::Text { chunk: "hi".into() },
        })
        .unwrap();
        assert_eq!(json["typ"], "event");
        assert_eq!(json["event"]["art"], "text");

        let json = serde_json::to_value(RemoteResponse::Zustand {
            zustand: "ok".into(),
        })
        .unwrap();
        assert_eq!(json["typ"], "zustand");

        // Und andersherum: was der Server schickt, muss der Client verstehen.
        let roh = r#"{"typ":"credits","credits":"42"}"#;
        let antwort: RemoteResponse = serde_json::from_str(roh).unwrap();
        match antwort {
            RemoteResponse::Credits { credits } => assert_eq!(credits, "42"),
            _ => panic!("falsche Variante"),
        }
    }
}
