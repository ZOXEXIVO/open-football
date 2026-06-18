//! Stage 2 (context-expectation) and Stage 3 (human-condition) rating
//! layers, plus the per-side [`TeamRatingSummary`] they consume.
//!
//! The raw stat-line rating from [`RatingContext::calculate`] answers
//! "what does the stat line say in a vacuum?". It does NOT know whether
//! the player was a passenger in a side that bossed the game or a
//! firefighter in a back-four under siege — two identical stat lines
//! read the same. That convergence is exactly the robotic ~7.10
//! clustering the rebalance targets.
//!
//! These two layers re-introduce the missing context **without** ever
//! reading ability / CA:
//!
//!   * **Stage 2 — expectation delta.** Every role has an *expected*
//!     contribution given how the team played (possession share, shot
//!     share, defensive load) and the match state (result, goal diff).
//!     A defender expected to defend a lot (low block, heavy defensive
//!     load) who actually made the interventions earns credit; one who
//!     coasted in a dominant side does not. We compare a normalised
//!     `actual_contribution_index` against that `expected_contribution`
//!     and fold a bounded `tanh` delta into the rating. The magnitude
//!     is deliberately small (±0.25) — context *separates* otherwise
//!     identical lines, it does not overturn the stat-line verdict.
//!
//!   * **Stage 3 — condition modifier.** A player who started tired, or
//!     who emptied the tank and then got sloppy, performed worse than
//!     the same stat line at full energy; a player who put in a heavy
//!     high-intensity shift and still delivered earns a small respect
//!     bump. Reads the post-match physical snapshot only.
//!
//! Both layers are applied by [`RatingContext::calculate_contextual`],
//! which the engine calls to seed the public `match_rating` while
//! leaving `raw_match_rating` as the pure stat-line value. The
//! reputation- and personality-driven shaping stays downstream in the
//! league pipeline (`compute_effective_ratings`); this module is the
//! deterministic, match-data-only half of the model.

use super::{EvidenceTier, RATING_MAX, RATING_MIN, RatingContext, RatingMath};
use crate::PlayerFieldPositionGroup;
use crate::club::player::events::PositionLoad;
use crate::r#match::PlayerMatchEndStats;
use crate::r#match::engine::result::PlayerMatchPhysicalSnapshot;
use std::cmp::Ordering;

// =====================================================================
// Stage-2 magnitude budget
// =====================================================================
//
// All of these are intentionally modest. The whole point of the
// contextual layer is to *de-cluster* similar stat lines by a few
// tenths, not to swamp the stat-line verdict. A two-goal striker still
// rates like a two-goal striker regardless of context; a tidy recycler
// and a progressive carrier in the same midfield now diverge by a
// little more than the stat line alone managed.

/// Final `tanh` scale on the expectation delta. With the index/expected
/// values living in a ~0..1 band, an actual-vs-expected gap of 0.5 maps
/// to `tanh(0.5) ≈ 0.46`, i.e. ≈ ±0.10 of rating. A blowout gap of ~1.0
/// approaches the clamp.
const CONTEXT_DELTA_SCALE: f32 = 0.22;
/// Hard clamp on the context delta so no amount of team-behaviour skew
/// can move a rating by more than a quarter point.
const CONTEXT_DELTA_CLAMP: f32 = 0.25;

// =====================================================================
// TeamRatingSummary
// =====================================================================

/// Aggregate match-behaviour summary for one team, folded from every
/// player's end-of-match stat line. The rating model only needs a few
/// team-level proxies — who shot, who held the ball, who had to defend —
/// and they all derive from stats the engine already records per player,
/// so nothing new has to be tracked during the sim.
#[derive(Clone, Copy, Debug, Default)]
pub struct TeamRatingSummary {
    pub shots_total: u32,
    pub shots_on_target: u32,
    pub xg: f32,
    pub passes_attempted: u32,
    pub passes_completed: u32,
    /// Tackles + interceptions + blocks + clearances across the side —
    /// the volume of defensive work the team had to do.
    pub defensive_actions: u32,
}

impl TeamRatingSummary {
    /// Fold one team's player stat lines into a behaviour summary.
    pub fn from_stats<'a, I>(stats: I) -> Self
    where
        I: IntoIterator<Item = &'a PlayerMatchEndStats>,
    {
        let mut s = TeamRatingSummary::default();
        for p in stats {
            s.shots_total += p.shots_total as u32;
            s.shots_on_target += p.shots_on_target as u32;
            s.xg += p.xg.max(0.0);
            s.passes_attempted += p.passes_attempted as u32;
            s.passes_completed += p.passes_completed as u32;
            s.defensive_actions +=
                (p.tackles + p.interceptions + p.blocks + p.clearances) as u32;
        }
        s
    }
}

