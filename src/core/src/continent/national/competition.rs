use chrono::NaiveDate;

/// Phase of a national team competition cycle
#[derive(Debug, Clone, PartialEq)]
pub enum CompetitionPhase {
    NotStarted,
    Qualifying,
    QualifyingPlayoff,
    GroupStage,
    Knockout,
    Completed,
}

/// A qualifying group for World Cup or European Championship qualifying
#[derive(Debug, Clone)]
pub struct QualifyingGroup {
    pub id: u8,
    pub team_country_ids: Vec<u32>,
    pub standings: Vec<GroupStanding>,
    pub fixtures: Vec<GroupFixture>,
}

impl QualifyingGroup {
    pub fn new(id: u8, team_country_ids: Vec<u32>) -> Self {
        let standings = team_country_ids
            .iter()
            .map(|&country_id| GroupStanding::new(country_id))
            .collect();

        QualifyingGroup {
            id,
            team_country_ids,
            standings,
            fixtures: Vec::new(),
        }
    }

    /// Update standings after a match result
    pub fn update_standings(
        &mut self,
        home_country_id: u32,
        away_country_id: u32,
        home_score: u8,
        away_score: u8,
    ) {
        // Update home team
        if let Some(home) = self
            .standings
            .iter_mut()
            .find(|s| s.country_id == home_country_id)
        {
            home.played += 1;
            home.goals_for += home_score as u16;
            home.goals_against += away_score as u16;
            if home_score > away_score {
                home.won += 1;
                home.points += 3;
            } else if home_score == away_score {
                home.drawn += 1;
                home.points += 1;
            } else {
                home.lost += 1;
            }
        }

        // Update away team
        if let Some(away) = self
            .standings
            .iter_mut()
            .find(|s| s.country_id == away_country_id)
        {
            away.played += 1;
            away.goals_for += away_score as u16;
            away.goals_against += home_score as u16;
            if away_score > home_score {
                away.won += 1;
                away.points += 3;
            } else if away_score == home_score {
                away.drawn += 1;
                away.points += 1;
            } else {
                away.lost += 1;
            }
        }

        // Sort standings: points desc, goal difference desc, goals for desc
        self.standings.sort_by(|a, b| {
            b.points
                .cmp(&a.points)
                .then_with(|| b.goal_difference().cmp(&a.goal_difference()))
                .then_with(|| b.goals_for.cmp(&a.goals_for))
        });
    }

    /// Get the group winner (first place)
    pub fn winner(&self) -> Option<u32> {
        self.standings.first().map(|s| s.country_id)
    }

    /// Get the runner-up (second place)
    pub fn runner_up(&self) -> Option<u32> {
        self.standings.get(1).map(|s| s.country_id)
    }

    /// Check if all fixtures in the group have been played
    pub fn is_complete(&self) -> bool {
        self.fixtures.iter().all(|f| f.result.is_some())
    }
}

/// Standing of a team within a qualifying group
#[derive(Debug, Clone)]
pub struct GroupStanding {
    pub country_id: u32,
    pub played: u8,
    pub won: u8,
    pub drawn: u8,
    pub lost: u8,
    pub goals_for: u16,
    pub goals_against: u16,
    pub points: u8,
}

impl GroupStanding {
    pub fn new(country_id: u32) -> Self {
        GroupStanding {
            country_id,
            played: 0,
            won: 0,
            drawn: 0,
            lost: 0,
            goals_for: 0,
            goals_against: 0,
            points: 0,
        }
    }

    pub fn goal_difference(&self) -> i16 {
        self.goals_for as i16 - self.goals_against as i16
    }
}

/// A single fixture in a qualifying group
#[derive(Debug, Clone)]
pub struct GroupFixture {
    pub matchday: u8,
    pub date: NaiveDate,
    pub home_country_id: u32,
    pub away_country_id: u32,
    pub result: Option<FixtureResult>,
}

/// Result of a group stage or qualifying fixture
#[derive(Debug, Clone)]
pub struct FixtureResult {
    pub home_score: u8,
    pub away_score: u8,
}

/// A knockout bracket round
#[derive(Debug, Clone)]
pub struct KnockoutBracket {
    pub round: KnockoutRound,
    pub fixtures: Vec<KnockoutFixture>,
}

impl KnockoutBracket {
    pub fn new(round: KnockoutRound) -> Self {
        KnockoutBracket {
            round,
            fixtures: Vec::new(),
        }
    }

    pub fn is_complete(&self) -> bool {
        self.fixtures.iter().all(|f| f.result.is_some())
    }

    /// Get the winners of all fixtures in this bracket
    pub fn winners(&self) -> Vec<u32> {
        self.fixtures
            .iter()
            .filter_map(|f| {
                f.result
                    .as_ref()
                    .map(|r| r.winner(f.home_country_id, f.away_country_id))
            })
            .collect()
    }
}

/// Knockout round type
#[derive(Debug, Clone, PartialEq)]
pub enum KnockoutRound {
    RoundOf16,
    QuarterFinals,
    SemiFinals,
    ThirdPlace,
    Final,
}

/// A single knockout fixture
#[derive(Debug, Clone)]
pub struct KnockoutFixture {
    pub date: NaiveDate,
    pub home_country_id: u32,
    pub away_country_id: u32,
    pub result: Option<KnockoutResult>,
}

/// Result of a knockout match, including potential penalty winner
#[derive(Debug, Clone)]
pub struct KnockoutResult {
    pub home_score: u8,
    pub away_score: u8,
    pub penalty_winner: Option<u32>,
}

impl KnockoutResult {
    /// Determine the winner of a knockout match
    pub fn winner(&self, home_country_id: u32, away_country_id: u32) -> u32 {
        if let Some(pw) = self.penalty_winner {
            return pw;
        }
        if self.home_score > self.away_score {
            home_country_id
        } else {
            away_country_id
        }
    }
}
