use crate::r#match::MAX_OWNER_TRACK_DISTANCE;
use crate::r#match::MatchPlayer;
use crate::r#match::StateProcessingContext;
use crate::r#match::player::strategies::players::ops::effective_skill::{
    SkillBands, SkillCategory,
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
#[derive(Debug, Clone, Copy, PartialEq)]
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
    /// **How much of the ground in front of his goal he owns.**
    ///
    /// `command_of_area` is the attribute whose whole meaning is the size
    /// of a keeper's territory, and until this field existed every site
    /// that wanted it read `skills.goalkeeping.command_of_area / 20.0`
    /// RAW — no fatigue band — while every other keeper read in the game
    /// went through one. A keeper's willingness to own his box was the
    /// one thing about him that never tired.
    ///
    /// Deliberately NOT put through `keeper_curve`. The curves are convex
    /// because EXECUTION is convex — a slightly quicker keeper saves a lot
    /// more shots. This is a tendency, like [`Self::eccentricity`]: it
    /// says how much space he claims, not how well he covers it, and a
    /// tendency is linear in the attribute. Curving it would also silently
    /// move every depth the rest model gives him.
    pub command_of_area: f32,
    /// Sweeper / coming-out execution.
    pub rushing_out_profile: f32,
    /// Risk appetite (0..1) from `goalkeeping.eccentricity` — a
    /// TENDENCY, not a quality: it widens how speculative a race the
    /// keeper is willing to join and how far he'll sweep, without
    /// making him any better at arriving. Raw read (no fatigue band):
    /// temperament doesn't tire.
    pub eccentricity: f32,
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

#[derive(Debug, Clone, Copy)]
struct GoalkeeperSkillReads {
    determination01: f32,
    nat_fitness01: f32,
    match_readiness01: f32,
    reflexes01: f32,
    handling01: f32,
    one_on_ones01: f32,
    aerial_reach01: f32,
    punching01: f32,
    kicking01: f32,
    throwing01: f32,
    gk_passing01: f32,
    gk_first_touch01: f32,
    rushing_out_raw01: f32,
    command_of_area01: f32,
    communication01: f32,
    positioning01: f32,
    anticipation01: f32,
    concentration01: f32,
    decisions01: f32,
    composure01: f32,
    bravery01: f32,
    teamwork01: f32,
    vision01: f32,
    agility01: f32,
    acceleration01: f32,
    pace01: f32,
    jumping01: f32,
    strength01: f32,
    balance01: f32,
    stamina01: f32,
    eccentricity01: f32,
}

