//! **Getting rid of it** — when a defender in his own penalty area stops
//! trying to play and puts the ball out.
//!
//! # The problem this exists to solve
//!
//! `DefenderRunningState::should_clear` asked two questions before
//! hoofing it: is there an opponent within `14u` — one and three quarter
//! metres — and does `find_best_pass_option_with_distance(220.0)` come up
//! empty. The first is a man practically standing on you, the second is
//! almost never true, so between them a defender in his own six-yard box
//! with two attackers around him went looking for a twenty-metre pass.
//!
//! Measured over twelve fixtures at level 14: **0.61 clearances per
//! defender per match against a real ~3.5**, and 59 clearances in the
//! whole sample, of which 39% came from the emergency-pass branch failing
//! rather than from anybody deciding to clear.
//!
//! The history matters, because the number has been wrong in both
//! directions. An earlier test — `in_own_penalty_area() && opponents
//! within 30u` — produced **15.2 clearances per defender per match**,
//! every centre-half hoofing every ball he received in his own box. The
//! answer is not a looser threshold, it is a model with the two things
//! the thresholds were standing in for: how squeezed he actually is, and
//! whether the alternative is genuinely an alternative.
//!
//! # The football
//!
//! Three continuous readings, and a fourth that is the man himself.
//!
//! * **The squeeze.** How close the nearest opponent is. A man six
//!   metres away is pressure; a man two metres away is a decision made
//!   for you.
//! * **The danger.** How near your own goal the ball is. The same
//!   pressure on the edge of the D and on the six-yard line are not the
//!   same situation, and every professional treats them differently.
//! * **The out.** Not "is there a pass" — *is there a safe one*. A
//!   team-mate with a man on him is not an out; playing him in inside
//!   your own area is how a clearance becomes a chance. Nor is a pass
//!   that goes closer to your own goal while you are being closed down.
//! * **The man.** `buildup_profile` is the composite for playing out
//!   from the back. A ball-playing centre-half rides pressure that makes
//!   a limited one hoof it, and that difference is most of what the
//!   attribute means.

use crate::r#match::StateProcessingContext;
use crate::r#match::player::strategies::common::players::ops::defender_skill::DefenderSkillProfile;

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

    /// Should this defender clear it?
    ///
    /// Only ever consulted with the ball in his own penalty area — the
    /// caller owns that test, because outside the area the same reading
    /// produces a pass under pressure rather than a hoof.
    pub fn now(ctx: &StateProcessingContext) -> bool {
        let squeeze = Self::squeeze(ctx);
        if squeeze <= 0.0 {
            // Nobody near him. Playing out is the whole point of having
            // defenders who can.
            return false;
        }

        // No out at all, with anybody near him: it goes.
        if !Self::has_outlet(ctx, squeeze) {
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
        let tolerance =
            Self::TOLERANCE_BASE + profile.buildup_profile.clamp(0.0, 1.0) * Self::TOLERANCE_SKILL;
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
        ((Self::SQUEEZE_FAR - nearest) / (Self::SQUEEZE_FAR - Self::SQUEEZE_NEAR)).clamp(0.0, 1.0)
    }

    /// How near our own goal the ball is, 0..1.
    fn danger(ctx: &StateProcessingContext) -> f32 {
        let own_goal = ctx.ball().direction_to_own_goal();
        let depth = (ctx.tick_context.positions.ball.position - own_goal).magnitude();
        (1.0 - depth / Self::DANGER_DEPTH).clamp(0.0, 1.0)
    }

    /// Is there a pass that is actually an alternative?
    ///
    /// `find_best_pass_option_with_distance` is the evaluator every line
    /// on the pitch uses, and it is the right search — but "the best
    /// available pass" and "a safe pass" are different questions, and
    /// inside your own area only the second one counts.
    fn has_outlet(ctx: &StateProcessingContext, squeeze: f32) -> bool {
        let Some((mate, _)) = ctx
            .player()
            .passing()
            .find_best_pass_option_with_distance(Self::OUTLET_RANGE)
        else {
            return false;
        };

        // A team-mate with a man on him is not an out.
        let marked = ctx
            .players()
            .opponents()
            .all()
            .any(|o| (o.position - mate.position).magnitude() < Self::OUTLET_MARKED);
        if marked {
            return false;
        }

        // …and neither is a ball played BACKWARDS across your own area
        // while somebody is closing you down. Under real pressure the
        // only safe direction is away from goal.
        if squeeze > 0.5 {
            let own_goal = ctx.ball().direction_to_own_goal();
            let mine = (ctx.player.position - own_goal).magnitude();
            let theirs = (mate.position - own_goal).magnitude();
            if theirs < mine {
                return false;
            }
        }

        true
    }
}
