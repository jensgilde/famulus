//! Anbieter, die das Anthropic-Messages-Protokoll sprechen.
//!
//! Das sind Anthropic selbst und alles, was dessen `/v1/messages`-Format
//! nachbaut - zum Beispiel Charm Hyper. Der Unterschied steckt nur in
//! Adresse, Key und Standardmodell, nicht im Code darunter.

use super::{
    http_client_ohne_gesamttimeout, require_api_key, send_mit_retry, LlmAntwort, LlmProvider,
    Message, ToolCall, ToolDefinition,
};
use anyhow::{Context, Result};
use async_trait::async_trait;
use futures_util::StreamExt;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::Duration;

pub struct AnthropicProvider {
    api_key: String,
    model: String,
    endpoint: String,
    label: &'static str,
    max_tokens: u32,
    client: reqwest::Client,
    /// Inaktivitätsgrenze für den Antwort-Strom: Solange Daten fließen, darf
    /// die Antwort beliebig lange dauern. Kommt länger als diese Zeit nichts,
    /// gilt die Verbindung als tot. Das ersetzt den alten Gesamt-Timeout, der
    /// lange, aber gesunde Antworten abgewürgt hat.
    pause_limit: Duration,
}

impl AnthropicProvider {
    pub fn neu(
        label: &'static str,
        model: String,
        base_url: String,
        api_key_env: &str,
        max_tokens: u32,
        timeout: Duration,
    ) -> Result<Self> {
        Ok(Self {
            api_key: require_api_key(api_key_env)?,
            model,
            endpoint: format!("{base_url}/v1/messages"),
            label,
            max_tokens,
            client: http_client_ohne_gesamttimeout()?,
            pause_limit: timeout,
        })
    }
}

/// Übersetzt den Verlauf in Anthropics Block-Format.
fn nachrichten_bauen(messages: &[Message]) -> Vec<Value> {
    messages
        .iter()
        .map(|m| match m {
            Message::User(text) => json!({
                "role": "user",
                "content": [{ "type": "text", "text": text }],
            }),
            Message::UserMitBild { text, bilder } => {
                let mut bloecke: Vec<Value> = Vec::new();
                // Text kommt zuerst, dann die Bilder
                if !text.trim().is_empty() {
                    bloecke.push(json!({ "type": "text", "text": text }));
                }
                for bild in bilder {
                    bloecke.push(json!({
                        "type": "image",
                        "source": {
                            "type": "base64",
                            "media_type": bild.medien_typ,
                            "data": bild.base64,
                        },
                    }));
                }
                json!({ "role": "user", "content": bloecke })
            }
            Message::Assistant { text, tool_calls } => {
                let mut bloecke = Vec::new();
                // Leere Textblöcke lehnt die API ab - nur anhängen, wenn
                // wirklich etwas drinsteht.
                if !text.trim().is_empty() {
                    bloecke.push(json!({ "type": "text", "text": text }));
                }
                for call in tool_calls {
                    bloecke.push(json!({
                        "type": "tool_use",
                        "id": call.id,
                        "name": call.name,
                        "input": call.arguments,
                    }));
                }
                json!({ "role": "assistant", "content": bloecke })
            }
            // Werkzeug-Ergebnisse sind protokollarisch Nachrichten des
            // Nutzers - das Modell hat gefragt, die Umgebung antwortet.
            Message::ToolResults(ergebnisse) => {
                let bloecke: Vec<Value> = ergebnisse
                    .iter()
                    .map(|e| {
                        json!({
                            "type": "tool_result",
                            "tool_use_id": e.call_id,
                            "content": e.inhalt,
                            "is_error": e.fehler,
                        })
                    })
                    .collect();
                json!({ "role": "user", "content": bloecke })
            }
        })
        .collect()
}

