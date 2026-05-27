use super::{AcademyDevelopmentIdentity, AcademyPlayerPhase, AcademyTier, ClubAcademy};
use crate::Staff;
use crate::context::GlobalContext;
use crate::{Person, Player, PlayerFieldPositionGroup};

/// Per-phase, per-category soft weekly growth caps. Applied as a hard
/// upper bound on the sum of positive gains for the week — keeps a hot
/// dice roll from minting a 12-year-old phenom.
#[derive(Copy, Clone)]
pub struct PhaseGrowthCaps {
    pub technical: f32,
    pub mental: f32,
    pub physical: f32,
}

/// Per-skill snapshot taken before training so the per-skill cap can
/// limit each individual gain after the training tick. Owns the
/// snapshot + cap math; the training loop just calls `snapshot` before
/// and `cap_positive_delta` after. Cap is applied per skill, NOT per
/// category sum — the previous category-sum cap meant a 14-skill
/// technical category effectively allowed only ~0.034/14 ≈ 0.002 per
/// skill, which made development glacial.
struct SkillSnapshot {
    technical: [f32; 14],
    mental: [f32; 14],
    physical: [f32; 8],
    goalkeeping: [f32; 13],
}

impl SkillSnapshot {
    fn snapshot(player: &Player) -> Self {
        let t = &player.skills.technical;
        let m = &player.skills.mental;
        let p = &player.skills.physical;
        let g = &player.skills.goalkeeping;
        SkillSnapshot {
            technical: [
                t.corners,
                t.crossing,
                t.dribbling,
                t.finishing,
                t.first_touch,
                t.free_kicks,
                t.heading,
                t.long_shots,
                t.long_throws,
                t.marking,
                t.passing,
                t.penalty_taking,
                t.tackling,
                t.technique,
            ],
            mental: [
                m.aggression,
                m.anticipation,
                m.bravery,
                m.composure,
                m.concentration,
                m.decisions,
                m.determination,
                m.flair,
                m.leadership,
                m.off_the_ball,
                m.positioning,
                m.teamwork,
                m.vision,
                m.work_rate,
            ],
            physical: [
                p.acceleration,
                p.agility,
                p.balance,
                p.jumping,
                p.natural_fitness,
                p.pace,
                p.stamina,
                p.strength,
            ],
            goalkeeping: [
                g.aerial_reach,
                g.command_of_area,
                g.communication,
                g.eccentricity,
                g.first_touch,
                g.handling,
                g.kicking,
                g.one_on_ones,
                g.passing,
                g.punching,
                g.reflexes,
                g.rushing_out,
                g.throwing,
            ],
        }
    }

    fn cap_positive_delta(&self, player: &mut Player, caps: PhaseGrowthCaps) {
        // GK skills share the technical learning-curve cap — academies
        // train a GK's technical work the same week as outfielders.
        Self::cap_technical(player, &self.technical, caps.technical);
        Self::cap_mental(player, &self.mental, caps.mental);
        Self::cap_physical(player, &self.physical, caps.physical);
        Self::cap_goalkeeping(player, &self.goalkeeping, caps.technical);
    }

    /// Per-skill cap: clamp the *positive* portion of the delta to
    /// `cap`, and re-add any negative component (so a growth-spurt
    /// coordination dip isn't masked).
    fn cap_skill(before: f32, after: f32, cap: f32) -> f32 {
        if cap <= 0.0 {
            return after;
        }
        let delta = after - before;
        let positive = delta.max(0.0).min(cap);
        let negative = delta.min(0.0);
        before + positive + negative
    }

    fn cap_technical(player: &mut Player, before: &[f32; 14], cap: f32) {
        let t = &mut player.skills.technical;
        t.corners = Self::cap_skill(before[0], t.corners, cap);
        t.crossing = Self::cap_skill(before[1], t.crossing, cap);
        t.dribbling = Self::cap_skill(before[2], t.dribbling, cap);
        t.finishing = Self::cap_skill(before[3], t.finishing, cap);
        t.first_touch = Self::cap_skill(before[4], t.first_touch, cap);
        t.free_kicks = Self::cap_skill(before[5], t.free_kicks, cap);
        t.heading = Self::cap_skill(before[6], t.heading, cap);
        t.long_shots = Self::cap_skill(before[7], t.long_shots, cap);
        t.long_throws = Self::cap_skill(before[8], t.long_throws, cap);
        t.marking = Self::cap_skill(before[9], t.marking, cap);
        t.passing = Self::cap_skill(before[10], t.passing, cap);
        t.penalty_taking = Self::cap_skill(before[11], t.penalty_taking, cap);
        t.tackling = Self::cap_skill(before[12], t.tackling, cap);
        t.technique = Self::cap_skill(before[13], t.technique, cap);
    }

    fn cap_mental(player: &mut Player, before: &[f32; 14], cap: f32) {
        let m = &mut player.skills.mental;
        m.aggression = Self::cap_skill(before[0], m.aggression, cap);
        m.anticipation = Self::cap_skill(before[1], m.anticipation, cap);
        m.bravery = Self::cap_skill(before[2], m.bravery, cap);
        m.composure = Self::cap_skill(before[3], m.composure, cap);
        m.concentration = Self::cap_skill(before[4], m.concentration, cap);
        m.decisions = Self::cap_skill(before[5], m.decisions, cap);
        m.determination = Self::cap_skill(before[6], m.determination, cap);
        m.flair = Self::cap_skill(before[7], m.flair, cap);
        m.leadership = Self::cap_skill(before[8], m.leadership, cap);
        m.off_the_ball = Self::cap_skill(before[9], m.off_the_ball, cap);
        m.positioning = Self::cap_skill(before[10], m.positioning, cap);
        m.teamwork = Self::cap_skill(before[11], m.teamwork, cap);
        m.vision = Self::cap_skill(before[12], m.vision, cap);
        m.work_rate = Self::cap_skill(before[13], m.work_rate, cap);
    }

