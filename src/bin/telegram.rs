//! Startet Famulus als Telegram-Bot. Siehe `famulus_core::telegram` für die
//! eigentliche Logik - hier steht nur das Laden der `.env` (muss VOR dem
//! ersten `TelegramConfig::from_env()` passiert sein, `Config::load()`
//! macht das sonst erst später, siehe `config.rs`).

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    if let Some(home) = dirs::home_dir() {
        let _ = dotenvy::from_path(home.join(".famulus").join(".env"));
    }

    let cfg = famulus_core::telegram::TelegramConfig::from_env()?;
    famulus_core::telegram::run(cfg).await
}
