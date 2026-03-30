use crate::context::GlobalContext;
use crate::{Person, Player, PlayerFieldPositionGroup};
use super::ClubAcademy;

// ───────────────────────────────────────────────────────────────────────────────
// Youth Development Phases — mirrors real-life academy structures
// ───────────────────────────────────────────────────────────────────────────────

/// Real academies organize players into age-based development phases, each with
/// distinct training philosophy, intensity, and focus areas.
#[derive(Debug, Clone, Copy, PartialEq)]
enum YouthDevelopmentPhase {
    /// Ages 8-11: Fun, basic motor skills, ball mastery, small-sided games.
    /// Training is play-based. No positional specialization yet.
    Foundation,
    /// Ages 12-14: Technical refinement, tactical introduction, positional awareness.
    /// Players begin to specialize and play 11v11.
    Development,
    /// Ages 15+: Physical conditioning, competitive preparation, match intensity.
    /// Bridging the gap to professional football.
    Professional,
}

impl YouthDevelopmentPhase {
    fn from_age(age: u8) -> Self {
        match age {
            0..=11 => YouthDevelopmentPhase::Foundation,
            12..=14 => YouthDevelopmentPhase::Development,
            _ => YouthDevelopmentPhase::Professional,
        }
    }
}

impl ClubAcademy {
    /// Apply weekly training to all academy players based on their development phase.
    ///
    /// Real academies train 3-6 times per week depending on age group and resources.
    /// We aggregate into a single weekly tick with gains scaled by session count.
    pub(super) fn train_academy_players(&mut self, ctx: &GlobalContext<'_>) {
        if !ctx.simulation.is_week_beginning() {
            return;
        }

        let date = ctx.simulation.date.date();
        let youth_facility_quality = ctx.club_facilities_youth();

        // Coaching quality: academy level (1-10) → 0.3-1.0
        // Even the weakest academy provides basic coaching
        let base_coaching = 0.2 + (self.level as f32 / 10.0) * 0.8;

        // If academy has dedicated coaches, blend their quality in
        let coaching_quality = self.effective_coaching_quality(base_coaching);

        // Facility modifier: 0.6-1.3 range (poor facilities hamper development)
        let facility_mod = 0.55 + youth_facility_quality * 0.75;

        // Sessions per week by academy level → normalized to base of 4
        let sessions = self.settings.sessions_per_week(self.level) as f32;
        let session_mult = sessions / 4.0;

        for player in &mut self.players.players {
            if player.player_attributes.is_injured {
                continue;
            }

            let age = player.age(date);
            let phase = YouthDevelopmentPhase::from_age(age);

            // Player personality affects training absorption
            let personality_mult = Self::personality_training_factor(player);

            // Combined multiplier for all gains
            let base_mult = coaching_quality * facility_mod * personality_mult * session_mult;

            // Random variance per player per week: ±15%
            let variance = 0.85 + rand::random::<f32>() * 0.30;
            let gain_mult = base_mult * variance;

            let is_gk = player.position().position_group() == PlayerFieldPositionGroup::Goalkeeper;

            match phase {
                YouthDevelopmentPhase::Foundation => {
                    if is_gk {
                        Self::train_foundation_gk(player, gain_mult);
                    } else {
                        Self::train_foundation(player, gain_mult);
                    }
                }
                YouthDevelopmentPhase::Development => {
                    if is_gk {
                        Self::train_development_gk(player, gain_mult);
                    } else {
                        Self::train_development(player, gain_mult);
                    }
                }
                YouthDevelopmentPhase::Professional => {
                    if is_gk {
                        Self::train_professional_gk(player, gain_mult);
                    } else {
                        Self::train_professional(player, gain_mult);
                    }
                }
            }

            // Puberty growth spurts (ages 13-15): physical gains with temporary
            // coordination disruption — a well-known phenomenon in youth development
            if (13..=15).contains(&age) {
                Self::apply_growth_spurt_effects(player);
            }

            // Enforce age-based and PA-based skill ceilings, then recalculate ability
            Self::enforce_skill_ceilings(player, age);
            let pos = player.position();
            player.player_attributes.current_ability =
                player.skills.calculate_ability_for_position(pos);
        }
    }

    /// Blend academy-level coaching with actual staff quality when coaches exist.
    fn effective_coaching_quality(&self, base: f32) -> f32 {
        if self.staff.staffs.is_empty() {
            return base;
        }

        // Average coaching ability of academy staff (technical + tactical + fitness) / 60
        let staff_quality: f32 = self
            .staff
            .staffs
            .iter()
            .map(|s| {
                (s.staff_attributes.coaching.technical as f32
                    + s.staff_attributes.coaching.tactical as f32
                    + s.staff_attributes.coaching.fitness as f32)
                    / 60.0
            })
            .sum::<f32>()
            / self.staff.staffs.len() as f32;

        // 40% base (academy infrastructure), 60% actual coaches
        base * 0.4 + staff_quality * 0.6
    }

