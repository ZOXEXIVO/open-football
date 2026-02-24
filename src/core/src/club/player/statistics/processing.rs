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
        league_name: &str,
        league_slug: &str,
        date: NaiveDate,
    ) {
        let is_loan = self.contract.as_ref()
            .map(|c| c.contract_type == ContractType::Loan)
            .unwrap_or(false);

        let old_stats = std::mem::take(&mut self.statistics);
        self.statistics_history.push_or_replace(PlayerStatisticsHistoryItem {
            season,
            team_name: team_name.to_string(),
            team_slug: team_slug.to_string(),
            league_name: league_name.to_string(),
            league_slug: league_slug.to_string(),
            is_loan,
            statistics: old_stats,
            created_at: date,
        });
    }
}
