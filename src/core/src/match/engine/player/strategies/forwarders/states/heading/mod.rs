use crate::r#match::events::Event;
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::forwarders::states::common::{ActivityIntensity, ForwardCondition};
use crate::r#match::player::events::{PlayerEvent, ShootingEventContext};
use crate::r#match::player::strategies::common::passing::CrossModel;
#[cfg(feature = "match-logs")]
use crate::r#match::player::strategies::common::players::ops::forward_shot_decision::mid_run_diag::CROSS_HEADER_ON_GOAL;
use crate::r#match::player::strategies::players::ShotType;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use nalgebra::Vector3;
use std::cmp::Ordering;
#[cfg(feature = "match-logs")]
use std::sync::atomic::Ordering as AtomicOrdering;

const HEADING_HEIGHT_THRESHOLD: f32 = 1.5;
const HEADING_DISTANCE_THRESHOLD: f32 = 4.0;

#[derive(Default, Clone)]
pub struct ForwardHeadingState {}

impl StateProcessingHandler for ForwardHeadingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        let ball_position = ctx.tick_context.positions.ball.position;

        // ── A WON CONTEST ALWAYS PRODUCES A CONTACT ──────────────────
        // When the engine-level aerial contest awarded this player the
        // ball and flew the delivery to his head, he touches it: a shot
        // when he is in headed-shot range and the cooldowns allow, a
        // glance / knock-down otherwise. This block sits ABOVE the
        // bail-out gates below on purpose — they were the 80% leak the
        // contest funnel measured (wins 2.5/match, headers struck 0.5):
        // contests resolve 12-16 m out, the range gate is 12 m, and the
        // team's 7.5 s shot window is usually warm in a crossing attack,
        // so most won headers hit a `return Running` and simply
        // vanished — leaving the granted ball hanging with nobody
        // allowed to claim it. A real winner who cannot shoot still
        // HEADS the ball somewhere; `glanced_contact` is that contact,
        // and it clears the grant through `record_touch`.
        let contest_awarded = ctx.tick_context.ball.aerial_contest_winner == Some(ctx.player.id);
        // …and a ball awarded to somebody ELSE is not ours to challenge
        // (double jeopardy — same rule as the defender/midfielder states).
        if !contest_awarded && ctx.tick_context.ball.aerial_contest_winner.is_some() {
            return Some(StateChangeResult::with_forward_state(ForwardState::Running));
        }
        if contest_awarded {
            if ball_position.z < HEADING_HEIGHT_THRESHOLD {
                // Dropped under the band un-struck — the grant lapses in
                // the ownership guard and this is an ordinary loose ball.
                return Some(StateChangeResult::with_forward_state(ForwardState::Running));
            }
            if ctx.ball().distance() > HEADING_DISTANCE_THRESHOLD {
                // Still arriving — keep attacking the drop point.
                return None;
            }
            // An AWARDED header is exempt from the TEAM shot window
            // (the player's own 2 s stays). That window exists to stop
            // rapid-fire recycling in open play — but corners FOLLOW
            // shots (a blocked shot is how you win one), so the window
            // was warm for essentially every set-piece header and
            // "set-piece header" sat at 0.04/match through two rounds
            // of contest fixes. Range 120u (15 m) rather than the foot
            // paths' 96u: the contest resolves where the delivery
            // descends through heading height, 12-16 m out, and a
            // glanced header from there is a real if low-value attempt
            // — the shot pipeline prices it.
            let may_shoot =
                ctx.player().can_shoot() && ctx.ball().distance_to_opponent_goal() <= 120.0;
            let heading = ctx.player.skills.technical.heading / 20.0;
            let jumping = ctx.player.skills.physical.jumping / 20.0;
            let on_corner_award = ctx.ball().is_team_attacking_corner();
            let (base, floor) = if on_corner_award {
                (0.62, 0.55)
            } else {
                (0.34, 0.28)
            };
            let p = (base + (heading + jumping) * 0.5 * 0.30).clamp(floor, 0.95);
            return if may_shoot && ctx.context.rng.unit_f32() < p {
                #[cfg(feature = "match-logs")]
                CROSS_HEADER_ON_GOAL.fetch_add(1, AtomicOrdering::Relaxed);
                Some(StateChangeResult::with_forward_state_and_event(
                    ForwardState::Running,
                    Event::PlayerEvent(PlayerEvent::Shoot(
                        ShootingEventContext::new()
                            .with_player_id(ctx.player.id)
                            .with_target(ctx.player().shooting_direction())
                            .with_reason("FWD_HEADING_ON_GOAL")
                            .with_shot_type(ShotType::Header)
                            .build(ctx),
                    )),
                ))
            } else {
                Some(StateChangeResult::with_forward_state_and_event(
                    ForwardState::Running,
                    Event::PlayerEvent(PlayerEvent::ClearBall(self.glanced_contact(ctx))),
                ))
            };
        }

        // Ball too far — transition back to running
        if ctx.ball().distance() > HEADING_DISTANCE_THRESHOLD {
            return Some(StateChangeResult::with_forward_state(ForwardState::Running));
        }

        // Ball too low to head — transition to running
        if ball_position.z < HEADING_HEIGHT_THRESHOLD {
            return Some(StateChangeResult::with_forward_state(ForwardState::Running));
        }

        // A header ON GOAL is a shot like any other: it must respect
        // the player + team shot cooldowns (this state used to ignore
        // both — headed attempts fired through the 2s/7.5s windows the
        // foot paths honour) and real headed-shot range (~12m; nobody
        // heads for goal from 25m). Outside these, a won aerial is a
        // knock-down, not an attempt.
        if !ctx.player().can_shoot()
            || !ctx.team().can_shoot()
            || ctx.ball().distance_to_opponent_goal() > 96.0
        {
            return Some(StateChangeResult::with_forward_state(ForwardState::Running));
        }

        // Corner carve-out (no engine-level award): the set-piece jump
        // at a ball aimed at your head — contact-only roll, the same
        // clean-contact reasoning as the awarded block above; the full
        // aerial duel below is for loose balls nothing upstream decided.
        let on_corner = ctx.ball().is_team_attacking_corner();
        if on_corner {
            let heading = ctx.player.skills.technical.heading / 20.0;
            let jumping = ctx.player.skills.physical.jumping / 20.0;
            // A corner is a set jump at a ball aimed at your head, so a
            // won contest nearly always produces an attempt (the awarded
            // block above owns the open-play flick-on/knock-down mix).
            let (base, floor) = (0.62, 0.55);
            let p = (base + (heading + jumping) * 0.5 * 0.30).clamp(floor, 0.95);
            return if ctx.context.rng.unit_f32() < p {
                Some(StateChangeResult::with_forward_state_and_event(
                    ForwardState::Running,
                    Event::PlayerEvent(PlayerEvent::Shoot(
                        ShootingEventContext::new()
                            .with_player_id(ctx.player.id)
                            .with_target(ctx.player().shooting_direction())
                            .with_reason("FWD_HEADING_ON_GOAL")
                            .with_shot_type(ShotType::Header)
                            .build(ctx),
                    )),
                ))
            } else {
                // Not a clean contact — but he still HEADED it. Leaving
                // the ball hanging at head height in the six-yard area
                // instead is not "no attempt", it is a free point-blank
                // scramble: whoever reacts first snapshots it from two
                // yards. That is a manufactured chance, and with the
                // open-play contest live it was worth about two extra
                // shots per team per match.
                //
                // A mistimed header goes SOMEWHERE — flicked on, nodded
                // down, glanced wide. Send it away from the six-yard box
                // so the next phase is a real second ball rather than a
                // tap-in queue.
                Some(StateChangeResult::with_forward_state_and_event(
                    ForwardState::Running,
                    Event::PlayerEvent(PlayerEvent::ClearBall(self.glanced_contact(ctx))),
                ))
            };
        }

        // Aerial duel against the closest defender first — losing the
        // duel means no header attempt at all. Goalkeepers handle their
        // own claim/punch in the GK state machine; we only resolve
        // outfield markers here.
        let attacker_full = ctx.context.players.by_id(ctx.player.id);
        let defender_full = ctx
            .players()
            .opponents()
            .all()
            .filter(|opp| {
                if let Some(full) = ctx.context.players.by_id(opp.id) {
                    !full.tactical_position.current_position.is_goalkeeper()
                } else {
                    true
                }
            })
            .min_by(|a, b| {
                let da = (a.position - ctx.player.position).magnitude();
                let db = (b.position - ctx.player.position).magnitude();
                da.partial_cmp(&db).unwrap_or(Ordering::Equal)
            })
            .and_then(|m| ctx.context.players.by_id(m.id));

        let minute = (ctx.context.total_match_time / 60_000) as u32;
        let won_duel = match attacker_full {
            Some(att) => CrossModel::resolve_aerial_duel(ctx, att, defender_full, minute),
            None => self.attempt_heading(ctx),
        };

        if !won_duel {
            return Some(StateChangeResult::with_forward_state(ForwardState::Running));
        }

        // Attempt the header — combine duel win with skill execution.
        if self.attempt_heading(ctx) {
            // Success — shoot toward opponent goal, marked as a Header
            // for downstream xG.
            Some(StateChangeResult::with_forward_state_and_event(
                ForwardState::Running,
                Event::PlayerEvent(PlayerEvent::Shoot(
                    ShootingEventContext::new()
                        .with_player_id(ctx.player.id)
                        .with_target(ctx.player().shooting_direction())
                        .with_reason("FWD_HEADING_ON_GOAL")
                        .with_shot_type(ShotType::Header)
                        .build(ctx),
                )),
            ))
        } else {
            // Failed header — transition to running
            Some(StateChangeResult::with_forward_state(ForwardState::Running))
        }
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let ball_position = ctx.tick_context.positions.ball.position;
        Some(
            SteeringBehavior::Arrive {
                target: ball_position,
                slowing_distance: 3.0,
            }
            .calculate(ctx.player)
            .velocity,
        )
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Heading is very high intensity - explosive jumping action
        ForwardCondition::new(ActivityIntensity::VeryHigh).process(ctx);
    }
}

