//! Position-anchor stall detector. Catches the case where the ball
//! ping-pongs in a small region (each ownership flip resets the
//! owned/unowned counters but the ball physically goes nowhere). The
//! anchor advances naturally during normal play; only a genuinely
//! stuck region trips the safety net and force-kicks the ball clear.

use crate::r#match::engine::ball::ball::Ball;
use crate::r#match::{MatchPlayer, PlayerSide};
use nalgebra::Vector3;

/// Who the ball dies on.
///
/// A stalled ball is nearly always a state machine with no way to act on
/// possession: the player owns the ball, his state has no `has_ball`
/// exit (or returns "stay put"), his `velocity()` is zero, and the same
/// evaluation runs again next tick. It is a fixed point, and the only
/// thing that ends it is `detect_position_stall` force-kicking the ball
/// clear — 1000 ticks later.
///
/// Three of those were found by hand off a replay dump, which does not
/// scale: by the time the obvious ones are fixed the rest are too rare to
/// catch in a handful of matches. This attributes every stuck tick to the
/// state that was holding the ball, so the next one shows up as a row in
/// a 200-match table instead of a needle in a replay.
///
/// TICKS ARE FULL TICKS (~20 ms) — `detect_position_stall` runs from
/// `Ball::update` only, never `update_light`. That is also why the
/// safety net fires at ~19.5 s rather than the 10 s its own constant
/// comment claims.
#[cfg(feature = "match-logs")]
pub mod dead_ball_diag {
    use crate::r#match::ActivityIntensity;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// Widest `PlayerState::compact_id()` plus headroom (forwards top
    /// out at 418 today).
    pub const STATES: usize = 448;
    const ZERO: AtomicU64 = AtomicU64::new(0);

    /// Full ticks the ball spent stuck while a player in this state held
    /// it, and how many separate episodes that was.
    pub static STUCK_TICKS_BY_STATE: [AtomicU64; STATES] = [ZERO; STATES];
    pub static STUCK_EPISODES_BY_STATE: [AtomicU64; STATES] = [ZERO; STATES];
    /// The dwell split, per state. Without it a row in the stuck table
    /// does not say which of the two failures it is — see the module
    /// note on `OWNER_DWELL`. Flattened `state * 5 + bucket`.
    pub static STUCK_DWELL_BY_STATE: [AtomicU64; STATES * 5] = [ZERO; STATES * 5];
    /// Same, for a stall nobody owned — a loose ball sitting untouched.
    pub static STUCK_TICKS_UNOWNED: AtomicU64 = AtomicU64::new(0);
    pub static STUCK_EPISODES_UNOWNED: AtomicU64 = AtomicU64::new(0);
    /// Longest single stall seen, in full ticks.
    pub static LONGEST_STUCK: AtomicU64 = AtomicU64::new(0);

    /// How long the owner had been in his state when a stuck tick was
    /// attributed to him, bucketed 0-1 / 2-10 / 11-50 / 51-250 / 250+ AI
    /// ticks. This separates the two ways a state can top the stuck
    /// table and they need opposite fixes: a state everybody passes
    /// THROUGH on the way into possession collects one tick per
    /// ownership grant and reads 0-1; a state that genuinely holds the
    /// ball and does nothing reads high.
    pub static OWNER_DWELL: [AtomicU64; 5] = [ZERO; 5];
    /// Same split, restricted to the four TakeBall states.
    pub static TAKEBALL_DWELL: [AtomicU64; 5] = [ZERO; 5];
    /// Distance from the attributed owner to the ball, in tenths of a
    /// unit, summed — so the mean says whether he is standing on it.
    pub static TAKEBALL_OWNER_DIST_X10: AtomicU64 = AtomicU64::new(0);
    pub static TAKEBALL_SAMPLES: AtomicU64 = AtomicU64::new(0);

    /// Every full tick the ball was owned by a player in a `TakeBall`
    /// state, and how many separate spells that was — over the WHOLE
    /// match, not just stalls. Mean ticks per spell settles the question
    /// the stuck table cannot: is `TakeBall` a state that holds the ball
    /// and does nothing, or is it just the state everybody is in on the
    /// tick they claim it?
    pub static TAKEBALL_OWN_TICKS: AtomicU64 = AtomicU64::new(0);
    pub static TAKEBALL_OWN_SPELLS: AtomicU64 = AtomicU64::new(0);

