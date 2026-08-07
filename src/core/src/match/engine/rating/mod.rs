use crate::PlayerFieldPositionGroup;
use crate::r#match::PlayerMatchEndStats;
#[cfg(test)]
use crate::r#match::engine::zones::ZoneStats;

// =====================================================================
// Public API
// =====================================================================
//
// Player match ratings (1.0 ..= 10.0) computed from a
// [`PlayerMatchEndStats`] snapshot. Build a context with
// [`RatingContext::new`] and call [`RatingContext::calculate`].
//
// The model has three moving parts and nothing else:
//
//   1. a **performance value** — each position group folds its stat
//      line into one signed number, in whatever units suit the role
//      (goals prevented for a keeper, goal-denominated contribution
//      for an outfielder);
//   2. a **scale** — that number standardised against the position's
//      own population mean and spread ([`PerformanceScale`]);
//   3. a **shape** — one curve, shared by every position, turning the
//      standardised score into a rating ([`RatingShape`]).
//
// Events a match report would lead with — an error that became a goal,
// a red card, an own goal — are applied as rating points *after* the
// shape, so a single catastrophe still reaches the disaster band
// instead of being compressed away with everything else.
//
// Splitting scale from shape is what makes the model hold its
// calibration. Component weights decide what counts as good; the scale
// decides what counts as normal; the shape decides how far off-normal
// moves the rating. Change a weight and only the first is affected —
// the population level is a property of the anchor alone.
//
// The rating never reads ability, current_ability, reputation or any
// hidden attribute. A player has a good rating because the match
// produced a good stat line for him. Everything about *who* he is
// belongs upstream, in what the engine lets him do.

const RATING_MIN: f32 = 1.0;
const RATING_MAX: f32 = 10.0;

// =====================================================================
// RatingMath — stateless scoring-curve primitives
// =====================================================================
//
// Saturation curves, the minute-confidence / event-minute policy, and
// the upside-compression helpers shared by every rating component.
// Grouped onto one zero-sized struct so the model exposes no free
// functions: each call site reads `RatingMath::sat(..)` /
// `RatingMath::soft_cap(..)` etc. Pure math, no state.

struct RatingMath;

impl RatingMath {
    /// Smooth positive saturation: `1 - exp(-x/scale)`. Returns 0 for
    /// non-positive `x`. At `x = scale` ≈ 0.63, at `x = 2·scale` ≈ 0.86,
    /// at `x = 3·scale` ≈ 0.95.
    #[inline]
    fn sat(x: f32, scale: f32) -> f32 {
        if x <= 0.0 || scale <= 0.0 {
            0.0
        } else {
            1.0 - (-x / scale).exp()
        }
    }

    /// Signed smooth saturation via `tanh`. Useful for percentage-like
    /// signals that swing both above and below a baseline.
    #[inline]
    fn signed_sat(x: f32, scale: f32) -> f32 {
        if scale <= 0.0 {
            0.0
        } else {
            (x / scale).tanh()
        }
    }

    /// Smooth minute-confidence curve. Reaches ~0.40 by 15 minutes, ~0.70
    /// by 35, ~0.93 by 70, ~1.0 by 90+. Players that didn't play (0
    /// minutes) get 0.0 so their event totals contribute nothing.
    fn minute_confidence(minutes: u16) -> f32 {
        if minutes == 0 {
            return 0.0;
        }
        let m = minutes as f32 / 35.0;
        m.tanh()
    }

    /// Damp factor for direct event deltas (goals, errors-to-goal, reds,
    /// own goals). Always ≥ 0.70 so a 5-minute winner keeps the bulk of
    /// the goal credit, but a cameo doesn't get the full routine credit
    /// either — that part still goes through `minute_confidence`.
    #[inline]
    fn event_minutes_factor(conf: f32) -> f32 {
        0.70 + 0.30 * conf
    }
}

