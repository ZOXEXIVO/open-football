use crate::Player;
use crate::league::Season;
use crate::shared::CurrencyValue;
use crate::transfers::reason::TransferReason;
use chrono::NaiveDate;

pub struct PlayerTransfer {
    pub player: Player,
    pub club_id: u32,
}

impl PlayerTransfer {
    pub fn new(player: Player, club_id: u32) -> Self {
        PlayerTransfer { player, club_id }
    }
}

#[derive(Debug, Clone)]
pub struct CompletedTransfer {
    pub player_id: u32,
    pub player_name: String,
    pub from_club_id: u32,
    pub from_team_id: u32,
    pub from_team_name: String,
    pub to_club_id: u32,
    pub to_team_name: String,
    pub transfer_date: NaiveDate,
    pub fee: CurrencyValue,
    pub transfer_type: TransferType,
    pub season_year: u16,
    pub reason: TransferReason,
    /// Country the player moved OUT of, `0` when genuinely unknown.
    ///
    /// `from_club_id` cannot answer this for a free signing: the pool has no
    /// club, so the row is written with `from_club_id: 0` and every reader
    /// that resolves the origin through the club index drops it. That is one
    /// row in a UI list and the whole free-agent half of the transfer
    /// GEOGRAPHY in the corridor census, which reported "0 of 0 cross-
    /// continent free signings" because it could not see a single one.
    ///
    /// Set to the seller's country for a transfer or loan, the player's last
    /// league for a pool signing, and the club's own country for a domestic
    /// expiry. Readers should still fall back to the club index when it is
    /// `0` — history written before this field existed carries no origin.
    pub origin_country_id: u32,
}

#[derive(Debug, Clone)]
pub enum TransferType {
    Permanent,
    Loan(NaiveDate), // End date
    Free,
}

impl CompletedTransfer {
    pub fn new(
        player_id: u32,
        player_name: String,
        from_club_id: u32,
        from_team_id: u32,
        from_team_name: String,
        to_club_id: u32,
        to_team_name: String,
        transfer_date: NaiveDate,
        fee: CurrencyValue,
        transfer_type: TransferType,
    ) -> Self {
        let season_year = Season::from_date(transfer_date).start_year;

        CompletedTransfer {
            player_id,
            player_name,
            from_club_id,
            from_team_id,
            from_team_name,
            to_club_id,
            to_team_name,
            transfer_date,
            fee,
            transfer_type,
            season_year,
            reason: TransferReason::default(),
            origin_country_id: 0,
        }
    }

    pub fn with_reason(mut self, reason: TransferReason) -> Self {
        self.reason = reason;
        self
    }

    /// Stamp the country the player left. See [`Self::origin_country_id`].
    pub fn with_origin_country(mut self, country_id: u32) -> Self {
        self.origin_country_id = country_id;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::Currency;

    fn row(from_club_id: u32) -> CompletedTransfer {
        CompletedTransfer::new(
            7,
            "Player".to_string(),
            from_club_id,
            0,
            "From".to_string(),
            9,
            "To".to_string(),
            NaiveDate::from_ymd_opt(2026, 8, 1).unwrap(),
            CurrencyValue::new(0.0, Currency::Usd),
            TransferType::Free,
        )
    }

    /// A free signing is written with `from_club_id: 0` — the pool has no
    /// club — so a reader that resolves the origin through the club index
    /// alone drops the row. The origin has to travel on the row itself.
    #[test]
    fn a_row_carries_the_country_the_player_left() {
        let stamped = row(0).with_origin_country(42);
        assert_eq!(stamped.origin_country_id, 42);
        assert_eq!(stamped.from_club_id, 0);
    }

    /// Unstamped rows read `0`, which is what tells a reader to fall back
    /// to the club index — history written before the field existed still
    /// resolves.
    #[test]
    fn an_unstamped_row_reads_zero_so_readers_fall_back() {
        assert_eq!(row(11).origin_country_id, 0);
    }

    #[test]
    fn the_reason_and_the_origin_do_not_overwrite_each_other() {
        let both = row(0)
            .with_reason(TransferReason::key("free_agent_market_clearing"))
            .with_origin_country(5);
        assert_eq!(both.origin_country_id, 5);
        assert_eq!(both.reason.key, "free_agent_market_clearing");
    }
}