impl ForwardHeadingState {
    /// Where a mistimed header goes. Not a clearance and not a shot — the
    /// glance, the flick-on, the ball headed across the face and away.
    ///
    /// Direction is sideways-and-on rather than back toward the crosser,
    /// which is what a contact you didn't quite get over actually does,
    /// and it takes the ball out of the six-yard box. A better header of
    /// the ball keeps more control over even his poor contacts, so the
    /// glance travels less far and stays more playable.
    fn glanced_contact(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let heading = (ctx.player.skills.technical.heading / 20.0).clamp(0.0, 1.0);
        let forward_x = ctx.player.side.map_or(1.0, |side| side.forward_dir_x());
        let field_height = ctx.context.field_size.height as f32;
        // Glance toward the nearer touchline — away from the goalmouth.
        let away_y = if ctx.player.position.y >= field_height / 2.0 {
            1.0
        } else {
            -1.0
        };
        // A clean striker of the ball glances it 8-10 m; a poor one skews
        // it further and higher.
        let power = 1.5 - heading * 0.4;
        let lift = 0.10 - heading * 0.03;
        Vector3::new(
            -forward_x * power * 0.35 + ctx.context.rng.jitter(0.0, 0.2),
            away_y * power,
            lift.max(0.03),
        )
    }

    /// Determines if the forward successfully heads the ball based on skills and random chance.
    fn attempt_heading(&self, ctx: &StateProcessingContext) -> bool {
        let heading_skill = ctx.player.skills.technical.heading / 20.0;
        let jumping_skill = ctx.player.skills.physical.jumping / 20.0;
        let overall_skill = (heading_skill + jumping_skill) / 2.0;

        // A won aerial becomes an attempt ON GOAL far less often than
        // half the time: real headed shots follow ~25-30% of won
        // attacking headers, the rest are knock-downs, flicks and
        // misdirected contacts. The bare skill roll here was a major
        // deterministic close-range shot source (emit% ~200% at 6-11m,
        // i.e. the non-helper paths matched the whole decision layer).
        let random_value: f32 = ctx.context.rng.unit_f32();
        random_value < overall_skill * 0.55
    }
}
