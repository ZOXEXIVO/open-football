use crate::club::player::traits::PlayerTrait;
use crate::r#match::MatchPlayer;
use crate::r#match::StateProcessingContext;
use crate::r#match::player::strategies::players::ops::effective_skill::{
    ActionContext as EffActionContext, effective_skill,
};
use crate::r#match::player::strategies::players::ops::skill_composites as sc;

// ---------------------------------------------------------------------------
// ShotSkillProfile — unified shooting model
// ---------------------------------------------------------------------------
//
// Single source of truth for every shot path: pre-shot xG, willingness,
// final shot trajectory, on-target probability, miskick odds, and the
// post-match rating's finishing-efficiency input. Before this profile,
// the pre-shot xG, the in-flight xG (event dispatcher), and the shot
// execution all read raw skills slightly differently — letting low-
// skill players inherit elite conversion through compounded clamps and
// linear blends. Centralising the math here means a 5/20 finisher
// behaves consistently poorly in every step.
//
// Inputs are explicit so the profile can be built from contexts that
// don't share the StateProcessingContext (the event dispatcher only has
// a `MatchPlayer` + ball state).

/// Inputs needed to build a `ShotSkillProfile` for a given shot moment.
#[derive(Debug, Clone, Copy)]
pub struct ShotSkillInputs {
    pub distance: f32,
    pub minute: u32,
    /// Player condition as 0..1.
    pub condition_pct: f32,
    pub pressure_count_5u: u32,
    pub pressure_count_10u: u32,
    /// 0..1 — how clean the lane to goal is (from `shot_clarity()`).
    pub shot_clarity: f32,
    pub has_clear_shot: bool,
    /// Distance to GK if a closing keeper is in scope; None otherwise.
    pub gk_distance: Option<f32>,
    pub is_sprinting_or_recent_sprint: bool,
}

