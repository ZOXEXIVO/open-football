//! Defensive and goalkeeping rating components.

use super::{RatingContext, sat};
use crate::r#match::engine::zones::ZoneCoeffs;

impl<'a> RatingContext<'a> {
    /// Defensive work: tackles, interceptions, blocks, clearances,
    /// pressures. Includes a zone-aware premium for actions inside
    /// the own box / six-yard area and pressing high up the pitch.
    ///
    /// Saturation denominators are deliberately set so that real-football
    /// "average per-90" volumes (a CB with 2-3 tackles + 1-2 ints + 3-4
    /// clearances) earn moderate credit, not elite saturation. A defender
    /// who genuinely dominates (5+ tackles, 5+ ints, 6+ clearances) still
    /// pushes the band; their fingerprints just have to look it.
    pub(super) fn defensive(&self) -> f32 {
        let s = self.stats;
        let z = s.zone_stats;

        // Raw routine volume — tackles / interceptions / blocks /
        // clearances anywhere on the pitch. Coefficients are deliberately
        // modest: a CB with 3-4 of each lands modest credit, not elite.
        // Real lift comes from zone-aware bonuses below (own-box / six-
        // yard actions, final-third pressure / tackles) where the work
        // actually stopped an attack.
        let effective_tackles = (s.tackles as f32 - s.fouls as f32 * 0.5).max(0.0);
        let tackles = sat(effective_tackles, 6.0) * 0.30;
        let interceptions = sat(s.interceptions as f32, 6.0) * 0.30;
        let blocks = sat(s.blocks as f32, 3.5) * 0.28;
        let clearances = sat(s.clearances as f32, 7.5) * 0.16;

        let succ_pressure = sat(s.successful_pressures as f32, 5.5) * 0.16;
        let raw_pressure = s.pressures.saturating_sub(s.successful_pressures);
        let press_volume = sat(raw_pressure as f32, 12.0) * 0.04;

        // Zone-aware premium on top of the flat work — actions in
        // high-danger zones deserve more credit. Tighter saturation
        // scale means even one own-box intervention reads as meaningful
        // evidence of a real defensive moment, not lost in volume noise.
        let danger_actions =
            (z.tackles_own_box + z.interceptions_own_box + z.blocks_own_box + z.clearances_own_box)
                as f32
                * 0.5
                + (z.tackles_own_six_yard
                    + z.interceptions_own_six_yard
                    + z.blocks_own_six_yard
                    + z.clearances_own_six_yard) as f32;
        let danger_zone = sat(danger_actions, 4.0) * 0.42;

        let final_third_pressure = sat(z.pressures_won_final_third as f32, 3.0) * 0.10;
        let middle_third_int = sat(z.interceptions_middle_third as f32, 4.0) * 0.05;
        let final_third_tackle = sat(z.tackles_final_third as f32, 3.0) * 0.07;

        tackles
            + interceptions
            + blocks
            + clearances
            + succ_pressure
            + press_volume
            + danger_zone
            + final_third_pressure
            + middle_third_int
            + final_third_tackle
    }

    /// Goalkeeping routine signal: saves volume, save percentage, xG
    /// prevented, command-box actions, workload absorbed, and the
    /// quiet-shutout credit. Exceptional negatives (failed claims,
    /// dangerous turnovers, errors-to-goal extras) live in
    /// [`Self::gk_exceptional_negatives`] so they stay at full bite
    /// regardless of minutes played.
    pub(super) fn goalkeeping(&self) -> f32 {
        if !self.is_goalkeeper() {
            return 0.0;
        }
        let s = self.stats;
        let z = s.zone_stats;

        // Per-save credit — saturates so a 10-save shift isn't 10× a
        // single save.
        let saves_v = sat(s.saves as f32, 2.8) * 1.35;

        // Save percentage band — relative to a 70% baseline. We use a
        // hard zero in the 50%-70% dead-zone to keep a "made the saves
        // they were supposed to" keeper at baseline.
        let shots_faced = self.shots_faced();
        let save_pct_v = if shots_faced >= 3 {
            let pct = s.saves as f32 / shots_faced as f32;
            if pct > 0.70 {
                ((pct - 0.70) * 2.7).min(0.80)
            } else if pct < 0.50 {
                ((pct - 0.50) * 2.0).max(-0.80)
            } else {
                0.0
            }
        } else {
            0.0
        };

        // xG-prevented: positive-only (upside-only by design).
        let direct = s.xg_prevented.max(0.0);
        let xg_prev = if direct > 0.0 {
            direct
        } else if shots_faced >= 3 {
            let expected = shots_faced as f32 * 0.70;
            ((s.saves as f32 - expected) * 0.30).max(0.0)
        } else {
            0.0
        };
        let xg_prev_v = sat(xg_prev, 1.5) * 0.90;

        // Workload absorbed: showing up under a barrage. Capped via sat.
        let workload = sat((shots_faced as f32 - 2.0).max(0.0), 6.0) * 0.35;

        // Command-zone actions (cross claims, sweeper interventions).
        let command = sat(z.gk_command_actions as f32, 4.0) * 0.30;

        // Quiet-shutout credit — keeper who organised the line and
        // never had to make a save still earned the clean sheet.
        let dominant_defense = if self.opponent_goals == 0 && shots_faced < 3 {
            0.12
        } else {
            0.0
        };

        saves_v + save_pct_v + xg_prev_v + workload + command + dominant_defense
    }

    /// GK-specific exceptional negatives kept at full strength: failed
    /// claims-to-shot / -goal, dangerous turnovers, errors-to-goal in
    /// the own box. These are "defining moments of failure" and should
    /// always land, regardless of minutes played.
    pub(super) fn gk_exceptional_negatives(&self) -> f32 {
        if !self.is_goalkeeper() {
            return 0.0;
        }
        let z = self.stats.zone_stats;
        let failed_shot = z.gk_failed_claims_to_shot as f32 * ZoneCoeffs::GK_FAILED_CLAIM_TO_SHOT;
        let failed_goal = z.gk_failed_claims_to_goal as f32 * ZoneCoeffs::GK_FAILED_CLAIM_TO_GOAL;
        let turnovers = sat(
            z.dangerous_turnovers_own_third as f32 * 0.5 + z.dangerous_turnovers_own_box as f32,
            4.0,
        ) * 0.55;
        let error_extra = z.errors_to_goal_own_box as f32 * ZoneCoeffs::ERROR_TO_GOAL_OWN_BOX_EXTRA;
        // Apply GK profile weight (1.0) implicitly — these are GK-only.
        failed_shot + failed_goal - turnovers + error_extra
    }
}
