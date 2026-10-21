use crate::fleet::{EngineConfig, Lane, Strategy, TriggerMode};

const PW: &str = "chpetxben10";

pub fn cfg() -> EngineConfig {
    EngineConfig {
        name: "kod1",
        strategy: Strategy::LaneSerial,
        host: "chpetxben10.chpet.ben10.com",
        boost_below: 17, // :D
        mfa_early_ms: 300_000,
        mfa_final_ms: 60_000,
        gw_lead_ms: 30_000,
        gw_warmup_ms: 300_000,
        shot_cap: 22,
        t0_ms: 0,
        t1_ms: 21,
        burst_first: 0,
        split_schedule: None,
        trigger: TriggerMode::TierDrop,
        listen1: ".Gvi6i0.-7OWJqTRc",
        listen2: ".GPGQWS.",
        webhook: "https://discord.com/api/webhooks//-",
        tls_pool_per_lane: 3,
        lanes: &[
Lane { lab: "T1", token: ".GCE6NA.-", gid: "", password: PW, tls: 22, h2: 0 }, // el chavo 9
Lane { lab: "T2", token: ".GWDYWb.", gid: "", password: PW, tls: 22, h2: 0 }, // el chavo 10
Lane { lab: "T3", token: ".G-Pqi5.", gid: "", password: PW, tls: 22, h2: 0 }, // el chavo 13
Lane { lab: "T4", token: ".GQWhNO.-5XPZ_lc", gid: "", password: PW, tls: 22, h2: 0 }, // el chavo 5
        ],
    }
}
