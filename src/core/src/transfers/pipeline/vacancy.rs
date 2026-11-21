//! A shirt a departure has left open, and the brief for the man to fill it.

use chrono::NaiveDate;

use crate::club::core::DepartedPlayer;
use crate::club::player::contract::PlayerSquadStatus;
use crate::transfers::pipeline::TransferNeedPriority;
use crate::{ClubPhilosophy, PlayerPositionType, PlayerPreferredFoot};

/// A shirt the first team was filling with a man who has left.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vacancy {
    pub departed: DepartedPlayer,
    /// The share of his fee the club recycled, held for this shirt ahead
    /// of everything else it is shopping for.
    pub reserve: f64,
    /// The recruitment review has read it.
    pub read: bool,
}

/// What the review makes of a vacancy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VacancyOutcome {
    /// A man like him is to be found.
    Search,
    /// A player already at the club holds the shirt at about his level.
    HeldFromWithin,
    /// A signing has taken the shirt.
    Filled,
    /// He was cover: the ordinary review of depth decides what, if
    /// anything, the squad is short of.
    AsksForLittle,
    /// A year on, the shirt is whatever the squad has made of it.
    Lapsed,
}

impl VacancyOutcome {
    pub fn closes(self) -> bool {
        self != VacancyOutcome::Search
    }
}

/// Who holds the vacant shirt now, as the first team's plan reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShirtHolder {
    /// The level the staff perceive him at.
    pub level: u8,
    /// He joined the club after the departure.
    pub signed_since: bool,
}

/// What a vacancy's brief is written from.
#[derive(Debug, Clone, Copy)]
pub struct VacancyReading {
    pub philosophy: ClubPhilosophy,
    /// What the division's standard asks of the shirt.
    pub baseline_level: u8,
    /// Transfer money the club has, the reserve included.
    pub budget: f64,
    /// The club's own window, when one is open.
    pub window: Option<(NaiveDate, NaiveDate)>,
    pub today: NaiveDate,
}

/// The terms of the search a vacancy opens.
#[derive(Debug, Clone, PartialEq)]
pub struct VacancyBrief {
    pub player_id: u32,
    pub position: PlayerPositionType,
    /// The level sought, and the least the search will look at.
    pub level: u8,
    pub min_level: u8,
    pub age_band: (u8, u8),
    pub foot: Option<PlayerPreferredFoot>,
    pub money: f64,
    pub priority: TransferNeedPriority,
    pub urgency: f32,
    pub promised_status: PlayerSquadStatus,
}

impl Vacancy {
    /// Importance below which a departure is the loss of cover.
    const ASKS_FOR_LITTLE: f32 = 0.3;
    /// Perceived levels within which a shirt is held at about his level.
    const ABOUT_HIS_LEVEL: u8 = 8;
    /// Levels the search gives up when it has none of the money a
    /// like-for-like player costs.
    const OUT_OF_REACH: f32 = 14.0;
    /// How far below the level sought a developing club looks: a player
    /// who reaches it within a season or two.
    const GROWS_INTO_IT: u8 = 15;
    /// Days of a window over which a search grows urgent.
    const WINDOW_SPAN: f32 = 60.0;
    /// Share of the budget a vacancy with no fee behind it may command.
    const UNFUNDED_SHARE: f64 = 0.30;
    /// Days after which an unanswered vacancy stops being one.
    const LAPSES_AFTER: i64 = 365;

    /// A vacancy for a man the first team was picking. `recycled` is the
    /// share of his fee the club put back into its transfer money.
    pub fn of(departed: &DepartedPlayer, recycled: f64) -> Option<Self> {
        departed.role.map(|_| Vacancy {
            departed: *departed,
            reserve: recycled.max(0.0),
            read: false,
        })
    }

    pub fn judge(&self, holder: Option<ShirtHolder>, today: NaiveDate) -> VacancyOutcome {
        if (today - self.departed.date).num_days() > Self::LAPSES_AFTER {
            return VacancyOutcome::Lapsed;
        }
        match holder {
            Some(holder) if holder.signed_since => VacancyOutcome::Filled,
            Some(holder)
                if holder.level.saturating_add(Self::ABOUT_HIS_LEVEL) >= self.departed.level =>
            {
                VacancyOutcome::HeldFromWithin
            }
            _ if self.departed.importance() < Self::ASKS_FOR_LITTLE => {
                VacancyOutcome::AsksForLittle
            }
            _ => VacancyOutcome::Search,
        }
    }

