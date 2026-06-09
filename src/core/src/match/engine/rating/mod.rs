use crate::PlayerFieldPositionGroup;
use crate::r#match::PlayerMatchEndStats;
#[cfg(test)]
use crate::r#match::engine::zones::ZoneStats;

// =====================================================================
// Public API
// =====================================================================
//
// Player match ratings (1.0 ..= 10.0, neutral baseline 6.0) computed
// from a [`PlayerMatchEndStats`] snapshot. The model is component-based:
//
//   rating = BASE
//          + compress(positive routine + scoring event) [soft-cap by profile]
//          + negative routine deltas
//          + always-on contextual deltas (result, clean sheet, conceded,
//            errors, cards, discipline, GK exceptional negatives)
//          + final clamp [1, 10]
//
// Each component evaluates to a small signed "impact" value driven by
// smooth saturation curves (`sat`, `signed_sat`). Routine on-the-ball
// signal is always confidence-damped by minutes. Direct event deltas
// (goals, errors-to-goal, red cards, own goals, failed claims) keep
// most of their bite even from a cameo via `event_minutes_factor`.
//
// A cross-component compression and contribution-aware soft caps keep
// the rating distribution realistic: an anonymous starter stays under
// ~7.1, a one-goal-only finisher under ~7.6, and a hat-trick scorer
// is uncapped. Distinct ratings still register because positive
// components stack inside the cap rather than hard-clamping.
//
// Build a context with [`RatingContext::new`] and call
// [`RatingContext::calculate`].

const BASE_RATING: f32 = 6.0;
const RATING_MIN: f32 = 1.0;
const RATING_MAX: f32 = 10.0;

// =====================================================================
// Saturation helpers
// =====================================================================

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

// =====================================================================
// Confidence + event-minute policy
// =====================================================================

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

/// Compress excessive cumulative positive upside. Below the knee passes
/// through unchanged; above, each extra unit is damped to `SLOPE`
/// contribution. Knee is set so that ordinary stat lines (typical
/// per-match routine sum 0.6-1.0) pass through, but accumulated routine
/// stacking past ~1.0 starts to hit diminishing returns — keeps a
/// volume passer / busy worker from drifting into the elite band on
/// routine alone, without flattening genuinely top-tier performances.
#[inline]
fn compress_positive_delta(delta: f32) -> f32 {
    const KNEE: f32 = 1.0;
    const SLOPE: f32 = 0.40;
    if delta <= KNEE {
        delta
    } else {
        KNEE + (delta - KNEE) * SLOPE
    }
}

/// Soft cap: below `cap`, passes through; above, the excess is
/// compressed by `slope_after`. Cheaper than a hard clamp because
/// the relative ordering of "great vs very great" survives.
#[inline]
fn soft_cap(value: f32, cap: f32, slope_after: f32) -> f32 {
    if value <= cap {
        value
    } else {
        cap + (value - cap) * slope_after
    }
}

// =====================================================================
// Evidence tier — drives soft caps, context-bonus damping, and
// engagement-penalty gating from a single stat-line classification.
// Pure stat-line read: never inspects ability, CA, or any hidden flag.
// =====================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EvidenceTier {
    /// 3+ goals or 3+ G/A — no cap, full team-result credit.
    HatTrick,
    /// 2 goals or G+A — cap +2.3, full team-result credit.
    TwoGoals,
    /// One goal with low all-around volume — cap +1.6.
    OneGoalLowVolume,
    /// Cameo (<30 min) with no decisive event — cap +0.7.
    QuietCameo,
    /// Strong evidence: multi-action decisive footprint (zone work,
    /// multiple key passes / dribbles). Cap +1.3.
    Strong,
    /// Modest evidence: at least one decisive creative event. Cap +0.95.
    Modest,
    /// Passenger: routine volume only, no decisive evidence. Tight cap
    /// + halved team-result credit + engagement penalty if low touches.
    Passenger,
    /// Anonymous edge case (60+ min, very low total volume). Cap +1.1.
    AnonymousStarter,
    /// Goalkeeper-specific tiers — separate ladder because save / claim
    /// activity reads as decisive there in a way it doesn't elsewhere.
    GkBusy,
    GkModest,
    GkPassenger,
    /// Player had a scoring or G/A footprint that the simple ladders
    /// don't pre-classify — uncapped positive_delta passes through.
    Uncapped,
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
    goalkeeping: f32,
}

