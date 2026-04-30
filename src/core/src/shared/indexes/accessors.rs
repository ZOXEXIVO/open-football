use super::TeamData;
use crate::continent::Continent;
use crate::country::Country;
use crate::league::League;
use crate::{Club, Player, SimulatorData, Staff, Team};
use chrono::NaiveDate;

/// One row in the player-transfers UI showing who is watching a player.
/// Cheap to clone (small fixed fields plus a couple of strings) — built
/// per request.
#[derive(Debug, Clone)]
pub struct PlayerMonitoringDetail {
    pub club_id: u32,
    pub club_name: String,
    pub team_slug: String,
    /// Lead scout name when monitoring is active; `None` when the
    /// interest came in via shortlist / staff recommendation only.
    pub scout_name: Option<String>,
    pub scout_staff_id: Option<u32>,
    /// Status string ready for i18n key lookup
    /// (e.g. "active", "report_ready", "shortlisted").
    pub status: String,
    pub last_observed: Option<NaiveDate>,
    pub confidence: f32,
    pub times_watched: u16,
    pub matches_watched: u16,
    pub shortlisted: bool,
    pub negotiating: bool,
}

/// One row in the staff-page workload UI showing what a scout is
/// currently working on. Mirrors `PlayerMonitoringDetail` but flipped:
/// per scout instead of per player.
#[derive(Debug, Clone)]
pub struct StaffMonitoringRow {
    /// Club paying the scout — useful when a scout has moved clubs.
    pub scouting_club_id: u32,
    pub scouting_club_name: String,
    pub player_id: u32,
    pub player_name: String,
    pub target_club_name: String,
    pub target_team_slug: String,
    pub status: String,
    pub last_observed: Option<NaiveDate>,
    pub confidence: f32,
    pub times_watched: u16,
    pub matches_watched: u16,
    pub assessed_ability: u8,
    pub assessed_potential: u8,
    /// Latest meeting vote choice if any — mapped to an i18n-friendly key.
    pub latest_vote: Option<String>,
}

impl SimulatorData {
    pub fn continent(&self, id: u32) -> Option<&Continent> {
        self.continents.iter().find(|c| c.id == id)
    }

    pub fn continent_mut(&mut self, id: u32) -> Option<&mut Continent> {
        self.continents.iter_mut().find(|c| c.id == id)
    }

    pub fn country(&self, id: u32) -> Option<&Country> {
        self.continents
            .iter()
            .flat_map(|c| &c.countries)
            .find(|c| c.id == id)
    }

    pub fn country_mut(&mut self, id: u32) -> Option<&mut Country> {
        self.continents
            .iter_mut()
            .flat_map(|c| &mut c.countries)
            .find(|c| c.id == id)
    }

    pub fn league(&self, id: u32) -> Option<&League> {
        self.indexes
            .as_ref()
            .and_then(|indexes| indexes.get_league_location(id))
            .and_then(|(league_continent_id, league_country_id)| {
                self.continent(league_continent_id)
                    .and_then(|continent| {
                        continent
                            .countries
                            .iter()
                            .find(|country| country.id == league_country_id)
                    })
                    .and_then(|country| {
                        country
                            .leagues
                            .leagues
                            .iter()
                            .find(|league| league.id == id)
                    })
            })
    }

    pub fn league_mut(&mut self, id: u32) -> Option<&mut League> {
        self.indexes
            .as_ref()
            .and_then(|indexes| indexes.get_league_location(id))
            .and_then(|(league_continent_id, league_country_id)| {
                self.continent_mut(league_continent_id)
                    .and_then(|continent| {
                        continent
                            .countries
                            .iter_mut()
                            .find(|country| country.id == league_country_id)
                    })
                    .and_then(|country| {
                        country
                            .leagues
                            .leagues
                            .iter_mut()
                            .find(|league| league.id == id)
                    })
            })
    }

    pub fn team_data(&self, id: u32) -> Option<&TeamData> {
        self.indexes.as_ref().unwrap().get_team_data(id)
    }

    pub fn country_by_club(&self, club_id: u32) -> Option<&Country> {
        self.indexes
            .as_ref()
            .and_then(|indexes| indexes.get_club_location(club_id))
            .and_then(|(club_continent_id, club_country_id)| {
                self.continent(club_continent_id).and_then(|continent| {
                    continent
                        .countries
                        .iter()
                        .find(|country| country.id == club_country_id)
                })
            })
    }

