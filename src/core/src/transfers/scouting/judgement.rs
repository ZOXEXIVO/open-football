//! What a scout concludes from what he can see.
//!
//! Three readings, all of them made from OBSERVABLE numbers and never from
//! hidden potential: how far a player might still grow, what the data
//! department makes of his output, and what the report has to flag as a
//! risk. They decide nothing on their own — the gates read them.

use crate::transfers::pipeline::ReportRiskFlag;
use crate::transfers::pipeline::processor::PlayerSummary;
use crate::transfers::scouting::breakout::{BreakoutPerformanceSignal, LeaguePerformanceLookup};
use crate::transfers::scouting::config::ScoutingConfig;

/// The scout's readings.
pub(in crate::transfers) struct ScoutJudgement;

impl ScoutJudgement {
    /// Derive risk flags for a scouted player from their observable signals.
    /// Buyer rep is passed in so we can flag wage demands that blow the budget.
    /// Thresholds (determination floor, age cutoff, contract-month window,
    /// rep gap) live in `ScoutingConfig::risk_flags`.
    pub(in crate::transfers) fn evaluate_risk_flags(
        is_injured: bool,
        determination: f32,
        age: u8,
        contract_months_remaining: i16,
        player_world_rep: i16,
        buyer_world_rep: i16,
    ) -> Vec<ReportRiskFlag> {
        ScoutingConfig::default().risk_flags_for(
            is_injured,
            determination,
            age,
            contract_months_remaining,
            player_world_rep,
            buyer_world_rep,
        )
    }

    /// Performance-adjusted data score used as a pre-scouting filter.
    /// Weights ability, form (rating × appearances), raw output (G+A), and
    /// the performance-breakout signal — so a high-output player whose
    /// *results* outrun his level rises up the data department's shortlist
    /// and actually gets watched, instead of being buried behind
    /// higher-ability names. The breakout term is league-reputation
    /// discounted inside the signal, so a flat-track scorer in a weak
    /// division doesn't leapfrog proven quality.
    pub(in crate::transfers) fn player_data_score(
        p: &PlayerSummary,
        perf: &LeaguePerformanceLookup,
    ) -> f32 {
        let ability = p.skill_ability as f32 * 0.4;
        let form = p.average_rating * (p.appearances.min(40) as f32 / 4.0);
        let output = ((p.goals + p.assists).min(30)) as f32 * 0.3;
        let breakout = BreakoutPerformanceSignal::compute(&perf.breakout_inputs(
            p.player_id,
            p.position_group,
            p.goals,
            p.assists,
            p.appearances,
            p.average_rating,
            p.age,
            p.seller_ctx.league_reputation,
        ));
        ability + form + output + breakout.score * 0.2
    }

    /// Estimate a player's growth potential from observable attributes.
    /// Scouts can't see PA — they judge ceiling from age, character, and current skill level.
    /// Young players with strong determination, work rate, composure show higher ceiling.
    pub(in crate::transfers) fn estimate_growth_potential(
        age: u8,
        determination: f32,
        work_rate: f32,
        composure: f32,
        anticipation: f32,
        current_skill_ability: u8,
    ) -> u8 {
        // Mental quality score: how much this player's character suggests growth (0.0-1.0)
        let mental_quality =
            ((determination + work_rate + composure + anticipation) / 4.0 - 1.0) / 19.0;
        let mental_factor = mental_quality.clamp(0.0, 1.0);

        // Age-based growth window: younger = more room to grow
        let base_growth = match age {
            0..=17 => 35.0,
            18 => 30.0,
            19 => 25.0,
            20 => 20.0,
            21 => 15.0,
            22 => 12.0,
            23 => 8.0,
            24 => 5.0,
            25 => 3.0,
            26..=27 => 1.0,
            _ => 0.0,
        };

        // Players already at high skill level have less room to grow
        let ceiling_factor = if current_skill_ability > 160 {
            0.3
        } else if current_skill_ability > 120 {
            0.6
        } else {
            1.0
        };

        (base_growth * mental_factor * ceiling_factor) as u8
    }
}
