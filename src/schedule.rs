use crate::fleet::{EngineConfig, SplitSchedule};

const MIN_DELAY_MS: u64 = 1;

fn clamp_delays(mut times: Vec<u64>) -> Vec<u64> {
    for t in &mut times {
        if *t < MIN_DELAY_MS {
            *t = MIN_DELAY_MS;
        }
    }
    times
}

/// Eşit aralık: start … end, n adet (ms kesişmez, min 1ms)
pub fn build_even_spread(n: usize, start_ms: u64, end_ms: u64) -> Vec<u64> {
    if n == 0 {
        return Vec::new();
    }
    let start = if start_ms == 0 {
        0
    } else {
        start_ms.max(MIN_DELAY_MS)
    };
    let end = end_ms.max(start);
    if n == 1 {
        return vec![start];
    }
    if start >= end {
        return vec![end; n];
    }
    let mut times: Vec<u64> = (0..n)
        .map(|i| start + ((end - start) * i as u64) / (n - 1) as u64)
        .collect();
    for j in 1..times.len() {
        if times[j] <= times[j - 1] {
            times[j] = times[j - 1] + 1;
        }
        if times[j] > end {
            times[j] = end;
        }
    }
    times
}

/// İlk `burst_n` shot @ 0ms, kalanları start..end arası eşit yay
pub fn build_burst_zero_then_spread(
    n: usize,
    burst_n: usize,
    start_ms: u64,
    end_ms: u64,
) -> Vec<u64> {
    if n == 0 {
        return Vec::new();
    }
    let burst = burst_n.min(n);
    let mut times = vec![0u64; burst];
    let rest = n - burst;
    if rest > 0 {
        times.extend(build_even_spread(
            rest,
            start_ms.max(MIN_DELAY_MS),
            end_ms.max(start_ms.max(MIN_DELAY_MS)),
        ));
    }
    times
}

/// kod5: zero@0ms yok — hepsi mid/tail spread
fn build_split_schedule(n_shots: usize, s: SplitSchedule) -> Vec<u64> {
    let z = s.zero_count as usize;
    let m = s.mid_count as usize;
    let t = s.tail_count as usize;
    let mut times = Vec::with_capacity(n_shots);

    let z_n = z.min(n_shots);
    if z_n > 0 {
        times.extend(build_even_spread(
            z_n,
            s.mid_start_ms.max(MIN_DELAY_MS),
            s.mid_end_ms.max(MIN_DELAY_MS),
        ));
    }

    let m_n = m.min(n_shots.saturating_sub(times.len()));
    times.extend(build_even_spread(
        m_n,
        s.mid_start_ms.max(MIN_DELAY_MS),
        s.mid_end_ms.max(MIN_DELAY_MS),
    ));

    let t_n = t.min(n_shots.saturating_sub(times.len()));
    times.extend(build_even_spread(
        t_n,
        s.tail_start_ms.max(MIN_DELAY_MS),
        s.tail_end_ms.max(MIN_DELAY_MS),
    ));

    while times.len() < n_shots {
        times.push(s.tail_end_ms.max(MIN_DELAY_MS));
    }
    times.truncate(n_shots);
    clamp_delays(times)
}

/// Shot listesi için zamanlama (lane-serial sırasıyla)
pub fn build_shot_delays(cfg: &EngineConfig, n_shots: usize) -> Vec<u64> {
    if n_shots == 0 {
        return Vec::new();
    }
    // kod12: hepsi 0ms
    if cfg.t0_ms == 0 && cfg.t1_ms == 0 {
        return vec![0u64; n_shots];
    }
    let times = if let Some(s) = cfg.split_schedule {
        build_split_schedule(n_shots, s)
    } else if cfg.burst_first > 0 && cfg.t0_ms == 0 {
        return build_burst_zero_then_spread(
            n_shots,
            cfg.burst_first as usize,
            1,
            cfg.t1_ms,
        );
    } else {
        let start = cfg.t0_ms;
        let end = cfg.t1_ms.max(start);
        build_even_spread(n_shots, start, end)
    };
    // 0ms baslangic korunur (kod11 0-50)
    if cfg.t0_ms == 0 {
        times
    } else {
        clamp_delays(times)
    }
}
