//! Match-time psychology: per-player confidence/nervousness/momentum
//! plus team momentum and leadership damping. Pure helpers + a stateful
//! `PsychologyState` that lives on `MatchContext`.
//!
//! All probability/skill modifiers are clamped at the consumer side; the
//! helpers here just produce the deltas. RNG belongs at event resolution.

use std::collections::HashMap;

use crate::r#match::engine::environment::MatchEnvironment;

/// Per-player transient state tracked across the match.
///
/// All fields on -1..+1 / 0..1 ranges; the consumer translates into skill
/// deltas via `skill_modifiers`.
#[derive(Debug, Clone, Copy)]
pub struct PsychState {
    /// -1..+1 — strong negative after errors, positive after good
    /// involvement. Initial value derives from morale + personality.
    pub confidence: f32,
    /// 0..1 — dampened by composure/pressure attributes; raised by
    /// recent errors and yellow cards (for low-temperament players).
    pub nervousness: f32,
    /// -0.5..+0.5 — short-lived swing after goal/error/red card.
    pub momentum_boost: f32,
    /// Tick of the last error leading to a shot/goal — drives short-
    /// window confidence damping.
    pub mistake_memory_tick: Option<u64>,
    /// Tick of the last goal/assist/key tackle — drives short-window
    /// confidence boost.
    pub goal_involvement_tick: Option<u64>,
}

impl Default for PsychState {
    fn default() -> Self {
        PsychState {
            confidence: 0.0,
            nervousness: 0.0,
            momentum_boost: 0.0,
            mistake_memory_tick: None,
            goal_involvement_tick: None,
        }
    }
}

impl PsychState {
    pub fn clamp(&mut self) {
        self.confidence = self.confidence.clamp(-1.0, 1.0);
        self.nervousness = self.nervousness.clamp(0.0, 1.0);
        self.momentum_boost = self.momentum_boost.clamp(-0.5, 0.5);
    }
}

/// Multiplicative/additive modifiers a player's psychology applies to
/// specific skills/probabilities. Caller applies these on top of the
/// player's normalised skill values.
#[derive(Debug, Clone, Copy, Default)]
pub struct SkillModifiers {
    /// Multiplier applied to composure (1.0 = no change).
    pub composure_mul: f32,
    /// Multiplier applied to decisions.
    pub decisions_mul: f32,
    /// Multiplier applied to flair.
    pub flair_mul: f32,
    /// Multiplier applied to first_touch.
    pub first_touch_mul: f32,
    /// Additive bump to miscontrol probability (0..1).
    pub miscontrol_add: f32,
    /// Additive bump to rushed-clearance probability.
    pub rushed_clearance_add: f32,
    /// Additive bump to foul risk.
    pub foul_risk_add: f32,
}

impl SkillModifiers {
    pub fn neutral() -> Self {
        SkillModifiers {
            composure_mul: 1.0,
            decisions_mul: 1.0,
            flair_mul: 1.0,
            first_touch_mul: 1.0,
            miscontrol_add: 0.0,
            rushed_clearance_add: 0.0,
            foul_risk_add: 0.0,
        }
    }
}

/// Initial confidence from morale + personality. All inputs in 0..20 except
/// `morale_0_100`.
pub fn initial_confidence(
    morale_0_100: f32,
    important_matches_0_20: f32,
    is_important_match: bool,
) -> f32 {
    // Morale 0..100 → -0.15..+0.15.
    let morale_term = ((morale_0_100 - 50.0) / 50.0).clamp(-1.0, 1.0) * 0.15;
    let big_match_bonus = if is_important_match {
        // Players with high important_matches lift in big games.
        ((important_matches_0_20 / 20.0).clamp(0.0, 1.0) - 0.5) * 0.10
    } else {
        0.0
    };
    (morale_term + big_match_bonus).clamp(-0.30, 0.30)
}

/// Initial nervousness — important matches raise nervousness, but
/// pressure attribute and high composure damp it.
pub fn initial_nervousness(
    pressure_attr_0_20: f32,
    composure_0_20: f32,
    match_importance_0_1: f32,
) -> f32 {
    let pressure = (pressure_attr_0_20 / 20.0).clamp(0.0, 1.0);
    let composure = (composure_0_20 / 20.0).clamp(0.0, 1.0);
    let raw = match_importance_0_1 * 0.30 - pressure * 0.18 - composure * 0.10;
    raw.clamp(0.0, 1.0)
}

