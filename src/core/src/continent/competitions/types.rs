use chrono::NaiveDate;

/// Reserved league_id values for continental competitions.
/// Used in match result processing to identify competition type.
pub const CHAMPIONS_LEAGUE_ID: u32 = 900_000_001;
pub const EUROPA_LEAGUE_ID: u32 = 900_000_002;
pub const CONFERENCE_LEAGUE_ID: u32 = 900_000_003;

#[derive(Debug, Clone)]
pub enum CompetitionStage {
    NotStarted,
    Qualifying,
    GroupStage,
    RoundOf32,
    RoundOf16,
    QuarterFinals,
    SemiFinals,
    Final,
}

#[derive(Debug, Clone)]
pub struct ContinentalMatch {
    pub home_team: u32,
    pub away_team: u32,
    pub date: NaiveDate,
    pub stage: CompetitionStage,
}

#[derive(Debug, Clone)]
pub struct ContinentalMatchResult {
    pub home_team: u32,
    pub away_team: u32,
    pub home_score: u8,
    pub away_score: u8,
    pub competition: CompetitionTier,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CompetitionTier {
    ChampionsLeague,
    EuropaLeague,
    ConferenceLeague,
}

// ─── Shared group / knockout types for all continental competitions ──

#[derive(Debug, Clone, Default)]
pub struct GroupTable {
    pub rows: Vec<GroupRow>,
}

#[derive(Debug, Clone)]
pub struct GroupRow {
    pub team_id: u32,
    pub played: u8,
    pub won: u8,
    pub drawn: u8,
    pub lost: u8,
    pub gf: u8,
    pub ga: u8,
    pub points: u8,
}

impl GroupTable {
    pub fn new(teams: &[u32]) -> Self {
        GroupTable {
            rows: teams.iter().map(|&id| GroupRow {
                team_id: id, played: 0, won: 0, drawn: 0, lost: 0,
                gf: 0, ga: 0, points: 0,
            }).collect(),
        }
    }

    pub fn update(&mut self, home_id: u32, away_id: u32, home_goals: u8, away_goals: u8) {
        use std::cmp::Ordering;
        match home_goals.cmp(&away_goals) {
            Ordering::Greater => {
                self.record(home_id, home_goals, away_goals, 3, true, false, false);
                self.record(away_id, away_goals, home_goals, 0, false, false, true);
            }
            Ordering::Less => {
                self.record(home_id, home_goals, away_goals, 0, false, false, true);
                self.record(away_id, away_goals, home_goals, 3, true, false, false);
            }
            Ordering::Equal => {
                self.record(home_id, home_goals, away_goals, 1, false, true, false);
                self.record(away_id, away_goals, home_goals, 1, false, true, false);
            }
        }
        self.sort();
    }

    fn record(&mut self, team_id: u32, gf: u8, ga: u8, pts: u8, won: bool, drawn: bool, lost: bool) {
        if let Some(row) = self.rows.iter_mut().find(|r| r.team_id == team_id) {
            row.played += 1;
            row.gf += gf;
            row.ga += ga;
            row.points += pts;
            if won { row.won += 1; }
            if drawn { row.drawn += 1; }
            if lost { row.lost += 1; }
        }
    }

    fn sort(&mut self) {
        self.rows.sort_by(|a, b| {
            b.points.cmp(&a.points)
                .then_with(|| (b.gf as i16 - b.ga as i16).cmp(&(a.gf as i16 - a.ga as i16)))
                .then_with(|| b.gf.cmp(&a.gf))
        });
    }

    /// Top 2 teams qualify for knockout
    pub fn qualifiers(&self) -> (u32, u32) {
        (self.rows[0].team_id, self.rows[1].team_id)
    }
}

#[derive(Debug, Clone)]
pub struct KnockoutTie {
    pub home_team: u32,
    pub away_team: u32,
    pub leg1_score: Option<(u8, u8)>,
    pub leg2_score: Option<(u8, u8)>,
    pub winner: Option<u32>,
}

impl KnockoutTie {
    pub fn new(home: u32, away: u32) -> Self {
        KnockoutTie { home_team: home, away_team: away, leg1_score: None, leg2_score: None, winner: None }
    }

    pub fn record_leg1(&mut self, home_goals: u8, away_goals: u8) {
        self.leg1_score = Some((home_goals, away_goals));
    }

    pub fn record_leg2(&mut self, home_goals: u8, away_goals: u8) {
        self.leg2_score = Some((home_goals, away_goals));
        if let (Some((h1, a1)), Some((h2, a2))) = (self.leg1_score, self.leg2_score) {
            let agg_home = h1 as u16 + a2 as u16;
            let agg_away = a1 as u16 + h2 as u16;
            self.winner = Some(if agg_home > agg_away {
                self.home_team
            } else if agg_away > agg_home {
                self.away_team
            } else {
                if h2 > a1 { self.away_team } else { self.home_team }
            });
        }
    }
}

#[derive(Debug, Clone)]
pub struct TransferInterest {
    pub player_id: u32,
    pub source_country: u32,
    pub interest_level: f32,
}

#[derive(Debug, Clone)]
pub struct TransferNegotiation {
    pub player_id: u32,
    pub selling_club: u32,
    pub buying_club: u32,
    pub current_offer: f64,
}
