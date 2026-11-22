//! **Getting rid of it** — when a player under pressure stops trying to
//! play and puts the ball away.
//!
//! Two places call for it, read the same way:
//!
//! * [`ClearanceCall`]: a defender in his own penalty area clears it.
//! * [`TouchlineCall`]: anyone pinned against his own touchline in his own
//!   half, with an opponent within a stride, puts it out up the line.
//!
//! # The readings
//!
//! Three continuous readings, and a fourth that is the man himself.
//!
//! * **The squeeze.** How close the nearest opponent is. A man six metres
//!   away is pressure; a man two metres away is a decision made for you.
//! * **The danger.** How near his own goal the ball is. The same pressure
//!   on the edge of the D and on the six-yard line are not the same
//!   situation.
//! * **The out.** Not "is there a pass" — *is there a safe one*. A
//!   team-mate with a man on him is not an out; playing him in inside your
//!   own area is how a clearance becomes a chance. At the touchline, room
//!   to carry up the line is an out too.
//! * **The man.** `buildup_profile` is the composite for playing out from
//!   the back. A ball-playing defender rides pressure that makes a limited
//!   one get rid of it.

use crate::r#match::defenders::states::DefenderState;
use crate::r#match::events::Event;
use crate::r#match::midfielders::states::shared::LaneAhead;
use crate::r#match::player::events::{PassingEventContext, PlayerEvent};
use crate::r#match::player::strategies::shared::players::ops::defender_skill::DefenderSkillProfile;
use crate::r#match::player::strategies::shared::team::KeeperVoice;
use crate::r#match::{MatchContext, MatchPlayerLite, StateChangeResult, StateProcessingContext};
#[cfg(feature = "match-logs")]
use crate::mid_run_diag::TouchlineCallDiag;
use nalgebra::Vector3;

/// What a defender who has the ball in his own area does with it.
///
/// Returned rather than acted on, so the branch that asked the question
/// is the branch that answers it. Nine separate ladders used to reach
/// `Clearing` and only one of them consulted [`ClearanceCall`]; the rest
/// sat ABOVE it and decided first.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DefensiveRelease {
    Clear,
    /// Play the out — this man.
    Outlet(u32),
    /// Over the touchline, up the line — see [`TouchlineCall`].
    PutOut,
}

impl DefensiveRelease {
    /// Turn the decision into the state change it implies.
    ///
    /// On the enum rather than on each state: `Running` and `Passing`
    /// both act on it, and a hand copy in the second silently dropped
    /// the clearance from the diagnostic that counts them.
    pub fn resolve(self, ctx: &StateProcessingContext) -> StateChangeResult {
        match self {
            Self::Clear => {
                #[cfg(feature = "match-logs")]
                crate::r#match::player::strategies::players::ops::forward_shot_decision::mid_run_diag::ClearDiag::note(0);
                StateChangeResult::with_defender_state(DefenderState::Clearing)
            }
            Self::Outlet(target) => StateChangeResult::with_defender_state_and_event(
                DefenderState::Standing,
                Event::PlayerEvent(PlayerEvent::PassTo(
                    PassingEventContext::builder()
                        .with_from_player_id(ctx.player.id)
                        .with_to_player_id(target)
                        .with_reason("DEF_CLEARANCE_OUTLET")
                        .build(ctx),
                )),
            ),
            Self::PutOut => StateChangeResult::with_defender_state_and_event(
                DefenderState::Standing,
                TouchlineCall::put_out(ctx),
            ),
        }
    }
}

pub struct ClearanceCall;

impl ClearanceCall {
    /// An opponent this close is full pressure. 24u = 3 m — inside this
    /// he can reach the ball as you touch it.
    const SQUEEZE_NEAR: f32 = 24.0;
    /// …and beyond this he is not pressure at all. 72u = 9 m.
    const SQUEEZE_FAR: f32 = 72.0;

    /// Ball within this of your own goal is the danger zone. 132u =
    /// 16.5 m, the depth of the penalty area.
    const DANGER_DEPTH: f32 = 132.0;

    /// A team-mate with an opponent inside this is not an outlet. 56u =
    /// 7 m — near enough that the opponent reads the pass and arrives
    /// with it.
    const OUTLET_MARKED: f32 = 56.0;

