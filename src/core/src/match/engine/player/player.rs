use crate::club::player::events::PositionLoad;
use crate::club::player::traits::PlayerTrait;
use crate::r#match::PlayerMatchEndStats;
use crate::r#match::defenders::states::DefenderState;
use crate::r#match::defenders::states::common::DefenderCondition;
use crate::r#match::engine::ball::ball::{Ball, GRAVITY_PER_TICK};
use crate::r#match::engine::engine::MATCH_HALF_TIME_MS;
use crate::r#match::engine::result::PlayerMatchPhysicalSnapshot;
use crate::r#match::engine::tactics::TacticalPositions;
use crate::r#match::events::EventCollection;
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::forwarders::states::common::ForwardCondition;
use crate::r#match::goalkeepers::states::common::GoalkeeperCondition;
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::midfielders::states::common::MidfielderCondition;
use crate::r#match::player::memory::PlayerMemory;
use crate::r#match::player::state::{PlayerMatchState, PlayerState};
use crate::r#match::player::statistics::MatchPlayerStatistics;
use crate::r#match::player::transition::TransitionSource;
use crate::r#match::player::waypoints::WaypointManager;
use crate::r#match::{
    ActivityIntensity, ConditionContext, GameTickContext, MatchContext, StateProcessingContext,
};
use crate::utils::DateUtils;
use crate::{
    PersonAttributes, Player, PlayerAttributes, PlayerFieldPositionGroup, PlayerPositionType,
    PlayerSkills,
};
use chrono::NaiveDate;
use nalgebra::Vector3;
use std::fmt::*;
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(debug_assertions)]
use log::debug;

#[derive(Debug, Clone)]
pub struct MatchPlayer {
    pub id: u32,
    pub position: Vector3<f32>,
    pub start_position: Vector3<f32>,
    pub attributes: PersonAttributes,
    pub team_id: u32,
    pub player_attributes: PlayerAttributes,
    pub skills: PlayerSkills,
    pub tactical_position: TacticalPositions,
    pub velocity: Vector3<f32>,
    /// Upward speed in **metres per tick**, and the reason it is not simply
    /// `velocity.z`.
    ///
    /// The two axes do not share a unit: `velocity.x/y` are 0.125 m grid
    /// units per 10 ms tick, while the vertical axis is metric — the same
    /// split the ball carries (see [`GRAVITY_PER_TICK`]). Folding a metric
    /// rise into the horizontal vector puts it inside the acceleration
    /// limiter and the `max_speed` clamp in `PlayerMatchState::process`,
    /// both of which take a 3-D norm, so a leap is scaled by a limit
    /// expressed in the wrong unit and then thrown away entirely by
    /// [`MatchPlayer::move_to`], which only ever integrated x and y.
    ///
    /// That is exactly what happened to the goalkeeper jump for as long as
    /// it existed: `GoalkeeperJumpingState` computed a textbook `sin` arc,
    /// spent part of the keeper's horizontal speed budget on it, and never
    /// moved him a millimetre. Kept apart here so it cannot happen again.
    pub vertical_speed: f32,
    /// Metres above the turf. Deliberately NOT `position.z`, which stays 0
    /// for every player exactly as it always has.
    ///
    /// `position` is a two-dimensional pitch coordinate with a vestigial
    /// third component, and the engine is full of helpers that treat it as a
    /// true 3-vector — `VectorExtensions::distance_to`, every
    /// `(ball - player).magnitude()` — while the ball's z is metres and the
    /// player's x/y are 0.125 m grid units. Putting a real height into
    /// `position.z` therefore silently adds the keeper's own leap to his
    /// distance from the ball, in the wrong unit, inside every reach and
    /// catch gate in the goalkeeper state machine. Measured: it moved
    /// save% by double digits.
    ///
    /// So the leap lives here instead, where nothing can read it by
    /// accident, and the recorder copies it into the sample's z on the way
    /// out (`MatchPositionData::write_match_positions`) so the replay still
    /// shows a keeper leaving the ground. Making those distance helpers
    /// genuinely 2-D-plus-height is the real fix, and a much larger one.
    pub height: f32,
    pub side: Option<PlayerSide>,
    pub state: PlayerState,
    /// Ticks spent in the current `state`, counted in **AI ticks** (full
    /// `game_tick_inner` passes), not raw simulation ticks. The engine
    /// alternates full AI ticks with movement-only "light" ticks; only
    /// full ticks run the state machine, so this counter advances once
    /// per AI decision. Every per-state timeout (`in_state_time > N`) and
    /// the fatigue curve are calibrated in these units. Reset to 0 by
    /// [`MatchPlayer::transition_to`] (a true state-machine transition);
    /// [`MatchPlayer::redirect_to`] (the out-of-band loose-ball / reset
    /// overrides) deliberately PRESERVES it. See `game_tick_light` for why
    /// light ticks leave it alone.
    pub in_state_time: u64,
    /// The state occupied immediately BEFORE the current one, ignoring
    /// self-transitions. Read by the decision-commitment guard in
    /// [`PlayerMatchState::process`] to recognise a transition that
    /// merely undoes the last one; see `DecisionCommitment`.
    pub previous_state: Option<PlayerState>,
    pub statistics: MatchPlayerStatistics,
    pub use_extended_state_logging: bool,

    pub waypoint_manager: WaypointManager,

    pub memory: PlayerMemory,

    /// Accumulates fractional condition changes across ticks
    pub fatigue_accumulator: f32,

    /// Exertion level the AI assigned to this player on the last full
    /// tick — the same `ActivityIntensity` the fatigue model reads.
    /// Drives movement speed via `MovementEffort` so off-ball play
    /// jogs/cruises instead of pinning to a sprint. Written in
    /// `ConditionProcessor::process`; defaults to `Moderate` pre-kickoff.
    pub last_activity_intensity: ActivityIntensity,

    /// Cached waypoint vectors (only changes on substitution/half-time swap)
    cached_waypoints: Vec<Vector3<f32>>,

    /// Signature moves (PPMs) — read by decision helpers to bias behaviour.
    pub traits: Vec<PlayerTrait>,

    /// Yellow cards accumulated in this match. 2 → red.
    pub yellow_cards: u8,
    /// Fouls committed in this match. Feeds end-of-match stats.
    pub fouls_committed: u8,
    /// Player has been sent off — skip state processing, treat as off field.
    pub is_sent_off: bool,
    /// Ticks remaining before this player may attempt another tackle.
    /// Decremented each tick in `update()`. Blocks Tackling-state entry
    /// via `can_attempt_tackle()`. Prevents the Tackling-state machine
    /// from re-firing attempts via Standing/Running/Covering/etc. paths
    /// within the same second, which would otherwise produce 40+ fouls
    /// per team in the first 5 minutes of every match.
    pub tackle_cooldown: u16,
    /// Tagged reason for the next Shoot event. Set by each transition
    /// point that routes into the Shooting state (e.g. "FWD_RUN_PRIO05",
    /// "FWD_POINT_BLANK", "MID_POINT_BLANK_RUN"). The Shooting state
    /// reads this and attaches it to the emitted Shoot event so the
    /// per-match shot-reason log shows which code path fired the shot.
    /// Cleared after consumption.
    pub pending_shot_reason: Option<&'static str>,

    /// Manager flag protecting this player from fatigue / development subs.
    /// Mirrored from `Player::is_force_match_selection` at squad-build time.
    pub is_force_match_selection: bool,

    /// Player's birth date, mirrored from the source `Player`. Read by the
    /// in-match substitution logic to apply age-appropriate protection
    /// thresholds for under-18 players (lower fatigue ceiling, condition
    /// floor that overrides the manager's force-selection flag).
    pub birth_date: NaiveDate,

