use crate::r#match::MatchPlayer;
use crate::r#match::StateProcessingContext;
use crate::r#match::player::strategies::players::ops::effective_skill::{
    SkillBands, SkillCategory,
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
#[derive(Debug, Clone, Copy, PartialEq)]
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

#[derive(Debug, Clone, Copy)]
struct MidfielderSkillReads {
    passing01: f32,
    technique01: f32,
    first_touch01: f32,
    dribbling01: f32,
    long_shots01: f32,
    finishing01: f32,
    tackling01: f32,
    vision01: f32,
    decisions01: f32,
    composure01: f32,
    concentration01: f32,
    anticipation01: f32,
    teamwork01: f32,
    off_ball01: f32,
    positioning01: f32,
    work_rate01: f32,
    bravery01: f32,
    aggression01: f32,
    flair01: f32,
    stamina01: f32,
    acceleration01: f32,
    agility01: f32,
    balance01: f32,
    strength01: f32,
}

impl MidfielderSkillReads {
    fn of(player: &MatchPlayer, minute: u32) -> Self {
        let bands = SkillBands::for_player(player, minute);
        let s = &player.skills;

        let passing_eff = bands.apply(s.technical.passing, SkillCategory::Technical);
        let technique_eff = bands.apply(s.technical.technique, SkillCategory::Technical);
        let first_touch_eff = bands.apply(s.technical.first_touch, SkillCategory::Technical);
        let dribbling_eff = bands.apply(s.technical.dribbling, SkillCategory::Technical);
        let long_shots_eff = bands.apply(s.technical.long_shots, SkillCategory::Technical);
        let finishing_eff = bands.apply(s.technical.finishing, SkillCategory::Technical);
        let tackling_eff = bands.apply(s.technical.tackling, SkillCategory::Technical);

        let vision_eff = bands.apply(s.mental.vision, SkillCategory::Mental);
        let decisions_eff = bands.apply(s.mental.decisions, SkillCategory::Mental);
        let composure_eff = bands.apply(s.mental.composure, SkillCategory::Mental);
        let concentration_eff = bands.apply(s.mental.concentration, SkillCategory::Mental);
        let anticipation_eff = bands.apply(s.mental.anticipation, SkillCategory::Mental);
        let teamwork_eff = bands.apply(s.mental.teamwork, SkillCategory::Mental);
        let off_ball_eff = bands.apply(s.mental.off_the_ball, SkillCategory::Mental);
        let positioning_eff = bands.apply(s.mental.positioning, SkillCategory::Mental);
        let work_rate_eff = bands.apply(s.mental.work_rate, SkillCategory::Mental);
        let bravery_eff = bands.apply(s.mental.bravery, SkillCategory::Mental);
        let aggression_eff = bands.apply(s.mental.aggression, SkillCategory::Mental);
        let flair_eff = bands.apply(s.mental.flair, SkillCategory::Mental);

        let stamina_eff = bands.apply(s.physical.stamina, SkillCategory::Explosive);
        let acceleration_eff = bands.apply(s.physical.acceleration, SkillCategory::Explosive);
        let agility_eff = bands.apply(s.physical.agility, SkillCategory::Explosive);
        let balance_eff = bands.apply(s.physical.balance, SkillCategory::Technical);
        let strength_eff = bands.apply(s.physical.strength, SkillCategory::Explosive);

        // ── Normalised reads ─────────────────────────────────────────

        Self {
            passing01: MidfielderSkillProfile::norm01(passing_eff),
            technique01: MidfielderSkillProfile::norm01(technique_eff),
            first_touch01: MidfielderSkillProfile::norm01(first_touch_eff),
            dribbling01: MidfielderSkillProfile::norm01(dribbling_eff),
            long_shots01: MidfielderSkillProfile::norm01(long_shots_eff),
            finishing01: MidfielderSkillProfile::norm01(finishing_eff),
            tackling01: MidfielderSkillProfile::norm01(tackling_eff),
            vision01: MidfielderSkillProfile::norm01(vision_eff),
            decisions01: MidfielderSkillProfile::norm01(decisions_eff),
            composure01: MidfielderSkillProfile::norm01(composure_eff),
            concentration01: MidfielderSkillProfile::norm01(concentration_eff),
            anticipation01: MidfielderSkillProfile::norm01(anticipation_eff),
            teamwork01: MidfielderSkillProfile::norm01(teamwork_eff),
            off_ball01: MidfielderSkillProfile::norm01(off_ball_eff),
            positioning01: MidfielderSkillProfile::norm01(positioning_eff),
            work_rate01: MidfielderSkillProfile::norm01(work_rate_eff),
            bravery01: MidfielderSkillProfile::norm01(bravery_eff),
            aggression01: MidfielderSkillProfile::norm01(aggression_eff),
            flair01: MidfielderSkillProfile::norm01(flair_eff),
            stamina01: MidfielderSkillProfile::norm01(stamina_eff),
            acceleration01: MidfielderSkillProfile::norm01(acceleration_eff),
            agility01: MidfielderSkillProfile::norm01(agility_eff),
            balance01: MidfielderSkillProfile::norm01(balance_eff),
            strength01: MidfielderSkillProfile::norm01(strength_eff),
        }
    }