    /// How far the pass search looks. Unchanged from the value the
    /// running state used, so the two calls see the same team-mates.
    const OUTLET_RANGE: f32 = 220.0;

    /// Where an ordinary defender's tolerance sits, and how much of it
    /// the `buildup_profile` is worth either way.
    ///
    /// At the centre a defender clears when the situation reads about
    /// 0.55 — squeezed and inside the area. The spread is deliberately
    /// wide: this is the one decision where a Rio Ferdinand and a
    /// journeyman visibly do different things with the same ball.
    const TOLERANCE_BASE: f32 = 0.40;
    const TOLERANCE_SKILL: f32 = 0.42;

    /// **The urgent on-ball choice**, or `None` when this is not one of
    /// those moments and the caller's own ladder applies.
    ///
    /// The whole point of returning it is that the caller cannot then
    /// reach a branch that answers the same question differently: a
    /// counter-attack outlet, a coach-tempo hold, an emergency pass and
    /// two separate `must_clear_under_pressure` ladders all used to run
    /// FIRST, so the model that knows about the squeeze, the danger and
    /// whether the out is genuinely an out was the last thing consulted
    /// and usually never reached at all.
    pub fn decide(ctx: &StateProcessingContext) -> Option<DefensiveRelease> {
        if !ctx.ball().in_own_penalty_area() {
            return None;
        }
        let squeeze = Self::squeeze(ctx);
        if squeeze <= 0.0 {
            // Nobody near him. Playing out is the whole point of having
            // defenders who can, and the ordinary build-up ladder is the
            // right one.
            return None;
        }
        // One squeeze reading and ONE outlet search. `find_best_pass_option`
        // draws from the match RNG, so asking twice is asking a different
        // question the second time — `decide` could answer `Clear` on a
        // tick where `now` had just found him a safe out.
        let outlet = Self::outlet(ctx, squeeze);
        if Self::clears(ctx, squeeze, outlet.is_some()) {
            return Some(DefensiveRelease::Clear);
        }
        Some(match outlet {
            Some(mate) => DefensiveRelease::Outlet(mate.id),
            None => DefensiveRelease::Clear,
        })
    }

    /// Should this defender clear it?
    ///
    /// Only ever consulted with the ball in his own penalty area — the
    /// caller owns that test, because outside the area the same reading
    /// produces a pass under pressure rather than a hoof.
    pub fn now(ctx: &StateProcessingContext) -> bool {
        let squeeze = Self::squeeze(ctx);
        Self::clears(ctx, squeeze, Self::outlet(ctx, squeeze).is_some())
    }

    /// The call itself, on readings the caller has already taken.
    fn clears(ctx: &StateProcessingContext, squeeze: f32, has_outlet: bool) -> bool {
        if squeeze <= 0.0 {
            // Nobody near him. Playing out is the whole point of having
            // defenders who can.
            return false;
        }

        // No out at all, with anybody near him: it goes.
        if !has_outlet {
            return true;
        }

        let danger = Self::danger(ctx);
        // The squeeze is what makes the decision; the danger is what
        // makes it urgent. Multiplying rather than adding keeps a
        // completely unpressured defender on the ball however deep he is,
        // which is right — a centre-half with time on his own goal line
        // plays, he does not hoof.
        let situation = squeeze * (0.55 + 0.45 * danger);

        let profile = DefenderSkillProfile::from_ctx(ctx);
        // **"AWAY!"**
        //
        // The third of the keeper's calls — see [`KeeperVoice`]. He is
        // the man who can see whether there is anything on, and "get rid
        // of it" is the single most audible thing said in a penalty area.
        // Subtracted from the tolerance, so a commanding keeper's defender
        // clears sooner and a quiet keeper's tries to play; worth about
        // an eighth of the skill axis at the extremes, and exactly
        // nothing for a median keeper.
        let tolerance = Self::TOLERANCE_BASE
            + profile.buildup_profile.clamp(0.0, 1.0) * Self::TOLERANCE_SKILL
            - KeeperVoice::away_shout(ctx);
        situation > tolerance
    }