    fn cap_physical(player: &mut Player, before: &[f32; 8], cap: f32) {
        let p = &mut player.skills.physical;
        p.acceleration = Self::cap_skill(before[0], p.acceleration, cap);
        p.agility = Self::cap_skill(before[1], p.agility, cap);
        p.balance = Self::cap_skill(before[2], p.balance, cap);
        p.jumping = Self::cap_skill(before[3], p.jumping, cap);
        p.natural_fitness = Self::cap_skill(before[4], p.natural_fitness, cap);
        p.pace = Self::cap_skill(before[5], p.pace, cap);
        p.stamina = Self::cap_skill(before[6], p.stamina, cap);
        p.strength = Self::cap_skill(before[7], p.strength, cap);
    }

    fn cap_goalkeeping(player: &mut Player, before: &[f32; 13], cap: f32) {
        let g = &mut player.skills.goalkeeping;
        g.aerial_reach = Self::cap_skill(before[0], g.aerial_reach, cap);
        g.command_of_area = Self::cap_skill(before[1], g.command_of_area, cap);
        g.communication = Self::cap_skill(before[2], g.communication, cap);
        g.eccentricity = Self::cap_skill(before[3], g.eccentricity, cap);
        g.first_touch = Self::cap_skill(before[4], g.first_touch, cap);
        g.handling = Self::cap_skill(before[5], g.handling, cap);
        g.kicking = Self::cap_skill(before[6], g.kicking, cap);
        g.one_on_ones = Self::cap_skill(before[7], g.one_on_ones, cap);
        g.passing = Self::cap_skill(before[8], g.passing, cap);
        g.punching = Self::cap_skill(before[9], g.punching, cap);
        g.reflexes = Self::cap_skill(before[10], g.reflexes, cap);
        g.rushing_out = Self::cap_skill(before[11], g.rushing_out, cap);
        g.throwing = Self::cap_skill(before[12], g.throwing, cap);
    }
}

impl PhaseGrowthCaps {
    pub fn for_phase(phase: AcademyPlayerPhase) -> Self {
        match phase {
            AcademyPlayerPhase::Foundation => PhaseGrowthCaps {
                technical: 0.014,
                mental: 0.010,
                physical: 0.006,
            },
            AcademyPlayerPhase::Development => PhaseGrowthCaps {
                technical: 0.020,
                mental: 0.014,
                physical: 0.008,
            },
            AcademyPlayerPhase::Professional => PhaseGrowthCaps {
                technical: 0.034,
                mental: 0.026,
                physical: 0.018,
            },
        }
    }
}

/// Identity bias per skill category. Returned as
/// `(technical, mental, physical)` multipliers on top of the base gain
/// multiplier. Forward/midfielders in PlayerTrading get an extra
/// technical nudge — the identity is "develop sellable attackers".
pub struct IdentityTrainingMultipliers;

impl IdentityTrainingMultipliers {
    pub fn for_identity(
        identity: AcademyDevelopmentIdentity,
        group: PlayerFieldPositionGroup,
    ) -> (f32, f32, f32) {
        match identity {
            AcademyDevelopmentIdentity::Balanced => (1.00, 1.00, 1.00),
            AcademyDevelopmentIdentity::TechnicalSchool => (1.12, 1.04, 0.95),
            AcademyDevelopmentIdentity::TacticalSchool => (1.02, 1.14, 0.96),
            AcademyDevelopmentIdentity::AthleticDevelopment => (0.96, 1.00, 1.14),
            AcademyDevelopmentIdentity::PlayerTrading => match group {
                PlayerFieldPositionGroup::Forward | PlayerFieldPositionGroup::Midfielder => {
                    (1.12, 1.04, 1.04)
                }
                _ => (1.08, 1.04, 1.04),
            },
        }
    }
}