/// Setzt die Cache-Marke auf den letzten Block der letzten Nachricht.
///
/// Zwischenspeichern funktioniert über den gemeinsamen Anfang: Der Anbieter
/// merkt sich alles bis zur Marke und muss es im nächsten Zug nicht neu
/// verarbeiten. In einer Schleife mit zwanzig Zügen ist das der Unterschied
/// zwischen "der Verlauf wird zwanzigmal bezahlt" und "einmal".
fn cache_marke_setzen(nachrichten: &mut [Value]) {
    let Some(letzte) = nachrichten.last_mut() else {
        return;
    };
    let Some(bloecke) = letzte.get_mut("content").and_then(Value::as_array_mut) else {
        return;
    };
    if let Some(block) = bloecke.last_mut() {
        if let Some(objekt) = block.as_object_mut() {
            objekt.insert("cache_control".to_string(), json!({ "type": "ephemeral" }));
        }
    }
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    fn name(&self) -> &'static str {
        self.label
    }

    async fn next(
        &self,
        system: Option<&str>,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> Result<LlmAntwort> {
        let anthropic_tools: Vec<Value> = tools
            .iter()
            .map(|t| {
                json!({
                    "name": t.name,
                    "description": t.description,
                    "input_schema": t.parameters_schema,
                })
            })
            .collect();

        let mut anthropic_messages = nachrichten_bauen(messages);
        cache_marke_setzen(&mut anthropic_messages);

        let mut body = json!({
            "model": self.model,
            "max_tokens": self.max_tokens,
            "messages": anthropic_messages,
            "stream": true,
        });

        if !anthropic_tools.is_empty() {
            body["tools"] = json!(anthropic_tools);
        }

        // Der Vorspann steht vor dem Gespräch und ändert sich während eines
        // Auftrags nicht. Die zweite Cache-Marke sitzt deshalb hier: sie
        // deckt Werkzeug-Beschreibungen UND Vorspann ab, weil beides vor den
        // Nachrichten gerendert wird.
        if let Some(text) = system {
            body["system"] = json!([{
                "type": "text",
                "text": text,
                "cache_control": { "type": "ephemeral" },
            }]);
        }

        // Anthropic selbst authentifiziert über "x-api-key", kompatible
        // Dienste erwarten teils "Authorization: Bearer". Beides mitschicken
        // kostet nichts - es ist derselbe Key an denselben Host - und spart
        // eine Konfigurationsoption, die man falsch setzen könnte.
        let resp = send_mit_retry(|| {
            self.client
                .post(&self.endpoint)
                .header("x-api-key", &self.api_key)
                .bearer_auth(&self.api_key)
                .header("anthropic-version", "2023-06-01")
                .json(&body)
        })
        .await
        .with_context(|| format!("Anfrage an {} fehlgeschlagen", self.endpoint))?;

        // Fehler kommen nicht gestreamt, sondern als normales JSON-Body.
        if !resp.status().is_success() {
            let status = resp.status();
            let value: Value = resp
                .json()
                .await
                .unwrap_or_else(|_| json!({"fehler": "Antwort war kein gültiges JSON"}));
            anyhow::bail!("API-Fehler von {} ({status}): {value}", self.endpoint);
        }

        // Bis hierher greift noch der Client-Timeout (Verbindungsaufbau +
        // Statuszeile). Ab jetzt zählt nur die Inaktivitätsgrenze pro Stück.
        antwort_aus_strom(resp.bytes_stream(), self.pause_limit)
            .await
            .with_context(|| format!("Antwortstrom von {} unvollständig", self.endpoint))
    }
}

/// Zerlegt den SSE-Antwortstrom eines Anthropic-kompatiblen Anbieters in
/// das, was Famulus braucht: gesammelter Text und vollständige Tool-Aufrufe.
///
/// Warum Stück für Stück mit eigener Wartezeit statt einem einzigen
/// Gesamt-Timeout: Der Server liefert bei langen Antworten durchaus
/// ununterbrochen kleine Happen - der alte Client-Timeout hat solche
/// gesunden, aber langsamen Antworten trotzdem nach Ablauf abgewürgt. So
/// stirbt die Anfrage nur, wenn der Strom wirklich stockt (länger als
/// `pause_limit` kein Byte).
///
/// Die Ereignis-Namen folgen dem Anthropic-Streaming-Protokoll
/// (content_block_start/_delta/_stop, message_delta). Kompatible Dienste wie
/// Hyper halten sich daran; ein Dienst, der `stream: true` ignoriert und
/// trotzdem ein JSON-Objekt schickt, würde hier scheitern - dann ist der
/// Anbieter aber ohnehin nicht Messages-API-kompatibel.
async fn antwort_aus_strom(
    mut strom: impl futures_util::Stream<Item = reqwest::Result<bytes::Bytes>> + Unpin,
    pause_limit: Duration,
) -> Result<LlmAntwort> {
    let mut text_teile: Vec<String> = Vec::new();
    // Tool-Aufrufe kommen gestückelt: Anfang mit id/name, dann JSON-Fragmente
    // in input_json_delta. Zusammensetzen über den Block-Index.
    let mut offene_calls: HashMap<u64, ToolCall> = HashMap::new();
    let mut json_fragemente: HashMap<u64, String> = HashMap::new();

    let mut puffer = String::new();

    loop {
        let stueck = match tokio::time::timeout(pause_limit, strom.next()).await {
            Ok(Some(Ok(stueck))) => stueck,
            Ok(Some(Err(fehler))) => return Err(anyhow::anyhow!(fehler)),
            Ok(None) => anyhow::bail!("Antwortstrom endete ohne message_stop-Ereignis"),
            Err(_) => anyhow::bail!("keine Daten mehr vom Anbieter innerhalb der Wartezeit"),
        };

        puffer.push_str(&String::from_utf8_lossy(&stueck));

        // SSE-Trenner ist eine Leerzeile; ein Ereignis kann über mehrere
        // Netzpakete verteilt sein, deshalb erst verarbeiten, wenn es ganz da ist.
        while let Some(trenn) = puffer.find("\n\n") {
            let ereignis = puffer[..trenn].to_string();
            puffer = puffer[trenn + 2..].to_string();
            if verarbeite_sse(&ereignis, &mut text_teile, &mut offene_calls, &mut json_fragemente)? {
                return fertig_bauen(text_teile, offene_calls, json_fragemente);
            }
        }
    }
}