// =====================================================================
// RatingExpectationContext
// =====================================================================

/// Per-player context for the Stage-2 / Stage-3 layers. Built from the
/// player's own team summary, the opponent summary, the scoreline (from
/// the player's perspective) and an optional physical snapshot.
///
/// `opponent_rep_gap` is part of the model for completeness but defaults
/// to `0.0` on the engine path (the match engine has no reputation
/// context). The downstream league pipeline owns the reputation- and
/// personality-driven shaping; the team-dominance proxies below
/// (`team_shot_share`, `team_defensive_load`) already stand in for
/// "favourite vs underdog" using only match data.
#[derive(Clone, Copy, Debug)]
pub struct RatingExpectationContext {
    /// opponent_reputation − own_reputation, clamped to −3..3. Positive
    /// when the opponent is stronger. `0.0` when no reputation context
    /// is available (engine path).
    pub opponent_rep_gap: f32,
    /// −1 loss / 0 draw / +1 win.
    pub team_result: i8,
    /// team_goals − opponent_goals.
    pub team_goal_diff: i8,
    /// Share of total match shots taken by the player's team (0..1,
    /// fallback 0.5). High → the team attacked; low → it was pinned.
    pub team_shot_share: f32,
    /// Share of total completed passes by the player's team (0..1,
    /// fallback 0.5). Stand-in for possession.
    pub team_possession_proxy: f32,
    /// opp_shots / total_shots (0..1, fallback 0.5). High → the team
    /// spent the match defending. Complement of `team_shot_share`.
    pub team_defensive_load: f32,
    /// Starting condition as a 0..1 fraction (snapshot.starting_condition
    /// / 10000), if a physical snapshot was available.
    pub starting_condition_pct: Option<f32>,
    /// End-of-shift energy as a 0..1 fraction, if available.
    pub final_energy_pct: Option<f32>,
    /// High-intensity load share for the shift (0..1), if available.
    pub high_intensity_load: Option<f32>,
}

impl RatingExpectationContext {
    /// Neutral context — 50/50 shares, drawn match, no rep gap, no
    /// condition data. Stage-2 and Stage-3 deltas all evaluate to ≈0 so
    /// `calculate_contextual` ≈ `calculate`. Used as the fallback when
    /// team metrics can't be derived.
    pub fn neutral() -> Self {
        RatingExpectationContext {
            opponent_rep_gap: 0.0,
            team_result: 0,
            team_goal_diff: 0,
            team_shot_share: 0.5,
            team_possession_proxy: 0.5,
            team_defensive_load: 0.5,
            starting_condition_pct: None,
            final_energy_pct: None,
            high_intensity_load: None,
        }
    }

    /// Build a context from the two team summaries, the scoreline (from
    /// the player's perspective) and an optional physical snapshot.
    pub fn from_match(
        own: &TeamRatingSummary,
        opp: &TeamRatingSummary,
        team_goals: u8,
        opponent_goals: u8,
        snapshot: Option<&PlayerMatchPhysicalSnapshot>,
    ) -> Self {
        let total_shots = (own.shots_total + opp.shots_total) as f32;
        let team_shot_share = if total_shots > 0.0 {
            own.shots_total as f32 / total_shots
        } else {
            0.5
        };
        let team_defensive_load = if total_shots > 0.0 {
            opp.shots_total as f32 / total_shots
        } else {
            0.5
        };
        let total_passes = (own.passes_completed + opp.passes_completed) as f32;
        let team_possession_proxy = if total_passes > 0.0 {
            own.passes_completed as f32 / total_passes
        } else {
            0.5
        };

        let team_result = match team_goals.cmp(&opponent_goals) {
            Ordering::Greater => 1,
            Ordering::Less => -1,
            Ordering::Equal => 0,
        };
        let team_goal_diff =
            (team_goals as i16 - opponent_goals as i16).clamp(-99, 99) as i8;

        let (starting_condition_pct, final_energy_pct, high_intensity_load) = match snapshot {
            Some(s) => (
                Some((s.starting_condition as f32 / 10_000.0).clamp(0.0, 1.0)),
                Some((s.final_match_energy as f32 / 10_000.0).clamp(0.0, 1.0)),
                Some(s.high_intensity_load_hint.clamp(0.0, 1.0)),
            ),
            None => (None, None, None),
        };

        RatingExpectationContext {
            opponent_rep_gap: 0.0,
            team_result,
            team_goal_diff,
            team_shot_share,
            team_possession_proxy,
            team_defensive_load,
            starting_condition_pct,
            final_energy_pct,
            high_intensity_load,
        }
    }

