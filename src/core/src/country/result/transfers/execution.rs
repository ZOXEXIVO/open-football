use chrono::NaiveDate;
use log::{debug, info};
use super::types::can_club_accept_player;
use crate::league::Season;
use crate::{
    Country, Person, PlayerClubContract, PlayerStatistics,
    PlayerStatisticsHistoryItem, PlayerStatusType,
};

pub(crate) fn execute_player_transfer(
    country: &mut Country,
    player_id: u32,
    selling_club_id: u32,
    buying_club_id: u32,
    fee: f64,
    date: NaiveDate,
) {
    let mut player = None;
    let mut selling_team_name = String::new();
    let mut selling_team_slug = String::new();
    let mut selling_team_reputation: u16 = 0;
    let mut selling_league_id = None;

    if let Some(selling_club) = country.clubs.iter_mut().find(|c| c.id == selling_club_id) {
        selling_team_name = selling_club.name.clone();

        // Use main team info for history (consistent with season snapshot)
        if let Some(main_team) = selling_club.teams.teams.iter()
            .find(|t| t.team_type == crate::TeamType::Main)
        {
            selling_team_slug = main_team.slug.clone();
            selling_team_reputation = main_team.reputation.world;
            selling_league_id = main_team.league_id;
        }

        for team in &mut selling_club.teams.teams {
            if let Some(p) = team.players.take_player(&player_id) {
                player = Some(p);
                team.transfer_list.remove(player_id);
                break;
            }
        }

        selling_club.finance.add_transfer_income(fee);
    }

    if let Some(mut player) = player {
        let season = Season::from_date(date);

        // If the team's league is friendly (reserves etc.), use the club's main league instead
        let (selling_league_name, selling_league_slug) = selling_league_id
            .and_then(|lid| country.leagues.leagues.iter().find(|l| l.id == lid))
            .and_then(|l| {
                if l.friendly {
                    country.leagues.leagues.iter().find(|ml| !ml.friendly)
                } else {
                    Some(l)
                }
            })
            .map(|l| (l.name.clone(), l.slug.clone()))
            .unwrap_or_default();

        let old_stats = std::mem::take(&mut player.statistics);

        // Skip 0-game selling team entry unless it's the player's only history
        // (a player must always have at least one record)
        let has_games = old_stats.played + old_stats.played_subs > 0;
        if has_games || player.statistics_history.items.is_empty() {
            player.statistics_history.push_or_replace(PlayerStatisticsHistoryItem {
                season: season.clone(),
                team_name: selling_team_name,
                team_slug: selling_team_slug,
                team_reputation: selling_team_reputation,
                league_name: selling_league_name,
                league_slug: selling_league_slug,
                is_loan: false,
                transfer_fee: None,
                statistics: old_stats,
                created_at: date,
            });
        }

        player.statistics = PlayerStatistics::default();

        let buying_info = country.clubs.iter()
            .find(|c| c.id == buying_club_id)
            .and_then(|c| {
                let main_team = c.teams.teams.iter()
                    .find(|t| t.team_type == crate::TeamType::Main)
                    .or(c.teams.teams.first())?;
                let league = main_team.league_id
                    .and_then(|lid| country.leagues.leagues.iter().find(|l| l.id == lid))
                    .map(|l| (l.name.clone(), l.slug.clone()))
                    .unwrap_or_default();
                Some((main_team.name.clone(), main_team.slug.clone(), main_team.reputation.world, league.0, league.1))
            });

        if let Some((buy_team_name, buy_team_slug, buy_team_rep, buy_league_name, buy_league_slug)) = buying_info {
            player.statistics_history.push_or_replace(PlayerStatisticsHistoryItem {
                season: season.clone(),
                team_name: buy_team_name,
                team_slug: buy_team_slug,
                team_reputation: buy_team_rep,
                league_name: buy_league_name,
                league_slug: buy_league_slug,
                is_loan: false,
                transfer_fee: Some(fee),
                statistics: PlayerStatistics::default(),
                created_at: date,
            });
        }

        player.last_transfer_date = Some(date);

        player.statuses.remove(PlayerStatusType::Lst);
        player.statuses.remove(PlayerStatusType::Loa);
        player.statuses.remove(PlayerStatusType::Req);
        player.statuses.remove(PlayerStatusType::Unh);
        player.statuses.remove(PlayerStatusType::Trn);
        player.statuses.remove(PlayerStatusType::Bid);
        player.statuses.remove(PlayerStatusType::Wnt);
        player.statuses.remove(PlayerStatusType::Sct);
        player.statuses.remove(PlayerStatusType::Enq);

        // Fresh start at new club — reset happiness to neutral
        player.happiness = crate::PlayerHappiness::new();

        let contract_years = if player.age(date) < 24 { 5 }
        else if player.age(date) < 28 { 4 }
        else if player.age(date) < 32 { 3 }
        else { 2 };

        let expiry = date.checked_add_signed(chrono::Duration::days(contract_years * 365))
            .unwrap_or(date);

        let salary = (fee / 200.0).max(500.0) as u32;

        player.contract = Some(PlayerClubContract::new(salary, expiry));

        if let Some(buying_club) = country.clubs.iter_mut().find(|c| c.id == buying_club_id) {
            if !can_club_accept_player(buying_club) {
                debug!("Transfer rejected: club {} squad full, returning player {}",
                       buying_club_id, player_id);
                if let Some(selling_club) = country.clubs.iter_mut().find(|c| c.id == selling_club_id) {
                    if !selling_club.teams.teams.is_empty() {
                        selling_club.teams.teams[0].players.add(player);
                    }
                    selling_club.finance.add_transfer_income(fee);
                }
                return;
            }

            buying_club.finance.spend_from_transfer_budget(fee);

            if !buying_club.teams.teams.is_empty() {
                buying_club.teams.teams[0].players.add(player);
            }
        }

        debug!(
            "Transfer completed: player {} moved from club {} to club {} for {}",
            player_id, selling_club_id, buying_club_id, fee
        );
    }
}

