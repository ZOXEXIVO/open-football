//! Squad-wide [`ChemistryContext`] builder.
//!
//! The per-relation update inside each player's `process_weekly_update`
//! recalculates a local (per-player) view of chemistry but can't see
//! captain, leadership, turnover. This builder feeds those squad-level
//! signals back to every player so they all share a coherent chemistry
//! number — the one read by training, match rating, selection.

use crate::club::relations::ChemistryContext;
use crate::club::team::Team;
use chrono::NaiveDate;
use std::collections::HashMap;

/// Cutoff window for "recent signing" status used by the chemistry model.
/// Anyone whose last transfer falls within this window counts toward the
/// dressing-room turnover signal.
const RECENT_SIGNING_WINDOW_DAYS: i64 = 90;

/// How many top leaders / influencers are surfaced to the chemistry model.
/// Tuned to the natural size of a dressing-room leadership core.
const TOP_RANK_SCORES: usize = 3;

pub struct ChemistryContextBuilder;

impl ChemistryContextBuilder {
    /// Build the squad-wide chemistry context consumed by every player's
    /// chemistry recalculation this week.
    pub fn build(team: &Team, today: NaiveDate) -> ChemistryContext {
        let top_leadership_scores = Self::top_leadership_scores(team);
        let top_influence_scores = Self::top_influence_scores(team);
        let recent_signings_90d = Self::recent_signings(team, today);
        let inner_circle_cohesion = Self::cohesion_avg(team);

        ChemistryContext {
            top_leadership_scores,
            top_influence_scores,
            captain_id: team.captain_id,
            vice_captain_id: team.vice_captain_id,
            recent_signings_90d,
            inner_circle_cohesion,
        }
    }

    /// Top-N leadership scores. Raw 0..20 attribute (skills.mental.leadership).
    fn top_leadership_scores(team: &Team) -> Vec<f32> {
        let mut leadership: Vec<f32> = team
            .players
            .players
            .iter()
            .map(|p| p.skills.mental.leadership.clamp(0.0, 20.0))
            .collect();
        leadership.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
        leadership.into_iter().take(TOP_RANK_SCORES).collect()
    }

    /// Top-N influence scores — sum of `relation.influence` references TO
    /// each player from every other player. Captures dressing-room
    /// standing distinct from raw leadership.
    fn top_influence_scores(team: &Team) -> Vec<f32> {
        let mut influence_totals: HashMap<u32, f32> = HashMap::new();
        for p in team.players.players.iter() {
            for (id, rel) in p.relations.player_relations_iter() {
                *influence_totals.entry(*id).or_insert(0.0) += rel.influence;
            }
        }
        let mut influences: Vec<f32> = influence_totals.into_values().collect();
        influences.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
        influences.into_iter().take(TOP_RANK_SCORES).collect()
    }

    fn recent_signings(team: &Team, today: NaiveDate) -> u8 {
        let cutoff = today - chrono::Duration::days(RECENT_SIGNING_WINDOW_DAYS);
        team.players
            .players
            .iter()
            .filter(|p| p.last_transfer_date.map(|d| d >= cutoff).unwrap_or(false))
            .count()
            .min(u8::MAX as usize) as u8
    }

    /// Average inner-circle cohesion across the squad — a coarse signal of
    /// how clique-y / cohesive the dressing room feels.
    fn cohesion_avg(team: &Team) -> f32 {
        if team.players.players.is_empty() {
            0.0
        } else {
            team.players
                .players
                .iter()
                .map(|p| p.relations.inner_circle_cohesion())
                .sum::<f32>()
                / team.players.players.len() as f32
        }
    }
}
