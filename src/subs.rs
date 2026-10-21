use std::collections::HashSet;

use anyhow::Result;
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde_json::Value;

use crate::fleet::EngineConfig;

const API: &str = "https://canary.discord.com/api/v9";
const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) discord/1.0.9017 Chrome/132.0.6834.83 Electron/34.0.0 Safari/537.36";
const SP: &str = "eyJvcyI6IldpbmRvd3MiLCJicm93c2VyIjoiRGlzY29yZCBDbGllbnQiLCJyZWxlYXNlX2NoYW5uZWwiOiJzdGFibGUiLCJjbGllbnRfdmVyc2lvbiI6IjEuMC45MDE3Iiwib3NfdmVyc2lvbiI6IjEwLjAuMjYyMDAiLCJvc19hcmNoIjoieDY0IiwiYXBwX2FyY2giOiJ4NjQiLCJzeXN0ZW1fbG9jYWxlIjoidHItVFIiLCJjbGllbnRfYnVpbGRfbnVtYmVyIjo1MTcwMDAsIm5hdGl2ZV9idWlsZF9udW1iZXIiOjc4MDAwfQ==";
const TIER3_BOOSTS: i64 = 14;

#[derive(Clone, Debug)]
pub struct DropHint {
    pub gid: String,
    pub vanity: String,
    pub guild_name: String,
    pub ends_at: DateTime<Utc>,
    pub label: String,
    pub current_boosts: i64,
    pub projected_boosts: i64,
}

fn client() -> Result<Client> {
    Ok(Client::builder()
        .danger_accept_invalid_certs(true)
        .use_rustls_tls()
        .build()?)
}

fn hdrs(token: &str) -> reqwest::header::HeaderMap {
    let mut h = reqwest::header::HeaderMap::new();
    h.insert(
        reqwest::header::AUTHORIZATION,
        token.parse().unwrap(),
    );
    h.insert(reqwest::header::USER_AGENT, UA.parse().unwrap());
    h.insert("X-Super-Properties", SP.parse().unwrap());
    h.insert(reqwest::header::ACCEPT, "*/*".parse().unwrap());
    h
}

async fn api_get(client: &Client, token: &str, path: &str) -> Result<(u16, Value)> {
    let resp = client
        .get(format!("{API}{path}"))
        .headers(hdrs(token))
        .send()
        .await?;
    let status = resp.status().as_u16();
    let text = resp.text().await.unwrap_or_default();
    let json: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    Ok((status, json))
}

fn parse_ends_at(v: &Value) -> Option<DateTime<Utc>> {
    let s = v.as_str()?;
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.with_timezone(&Utc))
}

fn sub_ended(s: &Value) -> bool {
    s.get("ended").and_then(|x| x.as_bool()) == Some(true)
        || s.get("ended_at").map(|x| !x.is_null()).unwrap_or(false)
}

fn is_sub_expiring(s: &Value) -> bool {
    !sub_ended(s) && s.get("ends_at").is_some() && s.get("pause_ends_at").is_none()
}

fn powerup_category(p: &Value) -> &str {
    p.get("sku")
        .and_then(|sku| sku.get("powerup_metadata"))
        .and_then(|m| m.get("category_type"))
        .and_then(|x| x.as_str())
        .or_else(|| {
            p.get("sku")
                .and_then(|sku| sku.get("tenant_metadata"))
                .and_then(|t| t.get("guild_monetization"))
                .and_then(|g| g.get("powerup"))
                .and_then(|pu| pu.get("category_type"))
                .and_then(|x| x.as_str())
        })
        .unwrap_or("")
}

fn is_level_powerup(p: &Value) -> bool {
    let slug = p
        .get("sku")
        .and_then(|s| s.get("slug"))
        .and_then(|x| x.as_str())
        .unwrap_or("");
    powerup_category(p) == "level" || slug.starts_with("level-")
}

fn is_perk_powerup(p: &Value) -> bool {
    powerup_category(p) == "perk"
}

fn parse_sub_expiring(subs: &[Value], now: DateTime<Utc>) -> Vec<(DateTime<Utc>, i64)> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for s in subs {
        if !is_sub_expiring(s) {
            continue;
        }
        if let Some(id) = s.get("id").and_then(|x| x.as_str()) {
            if !seen.insert(id.to_string()) {
                continue;
            }
        }
        if let Some(ends) = parse_ends_at(s.get("ends_at").unwrap_or(&Value::Null)) {
            if ends > now {
                out.push((ends, 1));
            }
        }
    }
    out.sort_by_key(|x| x.0);
    out
}

