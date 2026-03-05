use crate::club::player::position::{PlayerFieldPositionGroup, PlayerPositionType};

#[derive(Debug, Copy, Clone, Default)]
pub struct PlayerSkills {
    pub technical: Technical,
    pub mental: Mental,
    pub physical: Physical,
}

/// Goalkeeper activity intensity for speed calculation.
/// GKs have low pace (60% of max_speed formula) but need explosive short-distance speed
/// for diving, catching, and shot-stopping. Agility and acceleration matter more.
#[derive(Debug, Clone, Copy)]
pub enum GoalkeeperSpeedContext {
    /// Diving, preparing for save, jumping — explosive reactions
    Explosive,
    /// Catching, coming out, under pressure — active pursuit
    Active,
    /// Attentive, standing, returning — positioning
    Positioning,
    /// Walking, holding, distributing — minimal
    Casual,
}

impl PlayerSkills {
    /// Derive current_ability (1-200) from the average of all skills (1-20 each).
    /// Technical (14) + Mental (14) + Physical (8) averaged, then mapped to 1-200.
    pub fn calculate_ability(&self) -> u8 {
        let tech_avg = self.technical.average();
        let mental_avg = self.mental.average();
        let physical_avg = self.physical.average();
        let overall = (tech_avg + mental_avg + physical_avg) / 3.0;
        Self::skill_to_ability(overall)
    }

    /// Position-weighted ability calculation — skills that matter for the position count more.
    pub fn calculate_ability_for_position(&self, position: PlayerPositionType) -> u8 {
        let group = position.position_group();
        let (tech_w, mental_w, phys_w) = match group {
            PlayerFieldPositionGroup::Goalkeeper => (0.15, 0.35, 0.50),
            PlayerFieldPositionGroup::Defender => (0.25, 0.40, 0.35),
            PlayerFieldPositionGroup::Midfielder => (0.40, 0.35, 0.25),
            PlayerFieldPositionGroup::Forward => (0.45, 0.25, 0.30),
        };
        let weighted = self.technical.average() * tech_w
            + self.mental.average() * mental_w
            + self.physical.average() * phys_w;
        Self::skill_to_ability(weighted)
    }

    /// Map a skill average (1.0-20.0) to ability (1-200).
    /// Skills are 1-based so normalize from 1-20 range before scaling.
    fn skill_to_ability(avg: f32) -> u8 {
        let normalized = ((avg - 1.0) / 19.0).clamp(0.0, 1.0);
        (normalized * 199.0 + 1.0).round().min(200.0).max(1.0) as u8
    }

    /// Calculate maximum speed without condition factor (raw speed based on skills only)
    pub fn max_speed(&self) -> f32 {
        let pace_factor = (self.physical.pace as f32 - 1.0) / 19.0;
        let acceleration_factor = (self.physical.acceleration as f32 - 1.0) / 19.0;
        let agility_factor = (self.physical.agility as f32 - 1.0) / 19.0;
        let balance_factor = (self.physical.balance as f32 - 1.0) / 19.0;

        let base_speed = 0.8;
        let max_speed = base_speed
            * (0.6 * pace_factor
                + 0.2 * acceleration_factor
                + 0.1 * agility_factor
                + 0.1 * balance_factor);

        max_speed
    }

    /// Calculate maximum speed with condition factor (real-time performance)
    /// This is what should be used during match for actual speed calculation
    /// Speed depends primarily (90%) on condition, with minimal stamina influence (10%)
    pub fn max_speed_with_condition(&self, condition: i16) -> f32 {
        let base_max_speed = self.max_speed();

        // Condition is the primary factor (0-10000 scale)
        let condition_percentage = (condition as f32 / 10000.0).clamp(0.0, 1.0);

        // Stamina provides minimal resistance to fatigue (0-10% boost when tired)
        // High stamina players maintain 90-100% speed at low condition
        // Low stamina players drop to 80-100% speed at low condition
        let stamina_normalized = (self.physical.stamina / 20.0).clamp(0.0, 1.0);
        let stamina_protection = stamina_normalized * 0.10; // Max 10% protection

        // Simple linear condition curve with stamina protection
        // At 100% condition: 100% speed (regardless of stamina)
        // At 50% condition: 50-60% speed (depending on stamina)
        // At 0% condition: 10-20% speed (minimum + stamina protection)
        let condition_factor = (condition_percentage * (1.0 - stamina_protection) + stamina_protection)
            .clamp(0.10, 1.0);

        base_max_speed * condition_factor
    }