    /// Get the continent a club belongs to
    pub fn continent_by_club(&self, club_id: u32) -> Option<&Continent> {
        self.indexes
            .as_ref()
            .and_then(|indexes| indexes.get_club_location(club_id))
            .and_then(|(continent_id, _)| self.continent(continent_id))
    }

    /// Get all continental competition matches (CL, EL, Conference) for a club.
    /// Returns (competition_name, home_club_id, away_club_id, date, match_id, result).
    pub fn continental_matches_for_club(
        &self,
        club_id: u32,
    ) -> Vec<(&str, u32, u32, chrono::NaiveDate, &str, Option<(u8, u8)>)> {
        let Some(continent) = self.continent_by_club(club_id) else {
            return Vec::new();
        };

        let cc = &continent.continental_competitions;
        let mut matches = Vec::new();

        for m in &cc.champions_league.matches {
            if m.home_team == club_id || m.away_team == club_id {
                matches.push((
                    "Champions League",
                    m.home_team,
                    m.away_team,
                    m.date,
                    m.match_id.as_str(),
                    m.result,
                ));
            }
        }
        for m in &cc.europa_league.matches {
            if m.home_team == club_id || m.away_team == club_id {
                matches.push((
                    "Europa League",
                    m.home_team,
                    m.away_team,
                    m.date,
                    m.match_id.as_str(),
                    m.result,
                ));
            }
        }
        for m in &cc.conference_league.matches {
            if m.home_team == club_id || m.away_team == club_id {
                matches.push((
                    "Conference League",
                    m.home_team,
                    m.away_team,
                    m.date,
                    m.match_id.as_str(),
                    m.result,
                ));
            }
        }

        matches
    }

    pub fn club(&self, id: u32) -> Option<&Club> {
        self.indexes
            .as_ref()
            .and_then(|indexes| indexes.get_club_location(id))
            .and_then(|(club_continent_id, club_country_id)| {
                self.continent(club_continent_id).and_then(|continent| {
                    continent
                        .countries
                        .iter()
                        .find(|country| country.id == club_country_id)
                })
            })
            .and_then(|country| country.clubs.iter().find(|club| club.id == id))
    }

    pub fn club_mut(&mut self, id: u32) -> Option<&mut Club> {
        self.indexes
            .as_ref()
            .and_then(|indexes| indexes.get_club_location(id))
            .and_then(|(club_continent_id, club_country_id)| {
                self.continent_mut(club_continent_id).and_then(|continent| {
                    continent
                        .countries
                        .iter_mut()
                        .find(|country| country.id == club_country_id)
                })
            })
            .and_then(|country| country.clubs.iter_mut().find(|club| club.id == id))
    }

    pub fn team(&self, id: u32) -> Option<&Team> {
        self.indexes
            .as_ref()
            .and_then(|indexes| indexes.get_team_location(id))
            .and_then(|(team_continent_id, team_country_id, team_club_id)| {
                self.continent(team_continent_id)
                    .and_then(|continent| {
                        continent
                            .countries
                            .iter()
                            .find(|country| country.id == team_country_id)
                    })
                    .and_then(|country| country.clubs.iter().find(|club| club.id == team_club_id))
                    .and_then(|club| club.teams.find(id))
            })
    }

    pub fn team_mut(&mut self, id: u32) -> Option<&mut Team> {
        self.indexes
            .as_ref()
            .and_then(|indexes| indexes.get_team_location(id))
            .and_then(|(team_continent_id, team_country_id, team_club_id)| {
                self.continent_mut(team_continent_id)
                    .and_then(|continent| {
                        continent
                            .countries
                            .iter_mut()
                            .find(|country| country.id == team_country_id)
                    })
                    .and_then(|country| {
                        country
                            .clubs
                            .iter_mut()
                            .find(|club| club.id == team_club_id)
                    })
                    .and_then(|club| club.teams.find_mut(id))
            })
    }

    pub fn player(&self, id: u32) -> Option<&Player> {
        // Fast path: indexed lookup
        if let Some((player_continent_id, player_country_id, player_club_id, player_team_id)) = self
            .indexes
            .as_ref()
            .and_then(|indexes| indexes.get_player_location(id))
        {
            let found = self
                .continent(player_continent_id)
                .and_then(|continent| {
                    continent
                        .countries
                        .iter()
                        .find(|country| country.id == player_country_id)
                })
                .and_then(|country| country.clubs.iter().find(|club| club.id == player_club_id))
                .and_then(|club| {
                    club.teams
                        .teams
                        .iter()
                        .find(|team| team.id == player_team_id)
                })
                .and_then(|team| team.players.find(id));

            if found.is_some() {
                return found;
            }
        }

        // Fallback: brute-force scan (index may be stale after transfers)
        self.player_brute_force(id)
    }

