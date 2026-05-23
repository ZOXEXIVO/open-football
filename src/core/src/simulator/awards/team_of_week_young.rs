use super::cache::MondayAwardCache;
use super::weekly::WeeklyAwardsTick;
use crate::HappinessEventType;
use crate::league::awards::{
    CandidateAggregate, TeamOfTheWeekAward, TeamOfTheWeekSelector, TeamOfTheWeekSlot,
    YOUNG_WEEKLY_MAX_AGE,
};
use crate::simulator::SimulatorData;
use crate::utils::DateUtils;
use crate::{AwardReputationInput, AwardReputationKind};
use rayon::prelude::*;
use std::collections::HashMap;

/// Monday-only Young Team of the Week. Reuses `TeamOfTheWeekSelector`
/// over the same week window but with the candidate set restricted to
/// players aged ≤ `YOUNG_WEEKLY_MAX_AGE` on the award date.
pub(crate) struct YoungTeamOfTheWeekTick;

struct PendingYoungTeamOfWeek {
    league_id: u32,
    award: TeamOfTheWeekAward,
}

impl YoungTeamOfTheWeekTick {
    pub(crate) fn run(data: &mut SimulatorData, cache: &MondayAwardCache) {
        let today = data.date.date();
        let pending = Self::collect(data, today, cache);
        Self::apply(data, pending);
    }

    fn collect(
        data: &SimulatorData,
        today: chrono::NaiveDate,
        cache: &MondayAwardCache,
    ) -> Vec<PendingYoungTeamOfWeek> {
        let week_end = today;
        data.continents
            .par_iter()
            .flat_map(|c| c.countries.par_iter())
            .flat_map(|country| country.leagues.leagues.par_iter())
            .filter_map(|league| {
                if league.friendly {
                    return None;
                }
                if league.awards.has_young_team_of_week_for(week_end) {
                    return None;
                }
                let scores = cache.candidate_for(league.id)?;
                let young: HashMap<u32, CandidateAggregate> = scores
                    .iter()
                    .filter(|(id, _)| {
                        data.player(**id)
                            .map(|p| DateUtils::age(p.birth_date, today) <= YOUNG_WEEKLY_MAX_AGE)
                            .unwrap_or(false)
                    })
                    .map(|(id, agg)| (*id, *agg))
                    .collect();
                let team = TeamOfTheWeekSelector::pick(&young);
                if team.is_empty() {
                    return None;
                }

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
                    let (club_id, club_name, club_slug) =
                        WeeklyAwardsTick::resolve_club_card(data, pid);
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
                Some(PendingYoungTeamOfWeek {
                    league_id: league.id,
                    award: TeamOfTheWeekAward {
                        week_end_date: week_end,
                        slots,
                    },
                })
            })
            .collect()
    }

    fn apply(data: &mut SimulatorData, pending: Vec<PendingYoungTeamOfWeek>) {
        let now = data.date.date();
        for entry in pending {
            let league_rep = data.league(entry.league_id).map(|l| l.reputation);
            for slot in &entry.award.slots {
                let avg_rating = slot.average_rating;
                let matches_played = slot.matches_played;
                if let Some(player) = data.player_mut(slot.player_id) {
                    player.happiness.add_event_default_with_cooldown(
                        HappinessEventType::YoungTeamOfTheWeekSelection,
                        6,
                    );
                    let mut input = AwardReputationInput::new()
                        .with_avg_rating(avg_rating)
                        .with_matches_played(matches_played as u16)
                        .with_league_id(entry.league_id);
                    if let Some(rep) = league_rep {
                        input = input.with_league_reputation(rep);
                    }
                    player.apply_award_reputation_impact(
                        AwardReputationKind::YoungTeamOfTheWeekSelection,
                        input,
                        now,
                    );
                }
            }
            if let Some(league) = data.league_mut(entry.league_id) {
                league.awards.record_young_team_of_week(entry.award);
            }
        }
    }
}