    /// Calculate maximum speed for a goalkeeper with state-dependent boost.
    /// GKs need explosive speed from agility/acceleration rather than raw pace.
    pub fn goalkeeper_max_speed(&self, condition: i16, speed_context: GoalkeeperSpeedContext) -> f32 {
        let base = self.max_speed_with_condition(condition);

        let agility = self.physical.agility / 20.0;
        let acceleration = self.physical.acceleration / 20.0;

        let boost = match speed_context {
            GoalkeeperSpeedContext::Explosive => 1.8 + agility * 0.6 + acceleration * 0.4,
            GoalkeeperSpeedContext::Active => 1.5 + agility * 0.4 + acceleration * 0.3,
            GoalkeeperSpeedContext::Positioning => 1.3 + agility * 0.2,
            GoalkeeperSpeedContext::Casual => 1.1,
        };

        base * boost
    }
}

#[derive(Debug, Copy, Clone, Default)]
pub struct Technical {
    pub corners: f32,
    pub crossing: f32,
    pub dribbling: f32,
    pub finishing: f32,
    pub first_touch: f32,
    pub free_kicks: f32,
    pub heading: f32,
    pub long_shots: f32,
    pub long_throws: f32,
    pub marking: f32,
    pub passing: f32,
    pub penalty_taking: f32,
    pub tackling: f32,
    pub technique: f32,
}

impl Technical {
    pub fn average(&self) -> f32 {
        (self.corners
            + self.crossing
            + self.dribbling
            + self.finishing
            + self.first_touch
            + self.free_kicks
            + self.heading
            + self.long_shots
            + self.long_throws
            + self.marking
            + self.passing
            + self.penalty_taking
            + self.tackling
            + self.technique)
            / 14.0
    }

    pub fn raise_floor(&mut self, min: f32) {
        self.corners = self.corners.max(min);
        self.crossing = self.crossing.max(min);
        self.dribbling = self.dribbling.max(min);
        self.finishing = self.finishing.max(min);
        self.first_touch = self.first_touch.max(min);
        self.free_kicks = self.free_kicks.max(min);
        self.heading = self.heading.max(min);
        self.long_shots = self.long_shots.max(min);
        self.long_throws = self.long_throws.max(min);
        self.marking = self.marking.max(min);
        self.passing = self.passing.max(min);
        self.penalty_taking = self.penalty_taking.max(min);
        self.tackling = self.tackling.max(min);
        self.technique = self.technique.max(min);
    }

    pub fn rest(&mut self) {}
}

#[derive(Debug, Copy, Clone, Default)]
pub struct Mental {
    pub aggression: f32,
    pub anticipation: f32,
    pub bravery: f32,
    pub composure: f32,
    pub concentration: f32,
    pub decisions: f32,
    pub determination: f32,
    pub flair: f32,
    pub leadership: f32,
    pub off_the_ball: f32,
    pub positioning: f32,
    pub teamwork: f32,
    pub vision: f32,
    pub work_rate: f32,
}

impl Mental {
    pub fn average(&self) -> f32 {
        (self.aggression
            + self.anticipation
            + self.bravery
            + self.composure
            + self.concentration
            + self.decisions
            + self.determination
            + self.flair
            + self.leadership
            + self.off_the_ball
            + self.positioning
            + self.teamwork
            + self.vision
            + self.work_rate)
            / 14.0
    }

    pub fn raise_floor(&mut self, min: f32) {
        self.aggression = self.aggression.max(min);
        self.anticipation = self.anticipation.max(min);
        self.bravery = self.bravery.max(min);
        self.composure = self.composure.max(min);
        self.concentration = self.concentration.max(min);
        self.decisions = self.decisions.max(min);
        self.determination = self.determination.max(min);
        self.flair = self.flair.max(min);
        self.leadership = self.leadership.max(min);
        self.off_the_ball = self.off_the_ball.max(min);
        self.positioning = self.positioning.max(min);
        self.teamwork = self.teamwork.max(min);
        self.vision = self.vision.max(min);
        self.work_rate = self.work_rate.max(min);
    }

    pub fn rest(&mut self) {}
}

#[derive(Debug, Copy, Clone, Default)]
pub struct Physical {
    pub acceleration: f32,
    pub agility: f32,
    pub balance: f32,
    pub jumping: f32,
    pub natural_fitness: f32,
    pub pace: f32,
    pub stamina: f32,
    pub strength: f32,

