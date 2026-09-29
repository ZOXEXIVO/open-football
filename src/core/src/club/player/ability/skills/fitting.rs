use super::{PlayerPositionType, PlayerSkills};

impl PlayerSkills {
    /// Generated slots beside recorded ones are estimates, not free variables:
    /// unbounded, a gap they barely influence ran them to 1 or 20. Masking
    /// complete source profiles, estimation error is flat up to this band.
    const RECORDED_FLEX: f32 = 1.25;

    /// Fit a generated profile to a position-weighted CA, preserving relative
    /// strengths until an attribute reaches 1 or `cap`. Positive values in
    /// `recorded` are fixed; zero marks a generated slot, which then flexes
    /// only within `RECORDED_FLEX`. Absent (zero) attributes and match
    /// readiness are never scaled. Returns the closest reachable CA when
    /// fixed attributes, the flex or the cap make the requested ability
    /// impossible.
    pub fn fit_to_ability(
        &mut self,
        position: PlayerPositionType,
        target: u8,
        cap: f32,
        recorded: Option<&Self>,
    ) -> u8 {
        let cap = cap.clamp(1.0, 20.0);
        let target = target.clamp(1, 200);
        let original = *self;
        let scaled = |factor| original.scaled_attributes(factor, cap, recorded);
        *self = scaled(1.0);
        let current = self.calculate_ability_for_position(position);
        if current == target {
            return current;
        }

        // Scaling skills by target_CA/current_CA is not exact: the skill
        // scale starts at 1 and clipping makes the response piecewise linear.
        // Bisect the monotone response, always starting from the original
        // profile so intermediate clipping cannot erase its strengths.
        let (floor, ceiling) = match recorded {
            Some(_) => (1.0 / Self::RECORDED_FLEX, Self::RECORDED_FLEX),
            None => (0.0, cap),
        };
        let (mut low, mut high) = if current < target {
            (1.0, ceiling)
        } else {
            (floor, 1.0)
        };
        let boundary = scaled(if current < target { high } else { low });
        let reachable = boundary.calculate_ability_for_position(position);
        let boundary_score = boundary.ability_score_for_position(position);
        let aim = target as f32;
        if (current < target && boundary_score < aim) || (current > target && boundary_score > aim)
        {
            *self = boundary;
            return reachable;
        }

        let mut best_error = (self.ability_score_for_position(position) - aim).abs();
        for _ in 0..24 {
            let factor = (low + high) * 0.5;
            let candidate = scaled(factor);
            let score = candidate.ability_score_for_position(position);
            let error = (score - aim).abs();
            if error < best_error {
                *self = candidate;
                best_error = error;
            }
            if error < 0.49 {
                return target;
            }
            if score < aim {
                low = factor;
            } else {
                high = factor;
            }
        }
        self.calculate_ability_for_position(position)
    }

    /// Apply a generation ceiling to all attributes, including goalkeeping.
    /// Absent (zero) attributes, such as an outfielder's goalkeeping, stay absent.
    pub fn clamp_attributes(&mut self, cap: f32) {
        *self = self.scaled_attributes(1.0, cap.clamp(1.0, 20.0), None);
    }

