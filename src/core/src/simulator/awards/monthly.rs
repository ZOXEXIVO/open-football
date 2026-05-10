use super::weekly::WeeklyAwardsTick;
use crate::PlayerFieldPositionGroup;
use crate::league::awards::{
    AwardAggregator, CandidateAggregate, MonthlyAwardSelector, MonthlyAwardsSnapshot,
    MonthlyPlayerAward, MonthlyStatLeader, TeamOfTheWeekSelector, TeamOfTheWeekSlot,
};
use crate::simulator::SimulatorData;
use crate::utils::DateUtils;
use crate::{
    AwardReputationInput, AwardReputationKind, HappinessEventType, RecognitionEventContext,
    RecognitionEventKind,
};
use chrono::{Datelike, Duration};
use rayon::prelude::*;
use std::collections::HashMap;

const MONTHLY_TOP_N: usize = 5;
const MONTHLY_RATING_MIN_APPS: u8 = 2;

/// Monthly awards — POM and Young POM per league plus a frozen
/// per-league `MonthlyAwardsSnapshot` (Team of Month, Young Team of
/// Month, top scorers / assists / ratings). Runs on the 1st of each
/// calendar month, awarding the *previous* calendar month.
///
/// Empty months (no non-friendly matches with stats) are skipped
/// entirely — no PoM, no snapshot, no `last_monthly_award` bump — so
/// the web layer's "latest monthly" view always shows the most recent
/// month that actually had fixtures (winter break / split-season /
/// summer-calendar leagues all behave correctly without any
/// per-league special-casing).
pub(crate) struct MonthlyAwardsTick;

struct PendingMonthlyAward {
    league_id: u32,
    pom: Option<MonthlyPlayerAward>,
    young: Option<MonthlyPlayerAward>,
    snapshot: MonthlyAwardsSnapshot,
}

impl MonthlyAwardsTick {
    pub(crate) fn run(data: &mut SimulatorData) {
        let today = data.date.date();
        if today.day() != 1 {
            return;
        }
        let (start, end) = match Self::previous_month_window(today) {
            Some(w) => w,
            None => return,
        };

        let pending = Self::collect(data, today, start, end);
        Self::apply(data, pending);
    }

    /// First-of-month → start = first of previous month, end = first of
    /// this month (exclusive in `iter_in_range`).
    fn previous_month_window(
        today: chrono::NaiveDate,
    ) -> Option<(chrono::NaiveDate, chrono::NaiveDate)> {
        let first_this_month = chrono::NaiveDate::from_ymd_opt(today.year(), today.month(), 1)?;
        let prev_month = if today.month() == 1 {
            chrono::NaiveDate::from_ymd_opt(today.year() - 1, 12, 1)?
        } else {
            chrono::NaiveDate::from_ymd_opt(today.year(), today.month() - 1, 1)?
        };
        Some((prev_month, first_this_month))
    }

    fn collect(
        data: &SimulatorData,
        today: chrono::NaiveDate,
        start: chrono::NaiveDate,
        end: chrono::NaiveDate,
    ) -> Vec<PendingMonthlyAward> {
        let month_end = end - Duration::days(1);

        data.continents
            .par_iter()
            .flat_map(|c| c.countries.par_iter())
            .flat_map(|country| country.leagues.leagues.par_iter())
            .filter_map(|league| {
                if league.friendly {
                    return None;
                }
                if league.awards.has_monthly_award_for(month_end) {
                    return None;
                }

                // Count non-friendly matches that produced stats. If
                // the previous calendar month had none (winter break,
                // off-season gap, league not started yet), skip the
                // league entirely — no snapshot, no last_monthly_award
                // bump — so the web view keeps showing the previously
                // archived month.
                let matches_count = league
                    .matches
                    .iter_in_range(start, end)
                    .filter(|m| !m.friendly && m.details.is_some())
                    .count() as u32;
                if matches_count == 0 {
                    return None;
                }

                let scores = AwardAggregator::aggregate(league.matches.iter_in_range(start, end));

                let pom = Self::pick_pom(data, &scores, league.reputation, month_end);
                let young =
                    Self::pick_young_pom(data, &scores, league.reputation, today, month_end);

                let team_of_month = Self::build_team(data, &scores, |_| true);
                let young_team_of_month = Self::build_team(data, &scores, |id| {
                    data.player(id)
                        .map(|p| DateUtils::age(p.birth_date, today) <= 21)
                        .unwrap_or(false)
                });

                let top_scorers = Self::top_scorers(data, &scores);
                let top_assists = Self::top_assists(data, &scores);
                let best_ratings = Self::best_ratings(data, &scores);

                let snapshot = MonthlyAwardsSnapshot {
                    month_start_date: start,
                    month_end_date: month_end,
                    matches_count,
                    player_of_month: pom.clone(),
                    young_player_of_month: young.clone(),
                    team_of_month,
                    young_team_of_month,
                    top_scorers,
                    top_assists,
                    best_ratings,
                };

                Some(PendingMonthlyAward {
                    league_id: league.id,
                    pom,
                    young,
                    snapshot,
                })
            })
            .collect()
    }

