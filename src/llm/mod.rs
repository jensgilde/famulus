//! Die Schnittstelle zu den Sprachmodellen.
//!
//! Famulus spricht zwei Protokolle: das Anthropic-Messages-Format
//! (`/v1/messages`) und das OpenAI-Chat-Completions-Format
//! (`/v1/chat/completions`). Fast jeder Anbieter spricht eines von beiden,
//! deshalb reichen zwei Implementierungen für beliebig viele Dienste - ein
//! neuer Anbieter kostet meist nur einen Zweig in `build_provider`.

use async_trait::async_trait;
use std::time::Duration;

/// Ein Werkzeug-Aufruf, den das Modell angefordert hat.
#[derive(Debug, Clone)]
pub struct ToolCall {
    /// Die ID, die der Anbieter vergeben hat. Sie muss unverändert mit dem
    /// Ergebnis zurückgeschickt werden, sonst kann das Modell Aufruf und
    /// Antwort nicht zusammenbringen.
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

/// Was ein Werkzeug zurückgemeldet hat.
#[derive(Debug, Clone)]
pub struct ToolResult {
    pub call_id: String,
    pub inhalt: String,
    /// Gescheitert? Beide Protokolle haben dafür ein eigenes Feld - damit
    /// weiß das Modell, dass es etwas anderes probieren muss, statt das
    /// Ergebnis für bare Münze zu nehmen.
    pub fehler: bool,
}

/// Eine Station im Gesprächsverlauf.
///
/// Bewusst ein Aufzählungstyp statt `{ role, content }`: Ein Modell, das
/// Werkzeuge benutzt, schickt keine Textzeile, sondern strukturierte Blöcke.
/// Wer das in Text zurückverwandelt, verliert die Zuordnung von Aufruf zu
/// Ergebnis - und damit genau das, woran sich das Modell orientiert.
#[derive(Debug, Clone)]
pub enum Message {
    /// Was Jens gesagt hat.
    User(String),
    /// Was das Modell geantwortet hat: Text, Werkzeug-Aufrufe, oder beides.
    Assistant {
        text: String,
        tool_calls: Vec<ToolCall>,
    },
    /// Was die Werkzeuge daraufhin gemeldet haben. Alle Ergebnisse eines
    /// Zuges gehören in EINE Nachricht - werden sie auf mehrere verteilt,
    /// gewöhnen sich Modelle ab, mehrere Werkzeuge auf einmal aufzurufen.
    ToolResults(Vec<ToolResult>),
}

#[derive(Debug, Clone)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters_schema: serde_json::Value, // JSON-Schema fürs jeweilige Tool
}

/// Was bei einem Modellaufruf herauskommt.
///
/// Text und Werkzeug-Aufrufe schließen sich NICHT aus: Modelle sagen gern
/// erst, was sie vorhaben, und rufen dann das Werkzeug auf. Beides getrennt
/// zu führen kostet nichts und wirft nichts weg.
#[derive(Debug, Clone, Default)]
pub struct LlmAntwort {
    pub text: String,
    pub tool_calls: Vec<ToolCall>,
}

impl LlmAntwort {
    /// Keine Werkzeuge mehr angefordert heißt: das war die Schlussantwort.
    pub fn ist_fertig(&self) -> bool {
        self.tool_calls.is_empty()
    }
}

/// Gemeinsame Schnittstelle, die jeder Anbieter implementiert. Der Rest von
/// Famulus (die Agent-Schleife) kennt NUR dieses Trait, nie ein konkretes
/// Backend - so bleibt der Wechsel zwischen Anbietern eine Ein-Zeilen-
/// Änderung in der Config, kein Code-Umbau.
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// `system` ist der Vorspann, der über dem Gespräch steht und sich
    /// während eines Auftrags nicht ändert. Getrennt zu halten ist kein
    /// Schönheitsfehler: Anbieter können genau diesen stabilen Anfang
    /// zwischenspeichern, statt ihn in jedem Zug neu zu verarbeiten.
    async fn next(
        &self,
        system: Option<&str>,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmAntwort>;

    fn name(&self) -> &'static str;
}

pub mod anthropic;
pub mod openai;

/// Liest einen API-Key aus der Umgebung. Eine gesetzte, aber leere Variable
/// gilt dabei als fehlend - sonst schickt Famulus einen leeren Key los und
/// man bekommt statt einer klaren Ansage nur ein nacktes 401 vom Server.
fn require_api_key(var_name: &str) -> anyhow::Result<String> {
    match std::env::var(var_name) {
        Ok(key) if !key.trim().is_empty() => Ok(key.trim().to_string()),
        _ => anyhow::bail!(
            "{var_name} fehlt oder ist leer. Trag den Key in die .env-Datei im Projektordner ein."
        ),
    }
}

