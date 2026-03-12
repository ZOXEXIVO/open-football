use chrono::{Datelike, NaiveDate};
use log::debug;
use super::types::can_club_accept_player;
use crate::{
    Country, Person, PlayerClubContract, PlayerStatusType, TeamInfo,
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
    let mut from_info: Option<TeamInfo> = None;
    let mut selling_league_id = None;

    if let Some(selling_club) = country.clubs.iter_mut().find(|c| c.id == selling_club_id) {
        // Use main team info for history (consistent with season snapshot)
        if let Some(main_team) = selling_club.teams.teams.iter()
            .find(|t| t.team_type == crate::TeamType::Main)
        {
            selling_league_id = main_team.league_id;
            from_info = Some(TeamInfo {
                name: selling_club.name.clone(),
                slug: main_team.slug.clone(),
                reputation: main_team.reputation.world,
                league_name: String::new(), // filled below
                league_slug: String::new(),
            });
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
        // Resolve league name (skip friendly leagues, use main league)
        if let Some(ref mut from) = from_info {
            let (league_name, league_slug) = selling_league_id
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
            from.league_name = league_name;
            from.league_slug = league_slug;
        }

        // Resolve buying club info
        let to_info = country.clubs.iter()
            .find(|c| c.id == buying_club_id)
            .and_then(|c| {
                let main_team = c.teams.teams.iter()
                    .find(|t| t.team_type == crate::TeamType::Main)
                    .or(c.teams.teams.first())?;
                let (league_name, league_slug) = main_team.league_id
                    .and_then(|lid| country.leagues.leagues.iter().find(|l| l.id == lid))
                    .map(|l| (l.name.clone(), l.slug.clone()))
                    .unwrap_or_default();
                Some(TeamInfo {
                    name: main_team.name.clone(),
                    slug: main_team.slug.clone(),
                    reputation: main_team.reputation.world,
                    league_name,
                    league_slug,
                })
            });

        if let (Some(from), Some(to)) = (from_info, to_info) {
            player.on_transfer(&from, &to, fee, date);
        }

        player.statuses.remove(PlayerStatusType::Lst);
        player.statuses.remove(PlayerStatusType::Loa);
        player.statuses.remove(PlayerStatusType::Req);
        player.statuses.remove(PlayerStatusType::Unh);
        player.statuses.remove(PlayerStatusType::Trn);
        player.statuses.remove(PlayerStatusType::Bid);
        player.statuses.remove(PlayerStatusType::Wnt);
        player.statuses.remove(PlayerStatusType::Sct);
        player.statuses.remove(PlayerStatusType::Enq);

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
    let mut from_info: Option<TeamInfo> = None;
    let mut selling_league_id = None;
    let mut from_team_id = 0u32;

    if let Some(selling_club) = country.clubs.iter_mut().find(|c| c.id == selling_club_id) {
        // Capture main team info for history entries BEFORE moving to reserve
        if let Some(main_team) = selling_club.teams.teams.iter()
            .find(|t| t.team_type == crate::TeamType::Main)
        {
            selling_league_id = main_team.league_id;
            from_info = Some(TeamInfo {
                name: selling_club.name.clone(),
                slug: main_team.slug.clone(),
                reputation: main_team.reputation.world,
                league_name: String::new(),
                league_slug: String::new(),
            });
        }

        // Remember which team the player is on before moving
        from_team_id = selling_club.teams.teams.iter()
            .find(|t| t.players.players.iter().any(|p| p.id == player_id))
            .map(|t| t.id)
            .unwrap_or(0);

        // Move player to reserve team before loaning out
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
        // Resolve league name
        if let Some(ref mut from) = from_info {
            let (league_name, league_slug) = selling_league_id
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
            from.league_name = league_name;
            from.league_slug = league_slug;
        }

        // Resolve buying club info
        let to_info = country.clubs.iter()
            .find(|c| c.id == buying_club_id)
            .and_then(|c| {
                let main_team = c.teams.teams.iter()
                    .find(|t| t.team_type == crate::TeamType::Main)
                    .or(c.teams.teams.first())?;
                let (league_name, league_slug) = main_team.league_id
                    .and_then(|lid| country.leagues.leagues.iter().find(|l| l.id == lid))
                    .map(|l| (l.name.clone(), l.slug.clone()))
                    .unwrap_or_default();
                Some(TeamInfo {
                    name: main_team.name.clone(),
                    slug: main_team.slug.clone(),
                    reputation: main_team.reputation.world,
                    league_name,
                    league_slug,
                })
            });

        if let (Some(from), Some(to)) = (from_info, to_info) {
            player.on_loan(&from, &to, loan_fee, date);
        }

        player.statuses.remove(PlayerStatusType::Loa);
        player.statuses.remove(PlayerStatusType::Lst);
        player.statuses.remove(PlayerStatusType::Req);
        player.statuses.remove(PlayerStatusType::Unh);
        player.statuses.remove(PlayerStatusType::Trn);
        player.statuses.remove(PlayerStatusType::Bid);
        player.statuses.remove(PlayerStatusType::Wnt);
        player.statuses.remove(PlayerStatusType::Sct);
        player.statuses.remove(PlayerStatusType::Enq);

        player.happiness = crate::PlayerHappiness::new();

        // Loan expires at end of current season, derived from league settings.
        let loan_end = selling_league_id
            .and_then(|lid| country.leagues.leagues.iter().find(|l| l.id == lid))
            .map(|league| {
                let end = &league.settings.season_ending_half;
                let end_month = end.to_month as u32;
                let end_day = end.to_day as u32;
                // If we're already past the season end date, loan expires next year
                let year = if date.month() > end_month
                    || (date.month() == end_month && date.day() > end_day)
                {
                    date.year() + 1
                } else {
                    date.year()
                };
                NaiveDate::from_ymd_opt(year, end_month, end_day).unwrap_or(date)
            })
            .unwrap_or_else(|| {
                // Fallback if no league found: end of May
                let year = if date.month() >= 6 { date.year() + 1 } else { date.year() };
                NaiveDate::from_ymd_opt(year, 5, 31).unwrap_or(date)
            });

        let salary = (loan_fee / 50.0).max(200.0) as u32;
        player.contract = Some(PlayerClubContract::new_loan(salary, loan_end, selling_club_id, from_team_id, buying_club_id));

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