// ── The shape ────────────────────────────────────────────────────────
//
// One curve for every position. It takes a **standardised** performance
// (see `PerformanceScale` below) — how many population standard
// deviations off the positional average this shift was — and returns a
// rating movement. Because the input is standardised, a forward's goal
// and a keeper's clean sheet arrive on the same scale even though one is
// worth four times the other in goals, which is what lets a single
// shape serve all four lines instead of four hand-tuned ladders that
// drift apart.
//
// Fitted to published reference ratings, in z units:
//
// ```text
//   z   reference performance                          rating
//  +3.5 forward hat-trick                                9.2
//  +2.3 forward brace / keeper seven-save shutout        8.4-8.7
//  +1.1 forward goal / keeper four-save shutout          7.3-7.5
//   0.0 ordinary shift                                   6.72
//  -0.6 goalless forward / keeper beaten by his one shot 6.3
//  -1.9 keeper who shipped three of four                 5.2
// ```
//
// Both exponents exceed 1, so the curve is flat through the middle: a
// shift half a deviation off average is not a performance, it is noise,
// and it should not move the rating much. That flat middle is what a
// hard-capped additive model cannot produce — its caps flatten the
// middle by clipping, which also flattens everything above the cap and
// leaves the distribution bimodal.
//
// The down limb is steeper than the up limb through the ordinary range
// (0.69 vs 0.63 at z = 1) and the up limb overtakes it past z ≈ 2. That
// is the real asymmetry of football judgement: mistakes are punished
// faster than merit is rewarded, but genuine brilliance still outruns
// everything.

struct RatingShape;

impl RatingShape {
    /// Rating a positionally average shift earns. Real-football
    /// reference: WhoScored / Sofascore per-match means sit at 6.6–6.8
    /// in every position.
    const ANCHOR: f32 = 6.72;

    const UP_GAIN: f32 = 0.63;
    const UP_EXPONENT: f32 = 1.34;
    const UP_CEILING: f32 = 3.00;
    const DOWN_GAIN: f32 = 0.69;
    const DOWN_EXPONENT: f32 = 1.23;
    const DOWN_CEILING: f32 = 2.60;

    /// Map a standardised performance onto a rating.
    #[inline]
    fn rate(z: f32) -> f32 {
        Self::ANCHOR + Self::delta(z)
    }

    /// The signed rating movement for a standardised performance.
    /// Strictly increasing in `z` — a positive power and `tanh` are
    /// both monotone — which is what lets the position models state
    /// their invariants in performance units and have them hold as
    /// rating invariants.
    #[inline]
    fn delta(z: f32) -> f32 {
        if z >= 0.0 {
            Self::limb(z, Self::UP_GAIN, Self::UP_EXPONENT, Self::UP_CEILING)
        } else {
            -Self::limb(-z, Self::DOWN_GAIN, Self::DOWN_EXPONENT, Self::DOWN_CEILING)
        }
    }

    /// One limb: a power law soft-limited to `ceiling`. `x` ≥ 0.
    #[inline]
    fn limb(x: f32, gain: f32, exponent: f32, ceiling: f32) -> f32 {
        if x <= 0.0 {
            return 0.0;
        }
        ceiling * (gain * x.powf(exponent) / ceiling).tanh()
    }
}

// ── The scale ────────────────────────────────────────────────────────
//
// Per position: where the population sits and how widely it spreads, in
// that position's own performance units. Dividing by these is what
// standardises the score, and it is the only place a position's
// *distribution* is described — the component weights describe what
// counts as good, this describes what counts as normal.
//
// Both numbers are measured, not chosen: `dev_match league` prints the
// per-position mean and sd of the raw performance value (PERFORMANCE
// SCALE block). Re-derive them there whenever component weights or
// engine emission move; nothing else needs re-tuning when they do,
// because the anchor and the shape are independent of them. That
// separation is the whole point — under the old additive model every
// coefficient change moved the population level and had to be paid for
// with a compensating change somewhere else.

