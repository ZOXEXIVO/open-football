use crate::r#match::MatchPlayer;
use crate::r#match::StateProcessingContext;
use crate::r#match::player::strategies::players::ops::effective_skill::{
    ActionContext as EffActionContext, effective_skill,
};
use crate::r#match::player::strategies::players::ops::skill_composites as sc;

// ---------------------------------------------------------------------------
// GoalkeeperSkillProfile — unified skill model for goalkeepers.
// ---------------------------------------------------------------------------
//
// Single source of truth for the goalkeeper decision sites that used to
// each branch on raw `reflexes / 20.0`, `handling / 20.0`, fixed dive
// distances, or generous catch probability floors. Mirrors
// `DefenderSkillProfile` for outfield defenders and provides a non-linear
// skill curve so a 5/20 keeper sharply degrades vs a 15/20 one.
//
// The profile is consumed by the goalkeeping states (Diving, Jumping,
// Catching, PreparingForSave, ComingOut, Punching, Distributing,
// Passing, Kicking, Throwing) so that local raw formulas can be replaced
// by reads from one fatigue- and skill-aware source.

#[derive(Debug, Clone, Copy)]
pub struct GoalkeeperSkillInputs {
    pub minute: u32,
    pub condition_pct: f32,
}

/// Continuous selection / execution profile for goalkeepers. All values
/// are in 0..1 unless noted.
#[derive(Debug, Clone, Copy)]
pub struct GoalkeeperSkillProfile {
    /// Composite shot-stopping ability: reflexes/handling/agility blend.
    pub shot_stopping: f32,
    /// Goal-line / cross-cover positioning quality.
    pub positioning: f32,
    /// Explosive lateral reach in saves.
    pub dive_reach: f32,
    /// Hand-to-ball reliability for clean catches.
    pub handling_profile: f32,
    /// Ability to direct parries away from danger.
    pub parry_control: f32,
    /// High-ball / cross command.
    pub aerial_command: f32,
    /// Sweeper / coming-out execution.
    pub rushing_out_profile: f32,
    /// Close-range one-on-one save quality.
    pub one_v_one: f32,
    /// Distribution composite (kicks/throws/passes).
    pub distribution: f32,
    /// Communication / organisation read.
    pub communication: f32,
    /// Mental concentration component.
    pub concentration: f32,

    /// Conditioning multipliers (clamped 0.45..1.03).
    pub condition_mult: f32,
    /// Explosive-action multiplier (dives, bursts, leaps).
    pub explosive_mult: f32,
    /// Handling reliability multiplier (catches, parries).
    pub handling_mult: f32,
    /// Decision multiplier (coming-out, claim, distribution).
    pub decision_mult: f32,

    /// Smoothstep penalty applied to weak keepers (0..1, large = bad).
    pub poor_skill_penalty: f32,
    /// Small bonus for elite keepers.
    pub elite_lift: f32,

    /// Effective lateral reach in game units used by the save model.
    pub effective_dive_distance: f32,
    /// Effective catch radius in game units.
    pub effective_catch_distance: f32,
    /// Effective punch radius in game units.
    pub effective_punch_distance: f32,
}

#[inline]
fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    if (edge0 - edge1).abs() < f32::EPSILON {
        return if x < edge0 { 0.0 } else { 1.0 };
    }
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Map a raw 1..20 skill to 0..1 with a small dead zone at 1 so weak
/// keepers come out closer to 0.
#[inline]
fn skill01(value: f32) -> f32 {
    ((value - 1.0) / 19.0).clamp(0.0, 1.0)
}

/// Generic skill curve for goalkeeper attributes (slightly concave so
/// the 5..15 band spreads out non-linearly).
#[inline]
fn keeper_curve(x: f32) -> f32 {
    x.clamp(0.0, 1.0).powf(1.55)
}

/// Reaction-style curve — reflexes / one-on-ones use this to make the
/// difference between a quick and a slow keeper feel sharp.
#[inline]
fn reaction_curve(x: f32) -> f32 {
    x.clamp(0.0, 1.0).powf(1.65)
}

/// Handling curve — slightly steeper than `keeper_curve` so a 5/20
/// handler spills more often than a 5/20 jumper.
#[inline]
fn handling_curve(x: f32) -> f32 {
    x.clamp(0.0, 1.0).powf(1.60)
}