impl ClubAcademy {
    /// Apply weekly training to all academy players based on their development phase.
    ///
    /// The driver is `final_gain_mult`, a stack of multipliers:
    ///   * `environment_mult` — academy/facility/coaching/tier/pathway blend.
    ///   * `staff_mult_for_category` — best coach per skill family.
    ///   * `youth_bonus` — HoYD working-with-youngsters bonus.
    ///   * `personality_mult` — professionalism/ambition/work-rate weighted.
    ///   * `session_mult` — phase/tier session count divided by 4.
    ///   * `welfare_mult` — condition + jadedness.
    ///   * uniform `[0.88, 1.12]` weekly variance.
    ///
    /// Per-phase per-category caps then bound the total positive change,
    /// and a PA-derived `skill_ceiling` bounds each individual skill.
    /// PA is never raised by training.
    pub(super) fn train_academy_players(&mut self, ctx: &GlobalContext<'_>) {
        if !ctx.simulation.is_week_beginning() {
            return;
        }

        let date = ctx.simulation.date.date();

        // Shared per-week environment scalar.
        let tier = AcademyTier::from_level(self.level);
        let academy_env = 0.30 * ctx.club_academy_quality()
            + 0.25 * ctx.club_facilities_youth()
            + 0.20 * ctx.club_youth_coaching_quality()
            + 0.15 * tier.norm()
            + 0.10 * (self.pathway_reputation as f32 / 100.0);
        let environment_mult = (0.70 + academy_env.clamp(0.0, 1.0) * 0.55).clamp(0.70, 1.25);

        // Per-category staff multipliers from the best academy coach in
        // each family. Falls back to the academy's base coaching when no
        // dedicated coach exists.
        let staff = self.coaching_staff_multipliers();
        let youth_bonus = self.youth_coaching_bonus();
        let identity = self.development_identity;

        for player in &mut self.players.players {
            if player.player_attributes.is_injured {
                continue;
            }

            let age = player.age(date);
            let phase = AcademyPlayerPhase::from_age(age);
            let phase_idx = phase.index();
            let group = player.position().position_group();
            let is_gk = group == PlayerFieldPositionGroup::Goalkeeper;

            // Phase-aware session multiplier (real academies train
            // 2-6 times per week depending on age band and resources).
            let sessions = tier.sessions_for_phase(phase_idx) as f32;
            let session_mult = (sessions / 4.0).clamp(0.50, 1.50);

            let personality_mult = PersonalityTrainingFactor::compute(player);
            let welfare_mult = WelfareMultiplier::compute(player);
            let variance = 0.88 + rand::random::<f32>() * 0.24; // 0.88..1.12

            let base_mult = environment_mult
                * youth_bonus
                * personality_mult
                * session_mult
                * welfare_mult
                * variance;

            let tech_mult = base_mult * staff.technical;
            let mental_mult = base_mult * staff.mental;
            let physical_mult = base_mult * staff.physical;
            let gk_mult = base_mult * staff.goalkeeping;

            // Identity emphasis is applied directly on the per-category
            // gain multiplier so the cap still bounds the result.
            let (id_tech, id_mental, id_phys) =
                IdentityTrainingMultipliers::for_identity(identity, group);
            let tech_m = tech_mult * id_tech;
            let mental_m = mental_mult * id_mental;
            let physical_m = physical_mult * id_phys;
            let gk_m = gk_mult * id_tech; // goalkeeping is a technical family

            // Snapshot per-skill values before training so the
            // per-category weekly cap can uniformly shrink the positive
            // delta if training over-runs (`PhaseGrowthCaps`).
            let before = SkillSnapshot::snapshot(player);

            let session = PhaseTrainingSession { phase, is_gk };
            session.apply(player, tech_m, mental_m, physical_m, gk_m);

            // Growth spurts during puberty: small physical gain with a
            // temporary coordination cost. Bounded by the cap below.
            if (13..=15).contains(&age) {
                GrowthSpurt::roll_and_apply(player);
            }

            let caps = PhaseGrowthCaps::for_phase(phase);
            // Per-category cap on positive weekly delta. Scales the
            // post-training totals downward if a hot-rolled cocktail of
            // multipliers blew past the cap.
            before.cap_positive_delta(player, caps);

            // PA-derived per-skill ceilings — PA is the biological cap;
            // it's never raised by training. Position-weighted so a GK
            // doesn't develop full-scale finishing and a striker doesn't
            // peg max marking.
            SkillCeilings { age, caps }.enforce(player, group);

            let pos = player.position();
            let recomputed_ca = player.skills.calculate_ability_for_position(pos);
            // Never raise PA via training — it's the biological cap.
            player.player_attributes.current_ability =
                recomputed_ca.min(player.player_attributes.potential_ability);
        }
    }

    /// Per-category staff coaching multipliers (0.75..1.25). When the
    /// academy has no dedicated staff the multipliers fall back to the
    /// base-coaching curve so even a no-staff academy still trains.
    fn coaching_staff_multipliers(&self) -> StaffCategoryMultipliers {
        let tier = AcademyTier::from_level(self.level);
        // Base coaching curve: 1..10 tier maps to 0.495..0.90. Pre-staff
        // ceiling of 1.0 only after staff modifiers kick in.
        let base_coaching = 0.45 + tier.norm() * 0.45; // 0.495..0.90
        if self.staff.staffs.is_empty() {
            return StaffCategoryMultipliers {
                technical: base_coaching,
                mental: base_coaching,
                physical: base_coaching,
                goalkeeping: base_coaching,
            };
        }

        let best =
            |f: fn(&Staff) -> u8| -> u8 { self.staff.staffs.iter().map(f).max().unwrap_or(0) };

        let best_technical = best(|s| s.staff_attributes.coaching.technical);
        let best_tactical = best(|s| s.staff_attributes.coaching.tactical);
        let best_fitness = best(|s| s.staff_attributes.coaching.fitness);
        let best_gk_h = best(|s| s.staff_attributes.goalkeeping.handling);
        let best_gk_s = best(|s| s.staff_attributes.goalkeeping.shot_stopping);
        let best_gk_d = best(|s| s.staff_attributes.goalkeeping.distribution);
        let best_gk = best_gk_h.max(best_gk_s).max(best_gk_d);

        // Each staff multiplier sits in 0.75..1.25. Anchored on `0` =
        // 0.75 so even a hopeless coach is still better than none.
        let staff_mult = |score: u8| (0.75 + (score as f32 / 20.0) * 0.50).clamp(0.75, 1.25);

        // Blend base_coaching with the staff signal so an academy with
        // strong infrastructure but weak staff doesn't collapse:
        //   final = max(base_coaching, staff_mult)
        // The cap on total gain is still applied later.
        StaffCategoryMultipliers {
            technical: staff_mult(best_technical).max(base_coaching),
            mental: staff_mult(best_tactical).max(base_coaching),
            physical: staff_mult(best_fitness).max(base_coaching),
            goalkeeping: staff_mult(best_gk).max(base_coaching),
        }
    }

