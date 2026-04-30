use super::TeamData;
use crate::continent::Continent;
use crate::country::Country;
use crate::league::League;
use crate::transfers::ScoutingRegion;
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

/// Pre-resolved snapshot of a club's scouting department state, served
/// to the web crate so templates stay presentational. Names, slugs,
/// dates and enum-as-i18n-keys are already worked out — the UI just
/// formats and links.
#[derive(Debug, Clone, Default)]
pub struct ClubScoutingDashboard {
    pub summary: ScoutingSummary,
    pub scout_workload: Vec<ScoutWorkloadRow>,
    pub active_monitoring: Vec<ActiveMonitoringRow>,
    pub scouting_reports: Vec<ScoutingReportRow>,
    pub scouting_assignments: Vec<ScoutingAssignmentRow>,
    pub match_assignments: Vec<MatchAssignmentRow>,
    pub recruitment_meetings: Vec<RecruitmentMeetingRow>,
    pub known_players: Vec<KnownPlayerRow>,
    pub shadow_reports: Vec<ShadowReportRow>,
    pub transfer_requests: Vec<TransferRequestRow>,
}

#[derive(Debug, Clone, Default)]
pub struct ScoutingSummary {
    pub active_scouts: u32,
    pub active_monitored: u32,
    pub report_ready: u32,
    pub open_assignments: u32,
    pub recent_meetings: u32,
    pub promoted_to_shortlist: u32,
    pub rejected_or_blocked: u32,
    pub avg_confidence_pct: u8,
}

