//! Where a manager actually is — the ground truth he cannot read off
//! his own state.
//!
//! Gathered once by the caller and handed to the faculties, exactly as
//! `MindSituation` is on the player side, and for the same reason: the
//! mind never walks the simulator graph.
//!
//! **No situation, no thinking.** [`StaffMind::tick`] reviews goals and
//! consolidates but does not let the faculties reflect, because a
//! neutral situation is not a neutral input — to the ambition mind it
//! reads as a manager sitting mid-table with a board that half-trusts
//! him, which would quietly resolve wants the caller knows nothing
//! about. The player mind learned this the hard way; the staff mind
//! inherits the rule.
//!
//! [`StaffMind::tick`]: super::StaffMind::tick

/// Everything a faculty needs to know about the job, from outside.
#[derive(Debug, Clone, Copy)]
pub struct StaffSituation {
    // ── Him ─────────────────────────────────────────────────────
    /// Age in years.
    pub age: f32,
    /// His own standing in the game, 0..1 — normalised world
    /// reputation, not the club's.
    pub standing: f32,
    /// Physical and mental load, 0..1. The existing `Staff::fatigue`,
    /// normalised.
    pub strain: f32,

    // ── The job ─────────────────────────────────────────────────
    /// Months since he took it.
    pub months_in_the_job: u16,
    /// Months left on the contract. Negative once it has expired.
    pub contract_months_left: i16,
    /// The club's standing, 0..1, on the same scale as [`Self::standing`].
    /// Above his own means he is punching up.
    pub club_standing: f32,

    // ── The people above him ────────────────────────────────────
    /// What the board actually thinks, 0..1 — the mean of
    /// `ManagerRelationship`'s five facets. He does not read this
    /// directly; his faculties form a *belief* about it and the belief
    /// can be wrong.
    pub board_trust: f32,
    /// How hard the board is being leaned on, 0..1 — the
    /// `BoardPressure` gauges.
    pub board_pressure: f32,
    /// Has the board been backing him in the market, 0..1.
    pub board_backing: f32,

    // ── Results ─────────────────────────────────────────────────
    /// League position, 1-based. 0 when unknown.
    pub league_position: u8,
    pub league_size: u8,
    /// Where the club expects to finish. 0 when nobody has said.
    pub expected_position: u8,
    /// How far through the season, 0..1.
    pub season_progress: f32,
    /// Trophies won at this club.
    pub trophies_here: u8,

    // ── The squad and the stands ────────────────────────────────
    /// Fraction of the first-team squad he brought in himself, 0..1.
    pub squad_is_his: f32,
    /// How the dressing room is with him, 0..1.
    pub dressing_room: f32,
    /// How the supporters are with him, 0..1.
    pub terraces: f32,
    /// Is the board fielding offers for a player he wants to keep?
    pub best_player_wanted: bool,
}

impl StaffSituation {
    /// The read a caller with nothing to offer gets. Deliberately
    /// mid-range on every axis and deliberately **not** passed to
    /// `reflect` — see the module docs.
    pub fn neutral() -> Self {
        StaffSituation {
            age: 45.0,
            standing: 0.5,
            strain: 0.3,
            months_in_the_job: 12,
            contract_months_left: 24,
            club_standing: 0.5,
            board_trust: 0.55,
            board_pressure: 0.3,
            board_backing: 0.5,
            league_position: 0,
            league_size: 0,
            expected_position: 0,
            season_progress: 0.5,
            trophies_here: 0,
            squad_is_his: 0.5,
            dressing_room: 0.5,
            terraces: 0.5,
            best_player_wanted: false,
        }
    }

    /// How far short of expectation the side is, −1..+1. Positive means
    /// he is over-performing.
    ///
    /// Zero when nobody has stated an expectation *or* the league is
    /// unknown — an honest "no view", not "doing fine".
    pub fn against_expectation(&self) -> f32 {
        if self.league_position == 0 || self.expected_position == 0 || self.league_size == 0 {
            return 0.0;
        }
        let gap = self.expected_position as f32 - self.league_position as f32;
        (gap / self.league_size as f32 * 3.0).clamp(-1.0, 1.0)
    }

