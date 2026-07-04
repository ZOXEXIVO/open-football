//! Defensive and goalkeeping rating components.

use super::{RatingContext, RatingMath};
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
        let saves_v = RatingMath::sat(s.saves as f32, 2.8) * 1.35;

        // Save percentage band — relative to a 70% baseline. We use a
        // hard zero in the 50%-70% dead-zone to keep a "made the saves
        // they were supposed to" keeper at baseline.
        let shots_faced = self.shots_faced();
        // Positive slope lifted 2.7 → 3.0 (FM-parity season calibration)
        // so a genuinely high save percentage under real volume pays a
        // touch more — the busy-keeper-in-a-loss archetype was compressing
        // toward the quiet-clean-sheet keeper once CS credit was lifted.
        let save_pct_v = if shots_faced >= 3 {
            let pct = s.saves as f32 / shots_faced as f32;
            if pct > 0.70 {
                ((pct - 0.70) * 3.0).min(0.80)
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
        let xg_prev_v = RatingMath::sat(xg_prev, 1.5) * 0.90;

        // Workload absorbed: showing up under a barrage. Capped via sat.
        let workload = RatingMath::sat((shots_faced as f32 - 2.0).max(0.0), 6.0) * 0.35;

        // Command-zone actions (cross claims, sweeper interventions).
        let command = RatingMath::sat(z.gk_command_actions as f32, 4.0) * 0.30;

        // Protected-shutout credit — keeper who organised the line and
        // kept a clean sheet behind a back four that limited the
        // opposition to little or nothing. Keeping the goal intact IS
        // the keeper's headline job; when the defence allowed only a
        // handful of shots there is barely any save volume to carry that
        // credit, so a quiet shutout was landing marooned near the 6.0
        // baseline (a 0-save clean sheet read 6.47, barely above a
        // do-nothing match, and a leaky 29-conceded season out-rated a
        // 24-clean-sheet one because save volume — not goals kept out —
        // drove the score).
        //
        // The credit is solid at zero shots faced and tapers linearly to
        // zero by three shots, because from there the save-based credit
        // above takes over: a keeper peppered with shots earns through
        // `saves_v` / `save_pct` / `workload`, a keeper protected by his
        // defence earns through the shutout itself. Replacing the old
        // flat `shots_faced < 3` gate with a continuous taper collapses
        // the save-volume cliff (one routine save used to be worth +0.4
        // over the same clean sheet kept untested) without leaking a
        // second bonus to the moderate-workload keeper, who is already
        // paid by saves. Lifted 0.30 → 0.62 and reshaped in the
        // protected-shutout calibration pass.
        let dominant_defense = if self.opponent_goals == 0 {
            let shot_taper = (1.0 - shots_faced as f32 / 3.0).max(0.0);
            0.62 * shot_taper
        } else {
            0.0
        };

        // Limited-exposure relief — the conceded-side mirror of the
        // protected-shutout term. A keeper beaten by essentially the only
        // shot he faced (one conceded, no save to show for it) had no
        // chance to be the difference: a single goal off a single shot is
        // an ordinary keeper outing, not a failure, yet with zero save
        // volume to carry any positive signal he was marooned in the low
        // 5s (a 0-save / 1-conceded keeper read 5.76, and any small blemish
        // dropped him toward the disaster line). Pays ONLY the untested
        // 0-save case — the taper is gone by two shots faced, so a keeper
        // who made even one save, or faced more, earns through saves /
        // workload / save% instead, and the underperformance season
        // fixtures (whose conceding matches all carry a save) are
        // untouched. Gated to exactly one conceded AND a blameless one: a
        // keeper beaten repeatedly by the little he faced, or whose own
        // error or own-goal put it there (a howler, not bad luck), gets
        // nothing here.
        let limited_exposure = if self.opponent_goals == 1
            && self.stats.errors_leading_to_goal == 0
            && self.stats.own_goals == 0
        {
            let shot_taper = (1.0 - shots_faced as f32 / 2.0).max(0.0);
            0.55 * shot_taper
        } else {
            0.0
        };

        saves_v + save_pct_v + xg_prev_v + workload + command + dominant_defense + limited_exposure
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
        let turnovers = RatingMath::sat(
            z.dangerous_turnovers_own_third as f32 * 0.5 + z.dangerous_turnovers_own_box as f32,
            4.0,
        ) * 0.55;
        // `errors_to_goal_own_box` is intentionally NOT re-penalised here.
        // The engine sets it on the very same play that already bumped
        // `errors_leading_to_goal` (an own-box giveaway that became a goal),
        // and that goal-error is billed at near-own-goal strength in
        // `errors_and_cards`. Stacking a separate own-box extra on top
        // triple-counted one keeper mistake and dumped a single-goal keeper
        // into the disaster band (a 1-conceded GK reading ~3.9 in a draw).
        // Apply GK profile weight (1.0) implicitly — these are GK-only.
        failed_shot + failed_goal - turnovers
    }
}
