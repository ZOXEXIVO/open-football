//! Match-time psychology: per-player confidence/nervousness/momentum
//! plus team momentum and leadership damping. Pure helpers + a stateful
//! `PsychologyState` that lives on `MatchContext`.
//!
//! All probability/skill modifiers are clamped at the consumer side; the
//! helpers here just produce the deltas. RNG belongs at event resolution.

use std::collections::HashMap;

use crate::club::player::mind::{KickoffMind, MindSwitch};
use crate::r#match::MatchPlayer;
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

/// Stateless namespace for pure psychology helpers — initial state, pressure
/// load, skill modifiers, confidence deltas, leadership scoring. Holding the
/// stateful containers (`PsychState`, `PsychologyState`, `TeamMomentum`)
/// untouched; this struct just groups the formula functions.
pub struct Psychology;

impl Psychology {
    /// A step up of this size or more is as unsettling as a step gets.
    /// Four divisions' worth on the generator's scale.
    const STEP_FULL: f32 = 0.12;
    const STEP_NERVES: f32 = 0.60;
    const OCCASION_NERVES: f32 = 0.35;

    /// How much rides on the fixture, 0..1: what is at stake, who is
    /// watching, who the opponent is, and whether there is a tomorrow.
    pub fn occasion(env: &MatchEnvironment, knockout: bool) -> f32 {
        (env.match_importance * 0.5
            + env.crowd_intensity * 0.2
            + env.derby_intensity * 0.2
            + if knockout { 0.1 } else { 0.0 })
        .clamp(0.0, 1.0)
    }

    /// Kickoff confidence: how good he feels and how happy he is, and — on
    /// a night that matters — whether he is a man who rises to it.
    pub fn initial_confidence(mind: &KickoffMind, important_matches_0_20: f32, occasion: f32) -> f32 {
        let morale = ((mind.morale - 50.0) / 50.0).clamp(-1.0, 1.0) * 0.15;
        let belief = mind.self_belief.clamp(-1.0, 1.0) * 0.25;
        let rises = ((important_matches_0_20 / 20.0).clamp(0.0, 1.0) - 0.5) * 0.10 * occasion;
        (morale + belief + rises).clamp(-0.5, 0.5)
    }

    /// Kickoff nerves: the step up he is taking from the football he is
    /// used to, and the occasion, answered by his temperament, his record
    /// on big nights and his belief.
    pub fn initial_nervousness(
        mind: &KickoffMind,
        step_up: f32,
        occasion: f32,
        pressure_0_20: f32,
        important_matches_0_20: f32,
        composure_0_20: f32,
    ) -> f32 {
        let n = |v: f32| (v / 20.0).clamp(0.0, 1.0);
        let step = (step_up / Self::STEP_FULL).clamp(0.0, 1.0) * Self::STEP_NERVES;
        let occasion = occasion
            * Self::OCCASION_NERVES
            * (1.0 - 0.7 * n(pressure_0_20))
            * (1.0 - 0.5 * n(important_matches_0_20));
        let damp = n(composure_0_20) * 0.15
            + (mind.big_match_record.clamp(-5, 5) as f32) * 0.02
            + mind.self_belief.max(0.0) * 0.10;
        (step + occasion - damp).clamp(0.0, 1.0)
    }

    /// His psychology at kickoff, read off the state of mind he brought and
    /// the match he walked into.
    pub fn kickoff_state(player: &MatchPlayer, standard: f32, occasion: f32) -> PsychState {
        let mind = &player.kickoff_mind;
        let step_up = mind.assurance.map_or(0.0, |a| (standard - a).max(0.0));
        PsychState {
            confidence: Self::initial_confidence(mind, player.attributes.important_matches, occasion),
            nervousness: Self::initial_nervousness(
                mind,
                step_up,
                occasion,
                player.attributes.pressure,
                player.attributes.important_matches,
                player.skills.mental.composure,
            ),
            ..PsychState::default()
        }
    }

