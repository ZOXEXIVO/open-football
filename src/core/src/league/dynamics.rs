use crate::r#match::{MatchResult, MatchResultOutcome, Score};
use crate::league::LeagueTable;
use log::debug;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct LeagueDynamics {
    pub team_momentum: HashMap<u32, f32>,
    pub team_streaks: HashMap<u32, TeamStreak>,
    pub title_race: TitleRace,
    pub relegation_battle: RelegationBattle,
    pub european_race: EuropeanRace,
    pub rivalries: Vec<(u32, u32)>,
    pub attendance_multiplier: f32,
}

impl LeagueDynamics {
    pub fn new() -> Self {
        LeagueDynamics {
            team_momentum: HashMap::new(),
            team_streaks: HashMap::new(),
            title_race: TitleRace::default(),
            relegation_battle: RelegationBattle::default(),
            european_race: EuropeanRace::default(),
            rivalries: Vec::new(),
            attendance_multiplier: 1.0,
        }
    }

    pub fn get_team_momentum(&self, team_id: u32) -> f32 {
        *self.team_momentum.get(&team_id).unwrap_or(&0.5)
    }

    pub fn update_team_momentum_after_match(
        &mut self,
        home_id: u32,
        away_id: u32,
        result: &MatchResult,
    ) {
        // Use true outcome (including shootouts) so knockout ties that end
        // on penalties push momentum toward the actual winner, not a draw.
        let outcome = result.score.outcome();

        let home_val = *self.team_momentum.entry(home_id).or_insert(0.5);
        let away_val = *self.team_momentum.entry(away_id).or_insert(0.5);

        let (new_home, new_away) = match outcome {
            MatchResultOutcome::HomeWin => (
                (home_val * 0.8 + 0.3).min(1.0),
                (away_val * 0.8 - 0.1).max(0.0),
            ),
            MatchResultOutcome::Draw => (
                (home_val * 0.9 + 0.05).min(1.0),
                (away_val * 0.9 + 0.05).min(1.0),
            ),
            MatchResultOutcome::AwayWin => (
                (home_val * 0.8 - 0.1).max(0.0),
                (away_val * 0.8 + 0.3).min(1.0),
            ),
        };

        self.team_momentum.insert(home_id, new_home);
        self.team_momentum.insert(away_id, new_away);
    }

    pub fn update_team_streaks(&mut self, home_id: u32, away_id: u32, score: &Score) {
        let outcome = score.outcome();

        let home_streak = self.team_streaks.entry(home_id).or_insert(TeamStreak::default());
        match outcome {
            MatchResultOutcome::HomeWin => {
                home_streak.winning_streak += 1;
                home_streak.unbeaten_streak += 1;
                home_streak.losing_streak = 0;
            }
            MatchResultOutcome::Draw => {
                home_streak.unbeaten_streak += 1;
                home_streak.winning_streak = 0;
                home_streak.losing_streak = 0;
            }
            MatchResultOutcome::AwayWin => {
                home_streak.losing_streak += 1;
                home_streak.winning_streak = 0;
                home_streak.unbeaten_streak = 0;
            }
        }

        let away_streak = self.team_streaks.entry(away_id).or_insert(TeamStreak::default());
        match outcome {
            MatchResultOutcome::AwayWin => {
                away_streak.winning_streak += 1;
                away_streak.unbeaten_streak += 1;
                away_streak.losing_streak = 0;
            }
            MatchResultOutcome::Draw => {
                away_streak.unbeaten_streak += 1;
                away_streak.winning_streak = 0;
                away_streak.losing_streak = 0;
            }
            MatchResultOutcome::HomeWin => {
                away_streak.losing_streak += 1;
                away_streak.winning_streak = 0;
                away_streak.unbeaten_streak = 0;
            }
        }
    }

    pub fn get_team_losing_streak(&self, team_id: u32) -> u8 {
        self.team_streaks.get(&team_id).map(|s| s.losing_streak).unwrap_or(0)
    }

    pub fn update_title_race(&mut self, table: &LeagueTable) {
        if table.rows.len() < 2 { return; }

        let leader_points = table.rows[0].points;
        let second_points = table.rows[1].points;

        self.title_race.leader_id = table.rows[0].team_id;
        self.title_race.gap_to_second = (leader_points - second_points) as i8;
        self.title_race.contenders = table.rows.iter()
            .take(5)
            .filter(|r| (leader_points - r.points) <= 9)
            .map(|r| r.team_id)
            .collect();
    }

    pub fn update_relegation_battle(&mut self, table: &LeagueTable, total_teams: usize) {
        if total_teams < 4 { return; }

        let relegation_zone_start = total_teams - 3;
        self.relegation_battle.teams_in_danger = table.rows.iter()
            .skip(relegation_zone_start - 2)
            .map(|r| r.team_id)
            .collect();
    }

    pub fn update_european_race(&mut self, table: &LeagueTable) {
        self.european_race.teams_in_contention = table.rows.iter()
            .take(8)
            .map(|r| r.team_id)
            .collect();
    }

    pub fn is_derby(&self, team1: u32, team2: u32) -> bool {
        self.rivalries.iter().any(|(a, b)|
            (*a == team1 && *b == team2) || (*a == team2 && *b == team1)
        )
    }

    pub fn update_attendance_predictions(
        &mut self,
        table: &LeagueTable,
        day_of_week: chrono::Weekday,
        month: u32,
    ) {
        self.attendance_multiplier = 1.0;

        if day_of_week == chrono::Weekday::Sat || day_of_week == chrono::Weekday::Sun {
            self.attendance_multiplier *= 1.2;
        }

        if month >= 6 && month <= 8 {
            self.attendance_multiplier *= 0.9;
        }

        if table.rows.first().map(|r| r.played).unwrap_or(0) > 30 {
            self.attendance_multiplier *= 1.3;
        }
    }

    pub fn assign_referees(&mut self) {
        debug!("Referees assigned for upcoming matches");
    }

    pub fn reset_for_new_season(&mut self) {
        self.team_momentum.clear();
        self.team_streaks.clear();
        self.title_race = TitleRace::default();
        self.relegation_battle = RelegationBattle::default();
        self.european_race = EuropeanRace::default();
    }
}

#[derive(Debug, Clone, Default)]
pub struct TeamStreak {
    pub winning_streak: u8,
    pub losing_streak: u8,
    pub unbeaten_streak: u8,
}

#[derive(Debug, Clone, Default)]
pub struct TitleRace {
    pub leader_id: u32,
    pub gap_to_second: i8,
    pub contenders: Vec<u32>,
}

#[derive(Debug, Clone, Default)]
pub struct RelegationBattle {
    pub teams_in_danger: Vec<u32>,
}

#[derive(Debug, Clone, Default)]
pub struct EuropeanRace {
    pub teams_in_contention: Vec<u32>,
}