fn parse_level_expiring(powerups: &[Value], now: DateTime<Utc>) -> Vec<(DateTime<Utc>, i64)> {
    let mut out = Vec::new();
    for p in powerups {
        if !is_level_powerup(p) || is_perk_powerup(p) {
            continue;
        }
        let count = p
            .get("metadata")
            .and_then(|m| m.get("num_expiring_boosts"))
            .and_then(|x| x.as_i64())
            .unwrap_or(0);
        if count <= 0 {
            continue;
        }
        if let Some(ends) = parse_ends_at(p.get("ends_at").unwrap_or(&Value::Null)) {
            if ends > now {
                out.push((ends, count));
            }
        }
    }
    out.sort_by_key(|x| x.0);
    out
}

fn sku_has_vanity(p: &Value) -> bool {
    let feats = p
        .get("sku")
        .and_then(|sku| sku.get("powerup_metadata"))
        .and_then(|m| m.get("guild_features"))
        .and_then(|f| f.get("features"))
        .and_then(|x| x.as_array())
        .or_else(|| {
            p.get("sku")
                .and_then(|sku| sku.get("tenant_metadata"))
                .and_then(|t| t.get("guild_monetization"))
                .and_then(|g| g.get("powerup"))
                .and_then(|pu| pu.get("guild_features"))
                .and_then(|f| f.get("features"))
                .and_then(|x| x.as_array())
        });
    feats
        .map(|arr| arr.iter().any(|f| f.as_str() == Some("VANITY_URL")))
        .unwrap_or(false)
}

fn parse_level3_vanity(powerups: &[Value], now: DateTime<Utc>) -> Option<(DateTime<Utc>, i64)> {
    for p in powerups {
        let slug = p
            .get("sku")
            .and_then(|s| s.get("slug"))
            .and_then(|x| x.as_str())
            .unwrap_or("");
        if slug != "level-3" {
            continue;
        }
        if !sku_has_vanity(p) {
            continue;
        }
        let count = p
            .get("metadata")
            .and_then(|m| m.get("num_expiring_boosts"))
            .and_then(|x| x.as_i64())
            .unwrap_or(0);
        if count <= 0 {
            continue;
        }
        if let Some(ends) = parse_ends_at(p.get("ends_at").unwrap_or(&Value::Null)) {
            if ends > now {
                return Some((ends, count));
            }
        }
    }
    None
}

fn projected_after(current: i64, events: &[(DateTime<Utc>, i64)]) -> i64 {
    if events.is_empty() {
        return current;
    }
    (current - events[0].1).max(0)
}

async fn fetch_guild(client: &Client, token: &str, gid: &str) -> Option<Value> {
    let (status, j) = api_get(client, token, &format!("/guilds/{gid}?with_counts=true")).await.ok()?;
    if status == 200 {
        Some(j)
    } else {
        None
    }
}
/// REST live boost (GW backup / large guilds)
pub async fn fetch_live_boost(token: &str, gid: &str) -> Option<(u32, u32)> {
    let client = client().ok()?;
    let g = fetch_guild(&client, token, gid).await?;
    let boost = g
        .get("premium_subscription_count")
        .and_then(|x| x.as_u64())? as u32;
    let tier = g
        .get("premium_tier")
        .and_then(|x| x.as_u64())
        .unwrap_or(0) as u32;
    Some((boost, tier))
}