impl GoalkeeperSkillProfile {
    pub fn from_ctx(ctx: &StateProcessingContext) -> Self {
        let player = ctx.player;
        let minute = sc::minute_from_ms(ctx.context.total_match_time);
        let condition_pct = (player.player_attributes.condition as f32 / 10_000.0).clamp(0.0, 1.0);
        Self::from_player(
            player,
            &GoalkeeperSkillInputs {
                minute,
                condition_pct,
            },
        )
    }

    pub fn from_player(player: &MatchPlayer, inputs: &GoalkeeperSkillInputs) -> Self {
        let tech = EffActionContext::technical(inputs.minute);
        let mental = EffActionContext::mental(inputs.minute);
        let expl = EffActionContext::explosive(inputs.minute);
        let s = &player.skills;

        // ── Goalkeeping (technical-feel) reads ────────────────────────
        let reflexes_eff = effective_skill(player, s.goalkeeping.reflexes, tech);
        let handling_eff = effective_skill(player, s.goalkeeping.handling, tech);
        let one_on_ones_eff = effective_skill(player, s.goalkeeping.one_on_ones, tech);
        let aerial_reach_eff = effective_skill(player, s.goalkeeping.aerial_reach, tech);
        let punching_eff = effective_skill(player, s.goalkeeping.punching, tech);
        let kicking_eff = effective_skill(player, s.goalkeeping.kicking, tech);
        let throwing_eff = effective_skill(player, s.goalkeeping.throwing, tech);
        let gk_passing_eff = effective_skill(player, s.goalkeeping.passing, tech);
        let gk_first_touch_eff = effective_skill(player, s.goalkeeping.first_touch, tech);
        let rushing_out_eff = effective_skill(player, s.goalkeeping.rushing_out, tech);
        let command_of_area_eff = effective_skill(player, s.goalkeeping.command_of_area, mental);
        let communication_eff = effective_skill(player, s.goalkeeping.communication, mental);

        // ── Mental reads ─────────────────────────────────────────────
        let positioning_eff = effective_skill(player, s.mental.positioning, mental);
        let anticipation_eff = effective_skill(player, s.mental.anticipation, mental);
        let concentration_eff = effective_skill(player, s.mental.concentration, mental);
        let decisions_eff = effective_skill(player, s.mental.decisions, mental);
        let composure_eff = effective_skill(player, s.mental.composure, mental);
        let bravery_eff = effective_skill(player, s.mental.bravery, mental);
        let teamwork_eff = effective_skill(player, s.mental.teamwork, mental);
        let vision_eff = effective_skill(player, s.mental.vision, mental);
        let determination01 = skill01(effective_skill(player, s.mental.determination, mental));

        // ── Physical reads ───────────────────────────────────────────
        let agility_eff = effective_skill(player, s.physical.agility, expl);
        let acceleration_eff = effective_skill(player, s.physical.acceleration, expl);
        let pace_eff = effective_skill(player, s.physical.pace, expl);
        let jumping_eff = effective_skill(player, s.physical.jumping, expl);
        let strength_eff = effective_skill(player, s.physical.strength, expl);
        let balance_eff = effective_skill(player, s.physical.balance, tech);
        let stamina_eff = effective_skill(player, s.physical.stamina, expl);
        let nat_fitness01 = skill01(s.physical.natural_fitness);
        let match_readiness01 = skill01(s.physical.match_readiness);

        // ── Normalised reads (0..1 via skill01) ──────────────────────
        let reflexes01 = skill01(reflexes_eff);
        let handling01 = skill01(handling_eff);
        let one_on_ones01 = skill01(one_on_ones_eff);
        let aerial_reach01 = skill01(aerial_reach_eff);
        let punching01 = skill01(punching_eff);
        let kicking01 = skill01(kicking_eff);
        let throwing01 = skill01(throwing_eff);
        let gk_passing01 = skill01(gk_passing_eff);
        let gk_first_touch01 = skill01(gk_first_touch_eff);
        let rushing_out_raw01 = skill01(rushing_out_eff);
        let command_of_area01 = skill01(command_of_area_eff);
        let communication01 = skill01(communication_eff);

        let positioning01 = skill01(positioning_eff);
        let anticipation01 = skill01(anticipation_eff);
        let concentration01 = skill01(concentration_eff);
        let decisions01 = skill01(decisions_eff);
        let composure01 = skill01(composure_eff);
        let bravery01 = skill01(bravery_eff);
        let teamwork01 = skill01(teamwork_eff);
        let vision01 = skill01(vision_eff);

        let agility01 = skill01(agility_eff);
        let acceleration01 = skill01(acceleration_eff);
        let pace01 = skill01(pace_eff);
        let jumping01 = skill01(jumping_eff);
        let strength01 = skill01(strength_eff);
        let balance01 = skill01(balance_eff);
        let stamina01 = skill01(stamina_eff);

        // ── Headline penalty / lift (drives weak/elite differentiation) ─
        let primary = (reaction_curve(reflexes01) * 0.30
            + handling_curve(handling01) * 0.20
            + reaction_curve(one_on_ones01) * 0.15
            + keeper_curve(positioning01) * 0.12
            + keeper_curve(anticipation01) * 0.10
            + keeper_curve(concentration01) * 0.08
            + keeper_curve(command_of_area01) * 0.05)
            .clamp(0.0, 1.0);
        let poor_skill_penalty = smoothstep(0.45, 0.18, primary);
        let elite_lift = smoothstep(0.72, 0.95, primary) * 0.08;

        // ── Conditioning ─────────────────────────────────────────────
        let condition = inputs.condition_pct.clamp(0.0, 1.0);
        let fatigue = (1.0 - condition).max(0.0).powf(1.25);
        let jaded = (player.player_attributes.jadedness as f32 / 10_000.0).clamp(0.0, 1.0);

        let fitness_base = stamina01 * 0.30
            + nat_fitness01 * 0.25
            + match_readiness01 * 0.20
            + concentration01 * 0.15
            + determination01 * 0.10;

        let condition_mult = (1.03 - fatigue * (0.34 + (1.0 - fitness_base) * 0.24) - jaded * 0.18)
            .clamp(0.52, 1.03);

        let explosive_mult = (condition_mult - fatigue * 0.10).clamp(0.45, 1.02);
        let handling_mult = (condition_mult - fatigue * 0.07 - jaded * 0.06).clamp(0.50, 1.02);
        let decision_mult = (condition_mult - fatigue * 0.08 - jaded * 0.10).clamp(0.48, 1.02);

        // ── Composite profiles ───────────────────────────────────────
        let shot_stopping_raw = reaction_curve(reflexes01) * 0.28
            + reaction_curve(one_on_ones01) * 0.18
            + keeper_curve(agility01) * 0.16
            + keeper_curve(anticipation01) * 0.10
            + keeper_curve(positioning01) * 0.10
            + handling_curve(handling01) * 0.08
            + keeper_curve(concentration01) * 0.06
            + keeper_curve(composure01) * 0.04;
        let shot_stopping = (shot_stopping_raw * condition_mult - poor_skill_penalty * 0.10
            + elite_lift)
            .clamp(0.0, 1.0);

        let positioning_profile = (keeper_curve(positioning01) * 0.22
            + keeper_curve(anticipation01) * 0.18
            + keeper_curve(decisions01) * 0.16
            + keeper_curve(concentration01) * 0.14
            + keeper_curve(command_of_area01) * 0.10
            + keeper_curve(communication01) * 0.08
            + keeper_curve(teamwork01) * 0.07
            + keeper_curve(composure01) * 0.05)
            .clamp(0.0, 1.0);

        let dive_reach_raw = keeper_curve(agility01) * 0.24
            + keeper_curve(acceleration01) * 0.20
            + keeper_curve(jumping01) * 0.18
            + reaction_curve(reflexes01) * 0.16
            + keeper_curve(balance01) * 0.12
            + keeper_curve(strength01) * 0.10;
        let dive_reach = (dive_reach_raw * explosive_mult).clamp(0.0, 1.0);

        let handling_raw = handling_curve(handling01) * 0.34
            + reaction_curve(reflexes01) * 0.18
            + keeper_curve(concentration01) * 0.14
            + keeper_curve(composure01) * 0.12
            + keeper_curve(agility01) * 0.10
            + keeper_curve(command_of_area01) * 0.07
            + keeper_curve(balance01) * 0.05;
        let handling_profile = (handling_raw * handling_mult).clamp(0.0, 1.0);

        let parry_control = (handling_curve(handling01) * 0.24
            + keeper_curve(punching01) * 0.18
            + keeper_curve(strength01) * 0.16
            + reaction_curve(reflexes01) * 0.14
            + keeper_curve(decisions01) * 0.12
            + keeper_curve(composure01) * 0.10
            + keeper_curve(balance01) * 0.06)
            .clamp(0.0, 1.0);

        let aerial_command_raw = keeper_curve(aerial_reach01) * 0.22
            + keeper_curve(jumping01) * 0.18
            + handling_curve(handling01) * 0.15
            + keeper_curve(command_of_area01) * 0.15
            + keeper_curve(bravery01) * 0.10
            + keeper_curve(strength01) * 0.08
            + keeper_curve(anticipation01) * 0.07
            + keeper_curve(communication01) * 0.05;
        let aerial_command = (aerial_command_raw * explosive_mult).clamp(0.0, 1.0);

        let rushing_out_raw = keeper_curve(rushing_out_raw01) * 0.24
            + keeper_curve(acceleration01) * 0.18
            + keeper_curve(pace01) * 0.12
            + keeper_curve(decisions01) * 0.14
            + keeper_curve(anticipation01) * 0.12
            + keeper_curve(composure01) * 0.08
            + keeper_curve(one_on_ones01) * 0.08
            + keeper_curve(bravery01) * 0.04;
        let rushing_out_profile = (rushing_out_raw * explosive_mult).clamp(0.0, 1.0);

        let one_v_one = (reaction_curve(one_on_ones01) * 0.26
            + reaction_curve(reflexes01) * 0.18
            + keeper_curve(composure01) * 0.16
            + keeper_curve(agility01) * 0.14
            + keeper_curve(decisions01) * 0.12
            + keeper_curve(bravery01) * 0.08
            + keeper_curve(positioning01) * 0.06)
            .clamp(0.0, 1.0);

        let distribution_raw = keeper_curve(kicking01) * 0.22
            + keeper_curve(throwing01) * 0.18
            + keeper_curve(gk_passing01) * 0.16
            + keeper_curve(gk_first_touch01) * 0.10
            + keeper_curve(decisions01) * 0.14
            + keeper_curve(vision01) * 0.08
            + keeper_curve(composure01) * 0.08
            + keeper_curve(teamwork01) * 0.04;
        let distribution = (distribution_raw * decision_mult).clamp(0.0, 1.0);

        let communication = (keeper_curve(communication01) * 0.45
            + keeper_curve(command_of_area01) * 0.30
            + keeper_curve(concentration01) * 0.15
            + keeper_curve(positioning01) * 0.10)
            .clamp(0.0, 1.0);

        let concentration = keeper_curve(concentration01).clamp(0.0, 1.0);

        // Effective ranges in game units. The legacy code used fixed
        // 35..40u catch / dive distances regardless of skill; here, weak
        // keepers shrink to ~14..16u while elite ones extend to ~42..48u.
        let effective_dive_distance = 14.0 + dive_reach * 28.0 + positioning_profile * 6.0;
        let effective_catch_distance = 10.0 + handling_profile * 16.0 + positioning_profile * 5.0;
        let effective_punch_distance = 8.0 + aerial_command * 14.0 + parry_control * 6.0;

        GoalkeeperSkillProfile {
            shot_stopping,
            positioning: positioning_profile,
            dive_reach,
            handling_profile,
            parry_control,
            aerial_command,
            rushing_out_profile,
            one_v_one,
            distribution,
            communication,
            concentration,
            condition_mult,
            explosive_mult,
            handling_mult,
            decision_mult,
            poor_skill_penalty,
            elite_lift,
            effective_dive_distance,
            effective_catch_distance,
            effective_punch_distance,
        }
    }

