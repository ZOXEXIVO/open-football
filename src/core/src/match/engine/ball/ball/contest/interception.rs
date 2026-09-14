//! **Interception** — an opponent reads the delivery and takes it out of
//! the air on its way to somebody else.

use crate::r#match::ball::events::BallEvent;
use crate::r#match::engine::ball::ball::contest::contact::ContactInPlace;
#[cfg(feature = "match-logs")]
use crate::r#match::engine::ball::ball::flight_diag::FlightDiag;
#[cfg(feature = "match-logs")]
use crate::r#match::engine::ball::ball::lane_diag;
#[cfg(feature = "match-logs")]
use crate::r#match::engine::ball::ball::strike_diag::{GrantPath, StrikeCensus};
use crate::r#match::engine::ball::ball::{
    AerialReach, Ball, CONTROL_DISTANCE, LOOSE_CLAIM_DISTANCE,
};
use crate::r#match::engine::teamplay::standard::MatchStandard;
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
/// The duel says WHO wins a read; how often a read is won at all is
/// [`InterceptionContest`], and the check that matters after any change
/// to either is that `int/tm` in `dev_match levels` is flat across the
/// sweep.
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

/// **The take itself: one roll per man, at his closest approach.**
///
/// [`InterceptionDuel`] prices who wins a read; this prices whether a
/// man the ball actually passes gets his foot on it. Every opponent the
/// ball draws level with rolls once, on that tick, against how far he
/// has to reach for it, how hard it was struck, how high it is and how
/// long he has had since it left the boot.
///
/// # What it replaces
///
/// One roll per PASS, latched on the first tick anybody came within 5.5u
/// of the ball. Booked over 100 fixtures at L14 (`dev_match stats`, PASS
/// LANE CENSUS) that roll was made 1464 times a match at a mean 0.52 m
/// from the ball — the man standing on the passer as he struck it, not
/// the man downstream the ball was played through — at p=0.046, and it
/// was the only roll the pass got. The defender the ball then rolled
/// through at 40 cm had no contest at all: of the passes that came
/// within a metre of an opponent on the way, 85% were completed anyway.
pub(crate) struct InterceptionContest;

impl InterceptionContest {
    /// **Clean possession happens at the foot**: 8u = 1 m, a stride.
    /// Deliberately inside the pass block's 14u lunge corridor, because
    /// that channel is the stretch — a leg thrown at the ball, which
    /// deflects it and leaves it loose. Taking a ball cleanly out of
    /// flight from a metre and a half away is the generosity that had
    /// midfielders intercepting 8 passes a match against a real ~1.
    pub const REACH: f32 = 8.0;
    /// **The most ground a man will give up to contest a pass**: 24u =
    /// 3 m. He steps into the lane; he does not abandon his position to
    /// make a cover run for it.
    ///
    /// Without a bound the arithmetic is a full sprint for the whole
    /// flight — on a 20 m pass that is eight metres of lateral closing,
    /// so every opponent within eight metres of the line priced as
    /// reachable. The passer refused lanes that were in fact open, and
    /// passes into the box fell 35% for midfielders while the balls he
    /// did play were cut out at 30% against a priced 50%.
    const STEP_IN: f32 = 24.0;
    /// A driven ball is harder to take cleanly: the chance halves at
    /// 3 u/tick (37 m/s), so a 1 u/tick pass keeps three quarters of it.
    const SPEED_DRAG: f32 = 0.33;
    /// How long after the strike a man needs before he can act on a
    /// pass: a third of a second for the sharpest reader of the game,
    /// seven tenths for the dullest — a reaction and a step, which is the
    /// floor a human body has however well he reads. One clock for both
    /// halves of the question: whether he SETS OFF for the ball at all
    /// (`LooseBallChase`) and how ready his foot is when it draws level
    /// with him, which is nothing until half the clock has run. A pass
    /// played past somebody 3u away is gone before he has moved: with
    /// everybody ready inside 0.3 s, rolls within 0.2 s of the strike
    /// alone took 66 passes a match.
    const READ_QUICK: f32 = 35.0;
    const READ_SLOW: f32 = 70.0;
    /// What a mid-pyramid reader of the game scores — the same reading
    /// [`InterceptionDuel::PARITY`] is pinned to. The clock is quoted
    /// against it so a man is early or late compared with his PEERS
    /// rather than with a fixed 0-20 scale; see [`Self::read_delay`].
    const TYPICAL_READ: f32 = 0.66;
    /// Scale. **Measured, not chosen** — the same argument
    /// [`Ball::BLOCK_GAIN`] carries: the population interception rate is
    /// calibrated and the GEOMETRY was what was wrong, so the rate is
    /// held where it was while the rolls move to where the ball actually
    /// passes a man.
    ///
    /// Titrated on the level sweep (60 fixtures at each of levels 4-18),
    /// which is the reading that matters because it shows the rate AND
    /// whether it walks up the pyramid. At 0.70 the contest ran 50-67
    /// interceptions a team against the 30-38 the one-roll-per-pass
    /// model ran over the same sweep.
    const GAIN: f32 = 0.40;
    /// Nothing is a certainty.
    pub(crate) const CAP: f32 = 0.95;