#[derive(Debug, Clone)]
pub struct ScoutWorkloadRow {
    pub staff_id: u32,
    pub staff_name: String,
    pub role_key: String,
    pub active_count: u32,
    pub report_ready_count: u32,
    pub avg_confidence_pct: u8,
    pub last_observed: Option<NaiveDate>,
    /// Pre-translated i18n keys for regions covered by this scout's
    /// monitoring rows (e.g. `region_western_europe`).
    pub region_keys: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ActiveMonitoringRow {
    pub player_id: u32,
    pub player_slug: String,
    pub player_name: String,
    pub player_position_short: String,
    pub player_age: Option<u8>,
    pub current_club_id: Option<u32>,
    pub current_club_name: String,
    pub current_team_slug: String,
    pub scout_id: u32,
    pub scout_name: String,
    pub source_key: String,
    pub status_key: String,
    pub started_on: NaiveDate,
    pub last_observed: NaiveDate,
    pub times_watched: u16,
    pub matches_watched: u16,
    pub assessed_ability: u8,
    pub assessed_potential: u8,
    pub confidence_pct: u8,
    pub role_fit_pct: u8,
    pub estimated_value: f64,
    pub risk_flag_keys: Vec<String>,
    pub transfer_request_id: Option<u32>,
    pub transfer_request_position_short: Option<String>,
    pub latest_vote_key: Option<String>,
    pub latest_decision_key: Option<String>,
    /// Sort buckets (lower = higher in the table).
    pub sort_bucket: u8,
}

#[derive(Debug, Clone)]
pub struct ScoutingReportRow {
    pub player_id: u32,
    pub player_slug: String,
    pub player_name: String,
    pub player_position_short: String,
    pub current_club_name: String,
    pub current_team_slug: String,
    pub assignment_id: u32,
    pub transfer_request_id: Option<u32>,
    pub recommendation_key: String,
    pub assessed_ability: u8,
    pub assessed_potential: u8,
    pub confidence_pct: u8,
    pub role_fit_pct: u8,
    pub estimated_value: f64,
    pub risk_flag_keys: Vec<String>,
    /// `true` when the row was rehydrated from a shadow report and
    /// rebound to an active assignment.
    pub from_shadow: bool,
    /// Lower-is-higher sort bucket derived from the recommendation.
    pub sort_bucket: u8,
}

#[derive(Debug, Clone)]
pub struct ScoutingAssignmentRow {
    pub assignment_id: u32,
    pub transfer_request_id: u32,
    pub target_position_short: String,
    pub min_ability: u8,
    pub preferred_age_min: u8,
    pub preferred_age_max: u8,
    pub max_budget: f64,
    pub scout_id: Option<u32>,
    pub scout_name: Option<String>,
    pub observation_count: u32,
    pub reports_produced: u32,
    pub completed: bool,
    pub min_technical_avg: f32,
    pub min_mental_avg: f32,
    pub min_physical_avg: f32,
}

#[derive(Debug, Clone)]
pub struct MatchAssignmentRow {
    pub scout_id: u32,
    pub scout_name: String,
    pub target_team_id: u32,
    pub target_team_name: String,
    pub target_team_slug: String,
    pub linked_assignment_ids: Vec<u32>,
    pub last_attended: Option<NaiveDate>,
}

#[derive(Debug, Clone)]
pub struct RecruitmentMeetingRow {
    pub id: u32,
    pub date: NaiveDate,
    pub participants: Vec<MeetingParticipant>,
    pub agenda_request_ids: Vec<u32>,
    pub decisions: Vec<MeetingDecisionRow>,
    pub votes: Vec<MeetingVoteRow>,
}

#[derive(Debug, Clone)]
pub struct MeetingParticipant {
    pub staff_id: u32,
    pub staff_name: String,
    pub role_key: String,
}

#[derive(Debug, Clone)]
pub struct MeetingDecisionRow {
    pub player_id: u32,
    pub player_slug: String,
    pub player_name: String,
    pub transfer_request_id: Option<u32>,
    pub decision_key: String,
    pub consensus_score: f32,
    pub chief_scout_support: bool,
    pub data_support: bool,
    pub board_risk_score: f32,
    pub budget_fit: f32,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct MeetingVoteRow {
    pub scout_id: u32,
    pub scout_name: String,
    pub player_id: u32,
    pub player_slug: String,
    pub player_name: String,
    pub vote_key: String,
    pub score: f32,
    pub confidence_pct: u8,
    pub reason_key: String,
}

#[derive(Debug, Clone)]
pub struct KnownPlayerRow {
    pub player_id: u32,
    pub player_slug: String,
    pub player_name: String,
    pub last_known_club_name: String,
    pub last_known_team_slug: String,
    pub position_short: String,
    pub position_group_key: String,
    pub assessed_ability: u8,
    pub assessed_potential: u8,
    pub confidence_pct: u8,
    pub estimated_fee: f64,
    pub last_seen: NaiveDate,
    pub official_appearances_seen: u16,
    pub friendly_appearances_seen: u16,
}

#[derive(Debug, Clone)]
pub struct ShadowReportRow {
    pub player_id: u32,
    pub player_slug: String,
    pub player_name: String,
    pub current_club_name: String,
    pub current_team_slug: String,
    pub position_group_key: String,
    pub observed_ability: u8,
    pub recorded_on: NaiveDate,
    pub confidence_pct: u8,
    pub recommendation_key: String,
}

#[derive(Debug, Clone)]
pub struct TransferRequestRow {
    pub id: u32,
    pub position_short: String,
    pub priority_key: String,
    pub priority_sort_bucket: u8,
    pub reason_key: String,
    pub min_ability: u8,
    pub ideal_ability: u8,
    pub preferred_age_min: u8,
    pub preferred_age_max: u8,
    pub budget_allocation: f64,
    pub status_key: String,
    pub status_sort_bucket: u8,
    pub named_target_id: Option<u32>,
    pub named_target_slug: Option<String>,
    pub named_target_name: Option<String>,
    pub board_approved: Option<bool>,
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