    /// Match-pressure load formula from spec.
    ///
    /// `pressure_load = match_importance*0.30 + derby*0.18 + late_close*0.22
    /// + crowd*0.10 + recent_mistake*0.12 - leadership_support*0.10`
    ///
    /// All inputs in 0..1.
    ///
    /// TODO(matchday-leadership): pure + unit-tested, not yet wired into the
    /// live match loop. To use it, feed `leadership_support` from the on-pitch
    /// captain's leadership (resolved via `MatchdayLeadership`) and consume the
    /// load in the in-match psychology / decision path. Deferred to avoid
    /// shifting match outcomes until that change can be calibrated.
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
        let up = state.confidence.clamp(0.0, 1.0);
        let down = (-state.confidence).clamp(0.0, 1.0);
        let nerves = state.nervousness.clamp(0.0, 1.0) * Self::OUTFIELD_SHARE;
        SkillModifiers {
            composure_mul: 1.0 + 0.03 * up - 0.05 * down,
            decisions_mul: 1.0 + 0.02 * up - 0.04 * down,
            flair_mul: 1.0 + 0.03 * up,
            first_touch_mul: 1.0 - 0.04 * down,
            miscontrol_add: 0.06 * nerves,
            rushed_clearance_add: 0.08 * nerves,
            foul_risk_add: 0.04 * nerves,
        }
    }

    /// Share of the nerve-driven execution risks an outfield action carries.
    /// Every player's psychology is seeded now, so these reach every
    /// touch; a keeper's nerves have their own channel. Fitted against the
    /// realism bands.
    const OUTFIELD_SHARE: f32 = 0.5;

    /// Multiplier a player's head applies to any "do I take this on?"
    /// decision — pulling the trigger from distance, running at a
    /// defender, committing to a press.
    ///
    /// Distinct from [`skill_modifiers`](Self::skill_modifiers), which
    /// tilts how well an action is EXECUTED. This tilts whether it is
    /// ATTEMPTED at all, which is the half of football psychology the
    /// state machine had no access to: `PsychState` was only ever read by
    /// the pass evaluator and the ownership duel, so a striker who had
    /// just shanked two chances was exactly as eager to shoot as one who
    /// had scored twice.
    ///
    /// Continuous and deliberately narrow — roughly 0.82..1.14. Real
    /// confidence changes how often a player tries something, not whether
    /// they are capable of it, and outcome quality is already shaped
    /// elsewhere. Returns exactly 1.0 for a neutral head, so a player with
    /// no recorded psychology is unaffected.
    pub fn initiative_multiplier(state: &PsychState) -> f32 {
        // Confidence is the dominant term: it runs -1..+1 and moves on
        // goals, assists, errors and misses.
        let confidence_term = state.confidence * 0.10;
        // Nervousness only ever suppresses — 0..1, and a player gripped by
        // it defers rather than gambles.
        let nerves_term = -state.nervousness * 0.08;
        // Momentum is the short-lived swing right after a goal or an
        // error; smaller weight because it decays fast by design.
        let momentum_term = state.momentum_boost * 0.08;
        (1.0 + confidence_term + nerves_term + momentum_term).clamp(0.80, 1.15)
    }

    /// A keeper's appetite for taking things on, −1..1: his confidence
    /// less his nerves. Zero for a settled keeper.
    pub fn keeper_appetite(state: &PsychState) -> f32 {
        (state.confidence - state.nervousness).clamp(-1.0, 1.0)
    }

    /// `initiative_multiplier` for a player who may have no recorded
    /// psychology yet — neutral (1.0) in that case.
    pub fn initiative_for(psychology: &PsychologyState, player_id: u32) -> f32 {
        psychology
            .get(player_id)
            .map(Self::initiative_multiplier)
            .unwrap_or(1.0)
    }

    /// Confidence delta from a positive event. Caller adds it (clamped).
    pub fn confidence_delta_positive(event: PositiveEvent) -> f32 {
        match event {
            PositiveEvent::Goal => 0.10,
            PositiveEvent::Assist => 0.06,
            PositiveEvent::BigTackle | PositiveEvent::BigSave => 0.04,
        }
    }

    /// Confidence delta from a negative event.
    pub fn confidence_delta_negative(event: NegativeEvent) -> f32 {
        match event {
            NegativeEvent::ErrorLeadingToShot => -0.10,
            NegativeEvent::ErrorLeadingToGoal => -0.20,
            NegativeEvent::YellowCard => -0.04,
            NegativeEvent::GoalConceded => -0.08,
            NegativeEvent::Mistake => -0.06,
        }
    }

    /// Leadership team score (0..1).
    ///
    /// captain_leadership*0.35 + captain_teamwork*0.15 + captain_determination*0.18 +
    /// captain_pressure*0.18 + vice_leadership*0.08 + gk_communication*0.06.
    /// All 0..20 inputs.
    ///
    /// TODO(matchday-leadership): pure + unit-tested, not yet wired into the
    /// live match loop. Intended to be fed the actual matchday captain / vice
    /// (from `MatchdayLeadership`) plus the keeper, then drive
    /// `leadership_damped_momentum`. Deferred until the match-engine wiring can
    /// be calibrated.
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

    /// Damping applied by a captain/vice with high leadership. The higher
    /// the team leadership score, the more the negative momentum is
    /// absorbed (up to 35% per spec).
    ///
    /// TODO(matchday-leadership): pure + unit-tested, not yet wired into the
    /// live match loop. Consumes `team_leadership_score` once that is fed the
    /// real matchday captain; deferred until the momentum path can be
    /// recalibrated.
    pub fn leadership_damped_momentum(raw_momentum: f32, team_leadership_0_1: f32) -> f32 {
        if raw_momentum >= 0.0 {
            return raw_momentum;
        }
        let damp = 1.0 - (team_leadership_0_1.clamp(0.0, 1.0) * 0.35);
        raw_momentum * damp
    }
}

