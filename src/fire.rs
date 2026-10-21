use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use bytes::Bytes;
use http::{Method, Request, Uri};
use rustls::pki_types::ServerName;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::classify::{classify, Bucket};
use crate::packet::{build_duz_tls_base, build_vanity_body, compose_duz_tls_packet, DUZ_SP, DUZ_UA};
use crate::tls::{fire_connector, fire_h2_connector};
use crate::tls_pool::{self, TlsPool};

const READ_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_HTTP_BYTES: usize = 64 * 1024;

#[derive(Debug, Default, Clone)]
pub struct LaneStats {
    counts: HashMap<String, usize>,
    pub wins_ms: Vec<u64>,
}

impl LaneStats {
    fn bump(&mut self, key: &str) {
        *self.counts.entry(key.to_string()).or_insert(0) += 1;
    }

    fn format_counts(&self) -> String {
        const ORDER: &[&str] = &["200", "400", "403", "429", "500", "pair", "cf", "err", "other"];
        ORDER
            .iter()
            .filter_map(|k| {
                let n = self.counts.get(*k).copied().unwrap_or(0);
                if n > 0 {
                    Some(format!("{n}x{k}"))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn format_block(&self, lab: &str) -> String {
        let counts = self.format_counts();
        if counts.is_empty() && self.wins_ms.is_empty() {
            return format!("**{lab}**\n—");
        }
        let mut out = format!("**{lab}**\n{counts}");
        for ms in &self.wins_ms {
            out.push_str(&format!("\n200 = {ms}ms istek"));
        }
        out
    }
}

#[derive(Debug, Default)]
pub struct FireStats {
    pub sent: usize,
    pub tls: usize,
    pub h2: usize,
    pub rsp: usize,
    pub c200: usize,
    pub c400: usize,
    pub c403: usize,
    pub c429: usize,
    pub c500: usize,
    pub c_pair: usize,
    pub c_cf: usize,
    pub c_err: usize,
    pub first_rsp_ms: Option<u64>,
    pub last_rsp_ms: Option<u64>,
    pub first_200_ms: Option<u64>,
    pub per_lane: HashMap<String, LaneStats>,
    pub lines: Vec<String>,
}

impl FireStats {
    fn lane(&mut self, lab: &str) -> &mut LaneStats {
        self.per_lane.entry(lab.to_string()).or_default()
    }

    pub fn note_rsp(&mut self, elapsed: u64, is_200: bool) {
        self.first_rsp_ms = Some(match self.first_rsp_ms {
            Some(f) => f.min(elapsed),
            None => elapsed,
        });
        self.last_rsp_ms = Some(match self.last_rsp_ms {
            Some(f) => f.max(elapsed),
            None => elapsed,
        });
        if is_200 {
            self.first_200_ms = Some(match self.first_200_ms {
                Some(f) => f.min(elapsed),
                None => elapsed,
            });
        }
    }

    pub fn record_shot(&mut self, lab: &str, delay_ms: u64, rsp_ms: u64, status: &str, body: &str) {
        self.rsp += 1;
        let key = match classify(status, body) {
            Bucket::Ok200 => {
                self.c200 += 1;
                self.lane(lab).wins_ms.push(delay_ms);
                self.note_rsp(rsp_ms, true);
                "200"
            }
            Bucket::Taken400 => {
                self.c400 += 1;
                self.note_rsp(rsp_ms, false);
                "400"
            }
            Bucket::Forbidden403 => {
                self.c403 += 1;
                self.note_rsp(rsp_ms, false);
                "403"
            }
            Bucket::Rate429 => {
                self.c429 += 1;
                self.note_rsp(rsp_ms, false);
                "429"
            }
            Bucket::Pair => {
                self.c_pair += 1;
                self.note_rsp(rsp_ms, false);
                "pair"
            }
            Bucket::Cf => {
                self.c_cf += 1;
                self.note_rsp(rsp_ms, false);
                "cf"
            }
            Bucket::Other => {
                self.note_rsp(rsp_ms, false);
                if status == "500" {
                    self.c500 += 1;
                    "500"
                } else {
                    "other"
                }
            }
        };
        self.lane(lab).bump(key);

        if self.lines.len() < 50 {
            let snippet: String = body.chars().take(120).collect();
            self.lines
                .push(format!("{lab}:tls@{delay_ms}ms:HTTP{status} {snippet}"));
        }
    }

    pub fn record_err(&mut self, lab: &str, delay_ms: u64) {
        self.c_err += 1;
        self.lane(lab).bump("err");
        if self.lines.len() < 50 {
            self.lines.push(format!("{lab}:tls@{delay_ms}ms:CONN_ERR"));
        }
    }

    pub fn record_err_at(&mut self, lab: &str, delay_ms: u64, rsp_ms: u64) {
        self.record_err(lab, delay_ms);
        self.note_rsp(rsp_ms, false);
    }

    pub fn format_lane_report(&self, lane_order: &[String]) -> String {
        lane_order
            .iter()
            .map(|lab| {
                self.per_lane
                    .get(lab)
                    .map(|ls| ls.format_block(lab))
                    .unwrap_or_else(|| format!("**{lab}**\n—"))
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    pub fn merge(&mut self, other: FireStats) {
        self.sent += other.sent;
        self.tls += other.tls;
        self.h2 += other.h2;
        self.rsp += other.rsp;
        self.c200 += other.c200;
        self.c400 += other.c400;
        self.c403 += other.c403;
        self.c429 += other.c429;
        self.c500 += other.c500;
        self.c_pair += other.c_pair;
        self.c_cf += other.c_cf;
        self.c_err += other.c_err;
        if let Some(e) = other.first_rsp_ms {
            self.first_rsp_ms = Some(match self.first_rsp_ms {
                Some(f) => f.min(e),
                None => e,
            });
        }
        if let Some(e) = other.last_rsp_ms {
            self.last_rsp_ms = Some(match self.last_rsp_ms {
                Some(f) => f.max(e),
                None => e,
            });
        }
        if let Some(e) = other.first_200_ms {
            self.first_200_ms = Some(match self.first_200_ms {
                Some(f) => f.min(e),
                None => e,
            });
        }
        for (lab, ls) in other.per_lane {
            let entry = self.per_lane.entry(lab).or_default();
            for (k, v) in ls.counts {
                *entry.counts.entry(k).or_insert(0) += v;
            }
            entry.wins_ms.extend(ls.wins_ms);
        }
        self.lines.extend(other.lines);
    }
}

fn conn_reusable(status: &str, body: &str) -> bool {
    matches!(classify(status, body), Bucket::Ok200 | Bucket::Taken400 | Bucket::Rate429)
}

fn parse_http_response(raw: &[u8]) -> Option<(String, String, bool)> {
    let text = std::str::from_utf8(raw).ok()?;
    let hdr_end = text.find("\r\n\r\n")?;
    let status = text
        .lines()
        .next()?
        .split_whitespace()
        .nth(1)?
        .to_string();
    let header = &text[..hdr_end];
    let body_start = hdr_end + 4;
    let mut content_len: Option<usize> = None;
    for line in header.lines().skip(1) {
        let lower = line.to_ascii_lowercase();
        if let Some(v) = lower.strip_prefix("content-length:") {
            content_len = v.trim().parse().ok();
            break;
        }
    }
    let body_slice = &raw[body_start..];
    if let Some(cl) = content_len {
        if body_slice.len() < cl {
            return None;
        }
        let body = String::from_utf8_lossy(&body_slice[..cl]).into_owned();
        return Some((status, body, true));
    }
    if body_slice.is_empty() && status.chars().all(|c| c.is_ascii_digit()) {
        return Some((status, String::new(), true));
    }
    if !body_slice.is_empty() {
        return Some((
            status,
            String::from_utf8_lossy(body_slice).into_owned(),
            true,
        ));
    }
    None
}

async fn read_tls_response(tls: &mut tls_pool::TlsConn) -> Result<(String, String)> {
    let mut raw = Vec::with_capacity(4096);
    let mut buf = [0u8; 2048];
    let deadline = tokio::time::Instant::now() + READ_TIMEOUT;

    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        if left.is_zero() {
            anyhow::bail!("read timeout");
        }
        let n = tokio::time::timeout(left, tls.read(&mut buf)).await??;
        if n == 0 {
            if let Some((status, body, _)) = parse_http_response(&raw) {
                return Ok((status, body));
            }
            anyhow::bail!("connection closed before full response");
        }
        raw.extend_from_slice(&buf[..n]);
        if raw.len() > MAX_HTTP_BYTES {
            anyhow::bail!("response too large");
        }
        if let Some((status, body, complete)) = parse_http_response(&raw) {
            if complete {
                return Ok((status, body));
            }
        }
    }
}

async fn send_patch_read(
    tls: &mut tls_pool::TlsConn,
    pkt: &[u8],
) -> Result<(String, String)> {
    tls.write_all(pkt).await?;
    tls.flush().await?;
    read_tls_response(tls).await
}

pub async fn fire_tls_pooled(
    host: &str,
    pool: Option<&Arc<TlsPool>>,
    gid: &str,
    token: &str,
    mfa: &str,
    cookie: &str,
    vanity: &str,
    lab: &str,
    delay_ms: u64,
    idx: usize,
    wave_t0: Instant,
) -> Result<FireStats> {
    let _tag = format!("{lab}:tls#{idx}@{delay_ms}ms");
    let ck = (!cookie.is_empty()).then_some(cookie);
    let base = build_duz_tls_base(host, gid, token, mfa, ck);
    let pkt = compose_duz_tls_packet(&base, vanity);
    let slot = pool.and_then(|p| p.slot(token, gid));

    let mut stats = FireStats::default();
    stats.sent = 1;
    stats.tls = 1;

    let (mut tls, _) = match tls_pool::take_conn(host, slot.as_ref()).await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("[fire] {lab} conn err: {e}");
            let elapsed = wave_t0.elapsed().as_millis() as u64;
            stats.record_err_at(lab, delay_ms, elapsed);
            return Ok(stats);
        }
    };

    match send_patch_read(&mut tls, &pkt).await {
        Ok((status, body)) => {
            let elapsed = wave_t0.elapsed().as_millis() as u64;
            stats.record_shot(lab, delay_ms, elapsed, &status, &body);
            if conn_reusable(&status, &body) {
                tls_pool::return_conn(slot.as_ref(), tls).await;
            }
        }
        Err(e) => {
            tracing::warn!("[fire] {lab} pool err: {e}");
            let elapsed = wave_t0.elapsed().as_millis() as u64;
            stats.record_err_at(lab, delay_ms, elapsed);
        }
    }
    Ok(stats)
}

pub async fn fire_tls_one(
    host: &str,
    gid: &str,
    token: &str,
    mfa: &str,
    cookie: &str,
    vanity: &str,
    lab: &str,
    delay_ms: u64,
    idx: usize,
) -> Result<FireStats> {
    fire_tls_pooled(host, None, gid, token, mfa, cookie, vanity, lab, delay_ms, idx, Instant::now()).await
}

pub async fn fire_h2_streams(
    host: &str,
    gid: &str,
    token: &str,
    mfa: &str,
    vanity: &str,
    lab: &str,
    delay_ms: u64,
    n_streams: u8,
) -> Result<FireStats> {
    let connector = fire_h2_connector();
    let stream = TcpStream::connect(format!("{host}:443")).await?;
    stream.set_nodelay(true)?;
    let sn = ServerName::try_from(host.to_string()).unwrap();
    let tls = connector.connect(sn, stream).await?;
    let (mut client, conn) = h2::client::handshake(tls).await?;
    tokio::spawn(async move {
        let _ = conn.await;
    });

    let body_bytes = build_vanity_body(vanity);
    let path = format!("/api/v9/guilds/{gid}/vanity-url");
    let mut stats = FireStats::default();
    let h2_t0 = Instant::now();

    for i in 0..n_streams.max(1) {
        let uri: Uri = format!("https://{host}{path}").parse()?;
        let req = Request::builder()
            .method(Method::PATCH)
            .uri(uri)
            .header("authorization", token)
            .header("content-type", "application/json")
            .header("user-agent", DUZ_UA)
            .header("x-super-properties", DUZ_SP)
            .header("x-discord-mfa-authorization", mfa)
            .body(())?;

        let (resp_fut, mut send) = client.send_request(req, false)?;
        send.send_data(Bytes::from(body_bytes.clone()), true)?;
        let resp = resp_fut.await?;
        let status = resp.status().as_u16().to_string();
        let mut body_buf = Vec::new();
        let mut body_stream = resp.into_body();
        while let Some(chunk) = body_stream.data().await {
            body_buf.extend_from_slice(&chunk?);
        }
        let text = String::from_utf8_lossy(&body_buf);
        stats.sent += 1;
        stats.h2 += 1;
        let elapsed = h2_t0.elapsed().as_millis() as u64;
            stats.record_shot(lab, delay_ms, elapsed, &status, &text);
        let _ = i;
    }
    Ok(stats)
}

pub async fn burst_fire(
    host: &str,
    shots: &[(String, String, String, String, bool, usize)],
    vanity: &str,
) -> FireStats {
    let mut handles = Vec::new();
    for (lab, gid, token, mfa, is_h2, idx) in shots {
        let host = host.to_string();
        let gid = gid.clone();
        let token = token.clone();
        let mfa = mfa.clone();
        let vanity = vanity.to_string();
        let lab = lab.clone();
        let is_h2 = *is_h2;
        let idx = *idx;
        handles.push(tokio::spawn(async move {
            if is_h2 {
                fire_h2_streams(&host, &gid, &token, &mfa, &vanity, &lab, 0, 2).await
            } else {
                fire_tls_one(&host, &gid, &token, &mfa, "", &vanity, &lab, 0, idx).await
            }
        }));
    }
    let mut total = FireStats::default();
    for h in handles {
        match h.await {
            Ok(Ok(part)) => total.merge(part),
            Ok(Err(e)) => tracing::warn!("[fire] shot err: {e}"),
            Err(e) => tracing::warn!("[fire] task panic: {e}"),
        }
    }
    total
}

pub async fn stagger_tls_fire(
    host: &str,
    shots: &[(String, String, String, String, String, usize)],
    vanity: &str,
    gap_ms: u64,
) -> FireStats {
    let mut total = FireStats::default();
    for (i, (lab, gid, token, mfa, _, idx)) in shots.iter().enumerate() {
        if i > 0 {
            tokio::time::sleep(Duration::from_millis(gap_ms)).await;
        }
        let delay_ms = i as u64 * gap_ms;
        match fire_tls_one(host, gid, token, mfa, "", vanity, lab, delay_ms, *idx).await {
            Ok(part) => total.merge(part),
            Err(e) => {
                tracing::warn!("[fire] {lab} err: {e}");
                total.record_err(lab, delay_ms);
            }
        }
    }
    total
}

pub type MfaCache = Arc<tokio::sync::Mutex<HashMap<String, MfaMeta>>>;

#[derive(Clone, Debug)]
pub struct MfaMeta {
    pub token: String,
    pub cookie: String,
    pub fetched_at: Instant,
}

pub async fn warm_tls(host: &str, n: usize) {
    for i in 0..n {
        if i > 0 {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let host = host.to_string();
        let _ = tokio::spawn(async move {
            if let Ok(stream) = TcpStream::connect(format!("{host}:443")).await {
                let _ = stream.set_nodelay(true);
                let connector = fire_connector();
                if let Ok(sn) = ServerName::try_from(host.clone()) {
                    let _ = connector.connect(sn, stream).await;
                }
            }
        })
        .await;
    }
}

pub async fn scheduled_tls_fire(
    host: &str,
    shots: &[(String, String, String, String, String, u64, usize)],
    vanity: &str,
    t0: Instant,
    pool: Option<Arc<TlsPool>>,
) -> FireStats {
    let mut handles = Vec::new();
    for (lab, gid, token, mfa, cookie, delay_ms, idx) in shots {
        let host = host.to_string();
        let gid = gid.clone();
        let token = token.clone();
        let mfa = mfa.clone();
        let cookie = cookie.clone();
        let vanity = vanity.to_string();
        let lab = lab.clone();
        let delay = *delay_ms;
        let idx = *idx;
        let pool = pool.clone();
        handles.push(tokio::spawn(async move {
            let target = t0 + Duration::from_millis(delay);
            if let Some(wait) = target.checked_duration_since(Instant::now()) {
                tokio::time::sleep(wait).await;
            }
            match fire_tls_pooled(
                &host,
                pool.as_ref(),
                &gid,
                &token,
                &mfa,
                &cookie,
                &vanity,
                &lab,
                delay,
                idx,
                t0,
            )
            .await
            {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!("[fire] {lab}@{delay}ms err: {e}");
                    let mut s = FireStats::default();
                    let elapsed = t0.elapsed().as_millis() as u64;
                    s.record_err_at(&lab, delay, elapsed);
                    s
                }
            }
        }));
    }
    let mut total = FireStats::default();
    for h in handles {
        match h.await {
            Ok(part) => total.merge(part),
            Err(e) => tracing::warn!("[fire] task panic: {e}"),
        }
    }
    total
}
