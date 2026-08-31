//! Anbieter, die das OpenAI-Chat-Completions-Protokoll sprechen.
//!
//! Also OpenAI selbst, xAI (Grok) und OpenRouter. Vorher stand das zweimal
//! fast wörtlich identisch in `grok.rs` und `openrouter.rs` - die
//! Unterschiede sind Adresse, Key, Standardmodell und höchstens ein paar
//! Zusatz-Kopfzeilen, und dafür braucht es keine zweite Datei.
//!
//! Der Unterschied zum Anthropic-Format: Werkzeug-Aufrufe heißen hier
//! "function calling", die Argumente kommen als JSON-*Zeichenkette* statt als
//! Objekt, und jedes Ergebnis ist eine eigene Nachricht mit `role: "tool"`.

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

pub struct OpenAiProvider {
    temperature: Option<f32>,
    api_key: String,
    model: String,
    endpoint: String,
    label: &'static str,
    /// Zusätzliche Kopfzeilen, die manche Dienste sehen wollen (OpenRouter
    /// zeigt damit an, welche Anwendung anfragt).
    kopfzeilen: Vec<(&'static str, &'static str)>,
    max_tokens: u32,
    client: reqwest::Client,
    /// Inaktivitätsgrenze für den Antwort-Strom, siehe
    /// `anthropic.rs::AnthropicProvider::pause_limit` - dieselbe Begründung:
    /// eine lange, aber gesund fließende Antwort darf beliebig lange
    /// dauern, nur Stille länger als diese Zeit gilt als toter Anbieter.
    pause_limit: Duration,
}

impl OpenAiProvider {
    pub fn neu(
        temperature: Option<f32>,
        label: &'static str,
        model: String,
        base_url: String,
        api_key_env: &str,
        kopfzeilen: Vec<(&'static str, &'static str)>,
        max_tokens: u32,
        timeout: Duration,
    ) -> Result<Self> {
        Ok(Self {
            api_key: require_api_key(api_key_env)?,
            model,
            endpoint: format!("{base_url}/v1/chat/completions"),
            label,
            kopfzeilen,
            max_tokens,
            client: http_client_ohne_gesamttimeout()?,
            pause_limit: timeout,
            temperature,
        })
    }

}

/// Übersetzt den Verlauf ins OpenAI-Format.
fn nachrichten_bauen(system: Option<&str>, messages: &[Message]) -> Vec<Value> {
    let mut raus = Vec::new();

    // Der Vorspann ist hier keine eigene Zutat, sondern die erste Nachricht.
    if let Some(text) = system {
        raus.push(json!({ "role": "system", "content": text }));
    }

    for m in messages {
        match m {
            Message::User(text) => raus.push(json!({ "role": "user", "content": text })),
            Message::UserMitBild { text, bilder } => {
                // OpenAI Vision: content ist ein Array aus Text- und Bild-Blöcken
                let mut bloecke: Vec<Value> = Vec::new();
                if !text.trim().is_empty() {
                    bloecke.push(json!({ "type": "text", "text": text }));
                }
                for bild in bilder {
                    bloecke.push(json!({
                        "type": "image_url",
                        "image_url": {
                            "url": format!("data:{};base64,{}", bild.medien_typ, bild.base64),
                        },
                    }));
                }
                raus.push(json!({ "role": "user", "content": bloecke }));
            }
            Message::Assistant { text, tool_calls } => {
                let mut nachricht = json!({ "role": "assistant" });
                // `content` muss vorhanden sein, darf aber null sein, wenn
                // das Modell nur Werkzeuge aufrufen wollte.
                nachricht["content"] = if text.trim().is_empty() {
                    Value::Null
                } else {
                    json!(text)
                };
                if !tool_calls.is_empty() {
                    nachricht["tool_calls"] = json!(tool_calls
                        .iter()
                        .map(|c| json!({
                            "id": c.id,
                            "type": "function",
                            "function": {
                                "name": c.name,
                                // Hier verlangt das Protokoll eine Zeichenkette,
                                // kein Objekt - deshalb der Umweg über to_string.
                                "arguments": c.arguments.to_string(),
                            }
                        }))
                        .collect::<Vec<_>>());
                }
                raus.push(nachricht);
            }
            // Anders als bei Anthropic bekommt jedes Ergebnis eine eigene
            // Nachricht, zugeordnet über die tool_call_id.
            Message::ToolResults(ergebnisse) => {
                for e in ergebnisse {
                    raus.push(json!({
                        "role": "tool",
                        "tool_call_id": e.call_id,
                        "content": e.inhalt,
                    }));
                }
            }
        }
    }

    raus
}

