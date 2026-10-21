use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use reqwest::Client;
use serde_json::Value;

use crate::fleet::{EngineConfig, TriggerMode};
use crate::gw::WatchState;

const API: &str = "https://canary.discord.com/api/v9";
const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) discord/1.0.9017 Chrome/132.0.6834.83 Electron/34.0.0 Safari/537.36";
const POLL_MS: u64 = 200;

fn client() -> Option<Client> {
    Client::builder()
        .danger_accept_invalid_certs(true)
        .use_rustls_tls()
        .build()
        .ok()
}

fn hdrs(token: &str) -> reqwest::header::HeaderMap {
    let mut h = reqwest::header::HeaderMap::new();
    h.insert(reqwest::header::AUTHORIZATION, token.parse().unwrap());
    h.insert(reqwest::header::USER_AGENT, UA.parse().unwrap());
    h.insert(reqwest::header::ACCEPT, "*/*".parse().unwrap());
    h
}

async fn get_json(client: &Client, token: &str, path: &str) -> Option<Value> {
    let resp = client
        .get(format!("{API}{path}"))
        .headers(hdrs(token))
        .send()
        .await
        .ok()?;
    let t = resp.text().await.ok()?;
    serde_json::from_str(&t).ok()
}

/// (status, body_json). network err → None
async fn get_guild_status(
    client: &Client,
    token: &str,
    gid: &str,
) -> Option<(u16, Option<Value>)> {
    let resp = client
        .get(format!("{API}/guilds/{gid}"))
        .headers(hdrs(token))
        .send()
        .await
        .ok()?;
    let status = resp.status().as_u16();
    let t = resp.text().await.ok()?;
    let j = serde_json::from_str(&t).ok();
    Some((status, j))
}

fn powerups_l3_count(items: &Value) -> usize {
    let Some(arr) = items.as_array() else { return 0 };
    arr.iter()
        .filter(|p| {
            let slug = p
                .get("sku")
                .and_then(|s| s.get("slug"))
                .and_then(|x| x.as_str())
                .unwrap_or("");
            slug == "level-3"
        })
        .count()
}

fn powerups_n(items: &Value) -> usize {
    items.as_array().map(|a| a.len()).unwrap_or(0)
}

fn subs_summary(items: &Value) -> (usize, usize) {
    let Some(arr) = items.as_array() else { return (0, 0) };
    let n = arr.len();
    let active = arr
        .iter()
        .filter(|s| s.get("ended").and_then(|x| x.as_bool()) != Some(true))
        .count();
    (n, active)
}

fn guild_boost(j: &Value) -> Option<u32> {
    j.get("premium_subscription_count")
        .and_then(|x| x.as_u64())
        .map(|x| x as u32)
}

fn invite_boost(j: &Value) -> Option<u32> {
    j.get("guild")
        .and_then(|g| g.get("premium_subscription_count"))
        .and_then(|x| x.as_u64())
        .map(|x| x as u32)
}

