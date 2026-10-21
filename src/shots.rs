use std::collections::HashMap;

use crate::fleet::{EngineConfig, Lane, Strategy};

#[derive(Clone, Debug)]
pub struct Shot {
    pub lab: String,
    pub token: String,
    pub gid: String,
    pub password: String,
    pub is_h2: bool,
    pub sock_idx: usize,
}

pub fn build_shots(cfg: &EngineConfig) -> Vec<Shot> {
    let lanes = cfg.lanes;
    let mut raw = match cfg.strategy {
        Strategy::RoundRobin => round_robin(lanes),
        Strategy::LaneSerial | Strategy::TlsThenH2 => lane_serial(lanes),
        Strategy::H2First | Strategy::H2BurstTls => h2_first(lanes),
    };
    cap_shots(&mut raw, cfg.shot_cap);
    raw
}

/// Her lane kendi tls/h2 değerini kullanır — aynı gid olsa bile token başına tam bütçe
fn lane_budgets(lanes: &[Lane], budget: impl Fn(&Lane) -> u8) -> Vec<u8> {
    lanes.iter().map(|l| budget(l)).collect()
}

fn lane_serial(lanes: &[Lane]) -> Vec<Shot> {
    let tls_n = lane_budgets(lanes, |l| l.tls);
    let h2_n = lane_budgets(lanes, |l| l.h2);
    let mut out = Vec::new();
    for (lane, (&tls, &h2)) in lanes.iter().zip(tls_n.iter().zip(h2_n.iter())) {
        for i in 0..tls {
            out.push(mk_shot(lane, false, i as usize));
        }
        for i in 0..h2 {
            out.push(mk_shot(lane, true, i as usize));
        }
    }
    out
}

fn h2_first(lanes: &[Lane]) -> Vec<Shot> {
    let tls_n = lane_budgets(lanes, |l| l.tls);
    let h2_n = lane_budgets(lanes, |l| l.h2);
    let mut out = Vec::new();
    for (lane, &h2) in lanes.iter().zip(h2_n.iter()) {
        for i in 0..h2 {
            out.push(mk_shot(lane, true, i as usize));
        }
    }
    for (lane, &tls) in lanes.iter().zip(tls_n.iter()) {
        for i in 0..tls {
            out.push(mk_shot(lane, false, i as usize));
        }
    }
    out
}

fn round_robin(lanes: &[Lane]) -> Vec<Shot> {
    let tls_n = lane_budgets(lanes, |l| l.tls);
    let h2_n = lane_budgets(lanes, |l| l.h2);
    let max_r = tls_n
        .iter()
        .chain(h2_n.iter())
        .copied()
        .max()
        .unwrap_or(0) as usize;
    let mut out = Vec::new();
    for r in 0..max_r {
        for (lane, (&tls, &h2)) in lanes.iter().zip(tls_n.iter().zip(h2_n.iter())) {
            if r < tls as usize {
                out.push(mk_shot(lane, false, r));
            }
            if r < h2 as usize {
                out.push(mk_shot(lane, true, r));
            }
        }
    }
    out
}

fn mk_shot(lane: &Lane, is_h2: bool, sock_idx: usize) -> Shot {
    Shot {
        lab: lane.lab.to_string(),
        token: lane.token.to_string(),
        gid: lane.gid.to_string(),
        password: lane.password.to_string(),
        is_h2,
        sock_idx,
    }
}

fn cap_shots(shots: &mut Vec<Shot>, cap: usize) {
    if cap == 0 {
        return;
    }
    let mut seen: HashMap<(String, String), usize> = HashMap::new();
    shots.retain(|s| {
        let k = (s.token.clone(), s.gid.clone());
        let n = seen.entry(k).or_insert(0);
        if *n >= cap {
            return false;
        }
        *n += 1;
        true
    });
}

pub fn unique_tokens(lanes: &[Lane]) -> Vec<&Lane> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for lane in lanes {
        if seen.insert(lane.token) {
            out.push(lane);
        }
    }
    out
}

/// Probe — 2 token round-robin, n düz TLS
pub fn build_probe_dual_tls(lane_a: &Lane, lane_b: &Lane, n: usize) -> Vec<Shot> {
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let lane = if i % 2 == 0 { lane_a } else { lane_b };
        out.push(mk_shot(lane, false, i));
    }
    out
}

/// Probe — tek token: n düz TLS
pub fn build_probe_tls(lanes: &[Lane], token: &str, n: usize) -> Vec<Shot> {
    let glanes: Vec<&Lane> = lanes.iter().filter(|l| l.token == token).collect();
    if glanes.is_empty() {
        return Vec::new();
    }
    let tls_l = glanes.iter().find(|l| l.tls > 0).copied().unwrap_or(glanes[0]);
    (0..n).map(|i| mk_shot(tls_l, false, i)).collect()
}

/// @deprecated — H2 kapalı, build_probe_tls kullan
pub fn build_probe_221(lanes: &[Lane], token: &str) -> Vec<Shot> {
    build_probe_tls(lanes, token, 2)
}
