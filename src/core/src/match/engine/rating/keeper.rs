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
//! # Why shot-stopping is not the whole rating
//!
//! `goals_prevented` divides through by workload by construction — the
//! expectation it subtracts is proportional to shots faced. That makes
//! it the right measure of *shot-stopping skill*, and the wrong measure
//! of a keeper's night on its own: a keeper behind a sieve is judged
//! against a sieve's expectation, so the extra goals cancel and the
//! rating stops reading the scoreline at all. Measured over a full
//! season it flattened the entire middle of the ladder — 0.79 goals
//! against per game rating *below* 1.66 — with a season spread of 0.20
//! where real keeper ratings spread ~0.55.
//!
//! So the performance is shot-stopping **plus** a smaller
//! [`DEFENSIVE_OUTCOME`] term that reads goals conceded against a
//! league-average night rather than against this keeper's own workload.
//! Both halves are needed and neither is sufficient: the first is what
//! separates a keeper from his defence, the second is what stops the
//! model pretending he is independent of it.
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
const ON_TARGET_CONVERSION: f32 = 0.292;

/// Engine population mean **post-shot** expected goal per shot on target
/// — `SaveModel::expected_goal_on_target` averaged over the shots keepers
/// actually face. Used only as the denominator of the difficulty ratio,
/// so the ratio is 1.0 for a keeper facing ordinary strikes and the
/// anchor is untouched.
///
/// Re-derive from the `xG/shot` column of the KEEPER SEASON LADDER
/// (`SQUAD_SPREAD=2 dev_match league 20 2 8 18`). Measured 2026-08-08
/// across levels 8-18: **0.532 – 0.592, mean 0.562** — a 1.11× spread
/// where the pre-shot values this replaced ran 0.082 – 0.133, a 1.6×
/// spread that tracked the division. That flatness is the property the
/// whole design turns on: it is what lets [`DIFFICULTY_WEIGHT`] carry
/// real weight without dragging a division off the anchor.
///
/// It sits well above the population concede rate (~34%) because the
/// reference keeper is modelled standing at the goal centre while real
/// keepers track the ball, so his stretch is larger for the same strike.
/// That is a constant offset, and dividing by it removes it — the ratio
/// is what the model reads, never the absolute.
const REF_XGOT_PER_ON_TARGET: f32 = 0.562;

/// How much of the chance-quality difference is charged to the keeper's
/// expectation. This is the term that separates a keeper from the
/// defence in front of him, and it can now carry most of the weight
/// because the engine measures the difficulty of the STRIKE rather than
/// of the situation.
///
/// History worth keeping. It ran at 0.25 with a 0.82–1.22 clamp, against
/// a ratio built from **pre-shot** xG. Two things were wrong with that
/// and only one of them was the weight. Pre-shot xG describes the chance
/// the defence conceded — where the shot was taken from, under what
/// pressure — so a keeper behind a well-organised side, facing tame
/// strikes from good positions, was still judged to have faced
/// league-average shots; the term could not do the job it existed for.
/// And because the ratio was taken against a global constant while
/// pre-shot chance quality is a property of the division (lower divisions
/// ~0.08 xG per shot on target against a top flight's ~0.11), turning the
/// weight up dragged whole divisions off the anchor — which is why it was
/// cut to 0.25 and the clamp tightened, and why the term was then too
/// weak to matter. `SaveModel::expected_goal_on_target` removes the
/// second problem at the source: it reads placement, power and height
/// through the same contest-normalised save curve at every level, so the
/// population mean is flat across divisions and the weight is free to
/// carry the signal.
const DIFFICULTY_WEIGHT: f32 = 0.90;

/// Bounds on the difficulty multiplier. Wide, because the quantity is now
/// a genuine per-shot measurement rather than a proxy: a keeper who spent
/// the afternoon catching tame strikes down the middle really was asked
/// for less than one picking corner-bound shots out of the top corner,
/// and the model should say so.
///
/// The ceiling is the model's OWN maximum, not a taste judgement.
/// `expected_goal_on_target` is bounded above by `1 − SaveModel::MIN_SAVE`
/// (0.92), so the hardest possible average strike gives a ratio of
/// `0.92 / 0.562 = 1.637` and a multiplier of `1 + 0.90·0.637 = 1.573`.
/// Setting the clamp there means it never binds on anything the engine
/// can actually produce — and, just as importantly, it means a hand-built
/// fixture cannot feed the model an impossible chance value and buy a
/// keeper who shipped three the rating of one who shipped one.
///
/// The floor is deliberately tighter than the model's own minimum (0.228,
/// from `1 − MAX_SAVE`). It is small-sample protection: on a one-shot
/// night the mean IS that shot, and a collapsed expectation would make
/// the single concession catastrophic. The asymmetry is intentional —
/// the downside case is the one that produces absurd ratings.
const DIFFICULTY_MIN: f32 = 0.45;
const DIFFICULTY_MAX: f32 = 1.55;