    /// Per-shot save probability against an estimated shot difficulty in
    /// 0..1 space (combines power, placement, lateral error, reaction
    /// time, screen, deflection, keeper offline). Saves clamp into a
    /// realistic 0.02..0.88 band.
    pub fn save_probability(&self, shot_difficulty: f32) -> f32 {
        let shot_difficulty = shot_difficulty.clamp(0.0, 1.0);
        let keeper_save_power = self.shot_stopping * 0.42
            + self.positioning * 0.20
            + self.dive_reach * 0.18
            + self.concentration * 0.08
            + self.one_v_one * 0.08
            + self.elite_lift;
        let raw = sigmoid((keeper_save_power - shot_difficulty) * 3.20);
        let mut save_prob = raw.clamp(0.03, 0.86);
        save_prob -= self.poor_skill_penalty * 0.14;
        save_prob *= self.condition_mult;
        save_prob.clamp(0.02, 0.88)
    }

    /// Convert a per-shot save probability into a per-tick probability,
    /// preserving cumulative outcome over `expected_save_ticks`.
    pub fn per_tick_save(&self, save_prob: f32, expected_save_ticks: f32) -> f32 {
        let save_prob = save_prob.clamp(0.0, 1.0);
        let n = expected_save_ticks.max(1.0);
        1.0 - (1.0 - save_prob).powf(1.0 / n)
    }