    /// Personality multiplier for training absorption: 0.5-1.4
    fn personality_training_factor(player: &Player) -> f32 {
        let professionalism = player.attributes.professionalism;
        let determination = player.skills.mental.determination;
        let work_rate = player.skills.mental.work_rate;

        // Weighted: professionalism matters most for youth development
        let weighted = professionalism * 0.45 + determination * 0.30 + work_rate * 0.25;
        0.4 + (weighted / 20.0) * 1.0
    }

    // ───────────────────────────────────────────────────────────────────────
    // Foundation Phase (ages 8-11): Play-based learning, ball mastery
    // ───────────────────────────────────────────────────────────────────────

    /// Outfield foundation training: technique through play, basic game sense.
    fn train_foundation(player: &mut Player, m: f32) {
        // Technical: primary focus — ball mastery through small-sided games
        player.skills.technical.first_touch += 0.035 * m;
        player.skills.technical.dribbling += 0.030 * m;
        player.skills.technical.technique += 0.025 * m;
        player.skills.technical.passing += 0.020 * m;
        player.skills.technical.crossing += 0.005 * m;

        // Mental: game sense develops naturally through play
        player.skills.mental.teamwork += 0.015 * m;
        player.skills.mental.decisions += 0.010 * m;
        player.skills.mental.off_the_ball += 0.010 * m;
        player.skills.mental.flair += 0.008 * m;

        // Physical: coordination, not strength
        player.skills.physical.agility += 0.015 * m;
        player.skills.physical.balance += 0.015 * m;
        player.skills.physical.acceleration += 0.010 * m;
    }

    /// GK foundation: handling basics, bravery, footwork
    fn train_foundation_gk(player: &mut Player, m: f32) {
        player.skills.goalkeeping.handling += 0.030 * m;
        player.skills.goalkeeping.reflexes += 0.020 * m;
        player.skills.goalkeeping.kicking += 0.015 * m;
        player.skills.goalkeeping.first_touch += 0.015 * m;
        player.skills.goalkeeping.throwing += 0.010 * m;

        player.skills.mental.bravery += 0.015 * m;
        player.skills.mental.concentration += 0.010 * m;
        player.skills.mental.decisions += 0.008 * m;

        player.skills.physical.agility += 0.020 * m;
        player.skills.physical.balance += 0.015 * m;
    }

    // ───────────────────────────────────────────────────────────────────────
    // Development Phase (ages 12-14): Technical refinement, tactical awareness
    // ───────────────────────────────────────────────────────────────────────

    /// Outfield development: position-specific technique, tactical understanding.
    fn train_development(player: &mut Player, m: f32) {
        let group = player.position().position_group();

        // Core technique everyone needs
        player.skills.technical.first_touch += 0.025 * m;
        player.skills.technical.passing += 0.025 * m;
        player.skills.technical.technique += 0.020 * m;

        // Position-specific technical development
        match group {
            PlayerFieldPositionGroup::Defender => {
                player.skills.technical.tackling += 0.025 * m;
                player.skills.technical.marking += 0.020 * m;
                player.skills.technical.heading += 0.015 * m;
            }
            PlayerFieldPositionGroup::Midfielder => {
                player.skills.technical.passing += 0.015 * m;
                player.skills.technical.crossing += 0.015 * m;
                player.skills.technical.dribbling += 0.015 * m;
            }
            PlayerFieldPositionGroup::Forward => {
                player.skills.technical.finishing += 0.025 * m;
                player.skills.technical.dribbling += 0.020 * m;
                player.skills.technical.long_shots += 0.010 * m;
            }
            _ => {}
        }

        // Mental: tactical understanding through 11v11
        player.skills.mental.positioning += 0.025 * m;
        player.skills.mental.concentration += 0.020 * m;
        player.skills.mental.decisions += 0.020 * m;
        player.skills.mental.anticipation += 0.015 * m;
        player.skills.mental.teamwork += 0.015 * m;
        player.skills.mental.vision += 0.010 * m;

        // Physical: coordination and basic fitness
        player.skills.physical.agility += 0.015 * m;
        player.skills.physical.balance += 0.015 * m;
        player.skills.physical.pace += 0.010 * m;
        player.skills.physical.stamina += 0.010 * m;
    }

