use super::weekly::WeeklyAwardsTick;
use crate::league::awards::{
    AwardAggregator, TeamOfTheWeekSelector, TeamOfTheWeekSlot, TeamOfTheYearAward,
};
use crate::simulator::SimulatorData;
use crate::utils::DateUtils;
use crate::{
    AwardReputationInput, AwardReputationKind, HappinessEventType, RecognitionEventContext,
    RecognitionEventKind,
};
use chrono::Datelike;
use rayon::prelude::*;

/// Calendar-year XI per league. Runs once on December 31. Aggregates
/// every non-friendly league match played between Jan 1 (inclusive)
/// and Jan 1 of the next year (exclusive) and picks an XI with the
/// canonical Team of the Week quotas, gated by a per-player minimum
/// appearances threshold so a one-match wonder cannot win.
///
/// Distinct from `team_of_season` (which aligns to the league's
/// season end) — this archive is calendar-year aligned and lives at
/// `LeagueAwards::team_of_year`.
pub(crate) struct TeamOfTheYearTick;

struct PendingTeamOfYear {
    league_id: u32,
    award: TeamOfTheYearAward,
}

impl TeamOfTheYearTick {
    pub(crate) fn run(data: &mut SimulatorData) {
        let today = data.date.date();
        if !DateUtils::is_year_end(today) {
            return;
        }
        let year = today.year();
        let pending = Self::collect(data, today, year);
        Self::apply(data, pending);
    }

    fn collect(
        data: &SimulatorData,
        year_end_date: chrono::NaiveDate,
        year: i32,
    ) -> Vec<PendingTeamOfYear> {
        let year_start = chrono::NaiveDate::from_ymd_opt(year, 1, 1).unwrap_or(year_end_date);
        let next_year_start =
            chrono::NaiveDate::from_ymd_opt(year + 1, 1, 1).unwrap_or(year_end_date);

        data.continents
            .par_iter()
            .flat_map(|c| c.countries.par_iter())
            .flat_map(|country| country.leagues.leagues.par_iter())
            .filter_map(|league| {
                if league.friendly {
                    return None;
                }
                if league.awards.has_team_of_year_for(year) {
                    return None;
                }

                let scores = AwardAggregator::aggregate(
                    league.matches.iter_in_range(year_start, next_year_start),
                );
                if scores.is_empty() {
                    return None;
                }

                // Year-level appearance gate: max(10, ~25% of typical
                // matches per team). Calendar year ≈ one full league
                // campaign for most leagues, but split-season /
                // summer leagues may straddle two campaigns inside
                // one calendar year — the percentage scales with
                // whatever fixture density actually fell in this
                // window so a thin schedule isn't impossibly gated.
                let team_count = league.table.rows.len() as u32;
                let typical_matches_per_team = team_count.saturating_sub(1) * 2;
                let pct_floor = ((typical_matches_per_team as f32) * 0.25).round() as u8;
                let min_apps = pct_floor.max(10);

                let team = TeamOfTheWeekSelector::pick_with_min_apps(&scores, min_apps);
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
                        // Team-of-the-Year is a calendar-year XI: the
                        // regressed value protects the listing from the
                        // small-sample late-season callup who happens
                        // to top the raw board.
                        average_rating: agg.realistic_average_rating(),
                    });
                }
                Some(PendingTeamOfYear {
                    league_id: league.id,
                    award: TeamOfTheYearAward {
                        year,
                        year_end_date,
                        slots,
                    },
                })
            })
            .collect()
    }

    fn apply(data: &mut SimulatorData, pending: Vec<PendingTeamOfYear>) {
        let now = data.date.date();
        for entry in pending {
            let league_id = entry.league_id;
            let league_rep = data.league(league_id).map(|l| l.reputation);
            for slot in &entry.award.slots {
                let avg_rating = slot.average_rating;
                let matches_played = slot.matches_played;
                if let Some(player) = data.player_mut(slot.player_id) {
                    player.on_recognition_award(
                        HappinessEventType::TeamOfTheYearSelection,
                        RecognitionEventContext::new(RecognitionEventKind::TeamOfTheYearSelection)
                            .with_league(league_id)
                            .with_avg_rating(avg_rating)
                            .with_matches_played(matches_played as u16),
                        330,
                    );
                    let mut input = AwardReputationInput::new()
                        .with_avg_rating(avg_rating)
                        .with_matches_played(matches_played as u16);
                    if let Some(rep) = league_rep {
                        input = input.with_league_reputation(rep);
                    }
                    player.apply_award_reputation_impact(
                        AwardReputationKind::TeamOfTheYearSelection,
                        input,
                        now,
                    );
                }
            }
            if let Some(league) = data.league_mut(league_id) {
                league.awards.record_team_of_year(entry.award);
            }
        }
    }
}