impl GoalkeeperSkillReads {
    fn of(player: &MatchPlayer, minute: u32) -> Self {
        let bands = SkillBands::for_player(player, minute);
        let s = &player.skills;

        let reflexes_eff = bands.apply(s.goalkeeping.reflexes, SkillCategory::Technical);
        let handling_eff = bands.apply(s.goalkeeping.handling, SkillCategory::Technical);
        let one_on_ones_eff = bands.apply(s.goalkeeping.one_on_ones, SkillCategory::Technical);
        let aerial_reach_eff = bands.apply(s.goalkeeping.aerial_reach, SkillCategory::Technical);
        let punching_eff = bands.apply(s.goalkeeping.punching, SkillCategory::Technical);
        let kicking_eff = bands.apply(s.goalkeeping.kicking, SkillCategory::Technical);
        let throwing_eff = bands.apply(s.goalkeeping.throwing, SkillCategory::Technical);
        let gk_passing_eff = bands.apply(s.goalkeeping.passing, SkillCategory::Technical);
        let gk_first_touch_eff = bands.apply(s.goalkeeping.first_touch, SkillCategory::Technical);
        let rushing_out_eff = bands.apply(s.goalkeeping.rushing_out, SkillCategory::Technical);
        let command_of_area_eff = bands.apply(s.goalkeeping.command_of_area, SkillCategory::Mental);
        let communication_eff = bands.apply(s.goalkeeping.communication, SkillCategory::Mental);

        // ── Mental reads ─────────────────────────────────────────────
        let positioning_eff = bands.apply(s.mental.positioning, SkillCategory::Mental);
        let anticipation_eff = bands.apply(s.mental.anticipation, SkillCategory::Mental);
        let concentration_eff = bands.apply(s.mental.concentration, SkillCategory::Mental);
        let decisions_eff = bands.apply(s.mental.decisions, SkillCategory::Mental);
        let composure_eff = bands.apply(s.mental.composure, SkillCategory::Mental);
        let bravery_eff = bands.apply(s.mental.bravery, SkillCategory::Mental);
        let teamwork_eff = bands.apply(s.mental.teamwork, SkillCategory::Mental);
        let vision_eff = bands.apply(s.mental.vision, SkillCategory::Mental);

        // ── Physical reads ───────────────────────────────────────────
        let agility_eff = bands.apply(s.physical.agility, SkillCategory::Explosive);
        let acceleration_eff = bands.apply(s.physical.acceleration, SkillCategory::Explosive);
        let pace_eff = bands.apply(s.physical.pace, SkillCategory::Explosive);
        let jumping_eff = bands.apply(s.physical.jumping, SkillCategory::Explosive);
        let strength_eff = bands.apply(s.physical.strength, SkillCategory::Explosive);
        let balance_eff = bands.apply(s.physical.balance, SkillCategory::Technical);
        let stamina_eff = bands.apply(s.physical.stamina, SkillCategory::Explosive);

        // ── Normalised reads (0..1 via skill01) ──────────────────────
        //
        // The composites below are CONVEX in skill, so read against the
        // whole game the keeper improves faster than the shooters he
        // faces as the pyramid rises: saves per shot on target ran 56.6%
        // at level 4 to 70.7% at 18 against a real ~68% everywhere. The
        // squads are measured against the standard of the match at
        // kickoff (`MatchStandard::measure`), so the curves describe the
        // spread within a division.

        Self {
            reflexes01: GoalkeeperSkillProfile::skill01(reflexes_eff),
            handling01: GoalkeeperSkillProfile::skill01(handling_eff),
            one_on_ones01: GoalkeeperSkillProfile::skill01(one_on_ones_eff),
            aerial_reach01: GoalkeeperSkillProfile::skill01(aerial_reach_eff),
            punching01: GoalkeeperSkillProfile::skill01(punching_eff),
            kicking01: GoalkeeperSkillProfile::skill01(kicking_eff),
            throwing01: GoalkeeperSkillProfile::skill01(throwing_eff),
            gk_passing01: GoalkeeperSkillProfile::skill01(gk_passing_eff),
            gk_first_touch01: GoalkeeperSkillProfile::skill01(gk_first_touch_eff),
            rushing_out_raw01: GoalkeeperSkillProfile::skill01(rushing_out_eff),
            command_of_area01: GoalkeeperSkillProfile::skill01(command_of_area_eff),
            communication01: GoalkeeperSkillProfile::skill01(communication_eff),
            positioning01: GoalkeeperSkillProfile::skill01(positioning_eff),
            anticipation01: GoalkeeperSkillProfile::skill01(anticipation_eff),
            concentration01: GoalkeeperSkillProfile::skill01(concentration_eff),
            decisions01: GoalkeeperSkillProfile::skill01(decisions_eff),
            composure01: GoalkeeperSkillProfile::skill01(composure_eff),
            bravery01: GoalkeeperSkillProfile::skill01(bravery_eff),
            teamwork01: GoalkeeperSkillProfile::skill01(teamwork_eff),
            vision01: GoalkeeperSkillProfile::skill01(vision_eff),
            agility01: GoalkeeperSkillProfile::skill01(agility_eff),
            acceleration01: GoalkeeperSkillProfile::skill01(acceleration_eff),
            pace01: GoalkeeperSkillProfile::skill01(pace_eff),
            jumping01: GoalkeeperSkillProfile::skill01(jumping_eff),
            strength01: GoalkeeperSkillProfile::skill01(strength_eff),
            balance01: GoalkeeperSkillProfile::skill01(balance_eff),
            stamina01: GoalkeeperSkillProfile::skill01(stamina_eff),
            determination01: GoalkeeperSkillProfile::skill01(
                bands.apply(s.mental.determination, SkillCategory::Mental),
            ),
            nat_fitness01: GoalkeeperSkillProfile::skill01(s.physical.natural_fitness),
            match_readiness01: GoalkeeperSkillProfile::skill01(s.physical.match_readiness),
            eccentricity01: GoalkeeperSkillProfile::skill01(s.goalkeeping.eccentricity),
        }
    }