    pub fn player_with_team(&self, player_id: u32) -> Option<(&Player, &Team)> {
        // Fast path: indexed lookup
        if let Some((continent_id, country_id, club_id, team_id)) = self
            .indexes
            .as_ref()
            .and_then(|idx| idx.get_player_location(player_id))
        {
            let result = self
                .continent(continent_id)
                .and_then(|c| c.countries.iter().find(|co| co.id == country_id))
                .and_then(|co| co.clubs.iter().find(|cl| cl.id == club_id))
                .and_then(|cl| cl.teams.find(team_id))
                .and_then(|team| team.players.find(player_id).map(|p| (p, team)));

            if result.is_some() {
                return result;
            }
        }

        // Fallback: brute-force
        for continent in &self.continents {
            for country in &continent.countries {
                for club in &country.clubs {
                    for team in &club.teams.teams {
                        if let Some(player) = team.players.find(player_id) {
                            return Some((player, team));
                        }
                    }
                }
            }
        }
        None
    }

    fn player_brute_force(&self, id: u32) -> Option<&Player> {
        for continent in &self.continents {
            for country in &continent.countries {
                for club in &country.clubs {
                    for team in &club.teams.teams {
                        if let Some(player) = team.players.find(id) {
                            return Some(player);
                        }
                    }
                }
                // Also check retired players
                if let Some(player) = country.retired_players.iter().find(|p| p.id == id) {
                    return Some(player);
                }
                // Also check national team generated (synthetic) players
                if let Some(player) = country
                    .national_team
                    .generated_squad
                    .iter()
                    .find(|p| p.id == id)
                {
                    return Some(player);
                }
            }
        }
        // Check free agents pool
        if let Some(player) = self.free_agents.iter().find(|p| p.id == id) {
            return Some(player);
        }
        None
    }

    /// Find a retired player by ID across all countries.
    pub fn retired_player(&self, id: u32) -> Option<&Player> {
        for continent in &self.continents {
            for country in &continent.countries {
                if let Some(player) = country.retired_players.iter().find(|p| p.id == id) {
                    return Some(player);
                }
            }
        }
        None
    }

    pub fn player_mut(&mut self, id: u32) -> Option<&mut Player> {
        // Phase 1: immutable search for array indices
        let pos = self.find_player_position(id);

        if let Some((ci, coi, cli, ti)) = pos {
            // Phase 2: mutable access at known position
            return self.continents[ci].countries[coi].clubs[cli].teams.teams[ti]
                .players
                .find_mut(id);
        }

        // Check free agents pool
        self.free_agents.iter_mut().find(|p| p.id == id)
    }

    pub fn find_player_position(&self, id: u32) -> Option<(usize, usize, usize, usize)> {
        // Fast path: indexed lookup
        if let Some((pc, pco, pcl, pt)) = self
            .indexes
            .as_ref()
            .and_then(|indexes| indexes.get_player_location(id))
        {
            for (ci, continent) in self.continents.iter().enumerate() {
                if continent.id != pc {
                    continue;
                }
                for (coi, country) in continent.countries.iter().enumerate() {
                    if country.id != pco {
                        continue;
                    }
                    for (cli, club) in country.clubs.iter().enumerate() {
                        if club.id != pcl {
                            continue;
                        }
                        for (ti, team) in club.teams.iter().enumerate() {
                            if team.id != pt {
                                continue;
                            }
                            if team.players.contains(id) {
                                return Some((ci, coi, cli, ti));
                            }
                        }
                    }
                }
            }
        }

        // Fallback: brute-force scan (index may be stale after transfers)
        for (ci, continent) in self.continents.iter().enumerate() {
            for (coi, country) in continent.countries.iter().enumerate() {
                for (cli, club) in country.clubs.iter().enumerate() {
                    for (ti, team) in club.teams.iter().enumerate() {
                        if team.players.contains(id) {
                            return Some((ci, coi, cli, ti));
                        }
                    }
                }
            }
        }
        None
    }