    /// Match-time (in ms) at which the player entered the field.
    /// Starters are stamped with 0; substitutes are stamped with
    /// `context.total_match_time` at swap time. The end-of-match rating
    /// helper computes minutes-played as
    /// `(exit_or_now - entry_match_time_ms) / 60_000`, which fixes the
    /// previous behaviour where every player — even an 80th-minute sub —
    /// was credited with full match minutes.
    pub entry_match_time_ms: u64,
    /// True when the player entered via a forced (medical / emergency)
    /// substitution rather than a planned one. A planned sub was
    /// warming the touchline; a forced sub walks on cold and pays a
    /// larger post-entry settling penalty in `effective_skill`.
    pub entered_cold: bool,
    /// Last tick at which a pressure event was credited for this
    /// player. Used as a per-player cooldown so a defender shadowing
    /// the carrier across many ticks racks up one pressure per "press
    /// burst" rather than one per tick.
    pub last_pressure_tick: u64,

    /// Condition the player carried onto the pitch — kickoff for
    /// starters, swap-time for substitutes. Stamped once at
    /// construction (or at the substitution swap) and never mutated
    /// after that. Read by the engine end-of-match path to build the
    /// `PlayerMatchPhysicalSnapshot` that drives the post-match
    /// condition-drop formula on the persisted `Player`.
    pub starting_condition: i16,

    /// Recovery debt the player walked onto the pitch with — copied
    /// from `Player::load::recovery_debt` at construction (or at the
    /// substitution swap) and never mutated during the match. Read by
    /// the in-match condition processor so a player with heavy legs
    /// tires faster than a fresh player with the same NF/stamina, and
    /// so a back-to-back schedule shows up in the tick-by-tick drain
    /// not just the post-match summary. Lives on MatchPlayer (rather
    /// than reaching back through `PlayerLoad`) so the hot per-tick
    /// path doesn't touch the persisted simulator data.
    pub starting_recovery_debt: f32,

    /// Home-crowd arousal multiplier applied inside `effective_skill`
    /// (1.0 = neutral). Stamped once at match start from the match
    /// environment: home players slightly above 1.0, away players
    /// slightly below, both scaled by `crowd_intensity ×
    /// home_advantage`. This is the play-quality half of home
    /// advantage (the documented crowd-arousal / travel-discomfort
    /// effect, worth ~+0.35 home goals at equal strength in real
    /// football); the officiating half lives in
    /// `RefereeProfile::home_bias`. Flows through every skill-mediated
    /// action via `effective_skill`, so it shifts duels, passing,
    /// saves and finishing continuously instead of dialling any single
    /// outcome.
    pub crowd_arousal: f32,

    /// Pre-match performance settledness multiplier (1.0 = fully
    /// settled and match-sharp), stamped once at construction from
    /// `Player::match_performance_settledness`: transfer settling
    /// (linear recovery over the 84-day window, depth scaled by
    /// adaptability) × rustiness (`match_readiness` below the
    /// match-fit band). Consumed inside `effective_skill` alongside
    /// `crowd_arousal`, so an unsettled or rusty player genuinely
    /// plays worse across every skill-mediated action and the
    /// outcome-based rating dips on its own — no rating-side bias.
    /// Floor 0.90.
    pub settledness: f32,

    /// Pre-match form multiplier (1.0 = an ordinary day), stamped once
    /// at construction from `Player::matchday_form`. Mean 1.0 for every
    /// player; only its SPREAD varies, widened by youth and by low
    /// consistency / concentration / professionalism / composure.
    ///
    /// Consumed inside `effective_skill` alongside `settledness`, so a
    /// player having a bad day genuinely misplaces passes and loses
    /// duels rather than merely printing a lower number — and the
    /// outcome-based rating follows the stat line it always follows.
    /// See `club::player::personality::form` for why this belongs here
    /// and not at the rating layer.
    pub matchday_form: f32,

    /// Memo for `skills.max_speed_with_condition(condition)` keyed on
    /// the condition value it was computed for. Skills are static
    /// in-match and condition only moves when the fatigue accumulator
    /// crosses a whole point, yet the fatigue processor, the velocity
    /// speed cap and every steering behaviour re-derive this value —
    /// 3-4 times per player per AI tick. Same pure function, same
    /// inputs ⇒ the memoized value is bit-identical (the accessor
    /// carries a debug oracle). Packed into one relaxed atomic (not a
    /// `Cell`) so `MatchPlayer` stays `Sync` — league/world harnesses
    /// share rosters across match threads.
    max_speed_memo: MaxSpeedMemo,

    /// Per-tick motion / state-churn accumulator for the runtime flicker
    /// tracer. Dev-only: the field, and every site that touches it,
    /// compile out without `match-logs`.
    #[cfg(feature = "match-logs")]
    pub(crate) motion_trace: crate::r#match::player::motion_diag::MotionTrace,

    /// Memo for the fatigue processor's velocity curve, keyed on
    /// `(intensity_ratio_sq bits, sprint_peak bits)` — see
    /// `ConditionProcessor::velocity_fatigue_curve`. The curve costs a
    /// `sqrt` + `powf` per player per processed tick, yet its input is
    /// bit-stable across ticks whenever the player cruises at a steady
    /// velocity (every LOD-skipped tick, every settled `Arrive`). Plain
    /// tuple (not a `Cell`): the condition processor holds `&mut`, and a
    /// plain field keeps `MatchPlayer` `Sync`. The sentinel peak-bits `0`
    /// never matches a real key (sprint peaks are 7/9/10).
    pub(crate) velocity_fatigue_memo: (u32, u32, f32),
}

/// `(condition, max_speed)` packed into one `AtomicU64` (key in the high
/// half, `f32` bits in the low). Relaxed ordering is enough: the value is
/// a pure function of its key, so a racing writer can only store the
/// same bits.
#[derive(Debug)]
struct MaxSpeedMemo(AtomicU64);

impl MaxSpeedMemo {
    /// Real packed values keep the key in bits 32..48, so the top word
    /// never reaches `u64::MAX`.
    const EMPTY: u64 = u64::MAX;

    fn new() -> Self {
        MaxSpeedMemo(AtomicU64::new(Self::EMPTY))
    }

    #[inline]
    fn get(&self) -> Option<(i16, f32)> {
        let packed = self.0.load(Ordering::Relaxed);
        if packed == Self::EMPTY {
            return None;
        }
        Some(((packed >> 32) as u16 as i16, f32::from_bits(packed as u32)))
    }

    #[inline]
    fn set(&self, key: i16, value: f32) {
        let packed = ((key as u16 as u64) << 32) | value.to_bits() as u64;
        self.0.store(packed, Ordering::Relaxed);
    }
}

impl Clone for MaxSpeedMemo {
    fn clone(&self) -> Self {
        MaxSpeedMemo(AtomicU64::new(self.0.load(Ordering::Relaxed)))
    }
}

impl MatchPlayer {
    /// Age in whole years on `today`. Defined here (not on `Player`) so
    /// match-side code never has to reach back through the simulator
    /// data graph.
    #[inline]
    pub fn age_at(&self, today: NaiveDate) -> u8 {
        DateUtils::age(self.birth_date, today)
    }

    /// `skills.max_speed_with_condition(condition)` through the
    /// per-condition memo — see [`MatchPlayer::max_speed_memo`].
    #[inline]
    pub fn max_speed_with_condition_cached(&self) -> f32 {
        let condition = self.player_attributes.condition;
        if let Some((key, cached)) = self.max_speed_memo.get() {
            if key == condition {
                debug_assert_eq!(
                    cached.to_bits(),
                    self.skills.max_speed_with_condition(condition).to_bits(),
                    "max-speed memo mismatch"
                );
                return cached;
            }
        }
        let value = self.skills.max_speed_with_condition(condition);
        self.max_speed_memo.set(condition, value);
        value
    }
}

