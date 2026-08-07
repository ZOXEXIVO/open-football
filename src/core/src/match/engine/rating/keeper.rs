//! Goalkeeper match rating — a goals-prevented model.
//!
//! # Why this is not a save-volume model
//!
//! The rating a keeper deserves is not "how busy was he" but "how many
//! goals did he keep out that an ordinary keeper would have conceded".
//! Those two questions have opposite answers for the same keeper: a
//! keeper behind a bad defence faces more shots, so he makes more saves,
//! so a volume model pays him for the very thing that is going wrong.
//! Measured on a full 380-match season, the previous additive model
//! (`saves` up to +1.22, `workload` +0.48, and a save-count-gated
//! evidence tier worth another +0.8 at the boundary, against −0.16 for
//! the first goal conceded) produced a league where the keeper with 28
//! clean sheets and 0.37 goals against per game rated *identically* to
//! the keeper with 3 clean sheets and 2.26 — and where a 45-conceded
//! season out-rated the same player's 26-conceded season.
//!
//! # The model
//!
//! Everything hangs off one quantity, in units of goals:
//!
//! ```text
//!   expected_ga     = shots_on_target_faced · CONVERSION · difficulty
//!   goals_prevented = expected_ga − goals_conceded
//! ```
//!
//! Because `shots_faced == saves + conceded`, that expands to
//!
//! ```text
//!   goals_prevented = CONVERSION·saves − (1 − CONVERSION)·conceded
//! ```
//!
//! i.e. **a save is worth +0.34 of a goal and a concession costs −0.66**.
//! The two invariants a reader applies instinctively then hold by
//! construction, not by calibration:
//!
//!   * hold shots faced fixed, concede one more ⇒ the rating **always**
//!     falls (`monotonic_in_goals_conceded`);
//!   * hold goals conceded fixed, face one more shot ⇒ the rating
//!     **always** rises (`monotonic_in_shots_faced`).
//!
//! No arrangement of the remaining terms can break them, because the
//! remaining terms don't read save volume at all. That is the property
//! the old model lacked and the reason it had to be re-tuned every time
//! anything moved.
//!
//! A protected shutout (0 shots faced, 0 conceded) scores exactly zero
//! goals prevented and lands on the anchor — "did his job, nothing to
//! judge" — instead of needing the bespoke `dominant_defense` credit the
//! old model bolted on to rescue it.
//!
//! # Layout
//!
//! `performance` (goal units, compressed through the shared spread
//! shape) covers shot-stopping, command of the area, distribution and
//! the team result. `defining_moments` (rating units, applied *after*
//! the compression) covers the events a match report leads with — an
//! error that put the ball in the net, a flapped cross that became a
//! goal, a red card, a penalty given away. Those bypass the curve on
//! purpose: "solid apart from the howler that cost us" is a real verdict
//! and a compressed model can't produce it.

use super::{PerformanceScale, RatingContext, RatingMath};
use crate::r#match::engine::zones::ZoneCoeffs;

/// Share of shots on target that beat a league-average keeper. Real
/// football sits at ~1/3 and the engine population measures 34.5%
/// (`dev_match league` keeper ladder, 3061 shots faced / 1057 conceded)
/// — they agree, which is what makes the resulting band honest rather
/// than merely internally consistent.
///
/// This constant is the model's anchor: with the population value here,
/// the league-average keeper scores exactly zero goals prevented and
/// therefore rates exactly [`RatingShape::ANCHOR`]. Re-derive it from
/// the keeper ladder if the engine's shot or save model moves.
const ON_TARGET_CONVERSION: f32 = 0.345;

/// Engine population mean of pre-shot xG per shot **on target**
/// (measured alongside `ON_TARGET_CONVERSION`, same run). Used only as
/// the denominator of the difficulty ratio, so the ratio is 1.0 for a
/// keeper facing ordinary chances regardless of the absolute xG scale.
const REF_XG_PER_ON_TARGET: f32 = 0.1136;

/// How much of the chance-quality difference is charged to the keeper's
/// expectation. 1.0 would condition fully on xG — the FBref
/// "PSxG − GA" convention — but the engine reports *pre-shot* xG, which
/// carries the situation and not the strike, and conditioning fully on
/// it would erase the credit a keeper at a well-organised side earns for
/// the shots his defence forced wide. Half-conditioning keeps the real
/// effect (a keeper who faces tap-ins is expected to concede more) while
/// leaving the outcome as the dominant term.
const DIFFICULTY_WEIGHT: f32 = 0.50;