    /// Fill in the reputation gap (used by the downstream pipeline, which
    /// owns the team-reputation lookup). Clamped to ±3.
    pub fn with_rep_gap(mut self, opponent_rep_gap: f32) -> Self {
        self.opponent_rep_gap = opponent_rep_gap.clamp(-3.0, 3.0);
        self
    }
}

// =====================================================================
// Stage 2 + Stage 3 calculation
// =====================================================================

impl<'a> RatingContext<'a> {
    /// Public/contextual rating: the pure stat-line value plus the
    /// Stage-2 expectation delta and the Stage-3 condition modifier.
    ///
    /// `calculate()` (Stage 1) is preserved verbatim and remains the
    /// `raw_match_rating`; this method layers the two deterministic,
    /// match-data-only stages on top to seed the public `match_rating`.
    pub fn calculate_contextual(&self, ctx: &RatingExpectationContext) -> f32 {
        let raw = self.calculate();
        let delta = self.context_delta(ctx);
        let condition = self.condition_modifier(ctx, raw);
        (raw + delta + condition).clamp(RATING_MIN, RATING_MAX)
    }

    /// Stage-2 expectation delta: `tanh(actual − expected) · scale`,
    /// clamped. Positive when the player out-performed what their role
    /// and the team's behaviour demanded; negative when they coasted on
    /// the team's dominance.
    pub(super) fn context_delta(&self, ctx: &RatingExpectationContext) -> f32 {
        // A short cameo hasn't had time to build a representative
        // contribution index, so the delta would be noise — fade it in
        // with the same minute confidence the routine signal uses.
        let actual = self.actual_contribution_index();
        let expected = self.expected_contribution(ctx);
        // Fade with minute confidence — a short cameo hasn't built a
        // representative contribution index, so the delta would be noise.
        let confidence = self.confidence.clamp(0.0, 1.0);
        let raw = (actual - expected).tanh() * CONTEXT_DELTA_SCALE * confidence;
        raw.clamp(-CONTEXT_DELTA_CLAMP, CONTEXT_DELTA_CLAMP)
    }

    /// Normalised "how much did this player actually contribute to their
    /// core role" index, ~0..1.2. Built from the same stats the rating
    /// components read, but weighted toward the role-defining actions so
    /// it can be compared against `expected_contribution`.
    pub(super) fn actual_contribution_index(&self) -> f32 {
        let s = self.stats;
        let z = s.zone_stats;
        let idx = match self.pos {
            PlayerFieldPositionGroup::Goalkeeper => {
                let saves = RatingMath::sat(s.saves as f32, 3.5);
                let xgp = RatingMath::sat(s.xg_prevented.max(0.0), 1.5);
                let command = RatingMath::sat(z.gk_command_actions as f32, 3.0);
                // Count distinct error incidents once: every
                // `errors_leading_to_goal` is already inside
                // `errors_leading_to_shot` (the engine promotes the same
                // play), so summing both double-weighted a keeper's blunder
                // against their own contribution index.
                let errors = RatingMath::sat(
                    s.errors_leading_to_shot as f32
                        + (z.gk_failed_claims_to_shot + z.gk_failed_claims_to_goal) as f32,
                    2.0,
                );
                // A clean sheet is the keeper's defining contribution to the
                // result — save volume or not. A protected shutout means the
                // keeper organised a back line that conceded nothing, which
                // the raw save / command counters never register. Without
                // this floor a quiet shutout reads as "zero contribution" and
                // takes an expectation drag for being untested, inverting the
                // one outcome that matters most for a keeper. Sits just below
                // the dominant-side expectation baseline (≈0.35) so it lands a
                // protected shutout at ≈neutral rather than turning the
                // expectation layer into a second clean-sheet bonus.
                let clean_sheet = if self.opponent_goals == 0 { 0.40 } else { 0.0 };
                0.45 * saves + 0.30 * xgp + 0.15 * command + clean_sheet - 0.30 * errors
            }
            PlayerFieldPositionGroup::Defender => {
                let routine = RatingMath::sat(
                    (s.tackles + s.interceptions + s.blocks + s.clearances) as f32,
                    7.0,
                );
                let box_work = RatingMath::sat(self.danger_zone_actions(), 4.0);
                let build = RatingMath::sat(
                    (s.progressive_passes + s.progressive_carries + s.passes_into_box) as f32,
                    6.0,
                );
                0.45 * routine + 0.35 * box_work + 0.20 * build
            }
            PlayerFieldPositionGroup::Midfielder => {
                // Pass-volume weight lifted 0.25 → 0.30 (FM-parity
                // DEF/MID season pass): high-volume circulation is the
                // recycler role's actual contribution, and the prior
                // weighting read a 60-pass shift as barely half of the
                // expected midfield influence.
                let pass_vol = RatingMath::sat(s.passes_completed as f32, 50.0);
                let progression = RatingMath::sat(
                    (s.progressive_passes + s.progressive_carries) as f32,
                    6.0,
                );
                let creation = RatingMath::sat((s.key_passes + s.passes_into_box) as f32, 4.0);
                let press = RatingMath::sat(s.successful_pressures as f32, 5.0);
                0.30 * pass_vol + 0.30 * progression + 0.25 * creation + 0.20 * press
            }
            PlayerFieldPositionGroup::Forward => {
                let decisive = RatingMath::sat(s.goals as f32 + s.assists as f32 * 0.6, 1.5);
                let threat = RatingMath::sat(s.xg.max(0.0), 1.5);
                let sot = RatingMath::sat(s.shots_on_target as f32, 3.0);
                let creation = RatingMath::sat((s.key_passes + s.passes_into_box) as f32, 3.0);
                let dribbles = RatingMath::sat(s.successful_dribbles as f32, 3.0);
                0.40 * decisive + 0.20 * threat + 0.15 * sot + 0.15 * creation + 0.10 * dribbles
            }
        };
        idx.max(0.0)
    }

