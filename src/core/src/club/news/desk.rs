use super::types::{IssueResult, NewsStory, NewsStoryKind};
use crate::club::DistressLevel;
use crate::transfers::{CompletedTransfer, TransferType};
use crate::{
    AwardReputationKind, Club, HappinessEventType, Person, Player, PlayerFieldPositionGroup,
    PlayerSquadStatus, PlayerStatusType, Team,
};
use chrono::{Duration, NaiveDate};
use rustc_hash::{FxHashMap, FxHashSet};

/// Everything the desks need to know about the week just played that
/// they cannot read off a `Club`. Built once per Monday from the global
/// match store and shared by every club in the world.
pub struct WeeklyMatchFacts {
    /// Goals scored in a single match this week, for players who
    /// managed at least three in one game.
    pub hat_tricks: FxHashMap<u32, u8>,
    /// Players sent off this week.
    pub red_cards: FxHashSet<u32>,
}

impl WeeklyMatchFacts {
    pub fn empty() -> Self {
        WeeklyMatchFacts {
            hat_tricks: FxHashMap::default(),
            red_cards: FxHashSet::default(),
        }
    }
}

/// One club's slice of the week's completed transfer business.
#[derive(Default)]
pub struct ClubTransferWeek {
    /// Players who arrived: `(player_id, other_club_id, fee, is_loan)`.
    pub arrivals: Vec<(u32, u32, i64, bool)>,
    /// Players who left, same shape.
    pub departures: Vec<(u32, u32, i64, bool)>,
}

impl ClubTransferWeek {
    /// Fold a completed move into the buying club's arrivals or the
    /// selling club's departures. `club_id` says which side is being
    /// filled in.
    pub fn absorb(&mut self, club_id: u32, transfer: &CompletedTransfer) {
        let is_loan = matches!(transfer.transfer_type, TransferType::Loan(_));
        let fee = transfer.fee.amount.max(0.0) as i64;

        if transfer.to_club_id == club_id {
            self.arrivals
                .push((transfer.player_id, transfer.from_club_id, fee, is_loan));
        } else if transfer.from_club_id == club_id {
            self.departures
                .push((transfer.player_id, transfer.to_club_id, fee, is_loan));
        }
    }

    pub fn is_empty(&self) -> bool {
        self.arrivals.is_empty() && self.departures.is_empty()
    }
}

/// Where the club stands in its league on publication day.
#[derive(Debug, Clone, Copy)]
pub struct StandingSnapshot {
    pub position: u8,
    pub teams: u8,
    pub points: u8,
    pub played: u8,
    pub total_rounds: u8,
}

impl StandingSnapshot {
    /// Fraction of the programme completed, 0..1. The press only starts
    /// talking about titles and relegation once there is enough season
    /// behind the table for it to mean something.
    pub fn progress(&self) -> f32 {
        if self.total_rounds == 0 {
            return 0.0;
        }
        (self.played as f32 / self.total_rounds as f32).clamp(0.0, 1.0)
    }
}

/// Match reports and the run-of-form pieces that hang off them.
pub struct MatchDesk;

impl MatchDesk {
    pub fn file(
        out: &mut Vec<NewsStory>,
        results: &[IssueResult],
        rival_team_ids: &FxHashSet<u32>,
        team: &Team,
    ) {
        for result in results {
            let is_derby = rival_team_ids.contains(&result.opponent_team_id);
            let kind = Self::classify(result.goals_for, result.goals_against, is_derby);

            // A seven-goal thriller outranks a 1-0: the bigger the story
            // on the pitch, the higher up the page it goes.
            let goal_bonus = (result.goals_for as i32 + result.goals_against as i32) * 6;

            out.push(
                NewsStory::new(kind, result.date)
                    .against(result.opponent_team_id)
                    .with_numbers(result.goals_for as i32, result.goals_against as i32)
                    .weighted(goal_bonus),
            );
        }

        Self::file_runs(out, results, team);
    }