    /// GK development: shot-stopping, distribution, command.
    fn train_development_gk(player: &mut Player, m: f32) {
        player.skills.goalkeeping.handling += 0.025 * m;
        player.skills.goalkeeping.reflexes += 0.025 * m;
        player.skills.goalkeeping.one_on_ones += 0.020 * m;
        player.skills.goalkeeping.kicking += 0.020 * m;
        player.skills.goalkeeping.passing += 0.015 * m;
        player.skills.goalkeeping.communication += 0.015 * m;
        player.skills.goalkeeping.aerial_reach += 0.010 * m;
        player.skills.goalkeeping.command_of_area += 0.010 * m;

        player.skills.mental.positioning += 0.025 * m;
        player.skills.mental.concentration += 0.020 * m;
        player.skills.mental.composure += 0.015 * m;
        player.skills.mental.decisions += 0.015 * m;

        player.skills.physical.agility += 0.020 * m;
        player.skills.physical.jumping += 0.015 * m;
        player.skills.physical.acceleration += 0.010 * m;
    }

    // ───────────────────────────────────────────────────────────────────────
    // Professional Phase (ages 15+): Physical maturation, competitive prep
    // ───────────────────────────────────────────────────────────────────────

    /// Outfield professional: physical development window, advanced tactics, mental resilience.
    fn train_professional(player: &mut Player, m: f32) {
        let group = player.position().position_group();

        // Technical: advanced, position-specific mastery
        player.skills.technical.technique += 0.015 * m;
        player.skills.technical.first_touch += 0.015 * m;

        match group {
            PlayerFieldPositionGroup::Defender => {
                player.skills.technical.tackling += 0.020 * m;
                player.skills.technical.marking += 0.020 * m;
                player.skills.technical.heading += 0.020 * m;
                player.skills.technical.passing += 0.010 * m;
            }
            PlayerFieldPositionGroup::Midfielder => {
                player.skills.technical.passing += 0.020 * m;
                player.skills.technical.crossing += 0.015 * m;
                player.skills.technical.dribbling += 0.015 * m;
                player.skills.technical.long_shots += 0.010 * m;
            }
            PlayerFieldPositionGroup::Forward => {
                player.skills.technical.finishing += 0.025 * m;
                player.skills.technical.dribbling += 0.015 * m;
                player.skills.technical.heading += 0.010 * m;
                player.skills.technical.long_shots += 0.015 * m;
            }
            _ => {}
        }

        // Mental: competitive edge and game intelligence
        player.skills.mental.composure += 0.020 * m;
        player.skills.mental.concentration += 0.020 * m;
        player.skills.mental.decisions += 0.020 * m;
        player.skills.mental.positioning += 0.020 * m;
        player.skills.mental.anticipation += 0.015 * m;
        player.skills.mental.determination += 0.010 * m;
        player.skills.mental.work_rate += 0.010 * m;

        // Physical: main physical development window
        player.skills.physical.strength += 0.025 * m;
        player.skills.physical.stamina += 0.025 * m;
        player.skills.physical.pace += 0.020 * m;
        player.skills.physical.acceleration += 0.015 * m;
        player.skills.physical.jumping += 0.015 * m;
        player.skills.physical.natural_fitness += 0.010 * m;
        player.skills.physical.agility += 0.010 * m;
    }

    /// GK professional: elite shot-stopping, distribution, aerial dominance, match mentality.
    fn train_professional_gk(player: &mut Player, m: f32) {
        player.skills.goalkeeping.handling += 0.020 * m;
        player.skills.goalkeeping.reflexes += 0.020 * m;
        player.skills.goalkeeping.one_on_ones += 0.020 * m;
        player.skills.goalkeeping.aerial_reach += 0.020 * m;
        player.skills.goalkeeping.command_of_area += 0.020 * m;
        player.skills.goalkeeping.rushing_out += 0.015 * m;
        player.skills.goalkeeping.punching += 0.015 * m;
        player.skills.goalkeeping.kicking += 0.015 * m;
        player.skills.goalkeeping.passing += 0.015 * m;
        player.skills.goalkeeping.communication += 0.015 * m;
        player.skills.goalkeeping.throwing += 0.010 * m;

        player.skills.mental.positioning += 0.020 * m;
        player.skills.mental.concentration += 0.020 * m;
        player.skills.mental.composure += 0.020 * m;
        player.skills.mental.decisions += 0.015 * m;
        player.skills.mental.anticipation += 0.015 * m;
        player.skills.mental.leadership += 0.010 * m;

        player.skills.physical.strength += 0.020 * m;
        player.skills.physical.jumping += 0.020 * m;
        player.skills.physical.agility += 0.015 * m;
        player.skills.physical.acceleration += 0.010 * m;
        player.skills.physical.stamina += 0.010 * m;
    }

    // ───────────────────────────────────────────────────────────────────────
    // Growth Spurts & Skill Clamping
    // ───────────────────────────────────────────────────────────────────────

