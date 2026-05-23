use crate::league::awards::SeasonAwardsSnapshot;
use crate::simulator::SimulatorData;
use crate::{
    AwardReputationInput, AwardReputationKind, HappinessEventType, RecognitionEventContext,
    RecognitionEventKind,
};
use rayon::prelude::*;

/// Season awards — drains each league's pending snapshot (built inside
/// `process_season_end` before stats archive) and fires player events.
pub(crate) struct SeasonAwardsTick;

impl SeasonAwardsTick {
    pub(crate) fn run(data: &mut SimulatorData) {
        let today = data.date.date();
        let pending: Vec<(u32, SeasonAwardsSnapshot)> = data
            .continents
            .par_iter_mut()
            .flat_map(|c| c.countries.par_iter_mut())
            .flat_map(|c| c.leagues.leagues.par_iter_mut())
            .filter_map(|l| l.awards.pending_season_awards.take().map(|s| (l.id, s)))
            .collect();

        for (league_id, snapshot) in pending {
            let league_rep = data.league(league_id).map(|l| l.reputation);

            // Young POS first so the senior-POS emit is dampened when
            // the same player wins both in a single season.
            if let Some(id) = snapshot.young_player_of_season {
                Self::apply_player_award(
                    data,
                    id,
                    league_id,
                    league_rep,
                    today,
                    HappinessEventType::YoungPlayerOfTheSeason,
                    RecognitionEventKind::YoungPlayerOfTheSeason,
                    AwardReputationKind::YoungPlayerOfTheSeason,
                );
            }
            if let Some(id) = snapshot.player_of_season {
                Self::apply_player_award(
                    data,
                    id,
                    league_id,
                    league_rep,
                    today,
                    HappinessEventType::PlayerOfTheSeason,
                    RecognitionEventKind::PlayerOfTheSeason,
                    AwardReputationKind::PlayerOfTheSeason,
                );
            }
            for pid in &snapshot.team_of_season {
                Self::apply_player_award(
                    data,
                    *pid,
                    league_id,
                    league_rep,
                    today,
                    HappinessEventType::TeamOfTheSeasonSelection,
                    RecognitionEventKind::TeamOfTheSeasonSelection,
                    AwardReputationKind::TeamOfTheSeasonSelection,
                );
            }
            if let Some(id) = snapshot.top_scorer {
                Self::apply_player_award(
                    data,
                    id,
                    league_id,
                    league_rep,
                    today,
                    HappinessEventType::LeagueTopScorer,
                    RecognitionEventKind::LeagueTopScorer,
                    AwardReputationKind::LeagueTopScorer,
                );
            }
            if let Some(id) = snapshot.top_assists {
                Self::apply_player_award(
                    data,
                    id,
                    league_id,
                    league_rep,
                    today,
                    HappinessEventType::LeagueTopAssists,
                    RecognitionEventKind::LeagueTopAssists,
                    AwardReputationKind::LeagueTopAssists,
                );
            }
            if let Some(id) = snapshot.golden_glove {
                Self::apply_player_award(
                    data,
                    id,
                    league_id,
                    league_rep,
                    today,
                    HappinessEventType::LeagueGoldenGlove,
                    RecognitionEventKind::LeagueGoldenGlove,
                    AwardReputationKind::LeagueGoldenGlove,
                );
            }
            // Archive the snapshot once events have been applied.
            let mut snapshot = snapshot;
            snapshot.season_end_date = today;
            if let Some(league) = data.league_mut(league_id) {
                league.awards.record_season(snapshot);
            }
        }
    }

    /// Apply a season-end award: emit the recognition event with full
    /// season context (avg rating, matches, goals/assists) and route
    /// the centralised reputation impact through the same helper used
    /// by every other emit site, so the model is league-aware,
    /// quality-weighted, and headroom-bounded in one place.
    fn apply_player_award(
        data: &mut SimulatorData,
        player_id: u32,
        league_id: u32,
        league_rep: Option<u16>,
        now: chrono::NaiveDate,
        happiness_event: HappinessEventType,
        recognition_kind: RecognitionEventKind,
        reputation_kind: AwardReputationKind,
    ) {
        let Some(player) = data.player_mut(player_id) else {
            return;
        };
        // Season recognition: regressed value so the recognition event
        // context isn't anchored on a small-sample raw average for a
        // late-season-burst winner.
        let pos = player.position().position_group();
        let avg_rating = player.statistics.average_rating_realistic(pos);
        let matches_played = player.statistics.played + player.statistics.played_subs;
        let goals = player.statistics.goals;
        let assists = player.statistics.assists;
        let mut ctx = RecognitionEventContext::new(recognition_kind).with_league(league_id);
        if matches_played > 0 {
            ctx = ctx
                .with_avg_rating(avg_rating)
                .with_matches_played(matches_played as u16)
                .with_season_goals(goals as u16)
                .with_season_assists(assists as u16);
        }
        player.on_recognition_award(happiness_event, ctx, 330);

        let mut input = AwardReputationInput::new().with_league_id(league_id);
        if let Some(rep) = league_rep {
            input = input.with_league_reputation(rep);
        }
        if matches_played > 0 {
            input = input
                .with_avg_rating(avg_rating)
                .with_matches_played(matches_played as u16);
        }
        player.apply_award_reputation_impact(reputation_kind, input, now);
    }
}
