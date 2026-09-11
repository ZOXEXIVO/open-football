//! The weekly club press run.

mod dugout;
mod edition;
mod facts;
mod loans;
mod market;
mod run;

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
        let facts = WeeklyMatchFacts::from_world(data, week_start, week_end);
        let market = WeeklyMarket::from_world(data, week_start, week_end);
        let loans = WeeklyLoanWatch::from_world(data, week_end);
        let dugout = WeeklyDugout::from_world(data);

        let editions: Vec<(u32, u32, NewspaperIssue)> = data
            .continents
            .par_iter()
            .flat_map(|continent| continent.countries.par_iter())
            .flat_map_iter(|country| {
                country.clubs.iter().flat_map(|club| {
                    ClubPressRun::compile(
                        club, country, &facts, &market, &loans, &dugout, week_start, week_end,
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
