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
    pub player_name: String,
    pub from_club_id: u32,
    pub from_team_name: String,
    pub to_club_id: u32,
    pub to_team_name: String,
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
        player_name: String,
        from_club_id: u32,
        from_team_name: String,
        to_club_id: u32,
        to_team_name: String,
        transfer_date: NaiveDate,
        fee: CurrencyValue,
        transfer_type: TransferType,
    ) -> Self {
        let season_year = if transfer_date.month() >= 8 {
            transfer_date.year() as u16
        } else {
            (transfer_date.year() - 1) as u16
        };

        CompletedTransfer {
            player_id,
            player_name,
            from_club_id,
            from_team_name,
            to_club_id,
            to_team_name,
            transfer_date,
            fee,
            transfer_type,
            season_year,
        }
    }
}