use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};

use crate::fleet::EngineConfig;
use crate::gw::WatchState;
use crate::subs::fetch_live_boost;

const POLL_LEAD_MS: i64 = 30_000;
const POLL_TAIL_MS: i64 = 120_000;
const POLL_INTERVAL_MS: u64 = 600;

/// GW yedek — büyük sunucularda GUILD_UPDATE gecikirse REST ile boost izle
pub async fn run_boost_rest_watch(
    cfg: EngineConfig,
    watch: WatchState,
    ends_at: DateTime<Utc>,
    on_arm: Arc<dyn Fn(u32, &'static str) + Send + Sync>,
) {
    let name = cfg.name;
    let boost_below = cfg.boost_below;
    let tokens = cfg.listen_tokens();
    if tokens.is_empty() {
        tracing::warn!("[{name}] REST boost watch — listener yok");
        return;
    }
    if !cfg.trigger.arms_on_boost() {
        tracing::info!("[{name}] REST boost watch — trigger boost degil, atlandi");
        return;
    }

    let start = ends_at - chrono::Duration::milliseconds(POLL_LEAD_MS);
    let end = ends_at + chrono::Duration::milliseconds(POLL_TAIL_MS);
    let wait_ms = (start - Utc::now()).num_milliseconds().max(0) as u64;
    if wait_ms > 0 {
        tracing::info!("[{name}] REST boost watch — {wait_ms}ms sonra başlıyor");
        tokio::time::sleep(Duration::from_millis(wait_ms)).await;
    } else {
        tracing::info!("[{name}] REST boost watch — hemen başlıyor");
    }

    let mut last_boost: Option<u32> = None;
    loop {
        if Utc::now() > end {
            tracing::info!("[{name}] REST boost watch — pencere bitti");
            break;
        }
        if *watch.fired.lock().await {
            tracing::info!("[{name}] REST boost watch — ateş edildi, duruyor");
            break;
        }

        let gid = watch.gid.lock().await.clone();
        if gid.is_empty() {
            tokio::time::sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;
            continue;
        }

        let mut seen = None;
        for (i, token) in tokens.iter().enumerate() {
            if let Some((boost, tier)) = fetch_live_boost(token, &gid).await {
                seen = Some((boost, tier));
                if boost < boost_below {
                    tracing::info!(
                        "[{name}] [REST#{i}] ARM boost={boost} tier={tier} gid={gid} (<{boost_below})"
                    );
                    on_arm(boost, "REST");
                    return;
                }
                break;
            }
        }

        if let Some((boost, tier)) = seen {
            if last_boost != Some(boost) {
                tracing::info!(
                    "[{name}] [REST] boost={boost} tier={tier} gid={gid} (izleniyor)"
                );
                last_boost = Some(boost);
            }
        }

        tokio::time::sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;
    }
}
