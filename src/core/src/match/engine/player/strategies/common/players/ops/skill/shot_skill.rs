use crate::r#match::MatchPlayer;
use crate::r#match::engine::set_pieces::PENALTY_EXECUTION_REFERENCE;
use crate::r#match::player::strategies::players::ops::effective_skill::{
    ActionContext as EffActionContext, effective_skill,
};
use crate::r#match::player::strategies::players::ops::skill_composites as sc;
use crate::r#match::player::strategies::players::ops::xg::ShotType;
use std::env::var;
use std::sync::OnceLock;

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
    /// What KIND of chance this is. `FootOpenPlay` is the neutral
    /// default and every multiplier below it is exactly 1.0, so
    /// classifying a shot never moves the population conversion rate on
    /// its own.
    ///
    /// Three of the types are a different ACTION rather than a different
    /// look at goal, and the profile executes each off its own skills:
    ///
    /// * a **penalty** and a **direct free kick** are not
    ///   shots-from-a-distance — they are struck from a standing start
    ///   with their own attributes and their own real-world conversion
    ///   bands. Without this the profile scored both with the open-play
    ///   finishing curve, which is why `penalty_taking` and `free_kicks`
    ///   could pick a taker and then have no effect at all on whether he
    ///   scored;
    /// * a **header** is aimed by the neck, not by the standing foot.
    ///   Same defect, same shape: `heading` won the duel that produced
    ///   the chance and then the strike was resolved on `finishing`,
    ///   `technique` and `balance` as though he had hit it with his
    ///   instep.
    pub shot_type: ShotType,
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
    /// Carried through from [`ShotSkillInputs::shot_type`] so
    /// [`Self::expected_xg`] can apply the dead-ball conversion band and
    /// the per-type multiplier.
    pub shot_type: ShotType,
}

/// **A header is struck with the head.**
///
/// `OF_HEADER_SHOT_OFF=1` restores the pre-2026-08-26 engine exactly: a
/// headed attempt is executed off the finishing-led open-play composite
/// and priced at a foot shot's conversion rate. The A/B control for the
/// channel — see [`ShotType::is_header`].
pub struct HeaderStrike;

impl HeaderStrike {
    #[inline]
    pub fn armed() -> bool {
        static ON: OnceLock<bool> = OnceLock::new();
        *ON.get_or_init(|| {
            !var("OF_HEADER_SHOT_OFF")
                .map(|v| v != "0" && !v.is_empty())
                .unwrap_or(false)
        })
    }
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

        // Per-action override. `penalty_execution` is deliberately linear
        // (a penalty is a technically trivial kick, so the skill response
        // is close to linear and composure carries a quarter of it);
        // `dead_ball_strike` is curved like the open-play composites,
        // because bending one over a wall genuinely is a specialist
        // action a mediocre striker of the ball simply cannot perform;
        // `header_finish` is curved the same way and led by `heading`,
        // because meeting a cross cleanly and steering it down is a
        // specialist action too — and the one the engine was resolving
        // on the striker's instep.
        //
        // `header_finish` is a weight-1 linear blend of `n(skill)`, so
        // `peer`-ing it against the standard of football in this match is
        // exact and is zero at the calibration division — same argument
        // as `receiving_first_touch`, and required because
        // `execution_skill` feeds absolute anchors
        // (`0.50 + execution*0.85` and friends) further down.
        let header = HeaderStrike::armed() && inputs.shot_type.is_header();
        let header_execution = || {
            let skill01 =
                (sc::header_finish(player, inputs.minute) - inputs.standard_shift).clamp(0.0, 1.0);
            pow_curve(skill01, 1.45)
        };
        let execution_skill = match inputs.shot_type {
            ShotType::Penalty => sc::penalty_execution(player, inputs.minute),
            ShotType::DirectFreeKick => sc::dead_ball_strike(player, inputs.minute),
            _ if header => header_execution(),
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
        // attribute that names it, so the same override applies. A header
        // goes the same way: a man cannot pick a corner with his head any
        // better than he can direct the ball at all, and steering it down
        // and across the keeper IS heading ability.
        let placement_skill = match inputs.shot_type {
            _ if inputs.shot_type.is_dead_ball() || header => execution_skill,
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
            shot_type: inputs.shot_type,
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
        match self.shot_type {
            // Real penalties convert 70-82%. The band is centred on the
            // population of *selected* takers so wiring `penalty_taking`
            // in redistributes conversion rather than shifting it.
            ShotType::Penalty => {
                let scale = (0.975 + (self.execution_skill - PENALTY_EXECUTION_REFERENCE) * 0.50)
                    .clamp(0.85, 1.10);
                return (0.76 * scale * self.shooting_condition_mult).clamp(0.55, 0.88);
            }
            // Real direct free kicks convert ~6-8% overall, and a
            // specialist from 20m is worth several times a defender
            // from the same spot. 60u ≈ 7.5m, 200u = 25m.
            ShotType::DirectFreeKick => {
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
        // What KIND of chance this is, at the same geometry. Exactly 1.0
        // for `FootOpenPlay` — the type every open-play strike carries —
        // so this is calibration-neutral by construction and only the
        // classified minority (headers) move.
        let type_mult = if HeaderStrike::armed() {
            self.shot_type.xg_multiplier()
        } else {
            1.0
        };
        let mut xg = distance_factor
            * self.shot_quality_multiplier
            * self.shooting_condition_mult
            * pressure_mult
            * clarity_mult
            * type_mult;

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
