use super::{
    CHAMPIONS_LEAGUE_ID, CHAMPIONS_LEAGUE_SLUG, CONFERENCE_LEAGUE_ID, CONFERENCE_LEAGUE_SLUG,
    COPA_LIBERTADORES_ID, COPA_LIBERTADORES_SLUG, CompetitionStage, CompetitionTier,
    ContinentalCompetitions, ContinentalMatch, EUROPA_LEAGUE_ID, EUROPA_LEAGUE_SLUG, GroupTable,
    KnockoutTie,
};
use crate::club::finance::RevenueModel;
use crate::club::team::{FixtureRun, NearestFixtures};
use crate::continent::ContinentalRegulations;
use crate::league::simulation::context::MatchdaySide;
use crate::league::simulation::matchday::{MatchdayCommitments, MatchdayPool};
use crate::league::{KnockoutStakes, KnockoutTieReading, TableLine, TableStakes, TableStanding};
use crate::r#match::{CompetitionKind, FixtureContext, Match, SelectionCompetition};
use crate::{Club, Team};
use chrono::NaiveDate;
use rustc_hash::FxHashMap;

impl CompetitionTier {
    pub fn league_id(&self) -> u32 {
        match self {
            CompetitionTier::ChampionsLeague => CHAMPIONS_LEAGUE_ID,
            CompetitionTier::EuropaLeague => EUROPA_LEAGUE_ID,
            CompetitionTier::ConferenceLeague => CONFERENCE_LEAGUE_ID,
            CompetitionTier::CopaLibertadores => COPA_LIBERTADORES_ID,
        }
    }

    /// What finishing inside this competition's places is worth against a
    /// title, as a line of a league table.
    pub fn table_weight(&self) -> f32 {
        match self {
            CompetitionTier::ChampionsLeague | CompetitionTier::CopaLibertadores => 0.95,
            CompetitionTier::EuropaLeague => 0.85,
            CompetitionTier::ConferenceLeague => 0.8,
        }
    }

    pub fn slug(&self) -> &'static str {
        match self {
            CompetitionTier::ChampionsLeague => CHAMPIONS_LEAGUE_SLUG,
            CompetitionTier::EuropaLeague => EUROPA_LEAGUE_SLUG,
            CompetitionTier::ConferenceLeague => CONFERENCE_LEAGUE_SLUG,
            CompetitionTier::CopaLibertadores => COPA_LIBERTADORES_SLUG,
        }
    }

    fn match_prefix(&self) -> &'static str {
        match self {
            CompetitionTier::ChampionsLeague => "cl",
            CompetitionTier::EuropaLeague => "el",
            CompetitionTier::ConferenceLeague => "conf",
            CompetitionTier::CopaLibertadores => "lib",
        }
    }

    fn continent(&self) -> u32 {
        match self {
            CompetitionTier::CopaLibertadores => FixtureContext::SOUTH_AMERICA,
            _ => FixtureContext::EUROPE,
        }
    }
}

impl CompetitionStage {
    /// Where the stage sits in the competition, the group stage first.
    fn round(&self) -> u8 {
        match self {
            CompetitionStage::NotStarted
            | CompetitionStage::Qualifying
            | CompetitionStage::GroupStage => 1,
            CompetitionStage::RoundOf32 => 2,
            CompetitionStage::RoundOf16 => 3,
            CompetitionStage::QuarterFinals => 4,
            CompetitionStage::SemiFinals => 5,
            CompetitionStage::Final => 6,
        }
    }

    /// Level after ninety minutes, the tie goes on to extra time and
    /// penalties.
    fn is_decided_on_the_night(&self) -> bool {
        matches!(
            self,
            CompetitionStage::RoundOf16
                | CompetitionStage::QuarterFinals
                | CompetitionStage::SemiFinals
                | CompetitionStage::Final
        )
    }
}

