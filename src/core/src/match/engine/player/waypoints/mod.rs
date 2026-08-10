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
            loop_path: true,
        }
    }

    pub fn reset(&mut self) {
        self.current_index = 0;
        self.path_completed = false;
    }

    /// Advance along the route and return the waypoint to steer at.
    ///
    /// # The oscillator this replaces
    ///
    /// Every generated route is a ONE-WAY line: a defender's runs from his
    /// base position out to `base_x + 320` toward the opposition goal, a
    /// forward's to the opposing goal (see
    /// `TacticalPositions::generate_waypoints_for_position`). None of them
    /// is a circuit. `loop_path` was nevertheless `true`, so on reaching
    /// the last waypoint the index wrapped to 0 — a point now up to 320u
    /// BEHIND the player. The steering target inverted, and the player
    /// turned and sprinted back.
    ///
    /// On the very next tick `find_nearest_waypoint_ahead` — which despite
    /// its name simply took the NEAREST waypoint, forwards or backwards —
    /// snapped the index straight back to the far end the player was
    /// standing on, the reached-check advanced it past the end, and it
    /// wrapped to 0 again. The two rules undid each other every tick, so
    /// the target alternated between "240u behind you" and "right here"
    /// at 50 Hz, and the player vibrated on the spot.
    ///
    /// Measured, `Defender: Running` — where nearly every off-ball
    /// defender lives — produced **9.7 velocity reversals per second held,
    /// 386,617 of them across two matches**, the single largest source of
    /// position flicker in the engine, with `Forward: Running In Behind`
    /// and `Forward: Running` next on the same mechanism (`dev_match
    /// trace`).
    ///
    /// # The rule
    ///
    /// A route is walked in order. Progress is monotonic — a waypoint once
    /// passed is never steered at again, so the target can never jump
    /// backwards — and the far end is a terminus, not a wrap: a player who
    /// has run the whole route holds station at its end rather than
    /// spinning round. The route re-arms when the player is back at its
    /// start, which is what makes it a repeatable run rather than a
    /// one-shot.
    pub fn update(
        &mut self,
        player_position: &Vector3<f32>,
        waypoints: &[Vector3<f32>],
    ) -> Option<Vector3<f32>> {
        if waypoints.is_empty() {
            return None;
        }
        let last = waypoints.len() - 1;
        if self.current_index > last {
            self.current_index = last;
        }

        // Re-arm: the player is back at the start of the route (recovered
        // his shape, or was reset at a restart), so the run is available
        // again. Checked before the advance so a player standing on
        // waypoint 0 starts from there rather than being counted as
        // having already passed it.
        if self.path_completed
            && (player_position - waypoints[0]).magnitude() < WAYPOINT_REACHED_THRESHOLD
        {
            self.current_index = 0;
            self.path_completed = false;
        }

        // Walk forward past everything already behind us. Monotonic by
        // construction: this loop can only ever increase the index, which
        // is what makes a backwards target jump impossible.
        //
        // A waypoint counts as behind us either because we got close
        // enough to it, or because we are already at or past the NEXT one
        // measured along the leg between them. The second test is what
        // lets a player who was moved bodily up the pitch — a set-piece
        // teleport, a restart, or simply a long run made under some other
        // state — rejoin the route at the right place instead of being
        // steered back down it.
        while self.current_index < last {
            let here = waypoints[self.current_index];
            let leg = waypoints[self.current_index + 1] - here;
            let leg_len_sq = leg.norm_squared();
            let reached = (player_position - here).magnitude() < WAYPOINT_REACHED_THRESHOLD;
            let past_next = leg_len_sq > 0.0 && (player_position - here).dot(&leg) >= leg_len_sq;
            if reached || past_next {
                self.current_index += 1;
            } else {
                break;
            }
        }

        // Terminus. The player holds at the end of his route; his state
        // machine is what takes him elsewhere (a defender this far
        // upfield is already `Big` from his start position, which routes
        // him to `Returning`). Deliberately still returns the last
        // waypoint rather than `None` — a `None` here reads as "no
        // waypoint" to `FollowPath` and drops the player's velocity to
        // zero mid-pitch.
        if self.current_index == last
            && (player_position - waypoints[last]).magnitude() < WAYPOINT_REACHED_THRESHOLD
        {
            self.path_completed = true;
        }

        Some(waypoints[self.current_index])
    }
}

#[cfg(test)]
mod tests {
    use super::{WAYPOINT_REACHED_THRESHOLD, WaypointManager};
    use nalgebra::Vector3;

    fn route() -> Vec<Vector3<f32>> {
        // A defender's real route shape: a straight line marching upfield.
        vec![
            Vector3::new(180.0, 100.0, 0.0),
            Vector3::new(260.0, 100.0, 0.0),
            Vector3::new(340.0, 100.0, 0.0),
            Vector3::new(420.0, 100.0, 0.0),
        ]
    }

    #[test]
    fn target_never_moves_backwards_along_the_route() {
        let wps = route();
        let mut wm = WaypointManager::new();
        let mut last_index = 0usize;
        // Walk the whole route in 5u steps, then keep going past the end.
        let mut x = 100.0f32;
        while x < 600.0 {
            wm.update(&Vector3::new(x, 100.0, 0.0), &wps);
            assert!(
                wm.current_index >= last_index,
                "waypoint index went backwards at x={x}: {} -> {}",
                last_index,
                wm.current_index
            );
            last_index = wm.current_index;
            x += 5.0;
        }
    }

    #[test]
    fn completing_the_route_holds_at_the_end_instead_of_wrapping() {
        // The old behaviour wrapped to waypoint 0 — 240u behind — which
        // inverted the steering target and produced the twitch.
        let wps = route();
        let mut wm = WaypointManager::new();
        let at_end = Vector3::new(420.0, 100.0, 0.0);
        let target = wm.update(&at_end, &wps).expect("route still has a target");
        assert_eq!(wm.current_index, wps.len() - 1);
        assert_eq!(target, *wps.last().unwrap());
        assert!(wm.path_completed);

        // And it stays there rather than flipping on subsequent ticks.
        for _ in 0..10 {
            let t = wm.update(&at_end, &wps).unwrap();
            assert_eq!(t, *wps.last().unwrap());
        }
    }

    #[test]
    fn route_rearms_once_the_player_is_back_at_its_start() {
        let wps = route();
        let mut wm = WaypointManager::new();
        wm.update(&Vector3::new(420.0, 100.0, 0.0), &wps);
        assert!(wm.path_completed);

        // Recovered his shape — the run is available again.
        wm.update(&wps[0], &wps);
        assert!(!wm.path_completed);
        assert_eq!(wm.current_index, 1, "standing on wp0 counts as reaching it");
    }

    #[test]
    fn a_passed_waypoint_is_never_targeted_again() {
        let wps = route();
        let mut wm = WaypointManager::new();
        // Reach waypoint 1.
        wm.update(&Vector3::new(260.0, 100.0, 0.0), &wps);
        let advanced = wm.current_index;
        assert!(advanced >= 2);
        // Drift back toward waypoint 0 without reaching it: the target
        // must not snap back to the nearer, already-passed waypoint.
        let drift = Vector3::new(200.0, 100.0, 0.0);
        assert!((drift - wps[0]).magnitude() < WAYPOINT_REACHED_THRESHOLD * 3.0);
        wm.update(&drift, &wps);
        assert_eq!(wm.current_index, advanced);
    }
}