#[async_trait]
impl LlmProvider for OpenAiProvider {
    fn name(&self) -> &'static str {
        self.label
    }

    async fn next(
        &self,
        system: Option<&str>,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> Result<LlmAntwort> {
        let openai_tools: Vec<Value> = tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.parameters_schema,
                    }
                })
            })
            .collect();

        let mut body = json!({
            "model": self.model,
            "max_tokens": self.max_tokens,
            "messages": nachrichten_bauen(system, messages),
            "stream": true,
        });
        if !openai_tools.is_empty() {
            body["tools"] = json!(openai_tools);
        }

        if let Some(t) = self.temperature {
            body["temperature"] = json!(t);
        }

        let resp = send_mit_retry(|| {
            let mut anfrage = self.client.post(&self.endpoint).bearer_auth(&self.api_key);
            for (name, wert) in &self.kopfzeilen {
                anfrage = anfrage.header(*name, *wert);
            }
            anfrage.json(&body)
        })
        .await
        .with_context(|| format!("Anfrage an {} fehlgeschlagen", self.endpoint))?;

        // Status VOR dem JSON-Parsen prüfen: ein Fehler-Body ist nicht
        // garantiert gültiges JSON (z.B. eine HTML-Fehlerseite eines
        // Reverse-Proxys bei 502/503/504, oder ein leerer Body). Mit der
        // alten Reihenfolge (erst .json(), dann Status prüfen) scheiterte
        // `resp.json()` in genau diesem Fall zuerst - der Aufrufer bekam nur
        // "Antwort war kein gültiges JSON" ohne Statuscode zu sehen, und
        // `agent.rs::rufe_mit_wiederaufsetzen` konnte seinen 402-Sofort-
        // Abbruch (kein Guthaben, nicht wiederholen) nie greifen, weil der
        // Statuscode nie in die Fehlermeldung kam - stattdessen wurde ein
        // aussichtsloser 402 bis zu MAX_VERSUCHE lang wiederholt. Fehler
        // kommen bei `stream: true` weiterhin nicht gestreamt, sondern als
        // normales JSON-Body - erst ein erfolgreicher Status schaltet auf
        // SSE um.
        let status = resp.status();
        if !status.is_success() {
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

/// Zerlegt den SSE-Antwortstrom eines OpenAI-Chat-Completions-kompatiblen
/// Anbieters (OpenRouter, xAI/Grok, OpenAI selbst) in gesammelten Text und
/// vollständige Tool-Aufrufe - das OpenAI-Gegenstück zu
/// `anthropic.rs::antwort_aus_strom`, mit derselben Inaktivitäts-Wache statt
/// eines Gesamt-Timeouts (siehe dort für die Begründung).
///
/// Anders als Anthropics `message_stop` beendet das Protokoll den Strom mit
/// einer eigenen Ereigniszeile `data: [DONE]` - eine Verbindung, die vorher
/// zumacht, ist ein unvollständiger Strom, kein normales Ende.
async fn antwort_aus_strom(
    mut strom: impl futures_util::Stream<Item = reqwest::Result<bytes::Bytes>> + Unpin,
    pause_limit: Duration,
) -> Result<LlmAntwort> {
    let mut text_teile: Vec<String> = Vec::new();
    // Tool-Aufrufe kommen gestückelt, zugeordnet über den Index im
    // `tool_calls`-Array des Deltas - id und Funktionsname kommen meist nur
    // im ersten Fragment, die Argumente als JSON-*Zeichenkette* in
    // beliebig vielen weiteren Fragmenten danach.
    let mut ids: HashMap<u64, String> = HashMap::new();
    let mut namen: HashMap<u64, String> = HashMap::new();
    let mut json_fragmente: HashMap<u64, String> = HashMap::new();

    let mut puffer = String::new();

    loop {
        let stueck = match tokio::time::timeout(pause_limit, strom.next()).await {
            Ok(Some(Ok(stueck))) => stueck,
            Ok(Some(Err(fehler))) => return Err(anyhow::anyhow!(fehler)),
            Ok(None) => anyhow::bail!("Antwortstrom endete ohne [DONE]-Ereignis"),
            Err(_) => anyhow::bail!("keine Daten mehr vom Anbieter innerhalb der Wartezeit"),
        };

        puffer.push_str(&String::from_utf8_lossy(&stueck));

        // SSE-Trenner ist eine Leerzeile; ein Ereignis kann über mehrere
        // Netzpakete verteilt sein, deshalb erst verarbeiten, wenn es ganz da ist.
        while let Some(trenn) = puffer.find("\n\n") {
            let ereignis = puffer[..trenn].to_string();
            puffer = puffer[trenn + 2..].to_string();
            if verarbeite_sse(&ereignis, &mut text_teile, &mut ids, &mut namen, &mut json_fragmente)? {
                return fertig_bauen(text_teile, ids, namen, json_fragmente);
            }
        }
    }
}

/// Baut aus den gesammelten Stücken die fertige Antwort - aufgerufen, sobald
/// `[DONE]` kam. Ein Tool-Aufruf ohne gültiges Argument-JSON ist ein
/// Protokollfehler des Anbieters und muss auffallen, nicht still übersprungen werden.
fn fertig_bauen(
    text_teile: Vec<String>,
    mut ids: HashMap<u64, String>,
    mut namen: HashMap<u64, String>,
    json_fragmente: HashMap<u64, String>,
) -> Result<LlmAntwort> {
    let mut antwort = LlmAntwort::default();
    for (index, fragment) in json_fragmente {
        let Some(name) = namen.remove(&index) else {
            anyhow::bail!("Argument-Fragmente ohne zugehörigen Funktionsnamen (Index {index})");
        };
        let id = ids.remove(&index).unwrap_or_default();
        // Ein Tool ganz ohne Eingaben liefert keine Argument-Fragmente; der
        // leere Sammelstring wäre dann kein gültiges JSON. "{}" ist die
        // protokollgemäße Bedeutung: Werkzeug ohne Eingaben.
        let roh = if fragment.trim().is_empty() { "{}" } else { fragment.as_str() };
        let arguments = serde_json::from_str(roh)
            .with_context(|| format!("Werkzeug-Aufruf '{name}' lieferte kein gültiges Argument-JSON"))?;
        antwort.tool_calls.push(ToolCall { id, name, arguments });
    }
    antwort.text = text_teile.join("");
    Ok(antwort)
}

/// Verarbeitet ein SSE-Ereignis. `true` zurückgeben heißt: `[DONE]` gesehen,
/// die Antwort ist komplett.
fn verarbeite_sse(
    ereignis: &str,
    text_teile: &mut Vec<String>,
    ids: &mut HashMap<u64, String>,
    namen: &mut HashMap<u64, String>,
    json_fragmente: &mut HashMap<u64, String>,
) -> Result<bool> {
    for zeile in ereignis.lines() {
        let Some(daten) = zeile.strip_prefix("data:") else {
            continue; // Kommentare, Leerzeilen: egal
        };
        let daten = daten.trim();
        if daten == "[DONE]" {
            return Ok(true);
        }
        let Ok(value) = serde_json::from_str::<Value>(daten) else {
            continue; // kein JSON = kein nutzbares Ereignis
        };
        let delta = &value["choices"][0]["delta"];
        if let Some(t) = delta["content"].as_str() {
            text_teile.push(t.to_string());
        }
        if let Some(calls) = delta["tool_calls"].as_array() {
            for tc in calls {
                let index = tc["index"].as_u64().unwrap_or(0);
                if let Some(id) = tc["id"].as_str() {
                    if !id.is_empty() {
                        ids.insert(index, id.to_string());
                    }
                }
                if let Some(name) = tc["function"]["name"].as_str() {
                    if !name.is_empty() {
                        namen.insert(index, name.to_string());
                    }
                }
                if let Some(fragment) = tc["function"]["arguments"].as_str() {
                    json_fragmente.entry(index).or_default().push_str(fragment);
                }
            }
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

    const TEXT_STROM: &[&str] = &[
        "data: {\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"\"}}]}\n\n",
        "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hallo \"}}]}\n\n",
        "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Welt\"}}]}\n\n",
        "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
        "data: [DONE]\n\n",
    ];

    #[tokio::test]
    async fn text_wird_ueber_deltas_zusammengesetzt() {
        let antwort = antwort_aus_strom(strom(TEXT_STROM.to_vec()), Duration::from_secs(5))
            .await
            .unwrap();
        assert_eq!(antwort.text, "Hallo Welt");
        assert!(antwort.tool_calls.is_empty());
        assert!(antwort.ist_fertig());
    }

    #[tokio::test]
    async fn ereignis_ueber_chunk_grenze_wird_trotzdem_verarbeitet() {
        let stuecke = vec![
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"X\"}}]}\n\ndata: [DO",
            "NE]\n\n",
        ];
        let antwort = antwort_aus_strom(strom(stuecke), Duration::from_secs(5))
            .await
            .unwrap();
        assert_eq!(antwort.text, "X");
    }

    #[tokio::test]
    async fn tool_aufruf_wird_aus_argument_fragmenten_zusammengesetzt() {
        let stuecke = vec![
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"type\":\"function\",\"function\":{\"name\":\"wetter\",\"arguments\":\"\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"{\\\"stadt\\\": \"}}]}}]}\n\n",
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"\\\"Berlin\\\"}\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
            "data: [DONE]\n\n",
        ];
        let antwort = antwort_aus_strom(strom(stuecke), Duration::from_secs(5))
            .await
            .unwrap();
        assert_eq!(antwort.tool_calls.len(), 1);
        let call = &antwort.tool_calls[0];
        assert_eq!(call.id, "call_1");
        assert_eq!(call.name, "wetter");
        assert_eq!(call.arguments["stadt"], "Berlin");
        assert!(!antwort.ist_fertig());
    }

    #[tokio::test]
    async fn tool_ohne_argumente_bekommt_leeres_objekt() {
        let stuecke = vec![
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_2\",\"type\":\"function\",\"function\":{\"name\":\"status\",\"arguments\":\"\"}}]}}]}\n\n",
            "data: [DONE]\n\n",
        ];
        let antwort = antwort_aus_strom(strom(stuecke), Duration::from_secs(5))
            .await
            .unwrap();
        assert_eq!(antwort.tool_calls.len(), 1);
        assert_eq!(antwort.tool_calls[0].arguments, json!({}));
    }

    #[tokio::test]
    async fn strom_ende_ohne_done_ist_ein_fehler() {
        let stuecke = vec!["data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Anfang...\"}}]}\n\n"];
        let ergebnis = antwort_aus_strom(strom(stuecke), Duration::from_secs(5)).await;
        assert!(ergebnis.is_err());
    }

    #[tokio::test]
    async fn pause_ohne_daten_bricht_ab() {
        let unendlich = futures_util::stream::iter(vec![Ok(bytes::Bytes::from(
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"...\"}}]}\n\n".to_string(),
        ))])
        .chain(futures_util::stream::pending());
        let ergebnis = antwort_aus_strom(unendlich, Duration::from_millis(100)).await;
        let fehler = ergebnis.expect_err("muss wegen Inaktivität abbrechen");
        assert!(fehler.to_string().contains("Wartezeit"), "unerwarteter Fehler: {fehler}");
    }
}