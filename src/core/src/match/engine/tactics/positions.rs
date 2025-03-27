use crate::PlayerPositionType;
use crate::r#match::{PositionType, POSITION_POSITIONING};

#[derive(Debug, Clone)]
pub struct MatchTacticalPosition {
    pub position: PlayerPositionType,
    pub waypoints: Vec<(f32, f32)>,
}

#[derive(Debug, Clone)]
pub struct TacticalPositions {
    pub current_position: PlayerPositionType,
    pub tactical_positions: Vec<MatchTacticalPosition>
}

impl TacticalPositions {
    pub fn new(current_position: PlayerPositionType) -> Self {
        let tactical_positions = vec![
            MatchTacticalPosition {
                position: current_position,
                waypoints: Self::generate_waypoints_for_position(current_position),
            }
        ];

        TacticalPositions {
            current_position,
            tactical_positions
        }
    }

    fn generate_waypoints_for_position(position: PlayerPositionType) -> Vec<(f32, f32)> {
        // Get base position coordinates for this position
        let (base_x, base_y) = Self::get_base_position_coordinates(position);

        match position {
            // Goalkeeper positions - minimal movement
            PlayerPositionType::Goalkeeper => {
                vec![
                    (base_x, base_y),           // Central position
                    (base_x, base_y - 30.0),    // Slight left
                    (base_x, base_y + 30.0),    // Slight right
                ]
            },

            // Defender positions
            PlayerPositionType::DefenderLeft |
            PlayerPositionType::DefenderCenterLeft |
            PlayerPositionType::DefenderCenter |
            PlayerPositionType::DefenderCenterRight |
            PlayerPositionType::DefenderRight => {
                vec![
                    (base_x, base_y),            // Base position
                    (base_x - 20.0, base_y),     // More defensive
                    (base_x + 20.0, base_y),     // More forward
                    (base_x, base_y - 30.0),     // Shift left
                    (base_x, base_y + 30.0),     // Shift right
                    (base_x - 15.0, base_y - 20.0), // Diagonal back-left
                    (base_x - 15.0, base_y + 20.0), // Diagonal back-right
                ]
            },

            PlayerPositionType::Sweeper => {
                vec![
                    (base_x, base_y),          // Base position
                    (base_x - 20.0, base_y),   // More defensive
                    (base_x + 20.0, base_y),   // More forward
                    (base_x, base_y - 40.0),   // Left cover
                    (base_x, base_y + 40.0),   // Right cover
                ]
            },

            // Midfield positions - more movement range
            PlayerPositionType::DefensiveMidfielder => {
                vec![
                    (base_x, base_y),           // Base position
                    (base_x - 30.0, base_y),    // More defensive
                    (base_x + 30.0, base_y),    // More forward
                    (base_x, base_y - 40.0),    // Shift left
                    (base_x, base_y + 40.0),    // Shift right
                    (base_x - 20.0, base_y - 25.0), // Diagonal back-left
                    (base_x - 20.0, base_y + 25.0), // Diagonal back-right
                    (base_x + 20.0, base_y - 25.0), // Diagonal forward-left
                    (base_x + 20.0, base_y + 25.0), // Diagonal forward-right
                ]
            },

            PlayerPositionType::WingbackLeft => {
                vec![
                    (base_x, base_y),            // Base position
                    (base_x - 40.0, base_y),     // Defensive
                    (base_x + 40.0, base_y + 15.0), // Forward
                    (base_x + 50.0, base_y + 30.0), // Forward inside
                    (base_x - 30.0, base_y - 15.0), // Defensive wide
                ]
            },

            PlayerPositionType::WingbackRight => {
                vec![
                    (base_x, base_y),            // Base position
                    (base_x - 40.0, base_y),     // Defensive
                    (base_x + 40.0, base_y - 15.0), // Forward
                    (base_x + 50.0, base_y - 30.0), // Forward inside
                    (base_x - 30.0, base_y + 15.0), // Defensive wide
                ]
            },

            PlayerPositionType::MidfielderLeft |
            PlayerPositionType::MidfielderCenterLeft |
            PlayerPositionType::MidfielderCenter |
            PlayerPositionType::MidfielderCenterRight |
            PlayerPositionType::MidfielderRight => {
                vec![
                    (base_x, base_y),            // Base position
                    (base_x - 30.0, base_y),     // More defensive
                    (base_x + 30.0, base_y),     // More forward
                    (base_x, base_y - 40.0),     // Wider/left
                    (base_x, base_y + 40.0),     // Wider/right
                    (base_x - 25.0, base_y - 30.0), // Diagonal back-wide
                    (base_x + 25.0, base_y - 30.0), // Diagonal forward-wide
                    (base_x + 40.0, base_y),      // Advanced position
                ]
            },

            // Attacking midfield positions - aggressive movement
            PlayerPositionType::AttackingMidfielderLeft |
            PlayerPositionType::AttackingMidfielderCenter |
            PlayerPositionType::AttackingMidfielderRight => {
                vec![
                    (base_x, base_y),            // Base position
                    (base_x - 40.0, base_y),     // Drop deeper
                    (base_x + 25.0, base_y),     // More forward
                    (base_x, base_y - 35.0),     // Wider/left
                    (base_x, base_y + 35.0),     // Wider/right
                    (base_x - 30.0, base_y - 20.0), // Diagonal back-wide
                    (base_x + 20.0, base_y - 20.0), // Diagonal forward-wide
                    (base_x + 30.0, base_y),     // Advanced central position
                    (base_x - 35.0, base_y + 25.0), // Deep support position
                ]
            },

            // Forward positions - dynamic movement with focus on final third
            PlayerPositionType::ForwardLeft |
            PlayerPositionType::ForwardCenter |
            PlayerPositionType::ForwardRight => {
                vec![
                    (base_x, base_y),            // Base position
                    (base_x - 35.0, base_y),     // Drop deeper
                    (base_x + 30.0, base_y),     // Push forward
                    (base_x, base_y - 40.0),     // Move wider
                    (base_x, base_y + 40.0),     // Move central
                    (base_x - 25.0, base_y - 25.0), // Drop diagonally wide
                    (base_x + 25.0, base_y - 25.0), // Push diagonally wide
                    (base_x + 25.0, base_y + 15.0), // Push diagonally central
                    (base_x - 30.0, base_y + 15.0), // Drop diagonally central
                ]
            },

            PlayerPositionType::Striker => {
                vec![
                    (base_x, base_y),            // Base position
                    (base_x - 40.0, base_y),     // Drop deeper
                    (base_x + 25.0, base_y),     // Push forward
                    (base_x, base_y - 35.0),     // Move left
                    (base_x, base_y + 35.0),     // Move right
                    (base_x - 30.0, base_y - 25.0), // Drop diagonally left
                    (base_x - 30.0, base_y + 25.0), // Drop diagonally right
                    (base_x + 20.0, base_y - 20.0), // Push diagonally left
                    (base_x + 20.0, base_y + 20.0), // Push diagonally right
                ]
            },

            // Default for any other position
            _ => {
                vec![
                    (base_x, base_y),            // Base position
                    (base_x - 25.0, base_y),     // Back
                    (base_x + 25.0, base_y),     // Forward
                    (base_x, base_y - 25.0),     // Left
                    (base_x, base_y + 25.0),     // Right
                ]
            },
        }
    }

    fn get_base_position_coordinates(position: PlayerPositionType) -> (f32, f32) {
        // Find the base coordinates from POSITION_POSITIONING constant
        for (pos, home, away) in POSITION_POSITIONING {
            if *pos == position {
                if let PositionType::Home(x, y) = home {
                    return (*x as f32, *y as f32);
                }
            }
        }

        // Default position if not found (center of the field)
        (420.0, 272.5)
    }
}
