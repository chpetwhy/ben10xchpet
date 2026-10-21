#[derive(Clone, Debug)]
pub struct Lane {
    pub lab: &'static str,
    pub token: &'static str,
    pub gid: &'static str,
    pub password: &'static str,
    pub tls: u8,
    pub h2: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TriggerMode {
    /// Boost <14 → t0..t1
    BoostWindow,
    /// GW GUILD_POWERUP_ENTITLEMENTS_DELETE → t0..t1
    PowerupL3Delete,
    /// Boost <14 → hepsi 0ms
    BoostZero,
    /// GUILD_UPDATE: VANITY_URL feature silindi → 0ms
    FeatureVanityUrl,
    /// GUILD_UPDATE: premium_tier 3→alt → 0ms
    TierDrop,
    L3GwThenRest { gw_shots: u8, rest_shots: u8 },
    ApiPowerupsSubs { wave_shots: u8 },
    /// REST GET /guilds/{id} boost dususu → t0..t1
    ApiGuild,
    /// GW boost dususu → REST /guilds 0ms poll → boost<14 teyit → fire t0..t1
    GwThenApiGuild,
    /// REST GET /invites/{vanity} degisim → t0..t1
    ApiInvite,
    /// REST powerups L3 kayboldu → t0..t1
    RestL3Gone,
}

impl TriggerMode {
    pub fn arms_on_boost(self) -> bool {
        matches!(self, Self::BoostWindow | Self::BoostZero)
    }
    pub fn arms_on_powerup_l3_delete(self) -> bool {
        matches!(self, Self::PowerupL3Delete | Self::L3GwThenRest { .. })
    }
    pub fn arms_on_feature_vanity(self) -> bool {
        matches!(self, Self::FeatureVanityUrl)
    }
    pub fn arms_on_tier_drop(self) -> bool {
        matches!(self, Self::TierDrop)
    }
    pub fn needs_rest_l3(self) -> bool {
        matches!(self, Self::L3GwThenRest { .. } | Self::RestL3Gone)
    }
    pub fn needs_api_watch(self) -> bool {
        matches!(
            self,
            Self::ApiPowerupsSubs { .. }
                | Self::ApiGuild
                | Self::GwThenApiGuild
                | Self::ApiInvite
        )
    }
    pub fn is_gw_then_api_guild(self) -> bool {
        matches!(self, Self::GwThenApiGuild)
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Strategy {
    RoundRobin,
    LaneSerial,
    TlsThenH2,
    H2First,
    H2BurstTls,
}

impl Strategy {
    pub fn from_str(s: &str) -> Self {
        match s {
            "round-robin" => Self::RoundRobin,
            "lane-serial" => Self::LaneSerial,
            "tls-then-h2" => Self::TlsThenH2,
            "h2-first" => Self::H2First,
            "h2-burst-tls" => Self::H2BurstTls,
            _ => Self::TlsThenH2,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SplitSchedule {
    pub zero_count: u8,
    pub mid_start_ms: u64,
    pub mid_end_ms: u64,
    pub mid_count: u8,
    pub tail_start_ms: u64,
    pub tail_end_ms: u64,
    pub tail_count: u8,
}

#[derive(Clone, Debug)]
pub struct EngineConfig {
    pub name: &'static str,
    pub strategy: Strategy,
    pub host: &'static str,
    pub boost_below: u32,
    pub mfa_early_ms: u64,
    pub mfa_final_ms: u64,
    pub gw_lead_ms: u64,
    pub gw_warmup_ms: u64,
    pub shot_cap: usize,
    pub t0_ms: u64,
    pub t1_ms: u64,
    pub burst_first: u32,
    pub split_schedule: Option<SplitSchedule>,
    pub trigger: TriggerMode,
    /// Cift GW listener — her biri boost dusen tek sunucuda
    pub listen1: &'static str,
    pub listen2: &'static str,
    pub webhook: &'static str,
    pub lanes: &'static [Lane],
    pub tls_pool_per_lane: u8,
}

impl EngineConfig {
    pub const FLEET: &'static str = "Benten Rush";
    pub const DEFAULT_PW: &'static str = "bentenbabadir123";

    pub fn listen_tokens(&self) -> Vec<&'static str> {
        [self.listen1, self.listen2]
            .into_iter()
            .filter(|t| !t.is_empty() && !t.contains("BURAYA"))
            .collect()
    }
}
