use super::{AcademyDevelopmentIdentity, AcademyPlayerPhase, AcademyTier, ClubAcademy};
use crate::Staff;
use crate::club::player::development::{PositionalSkillCeilings, SkillKey};
use crate::context::GlobalContext;
use crate::utils::DateUtils;
use crate::{Person, Player, PlayerFieldPositionGroup};
use chrono::{Datelike, NaiveDate};

/// Deterministic per-(player, date, salt) roll in `[0.0, 1.0)`, mirroring
/// the development tick's RollSource seam: the same player on the same
/// day always rolls the same value, so academy development is
/// reproducible in tests and stable across thread scheduling.
struct AcademyRoll;

impl AcademyRoll {
    fn unit(player_id: u32, date: NaiveDate, salt: u32) -> f32 {
        let h = (player_id as u64)
            .wrapping_mul(0x9E37_79B9_7F4A_7C15)
            .wrapping_add((date.num_days_from_ce() as u64).wrapping_mul(0xC6BC_279E_9286_5A2B))
            .wrapping_add((salt as u64).wrapping_mul(0x1657_8F35_4D38_C5A7));
        let frac = ((h >> 11) as u32 as f32) / (u32::MAX as f32);
        frac.clamp(0.0, 0.999)
    }
}

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
pub(super) struct SkillSnapshot {
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
    /// and the first team's per-skill ceilings bound each individual skill.
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
            let variance = 0.88 + AcademyRoll::unit(player.id, date, 0xACAD) * 0.24; // 0.88..1.12

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
                GrowthSpurt::roll_and_apply(player, date);
            }

            let caps = PhaseGrowthCaps::for_phase(phase);
            // Per-category cap on positive weekly delta. Scales the
            // post-training totals downward if a hot-rolled cocktail of
            // multipliers blew past the cap.
            before.cap_positive_delta(player, caps);

            // The first team's own per-skill ceilings, so an academy
            // can never grow a boy past what his age and football allow
            // a senior player. The pre-training snapshot rides along so
            // a value that already sits above its ceiling (growth spurt,
            // reassessed PA, imported record) is frozen, not cut.
            SkillCeilings::enforce(player, date, &before);

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
    pub fn roll_and_apply(player: &mut Player, date: NaiveDate) {
        if AcademyRoll::unit(player.id, date, 0x5B02) > 0.12 {
            return;
        }

        let intensity = 0.01 + AcademyRoll::unit(player.id, date, 0x5B03) * 0.02;
        player.skills.physical.strength += intensity;
        player.skills.physical.jumping += intensity * 0.6;

        let coord_cost = intensity * 0.35;
        player.skills.physical.agility -= coord_cost;
        player.skills.physical.balance -= coord_cost;
    }
}

/// An academy player's per-skill ceilings are the ones first-team
/// development would set him: from his potential, his position's
/// development weights, his families' maturity at his fractional age and
/// his match exposure. PA is *never* raised here.
pub struct SkillCeilings;