    /// Ticks after a strike before a man has read the ball.
    ///
    /// ⚠ **`read` must be PEER-RELATIVE** — `MatchStandard::peer` of his
    /// interception composite, not the raw composite. This is the one
    /// absolute skill term the contest had, and absolute skill terms walk
    /// straight up the pyramid: [`InterceptionDuel`] cancels because it
    /// scores a DIFFERENCE, and a clock scored on one man's ability alone
    /// has nothing on the passer's side to cancel it. Measured over a
    /// level sweep (60 fixtures at each of levels 4-18), the absolute form
    /// ran 37 interceptions a team at level 4 against 72 at level 18,
    /// against a baseline that ran 28 to 38 — the same defect, in the same
    /// channel, that the duel was introduced to fix.
    #[inline]
    pub fn read_delay(read: f32) -> f32 {
        Self::READ_SLOW - (Self::READ_SLOW - Self::READ_QUICK) * read.clamp(0.0, 1.0)
    }

    /// The clock for a man who reads the game like his peers — what the
    /// chase election waits out before it will send anybody after a pass.
    /// The election is a question about a HUMAN BODY ("has he reacted
    /// yet"), not about who is the better reader, so it does not carry a
    /// skill term at all and cannot walk with the division.
    #[inline]
    pub fn typical_delay() -> f32 {
        Self::read_delay(Self::TYPICAL_READ)
    }

    /// **How near the pass line he will be when the ball draws level
    /// with him**, given where he stands now (`perp` off the line), how
    /// long the ball takes to get there, and how quickly he reads it.
    ///
    /// The one place this is worked out, because the passer prices a
    /// lane with it before the ball is struck and the census books it
    /// afterwards — an estimate the two disagreed on would be a passer
    /// playing into a lane nobody could explain. A man who has to move
    /// settles a stride off the line, the loose-ball claim band, rather
    /// than arriving exactly on it.
    pub fn closing_miss(perp: f32, arrives: f32, read: f32, max_speed: f32, shift: f32) -> f32 {
        let delay = Self::read_delay(MatchStandard::peer(read, shift));
        let closing = ((arrives - delay).max(0.0) * max_speed).min(Self::STEP_IN);
        if closing > 0.0 {
            (perp - closing).max(perp.min(LOOSE_CLAIM_DISTANCE))
        } else {
            perp
        }
    }

    /// `read` is the man's interception composite and `delivery` the
    /// passer's execution — the two halves of [`InterceptionDuel`], both
    /// RAW, because the duel scores the difference and a difference is
    /// already peer-relative. `shift` is [`MatchStandard::shift`], and it
    /// is what keeps the readiness clock from walking up the pyramid.
    #[allow(clippy::too_many_arguments)]
    pub fn chance(
        miss: f32,
        speed: f32,
        ticks_since_strike: f32,
        height_factor: f32,
        read: f32,
        delivery: f32,
        shift: f32,
    ) -> f32 {
        let stretch = (miss / Self::REACH).clamp(0.0, 1.0);
        let reach = 1.0 - stretch * stretch;
        let pace = 1.0 / (1.0 + speed.max(0.0) * Self::SPEED_DRAG);
        let half = Self::read_delay(MatchStandard::peer(read, shift)) * 0.5;
        let t = ((ticks_since_strike - half) / half).clamp(0.0, 1.0);
        let ready = t * t * (3.0 - 2.0 * t);
        let skill = InterceptionDuel::advantage(read, delivery);
        (reach * pace * ready * height_factor * skill * Self::GAIN).clamp(0.0, Self::CAP)
    }
}

