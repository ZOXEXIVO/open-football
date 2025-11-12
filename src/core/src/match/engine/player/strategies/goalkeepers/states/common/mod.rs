use crate::r#match::StateChangeResult;
use crate::r#match::{ConditionContext, StateProcessingContext, StateProcessingHandler};
use nalgebra::Vector3;

#[derive(Default)]
pub struct GoalkeeperCommonState {}

impl StateProcessingHandler for GoalkeeperCommonState {
    fn try_fast(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, _ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        Some(Vector3::new(0.0, 0.0, 0.0))
    }

    fn process_conditions(&self, _ctx: ConditionContext) {}
}

// Maximum condition value (Football Manager style)
pub const MAX_CONDITION: i16 = 20000;

/// Activity intensity levels for condition processing
#[derive(Debug, Clone, Copy)]
pub enum ActivityIntensity {
    /// Very high intensity - explosive actions (diving, jumping, punching, shooting, tackling, rushing out)
    VeryHigh,
    /// High intensity - sweeping, running, clearing, taking ball
    High,
    /// Moderate intensity - catching, throwing, kicking, distributing, preparing for save
    Moderate,
    /// Low intensity - holding ball, attentive, walking
    Low,
    /// Recovery - standing still, resting (goalkeepers recover more than outfield players)
    Recovery,
}

impl ActivityIntensity {
    /// Get the base fatigue for this activity intensity
    pub fn base_fatigue(&self) -> f32 {
        match self {
            ActivityIntensity::VeryHigh => 7.0,  // Lower than outfield players - explosive but infrequent
            ActivityIntensity::High => 4.5,      // Lower than outfield players
            ActivityIntensity::Moderate => 2.5,  // Moderate activity
            ActivityIntensity::Low => 0.8,       // Very low fatigue
            ActivityIntensity::Recovery => -4.0, // Better recovery than outfield players
        }
    }
}

/// Goalkeeper condition processor
pub struct GoalkeeperCondition {
    intensity: ActivityIntensity,
    use_velocity: bool,
}

impl GoalkeeperCondition {
    /// Create a new condition processor with fixed intensity
    pub fn new(intensity: ActivityIntensity) -> Self {
        Self {
            intensity,
            use_velocity: false,
        }
    }

    /// Create a new condition processor with velocity-based intensity
    pub fn with_velocity(intensity: ActivityIntensity) -> Self {
        Self {
            intensity,
            use_velocity: true,
        }
    }

    /// Process condition changes based on activity intensity and player attributes
    pub fn process(self, ctx: ConditionContext) {
        let stamina_skill = ctx.player.skills.physical.stamina;
        let natural_fitness = ctx.player.skills.physical.natural_fitness;

        let base_fatigue = self.intensity.base_fatigue();

        // Stamina affects how tired the player gets (better stamina = less fatigue)
        // Range: 0.5x to 1.5x (high stamina players tire 50% slower)
        let stamina_factor = 1.5 - (stamina_skill / 20.0);

        // Natural fitness affects recovery and fatigue resistance
        let fitness_factor = 1.3 - (natural_fitness / 20.0) * 0.6;

        // Calculate intensity multiplier based on velocity if needed
        let intensity_multiplier = if self.use_velocity {
            let velocity_magnitude = ctx.player.velocity.norm();
            let max_speed = ctx.player.skills.max_speed();
            let intensity_ratio = if max_speed > 0.0 {
                (velocity_magnitude / max_speed).clamp(0.0, 1.0)
            } else {
                0.5
            };

            // Intensity multiplier: goalkeepers rarely sprint long distances
            if intensity_ratio < 0.3 {
                0.2 // Walking
            } else if intensity_ratio < 0.6 {
                0.5 // Jogging
            } else if intensity_ratio < 0.85 {
                1.0 // Running
            } else {
                1.3 // Sprinting (less demanding than outfield players)
            }
        } else {
            1.0 // No velocity adjustment
        };

        // Calculate final fatigue/recovery
        let condition_change = (base_fatigue * stamina_factor * fitness_factor * intensity_multiplier) as i16;

        // Apply condition change (clamped to 0..MAX_CONDITION)
        ctx.player.player_attributes.condition =
            (ctx.player.player_attributes.condition - condition_change).clamp(0, MAX_CONDITION);

        // Goalkeepers rarely get jaded due to lower overall activity
        // Only increase jadedness if very tired and working hard
        if ctx.player.player_attributes.condition < 1500 && ctx.in_state_time % 150 == 0 {
            // Every 150 ticks when very tired (less frequent than outfield players)
            ctx.player.player_attributes.jadedness =
                (ctx.player.player_attributes.jadedness + 3).min(10000);
        }
    }
}