impl SkillCeilings {
    /// Ceilings gate *growth* — a value already above its ceiling before
    /// this week's session (growth spurt, PA edge case, imported record)
    /// is frozen at its pre-training level, never cut down. `before` is
    /// the same snapshot `cap_positive_delta` uses.
    pub fn enforce(player: &mut Player, date: NaiveDate, before: &SkillSnapshot) {
        let age = DateUtils::age_in_years(player.birth_date, date);
        let ceilings = PositionalSkillCeilings::for_player(player, age);
        let clamp =
            |v: f32, key: SkillKey, pre: f32| v.clamp(1.0, ceilings.get(key).max(pre.min(20.0)));

        let b = &before.technical;
        let t = &mut player.skills.technical;
        t.corners = clamp(t.corners, SkillKey::Corners, b[0]);
        t.crossing = clamp(t.crossing, SkillKey::Crossing, b[1]);
        t.dribbling = clamp(t.dribbling, SkillKey::Dribbling, b[2]);
        t.finishing = clamp(t.finishing, SkillKey::Finishing, b[3]);
        t.first_touch = clamp(t.first_touch, SkillKey::FirstTouch, b[4]);
        t.free_kicks = clamp(t.free_kicks, SkillKey::FreeKicks, b[5]);
        t.heading = clamp(t.heading, SkillKey::Heading, b[6]);
        t.long_shots = clamp(t.long_shots, SkillKey::LongShots, b[7]);
        t.long_throws = clamp(t.long_throws, SkillKey::LongThrows, b[8]);
        t.marking = clamp(t.marking, SkillKey::Marking, b[9]);
        t.passing = clamp(t.passing, SkillKey::Passing, b[10]);
        t.penalty_taking = clamp(t.penalty_taking, SkillKey::PenaltyTaking, b[11]);
        t.tackling = clamp(t.tackling, SkillKey::Tackling, b[12]);
        t.technique = clamp(t.technique, SkillKey::Technique, b[13]);

        let b = &before.mental;
        let m = &mut player.skills.mental;
        m.aggression = clamp(m.aggression, SkillKey::Aggression, b[0]);
        m.anticipation = clamp(m.anticipation, SkillKey::Anticipation, b[1]);
        m.bravery = clamp(m.bravery, SkillKey::Bravery, b[2]);
        m.composure = clamp(m.composure, SkillKey::Composure, b[3]);
        m.concentration = clamp(m.concentration, SkillKey::Concentration, b[4]);
        m.decisions = clamp(m.decisions, SkillKey::Decisions, b[5]);
        m.determination = clamp(m.determination, SkillKey::Determination, b[6]);
        m.flair = clamp(m.flair, SkillKey::Flair, b[7]);
        m.leadership = clamp(m.leadership, SkillKey::Leadership, b[8]);
        m.off_the_ball = clamp(m.off_the_ball, SkillKey::OffTheBall, b[9]);
        m.positioning = clamp(m.positioning, SkillKey::Positioning, b[10]);
        m.teamwork = clamp(m.teamwork, SkillKey::Teamwork, b[11]);
        m.vision = clamp(m.vision, SkillKey::Vision, b[12]);
        m.work_rate = clamp(m.work_rate, SkillKey::WorkRate, b[13]);

        let b = &before.physical;
        let p = &mut player.skills.physical;
        p.acceleration = clamp(p.acceleration, SkillKey::Acceleration, b[0]);
        p.agility = clamp(p.agility, SkillKey::Agility, b[1]);
        p.balance = clamp(p.balance, SkillKey::Balance, b[2]);
        p.jumping = clamp(p.jumping, SkillKey::Jumping, b[3]);
        p.natural_fitness = clamp(p.natural_fitness, SkillKey::NaturalFitness, b[4]);
        p.pace = clamp(p.pace, SkillKey::Pace, b[5]);
        p.stamina = clamp(p.stamina, SkillKey::Stamina, b[6]);
        p.strength = clamp(p.strength, SkillKey::Strength, b[7]);

        let b = &before.goalkeeping;
        let g = &mut player.skills.goalkeeping;
        g.aerial_reach = clamp(g.aerial_reach, SkillKey::GkAerialReach, b[0]);
        g.command_of_area = clamp(g.command_of_area, SkillKey::GkCommandOfArea, b[1]);
        g.communication = clamp(g.communication, SkillKey::GkCommunication, b[2]);
        g.eccentricity = clamp(g.eccentricity, SkillKey::GkEccentricity, b[3]);
        g.first_touch = clamp(g.first_touch, SkillKey::GkFirstTouch, b[4]);
        g.handling = clamp(g.handling, SkillKey::GkHandling, b[5]);
        g.kicking = clamp(g.kicking, SkillKey::GkKicking, b[6]);
        g.one_on_ones = clamp(g.one_on_ones, SkillKey::GkOneOnOnes, b[7]);
        g.passing = clamp(g.passing, SkillKey::GkPassing, b[8]);
        g.punching = clamp(g.punching, SkillKey::GkPunching, b[9]);
        g.reflexes = clamp(g.reflexes, SkillKey::GkReflexes, b[10]);
        g.rushing_out = clamp(g.rushing_out, SkillKey::GkRushingOut, b[11]);
        g.throwing = clamp(g.throwing, SkillKey::GkThrowing, b[12]);
    }
}

#[cfg(test)]
mod tests {
    use super::{PhaseGrowthCaps, SkillCeilings, SkillSnapshot};
    use crate::club::academy::AcademyPlayerPhase;
    use crate::club::academy::ClubAcademy;
    use crate::club::academy::tuning::AcademyTier;
    use crate::club::player::builder::PlayerBuilder;
    use crate::club::player::development::{PositionalSkillCeilings, SkillKey};
    use crate::context::{GlobalContext, SimulationContext};
    use crate::shared::fullname::FullName;
    use crate::utils::DateUtils;
    use crate::{
        PersonAttributes, Player, PlayerAttributes, PlayerPosition, PlayerPositionType,
        PlayerPositions, PlayerSkills,
    };
    use chrono::{Datelike, Duration, NaiveDate, Weekday};