    fn youth_coaching_bonus(&self) -> f32 {
        let best_wwy = self
            .staff
            .staffs
            .iter()
            .map(|s| s.staff_attributes.coaching.working_with_youngsters)
            .max()
            .unwrap_or(0);
        1.00 + (best_wwy as f32 / 20.0) * 0.12 // 1.00..1.12
    }
}

#[derive(Copy, Clone)]
struct StaffCategoryMultipliers {
    technical: f32,
    mental: f32,
    physical: f32,
    goalkeeping: f32,
}

/// Personality multiplier for training absorption: 0.55..1.45.
/// Professionalism dominates because that's the trait that actually
/// predicts academy → senior translation.
struct PersonalityTrainingFactor;

impl PersonalityTrainingFactor {
    fn compute(player: &Player) -> f32 {
        let pro = player.attributes.professionalism;
        let amb = player.attributes.ambition;
        let det = player.skills.mental.determination;
        let wr = player.skills.mental.work_rate;
        let weighted = (0.40 * pro + 0.25 * amb + 0.20 * det + 0.15 * wr) / 20.0;
        (0.55 + weighted * 0.90).clamp(0.55, 1.45)
    }
}

struct WelfareMultiplier;

impl WelfareMultiplier {
    fn compute(player: &Player) -> f32 {
        let condition = (player.player_attributes.condition as f32 / 10000.0).clamp(0.0, 1.0);
        let jaded = (player.player_attributes.jadedness as f32 / 10000.0).clamp(0.0, 1.0);
        (0.50 + 0.35 * condition + 0.15 * (1.0 - jaded)).clamp(0.45, 1.0)
    }
}

/// Per-phase training routine. The `apply` method dispatches to the
/// right outfield / GK body based on `phase` and `is_gk`, keeping the
/// individual `train_*` helpers as private associated functions on the
/// struct rather than free helpers floating in the module.
struct PhaseTrainingSession {
    phase: AcademyPlayerPhase,
    is_gk: bool,
}

impl PhaseTrainingSession {
    fn apply(&self, player: &mut Player, t: f32, m: f32, p: f32, gk: f32) {
        match (self.phase, self.is_gk) {
            (AcademyPlayerPhase::Foundation, false) => Self::train_foundation(player, t, m, p),
            (AcademyPlayerPhase::Foundation, true) => Self::train_foundation_gk(player, gk, m, p),
            (AcademyPlayerPhase::Development, false) => Self::train_development(player, t, m, p),
            (AcademyPlayerPhase::Development, true) => Self::train_development_gk(player, gk, m, p),
            (AcademyPlayerPhase::Professional, false) => Self::train_professional(player, t, m, p),
            (AcademyPlayerPhase::Professional, true) => {
                Self::train_professional_gk(player, gk, m, p)
            }
        }
    }

    // ─── Foundation Phase (ages 8-11): play-based learning, ball mastery
    fn train_foundation(player: &mut Player, t: f32, m: f32, p: f32) {
        player.skills.technical.first_touch += 0.035 * t;
        player.skills.technical.dribbling += 0.030 * t;
        player.skills.technical.technique += 0.025 * t;
        player.skills.technical.passing += 0.020 * t;
        player.skills.technical.crossing += 0.005 * t;

        player.skills.mental.teamwork += 0.015 * m;
        player.skills.mental.decisions += 0.010 * m;
        player.skills.mental.off_the_ball += 0.010 * m;
        player.skills.mental.flair += 0.008 * m;

        player.skills.physical.agility += 0.015 * p;
        player.skills.physical.balance += 0.015 * p;
        player.skills.physical.acceleration += 0.010 * p;
    }

    fn train_foundation_gk(player: &mut Player, gk: f32, m: f32, p: f32) {
        player.skills.goalkeeping.handling += 0.030 * gk;
        player.skills.goalkeeping.reflexes += 0.020 * gk;
        player.skills.goalkeeping.kicking += 0.015 * gk;
        player.skills.goalkeeping.first_touch += 0.015 * gk;
        player.skills.goalkeeping.throwing += 0.010 * gk;

        player.skills.mental.bravery += 0.015 * m;
        player.skills.mental.concentration += 0.010 * m;
        player.skills.mental.decisions += 0.008 * m;

        player.skills.physical.agility += 0.020 * p;
        player.skills.physical.balance += 0.015 * p;
    }

    // ─── Development Phase (ages 12-14)
    fn train_development(player: &mut Player, t: f32, m: f32, p: f32) {
        let group = player.position().position_group();

        player.skills.technical.first_touch += 0.025 * t;
        player.skills.technical.passing += 0.025 * t;
        player.skills.technical.technique += 0.020 * t;

        match group {
            PlayerFieldPositionGroup::Defender => {
                player.skills.technical.tackling += 0.025 * t;
                player.skills.technical.marking += 0.020 * t;
                player.skills.technical.heading += 0.015 * t;
            }
            PlayerFieldPositionGroup::Midfielder => {
                player.skills.technical.passing += 0.015 * t;
                player.skills.technical.crossing += 0.015 * t;
                player.skills.technical.dribbling += 0.015 * t;
            }
            PlayerFieldPositionGroup::Forward => {
                player.skills.technical.finishing += 0.025 * t;
                player.skills.technical.dribbling += 0.020 * t;
                player.skills.technical.long_shots += 0.010 * t;
            }
            _ => {}
        }

        player.skills.mental.positioning += 0.025 * m;
        player.skills.mental.concentration += 0.020 * m;
        player.skills.mental.decisions += 0.020 * m;
        player.skills.mental.anticipation += 0.015 * m;
        player.skills.mental.teamwork += 0.015 * m;
        player.skills.mental.vision += 0.010 * m;

        player.skills.physical.agility += 0.015 * p;
        player.skills.physical.balance += 0.015 * p;
        player.skills.physical.pace += 0.010 * p;
        player.skills.physical.stamina += 0.010 * p;
    }