    /// Pre-resolve everything the web Scouting tab needs into a single
    /// dashboard struct. Read-only — never mutates state.
    pub fn club_scouting_dashboard(&self, club_id: u32) -> ClubScoutingDashboard {
        ClubScoutingDashboardBuilder::new(self, club_id)
            .map(|b| b.build())
            .unwrap_or_default()
    }
}

// ════════════════════════════════════════════════════════════════════
// Scouting dashboard builder — wraps row construction so the loose
// resolution helpers don't leak as free functions. Lives in its own
// impl block to keep the SimulatorData accessor surface focused.
// ════════════════════════════════════════════════════════════════════

use crate::transfers::pipeline::{
    DetailedScoutingReport, RecruitmentDecisionType, ScoutMonitoringStatus, ScoutVoteChoice,
    ScoutingAssignment, TransferRequest,
};

/// Holds references to the simulator, club, and pre-computed lookups
/// used while assembling a `ClubScoutingDashboard`. Build once via
/// `new`, then call `build` to consume it.
pub struct ClubScoutingDashboardBuilder<'a> {
    data: &'a SimulatorData,
    club: &'a Club,
    today: NaiveDate,
    /// Latest scout vote per player across the meeting history.
    latest_vote_per_player: std::collections::HashMap<u32, ScoutVoteChoice>,
    /// Latest meeting decision per player.
    latest_decision_per_player: std::collections::HashMap<u32, RecruitmentDecisionType>,
    /// Open transfer requests indexed by id, used for monitoring context.
    request_lookup: std::collections::HashMap<u32, &'a TransferRequest>,
    /// Scouting assignments indexed by id, used for report context.
    assignment_lookup: std::collections::HashMap<u32, &'a ScoutingAssignment>,
}

/// Per-scout accumulator built up while folding monitoring rows; later
/// flattened into a `ScoutWorkloadRow`.
struct ScoutWorkAccumulator {
    staff_id: u32,
    staff_name: String,
    role_key: String,
    active: u32,
    report_ready: u32,
    confidence_sum: f32,
    confidence_count: u32,
    last_observed: Option<NaiveDate>,
    regions: std::collections::HashSet<ScoutingRegion>,
}

impl<'a> ClubScoutingDashboardBuilder<'a> {
    pub fn new(data: &'a SimulatorData, club_id: u32) -> Option<Self> {
        let club = data.club(club_id)?;
        let plan = &club.transfer_plan;

        // Walk meeting history newest-first so the first hit per player
        // wins for both votes and decisions.
        let mut latest_vote_per_player = std::collections::HashMap::new();
        let mut latest_decision_per_player = std::collections::HashMap::new();
        for meeting in plan.recruitment_meetings.iter().rev() {
            for v in &meeting.player_votes {
                latest_vote_per_player.entry(v.player_id).or_insert(v.vote);
            }
            for d in &meeting.decisions {
                latest_decision_per_player
                    .entry(d.player_id)
                    .or_insert(d.decision);
            }
        }

        let request_lookup: std::collections::HashMap<u32, &TransferRequest> =
            plan.transfer_requests.iter().map(|r| (r.id, r)).collect();
        let assignment_lookup: std::collections::HashMap<u32, &ScoutingAssignment> = plan
            .scouting_assignments
            .iter()
            .map(|a| (a.id, a))
            .collect();

        Some(ClubScoutingDashboardBuilder {
            data,
            club,
            today: data.date.date(),
            latest_vote_per_player,
            latest_decision_per_player,
            request_lookup,
            assignment_lookup,
        })
    }

    pub fn build(self) -> ClubScoutingDashboard {
        let summary = self.build_summary();
        let scout_workload = self.build_scout_workload();
        let active_monitoring = self.build_active_monitoring();
        let scouting_reports = self.build_scouting_reports();
        let (scouting_assignments, match_assignments) = self.build_assignments();
        let recruitment_meetings = self.build_recruitment_meetings();
        let known_players = self.build_known_players();
        let shadow_reports = self.build_shadow_reports();
        let transfer_requests = self.build_transfer_requests();

        ClubScoutingDashboard {
            summary,
            scout_workload,
            active_monitoring,
            scouting_reports,
            scouting_assignments,
            match_assignments,
            recruitment_meetings,
            known_players,
            shadow_reports,
            transfer_requests,
        }
    }

    fn plan(&self) -> &'a crate::transfers::pipeline::ClubTransferPlan {
        &self.club.transfer_plan
    }

    fn full_staff_name(staff: &Staff) -> String {
        format!(
            "{} {}",
            staff.full_name.first_name, staff.full_name.last_name
        )
    }

    /// Resolve `(name, slug)` for a player whose record may have moved
    /// to a retired pool. Returns `("","")` when the player can't be
    /// found at all — callers fall back to a non-linked label.
    fn resolve_player_link(&self, player_id: u32) -> (String, String) {
        if let Some((p, _)) = self.data.player_with_team(player_id) {
            return (
                format!(
                    "{} {}",
                    p.full_name.display_first_name(),
                    p.full_name.display_last_name()
                ),
                p.slug(),
            );
        }
        if let Some(p) = self.data.retired_player(player_id) {
            return (
                format!(
                    "{} {}",
                    p.full_name.display_first_name(),
                    p.full_name.display_last_name()
                ),
                p.slug(),
            );
        }
        (String::new(), String::new())
    }

