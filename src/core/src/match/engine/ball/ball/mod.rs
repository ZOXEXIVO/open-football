//! Match-engine ball model, split by concern. The `Ball` struct lives
//! here together with the per-tick orchestrator (`update` / `update_light`)
//! and the simple state queries the rest of the engine reads. The heavier
//! domain passes are grouped by what part of the ball's life they own:
//!
//! | Group           | Concern                                                       |
//! |-----------------|---------------------------------------------------------------|
//! | [`flight`]      | The ball in motion: integration, drag, spin, owner tracking    |
//! | [`contest`]     | Players intervening: intercept / block / save, and ownership   |
//! | [`boundary`]    | The edges: goal, woodwork, net, and the ground outside the lines |
//! | [`restarts`]    | Getting it back into play, and the stall detector that notices when it never was |
//! | [`diagnostics`] | `match-logs` instrumentation the `dev_match` harness reads      |
//!
//! Each group re-exports below under the module name it had before the
//! grouping, so every `ball::ball::<module>::Item` path in the engine and
//! in the dev harness keeps resolving.

pub mod boundary;
pub mod contest;
pub mod diagnostics;
pub mod flight;
pub mod restarts;
pub mod tick;

// `pub` for `GoalFrame` / `FramePart` — the replay viewer draws the same
// posts the physics rebounds off, and the two geometries must agree.
// `pub` for `GoalNet` / `BallInNet` — the celebration choreography in the
// flow layer reads the goal geometry to send a keeper in after the ball,
// and the replay viewer needs the same net depth to draw it.
// `pub` for `RunOff` — the player layer reads the same rectangle when it
// decides how far off the pitch a restart taker may go, and the two must
// be one constant or the taker is pinned short of the ball he is fetching.
pub use boundary::{Perimeter, RunOff, frame, net, runoff};
pub use contest::{
    ContactInPlace, PassChainEntry, PossessionSource, block, contact, interception, ownership,
    possession, save,
};
// `pub` for `SpinModel` — the strike sites (shot / cross) solve the
// rotation they need from the same Magnus coefficient the physics
// integrates, so the two can never drift apart.
pub use flight::{
    AIR_DRAG_PER_TICK, AerialDelivery, AerialOutcome, AerialReach, BallRoll, GRAVITY_PER_TICK,
    GROUND_FRICTION, aerial, ballistics, motion, roll,
};
// `pub` for `dead_ball_diag` — the stall attribution counters are read by
// the dev harness, same as `ownership::reception_diag`.
pub use restarts::{
    AwaitedRestart, CornerWalk, DeadBall, FoulWalk, OffsideLine, OffsideSnapshot,
    PassOriginRestart, ThrowIn, awaited, offside, stall,
};
// The woodwork's own per-tick ball trace, and the whole-tick relocation
// census. `flight_diag` below only sees `Ball::update`; `teleport` sees
// the resolvers and the player layer that run after it, which is where
// the set pieces live.
#[cfg(feature = "match-logs")]
pub use diagnostics::{assist_diag, block_diag, flight_diag, frame_trace, teleport};

use crate::r#match::PlayerSide;
use crate::r#match::engine::ball::ball::net::BallInNet;
use crate::r#match::engine::corner_shape::{CornerShapeHold, CornerStation};
use crate::r#match::engine::set_pieces::CornerRoutine;
use crate::r#match::player::strategies::passing::CrossType;
#[cfg(feature = "match-logs")]
use crate::mid_run_diag::{CrossDiag, PassWeightCensus};
use nalgebra::Vector3;
use std::collections::VecDeque;

/// How close a player must be to the ball to take control of it, in game
/// units (1u = 0.125 m, so this is 1.5 m — one stride, a real first-touch
/// distance).
///
/// This MUST stay at or below [`MAX_OWNER_TRACK_DISTANCE`]. The two used
/// to be independent numbers that disagreed by a factor of six: the
/// pass-target claim granted ownership at 100u while `Ball::move_to`
/// refused to track the ball to an owner beyond 15u and dropped the
/// ownership again. The effect was that a pass was booked COMPLETED on
/// the first tick of its flight — the receiver is within 100u of the
/// ball the moment it leaves the passer's foot — and then instantly
/// released, so the ball flew its whole course as a loose ball with no
/// owner and no intended receiver (the claim had already consumed
/// `pass_target_player_id`). Measured: 100% of receptions landed beyond
/// the tracking cap, `move_to` dropped ownership 5.4k times a match, and
/// 86% of all shots were struck off loose balls against a real ~15%.
/// Pass accuracy read 87% the whole time — the metric counted claims,
/// not deliveries.
pub const CONTROL_DISTANCE: f32 = 12.0;

/// Hard cap on how far the ball will track to its owner before ownership
/// is treated as impossible and dropped (1.9 m). See [`CONTROL_DISTANCE`].
pub const MAX_OWNER_TRACK_DISTANCE: f32 = 15.0;

/// How close the ball has to be for a player to kick it (1.9 m — within
/// reach at a stretch, which is what makes a first-time pass legal).
///
/// `PlayerEvent::PassTo` had no such check: any player in a passing state
/// rewrote the ball's velocity from anywhere on the pitch, whether or not
/// they had the ball. 59% of all passes were emitted on top of a pass
/// that was still in the air, which is why the engine recorded ~1150
/// passes a team against a real ~500 — the surplus was players kicking a
/// ball that was 40 m away, and each one destroyed the pass already in
/// flight.
pub const KICKABLE_DISTANCE: f32 = MAX_OWNER_TRACK_DISTANCE;

/// How long a pass stays assist-eligible, in ticks (100 ticks ≈ 1 s).
///
/// An assist is the pass that *led to* the goal, so the two have to be
/// close together. 6 s covers the slowest legitimate chain the engine
/// produces — a long ball is ~3 s of flight, plus a touch and a strike —
/// while excluding the case that used to dominate the charts: a goal
/// kick counted as the assist for a solo run that ended half a minute
/// later. The same-possession rule in `assist_for_goal` does most of the
/// work; this is the backstop for a phase that never changes hands.
pub const ASSIST_WINDOW_TICKS: u64 = 600;

pub struct Ball {
    pub start_position: Vector3<f32>,
    pub position: Vector3<f32>,
    pub velocity: Vector3<f32>,
    /// Angular velocity in rad/tick. Set at strike time from where on the
    /// ball the player's foot met it, integrated as a Magnus force while
    /// airborne, and scrubbed off on contact with the turf or a player.
    /// This is the only channel in the engine that can turn a flight
    /// sideways — see [`SpinModel`](super::ball::SpinModel).
    pub spin: Vector3<f32>,
    /// `velocity.z` as this tick's physics left it, so the next tick can
    /// tell a KICK from the rest of the flight. Anything that raises the
    /// vertical speed between two `update` calls came from outside the
    /// physics — a clearance, a punch, a shot — and is the only event the
    /// apex census wants to count. Diagnostic only.
    #[cfg(feature = "match-logs")]
    pub settled_vz: f32,
    pub center_field_position: f32,

    pub field_width: f32,
    pub field_height: f32,

    pub flags: BallFlags,

