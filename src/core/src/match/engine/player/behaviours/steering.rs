use crate::r#match::MatchPlayer;
use crate::r#match::common_states::LooseBallChase;
use crate::r#match::engine::ball::ball::CONTROL_DISTANCE;
use nalgebra::Vector3;

pub enum SteeringBehavior<'a> {
    Seek {
        target: Vector3<f32>,
    },
    Arrive {
        target: Vector3<f32>,
        slowing_distance: f32,
    },
    Pursuit {
        target: Vector3<f32>,
        target_velocity: Vector3<f32>,
    },
    /// Constant-bearing interception — steer so the line of sight to a
    /// moving target does not rotate.
    ///
    /// `target` and `target_velocity` are where the thing is NOW and how
    /// fast it is going.
    ///
    /// # Why [`Pursuit`](Self::Pursuit) is not this
    ///
    /// `Pursuit` picks a point the target will reach and runs at it, and
    /// the lead it applies is `target_velocity × intercept_time` with
    /// that time clamped to **5 ticks** — 50 ms, because the constant
    /// reads as seconds and never was (see
    /// [`calculate_interception_point`](Self::calculate_interception_point)).
    /// So its aim point sits a few centimetres from where the ball is
    /// now, which makes it a `Seek` in all but name. A runner aimed at
    /// where the ball IS turns to follow it as it goes past: his heading
    /// converges on the ball's and he trails it at a fixed gap for as
    /// long as he keeps running. That is a tail chase, and it is exactly
    /// the report — *"defenders with TakeBall don't intercept the ball,
    /// they run parallel with it"*.
    ///
    /// Measured before this existed (`mid_run_diag::CHASE_SAMPLES`, 200
    /// fixtures at L14, 65M samples of a player in a `TakeBall` state
    /// with the ball loose and moving): aimed AHEAD of the ball on **34%**
    /// of samples, running PARALLEL to it on **40%** (defenders 45%), the
    /// gap not shrinking at all on 38%, at a mean separation of 9.5 m.
    /// The ball averaged **0.889 u/tick against a 0.45-0.63 u/tick
    /// sprint** — so the thing being chased is normally FASTER than the
    /// man chasing it, and running at it is a race nobody can win.
    ///
    /// # The rule
    ///
    /// You do not reach a moving ball by running at it. You run to where
    /// it is going, and the test for being on that line is that the
    /// bearing to it holds steady — which is how a defender reads a pass
    /// and how anyone catches anything. So take the target's velocity
    /// ACROSS the line of sight and match it, since that is what holds
    /// the bearing still, and spend whatever speed is left over closing
    /// the gap:
    ///
    /// ```text
    /// v = v_across + r̂ · √(s² − |v_across|²)
    /// ```
    ///
    /// Every case falls out of the one expression instead of being named:
    ///
    /// * a stationary target has nothing across the line, the root is
    ///   `s`, and this reduces EXACTLY to `Seek`;
    /// * a ball rolling straight at him, or straight away from him, is
    ///   the same — there is nothing to cut off, so he runs at it flat
    ///   out;
    /// * a ball crossing in front of him is met on the diagonal, at the
    ///   angle that makes the two arrive together;
    /// * a ball whose cross-track speed exceeds his own leaves nothing
    ///   under the root — and holding the bearing there closes no gap at
    ///   all, so the law stops holding it. See the lost-cause read below.
    ///
    /// ## The lost-cause read — the hole, closed
    ///
    /// Constant bearing is exact against a target that keeps its speed,
    /// and a rolling ball never does. Whenever the achievable closing
    /// rate dies — the root shut by the cross-track term, or the ball
    /// taking more out along the line than the root leaves — the hold
    /// used to spend everything sideways: *beaten, but beaten running
    /// the right way*. Which was the reported picture, verbatim:
    /// *"defenders in TakeBall run parallel to the ball and it rolls out
    /// of bounds, even though the defender could have intercepted it."*
    /// The man ran the non-converging line for as long as the ball
    /// outpaced him, and by the time friction reopened the root the ball
    /// was over the line he could have cut it off short of.
    ///
    /// Now: as the closing rate falls through [`Self::LOST_CAUSE`] of
    /// top speed, the desired velocity crosses smoothly from the
    /// bearing-hold to a flat-out straight run at
    /// [`LooseBallChase::earliest_meeting`] — the first point on the
    /// decaying roll he can be at no later than the ball, computed from
    /// the same friction constant the physics integrates. A still-just-
    /// winnable race stays on the proven law; a lost one is read like a
    /// player reads it: let it run, take the line to where it slows.
    /// The fade is scaled by `aim`'s own ground/aerial band, because a
    /// flying ball's travel does not decay like a roll's (see below),
    /// and by the COMMITMENT HORIZON [`Self::COMMIT_NEAR`]/
    /// [`Self::COMMIT_FAR`], because rescuing meetings ten seconds of
    /// roll downstream turned out to rewrite the match economy — the
    /// unbounded rescue measured **+0.54 goals/match**, all of it shot
    /// VOLUME (13.0 → 15.0/team) at flat quality, from marathon cuts
    /// keeping attacking sequences alive that used to die over a line.
    /// `OF_CONCEDE` restores the old collapse as the A/B control, and
    /// `loose_ball_chase_tests` pins the rescue, the declined marathon,
    /// and the untouched winnable cases.
    ///
    /// ### The two prior attempts, and why their verdicts did not count
    ///
    /// Both of these were built and measured once before — steer at the
    /// meeting point outright, and blend the headings by the squared
    /// speed ratio — and both "measured worse". **Both measurements were
    /// confounded:** each was built on a version of
    /// [`LooseBallChase::aim`] that had folded its aerial branch into
    /// this behaviour, which steers a chaser at the XY a flying ball
    /// passes OVER rather than the one it lands on. That fold alone
    /// costs the whole change (55% aimed-ahead → 48%, tackles 16.3 →
    /// 12.6 per team over 200 fixtures), so neither arm isolated the
    /// idea it was supposed to test. This version differs from both:
    /// the aerial branch stays intact and explicitly gates the blend,
    /// the discriminator is the CLOSING RATE (which also catches a ball
    /// pulling away along the line, where the root never shuts), and
    /// the blend goes to the earliest reachable point rather than the
    /// resting point.
    ///
    /// What was established for the bearing-hold itself, against
    /// `OF_TAIL_CHASE`, over 200 fixtures each, reproduced on two
    /// independent builds —
    ///
    /// | | tail chase | this |
    /// |---|---|---|
    /// | aimed AHEAD | 34% | **54%** |
    /// | PARALLEL | 40% | **34%** |
    /// | goals/match (real ~2.5) | 2.48 | 2.52 |
    /// | shots/team (real ~13) | 12.9 | 13.7 |
    /// | pass accuracy (real ~85) | 82.4 | **85.5** |
    /// | tackles/team (real ~18) | 16.0 | 16.1 |
    /// | interceptions/team (real ~10) | 24.3 | 28.1 ← |
    ///
    /// Inside [`Self::SETTLE`] the desired velocity crosses smoothly to
    /// the target's own, because collecting a moving ball means arriving
    /// at its speed rather than braking to a halt beside it — and for a
    /// ball at rest that same term IS braking to a halt, with no branch
    /// needed to say so. It fades out again as the ball outruns him,
    /// since a ball he cannot live with is not being collected and
    /// falling in behind one is the whole defect again at a metre and a
    /// half.
    ///
    /// Worked in the XY plane throughout. The runner cannot fly, and the
    /// stored vectors carry game units in x/y against metres in z, so a
    /// norm taken over all three silently mixes scales.
    Intercept {
        target: Vector3<f32>,
        target_velocity: Vector3<f32>,
    },
    Evade {
        target: Vector3<f32>,
    },
    Wander {
        target: Vector3<f32>,
        radius: f32,
        jitter: f32,
        distance: f32,
        angle: f32,
    },
    Flee {
        target: Vector3<f32>,
    },
    FollowPath {
        waypoints: &'a [Vector3<f32>],
        current_waypoint: usize,
        /// Positional shift applied to the waypoint being steered at —
        /// personal space, as a place to stand rather than a shove.
        ///
        /// Replaces a scalar `path_offset` that was only ever tested for
        /// `> 0.0` (its magnitude was discarded, and several callers were
        /// re-rolling `IntegerUtils::random(1, 10)` into it every tick for
        /// no effect). Callers used to add `separation_velocity()` to the
        /// RESULT instead, which gave the system no resting place: the
        /// behaviour brakes to zero on the waypoint, so avoidance always
        /// won there and pushed the player off, and the waypoint pulled
        /// him back. Shifting the target instead means the player arrives
        /// at a spot a step clear of whoever is crowding him and stops.
        crowd_offset: Vector3<f32>,
    },
}

