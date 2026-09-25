//! A listing got through a window unsold.
//!
//! The country market states the fact — which of the club's listings the
//! market has now refused through a whole window, and what it saw of each.
//! The club asks its board what to do about every one, lets the player
//! answer for himself, and carries out the club's half of the verdict. The
//! market's half — the new price, or a retired row — goes back to the
//! caller, which owns the market.

use chrono::NaiveDate;
use log::debug;

use crate::club::board::mandate::{MandateExit, SigningMandate};
use crate::club::board::{ClubBoard, StrandedCase, StrandedRoute};
use crate::club::player::calculators::FreeAgentReleaseReason;
use crate::club::player::transfer::SettlementOutlook;
use crate::transfers::pipeline::LoanOutReason;
use crate::transfers::squad::ledger::{LedgerContext, LedgerPressure};
use crate::{Club, Person};

/// One of the club's listings, as the market saw it through the window.
#[derive(Debug, Clone, Copy)]
pub struct StrandedListing {
    pub player_id: u32,
    pub exposure_days: u16,
    pub current_ask: f64,
    pub best_rejected_bid: Option<f64>,
}

/// The market the club sells into — the level its players read their next
/// move against.
#[derive(Debug, Clone, Copy)]
pub struct StrandedMarket {
    pub league_reputation: u16,
    pub club_reputation: u16,
}

/// What the market has to change after the club has decided.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StrandedEffect {
    /// He stays on the list, from a new price the board will stand by.
    Reanchor {
        player_id: u32,
        anchor: f64,
        board_floor: f64,
    },
    /// He has gone: every row and every club's interest in him goes too.
    Retire { player_id: u32 },
}

impl Club {
    pub fn on_listings_stranded(
        &mut self,
        listings: &[StrandedListing],
        market: &StrandedMarket,
        date: NaiveDate,
    ) -> Vec<StrandedEffect> {
        let ledger = LedgerContext::of(self);
        let pressure = LedgerPressure::of(&ledger);
        let average_wage =
            ledger.wage_budget.max(ledger.annual_wages) / ledger.squad_size.max(1) as f64;
        let annual_wage_bill = ledger.annual_wages as u32;
        let outlook = SettlementOutlook {
            league_reputation: market.league_reputation,
            club_reputation: market.club_reputation,
        };

        let mut effects = Vec::with_capacity(listings.len());
        let mut released: Vec<(u32, SigningMandate, f32)> = Vec::new();
        for listing in listings {
            let Some(team) = self
                .teams
                .teams
                .iter_mut()
                .find(|t| t.players.players.iter().any(|p| p.id == listing.player_id))
            else {
                continue;
            };
            let Some(player) = team
                .players
                .players
                .iter_mut()
                .find(|p| p.id == listing.player_id)
            else {
                continue;
            };
            if player.is_on_loan() {
                continue;
            }
            let Some(contract) = player.contract.as_ref() else {
                continue;
            };
            let contract_days_left = (contract.expiration - date).num_days();
            let case = StrandedCase {
                exposure_days: listing.exposure_days,
                annual_wage: contract.salary as f64,
                average_wage,
                annual_wage_bill,
                contract_days_left,
                current_ask: listing.current_ask,
                best_rejected_bid: listing.best_rejected_bid,
                book_value: player.book_value(date),
                has_loan_runway: ClubBoard::has_loan_runway(
                    Some((contract_days_left / 30) as i32),
                    player.age(date),
                    player.plan.as_ref().map_or(0, |p| p.loans_used),
                ),
                settlement: player.settlement_ask(date, &outlook),
            };
            let verdict = self.board.review_stranded(&case, &pressure);

            match verdict.route {
                StrandedRoute::Settle { amount } => {
                    debug!(
                        "stranded settlement: player {} leaves club {} for {:.0} of {:.0} owed \
                         (asked {:.0}, resolve {:.2})",
                        player.id,
                        self.id,
                        amount,
                        case.settlement.carry,
                        case.settlement.ask,
                        verdict.resolve.value(),
                    );
                    if let Some(mandate) = player.mandate().filter(|m| m.is_purchase()) {
                        released.push((player.id, *mandate, player.delivered_minutes().0));
                    }
                    player.on_contract_terminated(date, FreeAgentReleaseReason::MutualTermination);
                    team.transfer_list.remove(listing.player_id);
                    self.finance
                        .balance
                        .push_expense_player_wages(amount.round() as i64);
                    effects.push(StrandedEffect::Retire {
                        player_id: listing.player_id,
                    });
                    continue;
                }
                StrandedRoute::LoanOut { subsidy } => {
                    self.on_stranded_loan_staged(listing.player_id, subsidy, date);
                }
                StrandedRoute::KeepSelling => {}
            }
            effects.push(StrandedEffect::Reanchor {
                player_id: listing.player_id,
                anchor: verdict.anchor,
                board_floor: verdict.board_floor,
            });
        }

        for (player_id, mandate, delivered) in released {
            self.on_mandate_closed(player_id, mandate, delivered, MandateExit::Released, date);
        }
        effects
    }

