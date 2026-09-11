//! What a player, a signing and a wage are worth.
//!
//! `PlayerValuationCalculator` below is the price the market puts on a
//! player; [`upgrade`] is what a given signing is worth to a given buyer,
//! and [`wage`] is what that buyer can actually pay him.
//!
//! What a player is worth **to the market**, and what a club asks for him.
//!
//! `PlayerValueCalculator` (in `club::player::calculators`) answers "what is
//! this footballer worth?" from ability, age, contract and league. This
//! module answers the market's version of the same question: what the asking
//! price becomes once the seller's position is visible — a listed player has
//! lost leverage, a player who has asked to leave has lost more, and a club
//! in the red asks under value while a solvent one asks over.
//!
//! It lived at the bottom of `window.rs` — the transfer *calendar* — for no
//! reason but the order things were written in, while seventeen files read
//! it. Price is not a date.

pub mod asking;
pub mod upgrade;
pub mod wage;

pub use upgrade::*;
pub use wage::*;

use crate::shared::{Currency, CurrencyValue};
use crate::{Club, Country, Player, PlayerStatusType, PlayerValueCalculator};
use chrono::NaiveDate;

/// Transfer-market-specific player valuation.
/// Wraps `PlayerValueCalculator` with market conditions (selling pressure, squad role).
pub struct PlayerValuationCalculator;

impl PlayerValuationCalculator {
    /// Premium (solvent club) or discount (club in the red) applied over a
    /// player's computed market value when setting an asking price: a
    /// motivated/distressed seller lists a little under value, a solvent
    /// club asks a little over. Centralizes the constant that previously
    /// lived inline in both the pipeline asking-price helper and the country
    /// listing path. A later pass makes this continuous in debt magnitude
    /// and contract length remaining.
    pub fn seller_distress_multiplier(balance: i64) -> f64 {
        if balance < 0 { 0.9 } else { 1.1 }
    }

    pub fn calculate_value(
        player: &Player,
        date: NaiveDate,
        league_reputation: u16,
        club_reputation: u16,
    ) -> CurrencyValue {
        Self::calculate_value_with_price_level(
            player,
            date,
            1.0,
            league_reputation,
            club_reputation,
        )
    }

    pub fn calculate_value_with_price_level(
        player: &Player,
        date: NaiveDate,
        price_level: f32,
        league_reputation: u16,
        club_reputation: u16,
    ) -> CurrencyValue {
        let base_value = PlayerValueCalculator::calculate(
            player,
            date,
            price_level,
            league_reputation,
            club_reputation,
        );

        // Transfer-listed players face market discount (buyer leverage)
        let mut market_value = base_value;

        if player.statuses.has(PlayerStatusType::Lst) {
            market_value *= 0.9;
        }

        // Players wanting to leave lose negotiating power
        if player.statuses.has(PlayerStatusType::Req) {
            market_value *= 0.85;
        }

        CurrencyValue {
            amount: market_value,
            currency: Currency::Usd,
        }
    }

    /// Resolve (league_reputation, club_market_score) for a club within
    /// its country. Single source of truth for seller-side market
    /// context — avoids each call site re-implementing the same league
    /// lookup or, worse, passing 0/0 and flattening price levels across
    /// every league. Returns (0, 0) only when the club has no main team
    /// or its league isn't registered.
    pub fn seller_context(country: &Country, club: &Club) -> (u16, u16) {
        let main = club.teams.main();
        let club_rep = main.map(|t| t.reputation.market_value_score()).unwrap_or(0);
        let league_rep = main
            .and_then(|t| t.league_id)
            .and_then(|lid| country.leagues.leagues.iter().find(|l| l.id == lid))
            .map(|l| l.reputation)
            .unwrap_or(0);
        (league_rep, club_rep)
    }

    /// Variant for callers that don't carry a `Country` reference (board
    /// audits, AI transfer-listing AI). League reputation is approximated
    /// from the club's blended score since the two correlate strongly
    /// (top-rep clubs play in top-rep leagues), keeping market values
    /// roughly correct without forcing every caller to plumb the country
    /// down.
    pub fn seller_context_from_club(club: &Club) -> (u16, u16) {
        let club_rep = club
            .teams
            .main()
            .map(|t| t.reputation.market_value_score())
            .unwrap_or(0);
        (club_rep, club_rep)
    }
}