/// Match-pressure load formula from spec.
///
/// `pressure_load = match_importance*0.30 + derby*0.18 + late_close*0.22
/// + crowd*0.10 + recent_mistake*0.12 - leadership_support*0.10`
///
/// All inputs in 0..1.
pub fn pressure_load(
    env: &MatchEnvironment,
    late_close_score: f32,
    recent_mistake: f32,
    leadership_support: f32,
) -> f32 {
    let raw = env.match_importance * 0.30
        + env.derby_intensity * 0.18
        + late_close_score.clamp(0.0, 1.0) * 0.22
        + env.crowd_intensity * 0.10
        + recent_mistake.clamp(0.0, 1.0) * 0.12
        - leadership_support.clamp(0.0, 1.0) * 0.10;
    raw.clamp(0.0, 1.0)
}

/// Compute the skill modifiers a `PsychState` applies. Pure function —
/// driven entirely by the state's confidence + nervousness.
pub fn skill_modifiers(state: &PsychState) -> SkillModifiers {
    let mut m = SkillModifiers::neutral();
    if state.confidence > 0.4 {
        m.composure_mul = 1.03;
        m.decisions_mul = 1.02;
        m.flair_mul = 1.03;
    } else if state.confidence < -0.4 {
        m.composure_mul = 0.95;
        m.first_touch_mul = 0.96;
        m.decisions_mul = 0.96;
    }
    if state.nervousness > 0.6 {
        m.miscontrol_add = 0.06;
        m.rushed_clearance_add = 0.08;
        m.foul_risk_add = 0.04;
    }
    m
}

/// Confidence delta from a positive event. Caller adds it (clamped).
#[derive(Debug, Clone, Copy)]
pub enum PositiveEvent {
    Goal,
    Assist,
    BigTackle,
    BigSave,
}

pub fn confidence_delta_positive(event: PositiveEvent) -> f32 {
    match event {
        PositiveEvent::Goal => 0.10,
        PositiveEvent::Assist => 0.06,
        PositiveEvent::BigTackle | PositiveEvent::BigSave => 0.04,
    }
}

/// Confidence delta from a negative event.
#[derive(Debug, Clone, Copy)]
pub enum NegativeEvent {
    /// Misplaced pass / miscontrol that led to an opposition shot.
    ErrorLeadingToShot,
    /// Same, but it produced a goal.
    ErrorLeadingToGoal,
    /// Yellow card (penalised more for low-temperament players elsewhere).
    YellowCard,
}

pub fn confidence_delta_negative(event: NegativeEvent) -> f32 {
    match event {
        NegativeEvent::ErrorLeadingToShot => -0.10,
        NegativeEvent::ErrorLeadingToGoal => -0.20,
        NegativeEvent::YellowCard => -0.04,
    }
}

/// Leadership team score (0..1).
///
/// captain_leadership*0.35 + captain_teamwork*0.15 + captain_determination*0.18
/// + captain_pressure*0.18 + vice_leadership*0.08 + gk_communication*0.06.
/// All 0..20 inputs.
pub fn team_leadership_score(
    captain_leadership: f32,
    captain_teamwork: f32,
    captain_determination: f32,
    captain_pressure: f32,
    vice_leadership: f32,
    gk_communication: f32,
) -> f32 {
    let n = |x: f32| (x / 20.0).clamp(0.0, 1.0);
    n(captain_leadership) * 0.35
        + n(captain_teamwork) * 0.15
        + n(captain_determination) * 0.18
        + n(captain_pressure) * 0.18
        + n(vice_leadership) * 0.08
        + n(gk_communication) * 0.06
}

/// Goalkeeper communication score (0..1) — drives defensive line quality.
///
/// communication*0.25 + command_of_area*0.25 + positioning*0.15
/// + concentration*0.15 + leadership*0.10 + age proxy*0.10.
pub fn keeper_communication_score(
    communication_0_20: f32,
    command_of_area_0_20: f32,
    positioning_0_20: f32,
    concentration_0_20: f32,
    leadership_0_20: f32,
    experience_0_1: f32,
) -> f32 {
    let n = |x: f32| (x / 20.0).clamp(0.0, 1.0);
    n(communication_0_20) * 0.25
        + n(command_of_area_0_20) * 0.25
        + n(positioning_0_20) * 0.15
        + n(concentration_0_20) * 0.15
        + n(leadership_0_20) * 0.10
        + experience_0_1.clamp(0.0, 1.0) * 0.10
}