    /// Find the array indices (continent, country, club, main team) for a club by ID.
    pub fn find_club_main_team(&self, club_id: u32) -> Option<(usize, usize, usize, usize)> {
        for (ci, continent) in self.continents.iter().enumerate() {
            for (coi, country) in continent.countries.iter().enumerate() {
                for (cli, club) in country.clubs.iter().enumerate() {
                    if club.id == club_id {
                        let ti = club.teams.main_index().unwrap_or(0);
                        return Some((ci, coi, cli, ti));
                    }
                }
            }
        }
        None
    }

    /// Rebuild player indexes after a transfer/loan move.
    pub fn rebuild_indexes(&mut self) {
        if let Some(mut indexes) = self.indexes.take() {
            indexes.refresh_player_indexes(self);
            self.indexes = Some(indexes);
        }
    }

    /// Rebuild indexes only if a transfer actually moved a player today.
    /// Resets `dirty_player_index` after a successful refresh so the next
    /// tick starts clean. Walking the world every day is wasteful — this
    /// is the cheap default the orchestrator should call.
    pub fn rebuild_indexes_if_dirty(&mut self) {
        if self.dirty_player_index {
            self.rebuild_indexes();
            self.dirty_player_index = false;
        }
    }

    pub fn staff_with_team(&self, staff_id: u32) -> Option<(&Staff, &Team)> {
        // Fast path: indexed lookup
        if let Some((continent_id, country_id, club_id, team_id)) = self
            .indexes
            .as_ref()
            .and_then(|idx| idx.get_staff_location(staff_id))
        {
            let result = self
                .continent(continent_id)
                .and_then(|c| c.countries.iter().find(|co| co.id == country_id))
                .and_then(|co| co.clubs.iter().find(|cl| cl.id == club_id))
                .and_then(|cl| cl.teams.find(team_id))
                .and_then(|team| team.staffs.find(staff_id).map(|s| (s, team)));

            if result.is_some() {
                return result;
            }
        }

        // Fallback: brute-force
        for continent in &self.continents {
            for country in &continent.countries {
                for club in &country.clubs {
                    for team in club.teams.iter() {
                        if let Some(staff) = team.staffs.find(staff_id) {
                            return Some((staff, team));
                        }
                    }
                }
            }
        }
        None
    }

    /// Detailed monitoring summary for a player. Returns one row per
    /// active scout-by-club monitoring + a fallback row per non-monitoring
    /// interest (shortlist / staff recommendation only). Used by the
    /// player transfers UI to show *who* is watching, not just *which
    /// club*. Sort order is: clubs with active scout monitoring first
    /// (by latest observation), then everyone else by club name.
    pub fn player_monitoring_details(&self, player_id: u32) -> Vec<PlayerMonitoringDetail> {
        let mut out: Vec<PlayerMonitoringDetail> = Vec::new();

        for continent in &self.continents {
            for country in &continent.countries {
                for club in &country.clubs {
                    let plan = &club.transfer_plan;
                    let team_slug = club
                        .teams
                        .teams
                        .first()
                        .map(|t| t.slug.clone())
                        .unwrap_or_default();

                    let monitorings: Vec<&crate::transfers::pipeline::ScoutPlayerMonitoring> =
                        plan.monitorings_for_player(player_id);
                    let on_shortlist = plan
                        .shortlists
                        .iter()
                        .any(|s| s.candidates.iter().any(|c| c.player_id == player_id));
                    let in_negotiation = country
                        .transfer_market
                        .negotiations
                        .values()
                        .any(|n| n.player_id == player_id && n.buying_club_id == club.id);
                    let recommended_by_staff: Vec<u32> = plan
                        .staff_recommendations
                        .iter()
                        .filter(|r| r.player_id == player_id)
                        .map(|r| r.recommender_staff_id)
                        .collect();

                    if monitorings.is_empty()
                        && !on_shortlist
                        && !in_negotiation
                        && recommended_by_staff.is_empty()
                        && !plan.known_players.iter().any(|m| m.player_id == player_id)
                    {
                        continue;
                    }

                    if monitorings.is_empty() {
                        // No active scout monitoring, but the player
                        // still surfaces because the club has a
                        // shortlist / negotiation / known-player
                        // record. Emit one summary row per recommender
                        // (or a single row if there's no recommender).
                        let recommender_id = recommended_by_staff.first().copied();
                        let scout_name = recommender_id.and_then(|id| {
                            self.staff_with_team(id).map(|(s, _)| Self::full_name(s))
                        });
                        out.push(PlayerMonitoringDetail {
                            club_id: club.id,
                            club_name: club.name.clone(),
                            team_slug: team_slug.clone(),
                            scout_name,
                            scout_staff_id: recommender_id,
                            status: if in_negotiation {
                                "negotiating".to_string()
                            } else if on_shortlist {
                                "shortlisted".to_string()
                            } else if !recommended_by_staff.is_empty() {
                                "recommended".to_string()
                            } else {
                                "known".to_string()
                            },
                            last_observed: None,
                            confidence: 0.0,
                            times_watched: 0,
                            matches_watched: 0,
                            shortlisted: on_shortlist,
                            negotiating: in_negotiation,
                        });
                        continue;
                    }

                    for m in monitorings {
                        let scout_name = self
                            .staff_with_team(m.scout_staff_id)
                            .map(|(s, _)| Self::full_name(s));
                        let status = match m.status {
                            crate::transfers::pipeline::ScoutMonitoringStatus::Active => "active",
                            crate::transfers::pipeline::ScoutMonitoringStatus::Paused => "paused",
                            crate::transfers::pipeline::ScoutMonitoringStatus::ReportReady => {
                                "report_ready"
                            }
                            crate::transfers::pipeline::ScoutMonitoringStatus::PromotedToShortlist => {
                                "shortlisted"
                            }
                            crate::transfers::pipeline::ScoutMonitoringStatus::Negotiating => {
                                "negotiating"
                            }
                            crate::transfers::pipeline::ScoutMonitoringStatus::Signed => "signed",
                            crate::transfers::pipeline::ScoutMonitoringStatus::Lost => "lost",
                            crate::transfers::pipeline::ScoutMonitoringStatus::Rejected => {
                                "rejected"
                            }
                        };
                        out.push(PlayerMonitoringDetail {
                            club_id: club.id,
                            club_name: club.name.clone(),
                            team_slug: team_slug.clone(),
                            scout_name,
                            scout_staff_id: Some(m.scout_staff_id),
                            status: status.to_string(),
                            last_observed: Some(m.last_observed),
                            confidence: m.confidence,
                            times_watched: m.times_watched,
                            matches_watched: m.matches_watched,
                            shortlisted: on_shortlist,
                            negotiating: in_negotiation,
                        });
                    }
                }
            }
        }

        // Sort: most-recent observations first, then by club name.
        out.sort_by(|a, b| {
            b.last_observed
                .cmp(&a.last_observed)
                .then(a.club_name.cmp(&b.club_name))
        });
        out
    }