/// Event tag for positive in-match moments. Translated to a confidence delta
/// via [`Psychology::confidence_delta_positive`].
#[derive(Debug, Clone, Copy)]
pub enum PositiveEvent {
    Goal,
    Assist,
    BigTackle,
    BigSave,
}

/// Event tag for negative in-match moments. Translated to a confidence delta
/// via [`Psychology::confidence_delta_negative`].
#[derive(Debug, Clone, Copy)]
pub enum NegativeEvent {
    /// Misplaced pass / miscontrol that led to an opposition shot.
    ErrorLeadingToShot,
    /// Same, but it produced a goal.
    ErrorLeadingToGoal,
    /// Yellow card (penalised more for low-temperament players elsewhere).
    YellowCard,
    /// Goal conceded by a goalkeeper — confidence drop.
    GoalConceded,
    /// Generic personal mistake (own goal, big miscontrol, etc.).
    Mistake,
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

    /// Seed a player's afternoon from the state of mind he brought, once
    /// the standard of the match is known. Nothing under `OF_MIND_OFF`.
    pub fn seed(&mut self, player: &MatchPlayer, standard: f32, occasion: f32) {
        if !MindSwitch::armed() {
            return;
        }
        self.players
            .insert(player.id, Psychology::kickoff_state(player, standard, occasion));
    }

    pub fn record_positive(&mut self, player_id: u32, event: PositiveEvent, tick: u64) {
        let s = self.get_or_default(player_id);
        s.confidence += Psychology::confidence_delta_positive(event);
        s.goal_involvement_tick = Some(tick);
        s.clamp();
    }

