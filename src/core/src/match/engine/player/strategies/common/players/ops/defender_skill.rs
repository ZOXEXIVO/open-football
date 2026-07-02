use crate::r#match::MatchPlayer;
use crate::r#match::StateProcessingContext;
use crate::r#match::player::strategies::players::ops::effective_skill::{SkillBands, SkillCategory};
use crate::r#match::player::strategies::players::ops::skill_composites as sc;

// ---------------------------------------------------------------------------
// DefenderSkillProfile — unified skill model for defenders.
// ---------------------------------------------------------------------------
//
// Single source of truth for the dozen defender decision sites that used
// to each branch on raw `vision >= 14.0`, `work_rate <= 10`, `pace*X`,
// or fixed danger-scan radii. Mirrors `MidfielderSkillProfile` for
// midfielders and `ShotSkillProfile` for shooting.

#[derive(Debug, Clone, Copy)]
pub struct DefenderSkillInputs {
    pub minute: u32,
    pub condition_pct: f32,
    pub pressure_count_5u: u32,
    pub pressure_count_10u: u32,
    pub distance_to_own_goal: f32,
    pub distance_to_opponent_goal: f32,
    pub recent_high_intensity: bool,
}

/// Continuous selection / execution profile for defenders. All values are
/// in 0..1 unless noted.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DefenderSkillProfile {
    pub poor_penalty: f32,
    pub elite_lift: f32,

    // Conditioning multipliers (0.50..1.03)
    pub def_condition_mult: f32,
    pub line_holding_mult: f32,
    pub marking_condition_mult: f32,
    pub tackling_condition_mult: f32,
    pub pressing_condition_mult: f32,
    pub recovery_run_mult: f32,
    pub clearance_condition_mult: f32,

    // Reading
    pub defensive_reading: f32,
    /// Reaction delay in ticks (lower is faster).
    pub reaction_delay_ticks: f32,
    pub hold_line_bias: f32,

    // Marking
    pub marking_profile: f32,
    pub ideal_marking_distance: f32,
    pub lost_runner_chance: f32,
    pub goal_side_weight: f32,

    // Cover
    pub cover_profile: f32,
    pub second_defender_cover_bonus: f32,

    // Duel / tackle
    pub tackle_profile: f32,
    pub discipline: f32,

    // Press
    pub press_profile: f32,

    // Interception
    pub interception_profile: f32,
    pub interception_radius: f32,
    pub shot_block_reaction: f32,

    // Aerial
    pub aerial_defense: f32,
    pub poor_aerial_mistake: f32,

    // Clearance
    pub clearance_profile: f32,
    pub poor_clearance_chance: f32,

    // Build-up / switch / overlap
    pub buildup_profile: f32,
    pub press_resistance: f32,
    pub defender_switch_profile: f32,
    pub overlap_profile: f32,

    // Pressure scaling
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

#[inline]
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t.clamp(0.0, 1.0)
}

impl DefenderSkillProfile {
    /// Memoized per (player, tick) — a defender's state machine reaches
    /// this from both `velocity()` and `process()` within one tick, and
    /// the ~26 banded skill reads + ~40 `powf` curves are the single
    /// costliest pure computation in the AI. Every input is tick-frozen
    /// (skills static in-match, condition updated once before the state
    /// runs, grid / ball / in_state_time snapshots), so the memo is
    /// bit-identical; the debug oracle recomputes and compares on every
    /// hit.
    pub fn from_ctx(ctx: &StateProcessingContext) -> Self {
        let tick = ctx.current_tick();
        let cached = ctx
            .tick_context
            .player_agg_cache
            .borrow_mut()
            .slot_mut(ctx.player.id, tick)
            .defender_profile;
        if let Some(profile) = cached {
            debug_assert!(
                profile == Self::compute_from_ctx(ctx),
                "defender-profile memo mismatch"
            );
            return profile;
        }
        let profile = Self::compute_from_ctx(ctx);
        ctx.tick_context
            .player_agg_cache
            .borrow_mut()
            .slot_mut(ctx.player.id, tick)
            .defender_profile = Some(profile);
        profile
    }