    pub previous_owner: Option<u32>,
    pub current_owner: Option<u32>,
    pub take_ball_notified_players: Vec<u32>,
    pub notification_cooldown: u32,
    pub notification_timeout: u32,
    pub last_boundary_position: Option<Vector3<f32>>,
    pub unowned_stopped_ticks: u32,
    pub ownership_duration: u32,
    pub claim_cooldown: u32,
    pub pass_target_player_id: Option<u32>,
    /// Passer id of the most-recent live pass. Set on pass emit,
    /// cleared on any opponent touch or when the pass's natural
    /// window (150 ticks ≈ 1.5 s) expires. The pass-completion stat
    /// uses this as the source of truth for "was this claim a pass
    /// reception?" — `pass_target_player_id` gets cleared in too
    /// many unrelated paths to serve that role. None outside an
    /// active pass window.
    pub pending_pass_passer: Option<u32>,
    pub pending_pass_set_tick: u64,
    pub recent_passers: VecDeque<PassChainEntry>,
    /// How `current_owner` came by the ball. See [`PossessionSource`].
    pub possession_source: PossessionSource,
    /// Who `possession_source` describes, so a repeat event for the
    /// player who already has the ball cannot relabel their acquisition.
    pub possession_source_for: Option<u32>,
    /// Whether the current pass has already had its one interception
    /// attempt. Mirrors `ShotTarget::block_rolled`: without a latch the
    /// intercept test fires every tick the ball is in flight, so its
    /// rate is set by how long the flight window happens to be rather
    /// than by the defending. Reset when a pass is struck.
    pub intercept_rolled: bool,
    pub contested_claim_count: u32,
    pub unowned_ticks: u32,
    /// Snapshot captured at the moment the ball became uncontrolled — ball
    /// kinematics plus every player's state/position/velocity. Held until
    /// the stall resolves, then attached to the resolution log (only if
    /// the stall was long enough to log). Provides the "what did the
    /// pitch look like when this got stuck" context in the same line as
    /// the duration. Cleared on ownership resume.
    pub stall_start_snapshot: Option<String>,
    pub goal_scored: bool,
    /// The ball is in the goal — see [`BallInNet`]. Set the instant it
    /// crosses the line and cleared by the restart, so it outlives
    /// `goal_scored` (which the flow layer consumes on the same tick to arm
    /// the celebration). Every resolver that would otherwise see a ball
    /// behind the goal line and award a corner, a goal kick or a boundary
    /// clamp keys off this being `Some`.
    pub in_net: Option<BallInNet>,
    pub kickoff_team_side: Option<PlayerSide>,
    pub cached_landing_position: Vector3<f32>,
    /// When a set-piece (corner, goal kick) rewrites ownership to a
    /// specific player, the ball can only mutate itself here — player
    /// teleport requires &mut field.players which lives one layer up.
    /// Populated inside `check_wide_of_goal` and drained by the engine
    /// after `ball.update` returns, so the owner is on the ball before
    /// the next `move_to` distance check can null their ownership.
    pub pending_set_piece_teleport: Option<(u32, Vector3<f32>)>,
    /// A dead ball lying on the touchline waiting for its taker to WALK to
    /// it. See [`AwaitedRestart`].
    pub awaiting_restart: Option<AwaitedRestart>,
    /// `(player, where he has to stand)` for the man CARRYING a dead ball
    /// to the spot it is taken from — the corner taker walking the ball to
    /// the arc, and nothing else.
    ///
    /// Same reason as [`Self::pending_set_piece_teleport`]: the ball can
    /// only mutate itself, and a station lives on the player. Drained into
    /// `MatchPlayer::set_piece_station` by the engine, which is what
    /// `CornerHold` steers off — see [`AwaitedRestart::carrying`] for why
    /// nothing else can move him.
    pub pending_restart_station: Option<(u32, Vector3<f32>)>,
    /// The corner set-up: where all twenty other players stand while the
    /// corner is taken, as planned by `CornerShape::plan` in the corner
    /// branch of `check_wide_of_goal` and drained by the engine alongside
    /// the taker teleport.
    ///
    /// In real football both sides walk into this shape during the
    /// stoppage. There is no stoppage here — the cross leaves the taker's
    /// boot 50 ms after the corner is awarded — so nobody can cover the
    /// ground, and without the plan the "corner shape" is just wherever
    /// open play left everyone: measured at 3-6 defenders in the box
    /// against a real 8-10, with a low of one (the goalkeeper).
    ///
    /// ⚠ **They are stations, not teleports, since the corner started
    /// waiting for its taker to fetch the ball.** That wait — a fetch and
    /// a carry, several seconds of it — is the stoppage this comment says
    /// the sim does not have, so both sides now WALK into the shape under
    /// `CornerHold` and the positions are no longer written. See
    /// `TickEngine::apply_pending_set_piece_teleport`.
    pub pending_corner_teleports: Vec<CornerStation>,
    /// The corner shape currently pinned on the players, if any — when it
    /// went up and who is taking the kick. `None` on every tick that is
    /// not a corner, which is nearly all of them, so the per-tick expiry
    /// check (`clear_expired_corner_stations`) costs one `Option` read.
    ///
    /// ⚠ THE SHAPE NEEDS A DEADLINE AND NOT ONLY A CONDITION. The obvious
    /// release — "hold until the restart stops being a corner" — is a
    /// feedback loop, because the restart origin only decays when somebody
    /// *touches* the ball and the pin is what stops anybody going to it. A
    /// delivery cleared out of the box left twenty-two men standing in a
    /// corner shape watching it: measured at **7 seconds of held shape per
    /// corner** before the deadline landed, against a corner that is over
    /// in one or two.
    pub corner_shape: Option<CornerShapeHold>,
    /// Fire-once guard for the discrete corner aerial contest. A played-out
    /// lofted corner can't thread the congested box to a specific runner, so
    /// once the cross is struck the engine resolves a single skill-weighted
    /// aerial contest (attacking headers vs the defending line + GK command)
    /// and, if an attacker wins, drops the ball on their head to be headed
    /// on goal. False = armed (a corner has been awarded, not yet resolved);
    /// true = nothing to resolve.
    pub corner_contest_resolved: bool,
    /// Corner routine picked by `pick_corner_routine` at corner setup.
    /// Lets the corner aerial-contest in `resolve_corner_contest` and
    /// downstream xG accounting know whether the delivery is targeting
    /// the near post, far post, penalty spot, or short. Cleared after
    /// the corner resolves. `None` whenever a corner isn't pending.
    pub pending_corner_routine: Option<CornerRoutine>,
    /// The corner taker's `set_piece_delivery` composite (0..1), stamped
    /// when the corner is awarded. `resolve_corner_contest` weighs the
    /// aerial contest by it, so a specialist's whipped ball genuinely
    /// finds a head more often than a full-back's hopeful clip. 0.5 —
    /// an ordinary delivery — whenever no corner is pending.
    pub pending_corner_delivery: f32,
    /// Fire-once guard for the OPEN-PLAY cross aerial contest, the
    /// sibling of `corner_contest_resolved`. A lofted cross is aimed at a
    /// patch of the box, not at a pair of feet, so it cannot be settled by
    /// whichever player's state machine happens to run first — the engine
    /// resolves one skill-weighted contest (best attacking header vs the
    /// nearest defenders vs the keeper's command of his area) and drops
    /// the ball on the winner. `false` = armed (a lofted cross is in the
    /// air, not yet resolved); `true` = nothing to resolve, which is also
    /// the resting state for ground deliveries and every ordinary pass.
    pub cross_contest_resolved: bool,
    /// Which delivery the crossing model chose for the ball currently in
    /// flight. Read by the contest (a whipped near-post ball is harder for
    /// a keeper to claim than a floated one) and cleared with the rest of
    /// the pending-pass metadata.
    pub pending_cross_type: Option<CrossType>,
    /// Player an engine-level aerial contest has already awarded the ball
    /// to. Their heading state must NOT roll a second duel — the contest
    /// is the duel, and re-rolling it is double jeopardy (the bug the
    /// corner path documents and works around with a clean-contact
    /// floor). Cleared on the next touch or when the ball settles.
    pub aerial_contest_winner: Option<u32>,
    /// A decided aerial contest whose ball is still on its way to the
    /// winner. See [`AerialDelivery`] — this is what lets the corner and
    /// cross contests keep their duel and lose their teleport.
    pub aerial_delivery: Option<AerialDelivery>,
    /// The man an [`AerialDelivery`] has just reached, waiting to be put
    /// into his heading state.
    ///
    /// Stashed rather than applied because `Ball::update` holds the
    /// players immutably — the same reason, and the same shape, as
    /// [`Self::pending_set_piece_teleport`]. Drained by
    /// `FootballEngine::apply_pending_aerial_strike`.
    pub pending_aerial_strike: Option<u32>,
    /// Counter for "ball is owned but nothing is happening" stalls.
    /// The unowned-stall warning can't see these because ownership is
    /// set, but visually the ball sits with a player who isn't moving,
    /// isn't passing, isn't dribbling — same "ball stuck" symptom, no
    /// warning. Reset whenever owner changes or any meaningful motion
    /// resumes; fires a separate warning once it crosses the threshold.
    pub owned_stuck_ticks: u32,
    /// Diagnostic only: was the ball owned by a player in a TakeBall
    /// state on the previous full tick? Used to count spells rather
    /// than ticks — see `dead_ball_diag::TAKEBALL_OWN_SPELLS`.
    #[cfg(feature = "match-logs")]
    pub takeball_owned_last_tick: bool,
    pub owned_stuck_logged: bool,
    /// Position-based stall detector — catches cases the owned/unowned
    /// counters miss, specifically: rapid ownership flipping keeps
    /// resetting both counters (each "change" looks like progress) but
    /// the ball physically never leaves a small region. We sample the
    /// ball's position every N ticks and if it hasn't moved more than
    /// a threshold distance over a window, it's stuck regardless of
    /// who "owns" it at any given instant.
    pub stall_anchor_pos: Vector3<f32>,
    pub stall_anchor_tick: u32,

