//! **Interception** — an opponent reads the delivery and takes it out of
//! the air on its way to somebody else.

use crate::r#match::ball::events::BallEvent;
use crate::r#match::engine::ball::ball::contest::contact::ContactInPlace;
use crate::r#match::engine::ball::ball::{AerialReach, Ball};
use crate::r#match::events::EventCollection;
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::player::strategies::players::ops::skill_composites as sc;
use crate::r#match::{MatchContext, MatchPlayer, PassOriginRestart};
use nalgebra::Vector3;

/// **The interception duel, priced as a contest.**
///
/// # The defect this fixes
///
/// `try_intercept` scored the chance as `sc::interception(defender)` and
/// nothing else — the man who PLAYED the pass did not appear in it at
/// all. So the rate was a property of one side's ability rather than of
/// the duel between two, and it walked straight up the pyramid with
/// squad quality: measured over 300 matches at each level with EQUAL
/// squads, interceptions ran 13.9 per team at level 4 against 31.7 at
/// level 18, and pass accuracy fell the other way, 89.6% → 79.2%.
///
/// Both of those are backwards. Real football's interception count does
/// not move between divisions, and better passers complete MORE of their
/// passes, not fewer. What actually happens as the standard rises is that
/// the defending improves and the passing improves with it, and the two
/// cancel — which is precisely what a contest does and an absolute skill
/// term cannot.
///
/// # The model
///
/// The same shape as [`SaveModel::skill_multiplier`], for the same
/// reason. An evenly-matched duel resolves to `FLOOR + SLOPE/2` at EVERY
/// level, so the population rate is a property of these constants and not
/// of the standard of football; the edge either man carries is a spread
/// around it. `sc::passing_execution` is the counter-skill because that
/// is what makes a pass hard to read and hard to reach: weight, lane and
/// disguise.
///
/// ⚠ **The population RATE is load-bearing and must not move.** The
/// turnover volume this site produces is what keeps attacking sequences
/// noisy; cutting it has been measured driving goals to 8-11 a match.
/// `PARITY` is therefore pinned to the value `sc::interception` used to
/// return for a mid-pyramid squad, and the check that matters after any
/// change here is that `int/tm` in `dev_match levels` stays near its
/// old level-14 reading (~26) — flat across the sweep, not lower.
pub(crate) struct InterceptionDuel;

impl InterceptionDuel {
    /// Chance multiplier for the worst possible reader of the game
    /// against the best possible delivery.
    const FLOOR: f32 = 0.40;
    /// Width of the duel axis. `FLOOR + SLOPE/2` = **0.66**, which is
    /// what `sc::interception` returned for a mid-pyramid squad — the
    /// level the 0.16 coefficient below was calibrated against.
    const SLOPE: f32 = 0.52;
    /// How much a skill edge is worth. Matches the save contest's own
    /// spread: a 0.38 advantage saturates the axis, and the ±0.2 edges
    /// that occur inside a real squad move the multiplier about ±0.13.
    const SPREAD: f32 = 1.30;

    /// Multiplier an evenly-matched duel resolves to. Quoted so callers
    /// and tests can name the population anchor instead of re-deriving
    /// it from the two constants above.
    #[allow(dead_code)] // quoted anchor: read by tests and by calibration notes
    pub(crate) const PARITY: f32 = Self::FLOOR + Self::SLOPE * 0.5;

    #[inline]
    pub(crate) fn advantage(interceptor: f32, delivery: f32) -> f32 {
        let edge = interceptor.clamp(0.0, 1.0) - delivery.clamp(0.0, 1.0);
        let advantage = (0.5 + edge * Self::SPREAD).clamp(0.0, 1.0);
        Self::FLOOR + advantage * Self::SLOPE
    }
}

