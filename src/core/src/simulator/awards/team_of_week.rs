use super::cache::MondayAwardCache;
use super::weekly::WeeklyAwardsTick;
use crate::HappinessEventType;
use crate::league::awards::{TeamOfTheWeekAward, TeamOfTheWeekSelector, TeamOfTheWeekSlot};
use crate::simulator::SimulatorData;
use crate::{AwardReputationInput, AwardReputationKind};
use rayon::prelude::*;

/// Monday-only Team of the Week selection. Builds an XI per league with
/// the canonical 1-4-4-2 quotas and emits `TeamOfTheWeekSelection` to each
/// selected player.
pub(crate) struct TeamOfTheWeekTick;

struct PendingTeamOfWeek {
    league_id: u32,
    award: TeamOfTheWeekAward,
}

impl TeamOfTheWeekTick {
    pub(crate) fn run(data: &mut SimulatorData, cache: &MondayAwardCache) {
        let week_end = data.date.date();
        let pending = Self::collect(data, week_end, cache);
        Self::apply(data, pending);
    }

    fn collect(
        data: &SimulatorData,
        week_end: chrono::NaiveDate,
        cache: &MondayAwardCache,
    ) -> Vec<PendingTeamOfWeek> {
        data.continents
            .par_iter()
            .flat_map(|c| c.countries.par_iter())
            .flat_map(|country| country.leagues.leagues.par_iter())
            .filter_map(|league| {
                if league.friendly {
                    return None;
                }
                if league.awards.has_team_of_week_for(week_end) {
                    return None;
                }
                let scores = cache.candidate_for(league.id)?;
                let team = TeamOfTheWeekSelector::pick(scores);
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
                Some(PendingTeamOfWeek {
                    league_id: league.id,
                    award: TeamOfTheWeekAward {
                        week_end_date: week_end,
                        slots,
                    },
                })
            })
            .collect()
    }

    fn apply(data: &mut SimulatorData, pending: Vec<PendingTeamOfWeek>) {
        let now = data.date.date();
        for entry in pending {
            let league_rep = data.league(entry.league_id).map(|l| l.reputation);
            for slot in &entry.award.slots {
                let avg_rating = slot.average_rating;
                let matches_played = slot.matches_played;
                if let Some(player) = data.player_mut(slot.player_id) {
                    player.happiness.add_event_default_with_cooldown(
                        HappinessEventType::TeamOfTheWeekSelection,
                        6,
                    );
                    let mut input = AwardReputationInput::new()
                        .with_avg_rating(avg_rating)
                        .with_matches_played(matches_played as u16);
                    if let Some(rep) = league_rep {
                        input = input.with_league_reputation(rep);
                    }
                    player.apply_award_reputation_impact(
                        AwardReputationKind::TeamOfTheWeekSelection,
                        input,
                        now,
                    );
                }
            }
            if let Some(league) = data.league_mut(entry.league_id) {
                league.awards.record_team_of_week(entry.award);
            }
        }
    }
}