    /// Trajectory projection cached at the moment a shot is fired. Lets
    /// the goalkeeper commit to an intercept line instead of re-chasing
    /// the ball's current position every tick (which lost ground vs a
    /// 5.6 u/tick shot). `None` whenever the ball isn't a shot in
    /// flight; cleared on catch, goal, or any ownership event.
    pub cached_shot_target: Option<ShotTarget>,

    /// Per-shot lifecycle marker: when the physics-level `try_save_shot`
    /// resolves a shot mid-flight (catch / parry / dangerous parry), it
    /// stores `(keeper_id, shooter_id)` here so the post-tick stat
    /// credit can fire saves and on-target without relying on the GK
    /// state machine to also re-detect the same shot.
    /// Consumed (cleared to `None`) by the event dispatcher once
    /// stats have been credited. This makes saves-on-target match
    /// physics-resolved saves 1:1 — the previous architecture had two
    /// independent save systems (physics and state-machine) where one
    /// changed ball state without crediting and the other rolled
    /// independent saves that often missed.
    pub pending_save_credit: Option<(u32, u32)>,

    /// How hard the keeper had to work for that save, in reach ratio
    /// (0 = straight at him, 1 = full-stretch). Consumed alongside
    /// `pending_save_credit` to put him into the matching STATE.
    ///
    /// Without it the physics save resolves a shot entirely inside ball
    /// physics and the keeper's own state machine never runs, so he never
    /// visibly dives, catches or gets up — the ball simply stops at a
    /// standing man. Measured: ~86 saves a match, of which only 8.4 put
    /// him in `Diving` and `Goalkeeper: Diving` sat below 0.25% of ticks.
    pub pending_save_reach: f32,

    /// Which KIND of save it was, as a `save_accounting_stats` site index
    /// (0 = parry, 1 = catch). Consumed alongside `pending_save_credit`.
    ///
    /// The physics path resolves three outcomes — clean catch, parry round
    /// the post, spilled parry — and used to book all three under "catch"
    /// because that was the only index it had. The accounting table
    /// therefore reported `parry 0` forever, which reads as "parries are
    /// never credited" when in fact they were credited under the wrong
    /// label. Carrying the outcome makes the table say what happened.
    pub pending_save_site: u8,

    /// Last meaningful touch on the ball. Drives restart resolution
    /// (throw-ins, corners, goal kicks) and pass-origin metadata. Updated
    /// from any path that hands ownership to a player (claim, intercept,
    /// block, save, pass) and from foot-deflections that don't transfer
    /// ownership but still count as a touch for the dead-ball decision.
    pub last_touch_player_id: Option<u32>,
    /// Where the last touch happened. Diagnostic-only, so it exists only
    /// under `match-logs` — see `EndlineCensus`.
    #[cfg(feature = "match-logs")]
    pub last_touch_position: Vector3<f32>,
    pub last_touch_team_id: Option<u32>,
    pub last_touch_tick: u64,
    pub last_touch_was_controlled: bool,
    /// Latest tick captured at update entry. Lets per-update helpers
    /// (intercept, block, save, claim, throw-in) record_touch without
    /// having to thread the tick through every signature.
    pub current_tick_cached: u64,

    /// Origin of the most recent live pass — set when a PassTo event
    /// fires from a restart (goal kick, throw-in, corner, free kick).
    /// Read by the delayed-offside resolver. Resets to OpenPlay on any
    /// non-restart pass or once the pass-window expires.
    pub pass_origin_restart: PassOriginRestart,
    /// Set at pass-kick. Lives for the pass window (~220 ticks) and the
    /// offside resolver fires the call only when the receiver becomes
    /// active (touches the ball or claims). Cleared on resolution,
    /// opponent touch, or expiry.
    pub offside_snapshot: Option<OffsideSnapshot>,

    /// Origin of the most-recent live pass (passer's position when the
    /// pass was emitted). Read by the pass-completion classifier to
    /// decide if the pass was progressive / cross / box-entry. None
    /// outside an active pass window.
    pub pending_pass_origin: Option<Vector3<f32>>,
    /// Intended target position of the most-recent live pass. Cleared
    /// alongside `pending_pass_passer`.
    pub pending_pass_target: Option<Vector3<f32>>,
    /// Pass was emitted from the wide channel toward the box — flagged
    /// at emit-time so the completion classifier can credit
    /// `crosses_completed` when the same pass is received.
    pub pending_pass_was_cross: bool,

    /// Snapshot of the most recently *completed* pass — populated by
    /// `credit_completed_pass` AFTER it bumps `passes_completed` and
    /// BEFORE it clears `pending_pass_*`. The shot-handler key-pass
    /// linker reads these (rather than `pending_pass_*` which the
    /// completion path nulls out) so a receive-then-shoot sequence
    /// still credits the assister with a key pass. None outside the
    /// shot-after-pass window.
    pub last_completed_pass_passer_id: Option<u32>,
    pub last_completed_pass_receiver_id: Option<u32>,
    pub last_completed_pass_tick: u64,

    /// Opponents that were within the pressing radius of the passer at
    /// pass-emit time. Read by the interception handler to credit a
    /// successful pressure when their close-range presence forced the
    /// turnover. Capped at 4 entries — the count of "real" pressers in
    /// any single moment is small. Cleared at pass-completion or
    /// pass-window expiry.
    pub pressers_at_pass: [u32; 4],
    pub pressers_at_pass_count: u8,

    /// Most-recent shot's **post-shot** expected goal — the probability a
    /// league-average keeper concedes it, from
    /// [`SaveModel::expected_goal_on_target`]. Booked against the
    /// defending keeper by `note_shot_faced` as both the expectation his
    /// goals-prevented is measured against and the sign of his
    /// `xg_prevented` ledger. Cleared on resolution (save / goal / wide /
    /// over) and on any non-shot ownership change.
    ///
    /// Post-shot, not pre-shot, and the distinction is the whole point:
    /// the pre-shot value describes the SITUATION the defence conceded,
    /// so charging the keeper's expectation with it made a keeper behind
    /// a good defence look like one facing league-average chances however
    /// tame the strikes actually were. This value describes the STRIKE.
    pub last_shot_xgot: f32,
    pub last_shot_shooter_id: Option<u32>,
    /// Tick the ball was last STRUCK as a shot, whoever has touched it
    /// since. `check_goal` needs a property of the BALL here, not of
    /// whoever happens to be its `previous_owner` when it crosses the
    /// line: a keeper who gets a hand to a shot becomes the previous
    /// owner, and the shot-provenance test then failed on him and
    /// refused the goal. Measured 2026-08: 2604 balls per 300 matches
    /// crossed the line and were rejected — 34% of all shots, and the
    /// single largest reason the engine scored 1.6 goals a game.
    pub last_shot_struck_tick: u64,

    /// Shot-lifecycle census state (`match-logs` only). Set at the strike
    /// and cleared the moment the shot resolves; see
    /// [`Ball::census_shot_fate`], which is the only reader. `0.0` in
    /// `census_shot_dist` means no shot is being tracked.
    #[cfg(feature = "match-logs")]
    pub census_shot_live: bool,
    #[cfg(feature = "match-logs")]
    pub census_shot_dist: f32,
    #[cfg(feature = "match-logs")]
    pub census_shot_side: Option<PlayerSide>,

    /// Tick of the most recent live rebound — a dangerous GK parry or
    /// a loose shot-block deflection that left the ball contestable in
    /// front of goal. Read by the team shot gate: within the rebound
    /// window (~3 s) the team-level shot SPACING and build-up gates
    /// are suspended so the box scramble / tap-in — one of football's
    /// core goal patterns — can actually fire. The per-possession shot
    /// cap (2) still rules out machine-gun scrambles. 0 = no rebound.
    pub last_rebound_tick: u64,

