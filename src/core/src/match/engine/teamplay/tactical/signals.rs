//! **The tactical signals** — the pure math behind every scalar
//! [`TeamTacticalState`] carries: game management, press intensity,
//! compactness, width, tempo, risk appetite, rest defence and build-up
//! patience.
//!
//! Every one of them is a function of the side's style
//! ([`StyleInputs`], read off its plan), the score, the clock and the phase
//! — nothing here reads a player's ability. The quality-aware corrections
//! that DO are in [`quality`](super::quality), and `refresh` applies them
//! on top of these.

use crate::r#match::engine::teamplay::coach::plan::GamePlan;
use crate::r#match::engine::teamplay::tactical::phase::GamePhase;
use crate::r#match::engine::teamplay::tactical::team_state::TeamTacticalState;

/// What the signals read of a side's football: how hard it presses and how
/// hard straight after losing the ball, how compact it stays, how wide it
/// plays, its tempo, its patience, its appetite for risk, how far its line
/// sits from the phase table's, and how many men it keeps back.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StyleInputs {
    pub pressing: f32,
    pub counter_press: f32,
    pub compactness: f32,
    pub width: f32,
    pub tempo: f32,
    pub patience: f32,
    pub risk: f32,
    /// Attacking progress added to every phase's line height.
    pub line_shift: f32,
    pub rest_defence: f32,
}

impl StyleInputs {
    /// How far a dial's full range moves each input either side of the
    /// neutral plan.
    const SPAN: f32 = 0.8;
    const RISK_SPAN: f32 = 0.4;
    const PATIENCE_SPAN: f32 = 0.4;
    /// A full-height block reads every phase this much further up the
    /// pitch, a full-depth one this much further back.
    const LINE_SPAN: f32 = 0.25;
    const REST_SPAN: f32 = 4.0;

    /// The neutral plan: the balanced style every constant in the engine
    /// was calibrated on, a 4-4-2's.
    fn neutral() -> Self {
        Self::of_style(0.6, 0.5, 0.6, 4)
    }

    fn of_style(pressing: f32, counter_press: f32, compactness: f32, defenders: usize) -> Self {
        let possession_signal = (counter_press - pressing.min(counter_press)).max(0.0);
        StyleInputs {
            pressing,
            counter_press,
            compactness,
            width: (1.0 - compactness).clamp(0.0, 1.0),
            tempo: (pressing * 0.5 + counter_press * 0.5).clamp(0.0, 1.0),
            patience: 0.45 + possession_signal * 0.40,
            risk: 0.45 + pressing * 0.20,
            line_shift: 0.0,
            rest_defence: defenders.clamp(2, 5) as f32,
        }
    }

    /// A plan's dials, read around the neutral plan. The press dial sets
    /// both presses; width is its own dial, not what compactness leaves.
    pub fn of_plan(plan: &GamePlan) -> Self {
        let n = Self::neutral();
        let d = |dial: f32| dial - 0.5;
        StyleInputs {
            pressing: (n.pressing + Self::SPAN * d(plan.press)).clamp(0.0, 1.0),
            counter_press: (n.counter_press + Self::SPAN * d(plan.press)).clamp(0.0, 1.0),
            compactness: (n.compactness + Self::SPAN * d(plan.compactness)).clamp(0.0, 1.0),
            width: (n.width + Self::SPAN * d(plan.width)).clamp(0.0, 1.0),
            tempo: (n.tempo + Self::SPAN * d(plan.tempo)).clamp(0.0, 1.0),
            patience: (n.patience - Self::PATIENCE_SPAN * d(plan.tempo)).clamp(0.0, 1.0),
            risk: (n.risk + Self::RISK_SPAN * d(plan.directness)).clamp(0.0, 1.0),
            line_shift: Self::LINE_SPAN * d(plan.block_height),
            rest_defence: (n.rest_defence - Self::REST_SPAN * d(plan.commitment)).clamp(2.0, 5.0),
        }
    }
}

impl TeamTacticalState {
    // ──────────────────────────────────────────────────────────────────
    // Pure compute helpers — no side effects, easy to unit-test. Kept
    // as associated functions on `TeamTacticalState` so all the team-
    // level math lives behind a single struct boundary instead of as
    // free helpers floating in the module.
    // ──────────────────────────────────────────────────────────────────

