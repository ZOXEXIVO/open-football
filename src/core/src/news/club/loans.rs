use crate::club::news::RecentEvents;
use crate::club::news::{ClubLoanWatch, LoanWatchEntry};
use crate::world::SimulatorData;
use crate::{HappinessEventType, Person, Player, PlayerFieldPositionGroup};
use chrono::NaiveDate;
use rustc_hash::FxHashMap;

/// Every loaned-out player in the world, bucketed under the club that
/// still owns his contract.
///
/// A loanee is rostered at the club borrowing him, so his parent club
/// cannot see him by walking its own squads — the loan column would
/// otherwise be the one part of the paper a club could not write.
pub(super) struct WeeklyLoanWatch {
    by_parent: FxHashMap<u32, ClubLoanWatch>,
}

impl WeeklyLoanWatch {
    /// A loanee this age or younger is out there to be developed, which
    /// changes what a wasted spell means.
    const PROSPECT_AGE: u8 = 23;

    pub(super) fn from_world(data: &SimulatorData, today: NaiveDate) -> Self {
        let mut by_parent: FxHashMap<u32, ClubLoanWatch> = FxHashMap::default();

        for continent in &data.continents {
            for country in &continent.countries {
                for club in &country.clubs {
                    for team in club.teams.iter() {
                        for player in team.players.iter() {
                            let Some(entry) = Self::entry(player, club.id, today) else {
                                continue;
                            };
                            let parent = player
                                .contract_loan
                                .as_ref()
                                .and_then(|loan| loan.loan_from_club_id)
                                .unwrap_or(0);
                            if parent == 0 {
                                continue;
                            }
                            by_parent.entry(parent).or_default().players.push(entry);
                        }
                    }
                }
            }
        }

        WeeklyLoanWatch { by_parent }
    }

    /// One loanee's spell, as the parent club would read it off a
    /// scouting report: what he has played, what he has produced, how
    /// long is left, and what he has been saying about it.
    fn entry(player: &Player, loan_club_id: u32, today: NaiveDate) -> Option<LoanWatchEntry> {
        let loan = player.contract_loan.as_ref()?;
        loan.loan_from_club_id?;

        let stats = &player.statistics;
        let rating = stats.average_rating_realistic(player.position().position_group());
        let feed = RecentEvents::fortnight(player);

        Some(LoanWatchEntry {
            player_id: player.id,
            loan_club_id,
            starts: stats.played as i32,
            sub_appearances: stats.played_subs as i32,
            goals: stats.goals as i32,
            assists: stats.assists as i32,
            is_goalkeeper: matches!(
                player.position().position_group(),
                PlayerFieldPositionGroup::Goalkeeper
            ),
            clean_sheets: stats.clean_sheets as i32,
            conceded: stats.conceded as i32,
            rating_x100: (rating * 100.0) as i32,
            days_left: (loan.expiration - today).num_days() as i32,
            days_elapsed: loan
                .started
                .map(|started| (today - started).num_days() as i32)
                .unwrap_or(0),
            recall_available: loan
                .loan_recall_available_after
                .is_some_and(|from| from <= today),
            permanent_option: loan.loan_future_fee.is_some(),
            is_prospect: player.age(today) <= Self::PROSPECT_AGE,
            wants_permanent: feed.happened(HappinessEventType::WantsLoanMadePermanent),
            wants_to_prove_himself: feed.happened(HappinessEventType::WantsToProveHimselfAtParent),
            recall_requested: feed.happened(HappinessEventType::LoanRecallRequested),
            development_concern: feed.happened(HappinessEventType::LoanDevelopmentConcern),
            form_concern: feed.happened(HappinessEventType::LoanFormConcern),
            level_mismatch: feed.happened(HappinessEventType::LoanLevelMismatch),
            // A goal big enough to have registered on the player's own
            // record. There is no plain "he scored" event to read, and
            // the season tally above already covers volume — what this
            // adds is that one of them landed *recently*.
            //
            // The event behind it fires on an ASSIST as well as a goal —
            // "decisive contribution in a one-goal win" — and the last
            // pass before a winner can come off a goalkeeper's boot. So
            // the tally is checked too: without it the column filed "he
            // keeps scoring" about a keeper who had never scored in his
            // life, which is the one thing a paper cannot come back from.
            scored_recently: feed.happened(HappinessEventType::DecisiveGoal) && stats.goals > 0,
        })
    }

    pub(super) fn for_club(&self, club_id: u32) -> Option<&ClubLoanWatch> {
        self.by_parent.get(&club_id)
    }
}