/// Bounds on the difficulty multiplier. A keeper cannot have his
/// expectation more than halved or more than half again by chance
/// quality alone — beyond that the sample is telling us about his
/// defence, not about him.
const DIFFICULTY_MIN: f32 = 0.65;
const DIFFICULTY_MAX: f32 = 1.50;

/// Clean sheet, in goals. Keeping the ball out of the net is the
/// keeper's headline currency and the counters can't see the half of it
/// he does with positioning and organisation — but it stays small
/// enough that it can never reorder the ladder, because a keeper who
/// concedes less collects more of them anyway.
const CLEAN_SHEET: f32 = 0.18;

/// Team result, in goals. Deliberately tiny: a keeper is on the same
/// pitch as ten other players and the scoreline at the other end is not
/// his work.
const WIN: f32 = 0.06;
const LOSS: f32 = -0.05;

/// Command of the area — cross claims, punches, sweeper interventions
/// outside the box. Saturating, so a keeper cannot farm it.
const COMMAND_SCALE: f32 = 3.0;
const COMMAND_VALUE: f32 = 0.16;

/// Distribution: completion rate against the keeper-typical baseline.
/// Small — a keeper is not judged on his passing, but a keeper who
/// cannot find a teammate is visibly costing possession.
const PASS_BASELINE: f32 = 0.72;
const PASS_MIN_ATTEMPTS: u16 = 8;
const PASS_VALUE: f32 = 0.10;

/// Mistakes that stayed mistakes, in goals. The ones that ended in the
/// net are billed below as defining moments instead.
const ERROR_TO_SHOT: f32 = -0.16;
const FAILED_CLAIM_TO_SHOT: f32 = -0.13;
const DANGEROUS_TURNOVER: f32 = -0.24;
const TURNOVER_SCALE: f32 = 2.0;

/// Defining moments, in **rating points**, applied after the spread
/// curve. An error that becomes a goal is already costing the keeper
/// −0.66 goals through the concession itself; this is the additional
/// "and it was his fault" verdict on top.
const ERROR_TO_GOAL: f32 = -1.05;
const FAILED_CLAIM_TO_GOAL: f32 = -0.85;
const OWN_GOAL: f32 = -1.30;
const RED_CARD: f32 = -1.50;
const YELLOW_CARD: f32 = -0.12;

impl<'a> RatingContext<'a> {
    /// Full goalkeeper rating: the shaped, standardised performance
    /// plus the defining moments, clamped to the band by the caller.
    pub(super) fn keeper_rating(&self) -> f32 {
        PerformanceScale::KEEPER.rate(self.keeper_performance()) + self.keeper_defining_moments()
    }

    /// The keeper's performance in goals of match value. Zero is a
    /// league-average shift.
    pub(super) fn keeper_performance(&self) -> f32 {
        let s = self.stats;
        let z = s.zone_stats;

        let mut value = self.goals_prevented();

        // Result / clean sheet — scaled by time on the pitch so a keeper
        // who came on at 80 minutes doesn't bank a full shutout.
        let share = self.minute_share();
        if self.opponent_goals == 0 {
            value += CLEAN_SHEET * share;
        }
        value += if self.team_goals > self.opponent_goals {
            WIN * share
        } else if self.team_goals < self.opponent_goals {
            LOSS * share
        } else {
            0.0
        };

        // Command of the area.
        value += RatingMath::sat(z.gk_command_actions as f32, COMMAND_SCALE) * COMMAND_VALUE;

        // Distribution.
        if s.passes_attempted >= PASS_MIN_ATTEMPTS {
            let pct = s.passes_completed as f32 / s.passes_attempted as f32;
            value += RatingMath::signed_sat(pct - PASS_BASELINE, 0.16) * PASS_VALUE;
        }

        // Mistakes short of a goal. `errors_leading_to_goal` is a subset
        // of `errors_leading_to_shot` (the goal handler promotes the
        // pending shot-error without clearing the shot counter), so bill
        // only the difference here — the promoted ones are defining
        // moments below. Same nesting for failed claims.
        let shot_errors = s
            .errors_leading_to_shot
            .saturating_sub(s.errors_leading_to_goal);
        value += shot_errors as f32 * ERROR_TO_SHOT;
        value += z
            .gk_failed_claims_to_shot
            .saturating_sub(z.gk_failed_claims_to_goal) as f32
            * FAILED_CLAIM_TO_SHOT;

        // Dangerous giveaways that stayed giveaways. A turnover that
        // became a shot is already billed through the error lane above
        // (the engine stamps both on the same play), so consume the
        // own-box count against the error count first — those are the
        // giveaways most likely to have produced the shot — then the
        // own-third remainder.
        let errors = s.errors_leading_to_shot;
        let own_box = z.dangerous_turnovers_own_box.saturating_sub(errors);
        let spill = errors.saturating_sub(z.dangerous_turnovers_own_box);
        let own_third = z.dangerous_turnovers_own_third.saturating_sub(spill);
        value += RatingMath::sat(own_third as f32 * 0.5 + own_box as f32, TURNOVER_SCALE)
            * DANGEROUS_TURNOVER;

        value
    }

