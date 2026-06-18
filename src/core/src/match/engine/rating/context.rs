//! Always-on contextual deltas: result, clean sheet, goals conceded,
//! discipline, and errors/cards. Applied at full strength (no minute
//! damping) because they are scoreline/team signals, not on-the-ball work.

use super::{RatingContext, RatingMath};
use crate::PlayerFieldPositionGroup;
use crate::r#match::engine::zones::ZoneCoeffs;

impl<'a> RatingContext<'a> {
    /// Win / loss nudge. Win credit lifted 0.12 → 0.16 in the FM-parity
    /// season calibration (see `season_tests.rs`): accumulated over a
    /// 30+ match season the team-result share of an FM-style rating is
    /// larger than a 0.12 nudge produced — high-output players at
    /// winning clubs were averaging 0.2-0.4 below the believable band.
    /// The passenger / goalless-forward damping in
    /// `context_credit_factor` still discounts unearned result credit,
    /// and losses stay at full -0.15 for everyone.
    pub(super) fn result_context(&self) -> f32 {
        if self.team_goals > self.opponent_goals {
            0.16
        } else if self.team_goals < self.opponent_goals {
            -0.15
        } else {
            0.0
        }
    }

    /// Position-aware clean-sheet bonus.
    ///
    /// Defenders get a tiered credit based on stat-line evidence of
    /// actual back-line involvement: a CB who made high-danger zone
    /// interventions or posted ≥6 routine defensive actions gets full
    /// credit; a CB with only modest activity gets a reduced bonus;
    /// a truly absent passenger gets the smallest bookkeeping bonus.
    /// This is evidence-based — the gating uses observed stats, not
    /// hidden ability — and stops a back-line passenger from riding
    /// the team's clean sheet into the elite band.
    pub(super) fn clean_sheet_context(&self) -> f32 {
        if self.opponent_goals != 0 {
            return 0.0;
        }
        match self.pos {
            PlayerFieldPositionGroup::Goalkeeper => {
                // Tiered like the defender bonus: evidence-based, not
                // unconditional. A keeper who made real interventions
                // (saves, command claims, xG prevented) gets full credit;
                // a quiet shutout that the defence handled gets a
                // softened bonus — still meaningful because a keeper
                // organising a CS without a save IS doing the job
                // (positioning, sweeping, command without claim event),
                // just not the headline kind. Tiers loosened from the
                // 2026-04 over-tightening (0.10/0.18) — that pass pulled
                // TOP-GK season averages to ~6.3 vs the 6.8-7.0 reference
                // band — while keeping the busy-keeper premium intact.
                let s = self.stats;
                let z = s.zone_stats;
                let saves = s.saves;
                let command = z.gk_command_actions;
                let xg_prev = s.xg_prevented;
                // Tiers lifted 0.30/0.22/0.18 → 0.34/0.29/0.26 in the
                // FM-parity season calibration: a 35-start keeper with
                // 12 clean sheets was accumulating to ~6.46 (below the
                // believable 6.6-7.0 band) because the engine's typical
                // quiet shutout carried too little credit. The clean
                // sheet is the GK's headline season currency — repeated
                // CS credit, not save volume, is what separates a
                // 12-CS league row from a 9-conceded continental row.
                if saves >= 4 || command >= 2 || xg_prev > 0.5 {
                    0.34
                } else if saves >= 2 || command >= 1 || xg_prev > 0.0 {
                    0.29
                } else {
                    0.26
                }
            }
            PlayerFieldPositionGroup::Defender => {
                let z = self.stats.zone_stats;
                let high_value = (z.tackles_own_box
                    + z.tackles_own_six_yard
                    + z.interceptions_own_box
                    + z.interceptions_own_six_yard
                    + z.blocks_own_box
                    + z.blocks_own_six_yard
                    + z.clearances_own_box
                    + z.clearances_own_six_yard) as u16;
                let routine = self
                    .stats
                    .tackles
                    .saturating_add(self.stats.interceptions)
                    .saturating_add(self.stats.blocks)
                    .saturating_add(self.stats.clearances)
                    .saturating_add(self.stats.successful_pressures);
                // Tiers lifted again 0.32/0.20/0.13 → 0.36/0.24/0.15
                // (FM-parity DEF season pass — prior steps 0.25/0.15/
                // 0.08 → 0.32/0.20/0.13 for the Cambiaso/Thuram
                // under-credit). A 14-CS season with normal defensive
                // volume must accumulate to the believable 6.60-6.95
                // band, and the clean sheet is the back line's season
                // currency just as it is the keeper's (GK top tier
                // 0.34). The evidence gating keeps a do-nothing
                // passenger at the bookkeeping tier.
                if high_value >= 1 || routine >= 6 {
                    0.36
                } else if routine >= 3 {
                    0.24
                } else {
                    0.15
                }
            }
            PlayerFieldPositionGroup::Midfielder => 0.05,
            _ => 0.0,
        }
    }