impl MatchPlayer {
    /// Fast trait lookup used inside hot decision paths.
    #[inline]
    pub fn has_trait(&self, t: PlayerTrait) -> bool {
        self.traits.iter().any(|x| *x == t)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum PlayerSide {
    Left,
    Right,
}

impl PlayerSide {
    /// Forward direction along the pitch's x-axis: +1 toward larger x
    /// for Left teams, -1 for Right. The math everywhere else in the
    /// engine uses this — never write `match side { Left => 1.0, ... }`
    /// inline or you'll inevitably miss a sign and produce the
    /// "right-side never reaches attacking third" bug we hit before.
    #[inline]
    pub fn forward_dir_x(self) -> f32 {
        match self {
            PlayerSide::Left => 1.0,
            PlayerSide::Right => -1.0,
        }
    }

    /// Attacking progress for an x-coordinate, normalised to [0.0, 1.0]:
    ///   0.0 = own goal line, 1.0 = opponent goal line.
    /// Use this everywhere a "what fraction of the pitch have we
    /// covered" check is needed (attacking third, defensive third,
    /// halfway thresholds). Replaces the legacy
    ///   `(x * forward_dir) / field_width`
    /// formula, which produced NEGATIVE values for right-side teams —
    /// that broke `> 0.66` (attacking third) tests by construction
    /// while leaving `< 0.33` (defensive third) accidentally correct.
    #[inline]
    pub fn attacking_progress_x(self, x: f32, field_width: f32) -> f32 {
        if field_width <= 0.0 {
            return 0.0;
        }
        let raw = match self {
            PlayerSide::Left => x / field_width,
            PlayerSide::Right => (field_width - x) / field_width,
        };
        raw.clamp(0.0, 1.0)
    }

    /// Forward delta along x: positive = toward opponent goal, negative
    /// = toward own goal. The right shape for "is this pass forward?",
    /// "did this player advance?", and any signed forward-progress
    /// arithmetic. Side-direction is baked in.
    #[inline]
    pub fn forward_delta(self, from_x: f32, to_x: f32) -> f32 {
        (to_x - from_x) * self.forward_dir_x()
    }

    /// Same as `forward_delta` but normalised by field width — gives a
    /// signed [-1.0, 1.0]-ish ratio for the cases that previously used
    /// `((to.x - from.x) * dir) / field_width`.
    #[inline]
    pub fn forward_delta_norm(self, from_x: f32, to_x: f32, field_width: f32) -> f32 {
        if field_width <= 0.0 {
            return 0.0;
        }
        self.forward_delta(from_x, to_x) / field_width
    }
}

impl MatchPlayer {
    /// Build a match player from the persisted `Player`. `now` is the
    /// matchday date, used to derive the pre-match settledness stamp
    /// (transfer-settling needs a calendar); `None` — synthetic squads
    /// and unit fixtures — skips the transfer component but still reads
    /// match-readiness rustiness from the player's own state.
    pub fn from_player(
        team_id: u32,
        player: &Player,
        position: PlayerPositionType,
        use_extended_state_logging: bool,
        now: Option<NaiveDate>,
    ) -> Self {
        MatchPlayer {
            id: player.id,
            position: Vector3::zeros(),
            start_position: Vector3::zeros(),
            attributes: player.attributes,
            team_id,
            player_attributes: player.player_attributes,
            skills: player.skills,
            velocity: Vector3::zeros(),
            vertical_speed: 0.0,
            height: 0.0,
            tactical_position: TacticalPositions::new(position, None),
            side: None,
            state: Self::default_state(position),
            in_state_time: 0,
            previous_state: None,
            statistics: MatchPlayerStatistics::new(),
            waypoint_manager: WaypointManager::new(),
            use_extended_state_logging,
            memory: PlayerMemory::new(),
            fatigue_accumulator: 0.0,
            last_activity_intensity: ActivityIntensity::Moderate,
            cached_waypoints: Vec::new(),
            traits: player.traits.clone(),
            yellow_cards: 0,
            fouls_committed: 0,
            is_sent_off: false,
            tackle_cooldown: 0,
            pending_shot_reason: None,
            is_force_match_selection: player.is_force_match_selection,
            birth_date: player.birth_date,
            entry_match_time_ms: 0,
            entered_cold: false,
            last_pressure_tick: 0,
            starting_condition: player.player_attributes.condition,
            starting_recovery_debt: player.load.recovery_debt,
            crowd_arousal: 1.0,
            settledness: player.match_performance_settledness(now),
            matchday_form: player.matchday_form(now),
            max_speed_memo: MaxSpeedMemo::new(),
            velocity_fatigue_memo: (0, 0, 0.0),
            #[cfg(feature = "match-logs")]
            motion_trace: Default::default(),
        }
    }

    /// Input-style constructor used by the distributed worker wire
    /// layer to rebuild a `MatchPlayer` from the bincode payload. Takes
    /// only the fields that meaningfully cross the network — engine
    /// runtime state (memory, waypoints, in-state timers, statistics,
    /// fatigue accumulator) is initialised to the same defaults
    /// `from_player` uses at kickoff, so the rebuilt player is
    /// indistinguishable from one freshly selected on the worker.
    #[allow(clippy::too_many_arguments)]
    pub fn from_inputs(
        id: u32,
        team_id: u32,
        position: [f32; 3],
        start_position: [f32; 3],
        attributes: PersonAttributes,
        player_attributes: PlayerAttributes,
        skills: PlayerSkills,
        tactical_position: PlayerPositionType,
        side: Option<PlayerSide>,
        traits: Vec<PlayerTrait>,
        birth_date: NaiveDate,
        is_force_match_selection: bool,
        starting_condition: i16,
        starting_recovery_debt: f32,
        settledness: f32,
        matchday_form: f32,
        use_extended_state_logging: bool,
    ) -> Self {
        MatchPlayer {
            id,
            position: Vector3::new(position[0], position[1], position[2]),
            start_position: Vector3::new(start_position[0], start_position[1], start_position[2]),
            attributes,
            team_id,
            player_attributes,
            skills,
            velocity: Vector3::zeros(),
            vertical_speed: 0.0,
            height: 0.0,
            tactical_position: TacticalPositions::new(tactical_position, side),
            side,
            state: Self::default_state(tactical_position),
            in_state_time: 0,
            previous_state: None,
            statistics: MatchPlayerStatistics::new(),
            waypoint_manager: WaypointManager::new(),
            use_extended_state_logging,
            memory: PlayerMemory::new(),
            fatigue_accumulator: 0.0,
            last_activity_intensity: ActivityIntensity::Moderate,
            cached_waypoints: Vec::new(),
            traits,
            yellow_cards: 0,
            fouls_committed: 0,
            is_sent_off: false,
            tackle_cooldown: 0,
            pending_shot_reason: None,
            is_force_match_selection,
            birth_date,
            entry_match_time_ms: 0,
            entered_cold: false,
            last_pressure_tick: 0,
            starting_condition,
            starting_recovery_debt,
            // Wire payloads predate the arousal field; the worker
            // re-stamps it at match start like the local path does.
            crowd_arousal: 1.0,
            // The pre-match settledness stamp is derived club-side (the
            // worker has no transfer calendar), so it crosses the wire
            // as a plain value.
            settledness,
            // Same for the form draw: it needs the matchday calendar,
            // which only the club side has.
            matchday_form,
            max_speed_memo: MaxSpeedMemo::new(),
            velocity_fatigue_memo: (0, 0, 0.0),
            #[cfg(feature = "match-logs")]
            motion_trace: Default::default(),
        }
    }

    /// Build the per-match end-of-game stats snapshot for this player.
    /// Owns the field-by-field mapping so the engine end-of-match path
    /// and the substitution snapshot path don't drift. `minutes_played`
    /// is computed by the caller from `entry_match_time_ms` and the
    /// exit / now match time — see `engine.rs` and `substitutions.rs`.
    pub fn to_match_end_stats(&self, minutes_played: u16) -> PlayerMatchEndStats {
        PlayerMatchEndStats {
            shots_on_target: self.memory.shots_on_target as u16,
            shots_total: self.memory.shots_taken as u16,
            passes_attempted: self.statistics.passes_attempted,
            passes_completed: self.statistics.passes_completed,
            tackles: self.statistics.tackles,
            interceptions: self.statistics.interceptions,
            saves: self.statistics.saves,
            shots_faced: self.statistics.shots_faced,
            goals: self.statistics.goals_count(),
            assists: self.statistics.assists_count(),
            match_rating: 0.0,
            raw_match_rating: 0.0,
            xg: self.memory.xg_total,
            position_group: self.tactical_position.current_position.position_group(),
            fouls: self.fouls_committed as u16,
            yellow_cards: self.statistics.yellow_cards_count(),
            red_cards: self.statistics.red_cards_count(),
            minutes_played,
            key_passes: self.statistics.key_passes,
            progressive_passes: self.statistics.progressive_passes,
            progressive_carries: self.statistics.progressive_carries,
            successful_dribbles: self.statistics.successful_dribbles,
            attempted_dribbles: self.statistics.attempted_dribbles,
            successful_pressures: self.statistics.successful_pressures,
            pressures: self.statistics.pressures,
            blocks: self.statistics.blocks,
            clearances: self.statistics.clearances,
            passes_into_box: self.statistics.passes_into_box,
            crosses_attempted: self.statistics.crosses_attempted,
            crosses_completed: self.statistics.crosses_completed,
            xg_chain: self.statistics.xg_chain,
            xg_buildup: self.statistics.xg_buildup,
            miscontrols: self.statistics.miscontrols,
            heavy_touches: self.statistics.heavy_touches,
            carry_distance: self.statistics.carry_distance,
            errors_leading_to_shot: self.statistics.errors_leading_to_shot,
            errors_leading_to_goal: self.statistics.errors_leading_to_goal,
            xg_prevented: self.statistics.xg_prevented,
            xg_faced: self.statistics.xg_faced,
            offsides: self.statistics.offsides,
            own_goals: self.statistics.own_goals_count(),
            zone_stats: self.statistics.zone_stats,
        }
    }

    /// Compute minutes spent on the pitch for this player given the
    /// current absolute match time (ms). Capped at 120 minutes so an
    /// extra-time match doesn't produce silly minute totals.
    pub fn minutes_played_at(&self, now_match_time_ms: u64) -> u16 {
        let elapsed = now_match_time_ms.saturating_sub(self.entry_match_time_ms);
        ((elapsed / 60_000) as u16).min(120)
    }

    /// Build the post-match physical snapshot for this player at the
    /// given absolute match time (substitution-off or full-time).
    /// Captures the starting tank, the current (drained) condition,
    /// and a high-intensity share that blends the position-group
    /// default with the player's actual high-intensity involvement
    /// (pressures, tackles, dribbles, crosses) so the persisted
    /// `Player::on_match_exertion` reflects how the player actually
    /// played, not just the position they nominally occupied. A
    /// fullback who pressed all match should bill more than one who
    /// sat in a low block.
    pub fn to_physical_snapshot(&self, now_match_time_ms: u64) -> PlayerMatchPhysicalSnapshot {
        let elapsed_ms = now_match_time_ms.saturating_sub(self.entry_match_time_ms);
        // Fractional minutes — `minutes_played_at` rounds down to a
        // u16 which loses information for cameo subs. The post-match
        // formula reads `duration = minutes / 90` so the fractional
        // resolution actually matters here.
        let minutes_played = (elapsed_ms as f32 / 60_000.0).min(120.0);
        let group = self.tactical_position.current_position.position_group();
        let position_default = PositionLoad::high_intensity_share(group);
        PlayerMatchPhysicalSnapshot {
            player_id: self.id,
            minutes_played,
            starting_condition: self.starting_condition,
            final_match_energy: self.player_attributes.condition,
            high_intensity_load_hint: Self::derive_high_intensity_hint(
                position_default,
                &self.statistics,
                minutes_played,
            ),
        }
    }

    /// Blend the position-group default high-intensity share with the
    /// observed action density (pressures, tackles, successful
    /// dribbles, crosses) so the post-match condition drop reflects
    /// how the player actually played. Keepers and defenders sitting
    /// deep stay near their position default; an attacking fullback
    /// who pressed every action will read materially higher.
    ///
    /// A "calibration baseline" of 0.50 actions/min maps to the
    /// position default; anything above lifts the hint linearly, up
    /// to a cap of 1.0 (the engine's mathematical ceiling). This is
    /// deliberately conservative — the position default is right for
    /// average involvement; behaviour-driven correction is a tilt,
    /// not a rewrite.
    pub(crate) fn derive_high_intensity_hint(
        position_default: f32,
        stats: &crate::r#match::engine::player::statistics::MatchPlayerStatistics,
        minutes_played: f32,
    ) -> f32 {
        if minutes_played < 1.0 {
            return position_default;
        }
        let hi_actions = stats.pressures as f32
            + stats.tackles as f32
            + stats.successful_dribbles as f32
            + stats.crosses_attempted as f32
            + stats.progressive_carries as f32;
        let per_min = hi_actions / minutes_played.max(1.0);
        // 0.50 actions/min ≈ the position default; deviation tilts the
        // hint by ±0.4 absolute at the extremes (per-min ≈ 0 → -0.4×,
        // per-min ≈ 1.5 → +0.4×). Clamped tightly so a stat-stuffer
        // can't pin the hint to 1.0 and a quiet game can't pin it to 0.
        const CAL_BASELINE: f32 = 0.50;
        const PER_MIN_GAIN: f32 = 0.40;
        let tilt = ((per_min - CAL_BASELINE) * PER_MIN_GAIN).clamp(-0.15, 0.20);
        (position_default + tilt).clamp(0.02, 1.0)
    }

    /// Consumes the tackle cooldown (ticks it down by 1). Called once per
    /// simulation tick from `update()`.
    #[inline]
    pub fn tick_tackle_cooldown(&mut self) {
        self.tackle_cooldown = self.tackle_cooldown.saturating_sub(1);
    }

    /// Can this player currently attempt a sliding tackle? False while the
    /// post-attempt cooldown is still counting down — regardless of which
    /// state routed them into Tackling.
    #[inline]
    pub fn can_attempt_tackle(&self) -> bool {
        self.tackle_cooldown == 0
    }

    /// Start the post-tackle cooldown. Called right after any attempt
    /// resolves (success, miss, or foul).
    #[inline]
    pub fn start_tackle_cooldown(&mut self) {
        // 3000 ticks ≈ 30 seconds. Real football: a player contests 2-4
        // tackles per 90 minutes — one every ~25 minutes. The previous
        // 15-second cooldown still let attempts run at 205/team/match
        // (5x real) because 10 outfield players × 15s allowed up to one
        // attempt per second team-wide. 30s halves the team-wide ceiling
        // and matches the realistic "commit, resolve, regroup,
        // reposition" cadence — a defender who lunges and either wins,
        // misses, or fouls is realistically out of the next play for
        // half a minute, not 15 seconds.
        self.tackle_cooldown = 3000;
    }

    pub fn rebuild_waypoint_cache(&mut self) {
        self.cached_waypoints = self
            .tactical_position
            .tactical_positions
            .iter()
            .filter(|tp| tp.position == self.tactical_position.current_position)
            .flat_map(|tp| &tp.waypoints)
            .map(|(x, y)| Vector3::new(*x, *y, 0.0))
            .collect();
    }

    /// Full AI tick for this player. `field_index` is the player's slot
    /// in `field.players` — it lets the waypoint refresh read the
    /// position store by index instead of an id hash probe (the store's
    /// order is `field.players` then substitutes, so the slot maps 1:1;
    /// the accessor verifies the id and falls back to the probe).
    pub fn update(
        &mut self,
        field_index: usize,
        context: &MatchContext,
        tick_context: &GameTickContext,
        events: &mut EventCollection,
    ) {
        self.tick_tackle_cooldown();

        let player_events = PlayerMatchState::process(self, context, tick_context);

        events.add_from_collection(player_events);

        self.update_waypoint_index_at(field_index, tick_context);

        // Move first, THEN clamp. Clamping before the move meant the
        // boundary check could never actually stop anyone leaving the
        // pitch — it corrected the position a tick late, and its
        // velocity-zeroing test read a stale position, so a player
        // steering outward was never stopped. He ended each tick a
        // fraction off-field (dumps caught x = -0.2) and was dragged
        // back the next, which is a velocity reversal per tick for as
        // long as his state kept pointing outward.
        self.move_to();
        self.check_boundary_collision(context);
        #[cfg(feature = "match-logs")]
        self.trace_motion(
            context,
            tick_context.positions.ball.position,
            tick_context.positions.ball.velocity,
        );
    }

    /// Reduced-cadence (LOD) update for a far-from-ball player in a
    /// passive movement state (see `play_players`): the per-state
    /// `velocity()`/`process()` pair — the expensive AI — is skipped
    /// this full tick, while everything calibration-coupled still runs
    /// at full rate:
    ///
    ///   * fatigue, at the intensity the state declared on its last
    ///     processed tick (the fatigue model is 75% velocity-driven and
    ///     velocity is unchanged on a skipped tick, so the drain matches
    ///     what a processed tick would have produced);
    ///   * `in_state_time` (state timeouts stay real-time — a skipped
    ///     tick counts exactly like a processed no-transition tick);
    ///   * the tackle cooldown and the periodic memory decay;
    ///   * motion integration (previous velocity carries, boundary
    ///     clamp applies — identical to a light tick).
    ///
    /// The player re-decides on the next full tick, ~20 ms later.
    pub fn lod_skip_update(
        &mut self,
        context: &MatchContext,
        ball_pos: Vector3<f32>,
        ball_vel: Vector3<f32>,
    ) {
        self.tick_tackle_cooldown();

        let half_ms = MATCH_HALF_TIME_MS as f32;
        let full_ms = half_ms * 2.0;
        let match_progress = (context.total_match_time as f32 / full_ms).clamp(0.0, 1.0);
        let intensity = self.last_activity_intensity;
        let group = self.tactical_position.current_position.position_group();
        let condition_ctx = ConditionContext {
            in_state_time: self.in_state_time,
            player: self,
            match_progress,
        };
        match group {
            PlayerFieldPositionGroup::Defender => {
                DefenderCondition::new(intensity).process(condition_ctx)
            }
            PlayerFieldPositionGroup::Midfielder => {
                MidfielderCondition::new(intensity).process(condition_ctx)
            }
            PlayerFieldPositionGroup::Forward => {
                ForwardCondition::new(intensity).process(condition_ctx)
            }
            // Goalkeepers never take the LOD path (`play_players` only
            // gates outfield passive states), but keep the dispatch
            // total so a future gate change can't silently skip GK
            // fatigue.
            PlayerFieldPositionGroup::Goalkeeper => {
                GoalkeeperCondition::new(intensity).process(condition_ctx)
            }
        }

        self.in_state_time += 1;
        // Move first, THEN clamp. Clamping before the move meant the
        // boundary check could never actually stop anyone leaving the
        // pitch — it corrected the position a tick late, and its
        // velocity-zeroing test read a stale position, so a player
        // steering outward was never stopped. He ended each tick a
        // fraction off-field (dumps caught x = -0.2) and was dragged
        // back the next, which is a velocity reversal per tick for as
        // long as his state kept pointing outward.
        self.move_to();
        self.check_boundary_collision(context);
        #[cfg(feature = "match-logs")]
        self.trace_motion(context, ball_pos, ball_vel);
    }

    #[inline]
    pub fn check_boundary_collision(&mut self, context: &MatchContext) {
        let field_width = context.field_size.width as f32 + 1.0;
        let field_height = context.field_size.height as f32 + 1.0;

        // Clamp position to field boundaries
        self.position.x = self.position.x.clamp(0.0, field_width);
        self.position.y = self.position.y.clamp(0.0, field_height);

        // Only stop velocity if player is trying to move OUT of bounds
        // Allow velocity that moves them back into the field
        if self.position.x <= 0.0 && self.velocity.x < 0.0 {
            // At left boundary, trying to move further left
            self.velocity.x = 0.0;
        } else if self.position.x >= field_width && self.velocity.x > 0.0 {
            // At right boundary, trying to move further right
            self.velocity.x = 0.0;
        }

        if self.position.y <= 0.0 && self.velocity.y < 0.0 {
            // At bottom boundary, trying to move further down
            self.velocity.y = 0.0;
        } else if self.position.y >= field_height && self.velocity.y > 0.0 {
            // At top boundary, trying to move further up
            self.velocity.y = 0.0;
        }
    }

    /// A TRUE state-machine transition: set the new state AND reset
    /// `in_state_time` to 0 so the destination's timers count from entry.
    ///
    /// Use this for the normal handler hand-off (`change_state`) and the
    /// set-piece teleport — the sites that reset the timer in the
    /// pre-refactor engine. For the out-of-band overrides that left the
    /// timer running, use [`redirect_to`](Self::redirect_to) instead.
    ///
    /// `source` records WHY the transition happened for the
    /// transition-graph audit (see [`TransitionSource`]); in production
    /// (`match-logs` off) the recording compiles out.
    ///
    /// Pending cross-state handoffs are deliberately NOT cleared here:
    /// `pending_shot_reason` is written by the transition INTO `Shooting`
    /// and consumed there, so a blanket wipe would drop the shot-reason tag.
    #[inline]
    pub fn transition_to(&mut self, state: PlayerState, source: TransitionSource) {
        // Record BEFORE zeroing: `set_state_internal` reads the running
        // `in_state_time` as the dwell of the state being left, which is
        // the only signal that separates a normal transition from an
        // every-tick oscillation (see `motion_diag`).
        self.set_state_internal(state, source);
        self.in_state_time = 0;
    }

    /// Out-of-band state override that PRESERVES the running `in_state_time`.
    ///
    /// Use only where the destination state genuinely does not read the
    /// timer, or where a continuing timer is the intended semantics. If
    /// the destination has ANY `in_state_time > N` guard, use
    /// [`redirect_to_fresh`](Self::redirect_to_fresh) — a preserved timer
    /// there trips the destination's timeout on its first real tick.
    #[inline]
    pub fn redirect_to(&mut self, state: PlayerState, source: TransitionSource) {
        self.set_state_internal(state, source);
    }

    /// Out-of-band override that RESETS `in_state_time`, so the
    /// destination's timers count from the moment the player was yanked
    /// into it.
    ///
    /// Fixes a genuine incoherence in the redirect path: the dispatcher
    /// ran the override tick with `override_state_time = 0` while the
    /// player's persisted counter kept climbing, so the destination saw 0
    /// on the entry tick and a stale (often huge) value on the very next
    /// one. A goalkeeper redirected into `TakeBall` after 300 ticks of
    /// `Standing` therefore hit `in_state_time > 200` on its SECOND tick
    /// and abandoned the chase immediately; the same stale timer made
    /// every defender leave `Standing` for `HoldingLine`, and every
    /// midfielder for `Running`, on the first tick after a goal reset.
    ///
    /// Timer-preserving `redirect_to` remains available for destinations
    /// that don't read the timer, but every entry into a timeout-driven
    /// state now routes through here.
    #[inline]
    pub fn redirect_to_fresh(&mut self, state: PlayerState, source: TransitionSource) {
        self.set_state_internal(state, source);
        self.in_state_time = 0;
    }

    /// The one place `self.state` is written. Both `transition_to` and
    /// `redirect_to` route through here, so the transition-graph audit sees
    /// every state change and the "no raw `.state =`" invariant has a
    /// single sanctioned site.
    #[inline]
    fn set_state_internal(&mut self, state: PlayerState, source: TransitionSource) {
        let _from = self.state;
        self.state = state;

        #[cfg(feature = "match-logs")]
        {
            crate::r#match::TransitionGraph::record(_from, state, source);
            // Dwell = how long the player actually stayed. The graph
            // dedups edges, so only this number tells a real hand-off
            // apart from a per-tick oscillation.
            crate::r#match::player::motion_diag::record_transition(
                self.id,
                _from,
                state,
                self.in_state_time,
                source,
                self.previous_state,
            );
        }
        #[cfg(not(feature = "match-logs"))]
        let _ = source;

        // A self-transition is not a move through the state sequence, so
        // it must not shift the history the commitment guard reads.
        if _from.compact_id() != state.compact_id() {
            self.previous_state = Some(_from);
        }
    }

    pub fn set_default_state(&mut self, source: TransitionSource) {
        let default = Self::default_state(self.tactical_position.current_position);
        // A restart rebuilds the formation: kickoff, goal reset, half or
        // extra-time restart, a substitute walking on. The player is
        // ENTERING their default state, so its timers must count from now
        // — the per-role `Standing` states all have timeouts (defenders
        // leave for `HoldingLine` after 100 ticks, midfielders for
        // `Running` after 8), and a carried-over timer fired every one of
        // them on the first tick after the whistle.
        self.redirect_to_fresh(default, source);
        self.rebuild_waypoint_cache();
    }

    fn default_state(position: PlayerPositionType) -> PlayerState {
        match position.position_group() {
            PlayerFieldPositionGroup::Goalkeeper => {
                PlayerState::Goalkeeper(GoalkeeperState::Standing)
            }
            PlayerFieldPositionGroup::Defender => PlayerState::Defender(DefenderState::Standing),
            PlayerFieldPositionGroup::Midfielder => {
                PlayerState::Midfielder(MidfielderState::Standing)
            }
            PlayerFieldPositionGroup::Forward => PlayerState::Forward(ForwardState::Standing),
        }
    }

    /// Loose-ball signal from the ball layer (`BallEvent::TakeMe`):
    /// abandon whatever you're doing and go claim it.
    ///
    /// Ignored while the player is committed to an un-abortable action —
    /// the same guard `should_force_takeball` applies. Without it this
    /// path reproduced the exact bug the override guard fixes, one tick
    /// later and through a different door (the observed graph carried
    /// `Forward: Shooting -> Take Ball` tagged `event_handler`).
    pub fn run_for_ball(&mut self) {
        if self.state.is_committed_action() {
            return;
        }
        // Already chasing — leave the state alone. `redirect_to_fresh`
        // below zeroes `in_state_time`, so re-firing this on a player who
        // is ALREADY in TakeBall restarted their chase clock every tick
        // the signal repeated, and every give-up timeout inside TakeBall
        // (the goalkeeper's 120 / 200-tick abort, the outfield yields)
        // became unreachable: the player chased a ball they were never
        // going to reach until something else moved them.
        // `should_force_takeball` in `strategies::processor` has carried
        // this same guard from the start — the event path never got it.
        if matches!(
            self.state,
            PlayerState::Goalkeeper(GoalkeeperState::TakeBall)
                | PlayerState::Defender(DefenderState::TakeBall)
                | PlayerState::Midfielder(MidfielderState::TakeBall)
                | PlayerState::Forward(ForwardState::TakeBall)
        ) {
            return;
        }
        let target = match self.tactical_position.current_position.position_group() {
            PlayerFieldPositionGroup::Goalkeeper => {
                PlayerState::Goalkeeper(GoalkeeperState::TakeBall)
            }
            PlayerFieldPositionGroup::Defender => PlayerState::Defender(DefenderState::TakeBall),
            PlayerFieldPositionGroup::Midfielder => {
                PlayerState::Midfielder(MidfielderState::TakeBall)
            }
            PlayerFieldPositionGroup::Forward => PlayerState::Forward(ForwardState::TakeBall),
        };
        // Entering TakeBall — the chase starts now, so its give-up timers
        // must start now too (the goalkeeper variant bails at 120 / 200
        // ticks and a carried-over timer tripped both on entry).
        self.redirect_to_fresh(target, TransitionSource::EventHandler);
    }

    /// Take off, aiming to peak `apex_metres` above the turf.
    ///
    /// Asked for in apex rather than in launch speed for the same reason the
    /// ball's kicks are (see [`Ball::launch_speed_for_apex`]): apex is the
    /// property a human being can be measured against — a good standing jump
    /// lifts a keeper's hips about three quarters of a metre — where a launch
    /// speed in metres per tick means nothing to anybody.
    ///
    /// Ignored in mid-air. Nobody jumps twice off the same ground, and a
    /// state re-entered while the player is still up would otherwise hold him
    /// there indefinitely.
    #[inline]
    pub fn leap(&mut self, apex_metres: f32) {
        if apex_metres > 0.0 && self.height <= 0.0 {
            self.vertical_speed = Ball::launch_speed_for_apex(apex_metres);
        }
    }

    /// True while both feet are off the ground.
    #[inline]
    pub fn is_airborne(&self) -> bool {
        self.height > 0.0
    }

    /// Where this player should be RECORDED: his pitch coordinate carrying
    /// his real height in the vertical slot the replay format expects.
    ///
    /// The two live apart inside the engine — see [`MatchPlayer::height`] —
    /// and are only brought back together here, on the way out to a file
    /// nothing makes decisions from.
    #[inline]
    pub fn recorded_position(&self) -> Vector3<f32> {
        Vector3::new(self.position.x, self.position.y, self.height)
    }

    /// One tick of free flight: the height and the rise a body at `height`
    /// climbing at `rise` has after `GRAVITY_PER_TICK` has had its turn. Both
    /// in metres and metres per tick — the vertical axis is metric where the
    /// horizontal one is grid units.
    ///
    /// A free function of the pair rather than a method so the trajectory can
    /// be checked against a stopwatch without standing up a whole match. The
    /// axis it integrates spent the engine's life being silently discarded by
    /// [`MatchPlayer::move_to`] (see [`MatchPlayer::vertical_speed`]), and
    /// nothing downstream complains when a height quietly disappears — so it
    /// gets a test rather than a comment.
    ///
    /// The turf is a floor, not a surface to fall through: a body already on
    /// it stays on it at rest instead of accumulating fall speed for the
    /// whole ninety minutes.
    #[inline]
    pub fn fall(height: f32, rise: f32) -> (f32, f32) {
        if !rise.is_finite() {
            return (height.max(0.0), 0.0);
        }
        let rise = rise - GRAVITY_PER_TICK;
        let height = height + rise;
        if !height.is_finite() || height <= 0.0 {
            (0.0, 0.0)
        } else {
            (height, rise)
        }
    }

    #[inline]
    pub fn move_to(&mut self) {
        #[cfg(debug_assertions)]
        let old_position = self.position;

        // Apply velocity only if finite. `is_finite` rules out both NaN
        // and ±Infinity — either poisons the position, and a corrupt
        // position is excluded from the viewer recording so the player
        // literally disappears mid-match.
        if self.velocity.x.is_finite() {
            self.position.x += self.velocity.x;
        }

        if self.velocity.y.is_finite() {
            self.position.y += self.velocity.y;
        }

        // The vertical axis, integrated on its own terms. Only a leap ever
        // makes `vertical_speed` non-zero, so for twenty-one players out of
        // twenty-two on almost every tick this is one comparison and out.
        if self.vertical_speed != 0.0 || self.height != 0.0 {
            let (height, rise) = Self::fall(self.height, self.vertical_speed);
            self.height = height;
            self.vertical_speed = rise;
        }

        // Last-resort salvage: if position is already corrupt from an
        // earlier tick (before the velocity guard was in place, or from
        // external code paths), reset to the player's tactical start
        // position. The player briefly teleports rather than vanishing.
        if !self.position.x.is_finite() || !self.position.y.is_finite() {
            self.position = self.start_position;
            self.velocity = Vector3::zeros();
        }

        #[cfg(debug_assertions)]
        {
            // Check for abnormally large position changes
            let position_delta = self.position - old_position;
            let position_change = position_delta.norm();

            const MAX_REASONABLE_POSITION_CHANGE: f32 = 20.0;

            if position_change > MAX_REASONABLE_POSITION_CHANGE {
                debug!(
                    "Player {:?} position jumped abnormally! {} from: ({:.2}, {:.2}) to: ({:.2}, {:.2}), delta: ({:.2}, {:.2}), distance: {:.2}, velocity: ({:.2}, {:.2})",
                    self.state,
                    self.id,
                    old_position.x,
                    old_position.y,
                    self.position.x,
                    self.position.y,
                    position_delta.x,
                    position_delta.y,
                    position_change,
                    self.velocity.x,
                    self.velocity.y
                );
            }
        }
    }

    /// Sample this tick's motion for the runtime flicker tracer. Called
    /// right after `move_to` on every tick that advances the player —
    /// full AI ticks, LOD-skipped ticks and light ticks alike — so the
    /// measured path is the path the viewer actually renders.
    ///
    /// Accumulation is entirely player-local; only the closing of a
    /// one-second window touches the shared store.
    #[cfg(feature = "match-logs")]
    pub fn trace_motion(
        &mut self,
        context: &MatchContext,
        ball_pos: Vector3<f32>,
        ball_vel: Vector3<f32>,
    ) {
        use crate::r#match::player::motion_diag;

        let pos = self.position;
        let vel = self.velocity;
        let state = self.state;
        let group = match self.tactical_position.current_position.position_group() {
            PlayerFieldPositionGroup::Goalkeeper => 0u8,
            PlayerFieldPositionGroup::Defender => 1,
            PlayerFieldPositionGroup::Midfielder => 2,
            PlayerFieldPositionGroup::Forward => 3,
        };
        let id = self.id;
        let t_ms = context.total_match_time;
        let t = &mut self.motion_trace;

        if !t.seeded {
            t.seeded = true;
            t.win_start = pos;
            t.prev_pos = pos;
            t.prev_velocity = vel;
            return;
        }

        let step = (pos - t.prev_pos).norm();
        t.prev_pos = pos;
        t.win_path += step;
        t.win_ticks += 1;
        if step < motion_diag::STILL_STEP_U {
            t.win_still += 1;
        }

        // Direction reversal: this tick's velocity points back against the
        // last one, with both fast enough for the direction to be a
        // decision rather than float noise. Light ticks reuse the previous
        // velocity unchanged, so they can never register a false reversal.
        let same_state = t.last_state.map(|s| s.compact_id()) == Some(state.compact_id());
        if !same_state {
            if t.last_state.is_some() {
                t.win_state_changes += 1;
            }
            t.last_state = Some(state);
        }

        let prev = t.prev_velocity;
        let speed = vel.norm();
        let prev_speed = prev.norm();
        let reversed = speed > motion_diag::REVERSAL_MIN_SPEED_U
            && prev_speed > motion_diag::REVERSAL_MIN_SPEED_U
            && vel.dot(&prev) < 0.0;
        // A reversal taken at running speed is a visible twitch; one taken
        // at a crawl is a player settling onto a target.
        let fast = speed > motion_diag::REVERSAL_FAST_SPEED_U
            && prev_speed > motion_diag::REVERSAL_FAST_SPEED_U;
        motion_diag::note_tick(state, reversed && same_state, fast);

        // Keep the last few ticks so a reversal can be shown in context.
        t.ring[t.ring_idx] = (pos, vel);
        t.ring_idx = (t.ring_idx + 1) % motion_diag::RING;
        if reversed && same_state && fast {
            // Unwind the ring oldest-to-newest.
            let mut samples = Vec::with_capacity(motion_diag::RING);
            for k in 0..motion_diag::RING {
                let s = t.ring[(t.ring_idx + k) % motion_diag::RING];
                if s.0 != Vector3::zeros() || s.1 != Vector3::zeros() {
                    samples.push(s);
                }
            }
            motion_diag::capture_reversal(state, t_ms, id, samples, ball_pos, ball_vel);
        }
        if reversed {
            t.win_reversals += 1;
            // A reversal with no state change under it means one state's
            // own velocity fn flipped direction — a steering problem.
            // With a state change, two states disagree about where to go
            // — a decision problem. The split decides which to fix.
            if same_state {
                t.win_reversals_in_state += 1;
            }
        }
        t.prev_velocity = vel;

        if t.win_ticks >= motion_diag::WINDOW_TICKS {
            let net = (pos - t.win_start).norm();
            motion_diag::record_window(
                id,
                group,
                t_ms,
                state,
                t.win_path,
                net,
                t.win_reversals,
                t.win_reversals_in_state,
                t.win_state_changes,
                t.win_still,
                t.win_ticks,
            );
            t.win_start = pos;
            t.win_path = 0.0;
            t.win_ticks = 0;
            t.win_reversals = 0;
            t.win_reversals_in_state = 0;
            t.win_state_changes = 0;
            t.win_still = 0;
        }
    }

    pub fn heading(&self) -> f32 {
        self.velocity.y.atan2(self.velocity.x)
    }

    pub fn has_ball(&self, ctx: &StateProcessingContext<'_>) -> bool {
        ctx.ball().owner_id() == Some(self.id)
    }

    pub fn update_waypoint_index(&mut self, tick_context: &GameTickContext) {
        if self.cached_waypoints.is_empty() {
            self.rebuild_waypoint_cache();
        }
        self.waypoint_manager.update(
            &tick_context.positions.players.position(self.id),
            &self.cached_waypoints,
        );
    }

    /// `update_waypoint_index` with the player's known `field.players`
    /// slot — same stored position, one hash probe less per tick.
    fn update_waypoint_index_at(&mut self, field_index: usize, tick_context: &GameTickContext) {
        if self.cached_waypoints.is_empty() {
            self.rebuild_waypoint_cache();
        }
        self.waypoint_manager.update(
            &tick_context
                .positions
                .players
                .position_by_index(field_index, self.id),
            &self.cached_waypoints,
        );
    }

    pub fn get_waypoints_as_vectors(&self) -> &[Vector3<f32>] {
        &self.cached_waypoints
    }

    pub fn should_follow_waypoints(&self, ctx: &StateProcessingContext) -> bool {
        // Ball carrier doesn't follow waypoints — they move freely
        if self.has_ball(ctx) {
            return false;
        }

        // Best chaser pursues the ball, not waypoints
        if !ctx.ball().is_owned() && ctx.team().is_best_player_to_chase_ball() {
            return false;
        }

        // If any teammate is too close (< 12u, the natural "shoulder-
        // to-shoulder" bunching distance), follow waypoints back to
        // formation. This is the anti-grouping reinforcement: the
        // moment two of our players are crammed into one yard, one
        // of them peels off to their assigned tactical position. The
        // shorter of them (by id) peels, to avoid both trying to move
        // simultaneously.
        let me_id = self.id;
        let me_pos = self.position;
        let teammate_crowding = ctx.players().teammates().all().any(|t| {
            if t.id == me_id {
                return false;
            }
            let d_sq = (t.position - me_pos).norm_squared();
            if d_sq >= 144.0 {
                return false;
            } // 12² = 144
            // Only one of the pair peels (the lower id). Keeps the
            // behaviour deterministic per-tick and avoids both
            // leaving their post simultaneously.
            t.id > me_id
        });
        if teammate_crowding {
            return true;
        }

        // Everyone else follows waypoints to maintain tactical shape
        // Waypoints represent position-specific movement patterns that keep
        // formation spread and prevent clustering
        true
    }
}

#[derive(Copy, Clone)]
pub struct MatchPlayerLite {
    pub id: u32,
    pub position: Vector3<f32>,
    pub tactical_positions: PlayerPositionType,
}

impl MatchPlayerLite {
    pub fn has_ball(&self, ctx: &StateProcessingContext<'_>) -> bool {
        ctx.ball().owner_id() == Some(self.id)
    }

