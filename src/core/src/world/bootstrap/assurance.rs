//! Every player's assurance — the standard of football he is used to — set
//! from the career he arrives with, or from his place in the side he joins.

use super::ClubIdentity;
use crate::club::player::mind::MindSwitch;
use crate::club::player::statistics::PlayerStatisticsHistoryItem;
use crate::world::SimulatorData;
use rayon::prelude::*;
use std::collections::HashMap;

/// The standard of football each club a career passes through played at.
///
/// A club in the world reads its own squad; one that is not reads its
/// league's mean; one whose league is not here either reads a fit of
/// standard on team reputation taken from the world's own clubs.
pub struct ClubStandardTable {
    by_team: HashMap<String, f32>,
    by_league: HashMap<String, f32>,
    intercept: f32,
    slope: f32,
}

impl ClubStandardTable {
    pub fn build(data: &SimulatorData) -> Self {
        let mut by_team = HashMap::new();
        let mut leagues: HashMap<String, (f32, u32)> = HashMap::new();
        let (mut n, mut sx, mut sy, mut sxx, mut sxy) = (0.0f64, 0.0f64, 0.0f64, 0.0f64, 0.0f64);
        for country in data.continents.iter().flat_map(|c| &c.countries) {
            let league_lookup = ClubIdentity::league_lookup(country);
            for team in country.clubs.iter().flat_map(|c| c.teams.iter()) {
                let standard = team.playing_standard;
                by_team.insert(team.slug.clone(), standard);
                if let Some((_, slug)) = team.league_id.and_then(|id| league_lookup.get(&id)) {
                    let e = leagues.entry(slug.clone()).or_insert((0.0, 0));
                    e.0 += standard;
                    e.1 += 1;
                }
                let x = team.reputation.world as f64;
                let y = standard as f64;
                n += 1.0;
                sx += x;
                sy += y;
                sxx += x * x;
                sxy += x * y;
            }
        }
        let by_league = leagues
            .into_iter()
            .map(|(slug, (sum, count))| (slug, sum / count as f32))
            .collect();
        let var = n * sxx - sx * sx;
        let slope = if var > 0.0 { (n * sxy - sx * sy) / var } else { 0.0 };
        let intercept = if n > 0.0 { (sy - slope * sx) / n } else { 0.0 };
        ClubStandardTable {
            by_team,
            by_league,
            intercept: intercept as f32,
            slope: slope as f32,
        }
    }

    pub fn standard_of(&self, item: &PlayerStatisticsHistoryItem) -> f32 {
        self.by_team
            .get(&item.team_slug)
            .or_else(|| self.by_league.get(&item.league_slug))
            .copied()
            .unwrap_or(self.intercept + self.slope * item.team_reputation as f32)
    }
}

impl SimulatorData {
    /// Seed every rostered player's assurance once, at world construction:
    /// from his recorded career where he has one, else from his squad role.
    /// Free agents with a career are seeded from it; the rest are caught by
    /// [`Self::seed_missing_player_assurance`] when they join a side.
    pub(in crate::world) fn seed_player_assurance(&mut self) {
        if !MindSwitch::armed() {
            return;
        }
        self.continents
            .par_iter_mut()
            .flat_map(|continent| continent.countries.par_iter_mut())
            .for_each(|country| {
                for team in country.clubs.iter_mut().flat_map(|c| c.teams.iter_mut()) {
                    team.refresh_playing_standard();
                }
            });
        let table = ClubStandardTable::build(self);
        self.continents
            .par_iter_mut()
            .flat_map(|continent| continent.countries.par_iter_mut())
            .for_each(|country| {
                for team in country.clubs.iter_mut().flat_map(|c| c.teams.iter_mut()) {
                    let standard = team.playing_standard;
                    for player in &mut team.players.players {
                        if !player.seed_assurance_from_career(|item| table.standard_of(item)) {
                            player.seed_assurance_from_role(standard);
                        }
                    }
                }
            });
        for player in &mut self.free_agents {
            player.seed_assurance_from_career(|item| table.standard_of(item));
        }
    }