    /// How squeezed he is, 0..1, from the nearest opponent.
    fn squeeze(ctx: &StateProcessingContext) -> f32 {
        let nearest = ctx
            .players()
            .opponents()
            .all()
            .map(|o| (o.position - ctx.player.position).magnitude())
            .fold(f32::MAX, f32::min);
        if !nearest.is_finite() {
            return 0.0;
        }
        Self::squeeze_at(nearest)
    }

    /// The squeeze of an opponent `nearest` away.
    fn squeeze_at(nearest: f32) -> f32 {
        ((Self::SQUEEZE_FAR - nearest) / (Self::SQUEEZE_FAR - Self::SQUEEZE_NEAR)).clamp(0.0, 1.0)
    }

    /// How near our own goal the ball is, 0..1.
    fn danger(ctx: &StateProcessingContext) -> f32 {
        let own_goal = ctx.ball().direction_to_own_goal();
        let depth = (ctx.tick_context.positions.ball.position - own_goal).magnitude();
        (1.0 - depth / Self::DANGER_DEPTH).clamp(0.0, 1.0)
    }

    /// The pass that is actually an alternative, if there is one.
    ///
    /// `find_best_pass_option_with_distance` is the evaluator every line
    /// on the pitch uses, and it is the right search — but "the best
    /// available pass" and "a safe pass" are different questions, and
    /// inside your own area only the second one counts.
    fn outlet(ctx: &StateProcessingContext, squeeze: f32) -> Option<MatchPlayerLite> {
        let (mate, _) = ctx
            .player()
            .passing()
            .find_best_pass_option_with_distance(Self::OUTLET_RANGE)?;

        // A team-mate with a man on him is not an out.
        let marked = ctx
            .players()
            .opponents()
            .all()
            .any(|o| (o.position - mate.position).magnitude() < Self::OUTLET_MARKED);
        if marked {
            return None;
        }

        // …and neither is a ball played BACKWARDS across your own area
        // while somebody is closing you down. Under real pressure the
        // only safe direction is away from goal.
        if squeeze > 0.5 {
            let own_goal = ctx.ball().direction_to_own_goal();
            let mine = (ctx.player.position - own_goal).magnitude();
            let theirs = (mate.position - own_goal).magnitude();
            if theirs < mine {
                return None;
            }
        }

        Some(mate)
    }
}

/// **Putting it out** — a man pinned against his own touchline in his own
/// half, with no safe out, plays the ball out of play up the line rather
/// than into the press or back across his goal. The four readings are the
/// clearance call's — the squeeze, the danger, the out and the man — and
/// the rule is the same for every role: a wide midfielder who has tracked
/// back is pinned exactly as a full-back is.
pub struct TouchlineCall;

impl TouchlineCall {
    /// Within this of his own touchline he is pinned against it (4 m)…
    const PINNED: f32 = 32.0;
    /// …by an opponent within a stride (2 m).
    const STRIDE: f32 = 16.0;
    /// Running room up the line that is an out on its own: the lane ahead
    /// at least this open (9 m clear).
    const ROOM: f32 = 0.5;
    /// The ball over the line: up it and out, ~20 m/s, low.
    const PUT_OUT_PACE: f32 = 1.6;
    const PUT_OUT_LIFT: f32 = 0.03;

