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

/// Ein Bild, das zusammen mit einer Nutzer-Nachricht ans Modell geschickt wird.
#[derive(Debug, Clone)]
pub struct BildAnhang {
    /// MIME-Typ: "image/png", "image/jpeg", "image/gif", "image/webp"
    pub medien_typ: String,
    /// Base64-kodierte Bilddaten (ohne data:...-Präfix)
    pub base64: String,
}

/// Eine Station im Gesprächsverlauf.
///
/// Bewusst ein Aufzählungstyp statt `{ role, content }`: Ein Modell, das
/// Werkzeuge benutzt, schickt keine Textzeile, sondern strukturierte Blöcke.
/// Wer das in Text zurückverwandelt, verliert die Zuordnung von Aufruf zu
/// Ergebnis - und damit genau das, woran sich das Modell orientiert.
#[derive(Debug, Clone)]
pub enum Message {
    /// Was Jens gesagt hat (reiner Text, keine Anhänge).
    User(String),
    /// Was Jens gesagt hat, mit Bildern. Die Bilder werden als Vision-Content-
    /// Blöcke direkt ans Modell geschickt, sodass es sie "sehen" kann.
    UserMitBild {
        text: String,
        bilder: Vec<BildAnhang>,
    },
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
pub mod router;

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

/// Baut den HTTP-Client, den ein Anbieter benutzt - bewusst OHNE
/// Gesamt-Timeout, weil beide Protokolle (`anthropic.rs`, `openai.rs`)
/// gestreamt werden. Ein globales Zeitlimit würde genau den Fehler wieder
/// einführen, den das Streaming behebt: eine lange Antwort stirbt nach
/// `timeout` Sekunden, egal wie fleißig der Anbieter Tokens liefert. Der
/// Verbindungsaufbau bleibt über `connect_timeout` begrenzt, danach übernimmt
/// die Inaktivitäts-Wache des jeweiligen Providers (`antwort_aus_strom` in
/// `anthropic.rs` bzw. `openai.rs`): Jedes gelesene Stück muss innerhalb
/// derselben `timeout`-Zeit eintreffen, sonst wird abgebrochen. "Der Dienst
/// hängt" und "die Antwort ist lang" sind damit sauber getrennt.
///
/// `pool_idle_timeout` ist bewusst kurz gesetzt: reqwest hält offene
/// Verbindungen im Pool warm, um sie für die nächste Anfrage
/// wiederzuverwenden. Schließt der Server (oder ein Proxy davor, wie bei
/// hyper.charm.land) eine Verbindung, während sie im Pool auf
/// Wiederverwendung wartet, merkt reqwest das erst beim nächsten
/// Schreibversuch - das Ergebnis ist "connection closed before message
/// completed", obwohl der Dienst selbst nichts abbekommen hat. 15 Sekunden
/// liegen sicher unter den Idle-Timeouts gängiger Reverse-Proxys (meist
/// 30-60s), sodass Famulus die Verbindung von sich aus aufgibt, bevor der
/// Server das tut.
fn http_client_ohne_gesamttimeout() -> anyhow::Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(20))
        .pool_idle_timeout(Duration::from_secs(15))
        .build()?)
}

/// Schickt eine Anfrage ab und wiederholt sie einmal, wenn der Fehler nach
/// einer toten Pool-Verbindung aussieht.
///
/// `pool_idle_timeout` oben verkleinert das Zeitfenster für eine tote
/// Verbindung, schließt es aber nicht: der Server kann jederzeit früher
/// abbauen. Ein zweiter Versuch mit `baue_anfrage()` erzeugt eine komplett
/// neue Anfrage (reqwest holt sich dafür automatisch eine frische
/// Verbindung aus dem Pool oder baut eine neue auf) und behebt damit genau
/// die Fehlerklasse, die als "connection closed before message completed"
/// auftaucht. Auf echte Zeitüberschreitungen wird NICHT erneut versucht -
/// die haben nichts mit einer toten Pool-Verbindung zu tun, und ein
/// zweiter Versuch würde die Wartezeit nur verdoppeln.
async fn send_mit_retry(
    baue_anfrage: impl Fn() -> reqwest::RequestBuilder,
) -> reqwest::Result<reqwest::Response> {
    match baue_anfrage().send().await {
        Err(fehler) if !fehler.is_timeout() && (fehler.is_connect() || fehler.is_request()) => {
            baue_anfrage().send().await
        }
        ergebnis => ergebnis,
    }
}