    const GOALKEEPING: [SkillKey; 13] = [
        SkillKey::GkAerialReach,
        SkillKey::GkCommandOfArea,
        SkillKey::GkCommunication,
        SkillKey::GkEccentricity,
        SkillKey::GkFirstTouch,
        SkillKey::GkHandling,
        SkillKey::GkKicking,
        SkillKey::GkOneOnOnes,
        SkillKey::GkPassing,
        SkillKey::GkPunching,
        SkillKey::GkReflexes,
        SkillKey::GkRushingOut,
        SkillKey::GkThrowing,
    ];

    fn goalkeeping(p: &mut Player) -> [&mut f32; 13] {
        let g = &mut p.skills.goalkeeping;
        [
            &mut g.aerial_reach,
            &mut g.command_of_area,
            &mut g.communication,
            &mut g.eccentricity,
            &mut g.first_touch,
            &mut g.handling,
            &mut g.kicking,
            &mut g.one_on_ones,
            &mut g.passing,
            &mut g.punching,
            &mut g.reflexes,
            &mut g.rushing_out,
            &mut g.throwing,
        ]
    }

    /// A PA 150 keeper of 18 whose goalkeeping sits just under the
    /// first-team ceilings for his age on `today`.
    fn eighteen_year_old_keeper(today: NaiveDate) -> Player {
        let mut keeper = PlayerBuilder::new()
            .id(7)
            .full_name(FullName::new("Academy".to_string(), "Keeper".to_string()))
            .birth_date(NaiveDate::from_ymd_opt(today.year() - 18, 3, 1).unwrap())
            .country_id(1)
            .attributes(PersonAttributes {
                professionalism: 16.0,
                ambition: 16.0,
                ..PersonAttributes::default()
            })
            .skills(PlayerSkills::flat_for_ability(70))
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: PlayerPositionType::Goalkeeper,
                    level: 20,
                }],
            })
            .player_attributes(PlayerAttributes {
                potential_ability: 150,
                current_ability: 70,
                condition: 9500,
                ..PlayerAttributes::default()
            })
            .build()
            .unwrap();
        let age = DateUtils::age_in_years(keeper.birth_date, today);
        let ceilings = PositionalSkillCeilings::for_player(&keeper, age);
        for (value, key) in goalkeeping(&mut keeper).into_iter().zip(GOALKEEPING) {
            *value = (ceilings.get(key) - 0.3).max(1.0);
        }
        keeper
    }

    /// A season of academy work never takes a keeper past what the first
    /// team's ceilings allow for his age and his football.
    #[test]
    fn an_academy_keeper_cannot_outgrow_the_first_teams_ceiling() {
        let start = NaiveDate::from_ymd_opt(2026, 8, 3).unwrap();
        let mut academy = ClubAcademy::new(20);
        academy.players.add(eighteen_year_old_keeper(start));
        let mut bound = false;
        for day in 0..365 {
            let date = start + Duration::days(day);
            let ctx =
                GlobalContext::new(SimulationContext::new(date.and_hms_opt(0, 0, 0).unwrap()));
            academy.train_academy_players(&ctx);
            if date.weekday() != Weekday::Mon {
                continue;
            }
            let keeper = &mut academy.players.players[0];
            let age = DateUtils::age_in_years(keeper.birth_date, date);
            let ceilings = PositionalSkillCeilings::for_player(keeper, age);
            for (value, key) in goalkeeping(keeper).into_iter().zip(GOALKEEPING) {
                let ceiling = ceilings.get(key);
                assert!(
                    *value <= ceiling + 1e-4,
                    "{key:?} reached {value:.3} on {date}, past the first-team ceiling {ceiling:.3}"
                );
                bound |= *value >= ceiling - 0.01;
            }
        }
        assert!(bound, "no goalkeeping attribute ever reached its ceiling");
    }

    /// An attribute already above its ceiling keeps its value: the ceiling
    /// stops its growth and never cuts it.
    #[test]
    fn an_attribute_above_its_ceiling_keeps_its_value() {
        let today = NaiveDate::from_ymd_opt(2026, 8, 3).unwrap();
        let mut keeper = eighteen_year_old_keeper(today);
        keeper.skills.goalkeeping.handling = 18.0;
        let before = SkillSnapshot::snapshot(&keeper);
        keeper.skills.goalkeeping.handling += 0.05;
        SkillCeilings::enforce(&mut keeper, today, &before);
        assert_eq!(keeper.skills.goalkeeping.handling, 18.0);
    }

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
        assert!((0.49..=0.55).contains(&weak_base));
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
