//! **One read of the game for every defender without the ball.**
//!
//! A defender is on one of four rungs: the shape, a job, the duel, the
//! recovery. Each state sits on one of them and asks this read for every
//! door above its own level; its own work at its level, and its give-ups,
//! stay in the state. So the same situation gets the same answer from
//! whatever state it finds him in, and no state hands him to an idle one
//! for the question to be asked again.

use crate::PlayerFieldPositionGroup;
use crate::r#match::common_states::{MarkEngagement, RecoveryChallenge, TackleEngagement};
use crate::r#match::defenders::states::DefenderState;
use crate::r#match::defenders::states::common::{DefensiveLine, Interception};
use crate::r#match::player::state::PlayerState;
use crate::r#match::player::strategies::RestartHold;
use crate::r#match::player::strategies::common::team::WideChannel;
use crate::r#match::{DefensiveDuty, MatchPlayerLite, StateChangeResult, StateProcessingContext};

/// The rungs of the read, highest door first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ReadLevel {
    /// The ball is his and at his feet; one he has to collect is the
    /// loose-ball rung's.
    Possession = 1,
    /// A high ball a contest has given him is on its way to him.
    Staged = 2,
    /// The loose-ball election names him, or a ball he can contest is
    /// coming to him.
    LooseBall = 3,
    /// He is the man for the carrier.
    Engage = 4,
    /// His duty is to cover the man engaging.
    Cover = 5,
    /// He has a man.
    Mark = 6,
    /// He is out of the line.
    Recover = 7,
    /// Nothing else: the shape, relaxed or alert, or squeezing up.
    Phase = 8,
}

impl ReadLevel {
    const ALL: [ReadLevel; 8] = [
        ReadLevel::Possession,
        ReadLevel::Staged,
        ReadLevel::LooseBall,
        ReadLevel::Engage,
        ReadLevel::Cover,
        ReadLevel::Mark,
        ReadLevel::Recover,
        ReadLevel::Phase,
    ];
}

pub struct DefenderRead;

impl DefenderRead {
    /// How far either side of his slot a man belongs to him when the plan
    /// gives him nobody (~6 m), the zone `HoldingLine` used; a man he is
    /// already marking stays his for this much further, as his distance
    /// does between `MarkEngagement::ENGAGE` and `RELEASE`.
    const ZONE_HALF_WIDTH: f32 = 50.0;
    const ZONE_HOLD: f32 = 25.0;
    /// How far past a containing defender the carrier has to be (0.5 m)
    /// before he is closing again.
    const GONE_PAST: f32 = 4.0;

    /// The phase distances of design D1, in units, and the margin a phase
    /// has to be crossed by before it changes back.
    const RELAXED_UPFIELD: f32 = 240.0;
    const ALERT_FROM_GOAL: f32 = 480.0;
    /// 35 m, about where an owned ball's 60 m from goal puts the line, so
    /// a pass in flight does not flip a relaxed defender to alert and back.
    const ALERT_LOOSE_FROM_LINE: f32 = 280.0;
    const ALERT_NEAR_LINE: f32 = 240.0;
    const HYSTERESIS: f32 = 40.0;
    /// Below this condition a defender stands down sooner, by this much of
    /// each alert distance per point of deficit.
    const TIRED_BELOW: f32 = 0.70;
    const TIRED_SHRINK: f32 = 2.0;

    /// The first open door above `level`.
    pub fn above(ctx: &StateProcessingContext, level: ReadLevel) -> Option<DefenderState> {
        ReadLevel::ALL
            .iter()
            .take_while(|l| **l < level)
            .find_map(|l| Self::door(ctx, *l))
    }

    /// [`Self::above`] as the state change a handler returns.
    pub fn change_above(
        ctx: &StateProcessingContext,
        level: ReadLevel,
    ) -> Option<StateChangeResult> {
        Self::above(ctx, level).map(StateChangeResult::with_defender_state)
    }

    /// The whole read: where a defender with nothing in hand belongs.
    pub fn resolve(ctx: &StateProcessingContext) -> DefenderState {
        Self::above(ctx, ReadLevel::Phase).unwrap_or_else(|| Self::phase(ctx))
    }

    /// [`Self::resolve`] as the state change a handler returns.
    pub fn change(ctx: &StateProcessingContext) -> StateChangeResult {
        StateChangeResult::with_defender_state(Self::resolve(ctx))
    }

    /// For a shape state, which the read answers whole: stay while the read
    /// still says `state`, go where it says otherwise.
    pub fn hold(ctx: &StateProcessingContext, state: DefenderState) -> Option<StateChangeResult> {
        let next = Self::resolve(ctx);
        (next != state).then(|| StateChangeResult::with_defender_state(next))
    }

    /// For a recovering defender: every door above the recovery; back in
    /// the line, the shape; otherwise the run the play asks for.
    pub fn recovering(
        ctx: &StateProcessingContext,
        state: DefenderState,
    ) -> Option<StateChangeResult> {
        if let Some(change) = Self::change_above(ctx, ReadLevel::Recover) {
            return Some(change);
        }
        let next = Self::recover(ctx).unwrap_or_else(|| Self::phase(ctx));
        (next != state).then(|| StateChangeResult::with_defender_state(next))
    }

