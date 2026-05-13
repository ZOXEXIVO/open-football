use crate::r#match::MatchPlayer;
use crate::r#match::StateProcessingContext;
use crate::r#match::player::strategies::players::ops::effective_skill::{
    ActionContext as EffActionContext, effective_skill,
};
use crate::r#match::player::strategies::players::ops::skill_composites as sc;

// ---------------------------------------------------------------------------
// MidfielderSkillProfile — unified skill model for midfielders.
// ---------------------------------------------------------------------------
//
// Single source of truth for the dozen midfielder decision sites that used
// to each branch on raw `vision > 0.4`, `passing > 11`, `long_shots > 0.55`
// thresholds. The profile reads every effective skill once, applies the
// same nonlinear curve and conditioning model, and exports a handful of
// 0..1 selection / execution scores that downstream state code consumes
// directly. Mirrors the role `ShotSkillProfile` plays for shooting.

#[derive(Debug, Clone, Copy)]
pub struct MidfielderSkillInputs {
    pub minute: u32,
    /// Player condition as 0..1.
    pub condition_pct: f32,
    pub pressure_count_5u: u32,
    pub pressure_count_10u: u32,
    /// Distance from the carrier to the opponent goal (engine units).
    pub distance_to_opponent_goal: f32,
    /// Distance from the carrier to their own goal (engine units).
    pub distance_to_own_goal: f32,
    /// Ball ownership duration in ticks.
    pub ownership_ticks: u64,
    pub recent_sprint_or_high_intensity: bool,
}

/// Continuous selection / execution profile for midfielders. All values
/// are in 0..1 unless noted.
#[derive(Debug, Clone, Copy)]
pub struct MidfielderSkillProfile {
    // Headline mappers
    pub poor_penalty: f32,
    pub elite_lift: f32,

    // Conditioning multipliers (0.52..1.03)
    pub mid_condition_mult: f32,
    pub passing_condition_mult: f32,
    pub first_touch_condition_mult: f32,
    pub pressing_condition_mult: f32,
    pub tackling_condition_mult: f32,
    pub off_ball_condition_mult: f32,

    // Pass-game outputs
    pub pass_execution: f32,
    pub progressive_selection: f32,
    pub long_pass_profile: f32,
    pub long_pass_risk_tolerance: f32,
    pub press_resistance: f32,

    // Carry / dribble outputs
    pub carry_selection: f32,

    // Distance shooting decision (midfielder-specific)
    pub mid_shot_selection: f32,

    // Defensive outputs
    pub pressing_profile: f32,
    pub tackle_profile: f32,
    pub discipline: f32,

    // Off-ball support
    pub support_profile: f32,

    // Pressure scaling (0..1ish)
    pub pressure_penalty: f32,
}

#[inline]
fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    if (edge0 - edge1).abs() < f32::EPSILON {
        return if x < edge0 { 0.0 } else { 1.0 };
    }
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[inline]
fn norm01(v: f32) -> f32 {
    (v / 20.0).clamp(0.0, 1.0)
}

#[inline]
fn pow_curve(skill01: f32, exp: f32) -> f32 {
    skill01.clamp(0.0, 1.0).powf(exp)
}

impl MidfielderSkillProfile {
    /// Build the profile from a state processing context.
    pub fn from_ctx(ctx: &StateProcessingContext) -> Self {
        let player = ctx.player;
        let minute = sc::minute_from_ms(ctx.context.total_match_time);
        let condition_pct = (player.player_attributes.condition as f32 / 10_000.0).clamp(0.0, 1.0);

        let mut pressure_5u: u32 = 0;
        let mut pressure_10u: u32 = 0;
        for (_id, dist) in ctx.tick_context.grid.opponents(player.id, 10.0) {
            if dist <= 5.0 {
                pressure_5u += 1;
            }
            pressure_10u += 1;
        }

        let inputs = MidfielderSkillInputs {
            minute,
            condition_pct,
            pressure_count_5u: pressure_5u,
            pressure_count_10u: pressure_10u,
            distance_to_opponent_goal: ctx.ball().distance_to_opponent_goal(),
            distance_to_own_goal: ctx.ball().distance_to_own_goal(),
            ownership_ticks: ctx.tick_context.ball.ownership_duration as u64,
            recent_sprint_or_high_intensity: ctx.in_state_time as f32 > 30.0,
        };
        Self::from_player(player, &inputs)
    }