    /// Which report a scoreline earns. A derby is its own story whatever
    /// the margin — losing 1-0 to the neighbours hurts more than losing
    /// 4-0 to anyone else, and the page has to read that way.
    pub fn classify(goals_for: u8, goals_against: u8, is_derby: bool) -> NewsStoryKind {
        let margin = goals_for as i32 - goals_against as i32;

        if is_derby && margin > 0 {
            NewsStoryKind::DerbyWin
        } else if is_derby && margin < 0 {
            NewsStoryKind::DerbyDefeat
        } else if margin >= 3 {
            NewsStoryKind::Rout
        } else if margin <= -3 {
            NewsStoryKind::HeavyDefeat
        } else if margin > 0 {
            NewsStoryKind::LeagueWin
        } else if margin < 0 {
            NewsStoryKind::LeagueDefeat
        } else {
            NewsStoryKind::LeagueDraw
        }
    }

    /// Streaks are read off the senior side's own match log rather than
    /// this week's fixtures — a run is by definition longer than a week.
    fn file_runs(out: &mut Vec<NewsStory>, results: &[IssueResult], team: &Team) {
        let Some(latest) = results.last() else {
            return;
        };

        let mut wins = 0u8;
        let mut winless = 0u8;
        let mut counting_wins = true;
        let mut counting_winless = true;

        for item in team.match_history.items().iter().rev() {
            let scored = item.score.0.get();
            let conceded = item.score.1.get();
            let won = scored > conceded;

            if counting_wins {
                if won {
                    wins = wins.saturating_add(1);
                } else {
                    counting_wins = false;
                }
            }
            if counting_winless {
                if won {
                    counting_winless = false;
                } else {
                    winless = winless.saturating_add(1);
                }
            }
            if !counting_wins && !counting_winless {
                break;
            }
        }

        if wins >= 3 {
            out.push(
                NewsStory::new(NewsStoryKind::WinningRun, latest.date)
                    .with_numbers(wins as i32, 0)
                    .weighted((wins as i32 - 3) * 25),
            );
        } else if winless >= 4 {
            out.push(
                NewsStory::new(NewsStoryKind::WinlessRun, latest.date)
                    .with_numbers(winless as i32, 0)
                    .weighted((winless as i32 - 4) * 25),
            );
        }
    }
}

/// The table story: who is climbing and who is in trouble.
pub struct TableDesk;

impl TableDesk {
    /// Below this share of the season the table is noise, and no serious
    /// paper leads on it.
    const MIN_PROGRESS: f32 = 0.30;

    pub fn file(out: &mut Vec<NewsStory>, standing: Option<StandingSnapshot>, date: NaiveDate) {
        let Some(standing) = standing else {
            return;
        };
        if standing.teams == 0 || standing.progress() < Self::MIN_PROGRESS {
            return;
        }

        let position = standing.position as i32;
        let points = standing.points as i32;

        if standing.position <= 3 {
            out.push(
                NewsStory::new(NewsStoryKind::TitleCharge, date)
                    .with_numbers(position, points)
                    // Leading the table is a bigger story than third.
                    .weighted((4 - position) * 30),
            );
            return;
        }

        let drop_edge = standing.teams.saturating_sub(3);
        if standing.position > drop_edge {
            out.push(
                NewsStory::new(NewsStoryKind::RelegationFight, date)
                    .with_numbers(position, points)
                    // Deeper in the mire, and later in the season, hurts more.
                    .weighted(
                        (position - drop_edge as i32) * 20 + (standing.progress() * 60.0) as i32,
                    ),
            );
        }
    }
}

/// Squad news: form, fitness, discipline, milestones, contracts.
pub struct SquadDesk;

impl SquadDesk {
    /// Career appearances are reported at each round hundred, goals at
    /// each round fifty — the numbers a club shop prints on a
    /// commemorative shirt.
    const APPS_STEP: i32 = 100;
    const GOALS_STEP: i32 = 50;

    pub fn file(
        out: &mut Vec<NewsStory>,
        club: &Club,
        facts: &WeeklyMatchFacts,
        played_this_week: bool,
        date: NaiveDate,
    ) {
        for team in club.teams.iter() {
            let senior = team.team_type.is_own_team();

            for player in team.players.iter() {
                if player.is_retired() {
                    continue;
                }

                Self::file_match_deeds(out, player.id, facts, date);
                Self::file_fitness(out, player, date);
                Self::file_awards(out, player, date);
                Self::file_life_events(out, player, date);

                if senior {
                    Self::file_form(out, player, played_this_week, date);
                    Self::file_contract(out, player, date);
                    Self::file_market_mood(out, player, date);
                }
            }
        }
    }

