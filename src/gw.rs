use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::sync::{Mutex, Notify};
use tokio::time::Instant;
use tokio_tungstenite::{connect_async, tungstenite::Message};

use crate::fleet::{EngineConfig, TriggerMode};

#[derive(Clone)]
pub struct WatchState {
    pub vanity: Arc<Mutex<String>>,
    pub gid: Arc<Mutex<String>>,
    pub ready: Arc<Notify>,
    pub armed: Arc<Mutex<bool>>,
    pub fired: Arc<Mutex<bool>>,
    pub shots_used: Arc<Mutex<usize>>,
    pub done_gw_l3: Arc<Mutex<bool>>,
    pub done_rest_l3: Arc<Mutex<bool>>,
    pub last_had_vanity_feat: Arc<Mutex<Option<bool>>>,
    pub last_tier: Arc<Mutex<Option<u32>>>,
    /// GwThenApiGuild: GW boost gelince true → REST 0ms poll baslar
    pub api_hot: Arc<Mutex<bool>>,
    pub api_go: Arc<Notify>,
}

impl WatchState {
    pub fn new() -> Self {
        Self {
            vanity: Arc::new(Mutex::new(String::new())),
            gid: Arc::new(Mutex::new(String::new())),
            ready: Arc::new(Notify::new()),
            armed: Arc::new(Mutex::new(false)),
            fired: Arc::new(Mutex::new(false)),
            shots_used: Arc::new(Mutex::new(0)),
            done_gw_l3: Arc::new(Mutex::new(false)),
            done_rest_l3: Arc::new(Mutex::new(false)),
            last_had_vanity_feat: Arc::new(Mutex::new(None)),
            last_tier: Arc::new(Mutex::new(None)),
            api_hot: Arc::new(Mutex::new(false)),
            api_go: Arc::new(Notify::new()),
        }
    }
}

enum GwExit {
    /// op7 / socket koptu — session koru, resume dene
    Reconnect,
    /// op9 resume=false — yeni identify
    Reidentify,
}

pub async fn run_gw_listeners(
    cfg: &EngineConfig,
    watch: WatchState,
    on_arm: Arc<dyn Fn(u32, &'static str) + Send + Sync>,
) {
    let tokens = cfg.listen_tokens();
    for (i, token) in tokens.iter().enumerate() {
        if token.is_empty() || token.contains("BURAYA") {
            continue;
        }
        for dup in 0..2u8 {
            let watch = watch.clone();
            let on_arm = on_arm.clone();
            let boost_below = cfg.boost_below;
            let trigger = cfg.trigger;
            let name = cfg.name;
            let webhook = cfg.webhook;
            let token = token.to_string();
            tokio::spawn(async move {
                let mut session_id: Option<String> = None;
                let mut resume_url: Option<String> = None;
                let mut seq: Option<u64> = None;
                loop {
                    match gw_session(
                        &token,
                        i,
                        dup,
                        &watch,
                        boost_below,
                        trigger,
                        &on_arm,
                        name,
                        webhook,
                        &mut session_id,
                        &mut resume_url,
                        &mut seq,
                    )
                    .await
                    {
                        GwExit::Reconnect => {}
                        GwExit::Reidentify => {
                            session_id = None;
                            seq = None;
                            tokio::time::sleep(Duration::from_millis(100)).await;
                        }
                    }
                }
            });
        }
    }
}

fn boost_signal(d: &Value, boost_below: u32) -> Option<u32> {
    if let Some(tier) = d.get("premium_tier").and_then(|x| x.as_u64()) {
        if tier < 3 {
            let boost = d
                .get("premium_subscription_count")
                .and_then(|x| x.as_u64())
                .map(|x| x as u32)
                .unwrap_or(boost_below.saturating_sub(1));
            return Some(boost);
        }
    }
    let boost = d.get("premium_subscription_count").and_then(|x| x.as_u64())? as u32;
    if boost < boost_below {
        Some(boost)
    } else {
        None
    }
}

async fn try_arm(
    watch: &WatchState,
    boost: u32,
    on_arm: &Arc<dyn Fn(u32, &'static str) + Send + Sync>,
    name: &str,
    event_gid: &str,
    source: &'static str,
) {
    let vanity = watch.vanity.lock().await.clone();
    if vanity.is_empty() {
        return;
    }
    if *watch.armed.lock().await {
        return;
    }
    *watch.armed.lock().await = true;
    tracing::info!("[{name}] [{source}] ARM boost={boost} gid={event_gid} vanity={vanity}");
    on_arm(boost, source);
    let watch = watch.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(15)).await;
        *watch.armed.lock().await = false;
    });
}

