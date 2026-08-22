//! **The tactical signals** — the pure math behind every scalar
//! [`TeamTacticalState`] carries: game management, press intensity,
//! compactness, width, tempo, risk appetite, rest defence and build-up
//! patience.
//!
//! Every one of them is a function of the tactic, the score, the clock
//! and the phase — nothing here reads a player's ability. The
//! quality-aware corrections that DO are in [`quality`](super::quality),
//! and `refresh` applies them on top of these.

use crate::r#match::MatchContext;
use crate::r#match::engine::teamplay::tactical::phase::GamePhase;
use crate::r#match::engine::teamplay::tactical::team_state::TeamTacticalState;

impl TeamTacticalState {
    // ──────────────────────────────────────────────────────────────────
    // Pure compute helpers — no side effects, easy to unit-test. Kept
    // as associated functions on `TeamTacticalState` so all the team-
    // level math lives behind a single struct boundary instead of as
    // free helpers floating in the module.
    // ──────────────────────────────────────────────────────────────────

    /// Game-management intensity from this team's perspective.
    /// Continuous [0.0, 1.0] signal driving "hold the score" behaviour:
    /// safer passes, slower tempo, hold the ball. A single scalar
    /// covers every case — strong team protecting a lead, underdog
    /// clinging to a narrow upset, team settling for a point late —
    /// instead of hard branching per scenario.
    pub(in crate::r#match::engine::teamplay::tactical) fn compute_game_management_intensity(
        score_diff: i8,
        minute: f32,
        my_avg_ability: u16,
        opp_avg_ability: u16,
    ) -> f32 {
        // Ramp up from minute 60 onward; max at minute 90.
        let late_factor = ((minute - 60.0).max(0.0) / 30.0).min(1.0);
        // Positive = we're the weaker side. 40 CA ≈ one league tier.
        let ability_gap =
            ((opp_avg_ability as f32 - my_avg_ability as f32) / 40.0).clamp(-1.0, 1.0);

        if score_diff > 0 {
            // Leading — defend the score. Lead_base 0.22 + late_bonus
            // 0.48 puts an equal-squad 1-goal lead at minute 90 at 0.70,
            // crossing the 0.55 prefer_possession threshold so coaches
            // and per-tick decisions both shift to "protect".
            //
            // The flat lead_base used to apply from minute 1 — a team
            // going 1-0 up in minute 10 sat off (tempo -9%, risk -12%,
            // press -12%) for eighty minutes while the trailer got the
            // chasing risk lift. That persistent asymmetry was the
            // equal-strength equalizer machine: 12v12 dev_match measured
            // 56% draws (real ~25%) with conceding teams scoring at ~3×
            // their baseline rate inside 15 minutes of falling behind.
            // Real teams keep playing after an early goal and only start
            // protecting around the hour mark — so the score-state part
            // of the signal now ramps with time (25% before HT, full at
            // minute 75) while the late_bonus keeps its existing shape.
            let lead_base = 0.22 + 0.18 * ((score_diff - 1).clamp(0, 2) as f32);
            let weaker_bonus = 0.25 * ability_gap.max(0.0);
            let manage_ramp =
                (0.25 + 0.75 * ((minute - 45.0).max(0.0) / 30.0).min(1.0)).clamp(0.25, 1.0);
            // Late bonus 0.48 → 0.30: at 0.48 an equal-squad 1-goal lead
            // hit 0.70 at minute 90 — deep into "protect" (the 0.55
            // prefer_possession threshold) — so leaders stopped scoring
            // entirely late while trailers pushed, compressing score
            // differences toward 0 (47% equal-strength draws) and
            // starving the 75-90min goal band (12% vs real 26%). At
            // 0.30 the same lead peaks at 0.52: cautious, slower, but
            // still capable of scoring the counter-punch goal that
            // kills real games off.
            let late_bonus = 0.30 * late_factor;
            // Scaled by the regime's single amplitude knob — see
            // `MatchContext::SCORE_REACTION_GAIN`. This is the biggest of
            // the score-reactive channels: `game_management_intensity`
            // feeds tempo, press, compactness, the defensive line drop,
            // risk appetite, build-up patience and rest defence, so it
            // has to move with the rest of the regime rather than being
            // tuned against it.
            (((lead_base + weaker_bonus) * manage_ramp + late_bonus)
                * MatchContext::score_reaction_gain())
            .clamp(0.0, 0.95)
        } else if score_diff == 0 && ability_gap > 0.2 && late_factor > 0.5 {
            // Weaker team late in a draw plays for the point.
            ((0.15 + 0.20 * late_factor) * MatchContext::score_reaction_gain()).clamp(0.0, 0.5)
        } else {
            0.0
        }
    }