    fn core01(&self) -> f32 {
        let core01 = (self.passing01 * 0.30
            + self.decisions01 * 0.25
            + self.vision01 * 0.20
            + self.composure01 * 0.15
            + self.technique01 * 0.10)
            .clamp(0.0, 1.0);
        core01
    }
}

#[derive(Debug, Clone, Copy)]
struct MidfielderConditioning {
    mid_condition_mult: f32,
    passing_condition_mult: f32,
    first_touch_condition_mult: f32,
    pressing_condition_mult: f32,
    tackling_condition_mult: f32,
    off_ball_condition_mult: f32,
}

impl MidfielderConditioning {
    fn of(
        player: &MatchPlayer,
        inputs: &MidfielderSkillInputs,
        reads: &MidfielderSkillReads,
        poor_penalty: f32,
    ) -> Self {
        let cond = inputs.condition_pct.clamp(0.0, 1.0);
        let nat_fit01 = MidfielderSkillProfile::norm01(player.skills.physical.natural_fitness);
        let match_readiness01 =
            MidfielderSkillProfile::norm01(player.skills.physical.match_readiness);
        let fitness = reads.stamina01 * 0.45 + nat_fit01 * 0.35 + match_readiness01 * 0.20;
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

        let stamina_curve = MidfielderSkillProfile::pow_curve(reads.stamina01, 1.20);
        let composure_curve = MidfielderSkillProfile::pow_curve(reads.composure01, 1.30);
        let balance_curve = MidfielderSkillProfile::pow_curve(reads.balance01, 1.20);
        let anticipation_curve = MidfielderSkillProfile::pow_curve(reads.anticipation01, 1.20);

        let passing_condition_mult = mid_condition_mult;
        let first_touch_condition_mult =
            mid_condition_mult * (0.92 + balance_curve * 0.08).clamp(0.85, 1.02);
        let pressing_condition_mult =
            mid_condition_mult * (0.82 + stamina_curve * 0.18).clamp(0.65, 1.05);
        let tackling_condition_mult = mid_condition_mult
            * (0.84 + composure_curve * 0.08 + balance_curve * 0.08).clamp(0.65, 1.03);
        let off_ball_condition_mult = mid_condition_mult
            * (0.80 + stamina_curve * 0.12 + anticipation_curve * 0.08).clamp(0.62, 1.05);

        MidfielderConditioning {
            mid_condition_mult,
            passing_condition_mult,
            first_touch_condition_mult,
            pressing_condition_mult,
            tackling_condition_mult,
            off_ball_condition_mult,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct MidfielderPassGame {
    pass_execution: f32,
    progressive_selection: f32,
    long_pass_profile: f32,
    long_pass_risk_tolerance: f32,
    press_resistance: f32,
}

impl MidfielderSkillReads {
    fn pass_game(
        &self,
        pressure_penalty: f32,
        passing_condition_mult: f32,
        first_touch_condition_mult: f32,
    ) -> MidfielderPassGame {
        // ── Pass execution ───────────────────────────────────────────
        let pass_execution = ((MidfielderSkillProfile::pow_curve(self.passing01, 1.45) * 0.30
            + MidfielderSkillProfile::pow_curve(self.technique01, 1.35) * 0.18
            + MidfielderSkillProfile::pow_curve(self.vision01, 1.45) * 0.16
            + MidfielderSkillProfile::pow_curve(self.decisions01, 1.40) * 0.14
            + MidfielderSkillProfile::pow_curve(self.composure01, 1.35) * 0.10
            + MidfielderSkillProfile::pow_curve(self.first_touch01, 1.25) * 0.06
            + MidfielderSkillProfile::pow_curve(self.concentration01, 1.20) * 0.06)
            * passing_condition_mult)
            .clamp(0.0, 1.0);
        // ── Progressive pass selection ───────────────────────────────
        let progressive_selection = (MidfielderSkillProfile::pow_curve(self.vision01, 1.55) * 0.28
            + MidfielderSkillProfile::pow_curve(self.decisions01, 1.45) * 0.24
            + MidfielderSkillProfile::pow_curve(self.passing01, 1.45) * 0.18
            + MidfielderSkillProfile::pow_curve(self.technique01, 1.30) * 0.10
            + MidfielderSkillProfile::pow_curve(self.composure01, 1.35) * 0.10
            + MidfielderSkillProfile::pow_curve(self.teamwork01, 1.20) * 0.06
            + MidfielderSkillProfile::pow_curve(self.flair01, 1.25) * 0.04)
            .clamp(0.0, 1.0);
        // ── Long pass / switch ───────────────────────────────────────
        let long_pass_profile = (MidfielderSkillProfile::pow_curve(self.passing01, 1.45) * 0.24
            + MidfielderSkillProfile::pow_curve(self.vision01, 1.55) * 0.24
            + MidfielderSkillProfile::pow_curve(self.technique01, 1.45) * 0.18
            + MidfielderSkillProfile::pow_curve(self.decisions01, 1.35) * 0.12
            + MidfielderSkillProfile::pow_curve(self.composure01, 1.25) * 0.08
            + MidfielderSkillProfile::pow_curve(self.strength01, 1.10) * 0.06
            + MidfielderSkillProfile::pow_curve(self.balance01, 1.15) * 0.04
            + MidfielderSkillProfile::pow_curve(self.concentration01, 1.15) * 0.04)
            .clamp(0.0, 1.0);

        let long_pass_risk_tolerance =
            (0.15 + long_pass_profile * 0.55 - pressure_penalty * 0.20).clamp(0.10, 0.78);
        // ── Press resistance ─────────────────────────────────────────
        let press_resistance = ((MidfielderSkillProfile::pow_curve(self.first_touch01, 1.40)
            * 0.22
            + MidfielderSkillProfile::pow_curve(self.technique01, 1.35) * 0.18
            + MidfielderSkillProfile::pow_curve(self.composure01, 1.45) * 0.18
            + MidfielderSkillProfile::pow_curve(self.balance01, 1.30) * 0.14
            + MidfielderSkillProfile::pow_curve(self.agility01, 1.25) * 0.10
            + MidfielderSkillProfile::pow_curve(self.decisions01, 1.35) * 0.10
            + MidfielderSkillProfile::pow_curve(self.strength01, 1.10) * 0.04
            + MidfielderSkillProfile::pow_curve(self.concentration01, 1.15) * 0.04)
            * first_touch_condition_mult)
            .clamp(0.0, 1.0);
        MidfielderPassGame {
            pass_execution,
            progressive_selection,
            long_pass_profile,
            long_pass_risk_tolerance,
            press_resistance,
        }
    }

    fn carry_and_shot(&self) -> (f32, f32) {
        // ── Carry / dribble selection ────────────────────────────────
        let carry_selection = (MidfielderSkillProfile::pow_curve(self.dribbling01, 1.55) * 0.24
            + MidfielderSkillProfile::pow_curve(self.decisions01, 1.40) * 0.18
            + MidfielderSkillProfile::pow_curve(self.composure01, 1.35) * 0.14
            + MidfielderSkillProfile::pow_curve(self.acceleration01, 1.25) * 0.12
            + MidfielderSkillProfile::pow_curve(self.agility01, 1.25) * 0.10
            + MidfielderSkillProfile::pow_curve(self.balance01, 1.25) * 0.10
            + MidfielderSkillProfile::pow_curve(self.technique01, 1.30) * 0.08
            + MidfielderSkillProfile::pow_curve(self.bravery01, 1.10) * 0.04)
            .clamp(0.0, 1.0);
        // ── Midfielder shot selection ────────────────────────────────
        // Real football: central midfielders take ~25-30% of a team's
        // shots (edge-of-box strikes, second-balls, late runs). The old
        // `long_shots^1.75 * 0.28` term punished the whole composite so
        // hard that a typical CM (long_shots ~10, finishing ~9) landed
        // ~0.30-0.39 and failed every `>= 0.40` shot gate — so 100% of a
        // team's shots (and goals) flowed through the two forwards. Soften
        // the exponents (a lower exponent on a 0..1 base RAISES the value)
        // and shift weight off raw long_shots toward the skills midfielders
        // actually have (technique / decisions / composure / finishing) so
        // a competent CM clears the standard-shot gate and contributes a
        // realistic share of attempts without flattering true long-range
        // specialists, who still separate via the top of the curve.
        let mid_shot_selection = (MidfielderSkillProfile::pow_curve(self.long_shots01, 1.45)
            * 0.23
            + MidfielderSkillProfile::pow_curve(self.technique01, 1.35) * 0.19
            + MidfielderSkillProfile::pow_curve(self.decisions01, 1.35) * 0.18
            + MidfielderSkillProfile::pow_curve(self.composure01, 1.35) * 0.17
            + MidfielderSkillProfile::pow_curve(self.finishing01, 1.45) * 0.13
            + MidfielderSkillProfile::pow_curve(self.balance01, 1.20) * 0.06
            + MidfielderSkillProfile::pow_curve(self.concentration01, 1.15) * 0.04)
            .clamp(0.0, 1.0);
        (carry_selection, mid_shot_selection)
    }
}

#[derive(Debug, Clone, Copy)]
struct MidfielderDefending {
    pressing_profile: f32,
    tackle_profile: f32,
    discipline: f32,
}

impl MidfielderSkillReads {
    fn defending(
        &self,
        pressing_condition_mult: f32,
        tackling_condition_mult: f32,
    ) -> MidfielderDefending {
        // ── Pressing ─────────────────────────────────────────────────
        let pressing_profile = ((MidfielderSkillProfile::pow_curve(self.work_rate01, 1.35) * 0.22
            + MidfielderSkillProfile::pow_curve(self.stamina01, 1.25) * 0.18
            + MidfielderSkillProfile::pow_curve(self.anticipation01, 1.35) * 0.16
            + MidfielderSkillProfile::pow_curve(self.acceleration01, 1.25) * 0.12
            + MidfielderSkillProfile::pow_curve(self.positioning01, 1.30) * 0.10
            + MidfielderSkillProfile::pow_curve(self.decisions01, 1.30) * 0.10
            + MidfielderSkillProfile::pow_curve(self.aggression01, 1.10) * 0.06
            + MidfielderSkillProfile::pow_curve(self.teamwork01, 1.15) * 0.06)
            * pressing_condition_mult)
            .clamp(0.0, 1.0);
        // ── Tackling ─────────────────────────────────────────────────
        let tackle_profile = ((MidfielderSkillProfile::pow_curve(self.tackling01, 1.55) * 0.28
            + MidfielderSkillProfile::pow_curve(self.positioning01, 1.35) * 0.16
            + MidfielderSkillProfile::pow_curve(self.anticipation01, 1.35) * 0.14
            + MidfielderSkillProfile::pow_curve(self.composure01, 1.35) * 0.12
            + MidfielderSkillProfile::pow_curve(self.balance01, 1.25) * 0.10
            + MidfielderSkillProfile::pow_curve(self.agility01, 1.20) * 0.08
            + MidfielderSkillProfile::pow_curve(self.strength01, 1.15) * 0.06
            + MidfielderSkillProfile::pow_curve(self.concentration01, 1.20) * 0.06)
            * tackling_condition_mult)
            .clamp(0.0, 1.0);
        // ── Discipline ───────────────────────────────────────────────
        // No `temperament` attribute on the player — fold it into a
        // composure/concentration blend so the discipline curve still
        // covers the full intended weight.
        let aggression_inverse = 1.0 - self.aggression01;
        let temperament_proxy = (self.composure01 + self.concentration01) * 0.5;
        let discipline = (MidfielderSkillProfile::pow_curve(self.composure01, 1.35) * 0.26
            + MidfielderSkillProfile::pow_curve(self.decisions01, 1.35) * 0.22
            + MidfielderSkillProfile::pow_curve(self.tackling01, 1.30) * 0.16
            + MidfielderSkillProfile::pow_curve(self.concentration01, 1.20) * 0.14
            + temperament_proxy * 0.12
            + aggression_inverse * 0.10)
            .clamp(0.0, 1.0);
        MidfielderDefending {
            pressing_profile,
            tackle_profile,
            discipline,
        }
    }

    fn support(&self, off_ball_condition_mult: f32) -> f32 {
        // ── Off-ball support ─────────────────────────────────────────
        let support_profile = ((MidfielderSkillProfile::pow_curve(self.off_ball01, 1.45) * 0.22
            + MidfielderSkillProfile::pow_curve(self.anticipation01, 1.35) * 0.18
            + MidfielderSkillProfile::pow_curve(self.decisions01, 1.35) * 0.18
            + MidfielderSkillProfile::pow_curve(self.teamwork01, 1.25) * 0.12
            + MidfielderSkillProfile::pow_curve(self.stamina01, 1.20) * 0.10
            + MidfielderSkillProfile::pow_curve(self.acceleration01, 1.20) * 0.08
            + MidfielderSkillProfile::pow_curve(self.composure01, 1.20) * 0.06
            + MidfielderSkillProfile::pow_curve(self.concentration01, 1.15) * 0.06)
            * off_ball_condition_mult)
            .clamp(0.0, 1.0);
        support_profile
    }
}

impl MidfielderSkillProfile {
    /// Build the profile from a state processing context.
    ///
    /// Memoized per (player, tick) — see `DefenderSkillProfile::from_ctx`
    /// for the rationale; all inputs are tick-frozen so the memo is
    /// bit-identical, and the debug oracle recomputes on every hit.
    pub fn from_ctx(ctx: &StateProcessingContext) -> Self {
        let tick = ctx.current_tick();
        let cached = ctx
            .tick_context
            .player_agg_cache
            .borrow_mut()
            .slot_mut(ctx.player.id, tick)
            .midfielder_profile;
        if let Some(profile) = cached {
            debug_assert!(
                profile == Self::compute_from_ctx(ctx),
                "midfielder-profile memo mismatch"
            );
            return profile;
        }
        let profile = Self::compute_from_ctx(ctx);
        ctx.tick_context
            .player_agg_cache
            .borrow_mut()
            .slot_mut(ctx.player.id, tick)
            .midfielder_profile = Some(profile);
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
        Self::from_player_memo(ctx, &inputs)
    }

    /// Cross-tick memo key — see `DefenderSkillProfile::memo_key`. The
    /// distance / ownership / recent-intensity inputs are declared but
    /// unused by the profile body (see the `let _ = (...)` marker), so
    /// only condition, jadedness, minute and the exact pressure counts
    /// participate.
    #[inline]
    fn memo_key(player: &MatchPlayer, inputs: &MidfielderSkillInputs) -> u64 {
        (player.player_attributes.condition as u16 as u64)
            | (player.player_attributes.jadedness as u16 as u64) << 16
            | (inputs.minute as u64 & 0xFF) << 32
            | (inputs.pressure_count_5u as u64 & 0xFF) << 40
            | (inputs.pressure_count_10u as u64 & 0xFF) << 48
            | player.injury_rank() << 57
    }

    /// Cross-tick memoized `from_player` — bit-identical between key
    /// changes; see `DefenderSkillProfile::from_player_memo`.
    fn from_player_memo(ctx: &StateProcessingContext, inputs: &MidfielderSkillInputs) -> Self {
        let player = ctx.player;
        let key = Self::memo_key(player, inputs);
        let cached = ctx
            .tick_context
            .profile_memos
            .borrow()
            .midfielder_get(player.id, key);
        if let Some(profile) = cached {
            debug_assert!(
                profile == Self::from_player(player, inputs),
                "midfielder-profile cross-tick memo mismatch"
            );
            return profile;
        }
        let profile = Self::from_player(player, inputs);
        ctx.tick_context
            .profile_memos
            .borrow_mut()
            .midfielder_put(player.id, key, profile);
        profile
    }

    pub fn from_player(player: &MatchPlayer, inputs: &MidfielderSkillInputs) -> Self {
        let reads = MidfielderSkillReads::of(player, inputs.minute);

        let core01 = reads.core01();
        let poor_penalty = Self::smoothstep(0.45, 0.18, core01);
        let elite_lift = Self::smoothstep(0.72, 0.95, core01);

        let conditioning = MidfielderConditioning::of(player, inputs, &reads, poor_penalty);

        let pressure_penalty = (inputs.pressure_count_5u as f32 * 0.20
            + inputs.pressure_count_10u as f32 * 0.07)
            .clamp(0.0, 1.0);

        let pass_game = reads.pass_game(
            pressure_penalty,
            conditioning.passing_condition_mult,
            conditioning.first_touch_condition_mult,
        );
        let (carry_selection, mid_shot_selection) = reads.carry_and_shot();
        let defending = reads.defending(
            conditioning.pressing_condition_mult,
            conditioning.tackling_condition_mult,
        );
        let support_profile = reads.support(conditioning.off_ball_condition_mult);

        let _ = (
            inputs.distance_to_opponent_goal,
            inputs.distance_to_own_goal,
            inputs.ownership_ticks,
            inputs.recent_sprint_or_high_intensity,
        );

        MidfielderSkillProfile {
            poor_penalty,
            elite_lift,
            mid_condition_mult: conditioning.mid_condition_mult,
            passing_condition_mult: conditioning.passing_condition_mult,
            first_touch_condition_mult: conditioning.first_touch_condition_mult,
            pressing_condition_mult: conditioning.pressing_condition_mult,
            tackling_condition_mult: conditioning.tackling_condition_mult,
            off_ball_condition_mult: conditioning.off_ball_condition_mult,
            pass_execution: pass_game.pass_execution,
            progressive_selection: pass_game.progressive_selection,
            long_pass_profile: pass_game.long_pass_profile,
            long_pass_risk_tolerance: pass_game.long_pass_risk_tolerance,
            press_resistance: pass_game.press_resistance,
            carry_selection,
            mid_shot_selection,
            pressing_profile: defending.pressing_profile,
            tackle_profile: defending.tackle_profile,
            discipline: defending.discipline,
            support_profile,
            pressure_penalty,
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

    // `allows_carry_into_space` (>= 0.32), `allows_take_on_one` (0.40)
    // and `allows_take_on_two` (0.58) used to live here. Three booleans
    // on one composite, and `carry_selection` is a weighted mean of
    // eight curved skills, so it clusters: a uniform 10/20 midfielder
    // scores 0.39 and a uniform 12/20 scores 0.50. The league-average
    // central midfielder was therefore on the wrong side of the take-on
    // line by a hundredth and never attempted one, while a slightly
    // better one attempted every single take-on the geometry allowed.
    // The decision is now continuous and situational — see
    // `midfielders::states::shared::carry::TakeOn`, which reads
    // `carry_selection` as one input among room, traffic, territory and
    // temperament rather than as a verdict.

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
        let attrs = PlayerAttributes {
            condition,
            jadedness: 0,
            ..Default::default()
        };
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
        MatchPlayer::from_player(
            1,
            &player,
            PlayerPositionType::MidfielderCenter,
            false,
            None,
        )
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
    fn carry_selection_separates_the_population() {
        // The take-on used to be three thresholds on this number; it is
        // now a continuous input to `TakeOn::appetite`. What still has
        // to hold is that it ORDERS players and spreads them over a
        // usable range — a composite that lands everyone within a
        // hundredth of each other cannot drive a decision either way.
        let poor = MidfielderSkillProfile::from_player(&build_player(5.0, 9000), &default_inputs());
        let mid = MidfielderSkillProfile::from_player(&build_player(11.0, 9000), &default_inputs());
        let elite =
            MidfielderSkillProfile::from_player(&build_player(18.0, 9000), &default_inputs());
        assert!(poor.carry_selection < mid.carry_selection);
        assert!(mid.carry_selection < elite.carry_selection);
        assert!(
            elite.carry_selection - poor.carry_selection > 0.35,
            "poor={} elite={}",
            poor.carry_selection,
            elite.carry_selection
        );
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