    /// The read for a man who has just played the ball: he no longer has
    /// it, whatever the snapshot of this tick says.
    pub fn released(ctx: &StateProcessingContext) -> DefenderState {
        ReadLevel::ALL[1..]
            .iter()
            .find_map(|l| Self::door(ctx, *l))
            .unwrap_or_else(|| Self::phase(ctx))
    }

    fn door(ctx: &StateProcessingContext, level: ReadLevel) -> Option<DefenderState> {
        match level {
            ReadLevel::Possession => ctx.ball().mine_to_play().then_some(DefenderState::Running),
            ReadLevel::Staged => Self::staged(ctx),
            ReadLevel::LooseBall => Self::loose_ball(ctx),
            ReadLevel::Engage => Self::engage(ctx),
            ReadLevel::Cover => Self::cover(ctx),
            ReadLevel::Mark => Self::mark(ctx),
            ReadLevel::Recover => Self::recover(ctx),
            ReadLevel::Phase => Some(Self::phase(ctx)),
        }
    }

    /// A high ball a contest has given him, still on its way to him: up for
    /// it — at his side's corner, in the state that attacks it.
    fn staged(ctx: &StateProcessingContext) -> Option<DefenderState> {
        if !ctx.player.aerial_award || !ctx.tick_context.ball.awaits_header_from(ctx.player.id) {
            return None;
        }
        Some(if Self::at_his_sides_corner(ctx) {
            DefenderState::AttackingCorner
        } else {
            DefenderState::Heading
        })
    }

    /// The election's own question, asked as the dispatcher will ask it on
    /// the next tick — so a state never starts a chase it will refuse —
    /// and the contest test for a ball in flight that is coming to him.
    fn loose_ball(ctx: &StateProcessingContext) -> Option<DefenderState> {
        if PlayerFieldPositionGroup::should_force_takeball(
            PlayerFieldPositionGroup::Defender,
            ctx.player,
            ctx.context,
            ctx.tick_context,
        ) {
            return Some(DefenderState::TakeBall);
        }
        (Interception::is_available(ctx) && ctx.ball().is_towards_player_with_angle(0.8))
            .then_some(DefenderState::Intercepting)
    }

    /// He is the man for the carrier: the engagement licence, or the
    /// nearest defender to a carrier arriving at the edge of his area. The
    /// plan's presser goes to a ball travelling to an opponent as well.
    fn engage(ctx: &StateProcessingContext) -> Option<DefenderState> {
        let Some(carrier) = ctx.players().opponents().with_ball().next() else {
            return (ctx.team().my_duty() == DefensiveDuty::Press && !ctx.team().is_control_ball())
                .then_some(DefenderState::Pressing);
        };
        if !Self::is_man_for_carrier(ctx) {
            return None;
        }
        Some(Self::duel_rung(ctx, &carrier))
    }

    /// The licence to engage the carrier, with the step-up as its fifth way.
    pub fn is_man_for_carrier(ctx: &StateProcessingContext) -> bool {
        TackleEngagement::may_engage_carrier(ctx)
            || ctx.player().defensive().should_step_up_to_meet_attacker()
    }

    /// Closing or containing: inside `COMMIT` and goal-side of him he
    /// contains; otherwise he is still closing.
    pub fn duel_rung(ctx: &StateProcessingContext, carrier: &MatchPlayerLite) -> DefenderState {
        if carrier.distance(ctx) <= TackleEngagement::COMMIT && Self::is_goal_side(ctx, carrier) {
            DefenderState::Jockeying
        } else {
            DefenderState::Pressing
        }
    }

    /// In front of the carrier on his line to goal — the side of him the
    /// block tackle is made from. Level or behind, he has been gone past,
    /// and the stretch is his challenge (`RecoveryChallenge`). A man already
    /// containing is gone past only once the carrier is half a body by him,
    /// as he gives the carrier up at `DISENGAGE` rather than `COMMIT`.
    pub fn is_goal_side(ctx: &StateProcessingContext, carrier: &MatchPlayerLite) -> bool {
        let slack = if ctx.player.state == PlayerState::Defender(DefenderState::Jockeying) {
            Self::GONE_PAST
        } else {
            0.0
        };
        RecoveryChallenge::lead(ctx, carrier.position) > -slack
    }

    fn cover(ctx: &StateProcessingContext) -> Option<DefenderState> {
        Self::has_cover_duty(ctx).then_some(DefenderState::Covering)
    }

    pub fn has_cover_duty(ctx: &StateProcessingContext) -> bool {
        !ctx.team().is_control_ball() && ctx.team().my_duty() == DefensiveDuty::Cover
    }

    fn mark(ctx: &StateProcessingContext) -> Option<DefenderState> {
        Self::man(ctx)
            .filter(|man| man.distance(ctx) < MarkEngagement::ENGAGE)
            .map(|_| DefenderState::Marking)
    }