    fn staff_name(&self, staff_id: u32) -> String {
        self.data
            .staff_with_team(staff_id)
            .map(|(s, _)| Self::full_staff_name(s))
            .unwrap_or_default()
    }

    fn staff_role_key(&self, staff_id: u32) -> String {
        self.data
            .staff_with_team(staff_id)
            .and_then(|(s, _)| s.contract.as_ref().map(|c| c.position.as_i18n_key()))
            .unwrap_or("staff_scout")
            .to_string()
    }

    fn build_summary(&self) -> ScoutingSummary {
        let plan = self.plan();
        let active_monitorings: Vec<_> = plan
            .scout_monitoring
            .iter()
            .filter(|m| m.is_active_interest())
            .collect();
        let active_monitored = active_monitorings.len() as u32;
        let report_ready = active_monitorings
            .iter()
            .filter(|m| matches!(m.status, ScoutMonitoringStatus::ReportReady))
            .count() as u32;
        let promoted_to_shortlist = active_monitorings
            .iter()
            .filter(|m| {
                matches!(
                    m.status,
                    ScoutMonitoringStatus::PromotedToShortlist | ScoutMonitoringStatus::Negotiating
                )
            })
            .count() as u32;
        let rejected_or_blocked = plan.rejected_players.len() as u32
            + plan
                .scout_monitoring
                .iter()
                .filter(|m| matches!(m.status, ScoutMonitoringStatus::Rejected))
                .count() as u32;
        let open_assignments = plan
            .scouting_assignments
            .iter()
            .filter(|a| !a.completed)
            .count() as u32;
        let recent_meetings = plan.recruitment_meetings.len() as u32;
        let avg_confidence_pct = if active_monitored > 0 {
            let sum: f32 = active_monitorings.iter().map(|m| m.confidence).sum();
            ((sum / active_monitored as f32) * 100.0).round().min(100.0) as u8
        } else {
            0
        };
        let mut scout_set: std::collections::HashSet<u32> = std::collections::HashSet::new();
        for m in &active_monitorings {
            scout_set.insert(m.scout_staff_id);
        }
        for a in &plan.scouting_assignments {
            if !a.completed {
                if let Some(id) = a.scout_staff_id {
                    scout_set.insert(id);
                }
            }
        }
        ScoutingSummary {
            active_scouts: scout_set.len() as u32,
            active_monitored,
            report_ready,
            open_assignments,
            recent_meetings,
            promoted_to_shortlist,
            rejected_or_blocked,
            avg_confidence_pct,
        }
    }

