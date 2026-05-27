use crate::continent::ContinentResult;
use crate::simulator::SimulatorData;
use crate::utils::DateUtils;
use crate::{
    AwardReputationInput, AwardReputationKind, HappinessEventType, RecognitionEventContext,
    RecognitionEventKind,
};
use rayon::prelude::*;
use std::cmp::Ordering;

/// World player-of-year. Runs once on year-end. Pools each continent's
/// ranking, picks the global top 3 (nominees) and the global #1
/// (winner). Reuses `ContinentResult::rank_continent` so the scoring
/// formula has a single source of truth.
pub(crate) struct WorldPlayerOfYearTick;

impl WorldPlayerOfYearTick {
    pub(crate) fn run(data: &mut SimulatorData) {
        let today = data.date.date();
        if !DateUtils::is_year_end(today) {
            return;
        }

        let mut combined: Vec<(u32, f32)> = data
            .continents
            .par_iter()
            .flat_map_iter(|c| ContinentResult::rank_continent(c, today))
            .collect();
        combined.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(Ordering::Equal)
                .then(a.0.cmp(&b.0))
        });

        let top_three: Vec<u32> = combined.iter().take(3).map(|(id, _)| *id).collect();
        let winner = combined.first().map(|(id, _)| *id);

        let runner_up_id = combined.get(1).map(|(id, _)| *id);
        let winner_score = combined.first().map(|(_, score)| *score);
        let winner_margin = combined
            .first()
            .zip(combined.get(1))
            .map(|((_, w), (_, r))| (*w - *r).max(0.0));

        for pid in &top_three {
            if let Some(player) = data.player_mut(*pid) {
                let mut ctx =
                    RecognitionEventContext::new(RecognitionEventKind::WorldPlayerOfYearNomination);
                if let Some(rup) = runner_up_id {
                    ctx = ctx.with_runner_up(rup);
                }
                player.on_recognition_award(
                    HappinessEventType::WorldPlayerOfYearNomination,
                    ctx,
                    330,
                );
            }
        }
        if let Some(id) = winner {
            if let Some(player) = data.player_mut(id) {
                let mut ctx = RecognitionEventContext::new(RecognitionEventKind::WorldPlayerOfYear);
                if let Some(rup) = runner_up_id {
                    ctx = ctx.with_runner_up(rup);
                }
                if let Some(margin) = winner_margin {
                    ctx = ctx.with_margin(margin);
                }
                if let Some(score) = winner_score {
                    ctx = ctx.with_avg_rating(score);
                }
                player.on_recognition_award(HappinessEventType::WorldPlayerOfYear, ctx, 330);
                player.apply_award_reputation_impact(
                    AwardReputationKind::WorldPlayerOfYear,
                    AwardReputationInput::new(),
                    today,
                );
            }
        }
    }
}