impl Ball {
    /// Opposing players near the ball's flight path can intercept passes.
    /// Interception chance depends on tackling, anticipation, positioning skills
    /// and proximity to the ball's trajectory.
    pub fn try_intercept(
        &mut self,
        context: &MatchContext,
        players: &[MatchPlayer],
        events: &mut EventCollection,
    ) {
        // `context` is held even when this site does not currently
        // draw from `context.rng` so future calibration / env-modifier
        // wiring (slide tackle range, sliding_tackle_success) lands
        // without changing the signature again.
        let _ = context;
        // Only intercept unowned balls that are in flight (active pass).
        // A ball in a keeper's gloves is neither, but guard it explicitly
        // — it is the one state where "unowned" could ever be wrong.
        if self.current_owner.is_some() || self.held_in_hands || self.flags.in_flight_state == 0 {
            return;
        }
        // A delivery whose aerial contest is already decided is not up for
        // interception on the way. The contest priced every defender in
        // the box and the keeper's command of his area before the ball
        // left the boot; rolling for a cut-out on top of that is the same
        // double jeopardy the heading states carve out for
        // `aerial_contest_winner`. See [`AerialDelivery`].
        if self.aerial_delivery.is_some() {
            return;
        }

        // HEIGHT IS A DIFFICULTY, NOT A DOOR.
        //
        // This was a single `z > 2.5` gate and nothing else in the whole
        // pass read the height again. Both halves of that were wrong, and
        // they produce the two complaints that sound like opposites:
        //
        //   * Below the bar, a ball at 2.4 m — a foot above a standing
        //     player's head — was exactly as interceptable as one rolling
        //     along the floor. A defender plucked it out of the air
        //     without moving, because nothing asked him to jump and
        //     nothing made it harder that he hadn't.
        //   * Above the bar the ball simply did not exist, whoever was
        //     under it and however well he leaps. 2.51 m was unreachable
        //     for the best header of the ball in the division.
        //
        // The ceiling now belongs to the PLAYER (`AerialReach::ceiling`,
        // from his `jumping`), the chance falls away as the ball climbs
        // toward it, and a defender who takes one above his standing
        // reach actually leaves the ground for it — see the `Leap` event
        // below. The flat cut here only skips the loop early for balls
        // beyond any human being.
        if self.position.z > AerialReach::ceiling(20.0) {
            return;
        }

        // Need to know who passed — for the opposing team, and for the
        // other half of the duel. See `InterceptionDuel`: the delivery is
        // what makes a pass hard to read, and without it this site scored
        // one man's ability instead of a contest between two.
        let passer = match self.previous_owner {
            Some(prev_id) => players.iter().find(|p| p.id == prev_id),
            None => return,
        };
        let passer = match passer {
            Some(p) => p,
            None => return,
        };
        let passer_team = passer.team_id;
        let delivery =
            sc::passing_execution(passer, sc::minute_from_ticks(self.current_tick_cached));

        // Ball velocity determines the interception corridor width.
        //
        // The floor only exists to hand a near-stationary ball to normal
        // claiming — it is not a calibration knob. It was `speed < 1.0`,
        // set when passes were struck at 0.5-2.7 u/tick under friction
        // ~3.7× real. With `GROUND_FRICTION` corrected, a real pass now
        // leaves the foot at 0.5-2.2 and arrives slower still, so a 1.0
        // floor excluded most passes outright and interceptions fell from
        // 37 to 2.6 per team against a real ~10. 0.25 u/tick is 3.1 m/s —
        // the same physical meaning of "the ball is actually travelling"
        // that 1.0 carried before the units moved under it.
        const MIN_INTERCEPTABLE_SPEED: f32 = 0.25;
        let ball_speed_sq = self.velocity.x * self.velocity.x + self.velocity.y * self.velocity.y;
        if ball_speed_sq < MIN_INTERCEPTABLE_SPEED * MIN_INTERCEPTABLE_SPEED {
            return; // Ball too slow, normal claiming handles it
        }

        // Interception reach in game units. Field is 840u = 105m, so 1u =
        // 0.125m. Old 2.5u left average defenders mathematically
        // unable to intercept (max score 0.039 vs 0.04 threshold). 5u
        // produced ~0.1 interceptions/team/match — defenders within
        // the radius hit ~0.025 chance, below the 0.035 threshold for
        // anyone but the closest, fastest, best-positioned. 6.5u
        // (~0.8m — a stretch-extension radius for the planted leg) and
        // a slightly higher base coefficient produces ~10
        // interceptions/team/match (real-football band) without the
        // intercept→snap→re-pass loops the previous 8u radius caused.
        const INTERCEPT_RADIUS: f32 = 5.5;
        const INTERCEPT_RADIUS_SQ: f32 = INTERCEPT_RADIUS * INTERCEPT_RADIUS;

        let mut best_interceptor: Option<u32> = None;
        let mut best_chance: f32 = 0.0;

        for player in players {
            // Only opposing team players can intercept
            if player.team_id == passer_team {
                continue;
            }

            // Don't let the pass target's team intercept their own pass target
            if Some(player.id) == self.pass_target_player_id {
                continue;
            }

            // Distance from player to ball
            let dx = player.position.x - self.position.x;
            let dy = player.position.y - self.position.y;
            let dist_sq = dx * dx + dy * dy;

            if dist_sq > INTERCEPT_RADIUS_SQ {
                continue;
            }

            // How high the ball is, measured against THIS player's leap.
            // Zero means it is over him however well he reads the play,
            // and he is skipped rather than scored at a token chance.
            let height_factor =
                AerialReach::reach_difficulty(self.position.z, player.skills.physical.jumping);
            if height_factor <= 0.0 {
                continue;
            }

            // Base chance: dedicated `interception` composite — anticipation,
            // positioning, concentration, marking, etc. routed through
            // `effective_skill` so fatigue applies. Drop-in replacement for
            // the legacy 4-skill average; magnitude lands in the same band
            // (0..1). Minute derived from the cached tick (10ms ticks).
            let minute = sc::minute_from_ticks(self.current_tick_cached);
            // A CONTEST — see `InterceptionDuel`.
            let skill_factor =
                InterceptionDuel::advantage(sc::interception(player, minute), delivery);

            // Proximity factor: closer = higher chance (1.0 at 0m, 0.3 at max radius)
            let dist = dist_sq.sqrt();
            let proximity_factor = 1.0 - (dist / INTERCEPT_RADIUS) * 0.7;

            // Fast passes are harder to intercept — penalty coefficient
            // moderated from 0.10 (which made 7 u/tick passes 41% harder
            // than slow ones) back toward a lighter slope.
            let speed_penalty = 1.0 / (1.0 + ball_speed_sq.sqrt() * 0.06);

            // Per-tick interception chance. The 0.13 coefficient with
            // the 0.035 threshold mathematically excluded average
            // defenders (skill 0.5 × proximity 0.65 × speed 0.6 ≈
            // 0.025 per the old radius), so observed interceptions
            // were ~0.1/team/match vs real ~10/team. 0.16 (with the
            // bumped 5.5u radius and lowered 0.030 threshold) brings
            // an average-positioned defender to ~0.038 (above
            // threshold), and an elite defender at point-blank to
            // ~0.07, while still leaving peripheral or off-the-pace
            // defenders below the bar. Population per-team
            // interceptions land near 12–13/match.
            let chance = skill_factor * proximity_factor * speed_penalty * height_factor * 0.16;

            if chance > best_chance {
                best_chance = chance;
                best_interceptor = Some(player.id);
            }
        }

        // ONE PASS, ONE ATTEMPT — and it is a roll, not a threshold.
        //
        // This used to fire deterministically whenever `best_chance`
        // cleared 0.030, re-evaluated every tick the ball was in flight.
        // That made the interception RATE a function of how long the
        // flight window happened to be rather than of the defending, and
        // the previous note here recorded the consequence honestly: ~120
        // interceptions per team against a real ~10, "~3× of that from
        // the flight-protection extension tripling the per-pass intercept
        // window". Correcting `GROUND_FRICTION` made the flights longer
        // and more realistic still, and the deterministic form promptly
        // ran to 1000+ per team.
        //
        // The chance the loop builds is already a per-event probability,
        // so roll it. Latch on the first tick a defender is genuinely in
        // reach — that is the moment the ball comes past him, and he gets
        // one go at it, exactly as `try_block_shot` gives one roll per
        // shot. Rate is now independent of the window length.
        // ── A SHOT IS NOT A PASS, AND THIS SITE ONLY KNOWS ABOUT PASSES ──
        //
        // Live shots used to keep a per-tick DETERMINISTIC path here
        // (`best_chance > 0.030`, re-evaluated every tick of the flight),
        // on the argument that `try_block_shot` was too weak to carry the
        // channel. Measured with the shot-lifecycle census, that is what
        // it was actually doing:
        //
        //   * **72.6% of every shot struck ended here** — claimed clean,
        //     mid-flight, by an outfield defender. Not blocked, not
        //     deflected: `velocity = zeros()` and possession handed over.
        //   * Shots lived **8.3 ticks** on average despite being struck
        //     from 102u (12.8 m), a distance that needs 40-60 ticks of
        //     flight. They were being eaten within a tick of leaving the
        //     boot.
        //   * Only 11% of shots ever reached the goal at all, and those
        //     that did were struck from 74u against 102u for the
        //     population — so the leak was distance-selective and long
        //     shots produced essentially no goals.
        //
        // That is the "aimed on frame but never resolves" report: 63% of
        // shots leave the boot between the posts and 9% are credited on
        // target. It is also why blocks read 4.84 per defender against a
        // real ~0.9 — this path was filing its takings as blocks.
        //
        // A defender getting a body in front of a strike is
        // `try_block_shot`: one roll per shot, a real corridor, a real
        // height limit, and a DEFLECTION rather than a clean pick-up.
        // Shots are excluded here so that model owns them, which is also
        // what makes its rate rise — it rolls on the first tick a
        // defender is in the lane, and shots now survive long enough to
        // find one.
        // `cached_shot_target` is exactly the right test: it is set at the
        // strike and cleared the moment anybody touches the ball, so a
        // shot the keeper has parried or a defender has deflected is a
        // genuine loose ball again and IS interceptable from here.
        if self.cached_shot_target.is_some() {
            return;
        }
        if let Some(interceptor_id) = best_interceptor.filter(|_| !self.intercept_rolled) {
            self.intercept_rolled = true;
            let fires = context.rng.unit_f32() < best_chance;
            if fires {
                // A ball taken above standing reach is taken in the air.
                // The ball code holds the squad immutably, so it asks for
                // the jump rather than performing it — see
                // `PlayerEvent::Leap`.
                let interceptor_jumping = players
                    .iter()
                    .find(|p| p.id == interceptor_id)
                    .map_or(10.0, |p| p.skills.physical.jumping);
                let leap = AerialReach::leap_for(self.position.z, interceptor_jumping);
                if leap > 0.0 {
                    events.add_player_event(PlayerEvent::Leap(interceptor_id, leap));
                }

                // How high was it, and was he off the ground? A ball
                // picked out of the air at head height by a man standing
                // flat-footed is the reported symptom, and no existing
                // counter can see it.
                #[cfg(feature = "match-logs")]
                {
                    let airborne = players
                        .iter()
                        .find(|p| p.id == interceptor_id)
                        .is_some_and(|p| p.is_airborne());
                    crate::r#match::engine::ball::ball::flight_diag::FlightDiag::note_intercept(
                        self.position.z,
                        AerialReach::STANDING,
                        airborne || leap > 0.0,
                    );
                }
                // Snap the ball to the interceptor and zero the
                // velocity. Before this, velocity was just scaled to
                // Zeroing velocity + handing ownership to the defender
                // prevents the old "own-goal after intercept" bug without
                // needing to teleport the ball. `move_to` will track the
                // ball toward its new owner at 1.5 u/tick over the next
                // 2-3 ticks, so visually the ball decelerates into the
                // defender's feet instead of jumping instantly from its
                // flight path onto the defender — which was visible to
                // the user as "ball appearing on another player without
                // moving".
                //
                // OG risk is fully handled by `self.velocity = zeros()`:
                // a stationary ball can't roll past the 15u owner-drop
                // threshold, so it can't cross the goal line unowned.
                let _ = interceptor_id; // no teleport, keep position as-is
                self.current_owner = Some(interceptor_id);
                self.pass_target_player_id = None;
                self.flags.in_flight_state = 0;
                self.claim_cooldown = 15;
                self.velocity = Vector3::zeros();
                // No height write. The interceptor owns the ball from this
                // line, so `move_to`'s `carry_toward` walks it down to his
                // carry height at 10 cm/tick — the machinery that exists
                // for exactly this. `position.z = 0.0` here was a snap of
                // up to 3.1 m (the gate's own ceiling) in one tick, and
                // `flight_diag` is blind to the axis, so it never showed.
                if !ContactInPlace::armed() {
                    self.position.z = 0.0;
                }
                // Interception ends any in-flight shot — a defender taking
                // control downfield extinguishes the shot. Without this,
                // the next time the keeper grabs a moving ball from an
                // opponent (a long pass that loops to them), the stale
                // shot flag credits a phantom save and inflates the
                // saves/on-target ratio above 100%.
                //
                // Note what was extinguished: if this was a live shot the
                // defender did not intercept a pass, he blocked a strike,
                // and that is what the stat sheet should say. Captured
                // before the flag is cleared and carried on the event.
                // A shot the defender got a body in front of is a BLOCK,
                // and the stat sheet should say so. Keying that purely
                // off `cached_shot_target` under-reported it badly: the
                // target is cleared by several paths (a failed save, a
                // keeper touch, a deflection) while the ball is still
                // very much a shot in flight, and every stop after that
                // point was filed as an ordinary interception. Blocks
                // measured 0.18 per defender against a real ~0.9 for
                // exactly this reason. `last_shot_struck_tick` is the
                // robust question — was this BALL struck at goal
                // recently — and is cleared on any dead ball.
                let was_live_shot = self.cached_shot_target.is_some()
                    || (self.last_shot_struck_tick > 0
                        && self
                            .current_tick_cached
                            .saturating_sub(self.last_shot_struck_tick)
                            < 400);
                self.cached_shot_target = None;
                let interceptor_team = players
                    .iter()
                    .find(|p| p.id == interceptor_id)
                    .map(|p| p.team_id)
                    .unwrap_or(0);
                let tick = self.current_tick_cached;
                self.record_touch(interceptor_id, interceptor_team, tick, true);
                self.offside_snapshot = None;
                self.pass_origin_restart = PassOriginRestart::OpenPlay;
                events.add_ball_event(BallEvent::Intercepted(
                    interceptor_id,
                    self.previous_owner,
                    was_live_shot,
                ));
            }
        }
    }
}