    pub fn from_player(player: &MatchPlayer, inputs: &MidfielderSkillInputs) -> Self {
        let tech = EffActionContext::technical(inputs.minute);
        let mental = EffActionContext::mental(inputs.minute);
        let expl = EffActionContext::explosive(inputs.minute);
        let s = &player.skills;

        // ── Effective skill reads ────────────────────────────────────
        let passing_eff = effective_skill(player, s.technical.passing, tech);
        let technique_eff = effective_skill(player, s.technical.technique, tech);
        let first_touch_eff = effective_skill(player, s.technical.first_touch, tech);
        let dribbling_eff = effective_skill(player, s.technical.dribbling, tech);
        let long_shots_eff = effective_skill(player, s.technical.long_shots, tech);
        let finishing_eff = effective_skill(player, s.technical.finishing, tech);
        let tackling_eff = effective_skill(player, s.technical.tackling, tech);

        let vision_eff = effective_skill(player, s.mental.vision, mental);
        let decisions_eff = effective_skill(player, s.mental.decisions, mental);
        let composure_eff = effective_skill(player, s.mental.composure, mental);
        let concentration_eff = effective_skill(player, s.mental.concentration, mental);
        let anticipation_eff = effective_skill(player, s.mental.anticipation, mental);
        let teamwork_eff = effective_skill(player, s.mental.teamwork, mental);
        let off_ball_eff = effective_skill(player, s.mental.off_the_ball, mental);
        let positioning_eff = effective_skill(player, s.mental.positioning, mental);
        let work_rate_eff = effective_skill(player, s.mental.work_rate, mental);
        let bravery_eff = effective_skill(player, s.mental.bravery, mental);
        let aggression_eff = effective_skill(player, s.mental.aggression, mental);
        let flair_eff = effective_skill(player, s.mental.flair, mental);

        let stamina_eff = effective_skill(player, s.physical.stamina, expl);
        let acceleration_eff = effective_skill(player, s.physical.acceleration, expl);
        let agility_eff = effective_skill(player, s.physical.agility, expl);
        let balance_eff = effective_skill(player, s.physical.balance, tech);
        let strength_eff = effective_skill(player, s.physical.strength, expl);

        // ── Normalised reads ─────────────────────────────────────────
        let passing01 = norm01(passing_eff);
        let technique01 = norm01(technique_eff);
        let first_touch01 = norm01(first_touch_eff);
        let dribbling01 = norm01(dribbling_eff);
        let long_shots01 = norm01(long_shots_eff);
        let finishing01 = norm01(finishing_eff);
        let tackling01 = norm01(tackling_eff);

        let vision01 = norm01(vision_eff);
        let decisions01 = norm01(decisions_eff);
        let composure01 = norm01(composure_eff);
        let concentration01 = norm01(concentration_eff);
        let anticipation01 = norm01(anticipation_eff);
        let teamwork01 = norm01(teamwork_eff);
        let off_ball01 = norm01(off_ball_eff);
        let positioning01 = norm01(positioning_eff);
        let work_rate01 = norm01(work_rate_eff);
        let bravery01 = norm01(bravery_eff);
        let aggression01 = norm01(aggression_eff);
        let flair01 = norm01(flair_eff);

        let stamina01 = norm01(stamina_eff);
        let acceleration01 = norm01(acceleration_eff);
        let agility01 = norm01(agility_eff);
        let balance01 = norm01(balance_eff);
        let strength01 = norm01(strength_eff);

        // ── Headline mappers ─────────────────────────────────────────
        // Use a blended "midfielder skill" centred on passing/decisions/
        // vision so the poor_penalty / elite_lift represent the core
        // midfielder package, not a single attribute.
        let core01 = (passing01 * 0.30
            + decisions01 * 0.25
            + vision01 * 0.20
            + composure01 * 0.15
            + technique01 * 0.10)
            .clamp(0.0, 1.0);
        let poor_penalty = smoothstep(0.45, 0.18, core01);
        let elite_lift = smoothstep(0.72, 0.95, core01);

        // ── Conditioning model ───────────────────────────────────────
        let cond = inputs.condition_pct.clamp(0.0, 1.0);
        let nat_fit01 = norm01(s.physical.natural_fitness);
        let match_readiness01 = norm01(s.physical.match_readiness);
        let fitness = stamina01 * 0.45 + nat_fit01 * 0.35 + match_readiness01 * 0.20;
        let fatigue = (1.0 - cond).max(0.0).powf(1.25);
        let jadedness = (player.player_attributes.jadedness as f32 / 10_000.0).clamp(0.0, 1.0);
        let jadedness_penalty = jadedness * 0.18;
        let fitness_recovery = 1.0 - fatigue * (0.16 + fitness * 0.20);
        let mental_fatigue = 1.0 - fatigue * (0.10 + poor_penalty * 0.18);
        let late_drop = if inputs.minute >= 65 {
            1.0 - ((inputs.minute as f32 - 65.0) / 55.0).clamp(0.0, 1.0)
                * (0.05 + poor_penalty * 0.12)
        } else {
            1.0
        };
        let mid_condition_mult =
            (fitness_recovery * mental_fatigue * late_drop - jadedness_penalty).clamp(0.52, 1.03);

        let stamina_curve = pow_curve(stamina01, 1.20);
        let composure_curve = pow_curve(composure01, 1.30);
        let balance_curve = pow_curve(balance01, 1.20);
        let anticipation_curve = pow_curve(anticipation01, 1.20);

        let passing_condition_mult = mid_condition_mult;
        let first_touch_condition_mult =
            mid_condition_mult * (0.92 + balance_curve * 0.08).clamp(0.85, 1.02);
        let pressing_condition_mult =
            mid_condition_mult * (0.82 + stamina_curve * 0.18).clamp(0.65, 1.05);
        let tackling_condition_mult = mid_condition_mult
            * (0.84 + composure_curve * 0.08 + balance_curve * 0.08).clamp(0.65, 1.03);
        let off_ball_condition_mult = mid_condition_mult
            * (0.80 + stamina_curve * 0.12 + anticipation_curve * 0.08).clamp(0.62, 1.05);

        // ── Pressure penalty ─────────────────────────────────────────
        let pressure_penalty = (inputs.pressure_count_5u as f32 * 0.20
            + inputs.pressure_count_10u as f32 * 0.07)
            .clamp(0.0, 1.0);

        // ── Pass execution ───────────────────────────────────────────
        let pass_execution = ((pow_curve(passing01, 1.45) * 0.30
            + pow_curve(technique01, 1.35) * 0.18
            + pow_curve(vision01, 1.45) * 0.16
            + pow_curve(decisions01, 1.40) * 0.14
            + pow_curve(composure01, 1.35) * 0.10
            + pow_curve(first_touch01, 1.25) * 0.06
            + pow_curve(concentration01, 1.20) * 0.06)
            * passing_condition_mult)
            .clamp(0.0, 1.0);

        // ── Progressive pass selection ───────────────────────────────
        let progressive_selection = (pow_curve(vision01, 1.55) * 0.28
            + pow_curve(decisions01, 1.45) * 0.24
            + pow_curve(passing01, 1.45) * 0.18
            + pow_curve(technique01, 1.30) * 0.10
            + pow_curve(composure01, 1.35) * 0.10
            + pow_curve(teamwork01, 1.20) * 0.06
            + pow_curve(flair01, 1.25) * 0.04)
            .clamp(0.0, 1.0);

        // ── Long pass / switch ───────────────────────────────────────
        let long_pass_profile = (pow_curve(passing01, 1.45) * 0.24
            + pow_curve(vision01, 1.55) * 0.24
            + pow_curve(technique01, 1.45) * 0.18
            + pow_curve(decisions01, 1.35) * 0.12
            + pow_curve(composure01, 1.25) * 0.08
            + pow_curve(strength01, 1.10) * 0.06
            + pow_curve(balance01, 1.15) * 0.04
            + pow_curve(concentration01, 1.15) * 0.04)
            .clamp(0.0, 1.0);

        let long_pass_risk_tolerance =
            (0.15 + long_pass_profile * 0.55 - pressure_penalty * 0.20).clamp(0.10, 0.78);

        // ── Press resistance ─────────────────────────────────────────
        let press_resistance = ((pow_curve(first_touch01, 1.40) * 0.22
            + pow_curve(technique01, 1.35) * 0.18
            + pow_curve(composure01, 1.45) * 0.18
            + pow_curve(balance01, 1.30) * 0.14
            + pow_curve(agility01, 1.25) * 0.10
            + pow_curve(decisions01, 1.35) * 0.10
            + pow_curve(strength01, 1.10) * 0.04
            + pow_curve(concentration01, 1.15) * 0.04)
            * first_touch_condition_mult)
            .clamp(0.0, 1.0);

        // ── Carry / dribble selection ────────────────────────────────
        let carry_selection = (pow_curve(dribbling01, 1.55) * 0.24
            + pow_curve(decisions01, 1.40) * 0.18
            + pow_curve(composure01, 1.35) * 0.14
            + pow_curve(acceleration01, 1.25) * 0.12
            + pow_curve(agility01, 1.25) * 0.10
            + pow_curve(balance01, 1.25) * 0.10
            + pow_curve(technique01, 1.30) * 0.08
            + pow_curve(bravery01, 1.10) * 0.04)
            .clamp(0.0, 1.0);

        // ── Midfielder shot selection ────────────────────────────────
        let mid_shot_selection = (pow_curve(long_shots01, 1.75) * 0.28
            + pow_curve(technique01, 1.50) * 0.18
            + pow_curve(decisions01, 1.45) * 0.18
            + pow_curve(composure01, 1.45) * 0.16
            + pow_curve(finishing01, 1.55) * 0.10
            + pow_curve(balance01, 1.25) * 0.06
            + pow_curve(concentration01, 1.20) * 0.04)
            .clamp(0.0, 1.0);

        // ── Pressing ─────────────────────────────────────────────────
        let pressing_profile = ((pow_curve(work_rate01, 1.35) * 0.22
            + pow_curve(stamina01, 1.25) * 0.18
            + pow_curve(anticipation01, 1.35) * 0.16
            + pow_curve(acceleration01, 1.25) * 0.12
            + pow_curve(positioning01, 1.30) * 0.10
            + pow_curve(decisions01, 1.30) * 0.10
            + pow_curve(aggression01, 1.10) * 0.06
            + pow_curve(teamwork01, 1.15) * 0.06)
            * pressing_condition_mult)
            .clamp(0.0, 1.0);

        // ── Tackling ─────────────────────────────────────────────────
        let tackle_profile = ((pow_curve(tackling01, 1.55) * 0.28
            + pow_curve(positioning01, 1.35) * 0.16
            + pow_curve(anticipation01, 1.35) * 0.14
            + pow_curve(composure01, 1.35) * 0.12
            + pow_curve(balance01, 1.25) * 0.10
            + pow_curve(agility01, 1.20) * 0.08
            + pow_curve(strength01, 1.15) * 0.06
            + pow_curve(concentration01, 1.20) * 0.06)
            * tackling_condition_mult)
            .clamp(0.0, 1.0);

        // ── Discipline ───────────────────────────────────────────────
        // No `temperament` attribute on the player — fold it into a
        // composure/concentration blend so the discipline curve still
        // covers the full intended weight.
        let aggression_inverse = 1.0 - aggression01;
        let temperament_proxy = (composure01 + concentration01) * 0.5;
        let discipline = (pow_curve(composure01, 1.35) * 0.26
            + pow_curve(decisions01, 1.35) * 0.22
            + pow_curve(tackling01, 1.30) * 0.16
            + pow_curve(concentration01, 1.20) * 0.14
            + temperament_proxy * 0.12
            + aggression_inverse * 0.10)
            .clamp(0.0, 1.0);

        // ── Off-ball support ─────────────────────────────────────────
        let support_profile = ((pow_curve(off_ball01, 1.45) * 0.22
            + pow_curve(anticipation01, 1.35) * 0.18
            + pow_curve(decisions01, 1.35) * 0.18
            + pow_curve(teamwork01, 1.25) * 0.12
            + pow_curve(stamina01, 1.20) * 0.10
            + pow_curve(acceleration01, 1.20) * 0.08
            + pow_curve(composure01, 1.20) * 0.06
            + pow_curve(concentration01, 1.15) * 0.06)
            * off_ball_condition_mult)
            .clamp(0.0, 1.0);

        let _ = (
            inputs.distance_to_opponent_goal,
            inputs.distance_to_own_goal,
            inputs.ownership_ticks,
            inputs.recent_sprint_or_high_intensity,
        );

        MidfielderSkillProfile {
            poor_penalty,
            elite_lift,
            mid_condition_mult,
            passing_condition_mult,
            first_touch_condition_mult,
            pressing_condition_mult,
            tackling_condition_mult,
            off_ball_condition_mult,
            pass_execution,
            progressive_selection,
            long_pass_profile,
            long_pass_risk_tolerance,
            press_resistance,
            carry_selection,
            mid_shot_selection,
            pressing_profile,
            tackle_profile,
            discipline,
            support_profile,
            pressure_penalty,
        }
    }