    fn build_scout_workload(&self) -> Vec<ScoutWorkloadRow> {
        let plan = self.plan();
        let mut workload_map: std::collections::HashMap<u32, ScoutWorkAccumulator> =
            std::collections::HashMap::new();

        // Seed every scouting-relevant staff on the club so empty rows
        // still show up in the workload table.
        for team in &self.club.teams.teams {
            for staff in team.staffs.iter() {
                let Some(contract) = &staff.contract else {
                    continue;
                };
                let position = &contract.position;
                let included = position.is_scouting()
                    || matches!(position, crate::StaffPosition::DirectorOfFootball);
                if !included {
                    continue;
                }
                workload_map
                    .entry(staff.id)
                    .or_insert_with(|| ScoutWorkAccumulator {
                        staff_id: staff.id,
                        staff_name: Self::full_staff_name(staff),
                        role_key: position.as_i18n_key().to_string(),
                        active: 0,
                        report_ready: 0,
                        confidence_sum: 0.0,
                        confidence_count: 0,
                        last_observed: None,
                        regions: std::collections::HashSet::new(),
                    });
            }
        }

        for m in plan
            .scout_monitoring
            .iter()
            .filter(|m| m.is_active_interest())
        {
            let entry =
                workload_map
                    .entry(m.scout_staff_id)
                    .or_insert_with(|| ScoutWorkAccumulator {
                        staff_id: m.scout_staff_id,
                        staff_name: self.staff_name(m.scout_staff_id),
                        role_key: self.staff_role_key(m.scout_staff_id),
                        active: 0,
                        report_ready: 0,
                        confidence_sum: 0.0,
                        confidence_count: 0,
                        last_observed: None,
                        regions: std::collections::HashSet::new(),
                    });
            entry.active += 1;
            if matches!(m.status, ScoutMonitoringStatus::ReportReady) {
                entry.report_ready += 1;
            }
            entry.confidence_sum += m.confidence;
            entry.confidence_count += 1;
            entry.last_observed = match entry.last_observed {
                Some(prev) => Some(prev.max(m.last_observed)),
                None => Some(m.last_observed),
            };
            if let Some(region) = m.region {
                entry.regions.insert(region);
            }
        }

        let mut rows: Vec<ScoutWorkloadRow> = workload_map
            .into_values()
            .map(|a| {
                let avg_confidence_pct = if a.confidence_count > 0 {
                    ((a.confidence_sum / a.confidence_count as f32) * 100.0)
                        .round()
                        .min(100.0) as u8
                } else {
                    0
                };
                let mut region_keys: Vec<String> = a
                    .regions
                    .iter()
                    .map(|r| r.as_i18n_key().to_string())
                    .collect();
                region_keys.sort();
                ScoutWorkloadRow {
                    staff_id: a.staff_id,
                    staff_name: a.staff_name,
                    role_key: a.role_key,
                    active_count: a.active,
                    report_ready_count: a.report_ready,
                    avg_confidence_pct,
                    last_observed: a.last_observed,
                    region_keys,
                }
            })
            .collect();
        rows.sort_by(|a, b| {
            b.active_count
                .cmp(&a.active_count)
                .then(b.report_ready_count.cmp(&a.report_ready_count))
                .then(a.staff_name.cmp(&b.staff_name))
        });
        rows
    }

    fn build_active_monitoring(&self) -> Vec<ActiveMonitoringRow> {
        let plan = self.plan();
        let mut rows: Vec<ActiveMonitoringRow> = plan
            .scout_monitoring
            .iter()
            .filter(|m| m.is_active_interest())
            .map(|m| {
                let player_lookup = self.data.player_with_team(m.player_id);
                let (player_name, player_slug, player_pos_short, player_age) = match player_lookup {
                    Some((p, _)) => (
                        format!(
                            "{} {}",
                            p.full_name.display_first_name(),
                            p.full_name.display_last_name()
                        ),
                        p.slug(),
                        p.position().get_short_name().to_string(),
                        Some(crate::utils::DateUtils::age(p.birth_date, self.today)),
                    ),
                    None => {
                        let (n, s) = self.resolve_player_link(m.player_id);
                        (n, s, String::new(), None)
                    }
                };
                let (current_club_id, current_club_name, current_team_slug) = match player_lookup {
                    Some((_, t)) => (
                        Some(t.club_id),
                        self.data
                            .club(t.club_id)
                            .map(|c| c.name.clone())
                            .unwrap_or_default(),
                        t.slug.clone(),
                    ),
                    None => (None, String::new(), String::new()),
                };
                ActiveMonitoringRow {
                    player_id: m.player_id,
                    player_slug,
                    player_name,
                    player_position_short: player_pos_short,
                    player_age,
                    current_club_id,
                    current_club_name,
                    current_team_slug,
                    scout_id: m.scout_staff_id,
                    scout_name: self.staff_name(m.scout_staff_id),
                    source_key: m.source.as_i18n_key().to_string(),
                    status_key: m.status.as_i18n_key().to_string(),
                    started_on: m.started_on,
                    last_observed: m.last_observed,
                    times_watched: m.times_watched,
                    matches_watched: m.matches_watched,
                    assessed_ability: m.current_assessed_ability,
                    assessed_potential: m.current_assessed_potential,
                    confidence_pct: ((m.confidence * 100.0).round().min(100.0)) as u8,
                    role_fit_pct: ((m.role_fit * 100.0).round().min(125.0)) as u8,
                    estimated_value: m.estimated_value,
                    risk_flag_keys: m
                        .risk_flags
                        .iter()
                        .map(|f| f.as_i18n_key().to_string())
                        .collect(),
                    transfer_request_id: m.transfer_request_id,
                    transfer_request_position_short: m
                        .transfer_request_id
                        .and_then(|id| self.request_lookup.get(&id))
                        .map(|r| r.position.get_short_name().to_string()),
                    latest_vote_key: self
                        .latest_vote_per_player
                        .get(&m.player_id)
                        .map(|v| v.as_i18n_key().to_string()),
                    latest_decision_key: self
                        .latest_decision_per_player
                        .get(&m.player_id)
                        .map(|d| d.as_i18n_key().to_string()),
                    sort_bucket: m.status.dashboard_sort_bucket(),
                }
            })
            .collect();
        rows.sort_by(|a, b| {
            a.sort_bucket
                .cmp(&b.sort_bucket)
                .then(b.confidence_pct.cmp(&a.confidence_pct))
                .then(b.last_observed.cmp(&a.last_observed))
        });
        rows
    }