    fn pick_pom(
        data: &SimulatorData,
        scores: &HashMap<u32, CandidateAggregate>,
        league_reputation: u16,
        month_end: chrono::NaiveDate,
    ) -> Option<MonthlyPlayerAward> {
        let (id, agg, score) =
            MonthlyAwardSelector::pick_best(scores, league_reputation, 3, |_| true)?;
        Self::monthly_award(data, id, agg, score, month_end)
    }

    fn pick_young_pom(
        data: &SimulatorData,
        scores: &HashMap<u32, CandidateAggregate>,
        league_reputation: u16,
        today: chrono::NaiveDate,
        month_end: chrono::NaiveDate,
    ) -> Option<MonthlyPlayerAward> {
        let (id, agg, score) =
            MonthlyAwardSelector::pick_best(scores, league_reputation, 2, |id| {
                data.player(id)
                    .map(|p| DateUtils::age(p.birth_date, today) <= 21)
                    .unwrap_or(false)
            })?;
        Self::monthly_award(data, id, agg, score, month_end)
    }

    fn monthly_award(
        data: &SimulatorData,
        id: u32,
        agg: CandidateAggregate,
        score: f32,
        month_end: chrono::NaiveDate,
    ) -> Option<MonthlyPlayerAward> {
        let player = data.player(id)?;
        let (club_id, club_name, club_slug) = WeeklyAwardsTick::resolve_club_card(data, id);
        Some(MonthlyPlayerAward {
            month_end_date: month_end,
            player_id: id,
            player_name: format!(
                "{} {}",
                player.full_name.display_first_name(),
                player.full_name.display_last_name()
            ),
            player_slug: player.slug(),
            club_id,
            club_name,
            club_slug,
            matches_played: agg.matches_played,
            goals: agg.goals,
            assists: agg.assists,
            average_rating: agg.average_rating(),
            score,
        })
    }

    /// Pick a Team of Month (1-4-4-2 quotas, min 2 apps) from the
    /// passed aggregate, restricted to ids passing `eligibility`. Used
    /// for both the open and Young (≤21) variants.
    fn build_team(
        data: &SimulatorData,
        scores: &HashMap<u32, CandidateAggregate>,
        eligibility: impl Fn(u32) -> bool,
    ) -> Vec<TeamOfTheWeekSlot> {
        let filtered: HashMap<u32, CandidateAggregate> = scores
            .iter()
            .filter(|(id, _)| eligibility(**id))
            .map(|(id, a)| (*id, *a))
            .collect();
        if filtered.is_empty() {
            return Vec::new();
        }
        let team = TeamOfTheWeekSelector::pick_with_min_apps(&filtered, 2);
        let mut slots: Vec<TeamOfTheWeekSlot> = Vec::with_capacity(team.len());
        for (pid, pos, score, agg) in team {
            let Some(player) = data.player(pid) else {
                continue;
            };
            let player_name = format!(
                "{} {}",
                player.full_name.display_first_name(),
                player.full_name.display_last_name()
            );
            let player_slug = player.slug();
            let (club_id, club_name, club_slug) = WeeklyAwardsTick::resolve_club_card(data, pid);
            slots.push(TeamOfTheWeekSlot {
                player_id: pid,
                player_name,
                player_slug,
                club_id,
                club_name,
                club_slug,
                position_group: pos,
                score,
                matches_played: agg.matches_played,
                goals: agg.goals,
                assists: agg.assists,
                average_rating: agg.average_rating(),
            });
        }
        slots
    }

