//! **The foot in** — a defender's leg, shin or body in the path of a
//! PASS, as opposed to a shot.
//!
//! # The hole this fills
//!
//! Reported from the viewer: *"the attacking team calmly passes through
//! the defenders in the penalty area, without them even attempting to
//! intercept the ball or put a foot in to block it."*
//!
//! That is a literally accurate description of the model. Two contests
//! can touch a ball in flight and neither of them is this one:
//!
//! * `Ball::try_block_shot` opens with
//!   `let shot_target = self.cached_shot_target else { return }`. It sees
//!   shots and nothing else.
//! * `Ball::try_intercept` is the only model that
//!   ever looks at a pass, and it scores nobody outside
//!   `INTERCEPT_RADIUS` — **5.5u, 69 cm**. That is a standing radius: the
//!   ball has to come within two thirds of a metre of where the defender
//!   is already stood. It also grants CLEAN POSSESSION when it fires,
//!   which is not what getting a toe to a ball does.
//!
//! Measured with the BOX-PASS census (`mid_run_diag::BOXPASS_SAMPLES`),
//! 200 fixtures at L14, over 1.7 M ticks of a pass in flight inside the
//! defending side's own penalty area: **4.4 defenders are standing in
//! that area**, the nearest is **3.73 m from the ball and 1.85 m off its
//! line**, and only **4.7% of those ticks** ever put anybody inside the
//! interception radius. The bodies are there; nothing lets them do
//! anything. The stat sheet says the same thing from the other side —
//! blocks 0.39 per defender against a real ~0.9, clearances 1.67 against
//! ~3.5, tackles 9.8 a team against ~18, while interceptions run 26.6
//! against ~10. The engine wins the ball by standing in the right place
//! and almost never by doing something.
//!
//! # What this is, and what it deliberately is not
//!
//! It is a **deflection**, not a takeaway. A blocked pass in football is
//! a loose ball: the defender kills its pace and its direction and
//! everybody scrambles. Modelling it as possession would be the same
//! error `try_intercept` makes, and it would move the interception rate,
//! which is load-bearing (see [`InterceptionDuel`]). So this channel adds
//! **turnovers of the kind that break up a move**, not turnovers of the
//! kind that hand the ball over — the one the report is about.
//!
//! It is also **priced by danger**, not applied flat across the pitch. A
//! defender does not throw a leg at a square ball on the halfway line;
//! he does it in front of his own goal, and the closer the ball is to it
//! the more of himself he will put in the way. [`Self::danger`] is that
//! ramp, and it is what keeps a model of last-ditch defending out of the
//! calibration of ordinary midfield passing.
//!
//! # Measured
//!
//! Matched 400-vs-400 at level 14 in the same binary, against
//! `OF_BOX_DEFENCE_OFF` (which also reverts the keeper's calls and the
//! box restraint — the three landed together and are judged together):
//!
//! | | off | on | real |
//! |---|---|---|---|
//! | goals/match | 3.15 | **3.03** | ~2.5 |
//! | shots a team | 15.0 | **14.5** | ~13 |
//! | pass accuracy | 88.5% | **87.6%** | ~85% |
//! | penalties/match | 0.110 | **0.157** | 0.25-0.30 |
//! | DEF blocks | 0.40 | **1.49** | ~1.4 (shots+passes) |
//! | MID blocks | 0.03 | **0.98** | ~1.3 |
//! | fouls a team | 12.2 | 12.3 | ~12 |
//! | yellows/match | 3.45 | 3.43 | 3.5-4.5 |
//!
//! 18.2 passes a match are blocked, **5.7 of them inside the penalty
//! area** — the part a viewer can see. Every calibration axis moved
//! toward its reference or did not move; the ones that did not move are
//! the discipline numbers, which is the pairing this had to preserve.

use crate::PlayerFieldPositionGroup;
use crate::r#match::ball::events::BallEvent;
use crate::r#match::engine::ball::ball::contest::interception::InterceptionDuel;
use crate::r#match::engine::ball::ball::{Ball, CONTROL_DISTANCE};
use crate::r#match::events::EventCollection;
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::player::strategies::players::ops::effective_skill::{
    ActionContext as EffSkillCtx, effective_skill,
};
use crate::r#match::player::strategies::players::ops::skill_composites as sc;
use crate::r#match::{MatchContext, MatchPlayer, PassOriginRestart, PlayerSide};
use nalgebra::Vector3;