impl<'a> SteeringBehavior<'a> {
    pub fn calculate(&self, player: &MatchPlayer) -> SteeringOutput {
        match self {
            SteeringBehavior::Seek { target } => {
                let to_target = *target - player.position;
                let max_speed = player.max_speed_with_condition_cached();
                // `normalize()` recomputes the norm — reuse the guard's
                // (`v / n` is exactly what normalize does).
                let distance = to_target.norm();
                let desired_velocity = if distance > 0.0 {
                    (to_target / distance) * max_speed
                } else {
                    Vector3::zeros()
                };

                let steering = desired_velocity - player.velocity;

                let max_force = player.skills.physical.acceleration / 20.0;
                let steering = Self::limit_magnitude(steering, max_force);

                // Apply steering force to get new absolute velocity
                let new_velocity = player.velocity + steering;
                let final_velocity = Self::limit_magnitude(new_velocity, max_speed);

                SteeringOutput {
                    velocity: final_velocity,
                    rotation: 0.0,
                }
            }
            SteeringBehavior::Arrive {
                target,
                slowing_distance,
            } => {
                let to_target = *target - player.position;
                let distance = to_target.norm();

                // Stop if very close to target — larger deadzone prevents oscillation
                const ARRIVAL_DEADZONE: f32 = 3.0;
                if distance < ARRIVAL_DEADZONE {
                    // Apply strong braking — kill velocity quickly to prevent overshoot
                    let braking_force = -player.velocity * 0.8;
                    let new_velocity = player.velocity + braking_force;
                    return SteeringOutput {
                        velocity: new_velocity,
                        rotation: 0.0,
                    };
                }

                let agility_normalized = 0.8 + (player.skills.physical.agility - 1.0) / 19.0;

                // Ensure slowing_distance is never zero but keep it small so players
                // can approach balls near boundaries without crawling to a halt
                let safe_slowing_distance = slowing_distance.max(3.0);

                // Calculate desired speed based on distance (with condition factor)
                // max_speed already incorporates pace, no additional multiplier needed
                let max_speed = player.max_speed_with_condition_cached();
                let desired_speed = if distance < safe_slowing_distance {
                    // Quadratic deceleration — no minimum speed floor so player
                    // can smoothly decelerate to zero without overshooting
                    let ratio = (distance / safe_slowing_distance).clamp(0.0, 1.0);
                    max_speed * ratio * ratio
                } else {
                    max_speed
                };

                // Calculate desired velocity direction. `to_target /
                // distance` == `normalize()` with the norm already in hand.
                let desired_velocity = if distance > 0.0 {
                    (to_target / distance) * desired_speed
                } else {
                    Vector3::zeros()
                };

                // Calculate steering force (the change in velocity needed)
                let steering = desired_velocity - player.velocity;

                // Limit steering force based on agility (with condition).
                // Reuses the `max_speed` computed above — same pure
                // function, same arguments.
                let max_force = max_speed * agility_normalized * 0.7;
                let steering = Self::limit_magnitude(steering, max_force);

                // Apply steering force to current velocity to get new absolute velocity
                let new_velocity = player.velocity + steering;

                // Clamp to max speed
                let final_velocity = Self::limit_magnitude(new_velocity, max_speed);

                SteeringOutput {
                    velocity: final_velocity,
                    rotation: 0.0,
                }
            }
            SteeringBehavior::Pursuit {
                target,
                target_velocity,
            } => {
                // Calculate interception point by predicting where the target will be
                let to_target = *target - player.position;
                let distance = to_target.norm();

                // Deadzone to prevent oscillation when very close
                const PURSUIT_DEADZONE: f32 = 1.5;
                const SLOWING_DISTANCE: f32 = 10.0;

                if distance < PURSUIT_DEADZONE {
                    // Very close to target - apply strong braking
                    let braking_force = -player.velocity * 0.9;
                    let new_velocity = player.velocity + braking_force;
                    return SteeringOutput {
                        velocity: new_velocity,
                        rotation: 0.0,
                    };
                }

                let acceleration_normalized =
                    0.8 + (player.skills.physical.acceleration - 1.0) / 19.0;
                let agility_normalized = 0.8 + (player.skills.physical.agility - 1.0) / 19.0;

                // max_speed already incorporates pace/acceleration via the skill blend
                let max_speed = player.max_speed_with_condition_cached();

                // Calculate interception point
                let interception_point = Self::calculate_interception_point(
                    player.position,
                    *target,
                    *target_velocity,
                    max_speed,
                );

                // Calculate direction to interception point
                let to_interception = interception_point - player.position;
                let interception_distance = to_interception.norm();

                // Calculate desired speed based on distance - slow down when approaching
                let desired_speed = if interception_distance < SLOWING_DISTANCE {
                    // Within slowing distance - reduce speed proportionally
                    let speed_ratio = (interception_distance / SLOWING_DISTANCE).clamp(0.2, 1.0);
                    max_speed * speed_ratio
                } else {
                    // Full speed when far away
                    max_speed
                };

                let desired_velocity = if interception_distance > 0.0 {
                    (to_interception / interception_distance) * desired_speed
                } else {
                    Vector3::zeros()
                };

                // Use direct velocity blending when close to prevent oscillation
                let final_velocity = if interception_distance < SLOWING_DISTANCE {
                    // Close to target - blend toward desired velocity to prevent overshoot
                    let blend_factor = (interception_distance / SLOWING_DISTANCE).clamp(0.0, 1.0);
                    let damping = 0.7 - (blend_factor * 0.3); // More damping when closer

                    desired_velocity * (1.0 - damping) + player.velocity * damping
                } else {
                    // Far from target - use normal steering accumulation.
                    // `max_speed` above is the same pure call — reuse it.
                    let steering = desired_velocity - player.velocity;
                    let max_acceleration = max_speed * agility_normalized * acceleration_normalized;
                    let limited_steering = Self::limit_magnitude(steering, max_acceleration);

                    let move_velocity = player.velocity + limited_steering;
                    Self::limit_magnitude(move_velocity, max_speed)
                };

                SteeringOutput {
                    velocity: final_velocity,
                    rotation: 0.0,
                }
            }
            SteeringBehavior::Intercept {
                target,
                target_velocity,
            } => {
                // A/B control — see `LooseBallChase::tail_chase`.
                if LooseBallChase::tail_chase() {
                    return SteeringBehavior::Pursuit {
                        target: *target,
                        target_velocity: *target_velocity,
                    }
                    .calculate(player);
                }

                let max_speed = player.max_speed_with_condition_cached();
                let here = Self::flat(player.position);
                let own_velocity = Self::flat(player.velocity);
                let to_target = Self::flat(*target) - here;
                let distance = to_target.norm();
                let target_velocity = Self::flat(*target_velocity);

                // Standing on it. Travel with it — for a ball at rest
                // that is a standstill, and there is no line of sight to
                // resolve anything against anyway.
                const ON_IT: f32 = 0.25;
                if distance < ON_IT {
                    return SteeringOutput {
                        velocity: Self::limit_magnitude(target_velocity, max_speed),
                        rotation: 0.0,
                    };
                }
                let line_of_sight = to_target / distance;

                // Split the target's travel into the part that carries it
                // ALONG our line of sight and the part that carries it
                // ACROSS. Only the second can leave us behind, and
                // matching it is what holds the bearing still.
                let across = target_velocity - line_of_sight * target_velocity.dot(&line_of_sight);
                let across_sq = across.norm_squared();
                let speed_sq = max_speed * max_speed;

                // The bearing-hold — exact while the race is winnable.
                let hold = if across_sq >= speed_sq {
                    // He cannot live with it across the line; everything
                    // he has goes sideways. Kept as one END of the blend
                    // below, never the whole answer any more.
                    across * (max_speed / across_sq.sqrt().max(1e-4))
                } else {
                    // Match it across, close the gap with what is left.
                    across + line_of_sight * (speed_sq - across_sq).sqrt()
                };

                // What holding that bearing actually shrinks the gap by,
                // per tick: the speed left under the root, less whatever
                // the ball takes out along the line. When this dies the
                // bearing-hold is treading water — running a line that
                // never converges, the reported "parallel to the ball"
                // frame — while the ball sheds speed it will never get
                // back. The rescue is not a better bearing, it is a
                // different read: the first point on the decaying roll
                // he can be at no later than the ball, straight at it,
                // flat out. Crossed smoothly so no tick can snap the
                // heading, and faded out over `aim`'s own height band —
                // a flying ball's travel does not decay like a roll's,
                // and modelling it as one is how the last attempt at
                // this went wrong (see the history on the variant).
                let closing =
                    (speed_sq - across_sq).max(0.0).sqrt() - target_velocity.dot(&line_of_sight);
                let t = ((target.z - LooseBallChase::GROUND_H)
                    / (LooseBallChase::AERIAL_H - LooseBallChase::GROUND_H))
                    .clamp(0.0, 1.0);
                let grounded = 1.0 - t * t * (3.0 - 2.0 * t);
                let lost = 1.0 - (closing / (Self::LOST_CAUSE * max_speed)).clamp(0.0, 1.0);
                let lost = (lost * lost * (3.0 - 2.0 * lost)) * grounded;

                let mut desired = if lost <= 0.0 || LooseBallChase::concede() {
                    hold
                } else {
                    let (meet, when) = LooseBallChase::earliest_meeting(
                        here,
                        max_speed,
                        Self::flat(*target),
                        target_velocity,
                    );
                    // Commitment is priced in TIME. A meeting he can
                    // make inside a few seconds is attacked flat out; one
                    // half a pitch of roll away is not an interception,
                    // it is following play, and the unbounded version of
                    // this sent players on ten-second cross-field
                    // sprints after balls a real player concedes —
                    // measured at +0.54 goals/match of phantom chance
                    // supply (3×300 fixtures against `OF_CONCEDE`, the
                    // whole rise in shot volume, none in shot quality).
                    // Past [`Self::COMMIT_FAR`] the law is byte-for-byte
                    // the pre-rescue one.
                    let commit = 1.0
                        - ((when - Self::COMMIT_NEAR)
                            / (Self::COMMIT_FAR - Self::COMMIT_NEAR))
                            .clamp(0.0, 1.0);
                    let commit = commit * commit * (3.0 - 2.0 * commit);
                    let cut = (meet - here)
                        .try_normalize(1e-4)
                        .map(|d| d * max_speed)
                        .unwrap_or(hold);
                    hold + (cut - hold) * (lost * commit)
                };

                // Arriving is travelling WITH it, not stopping next to
                // it. Continuous, and it collapses to an ordinary braking
                // arrival whenever the target is standing still.
                //
                // …but only onto something he can live with. A ball
                // moving faster than he can run is not being collected,
                // it is escaping, and settling in behind one is the tail
                // chase again at a metre and a half — the band a third of
                // the census samples sit in. So the settle fades out as
                // the target outruns him and he keeps cutting at it.
                let target_speed = target_velocity.norm();
                let catchable = if target_speed <= max_speed {
                    1.0
                } else {
                    max_speed / target_speed
                };
                let settle = 1.0 - (1.0 - (distance / Self::SETTLE).clamp(0.0, 1.0)) * catchable;
                desired = target_velocity + (desired - target_velocity) * settle;

                let acceleration_normalized =
                    0.8 + (player.skills.physical.acceleration - 1.0) / 19.0;
                let agility_normalized = 0.8 + (player.skills.physical.agility - 1.0) / 19.0;
                let max_acceleration = max_speed * agility_normalized * acceleration_normalized;

                let steering = Self::limit_magnitude(desired - own_velocity, max_acceleration);
                let velocity = Self::limit_magnitude(own_velocity + steering, max_speed);

                SteeringOutput {
                    velocity,
                    rotation: 0.0,
                }
            }
            SteeringBehavior::Evade { target } => {
                let to_player = player.position - *target;
                let max_speed = player.max_speed_with_condition_cached();

                let flee_distance = to_player.norm();
                let desired_velocity = if flee_distance > 0.0 {
                    (to_player / flee_distance) * max_speed
                } else {
                    Vector3::zeros()
                };

                let steering = desired_velocity - player.velocity;

                let max_force = player.skills.physical.acceleration / 20.0;
                let steering = Self::limit_magnitude(steering, max_force);

                // Apply steering force to get new absolute velocity
                let new_velocity = player.velocity + steering;
                let final_velocity = Self::limit_magnitude(new_velocity, max_speed);

                SteeringOutput {
                    velocity: final_velocity,
                    rotation: 0.0,
                }
            }
            SteeringBehavior::Wander {
                target: _,
                radius,
                jitter: _,
                distance,
                angle,
            } => {
                // The wander circle is projected in front of the player.
                // A fully-stopped player has no facing to project from —
                // fall back to +x so idle players still drift instead of
                // freezing (normalize() of a zero vector is NaN, which
                // zeroed the whole wander output).
                let facing = player.velocity.try_normalize(1e-4).unwrap_or(Vector3::x());
                let circle_center = player.position + facing * *distance;

                // Calculate the displacement around the circle using the stored angle
                let displacement = Vector3::new(angle.cos() * *radius, angle.sin() * *radius, 0.0);

                // The wander target is on the circle's edge
                let wander_target = circle_center + displacement;

                // Calculate desired velocity toward the wander target
                let to_target = wander_target - player.position;
                let desired_velocity = if to_target.norm() > 0.0 {
                    to_target.normalize() * player.max_speed_with_condition_cached() * 0.3 // Reduced speed for wandering
                } else {
                    Vector3::zeros()
                };

                let steering = desired_velocity - player.velocity;

                // Limit steering force
                let max_force = player.skills.physical.acceleration / 30.0; // Gentler force
                let steering = Self::limit_magnitude(steering, max_force);

                // Apply steering force to get new absolute velocity
                let new_velocity = player.velocity + steering;
                let wander_max_speed = player.max_speed_with_condition_cached() * 0.3; // Wandering is slower
                let final_velocity = Self::limit_magnitude(new_velocity, wander_max_speed);

                let rotation = if final_velocity.x != 0.0 || final_velocity.y != 0.0 {
                    final_velocity.y.atan2(final_velocity.x)
                } else {
                    0.0
                };

                SteeringOutput {
                    velocity: final_velocity,
                    rotation,
                }
            }
            SteeringBehavior::Flee { target } => {
                let to_player = player.position - *target;
                let max_speed = player.max_speed_with_condition_cached();
                let flee_distance = to_player.norm();
                let desired_velocity = if flee_distance > 0.0 {
                    (to_player / flee_distance) * max_speed
                } else {
                    Vector3::zeros()
                };

                let steering = desired_velocity - player.velocity;
                let max_force = player.skills.physical.acceleration / 20.0;
                let steering = Self::limit_magnitude(steering, max_force);

                // Apply steering force to get new absolute velocity
                let new_velocity = player.velocity + steering;
                let final_velocity = Self::limit_magnitude(new_velocity, max_speed);

                SteeringOutput {
                    velocity: final_velocity,
                    rotation: 0.0,
                }
            }

            SteeringBehavior::FollowPath {
                waypoints,
                current_waypoint,
                crowd_offset,
            } => {
                if waypoints.is_empty() {
                    return SteeringOutput {
                        velocity: Vector3::zeros(),
                        rotation: 0.0,
                    };
                }

                // Get the current target waypoint
                if *current_waypoint >= waypoints.len() {
                    return SteeringOutput {
                        velocity: Vector3::zeros(),
                        rotation: 0.0,
                    };
                }

                // Steer at the waypoint shifted clear of nearby players,
                // so the arrival point itself is somewhere the player can
                // actually come to rest.
                let target = waypoints[*current_waypoint] + *crowd_offset;

                // Calculate distance to current waypoint
                let to_waypoint = target - player.position;
                let distance = to_waypoint.norm();

                let max_speed = player.max_speed_with_condition_cached();

                // Settle on arrival. Without this the desired velocity
                // stayed at full speed right up to the waypoint, so the
                // player overshot and was steered back — the same
                // hunting-around-the-target oscillation `Arrive` avoids
                // with its deadzone.
                const ARRIVAL_DEADZONE: f32 = 3.0;
                if distance < ARRIVAL_DEADZONE {
                    return SteeringOutput {
                        velocity: player.velocity * 0.2,
                        rotation: 0.0,
                    };
                }

                let direction = to_waypoint / distance;
                let desired_velocity = direction * max_speed;
                let steering = desired_velocity - player.velocity;

                // Limit steering force
                let max_force = player.skills.physical.acceleration / 20.0;
                let steering = Self::limit_magnitude(steering, max_force);

                // Apply steering force to get new absolute velocity
                let new_velocity = player.velocity + steering;
                let final_velocity = Self::limit_magnitude(new_velocity, max_speed);

                SteeringOutput {
                    velocity: final_velocity,
                    rotation: 0.0,
                }
            }
        }
    }