async fn handle_guild_payload(
    g: &Value,
    watch: &WatchState,
    boost_below: u32,
    trigger: TriggerMode,
    on_arm: &Arc<dyn Fn(u32, &'static str) + Send + Sync>,
    name: &'static str,
    source: &'static str,
    listen_token: &str,
    webhook: &'static str,
) {
    let gid = g.get("id").and_then(|x| x.as_str()).unwrap_or("");
    let watch_gid = watch.gid.lock().await.clone();
    if !watch_gid.is_empty() && gid != watch_gid {
        return;
    }
    if let Some(v) = g
        .get("vanity_url_code")
        .and_then(|x| x.as_str())
        .filter(|s| !s.is_empty())
    {
        let mut wv = watch.vanity.lock().await;
        if wv.is_empty() {
            *wv = v.to_lowercase();
            if !gid.is_empty() {
                *watch.gid.lock().await = gid.to_string();
            }
            tracing::info!("[{name}] WATCH vanity={v} ({source})");
            watch.ready.notify_waiters();
        }
    }
    let boost_now = g
        .get("premium_subscription_count")
        .and_then(|x| x.as_u64())
        .map(|x| x as u32);
    let tier_now = g
        .get("premium_tier")
        .and_then(|x| x.as_u64())
        .map(|x| x as u32);
    if !watch_gid.is_empty() && gid == watch_gid {
        if let (Some(boost), Some(tier)) = (boost_now, tier_now) {
            tracing::info!(
                "[{name}] [{source}] watch gid boost={boost} tier={tier} (<{boost_below})"
            );
        }
    }

    // kod13: VANITY_URL feature kayboldu
    if let Some(feats) = g.get("features").and_then(|x| x.as_array()) {
        let has = feats.iter().any(|x| x.as_str() == Some("VANITY_URL"));
        let mut prev = watch.last_had_vanity_feat.lock().await;
        if trigger.arms_on_feature_vanity() {
            if *prev == Some(true) && !has {
                let vanity = watch.vanity.lock().await.clone();
                if !vanity.is_empty() {
                    tracing::info!("[{name}] ★ FEATURE_VANITY_URL GONE → ARM");
                    on_arm(boost_now.unwrap_or(0), "FEATURE-VANITY-URL");
                }
            }
        }
        *prev = Some(has);
    }

    // kod14: tier 3 → alt
    if let Some(tier) = tier_now {
        let mut prev = watch.last_tier.lock().await;
        if trigger.arms_on_tier_drop() {
            if let Some(pt) = *prev {
                if pt >= 3 && tier < 3 {
                    let vanity = watch.vanity.lock().await.clone();
                    if !vanity.is_empty() {
                        tracing::info!("[{name}] ★ TIER_DROP {pt}→{tier} → ARM");
                        on_arm(boost_now.unwrap_or(0), "TIER-DROP");
                    }
                }
            }
        }
        *prev = Some(tier);
    }

    if trigger.is_gw_then_api_guild() {
        if let Some(boost) = boost_signal(g, boost_below) {
            let vanity = watch.vanity.lock().await.clone();
            if !vanity.is_empty() {
                let mut hot = watch.api_hot.lock().await;
                if !*hot {
                    *hot = true;
                    drop(hot);
                    tracing::info!(
                        "[{name}] ★ GW BOOST={boost} → API-GUILD hot poll 0ms ({source})"
                    );
                    watch.api_go.notify_waiters();
                    crate::signal_watch::spawn_api_guild_confirm(
                        name,
                        listen_token.to_string(),
                        watch.clone(),
                        boost_below,
                        webhook.to_string(),
                        on_arm.clone(),
                    );
                }
            }
        }
    } else if trigger.arms_on_boost() {
        if let Some(boost) = boost_signal(g, boost_below) {
            try_arm(watch, boost, on_arm, name, gid, source).await;
        }
    }
}

fn powerup_is_l3(d: &Value) -> bool {
    let ents = d
        .get("entitlements")
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default();
    let one = d.get("entitlement").cloned();
    let list: Vec<Value> = if !ents.is_empty() {
        ents
    } else if let Some(e) = one {
        vec![e]
    } else {
        Vec::new()
    };
    for e in list {
        let slug = e
            .get("sku")
            .and_then(|s| s.get("slug"))
            .and_then(|x| x.as_str())
            .unwrap_or("");
        if slug == "level-3" || slug.contains("level-3") {
            return true;
        }
        let feats = e
            .get("sku")
            .and_then(|s| s.get("powerup_metadata"))
            .and_then(|m| m.get("guild_features"))
            .and_then(|g| g.get("features"))
            .and_then(|f| f.as_array());
        if let Some(arr) = feats {
            if arr.iter().any(|x| x.as_str() == Some("VANITY_URL")) {
                return true;
            }
        }
    }
    // bos payload olsa bile DELETE eventinde L3 varsay (guvenli tetik)
    true
}

async fn handle_powerup_delete(
    d: &Value,
    watch: &WatchState,
    trigger: TriggerMode,
    on_arm: &Arc<dyn Fn(u32, &'static str) + Send + Sync>,
    name: &str,
) {
    if !trigger.arms_on_powerup_l3_delete() {
        return;
    }
    if !powerup_is_l3(d) {
        return;
    }
    let vanity = watch.vanity.lock().await.clone();
    if vanity.is_empty() {
        return;
    }
    tracing::info!("[{name}] ★ GW POWERUP_L3_DELETE → ARM");
    on_arm(0, "GW-L3-DELETE");
}

async fn send_identify(
    write: &mut futures_util::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        Message,
    >,
    token: &str,
) -> anyhow::Result<()> {
    let identify = serde_json::json!({
        "op": 2,
        "d": {
            "token": token,
            "intents": 1,
            "properties": { "os": "linux", "browser": "disco", "device": "disco" },
            "compress": false
        }
    });
    write.send(Message::Text(identify.to_string().into())).await?;
    Ok(())
}

async fn send_resume(
    write: &mut futures_util::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        Message,
    >,
    token: &str,
    session_id: &str,
    seq: Option<u64>,
) -> anyhow::Result<()> {
    let resume = serde_json::json!({
        "op": 6,
        "d": { "token": token, "session_id": session_id, "seq": seq }
    });
    write.send(Message::Text(resume.to_string().into())).await?;
    Ok(())
}

async fn gw_session(
    token: &str,
    idx: usize,
    dup: u8,
    watch: &WatchState,
    boost_below: u32,
    trigger: TriggerMode,
    on_arm: &Arc<dyn Fn(u32, &'static str) + Send + Sync>,
    name: &'static str,
    webhook: &'static str,
    session_id: &mut Option<String>,
    resume_url: &mut Option<String>,
    seq: &mut Option<u64>,
) -> GwExit {
    let gw_url = resume_url
        .as_deref()
        .unwrap_or("wss://gateway.discord.gg/?v=10&encoding=json");
    let (ws, _) = match connect_async(gw_url).await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("[{name}] [GW#{idx}.{dup}] connect err: {e}");
            return GwExit::Reconnect;
        }
    };

    let resuming = session_id.is_some();
    tracing::info!(
        "[{name}] [GW#{idx}.{dup}] connect{}",
        if resuming { " resume" } else { "" }
    );

    let (mut write, mut read) = ws.split();
    let mut hb_ms = 41250u64;
    let mut last_hb_ack = Instant::now();
    let mut session_ready = false;

    loop {
        let since_ack = last_hb_ack.elapsed().as_millis() as u64;
        // Discord: interval'in ~%90'ında heartbeat at (ACK op11 gelene kadar say)
        let send_at = hb_ms.saturating_mul(9) / 10;
        let hb_wait = if since_ack >= send_at {
            Duration::from_millis(0)
        } else {
            Duration::from_millis(send_at - since_ack)
        };

        tokio::select! {
            biased;
            msg = read.next() => {
                let Some(msg) = msg else {
                    tracing::warn!("[{name}] [GW#{idx}.{dup}] socket kapandı");
                    return GwExit::Reconnect;
                };
                let msg = match msg {
                    Ok(m) => m,
                    Err(e) => {
                        tracing::warn!("[{name}] [GW#{idx}.{dup}] read err: {e}");
                        return GwExit::Reconnect;
                    }
                };

                let text = match msg {
                    Message::Text(t) => t.to_string(),
                    Message::Binary(b) => String::from_utf8_lossy(&b).into_owned(),
                    Message::Close(frame) => {
                        tracing::warn!("[{name}] [GW#{idx}.{dup}] close: {frame:?}");
                        return GwExit::Reconnect;
                    }
                    Message::Ping(p) => {
                        let _ = write.send(Message::Pong(p)).await;
                        continue;
                    }
                    _ => continue,
                };

                let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
                let op = v.get("op").and_then(|o| o.as_i64()).unwrap_or(-1);

                if op == 10 {
                    hb_ms = v
                        .get("d")
                        .and_then(|d| d.get("heartbeat_interval"))
                        .and_then(|x| x.as_u64())
                        .unwrap_or(41250);
                    last_hb_ack = Instant::now();
                    session_ready = false;

                    if let Some(sid) = session_id.clone() {
                        if send_resume(&mut write, token, &sid, *seq).await.is_err() {
                            return GwExit::Reconnect;
                        }
                    } else if let Err(e) = send_identify(&mut write, token).await {
                        tracing::warn!("[{name}] [GW#{idx}.{dup}] identify err: {e}");
                        return GwExit::Reconnect;
                    }
                    continue;
                }

                if op == 11 {
                    last_hb_ack = Instant::now();
                    continue;
                }

                if op == 7 {
                    tracing::warn!("[{name}] [GW#{idx}.{dup}] op7 → resume");
                    return GwExit::Reconnect;
                }

                if op == 9 {
                    let can_resume = v.get("d").and_then(|x| x.as_bool()).unwrap_or(false);
                    tracing::warn!("[{name}] [GW#{idx}.{dup}] op9 resume={can_resume}");
                    return if can_resume {
                        GwExit::Reconnect
                    } else {
                        GwExit::Reidentify
                    };
                }

                if op == 1 {
                    // sunucu heartbeat istedi — hemen ack bekle, biz zaten gönderiyoruz
                    continue;
                }

                if op == 0 {
                    if let Some(s) = v.get("s").and_then(|x| x.as_u64()) {
                        *seq = Some(s);
                    }
                    let t = v.get("t").and_then(|t| t.as_str()).unwrap_or("");
                    let d = v.get("d").cloned().unwrap_or(Value::Null);

                    if t == "READY" {
                        session_ready = true;
                        if let Some(sid) = d.get("session_id").and_then(|x| x.as_str()) {
                            *session_id = Some(sid.to_string());
                        }
                        if let Some(url) = d.get("resume_gateway_url").and_then(|x| x.as_str()) {
                            *resume_url = Some(url.to_string());
                        }
                        tracing::info!("[{name}] [GW#{idx}.{dup}] READY");
                        if let Some(guilds) = d.get("guilds").and_then(|g| g.as_array()) {
                            for g in guilds {
                                handle_guild_payload(g, watch, boost_below, trigger, on_arm, name, "GW-READY", token, webhook).await;
                            }
                        }
                    } else if t == "RESUMED" {
                        session_ready = true;
                        tracing::info!("[{name}] [GW#{idx}.{dup}] RESUMED seq={:?}", seq);
                    } else if t == "GUILD_UPDATE" || t == "GUILD_CREATE" {
                        let t_arm = Instant::now();
                        handle_guild_payload(&d, watch, boost_below, trigger, on_arm, name, "GW", token, webhook).await;
                        if *watch.armed.lock().await {
                            tracing::debug!("[{name}] [GW#{idx}.{dup}] GUILD_UPDATE işlendi {:?}", t_arm.elapsed());
                        }
                    } else if t == "GUILD_POWERUP_ENTITLEMENTS_DELETE"
                        || t == "GUILD_POWERUP_ENTITLEMENTS_CREATE"
                        || t == "GUILD_POWERUP_ENTITLEMENTS_UPDATE"
                    {
                        if t.ends_with("DELETE") {
                            handle_powerup_delete(&d, watch, trigger, on_arm, name).await;
                        }
                    }
                }
            }
            _ = tokio::time::sleep(hb_wait) => {
                if !session_ready {
                    continue;
                }
                let payload = match seq {
                    Some(s) => format!(r#"{{"op":1,"d":{s}}}"#),
                    None => r#"{"op":1,"d":null}"#.to_string(),
                };
                if write.send(Message::Text(payload.into())).await.is_err() {
                    tracing::warn!("[{name}] [GW#{idx}.{dup}] heartbeat gönderilemedi");
                    return GwExit::Reconnect;
                }
            }
        }
    }
}