/// Baut den HTTP-Client, den ein Anbieter benutzt.
///
/// Der Timeout ist der Punkt, um den es hier geht. Ohne ihn wartet Famulus
/// endlos, wenn der Dienst die Verbindung offen lässt, ohne zu antworten -
/// und das sieht von außen exakt aus wie ein Modell, das nachdenkt.
fn http_client(timeout: Duration) -> anyhow::Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .timeout(timeout)
        .connect_timeout(Duration::from_secs(20))
        .build()?)
}

/// Schneidet einen `base_url` aus der Config auf die Form, an die sich der
/// Pfad anhängen lässt.
fn basis(base_url: Option<String>, standard: &str) -> String {
    base_url
        .unwrap_or_else(|| standard.to_string())
        .trim_end_matches('/')
        .to_string()
}

pub fn build_provider(config: &crate::config::Config) -> anyhow::Result<Box<dyn LlmProvider>> {
    let timeout = Duration::from_secs(config.timeout_sekunden);
    let max_tokens = config.max_antwort_tokens;
    let modell = config.model.clone();
    let basis_url = config.base_url.clone();
    let key_var = config.api_key_env.clone();

    match config.provider.as_str() {
        "anthropic" => Ok(Box::new(anthropic::AnthropicProvider::neu(
            "anthropic",
            modell.unwrap_or_else(|| "claude-opus-5".to_string()),
            basis(basis_url, "https://api.anthropic.com"),
            &key_var.unwrap_or_else(|| "ANTHROPIC_API_KEY".to_string()),
            max_tokens,
            timeout,
        )?)),

        // Charm Hyper serviert unter /v1/messages dasselbe Protokoll wie
        // Anthropic - deshalb dieselbe Implementierung, nur mit anderer
        // Adresse, anderem Key und anderem Standardmodell. Genau dafür ist
        // das LlmProvider-Trait da: ein neuer Anbieter kostet hier sechs
        // Zeilen statt einer eigenen Datei.
        "hyper" => Ok(Box::new(anthropic::AnthropicProvider::neu(
            "hyper",
            modell.unwrap_or_else(|| "deepseek-v4-flash".to_string()),
            basis(basis_url, "https://hyper.charm.land"),
            &key_var.unwrap_or_else(|| "HYPER_API_KEY".to_string()),
            max_tokens,
            timeout,
        )?)),

        "grok" => Ok(Box::new(openai::OpenAiProvider::neu(
            "grok",
            modell.unwrap_or_else(|| "grok-4".to_string()),
            basis(basis_url, "https://api.x.ai"),
            &key_var.unwrap_or_else(|| "XAI_API_KEY".to_string()),
            Vec::new(),
            max_tokens,
            timeout,
        )?)),

        "openrouter" => Ok(Box::new(openai::OpenAiProvider::neu(
            "openrouter",
            modell.unwrap_or_else(|| "openai/gpt-4o".to_string()),
            basis(basis_url, "https://openrouter.ai/api"),
            &key_var.unwrap_or_else(|| "OPENROUTER_API_KEY".to_string()),
            // OpenRouter zeigt diese zwei Angaben in seiner Statistik an.
            vec![
                ("HTTP-Referer", "https://github.com/jgilde/famulus"),
                ("X-Title", "Famulus"),
            ],
            max_tokens,
            timeout,
        )?)),

        // Sammelzweig für alles andere, was OpenAI-kompatibel spricht -
        // lokale Server (Ollama, llama.cpp, vLLM) genauso wie fremde
        // Dienste. Braucht nur base_url und api_key_env in der Config.
        "openai" => Ok(Box::new(openai::OpenAiProvider::neu(
            "openai",
            modell.unwrap_or_else(|| "gpt-4o".to_string()),
            basis(basis_url, "https://api.openai.com"),
            &key_var.unwrap_or_else(|| "OPENAI_API_KEY".to_string()),
            Vec::new(),
            max_tokens,
            timeout,
        )?)),

        other => anyhow::bail!(
            "Unbekannter Provider '{other}' in famulus.toml. Erlaubt: 'anthropic', 'hyper', 'grok', 'openrouter', 'openai'."
        ),
    }
}