    fn file_match_deeds(
        out: &mut Vec<NewsStory>,
        player_id: u32,
        facts: &WeeklyMatchFacts,
        date: NaiveDate,
    ) {
        if let Some(goals) = facts.hat_tricks.get(&player_id) {
            out.push(
                NewsStory::new(NewsStoryKind::HatTrick, date)
                    .about(player_id)
                    .with_numbers(*goals as i32, 0)
                    // Four and five-goal hauls are a different story again.
                    .weighted((*goals as i32 - 3) * 60),
            );
        }

        if facts.red_cards.contains(&player_id) {
            out.push(NewsStory::new(NewsStoryKind::RedCard, date).about(player_id));
        }
    }

    fn file_fitness(out: &mut Vec<NewsStory>, player: &Player, date: NaiveDate) {
        let days_out = player.player_attributes.injury_days_remaining as i32;

        // Two weeks or more is a selection problem worth printing; a
        // knock that clears by Saturday is not news.
        if player.player_attributes.is_injured && days_out >= 14 {
            out.push(
                NewsStory::new(NewsStoryKind::InjuryBlow, date)
                    .about(player.id)
                    .with_numbers(days_out, 0)
                    .weighted(Self::importance(player) + (days_out.min(180) / 3)),
            );
        }
    }

    fn file_awards(out: &mut Vec<NewsStory>, player: &Player, date: NaiveDate) {
        let week_start = date - Duration::days(7);
        // The timeline is chronological and can run to a thousand entries
        // for a decorated career, so walk back from the newest and stop
        // the moment the week closes rather than reading a whole career
        // for every player in the world, every week.
        let won_pom = player
            .awards_count
            .timeline
            .iter()
            .rev()
            .take_while(|entry| entry.date >= week_start)
            .any(|entry| {
                entry.date <= date
                    && matches!(
                        entry.kind,
                        AwardReputationKind::PlayerOfTheMonth
                            | AwardReputationKind::YoungPlayerOfTheMonth
                    )
            });

        if won_pom {
            out.push(NewsStory::new(NewsStoryKind::PlayerOfMonth, date).about(player.id));
        }
    }

    /// Beats the player's own event feed records: a debut, a recovery, a
    /// career milestone. The feed is the only place these are marked, so
    /// the desk reads it for the trigger and then sources the real number
    /// from the player's career record.
    fn file_life_events(out: &mut Vec<NewsStory>, player: &Player, date: NaiveDate) {
        let mut debut = false;
        let mut back_in_training = false;
        let mut apps_milestone = false;
        let mut goals_milestone = false;

        for event in &player.happiness.recent_events {
            if event.days_ago > 7 {
                continue;
            }
            match event.event_type {
                HappinessEventType::SeniorDebut => debut = true,
                HappinessEventType::InjuryReturn => back_in_training = true,
                HappinessEventType::AppearanceMilestone => apps_milestone = true,
                HappinessEventType::GoalMilestone => goals_milestone = true,
                _ => {}
            }
        }

        if debut {
            out.push(
                NewsStory::new(NewsStoryKind::YouthDebut, date)
                    .about(player.id)
                    .with_numbers(player.age(date) as i32, 0),
            );
        }

        if back_in_training && !player.player_attributes.is_injured {
            out.push(NewsStory::new(NewsStoryKind::InjuryReturn, date).about(player.id));
        }

        if apps_milestone {
            let apps = CareerRecord::appearances(player);
            let milestone = apps - (apps % Self::APPS_STEP);
            if milestone >= Self::APPS_STEP {
                out.push(
                    NewsStory::new(NewsStoryKind::MilestoneApps, date)
                        .about(player.id)
                        .with_numbers(milestone, 0)
                        .weighted(milestone / 10),
                );
            }
        }

        if goals_milestone {
            let goals = CareerRecord::goals(player);
            let milestone = goals - (goals % Self::GOALS_STEP);
            if milestone >= Self::GOALS_STEP {
                out.push(
                    NewsStory::new(NewsStoryKind::MilestoneGoals, date)
                        .about(player.id)
                        .with_numbers(milestone, 0)
                        .weighted(milestone / 2),
                );
            }
        }
    }