    /// Press intensity — how aggressively we hunt the ball when we
    /// don't have it. Combines tactical style, coach intent, fatigue,
    /// and game state. Pure function so it's testable.
    ///
    /// A team only "presses" when it has the energy and stylistic
    /// intent. A tired late-lead team should sit off; a fresh attacking
    /// 4-3-3 with PushForward instruction should hunt.
    pub(in crate::r#match::engine::teamplay::tactical) fn compute_press_intensity(
        tactic_pressing: f32,
        counter_press: f32,
        coach_high_press: bool,
        avg_condition: f32,
        game_management_intensity: f32,
        in_defensive_transition: bool,
    ) -> f32 {
        // Counter-press: short burst right after losing possession.
        // Even a defensive tactic does this for a few seconds.
        let base = if in_defensive_transition {
            tactic_pressing.max(counter_press * 0.95)
        } else {
            tactic_pressing
        };

        // Coach instruction can push press up but never below the
        // tactical floor.
        let with_coach = if coach_high_press {
            (base + 0.15).min(1.0)
        } else {
            base
        };

        // Fatigue penalty: condition < 0.4 strongly suppresses press;
        // full condition leaves it untouched. 1.0 at cond ≥ 0.7, 0.4
        // at cond = 0.0.
        let fatigue_mult = (0.4 + (avg_condition / 0.7).min(1.0) * 0.6).clamp(0.4, 1.0);

        // Game management: protecting a lead late = sit off.
        let gm_mult = (1.0 - game_management_intensity * 0.55).clamp(0.45, 1.0);

        (with_coach * fatigue_mult * gm_mult).clamp(0.0, 1.0)
    }

    /// Compactness target — how tight the shape should be vertically
    /// and horizontally. Rises in defensive phases and falls in attack
    /// (need width to stretch defenders).
    pub(in crate::r#match::engine::teamplay::tactical) fn compute_compactness(
        tactic_compactness: f32,
        phase: GamePhase,
        game_management_intensity: f32,
    ) -> f32 {
        let phase_bias: f32 = match phase {
            GamePhase::LowBlock => 0.20,
            GamePhase::MidBlock | GamePhase::DefensiveTransition => 0.10,
            GamePhase::Attack | GamePhase::AttackingTransition => -0.15,
            GamePhase::HighPress => -0.05,
            GamePhase::BuildUp | GamePhase::Progression => 0.0,
        };
        (tactic_compactness + phase_bias + game_management_intensity * 0.15).clamp(0.0, 1.0)
    }

    /// Width target — how spread out we want to be laterally. Inverse
    /// of compactness, with a phase bias that pushes width up in attack
    /// and down when defending.
    pub(in crate::r#match::engine::teamplay::tactical) fn compute_team_width(
        tactic_compactness: f32,
        phase: GamePhase,
    ) -> f32 {
        let base_width = (1.0 - tactic_compactness).clamp(0.0, 1.0);
        let phase_bias: f32 = match phase {
            GamePhase::Attack => 0.15,
            GamePhase::Progression | GamePhase::AttackingTransition => 0.05,
            GamePhase::BuildUp => 0.10, // CBs split, full-backs push wide
            GamePhase::LowBlock => -0.20,
            GamePhase::MidBlock => -0.10,
            GamePhase::DefensiveTransition => -0.05,
            GamePhase::HighPress => 0.0,
        };
        (base_width + phase_bias).clamp(0.0, 1.0)
    }