/// Baut aus den gesammelten Stücken die fertige Antwort - aufgerufen, sobald
/// `message_stop` kam. Ein Tool-Aufruf ohne gültiges Argument-JSON ist ein
/// Protokollfehler des Anbieters und muss auffallen, nicht still übersprungen werden.
fn fertig_bauen(
    text_teile: Vec<String>,
    mut offene_calls: HashMap<u64, ToolCall>,
    json_fragemente: HashMap<u64, String>,
) -> Result<LlmAntwort> {
    let mut antwort = LlmAntwort::default();
    for (index, fragment) in json_fragemente {
        let call = offene_calls
            .remove(&index)
            .ok_or_else(|| anyhow::anyhow!("JSON-Deltas ohne zugehörigen tool_use-Anfang"))?;
        let mut call = call;
        // Ein Tool ohne Argumente liefert gar keine input_json_delta-Fragmente;
        // der leere Sammelstring wäre dann kein gültiges JSON. "{}" ist die
        // protokollgemäße Bedeutung: Werkzeug ohne Eingaben.
        let roh = if fragment.trim().is_empty() { "{}" } else { fragment.as_str() };
        call.arguments = serde_json::from_str(roh).with_context(|| {
            format!("Werkzeug-Aufruf '{}' lieferte kein gültiges Argument-JSON", call.name)
        })?;
        antwort.tool_calls.push(call);
    }
    antwort.text = text_teile.join("");
    Ok(antwort)
}

/// Verarbeitet ein SSE-Ereignis. `true` zurückgeben heißt: `message_stop`
/// gesehen, die Antwort ist komplett.
fn verarbeite_sse(
    ereignis: &str,
    text_teile: &mut Vec<String>,
    offene_calls: &mut HashMap<u64, ToolCall>,
    json_fragemente: &mut HashMap<u64, String>,
) -> Result<bool> {
    for zeile in ereignis.lines() {
        let Some(daten) = zeile.strip_prefix("data:") else {
            continue; // "event:"-Zeilen, Kommentare, Leerzeilen: egal
        };
        let daten = daten.trim();
        if daten == "[DONE]" {
            continue; // OpenAI-Erbstück einiger kompatibler Dienste
        }
        let Ok(value) = serde_json::from_str::<Value>(daten) else {
            continue; // kein JSON = kein nutzbares Ereignis
        };
        match value["type"].as_str() {
            Some("content_block_start") => {
                let index = value["index"].as_u64().unwrap_or(0);
                let block = &value["content_block"];
                if block["type"].as_str() == Some("tool_use") {
                    offene_calls.insert(
                        index,
                        ToolCall {
                            id: block["id"].as_str().unwrap_or_default().to_string(),
                            name: block["name"].as_str().unwrap_or_default().to_string(),
                            arguments: Value::Null, // kommt über die Deltas
                        },
                    );
                    json_fragemente.insert(index, String::new());
                }
            }
            Some("content_block_delta") => {
                let index = value["index"].as_u64().unwrap_or(0);
                let delta = &value["delta"];
                match delta["type"].as_str() {
                    Some("text_delta") => {
                        if let Some(t) = delta["text"].as_str() {
                            text_teile.push(t.to_string());
                        }
                    }
                    Some("input_json_delta") => {
                        if let Some(fragment) = delta["partial_json"].as_str() {
                            json_fragemente.entry(index).or_default().push_str(fragment);
                        }
                    }
                    _ => {} // thinking_delta u. Ä.: übergehen
                }
            }
            Some("message_stop") => return Ok(true),
            // message_start, ping, content_block_stop, message_delta,
            // "error" als Ereignis-Typ: keine für uns nutzbaren Daten.
            _ => {}
        }
    }
    Ok(false)
}
#[cfg(test)]
mod tests {
    use super::*;

    /// Wandelt Text-Stücke in einen reqwest-artigen Byte-Stream um, wie ihn
    /// die echte Antwort liefert - damit lassen sich auch Zerlegungen über
    /// Paketgrenzen hinweg prüfen.
    fn strom(stuecke: Vec<&str>) -> impl futures_util::Stream<Item = reqwest::Result<bytes::Bytes>> {
        futures_util::stream::iter(
            stuecke
                .into_iter()
                .map(|s| Ok(bytes::Bytes::from(s.to_string())))
                .collect::<Vec<_>>(),
        )
    }