// There is deliberately no separate clean-sheet bonus. The shutout is
// just the `conceded == 0` end of `DEFENSIVE_OUTCOME`, which already
// pays it continuously; a discrete bonus on top billed it twice (a
// 1-save shutout reached 7.21, above the guard that keeps protected
// keepers from averaging 7.0+) and put a cliff between conceding one
// and conceding none that no other step on the ladder has.

/// League-average goals conceded per 90, measured from the same keeper
/// ladder as [`ON_TARGET_CONVERSION`]. Re-derive it alongside.
const REF_CONCEDED_PER_90: f32 = 1.15;

/// Weight on the defensive outcome — goals conceded measured against a
/// league-average night instead of against this keeper's own workload.
///
/// [`Self::goals_prevented`] on its own is a pure save-rate measure:
/// the expectation it subtracts scales *linearly with shots faced*, so
/// a keeper behind a sieve is judged against a sieve's expectation and
/// the extra goals he ships cancel out exactly. Measured over a full
/// season, that left the whole middle of the ladder flat — 1.66 goals
/// against per game out-rating 0.79 on save volume alone, which is the
/// same blindness the old save-count model had, one level removed.
///
/// This term puts the scoreline at his end back in the rating, which is
/// what every real rating provider does with a keeper. It stays
/// deliberately secondary to the shot-stopping: conceding is his
/// defence's night as much as his, so it moves the rating about a third
/// as hard as a save-rate difference of the same size.
///
/// Calibrated against two numbers at once, which is what fixes its
/// value: it has to lift the keeper season spread to real football's
/// ~0.55 *without* driving `r(conceded/game, rating)` past −0.8. Those
/// pull opposite ways — at 0.30 the spread was right but `r` reached
/// −0.94, a league where a good keeper at a bad club could no longer
/// rate well, which is the realism the term was added to protect. 0.18
/// is where the per-match top also settles: at 0.22 a three-save shutout
/// reached 7.70 against a WhoScored/FM reference of 7.2-7.5, because the
/// shutout credit stacks on shot-stopping that already paid for the
/// saves which produced it.
const DEFENSIVE_OUTCOME: f32 = 0.18;

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

        // The scoreline at his end, against a league-average night. This
        // is the only term that reads goals conceded without dividing by
        // the workload that produced them — see `DEFENSIVE_OUTCOME` for
        // why the model is blind in the middle of the ladder without it.
        value += DEFENSIVE_OUTCOME * (REF_CONCEDED_PER_90 * share - self.keeper_conceded() as f32);
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

    /// How hard the strikes he faced were, relative to a league-average
    /// shot on target. `1.0` when the engine didn't record chance values
    /// (hand-built fixtures, stat lines from before the counter existed),
    /// which leaves the count-based expectation — and both monotonicity
    /// invariants — fully intact.
    ///
    /// `stats.xg_faced` accumulates
    /// [`SaveModel::expected_goal_on_target`](crate::r#match::engine::ball::interactions::SaveModel)
    /// per shot: the probability a league-average keeper concedes that
    /// exact strike, from its placement, power and height. Averaging it
    /// and re-expressing it as a ratio (rather than using the sum
    /// directly as the expectation) keeps the model anchored on
    /// [`ON_TARGET_CONVERSION`] — the constant that makes a
    /// league-average keeper score exactly zero goals prevented — so the
    /// population level does not move when the save model is re-tuned.
    fn chance_difficulty(&self, faced: u16) -> f32 {
        let xgot_faced = self.stats.xg_faced;
        if xgot_faced <= 0.0 {
            return 1.0;
        }
        let per_shot = xgot_faced / faced as f32;
        let ratio = per_shot / REF_XGOT_PER_ON_TARGET;
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
