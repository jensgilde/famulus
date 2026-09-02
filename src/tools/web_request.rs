use super::Tool;
use crate::llm::ToolDefinition;
use crate::permissions::PermissionManager;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Mutex;

/// Maximale Antwortgröße in Bytes, die an das Modell zurückgegeben wird.
/// Schützt den Kontext vor Aufblähung durch große HTML-Seiten.
const MAX_ANTWORT_BYTES: usize = 100_000;

/// Maximale Redirects, die reqwest folgen darf.
const MAX_REDIRECTS: usize = 5;

/// Standard-Timeout für einen einzelnen Request.
const TIMEOUT_SEKUNDEN: u64 = 30;

/// Pfad zur Cookie-Jar-Datei (relativ zum Famulus-Datenverzeichnis).
const COOKIE_DATEI: &str = "web_cookies.json";

/// Lädt das Cookie-Jar aus der JSON-Datei.
fn lade_cookies(daten_pfad: &std::path::Path) -> HashMap<String, String> {
    let pfad = daten_pfad.join(COOKIE_DATEI);
    match std::fs::read_to_string(&pfad) {
        Ok(json_str) => serde_json::from_str(&json_str).unwrap_or_default(),
        Err(_) => HashMap::new(),
    }
}

/// Speichert das Cookie-Jar als JSON-Datei.
fn speichere_cookies(daten_pfad: &std::path::Path, cookies: &HashMap<String, String>) {
    let pfad = daten_pfad.join(COOKIE_DATEI);
    if let Some(eltern) = pfad.parent() {
        let _ = std::fs::create_dir_all(eltern);
    }
    let _ = std::fs::write(&pfad, serde_json::to_string_pretty(cookies).unwrap_or_default());
}

/// Parst den `Set-Cookie`-Header in eine Key-Value-Map.
/// Format: `name=value; Path=/; HttpOnly; ...`
fn parse_set_cookie(header: &str) -> Option<(String, String)> {
    let teil = header.split(';').next()?;
    let mut kv = teil.splitn(2, '=');
    let key = kv.next()?.trim().to_string();
    let value = kv.next()?.trim().to_string();
    if key.is_empty() {
        return None;
    }
    Some((key, value))
}

/// Extrahiert den Host-Teil (ohne Port, ohne Klammern bei IPv6) aus einer URL.
fn host_aus_url(url: &str) -> String {
    let ohne_schema = url.split("://").nth(1).unwrap_or("");
    let autorität = ohne_schema.split('/').next().unwrap_or("");
    // IPv6-Literal in Klammern, z.B. "[::1]:8080" oder "[fe80::1]" - der Port
    // (falls vorhanden) folgt erst nach der schließenden Klammer, ein simples
    // split(':') würde die Adresse selbst zerreißen.
    if let Some(rest) = autorität.strip_prefix('[') {
        return rest.split(']').next().unwrap_or("").to_string();
    }
    autorität.split(':').next().unwrap_or("").to_string()
}

/// Ob eine bereits aufgelöste IP-Adresse privat/lokal ist - echte
/// Bit-Prüfung über die Standardbibliothek (deckt auch Link-Local
/// 169.254.0.0/16, die Cloud-Metadaten-Adresse 169.254.169.254 und
/// IPv6-Gegenstücke ab), statt der alten String-Präfix-Heuristik, die genau
/// diesen Bereich komplett überging.
fn ist_private_ip(ip: &std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            v4.is_loopback() || v4.is_private() || v4.is_link_local() || v4.is_unspecified()
        }
        std::net::IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unspecified()
                // Unique Local Address (fc00::/7) - IPv6-Äquivalent zu
                // 10.0.0.0/8 & Co., in std (noch) nicht als Methode verfügbar.
                || (v6.segments()[0] & 0xfe00) == 0xfc00
                // IPv4-mapped (::ffff:a.b.c.d) - sonst ließe sich der ganze
                // Schutz oben einfach durch diese Schreibweise umgehen.
                || v6.to_ipv4_mapped().is_some_and(|v4| {
                    v4.is_loopback() || v4.is_private() || v4.is_link_local() || v4.is_unspecified()
                })
        }
    }
}

