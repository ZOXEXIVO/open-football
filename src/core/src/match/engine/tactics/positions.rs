use crate::r#match::{PositionType, POSITION_POSITIONING};
use crate::r#match::player::PlayerSide;
use crate::PlayerPositionType;

#[derive(Debug, Clone)]
pub struct MatchTacticalPosition {
    pub position: PlayerPositionType,
    pub waypoints: Vec<(f32, f32)>,
}

#[derive(Debug, Clone)]
pub struct TacticalPositions {
    pub current_position: PlayerPositionType,
    pub tactical_positions: Vec<MatchTacticalPosition>,
}

impl TacticalPositions {
    pub fn new(current_position: PlayerPositionType, side: Option<PlayerSide>) -> Self {
        let tactical_positions = vec![
            MatchTacticalPosition {
                position: current_position,
                waypoints: Self::generate_waypoints_for_position(current_position, side),
            }
        ];

        TacticalPositions {
            current_position,
            tactical_positions,
        }
    }

    pub fn regenerate_waypoints(&mut self, side: Option<PlayerSide>) {
        for tactical_position in &mut self.tactical_positions {
            tactical_position.waypoints = Self::generate_waypoints_for_position(
                tactical_position.position,
                side,
            );
        }
    }

    fn generate_waypoints_for_position(position: PlayerPositionType, side: Option<PlayerSide>) -> Vec<(f32, f32)> {
        // Get base position coordinates for this position and side
        let (base_x, base_y) = Self::get_base_position_coordinates(position, side);

        // Determine direction multiplier based on side
        // Left side (Home): attacking right (positive x direction)
        // Right side (Away): attacking left (negative x direction)
        let direction = match side {
            Some(PlayerSide::Left) => 1.0,
            Some(PlayerSide::Right) => -1.0,
            None => 1.0, // Default to left/home
        };

        // Goal positions
        let (goal_x, goal_y) = match side {
            Some(PlayerSide::Left) => (840.0, 275.0),  // Attacking right goal
            Some(PlayerSide::Right) => (0.0, 275.0),   // Attacking left goal
            None => (840.0, 275.0), // Default
        };

        match position {
            // Goalkeeper - stay near own goal, minimal waypoints
            PlayerPositionType::Goalkeeper => {
                vec![
                    (base_x, base_y),
                ]
            }

            // Defenders - path from defensive position toward midfield
            PlayerPositionType::DefenderLeft => {
                vec![
                    (base_x, base_y),
                    (base_x + 50.0 * direction, base_y),      // Push forward
                    (base_x + 100.0 * direction, base_y),     // Further forward
                ]
            }

            PlayerPositionType::DefenderCenterLeft |
            PlayerPositionType::DefenderCenter |
            PlayerPositionType::DefenderCenterRight => {
                vec![
                    (base_x, base_y),
                    (base_x + 50.0 * direction, base_y),      // Push forward
                    (base_x + 100.0 * direction, base_y),     // Further forward
                ]
            }

            PlayerPositionType::DefenderRight => {
                vec![
                    (base_x, base_y),
                    (base_x + 50.0 * direction, base_y),      // Push forward
                    (base_x + 100.0 * direction, base_y),     // Further forward
                ]
            }

            PlayerPositionType::Sweeper => {
                vec![
                    (base_x, base_y),
                    (base_x + 40.0 * direction, base_y),
                ]
            }

            // Defensive midfielder - path from defensive midfield to attacking midfield
            PlayerPositionType::DefensiveMidfielder => {
                vec![
                    (base_x, base_y),
                    (base_x + 60.0 * direction, base_y),
                    (base_x + 120.0 * direction, base_y),
                ]
            }

            // Wingbacks - diagonal path toward opponent's corner
            PlayerPositionType::WingbackLeft => {
                let target_y = if direction > 0.0 { 50.0 } else { 500.0 };
                vec![
                    (base_x, base_y),
                    (base_x + 100.0 * direction, base_y),
                    (base_x + 200.0 * direction, target_y),
                ]
            }

            PlayerPositionType::WingbackRight => {
                let target_y = if direction > 0.0 { 500.0 } else { 50.0 };
                vec![
                    (base_x, base_y),
                    (base_x + 100.0 * direction, base_y),
                    (base_x + 200.0 * direction, target_y),
                ]
            }

            // Midfielders - path from midfield toward attacking third, staying in lane
            PlayerPositionType::MidfielderLeft => {
                vec![
                    (base_x, base_y),
                    (base_x + 80.0 * direction, base_y),
                    (base_x + 160.0 * direction, base_y),
                ]
            }

            PlayerPositionType::MidfielderCenterLeft |
            PlayerPositionType::MidfielderCenter |
            PlayerPositionType::MidfielderCenterRight => {
                vec![
                    (base_x, base_y),
                    (base_x + 80.0 * direction, base_y),
                    (base_x + 160.0 * direction, base_y),
                ]
            }

            PlayerPositionType::MidfielderRight => {
                vec![
                    (base_x, base_y),
                    (base_x + 80.0 * direction, base_y),
                    (base_x + 160.0 * direction, base_y),
                ]
            }

            // Attacking midfielders - path toward goal
            PlayerPositionType::AttackingMidfielderLeft |
            PlayerPositionType::AttackingMidfielderCenter |
            PlayerPositionType::AttackingMidfielderRight => {
                vec![
                    (base_x, base_y),
                    (base_x + 50.0 * direction, base_y),
                    (goal_x, goal_y),  // Final waypoint at opponent's goal
                ]
            }

            // Forwards - direct path to opponent's goal
            PlayerPositionType::ForwardLeft |
            PlayerPositionType::ForwardCenter |
            PlayerPositionType::ForwardRight => {
                vec![
                    (base_x, base_y),
                    (goal_x, goal_y),  // Direct line to opponent's goal
                ]
            }

            // Striker - direct path to opponent's goal
            PlayerPositionType::Striker => {
                vec![
                    (base_x, base_y),
                    (goal_x, goal_y),  // Direct line to opponent's goal
                ]
            }

            // Default - simple forward movement
            _ => {
                vec![
                    (base_x, base_y),
                    (base_x + 100.0 * direction, base_y),
                ]
            }
        }
    }

    fn get_base_position_coordinates(position: PlayerPositionType, side: Option<PlayerSide>) -> (f32, f32) {
        // Find the base coordinates from POSITION_POSITIONING constant based on side
        for (pos, home, away) in POSITION_POSITIONING {
            if *pos == position {
                match side {
                    Some(PlayerSide::Left) => {
                        if let PositionType::Home(x, y) = home {
                            return (*x as f32, *y as f32);
                        }
                    }
                    Some(PlayerSide::Right) => {
                        if let PositionType::Away(x, y) = away {
                            return (*x as f32, *y as f32);
                        }
                    }
                    None => {
                        // Default to home position if side is not specified
                        if let PositionType::Home(x, y) = home {
                            return (*x as f32, *y as f32);
                        }
                    }
                }
            }
        }

        // Default position if not found (center of the field)
        (420.0, 272.5)
    }
}