async fn fetch_powerups(client: &Client, token: &str, gid: &str) -> Vec<Value> {
    let q = "include_ends_at=true";
    let (status, j) = match api_get(client, token, &format!("/guilds/{gid}/powerups?{q}")).await {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    if status == 400 {
        if let Ok((s2, j2)) =
            api_get(client, token, &format!("/guilds/{gid}/powerups?country_code=US&{q}")).await
        {
            if s2 == 200 {
                return j2.as_array().cloned().unwrap_or_default();
            }
        }
        return Vec::new();
    }
    if status == 200 {
        j.as_array().cloned().unwrap_or_default()
    } else {
        Vec::new()
    }
}

async fn fetch_subs(client: &Client, token: &str, gid: &str) -> Vec<Value> {
    let (status, j) = match api_get(
        client,
        token,
        &format!("/guilds/{gid}/premium/subscriptions?include_ends_at=true"),
    )
    .await
    {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    if status == 200 {
        j.as_array().cloned().unwrap_or_default()
    } else {
        Vec::new()
    }
}

struct GuildDrop {
    hint: DropHint,
}

async fn analyze_guild(
    client: &Client,
    token: &str,
    gid: &str,
    meta_name: &str,
) -> Option<GuildDrop> {
    let g = fetch_guild(client, token, gid).await?;
    let vanity = g
        .get("vanity_url_code")
        .and_then(|x| x.as_str())
        .filter(|s| !s.is_empty())?;
    let name = g
        .get("name")
        .and_then(|x| x.as_str())
        .unwrap_or(meta_name);
    let current = g
        .get("premium_subscription_count")
        .and_then(|x| x.as_i64())
        .unwrap_or(0);

    if current < TIER3_BOOSTS {
        return None;
    }

    let now = Utc::now();
    let powerups = fetch_powerups(client, token, gid).await;
    let subs = fetch_subs(client, token, gid).await;

    let sub_events = parse_sub_expiring(&subs, now);
    let level_events = parse_level_expiring(&powerups, now);
    let has_level = !level_events.is_empty();
    let tier_events = if has_level {
        level_events
    } else {
        sub_events
    };

    let projected = projected_after(current, &tier_events);
    let l3 = parse_level3_vanity(&powerups, now);

    if projected >= TIER3_BOOSTS {
        if let Some((ends, count)) = l3 {
            let after = (current - count).max(0);
            return Some(GuildDrop {
                hint: DropHint {
                    gid: gid.to_string(),
                    vanity: vanity.to_string(),
                    guild_name: name.to_string(),
                    ends_at: ends,
                    label: "level-3-vanity".into(),
                    current_boosts: current,
                    projected_boosts: after,
                },
            });
        }
        return None;
    }

    let (ends, _) = tier_events.first().copied()?;
    Some(GuildDrop {
        hint: DropHint {
            gid: gid.to_string(),
            vanity: vanity.to_string(),
            guild_name: name.to_string(),
            ends_at: ends,
            label: if has_level {
                "level"
            } else {
                "boost-sub"
            }
            .into(),
            current_boosts: current,
            projected_boosts: projected,
        },
    })
}

async fn scan_listen_guilds(client: &Client, token: &str) -> Vec<DropHint> {
    let (status, guilds) = match api_get(client, token, "/users/@me/guilds?limit=200").await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("[subs] guilds err: {e}");
            return Vec::new();
        }
    };
    if status != 200 {
        tracing::warn!("[subs] guilds HTTP {status}");
        return Vec::new();
    }
    let Some(list) = guilds.as_array() else {
        return Vec::new();
    };
    tracing::info!("[subs] LISTEN auto-scan {} guild", list.len());
    let mut drops = Vec::new();
    let mut seen = HashSet::new();
    for meta in list {
        let Some(gid) = meta.get("id").and_then(|x| x.as_str()) else { continue };
        if !seen.insert(gid.to_string()) {
            continue;
        }
        let name = meta
            .get("name")
            .and_then(|x| x.as_str())
            .unwrap_or(gid);
        if let Some(gd) = analyze_guild(client, token, gid, name).await {
            tracing::info!(
                "[subs] ⚠ {} | gid={} | {}→{} | {} @ {}",
                gd.hint.guild_name,
                gd.hint.gid,
                gd.hint.current_boosts,
                gd.hint.projected_boosts,
                gd.hint.label,
                gd.hint.ends_at.format("%Y-%m-%d %H:%M:%S UTC")
            );
            drops.push(gd.hint);
        }
    }
    drops
}

/// listen1 + listen2 — her tokenin guildlerini tara (tek sunucudalar)
pub async fn fetch_nearest_drop(cfg: &EngineConfig) -> Result<Option<DropHint>> {
    let client = client()?;
    let tokens = cfg.listen_tokens();
    if tokens.is_empty() {
        tracing::warn!("[subs] listen token yok");
        return Ok(None);
    }

    let mut all = Vec::new();
    let mut seen_gid = HashSet::new();
    for (i, token) in tokens.iter().enumerate() {
        let label = if i == 0 { "LISTEN1" } else { "LISTEN2" };
        for hint in scan_listen_guilds(&client, token).await {
            tracing::info!(
                "[subs] {label} → {} | gid={} | {}→{} | {} @ {}",
                hint.guild_name,
                hint.gid,
                hint.current_boosts,
                hint.projected_boosts,
                hint.label,
                hint.ends_at.format("%Y-%m-%d %H:%M:%S UTC")
            );
            if seen_gid.insert(hint.gid.clone()) {
                all.push(hint);
            }
        }
    }

    if all.is_empty() {
        tracing::info!("[subs] boost drop bulunamadı");
        return Ok(None);
    }

    all.sort_by_key(|h| h.ends_at);
    let top = all[0].clone();
    tracing::info!(
        "[subs] en yakın → gid={} ({}) {}→{} @ {}",
        top.gid,
        top.guild_name,
        top.current_boosts,
        top.projected_boosts,
        top.ends_at.format("%Y-%m-%d %H:%M:%S UTC")
    );
    Ok(Some(top))
}
