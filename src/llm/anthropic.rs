//! Anbieter, die das Anthropic-Messages-Protokoll sprechen.
//!
//! Das sind Anthropic selbst und alles, was dessen `/v1/messages`-Format
//! nachbaut - zum Beispiel Charm Hyper. Der Unterschied steckt nur in
//! Adresse, Key und Standardmodell, nicht im Code darunter.

use super::{
    http_client, require_api_key, LlmAntwort, LlmProvider, Message, ToolCall, ToolDefinition,
};
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::time::Duration;

pub struct AnthropicProvider {
    api_key: String,
    model: String,
    endpoint: String,
    label: &'static str,
    max_tokens: u32,
    client: reqwest::Client,
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
            client: http_client(timeout)?,
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
        let resp = self
            .client
            .post(&self.endpoint)
            .header("x-api-key", &self.api_key)
            .bearer_auth(&self.api_key)
            .header("anthropic-version", "2023-06-01")
            .json(&body)
            .send()
            .await
            .with_context(|| format!("Anfrage an {} fehlgeschlagen", self.endpoint))?;

        let status = resp.status();
        let value: Value = resp.json().await.context("Antwort war kein gültiges JSON")?;

        if !status.is_success() {
            anyhow::bail!("API-Fehler von {} ({status}): {value}", self.endpoint);
        }

        let mut antwort = LlmAntwort::default();
        let mut text_teile = Vec::new();

        let leer = Vec::new();
        for block in value["content"].as_array().unwrap_or(&leer) {
            match block["type"].as_str() {
                Some("tool_use") => antwort.tool_calls.push(ToolCall {
                    id: block["id"].as_str().unwrap_or_default().to_string(),
                    name: block["name"].as_str().unwrap_or_default().to_string(),
                    arguments: block["input"].clone(),
                }),
                Some("text") => {
                    if let Some(t) = block["text"].as_str() {
                        text_teile.push(t);
                    }
                }
                // "thinking" und was Anbieter sonst noch erfinden: übergehen.
                _ => {}
            }
        }

        antwort.text = text_teile.join("\n");
        Ok(antwort)
    }
}
