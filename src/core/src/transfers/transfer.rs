use crate::shared::CurrencyValue;
use crate::Player;
use chrono::{Datelike, NaiveDate};

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
    pub from_club_id: u32,
    pub to_club_id: u32,
    pub transfer_date: NaiveDate,
    pub fee: CurrencyValue,
    pub transfer_type: TransferType,
    pub season_year: u16,
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
        from_club_id: u32,
        to_club_id: u32,
        transfer_date: NaiveDate,
        fee: CurrencyValue,
        transfer_type: TransferType,
    ) -> Self {
        // Determine the season year based on when transfer happened
        // Typically football seasons span Aug-May, so use that as reference
        let season_year = if transfer_date.month() >= 8 {
            transfer_date.year() as u16
        } else {
            (transfer_date.year() - 1) as u16
        };

        CompletedTransfer {
            player_id,
            from_club_id,
            to_club_id,
            transfer_date,
            fee,
            transfer_type,
            season_year,
        }
    }
}