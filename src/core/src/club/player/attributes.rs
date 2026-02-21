use crate::club::player::injury::InjuryType;

pub const CONDITION_MAX_VALUE: i16 = 10000;

#[derive(Debug, Clone, Copy, Default)]
pub struct PlayerAttributes {
    pub is_banned: bool,
    pub is_injured: bool,

    pub condition: i16,
    pub fitness: i16,
    pub jadedness: i16,

    pub weight: u8,
    pub height: u8,

    pub value: u32,

    //reputation
    pub current_reputation: i16,
    pub home_reputation: i16,
    pub world_reputation: i16,

    //ability
    pub current_ability: u8,
    pub potential_ability: u8,

    //international expirience
    pub international_apps: u16,
    pub international_goals: u16,

    pub under_21_international_apps: u16,
    pub under_21_international_goals: u16,

    // injury tracking
    pub injury_days_remaining: u16,
    pub injury_type: Option<InjuryType>,

    // injury proneness & recovery
    pub injury_proneness: u8,
    pub recovery_days_remaining: u16,
    pub last_injury_body_part: u8,
    pub injury_count: u8,

    // match load tracking
    pub days_since_last_match: u16,
}

impl PlayerAttributes {
    pub fn rest(&mut self, val: u16) {
        self.condition += val as i16;
        if self.condition > CONDITION_MAX_VALUE {
            self.condition = CONDITION_MAX_VALUE;
        }
    }

    pub fn condition_percentage(&self) -> u32 {
        (self.condition as f32 * 100.0 / CONDITION_MAX_VALUE as f32).floor() as u32
    }

    /// Set an injury on this player, calculating a random duration within the injury's range
    pub fn set_injury(&mut self, injury_type: InjuryType) {
        self.is_injured = true;
        self.injury_type = Some(injury_type);
        self.injury_days_remaining = injury_type.random_duration();
        self.last_injury_body_part = injury_type.body_part().to_u8();
        self.recovery_days_remaining = injury_type.recovery_days();
        self.injury_count = self.injury_count.saturating_add(1);
    }

    /// Decrement injury days by one. Returns true when the injury countdown reaches 0
    /// (transitioning to recovery phase).
    pub fn recover_injury_day(&mut self) -> bool {
        if self.injury_days_remaining > 0 {
            self.injury_days_remaining -= 1;
        }

        if self.injury_days_remaining == 0 && self.is_injured {
            // Transition to recovery phase — don't fully clear yet
            self.is_injured = false;
            self.injury_type = None;
            // recovery_days_remaining was already set in set_injury()
            return true;
        }

        false
    }

    /// Check if this player is in the post-injury recovery phase
    pub fn is_in_recovery(&self) -> bool {
        !self.is_injured && self.recovery_days_remaining > 0
    }

    /// Decrement recovery days. Returns true when fully fit.
    pub fn recover_recovery_day(&mut self) -> bool {
        if self.recovery_days_remaining > 0 {
            self.recovery_days_remaining -= 1;
        }

        if self.recovery_days_remaining == 0 {
            // Fully fit — clear last injury body part after full recovery
            // (we keep last_injury_body_part for a while to track recurring risk)
            return true;
        }

        false
    }

