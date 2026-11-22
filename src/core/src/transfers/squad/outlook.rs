//! How likely a first choice is to leave the club within a window.
//!
//! One continuous reading from what the club can see: it is selling him,
//! he has asked to go, somebody has approached, his contract is running
//! down, his career is, or he has outgrown the side. A vacancy is this
//! reading at certainty.

use chrono::{Datelike, NaiveDate};

use crate::transfers::squad::SuccessionAudit;
use crate::{Person, Player, PlayerFieldPositionGroup, PlayerStatusType};

/// Each reason a man may leave, as the chance it alone gives of it.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct DepartureOutlook {
    pub marketed: f32,
    pub asked: f32,
    pub approached: f32,
    pub contract: f32,
    pub ageing: f32,
    pub outgrown: f32,
}

impl DepartureOutlook {
    /// The club has put him on the market.
    const MARKETED: f32 = 0.6;
    /// He has asked to leave.
    const ASKED: f32 = 0.5;
    /// One recorded approach; each further one closes the same share of
    /// what is left.
    const APPROACH: f32 = 0.25;
    /// Months of contract from which the clock starts to matter.
    const CONTRACT_MONTHS: f32 = 18.0;
    /// Years before the age his position's first choices stop at from
    /// which the end is in sight.
    const CAREER_YEARS: f32 = 6.0;
    /// Perceived levels above the side's first choices at which a man
    /// starts to have outgrown it, and the span over which that fills.
    const ABOVE_FROM: f32 = 8.0;
    const ABOVE_SPAN: f32 = 24.0;
    const OUTGROWN: f32 = 0.5;

    /// `marketed` is the club's own answer about him; `side_level` is the
    /// perceived level of its first choices, and `level` his own.
    pub fn of(player: &Player, marketed: bool, level: u8, side_level: u8, date: NaiveDate) -> Self {
        let months = player.contract.as_ref().map(|contract| {
            (contract.expiration.year() - date.year()) * 12 + contract.expiration.month() as i32
                - date.month() as i32
        });
        DepartureOutlook {
            marketed: if marketed || player.is_on_the_market() {
                Self::MARKETED
            } else {
                0.0
            },
            asked: if player.statuses.has(PlayerStatusType::Req) {
                Self::ASKED
            } else {
                0.0
            },
            approached: Self::approach_term(player.approaches_recorded()),
            contract: Self::contract_term(months),
            ageing: Self::ageing_term(player.age(date), player.position().position_group()),
            outgrown: Self::outgrown_term(level, side_level),
        }
    }

    /// The chance he leaves within a window, the reasons taken together.
    pub fn likelihood(&self) -> f32 {
        let stays = [
            self.marketed,
            self.asked,
            self.approached,
            self.contract,
            self.ageing,
            self.outgrown,
        ]
        .iter()
        .map(|term| 1.0 - term.clamp(0.0, 1.0))
        .product::<f32>();
        1.0 - stays
    }

    pub fn approach_term(approaches: u8) -> f32 {
        1.0 - (1.0 - Self::APPROACH).powi(i32::from(approaches))
    }

    /// Nothing with a year and a half left, certainty the day it runs out.
    pub fn contract_term(months: Option<i32>) -> f32 {
        match months {
            Some(months) => (1.0 - months.max(0) as f32 / Self::CONTRACT_MONTHS).clamp(0.0, 1.0),
            None => 0.0,
        }
    }

    /// Nothing six years from the age his position's first choices stop
    /// at, certainty when he reaches it.
    pub fn ageing_term(age: u8, group: PlayerFieldPositionGroup) -> f32 {
        let remaining = f32::from(SuccessionAudit::career_end_age(group).saturating_sub(age));
        (1.0 - remaining / Self::CAREER_YEARS).clamp(0.0, 1.0)
    }

    pub fn outgrown_term(level: u8, side_level: u8) -> f32 {
        let above = f32::from(level) - f32::from(side_level) - Self::ABOVE_FROM;
        (above / Self::ABOVE_SPAN).clamp(0.0, 1.0) * Self::OUTGROWN
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_settled_regular_has_no_outlook_to_speak_of() {
        assert_eq!(DepartureOutlook::default().likelihood(), 0.0);
        assert_eq!(DepartureOutlook::contract_term(Some(36)), 0.0);
        assert_eq!(
            DepartureOutlook::ageing_term(26, PlayerFieldPositionGroup::Midfielder),
            0.0
        );
        assert_eq!(DepartureOutlook::outgrown_term(150, 148), 0.0);
        assert_eq!(DepartureOutlook::approach_term(0), 0.0);
    }

    #[test]
    fn a_contract_running_down_raises_it_month_by_month() {
        let mut previous = DepartureOutlook::contract_term(Some(24));
        for months in (0..24).rev() {
            let now = DepartureOutlook::contract_term(Some(months));
            assert!(
                now >= previous && now - previous < 0.06,
                "a step at {months} months"
            );
            previous = now;
        }
        assert_eq!(DepartureOutlook::contract_term(Some(0)), 1.0);
        assert_eq!(DepartureOutlook::contract_term(Some(9)), 0.5);
    }

    #[test]
    fn a_career_running_down_raises_it_year_by_year() {
        let group = PlayerFieldPositionGroup::Forward;
        let mut previous = 0.0;
        for age in 25..=34 {
            let now = DepartureOutlook::ageing_term(age, group);
            assert!(
                now >= previous && now - previous <= 1.0 / 6.0 + 1e-6,
                "a step at {age}"
            );
            previous = now;
        }
        assert_eq!(previous, 1.0);
        assert!(
            DepartureOutlook::ageing_term(33, PlayerFieldPositionGroup::Goalkeeper)
                < DepartureOutlook::ageing_term(33, group),
            "a keeper's career runs longer"
        );
    }

    #[test]
    fn every_further_approach_raises_it_and_none_makes_it_certain() {
        let mut previous = 0.0;
        for approaches in 1..=8 {
            let now = DepartureOutlook::approach_term(approaches);
            assert!(now > previous && now < 1.0);
            previous = now;
        }
    }

    #[test]
    fn standing_above_the_side_raises_it_level_by_level() {
        let mut previous = 0.0;
        for level in 150..=200 {
            let now = DepartureOutlook::outgrown_term(level, 150);
            assert!(
                now >= previous && now - previous < 0.03,
                "a step at {level}"
            );
            previous = now;
        }
        assert_eq!(previous, 0.5);
    }

    #[test]
    fn each_reason_adds_to_one_likelihood_and_none_takes_away() {
        let quiet = DepartureOutlook::default();
        let reasons = [
            DepartureOutlook {
                marketed: 0.6,
                ..quiet
            },
            DepartureOutlook {
                asked: 0.5,
                ..quiet
            },
            DepartureOutlook {
                approached: 0.25,
                ..quiet
            },
            DepartureOutlook {
                contract: 0.5,
                ..quiet
            },
            DepartureOutlook {
                ageing: 0.5,
                ..quiet
            },
            DepartureOutlook {
                outgrown: 0.3,
                ..quiet
            },
        ];
        for one in reasons {
            assert!(one.likelihood() > quiet.likelihood());
            let with_contract = DepartureOutlook {
                contract: one.contract.max(0.5),
                ageing: one.ageing.max(0.2),
                ..one
            };
            assert!(with_contract.likelihood() >= one.likelihood());
            assert!(with_contract.likelihood() < 1.0);
        }
    }
}