pub(crate) fn execute_loan_transfer(
    country: &mut Country,
    player_id: u32,
    selling_club_id: u32,
    buying_club_id: u32,
    loan_fee: f64,
    date: NaiveDate,
) {
    let mut player = None;
    let mut selling_team_name = String::new();
    let mut selling_team_slug = String::new();
    let mut selling_team_reputation: u16 = 0;
    let mut selling_league_id = None;

    if let Some(selling_club) = country.clubs.iter_mut().find(|c| c.id == selling_club_id) {
        selling_team_name = selling_club.name.clone();

        // Capture main team info for history entries BEFORE moving to reserve
        // (consistent with season snapshot which always uses main team slug)
        if let Some(main_team) = selling_club.teams.teams.iter()
            .find(|t| t.team_type == crate::TeamType::Main)
        {
            selling_team_slug = main_team.slug.clone();
            selling_team_reputation = main_team.reputation.world;
            selling_league_id = main_team.league_id;
        }

        // Move player to reserve team before loaning out
        // so the loan record originates from reserve, not main
        let main_idx = selling_club.teams.teams.iter()
            .position(|t| t.team_type == crate::TeamType::Main);
        let reserve_idx = selling_club.teams.teams.iter()
            .position(|t| t.team_type == crate::TeamType::Reserve)
            .or_else(|| selling_club.teams.teams.iter()
                .position(|t| t.team_type == crate::TeamType::B));

        if let (Some(mi), Some(ri)) = (main_idx, reserve_idx) {
            if mi != ri {
                if let Some(p) = selling_club.teams.teams[mi].players.take_player(&player_id) {
                    selling_club.teams.teams[ri].players.add(p);
                }
            }
        }

        for team in &mut selling_club.teams.teams {
            if let Some(p) = team.players.take_player(&player_id) {
                player = Some(p);
                team.transfer_list.remove(player_id);
                break;
            }
        }

        selling_club.finance.add_transfer_income(loan_fee);
    }

    if let Some(mut player) = player {
        let season = Season::from_date(date);

        // If the team's league is friendly (reserves etc.), use the club's main league instead
        let (selling_league_name, selling_league_slug) = selling_league_id
            .and_then(|lid| country.leagues.leagues.iter().find(|l| l.id == lid))
            .and_then(|l| {
                if l.friendly {
                    country.leagues.leagues.iter().find(|ml| !ml.friendly)
                } else {
                    Some(l)
                }
            })
            .map(|l| (l.name.clone(), l.slug.clone()))
            .unwrap_or_default();

        let old_stats = std::mem::take(&mut player.statistics);

        // Skip 0-game selling team entry unless it's the player's only history
        let has_games = old_stats.played + old_stats.played_subs > 0;
        if has_games || player.statistics_history.items.is_empty() {
            player.statistics_history.push_or_replace(PlayerStatisticsHistoryItem {
                season: season.clone(),
                team_name: selling_team_name,
                team_slug: selling_team_slug,
                team_reputation: selling_team_reputation,
                league_name: selling_league_name,
                league_slug: selling_league_slug,
                is_loan: false,
                transfer_fee: None,
                statistics: old_stats,
                created_at: date,
            });
        }

        // Reset current stats for the new club — history entry for the loan spell
        // will be created when the player moves again or the season ends
        player.statistics = PlayerStatistics::default();

        let buying_info = country.clubs.iter()
            .find(|c| c.id == buying_club_id)
            .and_then(|c| {
                let main_team = c.teams.teams.iter()
                    .find(|t| t.team_type == crate::TeamType::Main)
                    .or(c.teams.teams.first())?;
                let league = main_team.league_id
                    .and_then(|lid| country.leagues.leagues.iter().find(|l| l.id == lid))
                    .map(|l| (l.name.clone(), l.slug.clone()))
                    .unwrap_or_default();
                Some((main_team.name.clone(), main_team.slug.clone(), main_team.reputation.world, league.0, league.1))
            });

        if let Some((buy_team_name, buy_team_slug, buy_team_rep, buy_league_name, buy_league_slug)) = buying_info {
            player.statistics_history.push_or_replace(PlayerStatisticsHistoryItem {
                season: season.clone(),
                team_name: buy_team_name,
                team_slug: buy_team_slug,
                team_reputation: buy_team_rep,
                league_name: buy_league_name,
                league_slug: buy_league_slug,
                is_loan: true,
                transfer_fee: Some(loan_fee),
                statistics: PlayerStatistics::default(),
                created_at: date,
            });
        }

        player.last_transfer_date = Some(date);

        player.statuses.remove(PlayerStatusType::Loa);
        player.statuses.remove(PlayerStatusType::Lst);
        player.statuses.remove(PlayerStatusType::Req);
        player.statuses.remove(PlayerStatusType::Unh);
        player.statuses.remove(PlayerStatusType::Trn);
        player.statuses.remove(PlayerStatusType::Bid);
        player.statuses.remove(PlayerStatusType::Wnt);
        player.statuses.remove(PlayerStatusType::Sct);
        player.statuses.remove(PlayerStatusType::Enq);

        // Fresh start at new club — reset happiness to neutral
        player.happiness = crate::PlayerHappiness::new();

        let loan_end = date
            .checked_add_signed(chrono::Duration::days(180))
            .unwrap_or(date);

        let salary = (loan_fee / 50.0).max(200.0) as u32;
        player.contract = Some(PlayerClubContract::new_loan(salary, loan_end, selling_club_id, buying_club_id));

        if let Some(buying_club) = country.clubs.iter_mut().find(|c| c.id == buying_club_id) {
            if !can_club_accept_player(buying_club) {
                debug!("Loan rejected: club {} squad full, returning player {}",
                       buying_club_id, player_id);
                if let Some(selling_club) = country.clubs.iter_mut().find(|c| c.id == selling_club_id) {
                    if !selling_club.teams.teams.is_empty() {
                        selling_club.teams.teams[0].players.add(player);
                    }
                }
                return;
            }

            buying_club.finance.spend_from_transfer_budget(loan_fee);

            if !buying_club.teams.teams.is_empty() {
                buying_club.teams.teams[0].players.add(player);
            }
        }

        debug!(
            "Loan completed: player {} loaned from club {} to club {} (fee: {})",
            player_id, selling_club_id, buying_club_id, loan_fee
        );
    }
}