    /// Workload summary for a single scout — every player they're
    /// actively monitoring along with the relevant context the staff
    /// page renders.
    pub fn staff_monitoring_workload(&self, staff_id: u32) -> Vec<StaffMonitoringRow> {
        let mut out: Vec<StaffMonitoringRow> = Vec::new();

        for continent in &self.continents {
            for country in &continent.countries {
                for club in &country.clubs {
                    let plan = &club.transfer_plan;
                    let mut rows: Vec<&crate::transfers::pipeline::ScoutPlayerMonitoring> = plan
                        .scout_monitoring
                        .iter()
                        .filter(|m| m.scout_staff_id == staff_id)
                        .collect();
                    if rows.is_empty() {
                        continue;
                    }
                    rows.sort_by(|a, b| b.last_observed.cmp(&a.last_observed));
                    for m in rows {
                        let player = self.player(m.player_id);
                        let player_name = player
                            .map(|p| {
                                format!(
                                    "{} {}",
                                    p.full_name.display_first_name(),
                                    p.full_name.display_last_name()
                                )
                            })
                            .unwrap_or_default();
                        // Locate the player's current club for the UI.
                        let (target_club_name, target_team_slug) = self
                            .player_with_team(m.player_id)
                            .map(|(_, t)| {
                                let club_name = self
                                    .club(t.club_id)
                                    .map(|c| c.name.clone())
                                    .unwrap_or_default();
                                (club_name, t.slug.clone())
                            })
                            .unwrap_or_default();
                        let status = match m.status {
                            crate::transfers::pipeline::ScoutMonitoringStatus::Active => "active",
                            crate::transfers::pipeline::ScoutMonitoringStatus::Paused => "paused",
                            crate::transfers::pipeline::ScoutMonitoringStatus::ReportReady => {
                                "report_ready"
                            }
                            crate::transfers::pipeline::ScoutMonitoringStatus::PromotedToShortlist => {
                                "shortlisted"
                            }
                            crate::transfers::pipeline::ScoutMonitoringStatus::Negotiating => {
                                "negotiating"
                            }
                            crate::transfers::pipeline::ScoutMonitoringStatus::Signed => "signed",
                            crate::transfers::pipeline::ScoutMonitoringStatus::Lost => "lost",
                            crate::transfers::pipeline::ScoutMonitoringStatus::Rejected => {
                                "rejected"
                            }
                        };
                        // Latest meeting vote by this scout for this player, if any.
                        let latest_vote = plan
                            .recruitment_meetings
                            .iter()
                            .rev()
                            .flat_map(|mtg| mtg.player_votes.iter())
                            .find(|v| v.scout_staff_id == staff_id && v.player_id == m.player_id)
                            .map(|v| match v.vote {
                                crate::transfers::pipeline::ScoutVoteChoice::StrongApprove => {
                                    "strong_approve"
                                }
                                crate::transfers::pipeline::ScoutVoteChoice::Approve => "approve",
                                crate::transfers::pipeline::ScoutVoteChoice::Monitor => "monitor",
                                crate::transfers::pipeline::ScoutVoteChoice::Reject => "reject",
                                crate::transfers::pipeline::ScoutVoteChoice::NeedsMoreInfo => {
                                    "needs_more_info"
                                }
                            });
                        out.push(StaffMonitoringRow {
                            scouting_club_id: club.id,
                            scouting_club_name: club.name.clone(),
                            player_id: m.player_id,
                            player_name,
                            target_club_name,
                            target_team_slug,
                            status: status.to_string(),
                            last_observed: Some(m.last_observed),
                            confidence: m.confidence,
                            times_watched: m.times_watched,
                            matches_watched: m.matches_watched,
                            assessed_ability: m.current_assessed_ability,
                            assessed_potential: m.current_assessed_potential,
                            latest_vote: latest_vote.map(|s| s.to_string()),
                        });
                    }
                }
            }
        }

        out
    }