    /// Check if the current injury is serious (> 30 days remaining)
    pub fn is_injury_serious(&self) -> bool {
        self.is_injured && self.injury_days_remaining > 30
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::club::player::injury::InjuryType;

    fn default_attrs() -> PlayerAttributes {
        PlayerAttributes {
            is_banned: false,
            is_injured: false,
            condition: 5000,
            fitness: 8000,
            jadedness: 2000,
            weight: 75,
            height: 180,
            value: 1000000,
            current_reputation: 50,
            home_reputation: 60,
            world_reputation: 70,
            current_ability: 80,
            potential_ability: 90,
            international_apps: 10,
            international_goals: 5,
            under_21_international_apps: 15,
            under_21_international_goals: 7,
            injury_days_remaining: 0,
            injury_type: None,
            injury_proneness: 10,
            recovery_days_remaining: 0,
            last_injury_body_part: 0,
            injury_count: 0,
            days_since_last_match: 0,
        }
    }

    #[test]
    fn test_rest_increases_condition() {
        let mut player_attributes = default_attrs();
        player_attributes.rest(1000);
        assert_eq!(player_attributes.condition, 6000);
    }

    #[test]
    fn test_rest_does_not_exceed_max_condition() {
        let mut player_attributes = default_attrs();
        player_attributes.condition = 9500;
        player_attributes.rest(1000);
        assert_eq!(player_attributes.condition, CONDITION_MAX_VALUE);
    }

    #[test]
    fn test_condition_percentage() {
        let mut player_attributes = default_attrs();
        player_attributes.condition = 7500;
        let condition_percentage = player_attributes.condition_percentage();
        assert_eq!(condition_percentage, 75);
    }

    #[test]
    fn test_condition_percentage_rounding() {
        let mut player_attributes = default_attrs();
        player_attributes.condition = 7499;
        let condition_percentage = player_attributes.condition_percentage();
        assert_eq!(condition_percentage, 74);
    }

    #[test]
    fn test_set_injury() {
        let mut attrs = default_attrs();
        attrs.set_injury(InjuryType::Bruise);
        assert!(attrs.is_injured);
        assert_eq!(attrs.injury_type, Some(InjuryType::Bruise));
        assert!(attrs.injury_days_remaining >= 3 && attrs.injury_days_remaining <= 7);
        assert!(attrs.recovery_days_remaining >= 3 && attrs.recovery_days_remaining <= 5);
        assert_eq!(attrs.injury_count, 1);
        assert!(attrs.last_injury_body_part > 0);
    }

    #[test]
    fn test_set_injury_increments_count() {
        let mut attrs = default_attrs();
        attrs.set_injury(InjuryType::Bruise);
        assert_eq!(attrs.injury_count, 1);
        attrs.is_injured = false;
        attrs.injury_days_remaining = 0;
        attrs.recovery_days_remaining = 0;
        attrs.set_injury(InjuryType::Cramp);
        assert_eq!(attrs.injury_count, 2);
    }

    #[test]
    fn test_recover_injury_day_transitions_to_recovery() {
        let mut attrs = default_attrs();
        attrs.set_injury(InjuryType::Cramp);
        let saved_recovery = attrs.recovery_days_remaining;

        // Burn through injury days
        while attrs.injury_days_remaining > 1 {
            assert!(!attrs.recover_injury_day());
            assert!(attrs.is_injured);
        }

        // Last day — transitions to recovery
        assert!(attrs.recover_injury_day());
        assert!(!attrs.is_injured);
        assert!(attrs.injury_type.is_none());
        assert_eq!(attrs.recovery_days_remaining, saved_recovery);
    }

    #[test]
    fn test_is_in_recovery() {
        let mut attrs = default_attrs();
        assert!(!attrs.is_in_recovery());

        attrs.recovery_days_remaining = 5;
        assert!(attrs.is_in_recovery());

        attrs.is_injured = true;
        assert!(!attrs.is_in_recovery());
    }

    #[test]
    fn test_recover_recovery_day() {
        let mut attrs = default_attrs();
        attrs.recovery_days_remaining = 2;

        assert!(!attrs.recover_recovery_day());
        assert_eq!(attrs.recovery_days_remaining, 1);

        assert!(attrs.recover_recovery_day());
        assert_eq!(attrs.recovery_days_remaining, 0);
    }

    #[test]
    fn test_is_injury_serious() {
        let mut attrs = default_attrs();
        attrs.is_injured = true;
        attrs.injury_days_remaining = 31;
        assert!(attrs.is_injury_serious());

        attrs.injury_days_remaining = 30;
        assert!(!attrs.is_injury_serious());

        attrs.is_injured = false;
        attrs.injury_days_remaining = 50;
        assert!(!attrs.is_injury_serious());
    }
}