/// Unified shooting profile — drives every shot quality decision.
#[derive(Debug, Clone, Copy)]
pub struct ShotSkillProfile {
    pub selection_skill: f32,
    pub execution_skill: f32,
    pub composure_skill: f32,
    pub body_control: f32,
    pub placement_skill: f32,
    pub power_skill: f32,
    pub shot_quality_multiplier: f32,
    pub on_target_skill_multiplier: f32,
    pub random_error_scale: f32,
    pub miskick_probability: f32,
    pub poor_penalty: f32,
    pub elite_lift: f32,
    pub technique_curve: f32,
    pub shooting_condition_mult: f32,
    pub low_condition_penalty: f32,
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

impl ShotSkillProfile {
    /// Build the profile for `player` at the moment described by `inputs`.
    /// All technical / mental / physical skills are routed through
    /// `effective_skill` so fatigue is already applied before the curve.
    pub fn from_player(player: &MatchPlayer, inputs: &ShotSkillInputs) -> Self {
        let tech = EffActionContext::technical(inputs.minute);
        let mental = EffActionContext::mental(inputs.minute);
        let expl = EffActionContext::explosive(inputs.minute);
        let s = &player.skills;

        // Effective skill reads (1..20).
        let finishing_eff = effective_skill(player, s.technical.finishing, tech);
        let technique_eff = effective_skill(player, s.technical.technique, tech);
        let first_touch_eff = effective_skill(player, s.technical.first_touch, tech);
        let long_shots_eff = effective_skill(player, s.technical.long_shots, tech);
        let composure_eff = effective_skill(player, s.mental.composure, mental);
        let decisions_eff = effective_skill(player, s.mental.decisions, mental);
        let concentration_eff = effective_skill(player, s.mental.concentration, mental);
        let anticipation_eff = effective_skill(player, s.mental.anticipation, mental);
        let balance_eff = effective_skill(player, s.physical.balance, tech);
        let agility_eff = effective_skill(player, s.physical.agility, expl);
        let strength_eff = effective_skill(player, s.physical.strength, expl);

        // Normalised skill bands.
        let finishing01 = norm01(finishing_eff);
        let technique01 = norm01(technique_eff);
        let first_touch01 = norm01(first_touch_eff);
        let long_shots01 = norm01(long_shots_eff);
        let composure01 = norm01(composure_eff);
        let decisions01 = norm01(decisions_eff);
        let concentration01 = norm01(concentration_eff);
        let _anticipation01 = norm01(anticipation_eff);
        let balance01 = norm01(balance_eff);
        let agility01 = norm01(agility_eff);
        let strength01 = norm01(strength_eff);

        // Headline penalties / lifts. The "headline" skill for a shooter is
        // finishing — the smoothstep around it controls the heavy-handed
        // poor-finisher penalty applied by every downstream consumer.
        let poor_penalty = smoothstep(0.45, 0.15, finishing01);
        let elite_lift = smoothstep(0.70, 0.95, finishing01);
        let technique_curve = pow_curve(technique01, 1.65);

        // Conditioning: harsher fatigue for low-skill players & late games.
        let cond = inputs.condition_pct.clamp(0.0, 1.0);
        let stamina01 = norm01(s.physical.stamina);
        let nat_fit01 = norm01(s.physical.natural_fitness);
        let fitness = stamina01 * 0.55 + nat_fit01 * 0.45;
        let fatigue_penalty = (1.0 - cond).max(0.0).powf(1.35);
        let fitness_recovery = 1.0 - fatigue_penalty * (0.18 + fitness * 0.22);
        let low_skill_fatigue = 1.0 - fatigue_penalty * poor_penalty * 0.30;
        let late_pressure = if inputs.minute >= 70 {
            1.0 - ((inputs.minute as f32 - 70.0) / 50.0).clamp(0.0, 1.0) * poor_penalty * 0.12
        } else {
            1.0
        };
        let shooting_condition_mult =
            (fitness_recovery * low_skill_fatigue * late_pressure).clamp(0.48, 1.03);
        let low_condition_penalty = (1.0 - shooting_condition_mult).max(0.0).clamp(0.0, 0.55);

        // Per-distance execution composite — finishing-led for close,
        // long-shots/technique-led at distance.
        let execution_skill = if inputs.distance <= 30.0 {
            (pow_curve(finishing01, 1.65) * 0.42
                + pow_curve(composure01, 1.45) * 0.22
                + pow_curve(first_touch01, 1.45) * 0.13
                + pow_curve(technique01, 1.45) * 0.10
                + pow_curve(decisions01, 1.35) * 0.08
                + pow_curve(balance01, 1.25) * 0.05)
                .clamp(0.0, 1.0)
        } else if inputs.distance <= 80.0 {
            (pow_curve(finishing01, 1.65) * 0.30
                + pow_curve(technique01, 1.55) * 0.22
                + pow_curve(long_shots01, 1.65) * 0.18
                + pow_curve(composure01, 1.45) * 0.14
                + pow_curve(decisions01, 1.35) * 0.10
                + pow_curve(balance01, 1.25) * 0.06)
                .clamp(0.0, 1.0)
        } else {
            (pow_curve(long_shots01, 1.75) * 0.38
                + pow_curve(technique01, 1.60) * 0.24
                + pow_curve(composure01, 1.45) * 0.13
                + pow_curve(decisions01, 1.40) * 0.11
                + pow_curve(strength01, 1.25) * 0.07
                + pow_curve(balance01, 1.25) * 0.07)
                .clamp(0.0, 1.0)
        };

        // Selection — should we be shooting at all? Composure +
        // decisions + finishing + long_shots + concentration. Composure
        // and decisions are the dominant signals; finishing matters less
        // than for execution because we're scoring choice, not strike.
        let selection_skill = (pow_curve(composure01, 1.30) * 0.32
            + pow_curve(decisions01, 1.30) * 0.28
            + pow_curve(finishing01, 1.45) * 0.18
            + pow_curve(long_shots01, 1.45) * 0.10
            + pow_curve(concentration01, 1.20) * 0.07
            + pow_curve(technique01, 1.20) * 0.05)
            .clamp(0.0, 1.0);

        let composure_skill = pow_curve(composure01, 1.45);

        // Body control — sprinters with poor balance/agility/first_touch
        // lose body control. After a recent sprint apply ~25% penalty
        // scaled by how poor the underlying balance is.
        let raw_body = (pow_curve(balance01, 1.25) * 0.30
            + pow_curve(agility01, 1.25) * 0.22
            + pow_curve(first_touch01, 1.30) * 0.22
            + pow_curve(composure01, 1.30) * 0.16
            + pow_curve(strength01, 1.10) * 0.10)
            .clamp(0.0, 1.0);
        let sprint_factor = if inputs.is_sprinting_or_recent_sprint {
            (1.0 - (1.0 - raw_body) * 0.35).clamp(0.55, 1.0)
        } else {
            1.0
        };
        let body_control = (raw_body * sprint_factor).clamp(0.0, 1.0);

        // Placement — finishing + decisions + technique drive how well
        // the player can pick a corner.
        let placement_skill = (pow_curve(finishing01, 1.65) * 0.45
            + pow_curve(decisions01, 1.40) * 0.25
            + pow_curve(technique01, 1.40) * 0.20
            + pow_curve(composure01, 1.30) * 0.10)
            .clamp(0.0, 1.0);

        // Power — strength + technique + finishing + long_shots.
        let power_skill = (pow_curve(strength01, 1.15) * 0.32
            + pow_curve(technique01, 1.30) * 0.28
            + pow_curve(finishing01, 1.30) * 0.22
            + pow_curve(long_shots01, 1.40) * 0.18)
            .clamp(0.0, 1.0);

        // Pressure penalty (0..~1) used by xG & error scaling.
        let pressure_penalty = (inputs.pressure_count_5u as f32 * 0.20
            + inputs.pressure_count_10u as f32 * 0.07)
            .clamp(0.0, 1.0);

        // Multipliers consumed downstream. The execution-driven curve
        // shapes how much skill influences shot xG. Earlier 0.35-anchor
        // (with a steep 1.20 exponent) left avg players (exec~0.34)
        // at ~0.64 — a 25u shot only gave them xG 0.18 vs real ~0.20+.
        // 0.50-anchor with a tighter linear shape pulls the avg-tier
        // population xG into the 0.10/shot real-football band while
        // preserving the elite/poor spread.
        let shot_quality_multiplier = (0.50 + execution_skill * 0.85).clamp(0.50, 1.30);
        let on_target_skill_multiplier = (0.55 + execution_skill * 0.85 - poor_penalty * 0.20
            + elite_lift * 0.05)
            .clamp(0.30, 1.10);
        let random_error_scale =
            (1.15 - execution_skill * 0.85 + poor_penalty * 0.15).clamp(0.30, 1.50);

        // Miskick: dominated by poor_penalty + low technique. Pressure /
        // condition push it up further.
        let miskick_probability = (poor_penalty * 0.10
            + (1.0 - technique_curve).max(0.0).powf(2.2) * 0.08
            + inputs.pressure_count_5u as f32 * 0.025
            + low_condition_penalty * 0.05)
            .clamp(0.0, 0.55);

        ShotSkillProfile {
            selection_skill,
            execution_skill,
            composure_skill,
            body_control,
            placement_skill,
            power_skill,
            shot_quality_multiplier,
            on_target_skill_multiplier,
            random_error_scale,
            miskick_probability,
            poor_penalty,
            elite_lift,
            technique_curve,
            shooting_condition_mult,
            low_condition_penalty,
            pressure_penalty,
        }
    }