    /// Ein echter Stream-Ausschnitt im Format von hyper.charm.land
    /// (nachgemessen am 2026-08-23 gegen den echten Endpunkt):
    /// message_start, ping, thinking-Block, Textblock in Stücken, message_stop.
    const TEXT_STROM: &[&str] = &[
        "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"role\":\"assistant\"}}\n\n",
        "event: ping\ndata: {\"type\":\"ping\"}\n\n",
        "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"thinking\"}}\n\n",
        "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"interner Gedanke\"}}\n\n",
        "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
        "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
        "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hallo \"}}\n\n",
        "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"text_delta\",\"text\":\"Welt\"}}\n\n",
        "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
    ];

    #[tokio::test]
    async fn text_wird_ueber_deltas_zusammengesetzt_thinking_ignoriert() {
        let antwort = antwort_aus_strom(strom(TEXT_STROM.to_vec()), Duration::from_secs(5))
            .await
            .unwrap();
        assert_eq!(antwort.text, "Hallo Welt");
        assert!(antwort.tool_calls.is_empty());
        assert!(antwort.ist_fertig());
    }

    #[tokio::test]
    async fn ereignis_ueber_chunk_grenze_wird_trotzdem_verarbeitet() {
        // Beide message_stop-Hälften kommen in getrennten Netzstücken an;
        // der Puffer muss sie zusammenfügen, bevor er verarbeitet.
        let stuecke = vec![
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"X\"}}\n\nevent: mes",
            "sage_stop\ndata: {\"type\":\"message_stop\"}\n\n",
        ];
        let antwort = antwort_aus_strom(strom(stuecke), Duration::from_secs(5))
            .await
            .unwrap();
        assert_eq!(antwort.text, "X");
    }

    #[tokio::test]
    async fn tool_aufruf_wird_aus_json_fragmenten_zusammengesetzt() {
        // Nachbildung eines echten Hyper-Streams mit Werkzeug-Aufruf:
        let stuecke = vec![
            "data: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_1\",\"name\":\"wetter\",\"input\":{}}}\n\n",
            "data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"stadt\\\": \"}}\n\n",
            "data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"\\\"Berlin\\\"}\"}}\n\n",
            "data: {\"type\":\"message_stop\"}\n\n",
        ];
        let antwort = antwort_aus_strom(strom(stuecke), Duration::from_secs(5))
            .await
            .unwrap();
        assert_eq!(antwort.tool_calls.len(), 1);
        let call = &antwort.tool_calls[0];
        assert_eq!(call.id, "toolu_1");
        assert_eq!(call.name, "wetter");
        assert_eq!(call.arguments["stadt"], "Berlin");
        assert!(!antwort.ist_fertig());
    }

    #[tokio::test]
    async fn tool_ohne_argumente_bekommt_leeres_objekt() {
        // Ein Werkzeug ganz ohne Eingaben liefert keine input_json_delta-
        // Fragmente - darf trotzdem nicht an leerem Sammelstring scheitern.
        let stuecke = vec![
            "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_2\",\"name\":\"status\",\"input\":{}}}\n\n",
            "data: {\"type\":\"message_stop\"}\n\n",
        ];
        let antwort = antwort_aus_strom(strom(stuecke), Duration::from_secs(5))
            .await
            .unwrap();
        assert_eq!(antwort.tool_calls.len(), 1);
        assert_eq!(antwort.tool_calls[0].arguments, json!({}));
    }

    #[tokio::test]
    async fn strom_ende_ohne_message_stop_ist_ein_fehler() {
        // Netzabbruch mitten in der Antwort: kein message_stop, Stream zu.
        let stuecke = vec![
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Anfang...\"}}\n\n",
        ];
        let ergebnis = antwort_aus_strom(strom(stuecke), Duration::from_secs(5)).await;
        assert!(ergebnis.is_err());
    }

    #[tokio::test]
    async fn pause_ohne_daten_bricht_ab() {
        // Ein Stream, der nach einem Delta nichts mehr liefert: Die
        // Inaktivitätswache (hier extra kurz) muss zuschlagen.
        let unendlich = futures_util::stream::iter(vec![Ok(bytes::Bytes::from(
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"...\"}}\n\n".to_string(),
        ))])
        .chain(futures_util::stream::pending());
        let ergebnis = antwort_aus_strom(unendlich, Duration::from_millis(100)).await;
        let fehler = ergebnis.expect_err("muss wegen Inaktivität abbrechen");
        assert!(fehler.to_string().contains("Wartezeit"), "unerwarteter Fehler: {fehler}");
    }
}