    pub fn record_negative(&mut self, player_id: u32, event: NegativeEvent, tick: u64) {
        let s = self.get_or_default(player_id);
        s.confidence += Psychology::confidence_delta_negative(event);
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
    use crate::{PersonAttributes, PlayerAttributes, PlayerPositionType, PlayerSkills};
    use chrono::NaiveDate;

    fn mind(assurance: f32) -> KickoffMind {
        KickoffMind {
            assurance: Some(assurance),
            ..KickoffMind::neutral()
        }
    }

    fn keeper(kickoff_mind: KickoffMind, pressure: f32) -> MatchPlayer {
        let attributes = PersonAttributes {
            pressure,
            important_matches: 10.0,
            ..PersonAttributes::default()
        };
        let mut skills = PlayerSkills::default();
        skills.mental.composure = 10.0;
        MatchPlayer::from_inputs(
            1,
            1,
            [0.0; 3],
            [0.0; 3],
            attributes,
            PlayerAttributes::default(),
            skills,
            PlayerPositionType::Goalkeeper,
            None,
            Vec::new(),
            NaiveDate::from_ymd_opt(2004, 3, 1).unwrap(),
            false,
            10_000,
            0.0,
            1.0,
            1.0,
            kickoff_mind,
            false,
        )
    }

    fn league(importance: f32, crowd: f32) -> f32 {
        Psychology::occasion(
            &MatchEnvironment {
                match_importance: importance,
                crowd_intensity: crowd,
                ..Default::default()
            },
            false,
        )
    }

    #[test]
    fn high_morale_and_belief_lift_initial_confidence() {
        let up = KickoffMind {
            morale: 85.0,
            self_belief: 0.6,
            ..KickoffMind::neutral()
        };
        let down = KickoffMind {
            morale: 20.0,
            self_belief: -0.6,
            ..KickoffMind::neutral()
        };
        let high = Psychology::initial_confidence(&up, 12.0, 0.5);
        let low = Psychology::initial_confidence(&down, 12.0, 0.5);
        assert!(high > low);
        assert!((-0.5..=0.5).contains(&high) && (-0.5..=0.5).contains(&low));
        assert_eq!(Psychology::initial_confidence(&KickoffMind::neutral(), 10.0, 0.0), 0.0);
    }

    #[test]
    fn a_reserve_keepers_first_senior_start_is_nervous() {
        let reserve = keeper(mind(0.55), 10.0);
        let first_team = Psychology::kickoff_state(&reserve, 0.67, league(0.45, 0.5));
        let reserves = Psychology::kickoff_state(&reserve, 0.55, league(0.1, 0.1));
        assert!(first_team.nervousness > reserves.nervousness + 0.3);
    }

    #[test]
    fn a_veteran_at_his_usual_standard_is_calm() {
        let veteran = keeper(mind(0.67), 10.0);
        let state = Psychology::kickoff_state(&veteran, 0.67, league(0.45, 0.5));
        assert!(state.nervousness < 0.05, "{}", state.nervousness);
    }

    #[test]
    fn the_occasion_raises_nerves_on_its_own() {
        let veteran = keeper(mind(0.67), 10.0);
        let ordinary = Psychology::kickoff_state(&veteran, 0.67, league(0.45, 0.5));
        let final_night = Psychology::kickoff_state(
            &veteran,
            0.67,
            Psychology::occasion(
                &MatchEnvironment {
                    match_importance: 0.95,
                    crowd_intensity: 1.0,
                    ..Default::default()
                },
                true,
            ),
        );
        assert!(final_night.nervousness > ordinary.nervousness);
    }

    #[test]
    fn temperament_answers_the_occasion() {
        let big_night = league(0.95, 1.0);
        let cool = Psychology::kickoff_state(&keeper(mind(0.67), 18.0), 0.67, big_night);
        let rattled = Psychology::kickoff_state(&keeper(mind(0.67), 3.0), 0.67, big_night);
        assert!(cool.nervousness < rattled.nervousness);
    }

    #[test]
    fn a_player_with_no_history_is_at_home_at_any_standard() {
        let synthetic = keeper(KickoffMind::neutral(), 10.0);
        let state = Psychology::kickoff_state(&synthetic, 0.84, league(0.45, 0.5));
        assert!(state.nervousness < 0.05);
    }

    #[test]
    fn pressure_load_clamped_unit() {
        let env = MatchEnvironment {
            match_importance: 1.0,
            derby_intensity: 1.0,
            crowd_intensity: 1.0,
            ..Default::default()
        };
        let load = Psychology::pressure_load(&env, 1.0, 1.0, 0.0);
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
        let no_lead = Psychology::pressure_load(&env, 0.5, 0.3, 0.0);
        let strong_lead = Psychology::pressure_load(&env, 0.5, 0.3, 1.0);
        assert!(strong_lead < no_lead);
    }

    #[test]
    fn confidence_above_threshold_boosts_skills() {
        let s = PsychState {
            confidence: 0.6,
            nervousness: 0.0,
            ..Default::default()
        };
        let m = Psychology::skill_modifiers(&s);
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
        let m = Psychology::skill_modifiers(&s);
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
        let m = Psychology::skill_modifiers(&s);
        assert!(m.miscontrol_add > 0.0);
        assert!(m.foul_risk_add > 0.0);
    }

    #[test]
    fn a_small_change_in_confidence_makes_a_small_change() {
        let at = |c: f32, n: f32| {
            Psychology::skill_modifiers(&PsychState {
                confidence: c,
                nervousness: n,
                ..Default::default()
            })
        };
        let mut c = -1.0;
        while c < 1.0 {
            let a = at(c, c.abs() * 0.5);
            let b = at(c + 0.01, (c + 0.01).abs() * 0.5);
            assert!((a.composure_mul - b.composure_mul).abs() < 0.002);
            assert!((a.decisions_mul - b.decisions_mul).abs() < 0.002);
            assert!((a.first_touch_mul - b.first_touch_mul).abs() < 0.002);
            assert!((a.miscontrol_add - b.miscontrol_add).abs() < 0.002);
            c += 0.01;
        }
    }

    #[test]
    fn modifiers_stay_bounded() {
        for (c, n) in [(-1.0, 1.0), (1.0, 1.0), (-5.0, 9.0), (5.0, -3.0)] {
            let m = Psychology::skill_modifiers(&PsychState {
                confidence: c,
                nervousness: n,
                ..Default::default()
            });
            assert!((0.94..=1.04).contains(&m.composure_mul));
            assert!((0.95..=1.03).contains(&m.decisions_mul));
            assert!((0.0..=0.06).contains(&m.miscontrol_add));
        }
    }

    #[test]
    fn an_early_error_colours_the_rest_of_a_keepers_match() {
        let mut p = PsychologyState::default();
        p.seed(&keeper(mind(0.67), 10.0), 0.67, league(0.45, 0.5));
        let settled = p.get(1).unwrap().confidence;
        p.record_negative(1, NegativeEvent::ErrorLeadingToGoal, 60_000);
        let after = p.get(1).unwrap().confidence;
        assert!(after < settled - 0.15);
        // Nothing in the state hands it back as the match goes on.
        p.record_team_event(true, 0.35, 400_000);
        assert_eq!(p.get(1).unwrap().confidence, after);
    }

    #[test]
    fn conceding_costs_a_keeper_less_than_his_own_error() {
        let conceded = Psychology::confidence_delta_negative(NegativeEvent::GoalConceded);
        let own = Psychology::confidence_delta_negative(NegativeEvent::ErrorLeadingToGoal);
        assert!(conceded.abs() < own.abs());
    }

    #[test]
    fn a_big_save_lifts_a_keeper() {
        let mut p = PsychologyState::default();
        p.seed(&keeper(mind(0.67), 10.0), 0.67, league(0.45, 0.5));
        let before = p.get(1).unwrap().confidence;
        p.record_positive(1, PositiveEvent::BigSave, 30_000);
        assert!(p.get(1).unwrap().confidence > before);
    }

    #[test]
    fn captain_leadership_dampens_negative_momentum() {
        let raw = -0.6;
        let no_captain = Psychology::leadership_damped_momentum(raw, 0.0);
        let strong_captain = Psychology::leadership_damped_momentum(raw, 1.0);
        // Negative momentum is absorbed (closer to zero) with strong captain.
        assert!(strong_captain > no_captain);
        assert!(strong_captain < 0.0);
    }

    #[test]
    fn leadership_does_not_cap_positive_momentum() {
        let raw = 0.6;
        assert_eq!(Psychology::leadership_damped_momentum(raw, 1.0), 0.6);
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
        let young = Psychology::keeper_communication_score(14.0, 14.0, 14.0, 14.0, 12.0, 0.1);
        let veteran = Psychology::keeper_communication_score(14.0, 14.0, 14.0, 14.0, 12.0, 1.0);
        assert!(veteran > young);
    }
}
