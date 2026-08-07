//! Outfield match rating.
//!
//! Same two-part construction as [`super::keeper`]: a signed
//! performance value, standardised against the position's own
//! population and pushed through the shared [`RatingShape`], plus the
//! defining moments that bypass the curve.
//!
//! # What standardising fixes
//!
//! The model this replaces added every component to a flat 6.0 base and
//! then bounded the total with a ladder of hard "evidence tier" soft
//! caps — `Passenger` +0.20, `Modest` +0.85, `Strong` +1.30,
//! `TwoGoals` +2.30 — picked by threshold tests on the stat line. Two
//! things followed from that, both measured on a 300-match run:
//!
//!   * **The distribution went bimodal.** Caps flatten everything they
//!     touch, so the ordinary middle piled up at its cap while goals
//!     jumped clean over the top: goalless forwards averaged 5.71 and
//!     one-goal forwards 7.56, a 1.85-point cliff on a single event,
//!     against a real-football gap of about 1.0.
//!   * **Tier boundaries became the real rating model.** Whether a
//!     winger cleared `key_passes >= 3` was worth more than everything
//!     the rest of the stat line said, because it moved the cap by
//!     0.45. Every calibration pass then had to re-tune the thresholds,
//!     the caps, and the coefficients underneath them together.
//!
//! Centring on the position mean does the job the caps were reaching
//! for, without the cliff: a routine shift lands near zero because
//! routine is what the average player does, and it takes a genuinely
//! outsized contribution to move off it. And because the divisor is the
//! population spread, "how much is a key pass worth" only has to be
//! right *relative to a goal* — the absolute scale cancels. That is why
//! the per-event weights below can stay as they were researched while
//! the level they produce stops mattering.
//!
//! Role expectation falls out for free. A forward's population mean
//! includes the goals a forward is expected to score, so a goalless
//! forward is below average by construction — no separate
//! `attacking_role_expectation` drag, no engagement penalty, no
//! goalless-forward context damping. Three hand-tuned lanes, all of
//! which existed to say "a striker who didn't score had a quiet game",
//! collapse into the definition of the mean.

use super::{PerformanceScale, RatingContext, RatingMath};
use crate::PlayerFieldPositionGroup;
use crate::r#match::engine::zones::ZoneCoeffs;

/// A goal, in performance units. Everything else in the model is
/// denominated against this — the routine components keep the relative
/// weights they were researched with, and standardising removes any
/// need for the absolute scale to be right.
const GOAL: f32 = 1.00;
/// An assist. Decisive, but not as decisive as putting it in.
const ASSIST: f32 = 0.55;
/// Finishing measured against the chances taken: `goals − xG`, the
/// attacking mirror of the keeper's goals-prevented. A striker who
/// burned two and a half expected goals without scoring has had a bad
/// afternoon by the only measure that matters, and the model has to say
/// so — crediting the chance volume and then discounting the misses
/// separately was how a 6-shot 2.5-xG blank came out *above* average.
///
/// Bounded asymmetrically: over-performance is capped at half the
/// under-performance allowance because scoring from nothing is mostly
/// luck (and the goals themselves are already paid at full value),
/// while missing everything is a performance in its own right.
const FINISHING: f32 = 0.42;
const FINISHING_MIN: f32 = -2.00;
const FINISHING_MAX: f32 = 0.60;

/// Expected goals per 90 for the role — the floor a player is measured
/// against when his own chances fall short of it.
///
/// Without the floor, `goals − xG` self-zeroes for anyone who never
/// shoots, which makes never troubling the keeper *better* than getting
/// into positions and missing. Real judgement runs the other way round:
/// a striker who had a sight of goal and put it wide had a game; one who
/// never got near it did not. With the floor, and with the chance-volume
/// credit `shooting()` already pays, the three cases order correctly —
/// half an expected goal missed reads roughly neutral, no chances at all
/// reads poor, and two and a half burned reads clearly worse than both.
const ROLE_XG_FORWARD: f32 = 0.35;
const ROLE_XG_MIDFIELDER: f32 = 0.12;
const ROLE_XG_DEFENDER: f32 = 0.05;

/// Team result. Small: eleven players share it.
const WIN: f32 = 0.14;
const LOSS: f32 = -0.13;

/// Mistakes short of a goal.
const ERROR_TO_SHOT: f32 = -0.40;

/// Defining moments, in **rating points**, applied after the shape so a
/// single catastrophe still reaches the disaster band.
const ERROR_TO_GOAL: f32 = -1.35;
const OWN_GOAL: f32 = -1.30;
const RED_CARD: f32 = -1.50;
const YELLOW_CARD: f32 = -0.14;