    /// Pre-shot expected goals using this profile. Mirrors the formula
    /// in `handle_shoot_event` (which builds the same profile in-flight)
    /// so the decision-time xG and stat-time xG agree.
    pub fn expected_xg(&self, distance: f32, has_clear_shot: bool) -> f32 {
        // Distance factor — calibrated against real Opta xG by
        // distance:
        //   6yd  (~12u): 0.55
        //   12yd (~24u): 0.28
        //   18yd (~36u): 0.14
        //   25m  (~50u): 0.07
        //   30m  (~60u): 0.045
        //   35m  (~70u): 0.030
        // After shot_quality_multiplier (0.35 → 1.30 by skill) and
        // shooting_condition_mult (~0.95) the per-shot population xG
        // averages ~0.10, matching real Opta.
        let distance_factor = if distance <= 10.0 {
            0.72
        } else if distance <= 30.0 {
            0.72 - (distance - 10.0) / 20.0 * 0.40
        } else if distance <= 60.0 {
            0.32 - (distance - 30.0) / 30.0 * 0.22
        } else if distance <= 120.0 {
            0.10 - (distance - 60.0) / 60.0 * 0.07
        } else {
            0.025
        };

        let clarity_mult = if has_clear_shot { 1.0 } else { 0.35 };
        let pressure_mult = (1.0 - self.pressure_penalty * 0.85).clamp(0.20, 1.0);
        let mut xg = distance_factor
            * self.shot_quality_multiplier
            * self.shooting_condition_mult
            * pressure_mult
            * clarity_mult;

        // Long-range cap unless the player has elite long shots
        // (encoded via execution_skill above ~0.55 implies long_shots≥16).
        if distance > 100.0 && self.execution_skill < 0.55 {
            xg = xg.min(0.055);
        }
        // Low-skill conversion cap — even on easy chances a 5/20 player
        // can't post elite xG.
        if self.execution_skill < 0.20 {
            xg = xg.min(0.18);
        }
        xg.clamp(0.005, 0.82)
    }
}

/// Operations for shooting decision-making
pub struct ShootingOperationsImpl<'p> {
    ctx: &'p StateProcessingContext<'p>,
}