    /// True if the midfielder should attempt line-breaking through balls.
    #[inline]
    pub fn allows_through_ball(&self) -> bool {
        self.progressive_selection >= 0.42
    }

    /// True if the midfielder should attempt killer / final-third balls.
    #[inline]
    pub fn allows_killer_ball(&self) -> bool {
        self.progressive_selection >= 0.60
    }

    /// True if the midfielder should attempt switches of play.
    #[inline]
    pub fn allows_switch_play(&self) -> bool {
        self.long_pass_profile >= 0.44
    }

    /// True if the midfielder should carry into space.
    #[inline]
    pub fn allows_carry_into_space(&self) -> bool {
        self.carry_selection >= 0.38 && self.mid_condition_mult >= 0.72
    }

    /// True if the midfielder should attempt to take on a single defender.
    #[inline]
    pub fn allows_take_on_one(&self) -> bool {
        self.carry_selection >= 0.48
    }

    /// True if the midfielder should attempt to take on two defenders.
    #[inline]
    pub fn allows_take_on_two(&self) -> bool {
        self.carry_selection >= 0.66
    }

    /// True if the midfielder should engage in counterpress chasing.
    #[inline]
    pub fn allows_counterpress(&self) -> bool {
        self.pressing_profile >= 0.34
    }

    /// True if the midfielder should sustain a high press.
    #[inline]
    pub fn allows_high_press(&self) -> bool {
        self.pressing_profile >= 0.46 && self.mid_condition_mult >= 0.68
    }