/// One continental tie and the state of its competition: what the tie is
/// worth to each side, and the match built from it the way a domestic
/// fixture is built.
pub(crate) struct ContinentalFixture<'a> {
    pub tier: CompetitionTier,
    pub tie: &'a ContinentalMatch,
    pub groups: &'a [GroupTable],
    pub knockout: &'a [KnockoutTie],
}

impl ContinentalFixture<'_> {
    /// Matches each side plays in its group, and the places that go through.
    const GROUP_MATCHES: u16 = 6;
    const QUALIFYING_PLACES: usize = 2;

    /// What the tie is worth to the home side and to the away side.
    pub(crate) fn stakes(&self, home: &Team, away: &Team) -> (f32, f32) {
        let tie = self.tie;
        if !self.tie.stage.is_decided_on_the_night() && tie.stage.round() == 1 {
            return self.group_stakes();
        }
        // A second leg is the first leg's pairing turned round.
        let home_lead = self
            .knockout
            .iter()
            .find(|first| first.home_team == tie.away_team && first.away_team == tie.home_team)
            .and_then(|first| first.leg1_score)
            .map(|(first_home, first_away)| i16::from(first_away) - i16::from(first_home));
        let home_strength = f32::from(home.reputation.market_value_score().max(1));
        let away_strength = f32::from(away.reputation.market_value_score().max(1));
        let reading = |opponent_ratio: f32, aggregate_lead: Option<i16>| {
            KnockoutStakes::of(KnockoutTieReading {
                round: tie.stage.round(),
                total_rounds: CompetitionStage::Final.round(),
                opponent_ratio,
                aggregate_lead,
            })
        };
        (
            reading(away_strength / home_strength, home_lead),
            reading(home_strength / away_strength, home_lead.map(|lead| -lead)),
        )
    }

    fn group_stakes(&self) -> (f32, f32) {
        let tie = self.tie;
        let Some(group) = self.groups.iter().find(|group| {
            group.rows.iter().any(|r| r.team_id == tie.home_team)
                && group.rows.iter().any(|r| r.team_id == tie.away_team)
        }) else {
            return (TableStakes::ROUTINE, TableStakes::ROUTINE);
        };
        let standings: Vec<TableStanding> = group
            .rows
            .iter()
            .map(|row| TableStanding {
                team_id: row.team_id,
                points: i32::from(row.points),
            })
            .collect();
        let lines = [TableLine::below(
            Self::QUALIFYING_PLACES,
            TableLine::PROMOTION,
        )];
        let of = |club_id: u32| {
            let played = group
                .rows
                .iter()
                .find(|row| row.team_id == club_id)
                .map(|row| u16::from(row.played))
                .unwrap_or(0);
            TableStakes::of(
                &standings,
                club_id,
                played,
                Self::GROUP_MATCHES.saturating_sub(played),
                &lines,
            )
        };
        (of(tie.home_team), of(tie.away_team))
    }

    pub(crate) fn build(
        &self,
        clubs: &FxHashMap<u32, &Club>,
        date: NaiveDate,
        commitments: &MatchdayCommitments,
    ) -> Option<Match> {
        let tie = self.tie;
        let home_club = clubs.get(&tie.home_team)?;
        let away_club = clubs.get(&tie.away_team)?;
        let home_team = home_club.teams.main()?;
        let away_team = away_club.teams.main()?;

        let is_derby = home_club.is_rival(away_club.id) || away_club.is_rival(home_club.id);
        let (home_stakes, away_stakes) = self.stakes(home_team, away_team);
        let side = |team, club, opponent, home: bool, stakes: f32| MatchdaySide {
            team,
            club: Some(club),
            opponent,
            home,
            stakes: if is_derby {
                TableStakes::with_rivalry(stakes)
            } else {
                stakes
            },
            competition: SelectionCompetition::ContinentalCup,
            derby: is_derby,
            friendly: false,
            date,
            season_matches_played: None,
            cup_slug: Some(self.tier.slug()),
            bench_size: ContinentalRegulations::BENCH_SIZE,
        };
        let home_side = side(home_team, *home_club, away_team, true, home_stakes);
        let away_side = side(away_team, *away_club, home_team, false, away_stakes);

        // Same matchday pool the league builds — reserves, earned academy
        // call-ups, the keeper room and the shortfall borrow. A club in a
        // crisis names the same eighteen in Europe that it names on a
        // Saturday.
        let home_pool =
            MatchdayPool::offer(home_club, home_team.id, false, true, date, commitments);
        let away_pool =
            MatchdayPool::offer(away_club, away_team.id, false, true, date, commitments);
        let home_squad =
            home_team.get_enhanced_match_squad(&home_pool, &home_side.selection_context());
        let away_squad =
            away_team.get_enhanced_match_squad(&away_pool, &away_side.selection_context());

        let match_id = format!(
            "{}_{}_{}_{}",
            self.tier.match_prefix(),
            date.format("%Y%m%d"),
            tie.home_team,
            tie.away_team
        );
        let fixture = FixtureContext::new(
            &match_id,
            date,
            CompetitionKind::Continental,
            tie.stage.is_decided_on_the_night(),
        )
        .with_importance((home_side.stakes + away_side.stakes) * 0.5)
        .on_continent(self.tier.continent())
        .with_gate(RevenueModel::utilisation(
            home_team.reputation.overall_score(),
            1.0,
            1.0,
        ))
        .with_rivalry(if is_derby { 1.0 } else { 0.0 });
        Some(Match::make(
            match_id,
            self.tier.league_id(),
            self.tier.slug(),
            home_squad,
            away_squad,
            fixture,
        ))
    }
}