    fn headline(&self) -> (f32, f32) {
        let primary = (GoalkeeperSkillProfile::reaction_curve(self.reflexes01) * 0.30
            + GoalkeeperSkillProfile::handling_curve(self.handling01) * 0.20
            + GoalkeeperSkillProfile::reaction_curve(self.one_on_ones01) * 0.15
            + GoalkeeperSkillProfile::keeper_curve(self.positioning01) * 0.12
            + GoalkeeperSkillProfile::keeper_curve(self.anticipation01) * 0.10
            + GoalkeeperSkillProfile::keeper_curve(self.concentration01) * 0.08
            + GoalkeeperSkillProfile::keeper_curve(self.command_of_area01) * 0.05)
            .clamp(0.0, 1.0);
        let poor_skill_penalty = GoalkeeperSkillProfile::smoothstep(0.45, 0.18, primary);
        let elite_lift = GoalkeeperSkillProfile::smoothstep(0.72, 0.95, primary) * 0.08;
        (poor_skill_penalty, elite_lift)
    }
}

#[derive(Debug, Clone, Copy)]
struct GoalkeeperConditioning {
    condition_mult: f32,
    explosive_mult: f32,
    handling_mult: f32,
    decision_mult: f32,
}

impl GoalkeeperConditioning {
    fn of(
        inputs: &GoalkeeperSkillInputs,
        reads: &GoalkeeperSkillReads,
        player: &MatchPlayer,
    ) -> Self {
        let condition = inputs.condition_pct.clamp(0.0, 1.0);
        let fatigue = (1.0 - condition).max(0.0).powf(1.25);
        let jaded = (player.player_attributes.jadedness as f32 / 10_000.0).clamp(0.0, 1.0);

        let fitness_base = reads.stamina01 * 0.30
            + reads.nat_fitness01 * 0.25
            + reads.match_readiness01 * 0.20
            + reads.concentration01 * 0.15
            + reads.determination01 * 0.10;

        let condition_mult = (1.03 - fatigue * (0.34 + (1.0 - fitness_base) * 0.24) - jaded * 0.18)
            .clamp(0.52, 1.03);

        let explosive_mult = (condition_mult - fatigue * 0.10).clamp(0.45, 1.02);
        let handling_mult = (condition_mult - fatigue * 0.07 - jaded * 0.06).clamp(0.50, 1.02);
        let decision_mult = (condition_mult - fatigue * 0.08 - jaded * 0.10).clamp(0.48, 1.02);

        GoalkeeperConditioning {
            condition_mult,
            explosive_mult,
            handling_mult,
            decision_mult,
        }
    }
}