#[derive(Clone, Copy)]
struct PerformanceScale {
    /// The performance value that reads as an ordinary shift for this
    /// position — the one that earns [`RatingShape::ANCHOR`].
    mean: f32,
    /// Robust spread of the position's performance distribution,
    /// `(p90 − p10) / 2.563` (the decile estimator, normal-consistent).
    ///
    /// Deliberately robust rather than the raw standard deviation: a
    /// forward's raw sd is dominated by the handful of two- and
    /// three-goal matches in the tail, so dividing by it would compress
    /// the other ninety-five per cent of his season into nothing. The
    /// robust estimator describes the bulk, and the shape's ceiling
    /// handles the tail.
    sd: f32,
    /// How far this line's ratings spread in real football, relative to
    /// the shape's reference calibration. Not measurable from the
    /// engine — it is a property of how observers judge each position,
    /// and every published rating source shows the same ordering:
    /// forwards swing hardest (their contribution is lumpy and visible),
    /// defenders least (a good defensive shift looks much like an
    /// ordinary one), keepers and midfielders in between. Without it,
    /// standardising alone would give all four lines the same rating
    /// spread, and defenders would clear 7.5 five times as often as
    /// they do in life.
    sensitivity: f32,
}

impl PerformanceScale {
    /// Goals prevented plus the small secondary terms — see [`keeper`].
    /// The spread is the widest of the four lines because a keeper's
    /// value swings on whole goals: one concession moves him two thirds
    /// of a goal in a single event.
    const KEEPER: PerformanceScale = PerformanceScale {
        // Zero by construction: a keeper who neither prevents nor
        // concedes beyond what the chances were worth had an ordinary
        // afternoon. The measured population mean confirms it (0.04).
        mean: 0.00,
        sd: 0.88,
        sensitivity: 0.88,
    };
    /// The tightest line. A defender's shift is made of many small
    /// actions and almost never contains a goal, so the distribution is
    /// narrow and nearly symmetric — which is exactly why defender
    /// ratings cluster in real life too.
    const DEFENDER: PerformanceScale = PerformanceScale {
        mean: 0.46,
        sd: 0.44,
        sensitivity: 0.52,
    };
    const MIDFIELDER: PerformanceScale = PerformanceScale {
        mean: 0.26,
        sd: 0.56,
        sensitivity: 0.62,
    };
    /// Strongly right-skewed: the median forward shift (0.10) sits well
    /// below the mean (0.51) because most of the value is concentrated
    /// in the minority of matches containing a goal. Standardising
    /// against the mean is what makes a goalless forward read as
    /// slightly below par rather than as a failure — his position's
    /// average *includes* the goals he is expected to score.
    const FORWARD: PerformanceScale = PerformanceScale {
        mean: 0.13,
        sd: 1.17,
        sensitivity: 1.17,
    };

    /// Turn a raw performance value into a rating.
    #[inline]
    fn rate(&self, raw: f32) -> f32 {
        RatingShape::ANCHOR + self.sensitivity * RatingShape::delta(self.standardise(raw))
    }

    #[inline]
    fn standardise(&self, raw: f32) -> f32 {
        if self.sd <= 0.0 {
            return 0.0;
        }
        (raw - self.mean) / self.sd
    }
}

// =====================================================================
// Position weight profile
// =====================================================================

/// Multiplicative weight per component for a given position. Values
/// near 1.0 mean "this is core to the role"; values near 0 mean "this
/// component basically doesn't apply to this position".
#[derive(Clone, Copy)]
struct Profile {
    scoring: f32,
    shooting: f32,
    creation: f32,
    progression: f32,
    retention: f32,
    defensive: f32,
}

