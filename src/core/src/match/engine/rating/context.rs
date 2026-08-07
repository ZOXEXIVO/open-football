//! Outfield contextual deltas: clean sheet, shared blame for goals
//! conceded, and discipline. Applied at full strength (no minute
//! damping) because they are scoreline/team signals, not on-the-ball
//! work. Goalkeeper equivalents live in [`super::keeper`].

use super::{RatingContext, RatingMath};
use crate::PlayerFieldPositionGroup;
use crate::r#match::engine::zones::ZoneCoeffs;

impl<'a> RatingContext<'a> {
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
            // Keepers never reach here —  owns their
            // clean-sheet credit, in goal units.
            PlayerFieldPositionGroup::Goalkeeper => 0.0,
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

    /// Shared back-line blame for goals conceded.
    /// Smooth growth: gentle through the first two, steeper from the
    /// third, slows again past the sixth (so a 10-shipping disaster
    /// stays in the disaster band rather than pinning to the floor).
    pub(super) fn conceded_context(&self) -> f32 {
        match self.pos {
            // Keepers never reach here — the goals-prevented model in
            //  charges concessions directly, at −(1 − CONV)
            // of a goal each.
            PlayerFieldPositionGroup::Goalkeeper => 0.0,
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

    /// Fouls and offsides. Own goals and conceded penalties are NOT
    /// here — they are defining moments and are billed once, after the
    /// shape curve, by the position model.
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
        let (per, scale) = match self.pos {
            PlayerFieldPositionGroup::Forward => (0.08, 4.0),
            _ => (0.06, 3.0),
        };
        let offsides = -RatingMath::sat(s.offsides as f32, scale) * per * scale; // ≈ per-event ≤ scale*per

        fouls + own_third_extra + offsides
    }
}