/// Prüft, ob eine URL auf ein privates/loopback-Netzwerk zeigt (SSRF-Schutz).
///
/// Löst den Hostnamen tatsächlich per DNS auf und prüft ALLE zurückgegebenen
/// Adressen, statt nur den Hostnamen-Text zu mustern - ein Name, der (auch
/// erst später, DNS-Rebinding) auf eine private Adresse zeigt, kam an der
/// alten String-Prüfung vorbei, weil "localhost"/"127.0.0.1"/... nur exakt
/// diese Literale abfing.
async fn ist_private_url(url: &str) -> bool {
    let host = host_aus_url(&url.to_lowercase());
    if host.is_empty() {
        return true; // Kein Host geparst - lieber blockieren als offenlassen.
    }
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        return ist_private_ip(&ip);
    }
    // Hostname statt IP-Literal: auflösen und jede zurückgegebene Adresse
    // prüfen. Port ist für die Prüfung irrelevant, 0 reicht als Platzhalter.
    let ergebnis = match tokio::net::lookup_host((host.as_str(), 0u16)).await {
        Ok(adressen) => adressen.map(|a| a.ip()).any(|ip| ist_private_ip(&ip)),
        // Auflösung fehlgeschlagen (Tippfehler, kein DNS-Eintrag, ...) - kein
        // SSRF-Fall, der eigentliche Request scheitert gleich danach ohnehin
        // mit einem klaren Verbindungsfehler.
        Err(_) => false,
    };
    ergebnis
}

pub struct WebRequestTool {
    cookies: Mutex<HashMap<String, String>>,
    daten_pfad: std::path::PathBuf,
    client: reqwest::Client,
}

impl WebRequestTool {
    pub fn new(daten_pfad: std::path::PathBuf) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(TIMEOUT_SEKUNDEN))
            // SSRF-Schutz gilt nicht nur für die Start-URL: ein Server, den
            // die Start-URL-Prüfung passieren ließ, könnte per Redirect auf
            // eine private/lokale Adresse umleiten (z.B. auf die
            // Cloud-Metadaten-IP) und die Prüfung so umgehen, wenn Redirects
            // ungeprüft weiterverfolgt würden. Reine IP-Literal-Prüfung hier
            // (synchron, kein DNS) - fängt den direkten Fall ab; ein Redirect
            // auf einen Hostnamen, der erst per DNS-Rebinding auf eine
            // private Adresse zeigt, bleibt eine bekannte Restlücke ohne
            // eigenen DNS-Resolver-Hook (siehe ist_private_url-Kommentar).
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                let host = host_aus_url(&attempt.url().as_str().to_lowercase());
                let privat = host
                    .parse::<std::net::IpAddr>()
                    .map(|ip| ist_private_ip(&ip))
                    .unwrap_or(false);
                if privat || attempt.previous().len() >= MAX_REDIRECTS {
                    attempt.error("Redirect auf private/lokale Adresse verweigert (SSRF-Schutz)")
                } else {
                    attempt.follow()
                }
            }))
            .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) Famulus/1.1")
            .build()
            .expect("HTTP-Client bauen");

        let cookies = lade_cookies(&daten_pfad);

        WebRequestTool {
            cookies: Mutex::new(cookies),
            daten_pfad,
            client,
        }
    }
}