    /// Expected contribution for the role given the team's behaviour and
    /// the match state. Baselines and slopes follow the rebalance spec:
    /// a keeper / defender under heavy defensive load is expected to be
    /// busy; a midfielder in a balanced game is expected to influence it;
    /// a forward in a dominant attacking side is expected to threaten.
    pub(super) fn expected_contribution(&self, ctx: &RatingExpectationContext) -> f32 {
        // Reputation-favourite factor (0 on the engine path). When the
        // player's side is the stronger one (negative rep gap) more is
        // expected of the attacking players; when facing a stronger side
        // (positive rep gap) more is expected of the keeper / defence.
        let favourite_factor = (-ctx.opponent_rep_gap).max(0.0) / 3.0;
        let underdog_factor = ctx.opponent_rep_gap.max(0.0) / 3.0;

        let base = match self.pos {
            PlayerFieldPositionGroup::Goalkeeper => {
                // Spec baseline was 0.25 + 0.50·load; the dev_match
                // benchmark showed the 0.50 slope over-penalised busy
                // keepers — a keeper facing a barrage and making 3-4 saves
                // (actual index ≈ 0.4) sat *below* the expected 0.60 at
                // heavy load and took a context drag for shipping any goal,
                // pulling the already-low GK band down further. Softened to
                // 0.30 so a "did the job under fire" keeper sits near
                // neutral and only a keeper who let everything in is dinged.
                0.25 + 0.30 * ctx.team_defensive_load + 0.12 * underdog_factor
            }
            PlayerFieldPositionGroup::Defender => {
                let low_possession = (0.5 - ctx.team_possession_proxy).max(0.0) * 2.0;
                // Baseline softened 0.30 → 0.28 (FM-parity DEF season
                // pass): the routine clean-sheet defender was carrying
                // a small permanent expectation drag in every match
                // even when doing exactly the job the team's shape
                // asked of them.
                0.28 + 0.35 * ctx.team_defensive_load + 0.10 * low_possession
            }
            PlayerFieldPositionGroup::Midfielder => {
                // Balanced midfields demand the most: a 50/50 possession
                // split is where a midfielder's influence is most
                // expected. Lopsided games (very high or very low share)
                // relax the bar slightly. Baseline 0.35 → 0.31 and
                // balanced weight 0.15 → 0.12 (FM-parity MID season
                // pass): the old bar sat near 0.62 for possession
                // sides, a level only a heavy creator reaches, so every
                // ordinary midfielder carried -0.06..-0.08 of
                // expectation drag per match purely for existing.
                let balanced = 1.0 - 2.0 * (ctx.team_possession_proxy - 0.5).abs();
                0.31 + 0.25 * ctx.team_possession_proxy + 0.12 * balanced
            }
            PlayerFieldPositionGroup::Forward => {
                0.35 + 0.30 * ctx.team_shot_share + 0.15 * favourite_factor
            }
        };
        base.clamp(0.0, 1.2)
    }