/// Schneidet einen `base_url` aus der Config auf die Form, an die sich der
/// Pfad anhängen lässt.
fn basis(base_url: Option<String>, standard: &str) -> String {
    base_url
        .unwrap_or_else(|| standard.to_string())
        .trim_end_matches('/')
        .to_string()
}

/// Baut einen einzelnen Provider. `base_url`/`key_var` gelten nur für den
/// Hauptprovider aus der Config - Fallbacks rufen das mit `None` auf und
/// bekommen damit die Standard-Adresse/den Standard-Key ihres Providers
/// (siehe `Config::fallback_providers`-Doku für die Begründung).
fn build_single_provider(
    name: &str,
    modell: Option<String>,
    basis_url: Option<String>,
    key_var: Option<String>,
    max_tokens: u32,
    timeout: Duration,
) -> anyhow::Result<Box<dyn LlmProvider>> {
    match name {
        // Charm Hyper spricht das Anthropic-Messages-Protokoll - deshalb
        // dieselbe Implementierung wie ein echter Anthropic-Zugang, nur mit
        // anderer Adresse, anderem Key und anderem Standardmodell. Genau
        // dafür ist das LlmProvider-Trait da: ein neuer Anbieter kostet hier
        // sechs Zeilen statt einer eigenen Datei.
        "hyper" => Ok(Box::new(anthropic::AnthropicProvider::neu(
            "hyper",
            modell.unwrap_or_else(|| "deepseek-v4-flash".to_string()),
            basis(basis_url, "https://hyper.charm.land"),
            &key_var.unwrap_or_else(|| "HYPER_API_KEY".to_string()),
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

        other => anyhow::bail!(
            "Unbekannter Provider '{other}' in famulus.toml. Erlaubt: 'hyper', 'openrouter'."
        ),
    }
}

/// Baut den (oder die) konfigurierten Provider und verpackt sie in den
/// Router. Ohne `fallback_providers` (der Normalfall) liegt genau ein
/// Provider in der Liste - der Router versucht ihn einmal und reicht
/// Erfolg oder Fehler direkt durch. Verhalten und Fehlerfälle sind dann
/// identisch zu vorher, nur mit Scorecard-Eintrag nebenbei (siehe
/// `router.rs`). Erst mit mindestens einem Fallback probiert der Router
/// bei einem Fehlschlag den nächsten Provider, statt den Auftrag
/// abzubrechen.
pub fn build_provider(config: &crate::config::Config) -> anyhow::Result<Box<dyn LlmProvider>> {
    let timeout = Duration::from_secs(config.timeout_sekunden);
    let max_tokens = config.max_antwort_tokens;

    let haupt = build_single_provider(
        &config.provider,
        config.model.clone(),
        config.base_url.clone(),
        config.api_key_env.clone(),
        max_tokens,
        timeout,
    )?;

    let mut providers = vec![haupt];
    for fallback in &config.fallback_providers {
        providers.push(build_single_provider(
            &fallback.provider,
            fallback.model.clone(),
            None,
            None,
            max_tokens,
            timeout,
        )?);
    }

    Ok(Box::new(router::RouterProvider::new(providers)))
}

/// Baut das "günstige" Modell für die automatische Modellwahl, falls in
/// `famulus.toml` eines eingetragen ist. Separat von `build_provider`, weil
/// es kein Fallback-Ziel ist (der Router wechselt nur bei einem Fehler
/// dorthin) - es ist eine bewusste, aufgabenabhängige Alternative, die
/// `agent.rs` selbst auswählt, nicht der Router.
pub fn build_guenstiges_modell(config: &crate::config::Config) -> anyhow::Result<Option<Box<dyn LlmProvider>>> {
    let Some(guenstig) = &config.guenstiges_modell else {
        return Ok(None);
    };
    let timeout = Duration::from_secs(config.timeout_sekunden);
    let provider = build_single_provider(
        &guenstig.provider,
        guenstig.model.clone(),
        None,
        None,
        config.max_antwort_tokens,
        timeout,
    )?;
    Ok(Some(provider))
}