    fn compute_from_ctx(ctx: &StateProcessingContext) -> Self {
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

        let inputs = DefenderSkillInputs {
            minute,
            condition_pct,
            pressure_count_5u: pressure_5u,
            pressure_count_10u: pressure_10u,
            distance_to_own_goal: ctx.ball().distance_to_own_goal(),
            distance_to_opponent_goal: ctx.ball().distance_to_opponent_goal(),
            recent_high_intensity: ctx.in_state_time as f32 > 30.0,
        };
        Self::from_player_memo(ctx, &inputs)
    }

    /// Everything `from_player` reads that can vary in-match, packed into
    /// one cross-tick memo key: condition, jadedness, minute and the two
    /// pressure counts (≤ 11 opponents, so 8 bits each is exact). The
    /// distance / recent-intensity inputs are declared but unused by the
    /// profile body (see the `let _ = (...)` marker there), so they stay
    /// out of the key. Everything else the body touches — skills, traits,
    /// crowd arousal — is static for the whole match.
    #[inline]
    fn memo_key(player: &MatchPlayer, inputs: &DefenderSkillInputs) -> u64 {
        (player.player_attributes.condition as u16 as u64)
            | (player.player_attributes.jadedness as u16 as u64) << 16
            | (inputs.minute as u64 & 0xFF) << 32
            | (inputs.pressure_count_5u as u64 & 0xFF) << 40
            | (inputs.pressure_count_10u as u64 & 0xFF) << 48
    }

    /// Cross-tick memoized `from_player`. Condition / jadedness / minute /
    /// pressure counts move on a much slower cadence than the tick rate,
    /// so the ~40 `powf` curve evaluations are only re-run when one of
    /// them actually changes; the cached profile is bit-identical in
    /// between (debug oracle on every hit). The memo rows live on the
    /// tick context (see `ProfileMemos`) to keep `MatchPlayer` `Sync`.
    fn from_player_memo(ctx: &StateProcessingContext, inputs: &DefenderSkillInputs) -> Self {
        let player = ctx.player;
        let key = Self::memo_key(player, inputs);
        let cached = ctx
            .tick_context
            .profile_memos
            .borrow()
            .defender_get(player.id, key);
        if let Some(profile) = cached {
            debug_assert!(
                profile == Self::from_player(player, inputs),
                "defender-profile cross-tick memo mismatch"
            );
            return profile;
        }
        let profile = Self::from_player(player, inputs);
        ctx.tick_context
            .profile_memos
            .borrow_mut()
            .defender_put(player.id, key, profile);
        profile
    }

