use crate::PlayerFieldPositionGroup;
use crate::r#match::engine::zones::MatchZone;
use crate::r#match::events::Event;
use crate::r#match::player::strategies::players::skills::SkillCurve;
use crate::r#match::player::events::{PassingEventContext, ShootingEventContext};
use crate::r#match::player::statistics::MatchStatisticType;
use crate::r#match::player::strategies::players::ops::effective_skill::{
    ActionContext as EffSkillCtx, effective_skill,
};
use crate::r#match::player::strategies::players::ops::skill_composites as sc;
use crate::r#match::{
    GoalDetail, MatchContext, MatchField, MatchPlayer, OffsideSnapshot, PassOriginRestart,
    PlayerSide, ResultMatchPositionData, ShotTarget,
};
#[cfg(feature = "match-logs")]
use crate::match_log_info;
use log::debug;
use nalgebra::Vector3;
use rand::{Rng, RngExt};

// ───────────────────────────────────────────────────────────────────────────
// Save-accounting diagnostic counters. Trace each save event's credit pair
// to find why `saves > shots_on_target` (the impossible 145% baseline).
// Each save site increments both `saves_credited[site]` and
// `on_target_paired[site]` — divergence pinpoints where on_target is
// missed (shooter not found, etc.) or where saves are double-credited.
// match-logs feature only.
// ───────────────────────────────────────────────────────────────────────────
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
                // Cross detection at emit-time: passer in a wide channel
                // delivering toward the opposition box. Computed BEFORE
                // the pass handler runs so crosses can use the dedicated
                // crossing-skill error term (a low-crossing player's
                // cross sails harder off-target than their open-play
                // pass would).
                let was_cross = if let Some(side) = passer_side {
                    Self::is_cross_attempt(passer_position, pass_target, side, context)
                } else {
                    false
                };
                Self::handle_pass_to_event(
                    pass_event_model,
                    field,
                    context.total_match_time,
                    was_cross,
                );
                // Tag the ball with the passer for pass-accuracy
                // accounting. Lives for a short window (150 ticks)
                // and is cleared on opponent touch — see ball.rs
                // `pending_pass_passer` docs.
                field.ball.pending_pass_passer = Some(passer_id);
                field.ball.pending_pass_set_tick = context.current_tick();
                field.ball.pending_pass_origin = Some(passer_position);
                field.ball.pending_pass_target = Some(pass_target);
                field.ball.pending_pass_was_cross = was_cross;
                if was_cross {
                    if let Some(passer) = field.get_player_mut(passer_id) {
                        passer.statistics.add_cross_attempt();
                    }
                }
                field.ball.offside_snapshot = snapshot;
                // After the kick, restart context decays back to OpenPlay.
                field.ball.pass_origin_restart = PassOriginRestart::OpenPlay;
            }
            PlayerEvent::ClaimBall(player_id) => {
                Self::record_team_possession_if_switch(player_id, field, context);
                Self::handle_claim_ball_event(player_id, field, context);
            }
            PlayerEvent::MoveBall(player_id, ball_velocity) => {
                Self::handle_move_ball_event(player_id, ball_velocity, field);
            }
            PlayerEvent::GainBall(player_id) => {
                Self::handle_gain_ball_event(player_id, field);
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
                const KEY_PASS_WINDOW_TICKS: u64 = 300;
                let shooter_team = field.get_player(shooter_id).map(|p| p.team_id);
                let mut direct_assister_id: Option<u32> = None;
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
                    }
                    // Single-credit guarantee: even if the shot resolves
                    // into a goal that fires another event, the link is
                    // gone after the first read.
                    field.ball.last_completed_pass_passer_id = None;
                    field.ball.last_completed_pass_receiver_id = None;
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
                Self::handle_shoot_event(shoot_event_model, field, direct_assister_id);
            }
            PlayerEvent::CaughtBall(player_id) => {
                Self::handle_caught_ball_event(player_id, field);
            }
            PlayerEvent::ParriedBall(player_id) => {
                Self::handle_parried_ball_event(player_id, field);
            }
            PlayerEvent::MovePlayer(player_id, position) => {
                Self::handle_move_player_event(player_id, position, field);
            }
            PlayerEvent::TakeBall(player_id) => {
                Self::handle_take_ball_event(player_id, field);
            }
            PlayerEvent::ClearBall(velocity) => {
                Self::handle_clear_ball_event(velocity, field, context);
            }
            PlayerEvent::RequestBallReceive(player_id) => {
                Self::handle_request_ball_receive(player_id, field);
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
        let player = field.get_player_mut(player_id).unwrap();

        player
            .statistics
            .add_goal(context.total_match_time, is_auto_goal);

        // Goal stands → credit on-target to the real scorer. Own goals
        // aren't counted as an on-target shot for the defender that
        // deflected the ball.
        if !is_auto_goal {
            player.memory.credit_shot_on_target();
            #[cfg(feature = "match-logs")]
            save_accounting_stats::ON_TARGET_FROM_GOAL
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }

        // Credit the conceding goalkeeper's `shots_faced` so the rating
        // helper has the right denominator for save percentage. Auto-
        // goals (own-goals) don't count — those aren't shots the GK got
        // beaten by, they're defensive errors. While we're here, debit
        // the GK's xg_prevented by the shot's xG: a 0.7 xG goal beats
        // the keeper, but a 0.05 xG worldie barely dents the keeper's
        // rating because there's nothing to prevent.
        if !is_auto_goal {
            let shot_xg = field.ball.last_shot_xg;
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
                        gk.statistics.shots_faced += 1;
                        if shot_xg > 0.0 {
                            gk.statistics.record_xg_prevented(-shot_xg);
                        }
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
        }
        field.ball.clear_shot_metadata();
        field.ball.pending_error_to_shot_player_id = None;
        field.ball.clear_giveaway();

        context.score.add_goal_detail(GoalDetail {
            player_id,
            stat_type: MatchStatisticType::Goal,
            is_auto_goal,
            time: context.total_match_time,
        });
        context.record_stoppage_time(30_000);

        field.ball.previous_owner = None;
        field.ball.current_owner = None;
        field.ball.pass_target_player_id = None;

        field.reset_players_positions();
        field.ball.reset();
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
        field.ball.clear_pass_history();
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
                let composite = s.technical.first_touch * 0.40
                    + s.technical.technique * 0.20
                    + s.mental.composure * 0.20
                    + s.mental.anticipation * 0.20;
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
    /// `[0.0, 0.30]`. Tuned so:
    ///   * `first_touch ≈ 5` + 2 close opponents → ~11%
    ///   * `first_touch ≈ 10` + 2 close opponents → ~4%
    ///   * `first_touch ≈ 15` + 2 close opponents → ~0.7%
    /// The `^2.5` skill curve makes elite reception virtually immune
    /// while weak receivers under pressure visibly leak control.
    /// Extracted as a pure function so the gradient is unit-testable
    /// without standing up a `MatchField`.
    #[inline]
    pub(crate) fn first_touch_loss_probability(composite01: f32, pressure_count: usize) -> f32 {
        let pressure_mult = 1.0 + (pressure_count.min(4) as f32) * 0.6;
        ((1.0 - composite01.clamp(0.0, 1.0)).powf(2.5) * 0.10 * pressure_mult).clamp(0.0, 0.30)
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
        total_match_time_ms: u64,
        was_cross: bool,
    ) {
        let mut rng = rand::rng();
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
        let lead_ticks = flight_time_est * lead_fraction;
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
            let cross_multiplier = 1.0 + crossing_shortfall.powf(1.2) * 1.2;
            base_max_position_error * cross_multiplier
        } else {
            base_max_position_error
        };

        // Add random targeting error
        let mut target_error_x = if max_position_error > f32::EPSILON {
            rng.random_range(-max_position_error..max_position_error)
        } else {
            0.0
        };
        let mut target_error_y = if max_position_error > f32::EPSILON {
            rng.random_range(-max_position_error..max_position_error)
        } else {
            0.0
        };

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

        let actual_pass_vector = actual_target - pass_origin;
        let actual_horizontal_distance = Self::calculate_horizontal_distance(&actual_pass_vector);

        // Calculate pass force with power variation
        // Bad players hit passes with inconsistent power
        let power_consistency = 1.0 + (skills.technique * skills.stamina * 0.1);
        let power_variation_range = (1.0 - overall_quality) * 0.35;
        let power_variation = rng.random_range(
            power_consistency - power_variation_range..power_consistency + power_variation_range,
        );
        let adjusted_force = event_model.pass_force * power_variation;

        // Calculate horizontal velocity to reach target
        let horizontal_velocity =
            Self::calculate_horizontal_velocity(&actual_pass_vector, adjusted_force);

        // Determine trajectory type based on context, not just distance
        let passer = field.get_player_mut(event_model.from_player_id).unwrap();
        let passer_team_id = passer.team_id;
        let passer_is_goalkeeper = passer.tactical_position.current_position.is_goalkeeper();

        let trajectory_type = Self::select_trajectory_type_contextual(
            actual_horizontal_distance,
            &skills,
            &mut rng,
            &passer_position,
            &actual_target,
            passer_team_id,
            &field.players,
        );

        // Goalkeeper long kicks must always be high arcs (goal kicks from penalty area)
        let trajectory_type = if passer_is_goalkeeper && actual_horizontal_distance > 60.0 {
            TrajectoryType::HighArc
        } else {
            trajectory_type
        };

        // Calculate z-velocity to reach target with chosen trajectory type
        let z_velocity = Self::calculate_trajectory_to_target(
            actual_horizontal_distance,
            &horizontal_velocity,
            trajectory_type,
            &skills,
            &mut rng,
        );

        let base_max_z = Self::calculate_max_z_velocity(actual_horizontal_distance, &skills);
        // Goalkeeper long kicks get a higher z-cap — goal kicks should fly high
        let max_z_velocity = if passer_is_goalkeeper && actual_horizontal_distance > 60.0 {
            base_max_z * 1.5
        } else {
            base_max_z
        };
        let final_z_velocity = z_velocity.min(max_z_velocity);

        // Calculate final velocity
        let mut final_velocity = Vector3::new(
            horizontal_velocity.x,
            horizontal_velocity.y,
            final_z_velocity,
        );

        // CRITICAL: Validate velocity to prevent cosmic-speed passes.
        // Field is 840u = 105m (1u = 0.125m), simulation at 100 ticks/s.
        // Old 7.0 u/tick = 87.5 m/s = 315 km/h — the same unit-conversion
        // error family as everything else, assuming "1u ≈ 0.5m". Real
        // football pass speeds: short ball 5-15 m/s, medium 15-25 m/s,
        // elite driven/long pass tops out ~35 m/s. Cap at 3.2 u/tick
        // (40 m/s) — elite piledriver territory, never broken by any
        // outfield pass in real play. Matches the MAX_SHOT_VELOCITY
        // calibration and restores the real shot/player speed ratio.
        const MAX_PASS_VELOCITY: f32 = 3.2;

        // Check for NaN or infinity
        if final_velocity.x.is_nan()
            || final_velocity.y.is_nan()
            || final_velocity.z.is_nan()
            || final_velocity.x.is_infinite()
            || final_velocity.y.is_infinite()
            || final_velocity.z.is_infinite()
        {
            // Fallback to a safe default velocity
            let safe_direction = actual_pass_vector.normalize();
            final_velocity = Vector3::new(safe_direction.x * 1.5, safe_direction.y * 1.5, 0.3);
        }

        // Clamp velocity magnitude to maximum
        let velocity_magnitude = final_velocity.norm();
        if velocity_magnitude > MAX_PASS_VELOCITY {
            final_velocity = final_velocity * (MAX_PASS_VELOCITY / velocity_magnitude);
        }

        // Apply ball physics
        field.ball.velocity = final_velocity;

        // Record the passer in recent passers history before clearing ownership
        field.ball.record_passer(event_model.from_player_id);

        field.ball.previous_owner = field.ball.current_owner;
        field.ball.current_owner = None;
        field.ball.pass_target_player_id = Some(event_model.to_player_id);

        // Increase in_flight_state based on pass distance to prevent immediate reclaim
        // Ball velocity is low (~1-3 units/tick) so it needs significant protection
        // to travel most of the distance before opponents can claim
        // Short passes (< 30m): 40 ticks — covers ~45% of distance
        // Medium passes (30-80m): 60 ticks — covers ~60% of distance
        // Long passes (> 80m): 80 ticks — covers ~70% of distance
        let flight_protection = if actual_horizontal_distance < 30.0 {
            40
        } else if actual_horizontal_distance < 80.0 {
            60
        } else {
            80
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

        // Calculate velocity needed to reach target accounting for friction and air drag
        // With ground friction factor 0.985/tick, total roll distance = v0 / 0.015
        // So v0 = distance * 0.015 for ground passes
        // Lofted passes experience air drag (proportional to v²) which bleeds much more speed,
        // plus 5% horizontal loss on each bounce — so longer passes need more overshoot
        const GROUND_FRICTION: f32 = 0.015;

        // Distance-dependent overshoot: short passes need little extra,
        // long passes need significantly more to compensate for air drag and bounce losses
        let overshoot = if distance < 50.0 {
            1.15 // Short: ground friction only
        } else if distance < 100.0 {
            1.25 // Medium: slight air drag on lofted balls
        } else if distance < 200.0 {
            1.45 // Long: significant air drag compensation
        } else {
            1.65 // Very long: heavy air drag + multiple bounces
        };

        let needed_velocity = distance * GROUND_FRICTION * overshoot;

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
        rng: &mut impl Rng,
        from_position: &Vector3<f32>,
        to_position: &Vector3<f32>,
        passer_team_id: u32,
        players: &[MatchPlayer],
    ) -> TrajectoryType {
        // Check for obstacles in the passing lane
        let obstacles_in_lane = Self::count_obstacles_in_passing_lane(
            from_position,
            to_position,
            passer_team_id,
            players,
        );

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

        // Distance categories — calibrated for 840-unit field (1 unit ≈ 0.5m)
        let is_short = horizontal_distance <= 30.0; // ~15m — quick one-touch
        let is_medium = horizontal_distance > 30.0 && horizontal_distance <= 60.0; // 15-30m
        let is_long = horizontal_distance > 60.0 && horizontal_distance <= 120.0; // 30-60m
        // > 120 units = very long (60m+)

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
                // Long passes — mix of driven and arced
                if skill_influenced_random < 0.25 {
                    TrajectoryType::LowDriven
                } else if skill_influenced_random < 0.70 {
                    TrajectoryType::MediumArc
                } else {
                    TrajectoryType::HighArc
                }
            } else {
                // Very long passes — always lofted
                if skill_influenced_random < 0.40 {
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
            let crossing_p =
                SkillCurve::new(skills.crossing * 20.0, 14.0, 0.6).probability();
            let vision_p =
                SkillCurve::new(vision_quality * 20.0, 14.0, 0.6).probability();
            let has_good_crossing = rng.random_range(0.0..1.0) < crossing_p;

            if is_short {
                // Short pass with obstacles - chip or lift (NEVER low)
                let chip_p = vision_p * 0.65;
                if skill_influenced_random < chip_p {
                    TrajectoryType::Chip // Smart chip over defender, scaled by vision
                } else {
                    TrajectoryType::MediumArc // Medium loft
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

    /// Count how many opponent players are in the passing lane (obstacles)
    fn count_obstacles_in_passing_lane(
        from_position: &Vector3<f32>,
        to_position: &Vector3<f32>,
        passer_team_id: u32,
        players: &[MatchPlayer],
    ) -> usize {
        const LANE_WIDTH: f32 = 12.0; // Width of the passing lane corridor (accounts for player reach and movement)

        let pass_direction = (*to_position - *from_position).normalize();
        let pass_distance = (*to_position - *from_position).magnitude();

        players
            .iter()
            .filter(|player| {
                // Only count opponents
                if player.team_id == passer_team_id {
                    return false;
                }

                // Vector from passer to player
                let to_player = player.position - *from_position;

                // Project player onto pass line to find closest point
                let projection_length = to_player.dot(&pass_direction);

                // Player must be between passer and target
                if projection_length < 0.0 || projection_length > pass_distance {
                    return false;
                }

                // Calculate perpendicular distance to pass line
                let projection_point = *from_position + pass_direction * projection_length;
                let perpendicular_distance = (player.position - projection_point).magnitude();

                // Player is an obstacle if within lane width
                perpendicular_distance < LANE_WIDTH
            })
            .count()
    }

    /// Calculate z-velocity to reach target with chosen trajectory type
    /// Ground passes stay on the ground, aerial passes use physics
    fn calculate_trajectory_to_target(
        horizontal_distance: f32,
        horizontal_velocity: &Vector3<f32>,
        trajectory_type: TrajectoryType,
        skills: &PassSkills,
        rng: &mut impl Rng,
    ) -> f32 {
        const GRAVITY: f32 = 9.81;

        let horizontal_speed = horizontal_velocity.norm();
        if horizontal_speed < 0.1 {
            return 0.0; // Avoid division by zero
        }

        // Add very small random variation to all trajectories for realism
        let tiny_random = rng.random_range(0.98..1.02);

        match trajectory_type {
            // Ground pass - truly on the ground (rolling)
            TrajectoryType::Ground => {
                // Almost no lift - just enough to handle slight bumps
                // This keeps the ball rolling along the ground
                let base_lift = 0.02 * skills.technique;
                let random_variation = rng.random_range(0.0..0.1);
                base_lift * random_variation * tiny_random // 0.0 to ~0.002 m/s (truly ground)
            }

            // Low driven - stays very close to ground, minimal arc (like real driven passes)
            TrajectoryType::LowDriven => {
                // Very slight lift - driven passes should barely leave the ground
                // Maximum height should be ~0.3-0.8m for realistic driven passes
                let distance_factor = (horizontal_distance / 150.0).clamp(0.2, 0.8);
                let skill_factor = skills.technique * skills.availability_factor;

                let base_z = 0.2 + (distance_factor * 0.5); // 0.2 to 0.7 m/s (much lower)
                let variation = rng.random_range(0.9..1.1);

                base_z * skill_factor * variation * tiny_random
            }

            // Medium arc - moderate parabolic trajectory (height ~1.5-3m, reduced)
            TrajectoryType::MediumArc => {
                let base_flight_time = horizontal_distance / horizontal_speed;
                let flight_time = base_flight_time * 0.5; // Reduced from 0.7 - lower arc

                let ideal_z = 0.65 * GRAVITY * flight_time; // Reduced from 0.8

                // Skill affects consistency
                let execution_quality = skills.overall_quality();
                let error_range = (1.0 - execution_quality) * 0.12;
                let error = rng.random_range(1.0 - error_range..1.0 + error_range);

                ideal_z * error * tiny_random
            }

            // High arc - high parabolic trajectory (height ~4-8m)
            TrajectoryType::HighArc => {
                let base_flight_time = horizontal_distance / horizontal_speed;
                let flight_time = base_flight_time * 1.5; // High arc

                let ideal_z = 0.8 * GRAVITY * flight_time;

                // Requires good long passing ability
                let execution_quality =
                    (skills.overall_quality() + skills.long_shots + skills.crossing) / 3.0;
                let error_range = (1.0 - execution_quality) * 0.18;
                let error = rng.random_range(1.0 - error_range..1.0 + error_range);

                ideal_z * error * tiny_random
            }

            // Chip - very high arc over short distance (height ~3-6m)
            TrajectoryType::Chip => {
                // Chips are based on technique, not distance
                let chip_ability =
                    (skills.technique * 0.5 + skills.flair * 0.3 + skills.passing * 0.2)
                        * skills.availability_factor;

                // Base height for chip regardless of distance
                let base_chip_height = 2.5 + (chip_ability * 2.0); // 2.5 to 4.5 m/s

                // Execution error for this difficult skill
                let error_range = (1.0 - chip_ability) * 0.25;
                let error = rng.random_range(1.0 - error_range..1.0 + error_range);

                base_chip_height * error * tiny_random
            }
        }
    }

    fn calculate_max_z_velocity(horizontal_distance: f32, skills: &PassSkills) -> f32 {
        // Combine vision and long_shots for long pass capability
        let long_pass_ability =
            (skills.vision * 0.6 + skills.long_shots * 0.4) * skills.availability_factor;

        if horizontal_distance <= 20.0 {
            // Short passes - mostly ground, slight lift allowed
            0.5 + (long_pass_ability * 0.3)
        } else if horizontal_distance <= 45.0 {
            // Medium passes - driven or low lofted
            1.2 + (long_pass_ability * 0.8)
        } else if horizontal_distance <= 80.0 {
            // Long passes - can be lofted over defenders
            2.5 + (long_pass_ability * 1.5)
        } else if horizontal_distance <= 150.0 {
            // Very long passes - proper arcs
            4.0 + (long_pass_ability * 2.0)
        } else if horizontal_distance <= 250.0 {
            // Ultra-long diagonal switches
            6.0 + (long_pass_ability * 3.0)
        } else {
            // Extreme distance - goalkeeper goal kicks, clearances
            8.0 + (long_pass_ability * 4.0)
        }
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
        //
        // Pass-accuracy accounting: if the ball is within an active
        // pass window (`pending_pass_passer` set by the pass emit and
        // not yet cleared by an opponent touch), and this claimant is
        // a teammate of that passer, credit the pass as completed.
        // Using `pending_pass_passer` instead of `pass_target_player_id`
        // because the target flag gets cleared in many unrelated paths
        // (set-pieces, clearances, save handoffs) and was masking
        // legitimate same-team receptions. The dedicated passer flag
        // persists through the real pass window (~150 ticks).
        if let Some(passer_id) = field.ball.pending_pass_passer {
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
            if same_team && passer_id != player_id {
                // Same single completion path as `BallEvent::PassCompleted`
                // — increments `passes_completed`, classifies progressive
                // / cross / box-entry, and clears the metadata.
                Self::credit_completed_pass(player_id, passer_id, field, context);
            } else if !same_team {
                // Opponent won the pass — accuracy window ends.
                field.ball.clear_pending_pass_metadata();
            }
        }
        field.ball.previous_owner = field.ball.current_owner;
        field.ball.current_owner = Some(player_id);
        field.ball.pass_target_player_id = None;
        field.ball.ownership_duration = 0;
        field.ball.claim_cooldown = CLAIM_COOLDOWN_TICKS;
        field.ball.flags.in_flight_state = 30;
    }

    fn handle_move_ball_event(player_id: u32, ball_velocity: Vector3<f32>, field: &mut MatchField) {
        field.ball.previous_owner = field.ball.current_owner;
        field.ball.current_owner = Some(player_id);

        field.ball.velocity = ball_velocity;
    }

    fn handle_gain_ball_event(player_id: u32, field: &mut MatchField) {
        Self::secure_ball_for(player_id, field);
        field.ball.clear_pass_history();
        field.ball.flags.in_flight_state = 100;
    }

    // Snaps the ball to the winner's feet and zeros velocity — prevents
    // residual velocity from carrying it into the winner's own goal
    // after a tackle/interception/block.
    fn secure_ball_for(player_id: u32, field: &mut MatchField) {
        if let Some(player) = field.players.iter().find(|p| p.id == player_id) {
            field.ball.position = player.position;
            field.ball.position.z = 0.0;
        }
        field.ball.previous_owner = field.ball.current_owner;
        field.ball.current_owner = Some(player_id);
        field.ball.pass_target_player_id = None;
        field.ball.velocity = nalgebra::Vector3::zeros();
        field.ball.flags.in_flight_state = 0;
        field.ball.cached_shot_target = None;
    }

    fn handle_shoot_event(
        shoot_event_model: ShootingEventContext,
        field: &mut MatchField,
        direct_assister_id: Option<u32>,
    ) {
        const GOAL_WIDTH: f32 = 29.0; // Half-width of goal in game units (matches engine GOAL_WIDTH)
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
        const MIN_SHOT_DISTANCE: f32 = 1.0; // Minimum distance to prevent NaN from normalization

        let mut rng = rand::rng();

        // Build the unified shot profile from the same inputs the
        // pre-shot decision saw — the post-hoc xG, the trajectory error
        // budget, the on-target probability and the miskick odds all
        // come from this single object. Replaces the prior
        // (compress_high + raw skill) reads that let a 5/20 player
        // inherit elite conversion through linear blends.
        let minute = sc::minute_from_ticks(shoot_event_model.tick);
        let pre_distance = (shoot_event_model.target - field.ball.position).magnitude();

        // Snapshot the bits of state we need from `field` so we can
        // build the profile without juggling overlapping borrows.
        let (
            condition_pct,
            shooter_position,
            shooter_side,
            technique_skill,
            finishing_skill,
            long_shot_skill,
        ) = {
            let player = field.get_player(shoot_event_model.from_player_id).unwrap();
            (
                (player.player_attributes.condition as f32 / 10_000.0).clamp(0.0, 1.0),
                player.position,
                player.side,
                (player.skills.technical.technique / 20.0).clamp(0.1, 1.0),
                (player.skills.technical.finishing / 20.0).clamp(0.1, 1.0),
                (player.skills.technical.long_shots / 20.0).clamp(0.1, 1.0),
            )
        };

        // Pressure counts: scan opposing-side players close to the
        // shooter. We don't have the StateProcessingContext grid here,
        // so a small linear scan is used; the population of opponents
        // is bounded (<=11) so the cost is negligible.
        let mut pressure_5u: u32 = 0;
        let mut pressure_10u: u32 = 0;
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

        let inputs = crate::r#match::player::strategies::players::ShotSkillInputs {
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
            crate::r#match::player::strategies::players::ShotSkillProfile::from_player(
                player, &inputs,
            )
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

        // Determine which goal we're shooting at
        let goal_center = shoot_event_model.target;

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

        // Target placement — now driven by `placement_skill`. Spec:
        //   central_rate = clamp(0.68 - placement_skill * 0.58, 0.10, 0.68)
        //   corner_reach = GOAL_WIDTH * (0.42 + placement_skill * 0.48)
        let central_rate = (0.68 - placement_skill * 0.58).clamp(0.10, 0.68);
        let side_rate = (1.0 - central_rate) * 0.5;
        let target_preference = rng.random_range(0.0..1.0);
        let corner_reach = GOAL_WIDTH * (0.42 + placement_skill * 0.48);
        let ideal_y_target = if target_preference < side_rate {
            goal_center.y - corner_reach
        } else if target_preference < side_rate * 2.0 {
            goal_center.y + corner_reach
        } else {
            goal_center.y + rng.random_range(-GOAL_WIDTH * 0.3..GOAL_WIDTH * 0.3)
        };

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
        let max_y_error_raw = 16.0
            * distance_error
            * pressure_error
            * body_error
            * condition_error
            * random_error_scale
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

        // Add random error to y-coordinate
        let y_error = rng.random_range(-max_y_error..max_y_error);
        let mut actual_y_target = ideal_y_target + y_error;

        // Wide-miss chance now leans on the unified profile so a
        // 5/20 finisher pulls shots wide far more often than an elite.
        // Real Opta on-target rate is ~33% across all distances;
        // calibrated downward to land population on-target near that.
        let wide_base = if horizontal_distance < 30.0 {
            0.030
        } else if horizontal_distance < 60.0 {
            0.060
        } else if horizontal_distance < 100.0 {
            0.100
        } else {
            0.150
        };
        let wide_miss_chance = wide_base
            + (1.0 - execution_skill) * 0.07
            + poor_penalty * 0.05
            + pressure_penalty * 0.04
            + low_condition_penalty * 0.03;
        if rng.random_range(0.0f32..1.0) < wide_miss_chance {
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
            actual_y_target += rng.random_range(-GOAL_WIDTH * 1.5..GOAL_WIDTH * 1.5);
        }

        // Clamp to reasonable bounds — allow shots to miss by up to 3x goal width
        let max_miss_distance = GOAL_WIDTH * 3.0;
        let clamped_y_target = actual_y_target.clamp(
            goal_left_post - max_miss_distance,
            goal_right_post + max_miss_distance,
        );

        // Calculate final shot direction
        let actual_target = Vector3::new(goal_center.x, clamped_y_target, 0.0);
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

        let base_z_velocity = if horizontal_distance > 100.0 {
            // Long-range shot - varied heights (technique matters more)
            if shot_style < 0.4 {
                rng.random_range(0.7..1.3) * technique_skill // Low driven (40%)
            } else if shot_style < 0.8 {
                rng.random_range(1.3..2.2) * technique_skill // Normal (40%)
            } else {
                rng.random_range(2.2..3.0) * long_shot_skill // Rising shot (20%)
            }
        } else if horizontal_distance > 50.0 {
            // Medium-range shot - mostly low (finishing matters more)
            if shot_style < 0.6 {
                rng.random_range(0.5..1.0) * finishing_skill // Very low (60%)
            } else if shot_style < 0.9 {
                rng.random_range(1.0..1.7) * technique_skill // Medium (30%)
            } else {
                rng.random_range(1.7..2.3) * technique_skill // High (10%)
            }
        } else {
            // Close-range shot - very low and varied (finishing is key)
            if shot_style < 0.7 {
                rng.random_range(0.2..0.7) * finishing_skill // Ground shot (70%)
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
        let over_bar_base = if horizontal_distance < 30.0 {
            0.015
        } else if horizontal_distance < 60.0 {
            0.030
        } else if horizontal_distance < 100.0 {
            0.055
        } else {
            0.080
        };
        let over_bar_chance = over_bar_base
            + (1.0 - technique_curve).max(0.0) * 0.04
            + poor_penalty * 0.04
            + low_condition_penalty * 0.03;
        let shot_goes_over_bar = rng.random_range(0.0f32..1.0) < over_bar_chance;
        let z_velocity = if shot_goes_over_bar {
            // Shot goes over the bar — set z high enough to clear crossbar (GOAL_HEIGHT = 8.0)
            // Ball needs to reach height > 8.0 during flight, so z_velocity must be significant
            rng.random_range(3.0..6.0) // Guaranteed to fly high over the bar
        } else {
            (base_z_velocity * height_variation * vertical_spin_variation).min(5.0)
        };

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
            // Fallback to a safe default velocity toward the goal
            let safe_direction = (goal_center - field.ball.position).normalize();
            final_velocity = Vector3::new(safe_direction.x * 5.0, safe_direction.y * 5.0, 1.0);
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
        // The keeper's `xg_prevented` ledger is a different concept: a
        // GK only "prevents" goals from shots that threatened the goal.
        // Off-target shots produce no prevented xG (the keeper didn't
        // make a save). That's why `last_shot_xg` uses the on-target
        // adjustment — it's the value the keeper can earn or lose, not
        // the value attributed to the shooter.
        let base_xg = profile
            .expected_xg(horizontal_distance, true)
            .clamp(0.0, 0.82);
        let xg = base_xg;
        let prevented_xg = if on_target { base_xg } else { base_xg * 0.15 };
        if let Some(shooter) = field.get_player_mut(shoot_event_model.from_player_id) {
            shooter
                .memory
                .record_shot(shoot_event_model.tick, on_target);
            shooter.memory.record_shot_xg(shoot_event_model.tick, xg);
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
            let credits = Self::distribute_xg_credit(
                field.ball.recent_passers.iter().copied(),
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

        // Stash the in-flight shot's xG and shooter so the GK xG-prevented
        // hook (in save / catch / parry / goal handlers) can credit /
        // debit the keeper without re-deriving the value. Use the
        // target-adjusted value here — an off-target shot doesn't
        // require the keeper to save anything.
        field.ball.last_shot_xg = prevented_xg;
        field.ball.last_shot_shooter_id = Some(shoot_event_model.from_player_id);

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

        // Shorter flight protection for shots — allows defenders/GK to claim sooner
        field.ball.flags.in_flight_state = 40;

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
                let goal_line_y = field.ball.position.y + final_velocity.y * ticks_to_goal;
                // Arc approximation: z under gravity (~0.157 u/tick² from
                // update_velocity's 9.81 * 0.016 scaling).
                let goal_line_z = (field.ball.position.z + final_velocity.z * ticks_to_goal
                    - 0.5 * 0.157 * ticks_to_goal * ticks_to_goal)
                    .max(0.0);
                field.ball.cached_shot_target = Some(ShotTarget {
                    goal_line_y,
                    goal_line_z,
                    defending_side,
                });
            } else {
                field.ball.cached_shot_target = None;
            }
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
        let shot_xg = field.ball.last_shot_xg;
        if let Some(gk) = field.get_player_mut(player_id) {
            gk.statistics.saves += 1;
            gk.statistics.shots_faced += 1;
            if shot_xg > 0.0 {
                gk.statistics.record_xg_prevented(shot_xg);
            }
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

    fn handle_caught_ball_event(player_id: u32, field: &mut MatchField) {
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
                let shot_xg = field.ball.last_shot_xg;
                if let Some(player) = field.get_player_mut(player_id) {
                    player.statistics.saves += 1;
                    player.statistics.shots_faced += 1;
                    if shot_xg > 0.0 {
                        player.statistics.record_xg_prevented(shot_xg);
                    }
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
            } else {
                // Non-shot catch from an opponent ball (cross claim,
                // through-ball collected, aerial gathered) — the GK
                // commanded the box without a save. Counts as a small
                // command-zone credit, NOT a save.
                if let Some(gk) = field.get_player_mut(player_id) {
                    if gk.tactical_position.current_position.position_group()
                        == PlayerFieldPositionGroup::Goalkeeper
                    {
                        gk.statistics.note_gk_command_action();
                    }
                }
            }
        }

        field.ball.previous_owner = field.ball.current_owner;
        field.ball.current_owner = Some(player_id);
        // Ball must stop when caught — prevent it from continuing into the goal
        field.ball.velocity = Vector3::zeros();
        field.ball.flags.in_flight_state = 0;
        // GK holds ball in hands — long protection so opponents can't challenge.
        // Covers holding (25-60 ticks) + distribution time.
        field.ball.claim_cooldown = 200;
        field.ball.pass_target_player_id = None;
        // Shot is dead — clear the projected intercept so the keeper
        // doesn't keep chasing a ghost target next tick.
        field.ball.cached_shot_target = None;
    }

    fn handle_move_player_event(player_id: u32, position: Vector3<f32>, field: &mut MatchField) {
        let player = field.get_player_mut(player_id).unwrap();
        player.position = position;
    }

    fn handle_take_ball_event(player_id: u32, field: &mut MatchField) {
        let player = field.get_player_mut(player_id).unwrap();
        player.run_for_ball();
    }

    fn handle_request_ball_receive(player_id: u32, field: &mut MatchField) {
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
        // Free-kick protection: the victim gets time without being challenged.
        if field.ball.current_owner.is_some() {
            field.ball.claim_cooldown = 150; // ~2.5 seconds of protection
            field.ball.flags.in_flight_state = 150;
            field.ball.contested_claim_count = 0;
        }

        // Award the restart to the victim's team. Penalty if foul occurred
        // inside the fouler's penalty area, otherwise direct free kick at
        // ball position. Runs whether or not a card is given — most fouls
        // produce a free kick without a booking.
        Self::award_restart_for_foul(fouler_id, severity, field, context);

        let match_second = context.total_match_time;

        // Card decision — probability scales with severity and the fouler's
        // aggression/dirtiness. Composure reduces the chance a little
        // (cool-headed players get the benefit of the doubt).
        let (card_yellow_prob, card_red_prob) = {
            let player = match field.get_player_mut(fouler_id) {
                Some(p) => p,
                None => return,
            };

            // Count the foul itself whether or not it draws a card.
            player.fouls_committed = player.fouls_committed.saturating_add(1);
            player.statistics.add_foul(match_second);

            let aggression = player.skills.mental.aggression / 20.0;
            let composure = player.skills.mental.composure / 20.0;
            let teamwork = player.skills.mental.teamwork / 20.0;
            // Personality contributions — these used to be generated-only.
            // Dirtiness = how hard/cynical the challenges are; temperament
            // = how likely the player is to snap under provocation;
            // sportsmanship is a damper pulling the other way.
            let dirtiness = player.attributes.dirtiness / 20.0;
            let temperament = player.attributes.temperament / 20.0;
            let sportsmanship = player.attributes.sportsmanship / 20.0;

            // Persistent fouler escalation — 3+ fouls = next one much more likely booked.
            let persistent = if player.fouls_committed >= 3 {
                0.15
            } else {
                0.0
            };

            // High-aggression, low-composure, low-teamwork = "dirty" player.
            // Layer personality on top: dirtiness pushes cards up, sportsmanship
            // pulls them down, low temperament punishes you under pressure.
            let aggressor_factor = (aggression * 0.40 - composure * 0.12 - teamwork * 0.08
                + dirtiness * 0.18
                + (1.0 - temperament) * 0.10
                - sportsmanship * 0.10)
                .clamp(-0.25, 0.70);

            // Card probabilities calibrated to spec ranges:
            //   normal foul yellow  4-16%
            //   reckless yellow    45-80%
            //   reckless red       2-12%
            //   violent red       70-100%
            match severity {
                FoulSeverity::Normal => (
                    (0.04 + aggressor_factor * 0.18 + persistent * 0.6).clamp(0.02, 0.18),
                    0.0_f32,
                ),
                FoulSeverity::Reckless => (
                    (0.45 + aggressor_factor * 0.30 + persistent * 0.8).clamp(0.30, 0.85),
                    (0.04 + aggressor_factor * 0.12 + persistent).clamp(0.01, 0.18),
                ),
                FoulSeverity::Violent => {
                    (0.10_f32, (0.70 + aggressor_factor * 0.30).clamp(0.60, 1.0))
                }
            }
        };

        let mut rng = rand::rng();
        let roll_red = rng.random::<f32>();
        let roll_yellow = rng.random::<f32>();

        let direct_red = roll_red < card_red_prob;
        let got_yellow = !direct_red && roll_yellow < card_yellow_prob;

        if !direct_red && !got_yellow {
            return;
        }

        // Re-enabled red cards. The unified foul model + GK violent
        // recalibration brought foul frequency into the spec band, so
        // sending players off no longer cascades into half-empty teams.
        // Direct reds and second yellows both fully send the fouler off.
        let (second_yellow, ends_with_red) = {
            let player = match field.get_player_mut(fouler_id) {
                Some(p) => p,
                None => return,
            };
            if direct_red {
                player.statistics.add_red_card(match_second);
                player.is_sent_off = true;
                context.record_stoppage_time(45_000);
                (false, true)
            } else {
                player.yellow_cards = player.yellow_cards.saturating_add(1);
                player.statistics.add_yellow_card(match_second);
                context.record_stoppage_time(15_000);
                let promoted = player.yellow_cards >= 2;
                if promoted {
                    player.statistics.add_red_card(match_second);
                    player.is_sent_off = true;
                    context.record_stoppage_time(45_000);
                }
                (promoted, promoted)
            }
        };

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

        // Clearance cap. Needs more headroom than a pass because a
        // clearance is typically lofted — horizontal AND vertical
        // components are both meaningful, so the total magnitude
        // (sqrt(hx² + hy² + vz²)) legitimately exceeds a flat pass.
        // In-engine gravity is strong (balls fall fast), so a proper
        // hoof needs ~5 u/tick each of horizontal and vertical to
        // travel 30-40m before landing. 7.0 total covers that with a
        // little slack. Still well below the global MAX_VELOCITY safety.
        const MAX_CLEAR_VELOCITY: f32 = 7.0;
        let speed = velocity.norm();
        let mut capped_velocity = if speed > MAX_CLEAR_VELOCITY {
            velocity * (MAX_CLEAR_VELOCITY / speed)
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
                    // GK sweeper / routine clear-out is a command-zone
                    // action — counted alongside the clearance so the
                    // box-commanding GK gets the small per-event credit.
                    if zone.map_or(false, |z| z.is_own_box())
                        && clearer.tactical_position.current_position.position_group()
                            == PlayerFieldPositionGroup::Goalkeeper
                    {
                        clearer.statistics.note_gk_command_action();
                    }
                }
            }
        }

        // Apply the clearing velocity to the ball
        field.ball.velocity = capped_velocity;

        // Clear ownership - ball is now loose
        field.ball.previous_owner = field.ball.current_owner;
        field.ball.current_owner = None;
        field.ball.pass_target_player_id = None;
        field.ball.clear_pass_history();

        // Set in-flight state to prevent immediate reclaim after clearance
        field.ball.flags.in_flight_state = 40;
    }

    /// Award a free-kick or penalty restart to the victim's team after a
    /// foul. Penalty if the foul occurred inside the fouler's penalty
    /// area, otherwise a direct free kick at the ball's current position.
    /// Picks the taker dynamically by skill score (penalty: penalty_taking
    /// composite; FK: free_kicks composite). Idempotent on missing data:
    /// returns silently if fouler/victim team can't be resolved.
    fn award_restart_for_foul(
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