    /// Seed any rostered player whose assurance was never set — an academy
    /// graduate, a regen, a free agent with no recorded career who has just
    /// signed — from his place in the side. Skip-fast: the steady state is
    /// one flag read per player.
    pub fn seed_missing_player_assurance(&mut self) {
        if !MindSwitch::armed() {
            return;
        }
        self.continents
            .par_iter_mut()
            .flat_map(|continent| continent.countries.par_iter_mut())
            .for_each(|country| {
                for team in country.clubs.iter_mut().flat_map(|c| c.teams.iter_mut()) {
                    let standard = team.playing_standard;
                    for player in &mut team.players.players {
                        if !player.mind.competitive.is_seeded() {
                            player.seed_assurance_from_role(standard);
                        }
                    }
                }
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::club::player::builder::PlayerBuilder;
    use crate::club::player::statistics::PlayerStatistics;
    use crate::league::Season;
    use crate::shared::fullname::FullName;
    use crate::{
        PersonAttributes, Player, PlayerAttributes, PlayerPosition, PlayerPositionType,
        PlayerPositions, PlayerSkills,
    };
    use chrono::NaiveDate;

    fn table() -> ClubStandardTable {
        ClubStandardTable {
            by_team: HashMap::from([("top-club".to_string(), 0.72), ("second-tier".to_string(), 0.58)]),
            by_league: HashMap::from([("abroad-league".to_string(), 0.64)]),
            intercept: 0.30,
            slope: 0.0001,
        }
    }

    fn season(year: u16, team: &str, league: &str, starts: u16, subs: u16) -> PlayerStatisticsHistoryItem {
        PlayerStatisticsHistoryItem {
            season: Season::new(year),
            team_name: team.to_string(),
            team_slug: team.to_string(),
            team_reputation: 3000,
            league_name: league.to_string(),
            league_slug: league.to_string(),
            is_loan: false,
            transfer_fee: None,
            statistics: PlayerStatistics {
                played: starts,
                played_subs: subs,
                ..PlayerStatistics::default()
            },
            seq_id: year as u32,
        }
    }

    fn keeper(age_at_2026: i32) -> Player {
        PlayerBuilder::new()
            .id(1)
            .full_name(FullName::new("Test".to_string(), "Keeper".to_string()))
            .birth_date(NaiveDate::from_ymd_opt(2026 - age_at_2026, 3, 1).unwrap())
            .country_id(1)
            .attributes(PersonAttributes::default())
            .skills(PlayerSkills::default())
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: PlayerPositionType::Goalkeeper,
                    level: 20,
                }],
            })
            .player_attributes(PlayerAttributes::default())
            .build()
            .unwrap()
    }

    fn seed(player: &mut Player) -> f32 {
        let t = table();
        assert!(player.seed_assurance_from_career(|item| t.standard_of(item)));
        player.mind.competitive.assurance()
    }

    #[test]
    fn a_veteran_starts_assured() {
        let mut p = keeper(34);
        for year in 2015..2025 {
            p.statistics_history.items.push(season(year, "top-club", "x", 32, 1));
        }
        assert!((seed(&mut p) - 0.72).abs() < 0.01);
    }

    #[test]
    fn a_third_choice_with_no_senior_football_starts_unassured() {
        let mut p = keeper(19);
        for year in 2022..2025 {
            p.statistics_history.items.push(season(year, "top-club", "x", 0, 0));
        }
        assert!(seed(&mut p) < 0.72 - 0.10);
    }

    #[test]
    fn a_returning_loanee_starts_at_the_loan_level() {
        let mut p = keeper(23);
        p.statistics_history.items.push(season(2021, "top-club", "x", 2, 1));
        p.statistics_history.items.push(season(2022, "top-club", "x", 1, 0));
        p.statistics_history.items.push(season(2023, "second-tier", "y", 34, 0));
        p.statistics_history.items.push(season(2024, "second-tier", "y", 36, 0));
        let a = seed(&mut p);
        assert!((a - 0.58).abs() < (a - 0.72).abs());
    }

    #[test]
    fn a_player_with_no_record_is_seeded_by_his_place_in_the_side() {
        let mut p = keeper(17);
        assert!(!p.seed_assurance_from_career(|_| 0.70));
        assert!(!p.mind.competitive.is_seeded());
        p.seed_assurance_from_role(0.52);
        assert!(p.mind.competitive.is_seeded());
        assert!(p.mind.competitive.assurance() <= 0.52);
        assert!(p.mind.competitive.assurance() > 0.52 - 0.10);
    }

    #[test]
    fn a_club_out_of_the_world_reads_its_league_then_its_reputation() {
        let t = table();
        assert_eq!(t.standard_of(&season(2020, "top-club", "x", 0, 0)), 0.72);
        assert_eq!(t.standard_of(&season(2020, "gone", "abroad-league", 0, 0)), 0.64);
        assert!((t.standard_of(&season(2020, "gone", "gone", 0, 0)) - 0.60).abs() < 1e-6);
    }
}
