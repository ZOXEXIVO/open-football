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
    pub match_id: String,
    pub result: Option<(u8, u8)>,
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
            rows: teams
                .iter()
                .map(|&id| GroupRow {
                    team_id: id,
                    played: 0,
                    won: 0,
                    drawn: 0,
                    lost: 0,
                    gf: 0,
                    ga: 0,
                    points: 0,
                })
                .collect(),
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

    fn record(
        &mut self,
        team_id: u32,
        gf: u8,
        ga: u8,
        pts: u8,
        won: bool,
        drawn: bool,
        lost: bool,
    ) {
        if let Some(row) = self.rows.iter_mut().find(|r| r.team_id == team_id) {
            row.played += 1;
            row.gf += gf;
            row.ga += ga;
            row.points += pts;
            if won {
                row.won += 1;
            }
            if drawn {
                row.drawn += 1;
            }
            if lost {
                row.lost += 1;
            }
        }
    }

    fn sort(&mut self) {
        self.rows.sort_by(|a, b| {
            b.points
                .cmp(&a.points)
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
    /// Optional second-leg shootout result (home_kicks, away_kicks).
    /// Set by the caller when the leg was played as a knockout fixture
    /// and the aggregate ended level. `record_leg2_with_shootout` is
    /// the canonical entry point.
    pub shootout: Option<(u8, u8)>,
    pub winner: Option<u32>,
}

impl KnockoutTie {
    pub fn new(home: u32, away: u32) -> Self {
        KnockoutTie {
            home_team: home,
            away_team: away,
            leg1_score: None,
            leg2_score: None,
            shootout: None,
            winner: None,
        }
    }

    pub fn record_leg1(&mut self, home_goals: u8, away_goals: u8) {
        self.leg1_score = Some((home_goals, away_goals));
    }

    /// Record the second leg without an explicit shootout. If aggregate
    /// is level the tie has no winner yet — the caller is expected to
    /// either replay extra time / penalties externally or call
    /// `record_leg2_with_shootout` instead. Away-goals rule is NOT
    /// applied: UEFA dropped it in 2021 and we follow modern rules.
    pub fn record_leg2(&mut self, home_goals: u8, away_goals: u8) {
        self.record_leg2_with_shootout(home_goals, away_goals, None);
    }

    /// Canonical second-leg recorder. Aggregate decides the tie when
    /// it is level after both legs. If the aggregate is tied, the
    /// caller-provided shootout result (home_kicks, away_kicks)
    /// determines the winner. When neither aggregate nor shootout
    /// breaks the tie, `winner` stays `None` so the caller can detect
    /// the missing decisive event.
    pub fn record_leg2_with_shootout(
        &mut self,
        home_goals: u8,
        away_goals: u8,
        shootout: Option<(u8, u8)>,
    ) {
        self.leg2_score = Some((home_goals, away_goals));
        self.shootout = shootout;
        if let (Some((h1, a1)), Some((h2, a2))) = (self.leg1_score, self.leg2_score) {
            // Two-leg tie: home of leg1 hosts both fixtures of legs.
            // For aggregate purposes:
            //   leg1: home goals = h1, away goals = a1
            //   leg2: home goals = h2, away goals = a2 (sides reversed)
            // Aggregate from `self.home_team`'s perspective is
            // (h1 + a2): goals scored at home in leg 1 + goals scored
            // away in leg 2.
            let agg_home = h1 as u16 + a2 as u16;
            let agg_away = a1 as u16 + h2 as u16;
            self.winner = if agg_home > agg_away {
                Some(self.home_team)
            } else if agg_away > agg_home {
                Some(self.away_team)
            } else if let Some((sh, sa)) = shootout {
                // Aggregate level → shootout decides.
                if sh > sa {
                    Some(self.home_team)
                } else if sa > sh {
                    Some(self.away_team)
                } else {
                    None
                }
            } else {
                // Aggregate level, no shootout supplied — leave winner
                // undecided. Callers should either run extra time +
                // penalties externally or call this method again with
                // the resulting shootout score.
                None
            };
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knockout_aggregate_winner_decides_when_unequal() {
        // Home wins 2-1 at home, draws 1-1 away → aggregate 3-2 home.
        let mut tie = KnockoutTie::new(1, 2);
        tie.record_leg1(2, 1);
        tie.record_leg2(1, 1);
        assert_eq!(tie.winner, Some(1));
    }

    #[test]
    fn knockout_aggregate_winner_handles_road_advantage_without_away_goals_rule() {
        // Leg 1 (team 1 hosts): team 1 wins 1-0 → h1=1, a1=0.
        // Leg 2 (team 2 hosts): team 2 wins 1-0 → h2=1 (team 2 at home),
        //                                          a2=0 (team 1 away).
        // Aggregate: team 1 = h1 + a2 = 1, team 2 = a1 + h2 = 1 → tied.
        // Without away-goals rule the tie is undecided → None.
        let mut tie = KnockoutTie::new(1, 2);
        tie.record_leg1(1, 0);
        tie.record_leg2(1, 0);
        assert_eq!(tie.winner, None);
    }

    #[test]
    fn knockout_tied_aggregate_resolves_via_shootout() {
        // Build a tied aggregate (each side wins their home leg 1-0)
        // and feed a home-favouring shootout.
        let mut tie = KnockoutTie::new(1, 2);
        tie.record_leg1(1, 0);
        tie.record_leg2_with_shootout(1, 0, Some((4, 3)));
        // Home shootout score 4 > away 3 → home wins.
        assert_eq!(tie.winner, Some(1));
    }

    #[test]
    fn knockout_tied_aggregate_resolves_via_shootout_for_visitor() {
        let mut tie = KnockoutTie::new(1, 2);
        tie.record_leg1(1, 0);
        tie.record_leg2_with_shootout(1, 0, Some((3, 5)));
        assert_eq!(tie.winner, Some(2));
    }

    #[test]
    fn knockout_no_winner_when_aggregate_level_and_no_shootout() {
        let mut tie = KnockoutTie::new(1, 2);
        tie.record_leg1(0, 0);
        tie.record_leg2(2, 2);
        assert_eq!(tie.winner, None);
        // Caller can supply a shootout later (e.g. after running ET +
        // pens externally).
        tie.record_leg2_with_shootout(2, 2, Some((5, 4)));
        assert_eq!(tie.winner, Some(1));
    }
}