    /// Last meaningful giveaway: the player who lost possession via a
    /// misplaced pass that was intercepted by an opponent. Read by the
    /// "errors leading to shot/goal" linker — when an opponent shoots
    /// within the response window after this is stamped, the giver is
    /// charged with the error.
    pub last_giveaway_player_id: Option<u32>,
    pub last_giveaway_team_id: Option<u32>,
    pub last_giveaway_tick: u64,
    /// Defensive zone the giveaway happened in (from the giver's
    /// perspective). Lets the goal handler credit
    /// `errors_to_goal_own_box` when an opponent converts a giveaway
    /// from inside the giver's own box.
    pub last_giveaway_was_own_box: bool,
    /// Player charged with `errors_leading_to_shot` for the shot
    /// currently in flight. Held from shoot-time until the shot
    /// resolves; if the shot becomes a goal we also bump
    /// `errors_leading_to_goal` on this player.
    pub pending_error_to_shot_player_id: Option<u32>,
    /// Goalkeeper who has just flapped a claim — dropped a cross, punched
    /// it back into the box, missed the ball entirely. Held until the
    /// possession resolves so a shot that follows can be charged to the
    /// keeper as `gk_failed_claims_to_shot` (and, if it goes in,
    /// `gk_failed_claims_to_goal`).
    ///
    /// Deliberately SEPARATE from `pending_error_to_shot_player_id`: the
    /// rating de-dups nested mistake counters (see `errors_and_cards`),
    /// and a failed claim that also stamped `errors_leading_to_goal`
    /// would bill one incident through two lanes — the triple-counting
    /// bug that once dropped a one-conceded keeper to ~3.9.
    pub pending_failed_claim_gk_id: Option<u32>,
    pub pending_failed_claim_tick: u64,
    /// Set once the flap has been charged as `gk_failed_claims_to_shot`.
    /// The id survives so a goal from the same scramble can still be
    /// promoted, but a second shot in the same possession must not bill
    /// the keeper twice for one mistake.
    pub pending_failed_claim_charged: bool,

    /// Carry tracking. `carry_owner` is the player currently dribbling /
    /// running with the ball; `carry_start_position` is where the carry
    /// began. Evaluated when the carry ends (owner change / shot / pass)
    /// to credit progressive carries and box entries.
    pub carry_owner: Option<u32>,
    pub carry_start_position: Vector3<f32>,

    /// Who last put the ball into play out of their own control — a pass,
    /// a goal kick, a clearance — with where and when they did it.
    ///
    /// Read by [`Ball::blocked_recollect_player`] to stop the releaser
    /// immediately re-collecting a delivery that has barely moved. Real
    /// football has no rule against running onto your own pass, but the
    /// engine had a degenerate cycle that did need one: a goalkeeper
    /// whose kick landed at his feet picked it up, kicked again, and
    /// never got out of his own six-yard box. The ball-travel test (not a
    /// blanket ban) is what keeps a legitimate one-two or chip-over-the-
    /// top intact.
    ///
    /// Cleared the moment any OTHER player touches the ball, and on every
    /// dead-ball restart.
    pub last_release_player_id: Option<u32>,
    pub last_release_position: Vector3<f32>,
    pub last_release_tick: u64,
    /// Whether that release was out of a goalkeeper's HANDS. Drives the
    /// second-touch half of Law 12: once a keeper puts the ball back into
    /// play he may not handle it again until someone else has played it.
    pub last_release_from_hands: bool,

    /// The ball is in a goalkeeper's gloves.
    ///
    /// Distinct from `current_owner` being a keeper, which only says he has
    /// it at his feet. A ball in the hands is out of play in every sense
    /// that matters to the other twenty-one players: it cannot be tackled,
    /// intercepted, or claimed, and pressing it is pointless. Nothing
    /// represented that before — a keeper who had caught a cross could be
    /// dispossessed by a forward standing next to him, because
    /// `check_ball_ownership` just hands the ball to the best tackler
    /// within 5u whoever they are.
    pub held_in_hands: bool,

    /// The last touch was a team-mate deliberately playing the ball with
    /// their feet (a pass or a throw-in), which is what arms the back-pass
    /// prohibition. Set by [`Ball::note_deliberate_kick`] and cleared by
    /// [`Ball::record_touch`] — so ANY subsequent touch by anyone, of any
    /// kind, disarms it automatically. That is exactly the Law: a header
    /// back, a deflection, an opponent's touch, all restore the keeper's
    /// right to use his hands.
    pub last_touch_was_deliberate_kick: bool,
}

/// Whether a goalkeeper may pick this ball up, and if not, why not.
///
/// The engine had no notion of this at all: `Catching` never checked where
/// it was happening, so a keeper would take the ball cleanly in his hands
/// forty metres from his own goal, and a back-pass was gathered exactly
/// like a cross.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandlingVerdict {
    /// Hands are legal — gather it.
    Legal,
    /// Outside his own penalty area. Handling here is a direct free kick
    /// (and usually a red card), so a keeper simply does not do it.
    OutsideArea,
    /// Deliberately kicked to him by a team-mate, or thrown in by one.
    /// Indirect free kick if he handles it, so he plays it with his feet.
    BackPass,
    /// He has already released it and nobody else has touched it since.
    SecondTouch,
}

impl HandlingVerdict {
    #[inline]
    pub fn is_legal(self) -> bool {
        matches!(self, HandlingVerdict::Legal)
    }
}

/// Projection of a shot at the moment it's taken. The `PreparingForSave`
/// and `Catching` goalkeeper states read this to know where the ball
/// will actually arrive rather than chasing its current position — a
/// diving keeper commits to a spot on the line, they don't track the
/// ball every frame.
#[derive(Debug, Clone, Copy)]
pub struct ShotTarget {
    /// y-coordinate at which the shot is projected to cross the goal
    /// line, in field units. Falls outside the posts if the shot is
    /// going wide — the keeper should still attempt the save, the
    /// post-vs-net check happens in `check_goal`.
    pub goal_line_y: f32,
    /// z-coordinate (height) at projected crossing. Above `GOAL_HEIGHT`
    /// (2.44) is an over-the-bar ball the keeper shouldn't commit to.
    pub goal_line_z: f32,
    /// Goal the ball is heading for — left (x=0) or right (x=field_w).
    /// Used so the correct keeper reads the cache.
    pub defending_side: PlayerSide,
    /// True once the physics save roll has been resolved for THIS
    /// shot. The roll used to run on every tick the ball sat inside the
    /// keeper's reach window (~2-3 ticks), compounding to ~88% per shot
    /// from a 0.55 per-tick cap — which is why `skill_mult` needed five
    /// successive empirical retunes whenever state-machine timing moved
    /// the window length. One shot, one roll: the probability below is
    /// now a genuine per-shot save chance calibrated straight against
    /// real save% (~67% of shots on target).
    pub save_rolled: bool,
    /// True once the block roll has been resolved for THIS shot — the
    /// same one-shot-one-roll discipline `save_rolled` enforces. Without
    /// it, widening the block window means rolling once per tick the
    /// defender stays in the lane, so the block rate becomes a function
    /// of flight timing rather than of the model.
    pub block_rolled: bool,
    /// A defender who has WON the block but whom the ball has not reached
    /// yet, with the outcome roll already drawn for him.
    ///
    /// The block window reaches 90u (11 m) ahead of the ball, because that
    /// is the range over which a defender can still get across to a shot.
    /// The deflection used to fire on the tick the roll succeeded, so the
    /// ball turned up to eleven metres before it got to the man who turned
    /// it — the same defect the save had, and between them they were the
    /// only rebounds near a goal a viewer ever saw. Committing here and
    /// resolving when the ball arrives keeps the block RATE exactly where
    /// it was calibrated (one roll, at the same moment, off the same
    /// candidate) while putting the contact on the body.
    ///
    /// The outcome roll — which of controlled / corner / safe / loose /
    /// unlucky the deflection is — is drawn when the block is WON and
    /// carried here, so that the branch a block takes is decided at the same
    /// point in the shared RNG stream as before. Only the deflection's
    /// direction spread is drawn on arrival, and that picks an angle rather
    /// than an outcome.
    pub blocked_by: Option<(u32, f32)>,
    /// Set when the shot took a deflection off a body in the lane.
    /// Catching/Diving states damp the save probability — the keeper
    /// was set for the original trajectory and the redirected ball is
    /// arriving on a new line they haven't committed to.
    pub deflected: bool,
    /// The striker's `shot_threat` composite (0..1) at the moment he hit
    /// it. Carried on the shot rather than looked up at save time
    /// because the save resolves several ticks later, by which point
    /// `previous_owner` may have moved on and the shooter's fatigue
    /// bands have drifted.
    ///
    /// `SaveModel` reads this to score the save as a CONTEST against the
    /// man who struck the ball instead of against an absolute bar — see
    /// `SaveModel::skill_multiplier`. Defaults to
    /// `SaveModel::NEUTRAL_THREAT` on the paths that synthesise a shot
    /// target without a shooter, which reproduces the old
    /// absolute-quality behaviour exactly for those cases.
    pub shooter_threat: f32,
    /// Where it was struck from.
    ///
    /// The save contest is resolved when the ball reaches the goal line,
    /// several ticks downstream, by which point the ball's own position
    /// says nothing about the angle it came from. But the angle is the
    /// whole of the keeper's geometry: how much of the mouth his body
    /// covers, and how long he had to get there, are both properties of
    /// the line from HERE to the goal. See `SaveModel::wedge`.
    pub struck_from: Vector3<f32>,
}