impl Ball {
    /// How near the flight line a defender has to be to get something to
    /// the ball. 14u = 1.75 m — a committed lunge or an outstretched leg,
    /// the same order as the shot block's own 2 m and
    /// deliberately wider than `try_intercept`'s 69 cm standing radius,
    /// because this is a man throwing himself at it rather than one
    /// having it arrive at his feet.
    const PASS_BLOCK_CORRIDOR: f32 = 14.0;
    /// How far up the flight line the candidate search runs (~5 m). Much
    /// shorter than the shot block's 90u: a shot is struck from range and
    /// a defender has the whole flight to get across, while a pass in a
    /// crowded box is blocked by whoever is already beside its line.
    const PASS_BLOCK_LOOKAHEAD: f32 = 40.0;
    /// Reach at the moment of contact — the same 2 m the shot block's own
    /// `BLOCK_REACH` calls "a committed lunge or a slide rather than a
    /// standing body".
    const PASS_BLOCK_REACH: f32 = 16.0;
    /// A ball higher than this is over the leg that would block it. 1.6 m
    /// — chest height. Deliberately lower than the shot block's 2.2 m
    /// raised-arm figure: nobody blocks a PASS with his head, that is the
    /// aerial contest's business and it has already priced the box.
    const MAX_PASS_BLOCK_HEIGHT: f32 = 1.6;
    /// Below this the ball is not really travelling and the loose-ball
    /// machinery owns it. Matches `try_intercept`'s own floor.
    const MIN_PASS_SPEED: f32 = 0.25;

    /// **Will he put a leg in it?** — the depth ramp.
    ///
    /// Zero from the halfway line back, rising to one at his own goal
    /// line, squared so the weight is concentrated in the last twenty
    /// metres. This is the whole of the scoping argument: blocking passes
    /// is last-ditch defending, it belongs in front of your own goal, and
    /// a flat rate across the pitch would re-tune ordinary build-up play
    /// for every side in the game.
    fn danger(ball_x: f32, defending_side: PlayerSide, field_width: f32) -> f32 {
        let own_goal_x = match defending_side {
            PlayerSide::Left => 0.0,
            PlayerSide::Right => field_width,
        };
        let depth = (ball_x - own_goal_x).abs();
        let half = field_width * 0.5;
        if depth >= half {
            return 0.0;
        }
        let t = 1.0 - depth / half;
        t * t
    }