    /// Tempo — how fast we want to play. Counter-attack and
    /// transitions are high tempo; possession styles and game
    /// management are slow.
    pub(in crate::r#match::engine::teamplay::tactical) fn compute_tempo(
        tactic_pressing: f32,
        counter_press: f32,
        phase: GamePhase,
        game_management_intensity: f32,
    ) -> f32 {
        // Default tempo from style — pressing and counter-attack
        // tactics play faster.
        let style_tempo = (tactic_pressing * 0.5 + counter_press * 0.5).clamp(0.0, 1.0);
        let phase_tempo: f32 = match phase {
            GamePhase::AttackingTransition | GamePhase::DefensiveTransition => 0.95,
            GamePhase::Attack | GamePhase::HighPress => 0.75,
            GamePhase::Progression => 0.55,
            GamePhase::BuildUp => 0.35,
            GamePhase::MidBlock | GamePhase::LowBlock => 0.40,
        };
        let blended = style_tempo * 0.4 + phase_tempo * 0.6;
        // Game-management drags tempo down — protecting a lead = slow.
        (blended - game_management_intensity * 0.40).clamp(0.10, 1.0)
    }

    /// Risk appetite — willingness to take a forward pass / shot when
    /// the safe option exists. Low when leading or game-managing; HIGH
    /// when chasing late.
    pub(in crate::r#match::engine::teamplay::tactical) fn compute_risk_appetite(
        score_diff: i8,
        minute: f32,
        game_management_intensity: f32,
        tactic_pressing: f32,
    ) -> f32 {
        // Base from tactic — attacking tactics start more risk-tolerant.
        let base = 0.45 + tactic_pressing * 0.20;
        // Chasing late = take risks. Symmetric with game-management.
        // Early floor trimmed 0.4 → 0.25: a team conceding in minute 15
        // doesn't abandon its structure — real urgency arrives with the
        // clock. Pairs with the game-management time ramp above to break
        // the lead→equalizer feedback loop at equal strength.
        // Magnitude trimmed (0.20+0.10d → 0.14+0.07d) as part of making
        // the score-reactive regime net-neutral in goal expectation: a
        // score-blind A/B run measured the regime carrying the ENTIRE
        // +17pp equal-strength draw-correlation surplus (rho +0.49 with
        // score reactions on, −0.05 blind). Real chasing pressure exists
        // but converts poorly — the volume lift here is paired with the
        // desperation conversion penalty in the shot resolver.
        // Scaled with the rest of the regime — see
        // `MatchContext::SCORE_REACTION_GAIN`.
        let chasing_factor = if score_diff < 0 {
            let late_factor = ((minute - 60.0).max(0.0) / 30.0).min(1.0);
            let deficit = (-score_diff).min(3) as f32;
            (0.08 + deficit * 0.04)
                * (0.25 + late_factor * 0.75)
                * MatchContext::score_reaction_gain()
        } else {
            0.0
        };
        let base_with_chase = base + chasing_factor;
        // Game-management suppresses risk — slope 0.55 → 0.38: a
        // protecting team plays safer but keeps genuine counter intent
        // (the post-62' state-rate instrument showed leaders collapsing
        // to 0.61 goals/90 vs real ~1.4 — real leads are killed off on
        // the break, and the risk slope was the leader's main forward-
        // pass suppressor through the pass evaluator's 0.7-1.3x bias).
        (base_with_chase - game_management_intensity * 0.38).clamp(0.05, 1.0)
    }

