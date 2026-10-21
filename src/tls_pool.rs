use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use anyhow::Result;
use rustls::pki_types::ServerName;
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio_rustls::client::TlsStream;

use crate::tls::fire_connector;

pub type TlsConn = TlsStream<TcpStream>;

pub struct PoolSlot {
    pub max: usize,
    idle: Mutex<Vec<TlsConn>>,
}

pub struct TlsPool {
    host: String,
    slots: HashMap<(String, String), Arc<PoolSlot>>,
}

impl TlsPool {
    pub fn new(host: &str) -> Self {
        Self {
            host: host.to_string(),
            slots: HashMap::new(),
        }
    }

    pub fn register(&mut self, token: &str, gid: &str, max: usize) {
        let key = (token.to_string(), gid.to_string());
        self.slots
            .entry(key)
            .or_insert_with(|| Arc::new(PoolSlot { max, idle: Mutex::new(Vec::new()) }));
    }

    pub fn slot(&self, token: &str, gid: &str) -> Option<Arc<PoolSlot>> {
        self.slots
            .get(&(token.to_string(), gid.to_string()))
            .cloned()
    }

    pub async fn warm_keys(&self, keys: &[(&str, &str)], per_lane: usize) -> usize {
        let mut opened = 0usize;
        for (token, gid) in keys {
            let Some(slot) = self.slot(token, gid) else {
                continue;
            };
            let need = per_lane.saturating_sub(slot.idle.lock().await.len());
            for _ in 0..need {
                match open_conn(&self.host).await {
                    Ok(conn) => {
                        slot.idle.lock().await.push(conn);
                        opened += 1;
                    }
                    Err(e) => tracing::warn!("[tls-pool] warm err: {e}"),
                }
            }
        }
        opened
    }
}

pub async fn open_conn(host: &str) -> Result<TlsConn> {
    let stream = TcpStream::connect(format!("{host}:443")).await?;
    stream.set_nodelay(true)?;
    let sn = ServerName::try_from(host.to_string())?;
    Ok(fire_connector().connect(sn, stream).await?)
}

pub async fn take_conn(host: &str, slot: Option<&Arc<PoolSlot>>) -> Result<(TlsConn, bool)> {
    if let Some(slot) = slot {
        if let Some(conn) = slot.idle.lock().await.pop() {
            return Ok((conn, false));
        }
    }
    Ok((open_conn(host).await?, true))
}

pub async fn return_conn(slot: Option<&Arc<PoolSlot>>, conn: TlsConn) {
    let Some(slot) = slot else {
        return;
    };
    let mut idle = slot.idle.lock().await;
    if idle.len() < slot.max {
        idle.push(conn);
    }
}

pub fn unique_lane_keys(
    shots: &[(String, String, String, String, String, u64, usize)],
) -> Vec<(String, String)> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for (_, gid, token, _, _, _, _) in shots {
        if seen.insert((token.clone(), gid.clone())) {
            out.push((token.clone(), gid.clone()));
        }
    }
    out
}

/// Aynı token+gid için planlanan max shot (pool boyutu alt sınırı)
pub fn max_shots_per_lane(
    shots: &[(String, String, String, String, String, u64, usize)],
) -> usize {
    let mut counts: HashMap<(String, String), usize> = HashMap::new();
    for (_, gid, token, _, _, _, _) in shots {
        *counts.entry((token.clone(), gid.clone())).or_insert(0) += 1;
    }
    counts.values().copied().max().unwrap_or(1)
}

pub async fn build_pool(
    host: &str,
    shots: &[(String, String, String, String, String, u64, usize)],
    per_lane: usize,
) -> Arc<TlsPool> {
    let mut pool = TlsPool::new(host);
    for (token, gid) in unique_lane_keys(shots) {
        pool.register(&token, &gid, per_lane);
    }
    let keys: Vec<_> = pool
        .slots
        .keys()
        .map(|(t, g)| (t.as_str(), g.as_str()))
        .collect();
    let n = pool.warm_keys(&keys, per_lane).await;
    tracing::info!(
        "[tls-pool] {} lane × {per_lane} = {n} baglanti hazir ({host})",
        keys.len()
    );
    Arc::new(pool)
}
