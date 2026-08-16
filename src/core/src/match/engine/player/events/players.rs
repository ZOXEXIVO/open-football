use crate::PlayerFieldPositionGroup;
use crate::club::player::PlayerTrait;
use crate::r#match::engine::ball::ball::interactions::SaveModel;
#[cfg(feature = "match-logs")]
use crate::r#match::engine::ball::ball::interactions::block_diag::BlockDiag;
use crate::r#match::engine::ball::ball::motion::SpinModel;
use crate::r#match::engine::ball::ball::{
    Ball, GRAVITY_PER_TICK, GROUND_FRICTION, PossessionSource,
};
use crate::r#match::engine::flow::context::PendingAdvantage;
use crate::r#match::engine::flow::rng::MatchRng;
use crate::r#match::engine::goal::GOAL_WIDTH as GOAL_MOUTH_HALF_WIDTH;
use crate::r#match::engine::officiating::referee::{ContactLocation, FoulCallContext};
use crate::r#match::engine::psychology::{NegativeEvent, PositiveEvent};
use crate::r#match::engine::set_pieces::{FreeKickBand, wall_block_prob, wall_size_for};
use crate::r#match::engine::zones::MatchZone;
use crate::r#match::events::Event;
use crate::r#match::player::events::gk_claim::GkClaimContest;
#[cfg(feature = "match-logs")]
use crate::r#match::player::events::gk_claim::gk_claim_diag;
use crate::r#match::player::events::{PassingEventContext, ShootingEventContext};
#[cfg(feature = "match-logs")]
use crate::r#match::player::state::PlayerState;
use crate::r#match::player::statistics::MatchStatisticType;
use crate::r#match::player::strategies::passing::CrossType;
use crate::r#match::player::strategies::players::ShotSkillInputs;
use crate::r#match::player::strategies::players::ShotSkillProfile;
use crate::r#match::player::strategies::players::ops::effective_skill::{
    ActionContext as EffSkillCtx, effective_skill,
};
#[cfg(feature = "match-logs")]
use crate::r#match::player::strategies::players::ops::forward_shot_decision::time_band_diag;
use crate::r#match::player::strategies::players::ops::skill_composites as sc;
use crate::r#match::player::strategies::players::skills::SkillCurve;
use crate::r#match::{
    GoalDetail, GoalPosition, MatchContext, MatchField, MatchPlayer, OffsideSnapshot,
    PassOriginRestart, PlayerSide, ResultMatchPositionData, ShotTarget,
};
#[cfg(feature = "match-logs")]
use crate::match_log_info;
#[cfg(feature = "match-logs")]
use crate::mid_run_diag::FoulCensus;
use log::debug;
use nalgebra::Vector3;
#[cfg(feature = "match-logs")]
use std::sync::atomic::Ordering;

// ───────────────────────────────────────────────────────────────────────────
// Save-accounting diagnostic counters. Trace each save event's credit pair
// to find why `saves > shots_on_target` (the impossible 145% baseline).
// Each save site increments both `saves_credited[site]` and
// `on_target_paired[site]` — divergence pinpoints where on_target is
// missed (shooter not found, etc.) or where saves are double-credited.
// match-logs feature only.
// ───────────────────────────────────────────────────────────────────────────
/// Why a shot did or didn't produce a key pass. The engine emits ~2 key
/// passes per team per match against a real ~7-8, and the counter alone
/// cannot say whether the TAGGING is broken or whether the engine's shots
/// genuinely don't arrive from a pass to the shooter. These split the
/// two: `SHOTS` vs `NO_LINK` (no completed pass on record at all) vs
/// `WRONG_RECEIVER` (a pass completed, but to someone other than the
/// player who ended up shooting) vs `CREDITED`.
#[cfg(feature = "match-logs")]
/// Why shots end up off target.
///
/// The population on-target rate is set by three independent forced-miss
/// rolls (`wide_miss_chance`, `over_bar_chance`, `miskick_probability`)
/// plus the aim band, and the aim band is documented as NOT being the
/// lever. Without a breakdown, re-tuning that rate means moving two
/// numbers and re-running until it lands — this makes it arithmetic
/// instead. Every counter is per shot struck.
#[cfg(feature = "match-logs")]
pub mod shot_accuracy_diag {
    use std::sync::atomic::{AtomicU64, Ordering};

    /// Shots that reached the aiming code.
    pub static SHOTS: AtomicU64 = AtomicU64::new(0);
    /// The wide-miss roll fired (target pushed outside a post).
    pub static WIDE_FIRED: AtomicU64 = AtomicU64::new(0);
    /// The over-bar roll fired (apex skied clear of the crossbar).
    pub static OVER_FIRED: AtomicU64 = AtomicU64::new(0);
    /// The miskick roll fired (target smeared up to 1.5 goal-widths).
    pub static MISKICK_FIRED: AtomicU64 = AtomicU64::new(0);
    /// Final aim point landed between the posts.
    pub static AIM_INSIDE_POSTS: AtomicU64 = AtomicU64::new(0);
    /// Aim between the posts AND not skied — the shots that can be
    /// on target once they reach the goal line.
    pub static AIM_ON_FRAME: AtomicU64 = AtomicU64::new(0);

    pub fn reset() {
        for c in [
            &SHOTS,
            &WIDE_FIRED,
            &OVER_FIRED,
            &MISKICK_FIRED,
            &AIM_INSIDE_POSTS,
            &AIM_ON_FRAME,
        ] {
            c.store(0, Ordering::Relaxed);
        }
    }

    pub fn snapshot() -> [u64; 6] {
        [
            SHOTS.load(Ordering::Relaxed),
            WIDE_FIRED.load(Ordering::Relaxed),
            OVER_FIRED.load(Ordering::Relaxed),
            MISKICK_FIRED.load(Ordering::Relaxed),
            AIM_INSIDE_POSTS.load(Ordering::Relaxed),
            AIM_ON_FRAME.load(Ordering::Relaxed),
        ]
    }
}

pub mod key_pass_diag {
    use std::sync::atomic::{AtomicU64, Ordering};

    pub static SHOTS: AtomicU64 = AtomicU64::new(0);
    pub static NO_LINK: AtomicU64 = AtomicU64::new(0);
    pub static WRONG_RECEIVER: AtomicU64 = AtomicU64::new(0);
    pub static STALE_WINDOW: AtomicU64 = AtomicU64::new(0);
    pub static CREDITED: AtomicU64 = AtomicU64::new(0);
    /// Shots by how the shooter acquired the ball, indexed by
    /// `PossessionSource::index()`. Answers what feeds shots when a pass
    /// doesn't.
    pub static SUPPLY: [AtomicU64; crate::r#match::engine::ball::ball::PossessionSource::COUNT] = [
        AtomicU64::new(0),
        AtomicU64::new(0),
        AtomicU64::new(0),
        AtomicU64::new(0),
        AtomicU64::new(0),
    ];

    pub fn reset() {
        for c in [&SHOTS, &NO_LINK, &WRONG_RECEIVER, &STALE_WINDOW, &CREDITED] {
            c.store(0, Ordering::Relaxed);
        }
        for c in SUPPLY.iter() {
            c.store(0, Ordering::Relaxed);
        }
    }

    /// Shot counts per `PossessionSource`, in `PossessionSource::NAMES` order.
    pub fn supply_snapshot() -> [u64; crate::r#match::engine::ball::ball::PossessionSource::COUNT] {
        let mut out = [0u64; crate::r#match::engine::ball::ball::PossessionSource::COUNT];
        for (i, c) in SUPPLY.iter().enumerate() {
            out[i] = c.load(Ordering::Relaxed);
        }
        out
    }

    /// `(shots, no_link, wrong_receiver, stale_window, credited)`
    pub fn snapshot() -> (u64, u64, u64, u64, u64) {
        (
            SHOTS.load(Ordering::Relaxed),
            NO_LINK.load(Ordering::Relaxed),
            WRONG_RECEIVER.load(Ordering::Relaxed),
            STALE_WINDOW.load(Ordering::Relaxed),
            CREDITED.load(Ordering::Relaxed),
        )
    }
}

#[cfg(feature = "match-logs")]
pub mod save_accounting_stats {
    use std::sync::atomic::{AtomicU64, Ordering};

    // Index meaning: 0=parry 1=catch 2=clear
    pub static SITE_LABELS: [&str; 3] = ["parry", "catch", "clear"];
    pub static SAVES_CREDITED: [AtomicU64; 3] =
        [AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0)];
    pub static SHOTS_FACED_INC: [AtomicU64; 3] =
        [AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0)];
    pub static ON_TARGET_PAIRED: [AtomicU64; 3] =
        [AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0)];
    /// Site fired but the previous_owner / shooter could not be found in
    /// the live players slice — save was credited but on_target was NOT.
    /// This is the primary suspect for saves > on_target.
    pub static SHOOTER_MISSING: [AtomicU64; 3] =
        [AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0)];
    /// Site fired but `previous_owner` itself was None.
    pub static PREVIOUS_OWNER_NONE: [AtomicU64; 3] =
        [AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0)];
    /// On-goal credit (path 4): shot reached the net, scorer credited
    /// with on_target. No save here.
    pub static ON_TARGET_FROM_GOAL: AtomicU64 = AtomicU64::new(0);
    // Save-pipeline visibility — how many shot ticks reach the save check
    // (within reach window) and how often the keeper actually engages.
    pub static SAVE_TICKS_REACHED: AtomicU64 = AtomicU64::new(0);
    pub static SAVE_TICKS_OUT_OF_REACH: AtomicU64 = AtomicU64::new(0);
    pub static SAVE_TICKS_PAST_GOAL_LINE: AtomicU64 = AtomicU64::new(0);
    pub static SAVE_PHYSICS_FIRED: AtomicU64 = AtomicU64::new(0);
    pub static SAVE_PHYSICS_PASSED: AtomicU64 = AtomicU64::new(0);

    pub fn reset() {
        for arr in [
            &SAVES_CREDITED,
            &SHOTS_FACED_INC,
            &ON_TARGET_PAIRED,
            &SHOOTER_MISSING,
            &PREVIOUS_OWNER_NONE,
        ] {
            for a in arr.iter() {
                a.store(0, Ordering::Relaxed);
            }
        }
        ON_TARGET_FROM_GOAL.store(0, Ordering::Relaxed);
    }

    pub fn snapshot() -> Snapshot {
        Snapshot {
            saves: [
                SAVES_CREDITED[0].load(Ordering::Relaxed),
                SAVES_CREDITED[1].load(Ordering::Relaxed),
                SAVES_CREDITED[2].load(Ordering::Relaxed),
            ],
            on_target: [
                ON_TARGET_PAIRED[0].load(Ordering::Relaxed),
                ON_TARGET_PAIRED[1].load(Ordering::Relaxed),
                ON_TARGET_PAIRED[2].load(Ordering::Relaxed),
            ],
            shooter_missing: [
                SHOOTER_MISSING[0].load(Ordering::Relaxed),
                SHOOTER_MISSING[1].load(Ordering::Relaxed),
                SHOOTER_MISSING[2].load(Ordering::Relaxed),
            ],
            prev_owner_none: [
                PREVIOUS_OWNER_NONE[0].load(Ordering::Relaxed),
                PREVIOUS_OWNER_NONE[1].load(Ordering::Relaxed),
                PREVIOUS_OWNER_NONE[2].load(Ordering::Relaxed),
            ],
            on_target_goal: ON_TARGET_FROM_GOAL.load(Ordering::Relaxed),
        }
    }

    pub struct Snapshot {
        pub saves: [u64; 3],
        pub on_target: [u64; 3],
        pub shooter_missing: [u64; 3],
        pub prev_owner_none: [u64; 3],
        pub on_target_goal: u64,
    }
}

/// Helper struct to encapsulate player passing skills and condition.
/// Skill fields are already fatigue-folded via `effective_skill` —
/// callers should not multiply by raw condition again. The remaining
/// `availability_factor` covers ONLY independent physical-availability
/// effects (chronic fitness, jadedness from cumulative load) that
/// `effective_skill` doesn't model.
struct PassSkills {
    passing: f32,
    technique: f32,
    vision: f32,
    composure: f32,
    decisions: f32,
    concentration: f32,
    flair: f32,
    long_shots: f32,
    crossing: f32,
    stamina: f32,
    /// Independent of skill condition — captures fitness*(1-jadedness)
    /// only. Lightly applied to power consistency / miskick rates so
    /// it isn't a second fatigue penalty stacked on top of effective
    /// skills.
    availability_factor: f32,
}

impl PassSkills {
    /// Build a fatigue-aware skill snapshot. Skill values pass through
    /// `effective_skill` so a tired player produces lower passing
    /// numbers without each call site having to apply the band itself.
    /// `minute` should be the current match minute (0..=120). Use
    /// `MatchContext::total_match_time / 60_000` to derive it.
    fn from_player(player: &MatchPlayer, minute: u32) -> Self {
        let tech = EffSkillCtx::technical(minute);
        let mental = EffSkillCtx::mental(minute);
        let expl = EffSkillCtx::explosive(minute);

        // Floors lowered from 0.10 to 0.02 so the bottom of the 1-20
        // range visibly separates: a skill-1 player now lands at 0.05
        // (raw) rather than being lifted to the same 0.10 as a skill-2.
        // Stamina keeps a slightly higher floor (0.05) so a wrecked
        // player can still walk through a possession; flair was
        // already unfloored.
        let passing = (effective_skill(player, player.skills.technical.passing, tech) / 20.0)
            .clamp(0.02, 1.0);
        let technique = (effective_skill(player, player.skills.technical.technique, tech) / 20.0)
            .clamp(0.02, 1.0);
        let vision =
            (effective_skill(player, player.skills.mental.vision, mental) / 20.0).clamp(0.02, 1.0);
        let composure = (effective_skill(player, player.skills.mental.composure, mental) / 20.0)
            .clamp(0.02, 1.0);
        let decisions = (effective_skill(player, player.skills.mental.decisions, mental) / 20.0)
            .clamp(0.02, 1.0);
        let concentration = (effective_skill(player, player.skills.mental.concentration, mental)
            / 20.0)
            .clamp(0.02, 1.0);
        let flair =
            (effective_skill(player, player.skills.mental.flair, mental) / 20.0).clamp(0.0, 1.0);
        let long_shots = (effective_skill(player, player.skills.technical.long_shots, tech) / 20.0)
            .clamp(0.02, 1.0);
        let crossing = (effective_skill(player, player.skills.technical.crossing, tech) / 20.0)
            .clamp(0.02, 1.0);
        let stamina =
            (effective_skill(player, player.skills.physical.stamina, expl) / 20.0).clamp(0.05, 1.0);

        // Availability factor — independent of `effective_skill`'s
        // condition handling. Captures chronic fitness (long-term shape)
        // and jadedness (cumulative load not yet recovered). Lightly
        // applied so we don't double-stack on top of the per-skill
        // condition penalty.
        let fitness_factor = (player.player_attributes.fitness as f32 / 10000.0).clamp(0.5, 1.0);
        let jadedness_penalty = (player.player_attributes.jadedness as f32 / 10000.0) * 0.20;
        let availability_factor = (fitness_factor - jadedness_penalty).clamp(0.70, 1.0);

        Self {
            passing,
            technique,
            vision,
            composure,
            decisions,
            concentration,
            flair,
            long_shots,
            crossing,
            stamina,
            availability_factor,
        }
    }

    /// Overall passing quality. Skills are already fatigue-aware; the
    /// availability factor adds a small independent penalty for
    /// chronic fitness / jadedness without re-applying condition.
    fn overall_quality(&self) -> f32 {
        let base_quality = self.passing * 0.5 + self.technique * 0.3 + self.vision * 0.2;
        // 0.70..1.00 gives a max ~30% softening from chronic load — the
        // skill values already ship the per-match condition curve.
        base_quality * self.availability_factor
    }

    /// Decision-making quality for trajectory selection. Same rule:
    /// no second condition multiplier — only the independent
    /// availability factor on top of effective skills.
    fn decision_quality(&self) -> f32 {
        (self.decisions * 0.4 + self.vision * 0.3 + self.concentration * 0.2 + self.composure * 0.1)
            * self.availability_factor
    }
}

/// What is standing between a passer and their target.
struct LaneTraffic {
    /// Opponents inside the lane corridor.
    count: usize,
    /// Distance ALONG the lane to the nearest of them, in game units.
    /// `f32::MAX` when the lane is clear.
    nearest: f32,
}

impl Default for LaneTraffic {
    fn default() -> Self {
        LaneTraffic {
            count: 0,
            nearest: f32::MAX,
        }
    }
}

/// Different trajectory styles for passes
/// Each type represents a different flight time and arc height to reach the same target
#[derive(Debug, Clone, Copy)]
enum TrajectoryType {
    /// Ground pass - minimal flight time, almost zero arc (fastest)
    Ground,
    /// Low driven pass - short flight time, low arc (fast and direct)
    LowDriven,
    /// Medium arc - moderate flight time, balanced trajectory
    MediumArc,
    /// High arc - longer flight time, high parabolic arc (for distance/obstacles)
    HighArc,
    /// Chip - very high arc over short distance (for beating defenders)
    Chip,
    /// A modelled cross. The shape comes from the [`CrossType`] the
    /// crossing state chose, NOT from lane traffic — a cross is played
    /// over the traffic by definition, so routing it through the
    /// obstacle-driven selector produced the contradiction documented in
    /// `passing::cross`: the only deliveries that qualified as crosses
    /// were the ones with a clear lane, and a clear lane meant "keep it
    /// on the deck".
    Cross(CrossType),
}

impl TrajectoryType {
    /// Whether the launch solver should buy a ballistic arc (apex-first)
    /// rather than keeping the calibrated rolling speed and adding lift.
    fn is_ballistic(self) -> bool {
        match self {
            TrajectoryType::MediumArc | TrajectoryType::HighArc | TrajectoryType::Chip => true,
            TrajectoryType::Cross(ct) => ct.is_lofted(),
            _ => false,
        }
    }
}

/// How dirty was the foul — drives card probabilities.
/// `Normal`: shirt pull, mistimed challenge → occasional yellow.
/// `Reckless`: studs-up, late, from behind → high yellow, sometimes red.
/// `Violent`: denial of goalscoring opportunity or violent conduct → direct red.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FoulSeverity {
    Normal,
    Reckless,
    Violent,
}

#[derive(Debug, Clone)]
pub enum PlayerEvent {
    Goal(u32, bool),
    Assist(u32),
    BallCollision(u32),
    TacklingBall(u32),
    BallOwnerChange(u32),
    PassTo(PassingEventContext),
    ClearBall(Vector3<f32>),
    RushOut(u32),
    Shoot(ShootingEventContext),
    MovePlayer(u32, Vector3<f32>),
    /// `(player_id, apex_metres)` — take this player off the ground.
    ///
    /// A leap is normally requested by the state machine on the tick a
    /// player commits to an aerial action (see
    /// `PlayerMatchState::leap_apex`). The ball's own passes decide some
    /// aerial contests themselves — an interception plucked out of the
    /// air is the clearest — and they hold the players immutably, so they
    /// ask for the jump instead of performing it. Without this a defender
    /// took balls at head height standing perfectly still, which is the
    /// single most visible wrong thing in the 3D replay.
    Leap(u32, f32),
    StayInGoal(u32),
    MoveBall(u32, Vector3<f32>),
    CommunicateMessage(u32, &'static str),
    OfferSupport(u32),
    ClaimBall(u32),
    GainBall(u32),
    CaughtBall(u32),
    /// Goalkeeper got a touch on a real shot but couldn't catch it —
    /// the ball deflected away (parried wide, palmed over the bar,
    /// punched off the line). Emitted by the diving and catching states
    /// when they exit on "ball moving away" while `cached_shot_target`
    /// was set. The handler credits a save and increments `shots_faced`
    /// so the rating helper sees the GK's full workload.
    ParriedBall(u32),
    /// Foul committed by (fouler_id, severity). Dispatcher decides cards.
    CommitFoul(u32, FoulSeverity),
    Offside(u32, Vector3<f32>), // (offside_player_id, position_for_free_kick)
    RequestHeading(u32, Vector3<f32>),
    RequestShot(u32, Vector3<f32>),
    RequestBallReceive(u32),
    TakeBall(u32),
}

/// Foul / advantage / direct free-kick wall logic. Groups the
/// behaviours that the dispatcher used to host as scattered statics:
/// card-probability computation, advantage window resolution, deferred
/// card application, and direct free-kick wall checks. All methods are
/// associated functions on the unit struct — no instance state — but
/// living here keeps the foul lifecycle in one cohesive place rather
/// than mixed into the broader `PlayerEventDispatcher`.
pub struct FoulResolver;

impl FoulResolver {
    /// Resolve any pending advantage: if the fouled team has lost
    /// possession inside the referee's window, whistle the foul back
    /// and award the restart. If the window expires (or the advantage
    /// materialised into a shot / deep entry), settle the card without
    /// stopping play. Called once per tick from the engine loop.
    pub fn tick_advantage(field: &mut MatchField, context: &mut MatchContext) {
        let adv = match context.pending_advantage {
            Some(a) => a,
            None => return,
        };
        let now = context.current_tick();
        let owner_team = field
            .ball
            .current_owner
            .and_then(|id| field.players.iter().find(|p| p.id == id))
            .map(|p| p.team_id);
        let possession_with_fouled = owner_team == Some(adv.fouled_team_id);

        // Possession lost inside the window → pull play back.
        if now < adv.expire_tick && !possession_with_fouled && owner_team.is_some() {
            context.pending_advantage = None;
            // Free-kick protection.
            if field.ball.current_owner.is_some() {
                field.ball.claim_cooldown = 150;
                field.ball.flags.in_flight_state = 150;
                field.ball.contested_claim_count = 0;
            }
            PlayerEventDispatcher::award_restart_for_foul(
                adv.fouler_id,
                adv.severity,
                field,
                context,
            );
            PlayerEventDispatcher::apply_card_decision(
                adv.fouler_id,
                adv.severity,
                adv.yellow_prob,
                adv.red_prob,
                field,
                context,
            );
            return;
        }

        // Window expired (advantage played) → just settle the card,
        // play continues without restart.
        if now >= adv.expire_tick {
            context.pending_advantage = None;
            PlayerEventDispatcher::apply_card_decision(
                adv.fouler_id,
                adv.severity,
                adv.yellow_prob,
                adv.red_prob,
                field,
                context,
            );
        }
    }

    /// Build the marginal-call context for the referee gate in
    /// `handle_commit_foul_event`. Folds severity into a contact
    /// magnitude, determines the contact location (penalty box vs
    /// clear vs normal contact), figures out which side is fouled for
    /// the home-bias check, and computes a match temperature from
    /// derby intensity plus the cumulative foul count.
    pub fn build_call_context(
        fouler_id: u32,
        severity: FoulSeverity,
        field: &MatchField,
        context: &MatchContext,
    ) -> FoulCallContext {
        let location = if Self::contact_in_a_penalty_box(field) {
            ContactLocation::PenaltyBox
        } else {
            match severity {
                FoulSeverity::Normal => ContactLocation::Normal,
                FoulSeverity::Reckless | FoulSeverity::Violent => ContactLocation::ClearFoul,
            }
        };
        let contact_severity = match severity {
            FoulSeverity::Normal => 0.25,
            FoulSeverity::Reckless => 0.65,
            FoulSeverity::Violent => 0.92,
        };
        let fouled_team_is_home = Self::fouled_side_is_home(fouler_id, field);
        // The deeper into a heated match, the more marginal contacts the
        // referee whistles. Caps at 0.7 from foul history so derby
        // intensity can still nudge it the rest of the way.
        let temp_from_fouls = (field
            .players
            .iter()
            .map(|p| p.fouls_committed as f32)
            .sum::<f32>()
            / 30.0)
            .clamp(0.0, 0.7);
        let match_temperature =
            (temp_from_fouls + context.environment.derby_intensity * 0.3).clamp(0.0, 1.0);
        FoulCallContext {
            contact_severity,
            match_temperature,
            fouled_team_is_home,
            location,
        }
    }

    /// True if the ball is inside either team's penalty area at the
    /// moment of the foul — used by the marginal-call gate to bump
    /// strictness through `ContactLocation::PenaltyBox`.
    fn contact_in_a_penalty_box(field: &MatchField) -> bool {
        let field_w = field.size.width as f32;
        let field_h = field.size.height as f32;
        let scale = field_w / 105.0;
        let area_w = 40.32 * scale;
        let area_depth = 16.5 * scale;
        let y = field.ball.position.y;
        let x = field.ball.position.x;
        let in_y = y >= (field_h - area_w) * 0.5 && y <= (field_h + area_w) * 0.5;
        if !in_y {
            return false;
        }
        x <= area_depth || x >= field_w - area_depth
    }

    /// True if the side being fouled is the home team. Reads ball
    /// ownership; falls back to the fouler's opponent when the ball is
    /// loose at the moment of the contact.
    fn fouled_side_is_home(fouler_id: u32, field: &MatchField) -> bool {
        let owner_team = field
            .ball
            .current_owner
            .and_then(|id| field.players.iter().find(|p| p.id == id))
            .map(|p| p.team_id);
        if let Some(team) = owner_team {
            return team == field.home_team_id;
        }
        let fouler_team = field
            .players
            .iter()
            .find(|p| p.id == fouler_id)
            .map(|p| p.team_id);
        fouler_team != Some(field.home_team_id)
    }