    /// Gap inside which [`Intercept`](Self::Intercept) stops trying to
    /// close and simply travels with what it is chasing.
    ///
    /// [`CONTROL_DISTANCE`] — the range at which a player actually takes
    /// the ball — because that is the moment the chase turns into a
    /// first touch, and a number picked separately here would be a
    /// second opinion about the same event.
    const SETTLE: f32 = CONTROL_DISTANCE;

    /// Closing rate, as a fraction of the chaser's top speed, below which
    /// [`Intercept`](Self::Intercept) stops holding the bearing and runs
    /// at the first point of the roll it can make instead.
    ///
    /// Zero is the physical boundary — at zero the bearing-hold shrinks
    /// the gap by nothing per tick — and the band above it exists for
    /// continuity, so a ball hovering at the boundary cannot snap the
    /// heading between two laws. Its width is a judgement about how slow
    /// a race is still worth running as a race: at a quarter of top
    /// speed a 20 u gap takes 170+ ticks to close, and a man who can see
    /// that reads the roll instead.
    const LOST_CAUSE: f32 = 0.25;

    /// Meeting time, in ticks, inside which a lost-cause cut is attacked
    /// at full commitment — four seconds, about the far edge of a real
    /// interception read: the length of a hard 25-30 m run.
    const COMMIT_NEAR: f32 = 200.0;
    /// …and past which it is declined entirely — ten seconds out is not
    /// an interception anybody runs, it is the ball leaving the phase of
    /// play. Between the two the commitment fades smoothly.
    ///
    /// The pair is what separates the rescued population (the reported
    /// "he could have intercepted that": balls up to ~1.3× sprint speed,
    /// met within seconds) from the marathon population the unbounded
    /// rescue invented (the loose-ball SPEED MEAN is 0.892 u/tick, ~2×
    /// sprint, and such a ball crossing with any lateral offset meets a
    /// chaser 10-18 s downstream — nobody real makes that run, and
    /// paying it measured +0.54 goals/match of pure shot volume).
    const COMMIT_FAR: f32 = 500.0;

