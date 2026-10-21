use anyhow::Result;
use kod1::{config, init_crypto, Engine};

#[tokio::main]
async fn main() -> Result<()> {
    init_crypto();
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .with_target(false)
        .init();
    Engine::new(config::cfg()).run().await
}