impl GoalkeeperSkillReads {
    fn stopping_and_reach(
        &self,
        poor_skill_penalty: f32,
        elite_lift: f32,
        condition_mult: f32,
        explosive_mult: f32,
    ) -> (f32, f32, f32) {
        let shot_stopping_raw = GoalkeeperSkillProfile::reaction_curve(self.reflexes01) * 0.28
            + GoalkeeperSkillProfile::reaction_curve(self.one_on_ones01) * 0.18
            + GoalkeeperSkillProfile::keeper_curve(self.agility01) * 0.16
            + GoalkeeperSkillProfile::keeper_curve(self.anticipation01) * 0.10
            + GoalkeeperSkillProfile::keeper_curve(self.positioning01) * 0.10
            + GoalkeeperSkillProfile::handling_curve(self.handling01) * 0.08
            + GoalkeeperSkillProfile::keeper_curve(self.concentration01) * 0.06
            + GoalkeeperSkillProfile::keeper_curve(self.composure01) * 0.04;
        let shot_stopping = (shot_stopping_raw * condition_mult - poor_skill_penalty * 0.10
            + elite_lift)
            .clamp(0.0, 1.0);

        let positioning_profile = (GoalkeeperSkillProfile::keeper_curve(self.positioning01) * 0.22
            + GoalkeeperSkillProfile::keeper_curve(self.anticipation01) * 0.18
            + GoalkeeperSkillProfile::keeper_curve(self.decisions01) * 0.16
            + GoalkeeperSkillProfile::keeper_curve(self.concentration01) * 0.14
            + GoalkeeperSkillProfile::keeper_curve(self.command_of_area01) * 0.10
            + GoalkeeperSkillProfile::keeper_curve(self.communication01) * 0.08
            + GoalkeeperSkillProfile::keeper_curve(self.teamwork01) * 0.07
            + GoalkeeperSkillProfile::keeper_curve(self.composure01) * 0.05)
            .clamp(0.0, 1.0);

        let dive_reach_raw = GoalkeeperSkillProfile::keeper_curve(self.agility01) * 0.24
            + GoalkeeperSkillProfile::keeper_curve(self.acceleration01) * 0.20
            + GoalkeeperSkillProfile::keeper_curve(self.jumping01) * 0.18
            + GoalkeeperSkillProfile::reaction_curve(self.reflexes01) * 0.16
            + GoalkeeperSkillProfile::keeper_curve(self.balance01) * 0.12
            + GoalkeeperSkillProfile::keeper_curve(self.strength01) * 0.10;
        let dive_reach = (dive_reach_raw * explosive_mult).clamp(0.0, 1.0);
        (shot_stopping, positioning_profile, dive_reach)
    }

    fn handling_and_aerial(
        &self,
        explosive_mult: f32,
        handling_mult: f32,
    ) -> (f32, f32, f32, f32, f32) {
        let handling_raw = GoalkeeperSkillProfile::handling_curve(self.handling01) * 0.34
            + GoalkeeperSkillProfile::reaction_curve(self.reflexes01) * 0.18
            + GoalkeeperSkillProfile::keeper_curve(self.concentration01) * 0.14
            + GoalkeeperSkillProfile::keeper_curve(self.composure01) * 0.12
            + GoalkeeperSkillProfile::keeper_curve(self.agility01) * 0.10
            + GoalkeeperSkillProfile::keeper_curve(self.command_of_area01) * 0.07
            + GoalkeeperSkillProfile::keeper_curve(self.balance01) * 0.05;
        let handling_profile = (handling_raw * handling_mult).clamp(0.0, 1.0);

        let parry_control = (GoalkeeperSkillProfile::handling_curve(self.handling01) * 0.24
            + GoalkeeperSkillProfile::keeper_curve(self.punching01) * 0.18
            + GoalkeeperSkillProfile::keeper_curve(self.strength01) * 0.16
            + GoalkeeperSkillProfile::reaction_curve(self.reflexes01) * 0.14
            + GoalkeeperSkillProfile::keeper_curve(self.decisions01) * 0.12
            + GoalkeeperSkillProfile::keeper_curve(self.composure01) * 0.10
            + GoalkeeperSkillProfile::keeper_curve(self.balance01) * 0.06)
            .clamp(0.0, 1.0);

        let aerial_command_raw = GoalkeeperSkillProfile::keeper_curve(self.aerial_reach01) * 0.22
            + GoalkeeperSkillProfile::keeper_curve(self.jumping01) * 0.18
            + GoalkeeperSkillProfile::handling_curve(self.handling01) * 0.15
            + GoalkeeperSkillProfile::keeper_curve(self.command_of_area01) * 0.15
            + GoalkeeperSkillProfile::keeper_curve(self.bravery01) * 0.10
            + GoalkeeperSkillProfile::keeper_curve(self.strength01) * 0.08
            + GoalkeeperSkillProfile::keeper_curve(self.anticipation01) * 0.07
            + GoalkeeperSkillProfile::keeper_curve(self.communication01) * 0.05;
        let aerial_command = (aerial_command_raw * explosive_mult).clamp(0.0, 1.0);

        let rushing_out_raw = GoalkeeperSkillProfile::keeper_curve(self.rushing_out_raw01) * 0.24
            + GoalkeeperSkillProfile::keeper_curve(self.acceleration01) * 0.18
            + GoalkeeperSkillProfile::keeper_curve(self.pace01) * 0.12
            + GoalkeeperSkillProfile::keeper_curve(self.decisions01) * 0.14
            + GoalkeeperSkillProfile::keeper_curve(self.anticipation01) * 0.12
            + GoalkeeperSkillProfile::keeper_curve(self.composure01) * 0.08
            + GoalkeeperSkillProfile::keeper_curve(self.one_on_ones01) * 0.08
            + GoalkeeperSkillProfile::keeper_curve(self.bravery01) * 0.04;
        let rushing_out_profile = (rushing_out_raw * explosive_mult).clamp(0.0, 1.0);

        let one_v_one = (GoalkeeperSkillProfile::reaction_curve(self.one_on_ones01) * 0.26
            + GoalkeeperSkillProfile::reaction_curve(self.reflexes01) * 0.18
            + GoalkeeperSkillProfile::keeper_curve(self.composure01) * 0.16
            + GoalkeeperSkillProfile::keeper_curve(self.agility01) * 0.14
            + GoalkeeperSkillProfile::keeper_curve(self.decisions01) * 0.12
            + GoalkeeperSkillProfile::keeper_curve(self.bravery01) * 0.08
            + GoalkeeperSkillProfile::keeper_curve(self.positioning01) * 0.06)
            .clamp(0.0, 1.0);
        (
            handling_profile,
            parry_control,
            aerial_command,
            rushing_out_profile,
            one_v_one,
        )
    }