    /// Staged as any other loan the club wants, with the share of his wage
    /// the board has agreed to keep paying.
    fn on_stranded_loan_staged(&mut self, player_id: u32, subsidy: f32, date: NaiveDate) {
        self.on_pathway_loan_staged(player_id, LoanOutReason::FinancialRelief, date);
        if let Some(plan) = self
            .teams
            .teams
            .iter_mut()
            .find_map(|t| t.players.find_mut(player_id))
            .and_then(|p| p.plan.as_mut())
        {
            plan.loan_subsidy = Some(subsidy);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::academy::ClubAcademy;
    use crate::club::board::mandate::{MandateAuthor, MandatePurpose};
    use crate::club::player::core::builder::PlayerBuilder;
    use crate::shared::Location;
    use crate::shared::fullname::FullName;
    use crate::{
        ClubColors, ClubFacilities, ClubFinances, ClubStatus, PersonAttributes, Player,
        PlayerAttributes, PlayerClubContract, PlayerCollection, PlayerFieldPositionGroup,
        PlayerPlan, PlayerPosition, PlayerPositionType, PlayerPositions, PlayerSkills,
        StaffCollection, TeamBuilder, TeamCollection, TeamReputation, TeamType, TrainingSchedule,
    };
    use crate::{PathwayStage, PlayerStatusType};
    use chrono::{Datelike, Duration, NaiveTime};

    struct Fx;

    impl Fx {
        const PLAYER: u32 = 7;
        const MARKET: StrandedMarket = StrandedMarket {
            league_reputation: 6_000,
            club_reputation: 5_000,
        };

        fn date() -> NaiveDate {
            NaiveDate::from_ymd_opt(2027, 8, 31).unwrap()
        }

        /// A 28-year-old on roughly his market wage, two years left, a
        /// season on the bench with a transfer request in — and bought
        /// five years ago, so the fee is off the books.
        fn frozen_out() -> Player {
            let date = Self::date();
            let mut player = PlayerBuilder::new()
                .id(Self::PLAYER)
                .full_name(FullName::new("Test".to_string(), "Player".to_string()))
                .birth_date(NaiveDate::from_ymd_opt(date.year() - 28, 1, 1).unwrap())
                .country_id(1)
                .attributes(PersonAttributes::default())
                .skills(PlayerSkills::default())
                .positions(PlayerPositions {
                    positions: vec![PlayerPosition {
                        position: PlayerPositionType::MidfielderCenter,
                        level: 20,
                    }],
                })
                .player_attributes(PlayerAttributes {
                    current_ability: 100,
                    potential_ability: 100,
                    ..Default::default()
                })
                .contract(Some(PlayerClubContract::new(
                    700_000,
                    date + Duration::days(730),
                )))
                .build()
                .unwrap();
            player.happiness.starter_ratio = 0.0;
            player.happiness.appearances_tracked = 40;
            player
                .statuses
                .add(date - Duration::days(300), PlayerStatusType::Req);
            let bought = date - Duration::days(5 * 365);
            player.plan = Some(PlayerPlan::from_mandate(
                SigningMandate::new(
                    MandatePurpose::Rotation,
                    PlayerFieldPositionGroup::Midfielder,
                    23,
                    bought,
                    MandateAuthor::Manager,
                )
                .with_money(2_000_000.0, 700_000.0),
                bought,
            ));
            player
        }

        /// Twenty teammates on the same wage, so the club's wage bill and
        /// the lump it can pay are a real club's, not one man's.
        fn squad(stranded: Player) -> Vec<Player> {
            let mut squad: Vec<Player> = (100..120)
                .map(|id| {
                    let mut teammate = Self::frozen_out();
                    teammate.id = id;
                    teammate.plan = None;
                    teammate.statuses.remove(PlayerStatusType::Req);
                    teammate.happiness.starter_ratio = 1.0;
                    teammate
                })
                .collect();
            squad.push(stranded);
            squad
        }

        fn club(stranded: Player) -> Club {
            let team = TeamBuilder::new()
                .id(10)
                .league_id(Some(1))
                .club_id(100)
                .name("Main".to_string())
                .slug("main".to_string())
                .team_type(TeamType::Main)
                .players(PlayerCollection::new(Self::squad(stranded)))
                .staffs(StaffCollection::new(Vec::new()))
                .reputation(TeamReputation::new(500, 500, 500))
                .training_schedule(TrainingSchedule::new(
                    NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                    NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
                ))
                .build()
                .unwrap();
            Club::new(
                100,
                "Club".to_string(),
                Location::new(1),
                ClubFinances::new(10_000_000, Vec::new()),
                ClubAcademy::new(3),
                ClubStatus::Professional,
                ClubColors::default(),
                TeamCollection::new(vec![team]),
                ClubFacilities::default(),
            )
        }

        fn listing(exposure_days: u16) -> StrandedListing {
            StrandedListing {
                player_id: Self::PLAYER,
                exposure_days,
                current_ask: 1_000_000.0,
                best_rejected_bid: None,
            }
        }
    }

    #[test]
    fn a_settled_player_is_released_and_the_club_pays_what_was_agreed() {
        let mut club = Fx::club(Fx::frozen_out());
        let before = club.finance.balance.expense_player_wages;

        let effects = club.on_listings_stranded(&[Fx::listing(330)], &Fx::MARKET, Fx::date());

        assert_eq!(
            effects,
            vec![StrandedEffect::Retire {
                player_id: Fx::PLAYER
            }]
        );
        let player = club.teams.teams[0].players.find(Fx::PLAYER).unwrap();
        assert!(player.contract.is_none());
        assert_eq!(
            player.release_reason(),
            Some(FreeAgentReleaseReason::MutualTermination)
        );
        let paid = club.finance.balance.expense_player_wages - before;
        assert!(paid > 0 && (paid as f64) < 1_400_000.0, "paid {paid}");
        let outcome = club.board.mandate_ledger.rows().last().unwrap();
        assert_eq!(outcome.player_id, Fx::PLAYER);
        assert!(matches!(outcome.exit, MandateExit::Released));
    }

    #[test]
    fn a_man_the_market_has_barely_seen_stays_listed() {
        let mut club = Fx::club(Fx::frozen_out());

        let effects = club.on_listings_stranded(&[Fx::listing(3)], &Fx::MARKET, Fx::date());

        match effects.as_slice() {
            [
                StrandedEffect::Reanchor {
                    anchor,
                    board_floor,
                    ..
                },
            ] => {
                assert!(*anchor > 950_000.0 && *board_floor <= *anchor);
            }
            other => panic!("expected a re-price, got {other:?}"),
        }
        let player = club.teams.teams[0].players.find(Fx::PLAYER).unwrap();
        assert!(player.contract.is_some());
    }

    #[test]
    fn a_loan_carries_the_boards_subsidy() {
        let mut player = Fx::frozen_out();
        player.statuses.remove(PlayerStatusType::Req);
        player.happiness.starter_ratio = 1.0;
        player.contract.as_mut().unwrap().salary = 3_000_000;
        let mut club = Fx::club(player);

        club.on_listings_stranded(&[Fx::listing(80)], &Fx::MARKET, Fx::date());

        let player = club.teams.teams[0].players.find(Fx::PLAYER).unwrap();
        let plan = player.plan.as_ref().unwrap();
        assert_eq!(plan.stage, PathwayStage::LoanOut);
        assert_eq!(plan.loan_purpose, Some(LoanOutReason::FinancialRelief));
        assert!(plan.loan_subsidy.is_some_and(|s| s > 0.0));
        assert!(
            club.transfer_plan
                .loan_out_candidates
                .iter()
                .any(|c| c.player_id == Fx::PLAYER)
        );
    }
}