    /// The search for a like-for-like man: his shirt, his level as far as
    /// the money reaches and never below the division's standard, the foot
    /// the shirt asks for, in the age band the club's philosophy sets.
    pub fn brief(&self, reading: &VacancyReading) -> VacancyBrief {
        let departed = &self.departed;
        let money = if self.reserve > 0.0 {
            self.reserve.min(reading.budget.max(0.0))
        } else {
            reading.budget.max(0.0) * Self::UNFUNDED_SHARE
        };
        // What the club could put to a like-for-like signing if it chose:
        // everything it has, against what the market paid for him.
        let reach = if departed.fee > 0.0 {
            (reading.budget.max(money) / departed.fee).clamp(0.0, 1.0) as f32
        } else {
            1.0
        };
        let floor = reading.baseline_level.min(departed.level);
        let level = (f32::from(departed.level) - (1.0 - reach) * Self::OUT_OF_REACH)
            .round()
            .max(f32::from(floor)) as u8;

        let (age_band, below) = match reading.philosophy {
            ClubPhilosophy::DevelopAndSell => ((19, 24), Self::GROWS_INTO_IT),
            ClubPhilosophy::LoanFocused => ((20, 27), Self::ABOUT_HIS_LEVEL),
            ClubPhilosophy::Balanced => ((21, 29), Self::ABOUT_HIS_LEVEL),
            ClubPhilosophy::SignToCompete => ((23, 30), Self::ABOUT_HIS_LEVEL),
        };

        let urgency = self.urgency(reading);
        VacancyBrief {
            player_id: departed.player_id,
            position: departed.shirt,
            level,
            min_level: level.saturating_sub(below).max(1),
            age_band,
            foot: Self::foot_for(departed.shirt, departed.foot),
            money,
            priority: if urgency >= 0.6 {
                TransferNeedPriority::Critical
            } else if urgency >= 0.3 {
                TransferNeedPriority::Important
            } else {
                TransferNeedPriority::Optional
            },
            urgency,
            promised_status: if departed.first_choice {
                PlayerSquadStatus::FirstTeamRegular
            } else {
                PlayerSquadStatus::FirstTeamSquadRotation
            },
        }
    }

    /// How pressing the search is, 0..1: his importance, and how little of
    /// the club's own window is left. With no window open the search waits
    /// at half weight for the next one.
    pub fn urgency(&self, reading: &VacancyReading) -> f32 {
        let closeness = match reading.window {
            Some((_, closes)) if closes >= reading.today => {
                let left = (closes - reading.today).num_days() as f32;
                1.0 - (left / Self::WINDOW_SPAN).clamp(0.0, 1.0)
            }
            _ => 0.0,
        };
        (self.departed.importance() * (0.5 + 0.5 * closeness)).clamp(0.0, 1.0)
    }