    fn distribution_and_command(&self, decision_mult: f32) -> (f32, f32, f32, f32) {
        let distribution_raw = GoalkeeperSkillProfile::keeper_curve(self.kicking01) * 0.22
            + GoalkeeperSkillProfile::keeper_curve(self.throwing01) * 0.18
            + GoalkeeperSkillProfile::keeper_curve(self.gk_passing01) * 0.16
            + GoalkeeperSkillProfile::keeper_curve(self.gk_first_touch01) * 0.10
            + GoalkeeperSkillProfile::keeper_curve(self.decisions01) * 0.14
            + GoalkeeperSkillProfile::keeper_curve(self.vision01) * 0.08
            + GoalkeeperSkillProfile::keeper_curve(self.composure01) * 0.08
            + GoalkeeperSkillProfile::keeper_curve(self.teamwork01) * 0.04;
        let distribution = (distribution_raw * decision_mult).clamp(0.0, 1.0);

        let communication = (GoalkeeperSkillProfile::keeper_curve(self.communication01) * 0.45
            + GoalkeeperSkillProfile::keeper_curve(self.command_of_area01) * 0.30
            + GoalkeeperSkillProfile::keeper_curve(self.concentration01) * 0.15
            + GoalkeeperSkillProfile::keeper_curve(self.positioning01) * 0.10)
            .clamp(0.0, 1.0);

        // How much of the space in front of his goal he claims as his.
        // Linear in the attribute — see the field note. `communication`
        // and `bravery` are here because owning a box is shouting a back
        // four into place and going through a crowd to punch, and
        // `decisions` / `anticipation` because the keeper who comes for
        // everything and the one who comes for nothing are both wrong.
        let command_of_area = ((self.command_of_area01 * 0.40
            + self.communication01 * 0.15
            + self.bravery01 * 0.15
            + self.decisions01 * 0.15
            + self.anticipation01 * 0.15)
            * decision_mult)
            .clamp(0.0, 1.0);

        let concentration =
            GoalkeeperSkillProfile::keeper_curve(self.concentration01).clamp(0.0, 1.0);

        (distribution, communication, command_of_area, concentration)
    }
}

impl GoalkeeperSkillProfile {
    /// Where [`Self::positioning`] sits for the population.
    ///
    /// The composite is a weighted blend of eight attributes put through
    /// `keeper_curve`, so it does NOT centre on 0.5 — measured over 60
    /// matches at L14 (`KEEPER GUARD CENSUS`, printed by `dev_match
    /// stats`) it comes out at **0.479**.
    ///
    /// Any term that multiplies a CALIBRATED quantity by keeper quality
    /// has to be centred here rather than on 0.5, or it does not add a
    /// skill axis — it silently re-levels the whole model by the
    /// difference, which is exactly the trap `SaveModel::strike_power`
    /// documents. Re-derive it from the census if the composite's weights
    /// ever move.
    pub const POPULATION_READ: f32 = 0.479;