    fn build_report_row(&self, r: &DetailedScoutingReport, from_shadow: bool) -> ScoutingReportRow {
        let player_lookup = self.data.player_with_team(r.player_id);
        let (player_name, player_slug, player_pos_short) = match player_lookup {
            Some((p, _)) => (
                format!(
                    "{} {}",
                    p.full_name.display_first_name(),
                    p.full_name.display_last_name()
                ),
                p.slug(),
                p.position().get_short_name().to_string(),
            ),
            None => {
                let (n, s) = self.resolve_player_link(r.player_id);
                (n, s, String::new())
            }
        };
        let (current_club_name, current_team_slug) = match player_lookup {
            Some((_, t)) => (
                self.data
                    .club(t.club_id)
                    .map(|c| c.name.clone())
                    .unwrap_or_default(),
                t.slug.clone(),
            ),
            None => (String::new(), String::new()),
        };
        let transfer_request_id = self
            .assignment_lookup
            .get(&r.assignment_id)
            .map(|a| a.transfer_request_id);
        ScoutingReportRow {
            player_id: r.player_id,
            player_slug,
            player_name,
            player_position_short: player_pos_short,
            current_club_name,
            current_team_slug,
            assignment_id: r.assignment_id,
            transfer_request_id,
            recommendation_key: r.recommendation.as_i18n_key().to_string(),
            assessed_ability: r.assessed_ability,
            assessed_potential: r.assessed_potential,
            confidence_pct: ((r.confidence * 100.0).round().min(100.0)) as u8,
            role_fit_pct: ((r.role_fit * 100.0).round().min(125.0)) as u8,
            estimated_value: r.estimated_value,
            risk_flag_keys: r
                .risk_flags
                .iter()
                .map(|f| f.as_i18n_key().to_string())
                .collect(),
            from_shadow,
            sort_bucket: r.recommendation.dashboard_sort_bucket(),
        }
    }

    fn build_scouting_reports(&self) -> Vec<ScoutingReportRow> {
        let plan = self.plan();
        let active_report_player_ids: std::collections::HashSet<u32> =
            plan.scouting_reports.iter().map(|r| r.player_id).collect();
        let mut rows: Vec<ScoutingReportRow> = plan
            .scouting_reports
            .iter()
            .map(|r| self.build_report_row(r, false))
            .collect();
        for sr in &plan.shadow_reports {
            if active_report_player_ids.contains(&sr.report.player_id) {
                continue;
            }
            rows.push(self.build_report_row(&sr.report, true));
        }
        rows.sort_by(|a, b| {
            a.sort_bucket
                .cmp(&b.sort_bucket)
                .then(b.confidence_pct.cmp(&a.confidence_pct))
                .then(b.assessed_ability.cmp(&a.assessed_ability))
        });
        rows
    }