    /// How far up the pitch a phase holds the line, as attacking progress
    /// (0 its own goal line … 1 the opponent's), moved by the plan's block.
    pub(in crate::r#match::engine::teamplay::tactical) fn line_progress(
        phase: GamePhase,
        line_shift: f32,
    ) -> f32 {
        let base = match phase {
            GamePhase::HighPress | GamePhase::Attack => 0.55,
            GamePhase::AttackingTransition | GamePhase::Progression => 0.45,
            GamePhase::BuildUp => 0.25,
            GamePhase::MidBlock | GamePhase::DefensiveTransition => 1.0 / 3.0,
            GamePhase::LowBlock => 0.18,
        };
        (base + line_shift).clamp(0.05, 0.9)
    }

    /// How far each lever moves at a full stance (±1), before the regime's
    /// gain: the line in attacking progress, the men kept back, the press,
    /// the appetite for a risky pass, build-up patience and compactness.
    pub const STANCE_LINE: f32 = 0.12;
    pub const STANCE_REST: f32 = 2.0;
    pub const STANCE_PRESS: f32 = 0.4;
    pub const STANCE_RISK: f32 = 0.2;
    pub const STANCE_PATIENCE: f32 = 0.3;
    pub const STANCE_COMPACT: f32 = 0.15;
    /// The press, with half the stance on it, at which a side presses high.
    pub const HIGH_PRESS_FROM: f32 = 0.8;

    /// Whether a side presses high: its plan's press and how hard it is
    /// chasing the match.
    pub(in crate::r#match::engine::teamplay::tactical) fn wants_high_press(
        pressing: f32,
        stance: f32,
    ) -> bool {
        pressing + 0.5 * stance >= Self::HIGH_PRESS_FROM
    }

    /// Press intensity — how aggressively we hunt the ball when we
    /// don't have it: the plan's press, raised when the side wants to press
    /// high, cut by fatigue, and moved by the stance.
    pub(in crate::r#match::engine::teamplay::tactical) fn compute_press_intensity(
        tactic_pressing: f32,
        counter_press: f32,
        high_press: bool,
        avg_condition: f32,
        stance: f32,
        in_defensive_transition: bool,
    ) -> f32 {
        // Counter-press: short burst right after losing possession.
        // Even a defensive tactic does this for a few seconds.
        let base = if in_defensive_transition {
            tactic_pressing.max(counter_press * 0.95)
        } else {
            tactic_pressing
        };

        let with_coach = if high_press {
            (base + 0.15).min(1.0)
        } else {
            base
        };

        // Fatigue penalty: condition < 0.4 strongly suppresses press;
        // full condition leaves it untouched. 1.0 at cond ≥ 0.7, 0.4
        // at cond = 0.0.
        let fatigue_mult = (0.4 + (avg_condition / 0.7).min(1.0) * 0.6).clamp(0.4, 1.0);

        let stance_mult = (1.0 + Self::STANCE_PRESS * stance).clamp(0.45, 1.4);

        (with_coach * fatigue_mult * stance_mult).clamp(0.0, 1.0)
    }

    /// Compactness target — how tight the shape should be vertically
    /// and horizontally. Rises in defensive phases and falls in attack
    /// (need width to stretch defenders).
    pub(in crate::r#match::engine::teamplay::tactical) fn compute_compactness(
        tactic_compactness: f32,
        phase: GamePhase,
        stance: f32,
    ) -> f32 {
        let phase_bias: f32 = match phase {
            GamePhase::LowBlock => 0.20,
            GamePhase::MidBlock | GamePhase::DefensiveTransition => 0.10,
            GamePhase::Attack | GamePhase::AttackingTransition => -0.15,
            GamePhase::HighPress => -0.05,
            GamePhase::BuildUp | GamePhase::Progression => 0.0,
        };
        (tactic_compactness + phase_bias - Self::STANCE_COMPACT * stance).clamp(0.0, 1.0)
    }