    fn train_development_gk(player: &mut Player, gk: f32, m: f32, p: f32) {
        player.skills.goalkeeping.handling += 0.025 * gk;
        player.skills.goalkeeping.reflexes += 0.025 * gk;
        player.skills.goalkeeping.one_on_ones += 0.020 * gk;
        player.skills.goalkeeping.kicking += 0.020 * gk;
        player.skills.goalkeeping.passing += 0.015 * gk;
        player.skills.goalkeeping.communication += 0.015 * gk;
        player.skills.goalkeeping.aerial_reach += 0.010 * gk;
        player.skills.goalkeeping.command_of_area += 0.010 * gk;

        player.skills.mental.positioning += 0.025 * m;
        player.skills.mental.concentration += 0.020 * m;
        player.skills.mental.composure += 0.015 * m;
        player.skills.mental.decisions += 0.015 * m;

        player.skills.physical.agility += 0.020 * p;
        player.skills.physical.jumping += 0.015 * p;
        player.skills.physical.acceleration += 0.010 * p;
    }

    // ─── Professional Phase (ages 15-17)
    fn train_professional(player: &mut Player, t: f32, m: f32, p: f32) {
        let group = player.position().position_group();

        player.skills.technical.technique += 0.015 * t;
        player.skills.technical.first_touch += 0.015 * t;

        match group {
            PlayerFieldPositionGroup::Defender => {
                player.skills.technical.tackling += 0.020 * t;
                player.skills.technical.marking += 0.020 * t;
                player.skills.technical.heading += 0.020 * t;
                player.skills.technical.passing += 0.010 * t;
            }
            PlayerFieldPositionGroup::Midfielder => {
                player.skills.technical.passing += 0.020 * t;
                player.skills.technical.crossing += 0.015 * t;
                player.skills.technical.dribbling += 0.015 * t;
                player.skills.technical.long_shots += 0.010 * t;
            }
            PlayerFieldPositionGroup::Forward => {
                player.skills.technical.finishing += 0.025 * t;
                player.skills.technical.dribbling += 0.015 * t;
                player.skills.technical.heading += 0.010 * t;
                player.skills.technical.long_shots += 0.015 * t;
            }
            _ => {}
        }

        player.skills.mental.composure += 0.020 * m;
        player.skills.mental.concentration += 0.020 * m;
        player.skills.mental.decisions += 0.020 * m;
        player.skills.mental.positioning += 0.020 * m;
        player.skills.mental.anticipation += 0.015 * m;
        player.skills.mental.determination += 0.010 * m;
        player.skills.mental.work_rate += 0.010 * m;

        player.skills.physical.strength += 0.025 * p;
        player.skills.physical.stamina += 0.025 * p;
        player.skills.physical.pace += 0.020 * p;
        player.skills.physical.acceleration += 0.015 * p;
        player.skills.physical.jumping += 0.015 * p;
        player.skills.physical.natural_fitness += 0.010 * p;
        player.skills.physical.agility += 0.010 * p;
    }

    fn train_professional_gk(player: &mut Player, gk: f32, m: f32, p: f32) {
        player.skills.goalkeeping.handling += 0.020 * gk;
        player.skills.goalkeeping.reflexes += 0.020 * gk;
        player.skills.goalkeeping.one_on_ones += 0.020 * gk;
        player.skills.goalkeeping.aerial_reach += 0.020 * gk;
        player.skills.goalkeeping.command_of_area += 0.020 * gk;
        player.skills.goalkeeping.rushing_out += 0.015 * gk;
        player.skills.goalkeeping.punching += 0.015 * gk;
        player.skills.goalkeeping.kicking += 0.015 * gk;
        player.skills.goalkeeping.passing += 0.015 * gk;
        player.skills.goalkeeping.communication += 0.015 * gk;
        player.skills.goalkeeping.throwing += 0.010 * gk;

        player.skills.mental.positioning += 0.020 * m;
        player.skills.mental.concentration += 0.020 * m;
        player.skills.mental.composure += 0.020 * m;
        player.skills.mental.decisions += 0.015 * m;
        player.skills.mental.anticipation += 0.015 * m;
        player.skills.mental.leadership += 0.010 * m;

        player.skills.physical.strength += 0.020 * p;
        player.skills.physical.jumping += 0.020 * p;
        player.skills.physical.agility += 0.015 * p;
        player.skills.physical.acceleration += 0.010 * p;
        player.skills.physical.stamina += 0.010 * p;
    }
}

// ───────────────────────────────────────────────────────────────────────
// Growth Spurts & Skill Clamping
// ───────────────────────────────────────────────────────────────────────

/// Puberty growth-spurt effect. Owns the dice roll + the small skill
/// nudge so the training loop doesn't have to.
pub struct GrowthSpurt;