// Realistic shooting distances (field is typically 840 units)
// Real football: most goals from within 18m (~36 units), rare from 30m+ (~60 units)
const MAX_SHOOTING_DISTANCE: f32 = 100.0; // ~50m - absolute max for elite long shots
const MIN_SHOOTING_DISTANCE: f32 = 1.0;
const VERY_CLOSE_RANGE_DISTANCE: f32 = 28.0; // ~14m - anyone can shoot
const CLOSE_RANGE_DISTANCE: f32 = 48.0; // ~24m - close range shots
const OPTIMAL_SHOOTING_DISTANCE: f32 = 70.0; // ~35m - ideal shooting distance
const MEDIUM_RANGE_DISTANCE: f32 = 80.0; // ~40m - medium range shots

// Shooting decision thresholds
const SHOOT_OVER_PASS_CLOSE_THRESHOLD: f32 = 36.0; // Always prefer shooting if closer than this
const SHOOT_OVER_PASS_MEDIUM_THRESHOLD: f32 = 50.0; // Shoot over pass for decent finishers
const EXCELLENT_OPPORTUNITY_CLOSE_RANGE: f32 = 60.0; // Distance for close-range excellent opportunity

// Teammate advantage thresholds (multipliers)
const TEAMMATE_ADVANTAGE_RATIO: f32 = 0.4; // Teammate must be this much closer to prevent shot

impl<'p> ShootingOperationsImpl<'p> {
    pub fn new(ctx: &'p StateProcessingContext<'p>) -> Self {
        ShootingOperationsImpl { ctx }
    }

    /// Expected-goals estimate for a shot taken right now. Mirrors the
    /// xG formula in `handle_shoot_event` so decisions use the same
    /// quality curve the post-hoc stat does. Returns 0..0.9 on a scale
    /// where 0.55 = penalty-spot chance, 0.08 = 20-yard long shot,
    /// <0.04 = hopeless spray. Used as a pre-shot gate so forwards
    /// don't burn cooldowns on low-quality attempts that real players
    /// would skip in favour of a pass.
    pub fn expected_xg(&self) -> f32 {
        let profile = self.shot_profile();
        let d = self.ctx.ball().distance_to_opponent_goal();
        profile.expected_xg(d, self.ctx.player().has_clear_shot())
    }