    fn build_assignments(&self) -> (Vec<ScoutingAssignmentRow>, Vec<MatchAssignmentRow>) {
        let plan = self.plan();
        let mut scouting_assignments: Vec<ScoutingAssignmentRow> = plan
            .scouting_assignments
            .iter()
            .map(|a| ScoutingAssignmentRow {
                assignment_id: a.id,
                transfer_request_id: a.transfer_request_id,
                target_position_short: a.target_position.get_short_name().to_string(),
                min_ability: a.min_ability,
                preferred_age_min: a.preferred_age_min,
                preferred_age_max: a.preferred_age_max,
                max_budget: a.max_budget,
                scout_id: a.scout_staff_id,
                scout_name: a.scout_staff_id.map(|id| self.staff_name(id)),
                observation_count: a.observations.len() as u32,
                reports_produced: a.reports_produced,
                completed: a.completed,
                min_technical_avg: a.role_profile.min_technical_avg,
                min_mental_avg: a.role_profile.min_mental_avg,
                min_physical_avg: a.role_profile.min_physical_avg,
            })
            .collect();
        scouting_assignments.sort_by(|a, b| {
            a.completed
                .cmp(&b.completed)
                .then(a.transfer_request_id.cmp(&b.transfer_request_id))
                .then(a.assignment_id.cmp(&b.assignment_id))
        });

        let mut match_assignments: Vec<MatchAssignmentRow> = plan
            .scout_match_assignments
            .iter()
            .map(|m| {
                let (target_team_name, target_team_slug) = self
                    .data
                    .team(m.target_team_id)
                    .map(|t| (t.name.clone(), t.slug.clone()))
                    .unwrap_or_default();
                MatchAssignmentRow {
                    scout_id: m.scout_staff_id,
                    scout_name: self.staff_name(m.scout_staff_id),
                    target_team_id: m.target_team_id,
                    target_team_name,
                    target_team_slug,
                    linked_assignment_ids: m.linked_assignment_ids.clone(),
                    last_attended: m.last_attended,
                }
            })
            .collect();
        match_assignments.sort_by(|a, b| b.last_attended.cmp(&a.last_attended));

        (scouting_assignments, match_assignments)
    }

    fn build_recruitment_meetings(&self) -> Vec<RecruitmentMeetingRow> {
        let plan = self.plan();
        let mut rows: Vec<RecruitmentMeetingRow> = plan
            .recruitment_meetings
            .iter()
            .rev()
            .map(|mtg| {
                let participants: Vec<MeetingParticipant> = mtg
                    .participants
                    .iter()
                    .map(|sid| MeetingParticipant {
                        staff_id: *sid,
                        staff_name: self.staff_name(*sid),
                        role_key: self.staff_role_key(*sid),
                    })
                    .collect();
                let decisions: Vec<MeetingDecisionRow> = mtg
                    .decisions
                    .iter()
                    .map(|d| {
                        let (name, slug) = self.resolve_player_link(d.player_id);
                        MeetingDecisionRow {
                            player_id: d.player_id,
                            player_slug: slug,
                            player_name: name,
                            transfer_request_id: d.transfer_request_id,
                            decision_key: d.decision.as_i18n_key().to_string(),
                            consensus_score: d.consensus_score,
                            chief_scout_support: d.chief_scout_support,
                            data_support: d.data_support,
                            board_risk_score: d.board_risk_score,
                            budget_fit: d.budget_fit,
                            reason: d.reason.to_string(),
                        }
                    })
                    .collect();
                let votes: Vec<MeetingVoteRow> = mtg
                    .player_votes
                    .iter()
                    .map(|v| {
                        let (player_name, player_slug) = self.resolve_player_link(v.player_id);
                        MeetingVoteRow {
                            scout_id: v.scout_staff_id,
                            scout_name: self.staff_name(v.scout_staff_id),
                            player_id: v.player_id,
                            player_slug,
                            player_name,
                            vote_key: v.vote.as_i18n_key().to_string(),
                            score: v.score,
                            confidence_pct: ((v.confidence * 100.0).round().min(100.0)) as u8,
                            reason_key: v.reason.as_i18n_key().to_string(),
                        }
                    })
                    .collect();
                RecruitmentMeetingRow {
                    id: mtg.id,
                    date: mtg.date,
                    participants,
                    agenda_request_ids: mtg.agenda_request_ids.clone(),
                    decisions,
                    votes,
                }
            })
            .collect();
        rows.sort_by(|a, b| b.date.cmp(&a.date));
        rows
    }