    /// Roll a wall-block check for a direct free-kick shot. Returns
    /// true if the wall blocked the ball — caller then deflects /
    /// loses the shot. The wall lives only within close/mid bands;
    /// long-range FKs go around it.
    pub fn wall_blocks_direct_fk(
        shooter_id: u32,
        field: &MatchField,
        context: &MatchContext,
        pre_distance: f32,
    ) -> bool {
        let band = FreeKickBand::from_distance(pre_distance);
        if matches!(band, FreeKickBand::Far) {
            return false;
        }
        // Wide angle: the goal's narrow visible-width drops as the
        // shooter drifts laterally. Approximate via the y-deviation
        // from goal centre vs. field height.
        let field_h = context.field_size.height as f32;
        let center_y = field_h * 0.5;
        let shooter_side = field
            .players
            .iter()
            .find(|p| p.id == shooter_id)
            .and_then(|p| p.side);
        let shooter_y = field
            .players
            .iter()
            .find(|p| p.id == shooter_id)
            .map(|p| p.position.y)
            .unwrap_or(center_y);
        let is_wide_angle = (shooter_y - center_y).abs() > field_h * 0.18;

        let wall_size = wall_size_for(band, is_wide_angle).clamp(2, 8);
        // Wall composition: average bravery + positioning of the n
        // closest defenders to the ball, excluding the keeper.
        let mut wall: Vec<&MatchPlayer> = field
            .players
            .iter()
            .filter(|p| {
                p.id != shooter_id
                    && !p.is_sent_off
                    && p.tactical_position.current_position.position_group()
                        != PlayerFieldPositionGroup::Goalkeeper
                    && p.side != shooter_side
            })
            .collect();
        wall.sort_by(|a, b| {
            let da = (a.position - field.ball.position).magnitude();
            let db = (b.position - field.ball.position).magnitude();
            da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
        });
        let wall_n = (wall_size as usize).min(wall.len());
        if wall_n == 0 {
            return false;
        }
        let mut bravery_sum = 0.0;
        let mut positioning_sum = 0.0;
        for d in wall.iter().take(wall_n) {
            bravery_sum += d.skills.mental.bravery;
            positioning_sum += d.skills.mental.positioning;
        }
        let avg_bravery = bravery_sum / wall_n as f32;
        let avg_positioning = (positioning_sum / wall_n as f32 / 20.0).clamp(0.0, 1.0);

        // Taker error: 1.0 - finishing skill (proxy — the lower the
        // taker's finishing, the more often they hit the wall).
        let taker_error = field
            .players
            .iter()
            .find(|p| p.id == shooter_id)
            .map(|p| 1.0 - (p.skills.technical.finishing / 20.0).clamp(0.0, 1.0))
            .unwrap_or(0.5);

        let p = wall_block_prob(avg_positioning, avg_bravery, taker_error, band);
        context.rng.bernoulli(p)
    }
}

pub struct PlayerEventDispatcher;

impl PlayerEventDispatcher {
    pub fn dispatch(
        event: PlayerEvent,
        field: &mut MatchField,
        context: &mut MatchContext,
        match_data: &mut ResultMatchPositionData,
    ) -> Vec<Event> {
        let remaining_events = Vec::new();

        if context.logging_enabled {
            match event {
                PlayerEvent::TakeBall(_) | PlayerEvent::ClaimBall(_) => {}
                _ => debug!("Player event: {:?}, tick = {}", event, context.time.time),
            }
        }

        // The ball is in the net: the whistle has gone and nothing anybody
        // does with it counts.
        //
        // This is a one-tick window — `handle_goal_reset` freezes the world
        // at the end of the same tick the goal is scored in — but the events
        // queued during that tick are still waiting to be dispatched, and
        // several of them WRITE the ball: a keeper beaten on his line is
        // within `KICKABLE_DISTANCE` of a ball that has just crossed it, so a
        // pass or a clearance he asked for a moment earlier would drag it
        // straight back out of the goal. (The old engine hid this by
        // teleporting everybody into formation mid-dispatch, so the reach
        // check failed by accident.)
        //
        // Allow-list rather than deny-list: `Goal` and `Assist` ARE the goal
        // — they are queued by the goal handler itself and credit it — and
        // the rest are bookkeeping the goal does not invalidate.
        if field.ball.in_net.is_some() {
            let touches_the_ball = matches!(
                event,
                PlayerEvent::PassTo(_)
                    | PlayerEvent::Shoot(_)
                    | PlayerEvent::ClearBall(_)
                    | PlayerEvent::MoveBall(_, _)
                    | PlayerEvent::ClaimBall(_)
                    | PlayerEvent::GainBall(_)
                    | PlayerEvent::CaughtBall(_)
                    | PlayerEvent::ParriedBall(_)
                    | PlayerEvent::TakeBall(_)
                    | PlayerEvent::TacklingBall(_)
                    | PlayerEvent::BallCollision(_)
                    | PlayerEvent::BallOwnerChange(_)
                    | PlayerEvent::RequestHeading(_, _)
                    | PlayerEvent::RequestShot(_, _)
                    | PlayerEvent::RequestBallReceive(_)
            );
            if touches_the_ball {
                return remaining_events;
            }
        }

        match event {
            PlayerEvent::Goal(player_id, is_auto_goal) => {
                Self::handle_goal_event(player_id, is_auto_goal, field, context);
            }
            PlayerEvent::Assist(player_id) => {
                Self::handle_assist_event(player_id, field, context);
            }
            PlayerEvent::BallCollision(player_id) => {
                Self::handle_ball_collision_event(player_id, field);
            }
            PlayerEvent::TacklingBall(player_id) => {
                Self::record_team_possession_if_switch(player_id, field, context);
                Self::handle_tackling_ball_event(player_id, field, context);
            }
            PlayerEvent::BallOwnerChange(player_id) => {
                Self::handle_ball_owner_change_event(player_id, field);
            }
            PlayerEvent::PassTo(pass_event_model) => {
                // You can only kick a ball you can reach. Without this a
                // player in a passing state rewrote the ball's velocity
                // from anywhere on the pitch — see `KICKABLE_DISTANCE`.
                // Ownership isn't required: a first-time pass off a ball
                // arriving at your feet is legal, and the ball snaps to
                // its owner anyway, so reach covers both.
                {
                    let ball_pos = field.ball.position;
                    let in_reach = field
                        .get_player(pass_event_model.from_player_id)
                        .map(|p| {
                            let d = p.position - ball_pos;
                            d.x * d.x + d.y * d.y
                                <= crate::r#match::engine::ball::ball::KICKABLE_DISTANCE
                                    * crate::r#match::engine::ball::ball::KICKABLE_DISTANCE
                        })
                        .unwrap_or(false);
                    if !in_reach {
                        #[cfg(feature = "match-logs")]
                        crate::r#match::engine::ball::ball::ownership::reception_diag::OUT_OF_REACH
                            .fetch_add(1, Ordering::Relaxed);
                        return remaining_events;
                    }
                }
                // Build (but don't yet fire) the offside snapshot. The
                // resolver fires only when the receiver becomes active —
                // touches the ball, claims, or actively challenges. This
                // matches how offside is judged in real football: the
                // referee waits to see if the receiver gets involved
                // before raising the flag.
                //
                // Set-piece restarts (goal kicks, corners, throw-ins)
                // are exempt by rule. Normal goalkeeper open-play passes
                // are NOT exempt — the previous "exempt all GK passes"
                // shortcut hid genuine offsides on long GK clearances.
                let restart_origin = field.ball.pass_origin_restart;
                let snapshot = if restart_origin.is_offside_exempt() {
                    None
                } else {
                    Self::build_offside_snapshot(
                        pass_event_model.from_player_id,
                        pass_event_model.to_player_id,
                        restart_origin,
                        field,
                        context.current_tick(),
                    )
                };

                if match_data.is_tracking_events() {
                    match_data.add_pass_event(
                        context.total_match_time,
                        pass_event_model.from_player_id,
                        pass_event_model.to_player_id,
                    );
                }
                let passer_id = pass_event_model.from_player_id;
                let pass_target = pass_event_model.pass_target;
                let (passer_position, passer_side, passer_team) = field
                    .get_player(passer_id)
                    .map(|p| (p.position, p.side, p.team_id))
                    .unwrap_or((Vector3::zeros(), None, 0));
                // ── Pressure producer ─────────────────────────────────
                // Opponents within the pressing radius (~8u) of the
                // passer at emit time are credited with a pressure
                // event. Per-player cooldown prevents a single
                // shadowing defender from generating one pressure per
                // tick across a multi-second press burst. Pressers are
                // stashed on the ball so the interception handler can
                // promote them to "successful pressure" if the same
                // pass is intercepted within the response window.
                Self::credit_pressures_on_pass(
                    passer_id,
                    passer_position,
                    passer_team,
                    field,
                    context,
                );
                // A delivery the crossing model built declares itself. Any
                // OTHER pass is still geometry-classified at emit time —
                // passer in a wide channel delivering toward the box — so
                // it picks up the dedicated crossing-skill error term (a
                // low-crossing player's cross sails harder off-target than
                // their open-play pass would).
                let declared_cross = pass_event_model.cross_type;
                let was_cross = declared_cross.is_some()
                    || passer_side.is_some_and(|side| {
                        Self::is_cross_attempt(passer_position, pass_target, side, context)
                    });
                Self::handle_pass_to_event(pass_event_model, field, context, was_cross);
                // A modelled cross arms the open-play aerial contest: the
                // delivery is aimed at a patch of the box, so it must be
                // contestable rather than reserved for one named receiver
                // (see `resolve_cross_contest`). Ground deliveries resolve
                // through the normal reception path and stay disarmed.
                field.ball.cross_contest_resolved =
                    !declared_cross.map(|ct| ct.is_lofted()).unwrap_or(false);
                field.ball.pending_cross_type = declared_cross;
                // Tag the ball with the passer for pass-accuracy
                // accounting. Lives for a short window (150 ticks)
                // and is cleared on opponent touch — see ball.rs
                // `pending_pass_passer` docs.
                #[cfg(feature = "match-logs")]
                {
                    use crate::r#match::engine::ball::ball::ownership::reception_diag;
                    reception_diag::EMITTED.fetch_add(1, Ordering::Relaxed);
                    if field.ball.pending_pass_passer.is_some() {
                        reception_diag::SUPERSEDED.fetch_add(1, Ordering::Relaxed);
                    }
                }
                field.ball.pending_pass_passer = Some(passer_id);
                field.ball.pending_pass_set_tick = context.current_tick();
                // Fresh pass, fresh interception attempt.
                field.ball.intercept_rolled = false;
                field.ball.pending_pass_origin = Some(passer_position);
                field.ball.pending_pass_target = Some(pass_target);
                field.ball.pending_pass_was_cross = was_cross;
                if was_cross {
                    if let Some(passer) = field.get_player_mut(passer_id) {
                        passer.statistics.add_cross_attempt();
                    }
                }
                field.ball.offside_snapshot = snapshot;
                // After the kick, restart context decays back to OpenPlay —
                // EXCEPT a corner cross, which keeps its Corner origin through
                // the flight. That keeps the pushed-up centre-backs in
                // AttackingCorner (is_team_attacking_corner) and lets the
                // discrete corner aerial contest fire on the in-flight
                // delivery (engine `resolve_corner_contest`); the origin
                // decays to OpenPlay on the next touch / interception as usual.
                if !(was_cross && field.ball.pass_origin_restart == PassOriginRestart::Corner) {
                    field.ball.pass_origin_restart = PassOriginRestart::OpenPlay;
                }
            }
            PlayerEvent::ClaimBall(player_id) => {
                Self::record_team_possession_if_switch(player_id, field, context);
                Self::handle_claim_ball_event(player_id, field, context);
            }
            PlayerEvent::MoveBall(player_id, ball_velocity) => {
                Self::handle_move_ball_event(player_id, ball_velocity, field);
            }
            PlayerEvent::GainBall(player_id) => {
                Self::handle_gain_ball_event(player_id, field, context);
            }
            PlayerEvent::Shoot(shoot_event_model) => {
                // Capture field dimensions up-front so the log block
                // below (which runs under &Player borrow) doesn't try
                // to re-borrow field.size. Feature-gated so the capture
                // compiles away when logs are off.
                #[cfg(feature = "match-logs")]
                let field_w = field.size.width as f32;
                #[cfg(feature = "match-logs")]
                let field_h = field.size.height as f32;

                // Record shot at team level for cooldown. Log reason +
                // source context only when `match-logs` feature is
                // enabled (see `core/src/match/logs.rs`).
                if let Some(player) = field.get_player(shoot_event_model.from_player_id) {
                    let team_id = player.team_id;
                    let tick = context.current_tick();
                    #[cfg(feature = "match-logs")]
                    {
                        let pos = player.position;
                        let goal_dist = if let Some(side) = player.side {
                            let goal_x = match side {
                                PlayerSide::Left => field_w,
                                PlayerSide::Right => 0.0,
                            };
                            let goal_y = field_h / 2.0;
                            ((pos.x - goal_x).powi(2) + (pos.y - goal_y).powi(2)).sqrt()
                        } else {
                            0.0
                        };
                        let pos_tag =
                            match player.tactical_position.current_position.position_group() {
                                PlayerFieldPositionGroup::Goalkeeper => "GK",
                                PlayerFieldPositionGroup::Defender => "DEF",
                                PlayerFieldPositionGroup::Midfielder => "MID",
                                PlayerFieldPositionGroup::Forward => "FWD",
                            };
                        match_log_info!(
                            "SHOT team={} pos={} player={} state={} reason={} dist={:.1} tick={}",
                            team_id,
                            pos_tag,
                            shoot_event_model.from_player_id,
                            player.state,
                            shoot_event_model.reason,
                            goal_dist,
                            tick
                        );
                    }
                    context.coach_for_team_mut(team_id).record_shot(tick);
                }
                if let Some(player) = field.get_player_mut(shoot_event_model.from_player_id) {
                    player.pending_shot_reason = None;
                }
                // ── Key-pass linking ──────────────────────────────────
                // The shooter just received a completed pass within the
                // key-pass window — the passer earns a key pass. Reads
                // `last_completed_pass_*` (populated by
                // `credit_completed_pass` when the receiver claimed the
                // ball) rather than `pending_pass_*` (which the
                // completion path already cleared). Without the
                // separate snapshot the receive-then-shoot sequence
                // would silently drop the assist link.
                //
                // To avoid double-credit when the same passer also
                // appears as `pending_pass_passer` (can happen if the
                // shooter intercepted a NEW outgoing pass and shot in
                // the same tick), null `last_completed_pass_*` after
                // crediting.
                //
                // Capture the direct assister BEFORE clearing — the xG
                // buildup distribution in `handle_shoot_event` needs it
                // to exclude the assister from buildup credit. Without
                // this snapshot, the clear below races the buildup read.
                let shooter_id = shoot_event_model.from_player_id;
                let now_tick = context.current_tick();
                // Opta's key-pass definition has NO time window: it is simply
                // the last pass before a shot. A 3s cap excluded every
                // receive-turn-carry-shoot sequence — the engine's
                // dominant chance shape — so only ~1.2 key passes per
                // TEAM were tagged against a real ~9, starving the
                // creative credit that keeps forwards and midfielders in
                // their rating bands. 12s still bounds it to one
                // possession phase; the possession-change nulls below do
                // the real work.
                const KEY_PASS_WINDOW_TICKS: u64 = 1200;
                let shooter_team = field.get_player(shooter_id).map(|p| p.team_id);
                let mut direct_assister_id: Option<u32> = None;
                #[cfg(feature = "match-logs")]
                {
                    key_pass_diag::SHOTS.fetch_add(1, Ordering::Relaxed);
                    if field.ball.last_completed_pass_receiver_id.is_none() {
                        key_pass_diag::NO_LINK.fetch_add(1, Ordering::Relaxed);
                    }
                    // How the shooter came by the ball. The key-pass
                    // counter says a pass DIDN'T feed this shot; this
                    // says what did instead.
                    key_pass_diag::SUPPLY[field.ball.possession_source.index()]
                        .fetch_add(1, Ordering::Relaxed);
                }
                if let (Some(passer_id), Some(receiver_id)) = (
                    field.ball.last_completed_pass_passer_id,
                    field.ball.last_completed_pass_receiver_id,
                ) {
                    let elapsed = now_tick.saturating_sub(field.ball.last_completed_pass_tick);
                    let in_window = elapsed <= KEY_PASS_WINDOW_TICKS;
                    let receiver_is_shooter = receiver_id == shooter_id;
                    let passer_not_shooter = passer_id != shooter_id;
                    let passer_team = field.get_player(passer_id).map(|p| p.team_id);
                    let same_team = passer_team.is_some() && passer_team == shooter_team;
                    if in_window && receiver_is_shooter && passer_not_shooter && same_team {
                        if let Some(passer) = field.get_player_mut(passer_id) {
                            passer.statistics.add_key_pass();
                        }
                        direct_assister_id = Some(passer_id);
                        #[cfg(feature = "match-logs")]
                        key_pass_diag::CREDITED.fetch_add(1, Ordering::Relaxed);
                    }
                    #[cfg(feature = "match-logs")]
                    {
                        if !receiver_is_shooter {
                            key_pass_diag::WRONG_RECEIVER.fetch_add(1, Ordering::Relaxed);
                        } else if !in_window {
                            key_pass_diag::STALE_WINDOW.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    // Single-credit guarantee: even if the shot resolves
                    // into a goal that fires another event, the link is
                    // gone after the first read. Only clear on a SUCCESSFUL
                    // credit — clearing after a failed test meant one
                    // rebound or blocked effort destroyed the link for the
                    // genuine next attempt in the same phase.
                    if direct_assister_id.is_some() {
                        field.ball.last_completed_pass_passer_id = None;
                        field.ball.last_completed_pass_receiver_id = None;
                    }
                }
                // ── Error leading to shot/goal ────────────────────────
                // If an opponent gave the ball away within the response
                // window and the shooter is on the opposite team, the
                // giver is charged with the error. Persist to
                // `pending_error_to_shot_player_id` so the goal handler
                // can also charge `errors_leading_to_goal` if the shot
                // converts.
                const ERROR_TO_SHOT_WINDOW_TICKS: u64 = 600; // ~6s
                let shooter_team = field.get_player(shooter_id).map(|p| p.team_id);
                if let (Some(giver_id), Some(giver_team), shooter_team) = (
                    field.ball.last_giveaway_player_id,
                    field.ball.last_giveaway_team_id,
                    shooter_team,
                ) {
                    let in_window = now_tick.saturating_sub(field.ball.last_giveaway_tick)
                        <= ERROR_TO_SHOT_WINDOW_TICKS;
                    if in_window && Some(giver_team) != shooter_team {
                        if let Some(giver) = field.get_player_mut(giver_id) {
                            giver.statistics.add_error_leading_to_shot();
                        }
                        field.ball.pending_error_to_shot_player_id = Some(giver_id);
                    } else {
                        field.ball.pending_error_to_shot_player_id = None;
                    }
                } else {
                    field.ball.pending_error_to_shot_player_id = None;
                }
                // ── Failed claim leading to shot ──────────────────────
                // A keeper who has just flapped a cross is charged the
                // moment the attacking side gets a shot away from it.
                // Held on the ball (not promoted into the error lanes)
                // so the rating bills this incident exactly once.
                const FAILED_CLAIM_WINDOW_TICKS: u64 = 600; // ~6s
                if let Some(gk_id) = field.ball.pending_failed_claim_gk_id {
                    #[cfg(feature = "match-logs")]
                    gk_claim_diag::FLAP_SHOTS_SEEN.fetch_add(1, Ordering::Relaxed);
                    let in_window = now_tick.saturating_sub(field.ball.pending_failed_claim_tick)
                        <= FAILED_CLAIM_WINDOW_TICKS;
                    let gk_team = field.get_player(gk_id).map(|p| p.team_id);
                    if in_window && gk_team.is_some() && gk_team != shooter_team {
                        if !field.ball.pending_failed_claim_charged {
                            if let Some(gk) = field.get_player_mut(gk_id) {
                                gk.statistics.note_gk_failed_claim_to_shot();
                            }
                            #[cfg(feature = "match-logs")]
                            gk_claim_diag::FLAP_TO_SHOT.fetch_add(1, Ordering::Relaxed);
                            field.ball.pending_failed_claim_charged = true;
                        }
                    } else {
                        #[cfg(feature = "match-logs")]
                        gk_claim_diag::FLAP_DROPPED.fetch_add(1, Ordering::Relaxed);
                        field.ball.pending_failed_claim_gk_id = None;
                        field.ball.pending_failed_claim_charged = false;
                    }
                }
                Self::handle_shoot_event(shoot_event_model, field, context, direct_assister_id);
            }
            PlayerEvent::CaughtBall(player_id) => {
                Self::handle_caught_ball_event(player_id, field, context);
            }
            PlayerEvent::ParriedBall(player_id) => {
                Self::handle_parried_ball_event(player_id, field);
            }
            PlayerEvent::MovePlayer(player_id, position) => {
                Self::handle_move_player_event(player_id, position, field);
            }
            PlayerEvent::Leap(player_id, apex) => {
                Self::handle_leap_event(player_id, apex, field);
            }
            PlayerEvent::TakeBall(player_id) => {
                Self::handle_take_ball_event(player_id, field);
            }
            PlayerEvent::ClearBall(velocity) => {
                Self::handle_clear_ball_event(velocity, field, context);
            }
            PlayerEvent::RequestBallReceive(player_id) => {
                Self::handle_request_ball_receive(player_id, field, context);
            }
            PlayerEvent::CommitFoul(fouler_id, severity) => {
                Self::handle_commit_foul_event(fouler_id, severity, field, context);
            }
            PlayerEvent::Offside(player_id, position) => {
                Self::handle_offside_event(player_id, position, field);
            }
            _ => {} // Ignore unsupported events
        }

        remaining_events
    }

    fn handle_goal_event(
        player_id: u32,
        is_auto_goal: bool,
        field: &mut MatchField,
        context: &mut MatchContext,
    ) {
        let scorer_team_id = field.get_player(player_id).map(|p| p.team_id);
        // Stale scorer id (sent off / subbed between shot and goal
        // resolution) — no goal recorded for the player, but the score
        // line and ball-state cleanup downstream still need to run.
        let Some(player) = field.get_player_mut(player_id) else {
            return;
        };

        player
            .statistics
            .add_goal(context.total_match_time, is_auto_goal);

        // Goal stands → credit on-target to the real scorer. Own goals
        // aren't counted as an on-target shot for the defender that
        // deflected the ball.
        if !is_auto_goal {
            player.memory.credit_shot_on_target();
            #[cfg(feature = "match-logs")]
            {
                save_accounting_stats::ON_TARGET_FROM_GOAL.fetch_add(1, Ordering::Relaxed);
                let band =
                    time_band_diag::band_for_minute(sc::minute_from_ms(context.total_match_time));
                time_band_diag::GOALS_BY_BAND[band].fetch_add(1, Ordering::Relaxed);
            }
        }

        // Credit the conceding goalkeeper's `shots_faced` so the rating
        // helper has the right denominator for save percentage. Auto-
        // goals (own-goals) don't count — those aren't shots the GK got
        // beaten by, they're defensive errors. While we're here, debit
        // the GK's xg_prevented by the shot's xG: a 0.7 xG goal beats
        // the keeper, but a 0.05 xG worldie barely dents the keeper's
        // rating because there's nothing to prevent.
        if !is_auto_goal {
            let shot_xg = field.ball.last_shot_xgot;
            if let Some(scoring_team) = scorer_team_id {
                let conceding_gk_id = field
                    .players
                    .iter()
                    .find(|p| {
                        p.team_id != scoring_team
                            && p.tactical_position.current_position.position_group()
                                == PlayerFieldPositionGroup::Goalkeeper
                    })
                    .map(|p| p.id);
                if let Some(gk_id) = conceding_gk_id {
                    if let Some(gk) = field.get_player_mut(gk_id) {
                        gk.statistics.note_shot_faced(shot_xg, false);
                    }
                }
            }
            // Promote a pending error-to-shot into an error-to-goal.
            // Layer the own-box-extra zone counter when the original
            // giveaway happened inside the giver's own box — the rating
            // helper applies an extra penalty on top of the base
            // errors_leading_to_goal hit.
            if let Some(giver_id) = field.ball.pending_error_to_shot_player_id {
                let was_own_box = field.ball.last_giveaway_was_own_box;
                if let Some(giver) = field.get_player_mut(giver_id) {
                    giver.statistics.add_error_leading_to_goal();
                    if was_own_box {
                        giver.statistics.note_error_to_goal_own_box();
                    }
                }
            }
            // Promote a pending failed claim into a failed-claim-to-goal.
            // NB deliberately NOT also stamping `errors_leading_to_goal`:
            // the rating de-dups nested mistake counters, and billing one
            // flapped cross through both lanes is the triple-counting bug
            // that once buried a one-conceded keeper in the disaster band.
            if let Some(gk_id) = field.ball.pending_failed_claim_gk_id {
                let conceding_side = scorer_team_id
                    .and_then(|t| field.get_player(gk_id).map(|p| p.team_id != t))
                    .unwrap_or(false);
                if conceding_side {
                    if let Some(gk) = field.get_player_mut(gk_id) {
                        gk.statistics.note_gk_failed_claim_to_goal();
                    }
                }
            }
        }
        field.ball.clear_shot_metadata();
        field.ball.pending_error_to_shot_player_id = None;
        field.ball.pending_failed_claim_gk_id = None;
        field.ball.pending_failed_claim_charged = false;
        field.ball.clear_giveaway();

        context.score.add_goal_detail(GoalDetail {
            player_id,
            stat_type: MatchStatisticType::Goal,
            is_auto_goal,
            time: context.total_match_time,
        });
        context.record_stoppage_time(30_000);

        // Psychology nudges: scorer gains confidence; conceding GK takes
        // a hit. Own goals route the confidence penalty to the auto-
        // scorer instead of any reward path. Team momentum swings to
        // the scoring side. Cheap — single hashmap lookups.
        let tick = context.current_tick();
        if !is_auto_goal {
            context
                .psychology
                .record_positive(player_id, PositiveEvent::Goal, tick);
        } else {
            context
                .psychology
                .record_negative(player_id, NegativeEvent::Mistake, tick);
        }
        if let Some(scoring_team) = scorer_team_id {
            let is_home_scoring = scoring_team == field.home_team_id;
            // Goal momentum delta for the scoring side. Auto-goals also
            // shift momentum toward the side that profited.
            let scoring_side_is_home = if is_auto_goal {
                !is_home_scoring
            } else {
                is_home_scoring
            };
            context
                .psychology
                .record_team_event(scoring_side_is_home, 0.35, tick);
            // Conceding side's keeper takes a confidence hit.
            let conceding_gk_id = field
                .players
                .iter()
                .find(|p| {
                    p.team_id != scoring_team
                        && p.tactical_position.current_position.position_group()
                            == PlayerFieldPositionGroup::Goalkeeper
                })
                .map(|p| p.id);
            if let Some(gk_id) = conceding_gk_id {
                context
                    .psychology
                    .record_negative(gk_id, NegativeEvent::GoalConceded, tick);
            }
        }

        // The restart used to happen here — formation rebuild, ball back on
        // the centre spot — as well as in the ball-event handler and again in
        // `handle_goal_reset`, three times on the tick the ball crossed the
        // line. It now belongs solely to the end of the post-goal window (see
        // `GoalCelebration`), because putting the ball on the centre spot the
        // instant it goes in is exactly what left it looking like it stopped
        // dead on the goal line. `Ball::enter_net` has already dropped the
        // ownership and pass state this used to clear by hand.
    }

    fn handle_assist_event(player_id: u32, field: &mut MatchField, context: &mut MatchContext) {
        let player = field.get_player_mut(player_id).unwrap();

        context.score.add_goal_detail(GoalDetail {
            player_id,
            stat_type: MatchStatisticType::Assist,
            time: context.total_match_time,
            is_auto_goal: false,
        });

        player.statistics.add_assist(context.total_match_time);
    }

    fn handle_ball_collision_event(player_id: u32, field: &mut MatchField) {
        let player = field.get_player_mut(player_id).unwrap();

        if player.skills.technical.first_touch > 10.0 {
            // Handle player gaining control of the ball after collision
        }
    }

    fn handle_tackling_ball_event(player_id: u32, field: &mut MatchField, context: &MatchContext) {
        let ball_pos = field.ball.position;
        // Capture the carrier (= dispossessed player) BEFORE
        // secure_ball_for nulls them out — the tackle handler treats
        // the previous owner as the dribbler losing a duel.
        let dispossessed_id = field.ball.current_owner.filter(|&id| id != player_id);
        if let Some(player) = field.get_player_mut(player_id) {
            player.statistics.tackles += 1;
            if let Some(zone) = Self::zone_for_player(player, ball_pos, context) {
                player.statistics.note_tackle_zone(zone);
            }
        }
        // Failed-dribble producer: the carrier lost a 1-v-1 to the
        // tackler. Real engine doesn't separately resolve dribble
        // duels in live play, so we use the tackle outcome as a
        // proxy. This also gives the rating helper a non-zero
        // attempted_dribbles signal for fullbacks / wingers who get
        // tackled while attacking.
        if let Some(dispossessed) = dispossessed_id {
            if let Some(p) = field.get_player_mut(dispossessed) {
                p.statistics.add_failed_dribble();
            }
        }
        Self::secure_ball_for(player_id, field);
        if let Some(team_id) = field.get_player(player_id).map(|p| p.team_id) {
            field.ball.note_possession(team_id);
        }
        // The one acquisition that never passes through the ball-event
        // dispatcher, so it labels itself.
        field
            .ball
            .note_possession_source(player_id, PossessionSource::Tackle);
    }

    fn handle_ball_owner_change_event(player_id: u32, field: &mut MatchField) {
        Self::secure_ball_for(player_id, field);
    }

    /// Classify a position from a player's defensive perspective.
    /// Returns None if the player has no side assigned (shouldn't
    /// happen on field, but stays defensive).
    pub(crate) fn zone_for_player(
        player: &MatchPlayer,
        position: Vector3<f32>,
        context: &MatchContext,
    ) -> Option<MatchZone> {
        let side = player.side?;
        let field_w = context.field_size.width as f32;
        let is_home = side == PlayerSide::Left;
        let own = context.penalty_area(is_home);
        let opp = context.penalty_area(!is_home);
        Some(MatchZone::classify(position, side, field_w, own, opp))
    }

    /// Pressing radius — defenders within this distance of the
    /// passer at pass-emit time are credited with a pressure event.
    /// 8u in field coordinates (~1m at the engine's 1u≈0.125m scale —
    /// close-range pressing distance).
    pub(crate) const PRESSURE_RADIUS_SQ: f32 = 8.0 * 8.0;
    /// Per-player cooldown between pressure events (in ticks ≈ 100/s).
    /// 200 ticks ≈ 2 seconds. Without this a single defender shadowing
    /// the carrier across a buildup phase would post 50+ pressures per
    /// match. Real high-pressing midfielders sit around 25-40 per 90.
    pub(crate) const PRESSURE_COOLDOWN_TICKS: u64 = 200;

    /// Distribute a shot's xG across the chain participants. Returns a
    /// list of `(player_id, chain_credit, buildup_credit)` ready to
    /// apply to player statistics — `buildup_credit` is `None` for the
    /// shooter and the direct assister (already rewarded elsewhere).
    ///
    /// Pure function over `recent_passers`-style input so it can be
    /// unit-tested without spinning up a `MatchField`. Both pools split
    /// across the unique participants of the chain so a single player
    /// appearing multiple times in the ring buffer can't accumulate the
    /// per-event credit linearly.
    pub(crate) fn distribute_xg_credit<I>(
        recent_passers: I,
        shooter_id: u32,
        direct_assister_id: Option<u32>,
        xg: f32,
    ) -> Vec<(u32, f32, Option<f32>)>
    where
        I: IntoIterator<Item = u32>,
    {
        if xg <= 0.0 {
            return Vec::new();
        }
        const CHAIN_FRACTION: f32 = 0.30;
        const BUILDUP_FRACTION: f32 = 0.20;
        let chain_pool = xg * CHAIN_FRACTION;
        let buildup_pool = xg * BUILDUP_FRACTION;

        let mut unique_ids: Vec<u32> = Vec::new();
        for pid in recent_passers {
            if !unique_ids.contains(&pid) {
                unique_ids.push(pid);
            }
        }
        if unique_ids.is_empty() {
            return Vec::new();
        }

        let buildup_count = unique_ids
            .iter()
            .filter(|&&pid| pid != shooter_id && Some(pid) != direct_assister_id)
            .count();
        let chain_per = chain_pool / unique_ids.len() as f32;
        let buildup_per = if buildup_count > 0 {
            buildup_pool / buildup_count as f32
        } else {
            0.0
        };

        unique_ids
            .into_iter()
            .map(|pid| {
                let buildup = if pid != shooter_id && Some(pid) != direct_assister_id {
                    Some(buildup_per)
                } else {
                    None
                };
                (pid, chain_per, buildup)
            })
            .collect()
    }

    /// Producer: at pass-emit time, scan opponents within the pressing
    /// radius and credit them with a `pressures` event (subject to the
    /// per-player cooldown). The set of pressers is also stamped on
    /// the ball so a downstream `Intercepted` can promote them to
    /// `successful_pressures` when the close-range presence forced the
    /// turnover.
    pub(crate) fn credit_pressures_on_pass(
        passer_id: u32,
        passer_position: Vector3<f32>,
        passer_team: u32,
        field: &mut MatchField,
        context: &MatchContext,
    ) {
        let now = context.current_tick();
        // Reset the press snapshot — each new pass starts a clean slate.
        field.ball.pressers_at_pass = [0; 4];
        field.ball.pressers_at_pass_count = 0;

        // Two-pass borrow dance: collect ids first (immutable iter),
        // then bump stats (mutable get_player_mut per id).
        let mut presser_ids: [u32; 4] = [0; 4];
        let mut count: u8 = 0;
        for player in field.players.iter() {
            if player.id == passer_id || player.team_id == passer_team {
                continue;
            }
            if player.is_sent_off {
                continue;
            }
            let dx = player.position.x - passer_position.x;
            let dy = player.position.y - passer_position.y;
            if dx * dx + dy * dy > Self::PRESSURE_RADIUS_SQ {
                continue;
            }
            if (count as usize) < presser_ids.len() {
                presser_ids[count as usize] = player.id;
                count += 1;
            }
        }
        // Only stash pressers who actually received `add_pressure()`.
        // Cooldown shadows are excluded so a downstream interception
        // cannot promote them to `successful_pressures` — that would
        // produce `successful_pressures > pressures` and let coach
        // press-success rates exceed 1.0.
        let mut credited_ids: [u32; 4] = [0; 4];
        let mut credited_count: u8 = 0;
        for &id in presser_ids.iter().take(count as usize) {
            if let Some(presser) = field.get_player_mut(id) {
                let in_cooldown =
                    now.saturating_sub(presser.last_pressure_tick) < Self::PRESSURE_COOLDOWN_TICKS;
                if !in_cooldown {
                    presser.statistics.add_pressure();
                    presser.last_pressure_tick = now;
                    if (credited_count as usize) < credited_ids.len() {
                        credited_ids[credited_count as usize] = id;
                        credited_count += 1;
                    }
                }
            }
        }
        field.ball.pressers_at_pass = credited_ids;
        field.ball.pressers_at_pass_count = credited_count;
    }

    /// Single completion path: increment `passes_completed`, classify
    /// progressive / cross / box-entry, then clear the pass-window
    /// metadata. Both the "intended receiver claimed cleanly"
    /// (`BallEvent::PassCompleted`) flow and the "ball claimed by a
    /// teammate during the pass window" fallback in
    /// `handle_claim_ball_event` route through here, so the two
    /// branches produce identical stats and never double-credit.
    pub(crate) fn credit_completed_pass(
        receiver_id: u32,
        passer_id: u32,
        field: &mut MatchField,
        context: &MatchContext,
    ) {
        let origin = field.ball.pending_pass_origin;
        let target = field.ball.pending_pass_target;
        let was_cross = field.ball.pending_pass_was_cross;
        if let Some(passer) = field.get_player_mut(passer_id) {
            passer.statistics.passes_completed =
                passer.statistics.passes_completed.saturating_add(1);
        }
        // First-touch quality producer: a receiver with weak technique
        // / first_touch can fluff the reception (miscontrol) or kill
        // it with a heavy touch — particularly when defenders are
        // close. The carrier keeps the ball here, so this records a
        // stat-only signal; the rating helper drags accordingly via
        // `touch_quality`. High-skill receivers under no pressure
        // virtually never trip this.
        Self::maybe_record_first_touch_loss(receiver_id, field, context);
        if let (Some(origin), Some(target)) = (origin, target) {
            Self::classify_completed_pass(
                passer_id,
                receiver_id,
                origin,
                target,
                was_cross,
                field,
                context,
            );
        }
        // Snapshot the completed pass for the shot-after-pass key-pass
        // linker. MUST happen before clearing the pending-pass envelope
        // — the pending fields stop tracking the pass at this point,
        // but the key-pass window extends past completion until the
        // receiver shoots, passes again, or loses the ball.
        field
            .ball
            .record_completed_pass(passer_id, receiver_id, context.current_tick());
        field.ball.clear_pending_pass_metadata();
        // Pass completed — the pressers at emit time did NOT force a
        // turnover, so don't credit a successful pressure. Clear the
        // snapshot so it can't be reused by an unrelated future event.
        field.ball.pressers_at_pass_count = 0;
    }

    /// Deterministic first-touch quality roll. When a pass is claimed,
    /// the receiver's `first_touch / technique / composure` composite
    /// gates whether the touch was clean, a heavy touch (kept the ball
    /// but lost tempo), or — rarely, for very weak receivers under
    /// heavy pressure — a miscontrol. The carrier always keeps the ball
    /// here; the engine's physics handle the kept-but-loose case
    /// implicitly through follow-on positions, while this records the
    /// stat-line evidence the rating helper consumes via
    /// `touch_quality`. Pure deterministic seeding (receiver id × tick)
    /// keeps replays reproducible.
    pub(crate) fn maybe_record_first_touch_loss(
        receiver_id: u32,
        field: &mut MatchField,
        context: &MatchContext,
    ) {
        let (composite01, pressure_count, _) = match field.get_player(receiver_id) {
            Some(p) => {
                let s = &p.skills;
                // Decisions folded in (FM-parity skill pass, 2026-07):
                // control is not only feet — choosing the wrong touch
                // (taking a ball into traffic, controlling into a
                // closing lane) is the classic low-Decisions giveaway,
                // and without it a weak-mentals winger with a good
                // first touch never leaked control at all.
                let composite = s.technical.first_touch * 0.35
                    + s.technical.technique * 0.15
                    + s.mental.composure * 0.20
                    + s.mental.anticipation * 0.15
                    + s.mental.decisions * 0.15;
                let comp01 = (composite / 20.0).clamp(0.0, 1.0);
                let pos = p.position;
                let team = p.team_id;
                let opp_close = field
                    .players
                    .iter()
                    .filter(|q| q.team_id != team)
                    .filter(|q| (q.position - pos).magnitude() < 6.0)
                    .count();
                (comp01, opp_close, ())
            }
            None => return,
        };

        let bad_touch_prob = Self::first_touch_loss_probability(composite01, pressure_count);
        if bad_touch_prob < 0.02 {
            return;
        }

        // Deterministic per-event roll. Mixing tick × receiver_id × a
        // wide odd multiplier scatters consecutive ticks evenly across
        // the [0,1) interval — match-engine call sites can repeat the
        // same receive across re-simulation and get identical stats.
        let tick = context.current_tick();
        let seed = (receiver_id as u64).wrapping_mul(0x9E3779B97F4A7C15)
            ^ tick.wrapping_mul(0xBF58476D1CE4E5B9);
        let roll = ((seed >> 11) & 0xFFFFFF) as f32 / 16_777_215.0;
        if roll >= bad_touch_prob {
            return;
        }

        // Severity split — the worst ~25% of bad-touch rolls AND a
        // genuinely weak composite (≤ 0.45) get registered as a
        // miscontrol; the remainder is a heavy touch. The rating's
        // touch_quality drag treats miscontrols as roughly 2× the
        // weight of heavy touches, so the split shapes how visible
        // the failure is in the post-match line.
        let severe_threshold = bad_touch_prob * 0.25;
        if let Some(p) = field.get_player_mut(receiver_id) {
            if roll < severe_threshold && composite01 < 0.45 {
                p.statistics.add_miscontrol();
            } else {
                p.statistics.add_heavy_touch();
            }
        }
    }

    /// Pure first-touch-loss probability formula. Returns a value in
    /// `[0.0, 0.30]`. Two lanes:
    ///
    ///   * **Pressured** — the original curve, amplified by close
    ///     opponents: a weak receiver under a press visibly leaks.
    ///   * **Unforced** — pressure-independent (FM-parity skill pass,
    ///     2026-07): a low-composite player fluffs touches even
    ///     unmarked. Without this lane a weak-mentals player at a
    ///     dominant club in a weak league (rarely pressured) never
    ///     registered a single loss of control all season, so nothing
    ///     in his skill profile reached the rating.
    ///
    /// Tuned so (2 close opponents): composite ≈ 0.25 → ~13%,
    /// ≈ 0.50 → ~5%, ≈ 0.75 → ~1%. The `^2.5` / `^2` curves keep
    /// elite reception virtually immune in both lanes.
    /// Extracted as a pure function so the gradient is unit-testable
    /// without standing up a `MatchField`.
    #[inline]
    pub(crate) fn first_touch_loss_probability(composite01: f32, pressure_count: usize) -> f32 {
        let c = composite01.clamp(0.0, 1.0);
        let pressure_mult = 1.0 + (pressure_count.min(4) as f32) * 0.6;
        let pressured = (1.0 - c).powf(2.5) * 0.10 * pressure_mult;
        let unforced = (1.0 - c).powi(2) * 0.05;
        (pressured + unforced).clamp(0.0, 0.30)
    }

    /// Classify a completed pass and bump the passer's per-zone /
    /// chance-creation counters. Called from `credit_completed_pass`
    /// after `passes_completed` has been credited. Public to the
    /// crate so the ball-event dispatcher can stay thin.
    pub(crate) fn classify_completed_pass(
        passer_id: u32,
        _receiver_id: u32,
        origin: Vector3<f32>,
        target: Vector3<f32>,
        was_cross: bool,
        field: &mut MatchField,
        context: &MatchContext,
    ) {
        use crate::r#match::engine::zones::{LateralLane, MatchZone};

        let passer_side = match field.get_player(passer_id).and_then(|p| p.side) {
            Some(s) => s,
            None => return,
        };
        let field_w = context.field_size.width as f32;
        let field_h = context.field_size.height as f32;
        let is_home = passer_side == PlayerSide::Left;
        let opp_box = context.penalty_area(!is_home);

        // Forward progress along the attacking axis.
        let forward_progress = passer_side.forward_delta(origin.x, target.x);
        let target_in_final_third =
            passer_side.attacking_progress_x(target.x, field_w) >= 2.0 / 3.0;
        let origin_in_final_third =
            passer_side.attacking_progress_x(origin.x, field_w) >= 2.0 / 3.0;

        // Progressive pass: ≥25u outside final third, ≥12u inside.
        let progressive_threshold = if origin_in_final_third { 12.0 } else { 25.0 };
        let is_progressive = forward_progress >= progressive_threshold;

        let target_in_box = opp_box.contains(&target);
        let own_box = context.penalty_area(is_home);
        let endpoint_zone = MatchZone::classify(target, passer_side, field_w, own_box, opp_box);

        // Lateral-lane classification (origin & target).
        let origin_lane = LateralLane::classify(origin.y, field_h);
        let target_lane = LateralLane::classify(target.y, field_h);
        let from_half_space = matches!(
            origin_lane,
            LateralLane::HalfSpaceLeft | LateralLane::HalfSpaceRight
        );
        let from_central = matches!(origin_lane, LateralLane::CentralLane);
        // Switch of play: long lateral diagonal across the pitch's
        // y-axis. Tight definition — origin lane and target lane must
        // BOTH be wide-or-half-space AND on opposite sides of centre.
        let switch_of_play = {
            let opposite_sides = matches!(
                (origin_lane, target_lane),
                (LateralLane::WideLeft, LateralLane::WideRight)
                    | (LateralLane::WideLeft, LateralLane::HalfSpaceRight)
                    | (LateralLane::HalfSpaceLeft, LateralLane::WideRight)
                    | (LateralLane::HalfSpaceLeft, LateralLane::HalfSpaceRight)
                    | (LateralLane::WideRight, LateralLane::WideLeft)
                    | (LateralLane::WideRight, LateralLane::HalfSpaceLeft)
                    | (LateralLane::HalfSpaceRight, LateralLane::WideLeft)
                    | (LateralLane::HalfSpaceRight, LateralLane::HalfSpaceLeft)
            );
            // Switch must also cover meaningful y-distance — ≥ 35% of
            // pitch height. Filters out short outside-channel passes
            // that happen to clip lane boundaries.
            let lateral_distance = (target.y - origin.y).abs();
            let lateral_threshold = field_h * 0.35;
            opposite_sides && lateral_distance >= lateral_threshold
        };

        if let Some(passer) = field.get_player_mut(passer_id) {
            if is_progressive {
                passer.statistics.add_progressive_pass();
                if target_in_final_third && !origin_in_final_third {
                    passer.statistics.note_progressive_pass_into_final_third();
                }
            }
            if target_in_box {
                passer.statistics.add_pass_into_box();
                // Origin-lane breakdown for box-entry passes —
                // half-space and central balls beat compact defences
                // and earn a small extra credit on top of the regular
                // box-entry line.
                if from_half_space {
                    passer.statistics.note_half_space_pass_into_box();
                } else if from_central {
                    passer.statistics.note_central_pass_into_box();
                }
            }
            if was_cross {
                passer.statistics.add_cross_completed();
            }
            if switch_of_play {
                passer.statistics.note_switch_of_play();
            }
            // Future GK command-zone hookup point: an aerial cross that
            // ends inside the GK's own box could be a "claim candidate"
            // — leaving the marker for Slice C.
            let _ = endpoint_zone;
        }
    }

    /// Did this pass start in a wide channel and target the
    /// opposition box? That's the cross-attempt signal — wing-play
    /// service into the danger area, regardless of whether the
    /// pass was tagged as a "cross" by the strategy layer.
    fn is_cross_attempt(
        passer_position: Vector3<f32>,
        target: Vector3<f32>,
        side: PlayerSide,
        context: &MatchContext,
    ) -> bool {
        use crate::r#match::engine::zones::LateralLane;
        let field_h = context.field_size.height as f32;
        if !LateralLane::classify(passer_position.y, field_h).is_wide() {
            return false;
        }
        // Pass must travel forward and end in or near the opp box.
        let opp_box = match side {
            PlayerSide::Left => context.penalty_area(false),
            PlayerSide::Right => context.penalty_area(true),
        };
        if opp_box.contains(&target) {
            return true;
        }
        // Within ~10u of the box — wide-channel deliveries that
        // arrive at the edge of the area still count as crosses.
        let dx = (target.x - opp_box.min.x.max(0.0))
            .min(opp_box.max.x - target.x)
            .max(0.0);
        let dy = (target.y - opp_box.min.y)
            .min(opp_box.max.y - target.y)
            .max(0.0);
        let inside_x = target.x >= opp_box.min.x && target.x <= opp_box.max.x;
        let inside_y = target.y >= opp_box.min.y && target.y <= opp_box.max.y;
        if inside_x && inside_y {
            return true;
        }
        // Approximate "within 10u of box edge" using axis distances —
        // good enough for a binary classification.
        let edge_dist = if inside_x {
            dy
        } else if inside_y {
            dx
        } else {
            (dx * dx + dy * dy).sqrt()
        };
        edge_dist <= 10.0
    }

    fn handle_pass_to_event(
        event_model: PassingEventContext,
        field: &mut MatchField,
        context: &MatchContext,
        was_cross: bool,
    ) {
        let total_match_time_ms = context.total_match_time;
        let rng = &context.rng;
        let minute = sc::minute_from_ms(total_match_time_ms);

        // Only increment attempts here. `passes_completed` is bumped when
        // the intended receiver actually claims the ball (see
        // `handle_claim_ball_event`). Previously both were bumped at
        // emit-time, so the metric was "passes emitted" — 99% accuracy
        // regardless of whether the ball reached the target. Real-football
        // pass accuracy sits around 85%; the inflated metric hid the
        // fact that our possession loop wasn't losing the ball often
        // enough via wayward passes.
        if let Some(passer) = field.get_player_mut(event_model.from_player_id) {
            passer.statistics.passes_attempted += 1;
        }

        // Extract player skills and condition
        let player = field.get_player(event_model.from_player_id).unwrap();
        let passer_position = player.position;
        let passer_side = player.side;
        let skills = PassSkills::from_player(player, minute);

        // Calculate overall quality for accuracy - affected by condition
        let overall_quality = skills.overall_quality();

        // Calculate ideal target position — lead the pass toward receiver's movement
        let receiver_pos = event_model.pass_target;
        let receiver_velocity = field
            .get_player(event_model.to_player_id)
            .map(|p| p.velocity)
            .unwrap_or(Vector3::zeros());

        // Lead pass: target where the receiver will be when the ball
        // arrives. Ground-pass flight time is `distance * 0.015 * 1.15`
        // ÷ friction ≈ 70-95 ticks across short/medium passes. Elite
        // passers predict this correctly; poor passers under- or
        // over-estimate, so the lead itself is skill-dependent.
        //
        // Flight-time estimate: we aim `lead_ticks` ahead along the
        // receiver's current velocity, where `lead_ticks` = a fraction
        // of true flight time determined by vision + passing quality.
        //
        // A delivery aimed at a SPACE (a cross into a zone of the box)
        // skips this entirely: the aim point already encodes where the
        // runner is going, so leading it a second time would push the ball
        // through the back of the six-yard box.
        let pass_distance_est = (receiver_pos - passer_position).magnitude();
        let flight_time_est = (pass_distance_est * 0.85).clamp(25.0, 95.0);
        // Vision = how well we anticipate the receiver's run.
        // Passing = technical precision on the pass itself.
        let anticipation = (skills.vision * 0.6 + skills.passing * 0.4).clamp(0.0, 1.0);
        // Skilled passers lead fully; poor passers lead most of the way.
        // Widened base from 0.40 → 0.60 after the pass-accuracy audit
        // showed average-anticipation passers were under-leading by
        // enough that receivers arrived at the ball 3-6u short — just
        // outside the tightest receiver claim windows, enough passes
        // failed to push team accuracy to 72% instead of the 85% target.
        let lead_fraction = 0.60 + anticipation * 0.35; // 0.60..0.95
        let lead_ticks = if event_model.target_is_space {
            0.0
        } else {
            flight_time_est * lead_fraction
        };
        let ideal_target = receiver_pos + receiver_velocity * lead_ticks;

        // Always use passer's position as pass origin — ball position may lag behind
        let pass_origin = passer_position;
        let ideal_pass_vector = ideal_target - pass_origin;
        let horizontal_distance = Self::calculate_horizontal_distance(&ideal_pass_vector);

        // Skill-based targeting error. The accuracy_factor is the
        // overall passing × concentration product; everything below
        // shapes how much positional error gets applied to the actual
        // pass target.
        let accuracy_factor = (overall_quality * skills.concentration).clamp(0.0, 1.0);

        // Distance-based error: longer passes have more positional error.
        // Curve steepened so 20u passes are near-perfect for skilled
        // players, while 200u passes lose significant accuracy.
        let distance_error_factor = (horizontal_distance / 250.0).clamp(0.1, 1.8);

        // Max error: shaped on `(1 - accuracy_factor)^1.5` rather than
        // on `(1 - precision)` linearly. The previous linear-in-precision
        // formula collapsed the spread between *average* and *poor*
        // passers (both landing ~5.5u) — a poor passer was no worse
        // than an average one, which let low-skill players hide inside
        // possession-dominant teams. The exponent restores skill spread:
        //
        //   accuracy_factor 0.81 (elite):    max_err ≈ 1.0u
        //   accuracy_factor 0.50 (good):     max_err ≈ 3.5u
        //   accuracy_factor 0.30 (average):  max_err ≈ 5.5u
        //   accuracy_factor 0.10 (poor):     max_err ≈ 8.0u
        //
        // Distance scaling still applies on top, so long passes by
        // poor passers stretch toward ~14u of error — close to but
        // still inside the 40u receiver claim radius, so completion
        // rates don't collapse, but the offset is large enough that
        // first-touch rolls trigger more often on receive (poor pass
        // arrival → harder control → more miscontrols → lower rating).
        let shortfall = (1.0 - accuracy_factor).clamp(0.0, 1.0);
        let base_max_position_error = (0.3 + shortfall.powf(1.5) * 9.0) * distance_error_factor;

        // Crossing-specific error multiplier. Crosses are a distinct
        // skill from open-play passing — a low-crossing winger sails
        // the ball over the target far more than their open-play passes
        // suggest. The PassEvaluator's `crossing > 0.4` gate decides
        // whether the player ENTERS the crossing state, but until now
        // execution used the same passing/technique/vision error
        // budget as any other pass. Result: a skill=1 winger somehow
        // hit crosses as accurately as their flat passes — unrealistic.
        //
        // The multiplier scales with `1 - crossing_skill`, so:
        //   crossing 0.95 (elite): ×1.0  (no extra error)
        //   crossing 0.50 (good):  ×1.25
        //   crossing 0.20 (poor):  ×1.80
        //   crossing 0.05 (lowest): ×2.10
        // Applied multiplicatively on top of the base error, so even
        // an elite passer's cross is somewhat looser than their
        // open-play pass at the same distance, which matches real
        // football where crosses are a notoriously low-percentage skill.
        let max_position_error = if was_cross {
            let crossing_shortfall = (1.0 - skills.crossing).clamp(0.0, 1.0);
            // A CORNER is not an open-play cross. It is struck from a
            // stationary ball, unpressured, with the taker free to pick
            // their spot — so it should not inherit the full open-play
            // crossing penalty. It did, and because that penalty scales
            // with skill it hit youth football hardest: only 11.6% of
            // youth corner deliveries reached a centre-back's heading
            // range vs 19.3% at senior level (targeting was fine at both
            // — 99% were aimed AT a CB), which is most of why youth
            // defenders scored 4% of goals against a real ~10%.
            let cross_multiplier = if field.ball.pass_origin_restart == PassOriginRestart::Corner {
                1.0 + crossing_shortfall.powf(1.2) * 0.35
            } else {
                1.0 + crossing_shortfall.powf(1.2) * 1.2
            };
            // Not every cross is equally hard. A pull-back from the byline
            // is a short square ball; a whipped delivery that has to drop
            // inside the six-yard box ahead of a keeper is the hardest ball
            // in the game. `CrossType::difficulty` carries that spread so
            // the delivery MIX matters, not just the crosser's rating.
            let type_difficulty = event_model
                .cross_type
                .map(|ct| ct.difficulty())
                .unwrap_or(1.0);
            base_max_position_error * cross_multiplier * type_difficulty
        } else if event_model.reason == "DEF_COUNTER_ATTACK" {
            // Counter-attack outlet: the passer just won the ball and
            // had a clear moment to look up. Real football's "the
            // unpressed long ball" is a high-completion pass — the
            // passer isn't being closed down because the opposition
            // hasn't reorganised yet. Tightened 0.50 → 0.35 because
            // the 0.50 multiplier still produced 30-40% turnover on
            // long counter balls (the long-distance distance_error
            // multiplied through), and the resulting failed counters
            // burned weak team's possession faster than the safe-pass
            // baseline they were replacing. A real counter-attack
            // outlet completes ~75% of the time; 0.35 brings the
            // engine closer to that band without crossing into elite-
            // through-ball territory.
            base_max_position_error * 0.35
        } else {
            base_max_position_error
        };

        // Add random targeting error
        let mut target_error_x = rng.jitter(0.0, max_position_error);
        let mut target_error_y = rng.jitter(0.0, max_position_error);

        // Miskick chance — heavily gated by technique. Elite technique
        // basically never miskicks; poor technique (≤6) fires wild ~8%
        // of the time (5th-power curve concentrates miskicks among
        // genuinely unskilled players). Previous cubic curve produced
        // ~5% even for average players, which muddied the skill signal.
        let miskick_chance = (1.0 - skills.technique).powi(5) * 0.25;
        if rng.random_range(0.0f32..1.0) < miskick_chance {
            target_error_x += rng.random_range(-8.0f32..8.0);
            target_error_y += rng.random_range(-8.0f32..8.0);
        }

        // Calculate actual target with error
        let mut actual_target = Vector3::new(
            ideal_target.x + target_error_x,
            ideal_target.y + target_error_y,
            0.0,
        );

        // SAFETY: keep pass target away from BOTH goals.
        // Own goal: obvious — no one passes into their own net.
        // Opposing goal: a pass aimed within a few yards of the opponent
        // goal line with even a small error scoots straight into the net
        // as an unowned ball → `check_goal` credits the passer → logged
        // as a "goal" that never involved a Shoot event. Logs showed
        // 10-15 of these per team per match, which was the primary
        // source of 20+ goal scorelines. Passes should always land in
        // playable space — clearances and shots have their own explicit
        // paths for getting the ball across a goal line.
        {
            use crate::r#match::PlayerSide;
            let field_width = field.size.width as f32;
            let goal_safety_margin = 20.0;
            match passer_side {
                Some(PlayerSide::Left) => {
                    // Own goal at x ≈ 0; opposing goal at x ≈ field_width.
                    if actual_target.x < goal_safety_margin {
                        actual_target.x = passer_position.x.max(goal_safety_margin);
                    }
                    if actual_target.x > field_width - goal_safety_margin {
                        actual_target.x = field_width - goal_safety_margin;
                    }
                }
                Some(PlayerSide::Right) => {
                    // Own goal at x ≈ field_width; opposing goal at x ≈ 0.
                    if actual_target.x > field_width - goal_safety_margin {
                        actual_target.x = passer_position.x.min(field_width - goal_safety_margin);
                    }
                    if actual_target.x < goal_safety_margin {
                        actual_target.x = goal_safety_margin;
                    }
                }
                _ => {}
            }
        }

        // ── SWING ────────────────────────────────────────────────────
        // An aerial cross is whipped, and whether it swings in toward
        // goal or away from it is the difference between a ball the
        // keeper can come for and one he cannot. As with a curled shot,
        // the delivery is aimed OFF the target and bent back onto it —
        // the ball genuinely arcs across the box, but the crosser still
        // hits what he meant to hit.
        //
        // Sign is unbiased (in-swinger vs out-swinger), so this adds
        // shape rather than a systematic drift.
        let swing_units = match event_model.cross_type {
            Some(ct) if ct.is_lofted() => {
                let sign: f32 = if rng.random_range(0.0f32..1.0) < 0.5 {
                    -1.0
                } else {
                    1.0
                };
                // Up to ~2 m of swing for an elite crosser; a poor one
                // barely bends it because he cannot get across the ball.
                sign * rng.random_range(0.25f32..1.0) * 16.0 * skills.crossing
            }
            _ => 0.0,
        };
        let swing_perp = if swing_units.abs() > f32::EPSILON {
            Vector3::new(
                actual_target.x - pass_origin.x,
                actual_target.y - pass_origin.y,
                0.0,
            )
            .try_normalize(1.0e-4)
            .map(|d| Vector3::new(-d.y, d.x, 0.0))
        } else {
            None
        };
        if let Some(perp) = swing_perp {
            actual_target -= perp * swing_units;
        }

        let actual_pass_vector = actual_target - pass_origin;
        let actual_horizontal_distance = Self::calculate_horizontal_distance(&actual_pass_vector);

        // Calculate pass force with power variation
        // Bad players hit passes with inconsistent power
        let power_consistency = 1.0 + (skills.technique * skills.stamina * 0.1);
        let power_variation_range = (1.0 - overall_quality) * 0.35;
        let power_variation = rng.jitter(power_consistency, power_variation_range);
        let adjusted_force = event_model.pass_force * power_variation;

        // Rolling delivery speed — the pace a ground pass has to leave the
        // foot at to arrive. The trajectory solver below either uses this
        // as-is (driven / ground balls) or replaces it with a ballistic
        // solution (lofted balls).
        let rolling_velocity =
            Self::calculate_horizontal_velocity(&actual_pass_vector, adjusted_force);

        // Determine trajectory type based on context, not just distance
        let passer = field.get_player_mut(event_model.from_player_id).unwrap();
        let passer_team_id = passer.team_id;
        let passer_is_goalkeeper = passer.tactical_position.current_position.is_goalkeeper();

        // A modelled cross carries its own shape. Everything else is
        // chosen from lane traffic and distance. Keeping these separate is
        // the whole point — see the `TrajectoryType::Cross` doc.
        let trajectory_type = match event_model.cross_type {
            Some(ct) => TrajectoryType::Cross(ct),
            None => Self::select_trajectory_type_contextual(
                actual_horizontal_distance,
                &skills,
                rng,
                &passer_position,
                &actual_target,
                passer_team_id,
                &field.players,
            ),
        };

        // Goalkeeper long kicks must always be high arcs (goal kicks from
        // penalty area). Threshold rescaled with the bands above: 60u was
        // 7.5 m, so a keeper rolling the ball to a centre-back's feet was
        // being told to launch it. 240u = 30 m is a genuine long kick.
        //
        // The keeper's old `max_z * 1.5` height bonus is gone with the rest
        // of the height model — the solver now derives the height from the
        // range, and if the range is beyond what the gravity constant
        // permits it drives the ball instead (see `calculate_pass_velocity`).
        let trajectory_type = if passer_is_goalkeeper && actual_horizontal_distance > 240.0 {
            TrajectoryType::HighArc
        } else {
            trajectory_type
        };

        // Solve the whole launch vector at once — see
        // `calculate_pass_velocity` for why the z-component can never be
        // computed independently of the horizontal one.
        let mut final_velocity = Self::calculate_pass_velocity(
            &actual_pass_vector,
            &rolling_velocity,
            trajectory_type,
            &skills,
            rng,
        );

        // Check for NaN or infinity
        if final_velocity.x.is_nan()
            || final_velocity.y.is_nan()
            || final_velocity.z.is_nan()
            || final_velocity.x.is_infinite()
            || final_velocity.y.is_infinite()
            || final_velocity.z.is_infinite()
        {
            // Fallback to a safe default velocity. The `z` is an apex in
            // metres solved into a launch speed, not a raw number — the
            // 0.3 this used to carry meant a 46 m apex.
            let safe_direction = actual_pass_vector.normalize();
            final_velocity = Vector3::new(
                safe_direction.x * 1.5,
                safe_direction.y * 1.5,
                Ball::launch_speed_for_apex(1.5),
            );
        }

        // Safety net only — `calculate_pass_velocity` is budgeted by
        // construction, so this should never bind. It clamps the
        // HORIZONTAL speed alone, along its own direction, and leaves the
        // height out of it.
        //
        // Never clamp the magnitude of a ballistic vector. That is what
        // broke the pass model: `v * (CAP / |v|)` shrinks the forward
        // component along with the vertical one, and because range goes as
        // v² a modest-looking ×0.42 rescale left 18% of the distance. A
        // goalkeeper's 50 m kick came out as a 4 m-high pop that landed
        // 1.8 m from his own boot, which then fed a pick-up → kick →
        // pick-up cycle he never escaped.
        let horizontal_sq =
            final_velocity.x * final_velocity.x + final_velocity.y * final_velocity.y;
        if horizontal_sq > Self::MAX_PASS_VELOCITY * Self::MAX_PASS_VELOCITY {
            let scale = Self::MAX_PASS_VELOCITY / horizontal_sq.sqrt();
            final_velocity.x *= scale;
            final_velocity.y *= scale;
        }

        // Apply ball physics
        field.ball.velocity = final_velocity;

        // Rotation that produces the swing solved for above. With a
        // constant lateral acceleration `a` over flight time `T` the
        // deflection is `d = ½aT²`, and the Magnus term contributes
        // `a_perp = C·ω_z·|v|`, so `ω_z = 2·d·|v| / (C·dist²)`. Solving it
        // from the deflection rather than picking a spin figure is what
        // guarantees the ball still arrives where the crosser aimed.
        // Solved, then CLAMPED — `dist²` in the denominator means a short
        // delivery asked to swing asks for a physically impossible
        // rotation, and an unclamped Magnus force is a catapult. See
        // `SpinModel::clamped`.
        field.ball.spin = if swing_units.abs() > f32::EPSILON {
            let speed =
                (final_velocity.x * final_velocity.x + final_velocity.y * final_velocity.y).sqrt();
            if speed > 0.05 && actual_horizontal_distance > 1.0 {
                SpinModel::clamped(Vector3::new(
                    0.0,
                    0.0,
                    2.0 * swing_units * speed
                        / (SpinModel::MAGNUS_COEFF
                            * actual_horizontal_distance
                            * actual_horizontal_distance),
                ))
            } else {
                Vector3::zeros()
            }
        } else {
            Vector3::zeros()
        };

        // Remember who put this ball into play, from where. Two consumers:
        // the degenerate self-recollect cycle (`blocked_recollect_player`)
        // and, when the releaser is a keeper, Law 12's second-touch bar.
        field.ball.note_release(
            event_model.from_player_id,
            passer_position,
            context.current_tick(),
        );
        // A deliberate kick by a field player arms the back-pass
        // prohibition against his own keeper. A keeper's own distribution
        // does not — that is the second-touch rule above, and
        // `is_backpass_to` excludes him from his own bar anyway.
        if !passer_is_goalkeeper {
            field.ball.note_deliberate_kick(
                event_model.from_player_id,
                passer_team_id,
                context.current_tick(),
            );
        }

        // Record the passer in recent passers history before clearing
        // ownership. The team and tick ride along so the assist resolver
        // can tell a teammate's pass from an opponent's and a live chain
        // from a stale one.
        field.ball.record_passer(
            event_model.from_player_id,
            passer_team_id,
            context.current_tick(),
        );

        field.ball.previous_owner = field.ball.current_owner;
        field.ball.current_owner = None;
        field.ball.pass_target_player_id = Some(event_model.to_player_id);

        // In-flight protection window — keeps the intended receiver's
        // 40u claim privilege live until the ball actually arrives.
        //
        // Why: ground passes decay at 0.985/tick (1.5% per tick). Solving
        // x(t) = v0(1 - 0.985^t)/0.015 with v0 = d × 0.015 × overshoot
        // gives an actual arrival time of ~135-170 ticks for a 30u pass,
        // ~190-260 for a 60u, ~260-340 for a 100u+. Prior window of
        // 40/60/80 ticks covered only the first 25-30% of the flight —
        // once it expired, anyone within 5u of the CURRENT (still mid-
        // flight) ball position could claim, and the chasing defender
        // typically beat the intended receiver to it. Population pass
        // accuracy sat at ~50% vs real-football ~85% for that reason.
        // Sizing the window to match the physical flight time keeps the
        // receiver's priority through arrival; defenders still get the
        // ball when the receiver doesn't actually reach the landing spot.
        // Size the window to the ball's ACTUAL flight rather than to a
        // distance band. Solving `d(T) = v0 (1 - k^T) / (1 - k)` for T
        // gives the tick the ball reaches its target; the fixed
        // 120/180/240 bands were hand-fitted to the old friction and
        // under-covered the longer, more realistic flights the fixed
        // `GROUND_FRICTION` produces. An expired window is not harmless:
        // it ends the receiver's exclusive claim mid-pass and lets the
        // ball be taken in open play — the same failure the shot path
        // had (see the `ticks_to_goal` window in `handle_shoot_event`).
        // 1.35 margin absorbs the receiver's last stride.
        //
        // Both legs read the DELIVERED velocity. They used to read the
        // pre-clamp horizontal speed, which after the magnitude rescale
        // was up to 5× what the ball actually left with — so the window
        // was sized for a 50 m flight the ball never made, and stayed
        // armed for four seconds over a ball lying still. Nothing but the
        // intended receiver may claim inside the window (see
        // `Ball::process_ownership`), so an oversized one is not a
        // harmless over-estimate: it freezes the ball in place while the
        // designated chaser jogs alongside it unable to touch it.
        let flight_protection = {
            const MARGIN: f32 = 1.35;
            let vh = (final_velocity.x * final_velocity.x + final_velocity.y * final_velocity.y)
                .sqrt()
                .max(0.01);
            // Airborne leg: a ball launched at `vz` is back on the deck
            // after `2·vz/g` ticks.
            let air_ticks = 2.0 * final_velocity.z.max(0.0) / GRAVITY_PER_TICK;
            // Rolling leg: whatever distance the arc did not cover.
            let rolling_distance = (actual_horizontal_distance - vh * air_ticks).max(0.0);
            let decay_fraction = rolling_distance * GROUND_FRICTION / vh;
            let roll_ticks = if decay_fraction >= 0.95 {
                // Struck barely hard enough to arrive — hold it open.
                400.0
            } else {
                (1.0 - decay_fraction).ln() / (1.0 - GROUND_FRICTION).ln()
            };
            (((air_ticks + roll_ticks) * MARGIN) as usize).clamp(60, 400)
        };
        field.ball.flags.in_flight_state = flight_protection;
    }

    fn calculate_horizontal_distance(ball_pass_vector: &Vector3<f32>) -> f32 {
        (ball_pass_vector.x * ball_pass_vector.x + ball_pass_vector.y * ball_pass_vector.y).sqrt()
    }

    fn calculate_horizontal_velocity(
        ball_pass_vector: &Vector3<f32>,
        pass_force: f32,
    ) -> Vector3<f32> {
        let horizontal_direction =
            Vector3::new(ball_pass_vector.x, ball_pass_vector.y, 0.0).normalize();
        let distance = (ball_pass_vector.x * ball_pass_vector.x
            + ball_pass_vector.y * ball_pass_vector.y)
            .sqrt();

        // Weight the pass to ARRIVE, at a speed its man can take.
        //
        // The old model inverted the friction curve to make the ball's
        // total roll distance `distance * overshoot`, with an overshoot
        // table of 1.79-2.57 — every pass deliberately struck 79-157%
        // too far so it would still be moving when it got there. That was
        // a workaround for friction ~3.7× stronger than real grass (see
        // `GROUND_FRICTION`): with the ball dying that fast, a pass
        // weighted to its man arrived at walking pace, so the code hit it
        // 5-12 m past him instead. It was invisible while a reception was
        // credited at 100u — the pass was "complete" long before the ball
        // ran through — and it is a large part of why an honest pass
        // accuracy measured 34% against a real 85%.
        //
        // With realistic friction the ball keeps its pace on its own, so
        // the strike speed can just be the speed a real player would use.
        // Real ground passes leave the foot at ~8 m/s for a short ball
        // and ~25 m/s for a raking one; at 1u = 0.125 m and 10 ms a tick
        // that band is 0.64-2.0 u/tick.
        const BASE_SPEED: f32 = 0.55;
        const SPEED_PER_UNIT: f32 = 0.0028;
        let delivery_speed = (BASE_SPEED + distance * SPEED_PER_UNIT).clamp(0.50, 2.20);

        // Floor: the ball stops after `v / friction` units, so a pass
        // struck too softly for its distance never arrives at all. Keep a
        // margin over the bare minimum.
        let min_arriving_speed = distance * GROUND_FRICTION * 1.25;
        let needed_velocity = delivery_speed.max(min_arriving_speed);

        // pass_force (0.3-2.0) modulates: skilled players weight the pass better
        // Normalize to 0.90-1.1 range so it fine-tunes rather than drives the physics
        let skill_modifier = 0.90 + (pass_force.clamp(0.3, 2.0) - 0.3) * 0.12;

        horizontal_direction * (needed_velocity * skill_modifier)
    }

    /// Select trajectory type based on obstacles in the passing lane
    /// Simple rule: obstacles present → cross (lofted), no obstacles → ground pass
    fn select_trajectory_type_contextual(
        horizontal_distance: f32,
        skills: &PassSkills,
        rng: &MatchRng,
        from_position: &Vector3<f32>,
        to_position: &Vector3<f32>,
        passer_team_id: u32,
        players: &[MatchPlayer],
    ) -> TrajectoryType {
        // Traffic in the passing lane.
        let traffic = Self::scan_passing_lane(from_position, to_position, passer_team_id, players);
        // A body in the lane only forces the ball into the air if it is
        // CLOSE — the man actually pressing you, whom you have to lift it
        // over. A defender standing 15 m down the lane is played around or
        // driven past; nobody chips a 20 m ball to feet because someone is
        // loitering halfway.
        //
        // The old rule was "any opponent anywhere along the lane → never
        // play it low", which was survivable only because the distance
        // bands underneath it were four times too small. Rescaling those
        // bands turned it into a machine for lofting ordinary midfield
        // passes: pressed players started chipping every ball they played.
        //
        // 40u = 5 m: within one defensive stride of the passer.
        const LIFT_TRIGGER_DISTANCE: f32 = 40.0;
        let obstacles_in_lane = if traffic.nearest <= LIFT_TRIGGER_DISTANCE {
            traffic.count
        } else {
            0
        };

        // Calculate decision quality - determines how well player chooses trajectory
        let decision_quality = skills.decision_quality();
        let vision_quality = skills.vision;

        // Better decision makers make more appropriate choices
        let skill_influenced_random = {
            let pure_random = rng.random_range(0.0..1.0);
            let randomness_factor = 1.0 - (decision_quality * 0.6);
            let skill_bias = decision_quality * 0.3;
            (pure_random * randomness_factor + skill_bias).clamp(0.0, 1.0)
        };

        // Distance categories. The field is 840u = 105m, so **1 unit =
        // 0.125 m** — the old bands were annotated "1 unit ≈ 0.5m" and were
        // therefore four times too small. Under them, "short" ended at 3.8 m
        // and everything past 15 m fell into the always-lofted bucket, which
        // is essentially every pass in a football match: the engine chose an
        // aerial trajectory for a routine 20 m ball to feet.
        //
        // Rescaled to what the labels actually claim.
        let is_short = horizontal_distance <= 120.0; // ≤15m — quick one-touch
        let is_medium = horizontal_distance > 120.0 && horizontal_distance <= 240.0; // 15-30m
        let is_long = horizontal_distance > 240.0 && horizontal_distance <= 480.0; // 30-60m
        // > 480 units = very long (60m+)

        // Passes should be lofted based on BOTH obstacles AND distance.
        // In real football, passes over 25m often leave the ground even without obstacles.
        if obstacles_in_lane == 0 {
            // CLEAR LANE — trajectory based on distance
            if is_short {
                // Short passes — ground
                TrajectoryType::Ground
            } else if is_medium {
                // Medium passes — mostly ground, some driven
                if skill_influenced_random < 0.60 {
                    TrajectoryType::Ground
                } else {
                    TrajectoryType::LowDriven
                }
            } else if is_long {
                // Long passes (30-60 m) down an empty lane — driven along
                // the floor or clipped over the top. Both are real; the
                // driven ball is the more common of the two.
                if skill_influenced_random < 0.45 {
                    TrajectoryType::LowDriven
                } else if skill_influenced_random < 0.70 {
                    TrajectoryType::MediumArc
                } else {
                    TrajectoryType::HighArc
                }
            } else {
                // Very long (60 m+) — the switch of play and the goal kick.
                // These want to be in the air. Past ~44 m the solver cannot
                // buy the arc at this gravity and drives it instead, which
                // is handled for us (`calculate_pass_velocity`).
                if skill_influenced_random < 0.30 {
                    TrajectoryType::MediumArc
                } else {
                    TrajectoryType::HighArc
                }
            }
        } else {
            // OBSTACLES PRESENT - Use lofted passes (crosses).
            // Smooth crossing / vision gates via sigmoid pivots at the
            // old `> 0.7` (=14/20) thresholds — `skills.*` are already
            // normalised 0-1 so we re-scale to raw 1-20 for the curve.
            let many_obstacles = obstacles_in_lane >= 2;
            let crossing_p = SkillCurve::new(skills.crossing * 20.0, 14.0, 0.6).probability();
            let vision_p = SkillCurve::new(vision_quality * 20.0, 14.0, 0.6).probability();
            let has_good_crossing = rng.random_range(0.0..1.0) < crossing_p;

            if is_short {
                // Pressed, playing short. A dink over the man in front is
                // the picture-book answer but it is a hard, low-percentage
                // skill — most players just slide it past him instead, and
                // let the interception model decide whether that works.
                // "NEVER low" here was the other half of the always-lofted
                // problem.
                let chip_p = vision_p * 0.45;
                if skill_influenced_random < chip_p {
                    TrajectoryType::Chip // Smart chip over defender, scaled by vision
                } else {
                    TrajectoryType::Ground
                }
            } else if is_medium {
                // Medium pass with obstacles - cross with arc (NEVER low)
                if many_obstacles {
                    // Multiple obstacles - higher arc needed
                    if skill_influenced_random < 0.70 {
                        TrajectoryType::HighArc // 70% high cross
                    } else {
                        TrajectoryType::MediumArc // 30% medium cross
                    }
                } else {
                    // One obstacle - medium/high arc to clear it
                    if skill_influenced_random < 0.70 {
                        TrajectoryType::MediumArc // 70% medium cross
                    } else {
                        TrajectoryType::HighArc // 30% high cross
                    }
                }
            } else if is_long {
                // Long pass with obstacles - definitely need arc
                if many_obstacles || has_good_crossing {
                    // Multiple obstacles or good crosser - high arc
                    if skill_influenced_random < 0.75 {
                        TrajectoryType::HighArc // 75% high cross
                    } else {
                        TrajectoryType::MediumArc // 25% medium cross
                    }
                } else {
                    // One obstacle - medium/high arc mix
                    if skill_influenced_random < 0.60 {
                        TrajectoryType::MediumArc // 60% medium cross
                    } else {
                        TrajectoryType::HighArc // 40% high cross
                    }
                }
            } else {
                // Very long pass with obstacles - high cross
                let long_pass_ability = skills.long_shots * skills.vision * skills.crossing;
                if long_pass_ability > 0.7 {
                    // Elite crosser - controlled high arc
                    if skill_influenced_random < 0.80 {
                        TrajectoryType::HighArc // 80% high cross
                    } else {
                        TrajectoryType::MediumArc // 20% medium cross
                    }
                } else {
                    // Average crosser - mostly high arc
                    if skill_influenced_random < 0.70 {
                        TrajectoryType::HighArc // 70% high cross
                    } else {
                        TrajectoryType::MediumArc // 30% medium cross
                    }
                }
            }
        }
    }

    /// Opponents standing in the passing lane, and how far down the lane
    /// the closest of them is.
    ///
    /// The distance matters as much as the count: whether a pass has to
    /// leave the ground depends on whether the body is at the passer's
    /// feet or thirty yards away. See the `LIFT_TRIGGER_DISTANCE` note in
    /// `select_trajectory_type_contextual`.
    fn scan_passing_lane(
        from_position: &Vector3<f32>,
        to_position: &Vector3<f32>,
        passer_team_id: u32,
        players: &[MatchPlayer],
    ) -> LaneTraffic {
        const LANE_WIDTH: f32 = 12.0; // Width of the passing lane corridor (accounts for player reach and movement)

        let lane = *to_position - *from_position;
        let pass_distance = lane.magnitude();
        let Some(pass_direction) = lane.try_normalize(1e-4) else {
            return LaneTraffic::default();
        };

        let mut traffic = LaneTraffic::default();
        for player in players {
            // Only count opponents
            if player.team_id == passer_team_id {
                continue;
            }

            // Vector from passer to player
            let to_player = player.position - *from_position;

            // Project player onto pass line to find closest point
            let projection_length = to_player.dot(&pass_direction);

            // Player must be between passer and target
            if projection_length < 0.0 || projection_length > pass_distance {
                continue;
            }

            // Calculate perpendicular distance to pass line
            let projection_point = *from_position + pass_direction * projection_length;
            let perpendicular_distance = (player.position - projection_point).magnitude();

            // Player is an obstacle if within lane width
            if perpendicular_distance < LANE_WIDTH {
                traffic.count += 1;
                traffic.nearest = traffic.nearest.min(projection_length);
            }
        }
        traffic
    }

    /// How high, in metres, each delivery style is meant to peak.
    ///
    /// Apex is the knob a footballer actually turns — "keep it down",
    /// "clip it over him", "hang it up to the back post". It also happens
    /// to be the only one that is expressible here: the horizontal axis is
    /// in game units and the vertical one in metres (see
    /// `GRAVITY_PER_TICK`), so a launch ANGLE has no meaning, while an
    /// apex and a range between them pin the trajectory exactly.
    ///
    /// It replaces the old `calculate_max_z_velocity` height CAP, which was
    /// a knob on top of an independently-computed z velocity — one degree
    /// of freedom too many for a trajectory that is already determined, and
    /// the one that produced 68-84° launches on ordinary passes.
    ///
    /// Longer deliveries climb higher, as they must to hang up long enough
    /// to arrive.
    fn target_apex(trajectory_type: TrajectoryType, distance_units: f32) -> f32 {
        let metres = distance_units * 0.125;
        match trajectory_type {
            // Rolling — a couple of centimetres, so the ball rides bumps
            // rather than grinding along at exactly z = 0.
            TrajectoryType::Ground => 0.02,
            // Skimming drive: off the deck, never above the shin. Not
            // required to arrive on the fly — it lands early and runs on.
            TrajectoryType::LowDriven => (0.20 + metres * 0.010).clamp(0.20, 0.9),
            // The workhorse lofted ball — over a leg, onto his chest.
            TrajectoryType::MediumArc => (1.0 + metres * 0.075).clamp(1.0, 5.0),
            // Cross, switch, goal kick. A 50 m kick peaks at 10 m and hangs
            // ~2.9 s, which is what a real one does.
            TrajectoryType::HighArc => (2.0 + metres * 0.16).clamp(2.0, 12.0),
            // Dinked over a defender standing right in front of you.
            TrajectoryType::Chip => (1.2 + metres * 0.12).clamp(1.2, 3.5),
            // The crossing model owns its own shape — a floated ball to
            // the back post and a driven ball across the six-yard line
            // are different deliveries, not one delivery with different
            // aim.
            TrajectoryType::Cross(ct) => ct.apex_metres(distance_units),
        }
    }

    /// Horizontal speed ceiling for a pass, in u/tick. 3.2 u/tick = 40 m/s
    /// — elite piledriver territory, never beaten by a real outfield pass,
    /// and the same figure `MAX_SHOT_VELOCITY` uses.
    ///
    /// Purely horizontal. With the vertical axis in metres the two
    /// components no longer share a scale, so there is no meaningful
    /// magnitude to cap — and, more to the point, no way for a magnitude
    /// cap to eat the forward pace again. It also sits under the ball's
    /// global `MAX_VELOCITY` (8.0), whose clamp DOES rescale.
    const MAX_PASS_VELOCITY: f32 = 3.2;

    /// Solve the full launch vector for a pass.
    ///
    /// The horizontal and vertical components of a kick are not independent
    /// — they are two projections of one trajectory — so they have to be
    /// produced together. The previous code computed them separately (a
    /// rolling speed here, a "z velocity" there), discovered the pair broke
    /// the speed cap, and rescaled the vector to fit. That is the bug behind
    /// every symptom in the pass model: rescaling preserves the launch
    /// ANGLE, and since range goes as v² it destroys the range. With z
    /// dominating the vector 5-10×, the surviving horizontal component was a
    /// fifth of what was intended and passes went up instead of forward.
    ///
    /// Two regimes, and neither one ever rescales:
    ///
    /// * **Ballistic** (lofted styles). Pick the apex, which fixes the
    ///   vertical speed and therefore the hang time; the horizontal speed
    ///   is then just `range / hang`, so the ball LANDS on its target.
    /// * **Driven** (flat styles, and any loft the horizontal cap cannot
    ///   buy). Keep the calibrated rolling delivery speed untouched and add
    ///   only the small lift the style asks for. The ball skims and rolls
    ///   in — which is what a driven pass does.
    ///
    /// Solving apex-first rather than angle-first is what makes this work
    /// across the two unit systems on the ball's axes (units horizontally,
    /// metres vertically — see `GRAVITY_PER_TICK`): both halves are derived
    /// from quantities that are meaningful on their own axis, and never
    /// from a ratio between them.
    fn calculate_pass_velocity(
        pass_vector: &Vector3<f32>,
        rolling_velocity: &Vector3<f32>,
        trajectory_type: TrajectoryType,
        skills: &PassSkills,
        rng: &MatchRng,
    ) -> Vector3<f32> {
        let distance = Self::calculate_horizontal_distance(pass_vector);
        let direction = Vector3::new(pass_vector.x, pass_vector.y, 0.0)
            .try_normalize(1e-4)
            .unwrap_or_else(|| Vector3::new(1.0, 0.0, 0.0));

        // Weighting error. A mis-hit delivery is under- or over-struck —
        // it drops short or sails long — so the error belongs on the
        // SPEED, not on the height. Loft is the harder skill, so the
        // lofted styles read the crossing / long-ball composite.
        let execution = match trajectory_type {
            // A dink over a defender is touch and nerve, not power.
            TrajectoryType::Chip => {
                (skills.technique * 0.5 + skills.flair * 0.25 + skills.passing * 0.25)
                    * skills.availability_factor
            }
            TrajectoryType::HighArc => {
                (skills.overall_quality() + skills.long_shots + skills.crossing) / 3.0
            }
            // A cross is struck with the crossing foot, so the weighting
            // reads the crossing skill first — an elite winger hangs the
            // ball where he means to; a full-back sails it through.
            TrajectoryType::Cross(_) => {
                (skills.crossing * 0.6 + skills.technique * 0.25 + skills.passing * 0.15)
                    * skills.availability_factor
            }
            _ => skills.overall_quality(),
        };
        let weighting = rng.jitter(1.0, (1.0 - execution) * 0.14);

        if trajectory_type.is_ballistic() {
            let apex = Self::target_apex(trajectory_type, distance);
            let vertical = Ball::launch_speed_for_apex(apex);
            let hang = Ball::hang_ticks(vertical);
            if hang > 1.0 {
                let horizontal = (distance / hang) * weighting;
                if horizontal.is_finite() && horizontal <= Self::MAX_PASS_VELOCITY {
                    return Vector3::new(
                        direction.x * horizontal,
                        direction.y * horizontal,
                        vertical,
                    );
                }
            }
            // The ball would have to be struck harder than a pass to cover
            // the ground before it lands — drive it instead. Falls through
            // rather than launching something that cannot reach.
        }

        // Driven / grounded. `rolling_velocity` is already sized to arrive
        // under friction, so it is used verbatim; the lift is whatever the
        // style asks for, which keeps the ball's shape sane without
        // stealing any of its forward pace.
        let lift_apex = match trajectory_type {
            TrajectoryType::Ground => Self::target_apex(TrajectoryType::Ground, distance),
            // A ground cross keeps its own shape — a driven ball skims the
            // grass, a cutback rolls. Falling back to the generic
            // LowDriven apex here would flatten the two into one delivery.
            TrajectoryType::Cross(ct) => ct.apex_metres(distance),
            // A loft that could not be bought still leaves the deck a
            // little — it is a driven ball, not a pass along the carpet.
            _ => Self::target_apex(TrajectoryType::LowDriven, distance),
        };
        let lift = Ball::launch_speed_for_apex(lift_apex) * rng.random_range(0.85..1.15);

        // A ground cross is struck HARD. `rolling_velocity` is weighted to
        // arrive at a pace a receiver can take, which is right for a pass
        // to feet and wrong for a ball fizzed across the six-yard box: the
        // danger of a driven cross is precisely that nobody has time on
        // it. Bounded by the same horizontal cap every pass respects.
        let pace = match trajectory_type {
            TrajectoryType::Cross(CrossType::DrivenLowCross) => 1.40,
            TrajectoryType::Cross(CrossType::Cutback) => 1.15,
            _ => 1.0,
        };
        let driven = Vector3::new(rolling_velocity.x * pace, rolling_velocity.y * pace, 0.0);
        let speed = (driven.x * driven.x + driven.y * driven.y).sqrt();
        let scale = if speed > Self::MAX_PASS_VELOCITY {
            Self::MAX_PASS_VELOCITY / speed
        } else {
            1.0
        };

        Vector3::new(driven.x * scale, driven.y * scale, lift.max(0.0))
    }

    /// Records a possession gain on the claimant's team coach if this
    /// claim represents a team switch (previous owner was on the other
    /// team, or there was no previous owner). Used by `MatchCoach::can_shoot`
    /// to gate shots behind a build-up window.
    fn record_team_possession_if_switch(
        claimant_id: u32,
        field: &MatchField,
        context: &mut MatchContext,
    ) {
        let claimant_team = match field.players.iter().find(|p| p.id == claimant_id) {
            Some(p) => p.team_id,
            None => return,
        };
        let previous_team = field
            .ball
            .previous_owner
            .and_then(|pid| field.players.iter().find(|p| p.id == pid))
            .map(|p| p.team_id);
        let switched = previous_team.map_or(true, |pt| pt != claimant_team);
        if switched {
            let tick = context.current_tick();
            context
                .coach_for_team_mut(claimant_team)
                .record_possession_gain(tick);
        }
    }

    fn handle_claim_ball_event(player_id: u32, field: &mut MatchField, context: &MatchContext) {
        // A claim from out of reach is not a reception — it is a booking
        // that `Ball::move_to` cancels next tick, having already consumed
        // `pass_target_player_id` and the pass window with it. The
        // reception histogram says legitimate claims are 100% inside this
        // range, so nothing real is refused here.
        if !Self::can_take_possession(player_id, field) {
            return;
        }

        // CLAIM COOLDOWN: Prevent rapid ping-pong between players
        // If the ball was just claimed by someone else, reject this claim
        const CLAIM_COOLDOWN_TICKS: u32 = 15; // ~250ms at 60fps - time before ball can change hands

        // IN-FLIGHT CHECK: If ball was just passed, only allow the intended receiver to claim
        // This prevents the passer from reclaiming via a stale ClaimBall event that was
        // generated before the PassTo event cleared ownership in the same tick
        if field.ball.flags.in_flight_state > 0 {
            if let Some(target_id) = field.ball.pass_target_player_id {
                if player_id != target_id {
                    return; // Reject claim from non-target during in-flight
                }
            } else {
                // Ball is in flight with no target (e.g., shot) - reject all claims
                return;
            }
        }

        // If there's a cooldown active and this player doesn't already own the ball
        if field.ball.claim_cooldown > 0 {
            if let Some(current_owner) = field.ball.current_owner {
                if current_owner != player_id {
                    // Ball was just claimed by someone else - reject this claim
                    return;
                }
            }
        }

        // If there's already an owner and they're different from the claimer
        // Only allow the claim if enough time has passed (ownership_duration check)
        if let Some(current_owner) = field.ball.current_owner {
            if current_owner == player_id {
                // Already owns the ball (e.g. try_pass_target_claim already set ownership)
                // Don't reset previous_owner — it tracks who passed to us
                return;
            }

            // Different player trying to claim - this is a tackle/interception
            // Reject if current owner hasn't held ball long enough (prevents ping-pong)
            let min_duration = if field.ball.contested_claim_count > 3 {
                60
            } else {
                25
            };
            if field.ball.ownership_duration < min_duration {
                return;
            }

            // Allow claim with escalated cooldown
            field.ball.previous_owner = Some(current_owner);
            field.ball.current_owner = Some(player_id);
            field.ball.pass_target_player_id = None;
            field.ball.ownership_duration = 0;
            field.ball.contested_claim_count += 1;
            let cooldown = if field.ball.contested_claim_count > 6 {
                90
            } else if field.ball.contested_claim_count > 3 {
                45
            } else {
                CLAIM_COOLDOWN_TICKS
            };
            field.ball.claim_cooldown = cooldown;
            field.ball.flags.in_flight_state = cooldown as usize;
            return;
        }

        // No current owner - normal claim.
        Self::resolve_pending_pass_on_control(player_id, field, context);
        field.ball.previous_owner = field.ball.current_owner;
        field.ball.current_owner = Some(player_id);
        field.ball.pass_target_player_id = None;
        field.ball.ownership_duration = 0;
        field.ball.claim_cooldown = CLAIM_COOLDOWN_TICKS;
        field.ball.flags.in_flight_state = 30;
    }

    fn handle_move_ball_event(player_id: u32, ball_velocity: Vector3<f32>, field: &mut MatchField) {
        // Nobody dribbles a ball out of a keeper's hands either.
        if field.ball.held_in_hands && field.ball.current_owner != Some(player_id) {
            return;
        }
        // Nor dribbles one he is nowhere near — this grants ownership as
        // well as setting a velocity, so it can strand the ball the same
        // way the catch did.
        if !Self::can_take_possession(player_id, field) {
            return;
        }
        field.ball.previous_owner = field.ball.current_owner;
        field.ball.current_owner = Some(player_id);
        // Carrying it, therefore not holding it.
        field.ball.held_in_hands = false;

        field.ball.velocity = ball_velocity;
    }

    fn handle_gain_ball_event(player_id: u32, field: &mut MatchField, context: &MatchContext) {
        Self::resolve_pending_pass_on_control(player_id, field, context);
        Self::secure_ball_for(player_id, field);
        if let Some(team_id) = field.get_player(player_id).map(|p| p.team_id) {
            field.ball.note_possession(team_id);
        }
        field.ball.flags.in_flight_state = 100;
    }

    /// Settle the live pass window against a player who has just taken
    /// control of the ball.
    ///
    /// A teammate of the passer completes the pass — whether or not they
    /// were the intended target, and whichever route they took to the
    /// ball. This used to live only in the `ClaimBall` handler, so the
    /// other two ways a player gains control (`GainBall`, `TakeBall`)
    /// silently booked the pass as a failure: they set ownership and
    /// nulled `pass_target_player_id` without ever looking at the pass
    /// window. With deliveries no longer credited on contact at 100u,
    /// those two routes carry a large share of real receptions, and the
    /// omission was the single biggest term in the measured pass
    /// accuracy.
    ///
    /// Keys off `pending_pass_passer` rather than `pass_target_player_id`
    /// because the target flag gets nulled by many unrelated paths
    /// (set-pieces, clearances, save handoffs) while the passer flag
    /// tracks the real pass window.
    pub(crate) fn resolve_pending_pass_on_control(
        player_id: u32,
        field: &mut MatchField,
        context: &MatchContext,
    ) {
        let Some(passer_id) = field.ball.pending_pass_passer else {
            return;
        };
        if passer_id == player_id {
            return;
        }
        let same_team = field
            .players
            .iter()
            .find(|p| p.id == player_id)
            .and_then(|claimant| {
                field
                    .players
                    .iter()
                    .find(|p| p.id == passer_id)
                    .map(|passer| claimant.team_id == passer.team_id)
            })
            .unwrap_or(false);
        if same_team {
            // Single completion path, shared with `BallEvent::PassCompleted`
            // — increments `passes_completed`, classifies progressive /
            // cross / box-entry, and clears the metadata.
            Self::credit_completed_pass(player_id, passer_id, field, context);
            field
                .ball
                .note_possession_source(player_id, PossessionSource::PassReception);
        } else {
            // Opponent won the pass — accuracy window ends.
            field.ball.clear_pending_pass_metadata();
        }
    }

    /// May `player_id` be handed the ball right now?
    ///
    /// The backstop behind every ownership grant that does NOT snap the
    /// ball to the player (`secure_ball_for` does, so it is exempt).
    /// `Ball::move_to` has always refused to track a ball to an owner
    /// beyond `MAX_OWNER_TRACK_DISTANCE` — the problem was that it refused
    /// a tick late, after the handler had zeroed the velocity, leaving a
    /// dead ball sitting in mid-pitch. Asking before anything is mutated
    /// gives the same answer while the ball is still flying.
    ///
    /// Not a substitute for the states getting their own reach right: this
    /// is the net under them, and `reception_diag::GRANT_OUT_OF_REACH` is
    /// how many times a state asked for something impossible.
    fn can_take_possession(player_id: u32, field: &MatchField) -> bool {
        let Some(player) = field.players.iter().find(|p| p.id == player_id) else {
            return false;
        };
        if field.ball.within_possession_reach(player.position) {
            return true;
        }
        #[cfg(feature = "match-logs")]
        crate::r#match::engine::ball::ball::ownership::reception_diag::GRANT_OUT_OF_REACH
            .fetch_add(1, Ordering::Relaxed);
        false
    }

    // Snaps the ball to the winner's feet and zeros velocity — prevents
    // residual velocity from carrying it into the winner's own goal
    // after a tackle/interception/block.
    fn secure_ball_for(player_id: u32, field: &mut MatchField) {
        // You cannot take the ball out of a goalkeeper's gloves.
        //
        // `held_in_hands` was enforced on the ownership layer — the claim
        // scan, the interception check — but NOT here, and this is the
        // chokepoint every tackle, gain-ball and owner-change event goes
        // through. So opponents kept stripping held balls, the keeper kept
        // re-gathering, and the counters show exactly that shape: 184
        // gathers a match against a real 8-12, at a mean hold of 1.74 s —
        // shorter than the two-second minimum a keeper is supposed to hold
        // for, which is only possible if it is being taken off him.
        //
        // Downstream that was the whole ball game: `carrier_id` suppresses
        // pressing while the ball is held, so a quarter of every match ran
        // with the defending half of the engine switched off.
        if field.ball.held_in_hands && field.ball.current_owner != Some(player_id) {
            return;
        }
        #[cfg(feature = "match-logs")]
        if field.ball.cached_shot_target.is_some() {
            crate::r#match::engine::ball::ball::ownership::reception_diag::SHOT_CLAIMED
                .fetch_add(1, Ordering::Relaxed);
        }
        if let Some(player) = field.players.iter().find(|p| p.id == player_id) {
            field.ball.position = player.position;
            field.ball.position.z = 0.0;
        }
        field.ball.previous_owner = field.ball.current_owner;
        field.ball.current_owner = Some(player_id);
        field.ball.pass_target_player_id = None;
        field.ball.velocity = Vector3::zeros();
        field.ball.flags.in_flight_state = 0;
        field.ball.cached_shot_target = None;
        // Won at the feet — a tackle, an interception, a block. The
        // caught-ball path re-raises this immediately for the one case
        // that is genuinely a catch.
        field.ball.held_in_hands = false;
    }

    /// The frame a strike is judged against: the centre of the goal the
    /// shooter is attacking.
    ///
    /// `target` is the striker's INTENT — the goal mouth for a shot at
    /// goal, a point upfield for a defensive clearance (those are
    /// dispatched as Shoot events too, and keep their own target). It
    /// used to be read straight into `goal_center`, i.e. as the middle
    /// of the goal, with the posts derived from it. Any aim offset the
    /// state machine had already applied therefore MOVED THE POSTS with
    /// it, and `handle_shoot_event`'s own placement roll added a second,
    /// independent offset on top. The two compounded: a confident
    /// finisher aimed ~46u (5.8 m) off centre — two metres outside the
    /// post — while the `on_target` check, reading the same displaced
    /// frame, reported the strike as hitting the target. Struck from the
    /// six-yard box that trajectory runs almost parallel to the goal
    /// line and dribbles out for a goal kick, which is what "he missed
    /// from two metres and the ball just stopped next to the goal" looks
    /// like from the stands.
    fn resolve_goal_frame(
        target: Vector3<f32>,
        shooter_side: Option<PlayerSide>,
        goal_positions: &GoalPosition,
    ) -> Vector3<f32> {
        let attacking_goal = match shooter_side {
            Some(PlayerSide::Left) => goal_positions.right,
            Some(PlayerSide::Right) => goal_positions.left,
            None => return target,
        };
        // Aimed INTO that goal mouth ⇒ the frame is the goal, wherever
        // inside it the player was trying to put the ball. Anything
        // else — a clearance hoofed upfield — keeps its own target.
        if (target - attacking_goal).magnitude() <= GOAL_MOUTH_HALF_WIDTH {
            attacking_goal
        } else {
            target
        }
    }

    /// How far off centre a shooter INTENDS to place the ball, bounded
    /// by the frame he is aiming into.
    ///
    /// A striker picks the corner he can actually hit: the intended
    /// point is pulled back inside the posts by most of his own spread,
    /// so aim + a typical error still lands between them. Unbounded, the
    /// intended point sat within ~4u (0.5 m) of the post at EVERY range
    /// for a good finisher, so the random band alone decided the outcome
    /// of chances the player had already done the hard part of creating.
    /// Only 0.75 of the spread is given back — ambitious placement still
    /// finds the side-netting, which is what the side-netting is for.
    fn intended_corner_reach(placement_skill: f32, max_y_error: f32) -> f32 {
        let safe_reach =
            (GOAL_MOUTH_HALF_WIDTH - max_y_error * 0.75).max(GOAL_MOUTH_HALF_WIDTH * 0.20);
        (GOAL_MOUTH_HALF_WIDTH * (0.42 + placement_skill * 0.48)).min(safe_reach)
    }

    fn handle_shoot_event(
        shoot_event_model: ShootingEventContext,
        field: &mut MatchField,
        context: &MatchContext,
        direct_assister_id: Option<u32>,
    ) {
        // Half-width of the goal in game units — the engine's own
        // constant, not a copy of it, so the strike model and the
        // goal-line test can never disagree about where the posts are.
        const GOAL_WIDTH: f32 = GOAL_MOUTH_HALF_WIDTH;
        #[allow(dead_code)]
        const GOAL_HEIGHT: f32 = 8.0; // Height of crossbar
        // Shot velocity cap. Field is 840u = 105m, so 1u = 0.125m, and at
        // 100-tick/s simulation a cap of 5.6 u/tick meant 70 m/s (~252 km/h)
        // — about 2× the real-world top shot speed (~35 m/s = 2.8 u/tick).
        // Keepers covering 1.16 u/tick laterally had only 9 ticks of shot
        // flight from 50u out (~10u of coverage) against a 29u half-goal:
        // any shot aimed outside the central 15u bypassed the save check.
        // Calibrated to real-world peak shot speed — elite piledrivers now
        // arrive in ~18 ticks instead of ~9, matching the ~3.75× shot/player
        // speed ratio observed in real football (vs. the engine's prior ~10×).
        const MAX_SHOT_VELOCITY: f32 = 3.2;
        /// How far off the true crossing point a keeper's committed line
        /// can be, in game units, before the flight-time scaling. 18u is
        /// 2.25 m — the width a placed shot beats a set keeper by.
        const KEEPER_PLACEMENT_READ: f32 = 18.0;
        /// Half-width of the lane a defender must be inside to be credited
        /// with covering a shot, in game units (60u = 7.5m). Wide enough to
        /// mean "in front of goal in the danger zone" rather than "literally
        /// on the ball line" — a defender two yards off the strike still
        /// narrowed the angle it had to beat.
        const COVER_LANE_HALF_WIDTH: f32 = 60.0;

        const MIN_SHOT_DISTANCE: f32 = 1.0; // Minimum distance to prevent NaN from normalization

        let rng = &context.rng;

        // Build the unified shot profile from the same inputs the
        // pre-shot decision saw — the post-hoc xG, the trajectory error
        // budget, the on-target probability and the miskick odds all
        // come from this single object. Replaces the prior
        // (compress_high + raw skill) reads that let a 5/20 player
        // inherit elite conversion through linear blends.
        let minute = sc::minute_from_ticks(shoot_event_model.tick);
        let pre_distance = (shoot_event_model.target - field.ball.position).magnitude();

        // Direct free-kick wall block. If the ball came from a
        // DirectFreeKick origin AND the band is shootable, roll the
        // wall-block probability. A blocked shot has its trajectory
        // overwritten with a low-velocity deflection past the wall;
        // the ball stays live so the loose-ball state machine resumes.
        if field.ball.pass_origin_restart == PassOriginRestart::DirectFreeKick
            && FoulResolver::wall_blocks_direct_fk(
                shoot_event_model.from_player_id,
                field,
                context,
                pre_distance,
            )
        {
            let dir = (shoot_event_model.target - field.ball.position).normalize();
            // Deflect upward and slightly away — the ball clears the wall
            // but loses most of its goalward energy. The apex is solved,
            // not written: the vertical axis is in metres, and the 1.4
            // this used to carry was 140 m/s — a kilometre straight up.
            const WALL_DEFLECT_APEX_M: f32 = 4.0;
            field.ball.velocity = Vector3::new(
                dir.x * 0.6,
                dir.y * 0.6,
                Ball::launch_speed_for_apex(WALL_DEFLECT_APEX_M),
            );
            field.ball.flags.in_flight_state = 30;
            field.ball.previous_owner = Some(shoot_event_model.from_player_id);
            field.ball.current_owner = None;
            field.ball.cached_shot_target = None;
            field.ball.last_shot_xgot = 0.0;
            field.ball.last_shot_shooter_id = None;
            // Restart origin is consumed by the wall — return to
            // open play so the next tick doesn't repeat the block.
            field.ball.pass_origin_restart = PassOriginRestart::OpenPlay;
            return;
        }

        // Snapshot the bits of state we need from `field` so we can
        // build the profile without juggling overlapping borrows.
        let (
            condition_pct,
            shooter_position,
            shooter_side,
            technique_skill,
            finishing_skill,
            long_shot_skill,
            shooter_team_id,
            composure01,
        ) = {
            let player = field.get_player(shoot_event_model.from_player_id).unwrap();
            (
                (player.player_attributes.condition as f32 / 10_000.0).clamp(0.0, 1.0),
                player.position,
                player.side,
                (player.skills.technical.technique / 20.0).clamp(0.1, 1.0),
                (player.skills.technical.finishing / 20.0).clamp(0.1, 1.0),
                (player.skills.technical.long_shots / 20.0).clamp(0.1, 1.0),
                player.team_id,
                (player.skills.mental.composure / 20.0).clamp(0.0, 1.0),
            )
        };

        // Desperation factor — the missing half of the chasing dynamic.
        // The tactical layer already makes trailing teams shoot MORE
        // (risk lift, PushForward/AllOutAttack); real football's
        // counterweight is that they convert WORSE — snatched first-time
        // finishes, shots through traffic, hero-ball from bad angles.
        // (Documented as systematic xG underperformance when chasing.)
        // Without the counterweight, the trailer's volume push converts
        // at full quality, which manufactures equalizers and is one of
        // the levers behind the measured +18pp draw-correlation surplus
        // at equal strength. Continuous in deficit and clock; calm
        // finishers (composure) shed up to half the penalty.
        let desperation = if !context.behavioral_score_visible() {
            0.0
        } else {
            let home_goals = context.score.home_team.get() as i16;
            let away_goals = context.score.away_team.get() as i16;
            let own_diff = if shooter_team_id == context.field_home_team_id {
                home_goals - away_goals
            } else {
                away_goals - home_goals
            };
            let deficit01 = ((-own_diff).max(0) as f32 / 2.0).min(1.0);
            // Onset aligned with the coach's PushForward window (55')
            // so every minute of elevated chasing VOLUME carries the
            // conversion counterweight. Score-blind A/B measurement
            // showed the score-reactive regime alone carries the whole
            // +17pp draw-correlation surplus — the volume half of the
            // chase was modeled without the quality half.
            let late01 = ((minute as f32 - 55.0) / 30.0).clamp(0.0, 1.0);
            deficit01 * late01 * (1.0 - composure01 * 0.5)
        };

        // Pressure counts: scan opposing-side players close to the
        // shooter. We don't have the StateProcessingContext grid here,
        // so a small linear scan is used; the population of opponents
        // is bounded (<=11) so the cost is negligible.
        let mut pressure_5u: u32 = 0;
        let mut pressure_10u: u32 = 0;
        // Wider band for the recorded xG's pressure discount. 5u and 10u
        // are 0.63m and 1.25m — close enough to be standing on the
        // shooter's foot, and measured they are almost always zero, so a
        // discount built on them moved population xG by 2%. A defender
        // closing to ~3m is what a real xG model reads as pressure, and
        // it is what actually separates a free strike from a hurried
        // one. Kept separate rather than widening the two above, which
        // the shot-accuracy error model is calibrated against.
        let mut pressure_24u: u32 = 0;
        if let Some(side) = shooter_side {
            for other in field.players.iter() {
                if other.id == shoot_event_model.from_player_id {
                    continue;
                }
                if other.side == Some(side) {
                    continue;
                }
                let d = (other.position - shooter_position).magnitude();
                if d <= 5.0 {
                    pressure_5u += 1;
                }
                if d <= 10.0 {
                    pressure_10u += 1;
                }
                if d <= 24.0 {
                    pressure_24u += 1;
                }
            }
        }

        // Distance to opposing GK (any opponent goalkeeper).
        let gk_distance = field
            .players
            .iter()
            .filter(|p| p.side != shooter_side)
            .find(|p| {
                matches!(
                    p.tactical_position.current_position.position_group(),
                    PlayerFieldPositionGroup::Goalkeeper
                )
            })
            .map(|gk| (gk.position - shooter_position).magnitude());

        let inputs = ShotSkillInputs {
            distance: pre_distance,
            minute,
            condition_pct,
            pressure_count_5u: pressure_5u,
            pressure_count_10u: pressure_10u,
            // We don't have shot_clarity here, but `has_clear_shot` is
            // the gating signal the pre-shot xG used too.
            shot_clarity: 1.0,
            has_clear_shot: true,
            gk_distance,
            is_sprinting_or_recent_sprint: false,
        };
        let profile = {
            let player = field.get_player(shoot_event_model.from_player_id).unwrap();
            ShotSkillProfile::from_player(player, &inputs)
        };

        // Skill bands derived from the unified profile.
        let execution_skill = profile.execution_skill;
        let placement_skill = profile.placement_skill;
        let body_control = profile.body_control;
        let technique_curve = profile.technique_curve;
        let poor_penalty = profile.poor_penalty;
        let low_condition_penalty = profile.low_condition_penalty;
        let pressure_penalty = profile.pressure_penalty;
        let on_target_skill_multiplier = profile.on_target_skill_multiplier;
        let random_error_scale = profile.random_error_scale;
        let miskick_probability = profile.miskick_probability;
        let power_skill = profile.power_skill;

        // The goal frame comes from the PITCH, never from the event.
        let goal_center = Self::resolve_goal_frame(
            shoot_event_model.target,
            shooter_side,
            &context.goal_positions,
        );

        // Calculate goal bounds
        let goal_left_post = goal_center.y - GOAL_WIDTH;
        let goal_right_post = goal_center.y + GOAL_WIDTH;

        // Calculate distance to goal
        let ball_to_goal_vector = goal_center - field.ball.position;
        let horizontal_distance = (ball_to_goal_vector.x * ball_to_goal_vector.x
            + ball_to_goal_vector.y * ball_to_goal_vector.y)
            .sqrt();

        // Safety check: if ball is already at/very near the goal, just give it a gentle push
        if horizontal_distance < MIN_SHOT_DISTANCE {
            let direction = if ball_to_goal_vector.x.abs() > 0.01 {
                Vector3::new(ball_to_goal_vector.x.signum(), 0.0, 0.0)
            } else {
                Vector3::new(1.0, 0.0, 0.0) // Default direction
            };
            field.ball.previous_owner = Some(shoot_event_model.from_player_id);
            field.ball.current_owner = None;
            field.ball.velocity = direction * 2.0; // Gentle push
            field.ball.flags.in_flight_state = 20;
            return;
        }

        // Overall shot accuracy now flows from the unified profile —
        // execution_skill is already skill-curved, fatigue-aware, and
        // pressure-discounted, so a 5/20 finisher tops out around
        // ~0.10 instead of inheriting elite spread through linear
        // blends. `on_target_skill_multiplier` adds the smoothstep
        // poor/elite shaping (poor_penalty knocks 20% off, elite_lift
        // adds 5%).
        let base_accuracy = on_target_skill_multiplier * execution_skill;

        // Distance penalty multiplier for base_accuracy — close range should be very accurate
        let distance_penalty = if horizontal_distance > 200.0 {
            0.10
        } else if horizontal_distance > 150.0 {
            0.16
        } else if horizontal_distance > 100.0 {
            0.26
        } else if horizontal_distance > 70.0 {
            0.42
        } else if horizontal_distance > 50.0 {
            0.58
        } else if horizontal_distance > 30.0 {
            0.74
        } else if horizontal_distance > 15.0 {
            0.85
        } else {
            0.92
        };
        let adjusted_accuracy = base_accuracy * distance_penalty;

        // Base error: scaled by the unified profile's
        // `random_error_scale`, the per-shot pressure / body / condition
        // multipliers, and a distance term. Min-error floors are
        // boosted by `poor_penalty` so a 5/20 player has a hard floor
        // they can't aim under even at 6u.
        // Aim error is ANGULAR, so the lateral miss it produces at the
        // goal line grows with the distance the ball travels. The old
        // 0.75 floor flattened that below 60u (7.5 m): a two-metre
        // tap-in was sprayed exactly as widely as a strike from the
        // penalty spot, and at point-blank range that band alone decided
        // whether the ball went in or a metre wide. 0.18 leaves the
        // calibrated band from 60u outwards untouched and lets it fall
        // away properly underneath.
        let distance_error = (horizontal_distance / 80.0).clamp(0.75, 3.0);
        let pressure_error = 1.0 + pressure_penalty * 0.55 + (pressure_5u as f32) * 0.04;
        let body_error = 1.0 + (1.0 - body_control) * 0.30;
        let condition_error = 1.0 + low_condition_penalty * 0.45;
        // Base y-error tightened progressively: 34 → 22 → 16. Goal
        // half-width 29u. With pressure / body / distance multipliers
        // stacking up to ~2.5×, even 22u resolved to a 35-50u random
        // band for many long shots — wider than the goal — and the
        // resulting on-target rate sat at ~22% vs real ~33%. 16u with
        // damped pressure / body / condition multipliers keeps spread
        // realistic for both clean strikes and scrambling ones.
        // NOT the lever for the population on-target rate, despite
        // setting the aim band. Measured 2026-08-08: 16 → 13.5 left the
        // rate at 28.3% (from 28.6%). At a 29u half-goal this band is
        // already tight enough that most aims land between the posts —
        // the rate is set by the explicit `wide_miss_chance` /
        // `over_bar_chance` / `miskick_probability` rolls below, which
        // fire independently of it.
        let max_y_error_raw = 16.0
            * distance_error
            * pressure_error
            * body_error
            * condition_error
            * random_error_scale
            * (1.0 + desperation * 0.38)
            * (1.0 - adjusted_accuracy * 0.55);
        let min_error = if horizontal_distance < 18.0 {
            3.0 + poor_penalty * 3.0
        } else if horizontal_distance < 30.0 {
            4.0 + poor_penalty * 4.0
        } else if horizontal_distance < 60.0 {
            6.0 + poor_penalty * 5.5
        } else {
            8.5 + poor_penalty * 7.5
        };
        let max_y_error = max_y_error_raw.clamp(min_error, 60.0);

        // Target placement — driven by `placement_skill`, and bounded by
        // the frame the shooter is aiming into (see
        // `intended_corner_reach`). Spec for the corner/centre split:
        //   central_rate = clamp(0.68 - placement_skill * 0.58, 0.10, 0.68)
        //
        // Resolved AFTER the error budget precisely so it can be bounded
        // by it.
        let central_rate = (0.68 - placement_skill * 0.58).clamp(0.10, 0.68);
        let side_rate = (1.0 - central_rate) * 0.5;
        let target_preference = rng.random_range(0.0..1.0);
        let corner_reach = GOAL_WIDTH * (0.42 + placement_skill * 0.48);
        let _ = max_y_error;
        let ideal_y_target = if target_preference < side_rate {
            goal_center.y - corner_reach
        } else if target_preference < side_rate * 2.0 {
            goal_center.y + corner_reach
        } else {
            goal_center.y + rng.random_range(-GOAL_WIDTH * 0.3..GOAL_WIDTH * 0.3)
        };

        // Add random error to y-coordinate
        let y_error = rng.random_range(-max_y_error..max_y_error);
        let mut actual_y_target = ideal_y_target + y_error;

        // Wide-miss chance now leans on the unified profile so a
        // 5/20 finisher pulls shots wide far more often than an elite.
        // Real Opta on-target rate is ~33% across all distances;
        // calibrated downward to land population on-target near that.
        // Bases cut ~40% (0.030/0.060/0.100/0.150) 2026-08-08. This roll
        // and `over_bar_chance` are what actually set the population
        // on-target rate — the aim band above is tight enough that most
        // targets already fall between the posts, so tightening it moved
        // the rate 28.6% → 28.3% and nothing else. Measured 28.3%
        // against the real ~32-33% this model is written against, and
        // on-target volume is what caps goals/match: at 13.1 shots per
        // team a 32% rate lands 2.47 goals on the engine's own
        // arithmetic against the 2.21 it was producing.
        // Re-raised 2026-08-11, ~3.2x (0.018/0.036/0.060/0.090 -> below).
        //
        // The 2026-08-08 cut took these down ~40% to lift on-target from
        // 28.3% toward the real ~33%; the engine changed underneath them
        // since and the same bases were producing 43.8%.
        //
        // The `off-target causes` diagnostic (added alongside this) showed
        // why that was not a tuning miss but a modelling one: the three
        // forced-miss rolls between them pushed only 15% of shots off
        // target, so **83.4% of every shot struck left the boot aimed
        // between the posts and under the bar**. Real football is nowhere
        // near that — roughly 43% of shots are struck wide or over. The
        // aim band cannot fix it (documented above: tightening it
        // 16 -> 13.5 moved the rate 28.6% -> 28.3% and nothing else),
        // because at a 29u half-goal almost every aim already lands
        // between the posts. These rolls are the lever.
        //
        // Measured at 5x60 matches, this setting moves on-target
        // 43.8% -> 38.7% and on-frame-at-strike 83.4% -> 64%, and it
        // costs something: conversion of on-target shots rises
        // 31.6% -> 35.2% (real 30) and keeper save rate falls
        // 68.4% -> 64.8% (real 67). That is not the aim model being
        // wrong, it is the SHOT MIX: forcing shots off the frame removes
        // long-range attempts preferentially, so what survives is closer
        // and converts better. The engine strikes from **10.1 m on
        // average against a real ~17 m** — until shot SELECTION moves
        // out, a realistic aim model cannot land on-target at 33%
        // without distorting conversion. Pushing these bases a further
        // 25% was tried and reverted: on-target reached 39.2% but
        // conversion hit 40.8% and saves 59.2%, i.e. all cost, no gain.
        // Raised ~1.5x 2026-08-13, with `over_bar_base` below. Every
        // previous titration of this pair was made while ~73% of all
        // shots were being picked out of the air by a defender within a
        // tick of the strike (see `try_intercept`), so the population
        // these rolls were calibrated against was the ~11% of shots that
        // survived — and those were the short ones. With shots now
        // living their full flight, on-frame-at-strike measured 63%
        // against a real ~52% (Opta: ~33% on target plus ~19% blocked).
        let wide_base = if horizontal_distance < 30.0 {
            0.132
        } else if horizontal_distance < 60.0 {
            0.247
        } else if horizontal_distance < 100.0 {
            0.357
        } else {
            0.483
        };
        // Skill-and-condition contributions trimmed (0.07/0.05 → 0.05/0.03)
        // because they compound with `random_error_scale`, `over_bar_chance`,
        // and `miskick_probability` against the same weak shooter — when
        // stacked, the lvl-6 outfield was wide-missing >35% of its shots
        // vs the lvl-18 reference's ~12%, which was crushing the upset
        // tail to zero (see audit_engine_gap). Elite shooters are
        // unaffected: at execution_skill ≥ 0.55, `(1 - exec)` < 0.45 and
        // `poor_penalty` is already zero.
        let wide_miss_chance = wide_base
            + (1.0 - execution_skill) * 0.05
            + poor_penalty * 0.03
            + pressure_penalty * 0.04
            + low_condition_penalty * 0.03
            + desperation * 0.08;
        #[cfg(feature = "match-logs")]
        shot_accuracy_diag::SHOTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        if rng.random_range(0.0f32..1.0) < wide_miss_chance {
            #[cfg(feature = "match-logs")]
            shot_accuracy_diag::WIDE_FIRED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let extra_wide = rng.random_range(GOAL_WIDTH * 0.2..GOAL_WIDTH * 1.5);
            if rng.random_range(0.0f32..1.0) < 0.5 {
                actual_y_target = goal_right_post + extra_wide;
            } else {
                actual_y_target = goal_left_post - extra_wide;
            }
        }

        // Miskick chance — sourced from the unified profile so it
        // includes the smoothstep poor-penalty contribution rather
        // than a single technique^3 read.
        if rng.random_range(0.0f32..1.0) < miskick_probability {
            #[cfg(feature = "match-logs")]
            shot_accuracy_diag::MISKICK_FIRED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            actual_y_target += rng.random_range(-GOAL_WIDTH * 1.5..GOAL_WIDTH * 1.5);
        }

        #[cfg(feature = "match-logs")]
        if (actual_y_target - goal_center.y).abs() <= GOAL_WIDTH {
            shot_accuracy_diag::AIM_INSIDE_POSTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }

        // ── Deflection on the way to goal ───────────────────────────────
        //
        // Real Opta data: 3-5% of all Premier League goals come from
        // shots that took a deflection off a defender or attacker. The
        // engine had no representation of this — every off-target shot
        // sailed cleanly into the stands. Without a stochastic floor,
        // weak teams at extreme skill gaps never beat strong GKs because
        // their own poor placement crushed every shot's threat.
        //
        // Model: only OFF-TARGET shots are deflection candidates. If a
        // defender is roughly in the lane between shooter and goal,
        // there's a per-defender chance the shot ricochets back inside
        // the goal frame. Aim is biased toward corners (a deflection
        // off a leg / hip doesn't fall back to the centre of the goal —
        // that's where the keeper was already set).
        //
        // The deflected flag is forwarded into `cached_shot_target` so
        // the GK's save probability damps appropriately — without that,
        // the deflection actively HURT weak teams (it redirected a
        // would-be-wide shot to on-target where the GK then saved it).
        let was_offtarget = actual_y_target < goal_left_post || actual_y_target > goal_right_post;
        let goal_dir = (goal_center - shooter_position).normalize();
        let shot_lane_distance = (goal_center - shooter_position).magnitude();
        let mut defenders_in_lane: u32 = 0;
        if was_offtarget && shot_lane_distance > 10.0 {
            if let Some(side) = shooter_side {
                for other in field.players.iter() {
                    if other.id == shoot_event_model.from_player_id {
                        continue;
                    }
                    if other.side == Some(side) {
                        continue;
                    }
                    let to_other = other.position - shooter_position;
                    let along = to_other.dot(&goal_dir);
                    if along < 3.0 || along > shot_lane_distance - 3.0 {
                        continue;
                    }
                    let proj = shooter_position + goal_dir * along;
                    let perp = (other.position - proj).magnitude();
                    // Lane half-width 12u (~1.5m) — defenders within
                    // sliding / outstretched-leg / chest-block range of
                    // the shot path. Tightening below this and the
                    // deflection rate collapsed because defenders rarely
                    // stand RIGHT in the shot line — they're a couple of
                    // metres either side.
                    if perp < 12.0 {
                        defenders_in_lane += 1;
                    }
                }
            }
        }
        // Per-defender 4% chance, capped at 14% total. Sized so the
        // per-shot deflection rate on off-target shots in a crowded box
        // is ~10-14% — matching real PL's deflected-goal share.
        // Per-defender chance scaled so a crowded box (3-4 defenders in
        // the shot lane) reaches a ~12-14% deflection rate. Real Opta
        // shot-block / deflection data sits in that band. Solo shots
        // from outside the box rarely deflect (no body to clip).
        //
        // Set-piece bonus: shots originating from a corner / direct FK
        // arrive into a crowded box where the keeper's sight is
        // partially obstructed and bodies are everywhere — the chaos
        // factor that drives real football's "shock corner goal" upset
        // narrative. Bumps the deflection ceiling by 0.12 so set-piece
        // off-target attempts have a meaningful chance of redirecting
        // past the keeper.
        let is_set_piece = matches!(
            field.ball.pass_origin_restart,
            PassOriginRestart::Corner | PassOriginRestart::DirectFreeKick
        );
        let setpiece_bonus = if is_set_piece { 0.12 } else { 0.0 };
        let deflection_chance = (defenders_in_lane as f32 * 0.04 + setpiece_bonus).min(0.26);
        let deflected = was_offtarget && rng.random_range(0.0f32..1.0) < deflection_chance;
        // Save the pre-deflection trajectory: when a deflection fires, the
        // GK shouldn't track the new trajectory in mid-flight — real
        // deflections happen close to arrival and the keeper has already
        // committed to the original line. We keep `original_y_target` for
        // building the GK's cached intercept point further down so they
        // sprint toward the off-target path while the ball arcs into the
        // corner. Without this the GK would just reposition mid-flight
        // and reach the deflected target with no penalty.
        let original_y_target = actual_y_target;
        if deflected {
            // Bias to the FAR corner (|offset| 0.78..0.97 of goal half-
            // width = 22.6u..28.1u from centre). Beyond the strong
            // keeper's lateral reach (~19u) but inside the 29u post.
            let side_sign: f32 = if rng.random_range(0.0f32..1.0) < 0.5 {
                -1.0
            } else {
                1.0
            };
            let corner_offset = rng.random_range(0.78f32..0.97);
            actual_y_target = goal_center.y + side_sign * corner_offset * GOAL_WIDTH;
        }

        // Clamp to reasonable bounds — allow shots to miss by up to 3x goal width
        let max_miss_distance = GOAL_WIDTH * 3.0;
        let clamped_y_target = actual_y_target.clamp(
            goal_left_post - max_miss_distance,
            goal_right_post + max_miss_distance,
        );

        // ── CURL ─────────────────────────────────────────────────────
        // A curled shot is not aimed at where it ends up: the striker
        // aims OUTSIDE the target and bends the ball back onto it. So the
        // launch line is offset by `-curl_units` here and the spin set
        // below brings it back over the flight, which is the only way to
        // get a bending shot that still arrives where it was meant to.
        //
        // How much bend a player gets is technique first (you have to
        // strike across the ball cleanly to spin it) and then the
        // `CurlsBall` trait, which is exactly what that trait describes
        // and which nothing in the ball physics had ever read.
        let curls_ball = field
            .get_player(shoot_event_model.from_player_id)
            .is_some_and(|p| p.has_trait(PlayerTrait::CurlsBall));
        let curl_units = {
            // Most strikes are hit through the ball and barely bend.
            // 1.4 m of bend over a 25 m shot is a genuine whipped effort;
            // a specialist gets close to twice that.
            let ceiling = if curls_ball { 20.0 } else { 11.0 };
            let side_sign: f32 = if rng.random_range(0.0f32..1.0) < 0.5 {
                -1.0
            } else {
                1.0
            };
            // Skewed low: `u²` keeps the median bend small so the
            // straight-struck shot stays the norm.
            let u: f32 = rng.random_range(0.0f32..1.0);
            side_sign * u * u * ceiling * technique_skill
        };

        // Calculate final shot direction. The keeper's projection further
        // down reads this LAUNCH line, so he is genuinely beaten by the
        // part of the bend he doesn't read — see `KEEPER_CURL_READ`.
        let launch_y_target = clamped_y_target - curl_units;
        let actual_target = Vector3::new(goal_center.x, launch_y_target, 0.0);
        let shot_vector = actual_target - field.ball.position;

        // Skill-based power multiplier — sourced from the unified
        // profile's `power_skill` so a low-strength / low-technique
        // shot can't accidentally inherit the same power as an elite.
        let power_multiplier = 0.95 + (power_skill * 0.35); // Range: 0.95 to 1.30

        // Calculate horizontal velocity with skill-based power
        let horizontal_direction = Vector3::new(shot_vector.x, shot_vector.y, 0.0).normalize();
        let base_horizontal_velocity = shoot_event_model.force as f32 * power_multiplier * 1.6;

        // Add power randomness (better players have more consistent power)
        let power_consistency = 0.96 + (technique_skill * 0.08); // 0.96 to 1.04
        let power_random = rng.random_range(power_consistency - 0.04..power_consistency + 0.04);
        let horizontal_velocity = horizontal_direction * base_horizontal_velocity * power_random;

        // Calculate z-velocity based on shot style and player skills
        let shot_style: f32 = rng.random_range(0.0..1.0);
        let height_variation: f32 = rng.random_range(0.85..1.15);

        // Shot height, expressed as the APEX of the strike in metres and
        // converted to a launch speed by the shared ballistics helper.
        //
        // These used to be raw z velocities on a vertical axis whose
        // gravity was 160× too strong in metres, so the bands were fitted
        // to that: a "rising shot" at 3.0 climbed 3 m and was back on the
        // deck 25 units later, and every shot bounced its way to goal.
        // Restated as apexes they say what they mean, and the ball now
        // FLIES to the target — a 1.5 m apex from 25 m arrives at about
        // waist height, a 2.2 m one at the top corner.
        let apex_metres = if horizontal_distance > 100.0 {
            // Long-range shot - varied heights (technique matters more)
            if shot_style < 0.4 {
                rng.random_range(0.15..0.60) * technique_skill // Low driven (40%)
            } else if shot_style < 0.8 {
                rng.random_range(0.60..1.60) * technique_skill // Normal (40%)
            } else {
                rng.random_range(1.60..2.60) * long_shot_skill // Rising shot (20%)
            }
        } else if horizontal_distance > 50.0 {
            // Medium-range shot - mostly low (finishing matters more)
            if shot_style < 0.6 {
                rng.random_range(0.05..0.35) * finishing_skill // Very low (60%)
            } else if shot_style < 0.9 {
                rng.random_range(0.35..1.00) * technique_skill // Medium (30%)
            } else {
                rng.random_range(1.00..2.00) * technique_skill // High (10%)
            }
        } else {
            // Close-range shot - very low and varied (finishing is key)
            if shot_style < 0.7 {
                rng.random_range(0.02..0.20) * finishing_skill // Ground shot (70%)
            } else if shot_style < 0.95 {
                rng.random_range(0.7..1.3) * finishing_skill // Rising (25%)
            } else {
                rng.random_range(1.3..2.0) * technique_skill // Chip (5%)
            }
        };

        // Add spin/environmental variation to height
        let vertical_spin_variation = rng.random_range(0.90..1.10);

        // Over-the-bar miss chance: distance-dependent. Real football
        // data: close-range shots go over ~5%, long range ~20%. Earlier
        // 0.04/0.08/0.14/0.20 bases + 0.20 accuracy scaling were OK when
        // over-bar shots were silently counted on-target (the counting
        // bug just fixed). Now that over-bar shots correctly do nothing,
        // the same bases produce too many accuracy-less misses. Cut
        // bases ~40% and scaling 0.20 → 0.12 so the on-target rate can
        // recover toward the real ~33%.
        // Bases cut ~40% (0.015/0.030/0.055/0.080) 2026-08-08, alongside
        // `wide_base` — the two forced-miss rolls are the same class and
        // together they, not the aim band, set the population on-target
        // rate.
        // Re-raised 2026-08-11 alongside `wide_base`, same reasoning and
        // the same ratio — the two forced-miss rolls are one lever and
        // must move together or the wide/over split distorts.
        let over_bar_base = if horizontal_distance < 30.0 {
            0.067
        } else if horizontal_distance < 60.0 {
            0.132
        } else if horizontal_distance < 100.0 {
            0.178
        } else {
            0.253
        };
        // Skill contributions trimmed (0.04 / 0.04 → 0.025 / 0.025) — same
        // logic as `wide_miss_chance` above: the over-bar term compounds
        // with miskick and random-error sources against weak shooters and
        // was pushing the low-level outfield's on-target rate well below
        // real Opta's 33% baseline. Strong shooters (technique_curve ≥
        // 0.55, poor_penalty = 0) are unaffected.
        let over_bar_chance = over_bar_base
            + (1.0 - technique_curve).max(0.0) * 0.025
            + poor_penalty * 0.025
            + low_condition_penalty * 0.03;
        let shot_goes_over_bar = rng.random_range(0.0f32..1.0) < over_bar_chance;
        #[cfg(feature = "match-logs")]
        {
            use std::sync::atomic::Ordering as AtomicOrdering;
            if shot_goes_over_bar {
                shot_accuracy_diag::OVER_FIRED.fetch_add(1, AtomicOrdering::Relaxed);
            } else if (actual_y_target - goal_center.y).abs() <= GOAL_WIDTH {
                shot_accuracy_diag::AIM_ON_FRAME.fetch_add(1, AtomicOrdering::Relaxed);
            }
        }
        let struck_apex = if shot_goes_over_bar {
            // Skied. Peaks well clear of the 2.44 m crossbar whatever the
            // range, so the ball is over it at any plausible crossing time.
            rng.random_range(4.0..12.0)
        } else {
            (apex_metres * height_variation * vertical_spin_variation).clamp(0.0, 4.0)
        };
        let z_velocity = Ball::launch_speed_for_apex(struck_apex);

        // Calculate final velocity
        let mut final_velocity =
            Vector3::new(horizontal_velocity.x, horizontal_velocity.y, z_velocity);

        // CRITICAL: Validate and clamp velocity to prevent cosmic-speed shots
        // Check for NaN or infinity
        if final_velocity.x.is_nan()
            || final_velocity.y.is_nan()
            || final_velocity.z.is_nan()
            || final_velocity.x.is_infinite()
            || final_velocity.y.is_infinite()
            || final_velocity.z.is_infinite()
        {
            // Fallback to a safe default velocity toward the goal. Both
            // components were pre-metric: 5.0 u/tick is 62 m/s and a `z`
            // of 1.0 is a 510 m apex.
            let safe_direction = (goal_center - field.ball.position).normalize();
            final_velocity = Vector3::new(
                safe_direction.x * 2.4,
                safe_direction.y * 2.4,
                Ball::launch_speed_for_apex(1.5),
            );
        }

        // Clamp velocity magnitude to maximum realistic shot speed
        let velocity_magnitude = final_velocity.norm();
        if velocity_magnitude > MAX_SHOT_VELOCITY {
            final_velocity = final_velocity * (MAX_SHOT_VELOCITY / velocity_magnitude);
        }

        // Record shot in player memory. A shot counts as "on target" only
        // if the frame of the goal is actually threatened — the y-aim lies
        // between the posts AND we didn't just roll "over the bar" above.
        // The over-bar branch sets z-velocity to 3.0-6.0 (guaranteed clear
        // of crossbar at 8.0), so those shots fly into the stands. Before
        // this check, `on_target` counted them because it only looked at y,
        // producing a ~49% conversion leak where on-target shots were
        // neither saved nor scored — they just sailed over, off-screen.
        let on_target = clamped_y_target >= goal_left_post
            && clamped_y_target <= goal_right_post
            && !shot_goes_over_bar;
        // xG is a location-based chance value (Opta convention), so the
        // shooter's per-shot xG records the BASE chance regardless of
        // whether the strike actually threatened the goal — a skied
        // 0.78xG penalty still counts as 0.78 xG. Earlier code scaled
        // this by 0.15 for off-target shots, which collapsed
        // population xG to 0.2/team (real Opta ~1.3) and inflated
        // goals-vs-xG dramatically because actual conversion ran on
        // the unweighted shot count.
        //
        // The keeper's ledger is a different concept entirely and no
        // longer derives from this number at all: a GK is measured on
        // what a league-average keeper would have conceded from the
        // strike that actually arrived, which is stamped separately as
        // `last_shot_xgot` once the shot's crossing point is projected.
        // Convert to REAL xG units for recording. The decision-time curve
        // deliberately runs ~3x hot (every gameplay gate — min_xg floors,
        // the willingness quality pull, the Tier bars — is calibrated
        // against it), but the RECORDED stat feeds player pages, the
        // rating model and the calibration harness, all of which expect
        // real Opta units where population xG/shot is ~0.11 and xG
        // tracks actual conversion. Scaling here keeps gameplay bit-for
        // bit identical while making every reported figure honest.
        // 0.41 makes recorded xG track ACTUAL conversion per position
        // (FWD 0.113 xG vs 10.2% conv, MID 0.066 vs 6.6%) — the property
        // that defines a calibrated xG model, and what stops the rating
        // layer charging forwards for chances they did not really have.
        const XG_REPORT_SCALE: f32 = 0.45;
        // Angle discrimination. The profile at this site is built with
        // `shot_clarity: 1.0, has_clear_shot: true`, so the recorded xG
        // was pure DISTANCE — a byline effort from 6m priced identically
        // to a central one. Real xG falls off hard with the shooting
        // angle, and without it forwards (who take the tight-angle
        // chances) read as permanently wasteful while central arriving
        // midfielders read as clinical. Pure geometry, available here:
        // the share of the goal mouth actually visible from the ball.
        let angle_factor = {
            let x_dist = ball_to_goal_vector.x.abs().max(1.0);
            let y_off = ball_to_goal_vector.y.abs();
            const HALF_GOAL: f32 = 29.0;
            let near = (y_off - HALF_GOAL).max(0.0);
            let far = y_off + HALF_GOAL;
            let visible = (far / x_dist).atan() - (near / x_dist).atan();
            let central = 2.0 * (HALF_GOAL / x_dist).atan();
            if central > 1e-4 {
                (visible / central).clamp(0.0, 1.0).powf(0.7)
            } else {
                1.0
            }
        };
        // Pressure discount. The recorded xG was a function of DISTANCE
        // and ANGLE only — a strike with a defender's leg across it
        // priced identically to a free one from the same spot. Real xG
        // models all read the closing defender, and its absence here was
        // measurable: across 4 runs of `stats 300 14 14` the engine
        // scored **2.21 against its own 2.60 xG**, underperforming its
        // chance model by 15% in a way real teams do not, and the
        // per-line conversion-to-xG ratios came apart — MID 0.78-0.88 and
        // FWD 0.78-0.89 (open-play shots, pressured, over-priced).
        //
        // It also got worse the moment `DefensiveRecovery` put bodies
        // back in the box: a pressure-blind curve overstates a pressured
        // shot by more, so the better the defending got, the more the
        // recorded xG drifted above the goals it was supposed to predict.
        // That is why every shot-volume lever failed to move goals — the
        // extra shots were pressured ones being priced as free.
        //
        // `pressure_24u` (3m) is inclusive of `pressure_10u`, so the
        // 1.25-3m ring is their difference and carries about half the
        // weight of a body genuinely on top of the shot.
        let close_pressure = pressure_10u as f32;
        let ring_pressure = pressure_24u.saturating_sub(pressure_10u) as f32;
        let pressure_factor = (1.0 - close_pressure * 0.20 - ring_pressure * 0.10).clamp(0.35, 1.0);
        let base_xg = profile
            .expected_xg(horizontal_distance, true)
            .clamp(0.0, 0.82)
            * angle_factor
            * pressure_factor
            * XG_REPORT_SCALE;
        let xg = base_xg;
        if let Some(shooter) = field.get_player_mut(shoot_event_model.from_player_id) {
            shooter.memory.record_shot(shoot_event_model.tick);
            shooter.memory.record_shot_xg(shoot_event_model.tick, xg);
        }
        #[cfg(feature = "match-logs")]
        {
            use std::sync::atomic::Ordering;
            let band = time_band_diag::band_for_minute(minute);
            time_band_diag::SHOTS_BY_BAND[band].fetch_add(1, Ordering::Relaxed);
            if on_target {
                time_band_diag::ON_TARGET_BY_BAND[band].fetch_add(1, Ordering::Relaxed);
            }
            time_band_diag::XG_X1000_BY_BAND[band]
                .fetch_add((xg * 1000.0) as u64, Ordering::Relaxed);
            let dband = time_band_diag::band_for_distance(horizontal_distance);
            time_band_diag::SHOTS_BY_DIST[dband].fetch_add(1, Ordering::Relaxed);
            if let Some(sh) = field.get_player(shoot_event_model.from_player_id) {
                let g = match sh.tactical_position.current_position.position_group() {
                    crate::PlayerFieldPositionGroup::Goalkeeper => 0,
                    crate::PlayerFieldPositionGroup::Defender => 1,
                    crate::PlayerFieldPositionGroup::Midfielder => 2,
                    crate::PlayerFieldPositionGroup::Forward => 3,
                };
                time_band_diag::SHOTS_BY_POS_DIST[g][dband].fetch_add(1, Ordering::Relaxed);
            }
            if dband == 1 {
                time_band_diag::EMITTED_MID_BAND
                    [time_band_diag::emit_tag_index(shoot_event_model.reason)]
                .fetch_add(1, Ordering::Relaxed);
            }
            if dband == 0 {
                time_band_diag::EMITTED_CLOSE_BAND
                    [time_band_diag::emit_tag_index(shoot_event_model.reason)]
                .fetch_add(1, Ordering::Relaxed);
            }
            time_band_diag::XG_X1000_BY_DIST[dband]
                .fetch_add((xg * 1000.0) as u64, Ordering::Relaxed);
        }

        // ── xG chain / buildup distribution ──────────────────────────
        // The shooter's recent attacking partners share a small slice
        // of the shot's xG so a midfielder who set up the chance gets
        // some signal even when the shot misses or the assister is
        // someone else. `recent_passers` is a 5-deep ring updated on
        // every successful kick from a teammate.
        //
        // - xg_chain: anyone who touched the ball in the chain
        //   (including shooter and direct assister).
        // - xg_buildup: same set MINUS shooter and direct assister
        //   (the shooter already gets the shot's xG via memory.xg, the
        //   assister's value is already captured separately).
        //
        // The total chain pool (xg * 0.30) and buildup pool (xg * 0.20)
        // are split across the unique participants of the chain. Without
        // the split, a player who appeared in the ring multiple times
        // accumulated the per-event credit linearly — a single shot
        // with the same passer at indices 0/2/4 produced 3× the buildup
        // it should. Per-player caps in `rating.rs` clip the worst case,
        // but the raw stat reported on the match sheet was still wrong.
        let shooter_id = shoot_event_model.from_player_id;
        if xg > 0.0 {
            // Restrict the chain to the shooter's own team. The ring
            // survives a turnover (an interception hands the ball over
            // without clearing it), so an unfiltered read was paying
            // xg_chain / xg_buildup to the defenders and keeper who had
            // just been dispossessed — credit for the chance they
            // conceded.
            let shooter_team = field.get_player(shooter_id).map(|p| p.team_id);
            let credits = Self::distribute_xg_credit(
                field
                    .ball
                    .recent_passers
                    .iter()
                    .filter(|e| Some(e.team_id) == shooter_team)
                    .map(|e| e.player_id),
                shooter_id,
                direct_assister_id,
                xg,
            );
            for (pid, chain_credit, buildup_credit) in credits {
                if let Some(p) = field.get_player_mut(pid) {
                    p.statistics.record_xg_chain(chain_credit);
                    if let Some(buildup) = buildup_credit {
                        p.statistics.record_xg_buildup(buildup);
                    }
                }
            }
        }

        // Stash the shooter so the GK ledger hooks (save / catch / parry
        // / goal handlers) can resolve him without re-deriving. The
        // per-shot expectation itself is stamped below, once the shot's
        // projected crossing point is known — it is a function of WHERE
        // the ball is going, which this point in the handler does not
        // know yet.
        field.ball.last_shot_shooter_id = Some(shoot_event_model.from_player_id);
        field.ball.last_shot_struck_tick = context.current_tick();
        // Cleared here so a shot whose target cannot be projected (the
        // `else` arm below) can never leave the previous shot's value on
        // the ball for the next keeper credit to pick up.
        field.ball.last_shot_xgot = 0.0;

        field.ball.previous_owner = Some(shoot_event_model.from_player_id);
        field.ball.current_owner = None;
        field.ball.pass_target_player_id = None;
        // Shot arms the carry tracker — clear the carry; if a goal /
        // save resolves, no carry credit is owed for the player who
        // just shot.
        field.ball.carry_owner = None;
        // Likewise clear pass metadata — a shot ends the live pass
        // window and the receiver-becomes-shooter case has already
        // been credited by the dispatch site above.
        field.ball.clear_pending_pass_metadata();
        field.ball.velocity = final_velocity;

        // Rotation to match the strike. Two axes, both of them real:
        //
        // * SIDESPIN, solved so the Magnus force bends the ball back by
        //   exactly the `curl_units` the launch line was offset by. With
        //   constant lateral acceleration `a` over flight time `T`, the
        //   deflection is `d = ½aT²`, and the Magnus term contributes
        //   `a_y = C·ω_z·v_x`, so `ω_z = 2d / (T²·C·v_x)`.
        // * TOPSPIN on a rising strike, which is what makes a long-range
        //   effort dip back under the bar. The apex-solved parabola is
        //   symmetric and can never dip on its own.
        {
            let vx = final_velocity.x;
            let vy = final_velocity.y;
            let speed = (vx * vx + vy * vy).sqrt();
            let mut spin = Vector3::zeros();
            if speed > 0.05 && vx.abs() > 0.05 && horizontal_distance > 1.0 {
                let ticks = horizontal_distance / speed;
                if ticks > 1.0 {
                    let accel_y = 2.0 * curl_units / (ticks * ticks);
                    spin.z = accel_y / (SpinModel::MAGNUS_COEFF * vx);
                }
                // A shot struck to rise is struck with the laces over the
                // top of the ball. Scaled by how high the strike was
                // aimed, so a low drive carries none.
                let rise = (struck_apex - 0.8).max(0.0) / 3.0;
                if rise > 0.0 {
                    let dir = Vector3::new(vx / speed, vy / speed, 0.0);
                    spin += SpinModel::from_strike(dir, 0.0, -rise * 0.55, technique_skill);
                }
            }
            // The curl term above is solved by division (`accel_y / vx`),
            // so a shot struck slowly across the face of goal can ask for
            // a rotation no human generates. Clamped for the same reason
            // the cross path is — see `SpinModel::clamped`.
            field.ball.spin = SpinModel::clamped(spin);
        }

        // Shorter flight protection for shots — allows defenders/GK to claim sooner
        field.ball.flags.in_flight_state = 40;

        // Arm the shot-lifecycle census (diagnostic only). See
        // `Ball::census_shot_fate`.
        #[cfg(feature = "match-logs")]
        {
            use std::sync::atomic::Ordering;
            crate::r#match::engine::ball::ball::ownership::reception_diag::FATE_STRUCK
                .fetch_add(1, Ordering::Relaxed);
            crate::r#match::engine::ball::ball::ownership::reception_diag::FATE_STRUCK_DIST_X100
                .fetch_add((horizontal_distance * 100.0) as u64, Ordering::Relaxed);
            field.ball.census_shot_live = true;
            field.ball.census_shot_dist = horizontal_distance;
            field.ball.census_shot_side = shooter_side;
        }

        // Project where the ball will cross the goal line so the
        // defending keeper can commit to an intercept line rather than
        // chasing the ball's current position every tick. Uses
        // constant-velocity projection (close enough over the ~10-20
        // ticks of shot flight; drag and gravity effects are small
        // relative to the error the goalkeeper's own reaction
        // introduces). Without this cache the GK loses ground to the
        // 5.6 u/tick shot and never catches anything — the primary
        // reason saves/on-target sat below 1% in benchmarks.
        let shooter_side = field
            .get_player(shoot_event_model.from_player_id)
            .and_then(|p| p.side);
        if let Some(shooter_side) = shooter_side {
            let field_width = field.size.width as f32;
            let defending_side = match shooter_side {
                PlayerSide::Left => PlayerSide::Right,
                PlayerSide::Right => PlayerSide::Left,
            };
            let goal_line_x = match defending_side {
                PlayerSide::Left => 0.0,
                PlayerSide::Right => field_width,
            };
            let dx = goal_line_x - field.ball.position.x;
            let vx = final_velocity.x;
            // Only cache if the shot is actually heading toward that
            // goal — otherwise projection time is negative or infinite
            // and the cache would mislead the keeper.
            if (dx > 0.0 && vx > 0.1) || (dx < 0.0 && vx < -0.1) {
                let ticks_to_goal = (dx / vx).abs();
                // Keep the strike live for its whole flight.
                //
                // The flat 40 ticks set above is shorter than most shots
                // take to reach the line: at typical struck speeds a
                // 92u (11.5 m) effort is 45-60 ticks in the air. Once
                // the window lapsed, `check_ball_ownership` resumed and
                // any player within 5u simply picked the shot out of the
                // air in open play — no save, no block, no goal, no shot
                // going out. Measured: only 23% of shots ever resolved
                // as a save or a goal, against ~80% that were struck
                // between the posts, and the missing ~58% left no trace
                // in any outcome counter.
                //
                // A struck ball has to resolve AS a shot: saved, blocked,
                // in, or out of play. The 1.3 margin covers the drag and
                // gravity the constant-velocity projection ignores; the
                // floor keeps tap-ins protected and the ceiling stops a
                // mishit dribbling toward goal from freezing possession.
                field.ball.flags.in_flight_state = ((ticks_to_goal * 1.8) as usize).clamp(40, 320);
                let goal_line_y = if deflected {
                    // GK reads the ORIGINAL trajectory until the
                    // deflection happens; the redirect arrives after
                    // they've committed to the line. By storing the
                    // pre-deflection y here, the keeper's velocity()
                    // arrives at the wrong spot and the actual ball
                    // (which uses the deflected `final_velocity` below)
                    // sails past into the far corner.
                    original_y_target
                } else {
                    // Straight-line projection of the LAUNCH velocity —
                    // which, on a curled shot, is not where the ball ends
                    // up. A keeper does read spin, but late and
                    // incompletely: he sets for most of the bend and the
                    // rest beats him. That residual is what makes a curled
                    // finish worth striking, and it is bounded so curl
                    // can't quietly rewrite the save calibration.
                    const KEEPER_CURL_READ: f32 = 0.65;
                    field.ball.position.y
                        + final_velocity.y * ticks_to_goal
                        + curl_units * KEEPER_CURL_READ
                };
                // ── The keeper does not know where it is going ────────
                //
                // This cache is what the goalkeeper states steer onto, so
                // whatever it says is where he will be standing when the
                // ball arrives. Handing him the exact crossing point
                // means `lateral_error` collapses to ~0, `reach_ratio`
                // with it, and every save resolves at `CENTRED_BASE` —
                // the 0.88 "keeper is standing where the ball is going"
                // ceiling. Measured consequence: an 82% population save
                // rate against a real 67%, and it was not the save
                // model's calibration at fault, it was that the model's
                // hardest case never occurred.
                //
                // A keeper reads the striker's body and commits; he is
                // beaten by placement inside a couple of metres all the
                // time. The error shrinks with flight time, because a
                // ball he can watch for longer is one he reads better —
                // so a close-range piledriver beats him more often than a
                // 25-yard effort, which is the right way round.
                let read_error = {
                    let seen = (ticks_to_goal / 120.0).clamp(0.0, 1.0);
                    KEEPER_PLACEMENT_READ * (1.0 - seen * 0.55)
                };
                let goal_line_y = goal_line_y + rng.jitter(0.0, read_error);
                // Arc approximation: z under gravity, from the shared
                // constant rather than a copy of it. The literal here was
                // `0.157` — the pre-fix value — so the moment the physics
                // moved, the keeper's projected crossing height and the
                // ball's actual one would have parted company.
                let goal_line_z = (field.ball.position.z + final_velocity.z * ticks_to_goal
                    - 0.5 * GRAVITY_PER_TICK * ticks_to_goal * ticks_to_goal)
                    .max(0.0);
                // Stamp who struck it, in the units the save contest
                // scores. Read here rather than at save time: the save
                // resolves several ticks downstream, where the shooter
                // is no longer the ball's `previous_owner` on every
                // path and his fatigue bands have already moved on.
                let shooter_threat = field
                    .get_player(shoot_event_model.from_player_id)
                    .map(|p| sc::shot_threat(p, minute))
                    .unwrap_or(SaveModel::NEUTRAL_THREAT);
                // Post-shot expectation for the keeper ledger: what a
                // league-average keeper concedes from THIS strike. Built
                // from the projected crossing point, the ball's speed and
                // its height — the strike, not the man in goal — so the
                // rating can ask "how many did he keep out that an
                // ordinary keeper wouldn't have" instead of assuming
                // every shot on target was worth a league-average chance.
                // Placement is measured from the goal CENTRE, never from
                // the keeper's own position: reading where he stood would
                // let good positioning shrink his own expectation and
                // cancel the advantage it earned him.
                field.ball.last_shot_xgot = SaveModel::expected_goal_on_target(
                    goal_line_y - field.size.height as f32 * 0.5,
                    final_velocity.norm(),
                    goal_line_z,
                );
                // Positional-defending credit. Every other defensive
                // counter in the model is an EVENT — a tackle, an
                // interception, a clearance — and a defender who defends
                // by being in the right place produces none of them.
                // This is the one that pays for the position itself:
                // goal-side of the ball and inside the lane the shot is
                // travelling down, measured at the moment it is struck.
                // See `ZoneStats::shots_covered_in_position`.
                {
                    let ball_x = field.ball.position.x;
                    let ball_y = field.ball.position.y;
                    let span_x = goal_line_x - ball_x;
                    let span_y = goal_line_y - ball_y;
                    let span = (span_x * span_x + span_y * span_y).sqrt().max(1.0);
                    for p in field.players.iter_mut() {
                        if p.side != Some(defending_side)
                            || p.tactical_position.current_position.position_group()
                                == PlayerFieldPositionGroup::Goalkeeper
                        {
                            continue;
                        }
                        let goal_side = match defending_side {
                            PlayerSide::Left => p.position.x < ball_x,
                            PlayerSide::Right => p.position.x > ball_x,
                        };
                        if !goal_side {
                            continue;
                        }
                        let cross = ((p.position.x - ball_x) * span_y
                            - (p.position.y - ball_y) * span_x)
                            .abs();
                        if cross / span <= COVER_LANE_HALF_WIDTH {
                            p.statistics.note_shot_covered_in_position();
                        }
                    }
                }
                #[cfg(feature = "match-logs")]
                {
                    // Defensive picture at the strike — see `block_diag`.
                    let ball_x = field.ball.position.x;
                    let ball_y = field.ball.position.y;
                    let ball_to_goal = (goal_line_x - ball_x).abs();
                    let (mut goalside, mut near_line) = (0u64, 0u64);
                    let (mut def_seen, mut def_depth_sum) = (0u64, 0.0f32);
                    for p in &field.players {
                        if p.side != Some(defending_side)
                            || p.tactical_position.current_position.position_group()
                                == PlayerFieldPositionGroup::Goalkeeper
                        {
                            continue;
                        }
                        // How deep the BACK LINE actually is, whether or
                        // not it is goal-side of the ball — this is what
                        // says "the line never dropped" as opposed to
                        // "the line dropped but scattered". Defender
                        // group only: averaging all ten outfielders lets
                        // their forwards, camped on the halfway line,
                        // dominate the mean and hide the back four
                        // entirely.
                        if p.tactical_position.current_position.position_group()
                            == PlayerFieldPositionGroup::Defender
                        {
                            def_seen += 1;
                            def_depth_sum += (goal_line_x - p.position.x).abs();
                            if let PlayerState::Defender(ds) = p.state {
                                BlockDiag::note_defender_state(ds as usize);
                            }
                        }
                        let is_goalside = match defending_side {
                            PlayerSide::Left => p.position.x < ball_x,
                            PlayerSide::Right => p.position.x > ball_x,
                        };
                        if !is_goalside {
                            continue;
                        }
                        goalside += 1;
                        // Perpendicular offset from the straight line the
                        // ball is taking to its target.
                        let span_x = goal_line_x - ball_x;
                        let span_y = goal_line_y - ball_y;
                        let span = (span_x * span_x + span_y * span_y).sqrt().max(1.0);
                        let cross = ((p.position.x - ball_x) * span_y
                            - (p.position.y - ball_y) * span_x)
                            .abs();
                        if cross / span <= 30.0 {
                            near_line += 1;
                        }
                    }
                    BlockDiag::note_strike(
                        goalside,
                        near_line,
                        ball_to_goal,
                        if def_seen == 0 {
                            0.0
                        } else {
                            def_depth_sum / def_seen as f32
                        },
                    );
                }
                field.ball.cached_shot_target = Some(ShotTarget {
                    goal_line_y,
                    goal_line_z,
                    defending_side,
                    deflected,
                    save_rolled: false,
                    block_rolled: false,
                    shooter_threat,
                });
            } else {
                #[cfg(feature = "match-logs")]
                crate::r#match::engine::ball::ball::ownership::reception_diag::SHOT_NO_TARGET
                    .fetch_add(1, Ordering::Relaxed);
                field.ball.cached_shot_target = None;
            }
        } else {
            #[cfg(feature = "match-logs")]
            crate::r#match::engine::ball::ball::ownership::reception_diag::SHOT_NO_TARGET
                .fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Credit a goalkeeper save for a parried shot — the GK touched the
    /// ball mid-flight but couldn't claim it cleanly. Emitted from the
    /// diving and catching states when they exit on "ball moving away"
    /// while `cached_shot_target` was set. The same `cached_shot_target`
    /// is then cleared so the eventual rest position (out of bounds,
    /// to a defender, or back into play) doesn't double-credit anyone.
    #[cfg(feature = "match-logs")]
    #[inline]
    fn dbg_save_credit(site: usize, had_shooter: bool, prev_owner_none: bool, shooter_found: bool) {
        use std::sync::atomic::Ordering;
        save_accounting_stats::SAVES_CREDITED[site].fetch_add(1, Ordering::Relaxed);
        save_accounting_stats::SHOTS_FACED_INC[site].fetch_add(1, Ordering::Relaxed);
        if shooter_found {
            save_accounting_stats::ON_TARGET_PAIRED[site].fetch_add(1, Ordering::Relaxed);
        }
        if !had_shooter || prev_owner_none {
            save_accounting_stats::PREVIOUS_OWNER_NONE[site].fetch_add(1, Ordering::Relaxed);
        }
        if had_shooter && !shooter_found {
            save_accounting_stats::SHOOTER_MISSING[site].fetch_add(1, Ordering::Relaxed);
        }
    }

    fn handle_parried_ball_event(player_id: u32, field: &mut MatchField) {
        // Only credit when the ball was a real shot — guards against
        // the diving state calling this when the GK gave up on a long
        // pass. The state-machine emitters are gated on the same flag,
        // so this is belt-and-braces.
        if field.ball.cached_shot_target.is_none() {
            return;
        }
        let shooter_id = field.ball.previous_owner;
        let shot_xg = field.ball.last_shot_xgot;
        if let Some(gk) = field.get_player_mut(player_id) {
            gk.statistics.note_shot_faced(shot_xg, true);
        }
        field.ball.clear_shot_metadata();
        field.ball.pending_error_to_shot_player_id = None;
        // Credit on-target to the shooter — a parry IS the keeper
        // touching a shot that reached the goal frame. Without this,
        // saves > on-target shots, an impossible ratio.
        let mut shooter_found = false;
        if let Some(sid) = shooter_id {
            if let Some(shooter) = field.get_player_mut(sid) {
                shooter.memory.credit_shot_on_target();
                shooter_found = true;
            }
        }
        #[cfg(feature = "match-logs")]
        Self::dbg_save_credit(0, shooter_id.is_some(), shooter_id.is_none(), shooter_found);
        let _ = shooter_found;
        field.ball.cached_shot_target = None;
    }

    fn handle_caught_ball_event(player_id: u32, field: &mut MatchField, context: &MatchContext) {
        // You cannot catch a ball you are not near. Checked BEFORE the save
        // credit below, deliberately: a catch resolved while the ball was
        // still metres away was being booked as a save for a shot the
        // keeper never reached, which is how saves/on-target climbed above
        // its calibrated 67%.
        if !Self::can_take_possession(player_id, field) {
            return;
        }

        // Detect saves: ball was moving and came from an opponent
        let ball_was_moving = field.ball.velocity.norm_squared() > 0.25;
        let last_owner_team = field.ball.previous_owner.and_then(|prev_id| {
            field
                .players
                .iter()
                .find(|p| p.id == prev_id)
                .map(|p| p.team_id)
        });
        let gk_team = field
            .players
            .iter()
            .find(|p| p.id == player_id)
            .map(|p| p.team_id);
        let opponent_ball =
            ball_was_moving && last_owner_team.is_some() && last_owner_team != gk_team;

        // Save credit requires both: the ball was moving from an opponent
        // AND the catch resolves a real shot (cached_shot_target set).
        // Without the shot gate, every cross / through-ball / clearance
        // that ends in the keeper's hands counted as a save — pushing
        // saves/on-target above 100% (more "saves" than on-target shots).
        if opponent_ball {
            let was_shot = field.ball.cached_shot_target.is_some();
            if was_shot {
                let shooter_id = field.ball.previous_owner;
                let shot_xg = field.ball.last_shot_xgot;
                if let Some(player) = field.get_player_mut(player_id) {
                    player.statistics.note_shot_faced(shot_xg, true);
                }
                let mut shooter_found = false;
                if let Some(sid) = shooter_id {
                    if let Some(shooter) = field.get_player_mut(sid) {
                        shooter.memory.credit_shot_on_target();
                        shooter_found = true;
                    }
                }
                #[cfg(feature = "match-logs")]
                Self::dbg_save_credit(1, shooter_id.is_some(), shooter_id.is_none(), shooter_found);
                let _ = shooter_found;
                field.ball.clear_shot_metadata();
                field.ball.pending_error_to_shot_player_id = None;
            } else if Self::handle_gk_claim(player_id, field, context) {
                // Claim flapped — the keeper does NOT get the ball. The
                // spill is already staged on the ball, so fall through
                // without taking ownership; `GoalkeeperHoldingState`
                // drops him back to Standing next tick when it finds he
                // has nothing in his hands.
                return;
            }
        }

        field.ball.previous_owner = field.ball.current_owner;
        field.ball.current_owner = Some(player_id);
        // Ball must stop when caught — prevent it from continuing into the goal
        field.ball.velocity = Vector3::zeros();
        field.ball.flags.in_flight_state = 0;
        // It is in his GLOVES, which the rest of the engine can now see:
        // no claim, no tackle, no interception, and no point pressing him.
        // The `claim_cooldown` below used to be the whole of that
        // protection — a timer that merely made stealing the ball off a
        // keeper's hands unlikely for a while rather than impossible.
        if let Some(team_id) = field.get_player(player_id).map(|p| p.team_id) {
            field
                .ball
                .gather_in_hands(player_id, team_id, context.current_tick());
        }
        // Belt and braces behind `held_in_hands`: covers the hold (up to
        // ~5.5 s) plus the distribution that follows it.
        field.ball.claim_cooldown = 700;
        field.ball.pass_target_player_id = None;
        // Shot is dead — clear the projected intercept so the keeper
        // doesn't keep chasing a ghost target next tick.
        field.ball.cached_shot_target = None;
    }

    /// Resolve a goalkeeper's attempt to claim a non-shot ball from the
    /// opposition — a cross, a high ball into the box, a through-ball he
    /// comes to collect.
    ///
    /// Returns `true` if the claim was FLAPPED, in which case the ball has
    /// been spilled and the caller must not hand the keeper possession.
    ///
    /// Two things happen here that used to be unconditional. The command-
    /// action credit is now earned rather than granted (see
    /// [`GkClaimContest`] on why ~15 gathers a match were being counted as
    /// commanding the box), and the claim can fail — which is what makes
    /// `gk_failed_claims_to_shot` / `_to_goal` reachable at last.
    fn handle_gk_claim(player_id: u32, field: &mut MatchField, context: &MatchContext) -> bool {
        let is_keeper = field
            .players
            .iter()
            .find(|p| p.id == player_id)
            .map(|p| {
                p.tactical_position.current_position.position_group()
                    == PlayerFieldPositionGroup::Goalkeeper
            })
            .unwrap_or(false);
        if !is_keeper {
            return false;
        }

        let situation = GkClaimContest::assess(field, player_id);
        #[cfg(feature = "match-logs")]
        gk_claim_diag::GATHERS.fetch_add(1, Ordering::Relaxed);
        // Does this gather read as a command-of-area moment at all? The
        // weight IS the probability — no threshold, so a half-contested
        // ball counts half the time.
        let is_command_moment = context.rng.unit_f32() < situation.command_weight;
        if !is_command_moment {
            // Routine collection: no credit, no contest, keeper gathers.
            return false;
        }
        #[cfg(feature = "match-logs")]
        gk_claim_diag::COMMAND_MOMENTS.fetch_add(1, Ordering::Relaxed);

        let minute = sc::minute_from_ticks(field.ball.current_tick_cached);
        let failure = GkClaimContest::failure_probability(field, player_id, &situation, minute);
        if context.rng.unit_f32() >= failure {
            if let Some(gk) = field.get_player_mut(player_id) {
                gk.statistics.note_gk_command_action();
            }
            return false;
        }
        #[cfg(feature = "match-logs")]
        gk_claim_diag::FLAPS.fetch_add(1, Ordering::Relaxed);

        // Flapped it. The ball has to end up somewhere an attacker can
        // reach: a spill that dies at the keeper's feet is one he simply
        // picks up again, which is what made the first version of this
        // contest produce flaps that never led to a single shot.
        //
        // So it is pushed 10-26u (1.2-3.2m) OUT from the goal — the
        // punched-cross trajectory, landing around the six-yard line and
        // the penalty spot where the box contest actually happens — with
        // the keeper as last toucher so `try_intercept` treats the loose
        // ball as an attacking opportunity rather than a keeper's pass.
        let (keeper_pos, keeper_team, keeper_side) = field
            .players
            .iter()
            .find(|p| p.id == player_id)
            .map(|p| (p.position, p.team_id, p.side))
            .unwrap_or((field.ball.position, 0, None));
        let tick = field.ball.current_tick_cached;
        let out_distance = context.rng.range_f32(10.0, 26.0);
        let lateral = context.rng.range_f32(-14.0, 14.0);
        let out_x = match keeper_side {
            Some(PlayerSide::Left) => keeper_pos.x + out_distance,
            Some(PlayerSide::Right) => keeper_pos.x - out_distance,
            None => keeper_pos.x,
        };
        let drop = Vector3::new(out_x, keeper_pos.y + lateral, 0.0);
        // Above 1.0 u/tick on purpose: `try_intercept` ignores balls
        // whose speed² is under 1.0, so a slower spill is not a loose
        // ball anybody can attack — it just rolls until the nearest
        // player (usually the keeper himself) picks it up.
        let drop_speed = context.rng.range_f32(1.1, 1.8);
        let ball = &mut field.ball;
        let dx = drop.x - keeper_pos.x;
        let dy = drop.y - keeper_pos.y;
        let dist = (dx * dx + dy * dy).sqrt().max(1.0);
        ball.position = keeper_pos;
        ball.position.z = 0.0;
        ball.velocity.x = dx / dist * drop_speed;
        ball.velocity.y = dy / dist * drop_speed;
        ball.velocity.z = 0.0;
        ball.current_owner = None;
        ball.previous_owner = Some(player_id);
        ball.pass_target_player_id = None;
        ball.pending_pass_passer = None;
        ball.pending_pass_was_cross = false;
        ball.flags.in_flight_state = 10;
        ball.claim_cooldown = 0;
        // Arm the rebound window so the follow-up shot isn't killed by
        // the team shot-spacing gate, and stamp the pending failed claim
        // so a shot / goal from this possession is charged to the keeper.
        ball.last_rebound_tick = tick;
        ball.pending_failed_claim_gk_id = Some(player_id);
        ball.pending_failed_claim_tick = tick;
        ball.pending_failed_claim_charged = false;
        ball.record_touch(player_id, keeper_team, tick, false);
        true
    }

    fn handle_move_player_event(player_id: u32, position: Vector3<f32>, field: &mut MatchField) {
        // Stale event ids (sent-off player, halftime swap mid-tick)
        // are no-ops rather than crashes.
        if let Some(player) = field.get_player_mut(player_id) {
            player.position = position;
        }
    }

    /// Take a player off the ground. `MatchPlayer::leap` is a no-op for
    /// anyone already airborne, so a repeated request during a contest
    /// cannot hold him up there.
    fn handle_leap_event(player_id: u32, apex: f32, field: &mut MatchField) {
        if let Some(player) = field.get_player_mut(player_id) {
            player.leap(apex);
        }
    }

    fn handle_take_ball_event(player_id: u32, field: &mut MatchField) {
        if let Some(player) = field.get_player_mut(player_id) {
            player.run_for_ball();
        }
    }

    fn handle_request_ball_receive(player_id: u32, field: &mut MatchField, context: &MatchContext) {
        // Only allow if ball is close and either unowned or this player is the target
        let is_target = field.ball.pass_target_player_id == Some(player_id);
        let is_unowned = field.ball.current_owner.is_none();

        if !is_target && !is_unowned {
            return;
        }

        // Copy ball position to avoid borrow conflict
        let ball_pos = field.ball.position;

        let player = match field.get_player(player_id) {
            Some(p) => p,
            None => return,
        };

        let dx = player.position.x - ball_pos.x;
        let dy = player.position.y - ball_pos.y;
        let distance = (dx * dx + dy * dy).sqrt();

        if distance < 3.5 && ball_pos.z <= 2.8 {
            // Taking control settles the live pass window — see
            // `resolve_pending_pass_on_control`.
            Self::resolve_pending_pass_on_control(player_id, field, context);
            field.ball.previous_owner = field.ball.current_owner;
            field.ball.current_owner = Some(player_id);
            field.ball.pass_target_player_id = None;
            field.ball.ownership_duration = 0;
            field.ball.claim_cooldown = 15;
            field.ball.flags.in_flight_state = 0;
        }
    }

    fn handle_commit_foul_event(
        fouler_id: u32,
        severity: FoulSeverity,
        field: &mut MatchField,
        context: &mut MatchContext,
    ) {
        // Referee marginal-call gate. The tackling code emits a foul
        // whenever a contact passes its `committed_foul` roll; the
        // referee then decides whether to actually blow the whistle.
        // Clear / violent fouls are near-certain to be called; normal
        // contact is genuinely referee-dependent (a strict referee
        // calls more, a lenient one waves play on). The card decision
        // is computed only when the whistle goes — a missed call costs
        // nobody a yellow.
        // Foul-source census: which STATE emitted this contact, and was
        // the ball in the fouler's own box at the time (i.e. is this a
        // penalty candidate). Recorded before the referee gate so the
        // emitted rate and the whistled rate can be told apart. Restraining
        // the tackle model cut tackles 22.4 → 18.3 and moved the penalty
        // rate by nothing at all, which means the box fouls are being
        // emitted somewhere else — this says where.
        #[cfg(feature = "match-logs")]
        if let Some(f) = field.players.iter().find(|p| p.id == fouler_id) {
            let in_box = f
                .side
                .map(|s| {
                    context
                        .penalty_area(s == PlayerSide::Left)
                        .contains(&field.ball.position)
                })
                .unwrap_or(false);
            FoulCensus::note(f.state.compact_id(), in_box);
        }

        let call_ctx = FoulResolver::build_call_context(fouler_id, severity, field, context);
        let call_prob = context
            .referee
            .foul_call_prob(&context.environment, call_ctx);
        if context.rng.unit_f32() >= call_prob {
            // Missed / waved-on contact. Nothing is recorded: no foul
            // stat, no card, no restart. Play continues. This is the
            // marginal-call behaviour that lets a lenient referee
            // produce a low-foul match without changing the underlying
            // tackle / press model.
            return;
        }

        // Compute card probabilities up front — used either to record
        // the card immediately or to stash on a pending advantage.
        let match_second = context.total_match_time;
        let card_modifier = context.referee.card_modifier(&context.environment);
        let (card_yellow_prob, card_red_prob) =
            match Self::compute_card_probs(fouler_id, severity, field, match_second, card_modifier)
            {
                Some(p) => p,
                None => return,
            };

        // Advantage: if the fouled team kept possession, the ball is in
        // a good attacking position, and the foul wasn't violent, the
        // referee waves play on. The pending advantage is stashed on
        // context; the per-tick advantage check resolves it later.
        // Violent fouls always stop play — `should_play_advantage`
        // hard-blocks them on severity >= 0.85.
        let severity_score = match severity {
            FoulSeverity::Normal => 0.20,
            FoulSeverity::Reckless => 0.60,
            FoulSeverity::Violent => 0.90,
        };
        let (fouled_team_id, attack_value, possession_retained) =
            Self::estimate_advantage_inputs(fouler_id, field);
        if let Some(fouled_team_id) = fouled_team_id {
            if context.referee.should_play_advantage(
                attack_value,
                possession_retained,
                severity_score,
            ) {
                let start_tick = context.current_tick();
                let window = context.referee.advantage_window_ticks() as u64;
                context.pending_advantage = Some(PendingAdvantage {
                    fouler_id,
                    start_tick,
                    expire_tick: start_tick + window,
                    fouled_team_id,
                    severity,
                    yellow_prob: card_yellow_prob,
                    red_prob: card_red_prob,
                });
                // Foul still counts for the player's fouls_committed
                // tally even though play continues.
                if let Some(p) = field.get_player_mut(fouler_id) {
                    p.fouls_committed = p.fouls_committed.saturating_add(1);
                    p.statistics.add_foul(match_second);
                }
                return;
            }
        }

        // No advantage — proceed to the immediate restart / card path.
        // Free-kick protection: the victim gets time without being challenged.
        if field.ball.current_owner.is_some() {
            field.ball.claim_cooldown = 150; // ~2.5 seconds of protection
            field.ball.flags.in_flight_state = 150;
            field.ball.contested_claim_count = 0;
        }
        Self::award_restart_for_foul(fouler_id, severity, field, context);

        // Count the foul for the player (advantage path counted it above).
        if let Some(p) = field.get_player_mut(fouler_id) {
            p.fouls_committed = p.fouls_committed.saturating_add(1);
            p.statistics.add_foul(match_second);
        }

        Self::apply_card_decision(
            fouler_id,
            severity,
            card_yellow_prob,
            card_red_prob,
            field,
            context,
        );
    }

    /// Compute the (yellow_prob, red_prob) pair for the foul. Reads
    /// the fouler's skills/personality/foul history. Returns None if
    /// the fouler can't be found (red-card race condition).
    fn compute_card_probs(
        fouler_id: u32,
        severity: FoulSeverity,
        field: &MatchField,
        _match_second: u64,
        card_modifier: f32,
    ) -> Option<(f32, f32)> {
        let player = field.players.iter().find(|p| p.id == fouler_id)?;

        let aggression = player.skills.mental.aggression / 20.0;
        let composure = player.skills.mental.composure / 20.0;
        let teamwork = player.skills.mental.teamwork / 20.0;
        let dirtiness = player.attributes.dirtiness / 20.0;
        let temperament = player.attributes.temperament / 20.0;
        let sportsmanship = player.attributes.sportsmanship / 20.0;

        let persistent = if player.fouls_committed >= 3 {
            0.15
        } else {
            0.0
        };

        // Second-booking reluctance: real referees demand a clearly
        // card-worthy offence before showing a booked player the
        // second yellow — game management, not box-ticking. Without
        // this, mechanically-booked repeat foulers pushed reds to
        // ~0.65/match (real ~0.15) once the foul-rate fix landed.
        // Damping deepened 0.40 -> 0.15. The note above records reds
        // hitting 0.65/match the last time the foul rate rose; fouls have
        // since roughly doubled again (a second, non-tackle foul source
        // was added), and 0.40 put them straight back at 0.60. A booked
        // player picking up a second is genuinely rare — real football
        // sends off ~0.15-0.20 a match against ~7 bookings.
        let booked_damp = if player.yellow_cards >= 1 { 0.06 } else { 1.0 };

        let aggressor_factor = (aggression * 0.40 - composure * 0.12 - teamwork * 0.08
            + dirtiness * 0.18
            + (1.0 - temperament) * 0.10
            - sportsmanship * 0.10)
            .clamp(-0.25, 0.70);

        let (y, r) = match severity {
            FoulSeverity::Normal => (
                (0.04 + aggressor_factor * 0.18 + persistent * 0.6).clamp(0.02, 0.18),
                0.0_f32,
            ),
            // ⚠ RECKLESS IS A CAUTION. In the Laws that word is the
            // DEFINITION of a yellow card — "acts with disregard to the
            // danger to, or consequences for, an opponent" — and the
            // sending-off tier is the separate one above it, "using
            // excessive force or endangering the safety of an opponent",
            // which this engine models as `Violent`. A straight red for
            // the last genuine red offence, denying an obvious goalscoring
            // opportunity, is modelled separately again in
            // `officiating::management::professional_foul`.
            //
            // So a direct-red band on THIS tier is a third route to a
            // sending-off that the Laws do not have, and it was producing
            // most of them. Measured with the `CARD SOURCE CENSUS`:
            // **REDS 0.395/match = second-yellow 0.090 + direct-reckless
            // 0.285 + direct-violent 0.020** against a real 0.15-0.20.
            // 6.79 reckless fouls are whistled a match and each carried a
            // 1-8% red, so this one band out-produced every legitimate
            // route combined — 72% of all red cards in the game.
            //
            // The ceiling had already been cut 0.18 → 0.08 in the 2026-06
            // discipline round on exactly this reasoning ("reckless is
            // yellow-card territory... direct reds from a reckless (not
            // violent) challenge are rare"); it needed to go to a token
            // tail, not a smaller tax. What survives is the borderline
            // case a referee upgrades on the day.
            FoulSeverity::Reckless => (
                (0.45 + aggressor_factor * 0.30 + persistent * 0.8).clamp(0.30, 0.85),
                (0.005 + aggressor_factor * 0.018 + persistent * 0.1).clamp(0.002, 0.020),
            ),
            FoulSeverity::Violent => (0.10_f32, (0.70 + aggressor_factor * 0.30).clamp(0.60, 1.0)),
        };
        let (lo_y, hi_y, lo_r, hi_r) = match severity {
            FoulSeverity::Normal => (0.02, 0.18, 0.0, 0.0),
            FoulSeverity::Reckless => (0.30, 0.85, 0.002, 0.020),
            FoulSeverity::Violent => (0.0, 0.20, 0.60, 1.0),
        };
        Some((
            (y * card_modifier * booked_damp).clamp(lo_y * booked_damp, hi_y),
            (r * card_modifier).clamp(lo_r, hi_r),
        ))
    }

    /// Estimate the post-foul attack value for the referee's advantage
    /// check. Returns the fouled team's id (None if it can't be
    /// inferred), an attack_value in 0..1, and whether the fouled team
    /// currently retains possession.
    fn estimate_advantage_inputs(fouler_id: u32, field: &MatchField) -> (Option<u32>, f32, bool) {
        let fouler = match field.players.iter().find(|p| p.id == fouler_id) {
            Some(p) => p,
            None => return (None, 0.0, false),
        };
        let fouler_team = fouler.team_id;
        let owner_team = field
            .ball
            .current_owner
            .and_then(|id| field.players.iter().find(|p| p.id == id))
            .map(|p| p.team_id);
        let fouled_team_id = if let Some(t) = owner_team {
            if t == fouler_team { None } else { Some(t) }
        } else if fouler_team == field.home_team_id {
            Some(field.away_team_id)
        } else {
            Some(field.home_team_id)
        };
        let possession_retained = owner_team == fouled_team_id;
        let attack_value = if let Some(team) = fouled_team_id {
            let bx = field.ball.position.x;
            let target_right = team == field.home_team_id;
            let progress = if target_right {
                (bx / (field.size.width as f32)).clamp(0.0, 1.0)
            } else {
                (1.0 - bx / (field.size.width as f32)).clamp(0.0, 1.0)
            };
            let base = 0.30 + progress * 0.40;
            let center_y = field.size.height as f32 / 2.0;
            let centrality =
                1.0 - ((field.ball.position.y - center_y).abs() / center_y).clamp(0.0, 1.0);
            let nudge = 0.05 * centrality;
            (base + nudge).clamp(0.0, 1.0)
        } else {
            0.0
        };
        (fouled_team_id, attack_value, possession_retained)
    }

    /// Apply the rolled card decision (yellow / red / second-yellow)
    /// to the fouler. Shared between the immediate-whistle path and
    /// the advantage-resolution path.
    pub(crate) fn apply_card_decision(
        fouler_id: u32,
        severity: FoulSeverity,
        card_yellow_prob: f32,
        card_red_prob: f32,
        field: &mut MatchField,
        context: &mut MatchContext,
    ) {
        let match_second = context.total_match_time;
        let roll_red = context.rng.unit_f32();
        let roll_yellow = context.rng.unit_f32();

        let direct_red = roll_red < card_red_prob;
        let got_yellow = !direct_red && roll_yellow < card_yellow_prob;

        #[cfg(feature = "match-logs")]
        {
            use crate::mid_run_diag::CardDiag;
            CardDiag::note(0);
            CardDiag::note(match severity {
                FoulSeverity::Normal => 5,
                FoulSeverity::Reckless => 6,
                FoulSeverity::Violent => 7,
            });
            if direct_red {
                CardDiag::note(match severity {
                    FoulSeverity::Violent => 4,
                    _ => 3,
                });
            }
        }

        if !direct_red && !got_yellow {
            return;
        }

        // Re-enabled red cards. The unified foul model + GK violent
        // recalibration brought foul frequency into the spec band, so
        // sending players off no longer cascades into half-empty teams.
        // Direct reds and second yellows both fully send the fouler off.
        let (second_yellow, ends_with_red, temperament_for_psych) = {
            let player = match field.get_player_mut(fouler_id) {
                Some(p) => p,
                None => return,
            };
            let temperament = player.attributes.temperament;
            if direct_red {
                player.statistics.add_red_card(match_second);
                player.is_sent_off = true;
                context.record_stoppage_time(45_000);
                (false, true, temperament)
            } else {
                player.yellow_cards = player.yellow_cards.saturating_add(1);
                player.statistics.add_yellow_card(match_second);
                context.record_stoppage_time(15_000);
                let promoted = player.yellow_cards >= 2;
                #[cfg(feature = "match-logs")]
                crate::mid_run_diag::CardDiag::note(if promoted { 2 } else { 1 });
                if promoted {
                    player.statistics.add_red_card(match_second);
                    player.is_sent_off = true;
                    context.record_stoppage_time(45_000);
                }
                (promoted, promoted, temperament)
            }
        };

        // Psychology: a yellow raises nervousness (low-temperament
        // players take a bigger hit); both yellow and red drop
        // confidence via `record_negative`. Tick taken from the
        // outer event so the per-event ordering is preserved.
        let psych_tick = context.current_tick();
        context
            .psychology
            .apply_yellow_card(fouler_id, temperament_for_psych);
        context
            .psychology
            .record_negative(fouler_id, NegativeEvent::YellowCard, psych_tick);

        if ends_with_red {
            // Transfer ball ownership back to a neutral state so the
            // opposing side can restart. Zero the fouler's velocity and
            // park them off the pitch so distance / collision checks stop
            // treating them as an active participant.
            if field.ball.current_owner == Some(fouler_id) {
                field.ball.previous_owner = field.ball.current_owner;
                field.ball.current_owner = None;
                field.ball.pass_target_player_id = None;
            }

            // Capture team id before we stash the player off-pitch so we
            // can reshape teammates afterwards.
            let team_id = field.get_player_mut(fouler_id).map(|p| p.team_id);

            if let Some(player) = field.get_player_mut(fouler_id) {
                player.velocity = Vector3::zeros();
                // Stash them well beyond the sideline. Physics updates
                // still run but no one is close enough to interact.
                player.position = Vector3::new(-500.0, -500.0, 0.0);
            }

            // Compact the surviving team's shape — surviving players drop
            // deeper and narrower to cover the numerical disadvantage.
            if let Some(tid) = team_id {
                field.compact_after_dismissal(tid);
            }
            // Roster effectively changed (one fewer active player) —
            // invalidate the cached team skill composites.
            context.invalidate_skill_aggregates();

            if second_yellow {
                debug!(
                    "Second yellow → red: player {} at {}s (severity {:?})",
                    fouler_id, match_second, severity
                );
            } else {
                debug!(
                    "Direct red: player {} at {}s (severity {:?})",
                    fouler_id, match_second, severity
                );
            }
        } else {
            debug!(
                "Yellow card: player {} at {}s (severity {:?})",
                fouler_id, match_second, severity
            );
        }
    }

    /// Build an offside snapshot at pass-kick. The actual offside call
    /// fires later, when the receiver becomes active — see
    /// `evaluate_offside_snapshot`.
    fn build_offside_snapshot(
        passer_id: u32,
        receiver_id: u32,
        origin: PassOriginRestart,
        field: &MatchField,
        tick: u64,
    ) -> Option<OffsideSnapshot> {
        let passer = field.players.iter().find(|p| p.id == passer_id)?;
        let receiver = field.players.iter().find(|p| p.id == receiver_id)?;
        let passer_side = passer.side?;
        let receiver_side = receiver.side?;
        // Passes between players on different sides shouldn't happen,
        // but if it does (substitution race) skip the snapshot.
        if passer_side != receiver_side {
            return None;
        }
        let half_width = field.size.half_width as f32;
        let in_opponent_half = match receiver_side {
            PlayerSide::Left => receiver.position.x > half_width,
            PlayerSide::Right => receiver.position.x < half_width,
        };
        if !in_opponent_half {
            // Offside can only occur in the opponent half — no snapshot
            // needed.
            return None;
        }

        let opponent_xs: Vec<f32> = match receiver_side {
            PlayerSide::Left => field
                .players
                .iter()
                .filter(|p| p.side == Some(PlayerSide::Right))
                .map(|p| p.position.x)
                .collect(),
            PlayerSide::Right => field
                .players
                .iter()
                .filter(|p| p.side == Some(PlayerSide::Left))
                .map(|p| p.position.x)
                .collect(),
        };
        let mut sorted_xs = opponent_xs;
        // For Left attackers, defenders' goal is at x=field_width — so
        // sort DESCENDING (closest to their goal first). For Right
        // attackers, ASCENDING.
        match receiver_side {
            PlayerSide::Left => {
                sorted_xs.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal))
            }
            PlayerSide::Right => {
                sorted_xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            }
        }
        if sorted_xs.len() < 2 {
            return None;
        }
        let second_last = sorted_xs[1];

        Some(OffsideSnapshot {
            origin,
            passer_id,
            passer_side,
            receiver_id,
            ball_x_at_kick: field.ball.position.x,
            second_last_defender_x: second_last,
            receiver_x_at_kick: receiver.position.x,
            receiver_y_at_kick: receiver.position.y,
            set_tick: tick,
        })
    }

    /// Decide whether the snapshot represents an offside position.
    /// Tolerance 1.5u to absorb foot-vs-shoulder ambiguity. Kept for
    /// callers that want a free function rather than the snapshot
    /// method; the snapshot's `is_offside` is the canonical version.
    #[allow(dead_code)]
    pub(crate) fn snapshot_is_offside(snap: &OffsideSnapshot) -> bool {
        const TOLERANCE: f32 = 1.5;
        match snap.passer_side {
            PlayerSide::Left => {
                if snap.receiver_x_at_kick <= snap.ball_x_at_kick + TOLERANCE {
                    return false;
                }
                snap.receiver_x_at_kick > snap.second_last_defender_x + TOLERANCE
            }
            PlayerSide::Right => {
                if snap.receiver_x_at_kick >= snap.ball_x_at_kick - TOLERANCE {
                    return false;
                }
                snap.receiver_x_at_kick < snap.second_last_defender_x - TOLERANCE
            }
        }
    }

    /// Legacy direct check kept for any in-tree callers that still want
    /// pass-creation-time offside (none should remain after the delayed
    /// resolver).
    #[allow(dead_code)]
    fn is_receiver_offside(receiver_id: u32, passer_id: u32, field: &MatchField) -> bool {
        let receiver = match field.players.iter().find(|p| p.id == receiver_id) {
            Some(p) => p,
            None => return false,
        };

        // Verify passer exists
        if !field.players.iter().any(|p| p.id == passer_id) {
            return false;
        }

        let receiver_side = match receiver.side {
            Some(s) => s,
            None => return false,
        };

        let half_width = field.size.half_width as f32;
        let ball_x = field.ball.position.x;
        let receiver_x = receiver.position.x;

        // Tolerance to avoid marginal false positives
        const TOLERANCE: f32 = 1.0;

        match receiver_side {
            PlayerSide::Left => {
                // Left side attacks right: opponent goal at x = field_width
                // Must be in opponent's half (past halfway)
                if receiver_x < half_width {
                    return false;
                }
                // Must be ahead of the ball (closer to opponent goal)
                if receiver_x <= ball_x + TOLERANCE {
                    return false;
                }
                // Collect all opponents (Right side players)
                // Right side's own goal is at x = field_width
                // Sort DESCENDING so [0] = closest to their goal (GK), [1] = second-to-last
                let mut opponent_xs: Vec<f32> = field
                    .players
                    .iter()
                    .filter(|p| p.side == Some(PlayerSide::Right))
                    .map(|p| p.position.x)
                    .collect();
                opponent_xs.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));

                if opponent_xs.len() < 2 {
                    return false;
                }
                let second_last_x = opponent_xs[1];

                // Offside if receiver is beyond (greater x) the second-to-last opponent
                receiver_x > second_last_x + TOLERANCE
            }
            PlayerSide::Right => {
                // Right side attacks left: opponent goal at x = 0
                // Must be in opponent's half (before halfway)
                if receiver_x > half_width {
                    return false;
                }
                // Must be ahead of the ball (closer to opponent goal, i.e. smaller x)
                if receiver_x >= ball_x - TOLERANCE {
                    return false;
                }
                // Collect all opponents (Left side players)
                // Left side's own goal is at x = 0
                // Sort ASCENDING so [0] = closest to their goal (GK), [1] = second-to-last
                let mut opponent_xs: Vec<f32> = field
                    .players
                    .iter()
                    .filter(|p| p.side == Some(PlayerSide::Left))
                    .map(|p| p.position.x)
                    .collect();
                opponent_xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

                if opponent_xs.len() < 2 {
                    return false;
                }
                let second_last_x = opponent_xs[1];

                // Offside if receiver is beyond (smaller x) the second-to-last opponent
                receiver_x < second_last_x - TOLERANCE
            }
        }
    }

