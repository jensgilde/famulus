//! Guthaben-Abfrage für Hyper und OpenRouter - einmal im Kern, damit
//! GUI (`gui/src/lib.rs::credits`) und Telegram-Bot (`/status` in
//! `telegram.rs`) das Guthaben nicht jeder für sich abfragen und die
//! Logik still auseinanderläuft (dasselbe Prinzip wie bei den
//! Fernbedienungs-Typen, siehe Kopfkommentar von `remote.rs`).

use crate::config::Config;

/// Ermittelt das aktuelle Guthaben für den in `config` eingestellten
/// Provider. Für Ollama kommt `"lokal"` zurück - lokale Modelle kennen
/// kein Guthabenkonzept. Fehler kommen als lesbare Meldung, nicht als
/// Panic: Ein nicht erreichbarer Provider darf den `/status`-Befehl
/// nicht sprengen, er meldet das Guthaben dann eben als "?".
pub async fn anzeigen(config: &Config) -> Result<String, String> {
    let (url, key_var) = match config.provider.as_str() {
        "hyper" => (
            format!(
                "{}/v1/credits",
                config
                    .base_url
                    .as_deref()
                    .unwrap_or("https://hyper.charm.land")
                    .trim_end_matches('/')
            ),
            config
                .api_key_env
                .clone()
                .unwrap_or_else(|| "HYPER_API_KEY".to_string()),
        ),
        "openrouter" => (
            "https://openrouter.ai/api/v1/credits".to_string(),
            config
                .api_key_env
                .clone()
                .unwrap_or_else(|| "OPENROUTER_API_KEY".to_string()),
        ),
        "ollama" => return Ok("lokal".to_string()),
        other => return Err(format!("Unbekannter Provider '{other}'.")),
    };

    let key = std::env::var(&key_var).map_err(|e| format!("{key_var} nicht gesetzt: {e}"))?;

    let antwort = reqwest::Client::new()
        .get(&url)
        .bearer_auth(&key)
        .send()
        .await
        .map_err(|e| format!("Credits-Anfrage fehlgeschlagen: {e}"))?;

    // Status VOR dem Auswerten prüfen: ein abgelaufener/falscher API-Key
    // liefert oft trotzdem ein JSON-Objekt zurück (z.B. `{"error": "..."}`),
    // nur eben ohne "balance"/"total_credits". Ohne diesen Check griffen die
    // `.unwrap_or(0.0)`-Fallbacks unten still durch und zeigten "0" Credits -
    // ununterscheidbar von echtem Guthabenmangel, obwohl das eigentliche
    // Problem ein ungültiger Key oder ein Serverfehler war.
    let status = antwort.status();
    if !status.is_success() {
        let text = antwort.text().await.unwrap_or_default();
        return Err(format!("Credits-Anfrage fehlgeschlagen ({status}): {text}"));
    }

    let body: serde_json::Value = antwort
        .json()
        .await
        .map_err(|e| format!("Credits-Antwort kein JSON: {e}"))?;

    Ok(match config.provider.as_str() {
        "hyper" => {
            let credits = body["balance"].as_f64().unwrap_or(0.0).floor() as i64;
            credits.to_string()
        }
        "openrouter" => {
            let total = body["data"]["total_credits"].as_f64().unwrap_or(0.0);
            let verbraucht = body["data"]["total_usage"].as_f64().unwrap_or(0.0);
            let rest = total - verbraucht;
            (rest.floor() as i64).to_string()
        }
        _ => "?".to_string(),
    })
}
