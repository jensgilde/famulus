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
    http_client, require_api_key, send_mit_retry, LlmAntwort, LlmProvider, Message, ToolCall,
    ToolDefinition,
};
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::time::Duration;

pub struct OpenAiProvider {
    api_key: String,
    model: String,
    endpoint: String,
    label: &'static str,
    /// Zusätzliche Kopfzeilen, die manche Dienste sehen wollen (OpenRouter
    /// zeigt damit an, welche Anwendung anfragt).
    kopfzeilen: Vec<(&'static str, &'static str)>,
    max_tokens: u32,
    client: reqwest::Client,
}

impl OpenAiProvider {
    pub fn neu(
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
            client: http_client(timeout)?,
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
        });
        if !openai_tools.is_empty() {
            body["tools"] = json!(openai_tools);
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
        // aussichtsloser 402 bis zu MAX_VERSUCHE lang wiederholt.
        let status = resp.status();
        if !status.is_success() {
            let value: Value = resp
                .json()
                .await
                .unwrap_or_else(|_| json!({"fehler": "Antwort war kein gültiges JSON"}));
            anyhow::bail!("API-Fehler von {} ({status}): {value}", self.endpoint);
        }
        let value: Value = resp.json().await.context("Antwort war kein gültiges JSON")?;

        let nachricht = &value["choices"][0]["message"];

        let leer = Vec::new();
        let tool_calls = nachricht["tool_calls"]
            .as_array()
            .unwrap_or(&leer)
            .iter()
            .filter_map(|tc| {
                let name = tc["function"]["name"].as_str()?;
                // Die Argumente kommen als Zeichenkette an. Ist sie kaputt,
                // lieber mit leeren Argumenten weitermachen als den ganzen
                // Zug wegzuwerfen - das Werkzeug meldet dann einen klaren
                // Fehler zurück, und das Modell versucht es nochmal richtig.
                let arguments = tc["function"]["arguments"]
                    .as_str()
                    .and_then(|s| serde_json::from_str(s).ok())
                    .unwrap_or_else(|| json!({}));
                Some(ToolCall {
                    id: tc["id"].as_str().unwrap_or_default().to_string(),
                    name: name.to_string(),
                    arguments,
                })
            })
            .collect();

        Ok(LlmAntwort {
            text: nachricht["content"].as_str().unwrap_or_default().to_string(),
            tool_calls,
        })
    }
}