#[derive(Default, Clone)]
pub struct BallFlags {
    pub in_flight_state: usize,
    pub running_for_ball: bool,
}

impl BallFlags {
    pub fn reset(&mut self) {
        self.in_flight_state = 0;
        self.running_for_ball = false;
    }
}

impl Ball {
    pub fn with_coord(field_width: f32, field_height: f32) -> Self {
        let x = field_width / 2.0;
        let y = field_height / 2.0;

        Ball {
            position: Vector3::new(x, y, 0.0),
            start_position: Vector3::new(x, y, 0.0),
            field_width,
            field_height,
            velocity: Vector3::zeros(),
            spin: Vector3::zeros(),
            #[cfg(feature = "match-logs")]
            settled_vz: 0.0,
            center_field_position: x, // initial ball position = center field
            flags: BallFlags::default(),
            previous_owner: None,
            current_owner: None,
            take_ball_notified_players: Vec::new(),
            notification_cooldown: 0,
            notification_timeout: 0,
            last_boundary_position: None,
            unowned_stopped_ticks: 0,
            ownership_duration: 0,
            claim_cooldown: 0,
            pass_target_player_id: None,
            pending_pass_passer: None,
            pending_pass_set_tick: 0,
            recent_passers: VecDeque::with_capacity(5),
            possession_source: PossessionSource::Unknown,
            possession_source_for: None,
            intercept_rolled: false,
            contested_claim_count: 0,
            unowned_ticks: 0,
            stall_start_snapshot: None,
            goal_scored: false,
            in_net: None,
            kickoff_team_side: None,
            cached_landing_position: Vector3::new(x, y, 0.0),
            pending_set_piece_teleport: None,
            awaiting_restart: None,
            pending_restart_station: None,
            pending_corner_teleports: Vec::new(),
            corner_shape: None,
            corner_contest_resolved: true,
            pending_corner_routine: None,
            pending_corner_delivery: 0.5,
            cross_contest_resolved: true,
            pending_cross_type: None,
            aerial_contest_winner: None,
            aerial_delivery: None,
            pending_aerial_strike: None,
            owned_stuck_ticks: 0,
            #[cfg(feature = "match-logs")]
            takeball_owned_last_tick: false,
            owned_stuck_logged: false,
            stall_anchor_pos: Vector3::new(x, y, 0.0),
            stall_anchor_tick: 0,
            cached_shot_target: None,
            pending_save_credit: None,
            pending_save_reach: 0.0,
            pending_save_site: 1,
            last_touch_player_id: None,
            #[cfg(feature = "match-logs")]
            last_touch_position: Vector3::new(x, y, 0.0),
            last_touch_team_id: None,
            last_touch_tick: 0,
            last_touch_was_controlled: false,
            current_tick_cached: 0,
            pass_origin_restart: PassOriginRestart::OpenPlay,
            offside_snapshot: None,
            pending_pass_origin: None,
            pending_pass_target: None,
            pending_pass_was_cross: false,
            last_completed_pass_passer_id: None,
            last_completed_pass_receiver_id: None,
            last_completed_pass_tick: 0,
            pressers_at_pass: [0; 4],
            pressers_at_pass_count: 0,
            last_shot_xgot: 0.0,
            last_shot_shooter_id: None,
            last_shot_struck_tick: 0,
            #[cfg(feature = "match-logs")]
            census_shot_live: false,
            #[cfg(feature = "match-logs")]
            census_shot_dist: 0.0,
            #[cfg(feature = "match-logs")]
            census_shot_side: None,
            last_rebound_tick: 0,
            last_giveaway_player_id: None,
            last_giveaway_team_id: None,
            last_giveaway_tick: 0,
            last_giveaway_was_own_box: false,
            pending_error_to_shot_player_id: None,
            pending_failed_claim_gk_id: None,
            pending_failed_claim_tick: 0,
            pending_failed_claim_charged: false,
            carry_owner: None,
            carry_start_position: Vector3::new(x, y, 0.0),
            last_release_player_id: None,
            last_release_position: Vector3::new(x, y, 0.0),
            last_release_tick: 0,
            last_release_from_hands: false,
            held_in_hands: false,
            last_touch_was_deliberate_kick: false,
        }
    }

    /// Record that `player_id` has just released the ball into open play
    /// from `position`. See [`Ball::last_release_player_id`].
    pub fn note_release(&mut self, player_id: u32, position: Vector3<f32>, tick: u64) {
        self.last_release_player_id = Some(player_id);
        self.last_release_position = position;
        self.last_release_tick = tick;
        // Any release puts the ball back in open play — it is no longer in
        // anyone's gloves. `from_hands` is stamped separately by the
        // goalkeeper release paths.
        self.last_release_from_hands = self.held_in_hands;
        self.held_in_hands = false;
    }

    /// A field player has deliberately played the ball with their feet.
    ///
    /// Routed through `record_touch` so the touch bookkeeping stays in one
    /// place, then raises the deliberate-kick flag that
    /// [`Ball::is_backpass_to`] reads. Because `record_touch` LOWERS the
    /// flag, the very next touch by anybody disarms the back-pass bar with
    /// no explicit clearing anywhere.
    pub fn note_deliberate_kick(&mut self, player_id: u32, team_id: u32, tick: u64) {
        self.record_touch(player_id, team_id, tick, true);
        self.last_touch_was_deliberate_kick = true;
    }

    /// True when handling this ball would breach the back-pass law: the
    /// last touch was a team-mate of `keeper_id` deliberately kicking or
    /// throwing it.
    pub fn is_backpass_to(&self, keeper_id: u32, keeper_team: u32) -> bool {
        self.last_touch_was_deliberate_kick
            && self.last_touch_team_id == Some(keeper_team)
            && self.last_touch_player_id != Some(keeper_id)
    }

    /// True when `keeper_id` put this ball back into play from his hands
    /// and nobody has played it since — the second-touch prohibition.
    pub fn awaiting_touch_after_release_by(&self, keeper_id: u32) -> bool {
        self.last_release_from_hands && self.last_release_player_id == Some(keeper_id)
    }

    /// Height the ball rides at while it is being carried, in metres.
    ///
    /// At a player's feet normally — and at CHEST HEIGHT in a keeper's
    /// gloves. `held_in_hands` was a rules concept only: the ball still
    /// snapped to z = 0, so a keeper who had just caught a cross was drawn
    /// with it lying on the grass by his boots, and the replay showed a
    /// goalkeeper who never uses his hands for anything. Nothing else was
    /// wrong — the viewer draws exactly the height it is given.
    ///
    /// 1.15 m is where a man of the model's 1.79 m holds a ball into his
    /// chest. It stays well under `is_aerial`'s 2.3 m and under the 2.44 m
    /// crossbar, so no height-gated rule changes behaviour because of it.
    pub fn carry_height(&self) -> f32 {
        if self.held_in_hands { 1.15 } else { 0.0 }
    }

    /// Is this player close enough to the ball to be given it?
    ///
    /// # Why every grant has to ask
    ///
    /// [`MAX_OWNER_TRACK_DISTANCE`] is the furthest the ball will follow
    /// the player who owns it. Grant possession beyond that and
    /// [`Ball::move_to`] disowns the ball on the very next tick — but by
    /// then the granting handler has already **zeroed the velocity**, so
    /// what `move_to` releases is a dead ball. It stops in mid-pitch with
    /// nobody near it, everyone converges on it, and somebody eventually
    /// plays it backwards. Reported from the viewer exactly that way, and
    /// counted at 87 times a match by `reception_diag::OWNER_TOO_FAR`.
    ///
    /// So the check is not new — `move_to` has always made it. It was just
    /// made one tick too late to be survivable. Asking here, before
    /// anything is mutated, means the grant simply does not happen and the
    /// ball flies on untouched, which is the same outcome minus the
    /// wreckage.
    ///
    /// Measured in the XY plane, exactly as `move_to` measures it: a ball
    /// directly overhead is within reach whatever its height.
    pub fn within_possession_reach(&self, player_position: Vector3<f32>) -> bool {
        let dx = player_position.x - self.position.x;
        let dy = player_position.y - self.position.y;
        dx * dx + dy * dy <= MAX_OWNER_TRACK_DISTANCE * MAX_OWNER_TRACK_DISTANCE
    }