impl<'a> RatingContext<'a> {
    /// Full outfield rating, pre-clamp.
    pub(super) fn outfield_rating(&self) -> f32 {
        PerformanceScale::for_position(self.pos).rate(self.outfield_performance())
            + self.outfield_defining_moments()
    }

    /// Signed performance value for an outfield shift. Zero is not
    /// meaningful on its own — only its position relative to
    /// [`PerformanceScale`] is.
    pub(super) fn outfield_performance(&self) -> f32 {
        let p = self.profile;
        let conf = self.confidence;

        // Routine on-the-ball work. Damped by minute confidence: a
        // fifteen-minute cameo cannot accumulate a starter's volume, so
        // crediting it at face value would read every substitute as
        // anonymous.
        let routine = (p.shooting * self.shooting()
            + p.creation * self.creation()
            + p.progression * self.progression()
            + p.retention * self.retention()
            + p.defensive * self.defensive())
            * conf;

        // Decisive events keep most of their weight from a cameo — a
        // five-minute winner is a five-minute winner.
        let decisive = p.scoring * self.decisive_events() * RatingMath::event_minutes_factor(conf);

        routine + decisive + self.finishing() + self.team_context() + self.outfield_mistakes()
    }

    /// Goals and assists.
    fn decisive_events(&self) -> f32 {
        let s = self.stats;
        let g = s.goals as f32;
        let a = s.assists as f32;
        if g <= 0.0 && a <= 0.0 {
            return 0.0;
        }
        // Deliberately LINEAR in goals. The previous model saturated
        // them (`sat(goals, 1.6)`), so a hat-trick was worth 1.8 single
        // goals and the top of the band had to be re-inflated elsewhere
        // to compensate. Compression is the shape's job, applied once,
        // after standardising — doing it twice is what made the elite
        // end unresponsive.
        g * GOAL + a * ASSIST
    }

    /// Goals scored against the chances he had — or against what his
    /// role was owed, whichever is larger.
    fn finishing(&self) -> f32 {
        let s = self.stats;
        let role_xg = match self.pos {
            PlayerFieldPositionGroup::Forward => ROLE_XG_FORWARD,
            PlayerFieldPositionGroup::Midfielder => ROLE_XG_MIDFIELDER,
            PlayerFieldPositionGroup::Defender => ROLE_XG_DEFENDER,
            PlayerFieldPositionGroup::Goalkeeper => return 0.0,
        } * (s.minutes_played as f32 / 90.0).clamp(0.0, 1.0);
        let expected = s.xg.max(role_xg);
        (s.goals as f32 - expected).clamp(FINISHING_MIN, FINISHING_MAX) * FINISHING
    }

    /// Result, clean sheet, and shared blame for goals conceded.
    fn team_context(&self) -> f32 {
        let mut value = if self.team_goals > self.opponent_goals {
            WIN
        } else if self.team_goals < self.opponent_goals {
            LOSS
        } else {
            0.0
        };
        value += self.clean_sheet_context();
        value += self.conceded_context();
        value
    }

    /// Errors short of a goal, plus discipline and profligacy. Returns
    /// a non-positive value.
    fn outfield_mistakes(&self) -> f32 {
        let s = self.stats;
        // `errors_leading_to_goal` is a subset of
        // `errors_leading_to_shot` — the goal handler promotes the
        // pending shot-error without clearing the shot counter — so
        // bill only the ones that stayed shot-errors here. The promoted
        // ones are defining moments below, at several times the weight.
        let shot_errors = s
            .errors_leading_to_shot
            .saturating_sub(s.errors_leading_to_goal);
        RatingMath::sat(shot_errors as f32, 1.2) * ERROR_TO_SHOT + self.discipline()
    }

    /// Events a match report leads with, in rating points, applied
    /// after the shape.
    fn outfield_defining_moments(&self) -> f32 {
        let s = self.stats;
        let z = s.zone_stats;
        let mut delta = (s.errors_leading_to_goal as f32).min(3.0) * ERROR_TO_GOAL;
        delta += s.own_goals as f32 * OWN_GOAL;
        delta += s.red_cards as f32 * RED_CARD;
        delta += s.yellow_cards as f32 * YELLOW_CARD;
        delta += z.penalty_fouls_conceded as f32 * ZoneCoeffs::FOUL_PENALTY;
        delta
    }
}

impl PerformanceScale {
    /// The population a shift is judged against. Measured per position
    /// — see the type's docs for how to re-derive.
    pub(super) fn for_position(pos: PlayerFieldPositionGroup) -> Self {
        match pos {
            PlayerFieldPositionGroup::Goalkeeper => Self::KEEPER,
            PlayerFieldPositionGroup::Defender => Self::DEFENDER,
            PlayerFieldPositionGroup::Midfielder => Self::MIDFIELDER,
            PlayerFieldPositionGroup::Forward => Self::FORWARD,
        }
    }
}