    fn file_form(
        out: &mut Vec<NewsStory>,
        player: &Player,
        played_this_week: bool,
        date: NaiveDate,
    ) {
        if !played_this_week {
            return;
        }

        let stats = &player.statistics;
        let apps = stats.played as i32 + stats.played_subs as i32;
        if apps < 6 {
            return;
        }

        let group = player.position().position_group();
        let rating = stats.average_rating_realistic(group);

        if matches!(group, PlayerFieldPositionGroup::Goalkeeper) {
            // A goalkeeper's season is measured in shut-outs, not ratings.
            let clean_sheets = stats.clean_sheets as i32;
            if clean_sheets >= 4 && rating >= 6.9 {
                out.push(
                    NewsStory::new(NewsStoryKind::KeeperWall, date)
                        .about(player.id)
                        .with_numbers(clean_sheets, (rating * 100.0) as i32)
                        .weighted(clean_sheets * 12),
                );
            }
            return;
        }

        if rating >= 7.20 {
            let goals = stats.goals as i32;
            out.push(
                NewsStory::new(NewsStoryKind::StarForm, date)
                    .about(player.id)
                    .with_numbers(goals, (rating * 100.0) as i32)
                    .weighted(((rating - 7.20) * 200.0) as i32 + goals * 8),
            );
        }
    }

    fn file_contract(out: &mut Vec<NewsStory>, player: &Player, date: NaiveDate) {
        let Some(contract) = player.contract.as_ref() else {
            return;
        };
        if player.is_on_loan() {
            return;
        }

        let days_left = (contract.expiration - date).num_days();
        if !(0..=210).contains(&days_left) {
            return;
        }

        // Only a player the club would miss makes this a story.
        let importance = Self::importance(player);
        if importance < 30 {
            return;
        }

        let months_left = ((days_left as f32) / 30.0).round().max(1.0) as i32;
        out.push(
            NewsStory::new(NewsStoryKind::ContractStandoff, date)
                .about(player.id)
                .with_numbers(months_left, 0)
                .weighted(importance - (months_left * 6)),
        );
    }

    fn file_market_mood(out: &mut Vec<NewsStory>, player: &Player, date: NaiveDate) {
        let statuses = &player.statuses;

        if statuses.has(PlayerStatusType::Lst) {
            // Freshly listed is news; listed since last spring is not.
            if statuses
                .held_for_days(PlayerStatusType::Lst, date)
                .is_some_and(|days| days <= 28)
            {
                out.push(
                    NewsStory::new(NewsStoryKind::TransferListed, date)
                        .about(player.id)
                        .weighted(Self::importance(player)),
                );
            }
            return;
        }

        // A concrete bid is a much louder story than a scout in the
        // stand, so the further down the funnel, the higher it runs.
        let heat = if statuses.has(PlayerStatusType::Trn) {
            180
        } else if statuses.has(PlayerStatusType::Bid) {
            120
        } else if statuses.has(PlayerStatusType::Enq) {
            60
        } else if statuses.has(PlayerStatusType::Wnt) || statuses.has(PlayerStatusType::Sct) {
            0
        } else {
            return;
        };

        out.push(
            NewsStory::new(NewsStoryKind::TransferSpeculation, date)
                .about(player.id)
                .weighted(heat + Self::importance(player)),
        );
    }

    /// How much the local paper cares about this player, from squad role
    /// and standing in the game. Used as a priority nudge so the same
    /// story about a key player outranks one about a fringe squad man.
    fn importance(player: &Player) -> i32 {
        let role = player
            .contract
            .as_ref()
            .map(|contract| match contract.squad_status {
                PlayerSquadStatus::KeyPlayer => 90,
                PlayerSquadStatus::FirstTeamRegular => 65,
                PlayerSquadStatus::FirstTeamSquadRotation => 35,
                PlayerSquadStatus::HotProspectForTheFuture => 30,
                PlayerSquadStatus::MainBackupPlayer => 15,
                PlayerSquadStatus::DecentYoungster => 10,
                _ => 0,
            })
            .unwrap_or(0);

        role + (player.player_attributes.world_reputation as i32 / 250)
    }
}

/// Career totals a milestone story needs. The live season counters only
/// cover the current spell, so the ledger has to be folded in.
pub struct CareerRecord;