/// GW boost sonrasi REST /guilds 0ms — boost<14 confirm’e kadar (timeout YOK)
pub fn spawn_api_guild_confirm(
    name: &'static str,
    token: String,
    watch: WatchState,
    boost_below: u32,
    webhook: String,
    on_arm: Arc<dyn Fn(u32, &'static str) + Send + Sync>,
) {
    tokio::spawn(async move {
        let Some(client) = client() else {
            tracing::warn!("[{name}] API confirm — client yok");
            return;
        };
        tracing::info!("[{name}] ★ API-GUILD hot poll 0ms — confirm’e kadar (sonsuz)");
        let mut n: u64 = 0;
        let mut last_wh_status: u16 = 0;
        let mut last_wh_at = tokio::time::Instant::now() - Duration::from_secs(60);
        loop {
            if *watch.fired.lock().await {
                return;
            }
            let gid = watch.gid.lock().await.clone();
            if gid.is_empty() {
                tokio::task::yield_now().await;
                continue;
            }
            n += 1;
            match get_guild_status(&client, &token, &gid).await {
                Some((status, j)) => {
                    if status == 200 {
                        if let Some(b) = j.as_ref().and_then(guild_boost) {
                            if b < boost_below {
                                tracing::info!(
                                    "[{name}] ★ API GUILD confirm boost={b} (<{boost_below}) n={n}"
                                );
                                on_arm(b, "API-GUILD");
                                return;
                            }
                        }
                    } else if status == 401 || status == 403 || status == 429 || status >= 500 {
                        let now = tokio::time::Instant::now();
                        // ayni status icin en fazla 30sn’de bir webhook (spam yok)
                        if status != last_wh_status || now.duration_since(last_wh_at) >= Duration::from_secs(30)
                        {
                            last_wh_status = status;
                            last_wh_at = now;
                            tracing::warn!("[{name}] API-GUILD HTTP {status} n={n} — devam");
                            let wh = webhook.clone();
                            let desc = format!(
                                "**{name}** API-GUILD poll\nHTTP **{status}**\nn={n}\ngid=`{gid}`\n(devam ediyor, confirm bekleniyor)"
                            );
                            tokio::spawn(async move {
                                let _ = crate::webhook::send_webhook(
                                    &wh,
                                    &format!("⚠️ {name} | API-GUILD HTTP {status}"),
                                    &desc,
                                    0xFEE75C,
                                )
                                .await;
                            });
                        }
                    }
                }
                None => {
                    let now = tokio::time::Instant::now();
                    if last_wh_status != 1 || now.duration_since(last_wh_at) >= Duration::from_secs(30)
                    {
                        last_wh_status = 1; // network sentinel
                        last_wh_at = now;
                        tracing::warn!("[{name}] API-GUILD network err n={n} — devam");
                        let wh = webhook.clone();
                        let nn = n;
                        tokio::spawn(async move {
                            let _ = crate::webhook::send_webhook(
                                &wh,
                                &format!("⚠️ {name} | API-GUILD network"),
                                &format!("**{name}** API-GUILD poll network err n={nn}\n(devam)"),
                                0xFEE75C,
                            )
                            .await;
                        });
                    }
                }
            }
            tokio::task::yield_now().await;
        }
    });
}

/// REST powerups L3 / API guild / API invite / powerups-subs / GW→API hot
pub async fn run_signal_rest_watch(
    cfg: EngineConfig,
    watch: WatchState,
    ends_at: DateTime<Utc>,
    on_arm: Arc<dyn Fn(u32, &'static str) + Send + Sync>,
) {
    let name = cfg.name;
    let need = cfg.trigger.needs_rest_l3() || cfg.trigger.needs_api_watch();
    if !need {
        return;
    }
    let Some(client) = client() else { return };
    let token = cfg.listen_tokens().into_iter().next().unwrap_or("");
    if token.is_empty() {
        return;
    }

    let start = ends_at - chrono::Duration::milliseconds(30_000);
    let end = ends_at + chrono::Duration::milliseconds(180_000);
    let wait = (start - Utc::now()).num_milliseconds().max(0) as u64;
    if wait > 0 {
        tracing::info!("[{name}] signal REST watch — {wait}ms sonra");
        tokio::time::sleep(Duration::from_millis(wait)).await;
    }

    // GW boost → REST hot poll spawn confirm’e kadar; burada sadece GW bekle (timeout yok)
    if matches!(cfg.trigger, TriggerMode::GwThenApiGuild) {
        tracing::info!("[{name}] GwThenApiGuild — GW boost bekleniyor (timeout yok)");
        loop {
            if *watch.fired.lock().await {
                return;
            }
            if *watch.api_hot.lock().await {
                return;
            }
            tokio::select! {
                _ = watch.api_go.notified() => {}
                _ = tokio::time::sleep(Duration::from_millis(50)) => {}
            }
        }
    }

    let mut base_l3: Option<usize> = None;
    let mut base_n: Option<usize> = None;
    let mut base_subs: Option<(usize, usize)> = None;
    let mut base_guild_boost: Option<u32> = None;
    let mut base_invite_boost: Option<u32> = None;
    let mut saw_l3_gone = false;
    let mut saw_guild = false;
    let mut saw_invite = false;
    let mut powerups_waves = 0u8;

    tracing::info!("[{name}] signal REST watch aktif (trigger={:?})", cfg.trigger);

    loop {
        if Utc::now() > end || *watch.fired.lock().await {
            break;
        }
        let gid = watch.gid.lock().await.clone();
        if gid.is_empty() {
            tokio::time::sleep(Duration::from_millis(POLL_MS)).await;
            continue;
        }

        if cfg.trigger.needs_rest_l3() || matches!(cfg.trigger, TriggerMode::ApiPowerupsSubs { .. }) {
            if let Some(j) = get_json(&client, token, &format!("/guilds/{gid}/powerups?include_ends_at=true")).await {
                let l3 = powerups_l3_count(&j);
                let n = powerups_n(&j);
                if base_l3.is_none() {
                    base_l3 = Some(l3);
                    base_n = Some(n);
                } else {
                    if cfg.trigger.needs_rest_l3() && !saw_l3_gone {
                        if base_l3.unwrap_or(0) > 0 && l3 == 0 {
                            saw_l3_gone = true;
                            tracing::info!("[{name}] ★ REST L3 GONE n={n}");
                            on_arm(0, "REST-L3-GONE");
                        }
                    }
                    if let TriggerMode::ApiPowerupsSubs { .. } = cfg.trigger {
                        if let Some(bn) = base_n {
                            if n < bn && powerups_waves < 2 {
                                powerups_waves += 1;
                                base_n = Some(n);
                                tracing::info!("[{name}] ★ API powerups n {bn}→{n} wave={powerups_waves}");
                                on_arm(0, "API-POWERUPS");
                            }
                        }
                    }
                }
            }
        }

        if let TriggerMode::ApiPowerupsSubs { .. } = cfg.trigger {
            if let Some(j) = get_json(
                &client,
                token,
                &format!("/guilds/{gid}/premium/subscriptions?include_ends_at=true"),
            )
            .await
            {
                let sum = subs_summary(&j);
                if base_subs.is_none() {
                    base_subs = Some(sum);
                } else if let Some((bn, ba)) = base_subs {
                    if sum.0 < bn || sum.1 < ba {
                        base_subs = Some(sum);
                        tracing::info!("[{name}] ★ API subs {bn}/{ba}→{}/{}", sum.0, sum.1);
                        on_arm(0, "API-SUBS");
                    }
                }
            }
        }

        if matches!(cfg.trigger, TriggerMode::ApiGuild) && !saw_guild {
            if let Some(j) = get_json(&client, token, &format!("/guilds/{gid}")).await {
                if let Some(b) = guild_boost(&j) {
                    if base_guild_boost.is_none() {
                        base_guild_boost = Some(b);
                    } else if let Some(prev) = base_guild_boost {
                        if b < prev && b < cfg.boost_below {
                            saw_guild = true;
                            tracing::info!("[{name}] ★ API GUILD boost {prev}→{b}");
                            on_arm(b, "API-GUILD");
                        } else if b != prev {
                            base_guild_boost = Some(b);
                        }
                    }
                }
            }
        }

        if matches!(cfg.trigger, TriggerMode::ApiInvite) && !saw_invite {
            let vanity = watch.vanity.lock().await.clone();
            if !vanity.is_empty() {
                if let Some(j) = get_json(&client, token, &format!("/invites/{vanity}?with_counts=true")).await {
                    if let Some(b) = invite_boost(&j) {
                        if base_invite_boost.is_none() {
                            base_invite_boost = Some(b);
                        } else if let Some(prev) = base_invite_boost {
                            if b < prev && b < cfg.boost_below {
                                saw_invite = true;
                                tracing::info!("[{name}] ★ API INVITE boost {prev}→{b}");
                                on_arm(b, "API-INVITE");
                            } else if b != prev {
                                base_invite_boost = Some(b);
                            }
                        }
                    }
                }
            }
        }

        tokio::time::sleep(Duration::from_millis(POLL_MS)).await;
    }
    tracing::info!("[{name}] signal REST watch bitti");
}
