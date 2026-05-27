use super::cache::MondayAwardCache;
use super::weekly::WeeklyAwardsTick;
use crate::league::awards::{WeeklyAggregate, YOUNG_WEEKLY_MAX_AGE, YOUNG_WEEKLY_POW_MIN_SCORE};
use crate::league::player_of_week::{PlayerOfTheWeekAward, PlayerOfTheWeekSelector};
use crate::simulator::SimulatorData;
use crate::utils::DateUtils;
use crate::{AwardReputationInput, AwardReputationKind};
use chrono::NaiveDate;
use rayon::prelude::*;
use std::collections::HashMap;

/// Monday-only Young Player of the Week (age ≤ `YOUNG_WEEKLY_MAX_AGE`).
/// Mirrors `WeeklyAwardsTick` but filters the candidate set to under-20s
/// before scoring. Stored in `LeagueAwards::young_player_of_week`, not on
/// `League::player_of_week`, so the senior history stays untouched.
pub(crate) struct YoungWeeklyAwardsTick;

struct PendingYoungWeeklyAward {
    league_id: u32,
    winner_id: u32,
    award: PlayerOfTheWeekAward,
}

impl YoungWeeklyAwardsTick {
    pub(crate) fn run(data: &mut SimulatorData, cache: &MondayAwardCache) {
        let today = data.date.date();
        let pending = Self::collect(data, today, cache);
        Self::apply(data, pending);
    }

    fn collect(
        data: &SimulatorData,
        today: NaiveDate,
        cache: &MondayAwardCache,
    ) -> Vec<PendingYoungWeeklyAward> {
        let week_end = today;
        data.continents
            .par_iter()
            .flat_map(|c| c.countries.par_iter())
            .flat_map(|country| country.leagues.leagues.par_iter())
            .filter_map(|league| {
                if league.friendly {
                    return None;
                }
                if league.awards.has_young_player_of_week_for(week_end) {
                    return None;
                }

                let scores = cache.weekly_for(league.id)?;
                // Score-then-filter: identical scoring to the senior
                // award, then the eligibility gate runs over the
                // candidate aggregate to keep the tiebreak deterministic.
                let mut young: HashMap<u32, WeeklyAggregate> = HashMap::new();
                for (id, agg) in scores {
                    if data
                        .player(*id)
                        .map(|p| DateUtils::age(p.birth_date, today) <= YOUNG_WEEKLY_MAX_AGE)
                        .unwrap_or(false)
                    {
                        young.insert(*id, *agg);
                    }
                }
                // Score floor parallels Young TOTW: a thin U-20 pool
                // in a low-reputation league must not crown a 6.5-avg
                // padder. Below the floor, the league simply doesn't
                // have a Young POW that week.
                let (winner_id, agg) = PlayerOfTheWeekSelector::pick_winner_with_min_score(
                    &young,
                    YOUNG_WEEKLY_POW_MIN_SCORE,
                )?;

                let player = data.player(winner_id)?;
                let player_name = format!(
                    "{} {}",
                    player.full_name.display_first_name(),
                    player.full_name.display_last_name()
                );
                let player_slug = player.slug();
                let (club_id, club_name, club_slug) =
                    WeeklyAwardsTick::resolve_club_card(data, winner_id);
                let average_rating = if agg.matches_played > 0 {
                    agg.rating_sum / agg.matches_played as f32
                } else {
                    0.0
                };

                Some(PendingYoungWeeklyAward {
                    league_id: league.id,
                    winner_id,
                    award: PlayerOfTheWeekAward {
                        week_end_date: week_end,
                        player_id: winner_id,
                        player_name,
                        player_slug,
                        club_id,
                        club_name,
                        club_slug,
                        score: agg.score,
                        goals: agg.goals,
                        assists: agg.assists,
                        matches_played: agg.matches_played,
                        average_rating,
                    },
                })
            })
            .collect()
    }

    fn apply(data: &mut SimulatorData, pending: Vec<PendingYoungWeeklyAward>) {
        let now = data.date.date();
        for entry in pending {
            let league_rep = data.league(entry.league_id).map(|l| l.reputation);
            let avg_rating = entry.award.average_rating;
            let matches_played = entry.award.matches_played;
            if let Some(player) = data.player_mut(entry.winner_id) {
                player.on_young_player_of_the_week();
                let mut input = AwardReputationInput::new()
                    .with_avg_rating(avg_rating)
                    .with_matches_played(matches_played as u16)
                    .with_league_id(entry.league_id);
                if let Some(rep) = league_rep {
                    input = input.with_league_reputation(rep);
                }
                player.apply_award_reputation_impact(
                    AwardReputationKind::YoungPlayerOfTheWeek,
                    input,
                    now,
                );
            }
            if let Some(league) = data.league_mut(entry.league_id) {
                league.awards.record_young_player_of_week(entry.award);
            }
        }
    }
}