impl GrowthSpurt {
    /// 12% chance of firing. When it fires: small physical gain
    /// (strength/jumping), small coordination cost (agility/balance).
    pub fn roll_and_apply(player: &mut Player) {
        if rand::random::<f32>() > 0.12 {
            return;
        }

        let intensity = 0.01 + rand::random::<f32>() * 0.02;
        player.skills.physical.strength += intensity;
        player.skills.physical.jumping += intensity * 0.6;

        let coord_cost = intensity * 0.35;
        player.skills.physical.agility -= coord_cost;
        player.skills.physical.balance -= coord_cost;
    }
}

/// Age-based + PA-based skill ceilings. PA is *never* raised here —
/// the struct only clamps skills downward when they exceed the
/// position-weighted PA-derived ceiling. `caps` is kept on the struct
/// so a future extension can use the per-phase caps to feed a graded
/// clamp.
pub struct SkillCeilings {
    pub age: u8,
    #[allow(dead_code)]
    pub caps: PhaseGrowthCaps,
}

impl SkillCeilings {
    /// Apply both the age cap and a position-weighted PA-derived cap
    /// per skill. The position weight is clamped to 0.65..1.25 so the
    /// floor is never punishing (a GK still needs to be able to
    /// develop a baseline pass) and the ceiling never overshoots.
    pub fn enforce(&self, player: &mut Player, group: PlayerFieldPositionGroup) {
        let age_cap = match self.age {
            0..=8 => 3.0_f32,
            9 => 3.5,
            10 => 4.5,
            11 => 5.5,
            12 => 7.0,
            13 => 8.5,
            14 => 10.0,
            15 => 12.0,
            16 => 13.0,
            17 => 14.0,
            _ => 15.0,
        };

        let pa = player.player_attributes.potential_ability as f32;
        let base = (pa / 200.0 * 20.0).clamp(1.0, 20.0);
        let w = AcademyCeilingWeights::for_group(group);

        // Each skill gets `min(age_cap, (base * weight).clamp(1.0, 20.0))`.
        let cap_for = |weight: f32| -> f32 {
            let w = weight.clamp(0.65, 1.25);
            age_cap.min((base * w).clamp(1.0, 20.0))
        };
        let clamp = |v: f32, weight: f32| -> f32 { v.clamp(1.0, cap_for(weight)) };

        let t = &mut player.skills.technical;
        t.corners = clamp(t.corners, w.tech_corners);
        t.crossing = clamp(t.crossing, w.tech_crossing);
        t.dribbling = clamp(t.dribbling, w.tech_dribbling);
        t.finishing = clamp(t.finishing, w.tech_finishing);
        t.first_touch = clamp(t.first_touch, w.tech_first_touch);
        t.free_kicks = clamp(t.free_kicks, w.tech_free_kicks);
        t.heading = clamp(t.heading, w.tech_heading);
        t.long_shots = clamp(t.long_shots, w.tech_long_shots);
        t.long_throws = clamp(t.long_throws, w.tech_long_throws);
        t.marking = clamp(t.marking, w.tech_marking);
        t.passing = clamp(t.passing, w.tech_passing);
        t.penalty_taking = clamp(t.penalty_taking, w.tech_penalty);
        t.tackling = clamp(t.tackling, w.tech_tackling);
        t.technique = clamp(t.technique, w.tech_technique);

        let m = &mut player.skills.mental;
        m.aggression = clamp(m.aggression, w.mental_aggression);
        m.anticipation = clamp(m.anticipation, w.mental_anticipation);
        m.bravery = clamp(m.bravery, w.mental_bravery);
        m.composure = clamp(m.composure, w.mental_composure);
        m.concentration = clamp(m.concentration, w.mental_concentration);
        m.decisions = clamp(m.decisions, w.mental_decisions);
        m.determination = clamp(m.determination, w.mental_determination);
        m.flair = clamp(m.flair, w.mental_flair);
        m.leadership = clamp(m.leadership, w.mental_leadership);
        m.off_the_ball = clamp(m.off_the_ball, w.mental_off_the_ball);
        m.positioning = clamp(m.positioning, w.mental_positioning);
        m.teamwork = clamp(m.teamwork, w.mental_teamwork);
        m.vision = clamp(m.vision, w.mental_vision);
        m.work_rate = clamp(m.work_rate, w.mental_work_rate);

        let p = &mut player.skills.physical;
        p.acceleration = clamp(p.acceleration, w.phys_acceleration);
        p.agility = clamp(p.agility, w.phys_agility);
        p.balance = clamp(p.balance, w.phys_balance);
        p.jumping = clamp(p.jumping, w.phys_jumping);
        p.natural_fitness = clamp(p.natural_fitness, w.phys_fitness);
        p.pace = clamp(p.pace, w.phys_pace);
        p.stamina = clamp(p.stamina, w.phys_stamina);
        p.strength = clamp(p.strength, w.phys_strength);

        let g = &mut player.skills.goalkeeping;
        g.aerial_reach = clamp(g.aerial_reach, w.gk_aerial);
        g.command_of_area = clamp(g.command_of_area, w.gk_command);
        g.communication = clamp(g.communication, w.gk_communication);
        g.eccentricity = clamp(g.eccentricity, w.gk_eccentricity);
        g.first_touch = clamp(g.first_touch, w.gk_first_touch);
        g.handling = clamp(g.handling, w.gk_handling);
        g.kicking = clamp(g.kicking, w.gk_kicking);
        g.one_on_ones = clamp(g.one_on_ones, w.gk_one_on_ones);
        g.passing = clamp(g.passing, w.gk_passing);
        g.punching = clamp(g.punching, w.gk_punching);
        g.reflexes = clamp(g.reflexes, w.gk_reflexes);
        g.rushing_out = clamp(g.rushing_out, w.gk_rushing);
        g.throwing = clamp(g.throwing, w.gk_throwing);
    }
}

