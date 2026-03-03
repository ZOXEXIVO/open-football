use chrono::NaiveDate;
use crate::club::player::player::Player;
use crate::league::Season;
use crate::{ContractType, PlayerStatisticsHistoryItem};

impl Player {
    /// Snapshot current season statistics into history and reset for new season
    pub fn snapshot_season_statistics(
        &mut self,
        season: Season,
        team_name: &str,
        team_slug: &str,
        team_reputation: u16,
        league_name: &str,
        league_slug: &str,
        date: NaiveDate,
    ) {
        let is_loan = self.contract.as_ref()
            .map(|c| c.contract_type == ContractType::Loan)
            .unwrap_or(false);

        let old_stats = std::mem::take(&mut self.statistics);
        self.friendly_statistics = Default::default();

        // If the player transferred to this club AFTER the season being snapshotted
        // started, any accumulated stats belong to the transfer's season entry —
        // not the old season. Merge into the existing transfer placeholder instead
        // of creating a phantom entry (e.g. "2032/33 Inter" when transfer was Aug 2033).
        if let Some(transfer_date) = self.last_transfer_date {
            let transfer_season = Season::from_date(transfer_date);
            if transfer_season.start_year > season.start_year {
                // Find the transfer placeholder and merge stats into it
                if let Some(placeholder) = self.statistics_history.items.iter_mut().find(|e| {
                    e.season.start_year == transfer_season.start_year
                        && e.team_slug == team_slug
                }) {
                    placeholder.statistics = old_stats;
                }
                // Clear the flag — merged, won't trigger again next season
                self.last_transfer_date = None;
                return;
            }
        }

        // Normal snapshot — clear transfer flag
        self.last_transfer_date = None;

        self.statistics_history.push_or_replace(PlayerStatisticsHistoryItem {
            season,
            team_name: team_name.to_string(),
            team_slug: team_slug.to_string(),
            team_reputation,
            league_name: league_name.to_string(),
            league_slug: league_slug.to_string(),
            is_loan,
            transfer_fee: None,
            statistics: old_stats,
            created_at: date,
        });
    }
}