    /// During puberty (ages 13-15), players may experience growth spurts.
    /// Rapid height/muscle gain temporarily disrupts coordination and balance —
    /// a well-documented phenomenon in youth sports science.
    fn apply_growth_spurt_effects(player: &mut Player) {
        // ~12% chance per week of a noticeable growth effect
        if rand::random::<f32>() > 0.12 {
            return;
        }

        let intensity = 0.01 + rand::random::<f32>() * 0.02;

        // Physical gains from growth
        player.skills.physical.strength += intensity;
        player.skills.physical.jumping += intensity * 0.6;

        // Temporary coordination cost (smaller than gains, recovers via training)
        let coord_cost = intensity * 0.35;
        player.skills.physical.agility -= coord_cost;
        player.skills.physical.balance -= coord_cost;
    }

    /// Enforce age-based and PA-based skill ceilings.
    ///
    /// Youth players develop gradually — a 14-year-old even at the best
    /// academy should have modest skills compared to professionals:
    ///   age  8 → 3.0    age 12 →  6.0    age 16 → 10.0
    ///   age  9 → 3.5    age 13 →  7.0    age 17 → 11.0
    ///   age 10 → 4.0    age 14 →  8.0    age 18 → 12.0
    ///   age 11 → 5.0    age 15 →  9.0
    ///
    /// The effective ceiling is the LOWER of the age cap and the PA-derived cap.
    fn enforce_skill_ceilings(player: &mut Player, age: u8) {
        let age_cap = match age {
            0..=8 => 3.0_f32,
            9 => 3.5,
            10 => 4.0,
            11 => 5.0,
            12 => 6.0,
            13 => 7.0,
            14 => 8.0,
            15 => 9.0,
            16 => 10.0,
            17 => 11.0,
            _ => 12.0,
        };

        // PA-based ceiling: PA 200 → 20.0, PA 100 → 10.0
        let pa = player.player_attributes.potential_ability as f32;
        let pa_cap = (pa / 200.0 * 20.0).clamp(1.0, 20.0);

        let cap = age_cap.min(pa_cap);

        let clamp = |v: f32| -> f32 { v.clamp(1.0, cap) };

        let t = &mut player.skills.technical;
        t.corners = clamp(t.corners);
        t.crossing = clamp(t.crossing);
        t.dribbling = clamp(t.dribbling);
        t.finishing = clamp(t.finishing);
        t.first_touch = clamp(t.first_touch);
        t.free_kicks = clamp(t.free_kicks);
        t.heading = clamp(t.heading);
        t.long_shots = clamp(t.long_shots);
        t.long_throws = clamp(t.long_throws);
        t.marking = clamp(t.marking);
        t.passing = clamp(t.passing);
        t.penalty_taking = clamp(t.penalty_taking);
        t.tackling = clamp(t.tackling);
        t.technique = clamp(t.technique);

        let m = &mut player.skills.mental;
        m.aggression = clamp(m.aggression);
        m.anticipation = clamp(m.anticipation);
        m.bravery = clamp(m.bravery);
        m.composure = clamp(m.composure);
        m.concentration = clamp(m.concentration);
        m.decisions = clamp(m.decisions);
        m.determination = clamp(m.determination);
        m.flair = clamp(m.flair);
        m.leadership = clamp(m.leadership);
        m.off_the_ball = clamp(m.off_the_ball);
        m.positioning = clamp(m.positioning);
        m.teamwork = clamp(m.teamwork);
        m.vision = clamp(m.vision);
        m.work_rate = clamp(m.work_rate);

        let p = &mut player.skills.physical;
        p.acceleration = clamp(p.acceleration);
        p.agility = clamp(p.agility);
        p.balance = clamp(p.balance);
        p.jumping = clamp(p.jumping);
        p.natural_fitness = clamp(p.natural_fitness);
        p.pace = clamp(p.pace);
        p.stamina = clamp(p.stamina);
        p.strength = clamp(p.strength);

        let g = &mut player.skills.goalkeeping;
        g.aerial_reach = clamp(g.aerial_reach);
        g.command_of_area = clamp(g.command_of_area);
        g.communication = clamp(g.communication);
        g.eccentricity = clamp(g.eccentricity);
        g.first_touch = clamp(g.first_touch);
        g.handling = clamp(g.handling);
        g.kicking = clamp(g.kicking);
        g.one_on_ones = clamp(g.one_on_ones);
        g.passing = clamp(g.passing);
        g.punching = clamp(g.punching);
        g.reflexes = clamp(g.reflexes);
        g.rushing_out = clamp(g.rushing_out);
        g.throwing = clamp(g.throwing);
    }
}