    /// Stage-3 condition modifier (−0.22..0.12). Reads the physical
    /// snapshot only — never ability. A tired starter, or one who ran
    /// the tank dry and then got sloppy, is dragged; a player who put in
    /// a heavy high-intensity shift and still performed earns a small,
    /// strictly bounded respect bump.
    pub(super) fn condition_modifier(
        &self,
        ctx: &RatingExpectationContext,
        raw_rating: f32,
    ) -> f32 {
        let minutes = self.stats.minutes_played;
        let mut modifier = 0.0_f32;

        // Started the match already short of a full tank — every action
        // is run on fumes. Linear below 70% condition.
        if let Some(start) = ctx.starting_condition_pct {
            if start < 0.70 {
                modifier -= (0.70 - start) * 0.35;
            }
        }

        if let Some(end) = ctx.final_energy_pct {
            // Emptied the tank over a full shift — late-game drop-off.
            if end < 0.35 && minutes >= 60 {
                modifier -= (0.35 - end) * 0.25;
            }
            // Running on fumes AND it showed: errors / fouls / loose
            // touches beyond the position-normal floor get amplified.
            // This is the "tiredness caused sloppiness" punishment — it
            // only fires when the player both gassed out (<30%) and made
            // the mistakes.
            if end < 0.30 {
                let sloppy = (self.stats.errors_leading_to_shot
                    + self.stats.errors_leading_to_goal
                    + self.stats.fouls
                    + self.stats.miscontrols
                    + self.stats.heavy_touches) as f32;
                let normal = self.position_sloppy_floor();
                let excess = (sloppy - normal).max(0.0);
                modifier -= RatingMath::sat(excess, 3.0) * 0.18;
            }
        }

        // Heavy high-intensity shift that still delivered — a small
        // respect bump, capped hard at +0.10 so it can never substitute
        // for actual contribution. Gated on the raw rating being above
        // baseline: you have to have performed to earn the load credit.
        if let Some(hi) = ctx.high_intensity_load {
            let position_default = PositionLoad::high_intensity_share(self.pos);
            if hi > position_default + 0.12 && raw_rating > 6.0 {
                let bump = ((hi - position_default) * 0.25).min(0.10);
                modifier += bump;
            }
        }

        modifier.clamp(-0.22, 0.12)
    }

    /// Position-typical count of "sloppy" events (errors + fouls + loose
    /// touches) below which the tired-amplification penalty does not
    /// fire. Forwards and full-press midfielders naturally give the ball
    /// away more, so their floor is a touch higher.
    fn position_sloppy_floor(&self) -> f32 {
        match self.pos {
            PlayerFieldPositionGroup::Goalkeeper => 1.0,
            PlayerFieldPositionGroup::Defender => 2.0,
            PlayerFieldPositionGroup::Midfielder => 3.0,
            PlayerFieldPositionGroup::Forward => 3.0,
        }
    }

    /// Sum of own-box / six-yard defensive interventions — the
    /// high-danger work that distinguishes a firefighting defender from
    /// a volume one. Shared by the contribution index and the
    /// distinctiveness bonus.
    pub(super) fn danger_zone_actions(&self) -> f32 {
        let z = self.stats.zone_stats;
        (z.tackles_own_box
            + z.tackles_own_six_yard
            + z.interceptions_own_box
            + z.interceptions_own_six_yard
            + z.blocks_own_box
            + z.blocks_own_six_yard
            + z.clearances_own_box
            + z.clearances_own_six_yard) as f32
    }

    /// Deterministic per-identity texture band for this stat line's
    /// evidence tier. The downstream pipeline multiplies a seeded signed
    /// hash of `(player_id, date, team_id)` by this band to break
    /// identical-looking stat lines apart without changing the verdict.
    /// Wider for decisive performances (a goal day has room to vary),
    /// tightest for passengers and keepers.
    pub fn texture_band(&self) -> f32 {
        match self.evidence_tier() {
            EvidenceTier::Passenger
            | EvidenceTier::AnonymousStarter
            | EvidenceTier::QuietCameo => 0.03,
            EvidenceTier::Modest => 0.06,
            EvidenceTier::Strong | EvidenceTier::OneGoalLowVolume => 0.08,
            EvidenceTier::GkBusy | EvidenceTier::GkModest | EvidenceTier::GkPassenger => 0.04,
            EvidenceTier::TwoGoals | EvidenceTier::HatTrick | EvidenceTier::Uncapped => 0.08,
        }
    }
}
