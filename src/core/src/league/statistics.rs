use crate::Club;
use crate::league::LeagueTable;
use crate::r#match::MatchResult;
use log::debug;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct LeagueStatistics {
    pub total_goals: u32,
    pub total_matches: u32,
    pub top_scorer: Option<(u32, u16)>,
    pub top_assists: Option<(u32, u16)>,
    pub clean_sheets: HashMap<u32, u16>,
    pub competitive_balance_index: f32,
    pub average_attendance: u32,
    pub highest_scoring_match: Option<(u32, u32, u8, u8)>,
    pub biggest_win: Option<(u32, u32, u8)>,
    pub longest_unbeaten_run: Option<(u32, u8)>,
}

impl LeagueStatistics {
    pub fn new() -> Self {
        LeagueStatistics {
            total_goals: 0,
            total_matches: 0,
            top_scorer: None,
            top_assists: None,
            clean_sheets: HashMap::new(),
            competitive_balance_index: 1.0,
            average_attendance: 0,
            highest_scoring_match: None,
            biggest_win: None,
            longest_unbeaten_run: None,
        }
    }

    pub fn process_match_result(&mut self, result: &MatchResult) {
        let home_goals = result.score.home_team.get();
        let away_goals = result.score.away_team.get();

        self.total_goals += (home_goals + away_goals) as u32;
        self.total_matches += 1;

        let total_in_match = home_goals + away_goals;
        if let Some((_, _, _, current_high)) = self.highest_scoring_match {
            if total_in_match > current_high {
                self.highest_scoring_match = Some((
                    result.score.home_team.team_id,
                    result.score.away_team.team_id,
                    home_goals,
                    away_goals,
                ));
            }
        } else {
            self.highest_scoring_match = Some((
                result.score.home_team.team_id,
                result.score.away_team.team_id,
                home_goals,
                away_goals,
            ));
        }

        let goal_diff = (home_goals as i8 - away_goals as i8).abs() as u8;
        if goal_diff > 0 {
            if let Some((_, _, current_biggest)) = self.biggest_win {
                if goal_diff > current_biggest {
                    let (winner, loser) = if home_goals > away_goals {
                        (
                            result.score.home_team.team_id,
                            result.score.away_team.team_id,
                        )
                    } else {
                        (
                            result.score.away_team.team_id,
                            result.score.home_team.team_id,
                        )
                    };
                    self.biggest_win = Some((winner, loser, goal_diff));
                }
            } else {
                let (winner, loser) = if home_goals > away_goals {
                    (
                        result.score.home_team.team_id,
                        result.score.away_team.team_id,
                    )
                } else {
                    (
                        result.score.away_team.team_id,
                        result.score.home_team.team_id,
                    )
                };
                self.biggest_win = Some((winner, loser, goal_diff));
            }
        }
    }

    /// Refresh top-scorer / top-assist / clean-sheet rankings from clubs.
    /// `league_id` confines the candidate set to teams that compete in
    /// this league — without the gate, a country with multiple divisions
    /// would award the lower-tier top scorer to the upper-tier league.
    pub fn update_player_rankings(&mut self, league_id: u32, clubs: &[Club]) {
        let mut scorer_stats: HashMap<u32, u16> = HashMap::new();
        let mut assist_stats: HashMap<u32, u16> = HashMap::new();

        for club in clubs {
            for team in &club.teams.teams {
                if team.league_id != Some(league_id) {
                    continue;
                }
                for player in &team.players.players {
                    if player.statistics.goals > 0 {
                        scorer_stats.insert(player.id, player.statistics.goals);
                    }
                    if player.statistics.assists > 0 {
                        assist_stats.insert(player.id, player.statistics.assists);
                    }

                    if player.positions.is_goalkeeper() && player.statistics.played > 0 {
                        self.clean_sheets.insert(player.id, 0);
                    }
                }
            }
        }

        // Deterministic tiebreak by lower id so a recompute at a fixed
        // game state always names the same winner.
        self.top_scorer = scorer_stats
            .iter()
            .max_by(|(la, ga), (lb, gb)| ga.cmp(gb).then(lb.cmp(la)))
            .map(|(id, goals)| (*id, *goals));

        self.top_assists = assist_stats
            .iter()
            .max_by(|(la, aa), (lb, ab)| aa.cmp(ab).then(lb.cmp(la)))
            .map(|(id, assists)| (*id, *assists));
    }

    pub fn update_competitive_balance(&mut self, table: &LeagueTable) {
        if table.rows.len() < 2 {
            self.competitive_balance_index = 1.0;
            return;
        }

        let mean_points =
            table.rows.iter().map(|r| r.points as f32).sum::<f32>() / table.rows.len() as f32;

        let variance = table
            .rows
            .iter()
            .map(|r| {
                let diff = r.points as f32 - mean_points;
                diff * diff
            })
            .sum::<f32>()
            / table.rows.len() as f32;

        let std_dev = variance.sqrt();
        self.competitive_balance_index = 1.0 / (1.0 + std_dev / 10.0);
    }