    /// Take the ball into `keeper_id`'s gloves.
    pub fn gather_in_hands(&mut self, keeper_id: u32, team_id: u32, tick: u64) {
        #[cfg(feature = "match-logs")]
        ownership::reception_diag::GATHERS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.record_touch(keeper_id, team_id, tick, true);
        self.held_in_hands = true;
        self.last_release_from_hands = false;
    }

    /// The player currently barred from re-collecting the ball because
    /// they released it themselves and it has not yet gone anywhere, or
    /// `None` when nobody is barred.
    ///
    /// Bounded on BOTH axes so it can never become a deadlock of its own:
    /// the bar lifts as soon as the ball has travelled `MIN_TRAVEL`, and
    /// unconditionally after `MAX_BLOCK_TICKS` whether it moved or not.
    /// Without the time bound, a ball that stops 2 m from a lone player
    /// with no one else nearby would sit there forever.
    pub fn blocked_recollect_player(&self) -> Option<u32> {
        /// 5 m. Short enough that a genuine one-two or a chip over the top
        /// is unaffected; long enough that a delivery which never left the
        /// striker's own feet is caught.
        const MIN_TRAVEL: f32 = 40.0;
        /// 2 s. Deadlock escape — see above.
        const MAX_BLOCK_TICKS: u64 = 200;

        let releaser = self.last_release_player_id?;
        if self
            .current_tick_cached
            .saturating_sub(self.last_release_tick)
            > MAX_BLOCK_TICKS
        {
            return None;
        }
        let dx = self.position.x - self.last_release_position.x;
        let dy = self.position.y - self.last_release_position.y;
        if dx * dx + dy * dy >= MIN_TRAVEL * MIN_TRAVEL {
            return None;
        }
        Some(releaser)
    }

    /// The velocity of a defensive header put behind for a corner, struck
    /// from `from`.
    ///
    /// Extracted from `FootballEngine::hook_it_behind` so the two callers
    /// that need it — the tick-engine resolver, and
    /// [`tick_aerial_delivery`](Self::tick_aerial_delivery) applying a
    /// contest whose ball has just arrived — read one piece of geometry.
    ///
    /// A hooked header is high and short: it only has to cross the line,
    /// and it has to finish OUTSIDE the posts, because a clearance across
    /// the face of goal is an own goal rather than a clearance.
    pub fn hook_behind_velocity(
        from: Vector3<f32>,
        attacked_goal: Vector3<f32>,
        field_height: f32,
    ) -> Vector3<f32> {
        /// Wide of the post, on the side he is already on.
        const CLEAR_OF_POST: f32 = 55.0;
        let out_y = if from.y >= attacked_goal.y {
            (attacked_goal.y + CLEAR_OF_POST).min(field_height - 6.0)
        } else {
            (attacked_goal.y - CLEAR_OF_POST).max(6.0)
        };
        // Just past the goal line, on the far side of it.
        let goal_line_dir = (attacked_goal.x - from.x).signum();
        let out_x = attacked_goal.x + goal_line_dir * 18.0;
        let target = Vector3::new(out_x, out_y, 0.0);
        let to_target = target - from;
        let dist = to_target.magnitude().max(0.1);
        let vz = Self::launch_speed_for_apex(5.0);
        let hang = Self::hang_ticks(vz).max(1.0);
        let speed = ((dist / hang) * 1.5).clamp(0.30, 2.6);
        let dir = to_target / dist;
        Vector3::new(dir.x * speed, dir.y * speed, vz)
    }

    /// Record a meaningful touch. Drives restart resolution. `controlled`
    /// distinguishes a clean reception from a deflection / failed save.
    pub fn record_touch(&mut self, player_id: u32, team_id: u32, tick: u64, controlled: bool) {
        // Where the touch happened, so a downstream diagnostic can ask how
        // far the ball ran afterwards. Diagnostic-only — see
        // `EndlineCensus`.
        #[cfg(feature = "match-logs")]
        {
            self.last_touch_position = self.position;
            // A lofted delivery that somebody touches before the aerial
            // contest resolves it never gets contested at all — the ball
            // was reserved for one named receiver rather than fought for
            // by the box. Counting these says whether the crossing gap is
            // a DELIVERY problem or a RECEPTION problem.
            if !self.cross_contest_resolved
                && self.pending_cross_type.is_some_and(CrossType::is_lofted)
            {
                CrossDiag::note_touched_first();
            }
            // Pass OVERSHOOT, measured at the chokepoint every touch goes
            // through: a live pass that somebody has just touched tells us
            // how far it was meant to travel and how far it actually did.
            // The whole question "is the ball being struck too hard" is
            // this ratio, and nothing else measures it.
            if let (Some(origin), Some(target)) =
                (self.pending_pass_origin, self.pending_pass_target)
            {
                PassWeightCensus::note(
                    (target - origin).magnitude(),
                    (self.position - origin).magnitude(),
                    self.pass_target_player_id == Some(player_id),
                );
            }
        }
        // Somebody else has been on the ball — whatever the last releaser
        // did is history, and their re-collect bar lifts.
        if self
            .last_release_player_id
            .is_some_and(|id| id != player_id)
        {
            self.last_release_player_id = None;
            // Somebody else has played it, so the keeper who put it into
            // play may use his hands again (Law 12's second-touch bar
            // lifts on any other player's touch).
            self.last_release_from_hands = false;
        }
        // Every touch disarms the back-pass bar. `note_deliberate_kick`
        // re-raises it immediately afterwards for the one touch that
        // should — see its docs.
        self.last_touch_was_deliberate_kick = false;
        // A touch ends whatever aerial contest awarded the ball: the
        // planted header has been struck, or somebody else got there
        // first. Either way the "don't re-roll the duel" grant is spent —
        // and so is the delivery that was carrying it to him.
        self.aerial_contest_winner = None;
        self.aerial_delivery = None;
        // A foot or a chest kills the rotation. Whatever the ball was
        // doing in the air, the next kick decides what it does now.
        self.spin = Vector3::zeros();
        self.last_touch_player_id = Some(player_id);
        self.last_touch_team_id = Some(team_id);
        self.last_touch_tick = tick;
        self.last_touch_was_controlled = controlled;
    }

    /// Clear the offside snapshot. Called on opponent touch, claim, foul,
    /// or pass expiry.
    pub fn clear_offside_snapshot(&mut self) {
        self.offside_snapshot = None;
    }

    /// Force the ball into a clean dead-ball restart state. Centralises
    /// the flag clearing that every set-piece restart (corner / goal
    /// kick / throw-in / kickoff after goal) used to do by hand,
    /// dropping stale open-play metadata so a shot/pass that was in
    /// flight when the ball went dead cannot leak across the restart.
    ///
    /// This is the canonical "ball just went dead — reset everything
    /// open-play touched" helper. New restart paths should call this
    /// rather than zeroing individual fields, so a future field added
    /// to the open-play set is reset automatically.
    pub fn clear_open_play_metadata(&mut self) {
        #[cfg(feature = "match-logs")]
        if self.pending_pass_passer.is_some() {
            use std::sync::atomic::Ordering;
            ownership::reception_diag::DIED_DEAD_BALL.fetch_add(1, Ordering::Relaxed);
        }
        self.cached_shot_target = None;
        self.pass_target_player_id = None;
        self.pending_pass_passer = None;
        self.pending_pass_origin = None;
        self.pending_pass_target = None;
        self.pending_pass_was_cross = false;
        self.offside_snapshot = None;
        // ⚠ `pending_save_credit` is NOT cleared here — it is EARNED, not
        // in-flight. See `clear_for_dead_ball` for the full note.
        self.pending_error_to_shot_player_id = None;
        self.pending_failed_claim_gk_id = None;
        self.pending_failed_claim_charged = false;
        self.last_shot_xgot = 0.0;
        self.last_shot_shooter_id = None;
        // A dead ball ends the shot: without this a stale strike would
        // let the next pass that rolls over the line stand as a goal.
        self.last_shot_struck_tick = 0;
        // A restart is a fresh delivery — the taker may legally be the
        // player who last released the ball in open play, and no dead ball
        // is ever in a keeper's gloves.
        self.last_release_player_id = None;
        self.last_release_from_hands = false;
        self.held_in_hands = false;
        self.last_touch_was_deliberate_kick = false;
    }