    /// Probability of a clean catch given a catch_difficulty in 0..1.
    pub fn catch_probability(&self, catch_difficulty: f32) -> f32 {
        let catch_difficulty = catch_difficulty.clamp(0.0, 1.0);
        sigmoid((self.handling_profile - catch_difficulty) * 3.00).clamp(0.02, 0.78)
    }

    /// Probability of a safely-directed parry given a catch_difficulty.
    pub fn parry_safe_probability(&self, catch_difficulty: f32) -> f32 {
        let catch_difficulty = catch_difficulty.clamp(0.0, 1.0);
        sigmoid((self.parry_control - catch_difficulty * 0.85) * 2.70).clamp(0.06, 0.70)
    }

    /// Probability the parry / spill leaves a dangerous central rebound.
    pub fn dangerous_rebound_probability(&self, pressure_factor: f32) -> f32 {
        (0.36 - self.parry_control * 0.24
            + pressure_factor.clamp(0.0, 1.0) * 0.12
            + self.poor_skill_penalty * 0.12)
            .clamp(0.08, 0.52)
    }

    /// Whether the keeper should commit to a dive given threat / window.
    pub fn should_dive(
        &self,
        shot_threat: f32,
        ball_distance: f32,
        reaction_window: f32,
        required_reaction_window: f32,
        shot_difficulty: f32,
    ) -> bool {
        shot_threat > 0.35
            && self.effective_dive_distance >= ball_distance
            && reaction_window >= required_reaction_window
            && (self.shot_stopping - shot_difficulty) > -0.28
    }