    /// `Some(PutOut)` when he should put it out; `None` when this is not
    /// one of those moments and his own ladder applies.
    pub fn decide(ctx: &StateProcessingContext) -> Option<DefensiveRelease> {
        if MatchContext::cross_touchline_off() || !ctx.player.has_ball(ctx) {
            return None;
        }
        let side = ctx.player.side?;
        let field_width = ctx.context.field_size.width as f32;
        let field_height = ctx.context.field_size.height as f32;
        let me = ctx.player.position;
        let progress = side.attacking_progress_x(me.x, field_width);
        if progress >= 0.5 || me.y.min(field_height - me.y) > Self::PINNED {
            return None;
        }
        #[cfg(feature = "match-logs")]
        TouchlineCallDiag::note(TouchlineCallDiag::PINNED);
        let nearest = ctx
            .players()
            .opponents()
            .all()
            .map(|o| (o.position - me).magnitude())
            .fold(f32::MAX, f32::min);
        if nearest > Self::STRIDE {
            return None;
        }
        let squeeze = ClearanceCall::squeeze_at(nearest);
        let danger = ((0.5 - progress) / 0.5).clamp(0.0, 1.0);
        let buildup = DefenderSkillProfile::from_ctx(ctx).buildup_profile;
        let has_out = Self::has_out(ctx);
        let puts_out = Self::puts_out(squeeze, danger, has_out, buildup);
        #[cfg(feature = "match-logs")]
        {
            TouchlineCallDiag::note(TouchlineCallDiag::PRESSED);
            if Self::puts_out(squeeze, danger, false, buildup) {
                TouchlineCallDiag::note(TouchlineCallDiag::PAST_TOLERANCE);
            }
            if has_out {
                TouchlineCallDiag::note(TouchlineCallDiag::WITH_AN_OUT);
            }
            if puts_out {
                TouchlineCallDiag::note(TouchlineCallDiag::PUT_OUT);
            }
        }
        puts_out.then_some(DefensiveRelease::PutOut)
    }

    /// The call on its readings: with an out he plays; without one, he puts
    /// it out once the squeeze — made urgent by the danger — passes what
    /// his ability to play out under pressure tolerates. The squeeze and
    /// the tolerance are [`ClearanceCall`]'s own.
    pub fn puts_out(squeeze: f32, danger: f32, has_out: bool, buildup: f32) -> bool {
        if has_out {
            return false;
        }
        let situation = squeeze * (0.55 + 0.45 * danger);
        situation
            > ClearanceCall::TOLERANCE_BASE
                + buildup.clamp(0.0, 1.0) * ClearanceCall::TOLERANCE_SKILL
    }

    /// A safe pass by the lane price every pass decision reads, or room to
    /// carry it on up the line.
    fn has_out(ctx: &StateProcessingContext) -> bool {
        if ctx.player().passing().find_safe_pass_option().is_some() {
            return true;
        }
        let up_the_line =
            Vector3::new(ctx.player.side.map_or(1.0, |s| s.forward_dir_x()), 0.0, 0.0);
        LaneAhead::read_along(ctx, up_the_line).openness >= Self::ROOM
    }

    /// The ball over his own touchline, up the line, so the throw is taken
    /// further from his goal.
    pub fn put_out(ctx: &StateProcessingContext) -> Event {
        let field_height = ctx.context.field_size.height as f32;
        let forward = ctx.player.side.map_or(1.0, |s| s.forward_dir_x());
        let outward = if ctx.player.position.y < field_height * 0.5 {
            -1.0
        } else {
            1.0
        };
        let direction = Vector3::new(forward * 0.8, outward * 0.6, 0.0);
        Event::PlayerEvent(PlayerEvent::PutOut(
            ctx.player.id,
            Vector3::new(
                direction.x * Self::PUT_OUT_PACE,
                direction.y * Self::PUT_OUT_PACE,
                Self::PUT_OUT_LIFT,
            ),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::TouchlineCall;

    /// Closer pressure puts the ball out sooner.
    #[test]
    fn closer_pressure_puts_it_out_sooner() {
        assert!(TouchlineCall::puts_out(1.0, 0.5, false, 0.4));
        assert!(!TouchlineCall::puts_out(0.6, 0.5, false, 0.4));
    }

    /// The same pressure nearer his own goal puts it out sooner.
    #[test]
    fn a_spot_nearer_his_own_goal_puts_it_out_sooner() {
        assert!(TouchlineCall::puts_out(0.8, 1.0, false, 0.4));
        assert!(!TouchlineCall::puts_out(0.8, 0.0, false, 0.4));
    }

    /// A composed player who plays out from the back rides pressure that
    /// makes a limited one put it out.
    #[test]
    fn a_composed_player_rides_the_pressure_longer() {
        assert!(TouchlineCall::puts_out(0.9, 0.3, false, 0.2));
        assert!(!TouchlineCall::puts_out(0.9, 0.3, false, 0.8));
    }

    /// With an out he never puts it out, however hard he is pressed.
    #[test]
    fn with_an_out_he_plays() {
        assert!(!TouchlineCall::puts_out(1.0, 1.0, true, 0.0));
    }
}