    /// Build the unified `ShotSkillProfile` for the current player /
    /// shot context. Used by the pre-shot decision gates and by the
    /// in-flight event dispatcher (which builds the profile from raw
    /// inputs because it has no `StateProcessingContext`).
    pub fn shot_profile(&self) -> ShotSkillProfile {
        let player = self.ctx.player;
        let distance = self.ctx.ball().distance_to_opponent_goal();
        let minute = sc::minute_from_ms(self.ctx.context.total_match_time);
        let condition_pct = (player.player_attributes.condition as f32 / 10_000.0).clamp(0.0, 1.0);

        // Pressure counts (5u and 10u).
        let mut pressure_5u: u32 = 0;
        let mut pressure_10u: u32 = 0;
        for (_id, dist) in self.ctx.tick_context.grid.opponents(player.id, 10.0) {
            if dist <= 5.0 {
                pressure_5u += 1;
            }
            pressure_10u += 1;
        }

        let gk_distance = self
            .ctx
            .players()
            .opponents()
            .goalkeeper()
            .next()
            .map(|gk| (gk.position - player.position).magnitude());

        let is_sprinting_or_recent_sprint = self.ctx.in_state_time as f32 > 30.0;

        let inputs = ShotSkillInputs {
            distance,
            minute,
            condition_pct,
            pressure_count_5u: pressure_5u,
            pressure_count_10u: pressure_10u,
            shot_clarity: self.ctx.player().shot_clarity(),
            has_clear_shot: self.ctx.player().has_clear_shot(),
            gk_distance,
            is_sprinting_or_recent_sprint,
        };

        ShotSkillProfile::from_player(player, &inputs)
    }

    /// Check if player is in shooting range (skill-aware)
    pub fn in_shooting_range(&self) -> bool {
        let distance_to_goal = self.ctx.ball().distance_to_opponent_goal();
        let skills = &self.ctx.player.skills;
        let shooting_skill = skills.technical.finishing / 20.0;
        let long_shot_skill = skills.technical.long_shots / 20.0;

        // Very close range - most players should shoot
        if distance_to_goal <= VERY_CLOSE_RANGE_DISTANCE {
            return shooting_skill >= 0.3; // finishing >= 6
        }

        // Close range shots — need decent finishing ability
        if distance_to_goal <= CLOSE_RANGE_DISTANCE {
            return shooting_skill >= 0.5; // finishing >= 10
        }

        // Medium range shots - requires good finishing
        if distance_to_goal <= OPTIMAL_SHOOTING_DISTANCE {
            return shooting_skill >= 0.6; // finishing >= 12
        }

        // Medium-long range shots — need good long shot ability
        if distance_to_goal <= MEDIUM_RANGE_DISTANCE {
            return long_shot_skill >= 0.65 && shooting_skill >= 0.55;
        }

        // Long range shots — elite players only
        if distance_to_goal <= MAX_SHOOTING_DISTANCE {
            return long_shot_skill >= 0.75 && shooting_skill >= 0.6;
        }

        false
    }

    /// Check for excellent shooting opportunity (clear sight, good distance, no pressure)
    pub fn has_excellent_opportunity(&self) -> bool {
        let distance = self.ctx.ball().distance_to_opponent_goal();
        let clear_shot = self.ctx.player().has_clear_shot();

        // Very close to goal - excellent opportunity if any space
        if distance <= EXCELLENT_OPPORTUNITY_CLOSE_RANGE {
            let low_pressure = !self.ctx.players().opponents().exists(5.0);
            return clear_shot && low_pressure;
        }

        // Medium to optimal range - need good angle too
        if distance > MIN_SHOOTING_DISTANCE && distance <= MEDIUM_RANGE_DISTANCE {
            let low_pressure = !self.ctx.players().opponents().exists(10.0);
            let good_angle = self.has_good_angle();

            return clear_shot && low_pressure && good_angle;
        }

        false
    }

    /// Check shooting angle quality
    pub fn has_good_angle(&self) -> bool {
        let goal_angle = self.ctx.player().goal_angle();
        // Good angle is less than 45 degrees off center
        goal_angle < std::f32::consts::PI / 4.0
    }