#[async_trait]
impl Tool for WebRequestTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "web_request".to_string(),
            description: "Sendet eine HTTP-Anfrage (GET oder POST) und gibt den Antworttext zurück. \
                Cookies werden automatisch zwischen Aufrufen gespeichert – für Login-Sessions \
                (z. B. Kicktipp, Foren, Webdienste). POST sendet `application/x-www-form-urlencoded`, \
                es sei denn, du setzt den `Content-Type`-Header manuell. \
                Kein JavaScript, kein CSS-Rendering – reiner HTML/JSON-Text."
                .to_string(),
            parameters_schema: json!({
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "Die vollständige URL (https://...)."
                    },
                    "method": {
                        "type": "string",
                        "description": "HTTP-Methode: GET oder POST. Standard: GET.",
                        "enum": ["GET", "POST"]
                    },
                    "headers": {
                        "type": "object",
                        "description": "Zusätzliche HTTP-Header als Key-Value-Paare. Optional.",
                        "additionalProperties": { "type": "string" }
                    },
                    "body": {
                        "type": "string",
                        "description": "Body für POST-Requests. Bei Default-Content-Type: URL-encoded (key1=wert1&key2=wert2). Optional."
                    }
                },
                "required": ["url"]
            }),
        }
    }

    async fn execute(
        &self,
        args: Value,
        _permissions: &PermissionManager,
    ) -> anyhow::Result<String> {
        let url = args["url"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("'url' fehlt"))?;
        let method = args["method"].as_str().unwrap_or("GET").to_uppercase();

        // SSRF-Schutz
        if ist_private_url(url).await {
            anyhow::bail!(
                "Zugriff auf private/lokale Adressen verweigert: '{url}'. \
                Nur öffentliche URLs (https://...) sind erlaubt."
            );
        }

        if !url.starts_with("https://") && !url.starts_with("http://") {
            anyhow::bail!("Nur http:// und https:// URLs sind erlaubt, nicht: '{url}'");
        }

        // ── Cookie-Header bauen ─
        let cookie_header = {
            let jar = self.cookies.lock().unwrap();
            if jar.is_empty() {
                None
            } else {
                Some(
                    jar.iter()
                        .map(|(k, v)| format!("{k}={v}"))
                        .collect::<Vec<_>>()
                        .join("; "),
                )
            }
        };

        let mut request = match method.as_str() {
            "GET" => self.client.get(url),
            "POST" => {
                let mut req = self.client.post(url);
                let body = args["body"].as_str().unwrap_or("");
                let hat_content_type = args["headers"]
                    .as_object()
                    .map(|h| h.keys().any(|k| k.to_lowercase() == "content-type"))
                    .unwrap_or(false);
                if !hat_content_type && !body.is_empty() {
                    req = req.header("Content-Type", "application/x-www-form-urlencoded");
                }
                req = req.body(body.to_string());
                req
            }
            other => anyhow::bail!("Methode '{other}' nicht unterstützt (nur GET/POST)"),
        };

        // Benutzer-Header setzen.
        // `value` ist ein `serde_json::Value` – `as_str()` gibt `Option<&str>`,
        // aber `reqwest::header()` verlangt einen Typ, aus dem `HeaderValue`
        // gebaut werden kann. `Option<&str>` implementiert das nicht, also
        // mit `unwrap_or("")` in ein einfaches `&str` wandeln.
        if let Some(headers) = args["headers"].as_object() {
            for (key, value) in headers {
                request = request.header(key.as_str(), value.as_str().unwrap_or(""));
            }
        }

        // Cookie-Header anhängen (nach Benutzer-Headern, damit Benutzer-Cookies
        // Vorrang haben, falls sie denselben Key setzen).
        if let Some(ref cookie_val) = cookie_header {
            request = request.header("Cookie", cookie_val.as_str());
        }

        let response = request.send().await?;

        let status = response.status().as_u16();
        let neue_cookies: Vec<(String, String)> = response
            .headers()
            .get_all("set-cookie")
            .iter()
            .filter_map(|h| h.to_str().ok())
            .filter_map(parse_set_cookie)
            .collect();

        // Gestreamt statt response.bytes(), und mit hartem Abbruch sobald das
        // Limit erreicht ist: bytes() hätte die komplette Antwort erst in den
        // Speicher geladen und ERST DANACH auf MAX_ANTWORT_BYTES gekürzt -
        // eine riesige oder absichtlich endlose Antwort (von einer per
        // Prompt-Injection untergeschobenen URL, oder einfach einem großen
        // Download) wäre trotz des Limits komplett heruntergeladen worden,
        // bevor der Deckel überhaupt gegriffen hätte.
        use futures_util::StreamExt;
        let mut antwort_bytes: Vec<u8> = Vec::new();
        let mut abgeschnitten = false;
        let mut gesamtgroesse = 0usize;
        let mut strom = response.bytes_stream();
        while let Some(stueck) = strom.next().await {
            let stueck = stueck?;
            gesamtgroesse += stueck.len();
            if antwort_bytes.len() < MAX_ANTWORT_BYTES {
                let noch_platz = MAX_ANTWORT_BYTES - antwort_bytes.len();
                antwort_bytes.extend_from_slice(&stueck[..stueck.len().min(noch_platz)]);
            }
            if gesamtgroesse > MAX_ANTWORT_BYTES {
                abgeschnitten = true;
                if gesamtgroesse > MAX_ANTWORT_BYTES * 10 {
                    // Deutlich über dem Limit: Verbindung aktiv kappen statt
                    // brav bis zum Streamende weiterzulesen - sonst bliebe
                    // der Schutz gegen eine böswillig riesige/endlose
                    // Antwort wirkungslos, nur die Zielgröße wäre anders.
                    break;
                }
            }
        }
        let text = String::from_utf8_lossy(&antwort_bytes).into_owned();

        // ── Cookies aktualisieren und speichern ─
        {
            let mut jar = self.cookies.lock().unwrap();
            for (key, value) in &neue_cookies {
                jar.insert(key.clone(), value.clone());
            }
            jar.retain(|_k, v| !v.is_empty());
            drop(jar);
        }
        {
            let jar = self.cookies.lock().unwrap();
            speichere_cookies(&self.daten_pfad, &jar);
        }

        let mut ergebnis = format!("HTTP {status}\n\n{text}");
        if abgeschnitten {
            ergebnis.push_str(&format!(
                "\n\n[... Antwort auf {max} Bytes gekürzt, Original: {orig} Bytes ...]",
                max = MAX_ANTWORT_BYTES,
                orig = gesamtgroesse
            ));
        }
        if !neue_cookies.is_empty() {
            let cookie_namen: Vec<&str> = neue_cookies.iter().map(|(k, _)| k.as_str()).collect();
            ergebnis.push_str(&format!(
                "\n\n[Cookies gespeichert: {}]",
                cookie_namen.join(", ")
            ));
        }

        Ok(ergebnis)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_set_cookie_einfach() {
        let c = parse_set_cookie("session=abc123; Path=/; HttpOnly");
        assert_eq!(c, Some(("session".into(), "abc123".into())));
    }

    #[test]
    fn parse_set_cookie_leerer_teil() {
        assert_eq!(parse_set_cookie(""), None);
        assert_eq!(parse_set_cookie("; Path=/"), None);
    }

    #[tokio::test]
    async fn ist_private_erkennt_localhost() {
        assert!(ist_private_url("http://localhost:8080/api").await);
        assert!(ist_private_url("http://127.0.0.1/test").await);
        assert!(ist_private_url("http://192.168.1.1/admin").await);
        assert!(ist_private_url("http://10.0.0.1/foo").await);
        assert!(ist_private_url("http://172.16.0.1/bar").await);
        assert!(ist_private_url("http://172.31.255.254/baz").await);
    }

    /// Der eigentliche Grund für den Umbau auf echte IP-Klassifizierung
    /// (siehe `ist_private_ip`): die alte String-Präfix-Prüfung kannte
    /// 169.254.0.0/16 gar nicht - und genau das ist die Cloud-Metadaten-
    /// Adresse (169.254.169.254 bei AWS/GCP/Azure), über die SSRF-Angriffe
    /// typischerweise Zugangsdaten abgreifen.
    #[tokio::test]
    async fn ist_private_erkennt_link_local_metadata_adresse() {
        assert!(ist_private_url("http://169.254.169.254/latest/meta-data/").await);
        assert!(ist_private_url("http://169.254.1.1/").await);
    }

    #[tokio::test]
    async fn ist_private_erlaubt_oeffentliche() {
        // IP-Literale statt Hostnamen, damit der Test nicht von echtem DNS
        // abhängt (kicktipp.de/api.example.com wären sonst netzwerkabhängig
        // und potenziell flaky).
        assert!(!ist_private_url("https://1.1.1.1/login").await);
        assert!(!ist_private_url("https://8.8.8.8/data").await);
        assert!(!ist_private_url("http://172.15.0.1/foo").await);
        assert!(!ist_private_url("http://172.32.0.1/foo").await);
    }

    #[test]
    fn speichern_und_laden_roundtrip() {
        let tmp = std::env::temp_dir().join("famulus_test_cookies");
        let _ = std::fs::remove_file(&tmp.join(COOKIE_DATEI));

        let mut karte = HashMap::new();
        karte.insert("session".to_string(), "geheim".to_string());
        speichere_cookies(&tmp, &karte);

        let geladen = lade_cookies(&tmp);
        assert_eq!(geladen.get("session").unwrap(), "geheim");

        let _ = std::fs::remove_file(&tmp.join(COOKIE_DATEI));
    }
}