    fn scaled_attributes(&self, factor: f32, cap: f32, recorded: Option<&Self>) -> Self {
        let mut result = *self;
        let empty = Self::default();
        let fixed = recorded.unwrap_or(&empty);
        macro_rules! scale {
            ($group:ident: $($field:ident),+) => {
                $(if fixed.$group.$field > 0.0 {
                    result.$group.$field = fixed.$group.$field;
                } else if self.$group.$field > 0.0 {
                    result.$group.$field =
                        (self.$group.$field.clamp(1.0, cap) * factor).clamp(1.0, cap);
                })+
            };
        }
        scale!(technical: corners, crossing, dribbling, finishing, first_touch, free_kicks,
            heading, long_shots, long_throws, marking, passing, penalty_taking, tackling,
            technique);
        scale!(mental: aggression, anticipation, bravery, composure, concentration, decisions,
            determination, flair, leadership, off_the_ball, positioning, teamwork, vision,
            work_rate);
        scale!(physical: acceleration, agility, balance, jumping, natural_fitness, pace,
            stamina, strength);
        scale!(goalkeeping: aerial_reach, command_of_area, communication, eccentricity,
            first_touch, handling, kicking, one_on_ones, passing, punching, reflexes,
            rushing_out, throwing);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cap_limits_reachable_ability_and_preserves_readiness() {
        let mut skills = PlayerSkills::flat_for_ability(150);
        skills.physical.match_readiness = 15.0;
        skills.fit_to_ability(PlayerPositionType::Goalkeeper, 200, 12.0, None);
        assert_eq!(
            skills.calculate_ability_for_position(PlayerPositionType::Goalkeeper),
            118
        );
        assert_eq!(skills.goalkeeping.reflexes, 12.0);
        assert_eq!(skills.physical.match_readiness, 15.0);
    }

    #[test]
    fn fitting_preserves_outfield_specialisms_and_absent_goalkeeping() {
        let mut skills = PlayerSkills::flat_for_ability(80);
        skills.technical.finishing = 18.0;
        skills.technical.tackling = 4.0;
        skills.goalkeeping = Default::default();
        skills.fit_to_ability(PlayerPositionType::Striker, 120, 20.0, None);
        assert_eq!(
            skills.calculate_ability_for_position(PlayerPositionType::Striker),
            120
        );
        assert!(skills.technical.finishing > skills.technical.tackling + 8.0);
        assert_eq!(skills.goalkeeping.average(), 0.0);
    }

    #[test]
    fn endpoint_abilities_do_not_flatten_a_profile() {
        for target in [1, 200] {
            let mut skills = PlayerSkills::flat_for_ability(100);
            skills.technical.finishing = 18.0;
            skills.technical.tackling = 4.0;
            skills.fit_to_ability(PlayerPositionType::Striker, target, 20.0, None);
            assert_eq!(
                skills.calculate_ability_for_position(PlayerPositionType::Striker),
                target
            );
            assert!(skills.technical.finishing > skills.technical.tackling + 4.0);
        }
    }

    #[test]
    fn generated_slots_beside_a_record_flex_without_reaching_the_scale_ends() {
        let generated = PlayerSkills::flat_for_ability(100);
        let mut recorded = PlayerSkills::flat_for_ability(160);
        recorded.technical.corners = 0.0;
        recorded.mental.determination = 0.0;
        for (target, factor) in [(60, 0.8), (200, 1.25)] {
            let mut skills = generated;
            let reached = skills.fit_to_ability(
                PlayerPositionType::MidfielderCenter,
                target,
                20.0,
                Some(&recorded),
            );
            assert_ne!(reached, target, "a record's own gap stays with the record");
            let expected = generated.technical.corners * factor;
            assert!((skills.technical.corners - expected).abs() < 1e-3);
            assert!((skills.mental.determination - expected).abs() < 1e-3);
            assert_eq!(skills.technical.passing, recorded.technical.passing);
        }
    }

    #[test]
    fn absent_goalkeeping_stays_absent_beside_a_recorded_outfielder() {
        let mut recorded = PlayerSkills::flat_for_ability(170);
        recorded.goalkeeping = Default::default();
        recorded.goalkeeping.first_touch = recorded.technical.first_touch;
        recorded.goalkeeping.passing = recorded.technical.passing;
        let mut skills = recorded;
        skills.fit_to_ability(PlayerPositionType::Striker, 180, 20.0, Some(&recorded));
        assert_eq!(skills.goalkeeping.reflexes, 0.0);
        assert_eq!(skills.goalkeeping.handling, 0.0);
        assert_eq!(skills.goalkeeping.passing, recorded.technical.passing);
    }
}