    pub fn archive_season_stats(&mut self) {
        debug!("📊 Season Statistics Archived:");
        debug!("  Total Goals: {}", self.total_goals);
        debug!("  Total Matches: {}", self.total_matches);
        debug!(
            "  Goals per Match: {:.2}",
            self.total_goals as f32 / self.total_matches.max(1) as f32
        );
        debug!(
            "  Competitive Balance: {:.2}",
            self.competitive_balance_index
        );

        if let Some((player_id, goals)) = self.top_scorer {
            debug!("  Top Scorer: Player {} with {} goals", player_id, goals);
        }

        self.total_goals = 0;
        self.total_matches = 0;
        self.top_scorer = None;
        self.top_assists = None;
        self.clean_sheets.clear();
        self.highest_scoring_match = None;
        self.biggest_win = None;
        self.longest_unbeaten_run = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::academy::ClubAcademy;
    use crate::club::player::builder::PlayerBuilder;
    use crate::shared::Location;
    use crate::shared::fullname::FullName;
    use crate::{
        ClubColors, ClubFinances, ClubStatus, PersonAttributes, PlayerAttributes,
        PlayerCollection, PlayerPosition, PlayerPositionType, PlayerPositions, PlayerSkills,
        StaffCollection, TeamBuilder, TeamCollection, TeamReputation, TeamType, TrainingSchedule,
    };
    use chrono::{NaiveDate, NaiveTime};

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn make_player(id: u32, goals: u16, assists: u16) -> crate::Player {
        let mut p = PlayerBuilder::new()
            .id(id)
            .full_name(FullName::new("F".into(), format!("L{}", id)))
            .birth_date(d(1995, 1, 1))
            .country_id(1)
            .attributes(PersonAttributes::default())
            .skills(PlayerSkills::default())
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: PlayerPositionType::Striker,
                    level: 20,
                }],
            })
            .player_attributes(PlayerAttributes::default())
            .build()
            .unwrap();
        p.statistics.played = 25;
        p.statistics.goals = goals;
        p.statistics.assists = assists;
        p
    }

    fn make_team(id: u32, club_id: u32, league_id: u32, players: Vec<crate::Player>) -> crate::Team {
        TeamBuilder::new()
            .id(id)
            .league_id(Some(league_id))
            .club_id(club_id)
            .name(format!("Team{}", id))
            .slug(format!("team-{}", id))
            .team_type(TeamType::Main)
            .players(PlayerCollection::new(players))
            .staffs(StaffCollection::new(Vec::new()))
            .reputation(TeamReputation::new(100, 100, 200))
            .training_schedule(TrainingSchedule::new(
                NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
            ))
            .build()
            .unwrap()
    }

    fn make_club(id: u32, teams: Vec<crate::Team>) -> Club {
        Club::new(
            id,
            format!("Club{}", id),
            Location::new(1),
            ClubFinances::new(1_000_000, Vec::new()),
            ClubAcademy::new(3),
            ClubStatus::Professional,
            ClubColors::default(),
            TeamCollection::new(teams),
            crate::ClubFacilities::default(),
        )
    }

    /// Two divisions in the same country: a Tier-2 striker has more
    /// goals than the Tier-1 leader. Tier-1's `top_scorer` must NOT be
    /// the Tier-2 player — `update_player_rankings` is gated by
    /// `league_id`.
    #[test]
    fn top_scorer_does_not_cross_divisions_in_same_country() {
        const TIER1: u32 = 1;
        const TIER2: u32 = 2;
        // Tier-1 striker: 20 goals (the legitimate leader).
        let p_tier1 = make_player(101, 20, 4);
        // Tier-2 striker: 30 goals — would otherwise win the Tier-1 race.
        let p_tier2 = make_player(202, 30, 6);
        let club_a = make_club(10, vec![make_team(1000, 10, TIER1, vec![p_tier1])]);
        let club_b = make_club(20, vec![make_team(2000, 20, TIER2, vec![p_tier2])]);
        let clubs = vec![club_a, club_b];

        let mut stats = LeagueStatistics::new();
        stats.update_player_rankings(TIER1, &clubs);
        assert_eq!(stats.top_scorer.map(|(id, _)| id), Some(101));
        assert_eq!(stats.top_assists.map(|(id, _)| id), Some(101));

        let mut stats2 = LeagueStatistics::new();
        stats2.update_player_rankings(TIER2, &clubs);
        assert_eq!(stats2.top_scorer.map(|(id, _)| id), Some(202));
    }
}