/// Per-team momentum. Set positive after a goal scored; set negative
/// after concession or red card. Decays linearly over a window.
#[derive(Debug, Clone, Copy, Default)]
pub struct TeamMomentum {
    pub value: f32, // -1..+1
    /// Tick at which the momentum boost was applied. Used to decay
    /// over the configured window (~600 ticks).
    pub set_tick: u64,
}

const MOMENTUM_DECAY_TICKS: u64 = 600;

impl TeamMomentum {
    pub fn apply_event(&mut self, current_tick: u64, delta: f32) {
        // Stack with existing value but pull strongly toward the new event.
        let blended = self.value * 0.4 + delta;
        self.value = blended.clamp(-1.0, 1.0);
        self.set_tick = current_tick;
    }

    pub fn current(&self, now_tick: u64) -> f32 {
        let elapsed = now_tick.saturating_sub(self.set_tick);
        if elapsed >= MOMENTUM_DECAY_TICKS {
            return 0.0;
        }
        let remaining = (MOMENTUM_DECAY_TICKS - elapsed) as f32 / MOMENTUM_DECAY_TICKS as f32;
        self.value * remaining
    }
}

/// Damping applied by a captain/vice with high leadership. The higher
/// the team leadership score, the more the negative momentum is
/// absorbed (up to 35% per spec).
pub fn leadership_damped_momentum(raw_momentum: f32, team_leadership_0_1: f32) -> f32 {
    if raw_momentum >= 0.0 {
        return raw_momentum;
    }
    let damp = 1.0 - (team_leadership_0_1.clamp(0.0, 1.0) * 0.35);
    raw_momentum * damp
}

/// Container held on `MatchContext`.
#[derive(Debug, Clone, Default)]
pub struct PsychologyState {
    pub players: HashMap<u32, PsychState>,
    pub home_momentum: TeamMomentum,
    pub away_momentum: TeamMomentum,
}

impl PsychologyState {
    pub fn get_or_default(&mut self, player_id: u32) -> &mut PsychState {
        self.players.entry(player_id).or_default()
    }

    pub fn get(&self, player_id: u32) -> Option<&PsychState> {
        self.players.get(&player_id)
    }

    pub fn record_positive(&mut self, player_id: u32, event: PositiveEvent, tick: u64) {
        let s = self.get_or_default(player_id);
        s.confidence += confidence_delta_positive(event);
        s.goal_involvement_tick = Some(tick);
        s.clamp();
    }

    pub fn record_negative(&mut self, player_id: u32, event: NegativeEvent, tick: u64) {
        let s = self.get_or_default(player_id);
        s.confidence += confidence_delta_negative(event);
        s.mistake_memory_tick = Some(tick);
        // Yellows raise nervousness, weighted by the player's pressure
        // tolerance — caller should follow up with `apply_yellow_card`.
        s.clamp();
    }

    pub fn apply_yellow_card(&mut self, player_id: u32, temperament_0_20: f32) {
        let s = self.get_or_default(player_id);
        let temperament = (temperament_0_20 / 20.0).clamp(0.0, 1.0);
        // Low-temperament players rattled more.
        s.nervousness += 0.10 + (1.0 - temperament) * 0.12;
        s.clamp();
    }