    /// Rest-defence count — how many players the team keeps as a
    /// safety shield behind the ball during a settled attack. Function
    /// of the number of nominal defenders, the phase, and game state.
    pub(in crate::r#match::engine::teamplay::tactical) fn compute_rest_defense_count(
        nominal_defender_count: usize,
        phase: GamePhase,
        score_diff: i8,
        minute: f32,
    ) -> u8 {
        let base = nominal_defender_count.clamp(2, 5) as i8;
        let phase_delta: i8 = match phase {
            // Sustained attack — pull one defender forward to overload.
            GamePhase::Attack => -1,
            GamePhase::AttackingTransition | GamePhase::HighPress => -1,
            // Defending — everyone at home.
            GamePhase::LowBlock => 1,
            _ => 0,
        };
        // Chasing late — sacrifice a defender to push for the goal. A
        // whole man is not a magnitude the regime's gain can scale, so
        // this rung moves in the clock instead, like the coach ladder —
        // see `MatchContext::score_reaction_threshold`.
        let from = 90.0 - (90.0 - 75.0) * MatchContext::score_reaction_gain();
        let chasing_delta: i8 = if minute <= from {
            0
        } else if score_diff < 0 {
            -1
        } else if score_diff > 0 {
            1
        } else {
            0
        };
        (base + phase_delta + chasing_delta).clamp(2, 5) as u8
    }