    /// Memoized per (player, tick, vitals) in front of [`Self::from_memo`],
    /// the same two layers `DefenderSkillProfile::from_ctx` keeps.
    pub fn from_ctx(ctx: &StateProcessingContext) -> Self {
        let tick = ctx.current_tick();
        let vitals = (ctx.player.player_attributes.condition as u16 as u32)
            | (ctx.player.player_attributes.jadedness as u16 as u32) << 16;
        let cached = ctx
            .tick_context
            .player_agg_cache
            .borrow_mut()
            .slot_mut(ctx.player.id, tick)
            .goalkeeper_profile
            .filter(|(cached_vitals, _)| *cached_vitals == vitals)
            .map(|(_, profile)| profile);
        if let Some(profile) = cached {
            debug_assert!(
                profile == Self::from_memo(ctx),
                "goalkeeper-profile tick memo mismatch: player={} tick={}",
                ctx.player.id,
                tick
            );
            return profile;
        }
        let profile = Self::from_memo(ctx);
        ctx.tick_context
            .player_agg_cache
            .borrow_mut()
            .slot_mut(ctx.player.id, tick)
            .goalkeeper_profile = Some((vitals, profile));
        profile
    }

    /// Cross-tick memoized per (condition, jadedness, minute) — see
    /// `DefenderSkillProfile::from_player_memo` for the pattern. The GK
    /// profile's only in-match-varying inputs are those three integers
    /// (everything else it reads is static skills / crowd arousal), so
    /// the cached copy is bit-identical between key changes; the debug
    /// oracle recomputes on every hit.
    fn from_memo(ctx: &StateProcessingContext) -> Self {
        let player = ctx.player;
        let minute = sc::minute_from_ms(ctx.context.total_match_time);
        let condition_pct = (player.player_attributes.condition as f32 / 10_000.0).clamp(0.0, 1.0);
        let inputs = GoalkeeperSkillInputs {
            minute,
            condition_pct,
        };

        let key = (player.player_attributes.condition as u16 as u64)
            | (player.player_attributes.jadedness as u16 as u64) << 16
            | (minute as u64 & 0xFF) << 32
            | player.injury_rank() << 40;
        let cached = ctx
            .tick_context
            .profile_memos
            .borrow()
            .goalkeeper_get(player.id, key);
        if let Some(profile) = cached {
            debug_assert!(
                profile == Self::from_player(player, &inputs),
                "goalkeeper-profile cross-tick memo mismatch"
            );
            return profile;
        }
        let profile = Self::from_player(player, &inputs);
        ctx.tick_context
            .profile_memos
            .borrow_mut()
            .goalkeeper_put(player.id, key, profile);
        profile
    }