    /// One roll per pass at getting a body in its way. Runs on unowned
    /// balls in flight that are NOT shots — see the module note for why
    /// the two are kept apart.
    pub fn try_block_pass(
        &mut self,
        context: &MatchContext,
        players: &[MatchPlayer],
        events: &mut EventCollection,
    ) {
        // A block already won, waiting for the ball to reach the man who
        // won it — the same deferral the shot block uses, and for the
        // same reason: the rate is decided where the read happens, the
        // contact happens where the body is.
        if let Some((blocker_id, outcome_roll)) = self.pass_blocked_by {
            let reached = players
                .iter()
                .find(|p| p.id == blocker_id)
                .map(|p| {
                    (p.position.x - self.position.x).hypot(p.position.y - self.position.y)
                        <= Self::PASS_BLOCK_REACH
                })
                .unwrap_or(false);
            // A pass that has been claimed, has died, or has turned into
            // a shot in the meantime is no longer the ball he committed
            // to. Drop the commitment rather than deflecting something
            // else.
            if self.current_owner.is_some()
                || self.flags.in_flight_state == 0
                || self.cached_shot_target.is_some()
            {
                self.pass_blocked_by = None;
                return;
            }
            if reached {
                self.pass_blocked_by = None;
                self.resolve_pass_block(blocker_id, outcome_roll, context, players, events);
            }
            return;
        }

        if self.pass_block_rolled || MatchContext::box_defence_off() {
            return;
        }
        if self.current_owner.is_some()
            || self.held_in_hands
            || self.flags.in_flight_state == 0
            || self.cached_shot_target.is_some()
        {
            return;
        }
        // A delivery whose aerial contest is already decided has priced
        // every defender in the box once. Same double-jeopardy carve-out
        // `try_intercept` makes.
        if self.aerial_delivery.is_some() {
            return;
        }
        if self.position.z > Self::MAX_PASS_BLOCK_HEIGHT {
            return;
        }
        let speed = (self.velocity.x * self.velocity.x + self.velocity.y * self.velocity.y).sqrt();
        if speed < Self::MIN_PASS_SPEED {
            return;
        }
        let Some(passer) = self
            .previous_owner
            .and_then(|id| players.iter().find(|p| p.id == id))
        else {
            return;
        };
        // Which way is the defending side defending? Taken from the
        // passer, so a defender playing out of his own box is not
        // "blocking" his own team's pass.
        let Some(attacking_side) = passer.side else {
            return;
        };
        let defending_side = attacking_side.opposite();
        let field_width = context.field_size.width as f32;
        let danger = Self::danger(self.position.x, defending_side, field_width);
        if danger <= 0.0 {
            return;
        }

        let dir_x = self.velocity.x / speed;
        let dir_y = self.velocity.y / speed;
        let delivery = sc::passing_execution(passer, sc::minute_from_ticks(self.current_tick_cached));
        let minute = sc::minute_from_ticks(self.current_tick_cached);

        let mut best_blocker: Option<u32> = None;
        let mut best_chance = 0.0f32;

        for player in players {
            if player.team_id == passer.team_id {
                continue;
            }
            if player.tactical_position.current_position.position_group()
                == PlayerFieldPositionGroup::Goalkeeper
            {
                continue; // the keeper's own volume is `try_keeper_body_block`
            }
            // Not the man the ball is being played to — he is receiving
            // it, not blocking it. (Defenders can never be the target of
            // an opponent's pass, so this only guards a mis-set target.)
            if Some(player.id) == self.pass_target_player_id {
                continue;
            }
            let dx = player.position.x - self.position.x;
            let dy = player.position.y - self.position.y;
            let along = dx * dir_x + dy * dir_y;
            // Ahead of the ball, within the window. A man the ball has
            // already gone past cannot block it.
            if along < 0.5 || along > Self::PASS_BLOCK_LOOKAHEAD {
                continue;
            }
            let perp = (dx - along * dir_x).hypot(dy - along * dir_y);
            if perp > Self::PASS_BLOCK_CORRIDOR {
                continue;
            }

            // The read, as a CONTEST — see [`InterceptionDuel`]. An
            // absolute skill term here would walk straight up the
            // pyramid; the duel resolves to parity at every level, so
            // the population rate is a property of these constants and
            // not of the division.
            let tech = EffSkillCtx::technical(minute);
            let mental = EffSkillCtx::mental(minute);
            let expl = EffSkillCtx::explosive(minute);
            // The same blend the shot block uses, because it is the same
            // action: read the cue, be brave enough to be in the way, be
            // quick enough to get there, and know how to use the leg.
            let read = (effective_skill(player, player.skills.mental.bravery, mental) * 0.24
                + effective_skill(player, player.skills.mental.anticipation, mental) * 0.26
                + effective_skill(player, player.skills.mental.positioning, mental) * 0.22
                + effective_skill(player, player.skills.physical.agility, expl) * 0.14
                + effective_skill(player, player.skills.technical.tackling, tech) * 0.14)
                / 20.0;
            let skill = InterceptionDuel::advantage(read, delivery);

            // Right on the line is a block; the edge of the corridor is a
            // toe at full stretch.
            let perp_factor = 1.0 - (perp / Self::PASS_BLOCK_CORRIDOR) * 0.65;
            // Close to the ball is a block; the far end of the window is
            // a man who has to get there first.
            let line_factor = 1.0 - (along / Self::PASS_BLOCK_LOOKAHEAD) * 0.45;
            // A driven ball is harder to get a foot to than a rolled one.
            let speed_penalty = 1.0 / (1.0 + speed * 0.35);

            let chance =
                skill * perp_factor * line_factor * speed_penalty * danger * Self::BLOCK_GAIN;
            if chance > best_chance {
                best_chance = chance;
                best_blocker = Some(player.id);
            }
        }

        let Some(blocker_id) = best_blocker else {
            return;
        };
        // One pass, one roll. Latched the moment a candidate exists, so
        // the rate is a property of the defending rather than of how long
        // the flight happened to be — the correction `try_intercept`
        // already carries, made here from the start.
        self.pass_block_rolled = true;
        let chance = best_chance.clamp(0.0, 0.65);
        let fired = context.rng.unit_f32() < chance;
        #[cfg(feature = "match-logs")]
        crate::mid_run_diag::BoxPassDiag::note_block_roll(chance, fired);
        if !fired {
            return;
        }
        let outcome_roll = context.rng.unit_f32();
        let gap_now = players
            .iter()
            .find(|p| p.id == blocker_id)
            .map(|p| (p.position.x - self.position.x).hypot(p.position.y - self.position.y))
            .unwrap_or(f32::MAX);
        if gap_now > Self::PASS_BLOCK_REACH {
            self.pass_blocked_by = Some((blocker_id, outcome_roll));
            return;
        }
        self.resolve_pass_block(blocker_id, outcome_roll, context, players, events);
    }

