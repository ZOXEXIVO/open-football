use nalgebra::Vector3;

/// Route-usage census. Dev diagnostic only — compiles out without
/// `match-logs`.
#[cfg(feature = "match-logs")]
pub mod census;
#[cfg(feature = "match-logs")]
use census::WaypointCensus;

const WAYPOINT_REACHED_THRESHOLD: f32 = 25.0; // Increased threshold for larger waypoint distances

/// **Whether a pre-drawn tactical route may steer a player at all.**
///
/// Off by default. `OF_ROUTES=on` re-arms it, so the behaviour this
/// replaced stays measurable rather than only describable.
///
/// # What the route layer actually was
///
/// `TacticalPositions::generate_waypoints_for_position` draws every
/// player one straight line, before kickoff, from his formation dot
/// toward the goal his side attacks. Nothing about it moves again: it
/// does not know where the ball is, which phase the match is in, or
/// where his ten team-mates are standing. `dev_match waypoints` prints
/// all 44 of them; read as football they are not runs:
///
/// | position | route ends | |
/// |---|---|---|
/// | centre-half | 58% of the way to the opposition goal | 12 m inside their half |
/// | central midfielder | 92% | 7.5 m from their goal line |
/// | forward, attacking mid, striker | 100% | on the goal line, centre — inside the net |
///
/// (A fourth, since fixed in `POSITION_POSITIONING`: the away side's
/// wingbacks lined up on the wrong flank, so their routes — which
/// mirror correctly — ran them diagonally across the entire pitch, y 50
/// to y 500.)
///
/// # Three measured defects, `dev_match waypoints 3 14`
///
/// **It is not a walk — it is one fixed point.** 16,551,675 manager
/// updates produced **3,087 index advances**, and outfield players
/// re-armed a route **zero** times (all 1.1 M recorded re-arms are
/// goalkeepers flip-flopping on their single-waypoint route). The walk
/// is monotonic and only re-arms within 3 m of waypoint 0, so the index
/// reaches the far end within the opening exchanges and stays there:
/// **77-81% of every route-follow steers at the terminus.** What the
/// layer delivers is therefore not a path at all — it is one stationary
/// point near the opposition goal, per player, for the whole match.
///
/// **It outranks the team plan.** The branch is the FIRST thing
/// `velocity()` tries in each of the seven states that consult it, and
/// across three matches it was asked 2,308,877 times and answered yes
/// **1,723,959 times (74.7%)** — 91.4% of `Forward: Running`, 93.0% of
/// `Defender: Standing`, 97.3% of `Forward: Walking`. Everything written
/// below those branches — the assigned box slot and `BoxMovement`, the
/// `SupportOffer` outlet, the midfield's compact out-of-possession block
/// — was unreachable on those ticks.
///
/// **It disagrees with the plan, and disagrees hard.** Against the
/// anchor `TeamOperationsImpl::my_anchor` gave the same man on the
/// same tick:
///
/// | group | he is | the plan wants | the route wants | route ↔ anchor | pointing apart |
/// |---|---|---|---|---|---|
/// | defenders | 39.6% | 38.3% | **53.5%** | 19.9 m | 58.7% of ticks |
/// | midfielders | 44.2% | 44.3% | **87.6%** | 47.2 m | 56.3% |
/// | forwards | 65.9% | 57.7% | **95.7%** | 41.1 m | **88.6%** |
///
/// (depth as a fraction of the pitch toward the goal he attacks). The
/// shape layer is doing its job — the mean player sits 6.5-11.4 m from
/// his anchor. The route asks him to be 20-47 m away from it, in the
/// opposition third on 92% of midfield ticks and 99.8% of forward ticks,
/// and on more than half of them in the opposite direction. Every one of
/// those ticks is a tug-of-war `ShapeDiscipline` then has to arbitrate,
/// and a blend of two opposed intents is a player drifting sideways at
/// half speed — which is what it looks like on screen.
///
/// # Why it is not repaired in place
///
/// A pre-drawn line is not how anybody moves off the ball. A player has
/// a POSITION in the block and RUNS he makes when the picture licenses
/// them, and this engine already models both properly and per role:
/// `TeamShape` → `my_anchor` for the first, and `WidePlan` overlaps,
/// `BoxMovement` staging, `SupportOffer`, `CreatingSpace` and
/// `DefenderState::PushingUp` for the second. The route layer is a
/// third, older, blind copy of both, and it was winning the argument
/// only because it was checked first.
///
/// `Defender: Running` reached this conclusion first and dropped its own
/// branch outright — see the note on `DefenderRunningState::velocity`,
/// which measured 11.7 velocity reversals per second held in that one
/// state. The numbers above are the same finding in the six states it
/// left behind, plus `Defender: Standing`.
///
/// # What disarming it measured
///
/// Paired A/B on one binary, `OF_ROUTES=on` against live, three runs an
/// arm at `dev_match stats 400 14 14` (the harness repeats to ±0.15
/// goals, so single runs cannot see effects this size):
///
/// | | routes on | routes off | real |
/// |---|---|---|---|
/// | goals/match | 2.96 | **2.33** | ~2.5 |
/// | shots/team | 14.03 | **12.00** | ~13 |
/// | on-target → goal | 33.9% | **31.6%** | ~30% |
/// | saves/on-target | 66.1% | **68.4%** | ~67% |
/// | goal share MID / FWD | 38.3% / 61.0% | **31.7%** / 68.1% | 32% / 58% |
/// | home − away goals | +0.94 | **+0.75** | +0.35 |
/// | draws | 24.8% | 29.4% | ~25% |
///
/// The volume falls because the layer was manufacturing chances out of
/// geometry: a striker parked on `(840, 275)` is standing *in the goal*,
/// and a midfielder held at 88% depth is standing on the six-yard line
/// all match. `MID BOX-RUN` says as much directly — runner-in-box ticks
/// **1,640,143 → 918,663** — and the midfield's goal share lands on its
/// target as a result.
///
/// **Two costs, both named.** The forward share overshoots (68% against
/// 58%) because defenders score essentially nothing either way (0.2%
/// against a 10% target); that is the corner supply — 4.2 a match
/// against a real 10.4 — and it is not this layer's. And draws rise
/// 24.8% → 29.4%, which is **mechanical rather than structural**: the
/// harness's own independence baseline moves 21.8% → 25.8% over the same
/// pair, so the correlation surplus is +3.0pp against +3.6pp — unchanged
/// inside its own noise. Both come back with the shot supply, which
/// `OF_SHOT_BAR` re-titrates without a rebuild.
///
/// The flicker it was generating goes with it (`dev_match trace 3 14`,
/// same pair). In-state velocity reversals, the states the route
/// actually reached:
///
/// | state | routes on | routes off |
/// |---|---|---|
/// | `Forward: Running` (91.4% take) | 0.79/s | **0.39/s** |
/// | `Forward: Creating Space` | 1.14/s | **0.74/s** |
/// | `Forward: Running In Behind` | 1.77/s | **1.48/s** |
/// | `Defender: Guarding` | 3.42/s | **2.49/s** |
/// | whole match | 0.92/s | **0.80/s** |
///
/// `Forward: Running` halves on an identical tick count (2,642,071
/// against 2,656,385 held), which is the tug-of-war above, measured from
/// the other end.
pub struct TacticalRoutes;

