//! The weekly club press run.

mod dugout;
mod edition;
mod facts;
mod loans;
mod market;
mod run;

use crate::club::news::sightings::WeeklyScoutVisits;
use crate::club::news::{NewspaperIssue, WeeklyMatchFacts};
use crate::world::SimulatorData;
use chrono::NaiveDate;
use dugout::WeeklyDugout;
use loans::WeeklyLoanWatch;
use market::WeeklyMarket;
use rayon::prelude::*;
use run::ClubPressRun;

/// Weekly press run. Every local paper in the world goes to print on the
/// same Monday morning, covering the seven days just gone.
///
/// One paper per side competing under its own brand, not one per club: a
/// club with a first team, a "{Club} 2" side in a real lower division
/// and a B team goes to press three times, and each edition is about the
/// football that side played.
///
/// The pass is deliberately laid out as gather-then-write: all the
/// reading happens in parallel over an immutable world, and only the
/// finished editions are applied under `&mut`. That keeps it off the
/// critical path of the daily tick and away from the borrow gymnastics
/// the transfer pipeline needs.
pub(crate) struct ClubNewsroomTick;

impl ClubNewsroomTick {
    pub(crate) fn run(data: &mut SimulatorData, week_start: NaiveDate, week_end: NaiveDate) {
        // Each desk only reads the world, so the four gather side by side.
        let world: &SimulatorData = data;
        let ((facts, market), (loans, dugout)) = rayon::join(
            || {
                rayon::join(
                    || WeeklyMatchFacts::from_world(world, week_start, week_end),
                    || WeeklyMarket::from_world(world, week_start, week_end),
                )
            },
            || {
                rayon::join(
                    || WeeklyLoanWatch::from_world(world, week_end),
                    || WeeklyDugout::from_world(world),
                )
            },
        );

        let (facts, market, loans, dugout) = (&facts, &market, &loans, &dugout);
        let editions: Vec<(u32, u32, NewspaperIssue)> = data
            .continents
            .par_iter()
            .flat_map(|continent| continent.countries.par_iter())
            .flat_map_iter(|country| {
                let sightings =
                    WeeklyScoutVisits::gather(&country.scout_visits, week_start, week_end);
                country.clubs.iter().flat_map(move |club| {
                    ClubPressRun::compile(
                        club, country, facts, market, loans, dugout, &sightings, week_start,
                        week_end,
                    )
                    .into_iter()
                    .map(|(team_id, issue)| (club.id, team_id, issue))
                })
            })
            .collect();

        for (club_id, team_id, issue) in editions {
            if let Some(team) = data
                .club_mut(club_id)
                .and_then(|club| club.teams.find_mut(team_id))
            {
                team.newsroom.publish(issue);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::club::news::{NewsAngle, NewsStory, NewsStoryKind};
    use crate::continent::Continent;
    use crate::transfers::tests::kit::{TestClub, TestCountry, TestDate, TestPlayer};
    use crate::{
        GlobalCompetitions, HappinessEventCause, HappinessEventContext, HappinessEventScope,
        HappinessEventSeverity, HappinessEventType, Player, PlayerClubContract,
        TransferInterestContext, TransferInterestKind, TransferInterestReaction,
        TransferInterestSource, TransferInterestStage,
    };
    use chrono::Duration;

    struct Press;

    impl Press {
        const PARENT: u32 = 1;
        const BORROWER: u32 = 2;
        const PROVIDER: u32 = 70;
        const SUBJECT: u32 = 71;

        fn transfer_news(player: &mut Player, confirmed: bool) {
            let (stage, source, event) = if confirmed {
                (
                    TransferInterestStage::ConcreteInterest,
                    TransferInterestSource::ConfirmedApproach,
                    HappinessEventType::WantedByBiggerClub,
                )
            } else {
                (
                    TransferInterestStage::AgentSoundingOut,
                    TransferInterestSource::AgentLeak,
                    HappinessEventType::AgentStirsInterest,
                )
            };
            let mut interest = TransferInterestContext::new(
                stage,
                source,
                TransferInterestKind::Speculative,
                TransferInterestReaction::Flattered,
            );
            interest.interested_club_id = confirmed.then_some(Self::BORROWER);
            let context = HappinessEventContext::new(
                HappinessEventCause::MediaPressure,
                HappinessEventSeverity::Minor,
                HappinessEventScope::Media,
            )
            .with_transfer_interest_context(interest);
            player
                .happiness
                .add_event_with_context(event, 1.0, None, context);
        }

        fn world() -> SimulatorData {
            let today = TestDate::today();
            let mut loanee = TestPlayer::new(Self::PROVIDER).age(25).build();
            let mut loan = PlayerClubContract::new(10_000, today + Duration::days(150));
            loan.loan_from_club_id = Some(Self::PARENT);
            loan.loan_to_club_id = Some(Self::BORROWER);
            loan.started = Some(today - Duration::days(42));
            loanee.contract_loan = Some(loan);
            let subject = TestPlayer::new(Self::SUBJECT).age(25).build();
            let country = TestCountry::new(1)
                .clubs(vec![
                    TestClub::new(Self::PARENT).players(vec![subject]).build(),
                    TestClub::new(Self::BORROWER).players(vec![loanee]).build(),
                ])
                .build();
            SimulatorData::new(
                today.and_hms_opt(0, 0, 0).unwrap(),
                vec![Continent::new(
                    1,
                    "Europe".into(),
                    vec![country],
                    Vec::new(),
                )],
                GlobalCompetitions::new(Vec::new()),
            )
        }

        fn issue(world: &SimulatorData, index: usize) -> &NewspaperIssue {
            &world
                .club(Self::PARENT)
                .unwrap()
                .teams
                .main()
                .unwrap()
                .newsroom
                .issues[index]
        }

        fn story(issue: &NewspaperIssue, player_id: u32, kind: NewsStoryKind) -> &NewsStory {
            issue
                .stories
                .iter()
                .find(|story| story.kind == kind && story.player_id == player_id)
                .unwrap()
        }
    }

    #[test]
    fn successive_weekly_issues_preserve_the_earlier_loan_tally_and_agent_attribution() {
        let mut world = Press::world();
        let first = TestDate::today();
        let stats = &mut world.player_mut(Press::PROVIDER).unwrap().statistics;
        stats.played_subs = 6;
        stats.assists = 3;
        Press::transfer_news(world.player_mut(Press::SUBJECT).unwrap(), false);
        ClubNewsroomTick::run(&mut world, first - Duration::days(7), first);
        let issue = Press::issue(&world, 0);
        let initial = Press::story(issue, Press::PROVIDER, NewsStoryKind::LoanWatchAssists);
        assert_eq!(
            (initial.other_id, initial.a, initial.b),
            (Press::BORROWER, 3, 6)
        );
        let original_phrasing = initial.phrasing;
        let initial_rumour = Press::story(issue, Press::SUBJECT, NewsStoryKind::AgentTouting);
        assert_eq!(
            (initial_rumour.angle(), initial_rumour.other_id),
            (Some(NewsAngle::AgentSounding), 0)
        );

        let stats = &mut world.player_mut(Press::PROVIDER).unwrap().statistics;
        stats.played_subs = 7;
        stats.assists = 4;
        Press::transfer_news(world.player_mut(Press::SUBJECT).unwrap(), true);
        ClubNewsroomTick::run(&mut world, first, first + Duration::days(7));
        let latest = Press::issue(&world, 0);
        let earlier = Press::issue(&world, 1);
        assert_eq!((latest.number, earlier.number), (2, 1));
        let loan = Press::story(latest, Press::PROVIDER, NewsStoryKind::LoanWatchAssists);
        assert_eq!(
            (loan.other_id, loan.a, loan.b, loan.phrasing),
            (Press::BORROWER, 4, 7, original_phrasing ^ 1)
        );
        let loan = Press::story(earlier, Press::PROVIDER, NewsStoryKind::LoanWatchAssists);
        assert_eq!(
            (loan.other_id, loan.a, loan.b, loan.phrasing),
            (Press::BORROWER, 3, 6, original_phrasing)
        );
        let approach = Press::story(latest, Press::SUBJECT, NewsStoryKind::RumourInterest);
        assert_eq!(
            (approach.angle(), approach.other_id),
            (Some(NewsAngle::FormalApproach), Press::BORROWER)
        );
        let sounding = Press::story(earlier, Press::SUBJECT, NewsStoryKind::AgentTouting);
        assert_eq!(
            (sounding.angle(), sounding.other_id),
            (Some(NewsAngle::AgentSounding), 0)
        );
    }
}
