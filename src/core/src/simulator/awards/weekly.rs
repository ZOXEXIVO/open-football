use super::cache::MondayAwardCache;
use crate::league::player_of_week::{PlayerOfTheWeekAward, PlayerOfTheWeekSelector};
use crate::simulator::SimulatorData;
use crate::{AwardReputationInput, AwardReputationKind};
use rayon::prelude::*;

/// Monday-only orchestration that walks every non-friendly league, picks
/// its Player of the Week from last calendar week's matches, and applies
/// the side effects (player happiness event + league archive). Two-pass
/// design avoids overlapping `&` and `&mut` borrows of `SimulatorData`.
pub(crate) struct WeeklyAwardsTick;

impl WeeklyAwardsTick {
    pub(crate) fn run(data: &mut SimulatorData, cache: &MondayAwardCache) {
        let week_end = data.date.date();
        let pending = Self::collect_pending(data, week_end, cache);
        Self::apply_pending(data, pending);
    }

    fn collect_pending(
        data: &SimulatorData,
        week_end: chrono::NaiveDate,
        cache: &MondayAwardCache,
    ) -> Vec<PendingWeeklyAward> {
        data.continents
            .par_iter()
            .flat_map(|c| c.countries.par_iter())
            .flat_map(|country| country.leagues.leagues.par_iter())
            .filter_map(|league| {
                if league.friendly {
                    return None;
                }
                if league.player_of_week.has_award_for_week(week_end) {
                    return None;
                }

                let scores = cache.weekly_for(league.id)?;
                let (winner_id, agg) = PlayerOfTheWeekSelector::pick_winner(scores)?;

                let player = data.player(winner_id)?;
                let player_name = format!(
                    "{} {}",
                    player.full_name.display_first_name(),
                    player.full_name.display_last_name()
                );
                let player_slug = player.slug();
                let (club_id, club_name, club_slug) = Self::resolve_club_card(data, winner_id);
                let average_rating = if agg.matches_played > 0 {
                    agg.rating_sum / agg.matches_played as f32
                } else {
                    0.0
                };

                Some(PendingWeeklyAward {
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

    fn apply_pending(data: &mut SimulatorData, pending: Vec<PendingWeeklyAward>) {
        let now = data.date.date();
        for entry in pending {
            let league_rep = data.league(entry.league_id).map(|l| l.reputation);
            let avg_rating = entry.award.average_rating;
            let matches_played = entry.award.matches_played;
            if let Some(player) = data.player_mut(entry.winner_id) {
                player.on_player_of_the_week();
                let mut input = AwardReputationInput::new()
                    .with_avg_rating(avg_rating)
                    .with_matches_played(matches_played as u16);
                if let Some(rep) = league_rep {
                    input = input.with_league_reputation(rep);
                }
                player.apply_award_reputation_impact(
                    AwardReputationKind::PlayerOfTheWeek,
                    input,
                    now,
                );
            }
            if let Some(league) = data.league_mut(entry.league_id) {
                league.player_of_week.record(entry.award);
            }
        }
    }

    /// Resolve the active-club card (id, display name, slug) for a
    /// winning player. Falls back to empty values if the player isn't on
    /// a roster (free agent at award time — extremely unlikely
    /// Monday-morning, but guarded against).
    pub(super) fn resolve_club_card(data: &SimulatorData, player_id: u32) -> (u32, String, String) {
        let location = data
            .indexes
            .as_ref()
            .and_then(|i| i.get_player_location(player_id));
        let Some((_, _, club_id, _)) = location else {
            return (0, String::new(), String::new());
        };
        let Some(club) = data.club(club_id) else {
            return (club_id, String::new(), String::new());
        };
        let main_team = club.teams.main();
        let club_name = main_team
            .map(|t| t.name.clone())
            .unwrap_or_else(|| club.name.clone());
        let club_slug = main_team
            .map(|t| t.slug.clone())
            .unwrap_or_else(String::new);
        (club_id, club_name, club_slug)
    }
}

struct PendingWeeklyAward {
    league_id: u32,
    winner_id: u32,
    award: PlayerOfTheWeekAward,
}