    fn build_known_players(&self) -> Vec<KnownPlayerRow> {
        let plan = self.plan();
        let mut rows: Vec<KnownPlayerRow> = plan
            .known_players
            .iter()
            .map(|k| {
                let (name, slug) = self.resolve_player_link(k.player_id);
                let (last_known_club_name, last_known_team_slug) = self
                    .data
                    .club(k.last_known_club_id)
                    .map(|c| {
                        let team_slug = c
                            .teams
                            .teams
                            .iter()
                            .find(|t| t.team_type == crate::TeamType::Main)
                            .or_else(|| c.teams.teams.first())
                            .map(|t| t.slug.clone())
                            .unwrap_or_default();
                        (c.name.clone(), team_slug)
                    })
                    .unwrap_or_default();
                KnownPlayerRow {
                    player_id: k.player_id,
                    player_slug: slug,
                    player_name: name,
                    last_known_club_name,
                    last_known_team_slug,
                    position_short: k.position.get_short_name().to_string(),
                    position_group_key: k.position_group.as_i18n_key().to_string(),
                    assessed_ability: k.assessed_ability,
                    assessed_potential: k.assessed_potential,
                    confidence_pct: ((k.confidence * 100.0).round().min(100.0)) as u8,
                    estimated_fee: k.estimated_fee,
                    last_seen: k.last_seen,
                    official_appearances_seen: k.official_appearances_seen,
                    friendly_appearances_seen: k.friendly_appearances_seen,
                }
            })
            .collect();
        rows.sort_by(|a, b| b.last_seen.cmp(&a.last_seen));
        rows
    }

    fn build_shadow_reports(&self) -> Vec<ShadowReportRow> {
        let plan = self.plan();
        let mut rows: Vec<ShadowReportRow> = plan
            .shadow_reports
            .iter()
            .map(|s| {
                let (name, slug) = self.resolve_player_link(s.report.player_id);
                let (current_club_name, current_team_slug) = self
                    .data
                    .player_with_team(s.report.player_id)
                    .map(|(_, t)| {
                        let club_name = self
                            .data
                            .club(t.club_id)
                            .map(|c| c.name.clone())
                            .unwrap_or_default();
                        (club_name, t.slug.clone())
                    })
                    .unwrap_or_default();
                ShadowReportRow {
                    player_id: s.report.player_id,
                    player_slug: slug,
                    player_name: name,
                    current_club_name,
                    current_team_slug,
                    position_group_key: s.position_group.as_i18n_key().to_string(),
                    observed_ability: s.observed_ability,
                    recorded_on: s.recorded_on,
                    confidence_pct: ((s.report.confidence * 100.0).round().min(100.0)) as u8,
                    recommendation_key: s.report.recommendation.as_i18n_key().to_string(),
                }
            })
            .collect();
        rows.sort_by(|a, b| b.recorded_on.cmp(&a.recorded_on));
        rows
    }

    fn build_transfer_requests(&self) -> Vec<TransferRequestRow> {
        let plan = self.plan();
        let mut rows: Vec<TransferRequestRow> = plan
            .transfer_requests
            .iter()
            .map(|r| {
                let (named_target_slug, named_target_name) = match r.named_target {
                    Some(pid) => {
                        let (name, slug) = self.resolve_player_link(pid);
                        (
                            if slug.is_empty() { None } else { Some(slug) },
                            if name.is_empty() { None } else { Some(name) },
                        )
                    }
                    None => (None, None),
                };
                TransferRequestRow {
                    id: r.id,
                    position_short: r.position.get_short_name().to_string(),
                    priority_key: r.priority.as_i18n_key().to_string(),
                    priority_sort_bucket: r.priority.dashboard_sort_bucket(),
                    reason_key: r.reason.as_i18n_key().to_string(),
                    min_ability: r.min_ability,
                    ideal_ability: r.ideal_ability,
                    preferred_age_min: r.preferred_age_min,
                    preferred_age_max: r.preferred_age_max,
                    budget_allocation: r.budget_allocation,
                    status_key: r.status.as_i18n_key().to_string(),
                    status_sort_bucket: r.status.dashboard_sort_bucket(),
                    named_target_id: r.named_target,
                    named_target_slug,
                    named_target_name,
                    board_approved: r.board_approved,
                }
            })
            .collect();
        rows.sort_by(|a, b| {
            a.status_sort_bucket
                .cmp(&b.status_sort_bucket)
                .then(a.priority_sort_bucket.cmp(&b.priority_sort_bucket))
                .then(a.id.cmp(&b.id))
        });
        rows
    }
}