    /// Is anybody actually pressing the man on the ball?
    ///
    /// The single most-reported thing you cannot see in any existing
    /// counter: tackles and interceptions only count contact that
    /// HAPPENED, and the defensive-shape block measures where the line
    /// sits, not whether anyone engages the carrier. This samples the
    /// distance from the ball's owner to his nearest opponent on every
    /// full tick he holds it, bucketed in metres.
    ///
    /// 0 <2m, 1 2-5m, 2 5-10m, 3 10-20m, 4 20m+.
    pub static CARRIER_PRESSURE: [AtomicU64; 5] = [ZERO; 5];
    pub const PRESSURE_LABELS: [&str; 5] = ["<2m", "2-5m", "5-10m", "10-20m", "20m+"];
    /// Same, split by which third of the pitch the carrier is in
    /// (own / middle / attacking, from the CARRIER's point of view),
    /// flattened `third * 5 + bucket`.
    pub static CARRIER_PRESSURE_BY_THIRD: [AtomicU64; 15] = [ZERO; 15];
    /// Summed nearest-opponent distance in tenths of a unit, for a mean.
    pub static CARRIER_NEAREST_X10: AtomicU64 = AtomicU64::new(0);
    pub static CARRIER_SAMPLES: AtomicU64 = AtomicU64::new(0);
    /// How many opponents are within 10 m of the carrier — the "does
    /// anyone come to him" number, not just the nearest.
    pub static CARRIER_ENGAGERS: AtomicU64 = AtomicU64::new(0);

    /// **Can the nearest defender physically stay with the man on the
    /// ball?**
    ///
    /// Every other counter here measures whether somebody is NEAR the
    /// carrier. None of them can answer the question underneath the most
    /// persistent report about this engine — "the defender just runs
    /// alongside him and never gets there" — because a defender who is
    /// permanently three metres behind looks identical, in every existing
    /// statistic, to one who is closing.
    ///
    /// The speed a player is allowed this tick is not his top speed. It
    /// is `max_speed * carry_mult` for the man on the ball (0.75-1.00,
    /// from `movement_speed_with_ball`) and `max_speed *
    /// MovementEffort::speed_fraction(intensity)` for everybody else
    /// (0.12-0.95). Those two ceilings are set by completely different
    /// code and nothing compares them, so a chase can be lost before
    /// either player moves.
    ///
    /// Sampled on every full tick the ball is owned, for the carrier and
    /// his nearest opponent: the two CEILINGS, the two actual speeds, and
    /// how often the chaser's ceiling is the lower of the pair.
    pub static CHASE_SAMPLES: AtomicU64 = AtomicU64::new(0);
    /// Speed ceilings this tick, x1000 (u/tick), summed.
    pub static CHASE_CARRIER_CAP_X1000: AtomicU64 = AtomicU64::new(0);
    pub static CHASE_CHASER_CAP_X1000: AtomicU64 = AtomicU64::new(0);
    /// Actual speeds, x1000 (u/tick), summed.
    pub static CHASE_CARRIER_SPD_X1000: AtomicU64 = AtomicU64::new(0);
    pub static CHASE_CHASER_SPD_X1000: AtomicU64 = AtomicU64::new(0);
    /// Samples where the chaser's CEILING is below the carrier's — the
    /// races that cannot be won however well he steers.
    pub static CHASE_OUTPACED: AtomicU64 = AtomicU64::new(0);
    /// The chaser's declared effort tier, indexed like
    /// `ActivityIntensity` (0 VeryHigh … 4 Recovery). A chase run in a
    /// `High` or `Moderate` state is a mislabelled state, not a slow
    /// player.
    pub static CHASE_TIER: [AtomicU64; 5] = [ZERO; 5];
    pub const CHASE_TIER_LABELS: [&str; 5] = ["VeryHigh", "High", "Moderate", "Low", "Recovery"];

    /// **How far a footballer actually runs, and at what effort.**
    ///
    /// Every other counter in this module is about the ball. This one is
    /// about the twenty-two, and it exists because the viewer's own census
    /// read **17.6 km per ninety minutes** off a recorded match — a median
    /// of 3.26 m/s sustained for the whole game, against the 10-12 km a real
    /// footballer covers. A squad that never stops moving is the one thing
    /// no amount of animation can fix, and nothing here measured it.
    ///
    /// Fed from [`MatchPlayer::move_to`], which is the single point every
    /// player's position is integrated at, so it cannot miss a path — and
    /// the sampling lives HERE rather than there, because a census that
    /// spells itself out at its call site is a census that goes stale the
    /// first time somebody adds a counter.
    pub struct MotionCensus;