    /// Goals kept out beyond what the chances faced were worth. The
    /// spine of the model — see the module docs for why everything else
    /// is deliberately small next to it.
    pub(super) fn goals_prevented(&self) -> f32 {
        let conceded = self.keeper_conceded();
        // Every shot on target is either saved or conceded, so the two
        // counters define the workload between them. Derived rather than
        // read straight off `shots_faced` on purpose: the engine keeps
        // the three in step, but a hand-built stat line can claim more
        // shots faced than it accounts for, and an unaccounted shot
        // would then be paid as a save that never happened.
        let faced = self.stats.saves.saturating_add(conceded);
        if faced == 0 {
            return 0.0;
        }
        let expected = faced as f32 * ON_TARGET_CONVERSION * self.chance_difficulty(faced);
        expected - conceded as f32
    }

    /// How hard the shots faced were, relative to a league-average shot
    /// on target. `1.0` when the engine didn't record chance values
    /// (hand-built fixtures, stat lines from before the counter existed),
    /// which leaves the count-based expectation — and both monotonicity
    /// invariants — fully intact.
    fn chance_difficulty(&self, faced: u16) -> f32 {
        let xg_faced = self.stats.xg_faced;
        if xg_faced <= 0.0 {
            return 1.0;
        }
        let per_shot = xg_faced / faced as f32;
        let ratio = per_shot / REF_XG_PER_ON_TARGET;
        (1.0 + DIFFICULTY_WEIGHT * (ratio - 1.0)).clamp(DIFFICULTY_MIN, DIFFICULTY_MAX)
    }

    /// Goals this keeper personally conceded.
    ///
    /// Taken from his own `shots_faced` ledger rather than the
    /// scoreline, so a keeper substituted at half-time is not charged
    /// for what his replacement let in — but never more than the
    /// opposition actually scored. The cap matters for two real cases:
    /// a shot on target that hits the frame is faced without being
    /// saved or conceded, and an own goal by a team-mate is on the
    /// scoreline without ever being a shot this keeper faced.
    fn keeper_conceded(&self) -> u16 {
        self.shots_faced()
            .saturating_sub(self.stats.saves)
            .min(self.opponent_goals as u16)
    }

    /// Fraction of the match this keeper was on the pitch for, used to
    /// pro-rate the team-outcome terms.
    fn minute_share(&self) -> f32 {
        (self.stats.minutes_played as f32 / 90.0).clamp(0.0, 1.0)
    }

    /// The events a match report leads with, in rating points, applied
    /// after the spread curve so a single catastrophe can still take a
    /// keeper into the disaster band.
    fn keeper_defining_moments(&self) -> f32 {
        let s = self.stats;
        let z = s.zone_stats;
        let mut delta = 0.0;
        delta += (s.errors_leading_to_goal as f32).min(3.0) * ERROR_TO_GOAL;
        delta += z.gk_failed_claims_to_goal as f32 * FAILED_CLAIM_TO_GOAL;
        delta += s.own_goals as f32 * OWN_GOAL;
        delta += s.red_cards as f32 * RED_CARD;
        delta += s.yellow_cards as f32 * YELLOW_CARD;
        // A penalty given away by the keeper is a goal he handed over.
        delta += z.penalty_fouls_conceded as f32 * ZoneCoeffs::FOUL_PENALTY;
        delta
    }
}