    pub fn from_player(player: &MatchPlayer, inputs: &GoalkeeperSkillInputs) -> Self {
        let reads = GoalkeeperSkillReads::of(player, inputs.minute);
        let (poor_skill_penalty, elite_lift) = reads.headline();
        let conditioning = GoalkeeperConditioning::of(inputs, &reads, player);

        let (shot_stopping, positioning_profile, dive_reach) = reads.stopping_and_reach(
            poor_skill_penalty,
            elite_lift,
            conditioning.condition_mult,
            conditioning.explosive_mult,
        );
        let (handling_profile, parry_control, aerial_command, rushing_out_profile, one_v_one) =
            reads.handling_and_aerial(conditioning.explosive_mult, conditioning.handling_mult);
        let (distribution, communication, command_of_area, concentration) =
            reads.distribution_and_command(conditioning.decision_mult);

        let eccentricity = reads.eccentricity01;
        let condition_mult = conditioning.condition_mult;
        let explosive_mult = conditioning.explosive_mult;
        let handling_mult = conditioning.handling_mult;
        let decision_mult = conditioning.decision_mult;

        // Effective ranges in game units. The legacy code used fixed
        // 35..40u catch / dive distances regardless of skill; here, weak
        // keepers shrink to ~14..16u while elite ones extend to ~42..48u.
        let effective_dive_distance = 14.0 + dive_reach * 28.0 + positioning_profile * 6.0;
        // ⚠ A GATHER RADIUS, NOT A TRAVEL RADIUS — it is bounded by
        // `MAX_OWNER_TRACK_DISTANCE`, and that is a hard constraint, not a
        // taste call.
        //
        // How far the keeper will GO for a ball is `effective_dive_distance`
        // above (up to 48u / 6 m). This is the much smaller question of when
        // the ball is close enough to be **in his hands** — and the engine
        // already has an answer for that: `MAX_OWNER_TRACK_DISTANCE` (15u /
        // 1.9 m) is the furthest a ball can be from the player who owns it.
        //
        // This used to read `10.0 + handling * 16.0 + positioning * 5.0`, so
        // a good keeper gathered at up to **31u (3.9 m)** — past the cutoff,
        // and past any real arms-out reach. `Ball::move_to` then found the
        // owner too far and dropped the ownership on the NEXT tick, by which
        // time the catch handler had already zeroed the velocity: the ball
        // was left sitting dead on the grass metres from anyone, then played
        // away backwards by whoever converged on it. Reported from the
        // viewer as exactly that, and counted by
        // `reception_diag::OWNER_TOO_FAR` at 87/match.
        //
        // 8..15u is a keeper's real gather: ~1.0 m for a poor handler taking
        // it into his body, ~1.9 m stretching. The skill axis survives; it
        // now spans a range a man can actually reach. Below the cutoff at
        // every skill level, so the ball's own 1.5 u/tick tracking pulls it
        // into his chest instead of a teleport.
        let effective_catch_distance = (8.0 + handling_profile * 5.0 + positioning_profile * 2.0)
            .min(MAX_OWNER_TRACK_DISTANCE);
        let effective_punch_distance = 8.0 + aerial_command * 14.0 + parry_control * 6.0;

        GoalkeeperSkillProfile {
            shot_stopping,
            positioning: positioning_profile,
            dive_reach,
            handling_profile,
            parry_control,
            aerial_command,
            command_of_area,
            rushing_out_profile,
            eccentricity,
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
        // Sigmoid steepness 2.40 → 1.80 → 1.40 and floor 0.03 → 0.10 →
        // 0.12: keeps the equal-skill calibration intact (sigmoid(0)
        // = 0.50 either way) while further compressing the strong-vs-
        // weak skill swing. Real PL data: the worst-performing keeper
        // still posts ~60% save rate across the season because most
        // shots are speculative; the 0.03 floor allowed our weak keeper
        // to drop to ~25% per shot in the extreme-mismatch case, which
        // is below any real keeper. The 0.12 floor reflects that even
        // a Sunday-League keeper saves routine balls — the
        // differentiator is corner-of-the-net shots, which are handled
        // by the upper end of the curve.
        let raw = Self::sigmoid((keeper_save_power - shot_difficulty) * 1.40);
        let mut save_prob = raw.clamp(0.12, 0.86);
        // Poor-skill flat penalty trimmed 0.14 → 0.08 because it compounded
        // with the sigmoid-driven gap above to drag weak-GK save rate well
        // below the realistic ~40% floor. Strong GKs (poor_skill_penalty = 0)
        // are unaffected.
        save_prob -= self.poor_skill_penalty * 0.08;
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
        // Sigmoid 2.40 → 1.80 → 1.40 and floor 0.02 → 0.08 → 0.10,
        // matching the `save_probability` change above. Calibration-
        // neutral at equal skill, less crushing at extreme gaps.
        Self::sigmoid((self.handling_profile - catch_difficulty) * 1.40).clamp(0.10, 0.78)
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

    #[inline]
    fn sigmoid(x: f32) -> f32 {
        1.0 / (1.0 + (-x).exp())
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

    fn build_keeper(fill: f32, condition: i16) -> MatchPlayer {
        let attrs = PlayerAttributes {
            condition,
            jadedness: 0,
            ..Default::default()
        };
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
        MatchPlayer::from_player(1, &player, PlayerPositionType::Goalkeeper, false, None)
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