    impl MotionCensus {
        /// One player, one tick: how far he moved, at what declared effort,
        /// and whether that effort's ceiling was what stopped him.
        ///
        /// ⚠ Behind `match-logs` at the call site, and it has to be: this is
        /// the hottest loop in the engine — twenty-two players a hundred
        /// times a second — and half a dozen atomics a tick is not a
        /// measurement anybody should pay for in a shipped season.
        pub fn note(covered: f32, ceiling: f32, intensity: ActivityIntensity, keeper: bool) {
            let covered = if covered.is_finite() { covered } else { 0.0 };
            if keeper {
                MOTION_KEEPER_TICKS.fetch_add(1, Ordering::Relaxed);
                MOTION_KEEPER_DISTANCE_X1000
                    .fetch_add((covered * 1000.0) as u64, Ordering::Relaxed);
                return;
            }
            MOTION_TICKS.fetch_add(1, Ordering::Relaxed);
            MOTION_DISTANCE_X1000.fetch_add((covered * 1000.0) as u64, Ordering::Relaxed);
            // 1.2 m/s is a walk; below it he is standing or strolling.
            if covered < Self::WALKING_PACE {
                MOTION_STILL.fetch_add(1, Ordering::Relaxed);
            }
            MOTION_TIER[Self::tier(intensity)].fetch_add(1, Ordering::Relaxed);
            MOTION_CEILING_X1000.fetch_add((ceiling * 1000.0) as u64, Ordering::Relaxed);
            if ceiling > 0.0 && covered >= ceiling * Self::AT_THE_CEILING {
                MOTION_AT_CEILING.fetch_add(1, Ordering::Relaxed);
            }
        }

        /// `(km per 90 outfield, km per 90 keeper, share below a walk, tier
        /// shares, share at the ceiling, mean ceiling in u/tick)`
        pub fn snapshot() -> (f32, f32, f32, [f32; 5], f32, f32) {
            let ticks = MOTION_TICKS.load(Ordering::Relaxed).max(1);
            let keeper_ticks = MOTION_KEEPER_TICKS.load(Ordering::Relaxed).max(1);
            let mut tiers = [0f32; 5];
            for (i, share) in tiers.iter_mut().enumerate() {
                *share = MOTION_TIER[i].load(Ordering::Relaxed) as f32 / ticks as f32;
            }
            (
                Self::per_90(MOTION_DISTANCE_X1000.load(Ordering::Relaxed), ticks),
                Self::per_90(
                    MOTION_KEEPER_DISTANCE_X1000.load(Ordering::Relaxed),
                    keeper_ticks,
                ),
                MOTION_STILL.load(Ordering::Relaxed) as f32 / ticks as f32,
                tiers,
                MOTION_AT_CEILING.load(Ordering::Relaxed) as f32 / ticks as f32,
                MOTION_CEILING_X1000.load(Ordering::Relaxed) as f32 / (ticks as f32 * 1000.0),
            )
        }

        /// Thousandths of a game unit, over this many player-ticks, as
        /// kilometres over a ninety-minute match.
        ///
        /// A unit is 0.125 m and a tick is 10 ms — see
        /// `MATCH_TIME_INCREMENT_MS` — so a hundred ticks are a second and
        /// 5400 seconds are a match.
        fn per_90(units: u64, ticks: u64) -> f32 {
            const METRES_PER_MILLI_UNIT: f64 = 0.125 / 1000.0;
            const TICKS_PER_SECOND: f64 = 100.0;
            const SECONDS: f64 = 5400.0;
            (units as f64 * METRES_PER_MILLI_UNIT / ticks as f64 * TICKS_PER_SECOND * SECONDS
                / 1000.0) as f32
        }

        /// Indexed like [`ActivityIntensity`] and like
        /// [`CHASE_TIER_LABELS`], which is what prints them.
        fn tier(intensity: ActivityIntensity) -> usize {
            match intensity {
                ActivityIntensity::VeryHigh => 0,
                ActivityIntensity::High => 1,
                ActivityIntensity::Moderate => 2,
                ActivityIntensity::Low => 3,
                ActivityIntensity::Recovery => 4,
            }
        }