    /// True if the midfielder should make a late-box run into the area.
    #[inline]
    pub fn allows_late_box_run(&self) -> bool {
        self.support_profile >= 0.52 && self.mid_condition_mult >= 0.70
    }

    /// Press time budget in ticks for this profile.
    #[inline]
    pub fn press_time_ticks(&self, press_intensity: f32) -> u64 {
        (35.0 + press_intensity * 55.0 + self.pressing_profile * 45.0).max(20.0) as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PlayerSkills;
    use crate::club::player::builder::PlayerBuilder;
    use crate::shared::fullname::FullName;
    use crate::{
        PersonAttributes, PlayerAttributes, PlayerPosition, PlayerPositionType, PlayerPositions,
    };
    use chrono::NaiveDate;

    fn build_player(fill: f32, condition: i16) -> MatchPlayer {
        let mut attrs = PlayerAttributes::default();
        attrs.condition = condition;
        attrs.jadedness = 0;
        let mut skills = PlayerSkills::default();
        // Fill all skills uniformly so the profile reads a single band.
        let s = &mut skills;
        s.technical.passing = fill;
        s.technical.technique = fill;
        s.technical.first_touch = fill;
        s.technical.dribbling = fill;
        s.technical.long_shots = fill;
        s.technical.finishing = fill;
        s.technical.tackling = fill;
        s.technical.marking = fill;
        s.technical.crossing = fill;
        s.mental.vision = fill;
        s.mental.decisions = fill;
        s.mental.composure = fill;
        s.mental.concentration = fill;
        s.mental.anticipation = fill;
        s.mental.teamwork = fill;
        s.mental.off_the_ball = fill;
        s.mental.positioning = fill;
        s.mental.work_rate = fill;
        s.mental.bravery = fill;
        s.mental.aggression = fill;
        s.mental.flair = fill;
        s.mental.determination = fill;
        s.mental.leadership = fill;
        s.physical.stamina = fill;
        s.physical.natural_fitness = fill;
        s.physical.match_readiness = fill;
        s.physical.acceleration = fill;
        s.physical.pace = fill;
        s.physical.agility = fill;
        s.physical.balance = fill;
        s.physical.strength = fill;
        s.physical.jumping = fill;
        let player = PlayerBuilder::new()
            .id(1)
            .full_name(FullName::new("M".to_string(), "P".to_string()))
            .birth_date(NaiveDate::from_ymd_opt(2000, 1, 1).unwrap())
            .country_id(1)
            .attributes(PersonAttributes::default())
            .skills(skills)
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: PlayerPositionType::MidfielderCenter,
                    level: 18,
                }],
            })
            .player_attributes(attrs)
            .build()
            .unwrap();
        MatchPlayer::from_player(1, &player, PlayerPositionType::MidfielderCenter, false)
    }

    fn default_inputs() -> MidfielderSkillInputs {
        MidfielderSkillInputs {
            minute: 30,
            condition_pct: 0.95,
            pressure_count_5u: 0,
            pressure_count_10u: 0,
            distance_to_opponent_goal: 200.0,
            distance_to_own_goal: 400.0,
            ownership_ticks: 20,
            recent_sprint_or_high_intensity: false,
        }
    }

    #[test]
    fn poor_player_has_low_pass_execution() {
        let p = build_player(5.0, 9000);
        let prof = MidfielderSkillProfile::from_player(&p, &default_inputs());
        assert!(prof.pass_execution < 0.30);
        assert!(prof.poor_penalty > 0.5);
    }

    #[test]
    fn elite_player_has_high_pass_execution() {
        let p = build_player(18.0, 9000);
        let prof = MidfielderSkillProfile::from_player(&p, &default_inputs());
        assert!(prof.pass_execution > 0.65);
        assert!(prof.elite_lift > 0.0);
    }

    #[test]
    fn elite_unlocks_progressive_play() {
        let elite = build_player(17.0, 9000);
        let poor = build_player(5.0, 9000);
        let pe = MidfielderSkillProfile::from_player(&elite, &default_inputs());
        let pp = MidfielderSkillProfile::from_player(&poor, &default_inputs());
        assert!(pe.allows_through_ball());
        assert!(!pp.allows_through_ball());
        assert!(pe.allows_switch_play());
    }

    #[test]
    fn poor_skill_blocks_take_ons() {
        let poor = build_player(5.0, 9000);
        let prof = MidfielderSkillProfile::from_player(&poor, &default_inputs());
        assert!(!prof.allows_take_on_one());
        assert!(!prof.allows_take_on_two());
    }

    #[test]
    fn elite_unlocks_take_ons() {
        let elite = build_player(18.0, 9000);
        let prof = MidfielderSkillProfile::from_player(&elite, &default_inputs());
        assert!(prof.allows_take_on_one());
    }

    #[test]
    fn pressure_lowers_long_pass_risk_tolerance() {
        let p = build_player(15.0, 9000);
        let mut clean = default_inputs();
        clean.pressure_count_5u = 0;
        clean.pressure_count_10u = 0;
        let mut crowded = default_inputs();
        crowded.pressure_count_5u = 2;
        crowded.pressure_count_10u = 4;
        let cp = MidfielderSkillProfile::from_player(&p, &clean);
        let pp = MidfielderSkillProfile::from_player(&p, &crowded);
        assert!(cp.long_pass_risk_tolerance > pp.long_pass_risk_tolerance);
    }

    #[test]
    fn fatigue_reduces_pressing() {
        let fresh_skills = build_player(15.0, 9500);
        let tired_skills = build_player(15.0, 2500);
        let fresh = MidfielderSkillProfile::from_player(
            &fresh_skills,
            &MidfielderSkillInputs {
                minute: 80,
                condition_pct: 0.95,
                ..default_inputs()
            },
        );
        let tired = MidfielderSkillProfile::from_player(
            &tired_skills,
            &MidfielderSkillInputs {
                minute: 80,
                condition_pct: 0.25,
                ..default_inputs()
            },
        );
        assert!(fresh.pressing_profile > tired.pressing_profile);
        assert!(fresh.mid_condition_mult > tired.mid_condition_mult);
    }

    #[test]
    fn discipline_higher_for_composed_player() {
        let mut composed = build_player(12.0, 9000);
        composed.skills.mental.composure = 18.0;
        composed.skills.mental.decisions = 17.0;
        composed.skills.mental.aggression = 6.0;
        let mut reckless = build_player(12.0, 9000);
        reckless.skills.mental.composure = 7.0;
        reckless.skills.mental.decisions = 7.0;
        reckless.skills.mental.aggression = 18.0;
        let cd = MidfielderSkillProfile::from_player(&composed, &default_inputs()).discipline;
        let rd = MidfielderSkillProfile::from_player(&reckless, &default_inputs()).discipline;
        assert!(cd > rd + 0.10, "composed={cd} reckless={rd}");
    }
}