    /// Soft invariant check on the ball's lifecycle flags. Returns the
    /// first violation as `Err(msg)` so debug builds and tests can
    /// assert the ball never enters a contradictory state. Production
    /// callers ignore the result — the cost is a handful of field
    /// reads.
    ///
    /// Invariants checked:
    ///   * Open-play shot metadata implies a previous owner (someone
    ///     fired the shot).
    ///   * Pending save credit references a real shooter id (so the
    ///     stat dispatch can fold the on-target back to a shot taker).
    ///   * A pass target id implies a passer id was set when the pass
    ///     was launched (else the receive-classifier has nothing to
    ///     pair the completion to).
    ///   * Ball/owner position coordinates are finite — non-finite x/y/z
    ///     leak into distance comparisons and trigger
    ///     `partial_cmp().unwrap()` panics in sort paths.
    ///   * On a dead-ball restart (corner / goal kick / throw-in /
    ///     free kick / penalty), open-play metadata (cached shot,
    ///     pending pass envelope, save credit, offside snapshot) must
    ///     be cleared — otherwise a shot that was in flight when the
    ///     ball went dead can leak across the restart and credit
    ///     phantom stats.
    ///   * Pending shot xG implies a shooter id (paired metadata,
    ///     consumed together).
    ///   * Pending pass envelope is coherent: a passer implies an
    ///     origin and target position.
    ///   * Carry tracking is consistent: a carrying owner means the
    ///     current owner matches the carrier.
    pub fn check_invariants(&self) -> Result<(), &'static str> {
        if self.cached_shot_target.is_some() && self.previous_owner.is_none() {
            return Err("cached_shot_target without previous_owner");
        }
        if let Some((_keeper, shooter)) = self.pending_save_credit {
            if shooter == 0 {
                return Err("pending_save_credit shooter id is sentinel zero");
            }
        }
        if self.pass_target_player_id.is_some() && self.pending_pass_passer.is_none() {
            return Err("pass_target without pending_pass_passer");
        }
        // Non-finite coordinates leak into distance comparisons and
        // trigger `partial_cmp().unwrap()` panics in nearby/sort paths.
        if !self.position.x.is_finite()
            || !self.position.y.is_finite()
            || !self.position.z.is_finite()
        {
            return Err("ball position has non-finite coordinate");
        }
        if !self.velocity.x.is_finite()
            || !self.velocity.y.is_finite()
            || !self.velocity.z.is_finite()
        {
            return Err("ball velocity has non-finite coordinate");
        }
        // Dead-ball restart cleanliness — any restart origin must drop
        // open-play metadata.
        if matches!(
            self.pass_origin_restart,
            PassOriginRestart::Corner
                | PassOriginRestart::GoalKick
                | PassOriginRestart::ThrowIn
                | PassOriginRestart::Penalty
        ) {
            if self.cached_shot_target.is_some() {
                return Err("dead-ball restart with leftover cached_shot_target");
            }
            // `pending_save_credit` is deliberately NOT checked here.
            //
            // A save that tips the ball round the post stages its credit
            // and triggers the corner in the same `Ball::update`, so the
            // credit is legitimately present at a dead-ball restart for the
            // rest of that tick. The leak this clause was defending
            // against — a credit surviving into a LATER, unrelated restart
            // — cannot happen: `apply_pending_save_credit` drains
            // unconditionally after every ball update in both tick paths.
            // Enforcing the clause instead deleted 1689 earned saves per
            // 200 matches; see `clear_for_dead_ball`.
            if self.offside_snapshot.is_some() {
                return Err("dead-ball restart with leftover offside_snapshot");
            }
        }
        // Pending shot xG and shooter id are kept in lock-step.
        if self.last_shot_xgot > 0.0 && self.last_shot_shooter_id.is_none() {
            return Err("last_shot_xgot without last_shot_shooter_id");
        }
        // Pending pass envelope: any leg must imply the rest.
        if self.pending_pass_passer.is_some()
            && (self.pending_pass_origin.is_none() || self.pending_pass_target.is_none())
        {
            return Err("pending_pass_passer without origin/target metadata");
        }
        // Carry tracking — a current carrier must match the ball owner.
        if let (Some(carrier), Some(owner)) = (self.carry_owner, self.current_owner) {
            if carrier != owner {
                return Err("carry_owner disagrees with current_owner");
            }
        }
        // A ball in the gloves has a keeper holding it. Nothing else in
        // the engine may take ownership away without lowering the flag,
        // or the ball becomes permanently unclaimable.
        if self.held_in_hands && self.current_owner.is_none() {
            return Err("held_in_hands with no owner");
        }
        Ok(())
    }
}

impl Ball {
    /// Calculate where an aerial ball will land (when z reaches 0).
    /// Uses projectile motion: z(t) = h + vz·t − ½g·t² = 0, solving for
    /// the positive root. Ignores air drag — close enough for chase
    /// positioning, and erring long is better than erring short.
    ///
    /// Units are ticks, not seconds: position integration is
    /// `position += velocity` per tick (no dt scaling), while gravity
    /// applies `velocity.z += -GRAVITY * 0.016` per tick. So the
    /// effective per-tick² gravity is `9.81 * 0.016 ≈ 0.157`, and the
    /// resulting `time_to_ground` comes out in ticks — which matches
    /// the horizontal integration `x += vx` per tick.
    pub fn calculate_landing_position(&self) -> Vector3<f32> {
        if self.position.z <= 0.1 || self.current_owner.is_some() {
            return self.position;
        }

        const G_PER_TICK: f32 = GRAVITY_PER_TICK;
        let vz = self.velocity.z;
        let h = self.position.z;

        // Positive root of ½g·t² − vz·t − h = 0
        let discriminant = vz * vz + 2.0 * G_PER_TICK * h;
        let time_to_ground = (vz + discriminant.sqrt()) / G_PER_TICK;

        let landing_x = self.position.x + self.velocity.x * time_to_ground;
        let landing_y = self.position.y + self.velocity.y * time_to_ground;

        // Clamped to the RUN-OFF, not to the pitch. Every chaser steers at
        // this point (it is copied into each player's tick view and read by
        // `LooseBallChase::aim`), so a pitch-bounded answer told them a
        // ball flying out of play was going to land on the line — and the
        // man fetching it stopped there, a couple of metres short of where
        // it actually came down. See [`RunOff`].
        let (min_x, max_x, min_y, max_y) = RunOff::ball_bounds(self.field_width, self.field_height);
        let clamped_x = landing_x.clamp(min_x, max_x);
        let clamped_y = landing_y.clamp(min_y, max_y);

        Vector3::new(clamped_x, clamped_y, 0.0)
    }

    /// Check if the ball is aerial (in the air above player reach)
    pub fn is_aerial(&self) -> bool {
        const PLAYER_REACH_HEIGHT: f32 = 2.3;
        // 0.005 m/tick = 0.5 m/s. The old 0.1 was 10 m/s — a bar set in
        // the units gravity used to be written in, which meant a ball
        // hanging at head height read as "not aerial" the moment it
        // slowed near its apex.
        const MOVING_VERTICALLY: f32 = 0.005;
        self.position.z > PLAYER_REACH_HEIGHT && self.velocity.z.abs() > MOVING_VERTICALLY
    }

    pub fn is_stands_outside(&self) -> bool {
        self.is_ball_outside()
            && self.velocity.norm_squared() < 0.25 // 0.5^2, allow tiny velocities from physics
            && self.current_owner.is_none()
    }

    pub fn is_ball_stopped_on_field(&self) -> bool {
        !self.is_ball_outside()
            && self.velocity.norm_squared() < 6.25 // 2.5^2, catch slow rolling balls that need claiming
            && self.current_owner.is_none()
    }

    pub fn is_ball_outside(&self) -> bool {
        self.position.x <= 0.0
            || self.position.x >= self.field_width
            || self.position.y <= 0.0
            || self.position.y >= self.field_height
    }

    /// Lightweight movement: just apply velocity to position (no ownership logic)
    pub fn apply_movement(&mut self) {
        self.position.x += self.velocity.x;
        self.position.y += self.velocity.y;
        self.position.z += self.velocity.z;
        if self.position.z < 0.0 {
            self.position.z = 0.0;
        }
    }