        /// A walking pace, in units a tick: 1.2 m/s.
        const WALKING_PACE: f32 = 0.096;
        /// How close to the ceiling counts as being held by it.
        const AT_THE_CEILING: f32 = 0.97;
    }

    pub static MOTION_TICKS: AtomicU64 = AtomicU64::new(0);
    /// Ground covered, in thousandths of a game unit (a unit is 0.125 m).
    pub static MOTION_DISTANCE_X1000: AtomicU64 = AtomicU64::new(0);
    /// Ticks spent below a walking pace — 1.2 m/s, or 0.096 u/tick.
    pub static MOTION_STILL: AtomicU64 = AtomicU64::new(0);
    /// …and the declared effort tier, indexed like `ActivityIntensity`.
    pub static MOTION_TIER: [AtomicU64; 5] = [ZERO; 5];
    /// Ticks the player is at his own speed CEILING — which decides
    /// whether `ActivityIntensity` is even the lever for total distance.
    /// A ceiling that rarely binds cannot be lowered to slow anybody down;
    /// it can only be lowered to stop him keeping up when it matters.
    pub static MOTION_AT_CEILING: AtomicU64 = AtomicU64::new(0);
    /// …and the mean ceiling itself, x1000 u/tick.
    pub static MOTION_CEILING_X1000: AtomicU64 = AtomicU64::new(0);
    /// Outfielders only above: a keeper's match is a different shape, and
    /// averaging him in hides both.
    pub static MOTION_KEEPER_TICKS: AtomicU64 = AtomicU64::new(0);
    pub static MOTION_KEEPER_DISTANCE_X1000: AtomicU64 = AtomicU64::new(0);

    /// `(samples, mean carrier cap, mean chaser cap, mean carrier speed,
    ///   mean chaser speed, share outpaced, tier counts)`
    pub fn chase_speed_snapshot() -> (u64, f32, f32, f32, f32, f32, [u64; 5]) {
        let n = CHASE_SAMPLES.load(Ordering::Relaxed);
        let d = n.max(1) as f32 * 1000.0;
        let mut tiers = [0u64; 5];
        for (i, t) in tiers.iter_mut().enumerate() {
            *t = CHASE_TIER[i].load(Ordering::Relaxed);
        }
        (
            n,
            CHASE_CARRIER_CAP_X1000.load(Ordering::Relaxed) as f32 / d,
            CHASE_CHASER_CAP_X1000.load(Ordering::Relaxed) as f32 / d,
            CHASE_CARRIER_SPD_X1000.load(Ordering::Relaxed) as f32 / d,
            CHASE_CHASER_SPD_X1000.load(Ordering::Relaxed) as f32 / d,
            CHASE_OUTPACED.load(Ordering::Relaxed) as f32 / n.max(1) as f32,
            tiers,
        )
    }

    /// `(bucket counts, by-third counts, mean nearest in metres, mean
    /// engagers within 10 m)`.
    pub fn carrier_pressure_snapshot() -> ([u64; 5], [u64; 15], f32, f32) {
        let mut buckets = [0u64; 5];
        for i in 0..5 {
            buckets[i] = CARRIER_PRESSURE[i].load(Ordering::Relaxed);
        }
        let mut thirds = [0u64; 15];
        for i in 0..15 {
            thirds[i] = CARRIER_PRESSURE_BY_THIRD[i].load(Ordering::Relaxed);
        }
        let n = CARRIER_SAMPLES.load(Ordering::Relaxed).max(1);
        (
            buckets,
            thirds,
            CARRIER_NEAREST_X10.load(Ordering::Relaxed) as f32 / n as f32 / 10.0 * 0.125,
            CARRIER_ENGAGERS.load(Ordering::Relaxed) as f32 / n as f32,
        )
    }

    /// What actually happens INSIDE a stall.
    ///
    /// Every remaining row in the stuck table now reads 100% in the 0-1
    /// dwell bucket, i.e. it is the state a player happens to be in on
    /// the tick he claims the ball. So the question is no longer "which
    /// state holds it" but "why does it keep changing hands inside two
    /// metres". These answer that: how many times possession turns over
    /// during a stall, whether it alternates between the SIDES (a real
    /// scramble) or bounces around one of them (a passing problem), and
    /// where on the pitch it happens.
    pub static STALL_TURNOVERS: AtomicU64 = AtomicU64::new(0);
    pub static STALL_TURNOVERS_CROSS_TEAM: AtomicU64 = AtomicU64::new(0);
    /// Stall ticks by zone: 0 near a corner flag, 1 in either penalty
    /// area, 2 near a touchline, 3 open play.
    pub static STALL_ZONE: [AtomicU64; 4] = [ZERO; 4];
    pub const ZONE_LABELS: [&str; 4] = ["corner flag", "penalty area", "touchline", "open play"];
    /// Stall ticks where the ball was in a protected flight window (a
    /// pass or set-piece delivery in progress).
    pub static STALL_IN_FLIGHT: AtomicU64 = AtomicU64::new(0);

