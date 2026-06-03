//! Always-on contextual deltas: result, clean sheet, goals conceded,
//! discipline, and errors/cards. Applied at full strength (no minute
//! damping) because they are scoreline/team signals, not on-the-ball work.

use super::{RatingContext, sat};
use crate::PlayerFieldPositionGroup;
use crate::r#match::engine::zones::ZoneCoeffs;

impl<'a> RatingContext<'a> {
    /// Win / loss nudge.
    pub(super) fn result_context(&self) -> f32 {
        if self.team_goals > self.opponent_goals {
            0.12
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
                // a quiet shutout that the defence handled gets the
                // bookkeeping bonus only. The previous flat +0.30 stacked
                // on top of GkModest's already generous routine cap and
                // collapsed second-tier keepers into the elite band.
                let s = self.stats;
                let z = s.zone_stats;
                let saves = s.saves;
                let command = z.gk_command_actions;
                let xg_prev = s.xg_prevented;
                if saves >= 4 || command >= 2 || xg_prev > 0.5 {
                    0.30
                } else if saves >= 2 || command >= 1 || xg_prev > 0.0 {
                    0.18
                } else {
                    0.10
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
                if high_value >= 1 || routine >= 6 {
                    0.25
                } else if routine >= 3 {
                    0.15
                } else {
                    0.08
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
                let base = 0.30 * g.min(2.0);
                let mid = (g - 2.0).max(0.0) * 0.55;
                let heavy = (g - 5.0).max(0.0) * 0.20;
                -(base + mid + heavy)
            }
            PlayerFieldPositionGroup::Defender if self.opponent_goals >= 3 => {
                // Defenders share blame from the 3rd onward, smoothly.
                let extra = (self.opponent_goals as f32 - 2.0).max(0.0);
                -sat(extra, 3.0) * 1.10
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
        let fouls = sat(s.fouls as f32, 5.0) * -0.30;

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
        let offsides = -sat(s.offsides as f32, scale) * per * scale; // ≈ per-event ≤ scale*per

        let own_goals =
            s.own_goals as f32 * (ZoneCoeffs::OWN_GOAL_BASE + ZoneCoeffs::OWN_GOAL_OWN_BOX_EXTRA);

        fouls + own_third_extra + penalty_foul + offsides + own_goals
    }

    /// Errors that led to a shot or goal + yellow/red cards. Errors-
    /// to-goal hit hard per event — a single mistake is a defining
    /// moment. Always at full strength so a cameo error still lands.
    pub(super) fn errors_and_cards(&self) -> f32 {
        let s = self.stats;
        let err_shot = sat(s.errors_leading_to_shot as f32, 1.0) * -0.55;
        let err_goal = sat(s.errors_leading_to_goal as f32, 1.2) * -2.40;
        let yellow = s.yellow_cards as f32 * -0.15;
        let red = s.red_cards as f32 * -1.50;
        err_shot + err_goal + yellow + red
    }
}