    /// Overall scale on the block chance.
    ///
    /// **Measured, not chosen.** At 1.05 the corridor, window and danger
    /// ramp above produced 46.9 blocked passes a match — 23.5 a team
    /// against a real ~9-10 — and took DEF blocks from 0.39 to 3.18 and
    /// MID blocks from 0.02 to 2.46. 0.40 is that rate scaled onto the
    /// real one. The geometry is deliberately left alone: the shape of
    /// the model (who is a candidate, and how the chance falls off across
    /// the corridor) measured right, and only the level was wrong.
    ///
    /// At 0.40 it produces **18.2 blocked passes a match — 9.1 a team,
    /// against a real 9-11** — at a mean per-pass chance of 0.019 over
    /// the ~960 passes a match that reach the roll at all.
    const BLOCK_GAIN: f32 = 0.40;

    /// Turn a won pass-block into a deflection, at the blocker.
    fn resolve_pass_block(
        &mut self,
        blocker_id: u32,
        outcome_roll: f32,
        context: &MatchContext,
        players: &[MatchPlayer],
        events: &mut EventCollection,
    ) {
        let speed = (self.velocity.x * self.velocity.x + self.velocity.y * self.velocity.y).sqrt();
        if speed < Self::MIN_PASS_SPEED {
            return;
        }
        let Some(blocker) = players.iter().find(|p| p.id == blocker_id) else {
            return;
        };
        let blocker_team = blocker.team_id;
        let blocker_gap =
            (blocker.position.x - self.position.x).hypot(blocker.position.y - self.position.y);
        let tick = self.current_tick_cached;

        // ── Did he keep it? ──────────────────────────────────────────
        //
        // Rarely, and only if the ball was genuinely at his feet. A block
        // from a stretch always leaves a loose ball; handing possession
        // to a man 2 m away would be the same error `try_intercept`
        // makes, and `move_to` would drop the ball again on the next tick
        // as an unreachable owner.
        let composure = (blocker.skills.mental.composure / 20.0).clamp(0.0, 1.0);
        let first_touch = (blocker.skills.technical.first_touch / 20.0).clamp(0.0, 1.0);
        let controlled = blocker_gap <= CONTROL_DISTANCE
            && outcome_roll < (0.08 + composure * 0.06 + first_touch * 0.06).clamp(0.08, 0.24);
        #[cfg(feature = "match-logs")]
        if controlled {
            crate::mid_run_diag::BoxPassDiag::note_block_controlled();
        }

        // He got something on it. That is a touch, and the touch is his —
        // which is also what makes a deflection over his own byline a
        // corner rather than a goal kick.
        #[cfg(feature = "match-logs")]
        if context.penalty_area(true).contains(&self.position)
            || context.penalty_area(false).contains(&self.position)
        {
            crate::mid_run_diag::BoxPassDiag::note_block_in_box();
        }
        self.record_touch(blocker_id, blocker_team, tick, controlled);
        self.pass_target_player_id = None;
        self.clear_pending_pass_metadata();
        self.offside_snapshot = None;
        self.pass_origin_restart = PassOriginRestart::OpenPlay;
        events.add_ball_event(BallEvent::Blocked(blocker_id, self.position));

        if controlled {
            self.current_owner = Some(blocker_id);
            self.flags.in_flight_state = 0;
            self.claim_cooldown = 15;
            self.velocity = Vector3::zeros();
            events.add_player_event(PlayerEvent::ClaimBall(blocker_id));
            return;
        }

        // ── Otherwise it squirts ─────────────────────────────────────
        //
        // A blocked pass loses most of its pace and comes off at an
        // angle. WHICH angle is not arbitrary: a defender getting a leg
        // to a ball in his own box is trying to send it away from his
        // goal, and the ones that go the other way are the deflections
        // that produce chaos. So the direction is the reversed flight
        // blended toward "away from my own goal", with a wide spread on
        // top — the same shape as the shot block's deflection, aimed by
        // the one thing a defender in that position is actually trying to
        // do.
        let field_width = context.field_size.width as f32;
        let own_goal = Vector3::new(
            if blocker.side == Some(PlayerSide::Left) {
                0.0
            } else {
                field_width
            },
            context.field_size.height as f32 / 2.0,
            0.0,
        );
        let away = (self.position - own_goal)
            .try_normalize(1.0e-3)
            .unwrap_or_else(|| Vector3::new(-self.velocity.x, -self.velocity.y, 0.0).normalize());
        let back = Vector3::new(-self.velocity.x / speed, -self.velocity.y / speed, 0.0);
        let aim = (back * 0.45 + away * 0.55)
            .try_normalize(1.0e-3)
            .unwrap_or(back);
        // ±60° of spread — a block is not an aimed clearance.
        let angle = (context.rng.unit_f32() - 0.5) * 2.1;
        let (s, c) = angle.sin_cos();
        let dir = Vector3::new(aim.x * c - aim.y * s, aim.x * s + aim.y * c, 0.0);
        // Pace kept: 25-60%. A block off the shin dies; one off the toe
        // of a stretching leg runs.
        let keep = 0.25 + context.rng.unit_f32() * 0.35;
        self.velocity = dir * speed * keep;
        // It stays a live loose ball at whatever height it was — nothing
        // here writes the vertical axis, which is `ballistics`' business.
        self.claim_cooldown = self.claim_cooldown.max(4);
    }
}