    /// `(turnovers during stalls, of which cross-team, zone histogram,
    /// in-flight ticks)`.
    pub fn stall_churn_snapshot() -> (u64, u64, [u64; 4], u64) {
        let mut zones = [0u64; 4];
        for i in 0..4 {
            zones[i] = STALL_ZONE[i].load(Ordering::Relaxed);
        }
        (
            STALL_TURNOVERS.load(Ordering::Relaxed),
            STALL_TURNOVERS_CROSS_TEAM.load(Ordering::Relaxed),
            zones,
            STALL_IN_FLIGHT.load(Ordering::Relaxed),
        )
    }

    /// Ownership churn. A stall whose owner is always freshly-entered
    /// into `TakeBall` means possession is being granted and revoked in
    /// a tight cycle; these say how tight and how often.
    pub static OWNERSHIP_GAINED: AtomicU64 = AtomicU64::new(0);
    pub static OWNERSHIP_LOST: AtomicU64 = AtomicU64::new(0);
    /// Gains where the SAME player had it a moment ago — a re-claim
    /// rather than a genuine change of possession.
    pub static OWNERSHIP_RECLAIMED_SELF: AtomicU64 = AtomicU64::new(0);
    /// How long each spell of possession lasted, bucketed in full ticks:
    /// 1 / 2-5 / 6-25 / 26-100 / 100+.
    pub static SPELL_LENGTH: [AtomicU64; 5] = [ZERO; 5];

    #[inline]
    pub fn spell_bucket(ticks: u32) -> usize {
        match ticks {
            0..=1 => 0,
            2..=5 => 1,
            6..=25 => 2,
            26..=100 => 3,
            _ => 4,
        }
    }

    pub const SPELL_LABELS: [&str; 5] = ["1", "2-5", "6-25", "26-100", "100+"];

    /// Dwell split for one state id, as percentages of its stuck ticks.
    pub fn dwell_for_state(state: u16) -> [u64; 5] {
        let mut out = [0u64; 5];
        let base = state as usize * 5;
        for i in 0..5 {
            out[i] = STUCK_DWELL_BY_STATE[base + i].load(Ordering::Relaxed);
        }
        out
    }

    /// `(ticks a TakeBall state owned the ball, distinct spells)`.
    pub fn takeball_ownership_snapshot() -> (u64, u64) {
        (
            TAKEBALL_OWN_TICKS.load(Ordering::Relaxed),
            TAKEBALL_OWN_SPELLS.load(Ordering::Relaxed),
        )
    }

    /// `(gained, lost, self-reclaims, spell-length histogram)`.
    pub fn churn_snapshot() -> (u64, u64, u64, [u64; 5]) {
        let mut spells = [0u64; 5];
        for i in 0..5 {
            spells[i] = SPELL_LENGTH[i].load(Ordering::Relaxed);
        }
        (
            OWNERSHIP_GAINED.load(Ordering::Relaxed),
            OWNERSHIP_LOST.load(Ordering::Relaxed),
            OWNERSHIP_RECLAIMED_SELF.load(Ordering::Relaxed),
            spells,
        )
    }

    #[inline]
    pub fn dwell_bucket(in_state_time: u64) -> usize {
        match in_state_time {
            0..=1 => 0,
            2..=10 => 1,
            11..=50 => 2,
            51..=250 => 3,
            _ => 4,
        }
    }

    pub const DWELL_LABELS: [&str; 5] = ["0-1", "2-10", "11-50", "51-250", "250+"];