    pub fn reset(&mut self) {
        self.position.x = self.start_position.x;
        self.position.y = self.start_position.y;
        self.position.z = 0.0;

        self.velocity = Vector3::zeros();
        // The goal is over — whatever is left of it goes with the restart.
        self.in_net = None;

        self.clear_for_dead_ball();
    }

    /// Everything [`Ball::reset`] drops apart from where the ball IS.
    ///
    /// Split out for the goal path: a ball that has just crossed the line is
    /// as dead as one on the centre spot — no owner, no pass in flight, no
    /// shot target, no offside snapshot — but it is emphatically not on the
    /// centre spot, it is in the net travelling at whatever it was hit at.
    /// Sharing the body is what stops the two drifting apart.
    fn clear_for_dead_ball(&mut self) {
        self.current_owner = None;
        self.previous_owner = None;
        self.ownership_duration = 0;
        self.claim_cooldown = 0;

        self.flags.reset();
        self.pass_target_player_id = None;
        self.clear_pass_history();
        self.possession_source = PossessionSource::Unknown;
        self.possession_source_for = None;
        self.intercept_rolled = false;
        self.contested_claim_count = 0;
        self.unowned_ticks = 0;
        self.cached_landing_position = self.position;
        self.pending_set_piece_teleport = None;
        self.awaiting_restart = None;
        self.pending_corner_teleports.clear();
        self.owned_stuck_ticks = 0;
        self.owned_stuck_logged = false;
        self.stall_anchor_pos = self.position;
        self.stall_anchor_tick = 0;
        self.cached_shot_target = None;
        // ⚠ `pending_save_credit` IS NOT OPEN-PLAY METADATA — DO NOT CLEAR.
        //
        // Everything else in this function is state describing a move that
        // is still happening (a shot in flight, a pass in the air, an
        // offside snapshot) and is meaningless once the ball is dead. A
        // save credit is the opposite: it records something that has
        // already HAPPENED. The keeper stopped the shot; the only reason it
        // is "pending" at all is that `Ball` holds `&[MatchPlayer]` and
        // cannot write to the stats sheet itself.
        //
        // Clearing it here deleted the save between earning and delivery,
        // and it did so on the largest class of saves there is. Inside one
        // `Ball::update`: `try_save_shot` stages the credit and tips the
        // ball round the post; sixty lines later, in the SAME call,
        // `check_over_goal` / `check_wide_of_goal` / `check_throw_in` see
        // the ball out of play and restart — wiping the credit before
        // `apply_pending_save_credit` runs. Every save that put the ball
        // out of play was uncredited: 10506 physics saves passed, 8817 were
        // credited, and the missing 1689 dragged saves/on-target down to
        // 63.5% against a calibrated 67%.
        //
        // Nothing can go stale: `apply_pending_save_credit` is called
        // unconditionally right after the ball update in BOTH tick paths
        // (`game_tick_light` and `game_tick_inner`), so a credit is always
        // delivered on the tick it was earned and can never survive into a
        // later restart — which is the only thing the invariant that used
        // to sit on this field was defending against.
        self.last_touch_player_id = None;
        self.last_touch_team_id = None;
        self.last_touch_tick = 0;
        self.last_touch_was_controlled = false;
        self.pass_origin_restart = PassOriginRestart::OpenPlay;
        self.offside_snapshot = None;
        self.last_completed_pass_passer_id = None;
        self.last_completed_pass_receiver_id = None;
        self.last_completed_pass_tick = 0;
        self.last_shot_struck_tick = 0;
        self.last_release_player_id = None;
        self.last_release_from_hands = false;
        self.held_in_hands = false;
        self.last_touch_was_deliberate_kick = false;
    }
}

#[cfg(test)]
mod gk_handling_tests {
    use super::*;

    const KEEPER: u32 = 1;
    const KEEPER_TEAM: u32 = 10;
    const DEFENDER: u32 = 2;
    const OPPONENT: u32 = 3;
    const OPPONENT_TEAM: u32 = 20;

    fn ball() -> Ball {
        Ball::with_coord(840.0, 545.0)
    }

    #[test]
    fn a_teammates_deliberate_kick_bars_the_keepers_hands() {
        let mut b = ball();
        b.note_deliberate_kick(DEFENDER, KEEPER_TEAM, 100);
        assert!(b.is_backpass_to(KEEPER, KEEPER_TEAM));
    }

    #[test]
    fn any_later_touch_disarms_the_backpass_bar() {
        // The Law: a header back, a deflection, an opponent's touch — each
        // restores the keeper's right to use his hands. This falls out of
        // `record_touch` lowering the flag rather than from any explicit
        // clearing, so it holds for touch paths that do not exist yet.
        for (toucher, team, controlled) in [
            (DEFENDER, KEEPER_TEAM, false),   // deflection off a team-mate
            (OPPONENT, OPPONENT_TEAM, true),  // opponent played it
            (OPPONENT, OPPONENT_TEAM, false), // opponent deflected it
        ] {
            let mut b = ball();
            b.note_deliberate_kick(DEFENDER, KEEPER_TEAM, 100);
            b.record_touch(toucher, team, 120, controlled);
            assert!(
                !b.is_backpass_to(KEEPER, KEEPER_TEAM),
                "touch by {toucher} (controlled={controlled}) should have disarmed the bar"
            );
        }
    }

    #[test]
    fn an_opponents_pass_is_not_a_backpass() {
        let mut b = ball();
        b.note_deliberate_kick(OPPONENT, OPPONENT_TEAM, 100);
        assert!(!b.is_backpass_to(KEEPER, KEEPER_TEAM));
    }

    #[test]
    fn a_keeper_does_not_bar_himself_by_kicking() {
        // His own distribution is governed by the second-touch rule, not
        // the back-pass one — and that rule only bites if he released it
        // from his HANDS.
        let mut b = ball();
        b.note_deliberate_kick(KEEPER, KEEPER_TEAM, 100);
        assert!(!b.is_backpass_to(KEEPER, KEEPER_TEAM));
    }

    #[test]
    fn releasing_from_the_hands_bars_a_second_handling() {
        let mut b = ball();
        b.gather_in_hands(KEEPER, KEEPER_TEAM, 100);
        assert!(b.held_in_hands);

        b.note_release(KEEPER, Vector3::new(20.0, 270.0, 0.0), 400);
        assert!(!b.held_in_hands, "releasing empties the gloves");
        assert!(b.awaiting_touch_after_release_by(KEEPER));
    }

    #[test]
    fn the_second_touch_bar_lifts_once_anyone_else_plays_it() {
        let mut b = ball();
        b.gather_in_hands(KEEPER, KEEPER_TEAM, 100);
        b.note_release(KEEPER, Vector3::new(20.0, 270.0, 0.0), 400);
        b.record_touch(DEFENDER, KEEPER_TEAM, 460, true);
        assert!(!b.awaiting_touch_after_release_by(KEEPER));
    }

    #[test]
    fn a_kick_off_the_deck_does_not_arm_the_second_touch_bar() {
        // Only a release FROM THE HANDS does. A keeper who sweeps a ball
        // clear with his feet may pick up the next one.
        let mut b = ball();
        b.note_release(KEEPER, Vector3::new(20.0, 270.0, 0.0), 400);
        assert!(!b.awaiting_touch_after_release_by(KEEPER));
    }

    #[test]
    fn a_dead_ball_clears_every_handling_bar() {
        let mut b = ball();
        b.note_deliberate_kick(DEFENDER, KEEPER_TEAM, 100);
        b.gather_in_hands(KEEPER, KEEPER_TEAM, 110);
        b.note_release(KEEPER, Vector3::new(20.0, 270.0, 0.0), 400);

        b.clear_open_play_metadata();

        assert!(!b.held_in_hands);
        assert!(!b.is_backpass_to(KEEPER, KEEPER_TEAM));
        assert!(!b.awaiting_touch_after_release_by(KEEPER));
    }

    #[test]
    fn a_held_ball_keeps_the_invariants() {
        let mut b = ball();
        b.current_owner = Some(KEEPER);
        b.gather_in_hands(KEEPER, KEEPER_TEAM, 100);
        assert!(b.check_invariants().is_ok());

        // Ownership taken away without lowering the flag would leave the
        // ball permanently unclaimable — the claim path skips it entirely.
        b.current_owner = None;
        assert!(b.check_invariants().is_err());
    }
}