    fn top_scorers(
        data: &SimulatorData,
        scores: &HashMap<u32, CandidateAggregate>,
    ) -> Vec<MonthlyStatLeader> {
        let mut all: Vec<(u32, CandidateAggregate)> = scores
            .iter()
            .filter(|(_, a)| a.goals > 0)
            .map(|(id, a)| (*id, *a))
            .collect();
        all.sort_by(|(la, aa), (lb, ab)| {
            ab.goals
                .cmp(&aa.goals)
                .then(ab.assists.cmp(&aa.assists))
                .then(
                    ab.average_rating()
                        .partial_cmp(&aa.average_rating())
                        .unwrap_or(std::cmp::Ordering::Equal),
                )
                .then(la.cmp(lb))
        });
        all.into_iter()
            .take(MONTHLY_TOP_N)
            .filter_map(|(id, agg)| Self::stat_leader(data, id, agg))
            .collect()
    }

    fn top_assists(
        data: &SimulatorData,
        scores: &HashMap<u32, CandidateAggregate>,
    ) -> Vec<MonthlyStatLeader> {
        let mut all: Vec<(u32, CandidateAggregate)> = scores
            .iter()
            .filter(|(_, a)| a.assists > 0)
            .map(|(id, a)| (*id, *a))
            .collect();
        all.sort_by(|(la, aa), (lb, ab)| {
            ab.assists
                .cmp(&aa.assists)
                .then(ab.goals.cmp(&aa.goals))
                .then(
                    ab.average_rating()
                        .partial_cmp(&aa.average_rating())
                        .unwrap_or(std::cmp::Ordering::Equal),
                )
                .then(la.cmp(lb))
        });
        all.into_iter()
            .take(MONTHLY_TOP_N)
            .filter_map(|(id, agg)| Self::stat_leader(data, id, agg))
            .collect()
    }

    fn best_ratings(
        data: &SimulatorData,
        scores: &HashMap<u32, CandidateAggregate>,
    ) -> Vec<MonthlyStatLeader> {
        let mut all: Vec<(u32, CandidateAggregate)> = scores
            .iter()
            .filter(|(_, a)| a.matches_played >= MONTHLY_RATING_MIN_APPS)
            .map(|(id, a)| (*id, *a))
            .collect();
        all.sort_by(|(la, aa), (lb, ab)| {
            ab.average_rating()
                .partial_cmp(&aa.average_rating())
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(ab.matches_played.cmp(&aa.matches_played))
                .then(ab.goals.cmp(&aa.goals))
                .then(la.cmp(lb))
        });
        all.into_iter()
            .take(MONTHLY_TOP_N)
            .filter_map(|(id, agg)| Self::stat_leader(data, id, agg))
            .collect()
    }

    fn stat_leader(
        data: &SimulatorData,
        id: u32,
        agg: CandidateAggregate,
    ) -> Option<MonthlyStatLeader> {
        let player = data.player(id)?;
        let (club_id, club_name, club_slug) = WeeklyAwardsTick::resolve_club_card(data, id);
        Some(MonthlyStatLeader {
            player_id: id,
            player_name: format!(
                "{} {}",
                player.full_name.display_first_name(),
                player.full_name.display_last_name()
            ),
            player_slug: player.slug(),
            club_id,
            club_name,
            club_slug,
            position_group: agg
                .primary_position
                .unwrap_or(PlayerFieldPositionGroup::Midfielder),
            matches_played: agg.matches_played,
            goals: agg.goals,
            assists: agg.assists,
            average_rating: agg.average_rating(),
        })
    }