impl CareerRecord {
    pub fn appearances(player: &Player) -> i32 {
        let live = player.statistics.played as i32 + player.statistics.played_subs as i32;
        let historic: i32 = player
            .statistics_history
            .season_ledger
            .iter()
            .map(|entry| entry.statistics.played as i32 + entry.statistics.played_subs as i32)
            .sum();
        live + historic
    }

    pub fn goals(player: &Player) -> i32 {
        let live = player.statistics.goals as i32;
        let historic: i32 = player
            .statistics_history
            .season_ledger
            .iter()
            .map(|entry| entry.statistics.goals as i32)
            .sum();
        live + historic
    }
}

/// Arrivals and departures.
pub struct MarketDesk;

impl MarketDesk {
    /// A fee at or above this multiple of the club's most valuable asset
    /// is treated as club-record business — the framing a local paper
    /// reaches for when a deal is out of scale with everything the club
    /// has done before.
    const RECORD_MULTIPLE: i64 = 2;

    pub fn file(
        out: &mut Vec<NewsStory>,
        week: &ClubTransferWeek,
        squad_peak_value: i64,
        date: NaiveDate,
    ) {
        let record_bar = squad_peak_value.saturating_mul(Self::RECORD_MULTIPLE);

        for (player_id, from_club_id, fee, is_loan) in &week.arrivals {
            let kind = if *is_loan {
                NewsStoryKind::LoanArrival
            } else if *fee <= 0 {
                NewsStoryKind::FreeSigning
            } else if record_bar > 0 && *fee >= record_bar {
                NewsStoryKind::RecordSigning
            } else {
                NewsStoryKind::NewSigning
            };

            out.push(
                NewsStory::new(kind, date)
                    .about(*player_id)
                    .against(*from_club_id)
                    .with_money(*fee)
                    .weighted(Self::fee_weight(*fee)),
            );
        }

        for (player_id, to_club_id, fee, is_loan) in &week.departures {
            let kind = if *is_loan {
                NewsStoryKind::LoanExit
            } else if record_bar > 0 && *fee >= record_bar {
                NewsStoryKind::StarSold
            } else {
                NewsStoryKind::PlayerSold
            };

            out.push(
                NewsStory::new(kind, date)
                    .about(*player_id)
                    .against(*to_club_id)
                    .with_money(*fee)
                    .weighted(Self::fee_weight(*fee)),
            );
        }
    }

    /// Money talks, but with diminishing returns: the jump from one to
    /// ten million is far bigger news than ninety to a hundred.
    fn fee_weight(fee: i64) -> i32 {
        if fee <= 0 {
            return 0;
        }
        let millions = fee as f64 / 1_000_000.0;
        ((millions + 1.0).ln() * 45.0) as i32
    }
}

/// Boardroom and balance sheet.
pub struct BoardroomDesk;