impl Ball {
    /// Every opponent the ball draws level with gets one go at it — see
    /// [`InterceptionContest`]. Runs only on unowned balls in flight.
    pub fn try_intercept(
        &mut self,
        context: &MatchContext,
        players: &[MatchPlayer],
        events: &mut EventCollection,
    ) {
        // A ball in a keeper's gloves is neither owned nor in flight, but
        // guard it explicitly — it is the one state where "unowned" could
        // ever be wrong.
        if self.current_owner.is_some() || self.held_in_hands || self.flags.in_flight_state == 0 {
            return;
        }
        // A PASS — a ball on its way to a named man. A dribbler's heavy
        // touch also leaves the ball unowned and moving, with the man
        // closing him down half a metre away, and measured that was 787
        // rolls a match at 0.47 m filed as interceptions: it is the
        // tackle's business, not this contest's.
        if self.pass_target_player_id.is_none() {
            return;
        }
        // A delivery whose aerial contest is already decided priced every
        // defender in the box before the ball left the boot; rolling for
        // a cut-out on top is the double jeopardy the heading states carve
        // out for `aerial_contest_winner`. See [`AerialDelivery`].
        if self.aerial_delivery.is_some() {
            return;
        }
        // Beyond any human being. The ceiling belongs to the PLAYER
        // (`AerialReach::ceiling`) and the chance falls away toward it in
        // the loop below; this only skips the loop for balls nobody could
        // reach.
        if self.position.z > AerialReach::ceiling(20.0) {
            return;
        }
        // A SHOT IS NOT A PASS. A defender getting a body in front of a
        // strike is `try_block_shot`: one roll per shot, a real corridor, a
        // deflection rather than a clean pick-up. Left to this site, 72.6%
        // of shots were once claimed clean mid-flight. `cached_shot_target`
        // is set at the strike and cleared the moment anybody touches the
        // ball, so a parried or deflected shot is a genuine loose ball
        // again and IS interceptable from here.
        if self.cached_shot_target.is_some() {
            return;
        }
        // Who played it: for the opposing side, and for the other half of
        // the duel.
        let Some(passer) = self
            .previous_owner
            .and_then(|id| players.iter().find(|p| p.id == id))
        else {
            return;
        };
        let passer_team = passer.team_id;
        let minute = sc::minute_from_ticks(self.current_tick_cached);
        let delivery = sc::passing_execution(passer, minute);
        let shift = MatchStandard::shift(context);
        // The last stride and a half to its man is the receiver's: what
        // happens there is his first touch under pressure and the duel
        // after it, both priced elsewhere, not an interception.
        let Some(target) = self
            .pass_target_player_id
            .and_then(|id| players.iter().find(|p| p.id == id))
        else {
            return;
        };
        if (self.position.x - target.position.x).hypot(self.position.y - target.position.y)
            <= CONTROL_DISTANCE
        {
            return;
        }

        // Below this the ball is trickling, not travelling, and the claim
        // scan owns it. 0.25 u/tick is 3.1 m/s.
        const MIN_INTERCEPTABLE_SPEED: f32 = 0.25;
        let speed = (self.velocity.x * self.velocity.x + self.velocity.y * self.velocity.y).sqrt();
        if speed < MIN_INTERCEPTABLE_SPEED {
            return;
        }
        let dir_x = self.velocity.x / speed;
        let dir_y = self.velocity.y / speed;
        // How far a man moves in a tick, so the crossing test still finds
        // one running alongside the ball.
        const STRIDE: f32 = 0.7;
        let since_strike = self
            .current_tick_cached
            .saturating_sub(self.last_release_tick) as f32;

        for (slot, player) in players.iter().enumerate() {
            if player.team_id == passer_team {
                continue;
            }
            let bit = 1u64 << slot;
            if self.intercept_rolled & bit != 0 {
                continue;
            }
            let dx = player.position.x - self.position.x;
            let dy = player.position.y - self.position.y;
            // Positive while the ball is still travelling toward him.
            let along = dx * dir_x + dy * dir_y;
            // His roll is at his closest approach — the tick the ball draws
            // level with him. A man it was already moving away from (the
            // presser it was played past at the kick, a chaser behind it)
            // never crosses, so never rolls.
            if along > 0.0 || along < -(speed + STRIDE) {
                continue;
            }
            let miss = (dx * dx + dy * dy).sqrt();
            #[cfg(feature = "match-logs")]
            if since_strike >= 20.0 {
                if let Some(census) = self.lane_census.as_mut() {
                    census.note_gap(miss);
                }
            }
            if miss > InterceptionContest::REACH {
                continue;
            }
            // One go, whatever the height: a ball over his head at the
            // crossing was his moment too.
            self.intercept_rolled |= bit;
            let height_factor =
                AerialReach::reach_difficulty(self.position.z, player.skills.physical.jumping);
            if height_factor <= 0.0 {
                continue;
            }
            let chance = InterceptionContest::chance(
                miss,
                speed,
                since_strike,
                height_factor,
                sc::interception(player, minute),
                delivery,
                shift,
            );
            let fires = context.rng.unit_f32() < chance;
            #[cfg(feature = "match-logs")]
            {
                let flat = |a: Vector3<f32>, b: Vector3<f32>| (a.x - b.x).hypot(a.y - b.y);
                lane_diag::LaneDiag::note_roll(
                    chance,
                    miss,
                    since_strike,
                    fires,
                    flat(self.position, target.position),
                    flat(self.position, self.last_release_position),
                    flat(player.position, target.position),
                    speed,
                    self.lane_census
                        .as_ref()
                        .and_then(|c| c.perp_at_strike(player.id)),
                    InterceptionContest::REACH,
                    &player.state,
                );
            }
            if fires {
                #[cfg(feature = "match-logs")]
                {
                    let in_box = context.penalty_area(true).contains(&self.position)
                        || context.penalty_area(false).contains(&self.position);
                    crate::mid_run_diag::BoxPassDiag::note_cut_out(in_box);
                }
                self.take_intercepted(player, events);
                return;
            }
        }
    }