    /// Determine if should shoot instead of looking for pass
    pub fn should_shoot_over_pass(&self) -> bool {
        let distance = self.ctx.ball().distance_to_opponent_goal();
        let has_clear_shot = self.ctx.player().has_clear_shot();
        let skills = &self.ctx.player.skills;
        let confidence = skills.mental.composure / 20.0;
        let finishing = skills.technical.finishing / 20.0;
        let long_shots = skills.technical.long_shots / 20.0;
        let teamwork = skills.mental.teamwork / 20.0;

        // Must have clear shot for any shooting decision
        if !has_clear_shot {
            return false;
        }

        // Signature moves (PPMs): two hard-override traits that reshape the
        // whole decision tree. Only apply in realistic ranges so a 100m
        // "shoots from distance" shot still gets filtered out.
        let player = self.ctx.player;
        let prefers_shot = player.has_trait(PlayerTrait::ShootsFromDistance);
        let prefers_pass = player.has_trait(PlayerTrait::LooksForPassRatherThanAttemptShot);

        // Single scan: count opponents within 8 units (reused below)
        let opponents_within_8 = self
            .ctx
            .tick_context
            .grid
            .opponents(self.ctx.player.id, 8.0)
            .count();

        // Check if heavily marked — prefer pass if 2+ opponents very close
        // (a pass-first trait makes players even less willing to shoot here)
        let heavy_marking_threshold = if prefers_pass { 1 } else { 2 };
        if opponents_within_8 >= heavy_marking_threshold && distance > VERY_CLOSE_RANGE_DISTANCE {
            return false;
        }

        // Very close range - almost always shoot (even pass-first players)
        if distance <= VERY_CLOSE_RANGE_DISTANCE {
            return true;
        }

        // Pass-first players need an extra-clean opportunity before shooting
        // anywhere outside the box.
        let finishing_close_threshold = if prefers_pass { 0.55 } else { 0.4 };
        let finishing_medium_threshold = if prefers_pass { 0.65 } else { 0.5 };

        // Close range - shoot if any finishing ability
        if distance <= SHOOT_OVER_PASS_CLOSE_THRESHOLD && finishing > finishing_close_threshold {
            return true;
        }

        // Check if teammates are in MUCH better positions first
        let opponent_goal_pos = self.ctx.player().opponent_goal_position();
        let better_positioned_teammate = self.ctx.players().teammates().nearby(100.0).any(|t| {
            let t_dist = (t.position - opponent_goal_pos).magnitude();
            t_dist < distance * TEAMMATE_ADVANTAGE_RATIO
        });

        // High teamwork players defer to better-positioned teammates.
        // "Looks for pass" reinforces this; "Shoots from distance" ignores it.
        if better_positioned_teammate && !prefers_shot {
            let deference_threshold = if prefers_pass { 0.45 } else { 0.6 };
            if teamwork > deference_threshold {
                return false;
            }
        }

        // Medium range - shoot if decent skills
        if distance <= SHOOT_OVER_PASS_MEDIUM_THRESHOLD && finishing > finishing_medium_threshold {
            return true;
        }

        // Optimal distance with reasonable ability
        if distance <= OPTIMAL_SHOOTING_DISTANCE && (confidence + finishing) / 2.0 > 0.55 {
            return true;
        }

        // Medium-long range with good long shot skills and no heavy pressure.
        // "Shoots from distance" players lower the long-shot bar significantly
        // and accept a bit more pressure — this is where the PPM most changes
        // match feel (Robben, Lampard, Steven Gerrard-style hits).
        if distance <= MEDIUM_RANGE_DISTANCE
            && ((prefers_shot && long_shots > 0.35 && finishing > 0.35 && opponents_within_8 <= 1)
                || (long_shots > 0.5 && finishing > 0.45 && opponents_within_8 == 0))
        {
            return true;
        }

        // "Shoots from distance" opens the door for genuine long-range attempts
        // in the 80-100 unit bracket if the player has real ability.
        if prefers_shot
            && distance <= MAX_SHOOTING_DISTANCE
            && long_shots > 0.6
            && opponents_within_8 == 0
        {
            return true;
        }

        false
    }