impl Profile {
    fn for_position(pos: PlayerFieldPositionGroup) -> Self {
        match pos {
            // Never consulted — goalkeepers return from `calculate`
            // before the profile is read (see `keeper.rs`). Kept as a
            // neutral row so the match stays exhaustive.
            PlayerFieldPositionGroup::Goalkeeper => Profile {
                scoring: 1.0,
                shooting: 0.5,
                creation: 0.2,
                progression: 0.2,
                retention: 0.4,
                defensive: 0.4,
            },
            PlayerFieldPositionGroup::Defender => Profile {
                scoring: 1.10,
                shooting: 0.6,
                creation: 0.7,
                progression: 0.7,
                retention: 0.8,
                defensive: 1.00,
            },
            PlayerFieldPositionGroup::Midfielder => Profile {
                scoring: 1.05,
                shooting: 0.85,
                creation: 1.10,
                progression: 1.00,
                retention: 0.90,
                defensive: 0.85,
            },
            PlayerFieldPositionGroup::Forward => Profile {
                // Forward weights skew hard toward decisive output:
                // goals / assists / direct goal threat. Routine creation,
                // progression, retention, and defensive work register but
                // can't carry a forward into the good-rating band on
                // their own — that's the spec's role expectation.
                scoring: 1.00,
                shooting: 1.05,
                creation: 0.60,
                progression: 0.45,
                retention: 0.30,
                defensive: 0.20,
            },
        }
    }
}

// =====================================================================
// RatingContext
// =====================================================================

pub struct RatingContext<'a> {
    stats: &'a PlayerMatchEndStats,
    team_goals: u8,
    opponent_goals: u8,
    pos: PlayerFieldPositionGroup,
    profile: Profile,
    /// Smooth confidence factor for time on the pitch. Applied to all
    /// routine (on-the-ball) components.
    confidence: f32,
}

impl<'a> RatingContext<'a> {
    /// Build a rating context from a player's end-of-match stats and
    /// the final scoreline (from that player's perspective).
    pub fn new(stats: &'a PlayerMatchEndStats, team_goals: u8, opponent_goals: u8) -> Self {
        let pos = stats.position_group;
        let profile = Profile::for_position(pos);
        let confidence = RatingMath::minute_confidence(stats.minutes_played);
        Self {
            stats,
            team_goals,
            opponent_goals,
            pos,
            profile,
            confidence,
        }
    }

    /// Calculate the match rating (1.0..=10.0).
    ///
    /// Two models, one shape. Each position group turns its stat line
    /// into a signed performance value in its own units, that value is
    /// standardised against the position's population (see
    /// [`PerformanceScale`]) and pushed through [`RatingShape`], and
    /// the events a match report leads with are applied afterwards so
    /// they can still reach the disaster band through the compression.
    pub fn calculate(&self) -> f32 {
        let rating = if self.is_goalkeeper() {
            self.keeper_rating()
        } else {
            self.outfield_rating()
        };
        rating.clamp(RATING_MIN, RATING_MAX)
    }

    /// The raw, un-standardised performance value for this stat line.
    /// Exposed for the calibration harness, which measures the
    /// per-position mean and sd that [`PerformanceScale`] is built
    /// from — the constants must be derived from the same expression
    /// the rating consumes, never estimated separately.
    pub fn performance_value(&self) -> f32 {
        if self.is_goalkeeper() {
            self.keeper_performance()
        } else {
            self.outfield_performance()
        }
    }

    #[inline]
    fn is_goalkeeper(&self) -> bool {
        self.pos == PlayerFieldPositionGroup::Goalkeeper
    }

    /// Effective denominator for save% calculations. The engine populates
    /// `shots_faced` directly; legacy fixtures / save files leave it at
    /// zero, in which case we synthesise it from saves + goals conceded.
    fn shots_faced(&self) -> u16 {
        self.stats
            .shots_faced
            .max(self.stats.saves + self.opponent_goals as u16)
    }
}

mod context;
mod defending;
mod expectation;
mod keeper;
mod outfield;
mod scoring;
mod volume;

pub use expectation::{RatingExpectationContext, TeamRatingSummary};
pub use volume::EngineVolumeCalibration;

#[cfg(test)]
mod season_tests;
#[cfg(test)]
mod tests;