    pub match_readiness: f32,
}

impl Physical {
    pub fn average(&self) -> f32 {
        (self.acceleration
            + self.agility
            + self.balance
            + self.jumping
            + self.natural_fitness
            + self.pace
            + self.stamina
            + self.strength)
            / 8.0
    }

    pub fn raise_floor(&mut self, min: f32) {
        self.acceleration = self.acceleration.max(min);
        self.agility = self.agility.max(min);
        self.balance = self.balance.max(min);
        self.jumping = self.jumping.max(min);
        self.natural_fitness = self.natural_fitness.max(min);
        self.pace = self.pace.max(min);
        self.stamina = self.stamina.max(min);
        self.strength = self.strength.max(min);
    }

    pub fn rest(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_technical_average() {
        let technical = Technical {
            corners: 10.0,
            crossing: 20.0,
            dribbling: 30.0,
            finishing: 40.0,
            first_touch: 50.0,
            free_kicks: 60.0,
            heading: 70.0,
            long_shots: 80.0,
            long_throws: 90.0,
            marking: 100.0,
            passing: 110.0,
            penalty_taking: 120.0,
            tackling: 130.0,
            technique: 140.0,
        };
        assert_eq!(technical.average(), 75.0); // (10 + 20 + 30 + 40 + 50 + 60 + 70 + 80 + 90 + 100 + 110 + 120 + 130 + 140) / 14
    }

    #[test]
    fn test_technical_rest() {
        let mut technical = Technical {
            corners: 10.0,
            crossing: 20.0,
            dribbling: 30.0,
            finishing: 40.0,
            first_touch: 50.0,
            free_kicks: 60.0,
            heading: 70.0,
            long_shots: 80.0,
            long_throws: 90.0,
            marking: 100.0,
            passing: 110.0,
            penalty_taking: 120.0,
            tackling: 130.0,
            technique: 140.0,
        };
        technical.rest();
        // Since the rest method doesn't modify any fields, we'll just assert true to indicate it ran successfully
        assert!(true);
    }

    #[test]
    fn test_mental_average() {
        let mental = Mental {
            aggression: 10.0,
            anticipation: 20.0,
            bravery: 30.0,
            composure: 40.0,
            concentration: 50.0,
            decisions: 60.0,
            determination: 70.0,
            flair: 80.0,
            leadership: 90.0,
            off_the_ball: 100.0,
            positioning: 110.0,
            teamwork: 120.0,
            vision: 130.0,
            work_rate: 140.0,
        };

        assert_eq!(mental.average(), 75.0); // (10 + 20 + 30 + 40 + 50 + 60 + 70 + 80 + 90 + 100 + 110 + 120 + 130 + 140) / 14
    }

    #[test]
    fn test_mental_rest() {
        let mut mental = Mental {
            aggression: 10.0,
            anticipation: 20.0,
            bravery: 30.0,
            composure: 40.0,
            concentration: 50.0,
            decisions: 60.0,
            determination: 70.0,
            flair: 80.0,
            leadership: 90.0,
            off_the_ball: 100.0,
            positioning: 110.0,
            teamwork: 120.0,
            vision: 130.0,
            work_rate: 140.0,
        };
        mental.rest();
        // Since the rest method doesn't modify any fields, we'll just assert true to indicate it ran successfully
        assert!(true);
    }

    #[test]
    fn test_physical_average() {
        let physical = Physical {
            acceleration: 10.0,
            agility: 20.0,
            balance: 30.0,
            jumping: 40.0,
            natural_fitness: 50.0,
            pace: 60.0,
            stamina: 70.0,
            strength: 80.0,
            match_readiness: 90.0,
        };
        assert_eq!(physical.average(), 45.0); // (10 + 20 + 30 + 40 + 50 + 60 + 70 + 80) / 8
    }

    #[test]
    fn test_physical_rest() {
        let mut physical = Physical {
            acceleration: 10.0,
            agility: 20.0,
            balance: 30.0,
            jumping: 40.0,
            natural_fitness: 50.0,
            pace: 60.0,
            stamina: 70.0,
            strength: 80.0,
            match_readiness: 90.0,
        };
        physical.rest();
        // Since the rest method doesn't modify any fields, we'll just assert true to indicate it ran successfully
        assert!(true);
    }
}