    pub fn from_player(player: &MatchPlayer, inputs: &DefenderSkillInputs) -> Self {
        let bands = SkillBands::for_player(player, inputs.minute);
        let s = &player.skills;

        // ── Effective skill reads (factored fatigue bands; bit-identical
        //    to per-read effective_skill — see SkillBands) ──────────────
        let tackling_eff = bands.apply(s.technical.tackling, SkillCategory::Technical);
        let marking_eff = bands.apply(s.technical.marking, SkillCategory::Technical);
        let heading_eff = bands.apply(s.technical.heading, SkillCategory::Technical);
        let passing_eff = bands.apply(s.technical.passing, SkillCategory::Technical);
        let technique_eff = bands.apply(s.technical.technique, SkillCategory::Technical);
        let first_touch_eff = bands.apply(s.technical.first_touch, SkillCategory::Technical);
        let crossing_eff = bands.apply(s.technical.crossing, SkillCategory::Technical);

        let positioning_eff = bands.apply(s.mental.positioning, SkillCategory::Mental);
        let anticipation_eff = bands.apply(s.mental.anticipation, SkillCategory::Mental);
        let concentration_eff = bands.apply(s.mental.concentration, SkillCategory::Mental);
        let decisions_eff = bands.apply(s.mental.decisions, SkillCategory::Mental);
        let composure_eff = bands.apply(s.mental.composure, SkillCategory::Mental);
        let bravery_eff = bands.apply(s.mental.bravery, SkillCategory::Mental);
        let aggression_eff = bands.apply(s.mental.aggression, SkillCategory::Mental);
        let teamwork_eff = bands.apply(s.mental.teamwork, SkillCategory::Mental);
        let work_rate_eff = bands.apply(s.mental.work_rate, SkillCategory::Mental);
        let leadership_eff = bands.apply(s.mental.leadership, SkillCategory::Mental);
        let vision_eff = bands.apply(s.mental.vision, SkillCategory::Mental);
        let off_ball_eff = bands.apply(s.mental.off_the_ball, SkillCategory::Mental);

        let strength_eff = bands.apply(s.physical.strength, SkillCategory::Explosive);
        let jumping_eff = bands.apply(s.physical.jumping, SkillCategory::Explosive);
        let pace_eff = bands.apply(s.physical.pace, SkillCategory::Explosive);
        let acceleration_eff = bands.apply(s.physical.acceleration, SkillCategory::Explosive);
        let agility_eff = bands.apply(s.physical.agility, SkillCategory::Explosive);
        let balance_eff = bands.apply(s.physical.balance, SkillCategory::Technical);
        let stamina_eff = bands.apply(s.physical.stamina, SkillCategory::Explosive);

        // ── Normalised reads ─────────────────────────────────────────
        let tackling01 = norm01(tackling_eff);
        let marking01 = norm01(marking_eff);
        let heading01 = norm01(heading_eff);
        let passing01 = norm01(passing_eff);
        let technique01 = norm01(technique_eff);
        let first_touch01 = norm01(first_touch_eff);
        let crossing01 = norm01(crossing_eff);

        let positioning01 = norm01(positioning_eff);
        let anticipation01 = norm01(anticipation_eff);
        let concentration01 = norm01(concentration_eff);
        let decisions01 = norm01(decisions_eff);
        let composure01 = norm01(composure_eff);
        let bravery01 = norm01(bravery_eff);
        let aggression01 = norm01(aggression_eff);
        let teamwork01 = norm01(teamwork_eff);
        let work_rate01 = norm01(work_rate_eff);
        let leadership01 = norm01(leadership_eff);
        let vision01 = norm01(vision_eff);
        let off_ball01 = norm01(off_ball_eff);

        let strength01 = norm01(strength_eff);
        let jumping01 = norm01(jumping_eff);
        let pace01 = norm01(pace_eff);
        let acceleration01 = norm01(acceleration_eff);
        let agility01 = norm01(agility_eff);
        let balance01 = norm01(balance_eff);
        let stamina01 = norm01(stamina_eff);

        // ── Headline mappers ─────────────────────────────────────────
        // Defender "core" centres on positioning / tackling / marking /
        // anticipation — the four attributes that separate good CBs from
        // weak ones.
        let core01 = (positioning01 * 0.28
            + tackling01 * 0.22
            + marking01 * 0.20
            + anticipation01 * 0.18
            + concentration01 * 0.12)
            .clamp(0.0, 1.0);
        let poor_penalty = smoothstep(0.45, 0.18, core01);
        let elite_lift = smoothstep(0.72, 0.95, core01);

        // ── Conditioning model ───────────────────────────────────────
        let cond = inputs.condition_pct.clamp(0.0, 1.0);
        let nat_fit01 = norm01(s.physical.natural_fitness);
        let match_readiness01 = norm01(s.physical.match_readiness);
        let determination01 = norm01(s.mental.determination);
        let fitness =
            stamina01 * 0.40 + nat_fit01 * 0.30 + match_readiness01 * 0.20 + determination01 * 0.10;
        let fatigue = (1.0 - cond).max(0.0).powf(1.30);
        let jadedness = (player.player_attributes.jadedness as f32 / 10_000.0).clamp(0.0, 1.0);
        let jadedness_penalty = jadedness * 0.16;
        let fitness_recovery = 1.0 - fatigue * (0.17 + fitness * 0.22);
        let mental_fatigue = 1.0 - fatigue * (0.08 + poor_penalty * 0.20);
        let late_drop = if inputs.minute >= 70 {
            1.0 - ((inputs.minute as f32 - 70.0) / 50.0).clamp(0.0, 1.0)
                * (0.05 + poor_penalty * 0.12)
        } else {
            1.0
        };
        let def_condition_mult =
            (fitness_recovery * mental_fatigue * late_drop - jadedness_penalty).clamp(0.50, 1.03);

        let stamina_curve = pow_curve(stamina01, 1.20);
        let composure_curve = pow_curve(composure01, 1.30);
        let balance_curve = pow_curve(balance01, 1.20);
        let concentration_curve = pow_curve(concentration01, 1.20);
        let work_rate_curve = pow_curve(work_rate01, 1.20);
        let pace_curve = pow_curve(pace01, 1.20);
        let technique_curve = pow_curve(technique01, 1.30);
        let strength_curve = pow_curve(strength01, 1.20);

        let line_holding_mult =
            def_condition_mult * (0.90 + concentration_curve * 0.10).clamp(0.75, 1.03);
        let marking_condition_mult = def_condition_mult
            * (0.85 + stamina_curve * 0.08 + concentration_curve * 0.07).clamp(0.68, 1.04);
        let tackling_condition_mult = def_condition_mult
            * (0.82 + balance_curve * 0.08 + composure_curve * 0.10).clamp(0.62, 1.04);
        let pressing_condition_mult = def_condition_mult
            * (0.80 + stamina_curve * 0.12 + work_rate_curve * 0.08).clamp(0.60, 1.05);
        let recovery_run_mult = def_condition_mult
            * (0.78 + pace_curve * 0.10 + stamina_curve * 0.12).clamp(0.58, 1.05);
        let clearance_condition_mult = def_condition_mult
            * (0.88 + technique_curve * 0.07 + strength_curve * 0.05).clamp(0.70, 1.03);

        // ── Pressure penalty ─────────────────────────────────────────
        let pressure_penalty = (inputs.pressure_count_5u as f32 * 0.20
            + inputs.pressure_count_10u as f32 * 0.07)
            .clamp(0.0, 1.0);

        // ── Defensive reading ────────────────────────────────────────
        // No `communication` mental attribute on outfield players —
        // proxy with leadership/teamwork average so the slot still loads.
        let communication_proxy = (leadership01 + teamwork01) * 0.5;
        let defensive_reading = (pow_curve(positioning01, 1.45) * 0.24
            + pow_curve(anticipation01, 1.45) * 0.22
            + pow_curve(concentration01, 1.35) * 0.16
            + pow_curve(decisions01, 1.35) * 0.14
            + pow_curve(teamwork01, 1.25) * 0.08
            + pow_curve(composure01, 1.25) * 0.08
            + pow_curve(leadership01, 1.15) * 0.04
            + communication_proxy * 0.04)
            .clamp(0.0, 1.0);

        let reaction_delay_ticks =
            (30.0 - defensive_reading * 22.0 + fatigue * 12.0).clamp(4.0, 40.0);
        let hold_line_bias = defensive_reading - poor_penalty * 0.20;

        // ── Marking ──────────────────────────────────────────────────
        let marking_profile = ((pow_curve(marking01, 1.55) * 0.24
            + pow_curve(positioning01, 1.45) * 0.20
            + pow_curve(concentration01, 1.35) * 0.16
            + pow_curve(anticipation01, 1.35) * 0.14
            + pow_curve(strength01, 1.20) * 0.08
            + pow_curve(agility01, 1.20) * 0.08
            + pow_curve(pace01, 1.15) * 0.05
            + pow_curve(bravery01, 1.10) * 0.05)
            * marking_condition_mult)
            .clamp(0.0, 1.0);
        let ideal_marking_distance = lerp(14.0, 7.0, marking_profile);
        let lost_runner_chance =
            (0.22 - marking_profile * 0.18 + fatigue * 0.12 + poor_penalty * 0.10)
                .clamp(0.02, 0.35);
        let goal_side_weight =
            (0.45 + pow_curve(positioning01, 1.30) * 0.30 + pow_curve(decisions01, 1.30) * 0.15)
                .clamp(0.45, 0.85);

        // ── Cover ────────────────────────────────────────────────────
        let cover_profile = (pow_curve(positioning01, 1.50) * 0.24
            + pow_curve(anticipation01, 1.45) * 0.22
            + pow_curve(acceleration01, 1.25) * 0.12
            + pow_curve(pace01, 1.20) * 0.10
            + pow_curve(decisions01, 1.35) * 0.10
            + pow_curve(concentration01, 1.25) * 0.10
            + pow_curve(teamwork01, 1.20) * 0.08
            + pow_curve(agility01, 1.15) * 0.04)
            .clamp(0.0, 1.0);
        let second_defender_cover_bonus = 0.06 + cover_profile * 0.10;

        // ── Tackle ───────────────────────────────────────────────────
        let tackle_profile = ((pow_curve(tackling01, 1.65) * 0.28
            + pow_curve(positioning01, 1.45) * 0.16
            + pow_curve(anticipation01, 1.40) * 0.14
            + pow_curve(composure01, 1.35) * 0.12
            + pow_curve(strength01, 1.25) * 0.10
            + pow_curve(balance01, 1.25) * 0.08
            + pow_curve(agility01, 1.20) * 0.06
            + pow_curve(concentration01, 1.20) * 0.06)
            * tackling_condition_mult)
            .clamp(0.0, 1.0);

        // ── Discipline ───────────────────────────────────────────────
        // No `temperament` attribute — proxy with composure+concentration.
        let temperament_proxy = (composure01 + concentration01) * 0.5;
        let aggression_inverse = 1.0 - aggression01;
        let discipline = (pow_curve(composure01, 1.35) * 0.24
            + pow_curve(decisions01, 1.35) * 0.22
            + pow_curve(tackling01, 1.35) * 0.18
            + pow_curve(concentration01, 1.20) * 0.12
            + temperament_proxy * 0.10
            + aggression_inverse * 0.10
            + pow_curve(bravery01, 1.10) * 0.04)
            .clamp(0.0, 1.0);

        // ── Press ────────────────────────────────────────────────────
        let press_profile = ((pow_curve(work_rate01, 1.35) * 0.20
            + pow_curve(acceleration01, 1.25) * 0.16
            + pow_curve(stamina01, 1.25) * 0.14
            + pow_curve(anticipation01, 1.35) * 0.14
            + pow_curve(aggression01, 1.15) * 0.10
            + pow_curve(positioning01, 1.30) * 0.10
            + pow_curve(decisions01, 1.25) * 0.08
            + pow_curve(teamwork01, 1.15) * 0.08)
            * pressing_condition_mult)
            .clamp(0.0, 1.0);

        // ── Interception ─────────────────────────────────────────────
        let interception_profile = (pow_curve(anticipation01, 1.55) * 0.24
            + pow_curve(positioning01, 1.45) * 0.22
            + pow_curve(concentration01, 1.35) * 0.16
            + pow_curve(acceleration01, 1.25) * 0.12
            + pow_curve(pace01, 1.15) * 0.08
            + pow_curve(agility01, 1.15) * 0.06
            + pow_curve(decisions01, 1.30) * 0.06
            + pow_curve(bravery01, 1.10) * 0.06)
            .clamp(0.0, 1.0);
        let interception_radius = 3.0 + interception_profile * 6.0;
        let shot_block_reaction =
            (0.08 + interception_profile * 0.38 + pow_curve(bravery01, 1.10) * 0.08)
                .clamp(0.05, 0.58);

        // ── Aerial defense ───────────────────────────────────────────
        let aerial_defense = (pow_curve(heading01, 1.55) * 0.24
            + pow_curve(jumping01, 1.45) * 0.18
            + pow_curve(strength01, 1.30) * 0.14
            + pow_curve(positioning01, 1.35) * 0.14
            + pow_curve(bravery01, 1.25) * 0.10
            + pow_curve(anticipation01, 1.30) * 0.10
            + pow_curve(concentration01, 1.20) * 0.06
            + pow_curve(balance01, 1.15) * 0.04)
            .clamp(0.0, 1.0);
        let poor_aerial_mistake =
            (poor_penalty * 0.12 + fatigue * 0.08 - aerial_defense * 0.06).clamp(0.01, 0.22);

        // ── Clearance ────────────────────────────────────────────────
        let clearance_profile = ((pow_curve(technique01, 1.35) * 0.18
            + pow_curve(passing01, 1.25) * 0.14
            + pow_curve(decisions01, 1.35) * 0.16
            + pow_curve(composure01, 1.35) * 0.16
            + pow_curve(strength01, 1.20) * 0.14
            + pow_curve(balance01, 1.15) * 0.08
            + pow_curve(concentration01, 1.20) * 0.08
            + pow_curve(vision01, 1.15) * 0.06)
            * clearance_condition_mult)
            .clamp(0.0, 1.0);
        let poor_clearance_chance =
            (0.18 - clearance_profile * 0.12 + pressure_penalty * 0.08 + fatigue * 0.08)
                .clamp(0.03, 0.35);

        // ── Build-up ─────────────────────────────────────────────────
        let buildup_profile = ((pow_curve(passing01, 1.45) * 0.24
            + pow_curve(technique01, 1.35) * 0.16
            + pow_curve(first_touch01, 1.35) * 0.14
            + pow_curve(composure01, 1.45) * 0.14
            + pow_curve(decisions01, 1.40) * 0.14
            + pow_curve(vision01, 1.35) * 0.10
            + pow_curve(concentration01, 1.20) * 0.04
            + pow_curve(teamwork01, 1.15) * 0.04)
            * def_condition_mult)
            .clamp(0.0, 1.0);

        // Press resistance — same shape as midfielder profile (scaled
        // for defenders): first_touch + technique + composure + balance.
        let press_resistance = (pow_curve(first_touch01, 1.40) * 0.24
            + pow_curve(technique01, 1.35) * 0.18
            + pow_curve(composure01, 1.45) * 0.18
            + pow_curve(balance01, 1.30) * 0.14
            + pow_curve(strength01, 1.10) * 0.10
            + pow_curve(decisions01, 1.30) * 0.10
            + pow_curve(concentration01, 1.15) * 0.06)
            .clamp(0.0, 1.0);

        let defender_switch_profile = (pow_curve(passing01, 1.45) * 0.24
            + pow_curve(vision01, 1.50) * 0.24
            + pow_curve(technique01, 1.40) * 0.18
            + pow_curve(decisions01, 1.35) * 0.14
            + pow_curve(composure01, 1.30) * 0.10
            + pow_curve(strength01, 1.10) * 0.05
            + pow_curve(concentration01, 1.15) * 0.05)
            .clamp(0.0, 1.0);

        // ── Overlap (fullback) ───────────────────────────────────────
        let overlap_profile = ((pow_curve(stamina01, 1.25) * 0.18
            + pow_curve(work_rate01, 1.30) * 0.18
            + pow_curve(pace01, 1.20) * 0.14
            + pow_curve(acceleration01, 1.20) * 0.12
            + pow_curve(off_ball01, 1.35) * 0.12
            + pow_curve(decisions01, 1.30) * 0.10
            + pow_curve(crossing01, 1.35) * 0.08
            + pow_curve(teamwork01, 1.15) * 0.08)
            * recovery_run_mult)
            .clamp(0.0, 1.0);

        let _ = (
            inputs.distance_to_own_goal,
            inputs.distance_to_opponent_goal,
            inputs.recent_high_intensity,
        );

        DefenderSkillProfile {
            poor_penalty,
            elite_lift,
            def_condition_mult,
            line_holding_mult,
            marking_condition_mult,
            tackling_condition_mult,
            pressing_condition_mult,
            recovery_run_mult,
            clearance_condition_mult,
            defensive_reading,
            reaction_delay_ticks,
            hold_line_bias,
            marking_profile,
            ideal_marking_distance,
            lost_runner_chance,
            goal_side_weight,
            cover_profile,
            second_defender_cover_bonus,
            tackle_profile,
            discipline,
            press_profile,
            interception_profile,
            interception_radius,
            shot_block_reaction,
            aerial_defense,
            poor_aerial_mistake,
            clearance_profile,
            poor_clearance_chance,
            buildup_profile,
            press_resistance,
            defender_switch_profile,
            overlap_profile,
            pressure_penalty,
        }
    }