    /// Whether the keeper prefers to catch (vs. parry/punch) under given
    /// crowd pressure.
    pub fn should_catch(&self, catch_prob: f32, pressure_factor: f32) -> bool {
        catch_prob > 0.42
            && self.handling_profile > self.aerial_command * 0.5 + 0.08
            && pressure_factor < 0.65
    }

    /// Whether the keeper should punch instead of catch.
    pub fn should_punch(&self, catch_prob: f32, crowd_factor: f32, shot_power_factor: f32) -> bool {
        self.aerial_command > 0.30
            && (catch_prob < 0.44 || crowd_factor > 0.55 || shot_power_factor > 0.65)
    }

    /// Distribution turnover risk in 0..1 given how hard the pass is and
    /// how much pressure the keeper is under.
    pub fn turnover_risk(
        &self,
        pass_difficulty: f32,
        pressure_factor: f32,
        weak_foot_or_angle: f32,
    ) -> f32 {
        (pass_difficulty.clamp(0.0, 1.0) * 0.34
            + pressure_factor.clamp(0.0, 1.0) * 0.24
            + weak_foot_or_angle.clamp(0.0, 1.0) * 0.10
            + self.poor_skill_penalty * 0.20
            + (1.0 - self.condition_mult) * 0.12
            - self.distribution * 0.36)
            .clamp(0.02, 0.85)
    }
}