    /// Drop a stored vector into the plane the runner moves in.
    ///
    /// `x`/`y` are game units and `z` is metres (see `GRAVITY_PER_TICK`),
    /// so any norm or dot product taken across all three mixes two
    /// scales. Nobody chasing a ball can leave the ground anyway, so the
    /// vertical is not merely inconsistent here, it is not part of the
    /// question.
    #[inline]
    fn flat(v: Vector3<f32>) -> Vector3<f32> {
        Vector3::new(v.x, v.y, 0.0)
    }

    fn limit_magnitude(v: Vector3<f32>, max_magnitude: f32) -> Vector3<f32> {
        let current_magnitude = v.norm();
        if current_magnitude > max_magnitude && current_magnitude > 0.0 {
            v * (max_magnitude / current_magnitude)
        } else {
            v
        }
    }

    fn calculate_interception_point(
        pursuer_pos: Vector3<f32>,
        target_pos: Vector3<f32>,
        target_vel: Vector3<f32>,
        pursuer_speed: f32,
    ) -> Vector3<f32> {
        // If target is not moving, just return its current position
        let target_speed = target_vel.norm();
        if target_speed < 0.01 {
            return target_pos;
        }

        // Calculate relative position
        let relative_pos = target_pos - pursuer_pos;
        let distance_sq = relative_pos.norm_squared();

        // If target is very close, just return its current position
        if distance_sq < 1.0 {
            return target_pos;
        }

        // Time to intercept, as a CONTINUOUS function of the geometry.
        //
        // This used to solve the interception quadratic
        // `|relative_pos + target_vel*t| = pursuer_speed*t` and pick a
        // root, with hard fallbacks to a constant when there wasn't one.
        // The maths is right; the shape of the answer is not. Almost
        // everything worth pursuing in this engine moves faster than a
        // player — a rolling ball is ~3.2 u/tick against a 0.63 u/tick
        // sprint — so `a = target_speed² - pursuer_speed²` is positive,
        // the discriminant is usually negative, and which branch fires
        // depends on `b = 2·relative_pos·target_vel`, i.e. on the angle
        // between the chase and the ball's travel. That angle changes
        // constantly during a chase, so the branch flipped constantly,
        // and each flip jumped `t` between a root and the constant.
        //
        // With the clamp below that jump moves the aim point by up to
        // `target_vel * 4.9` — about 15u for a rolling ball — which is
        // more than enough to invert a chaser's heading while he is at
        // full speed. It was the largest genuine source of flicker left
        // in the engine: `Defender: Take Ball` ran 6.03 velocity
        // reversals per second held with essentially ALL of them at
        // running speed (6.06 total), i.e. not settling jitter but a
        // sprinting player visibly snapping around (`dev_match trace`).
        //
        // Replaced with the closing-speed estimate: how long the gap
        // takes to shut, given how fast the target is pulling away along
        // the line of the chase. `max` and `min` are continuous, and
        // there are no branches, so the aim point can no longer jump. The
        // clamp is unchanged, which keeps the lead inside exactly the
        // envelope the rest of the engine is calibrated against.
        //
        // NB the units are TICKS, not seconds — velocities here are
        // u/tick. The old `clamp(0.1, 5.0)` and its "aim ahead by 1
        // second" comments read as seconds but were only ever 1-50 ms of
        // lead. Left at the same numbers deliberately: correcting the
        // lead to a real anticipation window is a much larger behavioural
        // change (it would transform interceptions) and belongs with a
        // calibration pass, not with a flicker fix.
        let to_target = relative_pos / distance_sq.sqrt();
        // How fast the gap actually closes. A target running away along
        // the chase line subtracts from our speed; one coming at us adds.
        // Floored at a quarter of our speed so the estimate stays finite
        // and continuous when the target is outrunning us outright.
        let closing_speed =
            (pursuer_speed - target_vel.dot(&to_target)).max(pursuer_speed * 0.25 + 1e-4);
        let intercept_time = distance_sq.sqrt() / closing_speed;

        // Clamp intercept time to the same range as before.
        let clamped_time = intercept_time.clamp(0.1, 5.0);

        // Calculate predicted position
        target_pos + target_vel * clamped_time
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SteeringOutput {
    pub velocity: Vector3<f32>,
    pub rotation: f32,
}

impl SteeringOutput {
    pub fn new(velocity: Vector3<f32>, rotation: f32) -> Self {
        SteeringOutput { velocity, rotation }
    }
}
