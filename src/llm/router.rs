//! Provider-Router mit Fallback und Scorecard.
//!
//! `RouterProvider` implementiert `LlmProvider` genauso wie jeder echte
//! Anbieter - der Rest von Famulus (die Agent-Schleife) merkt nicht, dass
//! dahinter mehrere Provider stehen können. Ein Decorator, kein Umbau.
//!
//! Standardfall (kein Fallback konfiguriert): genau ein Provider in der
//! Liste, `next()` versucht ihn einmal, gibt Erfolg oder Fehler direkt
//! durch - exakt dasselbe Verhalten wie der Provider allein, nur mit
//! Scorecard-Eintrag nebenbei. Erst mit konfigurierten Fallbacks kommt der
//! Ausweich-Mechanismus überhaupt zum Tragen.
//!
//! Bewusst NICHT drin: dynamisches Umsortieren nach Erfolgsquote oder
//! kostenbasiertes Routing. Beides bräuchte mehr als das, was aktuell aus
//! den Provider-Antworten herauskommt (keine Token-/Kosten-Daten), und eine
//! Reihenfolge, die sich zur Laufzeit selbst ändert, ist schwerer
//! nachzuvollziehen als eine feste. Die Statistik wird trotzdem von Anfang
//! an mitgeschrieben - falls das später doch gebraucht wird, sind die Daten
//! schon da.

use super::{LlmAntwort, LlmProvider, Message, ToolDefinition};
#[cfg(not(test))]
use crate::memory::Gedaechtnis;
use async_trait::async_trait;
use std::time::Instant;

pub struct RouterProvider {
    providers: Vec<Box<dyn LlmProvider>>,
}

impl RouterProvider {
    /// `providers` darf nicht leer sein - `build_provider` stellt das sicher,
    /// indem der Hauptprovider immer an erster Stelle steht.
    pub fn new(providers: Vec<Box<dyn LlmProvider>>) -> Self {
        debug_assert!(!providers.is_empty(), "RouterProvider braucht mindestens einen Provider");
        Self { providers }
    }

    /// Schreibt die Statistik weg. Öffnet die DB bewusst frisch statt eine
    /// Verbindung durchzureichen - dieselbe Begründung wie bei
    /// `history_db()` in der GUI: so bleibt build_provider() unverändert
    /// aufrufbar, ohne dass jeder Aufrufer erst ein Gedächtnis besorgen
    /// muss. Scheitert das Protokollieren, ist das kein Grund, die
    /// eigentliche Modellantwort zu verwerfen - deshalb wird der Fehler
    /// hier verschluckt, nicht durchgereicht.
    #[cfg(not(test))]
    fn protokollieren(provider: &str, erfolg: bool, dauer: std::time::Duration) {
        if let Ok(g) = Gedaechtnis::standard() {
            let _ = g.provider_aufruf_protokollieren(provider, erfolg, dauer.as_millis() as u64);
        }
    }

    /// Test-Variante: schreibt bewusst NICHT in die echte gedaechtnis.db.
    /// Die Tests unten prüfen die Fallback-Logik, nicht den Seiteneffekt -
    /// ohne diesen Schalter würde jeder `cargo test`-Lauf die reale
    /// Datenbank mit Testdaten ("primaer"/"fallback") verschmutzen.
    #[cfg(test)]
    fn protokollieren(_provider: &str, _erfolg: bool, _dauer: std::time::Duration) {}
}

#[async_trait]
impl LlmProvider for RouterProvider {
    async fn next(
        &self,
        system: Option<&str>,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmAntwort> {
        let mut letzter_fehler = None;

        for (index, provider) in self.providers.iter().enumerate() {
            let start = Instant::now();
            match provider.next(system, messages, tools).await {
                Ok(antwort) => {
                    Self::protokollieren(provider.name(), true, start.elapsed());
                    return Ok(antwort);
                }
                Err(fehler) => {
                    Self::protokollieren(provider.name(), false, start.elapsed());
                    if index + 1 < self.providers.len() {
                        eprintln!(
                            "[router] {} fehlgeschlagen, versuche naechsten Provider: {fehler:#}",
                            provider.name()
                        );
                    }
                    letzter_fehler = Some(fehler);
                }
            }
        }

        Err(letzter_fehler.expect("providers darf nicht leer sein, siehe RouterProvider::new"))
    }

    fn name(&self) -> &'static str {
        self.providers[0].name()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::ToolCall;

    /// Ein Test-Provider, der entweder immer scheitert oder immer klappt -
    /// steuerbar, um den Fallback-Pfad ohne echten Netzwerkaufruf zu prüfen.
    struct FakeProvider {
        name: &'static str,
        klappt: bool,
    }

    #[async_trait]
    impl LlmProvider for FakeProvider {
        async fn next(
            &self,
            _system: Option<&str>,
            _messages: &[Message],
            _tools: &[ToolDefinition],
        ) -> anyhow::Result<LlmAntwort> {
            if self.klappt {
                Ok(LlmAntwort {
                    text: format!("Antwort von {}", self.name),
                    tool_calls: Vec::<ToolCall>::new(),
                })
            } else {
                anyhow::bail!("{} ist absichtlich kaputt", self.name)
            }
        }

        fn name(&self) -> &'static str {
            self.name
        }
    }

    #[tokio::test]
    async fn ohne_fallback_gibt_fehler_direkt_durch() {
        let router = RouterProvider::new(vec![Box::new(FakeProvider {
            name: "primaer",
            klappt: false,
        })]);
        let ergebnis = router.next(None, &[], &[]).await;
        assert!(ergebnis.is_err());
    }

    #[tokio::test]
    async fn fallback_greift_wenn_primaerer_provider_scheitert() {
        let router = RouterProvider::new(vec![
            Box::new(FakeProvider {
                name: "primaer",
                klappt: false,
            }),
            Box::new(FakeProvider {
                name: "fallback",
                klappt: true,
            }),
        ]);
        let antwort = router.next(None, &[], &[]).await.unwrap();
        assert_eq!(antwort.text, "Antwort von fallback");
    }

    #[tokio::test]
    async fn primaerer_provider_wird_bevorzugt_wenn_er_klappt() {
        let router = RouterProvider::new(vec![
            Box::new(FakeProvider {
                name: "primaer",
                klappt: true,
            }),
            Box::new(FakeProvider {
                name: "fallback",
                klappt: true,
            }),
        ]);
        let antwort = router.next(None, &[], &[]).await.unwrap();
        assert_eq!(antwort.text, "Antwort von primaer");
    }
}