impl ContinentalCompetitions {
    /// Every club's nearest continental fixtures either side of `today`,
    /// each upcoming one with what it is worth to the club as things stand.
    pub(crate) fn fixture_runs(
        &self,
        today: NaiveDate,
        clubs: &FxHashMap<u32, &Club>,
    ) -> FxHashMap<u32, FixtureRun> {
        let competitions = [
            (
                CompetitionTier::ChampionsLeague,
                &self.champions_league.matches,
                &self.champions_league.groups,
                &self.champions_league.knockout_round,
            ),
            (
                CompetitionTier::EuropaLeague,
                &self.europa_league.matches,
                &self.europa_league.groups,
                &self.europa_league.knockout_round,
            ),
            (
                CompetitionTier::ConferenceLeague,
                &self.conference_league.matches,
                &self.conference_league.groups,
                &self.conference_league.knockout_round,
            ),
            (
                CompetitionTier::CopaLibertadores,
                &self.copa_libertadores.matches,
                &self.copa_libertadores.groups,
                &self.copa_libertadores.knockout_round,
            ),
        ];
        let mut nearest: FxHashMap<u32, NearestFixtures> = FxHashMap::default();
        for (tier, matches, groups, knockout) in competitions {
            for tie in matches {
                if tie.result.is_some() && tie.date <= today {
                    nearest
                        .entry(tie.home_team)
                        .or_default()
                        .offer_recent(tie.date);
                    nearest
                        .entry(tie.away_team)
                        .or_default()
                        .offer_recent(tie.date);
                    continue;
                }
                if tie.result.is_some() || tie.date <= today {
                    continue;
                }
                let teams = clubs
                    .get(&tie.home_team)
                    .and_then(|club| club.teams.main())
                    .zip(clubs.get(&tie.away_team).and_then(|club| club.teams.main()));
                let Some((home, away)) = teams else {
                    continue;
                };
                let (home_stakes, away_stakes) = ContinentalFixture {
                    tier: tier.clone(),
                    tie,
                    groups,
                    knockout,
                }
                .stakes(home, away);
                nearest
                    .entry(tie.home_team)
                    .or_default()
                    .offer_upcoming(tie.date, home_stakes);
                nearest
                    .entry(tie.away_team)
                    .or_default()
                    .offer_upcoming(tie.date, away_stakes);
            }
        }
        nearest
            .into_iter()
            .map(|(club_id, fixtures)| (club_id, fixtures.into_run()))
            .collect()
    }
}