    /// Width target — how spread out we want to be laterally: the plan's
    /// width, with a phase bias that pushes it up in attack and down when
    /// defending.
    pub(in crate::r#match::engine::teamplay::tactical) fn compute_team_width(
        base_width: f32,
        phase: GamePhase,
    ) -> f32 {
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

    /// Tempo — how fast we want to play: the plan's tempo, quicker in
    /// transitions and slower while building up.
    pub(in crate::r#match::engine::teamplay::tactical) fn compute_tempo(
        style_tempo: f32,
        phase: GamePhase,
    ) -> f32 {
        let phase_tempo: f32 = match phase {
            GamePhase::AttackingTransition | GamePhase::DefensiveTransition => 0.95,
            GamePhase::Attack | GamePhase::HighPress => 0.75,
            GamePhase::Progression => 0.55,
            GamePhase::BuildUp => 0.35,
            GamePhase::MidBlock | GamePhase::LowBlock => 0.40,
        };
        (style_tempo * 0.4 + phase_tempo * 0.6).clamp(0.10, 1.0)
    }

    /// Risk appetite — willingness to take a forward pass when the safe
    /// option exists: the plan's directness, moved by the stance.
    pub(in crate::r#match::engine::teamplay::tactical) fn compute_risk_appetite(
        base: f32,
        stance: f32,
    ) -> f32 {
        (base + Self::STANCE_RISK * stance).clamp(0.05, 1.0)
    }

    /// Rest-defence count — how many players the team keeps as a
    /// safety shield behind the ball during a settled attack: the plan's
    /// count, moved by the phase and the game state.
    pub(in crate::r#match::engine::teamplay::tactical) fn compute_rest_defense_count(
        rest_defence: f32,
        phase: GamePhase,
        stance: f32,
    ) -> u8 {
        let base = (rest_defence - Self::STANCE_REST * stance)
            .round()
            .clamp(2.0, 5.0) as i8;
        let phase_delta: i8 = match phase {
            // Sustained attack — pull one defender forward to overload.
            GamePhase::Attack => -1,
            GamePhase::AttackingTransition | GamePhase::HighPress => -1,
            // Defending — everyone at home.
            GamePhase::LowBlock => 1,
            _ => 0,
        };
        (base + phase_delta).clamp(2, 5) as u8
    }

    /// Build-up patience — how willing we are to recycle when forward
    /// progress is hard: the plan's, raised while protecting a result and
    /// lowered while chasing one.
    pub(in crate::r#match::engine::teamplay::tactical) fn compute_build_up_patience(
        base: f32,
        stance: f32,
        risk_appetite: f32,
    ) -> f32 {
        let risk_penalty = (1.0 - risk_appetite) * 0.10; // risk-averse → patient
        (base - Self::STANCE_PATIENCE * stance + risk_penalty).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn a_protecting_stance_suppresses_press() {
        let normal = TeamTacticalState::compute_press_intensity(0.8, 0.7, false, 0.9, 0.0, false);
        let leading = TeamTacticalState::compute_press_intensity(0.8, 0.7, false, 0.9, -0.7, false);
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
    fn the_neutral_plan_reads_as_the_balanced_style() {
        assert_eq!(StyleInputs::of_plan(&GamePlan::neutral()), StyleInputs::neutral());
    }

    #[test]
    fn width_rises_in_attack() {
        let attacking = TeamTacticalState::compute_team_width(0.5, GamePhase::Attack);
        let low_block = TeamTacticalState::compute_team_width(0.5, GamePhase::LowBlock);
        assert!(attacking > low_block);
    }

    #[test]
    fn tempo_high_in_transition_low_in_buildup() {
        let trans = TeamTacticalState::compute_tempo(0.6, GamePhase::AttackingTransition);
        let build = TeamTacticalState::compute_tempo(0.6, GamePhase::BuildUp);
        assert!(trans > build, "trans={trans} build={build}");
    }

    #[test]
    fn risk_appetite_rises_when_chasing() {
        let drawn = TeamTacticalState::compute_risk_appetite(0.57, 0.0);
        let chasing = TeamTacticalState::compute_risk_appetite(0.57, 0.6);
        assert!(chasing > drawn, "chasing={chasing} drawn={drawn}");
    }

    #[test]
    fn risk_appetite_falls_when_protecting() {
        let normal = TeamTacticalState::compute_risk_appetite(0.57, 0.0);
        let leading = TeamTacticalState::compute_risk_appetite(0.57, -0.7);
        assert!(leading < normal, "leading={leading} normal={normal}");
    }

    #[test]
    fn rest_defense_drops_when_chasing() {
        let normal =
            TeamTacticalState::compute_rest_defense_count(4.0, GamePhase::Progression, 0.0);
        let chasing_late =
            TeamTacticalState::compute_rest_defense_count(4.0, GamePhase::Progression, 0.8);
        assert!(chasing_late < normal);
    }

    #[test]
    fn rest_defense_rises_when_protecting_and_moves_at_most_two() {
        let normal =
            TeamTacticalState::compute_rest_defense_count(4.0, GamePhase::Progression, 0.0);
        let leading_late =
            TeamTacticalState::compute_rest_defense_count(4.0, GamePhase::Progression, -0.8);
        assert!(leading_late > normal);
        for stance in [-1.0, 1.0] {
            let moved =
                TeamTacticalState::compute_rest_defense_count(4.0, GamePhase::Progression, stance);
            assert!(moved.abs_diff(normal) <= 2);
        }
    }

    #[test]
    fn a_side_protecting_late_holds_a_deeper_line_and_keeps_more_men_back() {
        use crate::r#match::engine::teamplay::coach::card::Temperament;
        use crate::r#match::engine::teamplay::coach::stance::{
            Occasion, Outlook, Situation, Stance,
        };
        let outlook = Outlook::default();
        let at = |difference: i32| Situation {
            difference,
            minutes_left: 5.0,
            ..Situation::default()
        };
        // At the levers' full amplitude: the regime's gain only scales it.
        let lever =
            |difference: i32| Stance::reading(&outlook, &Temperament::neutral(), &at(difference));
        let (protecting, level) = (lever(1), lever(0));
        assert_eq!(outlook.occasion, Occasion::League);
        for phase in [GamePhase::MidBlock, GamePhase::LowBlock, GamePhase::Progression] {
            assert!(
                TeamTacticalState::line_progress(phase, TeamTacticalState::STANCE_LINE * protecting)
                    < TeamTacticalState::line_progress(phase, TeamTacticalState::STANCE_LINE * level)
            );
        }
        assert!(
            TeamTacticalState::compute_rest_defense_count(4.0, GamePhase::Progression, protecting)
                > TeamTacticalState::compute_rest_defense_count(
                    4.0,
                    GamePhase::Progression,
                    level
                )
        );
    }

    #[test]
    fn width_is_its_own_dial_not_what_compactness_leaves() {
        let compact = StyleInputs::of_plan(&GamePlan::from_pin("compact=0.9/width=0.8").unwrap());
        let loose = StyleInputs::of_plan(&GamePlan::from_pin("compact=0.2/width=0.8").unwrap());
        assert!(compact.compactness > loose.compactness);
        assert_eq!(compact.width, loose.width);
        let wide = StyleInputs::of_plan(&GamePlan::from_pin("compact=0.9/width=0.9").unwrap());
        assert!(
            TeamTacticalState::compute_team_width(wide.width, GamePhase::Attack)
                > TeamTacticalState::compute_team_width(compact.width, GamePhase::Attack)
        );
    }

    #[test]
    fn a_high_block_raises_the_line_in_every_phase() {
        let high = StyleInputs::of_plan(&GamePlan::from_pin("block=0.9").unwrap());
        let deep = StyleInputs::of_plan(&GamePlan::from_pin("block=0.1").unwrap());
        for phase in [
            GamePhase::BuildUp,
            GamePhase::Progression,
            GamePhase::Attack,
            GamePhase::AttackingTransition,
            GamePhase::HighPress,
            GamePhase::MidBlock,
            GamePhase::LowBlock,
            GamePhase::DefensiveTransition,
        ] {
            assert!(
                TeamTacticalState::line_progress(phase, high.line_shift)
                    > TeamTacticalState::line_progress(phase, 0.0)
                    && TeamTacticalState::line_progress(phase, 0.0)
                        > TeamTacticalState::line_progress(phase, deep.line_shift),
                "{phase:?}"
            );
        }
    }

    #[test]
    fn the_press_dial_sets_both_presses_and_commitment_the_men_kept_back() {
        let hunting = StyleInputs::of_plan(&GamePlan::from_pin("press=0.9/commit=0.9").unwrap());
        let sitting = StyleInputs::of_plan(&GamePlan::from_pin("press=0.1/commit=0.1").unwrap());
        assert!(hunting.pressing > sitting.pressing);
        assert!(hunting.counter_press > sitting.counter_press);
        assert!(hunting.rest_defence < sitting.rest_defence);
    }

    #[test]
    fn build_up_patience_higher_in_possession_style_with_lead() {
        let direct = TeamTacticalState::compute_build_up_patience(0.45, 0.0, 0.6);
        let possession_lead = TeamTacticalState::compute_build_up_patience(0.65, -0.7, 0.3);
        assert!(
            possession_lead > direct,
            "poss_lead={possession_lead} direct={direct}"
        );
    }
}