    /// Check if in close range for finishing
    pub fn in_close_range(&self) -> bool {
        let distance = self.ctx.ball().distance_to_opponent_goal();
        distance >= MIN_SHOOTING_DISTANCE && distance <= CLOSE_RANGE_DISTANCE
    }

    /// Check if in optimal shooting distance
    pub fn in_optimal_range(&self) -> bool {
        let distance = self.ctx.ball().distance_to_opponent_goal();
        distance >= MIN_SHOOTING_DISTANCE && distance <= OPTIMAL_SHOOTING_DISTANCE
    }

    /// Get shooting confidence factor (0.0 - 1.0).
    /// Routes the per-distance shooting composite (close / medium / long)
    /// through the same fatigue + composure curve that drives `expected_xg`
    /// and `ShotQualityEvaluator::skill_factor`.
    pub fn shooting_confidence(&self) -> f32 {
        let distance = self.ctx.ball().distance_to_opponent_goal();
        let minute = sc::minute_from_ms(self.ctx.context.total_match_time);
        let skill_factor = if distance > 100.0 {
            sc::long_shot(self.ctx.player, minute)
        } else if distance > 30.0 {
            sc::shooting_medium(self.ctx.player, minute)
        } else {
            sc::shooting_close(self.ctx.player, minute)
        };

        let distance_factor = self.distance_factor();
        let pressure_factor = self.pressure_factor();

        let base = (skill_factor * distance_factor * pressure_factor).clamp(0.0, 1.0);

        // Trait-flavoured final adjustments
        let player = self.ctx.player;
        let distance = self.ctx.ball().distance_to_opponent_goal();
        let mut adjusted = base;
        if player.has_trait(PlayerTrait::PlacesShots) && distance <= OPTIMAL_SHOOTING_DISTANCE {
            adjusted += 0.05;
        }
        if player.has_trait(PlayerTrait::PowersShots) {
            adjusted += 0.03;
        }
        if player.has_trait(PlayerTrait::ShootsFromDistance) && distance > OPTIMAL_SHOOTING_DISTANCE
        {
            adjusted += 0.08;
        }
        adjusted.clamp(0.0, 1.0)
    }

    /// Get distance factor for shooting confidence (1.0 = optimal, 0.0 = too far/close)
    fn distance_factor(&self) -> f32 {
        let distance = self.ctx.ball().distance_to_opponent_goal();

        if distance < MIN_SHOOTING_DISTANCE {
            return 0.3; // Too close, awkward angle
        }

        if distance <= OPTIMAL_SHOOTING_DISTANCE {
            // Optimal range - linear increase to peak
            return (distance / OPTIMAL_SHOOTING_DISTANCE).min(1.0);
        }

        if distance <= MAX_SHOOTING_DISTANCE {
            // Beyond optimal - linear decrease
            let beyond_optimal = distance - OPTIMAL_SHOOTING_DISTANCE;
            let range = MAX_SHOOTING_DISTANCE - OPTIMAL_SHOOTING_DISTANCE;
            return 1.0 - (beyond_optimal / range);
        }

        0.0 // Too far
    }

    /// Get pressure factor for shooting confidence (1.0 = no pressure, 0.0 = extreme pressure)
    fn pressure_factor(&self) -> f32 {
        // Single scan at max distance, bucket by distance
        let mut close_opponents = 0;
        let mut medium_opponents = 0;
        for (_id, dist) in self
            .ctx
            .tick_context
            .grid
            .opponents(self.ctx.player.id, 10.0)
        {
            if dist <= 5.0 {
                close_opponents += 1;
            }
            medium_opponents += 1;
        }

        if close_opponents >= 2 {
            return 0.3;
        } else if close_opponents == 1 {
            return 0.6;
        } else if medium_opponents >= 2 {
            return 0.8;
        }

        1.0
    }
}
