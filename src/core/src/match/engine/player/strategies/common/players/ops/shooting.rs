use crate::club::player::traits::PlayerTrait;
use crate::r#match::MatchPlayer;
use crate::r#match::StateProcessingContext;
use crate::r#match::engine::set_pieces::PENALTY_EXECUTION_REFERENCE;
use crate::r#match::engine::teamplay::standard::MatchStandard;
use crate::r#match::player::strategies::players::ops::effective_skill::{
    ActionContext as EffActionContext, effective_skill,
};
use crate::r#match::player::strategies::players::ops::skill_composites as sc;
use crate::r#match::player::strategies::players::ops::xg::ShotType;

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
    /// Set piece this strike is, if any. `None` for open play.
    ///
    /// A penalty and a direct free kick are not shots-from-a-distance:
    /// they are their own actions with their own attributes and their
    /// own conversion bands. Without this the profile scored both with
    /// the open-play finishing curve, which is why `penalty_taking` and
    /// `free_kicks` could pick a taker and then have no effect at all on
    /// whether he scored.
    pub set_piece: Option<ShotType>,
    /// How far the standard of football in this match sits from the
    /// division these curves were fitted in — see
    /// [`crate::r#match::engine::teamplay::standard::MatchStandard`].
    ///
    /// Subtracted from every skill read below, and it is the other half
    /// of the goalkeeper's own centring. The two axes have to move
    /// together or the goal total tilts: measured over
    /// `dev_match levels 300 4 20 2`, shots on target ran **26.4% at
    /// level 4 to 42.5% at level 20** against a real ~33% flat, and saves
    /// per shot on target ran 56.6% to 70.7% against a real ~68% flat.
    /// The two errors were cancelling in the scoreline while each was
    /// wrong on its own, and flattening either alone walks the goals off
    /// their target.
    pub standard_shift: f32,
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
    /// Carried through from [`ShotSkillInputs::set_piece`] so
    /// [`Self::expected_xg`] can apply the dead-ball conversion band.
    pub set_piece: Option<ShotType>,
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

        // Normalised skill bands, each measured against the standard of
        // football in this match — see `ShotSkillInputs::standard_shift`.
        // The curves below are convex, `poor_penalty` and `elite_lift`
        // are smoothsteps on absolute pivots (0.45/0.15 and 0.70/0.95),
        // and `POPULATION_EXECUTION` compares the result against one
        // division's measured mean, so read absolutely the whole striking
        // model prices the league rather than the striker.
        let peer = |v: f32| (norm01(v) - inputs.standard_shift).clamp(0.0, 1.0);
        let finishing01 = peer(finishing_eff);
        let technique01 = peer(technique_eff);
        let first_touch01 = peer(first_touch_eff);
        let long_shots01 = peer(long_shots_eff);
        let composure01 = peer(composure_eff);
        let decisions01 = peer(decisions_eff);
        let concentration01 = peer(concentration_eff);
        let _anticipation01 = peer(anticipation_eff);
        let balance01 = peer(balance_eff);
        let agility01 = peer(agility_eff);
        let strength01 = peer(strength_eff);

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
        // long-shots/technique-led at distance. A dead ball overrides the
        // distance bands entirely below: it is struck from a standing
        // start against a set defence, so distance is not what decides it.
        let open_play_execution = if inputs.distance <= 30.0 {
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

        // Dead-ball override. `penalty_execution` is deliberately linear
        // (a penalty is a technically trivial kick, so the skill response
        // is close to linear and composure carries a quarter of it);
        // `dead_ball_strike` is curved like the open-play composites,
        // because bending one over a wall genuinely is a specialist
        // action a mediocre striker of the ball simply cannot perform.
        let execution_skill = match inputs.set_piece {
            Some(ShotType::Penalty) => sc::penalty_execution(player, inputs.minute),
            Some(ShotType::DirectFreeKick) => sc::dead_ball_strike(player, inputs.minute),
            _ => open_play_execution,
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
        // the player can pick a corner. On a dead ball, picking the
        // corner IS the skill being tested and it is the specialist
        // attribute that names it, so the same override applies.
        let placement_skill = match inputs.set_piece {
            Some(ShotType::Penalty) | Some(ShotType::DirectFreeKick) => execution_skill,
            _ => (pow_curve(finishing01, 1.65) * 0.45
                + pow_curve(decisions01, 1.40) * 0.25
                + pow_curve(technique01, 1.40) * 0.20
                + pow_curve(composure01, 1.30) * 0.10)
                .clamp(0.0, 1.0),
        };

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
        // Anchor lifted 0.55 → 0.62: population accuracy measured 30.7%
        // of shots on target against a real ~33%, which held conversion
        // at 10.0% vs real 11% and left strikers on 0.34 goals/app
        // against the 0.44 the FM season fixtures are built around —
        // i.e. the forward RATING shortfall was a goals shortfall, not a
        // rating-model problem.
        let on_target_skill_multiplier = (0.80 + execution_skill * 0.85 - poor_penalty * 0.20
            + elite_lift * 0.05)
            .clamp(0.42, 1.30);
        // Aim error trimmed alongside the on-target anchor: measured
        // accuracy was 26.8% of shots on target vs a real ~33%, which
        // capped conversion at 8.8% (real 11%) and left strikers short
        // of the 0.44 goals/app the FM season fixtures assume.
        let random_error_scale =
            (0.98 - execution_skill * 0.85 + poor_penalty * 0.15).clamp(0.26, 1.30);

        // Miskick: dominated by poor_penalty + low technique. Pressure /
        // condition push it up further. Exponent 2.2 → 1.6 because the
        // earlier curve made weak-technique miskicks balloon disproportionately
        // (a technique_curve of 0.20 produced (0.80)^2.2 * 0.08 = 4.5% just
        // from technique, stacking with poor_penalty / pressure / condition
        // to push the realistic-weak shot population over 30% miskick rate).
        // 1.6 keeps the soft-curve shape but lets weak players still strike
        // cleanly often enough to convert tap-ins.
        let miskick_probability = (poor_penalty * 0.10
            + (1.0 - technique_curve).max(0.0).powf(1.6) * 0.08
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
            set_piece: inputs.set_piece,
        }
    }

    /// Pre-shot expected goals using this profile. Mirrors the formula
    /// in `handle_shoot_event` (which builds the same profile in-flight)
    /// so the decision-time xG and stat-time xG agree.
    pub fn expected_xg(&self, distance: f32, has_clear_shot: bool) -> f32 {
        // Dead balls short-circuit the geometry entirely: from the spot
        // the kicker's only opponent is the keeper, and a direct free
        // kick has to beat a wall as well as him. Both have their own
        // well-established real-world conversion bands and neither is
        // described by "an unpressured shot from this distance".
        match self.set_piece {
            // Real penalties convert 70-82%. The band is centred on the
            // population of *selected* takers so wiring `penalty_taking`
            // in redistributes conversion rather than shifting it.
            Some(ShotType::Penalty) => {
                let scale = (0.975 + (self.execution_skill - PENALTY_EXECUTION_REFERENCE) * 0.50)
                    .clamp(0.85, 1.10);
                return (0.76 * scale * self.shooting_condition_mult).clamp(0.55, 0.88);
            }
            // Real direct free kicks convert ~6-8% overall, and a
            // specialist from 20m is worth several times a defender
            // from the same spot. 60u ≈ 7.5m, 200u = 25m.
            Some(ShotType::DirectFreeKick) => {
                let dist_score =
                    (1.0 - (distance.clamp(60.0, 200.0) - 60.0) / 140.0).clamp(0.0, 1.0);
                return (0.03 + dist_score * 0.05 + self.execution_skill * 0.04).clamp(0.03, 0.12);
            }
            _ => {}
        }

        // Distance factor — calibrated against real Opta xG at the
        // TRUE field scale: 840u = 105m → 1u = 0.125m (goal 58u=7.32m).
        // The previous breakpoints (10/30/60/120u) were written as if
        // 1u ≈ 0.5m — a 4× compression that flattened the curve to
        // 0.025 for everything beyond 15m real, forced every shooting
        // range constant in the state machine down to ≤14m, and made
        // long-range shooting (38-40% of real shots) impossible.
        // Anchors (before the quality/condition/pressure multipliers;
        // an average profile multiplies by ~0.77, elite ~1.25):
        //   tap-in  ≤2.5m  (20u): 0.72
        //   6yd      5.5m  (44u): 0.55
        //   pen spot  11m  (88u): 0.34
        //   box edge 16.5m (132u): 0.15
        //   25m           (200u): 0.05
        //   30m+          (240u+): 0.03
        let distance_factor = if distance <= 20.0 {
            0.72
        } else if distance <= 44.0 {
            0.72 - (distance - 20.0) / 24.0 * 0.17
        } else if distance <= 88.0 {
            0.55 - (distance - 44.0) / 44.0 * 0.21
        } else if distance <= 132.0 {
            0.34 - (distance - 88.0) / 44.0 * 0.19
        } else if distance <= 200.0 {
            0.15 - (distance - 132.0) / 68.0 * 0.10
        } else if distance <= 280.0 {
            0.05 - (distance - 200.0) / 80.0 * 0.02
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
        // 180u = 22.5m — beyond that only a genuine long-shot specialist
        // carries meaningful xG.
        if distance > 180.0 && self.execution_skill < 0.55 {
            xg = xg.min(0.055);
        }
        // Low-skill conversion cap — even on easy chances a 5/20 player
        // can't post elite xG. Tightened from `< 0.20 → cap 0.18` because
        // that crushed tap-in conversion for whole-team lvl-6 outfields
        // (audit_engine_gap lvl 6 vs lvl 18: 0.16 goals/match, real ~0.5).
        // 0.30 still keeps a weak striker well below an elite's 0.55+ xG
        // at penalty distance while permitting the occasional dogged-shock
        // goal that real football preserves (~9% upset at gap-9+ vs the
        // engine's prior 0%).
        if self.execution_skill < 0.18 {
            xg = xg.min(0.30);
        }
        xg.clamp(0.005, 0.82)
    }
}

/// Operations for shooting decision-making
pub struct ShootingOperationsImpl<'p> {
    ctx: &'p StateProcessingContext<'p>,
}

