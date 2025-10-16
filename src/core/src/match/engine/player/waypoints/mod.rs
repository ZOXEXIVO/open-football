use nalgebra::Vector3;

const WAYPOINT_REACHED_THRESHOLD: f32 = 25.0; // Increased threshold for larger waypoint distances

#[derive(Debug, Clone)]
pub struct WaypointManager {
    pub current_index: usize,
    pub path_completed: bool,
    pub loop_path: bool,
}

impl WaypointManager {
    pub fn new() -> Self {
        WaypointManager {
            current_index: 0,
            path_completed: false,
            loop_path: false,
        }
    }
    
    pub fn reset(&mut self) {
        self.current_index = 0;
        self.path_completed = false;
    }

    pub fn update(&mut self, player_position: &Vector3<f32>, waypoints: &[Vector3<f32>]) -> Option<Vector3<f32>> {
        if waypoints.is_empty() || self.path_completed {
            return None;
        }

        // Find the nearest waypoint ahead of or at the player's current position
        let nearest_ahead = self.find_nearest_waypoint_ahead(player_position, waypoints);

        // Update current index to the nearest waypoint ahead
        if let Some(nearest_idx) = nearest_ahead {
            self.current_index = nearest_idx;
        }

        let current_waypoint = waypoints[self.current_index];
        let distance = (player_position - current_waypoint).magnitude();

        // If close enough to current waypoint, advance to next
        if distance < WAYPOINT_REACHED_THRESHOLD {
            self.current_index += 1;

            if self.current_index >= waypoints.len() {
                if self.loop_path {
                    self.current_index = 0;
                } else {
                    self.path_completed = true;
                    return None;
                }
            }
        }

        if self.current_index < waypoints.len() {
            Some(waypoints[self.current_index])
        } else {
            None
        }
    }

    fn find_nearest_waypoint_ahead(&self, player_position: &Vector3<f32>, waypoints: &[Vector3<f32>]) -> Option<usize> {
        if waypoints.is_empty() {
            return None;
        }

        // Start searching from current index (don't go backward unless necessary)
        let mut nearest_index = self.current_index;
        let mut nearest_distance = (player_position - waypoints[self.current_index]).magnitude();

        // Check if any waypoint from current onwards is closer
        for i in self.current_index..waypoints.len() {
            let distance = (player_position - waypoints[i]).magnitude();
            if distance < nearest_distance {
                nearest_distance = distance;
                nearest_index = i;
            }
        }

        // If player moved backward significantly, find the absolute nearest waypoint
        if nearest_distance > WAYPOINT_REACHED_THRESHOLD * 3.0 {
            for i in 0..self.current_index {
                let distance = (player_position - waypoints[i]).magnitude();
                if distance < nearest_distance {
                    nearest_distance = distance;
                    nearest_index = i;
                }
            }
        }

        Some(nearest_index)
    }
}