/// Per-skill position weights used by `SkillCeilings`. Kept academy-
/// local on purpose: the development module has its own (private)
/// weight table tuned for daily growth rates, and that one has GK-only
/// skills zeroed out for outfielders — which would *strand* an existing
/// GK skill at value 1.0 forever. The academy ceiling table instead
/// uses a small, non-zero baseline (≥ 0.65 after clamping) for
/// out-of-position skills so prospects can still develop fundamentals
/// across the squad.
#[derive(Copy, Clone)]
struct AcademyCeilingWeights {
    tech_corners: f32,
    tech_crossing: f32,
    tech_dribbling: f32,
    tech_finishing: f32,
    tech_first_touch: f32,
    tech_free_kicks: f32,
    tech_heading: f32,
    tech_long_shots: f32,
    tech_long_throws: f32,
    tech_marking: f32,
    tech_passing: f32,
    tech_penalty: f32,
    tech_tackling: f32,
    tech_technique: f32,
    mental_aggression: f32,
    mental_anticipation: f32,
    mental_bravery: f32,
    mental_composure: f32,
    mental_concentration: f32,
    mental_decisions: f32,
    mental_determination: f32,
    mental_flair: f32,
    mental_leadership: f32,
    mental_off_the_ball: f32,
    mental_positioning: f32,
    mental_teamwork: f32,
    mental_vision: f32,
    mental_work_rate: f32,
    phys_acceleration: f32,
    phys_agility: f32,
    phys_balance: f32,
    phys_jumping: f32,
    phys_fitness: f32,
    phys_pace: f32,
    phys_stamina: f32,
    phys_strength: f32,
    gk_aerial: f32,
    gk_command: f32,
    gk_communication: f32,
    gk_eccentricity: f32,
    gk_first_touch: f32,
    gk_handling: f32,
    gk_kicking: f32,
    gk_one_on_ones: f32,
    gk_passing: f32,
    gk_punching: f32,
    gk_reflexes: f32,
    gk_rushing: f32,
    gk_throwing: f32,
}

impl AcademyCeilingWeights {
    fn for_group(group: PlayerFieldPositionGroup) -> Self {
        // Default = 1.0; will be clamped to 0.65..1.25 on use anyway.
        let mut w = AcademyCeilingWeights {
            tech_corners: 1.0,
            tech_crossing: 1.0,
            tech_dribbling: 1.0,
            tech_finishing: 1.0,
            tech_first_touch: 1.0,
            tech_free_kicks: 1.0,
            tech_heading: 1.0,
            tech_long_shots: 1.0,
            tech_long_throws: 1.0,
            tech_marking: 1.0,
            tech_passing: 1.0,
            tech_penalty: 1.0,
            tech_tackling: 1.0,
            tech_technique: 1.0,
            mental_aggression: 1.0,
            mental_anticipation: 1.0,
            mental_bravery: 1.0,
            mental_composure: 1.0,
            mental_concentration: 1.0,
            mental_decisions: 1.0,
            mental_determination: 1.0,
            mental_flair: 1.0,
            mental_leadership: 1.0,
            mental_off_the_ball: 1.0,
            mental_positioning: 1.0,
            mental_teamwork: 1.0,
            mental_vision: 1.0,
            mental_work_rate: 1.0,
            phys_acceleration: 1.0,
            phys_agility: 1.0,
            phys_balance: 1.0,
            phys_jumping: 1.0,
            phys_fitness: 1.0,
            phys_pace: 1.0,
            phys_stamina: 1.0,
            phys_strength: 1.0,
            // For outfielders we keep GK skills at the floor so they don't
            // collect bonus academy gains there. Clamped to 0.65 on use.
            gk_aerial: 0.5,
            gk_command: 0.5,
            gk_communication: 0.5,
            gk_eccentricity: 0.5,
            gk_first_touch: 0.6,
            gk_handling: 0.5,
            gk_kicking: 0.5,
            gk_one_on_ones: 0.5,
            gk_passing: 0.6,
            gk_punching: 0.5,
            gk_reflexes: 0.5,
            gk_rushing: 0.5,
            gk_throwing: 0.5,
        };

        match group {
            PlayerFieldPositionGroup::Goalkeeper => {
                // GK-specific outfield weights — keep technical/physical
                // low so a goalkeeper doesn't develop full-scale finishing.
                w.tech_corners = 0.5;
                w.tech_crossing = 0.5;
                w.tech_dribbling = 0.5;
                w.tech_finishing = 0.5;
                w.tech_free_kicks = 0.55;
                w.tech_heading = 0.55;
                w.tech_long_shots = 0.5;
                w.tech_long_throws = 0.6;
                w.tech_marking = 0.55;
                w.tech_penalty = 0.5;
                w.tech_tackling = 0.55;
                // Still useful for sweeper-keepers.
                w.tech_first_touch = 1.05;
                w.tech_passing = 1.05;
                w.tech_technique = 1.0;

                w.mental_positioning = 1.20;
                w.mental_concentration = 1.20;
                w.mental_composure = 1.15;
                w.mental_decisions = 1.15;
                w.mental_anticipation = 1.15;
                w.mental_bravery = 1.10;
                w.mental_flair = 0.7;
                w.mental_off_the_ball = 0.7;

                w.phys_agility = 1.20;
                w.phys_jumping = 1.20;
                w.phys_balance = 1.10;
                w.phys_pace = 0.8;
                w.phys_stamina = 0.8;
                w.phys_acceleration = 0.9;

                // GK-specific are the core for goalkeepers.
                w.gk_handling = 1.25;
                w.gk_reflexes = 1.25;
                w.gk_one_on_ones = 1.20;
                w.gk_aerial = 1.20;
                w.gk_command = 1.20;
                w.gk_communication = 1.15;
                w.gk_rushing = 1.15;
                w.gk_punching = 1.10;
                w.gk_kicking = 1.10;
                w.gk_throwing = 1.05;
                w.gk_first_touch = 1.05;
                w.gk_passing = 1.05;
                w.gk_eccentricity = 0.8;
            }
            PlayerFieldPositionGroup::Defender => {
                w.tech_tackling = 1.20;
                w.tech_marking = 1.20;
                w.tech_heading = 1.15;
                w.tech_passing = 1.05;
                w.tech_finishing = 0.6;
                w.tech_dribbling = 0.7;
                w.tech_long_shots = 0.65;
                w.tech_corners = 0.7;
                w.tech_free_kicks = 0.7;

                w.mental_positioning = 1.20;
                w.mental_concentration = 1.15;
                w.mental_anticipation = 1.15;
                w.mental_bravery = 1.15;
                w.mental_flair = 0.7;

                w.phys_strength = 1.15;
                w.phys_jumping = 1.15;
                w.phys_pace = 1.05;
                w.phys_stamina = 1.05;
            }
            PlayerFieldPositionGroup::Midfielder => {
                w.tech_passing = 1.20;
                w.tech_technique = 1.15;
                w.tech_first_touch = 1.15;
                w.tech_dribbling = 1.05;
                w.tech_heading = 0.75;

                w.mental_vision = 1.20;
                w.mental_decisions = 1.15;
                w.mental_teamwork = 1.15;
                w.mental_work_rate = 1.15;

                w.phys_stamina = 1.15;
                w.phys_pace = 1.05;
                w.phys_agility = 1.05;
            }
            PlayerFieldPositionGroup::Forward => {
                w.tech_finishing = 1.25;
                w.tech_dribbling = 1.20;
                w.tech_first_touch = 1.15;
                w.tech_long_shots = 1.05;
                w.tech_tackling = 0.6;
                w.tech_marking = 0.6;

                w.mental_off_the_ball = 1.20;
                w.mental_composure = 1.15;
                w.mental_anticipation = 1.15;
                w.mental_positioning = 0.8;

                w.phys_pace = 1.20;
                w.phys_acceleration = 1.20;
                w.phys_strength = 1.05;
            }
        }
        w
    }
}