    pub fn velocity(&self, ctx: &StateProcessingContext<'_>) -> Vector3<f32> {
        ctx.tick_context.positions.players.velocity(self.id)
    }

    pub fn distance(&self, ctx: &StateProcessingContext<'_>) -> f32 {
        ctx.tick_context.grid.get(self.id, ctx.player.id)
    }
}

impl From<&MatchPlayer> for MatchPlayerLite {
    fn from(player: &MatchPlayer) -> Self {
        MatchPlayerLite {
            id: player.id,
            position: player.position,
            tactical_positions: player.tactical_position.current_position,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::club::player::builder::PlayerBuilder;
    use crate::r#match::MatchPlayer;
    use crate::r#match::defenders::states::DefenderState;
    use crate::r#match::forwarders::states::ForwardState;
    use crate::r#match::midfielders::states::MidfielderState;
    use crate::r#match::player::state::PlayerState;
    use crate::r#match::player::transition::TransitionSource;
    use crate::shared::fullname::FullName;
    use crate::{
        PersonAttributes, PlayerAttributes, PlayerPosition, PlayerPositionType, PlayerPositions,
        PlayerSkills,
    };
    use chrono::NaiveDate;

    fn build_player(pos: PlayerPositionType) -> MatchPlayer {
        let player = PlayerBuilder::new()
            .id(1)
            .full_name(FullName::new("T".to_string(), "P".to_string()))
            .birth_date(NaiveDate::from_ymd_opt(2000, 1, 1).unwrap())
            .country_id(1)
            .attributes(PersonAttributes::default())
            .skills(PlayerSkills::default())
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: pos,
                    level: 18,
                }],
            })
            .player_attributes(PlayerAttributes::default())
            .build()
            .unwrap();
        MatchPlayer::from_player(1, &player, pos, false, None)
    }

    #[test]
    fn starter_minutes_count_full_90() {
        let p = build_player(PlayerPositionType::DefenderCenter);
        // entry_match_time_ms defaults to 0 (kickoff). At full time
        // the helper returns the elapsed minutes.
        let minutes = p.minutes_played_at(90 * 60_000);
        assert_eq!(minutes, 90);
    }

    #[test]
    fn substituted_in_player_minutes_only_count_post_entry() {
        // Substitute came on at the 70th minute; at full time their
        // minutes-played should be 20, not 90 — the rating helper
        // damps cameo bonuses based on this number, so an 80th-minute
        // sub posting a key pass shouldn't be rated like a starter.
        let mut p = build_player(PlayerPositionType::ForwardCenter);
        p.entry_match_time_ms = 70 * 60_000;
        let minutes = p.minutes_played_at(90 * 60_000);
        assert_eq!(minutes, 20);
    }

    #[test]
    fn match_end_stats_carry_substituted_minutes() {
        // The end-of-match snapshot accepts the minutes count from the
        // caller — the substitution path computes it via
        // `minutes_played_at` and writes the right value into the
        // resulting `PlayerMatchEndStats`.
        let mut p = build_player(PlayerPositionType::MidfielderCenter);
        p.entry_match_time_ms = 60 * 60_000;
        let minutes = p.minutes_played_at(90 * 60_000);
        let stats = p.to_match_end_stats(minutes);
        assert_eq!(stats.minutes_played, 30);
    }

    #[test]
    fn minutes_played_at_caps_at_120_for_extra_time() {
        let p = build_player(PlayerPositionType::DefenderCenter);
        // Pathological — 200 minute match should clamp to 120.
        let minutes = p.minutes_played_at(200 * 60_000);
        assert_eq!(minutes, 120);
    }

    #[test]
    fn transition_to_sets_state_and_resets_timer() {
        let mut p = build_player(PlayerPositionType::ForwardCenter);
        p.in_state_time = 137;
        p.transition_to(
            PlayerState::Forward(ForwardState::Dribbling),
            TransitionSource::Handler,
        );
        assert_eq!(p.state, PlayerState::Forward(ForwardState::Dribbling));
        assert_eq!(p.in_state_time, 0, "transition must reset the state timer");
    }

    #[test]
    fn redirect_to_sets_state_but_preserves_timer() {
        // The timer-preserving primitive is still available for
        // destinations that don't read `in_state_time`.
        let mut p = build_player(PlayerPositionType::DefenderCenter);
        p.in_state_time = 137;
        p.redirect_to(
            PlayerState::Defender(DefenderState::Marking),
            TransitionSource::LooseBallOverride,
        );
        assert_eq!(p.state, PlayerState::Defender(DefenderState::Marking));
        assert_eq!(p.in_state_time, 137, "redirect_to must preserve the timer");
    }

    #[test]
    fn redirect_to_fresh_resets_timer() {
        // Every entry into a timeout-driven state uses the fresh variant,
        // so the destination's guards count from the redirect.
        let mut p = build_player(PlayerPositionType::DefenderCenter);
        p.in_state_time = 137;
        p.redirect_to_fresh(
            PlayerState::Defender(DefenderState::TakeBall),
            TransitionSource::LooseBallOverride,
        );
        assert_eq!(p.state, PlayerState::Defender(DefenderState::TakeBall));
        assert_eq!(p.in_state_time, 0, "redirect_to_fresh must reset the timer");
    }

    #[test]
    fn run_for_ball_enters_takeball_with_a_fresh_timer() {
        // The chase starts at the redirect, so TakeBall's give-up timers
        // must start there too — a carried-over timer made the goalkeeper
        // variant abandon on its second tick.
        let mut p = build_player(PlayerPositionType::MidfielderCenter);
        p.in_state_time = 90;
        p.run_for_ball();
        assert_eq!(p.state, PlayerState::Midfielder(MidfielderState::TakeBall));
        assert_eq!(p.in_state_time, 0, "run_for_ball resets in_state_time");
    }

    #[test]
    fn run_for_ball_is_ignored_while_committed() {
        // A header already launched is not abandoned because the ball came
        // loose — the loose-ball claim passes to the next-closest player.
        let mut p = build_player(PlayerPositionType::ForwardCenter);
        p.transition_to(
            PlayerState::Forward(ForwardState::Heading),
            TransitionSource::Handler,
        );
        p.run_for_ball();
        assert_eq!(
            p.state,
            PlayerState::Forward(ForwardState::Heading),
            "committed action must survive the loose-ball signal"
        );
    }

    #[test]
    fn set_default_state_resets_the_timer() {
        // reset_players_positions / substitutions route through
        // set_default_state. A restart is an ENTRY into the default state,
        // so its timers start at the whistle — otherwise every defender
        // left Standing for HoldingLine on the first tick after a goal.
        let mut p = build_player(PlayerPositionType::DefenderCenter);
        p.redirect_to(
            PlayerState::Defender(DefenderState::Marking),
            TransitionSource::Handler,
        );
        p.in_state_time = 250;
        p.set_default_state(TransitionSource::Reset);
        assert_eq!(p.state, PlayerState::Defender(DefenderState::Standing));
        assert_eq!(p.in_state_time, 0, "set_default_state resets the timer");
    }

    /// The vertical axis is the one that goes missing without anything
    /// downstream complaining. `move_to` integrated x and y only for the
    /// engine's whole life, so `GoalkeeperJumpingState` computed a textbook
    /// arc every tick into a value nobody applied — and the only symptom was
    /// a keeper who never left the ground. These pin the trajectory against
    /// a stopwatch and a tape measure so it cannot quietly vanish again.
    #[test]
    fn a_leap_peaks_where_it_was_aimed_and_comes_back_down() {
        let mut keeper = build_player(PlayerPositionType::Goalkeeper);
        keeper.leap(0.75);

        let mut peak = 0.0f32;
        let mut ticks = 0u32;
        loop {
            keeper.move_to();
            ticks += 1;
            peak = peak.max(keeper.height);
            if !keeper.is_airborne() {
                break;
            }
            assert!(ticks < 1000, "a leap that never lands");
        }

        // Stepwise integration undershoots the closed-form apex by about half
        // a tick of climb — 2.5% here, and 4% covers every height a
        // footballer reaches.
        assert!((peak - 0.75).abs() < 0.75 * 0.04, "peaked at {peak} m");
        // `2v/g` ticks at 10 ms each: 0.78 s off the ground for a 0.75 m
        // jump, which is what a human being does.
        assert!(
            (ticks as i64 - 78).abs() <= 2,
            "{ticks} ticks in the air, expected ~78"
        );
        assert_eq!(keeper.height, 0.0, "landed short of the turf");
        assert_eq!(keeper.vertical_speed, 0.0, "still climbing on the ground");
    }

    #[test]
    fn nobody_jumps_twice_off_the_same_ground() {
        let mut keeper = build_player(PlayerPositionType::Goalkeeper);
        keeper.leap(0.75);
        keeper.move_to();
        let climbing = keeper.vertical_speed;

        // A state re-entered mid-flight must not re-arm the take-off, or the
        // keeper hangs up there for as long as it keeps firing.
        keeper.leap(0.75);
        assert_eq!(keeper.vertical_speed, climbing);
    }

    #[test]
    fn a_player_on_the_turf_never_sinks_into_it() {
        let mut player = build_player(PlayerPositionType::MidfielderCenter);
        for _ in 0..500 {
            player.move_to();
        }
        assert_eq!(player.height, 0.0);
        assert_eq!(
            player.vertical_speed, 0.0,
            "gravity accumulated on a man standing still"
        );
    }
}
