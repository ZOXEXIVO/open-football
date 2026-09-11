//! What the seller puts on the board.
//!
//! The club's own ledger price wins when its monthly pass has reached the
//! player — that number knows three things a valuation cannot: what he is to
//! THIS side, how long the club still controls him, and where he sits on his
//! own career arc. Failing that, the market value read through the seller's
//! league and club standing, because a Serie A club asking the same fee as a
//! Maltese side for an identical player is an obvious flatness bug.

use chrono::NaiveDate;

use crate::shared::{Currency, CurrencyValue};
use crate::transfers::value::PlayerValuationCalculator;
use crate::utils::FormattingUtils;
use crate::{Club, Country, Player};

/// The seller's ask.
pub(in crate::transfers) struct AskingPrice;

impl AskingPrice {
    pub(in crate::transfers) fn calculate_asking_price(
        player: &Player,
        country: &Country,
        club: &Club,
        date: NaiveDate,
        price_level: f32,
    ) -> CurrencyValue {
        // The club's own ledger price, when its monthly pass has reached
        // him. That number knows three things this function cannot: what he
        // is to THIS side (a core player costs twice what his market value
        // says, because the club does not want to sell him), how long the
        // club still controls him, and where he sits on his own career arc.
        // See [`AssetLedger::asking_for`]. `SellerFeeFloor` is still the
        // absolute floor underneath whatever comes out.
        if let Some(asking) = club.transfer_plan.asking_for(player.id) {
            if asking > 0.0 {
                return CurrencyValue {
                    amount: FormattingUtils::round_fee(asking),
                    currency: Currency::Usd,
                };
            }
        }
        // Selling clubs anchor on their own market context — a Serie A
        // club asking the same fee as a Maltese side for an identical
        // player is an obvious flatness bug. Pull the seller's blended
        // league + club reputation so the base value reflects who is
        // actually selling.
        let (league_rep, club_rep) = PlayerValuationCalculator::seller_context(country, club);
        let base_value = PlayerValuationCalculator::calculate_value_with_price_level(
            player,
            date,
            price_level,
            league_rep,
            club_rep,
        );

        let multiplier =
            PlayerValuationCalculator::seller_distress_multiplier(club.finance.balance.balance);

        CurrencyValue {
            amount: FormattingUtils::round_fee(base_value.amount * multiplier),
            currency: base_value.currency,
        }
    }
}