    /// Is he in a relegation fight? Continuous rather than a threshold:
    /// how deep into the bottom quarter he is, weighted by how late in
    /// the season it is — the same position in August and in April are
    /// not the same situation.
    pub fn relegation_danger(&self) -> f32 {
        if self.league_position == 0 || self.league_size < 4 {
            return 0.0;
        }
        let size = self.league_size as f32;
        let from_bottom = (size - self.league_position as f32) / size;
        let depth = (1.0 - from_bottom / 0.25).clamp(0.0, 1.0);
        depth * (0.35 + self.season_progress * 0.65)
    }

    /// How exposed the job is, 0..1. Board trust is the dominant term
    /// and results are the second; pressure from outside only matters
    /// when the board is already wavering, which is what makes a board
    /// that backs its manager worth having.
    pub fn job_exposure(&self) -> f32 {
        let distrust = (1.0 - self.board_trust).clamp(0.0, 1.0);
        let results = (-self.against_expectation()).clamp(0.0, 1.0);
        let danger = self.relegation_danger();
        let outside = self.board_pressure * distrust;
        (distrust * 0.45 + results * 0.25 + danger * 0.20 + outside * 0.10).clamp(0.0, 1.0)
    }

    /// Is he punching above the club, and by how much? The signal that
    /// turns into `GetABiggerJob`.
    pub fn outgrown_the_club(&self) -> f32 {
        (self.standing - self.club_standing).clamp(0.0, 1.0)
    }

    /// Is the contract itself a reason to be thinking about the future?
    pub fn contract_pressure(&self) -> f32 {
        if self.contract_months_left <= 0 {
            return 1.0;
        }
        let months = self.contract_months_left as f32;
        // Nothing for two years out, rising continuously to the last day.
        ((24.0 - months) / 24.0).clamp(0.0, 1.0)
    }

    /// How far into a career he is, 0..1. Drives the winding-down
    /// wants without a birthday threshold.
    pub fn career_stage(&self) -> f32 {
        ((self.age - 35.0) / 30.0).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_neutral_situation_reads_as_no_view_rather_than_bad_news() {
        let situation = StaffSituation::neutral();
        assert_eq!(
            situation.against_expectation(),
            0.0,
            "no stated expectation is not the same as meeting one"
        );
        assert_eq!(situation.relegation_danger(), 0.0);
        assert_eq!(situation.outgrown_the_club(), 0.0);
    }

    #[test]
    fn over_and_under_performing_point_opposite_ways() {
        let mut over = StaffSituation::neutral();
        over.league_size = 20;
        over.expected_position = 12;
        over.league_position = 4;

        let mut under = over;
        under.league_position = 17;

        assert!(over.against_expectation() > 0.0);
        assert!(under.against_expectation() < 0.0);
    }

    #[test]
    fn the_same_position_is_worse_in_april_than_in_august() {
        let mut august = StaffSituation::neutral();
        august.league_size = 20;
        august.league_position = 19;
        august.season_progress = 0.05;

        let mut april = august;
        april.season_progress = 0.92;

        assert!(april.relegation_danger() > august.relegation_danger() * 1.8);
    }

    #[test]
    fn a_board_that_backs_him_absorbs_the_noise_outside() {
        let mut backed = StaffSituation::neutral();
        backed.board_trust = 0.9;
        backed.board_pressure = 1.0;

        let mut wavering = backed;
        wavering.board_trust = 0.25;

        assert!(
            wavering.job_exposure() > backed.job_exposure() * 2.0,
            "outside pressure only bites when the board is already unsure"
        );
    }

    #[test]
    fn contract_pressure_builds_over_the_last_two_years() {
        let mut situation = StaffSituation::neutral();
        situation.contract_months_left = 36;
        assert_eq!(situation.contract_pressure(), 0.0);

        situation.contract_months_left = 12;
        let halfway = situation.contract_pressure();
        assert!(halfway > 0.4 && halfway < 0.6);

        situation.contract_months_left = -1;
        assert_eq!(situation.contract_pressure(), 1.0);
    }

    #[test]
    fn a_career_winds_down_continuously() {
        let mut young = StaffSituation::neutral();
        young.age = 34.0;
        assert_eq!(young.career_stage(), 0.0);

        let mut older = young;
        older.age = 58.0;
        let mut oldest = young;
        oldest.age = 70.0;

        assert!(older.career_stage() > 0.0);
        assert!(oldest.career_stage() > older.career_stage());
    }
}