// Realistic shooting distances at the true field scale: 840u = 105m,
// 1u = 0.125m. Real football: ~60% of shots inside the 16.5m box
// (132u), ~10% beyond 25m (200u). The previous values were written on
// a ~0.5m/unit assumption, capping ALL shooting at ~12.5m real.
// 40 m — the same absolute cap `evaluate_forward_shot_decision` uses, so
// the ops layer and the decision layer agree on where a strike stops
// being worth considering. Was 27.5 m, which cut the tail off the shot
// distribution before the per-player `StrikingRange` model ever saw it.
const MAX_SHOOTING_DISTANCE: f32 = 320.0;
const MIN_SHOOTING_DISTANCE: f32 = 1.0;
const VERY_CLOSE_RANGE_DISTANCE: f32 = 60.0; // 7.5m - anyone can shoot
const CLOSE_RANGE_DISTANCE: f32 = 100.0; // 12.5m - close range shots
const OPTIMAL_SHOOTING_DISTANCE: f32 = 90.0; // 11.25m - ideal shooting distance
const MEDIUM_RANGE_DISTANCE: f32 = 150.0; // 18.75m - medium range shots

// Shooting decision thresholds
const SHOOT_OVER_PASS_CLOSE_THRESHOLD: f32 = 60.0; // Always prefer shooting if closer than this (7.5m)
const SHOOT_OVER_PASS_MEDIUM_THRESHOLD: f32 = 100.0; // Shoot over pass for decent finishers (12.5m)
const EXCELLENT_OPPORTUNITY_CLOSE_RANGE: f32 = 110.0; // Distance for close-range excellent opportunity

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
            // The ball is still on its restart, so a strike now IS that
            // set piece — same rule the event builder classifies by, so
            // the pre-shot gate and the in-flight resolution agree.
            set_piece: ShotType::from_restart(self.ctx.tick_context.ball.pass_origin_restart),
            standard_shift: MatchStandard::shift(self.ctx.context),
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
            // Anywhere inside the ideal band a striker is fully
            // confident — the previous linear ramp INVERTED reality
            // (a 2.5m tap-in read as low-confidence, an 11m strike as
            // peak).
            return 1.0;
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
