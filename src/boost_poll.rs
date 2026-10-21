use anyhow::Result;

use crate::fleet::EngineConfig;
use crate::subs::{fetch_nearest_drop, DropHint};

/// Boot'ta tek sefer — subs ile drop tarihi. Drop yoksa bekleyip tekrar dener, bulunca API durur.
pub async fn wait_for_drop(cfg: &EngineConfig) -> Result<DropHint> {
    let name = cfg.name;
    loop {
        match fetch_nearest_drop(cfg).await {
            Ok(Some(h)) => {
                tracing::info!(
                    "[{name}] [subs] drop gg/{} ({}) {}→{} ends @ {} ({})",
                    h.vanity,
                    h.guild_name,
                    h.current_boosts,
                    h.projected_boosts,
                    h.ends_at.format("%Y-%m-%d %H:%M:%S UTC"),
                    h.label,
                );
                return Ok(h);
            }
            Ok(None) => {
                tracing::info!("[{name}] [subs] drop yok — 30s sonra tekrar");
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            }
            Err(e) => {
                tracing::warn!("[{name}] [subs] err: {e}");
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            }
        }
    }
}
