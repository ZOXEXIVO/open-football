//! What a contracted player will take to walk away from his deal.
//!
//! The club can offer to tear a contract up; only the player can agree.
//! He weighs the wages he would give up against what the market pays a
//! man of his level once he is free, and against how badly he wants to
//! play football again. The club never reads his mood or his market —
//! it gets his number.

use chrono::NaiveDate;

use crate::club::player::calculators::WageCalculator;
use crate::club::player::player::Player;
use crate::country::result::transfers::free::pricing::FreeAgentMarketCalculator;
use crate::{Person, PlayerStatusType};

/// The reputations the player reads his next market against: his own
/// club's, which is where his sights start before they fall.
#[derive(Debug, Clone, Copy)]
pub struct SettlementOutlook {
    pub league_reputation: u16,
    pub club_reputation: u16,
}

/// The least he will take to leave, and the wages left on the contract he
/// would be leaving.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SettlementAsk {
    pub ask: f64,
    pub carry: f64,
}

/// How much of the money he is owed a player gives up to get back to
/// football, 0..`MAX`.
pub struct SettlementAppetite;

impl SettlementAppetite {
    const DROUGHT: f32 = 0.25;
    const AMBITION: f32 = 0.15;
    const WANTS_OUT: f32 = 0.15;
    /// From this age the guaranteed money starts to matter more than the
    /// next season on the pitch.
    const AGE_FROM: f32 = 29.0;
    const AGE_PER_YEAR: f32 = 0.02;
    const MAX: f32 = 0.5;
    /// Share of his own club and league a fully resigned man still reads
    /// his next market at — a step down, not the bottom of the pyramid.
    const SIGHTS_FLOOR: f32 = 0.5;

    pub fn of(drought: f32, ambition: f32, wants_out: bool, age: u8) -> f32 {
        let wants_out = if wants_out { Self::WANTS_OUT } else { 0.0 };
        let age_drag = (age as f32 - Self::AGE_FROM).max(0.0) * Self::AGE_PER_YEAR;
        (Self::DROUGHT * drought.clamp(0.0, 1.0)
            + Self::AMBITION * (ambition / 20.0).clamp(0.0, 1.0)
            + wants_out
            - age_drag)
            .clamp(0.0, Self::MAX)
    }

    /// The fraction of his current standing he expects the next club to
    /// have, from how far the market has already worn his sights down.
    pub fn sights(resignation: f32) -> f32 {
        Self::SIGHTS_FLOOR + (1.0 - Self::SIGHTS_FLOOR) * (1.0 - resignation.clamp(0.0, 1.0))
    }
}