#[cfg(test)]
mod pass_block_tests {
    use super::*;

    /// The danger ramp is the scoping argument, so it has to actually
    /// scope: nothing from the halfway line back, everything at the goal.
    #[test]
    fn the_danger_ramp_is_zero_upfield_and_one_at_the_goal() {
        let w = 840.0;
        assert_eq!(Ball::danger(w * 0.5, PlayerSide::Left, w), 0.0);
        assert_eq!(Ball::danger(w * 0.9, PlayerSide::Left, w), 0.0);
        assert!((Ball::danger(0.0, PlayerSide::Left, w) - 1.0).abs() < 1e-6);
        assert!((Ball::danger(w, PlayerSide::Right, w) - 1.0).abs() < 1e-6);
        // …and it is monotone toward the goal being defended.
        let near = Ball::danger(w * 0.05, PlayerSide::Left, w);
        let far = Ball::danger(w * 0.35, PlayerSide::Left, w);
        assert!(near > far, "{near} vs {far}");
    }

    /// Mirror image for the other side, because a rule that reads
    /// differently on the two halves of the pitch is a bug that only
    /// shows up in one direction of play.
    #[test]
    fn the_ramp_is_symmetric_between_the_sides() {
        let w = 840.0;
        for t in [0.0_f32, 0.1, 0.2, 0.35, 0.49] {
            let left = Ball::danger(w * t, PlayerSide::Left, w);
            let right = Ball::danger(w * (1.0 - t), PlayerSide::Right, w);
            assert!((left - right).abs() < 1e-6, "{left} vs {right} at {t}");
        }
    }
}