impl Profile {
    fn for_position(pos: PlayerFieldPositionGroup) -> Self {
        match pos {
            PlayerFieldPositionGroup::Goalkeeper => Profile {
                scoring: 1.0,
                shooting: 0.5,
                creation: 0.2,
                progression: 0.2,
                retention: 0.4,
                defensive: 0.4,
                goalkeeping: 1.0,
            },
            PlayerFieldPositionGroup::Defender => Profile {
                scoring: 1.10,
                shooting: 0.6,
                creation: 0.7,
                progression: 0.7,
                retention: 0.8,
                defensive: 1.00,
                goalkeeping: 0.0,
            },
            PlayerFieldPositionGroup::Midfielder => Profile {
                scoring: 1.05,
                shooting: 0.85,
                creation: 1.10,
                progression: 1.00,
                retention: 0.90,
                defensive: 0.85,
                goalkeeping: 0.0,
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
                goalkeeping: 0.0,
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
        let confidence = minute_confidence(stats.minutes_played);
        Self {
            stats,
            team_goals,
            opponent_goals,
            pos,
            profile,
            confidence,
        }
    }

    /// Calculate the match rating (1.0 - 10.0, base 6.0).
    ///
    /// Routine components are always damped by minute confidence so a
    /// short cameo of small touches doesn't farm a high rating. Direct
    /// event deltas (goals + clinical/decisive bonuses) keep most of
    /// their bite even from a cameo via `event_minutes_factor`.
    ///
    /// The positive sum is then compressed (a single decisive moment
    /// shouldn't combine with five tiny bonuses to reach elite band)
    /// and gated by contribution-aware soft caps: anonymous starters
    /// stay around 7.1, one-goal-only finishers around 7.6, multi-goal
    /// scorers are uncapped. Negative events (errors-to-goal, reds,
    /// own goals, conceded penalty, GK failed claims) stay at full
    /// strength so a defining moment of failure always lands.
    pub fn calculate(&self) -> f32 {
        let p = self.profile;
        let conf = self.confidence;
        let ev_factor = event_minutes_factor(conf);

        // Routine on-the-ball signal — minute-confidence damped.
        let routine = p.shooting * self.shooting()
            + p.creation * self.creation()
            + p.progression * self.progression()
            + p.retention * self.retention()
            + p.defensive * self.defensive()
            + p.goalkeeping * self.goalkeeping();
        let routine_damped = routine * conf;

        // Direct event delta — goals, decisive/clinical bonus. Softer
        // minute policy so a 5-minute winner keeps most of its credit.
        let event_pos = p.scoring * self.scoring_event();
        let event_damped = event_pos * ev_factor;

        // Split positive/negative pieces so compression only fires on
        // the upside. Routine positives get cross-component compression;
        // event positives are kept intact (one decisive moment should
        // not be sanded down by the same curve that bounds spam).
        //
        // Goalkeepers skip routine compression: every save is decisive
        // evidence in a way an outfield interception isn't, and the
        // gk_busy / gk_modest / passenger tiers in `apply_soft_caps`
        // already gate the upside. Without this exemption a barrage
        // keeper's two-plus rating units get sanded down before the
        // tier cap even sees them.
        let raw_pos_routine = routine_damped.max(0.0);
        let positive_routine = if self.is_goalkeeper() {
            raw_pos_routine
        } else {
            compress_positive_delta(raw_pos_routine)
        };
        let negative_routine = routine_damped.min(0.0);
        let positive_event = event_damped.max(0.0);
        let negative_event = event_damped.min(0.0);

        // Contribution-aware soft caps on the combined positive total.
        // The `distinctiveness_bonus` rides INSIDE the cap on purpose: a
        // passenger (cap +0.20) has its tiny texture absorbed so the
        // "did nothing" verdict holds, while a Modest / Strong line has
        // headroom for the varied-performance lift to register. This is
        // the continuous, stat-derived half of the spec's de-clustering
        // — it breaks identical-tier stat lines apart by how varied and
        // role-complete the actual work was, never by ability.
        let tier = self.evidence_tier();
        let positive_total = self.apply_soft_caps_for(
            positive_routine + positive_event + self.distinctiveness_bonus(),
            tier,
        );

        let mut rating = BASE_RATING + positive_total + negative_routine + negative_event;
        // Positive team-result credit (win bonus, clean-sheet bonus) is
        // damped when the player did nothing decisive — a passenger
        // doesn't earn the full team-result credit. Negative results
        // (a loss, goals conceded) still apply in full — being on the
        // losing side hits everyone equally regardless of tier.
        // Evidence-based: read from the same tier classification, never
        // from CA / skills.
        let context_factor = self.context_credit_factor(tier);
        let result = self.result_context();
        rating += if result > 0.0 {
            result * context_factor
        } else {
            result
        };
        rating += self.clean_sheet_context() * context_factor;
        rating += self.conceded_context();
        rating += self.discipline();
        rating += self.errors_and_cards();
        rating += self.gk_exceptional_negatives();
        // Engagement gate — a 60+ min outfield starter whose touches per
        // minute fall well below the position-typical floor visibly
        // didn't engage with the match. Pure stat-line signal that real
        // punditry catches: "anonymous shift". Limited to passenger /
        // anonymous-starter tiers so a decisive moment (G/A or zone
        // work) is never overridden by a low-touch underlying stat line.
        if matches!(
            tier,
            EvidenceTier::Passenger | EvidenceTier::AnonymousStarter
        ) {
            rating += self.engagement_penalty();
        }
        // Forward role-expectation drag: a forward who played meaningful
        // minutes without G/A and without enough goal threat / creative
        // footprint hasn't done their primary job. Pure stat-line read,
        // smooth penalty (no hard cap), self-zero outside the gate.
        rating += self.attacking_role_expectation();

        rating.clamp(RATING_MIN, RATING_MAX)
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

    /// Continuous, stat-derived "texture of a varied shift" lift, clamped
    /// to 0..=0.12 (spec Section A). Three signals, none of which read
    /// ability:
    ///
    ///   * `unique_positive_action_types` — how many *different* kinds of
    ///     positive action the player registered. A one-note line (only
    ///     clearances) scores low; a player who tackled, intercepted,
    ///     progressed and created scores high. Logarithmic so the first
    ///     few distinct actions matter most.
    ///   * `role_core_action_balance` — did the player do *both* halves
    ///     of their role (defend AND build, create AND finish)? Rewards a
    ///     complete shift over a lopsided one.
    ///   * `high_value_zone_share` — share of the player's actions that
    ///     happened in high-danger / high-value zones.
    ///
    /// Damped by minute confidence so a cameo doesn't farm texture, and
    /// (because it rides inside the tier soft cap upstream) a passenger's
    /// lift is absorbed rather than lifting the "did nothing" verdict.
    fn distinctiveness_bonus(&self) -> f32 {
        let types = self.unique_positive_action_types() as f32;
        // 0.04 · ln(1 + n): 0 at n=0, ≈0.044 at n=2, ≈0.083 at n=6.
        let unique = 0.04 * (1.0 + types).ln();
        let role_core = 0.03 * self.role_core_action_balance();
        let high_value = 0.02 * self.high_value_zone_share();
        ((unique + role_core + high_value) * self.confidence).clamp(0.0, 0.12)
    }

    /// Count of distinct *role dimensions* the player was active in —
    /// finishing, creating, carrying, progressing, defending, keeping.
    /// Measures the *breadth* of a performance (did the player do several
    /// different KINDS of football?) rather than raw counter variety, so
    /// a busy-but-one-note shift — e.g. a CB who only tackled and cleared —
    /// reads as a single dimension, not five. This is what keeps routine
    /// volume from buying a distinctiveness lift it hasn't earned.
    fn unique_positive_action_types(&self) -> u32 {
        let s = self.stats;
        let z = s.zone_stats;
        let dimensions = [
            // finishing / direct goal threat
            s.goals > 0 || s.assists > 0 || s.shots_on_target > 0,
            // chance creation
            s.key_passes > 0 || s.passes_into_box > 0 || s.crosses_completed > 0,
            // ball-carrying
            s.successful_dribbles > 0 || s.progressive_carries > 0,
            // progressive passing
            s.progressive_passes > 0,
            // defending
            s.tackles > 0
                || s.interceptions > 0
                || s.blocks > 0
                || s.clearances > 0
                || s.successful_pressures > 0,
            // goalkeeping
            s.saves > 0 || z.gk_command_actions > 0,
        ];
        dimensions.iter().filter(|d| **d).count() as u32
    }

    /// 0..1 measure of whether the player did both halves of their role.
    /// `2·min / (a+b)` peaks at 1.0 when the two halves are balanced and
    /// falls toward 0 for a one-sided contribution (or no contribution).
    fn role_core_action_balance(&self) -> f32 {
        let s = self.stats;
        let (a, b) = match self.pos {
            PlayerFieldPositionGroup::Forward => (
                (s.goals + s.assists + s.shots_on_target) as f32,
                (s.key_passes + s.passes_into_box + s.successful_dribbles) as f32,
            ),
            PlayerFieldPositionGroup::Midfielder => (
                (s.key_passes + s.passes_into_box + s.progressive_passes + s.progressive_carries)
                    as f32,
                (s.tackles + s.interceptions + s.successful_pressures) as f32,
            ),
            PlayerFieldPositionGroup::Defender => (
                (s.tackles + s.interceptions + s.blocks + s.clearances) as f32,
                (s.progressive_passes + s.progressive_carries + s.passes_into_box) as f32,
            ),
            PlayerFieldPositionGroup::Goalkeeper => {
                (s.saves as f32, s.zone_stats.gk_command_actions as f32)
            }
        };
        let sum = a + b;
        if sum <= 0.0 {
            0.0
        } else {
            (2.0 * a.min(b) / sum).clamp(0.0, 1.0)
        }
    }

    /// Share of the player's countable actions that happened in
    /// high-value zones (own box / six-yard for defending, into-box /
    /// final-third creation for attacking). `+1.0` in the denominator
    /// keeps a low-volume line from reading as a misleading 100% share.
    fn high_value_zone_share(&self) -> f32 {
        let s = self.stats;
        let z = s.zone_stats;
        let high_value = (z.tackles_own_box
            + z.tackles_own_six_yard
            + z.interceptions_own_box
            + z.interceptions_own_six_yard
            + z.blocks_own_box
            + z.blocks_own_six_yard
            + z.clearances_own_box
            + z.clearances_own_six_yard
            + z.carries_into_box
            + z.half_space_passes_into_box
            + z.central_passes_into_box
            + z.pressures_won_final_third
            + z.tackles_final_third) as f32;
        let baseline = (s.tackles
            + s.interceptions
            + s.blocks
            + s.clearances
            + s.passes_into_box
            + s.key_passes
            + s.successful_pressures) as f32;
        (high_value / (high_value + baseline + 1.0)).clamp(0.0, 1.0)
    }
}

mod calibration;
mod context;
mod defending;
mod expectation;
mod scoring;

pub use expectation::{RatingExpectationContext, TeamRatingSummary};

#[cfg(test)]
mod season_tests;
#[cfg(test)]
mod tests;