impl TacticalRoutes {
    /// True only when `OF_ROUTES=on` — the legacy arm, kept so the
    /// change stays A/B-measurable against the tables above.
    pub fn armed() -> bool {
        static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *ON.get_or_init(|| {
            std::env::var("OF_ROUTES")
                .map(|v| v == "on" || v == "1")
                .unwrap_or(false)
        })
    }
}

/// Which exit `MatchPlayer::should_follow_waypoints` took.
///
/// The predicate is a bool at its call sites, but the reason behind the
/// bool is the interesting part — a route declined because the man is
/// carrying the ball and one declined because he is chasing it are
/// different facts about the same tick. Named so the census can
/// attribute them; the call sites still read the bool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaypointExit {
    /// Routes are not a velocity source — see [`TacticalRoutes`].
    Disarmed,
    /// He has the ball; the route never applies.
    Carrier,
    /// He is the designated chaser of a loose ball.
    Chaser,
    /// A team-mate is inside the bunching radius, so he peels off.
    Crowded,
    /// Nothing said otherwise, so the route applies.
    Default,
}

impl WaypointExit {
    /// Whether this exit means the player follows his route.
    #[inline]
    pub fn follows(self) -> bool {
        matches!(self, WaypointExit::Crowded | WaypointExit::Default)
    }
}

#[derive(Debug, Clone)]
pub struct WaypointManager {
    pub current_index: usize,
    pub path_completed: bool,
}

impl WaypointManager {
    pub fn new() -> Self {
        WaypointManager {
            current_index: 0,
            path_completed: false,
        }
    }

    /// Put the player back at the start of his route.
    ///
    /// ⚠ **Nothing calls this, and that is one of the reasons the walk
    /// parks.** `MatchPlayer::set_default_state` rebuilds the route
    /// cache at every restart — kickoff, a goal, half-time, a
    /// substitute walking on — and does not reset the index, so the
    /// walk carries across the whistle. Combined with a monotonic index
    /// and a re-arm that only fires within 3 m of waypoint 0, an
    /// outfield route is walked to its end once and then never again
    /// (measured: 3,087 advances in 16.5 M updates, zero outfield
    /// re-arms). This is the hook to call from `set_default_state` if
    /// [`TacticalRoutes`] is ever re-armed.
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
        #[cfg(feature = "match-logs")]
        let was_completed = self.path_completed;
        if self.path_completed
            && (player_position - waypoints[0]).magnitude() < WAYPOINT_REACHED_THRESHOLD
        {
            self.current_index = 0;
            self.path_completed = false;
        }
        #[cfg(feature = "match-logs")]
        let rearmed = was_completed && !self.path_completed;
        #[cfg(feature = "match-logs")]
        let (mut advances, mut past_next_advances) = (0u32, 0u32);

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
                #[cfg(feature = "match-logs")]
                {
                    advances += 1;
                    if past_next && !reached {
                        past_next_advances += 1;
                    }
                }
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

        #[cfg(feature = "match-logs")]
        WaypointCensus::note_manager(
            advances,
            past_next_advances,
            self.path_completed && !was_completed,
            rearmed,
        );

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