    /// Apply an event-driven momentum shift to a team.
    pub fn record_team_event(&mut self, is_home: bool, delta: f32, tick: u64) {
        let m = if is_home {
            &mut self.home_momentum
        } else {
            &mut self.away_momentum
        };
        m.apply_event(tick, delta);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn high_morale_lifts_initial_confidence() {
        let high = initial_confidence(85.0, 14.0, true);
        let low = initial_confidence(20.0, 14.0, true);
        assert!(high > low);
        assert!((-0.30..=0.30).contains(&high));
        assert!((-0.30..=0.30).contains(&low));
    }

    #[test]
    fn pressure_attribute_reduces_initial_nervousness() {
        let cool = initial_nervousness(18.0, 16.0, 0.9);
        let nervous = initial_nervousness(6.0, 8.0, 0.9);
        assert!(cool < nervous);
    }

    #[test]
    fn pressure_load_clamped_unit() {
        let env = MatchEnvironment {
            match_importance: 1.0,
            derby_intensity: 1.0,
            crowd_intensity: 1.0,
            ..Default::default()
        };
        let load = pressure_load(&env, 1.0, 1.0, 0.0);
        assert!((0.0..=1.0).contains(&load));
        assert!(load > 0.5);
    }

    #[test]
    fn leadership_support_dampens_pressure() {
        let env = MatchEnvironment {
            match_importance: 0.7,
            derby_intensity: 0.5,
            ..Default::default()
        };
        let no_lead = pressure_load(&env, 0.5, 0.3, 0.0);
        let strong_lead = pressure_load(&env, 0.5, 0.3, 1.0);
        assert!(strong_lead < no_lead);
    }

    #[test]
    fn confidence_above_threshold_boosts_skills() {
        let s = PsychState {
            confidence: 0.6,
            nervousness: 0.0,
            ..Default::default()
        };
        let m = skill_modifiers(&s);
        assert!(m.composure_mul > 1.0);
        assert!(m.decisions_mul > 1.0);
        assert!(m.flair_mul > 1.0);
    }

    #[test]
    fn confidence_below_threshold_reduces_skills() {
        let s = PsychState {
            confidence: -0.6,
            nervousness: 0.0,
            ..Default::default()
        };
        let m = skill_modifiers(&s);
        assert!(m.composure_mul < 1.0);
        assert!(m.first_touch_mul < 1.0);
    }

    #[test]
    fn high_nervousness_increases_miscontrol_and_foul_risk() {
        let s = PsychState {
            confidence: 0.0,
            nervousness: 0.8,
            ..Default::default()
        };
        let m = skill_modifiers(&s);
        assert!(m.miscontrol_add > 0.0);
        assert!(m.foul_risk_add > 0.0);
    }

    #[test]
    fn captain_leadership_dampens_negative_momentum() {
        let raw = -0.6;
        let no_captain = leadership_damped_momentum(raw, 0.0);
        let strong_captain = leadership_damped_momentum(raw, 1.0);
        // Negative momentum is absorbed (closer to zero) with strong captain.
        assert!(strong_captain > no_captain);
        assert!(strong_captain < 0.0);
    }

    #[test]
    fn leadership_does_not_cap_positive_momentum() {
        let raw = 0.6;
        assert_eq!(leadership_damped_momentum(raw, 1.0), 0.6);
    }

    #[test]
    fn team_momentum_decays_to_zero() {
        let mut m = TeamMomentum::default();
        m.apply_event(100, 0.4);
        assert!(m.current(100) > 0.0);
        // Past decay window — fully decayed.
        assert_eq!(m.current(100 + MOMENTUM_DECAY_TICKS + 1), 0.0);
    }

    #[test]
    fn psychology_state_records_goal_and_error() {
        let mut p = PsychologyState::default();
        p.record_positive(7, PositiveEvent::Goal, 1000);
        let after_goal = p.get(7).unwrap().confidence;
        assert!(after_goal > 0.0);

        p.record_negative(7, NegativeEvent::ErrorLeadingToGoal, 1100);
        let after_error = p.get(7).unwrap().confidence;
        assert!(after_error < after_goal);
    }

    #[test]
    fn psychology_state_yellow_raises_low_temperament_nervousness_more() {
        let mut p = PsychologyState::default();
        p.apply_yellow_card(1, 18.0); // High temperament
        p.apply_yellow_card(2, 4.0); // Low temperament
        let cool = p.get(1).unwrap().nervousness;
        let rattled = p.get(2).unwrap().nervousness;
        assert!(rattled > cool);
    }

    #[test]
    fn keeper_communication_better_with_experience() {
        let young = keeper_communication_score(14.0, 14.0, 14.0, 14.0, 12.0, 0.1);
        let veteran = keeper_communication_score(14.0, 14.0, 14.0, 14.0, 12.0, 1.0);
        assert!(veteran > young);
    }
}