    /// The foot a shirt on one side of the pitch asks for, when the man
    /// who left had it.
    fn foot_for(shirt: PlayerPositionType, foot: PlayerPreferredFoot) -> Option<PlayerPreferredFoot> {
        use PlayerPositionType::*;
        let side = match shirt {
            DefenderLeft | DefenderCenterLeft | WingbackLeft | MidfielderLeft
            | MidfielderCenterLeft | AttackingMidfielderLeft | ForwardLeft => {
                PlayerPreferredFoot::Left
            }
            DefenderRight | DefenderCenterRight | WingbackRight | MidfielderRight
            | MidfielderCenterRight | AttackingMidfielderRight | ForwardRight => {
                PlayerPreferredFoot::Right
            }
            _ => return None,
        };
        (foot == side).then_some(side)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::club::core::DepartureKind;
    use crate::club::staff::PlannedRole;
    use chrono::Duration;

    struct Gone;

    impl Gone {
        fn day() -> NaiveDate {
            NaiveDate::from_ymd_opt(2026, 8, 4).unwrap()
        }

        /// A first-choice striker of level 160, sold for 80M.
        fn first_choice() -> DepartedPlayer {
            DepartedPlayer {
                player_id: 9,
                shirt: PlayerPositionType::ForwardLeft,
                level: 160,
                ceiling: 165,
                age: 27,
                foot: PlayerPreferredFoot::Left,
                role: Some(PlannedRole::Starter),
                first_choice: true,
                share: 0.8,
                fee: 80_000_000.0,
                date: Self::day(),
                kind: DepartureKind::Sold,
                window_closes: None,
            }
        }

        fn cover() -> DepartedPlayer {
            DepartedPlayer {
                role: Some(PlannedRole::Cover),
                first_choice: false,
                share: 0.05,
                fee: 2_000_000.0,
                ..Self::first_choice()
            }
        }

        fn reading(philosophy: ClubPhilosophy, budget: f64, days_left: Option<i64>) -> VacancyReading {
            VacancyReading {
                philosophy,
                baseline_level: 120,
                budget,
                window: days_left.map(|left| {
                    (Self::day() - Duration::days(40), Self::day() + Duration::days(left))
                }),
                today: Self::day(),
            }
        }

        fn brief(departed: DepartedPlayer, recycled: f64, reading: &VacancyReading) -> VacancyBrief {
            Vacancy::of(&departed, recycled).unwrap().brief(reading)
        }
    }

    #[test]
    fn a_man_the_first_team_was_not_picking_opens_no_vacancy() {
        let away = DepartedPlayer {
            role: None,
            first_choice: false,
            share: 0.0,
            ..Gone::first_choice()
        };
        assert!(Vacancy::of(&away, 0.0).is_none());
    }

    #[test]
    fn an_heir_at_his_level_closes_the_vacancy_and_a_lesser_man_does_not() {
        let vacancy = Vacancy::of(&Gone::first_choice(), 40_000_000.0).unwrap();
        let holder = |level: u8, signed_since: bool| Some(ShirtHolder { level, signed_since });
        assert_eq!(vacancy.judge(holder(154, false), Gone::day()), VacancyOutcome::HeldFromWithin);
        assert_eq!(vacancy.judge(holder(140, false), Gone::day()), VacancyOutcome::Search);
        assert_eq!(vacancy.judge(None, Gone::day()), VacancyOutcome::Search);
        assert_eq!(
            vacancy.judge(None, Gone::day() + Duration::days(400)),
            VacancyOutcome::Lapsed
        );
        assert_eq!(vacancy.judge(holder(140, true), Gone::day()), VacancyOutcome::Filled);
    }

    #[test]
    fn a_squad_player_who_leaves_asks_for_little() {
        let vacancy = Vacancy::of(&Gone::cover(), 1_000_000.0).unwrap();
        assert_eq!(
            vacancy.judge(Some(ShirtHolder { level: 100, signed_since: false }), Gone::day()),
            VacancyOutcome::AsksForLittle
        );
    }

    #[test]
    fn a_star_above_the_divisions_standard_is_replaced_near_his_own_level() {
        let reading = Gone::reading(ClubPhilosophy::SignToCompete, 40_000_000.0, Some(30));
        let brief = Gone::brief(Gone::first_choice(), 40_000_000.0, &reading);
        assert!(brief.level >= 150, "sought at {}, the division asks 120", brief.level);
        assert_eq!(brief.position, PlayerPositionType::ForwardLeft);
    }

    #[test]
    fn a_left_footed_man_on_the_left_is_replaced_by_a_left_footer_for_preference() {
        let reading = Gone::reading(ClubPhilosophy::Balanced, 40_000_000.0, Some(30));
        let left_sided = DepartedPlayer {
            shirt: PlayerPositionType::DefenderCenterLeft,
            ..Gone::first_choice()
        };
        assert_eq!(
            Gone::brief(left_sided, 40_000_000.0, &reading).foot,
            Some(PlayerPreferredFoot::Left)
        );
        let right_footed = DepartedPlayer {
            foot: PlayerPreferredFoot::Right,
            ..left_sided
        };
        assert_eq!(Gone::brief(right_footed, 40_000_000.0, &reading).foot, None);
        let central = DepartedPlayer {
            shirt: PlayerPositionType::DefenderCenter,
            ..Gone::first_choice()
        };
        assert_eq!(Gone::brief(central, 40_000_000.0, &reading).foot, None);
    }

    #[test]
    fn a_developing_club_looks_younger_and_for_a_man_who_grows_into_the_level() {
        let money = 64_000_000.0;
        let compete = Gone::brief(
            Gone::first_choice(),
            money,
            &Gone::reading(ClubPhilosophy::SignToCompete, money, Some(30)),
        );
        let develop = Gone::brief(
            Gone::first_choice(),
            money,
            &Gone::reading(ClubPhilosophy::DevelopAndSell, money, Some(30)),
        );
        assert!(develop.age_band.1 < 27, "younger than the man of 27 who left");
        assert!(develop.age_band.1 < compete.age_band.1);
        assert!(develop.min_level < compete.min_level);
        assert_eq!(develop.level, compete.level, "aimed at the same level in the end");
    }

    #[test]
    fn a_level_out_of_reach_is_lowered_as_far_as_the_money_is_short_and_no_further() {
        let reach = |recycled: f64| {
            Gone::brief(
                Gone::first_choice(),
                recycled,
                &Gone::reading(ClubPhilosophy::Balanced, recycled, Some(30)),
            )
            .level
        };
        assert!(reach(80_000_000.0) > reach(40_000_000.0));
        assert!(reach(40_000_000.0) > reach(8_000_000.0));
        assert!(reach(1.0) >= 120, "never below what the division asks");
    }

    #[test]
    fn a_vacancy_spends_the_half_its_sale_recycled_and_not_a_tiers_fraction() {
        // No other money at all: the budget is the recycled half.
        let brief = Gone::brief(
            Gone::first_choice(),
            40_000_000.0,
            &Gone::reading(ClubPhilosophy::Balanced, 40_000_000.0, Some(30)),
        );
        assert_eq!(brief.money, 40_000_000.0);
    }

    #[test]
    fn a_first_choice_sold_with_two_weeks_left_is_a_critical_search() {
        let late = Gone::brief(
            Gone::first_choice(),
            40_000_000.0,
            &Gone::reading(ClubPhilosophy::Balanced, 40_000_000.0, Some(14)),
        );
        let early = Gone::brief(
            Gone::first_choice(),
            40_000_000.0,
            &Gone::reading(ClubPhilosophy::Balanced, 40_000_000.0, Some(60)),
        );
        assert_eq!(late.priority, TransferNeedPriority::Critical);
        assert!(late.urgency > early.urgency);
        let at = |left: i64| {
            Vacancy::of(&Gone::first_choice(), 0.0)
                .unwrap()
                .urgency(&Gone::reading(ClubPhilosophy::Balanced, 0.0, Some(left)))
        };
        let mut previous = at(0);
        for left in 1..=70 {
            let now = at(left);
            assert!(now <= previous && previous - now < 0.02, "a step at {left} days");
            previous = now;
        }
    }

    #[test]
    fn urgency_follows_how_much_he_mattered() {
        let reading = Gone::reading(ClubPhilosophy::Balanced, 0.0, Some(14));
        let star = Vacancy::of(&Gone::first_choice(), 0.0).unwrap().urgency(&reading);
        let squad = Vacancy::of(&Gone::cover(), 0.0).unwrap().urgency(&reading);
        assert!(star > squad * 4.0, "{star} against {squad}");
    }

    #[test]
    fn a_search_opened_after_the_deadline_waits_at_half_weight_for_the_next_window() {
        let shut = Gone::reading(ClubPhilosophy::Balanced, 40_000_000.0, None);
        let brief = Gone::brief(Gone::first_choice(), 40_000_000.0, &shut);
        assert_eq!(brief.money, 40_000_000.0, "its money is still its own");
        assert!((brief.urgency - Gone::first_choice().importance() * 0.5).abs() < 1e-6);
    }
}