impl BoardroomDesk {
    pub fn file(out: &mut Vec<NewsStory>, club: &Club, date: NaiveDate) {
        let confidence = club.board.confidence.level;

        if club.board.manager_on_final_warning || confidence <= 30 {
            let urgency = if club.board.manager_on_final_warning {
                120
            } else {
                (35 - confidence).max(0) * 4
            };
            out.push(
                NewsStory::new(NewsStoryKind::ManagerPressure, date)
                    .with_numbers(confidence, 0)
                    .weighted(urgency),
            );
        } else if confidence >= 82 {
            out.push(NewsStory::new(NewsStoryKind::BoardBacking, date).with_numbers(confidence, 0));
        }

        let severity = match club.finance.distress_level {
            DistressLevel::Insolvency => 220,
            DistressLevel::Severe => 130,
            DistressLevel::Distress => 50,
            DistressLevel::None => 0,
        };
        if severity > 0 {
            out.push(
                NewsStory::new(NewsStoryKind::MoneyWorries, date)
                    .with_numbers(severity, 0)
                    .weighted(severity),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ClubTransferWeek, MarketDesk, MatchDesk, StandingSnapshot, TableDesk};
    use crate::club::news::types::{NewsStory, NewsStoryKind};
    use chrono::NaiveDate;

    struct Fixture;

    impl Fixture {
        fn day() -> NaiveDate {
            NaiveDate::from_ymd_opt(2026, 3, 2).unwrap()
        }

        fn table(position: u8, teams: u8, played: u8, total_rounds: u8) -> StandingSnapshot {
            StandingSnapshot {
                position,
                teams,
                points: played,
                played,
                total_rounds,
            }
        }

        fn kinds(stories: &[NewsStory]) -> Vec<NewsStoryKind> {
            stories.iter().map(|story| story.kind).collect()
        }
    }

    #[test]
    fn a_derby_outranks_the_margin() {
        assert_eq!(
            MatchDesk::classify(1, 0, true),
            NewsStoryKind::DerbyWin,
            "a one-goal derby win is still a derby story"
        );
        assert_eq!(MatchDesk::classify(0, 4, true), NewsStoryKind::DerbyDefeat);
        assert_eq!(MatchDesk::classify(4, 0, false), NewsStoryKind::Rout);
        assert_eq!(MatchDesk::classify(0, 3, false), NewsStoryKind::HeavyDefeat);
        assert_eq!(MatchDesk::classify(2, 1, false), NewsStoryKind::LeagueWin);
        assert_eq!(
            MatchDesk::classify(1, 2, false),
            NewsStoryKind::LeagueDefeat
        );
        assert_eq!(MatchDesk::classify(1, 1, false), NewsStoryKind::LeagueDraw);
    }

    #[test]
    fn the_table_is_not_a_story_in_august() {
        let mut out = Vec::new();
        TableDesk::file(&mut out, Some(Fixture::table(1, 20, 4, 38)), Fixture::day());

        assert!(
            out.is_empty(),
            "four games in, top spot means nothing and no paper leads on it"
        );
    }

    #[test]
    fn a_title_race_and_a_drop_fight_both_reach_the_page() {
        let mut leaders = Vec::new();
        TableDesk::file(
            &mut leaders,
            Some(Fixture::table(2, 20, 24, 38)),
            Fixture::day(),
        );
        assert_eq!(Fixture::kinds(&leaders), vec![NewsStoryKind::TitleCharge]);

        let mut strugglers = Vec::new();
        TableDesk::file(
            &mut strugglers,
            Some(Fixture::table(19, 20, 24, 38)),
            Fixture::day(),
        );
        assert_eq!(
            Fixture::kinds(&strugglers),
            vec![NewsStoryKind::RelegationFight]
        );

        let mut mid = Vec::new();
        TableDesk::file(
            &mut mid,
            Some(Fixture::table(10, 20, 24, 38)),
            Fixture::day(),
        );
        assert!(mid.is_empty(), "mid-table is not news");
    }

    #[test]
    fn a_fee_out_of_scale_with_the_squad_is_record_business() {
        let week = ClubTransferWeek {
            arrivals: vec![(11, 90, 30_000_000, false), (12, 91, 2_000_000, false)],
            departures: vec![(13, 92, 40_000_000, false), (14, 93, 1_000_000, false)],
        };

        let mut out = Vec::new();
        MarketDesk::file(&mut out, &week, 10_000_000, Fixture::day());

        assert_eq!(
            Fixture::kinds(&out),
            vec![
                NewsStoryKind::RecordSigning,
                NewsStoryKind::NewSigning,
                NewsStoryKind::StarSold,
                NewsStoryKind::PlayerSold,
            ]
        );
    }

    #[test]
    fn free_transfers_and_loans_are_told_apart_from_purchases() {
        let week = ClubTransferWeek {
            arrivals: vec![(11, 90, 0, false), (12, 91, 0, true)],
            departures: vec![(13, 92, 0, true)],
        };

        let mut out = Vec::new();
        MarketDesk::file(&mut out, &week, 10_000_000, Fixture::day());

        assert_eq!(
            Fixture::kinds(&out),
            vec![
                NewsStoryKind::FreeSigning,
                NewsStoryKind::LoanArrival,
                NewsStoryKind::LoanExit,
            ]
        );
    }

    #[test]
    fn a_bigger_fee_carries_more_weight_than_a_smaller_one() {
        let week = ClubTransferWeek {
            arrivals: vec![(11, 90, 1_000_000, false), (12, 91, 25_000_000, false)],
            departures: Vec::new(),
        };

        let mut out = Vec::new();
        MarketDesk::file(&mut out, &week, 500_000_000, Fixture::day());

        assert!(
            out[1].priority > out[0].priority,
            "the expensive signing must outrank the cheap one on the page"
        );
    }
}