    /// Build-up patience — how willing we are to recycle when forward
    /// progress is hard. High in possession styles + leading; low in
    /// counter-attack / chasing.
    pub(in crate::r#match::engine::teamplay::tactical) fn compute_build_up_patience(
        tactic_pressing: f32,
        counter_press: f32,
        game_management_intensity: f32,
        risk_appetite: f32,
    ) -> f32 {
        // Possession style: counter_press elevated relative to
        // pressing intensity. Counter-attack: counter_press low.
        let possession_signal = (counter_press - tactic_pressing.min(counter_press)).max(0.0);
        let base = 0.45 + possession_signal * 0.40;
        let gm_bonus = game_management_intensity * 0.30;
        let risk_penalty = (1.0 - risk_appetite) * 0.10; // risk-averse → patient
        (base + gm_bonus + risk_penalty).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn losing_side_never_manages_the_game() {
        assert_eq!(
            TeamTacticalState::compute_game_management_intensity(-1, 85.0, 120, 140),
            0.0
        );
        assert_eq!(
            TeamTacticalState::compute_game_management_intensity(-2, 30.0, 150, 150),
            0.0
        );
    }

    #[test]
    fn early_small_lead_produces_mild_signal() {
        let v = TeamTacticalState::compute_game_management_intensity(1, 20.0, 140, 140);
        assert!(v > 0.0 && v < 0.25, "got {v}");
    }

    #[test]
    fn weaker_side_protecting_late_lead_parks_the_bus() {
        let strong_even = TeamTacticalState::compute_game_management_intensity(1, 85.0, 150, 150);
        let weak_late = TeamTacticalState::compute_game_management_intensity(1, 85.0, 110, 150);
        assert!(
            weak_late > strong_even,
            "weak_late={weak_late} strong_even={strong_even}"
        );
        // The absolute height belongs to `MatchContext::SCORE_REACTION_GAIN`
        // — it is the calibration knob for the whole score-reactive regime
        // and it is titrated against the draw-correlation surplus, not
        // against this shape. Divide it back out so the test asks what it
        // has always meant to ask: that the SHAPE of the signal puts a
        // weaker side protecting a late lead deep into "park the bus".
        let gain = MatchContext::score_reaction_gain();
        assert!(
            gain > 0.0,
            "the regime is off; this test cannot mean anything"
        );
        assert!(weak_late / gain > 0.5, "got {weak_late} at gain {gain}");
    }

    #[test]
    fn weaker_side_late_draw_plays_for_point() {
        let v = TeamTacticalState::compute_game_management_intensity(0, 85.0, 110, 150);
        assert!(v > 0.0 && v < 0.5, "got {v}");
    }

    #[test]
    fn intensity_is_clamped_below_one() {
        let v = TeamTacticalState::compute_game_management_intensity(5, 90.0, 100, 160);
        assert!(v <= 0.95, "got {v}");
    }

    #[test]
    fn fresh_attacking_press_is_high() {
        // Pressing tactic, fresh players, no game management → high.
        let v = TeamTacticalState::compute_press_intensity(1.0, 0.9, false, 0.9, 0.0, false);
        assert!(v > 0.85, "got {v}");
    }

    #[test]
    fn tired_team_presses_less_than_fresh() {
        let fresh = TeamTacticalState::compute_press_intensity(0.8, 0.7, false, 0.9, 0.0, false);
        let tired = TeamTacticalState::compute_press_intensity(0.8, 0.7, false, 0.30, 0.0, false);
        assert!(tired < fresh, "tired={tired} fresh={fresh}");
    }

    #[test]
    fn late_lead_suppresses_press() {
        let normal = TeamTacticalState::compute_press_intensity(0.8, 0.7, false, 0.9, 0.0, false);
        let leading = TeamTacticalState::compute_press_intensity(0.8, 0.7, false, 0.9, 0.7, false);
        assert!(leading < normal, "leading={leading} normal={normal}");
    }

    #[test]
    fn defensive_transition_boosts_press() {
        // Even a defensive tactic counter-presses briefly.
        let v = TeamTacticalState::compute_press_intensity(0.3, 0.85, false, 0.9, 0.0, true);
        assert!(v > 0.5, "got {v}");
    }

    #[test]
    fn compactness_rises_in_low_block() {
        let attacking = TeamTacticalState::compute_compactness(0.5, GamePhase::Attack, 0.0);
        let low_block = TeamTacticalState::compute_compactness(0.5, GamePhase::LowBlock, 0.0);
        assert!(low_block > attacking);
    }

    #[test]
    fn width_rises_in_attack() {
        let attacking = TeamTacticalState::compute_team_width(0.5, GamePhase::Attack);
        let low_block = TeamTacticalState::compute_team_width(0.5, GamePhase::LowBlock);
        assert!(attacking > low_block);
    }

    #[test]
    fn tempo_high_in_transition_low_in_buildup() {
        let trans = TeamTacticalState::compute_tempo(0.6, 0.6, GamePhase::AttackingTransition, 0.0);
        let build = TeamTacticalState::compute_tempo(0.6, 0.6, GamePhase::BuildUp, 0.0);
        assert!(trans > build, "trans={trans} build={build}");
    }

    #[test]
    fn game_management_drops_tempo() {
        let normal = TeamTacticalState::compute_tempo(0.6, 0.6, GamePhase::Progression, 0.0);
        let managing = TeamTacticalState::compute_tempo(0.6, 0.6, GamePhase::Progression, 0.7);
        assert!(managing < normal);
    }

    #[test]
    fn risk_appetite_rises_when_chasing_late() {
        let drawn = TeamTacticalState::compute_risk_appetite(0, 80.0, 0.0, 0.6);
        let chasing = TeamTacticalState::compute_risk_appetite(-2, 80.0, 0.0, 0.6);
        assert!(chasing > drawn, "chasing={chasing} drawn={drawn}");
    }

    #[test]
    fn risk_appetite_falls_when_leading_late() {
        let normal = TeamTacticalState::compute_risk_appetite(0, 80.0, 0.0, 0.6);
        let leading = TeamTacticalState::compute_risk_appetite(1, 85.0, 0.7, 0.6);
        assert!(leading < normal, "leading={leading} normal={normal}");
    }

    #[test]
    fn rest_defense_drops_when_chasing_late() {
        let normal =
            TeamTacticalState::compute_rest_defense_count(4, GamePhase::Progression, 0, 60.0);
        let chasing_late =
            TeamTacticalState::compute_rest_defense_count(4, GamePhase::Attack, -1, 85.0);
        assert!(chasing_late < normal);
    }

    #[test]
    fn rest_defense_rises_when_leading_late() {
        let normal =
            TeamTacticalState::compute_rest_defense_count(4, GamePhase::Progression, 0, 60.0);
        let leading_late =
            TeamTacticalState::compute_rest_defense_count(4, GamePhase::LowBlock, 1, 85.0);
        assert!(leading_late >= normal);
    }

    #[test]
    fn build_up_patience_higher_in_possession_style_with_lead() {
        let direct = TeamTacticalState::compute_build_up_patience(0.9, 0.4, 0.0, 0.6);
        let possession_lead = TeamTacticalState::compute_build_up_patience(0.4, 0.9, 0.7, 0.3);
        assert!(
            possession_lead > direct,
            "poss_lead={possession_lead} direct={direct}"
        );
    }
}