    /// His man: the one the plan gave him or, with none, the most
    /// dangerous opponent in his zone that the plan has given to nobody.
    /// Nobody while his own side has the ball.
    pub fn man(ctx: &StateProcessingContext) -> Option<MatchPlayerLite> {
        if ctx.team().is_control_ball() {
            return None;
        }
        if let Some(man) = ctx.team().my_mark() {
            return Some(man);
        }
        let team = ctx.team();
        let plan = team.defensive_plan();
        let slot_y = DefensiveLine::compact_slot_y(ctx);
        let zone = if ctx.player.state == PlayerState::Defender(DefenderState::Marking) {
            Self::ZONE_HALF_WIDTH + Self::ZONE_HOLD
        } else {
            Self::ZONE_HALF_WIDTH
        };
        let own_goal = ctx.ball().direction_to_own_goal();
        let players = ctx.players();
        players
            .opponents()
            .nearby(MarkEngagement::RELEASE)
            .filter(|o| !o.has_ball(ctx) && !o.tactical_positions.is_goalkeeper())
            .filter(|o| !plan.is_marked(o.id))
            .filter(|o| (o.position.y - slot_y).abs() <= zone)
            .min_by(|a, b| {
                let threat = |o: &MatchPlayerLite| {
                    (o.position - own_goal).norm() + (o.position - ctx.player.position).norm()
                };
                threat(a).total_cmp(&threat(b))
            })
    }

    fn recover(ctx: &StateProcessingContext) -> Option<DefenderState> {
        if DefensiveLine::is_in_line(ctx) {
            return None;
        }
        let dead = RestartHold::taker(ctx.tick_context).is_some();
        Some(if ctx.team().is_control_ball() || dead {
            DefenderState::Returning
        } else {
            DefenderState::TrackingBack
        })
    }

    /// The shape: in possession, the named overlap, the corner or squeezing
    /// up, relaxed once the ball has gone well upfield; out of it, alert
    /// while the ball is in reach of the line and relaxed when it is not.
    pub fn phase(ctx: &StateProcessingContext) -> DefenderState {
        if ctx.team().is_control_ball() {
            if WideChannel::still_mine(ctx) && ctx.team().is_overlap_runner() {
                return DefenderState::Overlapping;
            }
            if Self::at_his_sides_corner(ctx) {
                return DefenderState::AttackingCorner;
            }
            let upfield = Self::ball_upfield_of_line(ctx);
            let relaxed = Self::RELAXED_UPFIELD + Self::margin(ctx, DefenderState::Walking);
            return if upfield > relaxed {
                DefenderState::Walking
            } else {
                DefenderState::PushingUp
            };
        }
        if Self::is_alert(ctx) {
            DefenderState::HoldingLine
        } else {
            DefenderState::Walking
        }
    }

    fn is_alert(ctx: &StateProcessingContext) -> bool {
        // Relaxed already, he needs the play to come inside the distance;
        // alert, he stays until it has gone the margin beyond it.
        let reach = Self::tired_reach(ctx);
        let extra = Self::margin(ctx, DefenderState::HoldingLine);
        let to_line = Self::ball_upfield_of_line(ctx).abs();
        if to_line < (Self::ALERT_NEAR_LINE + extra) * reach {
            return true;
        }
        if ctx.ball().is_owned() {
            ctx.ball().distance_to_own_goal() < (Self::ALERT_FROM_GOAL + extra) * reach
        } else {
            to_line < (Self::ALERT_LOOSE_FROM_LINE + extra) * reach
        }
    }

    /// The hysteresis: a phase he is in is held by the margin, and an alert
    /// man whose side wins the ball needs it the margin further upfield
    /// before he relaxes — a ball won and lost again is not settled
    /// possession.
    fn margin(ctx: &StateProcessingContext, phase: DefenderState) -> f32 {
        let PlayerState::Defender(current) = ctx.player.state else {
            return 0.0;
        };
        match (phase, current) {
            (DefenderState::HoldingLine, DefenderState::HoldingLine)
            | (DefenderState::Walking, DefenderState::HoldingLine) => Self::HYSTERESIS,
            (DefenderState::Walking, DefenderState::Walking) => -Self::HYSTERESIS,
            _ => 0.0,
        }
    }

    /// The share of each alert distance a tired defender still reads.
    fn tired_reach(ctx: &StateProcessingContext) -> f32 {
        let condition = ctx.player.player_attributes.condition_percentage() as f32 / 100.0;
        (1.0 - (Self::TIRED_BELOW - condition).max(0.0) * Self::TIRED_SHRINK).clamp(0.4, 1.0)
    }

    /// Signed: positive with the ball upfield of the back line.
    fn ball_upfield_of_line(ctx: &StateProcessingContext) -> f32 {
        let forward = ctx.player.side.map_or(1.0, |s| s.forward_dir_x());
        (ctx.tick_context.positions.ball.position.x - DefensiveLine::position_x(ctx)) * forward
    }

    fn at_his_sides_corner(ctx: &StateProcessingContext) -> bool {
        ctx.player
            .tactical_position
            .current_position
            .is_central_defender()
            && ctx.ball().is_team_attacking_corner()
    }
}