impl Player {
    /// The least settlement he accepts to end his contract today.
    pub fn settlement_ask(&self, date: NaiveDate, outlook: &SettlementOutlook) -> SettlementAsk {
        let Some(contract) = self.contract.as_ref() else {
            return SettlementAsk {
                ask: 0.0,
                carry: 0.0,
            };
        };
        let days_left = (contract.expiration - date).num_days().max(0) as f64;
        let carry = contract.salary as f64 * days_left / 365.0;

        let age = self.age(date);
        let sights = SettlementAppetite::sights(self.market_resignation(date));
        let wage_out = WageCalculator::expected_annual_wage(
            self,
            age,
            outlook.club_reputation as f32 / 10_000.0 * sights,
            (outlook.league_reputation as f32 * sights) as u16,
        ) as f64;
        let gap_days = 100.0
            / FreeAgentMarketCalculator::daily_signing_chance(
                0.0,
                self.player_attributes.current_ability,
                0.0,
            ) as f64;
        let earn_out = wage_out * (days_left - gap_days).max(0.0) / 365.0;

        let wants_out =
            self.statuses.has(PlayerStatusType::Req) || self.statuses.has(PlayerStatusType::Unh);
        let appetite = SettlementAppetite::of(
            self.football_drought(date),
            self.attributes.ambition,
            wants_out,
            age,
        ) as f64;

        SettlementAsk {
            ask: (carry - earn_out - carry * appetite).max(0.0),
            carry,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::club::player::builder::PlayerBuilder;
    use crate::shared::fullname::FullName;
    use crate::{
        PersonAttributes, PlayerAttributes, PlayerClubContract, PlayerPosition, PlayerPositionType,
        PlayerPositions, PlayerSkills,
    };
    use chrono::{Datelike, Duration};

    struct Fx;

    impl Fx {
        const OUTLOOK: SettlementOutlook = SettlementOutlook {
            league_reputation: 6_000,
            club_reputation: 5_000,
        };

        fn today() -> NaiveDate {
            NaiveDate::from_ymd_opt(2027, 1, 31).unwrap()
        }

        fn person(ambition: f32) -> PersonAttributes {
            PersonAttributes {
                adaptability: 10.0,
                ambition,
                controversy: 10.0,
                loyalty: 10.0,
                pressure: 10.0,
                professionalism: 10.0,
                sportsmanship: 10.0,
                temperament: 10.0,
                consistency: 10.0,
                important_matches: 10.0,
                dirtiness: 10.0,
            }
        }

        /// A contracted midfielder whose wage is `wage_multiple` times what
        /// his ability commands at his own club, with `years` left to run.
        fn player(age: i32, ambition: f32, wage_multiple: f64, years: i64) -> Player {
            let today = Self::today();
            let mut player = PlayerBuilder::new()
                .id(1)
                .full_name(FullName::new("Test".to_string(), "Player".to_string()))
                .birth_date(NaiveDate::from_ymd_opt(today.year() - age, 1, 1).unwrap())
                .country_id(1)
                .attributes(Self::person(ambition))
                .skills(PlayerSkills::default())
                .positions(PlayerPositions {
                    positions: vec![PlayerPosition {
                        position: PlayerPositionType::MidfielderCenter,
                        level: 20,
                    }],
                })
                .player_attributes(PlayerAttributes {
                    current_ability: 120,
                    potential_ability: 125,
                    ..Default::default()
                })
                .build()
                .unwrap();
            let market = WageCalculator::expected_annual_wage(
                &player,
                age as u8,
                Self::OUTLOOK.club_reputation as f32 / 10_000.0,
                Self::OUTLOOK.league_reputation,
            ) as f64;
            player.contract = Some(PlayerClubContract::new(
                (market * wage_multiple) as u32,
                today + Duration::days(365 * years),
            ));
            player
        }

        /// A season sat on the bench with a transfer request in.
        fn wants_out(mut player: Player) -> Player {
            player.happiness.starter_ratio = 0.0;
            player.happiness.appearances_tracked = 40;
            player
                .statuses
                .add(Self::today() - Duration::days(200), PlayerStatusType::Req);
            player
        }

        fn content(mut player: Player) -> Player {
            player.happiness.starter_ratio = 1.0;
            player.happiness.appearances_tracked = 40;
            player
        }
    }

    #[test]
    fn a_starved_player_with_a_market_asks_for_little() {
        let player = Fx::wants_out(Fx::player(25, 15.0, 1.0, 3));
        let ask = player.settlement_ask(Fx::today(), &Fx::OUTLOOK);
        assert!(
            ask.ask < ask.carry * 0.15,
            "asked {:.0} of {:.0} remaining",
            ask.ask,
            ask.carry
        );
    }

    #[test]
    fn a_veteran_without_a_market_holds_out_for_his_money() {
        let player = Fx::content(Fx::player(33, 4.0, 10.0, 2));
        let ask = player.settlement_ask(Fx::today(), &Fx::OUTLOOK);
        assert!(
            ask.ask > ask.carry * 0.85,
            "asked {:.0} of {:.0} remaining",
            ask.ask,
            ask.carry
        );
    }

    #[test]
    fn wanting_to_play_lowers_the_price_of_leaving() {
        let restless = Fx::wants_out(Fx::player(26, 10.0, 2.0, 2));
        let settled = Fx::content(Fx::player(26, 10.0, 2.0, 2));
        let restless = restless.settlement_ask(Fx::today(), &Fx::OUTLOOK);
        let settled = settled.settlement_ask(Fx::today(), &Fx::OUTLOOK);
        assert_eq!(restless.carry, settled.carry);
        assert!(
            restless.ask < settled.ask,
            "restless {:.0} vs settled {:.0}",
            restless.ask,
            settled.ask
        );
    }

    #[test]
    fn the_ask_is_never_negative_and_never_above_the_carry() {
        for (age, multiple) in [(22, 0.3), (27, 1.0), (31, 4.0)] {
            let player = Fx::wants_out(Fx::player(age, 20.0, multiple, 4));
            let ask = player.settlement_ask(Fx::today(), &Fx::OUTLOOK);
            assert!(ask.ask >= 0.0 && ask.ask <= ask.carry);
        }
    }
}