    /// Whether the defender should engage in primary pressing.
    #[inline]
    pub fn allows_primary_press(&self) -> bool {
        self.press_profile >= 0.34
    }

    /// Whether the defender should engage in counter-pressing.
    #[inline]
    pub fn allows_counterpress(&self) -> bool {
        self.press_profile >= 0.42 && self.def_condition_mult >= 0.68
    }

    /// Whether the defender should attempt a switch-of-play pass.
    #[inline]
    pub fn allows_switch(&self) -> bool {
        self.defender_switch_profile >= 0.52
    }

    /// Whether a CB / FB should attempt a progressive pass under normal
    /// pressure conditions.
    #[inline]
    pub fn allows_progressive_pass(&self) -> bool {
        self.buildup_profile >= 0.46
    }

    /// Whether the defender should clear under heavy pressure rather
    /// than try to play out.
    #[inline]
    pub fn must_clear_under_pressure(&self) -> bool {
        self.buildup_profile < 0.36
    }

    /// Whether a fullback should attempt an overlap run.
    #[inline]
    pub fn allows_overlap(&self) -> bool {
        self.overlap_profile >= 0.46 && self.def_condition_mult >= 0.72
    }

    /// Whether a fullback should attempt a late-lead overlap (riskier).
    #[inline]
    pub fn allows_late_lead_overlap(&self) -> bool {
        self.overlap_profile >= 0.70
    }

