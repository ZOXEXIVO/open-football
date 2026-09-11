//! The board telling the manager that somebody has to go.
//!
//! `DemandPlayerSale` used to record a headline and stop. Nobody was
//! listed, no price was set, no deadline existed, and nothing anywhere
//! noticed whether the money ever came in — a board could demand a sale
//! every month of a financial crisis and the squad would not lose a single
//! player.
//!
//! A real demand names somebody. It is the highest earner the club can
//! stand to lose rather than the best player it has: a board short of money
//! is looking at the wage bill, not at the team sheet.

use chrono::NaiveDate;
use log::debug;

use crate::Player;
use crate::club::board::decision::DecisionReason;
use crate::club::{Club, PlayerSquadStatus, PlayerStatusType};
use crate::transfers::pipeline::trace::TransferTrace;
use crate::utils::DateUtils;

/// Who goes when the board says somebody must.
pub struct ForcedSale;

/// The player the board picked, and what it expects for him.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ForcedSaleTarget {
    pub player_id: u32,
    /// What the club is asking. The mandate is kept when this much comes
    /// in before the deadline.
    pub asking_price: f64,
}

impl ForcedSale {
    /// Age at or above which a player is squarely a sale candidate whatever
    /// his standing — a club raising money looks first at the end of the
    /// squad it will have to replace soonest anyway.
    pub const MIN_AGE: u8 = 27;

    /// Haircut on market value when regulatory trouble means the club is a
    /// forced seller and everybody knows it.
    pub const DISCOUNT_FFP: f64 = 0.85;

    /// The gentler haircut when the board is only trimming the wage bill.
    pub const DISCOUNT_WAGE: f64 = 0.95;

    /// Days the manager has to turn the demand into money.
    pub const MANDATE_DAYS: i64 = 90;

    /// Pick somebody and put him on the market.
    ///
    /// Returns the target when a listing actually happened, so the caller
    /// can open the promise and file the story against a real name.
    pub fn execute(
        club: &mut Club,
        reason: DecisionReason,
        date: NaiveDate,
        league_reputation: u16,
        club_reputation: u16,
    ) -> Option<ForcedSaleTarget> {
        let player_id = Self::choose(club, date)?;

        let discount = match reason {
            DecisionReason::FfpPressure => Self::DISCOUNT_FFP,
            _ => Self::DISCOUNT_WAGE,
        };

        let main = club.teams.main_mut()?;
        let asking_price = {
            let player = main.players.find(player_id)?;
            player.value(date, league_reputation, club_reputation) * discount
        };
        let player = main.players.find_mut(player_id)?;

        player.statuses.add(date, PlayerStatusType::Lst);
        // The badge is the visible half; the contract flag is the durable
        // one. Without it the next listing sweep strips the badge, the
        // renewal pass sees a clean player and re-signs him, and the board's
        // demand quietly evaporates.
        if let Some(contract) = player.contract.as_mut() {
            contract.is_transfer_listed = true;
        }
        player.decision_history.add(
            date,
            "dec_board_forced_sale".to_string(),
            match reason {
                DecisionReason::FfpPressure => "dec_reason_ffp_pressure".to_string(),
                _ => "dec_reason_wage_control".to_string(),
            },
            "dec_decided_board".to_string(),
        );
        TransferTrace::list(player, date, "board_forced_sale", "board_demanded_sale");

        debug!(
            "Board demanded a sale: {} listed at {:.0}",
            player.full_name, asking_price
        );

        Some(ForcedSaleTarget {
            player_id,
            asking_price,
        })
    }

    /// The man the board would rather lose.
    ///
    /// Highest wage first, because the bill is the problem. Ties broken by
    /// the lowest ability, because between two equally expensive players
    /// the club keeps the better one. Key players and the injured are off
    /// the table — one because selling him is a different decision the
    /// board has not taken, the other because nobody buys him this window.
    fn choose(club: &Club, date: NaiveDate) -> Option<u32> {
        let main = club.teams.main()?;
        main.players
            .players()
            .iter()
            .filter(|player| {
                let player: &Player = player;
                let contract = match player.contract.as_ref() {
                    Some(contract) => contract,
                    None => return false,
                };
                if contract.is_transfer_listed || contract.loan_from_club_id.is_some() {
                    return false;
                }
                if player.player_attributes.is_injured {
                    return false;
                }
                if matches!(contract.squad_status, PlayerSquadStatus::KeyPlayer) {
                    return false;
                }
                // Either he is at the age the club was going to replace
                // anyway, or he is already outside the first-choice group.
                DateUtils::age(player.birth_date, date) >= Self::MIN_AGE
                    || !matches!(contract.squad_status, PlayerSquadStatus::FirstTeamRegular)
            })
            .max_by(|a, b| {
                let wage = |p: &Player| p.contract.as_ref().map(|c| c.salary).unwrap_or(0);
                wage(a).cmp(&wage(b)).then_with(|| {
                    b.player_attributes
                        .current_ability
                        .cmp(&a.player_attributes.current_ability)
                })
            })
            .map(|player| player.id)
    }
}
