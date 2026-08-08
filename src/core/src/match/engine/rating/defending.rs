//! Outfield defensive rating components. Goalkeeping lives in
//! [`super::keeper`] — it shares no machinery with this file.

use super::{RatingContext, RatingMath};

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
        // Saturation scales tightened from 6.0 → 4.5 so two tackles or
        // two interceptions — a typical fullback / CB shift — registers
        // at 36% saturation rather than 28%. The prior scale was chosen
        // assuming engine output 4-5 routine actions per defender per
        // match, but observed output sits closer to 2-3, leaving the
        // routine work under-credited and dragging defender season
        // averages (Cambiaso 6.20, Thuram 6.09).
        // Coefficients lifted 0.30/0.30/0.28/0.16 → 0.34/0.34/0.30/0.18
        // in the FM-parity DEF/MID season pass: a clean-sheet CB season
        // with normal volume (2-3 tackles/ints, 3-5 clearances) was
        // accumulating to ~6.49 against the believable 6.60-6.95 band.
        // Routine honest defending is the back-line's primary output;
        // the saturation scales keep extraordinary volume from running
        // away, and the busy-CB cluster guards in tests.rs bound the
        // top end.
        let effective_tackles = (s.tackles as f32 - s.fouls as f32 * 0.5).max(0.0);
        let tackles = RatingMath::sat(effective_tackles, 4.5) * 0.34;
        let interceptions = RatingMath::sat(s.interceptions as f32, 4.5) * 0.34;
        let blocks = RatingMath::sat(s.blocks as f32, 3.5) * 0.30;
        // Clearances saturation scale tightened 7.5 → 6.0: 3 clearances
        // — a typical CB / fullback match — now registers at 39% rather
        // than 33% saturation. Same calibration motive as the tackles
        // / interceptions tighten above.
        let clearances = RatingMath::sat(s.clearances as f32, 6.0) * 0.18;

        // Positional defending — shots the opposition struck with this
        // defender goal-side of the ball and inside its lane.
        //
        // The one component here that is not an EVENT. Everything above
        // pays a defender for doing something: winning a tackle, cutting
        // out a pass, throwing a body in the way. A defender who is
        // simply in the right place produces none of those, and the
        // model had no way to pay him — which stopped being survivable
        // the moment defenders actually held a line (`DefensiveRecovery`):
        // their measured performance distribution HALVED (mean 0.42 →
        // 0.22) because the chase-the-ball volume that used to carry
        // them collapsed, and the whole line lost its ceiling — 90% of
        // defender matches finishing under 6.92, only 1.4% reaching 7.5
        // against a real ~8-10%.
        //
        // Saturating, and modest per shot: covering the ball is the
        // baseline expectation of the job, so it lifts the routine
        // honest shift off the floor rather than manufacturing standout
        // ratings. A defender goal-side for most of the ~13 shots his
        // team faces reads as having held his shape all afternoon.
        let in_position = RatingMath::sat(z.shots_covered_in_position as f32, 2.5) * 0.34;

        let succ_pressure = RatingMath::sat(s.successful_pressures as f32, 5.5) * 0.16;
        let raw_pressure = s.pressures.saturating_sub(s.successful_pressures);
        let press_volume = RatingMath::sat(raw_pressure as f32, 12.0) * 0.04;

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
        let danger_zone = RatingMath::sat(danger_actions, 4.0) * 0.42;

        let final_third_pressure = RatingMath::sat(z.pressures_won_final_third as f32, 3.0) * 0.10;
        let middle_third_int = RatingMath::sat(z.interceptions_middle_third as f32, 4.0) * 0.05;
        let final_third_tackle = RatingMath::sat(z.tackles_final_third as f32, 3.0) * 0.07;

        tackles
            + interceptions
            + blocks
            + clearances
            + in_position
            + succ_pressure
            + press_volume
            + danger_zone
            + final_third_pressure
            + middle_third_int
            + final_third_tackle
    }
}