    /// Goals-conceded penalty for goalkeepers and (lightly) defenders.
    /// Smooth growth: gentle through the first two, steeper from the
    /// third, slows again past the sixth (so a 10-shipping disaster
    /// stays in the disaster band rather than pinning to the floor).
    pub(super) fn conceded_context(&self) -> f32 {
        match self.pos {
            PlayerFieldPositionGroup::Goalkeeper => {
                let g = self.opponent_goals as f32;
                // First goal softened 0.30 → 0.16, second steepened to
                // 0.40 (FM-parity season calibration). Shipping exactly
                // one goal is the most common keeper match by far, and
                // at -0.30 it cancelled the entire save credit of a
                // routine 2-1 win — over 35 starts that single constant
                // dragged a 0.83-conceded-per-start season to ~6.4.
                // The marginal cost of the second goal rises (0.30 →
                // 0.40) so multi-concession nights stay clearly worse
                // than one-goal nights; cumulative totals from two
                // conceded on sit 0.04 below the old curve, which the
                // disaster-band and continental-cluster guards absorb.
                let first = g.min(1.0) * 0.16;
                let second = (g - 1.0).clamp(0.0, 1.0) * 0.40;
                let mid = (g - 2.0).max(0.0) * 0.55;
                let heavy = (g - 5.0).max(0.0) * 0.20;
                -(first + second + mid + heavy)
            }
            PlayerFieldPositionGroup::Defender if self.opponent_goals >= 2 => {
                // Defenders share blame from the 2nd goal onward,
                // smoothly (gate moved 3 → 2 in the FM-parity DEF
                // season pass: a two-conceded match now costs the back
                // line ≈ -0.10, which is what keeps a leaky-side season
                // separated from a clean-sheet one once routine
                // defending earns honest credit). The curve stays
                // gentle — a defender losing 0-3 takes ≈ -0.27 on top
                // of the loss penalty, landing in the real-football
                // 5.7-6.0 band for a bad day, never the disaster band.
                let extra = self.opponent_goals as f32 - 1.5;
                -RatingMath::sat(extra, 4.0) * 0.85
            }
            _ => 0.0,
        }
    }

    /// Fouls, offsides, own-goals, penalty-foul-conceded. Position-
    /// sensitive (forwards live with offsides; back-line players are
    /// extra penalised for own-third fouls).
    pub(super) fn discipline(&self) -> f32 {
        let s = self.stats;
        let z = s.zone_stats;

        // Fouls — saturating drag so a 10-foul shift doesn't compound
        // linearly.
        let fouls = RatingMath::sat(s.fouls as f32, 5.0) * -0.30;

        let own_third_extra = if matches!(
            self.pos,
            PlayerFieldPositionGroup::Defender | PlayerFieldPositionGroup::Goalkeeper
        ) {
            z.own_third_def_fouls as f32 * ZoneCoeffs::FOUL_OWN_THIRD_DEF_EXTRA_PER
        } else {
            0.0
        };
        let penalty_foul = z.penalty_fouls_conceded as f32 * ZoneCoeffs::FOUL_PENALTY;

        let (per, scale) = match self.pos {
            PlayerFieldPositionGroup::Forward => (0.08, 4.0),
            _ => (0.06, 3.0),
        };
        let offsides = -RatingMath::sat(s.offsides as f32, scale) * per * scale; // ≈ per-event ≤ scale*per

        let own_goals =
            s.own_goals as f32 * (ZoneCoeffs::OWN_GOAL_BASE + ZoneCoeffs::OWN_GOAL_OWN_BOX_EXTRA);

        fouls + own_third_extra + penalty_foul + offsides + own_goals
    }

    /// Errors that led to a shot or goal + yellow/red cards. Errors-
    /// to-goal hit hard per event — a single mistake is a defining
    /// moment. Always at full strength so a cameo error still lands.
    pub(super) fn errors_and_cards(&self) -> f32 {
        let s = self.stats;
        // A shot-error that converts is promoted to `errors_leading_to_goal`
        // at goal time, but the engine never clears the shot counter — so
        // the same mistake sits in BOTH counters. Bill only the shot-errors
        // that stayed shot-errors; the ones that became goals are punished,
        // far more harshly, through `err_goal` below. Without this a single
        // own-box giveaway-to-goal was charged as a shot-error AND a
        // goal-error (AND an own-box extra), triple-counting one mistake and
        // dropping a 1-conceded keeper into the disaster band.
        let non_goal_shot_errors = s
            .errors_leading_to_shot
            .saturating_sub(s.errors_leading_to_goal);
        let err_shot = RatingMath::sat(non_goal_shot_errors as f32, 1.0) * -0.55;
        let err_goal = RatingMath::sat(s.errors_leading_to_goal as f32, 1.2) * -2.40;
        let yellow = s.yellow_cards as f32 * -0.15;
        let red = s.red_cards as f32 * -1.50;
        err_shot + err_goal + yellow + red
    }
}