    fn full_name(staff: &Staff) -> String {
        format!(
            "{} {}",
            staff.full_name.first_name, staff.full_name.last_name
        )
    }

    /// Find all clubs that have this player in their scouting assignments,
    /// shortlists, or scouting reports (i.e. clubs interested in signing the player).
    /// Returns Vec of (club_id, club_name, team_slug).
    pub fn clubs_interested_in_player(&self, player_id: u32) -> Vec<(u32, String, String)> {
        let mut interested = Vec::new();

        for continent in &self.continents {
            for country in &continent.countries {
                for club in &country.clubs {
                    let mut is_interested = false;

                    // Check scouting assignments for observations of this player
                    for assignment in &club.transfer_plan.scouting_assignments {
                        if assignment
                            .observations
                            .iter()
                            .any(|o| o.player_id == player_id)
                        {
                            is_interested = true;
                            break;
                        }
                    }

                    // Check scouting reports
                    if !is_interested {
                        if club
                            .transfer_plan
                            .scouting_reports
                            .iter()
                            .any(|r| r.player_id == player_id)
                        {
                            is_interested = true;
                        }
                    }

                    // Check shortlists
                    if !is_interested {
                        for shortlist in &club.transfer_plan.shortlists {
                            if shortlist
                                .candidates
                                .iter()
                                .any(|c| c.player_id == player_id)
                            {
                                is_interested = true;
                                break;
                            }
                        }
                    }

                    // Check staff recommendations
                    if !is_interested {
                        if club
                            .transfer_plan
                            .staff_recommendations
                            .iter()
                            .any(|r| r.player_id == player_id)
                        {
                            is_interested = true;
                        }
                    }

                    // Persistent scouting memory: clubs may know a player
                    // from a past loan spell or observed match even after he
                    // has returned to another country.
                    if !is_interested {
                        if club
                            .transfer_plan
                            .known_players
                            .iter()
                            .any(|m| m.player_id == player_id)
                        {
                            is_interested = true;
                        }
                    }

                    // Active scout monitoring rows count too — a club
                    // may have a scout watching the player even before
                    // observations roll up into a `scouting_reports`
                    // entry.
                    if !is_interested {
                        if club
                            .transfer_plan
                            .scout_monitoring
                            .iter()
                            .any(|m| m.player_id == player_id && m.is_active_interest())
                        {
                            is_interested = true;
                        }
                    }

                    if is_interested {
                        let team_slug = club
                            .teams
                            .teams
                            .first()
                            .map(|t| t.slug.clone())
                            .unwrap_or_default();
                        interested.push((club.id, club.name.clone(), team_slug));
                    }
                }
            }
        }

        interested
    }
}