    fn apply(data: &mut SimulatorData, pending: Vec<PendingMonthlyAward>) {
        let now = data.date.date();
        for entry in pending {
            let league_rep = data.league(entry.league_id).map(|l| l.reputation);
            // Young POM fires before senior POM so the centralised
            // award-reputation pipeline dampens the senior emit when
            // the same player wins both — young base is larger, so it
            // takes the full impact.
            if let Some(award) = &entry.young {
                let ctx = Self::pom_context(
                    RecognitionEventKind::YoungPlayerOfTheMonth,
                    entry.league_id,
                    award,
                );
                let avg_rating = award.average_rating;
                let matches_played = award.matches_played;
                if let Some(player) = data.player_mut(award.player_id) {
                    player.on_recognition_award(HappinessEventType::YoungPlayerOfTheMonth, ctx, 28);
                    let mut input = AwardReputationInput::new()
                        .with_avg_rating(avg_rating)
                        .with_matches_played(matches_played as u16);
                    if let Some(rep) = league_rep {
                        input = input.with_league_reputation(rep);
                    }
                    player.apply_award_reputation_impact(
                        AwardReputationKind::YoungPlayerOfTheMonth,
                        input,
                        now,
                    );
                }
            }
            if let Some(award) = &entry.pom {
                let ctx = Self::pom_context(
                    RecognitionEventKind::PlayerOfTheMonth,
                    entry.league_id,
                    award,
                );
                let avg_rating = award.average_rating;
                let matches_played = award.matches_played;
                if let Some(player) = data.player_mut(award.player_id) {
                    player.on_recognition_award(HappinessEventType::PlayerOfTheMonth, ctx, 28);
                    let mut input = AwardReputationInput::new()
                        .with_avg_rating(avg_rating)
                        .with_matches_played(matches_played as u16);
                    if let Some(rep) = league_rep {
                        input = input.with_league_reputation(rep);
                    }
                    player.apply_award_reputation_impact(
                        AwardReputationKind::PlayerOfTheMonth,
                        input,
                        now,
                    );
                }
            }
            // Team-of-month XIs fire after the individual monthly
            // awards so the centralised stacking dampener can suppress
            // the team-XI reputation gain when the same player just
            // won POM / Young POM. Young XI runs first by the same
            // larger-base-takes-full-impact rule used for POW/TOTW.
            for slot in &entry.snapshot.young_team_of_month {
                let avg_rating = slot.average_rating;
                let matches_played = slot.matches_played;
                let ctx = RecognitionEventContext::new(
                    RecognitionEventKind::YoungTeamOfTheMonthSelection,
                )
                .with_league(entry.league_id)
                .with_avg_rating(avg_rating)
                .with_matches_played(matches_played as u16)
                .with_season_goals(slot.goals as u16)
                .with_season_assists(slot.assists as u16);
                if let Some(player) = data.player_mut(slot.player_id) {
                    player.on_recognition_award(
                        HappinessEventType::YoungTeamOfTheMonthSelection,
                        ctx,
                        28,
                    );
                    let mut input = AwardReputationInput::new()
                        .with_avg_rating(avg_rating)
                        .with_matches_played(matches_played as u16);
                    if let Some(rep) = league_rep {
                        input = input.with_league_reputation(rep);
                    }
                    player.apply_award_reputation_impact(
                        AwardReputationKind::YoungTeamOfTheMonthSelection,
                        input,
                        now,
                    );
                }
            }
            for slot in &entry.snapshot.team_of_month {
                let avg_rating = slot.average_rating;
                let matches_played = slot.matches_played;
                let ctx =
                    RecognitionEventContext::new(RecognitionEventKind::TeamOfTheMonthSelection)
                        .with_league(entry.league_id)
                        .with_avg_rating(avg_rating)
                        .with_matches_played(matches_played as u16)
                        .with_season_goals(slot.goals as u16)
                        .with_season_assists(slot.assists as u16);
                if let Some(player) = data.player_mut(slot.player_id) {
                    player.on_recognition_award(
                        HappinessEventType::TeamOfTheMonthSelection,
                        ctx,
                        28,
                    );
                    let mut input = AwardReputationInput::new()
                        .with_avg_rating(avg_rating)
                        .with_matches_played(matches_played as u16);
                    if let Some(rep) = league_rep {
                        input = input.with_league_reputation(rep);
                    }
                    player.apply_award_reputation_impact(
                        AwardReputationKind::TeamOfTheMonthSelection,
                        input,
                        now,
                    );
                }
            }
            if let Some(league) = data.league_mut(entry.league_id) {
                if let Some(a) = entry.pom {
                    league.awards.record_player_of_month(a);
                }
                if let Some(a) = entry.young {
                    league.awards.record_young_player_of_month(a);
                }
                league.awards.record_monthly_snapshot(entry.snapshot);
            }
        }
    }

    fn pom_context(
        kind: RecognitionEventKind,
        league_id: u32,
        award: &MonthlyPlayerAward,
    ) -> RecognitionEventContext {
        RecognitionEventContext::new(kind)
            .with_league(league_id)
            .with_season_goals(award.goals as u16)
            .with_season_assists(award.assists as u16)
            .with_avg_rating(award.average_rating)
            .with_matches_played(award.matches_played as u16)
    }
}