#[cfg(test)]
mod interception_duel_tests {
    use super::InterceptionDuel;

    /// **The population interception rate must not be a function of the
    /// division.** This is the whole point of the contest: an evenly
    /// matched duel resolves to the same multiplier whether it is two
    /// fourth-tier players or two internationals, so the rate the 0.16
    /// coefficient was calibrated against survives everywhere.
    ///
    /// It is also the guard on a load-bearing number. The turnover volume
    /// this site produces is what keeps attacking sequences noisy, and
    /// cutting it has been measured driving goals to 8-11 a match — so a
    /// change here that quietly moves parity moves the whole engine.
    #[test]
    fn an_even_duel_resolves_identically_at_every_level() {
        for level in [0.20_f32, 0.35, 0.50, 0.66, 0.80, 0.95] {
            let even = InterceptionDuel::advantage(level, level);
            assert!(
                (even - InterceptionDuel::PARITY).abs() < 1e-6,
                "parity at {level} was {even}, expected {}",
                InterceptionDuel::PARITY
            );
        }
    }

    /// The reader of the game still beats the poor passer, and the good
    /// passer still beats the poor reader — the contest is a spread
    /// around parity, not a flattening of the skill axis.
    #[test]
    fn the_edge_still_decides_the_duel() {
        let sharp_vs_sloppy = InterceptionDuel::advantage(0.80, 0.40);
        let sloppy_vs_sharp = InterceptionDuel::advantage(0.40, 0.80);
        assert!(sharp_vs_sloppy > InterceptionDuel::PARITY);
        assert!(sloppy_vs_sharp < InterceptionDuel::PARITY);
        // Symmetric about parity, so neither side of the duel is
        // structurally favoured the way the old one-sided form was.
        let above = sharp_vs_sloppy - InterceptionDuel::PARITY;
        let below = InterceptionDuel::PARITY - sloppy_vs_sharp;
        assert!((above - below).abs() < 1e-6, "{above} vs {below}");
    }

    /// Monotone in both arguments, and bounded — a defender who cannot
    /// read the game at all still gets something, and the best reader
    /// alive against the worst delivery does not get a certainty.
    #[test]
    fn the_axis_is_monotone_and_bounded() {
        let mut previous = 0.0;
        for step in 0..=20 {
            let skill = step as f32 / 20.0;
            let v = InterceptionDuel::advantage(skill, 0.5);
            assert!(v >= previous, "not monotone at {skill}");
            assert!((0.0..=1.0).contains(&v), "out of range at {skill}: {v}");
            previous = v;
        }
        assert!(InterceptionDuel::advantage(0.0, 1.0) > 0.0);
        assert!(InterceptionDuel::advantage(1.0, 0.0) < 1.0);
    }
}