#[inline]
fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
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

    fn build_keeper(fill: f32, condition: i16) -> MatchPlayer {
        let mut attrs = PlayerAttributes::default();
        attrs.condition = condition;
        attrs.jadedness = 0;
        let mut skills = PlayerSkills::default();
        // Goalkeeping
        skills.goalkeeping.aerial_reach = fill;
        skills.goalkeeping.command_of_area = fill;
        skills.goalkeeping.communication = fill;
        skills.goalkeeping.eccentricity = fill;
        skills.goalkeeping.first_touch = fill;
        skills.goalkeeping.handling = fill;
        skills.goalkeeping.kicking = fill;
        skills.goalkeeping.one_on_ones = fill;
        skills.goalkeeping.passing = fill;
        skills.goalkeeping.punching = fill;
        skills.goalkeeping.reflexes = fill;
        skills.goalkeeping.rushing_out = fill;
        skills.goalkeeping.throwing = fill;
        // Mental
        skills.mental.positioning = fill;
        skills.mental.anticipation = fill;
        skills.mental.concentration = fill;
        skills.mental.decisions = fill;
        skills.mental.composure = fill;
        skills.mental.bravery = fill;
        skills.mental.teamwork = fill;
        skills.mental.vision = fill;
        skills.mental.determination = fill;
        skills.mental.work_rate = fill;
        skills.mental.leadership = fill;
        skills.mental.off_the_ball = fill;
        skills.mental.flair = fill;
        skills.mental.aggression = fill;
        // Physical
        skills.physical.acceleration = fill;
        skills.physical.agility = fill;
        skills.physical.balance = fill;
        skills.physical.jumping = fill;
        skills.physical.pace = fill;
        skills.physical.stamina = fill;
        skills.physical.strength = fill;
        skills.physical.natural_fitness = fill;
        skills.physical.match_readiness = fill;
        let player = PlayerBuilder::new()
            .id(1)
            .full_name(FullName::new("G".to_string(), "K".to_string()))
            .birth_date(NaiveDate::from_ymd_opt(2000, 1, 1).unwrap())
            .country_id(1)
            .attributes(PersonAttributes::default())
            .skills(skills)
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: PlayerPositionType::Goalkeeper,
                    level: 18,
                }],
            })
            .player_attributes(attrs)
            .build()
            .unwrap();
        MatchPlayer::from_player(1, &player, PlayerPositionType::Goalkeeper, false)
    }

    fn default_inputs() -> GoalkeeperSkillInputs {
        GoalkeeperSkillInputs {
            minute: 30,
            condition_pct: 0.95,
        }
    }

    #[test]
    fn weak_keeper_has_low_profiles_and_high_penalty() {
        let p = build_keeper(5.0, 9000);
        let prof = GoalkeeperSkillProfile::from_player(&p, &default_inputs());
        assert!(prof.shot_stopping < 0.30);
        assert!(prof.handling_profile < 0.30);
        assert!(prof.dive_reach < 0.35);
        assert!(prof.poor_skill_penalty > 0.5);
        assert!(prof.effective_dive_distance < 28.0);
    }

    #[test]
    fn elite_keeper_has_high_profiles_and_lift() {
        let p = build_keeper(18.0, 9000);
        let prof = GoalkeeperSkillProfile::from_player(&p, &default_inputs());
        assert!(prof.shot_stopping > 0.55);
        assert!(prof.handling_profile > 0.55);
        assert!(prof.dive_reach > 0.55);
        assert!(prof.elite_lift > 0.0);
        assert!(prof.effective_dive_distance > 36.0);
    }

    #[test]
    fn weak_keeper_concedes_high_quality_shots() {
        let weak = GoalkeeperSkillProfile::from_player(&build_keeper(5.0, 9000), &default_inputs());
        let elite =
            GoalkeeperSkillProfile::from_player(&build_keeper(17.0, 9000), &default_inputs());
        // High shot difficulty (well-placed, powerful shot).
        let weak_save = weak.save_probability(0.85);
        let elite_save = elite.save_probability(0.85);
        assert!(weak_save < 0.20);
        assert!(elite_save > weak_save + 0.15);
    }

    #[test]
    fn weak_keeper_creates_more_dangerous_rebounds() {
        let weak = GoalkeeperSkillProfile::from_player(&build_keeper(5.0, 9000), &default_inputs());
        let elite =
            GoalkeeperSkillProfile::from_player(&build_keeper(17.0, 9000), &default_inputs());
        let pressure = 0.4;
        assert!(
            weak.dangerous_rebound_probability(pressure)
                > elite.dangerous_rebound_probability(pressure) + 0.10
        );
    }

    #[test]
    fn fatigue_drops_explosive_more_than_handling() {
        let fresh = GoalkeeperSkillProfile::from_player(
            &build_keeper(15.0, 9500),
            &GoalkeeperSkillInputs {
                minute: 80,
                condition_pct: 0.95,
            },
        );
        let tired = GoalkeeperSkillProfile::from_player(
            &build_keeper(15.0, 2500),
            &GoalkeeperSkillInputs {
                minute: 80,
                condition_pct: 0.25,
            },
        );
        assert!(fresh.explosive_mult > tired.explosive_mult);
        assert!(fresh.dive_reach > tired.dive_reach);
        assert!(fresh.condition_mult > tired.condition_mult);
    }

    #[test]
    fn distribution_skill_lowers_turnover_risk() {
        let weak = GoalkeeperSkillProfile::from_player(&build_keeper(5.0, 9000), &default_inputs());
        let elite =
            GoalkeeperSkillProfile::from_player(&build_keeper(17.0, 9000), &default_inputs());
        assert!(weak.turnover_risk(0.4, 0.4, 0.2) > elite.turnover_risk(0.4, 0.4, 0.2) + 0.10);
    }
}