    pub fn reset() {
        for c in STUCK_TICKS_BY_STATE
            .iter()
            .chain(STUCK_EPISODES_BY_STATE.iter())
            .chain(STUCK_DWELL_BY_STATE.iter())
        {
            c.store(0, Ordering::Relaxed);
        }
        for c in OWNER_DWELL
            .iter()
            .chain(TAKEBALL_DWELL.iter())
            .chain(SPELL_LENGTH.iter())
            .chain(STALL_ZONE.iter())
            .chain(CARRIER_PRESSURE.iter())
            .chain(CARRIER_PRESSURE_BY_THIRD.iter())
            .chain(CHASE_TIER.iter())
            .chain(MOTION_TIER.iter())
        {
            c.store(0, Ordering::Relaxed);
        }
        for c in [
            &STALL_TURNOVERS,
            &STALL_TURNOVERS_CROSS_TEAM,
            &STALL_IN_FLIGHT,
            &CARRIER_NEAREST_X10,
            &CARRIER_SAMPLES,
            &CARRIER_ENGAGERS,
            &CHASE_SAMPLES,
            &MOTION_TICKS,
            &MOTION_DISTANCE_X1000,
            &MOTION_STILL,
            &MOTION_AT_CEILING,
            &MOTION_CEILING_X1000,
            &MOTION_KEEPER_TICKS,
            &MOTION_KEEPER_DISTANCE_X1000,
            &CHASE_CARRIER_CAP_X1000,
            &CHASE_CHASER_CAP_X1000,
            &CHASE_CARRIER_SPD_X1000,
            &CHASE_CHASER_SPD_X1000,
            &CHASE_OUTPACED,
            &STUCK_TICKS_UNOWNED,
            &STUCK_EPISODES_UNOWNED,
            &LONGEST_STUCK,
            &TAKEBALL_OWNER_DIST_X10,
            &TAKEBALL_SAMPLES,
            &OWNERSHIP_GAINED,
            &OWNERSHIP_LOST,
            &OWNERSHIP_RECLAIMED_SELF,
            &TAKEBALL_OWN_TICKS,
            &TAKEBALL_OWN_SPELLS,
        ] {
            c.store(0, Ordering::Relaxed);
        }
    }

    /// `(owner dwell histogram, TakeBall dwell histogram, mean owner→ball
    /// distance in units while a TakeBall state held a stuck ball)`.
    pub fn dwell_snapshot() -> ([u64; 5], [u64; 5], f32) {
        let mut all = [0u64; 5];
        let mut tb = [0u64; 5];
        for i in 0..5 {
            all[i] = OWNER_DWELL[i].load(Ordering::Relaxed);
            tb[i] = TAKEBALL_DWELL[i].load(Ordering::Relaxed);
        }
        let n = TAKEBALL_SAMPLES.load(Ordering::Relaxed).max(1);
        let mean = TAKEBALL_OWNER_DIST_X10.load(Ordering::Relaxed) as f32 / n as f32 / 10.0;
        (all, tb, mean)
    }

    /// `(rows of (compact_id, ticks, episodes), unowned ticks, unowned
    /// episodes, longest stall in ticks)`. Rows are only the states that
    /// actually stalled, so the caller can print a short table.
    pub fn snapshot() -> (Vec<(u16, u64, u64)>, u64, u64, u64) {
        let mut rows = Vec::new();
        for id in 0..STATES {
            let t = STUCK_TICKS_BY_STATE[id].load(Ordering::Relaxed);
            if t > 0 {
                rows.push((
                    id as u16,
                    t,
                    STUCK_EPISODES_BY_STATE[id].load(Ordering::Relaxed),
                ));
            }
        }
        rows.sort_by(|a, b| b.1.cmp(&a.1));
        (
            rows,
            STUCK_TICKS_UNOWNED.load(Ordering::Relaxed),
            STUCK_EPISODES_UNOWNED.load(Ordering::Relaxed),
            LONGEST_STUCK.load(Ordering::Relaxed),
        )
    }
}