    /// Handle an offside event: stop the ball, award a free kick to the nearest opponent.
    fn handle_offside_event(
        offside_player_id: u32,
        position: Vector3<f32>,
        field: &mut MatchField,
    ) {
        // Increment offside stat on the player
        if let Some(player) = field.players.iter_mut().find(|p| p.id == offside_player_id) {
            player.statistics.offsides += 1;
        }

        // Determine the offside player's side to find opponents
        let offside_side = field
            .players
            .iter()
            .find(|p| p.id == offside_player_id)
            .and_then(|p| p.side);

        // Find nearest opponent to the offside position to award free kick
        let nearest_opponent_id = field
            .players
            .iter()
            .filter(|p| p.side != offside_side && p.side.is_some())
            .min_by(|a, b| {
                let dist_a = (a.position - position).norm();
                let dist_b = (b.position - position).norm();
                dist_a
                    .partial_cmp(&dist_b)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|p| p.id);

        // Stop ball at offside position
        field.ball.position = position;
        field.ball.velocity = Vector3::new(0.0, 0.0, 0.0);

        // Award possession to nearest opponent (free kick)
        if let Some(opponent_id) = nearest_opponent_id {
            field.ball.previous_owner = field.ball.current_owner;
            field.ball.current_owner = Some(opponent_id);
            field.ball.ownership_duration = 0;
            // Walk him to the spot, exactly as the foul free kick does.
            //
            // "Nearest opponent" is nearest, not near: he is routinely tens
            // of units from where the flag went up. Awarding him the ball
            // without moving him left the restart beyond
            // `MAX_OWNER_TRACK_DISTANCE`, so `Ball::move_to` disowned it on
            // the next tick — and the two lines above had already parked the
            // ball at `position` with zero velocity, which is the dead-ball-
            // in-mid-pitch artefact exactly. The free kick path has staged
            // this teleport all along; the offside restart was the one
            // set piece that forgot to.
            field.ball.pending_set_piece_teleport = Some((opponent_id, position));
        }

        // Protected possession (same pattern as foul free kick)
        field.ball.claim_cooldown = 60;
        field.ball.flags.in_flight_state = 60;
        field.ball.contested_claim_count = 0;
        field.ball.pass_target_player_id = None;
        field.ball.clear_pass_history();
    }

    /// Identify the goalkeeper who is about to clear a shot, if any.
    /// Returns `Some(gk_id)` when the current ball owner is a GK *and*
    /// the ball is mid-flight from a real shot (`cached_shot_target` set)
    /// *and* there is a real shooter to credit on-target to. Without the
    /// shooter check, dead-ball restarts where `previous_owner` is None
    /// (goal kicks, corners, kickoffs after goals) credit phantom saves
    /// that have no paired on-target — pushing the saves/SOT ratio above
    /// 100%.
    fn gk_clearing_shot(field: &MatchField) -> Option<u32> {
        let clearer_id = field.ball.current_owner?;
        if field.ball.cached_shot_target.is_none() {
            return None;
        }
        // Real shot has a real shooter, otherwise this is residual cache
        // state surviving a dead-ball restart and we must not credit.
        let prev = field.ball.previous_owner?;
        if prev == clearer_id {
            return None;
        }
        // Iterate directly because `MatchField::get_player` takes `&mut
        // self`; we only need a read here and want to keep the borrow
        // immutable so the caller can re-borrow `field` mutably afterward.
        let clearer = field.players.iter().find(|p| p.id == clearer_id)?;
        if clearer.tactical_position.current_position.position_group()
            != PlayerFieldPositionGroup::Goalkeeper
        {
            return None;
        }
        // Verify the shooter is from the OTHER team — otherwise this is
        // a teammate clearance, not a save.
        let shooter = field.players.iter().find(|p| p.id == prev)?;
        if shooter.team_id == clearer.team_id {
            return None;
        }
        Some(clearer_id)
    }

    fn handle_clear_ball_event(
        velocity: Vector3<f32>,
        field: &mut MatchField,
        context: &MatchContext,
    ) {
        // Punches / dive-parries from a shot: the GK touched a real
        // attempt-on-goal and steered it away. The catching path
        // (handle_caught_ball_event) already credits saves; this is the
        // companion that closes the long-standing gap where punched and
        // parried shots stayed at zero saves regardless of effort.
        let gk_save_id = Self::gk_clearing_shot(field);
        let is_gk_shot_save = gk_save_id.is_some();
        if let Some(gk_id) = gk_save_id {
            // Capture shooter BEFORE we mutate the field — the previous
            // owner is the player whose shot the GK is now clearing.
            let shooter_id = field.ball.previous_owner.filter(|&sid| sid != gk_id);
            if let Some(gk) = field.get_player_mut(gk_id) {
                gk.statistics.saves += 1;
                gk.statistics.shots_faced += 1;
            }
            let mut shooter_found = false;
            if let Some(sid) = shooter_id {
                if let Some(shooter) = field.get_player_mut(sid) {
                    shooter.memory.credit_shot_on_target();
                    shooter_found = true;
                }
            }
            #[cfg(feature = "match-logs")]
            Self::dbg_save_credit(2, shooter_id.is_some(), shooter_id.is_none(), shooter_found);
            let _ = shooter_found;
            field.ball.cached_shot_target = None;
        }

        // Clearance cap, HORIZONTAL only — the vertical axis is in metres
        // and shares no scale with it, so there is no total magnitude worth
        // capping. 2.8 u/tick = 35 m/s, harder than any pass and about as
        // hard as a defender hits a ball.
        //
        // Was a 7.0 cap on the whole vector, which (a) mixed the two unit
        // systems and (b) rescaled, so a big hoof lost forward travel to
        // stay inside the budget — the same defect the pass path had.
        const MAX_CLEAR_HORIZONTAL: f32 = 2.8;
        let horizontal_speed = (velocity.x * velocity.x + velocity.y * velocity.y).sqrt();
        let mut capped_velocity = if horizontal_speed > MAX_CLEAR_HORIZONTAL {
            let scale = MAX_CLEAR_HORIZONTAL / horizontal_speed;
            Vector3::new(velocity.x * scale, velocity.y * scale, velocity.z)
        } else {
            velocity
        };

        // SAFETY: Prevent clearances from going toward own goal
        // A clearance should always go AWAY from own goal, never toward it
        {
            use crate::r#match::PlayerSide;
            if let Some(clearer_id) = field.ball.current_owner {
                if let Some(clearer) = field.get_player(clearer_id) {
                    match clearer.side {
                        Some(PlayerSide::Left) => {
                            // Own goal at x ≈ 0 — clearance must go forward (positive x)
                            if capped_velocity.x < 0.0 {
                                capped_velocity.x = capped_velocity.x.abs();
                            }
                        }
                        Some(PlayerSide::Right) => {
                            // Own goal at x ≈ field_width — clearance must go backward (negative x)
                            if capped_velocity.x > 0.0 {
                                capped_velocity.x = -capped_velocity.x.abs();
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        // A clearance is a release: it ends any hold and, for a keeper,
        // arms Law 12's second-touch bar until somebody else plays it.
        if let Some(clearer_id) = field.ball.current_owner {
            let from = field.ball.position;
            field
                .ball
                .note_release(clearer_id, from, context.current_tick());
        }

        // Clearance credit. A GK who just deflected a real shot away
        // (handled at the top of this fn) is already getting the save —
        // crediting a clearance on top would double-stamp the action,
        // so the shot-save branch is excluded here. Outfielders and a
        // GK doing routine sweeper work both get clearance credit + the
        // zone classification.
        if !is_gk_shot_save {
            if let Some(clearer_id) = field.ball.current_owner {
                let ball_pos = field.ball.position;
                if let Some(clearer) = field.get_player_mut(clearer_id) {
                    clearer.statistics.add_clearance();
                    let zone = Self::zone_for_player(clearer, ball_pos, context);
                    if let Some(zone) = zone {
                        clearer.statistics.note_clearance_zone(zone);
                    }
                    // Sweeper credit: a keeper who clears the ball from
                    // OUTSIDE his own box has come out to sweep, which is
                    // command of the area. A clearance from inside the box
                    // is not — that is a keeper using his feet, and it was
                    // the dominant producer of `gk_command_actions`
                    // (~12/match against a real-football 1-3). It mattered
                    // well beyond its own small credit: `>= 3` promotes the
                    // keeper to the `GkBusy` evidence tier and `>= 2` to the
                    // top clean-sheet tier, so every keeper cleared both
                    // bars every week no matter how he played.
                    if zone.map_or(false, |z| !z.is_own_box())
                        && clearer.tactical_position.current_position.position_group()
                            == PlayerFieldPositionGroup::Goalkeeper
                    {
                        clearer.statistics.note_gk_command_action();
                    }
                }
            }
        }

        // Captured before ownership is dropped below — the possession
        // check needs to know whose team just played the ball.
        let clearer_team_id = field
            .ball
            .current_owner
            .and_then(|id| field.get_player(id))
            .map(|p| p.team_id);

        // Apply the clearing velocity to the ball
        field.ball.velocity = capped_velocity;

        // Clear ownership - ball is now loose
        field.ball.previous_owner = field.ball.current_owner;
        field.ball.current_owner = None;
        field.ball.pass_target_player_id = None;
        // A defender or keeper hoofing it away ends the attacking phase,
        // so the chain goes. But `ClearBall` is also how an attacking
        // midfielder's knock-down / flick-on is modelled (see
        // `midfielders::states::heading`), and that keeps the move — and
        // the cross that started it — alive.
        if let Some(team_id) = clearer_team_id {
            field.ball.note_possession(team_id);
        } else {
            field.ball.clear_pass_history();
        }

        // Set in-flight state to prevent immediate reclaim after clearance
        field.ball.flags.in_flight_state = 40;
    }

    /// Award a free-kick or penalty restart to the victim's team after a
    /// foul. Penalty if the foul occurred inside the fouler's penalty
    /// area, otherwise a direct free kick at the ball's current position.
    /// Picks the taker dynamically by skill score (penalty: penalty_taking
    /// composite; FK: free_kicks composite). Idempotent on missing data:
    /// returns silently if fouler/victim team can't be resolved.
    pub(crate) fn award_restart_for_foul(
        fouler_id: u32,
        _severity: FoulSeverity,
        field: &mut MatchField,
        context: &mut MatchContext,
    ) {
        // Resolve fouler + side; the victim is everyone on the OTHER side.
        let (fouler_side, fouler_team_id) = match field
            .players
            .iter()
            .find(|p| p.id == fouler_id)
            .and_then(|p| p.side.map(|s| (s, p.team_id)))
        {
            Some(x) => x,
            None => return,
        };
        let victim_side = match fouler_side {
            PlayerSide::Left => PlayerSide::Right,
            PlayerSide::Right => PlayerSide::Left,
        };

        // Penalty if the foul (ball location) is inside the fouler's
        // penalty area — i.e. the box defending the fouler's goal.
        let pa = match fouler_side {
            PlayerSide::Left => context.penalty_area(true),
            PlayerSide::Right => context.penalty_area(false),
        };
        let foul_pos = field.ball.position;
        let in_penalty_area = pa.contains(&foul_pos);
        #[cfg(feature = "match-logs")]
        {
            use crate::r#match::player::strategies::players::ops::forward_shot_decision::mid_run_diag;
            use std::sync::atomic::Ordering;
            if in_penalty_area {
                mid_run_diag::PENALTY_AWARDED.fetch_add(1, Ordering::Relaxed);
            } else {
                mid_run_diag::DIRECT_FK_AWARDED.fetch_add(1, Ordering::Relaxed);
            }
        }

        // Tag the fouler's stats with the zone-aware bumps the rating
        // helper consumes. Penalty-conceded fouls and DEF/GK fouls in
        // the team's own third both carry extra rating penalties on
        // top of the per-foul base.
        {
            let field_w = context.field_size.width as f32;
            let foul_progress = fouler_side.attacking_progress_x(foul_pos.x, field_w);
            let in_own_third = foul_progress < 1.0 / 3.0;
            if let Some(fouler) = field.get_player_mut(fouler_id) {
                if in_penalty_area {
                    fouler.statistics.note_penalty_foul_conceded();
                }
                if in_own_third {
                    let pos_group = fouler.tactical_position.current_position.position_group();
                    if matches!(
                        pos_group,
                        PlayerFieldPositionGroup::Defender | PlayerFieldPositionGroup::Goalkeeper
                    ) {
                        fouler.statistics.note_own_third_def_foul();
                    }
                }
            }
        }

        // Restart position: penalty spot vs foul spot. Penalty spot is
        // 88u from the defending goal line on the centre y axis.
        let field_w = context.field_size.width as f32;
        let field_h = context.field_size.height as f32;
        let restart_pos = if in_penalty_area {
            let px = match fouler_side {
                PlayerSide::Left => 88.0_f32.min(field_w * 0.5),
                PlayerSide::Right => (field_w - 88.0_f32).max(field_w * 0.5),
            };
            Vector3::new(px, field_h * 0.5, 0.0)
        } else {
            // Tiny inset so the ball isn't sitting on a boundary.
            let x = foul_pos.x.clamp(2.0, field_w - 2.0);
            let y = foul_pos.y.clamp(2.0, field_h - 2.0);
            Vector3::new(x, y, 0.0)
        };

        // Pick a taker from the victim's on-field players. Penalty uses
        // penalty-taking composite; FK uses free-kick composite.
        let taker_id = if in_penalty_area {
            Self::pick_penalty_taker(field, victim_side)
        } else {
            Self::pick_free_kick_taker(field, victim_side, restart_pos)
        };
        let taker_id = match taker_id {
            Some(id) => id,
            None => return, // Whole team off the field — nothing to do.
        };

        // Tell the engine to teleport the taker onto the ball next tick.
        let ball = &mut field.ball;
        ball.position = restart_pos;
        ball.velocity = Vector3::zeros();
        ball.previous_owner = ball.current_owner;
        ball.current_owner = Some(taker_id);
        ball.ownership_duration = 0;
        ball.claim_cooldown = if in_penalty_area { 200 } else { 90 };
        ball.flags.in_flight_state = if in_penalty_area { 200 } else { 60 };
        ball.contested_claim_count = 0;
        ball.pass_target_player_id = None;
        ball.recent_passers.clear();
        ball.cached_shot_target = None;
        ball.offside_snapshot = None;
        ball.pass_origin_restart = if in_penalty_area {
            PassOriginRestart::Penalty
        } else {
            PassOriginRestart::DirectFreeKick
        };
        let team_id = field
            .players
            .iter()
            .find(|p| p.id == taker_id)
            .map(|p| p.team_id)
            .unwrap_or(0);
        let _ = fouler_team_id; // Kept for future calibration hooks.
        field
            .ball
            .record_touch(taker_id, team_id, context.current_tick(), true);
        field.ball.pending_set_piece_teleport = Some((taker_id, restart_pos));
    }

    fn pick_penalty_taker(field: &MatchField, victim_side: PlayerSide) -> Option<u32> {
        use crate::r#match::engine::set_pieces::{TakerScore, score_penalty_taker};
        field
            .players
            .iter()
            .filter(|p| {
                p.side == Some(victim_side)
                    && !p.is_sent_off
                    && p.tactical_position.current_position.position_group()
                        != PlayerFieldPositionGroup::Goalkeeper
            })
            .map(|p| TakerScore {
                player_id: p.id,
                score: score_penalty_taker(
                    p.skills.technical.penalty_taking,
                    p.skills.technical.finishing,
                    p.skills.mental.composure,
                    p.attributes.pressure,
                    p.skills.technical.technique,
                    0.0,
                ),
            })
            .max_by(|a, b| {
                a.score
                    .partial_cmp(&b.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|t| t.player_id)
    }

    fn pick_free_kick_taker(
        field: &MatchField,
        victim_side: PlayerSide,
        restart_pos: Vector3<f32>,
    ) -> Option<u32> {
        use crate::r#match::engine::set_pieces::{TakerScore, score_free_kick_taker};
        field
            .players
            .iter()
            .filter(|p| {
                p.side == Some(victim_side)
                    && !p.is_sent_off
                    && p.tactical_position.current_position.position_group()
                        != PlayerFieldPositionGroup::Goalkeeper
            })
            .map(|p| {
                let dx = p.position.x - restart_pos.x;
                let dy = p.position.y - restart_pos.y;
                let dist = (dx * dx + dy * dy).sqrt();
                // Distance penalty: a player 200u away isn't realistically
                // walking over to take a quick free kick.
                let dist_penalty = (dist / 200.0).clamp(0.0, 1.0) * 0.20;
                let base = score_free_kick_taker(
                    p.skills.technical.free_kicks,
                    p.skills.technical.technique,
                    p.skills.technical.long_shots,
                    p.skills.technical.crossing,
                    p.skills.mental.vision,
                    p.skills.mental.composure,
                    p.attributes.pressure,
                );
                TakerScore {
                    player_id: p.id,
                    score: base - dist_penalty,
                }
            })
            .max_by(|a, b| {
                a.score
                    .partial_cmp(&b.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|t| t.player_id)
    }
}

#[cfg(test)]
mod pass_ballistics_tests {
    use super::{PassSkills, PlayerEventDispatcher, TrajectoryType};
    use crate::r#match::engine::ball::ball::Ball;
    use crate::r#match::engine::flow::rng::MatchRng;
    use nalgebra::Vector3;

    /// A competent but unremarkable passer, so the execution jitter is
    /// small and the assertions read the model rather than the dice.
    fn skills() -> PassSkills {
        PassSkills {
            passing: 0.8,
            technique: 0.8,
            vision: 0.8,
            composure: 0.8,
            decisions: 0.8,
            concentration: 0.8,
            flair: 0.8,
            long_shots: 0.8,
            crossing: 0.8,
            stamina: 0.8,
            availability_factor: 1.0,
        }
    }

    /// `calculate_horizontal_velocity`'s output for the same vector — the
    /// rolling delivery the solver is handed.
    fn rolling(vector: &Vector3<f32>) -> Vector3<f32> {
        PlayerEventDispatcher::calculate_horizontal_velocity(vector, 1.0)
    }

    fn solve(distance: f32, trajectory: TrajectoryType, seed: u64) -> Vector3<f32> {
        let vector = Vector3::new(distance, 0.0, 0.0);
        PlayerEventDispatcher::calculate_pass_velocity(
            &vector,
            &rolling(&vector),
            trajectory,
            &skills(),
            &MatchRng::from_seed(seed),
        )
    }

    /// Horizontal distance in game units covered before the ball first
    /// touches down.
    fn air_range(v: &Vector3<f32>) -> f32 {
        let vh = (v.x * v.x + v.y * v.y).sqrt();
        vh * Ball::hang_ticks(v.z)
    }

    /// Peak height in metres.
    fn apex(v: &Vector3<f32>) -> f32 {
        Ball::apex_for_launch(v.z)
    }

    #[test]
    fn a_delivery_travels_further_than_it_climbs() {
        // The user-visible form of the contract, and the exact thing that
        // was reported: "the ball bounces twice as high as the player and
        // falls to the ground". A ball that peaks higher than it advances
        // is a shell, not a pass — whatever its nominal style.
        //
        // Both sides in METRES: the horizontal axis is in game units, the
        // vertical one is not (see `GRAVITY_PER_TICK`), and comparing them
        // raw is exactly the class of mistake this whole area was full of.
        for &trajectory in &[
            TrajectoryType::Ground,
            TrajectoryType::LowDriven,
            TrajectoryType::MediumArc,
            TrajectoryType::HighArc,
            TrajectoryType::Chip,
        ] {
            for distance in [20.0, 60.0, 120.0, 240.0, 400.0, 700.0] {
                for seed in 0..8 {
                    let v = solve(distance, trajectory, seed);
                    let climbed = apex(&v);
                    let travelled = air_range(&v) * 0.125;
                    assert!(
                        travelled > climbed,
                        "{trajectory:?} over {distance}u climbed {climbed:.2} m \
                         but only travelled {travelled:.2} m"
                    );
                }
            }
        }
    }

    #[test]
    fn lofted_passes_hang_for_a_realistic_time() {
        // The point of fixing the gravity constant. A 30 m cross hangs
        // around 2 s in real football; with gravity 160× too strong in
        // metres it hung for 0.5 s, and the only way to cover ground in
        // that window was to strike the ball at 85 m/s.
        let v = solve(240.0, TrajectoryType::HighArc, 2);
        let seconds = Ball::hang_ticks(v.z) / 100.0;
        assert!(
            (1.5..3.5).contains(&seconds),
            "30 m cross hung for {seconds:.2} s"
        );

        // …and it is struck at a speed a human leg can produce. 1u = 0.125 m
        // over a 10 ms tick, so 1 u/tick = 12.5 m/s.
        let speed = (v.x * v.x + v.y * v.y).sqrt() * 12.5;
        assert!(speed < 30.0, "30 m cross struck at {speed:.0} m/s");
    }

    #[test]
    fn a_lofted_pass_lands_on_its_target() {
        // Inside the range the gravity constant permits, an arced ball is
        // solved ballistically and must arrive — not drop a fifth of the
        // way and roll. Tolerance covers the execution jitter, which is
        // applied to the speed (range goes as v², so ±14% of speed is
        // roughly ±30% of range).
        for distance in [80.0, 160.0, 200.0] {
            let v = solve(distance, TrajectoryType::MediumArc, 3);
            let landed = air_range(&v);
            assert!(
                landed > distance * 0.60 && landed < distance * 1.45,
                "medium arc over {distance}u first bounced at {landed:.0}u"
            );
        }
    }

    #[test]
    fn a_long_ball_actually_gets_airborne() {
        // A raking diagonal is supposed to FLY — well above a defender's
        // head, and land near its man. Flattening these was the failure
        // mode the second time round: the arcs were geometrically correct
        // but unaffordable, so every one of them fell through to the
        // driven fallback.
        for &distance in &[240.0_f32, 400.0, 560.0] {
            for seed in 0..8 {
                let v = solve(distance, TrajectoryType::HighArc, seed);
                assert!(
                    apex(&v) > 4.0,
                    "{:.0} m ball peaked at {:.1} m — barely off the deck",
                    distance * 0.125,
                    apex(&v)
                );
                let landed = air_range(&v);
                assert!(
                    landed > distance * 0.60,
                    "long ball first bounced at {landed:.0}u of {distance}u"
                );
            }
        }
    }

    #[test]
    fn a_loft_that_cannot_arrive_is_driven_not_shelled() {
        // A medium arc peaks at 5 m at most, which buys ~2 s of hang; the
        // length of the pitch inside that window would need a 52 m/s pass.
        // The solver must fall back to a driven ball that keeps its full
        // rolling pace, NOT launch the steepest thing it can afford.
        let distance = 840.0;
        let v = solve(distance, TrajectoryType::MediumArc, 5);
        let vh = (v.x * v.x + v.y * v.y).sqrt();
        let reference = rolling(&Vector3::new(distance, 0.0, 0.0)).norm();

        assert!(
            (vh - reference).abs() < 1e-3,
            "driven fallback must keep the rolling delivery speed: {vh:.3} vs {reference:.3}"
        );
        // A driven long ball skips off the deck; it does not arc.
        assert!(
            apex(&v) < 1.5,
            "fallback peaked at {:.2} m — that is an arc, not a drive",
            apex(&v)
        );
    }

    #[test]
    fn every_launch_respects_the_horizontal_cap() {
        // Honoured by construction so the ball's global `MAX_VELOCITY`
        // clamp — which DOES rescale, and would re-introduce the original
        // bug — can never fire on a pass.
        for &trajectory in &[
            TrajectoryType::Ground,
            TrajectoryType::LowDriven,
            TrajectoryType::MediumArc,
            TrajectoryType::HighArc,
            TrajectoryType::Chip,
        ] {
            for distance in [5.0, 40.0, 120.0, 300.0, 840.0] {
                for seed in 0..8 {
                    let v = solve(distance, trajectory, seed);
                    let vh = (v.x * v.x + v.y * v.y).sqrt();
                    assert!(
                        vh <= PlayerEventDispatcher::MAX_PASS_VELOCITY + 1e-3,
                        "{trajectory:?} over {distance}u struck at {vh:.2} u/tick"
                    );
                    // Nothing goes into orbit either.
                    assert!(
                        apex(&v) < 15.0,
                        "{trajectory:?} peaked at {:.1} m",
                        apex(&v)
                    );
                }
            }
        }
    }

    #[test]
    fn a_zero_length_pass_does_not_produce_nan() {
        // Degenerate input: target on top of the passer. `normalize()` on a
        // zero vector would poison the ball's position through the whole
        // recording.
        let vector = Vector3::new(0.0, 0.0, 0.0);
        let v = PlayerEventDispatcher::calculate_pass_velocity(
            &vector,
            &Vector3::new(0.0, 0.0, 0.0),
            TrajectoryType::HighArc,
            &skills(),
            &MatchRng::from_seed(1),
        );
        assert!(v.x.is_finite() && v.y.is_finite() && v.z.is_finite());
    }
}

#[cfg(test)]
mod xg_distribution_tests {
    use super::PlayerEventDispatcher;

    fn credit_for(out: &[(u32, f32, Option<f32>)], pid: u32) -> Option<(f32, Option<f32>)> {
        out.iter()
            .find(|(id, _, _)| *id == pid)
            .map(|(_, c, b)| (*c, *b))
    }

    #[test]
    fn assister_excluded_from_buildup() {
        // Chain: passer P1 → P2 (assister, last completed pass) → shooter S.
        // Recent passers contains [P1, P2] (S is shooter, not in passers).
        // Buildup must credit only P1 — P2 is the direct assister.
        let recent = vec![1, 2];
        let credits = PlayerEventDispatcher::distribute_xg_credit(
            recent.into_iter(),
            99,      // shooter
            Some(2), // assister
            0.40,
        );
        let p1 = credit_for(&credits, 1).expect("P1 must be credited");
        let p2 = credit_for(&credits, 2).expect("P2 must be credited (chain)");
        assert!(p1.1.is_some(), "P1 buildup should be set");
        assert!(p2.1.is_none(), "assister P2 must be excluded from buildup");
    }

    #[test]
    fn shooter_excluded_from_buildup() {
        // Shooter appearing in recent_passers (e.g. they kicked it earlier
        // in the chain) gets chain credit but not buildup.
        let recent = vec![1, 2, 99];
        let credits =
            PlayerEventDispatcher::distribute_xg_credit(recent.into_iter(), 99, Some(2), 0.50);
        let s = credit_for(&credits, 99).expect("shooter chain credit");
        assert!(s.1.is_none(), "shooter must not get buildup credit");
    }

    #[test]
    fn third_player_in_chain_still_receives_buildup() {
        // Passer P1 (deeper in chain), P2 assister, S shooter — P1 still
        // earns buildup even though P2/S are excluded.
        let recent = vec![1, 2];
        let credits =
            PlayerEventDispatcher::distribute_xg_credit(recent.into_iter(), 99, Some(2), 0.40);
        let p1 = credit_for(&credits, 1).expect("P1 in chain");
        assert!(p1.1.is_some(), "third-player buildup must be present");
        // Pool is xg * 0.20 = 0.08, split across 1 eligible player.
        let buildup = p1.1.unwrap();
        assert!(
            (buildup - 0.40 * 0.20).abs() < 1e-4,
            "P1 should receive the entire buildup pool — got {}",
            buildup
        );
    }

    #[test]
    fn duplicate_recent_passers_credited_once() {
        // Same passer appears 3 times in the ring; chain credit and
        // buildup credit must each fire ONCE for that player.
        let recent = vec![1, 1, 1];
        let credits =
            PlayerEventDispatcher::distribute_xg_credit(recent.into_iter(), 99, None, 0.40);
        assert_eq!(credits.len(), 1, "dedupe — one entry per unique id");
        let (pid, chain, buildup) = credits[0];
        assert_eq!(pid, 1);
        // Chain pool 0.40 * 0.30 = 0.12, divided by 1 unique = 0.12.
        assert!((chain - 0.12).abs() < 1e-4);
        // Buildup pool 0.40 * 0.20 = 0.08, divided by 1 unique = 0.08.
        assert!((buildup.unwrap() - 0.08).abs() < 1e-4);
    }

    #[test]
    fn buildup_does_not_scale_linearly_with_duplicate_ring_entries() {
        // Same chain ([1, 1, 2, 2, 1]) once and a deduped equivalent
        // ([1, 2]) must produce the same TOTAL credit per player —
        // duplicates can't inflate buildup.
        let dup_credits = PlayerEventDispatcher::distribute_xg_credit(
            vec![1u32, 1, 2, 2, 1].into_iter(),
            99,
            None,
            0.40,
        );
        let unique_credits =
            PlayerEventDispatcher::distribute_xg_credit(vec![1u32, 2].into_iter(), 99, None, 0.40);
        // Both must be length 2 and credit the same per-player amounts.
        assert_eq!(dup_credits.len(), 2);
        assert_eq!(unique_credits.len(), 2);
        for &(pid, chain_a, build_a) in dup_credits.iter() {
            let (chain_b, build_b) = credit_for(&unique_credits, pid).expect("paired credit");
            assert!(
                (chain_a - chain_b).abs() < 1e-4,
                "chain credit for {} differs: dup {} vs unique {}",
                pid,
                chain_a,
                chain_b
            );
            match (build_a, build_b) {
                (Some(a), Some(b)) => assert!((a - b).abs() < 1e-4),
                (None, None) => {}
                _ => panic!("buildup presence differs for {}", pid),
            }
        }
    }

    #[test]
    fn pool_total_bounded_by_xg_fractions() {
        // Aggregate buildup credit summed across all eligible players
        // must equal the pool — 0.40 * 0.20 = 0.08 — regardless of
        // how many participants there are.
        let recent = vec![1, 2, 3, 4]; // 4 unique participants
        let credits =
            PlayerEventDispatcher::distribute_xg_credit(recent.into_iter(), 99, None, 0.40);
        let total_buildup: f32 = credits.iter().map(|(_, _, b)| b.unwrap_or(0.0)).sum();
        assert!(
            (total_buildup - 0.08).abs() < 1e-4,
            "total buildup pool should be 0.08, got {}",
            total_buildup
        );
        let total_chain: f32 = credits.iter().map(|(_, c, _)| *c).sum();
        assert!(
            (total_chain - 0.12).abs() < 1e-4,
            "total chain pool should be 0.12, got {}",
            total_chain
        );
    }
}

#[cfg(test)]
mod shot_aim_tests {
    use super::{GOAL_MOUTH_HALF_WIDTH, PlayerEventDispatcher};
    use crate::r#match::{GoalPosition, PlayerSide};
    use nalgebra::Vector3;

    /// 840u × 545u, the pitch the shot model is calibrated against.
    fn goals() -> GoalPosition {
        GoalPosition {
            left: Vector3::new(0.0, 272.5, 0.0),
            right: Vector3::new(840.0, 272.5, 0.0),
        }
    }

    #[test]
    fn frame_is_the_goal_centre_wherever_inside_the_mouth_the_player_aims() {
        // The regression: the posts must NOT travel with the aim. Any
        // intent inside the goal mouth resolves to the same frame, so
        // the strike model's own placement roll is the only offset the
        // ball ever receives. Before, an aim 20u off centre moved both
        // posts 20u with it and the placement roll then added a second
        // ~25u on top — a strike 2 m outside the near post that the
        // on-target check happily called on target.
        let g = goals();
        let centre = g.right;
        for offset in [-28.0, -20.0, -5.0, 0.0, 5.0, 20.0, 28.0] {
            let intent = Vector3::new(centre.x, centre.y + offset, 0.0);
            let frame =
                PlayerEventDispatcher::resolve_goal_frame(intent, Some(PlayerSide::Left), &g);
            assert_eq!(frame, centre, "aim offset {offset} must not move the frame");
        }
    }

    #[test]
    fn each_side_is_framed_by_the_goal_it_attacks() {
        let g = goals();
        assert_eq!(
            PlayerEventDispatcher::resolve_goal_frame(g.right, Some(PlayerSide::Left), &g),
            g.right
        );
        assert_eq!(
            PlayerEventDispatcher::resolve_goal_frame(g.left, Some(PlayerSide::Right), &g),
            g.left
        );
    }

    #[test]
    fn a_clearance_keeps_its_own_target() {
        // Defensive clearances are dispatched as Shoot events too. They
        // aim at open space, so they must not be re-pointed at a goal.
        let g = goals();
        let upfield = Vector3::new(400.0, 120.0, 0.0);
        assert_eq!(
            PlayerEventDispatcher::resolve_goal_frame(upfield, Some(PlayerSide::Left), &g),
            upfield
        );
    }

    #[test]
    fn intended_placement_stays_inside_the_posts_at_point_blank() {
        // A finisher's aim plus his own spread has to land between the
        // posts, or the chance was decided by the dice rather than by
        // him. Point-blank spread is ~3u (0.4 m) after the angular
        // distance term; even an elite placer must leave room for it.
        for placement_skill in [0.1_f32, 0.35, 0.6, 0.9, 1.0] {
            for max_y_error in [3.0_f32, 5.2, 8.0] {
                let reach =
                    PlayerEventDispatcher::intended_corner_reach(placement_skill, max_y_error);
                assert!(
                    reach + max_y_error * 0.75 <= GOAL_MOUTH_HALF_WIDTH + 1e-3,
                    "aim {reach} + spread {max_y_error} escapes the frame \
                     (placement {placement_skill})"
                );
                assert!(reach > 0.0, "a shooter always aims somewhere");
            }
        }
    }

    #[test]
    fn better_placers_still_aim_closer_to_the_post() {
        // The bound must not flatten the skill axis it sits on top of.
        let spread = 3.0;
        let poor = PlayerEventDispatcher::intended_corner_reach(0.15, spread);
        let elite = PlayerEventDispatcher::intended_corner_reach(0.90, spread);
        assert!(
            elite > poor,
            "elite placement {elite} must out-reach poor {poor}"
        );
    }

    #[test]
    fn a_wild_spread_still_leaves_a_target_to_aim_at() {
        // A 60u spread (the model's ceiling, a scrambled long shot)
        // exceeds the goal entirely; the shooter still aims at the
        // frame rather than at a degenerate zero-width point.
        let reach = PlayerEventDispatcher::intended_corner_reach(0.5, 60.0);
        assert!(reach >= GOAL_MOUTH_HALF_WIDTH * 0.20 - 1e-3);
    }
}

#[cfg(test)]
mod first_touch_loss_tests {
    use super::PlayerEventDispatcher;

    /// Skill-loss curve must be strictly decreasing in composite skill.
    /// A 5/20 receiver must register a clearly higher loss probability
    /// than a 15/20 receiver, holding pressure constant.
    #[test]
    fn weak_receiver_has_higher_loss_probability_than_strong() {
        for opp in 0..=3 {
            let weak = PlayerEventDispatcher::first_touch_loss_probability(0.25, opp);
            let avg = PlayerEventDispatcher::first_touch_loss_probability(0.50, opp);
            let elite = PlayerEventDispatcher::first_touch_loss_probability(0.85, opp);
            assert!(
                weak > avg && avg > elite,
                "monotonic in skill (opp={}): weak={} avg={} elite={}",
                opp,
                weak,
                avg,
                elite
            );
        }
    }

    /// Pressure must amplify the probability — same receiver under
    /// two close opponents should fluff more than the same receiver
    /// unmarked.
    #[test]
    fn pressure_amplifies_loss_probability() {
        let no_pressure = PlayerEventDispatcher::first_touch_loss_probability(0.40, 0);
        let two_close = PlayerEventDispatcher::first_touch_loss_probability(0.40, 2);
        assert!(
            two_close > no_pressure * 1.5,
            "pressure must amplify: 0 opp = {} vs 2 opp = {}",
            no_pressure,
            two_close
        );
    }

    /// Elite receivers (≥ 0.85 composite) should land near-immune to
    /// pressure-induced fluffs — well under 2% even under heavy press.
    #[test]
    fn elite_receiver_virtually_immune_to_pressure() {
        let elite_heavy_press = PlayerEventDispatcher::first_touch_loss_probability(0.90, 4);
        assert!(
            elite_heavy_press < 0.02,
            "elite receiver under heavy press shouldn't trip the producer: got {}",
            elite_heavy_press
        );
    }

    /// Cap holds — even 0-skill under maximum pressure can't exceed
    /// the 0.30 ceiling.
    #[test]
    fn loss_probability_capped_at_thirty_percent() {
        let worst = PlayerEventDispatcher::first_touch_loss_probability(0.0, 10);
        assert!(worst <= 0.30 + 1e-6, "cap violated: got {}", worst);
    }

    /// Clamping protects against out-of-range composite inputs.
    #[test]
    fn out_of_range_composite_clamped() {
        let neg = PlayerEventDispatcher::first_touch_loss_probability(-1.0, 2);
        let over = PlayerEventDispatcher::first_touch_loss_probability(2.0, 2);
        // negative skill clamps to 0 → max probability under that pressure
        let zero = PlayerEventDispatcher::first_touch_loss_probability(0.0, 2);
        // skill > 1 clamps to 1.0 → zero probability
        assert!(
            (neg - zero).abs() < 1e-6,
            "negative composite must clamp to 0: got {}",
            neg
        );
        assert!(
            over < 1e-6,
            "composite > 1 must clamp to 1 (zero prob): got {}",
            over
        );
    }
}
