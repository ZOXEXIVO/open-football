//! Where the player actually is — the read-only picture the sub-minds
//! reason over.
//!
//! Gathered once per tick by the caller, in the same spirit as
//! `TransferDesireContext`: the mind never walks the simulator world,
//! and a sub-mind never reaches back into `Player` for a field. That
//! keeps every faculty unit-testable against a plain struct and stops
//! the reflect rules quietly growing dependencies on the whole
//! simulation.
//!
//! Deliberately thin. Anything a sub-mind can work out from what he
//! remembers or what he wants belongs in the organs, not here — this is
//! only the ground truth he cannot know from the inside.

use super::organs::memory::ActorRef;

/// The player's objective circumstances this tick.
#[derive(Debug, Clone, Copy, Default)]
pub struct MindSituation {
    pub age: u8,
    /// Personality, 0–20.
    pub ambition: f32,
    pub pressure: f32,
    pub adaptability: f32,
    /// Rolling share of recent competitive matches started, 0..1.
    /// Neutral 0.5 before he has played enough for it to mean anything.
    pub starter_ratio: f32,
    /// Competitive appearances since his last goal. Saturates.
    pub apps_since_goal: u8,
    /// Days left on his deal. 0 when he has none.
    pub contract_days_left: u16,
    /// Days since he joined. 0 when unknown.
    pub days_at_club: u16,
    /// Share of matches his squad role implies he should start, 0..1.
    ///
    /// Carried as the derived number rather than the status itself, for
    /// two reasons: `PlayerSquadStatus` is not `Copy` (and the whole
    /// situation is), and more importantly the table that turns a role
    /// into an expectation already exists at
    /// [`PlayingTimeFrustrationConfig::expected_start_share`]. Storing
    /// the answer keeps one source of truth instead of a second copy
    /// that can drift.
    ///
    /// [`PlayingTimeFrustrationConfig::expected_start_share`]: crate::club::player::happiness::PlayingTimeFrustrationConfig::expected_start_share
    pub expected_start_share: f32,
    /// Who the manager is, so the professional mind can hold a read of a
    /// specific person rather than of "the manager" in the abstract —
    /// and notice when it becomes somebody else.
    pub manager: ActorRef,
    /// Club reputation, 0..1.
    pub club_reputation: f32,
    /// Is he playing abroad?
    pub is_abroad: bool,
    /// Does he speak the local language?
    pub speaks_local_language: bool,
    /// Compatriots and shared-language teammates in the squad.
    pub familiar_teammates: u8,
    pub is_on_loan: bool,
}

impl MindSituation {
    /// A neutral situation, for tests and for sites with nothing to
    /// offer yet. Every faculty reads this as "no view" rather than as
    /// bad news.
    pub fn neutral() -> Self {
        MindSituation {
            age: 26,
            ambition: 10.0,
            pressure: 10.0,
            adaptability: 10.0,
            starter_ratio: 0.5,
            apps_since_goal: 0,
            contract_days_left: 0,
            days_at_club: 0,
            expected_start_share: 0.50,
            manager: ActorRef::NONE,
            club_reputation: 0.5,
            is_abroad: false,
            speaks_local_language: true,
            familiar_teammates: 0,
            is_on_loan: false,
        }
    }

    /// Has he been here long enough to have formed a view? Below this
    /// the social and professional faculties hold their tongue rather
    /// than reading a settling-in period as a problem — the same
    /// honeymoon the happiness path already respects.
    pub const SETTLING_DAYS: u16 = 90;

    #[inline]
    pub fn is_settled(&self) -> bool {
        self.days_at_club >= Self::SETTLING_DAYS
    }

    /// Years of prime left, 0..1. Nothing at the very end of a career,
    /// full through the mid twenties. Continuous, so there is no
    /// birthday at which a player's outlook flips.
    pub fn career_runway(&self) -> f32 {
        ((34.0 - self.age as f32) / 12.0).clamp(0.0, 1.0)
    }

    /// How far into his career he is, 0..1 — the inverse read, for the
    /// faculties that care about time running out rather than time left.
    #[inline]
    pub fn career_spent(&self) -> f32 {
        1.0 - self.career_runway()
    }

    /// Is he playing regularly?
    #[inline]
    pub fn is_playing(&self) -> bool {
        self.starter_ratio >= 0.5
    }

    /// Contract pressure, 0..1 — nothing with two years left, total at
    /// expiry.
    pub fn contract_pressure(&self) -> f32 {
        if self.contract_days_left == 0 {
            return 0.0;
        }
        (1.0 - self.contract_days_left as f32 / 730.0).clamp(0.0, 1.0)
    }

    /// Is he socially adrift — abroad, without the language, with nobody
    /// who shares his background?
    pub fn is_culturally_isolated(&self) -> bool {
        self.is_abroad && !self.speaks_local_language && self.familiar_teammates == 0
    }

    /// How far short of what he was promised he is falling, −1..+1.
    /// Positive means he is playing more than his role implies.
    pub fn playing_time_gap(&self) -> f32 {
        (self.starter_ratio - self.expected_start_share).clamp(-1.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_neutral_situation_reads_as_no_view_rather_than_bad_news() {
        let s = MindSituation::neutral();
        assert_eq!(s.playing_time_gap(), 0.0);
        assert_eq!(s.contract_pressure(), 0.0);
        assert!(!s.is_culturally_isolated());
        assert!(!s.is_settled(), "day one is not settled");
    }

    #[test]
    fn the_runway_shortens_continuously() {
        let at = |age: u8| {
            MindSituation {
                age,
                ..MindSituation::neutral()
            }
            .career_runway()
        };

        assert_eq!(at(22), 1.0);
        assert!(at(28) > at(31));
        assert!(at(31) > at(33));
        assert_eq!(at(35), 0.0);
        // No cliff anywhere in between.
        for age in 22..35u8 {
            assert!(at(age) >= at(age + 1));
        }
    }

    #[test]
    fn contract_pressure_builds_over_the_last_two_years() {
        let with = |days: u16| {
            MindSituation {
                contract_days_left: days,
                ..MindSituation::neutral()
            }
            .contract_pressure()
        };

        assert_eq!(with(730), 0.0);
        assert!((with(365) - 0.5).abs() < 0.01);
        assert!(with(30) > 0.9);
        assert_eq!(
            with(0),
            0.0,
            "no contract is not the same as an expiring one"
        );
    }

    #[test]
    fn the_playing_time_gap_is_measured_against_the_role_he_was_given() {
        let benched_key_player = MindSituation {
            expected_start_share: 0.70,
            starter_ratio: 0.2,
            ..MindSituation::neutral()
        };
        let same_minutes_as_a_backup = MindSituation {
            expected_start_share: 0.15,
            starter_ratio: 0.2,
            ..MindSituation::neutral()
        };

        assert!(benched_key_player.playing_time_gap() < -0.4);
        assert!(
            same_minutes_as_a_backup.playing_time_gap() > -0.1,
            "identical minutes, and only one of them has a grievance"
        );
    }

    #[test]
    fn cultural_isolation_needs_all_three() {
        let adrift = MindSituation {
            is_abroad: true,
            speaks_local_language: false,
            familiar_teammates: 0,
            ..MindSituation::neutral()
        };
        assert!(adrift.is_culturally_isolated());

        let has_a_compatriot = MindSituation {
            familiar_teammates: 2,
            ..adrift
        };
        assert!(
            !has_a_compatriot.is_culturally_isolated(),
            "one man who speaks your language changes everything"
        );
    }
}