impl Ball {
    /// Position-based stall: the ball hasn't left a small region in N
    /// ticks, regardless of who owns it. Catches the case where
    /// ownership rapidly flips between teammates (each flip resets
    /// owned/unowned counters) but the ball physically stays put.
    /// The anchor resets whenever the ball travels outside the radius,
    /// so normal play keeps advancing the anchor every few ticks.
    pub(in crate::r#match::engine::ball::ball) fn detect_position_stall(
        &mut self,
        players: &[MatchPlayer],
    ) {
        // Raised thresholds so normal possession play doesn't trigger.
        // A team can legitimately keep the ball in a 15-unit zone for
        // 8-10 seconds during sideline passing or defensive possession;
        // 1000 ticks = 10 sec is the floor for "genuinely stuck".
        const STALL_RADIUS: f32 = 15.0;
        const STALL_RADIUS_SQ: f32 = STALL_RADIUS * STALL_RADIUS;
        const STALL_TICKS: u32 = 1000;

        let ball_xy = Vector3::new(self.position.x, self.position.y, 0.0);
        let anchor_xy = Vector3::new(self.stall_anchor_pos.x, self.stall_anchor_pos.y, 0.0);
        let drift_sq = (ball_xy - anchor_xy).norm_squared();

        if drift_sq > STALL_RADIUS_SQ {
            self.stall_anchor_pos = self.position;
            self.stall_anchor_tick = 0;
            return;
        }

        self.stall_anchor_tick += 1;

        // Attribute the stall long before the safety net fires. Inside a
        // 15u (1.9 m) circle for five seconds is not build-up play — the
        // ball is stuck, and whoever is standing on it is the bug.
        #[cfg(feature = "match-logs")]
        {
            use std::sync::atomic::Ordering;
            const DEAD_AFTER: u32 = 250;
            if self.stall_anchor_tick >= DEAD_AFTER {
                let first = self.stall_anchor_tick == DEAD_AFTER;
                let owner = self
                    .current_owner
                    .and_then(|id| players.iter().find(|p| p.id == id));
                if let Some(p) = owner {
                    let d = dead_ball_diag::dwell_bucket(p.in_state_time);
                    dead_ball_diag::OWNER_DWELL[d].fetch_add(1, Ordering::Relaxed);
                    if p.state.is_take_ball() {
                        dead_ball_diag::TAKEBALL_DWELL[d].fetch_add(1, Ordering::Relaxed);
                        let dist = (p.position - self.position).magnitude();
                        dead_ball_diag::TAKEBALL_OWNER_DIST_X10
                            .fetch_add((dist * 10.0) as u64, Ordering::Relaxed);
                        dead_ball_diag::TAKEBALL_SAMPLES.fetch_add(1, Ordering::Relaxed);
                    }
                }
                // Where, and whether a delivery is in the air.
                let zone = {
                    let x = self.position.x;
                    let y = self.position.y;
                    let w = self.field_width;
                    let h = self.field_height;
                    const FLAG: f32 = 80.0; // 10 m of the corner
                    const TOUCH: f32 = 40.0; // 5 m of a touchline
                    let near_end = x < FLAG || x > w - FLAG;
                    let near_side = y < FLAG || y > h - FLAG;
                    if near_end && near_side {
                        0
                    } else if (x < 132.0 || x > w - 132.0) && (y - h * 0.5).abs() < h * 0.35 {
                        1
                    } else if y < TOUCH || y > h - TOUCH {
                        2
                    } else {
                        3
                    }
                };
                dead_ball_diag::STALL_ZONE[zone].fetch_add(1, Ordering::Relaxed);
                if self.flags.in_flight_state > 0 {
                    dead_ball_diag::STALL_IN_FLIGHT.fetch_add(1, Ordering::Relaxed);
                }

                let bucket = owner
                    .map(|p| p.state.compact_id() as usize)
                    .filter(|b| *b < dead_ball_diag::STATES);
                match bucket {
                    Some(b) => {
                        dead_ball_diag::STUCK_TICKS_BY_STATE[b].fetch_add(1, Ordering::Relaxed);
                        if let Some(p) = owner {
                            let d = dead_ball_diag::dwell_bucket(p.in_state_time);
                            dead_ball_diag::STUCK_DWELL_BY_STATE[b * 5 + d]
                                .fetch_add(1, Ordering::Relaxed);
                        }
                        if first {
                            dead_ball_diag::STUCK_EPISODES_BY_STATE[b]
                                .fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    None => {
                        dead_ball_diag::STUCK_TICKS_UNOWNED.fetch_add(1, Ordering::Relaxed);
                        if first {
                            dead_ball_diag::STUCK_EPISODES_UNOWNED.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
                dead_ball_diag::LONGEST_STUCK
                    .fetch_max(self.stall_anchor_tick as u64, Ordering::Relaxed);
            }
        }

        if self.stall_anchor_tick == STALL_TICKS {
            #[cfg(feature = "match-logs")]
            {
                let owner_str = self
                    .current_owner
                    .map(|id| format!("Some({})", id))
                    .unwrap_or_else(|| "None".to_string());
                let owner_state = self
                    .current_owner
                    .and_then(|id| players.iter().find(|p| p.id == id))
                    .map(|p| format!("{:?}", p.state))
                    .unwrap_or_else(|| "-".to_string());
                crate::match_log_debug!(
                    "ball position-stall: stayed within {}u of ({:.1}, {:.1}) for {} ticks — owner={} state={} ball_vel=({:.2}, {:.2})",
                    STALL_RADIUS,
                    self.stall_anchor_pos.x,
                    self.stall_anchor_pos.y,
                    STALL_TICKS,
                    owner_str,
                    owner_state,
                    self.velocity.x,
                    self.velocity.y,
                );
            }
            // Force-kick out of the zone. Previous attempts with a
            // small push got immediately re-claimed by the same player
            // in `process_ownership` the SAME tick — ball never
            // escaped the 12-unit radius. Solution: kick harder AND
            // set `in_flight_state` so normal ownership checks are
            // suppressed long enough for the ball to actually leave.
            let owner_side = self
                .current_owner
                .and_then(|id| players.iter().find(|p| p.id == id))
                .and_then(|p| p.side);
            // Both components were pre-metric. 7.0 u/tick is 87 m/s —
            // faster than any ball in the history of football — and a
            // `z` of 1.5 is 150 m/s straight up, an apex of 1.1 km. A
            // stall-breaker firing on a ball nobody had touched is
            // exactly the "the ball suddenly shot off for no reason"
            // case: there is no player action behind it to explain it.
            //
            // It still has to be a decisive kick — the whole point is to
            // clear the stall radius before the same player re-claims —
            // so it is a firm 30 m outlet, solved from its apex like
            // every other kick.
            let push_x: f32 = match owner_side {
                Some(PlayerSide::Left) => 1.0,
                Some(PlayerSide::Right) => -1.0,
                _ => 1.0,
            };
            const STALL_KICK_APEX_M: f32 = 8.0;
            const STALL_KICK_RANGE_U: f32 = 240.0; // 30 m
            let vz = Ball::launch_speed_for_apex(STALL_KICK_APEX_M);
            let speed = STALL_KICK_RANGE_U / Ball::hang_ticks(vz).max(1.0);
            self.velocity = Vector3::new(push_x * speed, 0.0, vz);
            self.previous_owner = self.current_owner;
            self.current_owner = None;
            self.ownership_duration = 0;
            self.claim_cooldown = 0;
            // 40 ticks of protected flight — matches a short pass,
            // long enough for the ball to clear the stall radius.
            self.flags.in_flight_state = 40;
            self.pass_target_player_id = None;
            self.owned_stuck_ticks = 0;
            self.owned_stuck_logged = false;
            self.stall_anchor_tick = 0;
            // Teleport anchor so post-release ball travel advances
            // the anchor naturally instead of re-triggering.
            self.stall_anchor_pos = self.position;
        }
    }

    /// Only the stall-resolution log (match-logs builds) consumes the
    /// snapshot — production builds never call this (the capture site in
    /// `force_takeball_if_unowned_too_long` is feature-gated). `write!`
    /// straight into the output buffer: the previous `push_str(&format!)`
    /// pattern built ~23 intermediate Strings per capture, and captures
    /// fire on every owned→unowned transition.
    #[cfg(feature = "match-logs")]
    pub(in crate::r#match::engine::ball::ball) fn format_stall_snapshot(
        &self,
        players: &[MatchPlayer],
    ) -> String {
        use std::fmt::Write;

        let mut out = String::with_capacity(2048);
        let _ = write!(
            out,
            "  ball pos=({:.1}, {:.1}, {:.1}) velocity=({:.2}, {:.2}, {:.2}) in_flight={} previous_owner={:?}",
            self.position.x,
            self.position.y,
            self.position.z,
            self.velocity.x,
            self.velocity.y,
            self.velocity.z,
            self.flags.in_flight_state,
            self.previous_owner,
        );
        for p in players {
            if p.is_sent_off {
                continue;
            }
            let _ = write!(
                out,
                "\n  id={} team={} pos=({:.1}, {:.1}) vel=({:.2}, {:.2}) state={} tactical={:?}",
                p.id,
                p.team_id,
                p.position.x,
                p.position.y,
                p.velocity.x,
                p.velocity.y,
                p.state,
                p.tactical_position.current_position,
            );
        }
        out
    }
}