#[cfg(test)]
mod tests {
    use super::{PhaseGrowthCaps, SkillSnapshot};
    use crate::club::academy::AcademyPlayerPhase;
    use crate::club::academy::tuning::AcademyTier;

    #[test]
    fn base_coaching_stays_under_one_at_level_20() {
        // The base_coaching curve must not exceed 1.0 at level 20.
        // Staff/facility modifiers can push the final multiplier above
        // it, but the *base* must stay within 0.45 + 0.45 = 0.90.
        let tier_norm = AcademyTier::from_level(20).norm();
        let base_coaching = 0.45 + tier_norm * 0.45;
        assert!(base_coaching <= 0.90 + 1e-6);
        // Sanity: weakest academy still trains.
        let weak_base = 0.45 + AcademyTier::from_level(1).norm() * 0.45;
        assert!(weak_base >= 0.49 && weak_base <= 0.55);
    }

    #[test]
    fn cap_skill_limits_single_skill_to_phase_cap() {
        // A single absurdly large delta is clamped to the phase cap;
        // negative deltas pass through untouched.
        let pro = PhaseGrowthCaps::for_phase(AcademyPlayerPhase::Professional);
        // After - before far larger than cap → capped at exactly `cap`.
        let capped = SkillSnapshot::cap_skill(10.0, 10.0 + 0.5, pro.technical);
        assert!((capped - (10.0 + pro.technical)).abs() < 1e-6);
        // After - before negative → preserved (coordination dip).
        let dip = SkillSnapshot::cap_skill(10.0, 9.95, pro.physical);
        assert!((dip - 9.95).abs() < 1e-6);
    }

    #[test]
    fn cap_lets_multiple_skills_grow_independently() {
        // Per-skill cap means N skills can each grow up to the cap;
        // the category as a whole grows N*cap. This is the fix —
        // previously the category sum was the cap, so doubling the
        // number of trained skills *halved* each skill's gain.
        let dev = PhaseGrowthCaps::for_phase(AcademyPlayerPhase::Development);

        let a = SkillSnapshot::cap_skill(10.0, 10.0 + 0.1, dev.technical);
        let b = SkillSnapshot::cap_skill(10.0, 10.0 + 0.1, dev.technical);
        // Both clamp to the same single-skill cap.
        assert!((a - (10.0 + dev.technical)).abs() < 1e-6);
        assert!((b - (10.0 + dev.technical)).abs() < 1e-6);
        // Sum should be ~ 2*cap added, not 1*cap.
        let total_gain = (a - 10.0) + (b - 10.0);
        assert!(
            total_gain > dev.technical * 1.5,
            "per-skill cap should allow N*cap total; got {total_gain}"
        );
    }
}