    /// Whether the defender should step out of the line.
    #[inline]
    pub fn allows_step_up(&self) -> bool {
        self.defensive_reading >= 0.42
    }

    /// Press boost multiplier on closing speed.
    #[inline]
    pub fn press_boost(&self) -> f32 {
        // Skill-driven swing trimmed 0.55 → 0.35 because the steeper curve
        // gave strong defenders a 30%+ closing-speed advantage over weak
        // ones (1.16 → 1.51). Real elite-vs-poor defenders aren't that
        // far apart in raw closing speed — the gap lives more in
        // anticipation/positioning (which the gate functions already
        // model) than in straight-line pace. The trimmed curve keeps the
        // floor (1.10) above slow-jog territory while capping the
        // ceiling (1.45) so weak attackers carrying into the final third
        // get realistic milliseconds to make a decision before contact.
        (1.10 + self.press_profile * 0.35).clamp(1.10, 1.45)
    }

    /// Tackling closing-speed boost (tackle_profile + press_profile).
    #[inline]
    pub fn tackle_speed_boost(&self) -> f32 {
        // Differential trimmed 0.35/0.20 → 0.22/0.13 — same reasoning as
        // `press_boost`. Strong defenders still close faster, but the
        // gap doesn't compound into the strong-defender-always-catches
        // dynamic at extreme skill mismatches.
        1.05 + self.press_profile * 0.22 + self.tackle_profile * 0.13
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
        let s = &mut skills;
        s.technical.tackling = fill;
        s.technical.marking = fill;
        s.technical.heading = fill;
        s.technical.passing = fill;
        s.technical.technique = fill;
        s.technical.first_touch = fill;
        s.technical.crossing = fill;
        s.technical.dribbling = fill;
        s.mental.positioning = fill;
        s.mental.anticipation = fill;
        s.mental.concentration = fill;
        s.mental.decisions = fill;
        s.mental.composure = fill;
        s.mental.bravery = fill;
        s.mental.aggression = fill;
        s.mental.teamwork = fill;
        s.mental.work_rate = fill;
        s.mental.leadership = fill;
        s.mental.vision = fill;
        s.mental.off_the_ball = fill;
        s.mental.determination = fill;
        s.mental.flair = fill;
        s.physical.strength = fill;
        s.physical.jumping = fill;
        s.physical.pace = fill;
        s.physical.acceleration = fill;
        s.physical.agility = fill;
        s.physical.balance = fill;
        s.physical.stamina = fill;
        s.physical.natural_fitness = fill;
        s.physical.match_readiness = fill;
        let player = PlayerBuilder::new()
            .id(1)
            .full_name(FullName::new("D".to_string(), "P".to_string()))
            .birth_date(NaiveDate::from_ymd_opt(2000, 1, 1).unwrap())
            .country_id(1)
            .attributes(PersonAttributes::default())
            .skills(skills)
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: PlayerPositionType::DefenderCenter,
                    level: 18,
                }],
            })
            .player_attributes(attrs)
            .build()
            .unwrap();
        MatchPlayer::from_player(1, &player, PlayerPositionType::DefenderCenter, false)
    }

    fn default_inputs() -> DefenderSkillInputs {
        DefenderSkillInputs {
            minute: 30,
            condition_pct: 0.95,
            pressure_count_5u: 0,
            pressure_count_10u: 0,
            distance_to_own_goal: 100.0,
            distance_to_opponent_goal: 700.0,
            recent_high_intensity: false,
        }
    }

    #[test]
    fn poor_defender_has_low_reading_and_marking() {
        let p = build_player(5.0, 9000);
        let prof = DefenderSkillProfile::from_player(&p, &default_inputs());
        assert!(prof.defensive_reading < 0.30);
        assert!(prof.marking_profile < 0.30);
        assert!(prof.poor_penalty > 0.5);
        // Poor defender stands further from his man.
        assert!(prof.ideal_marking_distance > 11.0);
    }

    #[test]
    fn elite_defender_has_high_reading_and_marking() {
        let p = build_player(18.0, 9000);
        let prof = DefenderSkillProfile::from_player(&p, &default_inputs());
        assert!(prof.defensive_reading > 0.65);
        assert!(prof.marking_profile > 0.65);
        // Elite defender marks closer.
        assert!(prof.ideal_marking_distance < 9.0);
        // Elite defender allows step-up.
        assert!(prof.allows_step_up());
    }

    #[test]
    fn elite_defender_has_lower_lost_runner_chance() {
        let elite = build_player(17.0, 9000);
        let poor = build_player(5.0, 9000);
        let pe = DefenderSkillProfile::from_player(&elite, &default_inputs());
        let pp = DefenderSkillProfile::from_player(&poor, &default_inputs());
        assert!(pe.lost_runner_chance < pp.lost_runner_chance);
    }

    #[test]
    fn elite_buildup_unlocks_progressive_play() {
        let elite = build_player(17.0, 9000);
        let poor = build_player(5.0, 9000);
        let pe = DefenderSkillProfile::from_player(&elite, &default_inputs());
        let pp = DefenderSkillProfile::from_player(&poor, &default_inputs());
        assert!(pe.allows_progressive_pass());
        assert!(!pp.allows_progressive_pass());
        assert!(pp.must_clear_under_pressure());
    }

    #[test]
    fn elite_switch_pass_gated_by_skill() {
        let elite = build_player(17.0, 9000);
        let poor = build_player(5.0, 9000);
        let pe = DefenderSkillProfile::from_player(&elite, &default_inputs());
        let pp = DefenderSkillProfile::from_player(&poor, &default_inputs());
        assert!(pe.allows_switch());
        assert!(!pp.allows_switch());
    }

    #[test]
    fn fatigue_drops_pressing_and_clearance() {
        let fresh_p = build_player(15.0, 9500);
        let tired_p = build_player(15.0, 2500);
        let fresh = DefenderSkillProfile::from_player(
            &fresh_p,
            &DefenderSkillInputs {
                minute: 80,
                condition_pct: 0.95,
                ..default_inputs()
            },
        );
        let tired = DefenderSkillProfile::from_player(
            &tired_p,
            &DefenderSkillInputs {
                minute: 80,
                condition_pct: 0.25,
                ..default_inputs()
            },
        );
        assert!(fresh.press_profile > tired.press_profile);
        assert!(fresh.clearance_profile > tired.clearance_profile);
        assert!(fresh.def_condition_mult > tired.def_condition_mult);
    }

    #[test]
    fn discipline_higher_for_composed_defender() {
        let mut composed = build_player(12.0, 9000);
        composed.skills.mental.composure = 18.0;
        composed.skills.mental.decisions = 17.0;
        composed.skills.mental.aggression = 6.0;
        let mut reckless = build_player(12.0, 9000);
        reckless.skills.mental.composure = 7.0;
        reckless.skills.mental.decisions = 7.0;
        reckless.skills.mental.aggression = 18.0;
        let cd = DefenderSkillProfile::from_player(&composed, &default_inputs()).discipline;
        let rd = DefenderSkillProfile::from_player(&reckless, &default_inputs()).discipline;
        assert!(cd > rd + 0.10);
    }

    #[test]
    fn elite_overlap_skill_gate() {
        let mut elite = build_player(15.0, 9000);
        elite.skills.physical.stamina = 18.0;
        elite.skills.mental.work_rate = 18.0;
        elite.skills.physical.pace = 17.0;
        elite.skills.mental.off_the_ball = 16.0;
        let weak = build_player(7.0, 9000);
        let pe = DefenderSkillProfile::from_player(&elite, &default_inputs());
        let pw = DefenderSkillProfile::from_player(&weak, &default_inputs());
        assert!(pe.allows_overlap());
        assert!(!pw.allows_overlap());
    }
}