    /// The ball is his: stopped where it is and drawn in by `move_to`,
    /// exactly as every other grant — see [`ContactInPlace`].
    fn take_intercepted(&mut self, interceptor: &MatchPlayer, events: &mut EventCollection) {
        // A ball taken above standing reach is taken in the air. The ball
        // code holds the squad immutably, so it asks for the jump rather
        // than performing it — see `PlayerEvent::Leap`.
        let leap = AerialReach::leap_for(self.position.z, interceptor.skills.physical.jumping);
        if leap > 0.0 {
            events.add_player_event(PlayerEvent::Leap(interceptor.id, leap));
        }
        #[cfg(feature = "match-logs")]
        {
            FlightDiag::note_intercept(
                self.position.z,
                AerialReach::STANDING,
                interceptor.is_airborne() || leap > 0.0,
            );
            StrikeCensus::note_grant(GrantPath::CONTEST, self.position.z);
        }
        self.current_owner = Some(interceptor.id);
        self.pass_target_player_id = None;
        self.flags.in_flight_state = 0;
        self.claim_cooldown = 15;
        // Stopped dead: a stationary ball cannot roll past the owner-drop
        // threshold and cross a line unowned.
        self.velocity = Vector3::zeros();
        // No height write: `move_to`'s `carry_toward` walks it down to his
        // carry height, which is the machinery that exists for this.
        if !ContactInPlace::armed() {
            self.position.z = 0.0;
        }
        // A shot the defender got a body in front of is a BLOCK, and the
        // stat sheet should say so. `cached_shot_target` alone under-reports
        // it — several paths clear it while the ball is still a shot in
        // flight — so `last_shot_struck_tick` is the robust question.
        let was_live_shot = self.cached_shot_target.is_some()
            || (self.last_shot_struck_tick > 0
                && self
                    .current_tick_cached
                    .saturating_sub(self.last_shot_struck_tick)
                    < 400);
        self.cached_shot_target = None;
        let tick = self.current_tick_cached;
        self.record_touch(interceptor.id, interceptor.team_id, tick, true);
        self.offside_snapshot = None;
        self.pass_origin_restart = PassOriginRestart::OpenPlay;
        events.add_ball_event(BallEvent::Intercepted(
            interceptor.id,
            self.previous_owner,
            was_live_shot,
        ));
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

#[cfg(test)]
mod interception_contest_tests {
    use super::InterceptionContest;

    const READY: f32 = 100.0;

    /// The contest at the standard of football it is being played at —
    /// `shift` is zero for a mid-pyramid match, and the level sweep is
    /// what guards the rest.
    fn chance(miss: f32, speed: f32, ticks: f32, height: f32, read: f32, delivery: f32) -> f32 {
        InterceptionContest::chance(miss, speed, ticks, height, read, delivery, 0.0)
    }

    /// A ball through an even reader's feet is a real chance — not a
    /// certainty, and worth several times one he has to stretch for.
    /// The LEVEL is calibration and lives in `GAIN`, titrated on the
    /// level sweep, so this pins the shape and only bounds the level.
    #[test]
    fn a_ball_through_his_feet_is_a_real_chance() {
        let feet = chance(0.0, 1.0, READY, 1.0, 0.5, 0.5);
        assert!((0.05..=0.50).contains(&feet), "{feet}");
        let stretch = chance(InterceptionContest::REACH * 0.75, 1.0, READY, 1.0, 0.5, 0.5);
        assert!(feet > stretch * 2.0, "{feet} vs {stretch}");
    }

    /// The further he has to reach the less he gets, down to nothing at
    /// the edge of a stride and a leg.
    #[test]
    fn the_chance_falls_with_the_stretch_to_zero_at_the_edge() {
        let mut previous = f32::MAX;
        for step in 0..=12 {
            let miss = step as f32;
            let p = chance(miss, 1.0, READY, 1.0, 0.5, 0.5);
            assert!(p <= previous, "not monotone at {miss}u");
            previous = p;
        }
        assert_eq!(
            chance(InterceptionContest::REACH, 1.0, READY, 1.0, 1.0, 0.0),
            0.0
        );
        assert_eq!(
            chance(InterceptionContest::REACH + 5.0, 1.0, READY, 1.0, 1.0, 0.0),
            0.0
        );
    }

    /// A driven ball is harder to take cleanly than a rolled one.
    #[test]
    fn a_driven_ball_is_harder() {
        let rolled = chance(0.0, 0.5, READY, 1.0, 0.5, 0.5);
        let driven = chance(0.0, 2.5, READY, 1.0, 0.5, 0.5);
        assert!(rolled > driven * 1.3, "{rolled} vs {driven}");
    }

    /// Nothing at the instant of the strike, everything once he has read
    /// it: the presser the ball is played past is not an interceptor.
    #[test]
    fn he_needs_time_to_read_it() {
        assert_eq!(chance(0.0, 1.0, 0.0, 1.0, 1.0, 0.0), 0.0);
        assert_eq!(chance(0.0, 1.0, 15.0, 1.0, 1.0, 0.0), 0.0);
        let early = chance(0.0, 1.0, 30.0, 1.0, 0.5, 0.5);
        let late = chance(0.0, 1.0, 100.0, 1.0, 0.5, 0.5);
        assert!(early < late * 0.2, "{early} vs {late}");
        assert_eq!(late, chance(0.0, 1.0, 300.0, 1.0, 0.5, 0.5));
    }

    /// The sharper reader is on it sooner — a third of a second for the
    /// best against seven tenths for the worst, and the ready curve
    /// follows. Quoted against his PEERS, so the clock is the same at
    /// every level of the pyramid.
    #[test]
    fn the_sharper_reader_is_ready_sooner() {
        assert!(InterceptionContest::read_delay(1.0) < InterceptionContest::read_delay(0.0));
        assert!((InterceptionContest::read_delay(1.0) - 35.0).abs() < 1e-6);
        assert!((InterceptionContest::read_delay(0.0) - 70.0).abs() < 1e-6);
        let sharp = chance(0.0, 1.0, 30.0, 1.0, 0.9, 0.5);
        let dull = chance(0.0, 1.0, 30.0, 1.0, 0.3, 0.5);
        assert!(sharp > dull * 2.0, "{sharp} vs {dull}");
    }

    /// The best reader alive against the worst delivery, a ball rolled
    /// gently onto his boot: the top of the axis, and still not a
    /// certainty.
    #[test]
    fn nothing_is_a_certainty() {
        let best = chance(0.0, 0.25, READY, 1.0, 1.0, 0.0);
        assert!(best < InterceptionContest::CAP, "{best}");
        let worst = chance(0.0, 0.25, READY, 1.0, 0.0, 1.0);
        assert!(best > worst * 2.0, "{best} vs {worst}");
    }
}
