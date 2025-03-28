use nalgebra::Vector3;

const WAYPOINT_REACHED_THRESHOLD: f32 = 5.0;

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

    pub fn update(&mut self, player_position: &Vector3<f32>, waypoints: &[Vector3<f32>]) -> Option<Vector3<f32>> {
        if waypoints.is_empty() || self.path_completed {
            return None;
        }

        let current_waypoint = waypoints[self.current_index];
        let distance = (player_position - current_waypoint).magnitude();

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
}