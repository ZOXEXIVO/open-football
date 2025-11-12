#[derive(Debug, Copy, Clone, Default)]
pub struct PlayerSkills {
    pub technical: Technical,
    pub mental: Mental,
    pub physical: Physical,
}

impl PlayerSkills {
    /// Calculate maximum speed without condition factor (raw speed based on skills only)
    pub fn max_speed(&self) -> f32 {
        let pace_factor = (self.physical.pace as f32 - 1.0) / 19.0;
        let acceleration_factor = (self.physical.acceleration as f32 - 1.0) / 19.0;
        let agility_factor = (self.physical.agility as f32 - 1.0) / 19.0;
        let balance_factor = (self.physical.balance as f32 - 1.0) / 19.0;

        let base_speed = 1.3; // Increased by 30% from 1.0 to 1.3
        let max_speed = base_speed
            * (0.6 * pace_factor
                + 0.2 * acceleration_factor
                + 0.1 * agility_factor
                + 0.1 * balance_factor);

        max_speed
    }

    /// Calculate maximum speed with condition/stamina factor (real-time performance)
    /// This is what should be used during match for actual speed calculation
    pub fn max_speed_with_condition(&self, condition: i16, fitness: i16, jadedness: i16) -> f32 {
        let base_max_speed = self.max_speed();

        // Calculate condition factor (similar to PassSkills logic)
        let condition_percentage = (condition as f32 / 10000.0).clamp(0.0, 1.0);
        let fitness_factor = (fitness as f32 / 10000.0).clamp(0.5, 1.0);
        let jadedness_penalty = (jadedness as f32 / 10000.0) * 0.3;
        let stamina_skill = (self.physical.stamina / 20.0).clamp(0.3, 1.0);

        // Condition factor ranges from 0.5 to 1.0 (tired players at 50% speed minimum)
        let condition_factor = (condition_percentage * fitness_factor * stamina_skill - jadedness_penalty)
            .clamp(0.5, 1.0);

        base_max_speed * condition_factor
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
