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
//! the more of himself he will put in the way. [`PassBlock::danger`] is that
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
#[cfg(feature = "match-logs")]
use crate::r#match::engine::ball::ball::diagnostics::block_diag::{BlockDiag, PassBlockCensus};
use crate::r#match::engine::ball::ball::{
    AerialReach, Ball, BlockContact, CONTROL_DISTANCE, FlightProtection, GRAVITY_PER_TICK,
};
use crate::r#match::events::EventCollection;
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::player::strategies::players::ops::effective_skill::{
    ActionContext as EffSkillCtx, effective_skill,
};
use crate::r#match::player::strategies::players::ops::skill_composites as sc;
use crate::r#match::{MatchContext, MatchPlayer, MatchPlayerLite, PassOriginRestart, PlayerSide};
use nalgebra::Vector3;

/// A ball about to be struck, as its striker prices it.
pub(crate) struct StrikeLine {
    pub from: Vector3<f32>,
    /// Flat and unit length.
    pub direction: Vector3<f32>,
    pub pace: f32,
    /// Vertical launch speed in m/tick — zero for a ball along the deck.
    pub lift: f32,
    /// The striker's execution for this strike ([`PassBlock::technique`]).
    pub delivery: f32,
    pub defending_side: PlayerSide,
}

/// Where one man stands against a ball on its way to him — everything
/// the block rule reads except the two players' skills, measured by the
/// contest from the live ball and by the striker from the ball he is
/// about to strike.
#[derive(Clone, Copy, Debug)]
pub(crate) struct BlockGeometry {
    /// Distance off the ball's line, units.
    pub perp: f32,
    /// Distance up the ball's line to him, units.
    pub along: f32,
    /// Ball speed across the grass, u/tick.
    pub speed: f32,
    /// Ticks from the strike until the ball gets to him — his whole time
    /// to see it coming, however late in the flight he is rolled.
    pub ticks_to_him: f32,
    /// Ball height in metres when it gets to him.
    pub height: f32,
    /// [`PassBlock::danger`] where the ball is when he is rolled.
    pub danger: f32,
}

impl BlockGeometry {
    /// Measured by the contest, from the live ball moving at `speed`
    /// across the grass, for a man at `blocker`.
    pub(crate) fn in_flight(ball: &Ball, blocker: Vector3<f32>, speed: f32, danger: f32) -> Self {
        let direction = ball.velocity.xy() / speed;
        let to_blocker = (blocker - ball.position).xy();
        let along = to_blocker.dot(&direction);
        let ticks = along / speed;
        let since_strike = ball
            .current_tick_cached
            .saturating_sub(ball.last_release_tick) as f32;
        BlockGeometry {
            perp: (to_blocker - direction * along).norm(),
            along,
            speed,
            ticks_to_him: since_strike + ticks,
            height: Self::height_after(ball.position.z, ball.velocity.z, ticks),
            danger,
        }
    }

    /// Priced by the striker before the strike, for a man at `blocker`,
    /// at the point the contest will roll him: the first tick he is
    /// inside the window — [`PassBlock::LOOKAHEAD`] short of him, or at
    /// the strike when he is nearer than that — with the danger read
    /// where the ball is then. `None` for a man behind the strike.
    pub(crate) fn at_strike(
        line: &StrikeLine,
        blocker: Vector3<f32>,
        field_width: f32,
    ) -> Option<Self> {
        let to_blocker = (blocker - line.from).xy();
        let along = to_blocker.dot(&line.direction.xy());
        if along <= 0.0 {
            return None;
        }
        let rolled_at = along.min(PassBlock::LOOKAHEAD);
        let ticks = along / line.pace.max(1.0e-3);
        Some(BlockGeometry {
            perp: (to_blocker - line.direction.xy() * along).norm(),
            along: rolled_at,
            speed: line.pace,
            ticks_to_him: ticks,
            height: Self::height_after(0.0, line.lift, ticks),
            danger: PassBlock::danger(
                line.from.x + line.direction.x * (along - rolled_at),
                line.defending_side,
                field_width,
            ),
        })
    }

    /// Ball height `ticks` from now, drag left out over a few metres.
    fn height_after(height: f32, lift: f32, ticks: f32) -> f32 {
        (height + lift * ticks - 0.5 * GRAVITY_PER_TICK * ticks * ticks).max(0.0)
    }

    /// How little time he has, from 1 (the ball is on him before he can
    /// move) to 0 (he has a full reaction to read it). The one signal that
    /// decides both what kind of block it is and where the ball goes.
    pub(crate) fn reaction(&self) -> f32 {
        if MatchContext::charge_down_off() {
            return 0.0;
        }
        (1.0 - self.ticks_to_him / PassBlock::REACTION_TICKS).clamp(0.0, 1.0)
    }

    /// His body is on the line and the ball is not over it.
    fn charge_down_open(&self) -> bool {
        self.along >= 0.5 && self.perp <= PassBlock::BODY && self.height <= AerialReach::STANDING
    }

    /// He is near his own goal, inside the lunge window and below the leg.
    fn lunge_open(&self) -> bool {
        self.danger > 0.0
            && (0.5..=PassBlock::LOOKAHEAD).contains(&self.along)
            && self.perp <= PassBlock::CORRIDOR
            && self.height <= PassBlock::MAX_HEIGHT
    }

    /// Can either kind of block reach this ball at all? Callers ask this
    /// before paying for [`PassBlock::read`].
    pub(crate) fn is_live(&self) -> bool {
        (self.reaction() > 0.0 && self.charge_down_open()) || self.lunge_open()
    }
}

/// A block won at the roll, waiting for the ball to reach the man who
/// won it — see [`Ball::try_block_pass`].
#[derive(Clone, Copy, Debug)]
pub struct PassBlockCommit {
    pub blocker_id: u32,
    pub outcome_roll: f32,
    /// [`BlockGeometry::reaction`] at the roll.
    pub reaction: f32,
}

/// **One man's chance of getting something on a pass** — the rule
/// [`Ball::try_block_pass`] rolls and the striker prices
/// (`PassEvaluator::lane_risk`, `CrossModel::pick_rated`), from
/// primitives either side can supply, so the two can never disagree about
/// what a body in the lane is worth.
///
/// Two kinds of block, blended by the man's time to react
/// ([`BlockGeometry::reaction`]):
///
/// * the **charge-down** — a ball struck into a body a stride away. No
///   read, no choice: his body is on its line under his reach or it is
///   not, anywhere on the pitch;
/// * the **lunge** — a man who has seen it coming throwing a leg at it,
///   which is last-ditch defending and only happens toward his own goal.
pub(crate) struct PassBlock;

impl PassBlock {
    /// How near the flight line a defender has to be to get something to
    /// the ball. 14u = 1.75 m — a committed lunge or an outstretched leg,
    /// the same order as the shot block's own 2 m and
    /// deliberately wider than `try_intercept`'s 69 cm standing radius,
    /// because this is a man throwing himself at it rather than one
    /// having it arrive at his feet.
    pub(crate) const CORRIDOR: f32 = 14.0;
    /// How far up the flight line the candidate search runs (~5 m). Much
    /// shorter than the shot block's 90u: a shot is struck from range and
    /// a defender has the whole flight to get across, while a pass in a
    /// crowded box is blocked by whoever is already beside its line.
    pub(crate) const LOOKAHEAD: f32 = 40.0;
    /// A ball higher than this is over the leg that would block it. 1.6 m
    /// — chest height. Deliberately lower than the shot block's 2.2 m
    /// raised-arm figure: nobody lunges at a pass with his head, that is
    /// the aerial contest's business and it has already priced the box.
    pub(crate) const MAX_HEIGHT: f32 = 1.6;
    /// How far off the line a body charges a ball down: half a torso and
    /// a leg stuck out, 1 m.
    const BODY: f32 = 8.0;
    /// A man's reaction, 0.25 s. A ball that reaches him sooner was on him
    /// before he could do anything about it.
    const REACTION_TICKS: f32 = 25.0;
    /// Scale on the charge-down, before the duel. `OF_CHARGE_GAIN`
    /// overrides for titration.
    ///
    /// **Measured, not chosen** — `stats 300 14 14` per arm, 2026-10-10,
    /// blocked passes / pass accuracy (HEAD 17.6 / 85.7%):
    /// 0.60 → 55.5 / 83.6%, 0.30 → 38.3 / 84.6%, 0.15 → 24.4 / 85.3%,
    /// 0.08 → 17.7 / 85.7%. The block price is calibrated at every gain:
    /// the surplus comes from pass pickers that call any lane under
    /// `PlayerOps::clear_lane` clear and do not prefer the cleaner of
    /// two clear lanes. Tightening that threshold instead sent passes
    /// over the side's own byline when nothing was clear (4.8 "ordinary
    /// play" corners a match at 0.05), so the gain carries the level
    /// until the pickers rank lanes by risk.
    fn charge_gain() -> f32 {
        use std::sync::OnceLock;
        static V: OnceLock<f32> = OnceLock::new();
        *V.get_or_init(|| {
            std::env::var("OF_CHARGE_GAIN")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0.11)
        })
    }
    /// The most any one man is ever given.
    const CEILING: f32 = 0.65;

    /// Overall scale on the lunge.
    ///
    /// **Measured, not chosen.** At 1.05 the corridor, window and danger
    /// ramp above produced 46.9 blocked passes a match — 23.5 a team
    /// against a real ~9-10 — and took DEF blocks from 0.39 to 3.18 and
    /// MID blocks from 0.02 to 2.46. 0.40 is that rate scaled onto the
    /// real one. The geometry is deliberately left alone: the shape of
    /// the model (who is a candidate, and how the chance falls off across
    /// the corridor) measured right, and only the level was wrong.
    ///
    /// At 0.40 it produced **18.2 blocked passes a match — 9.1 a team,
    /// against a real 9-11** — at a mean per-pass chance of 0.019 over
    /// the ~960 passes a match that reach the roll at all.
    ///
    /// ⚠ **Re-titrated 2026-09-06 when the CONTACT moved onto the body.**
    /// The deferral used to fire on the first tick the ball came within
    /// two metres of the man who had won the roll, which is the far edge
    /// of his reach; it now waits for the ball to be level with him
    /// ([`Ball::block_contact_with`]). That is where a leg actually goes
    /// in, and it is what the picture needs — but it also means a pass
    /// received before it ever reaches him is no longer blocked, and the
    /// channel lost 17% of its volume: 16.5 blocked passes a match against
    /// the 16.4 the same binary measured on the old geometry (3 x 200
    /// fixtures at L14 per arm, 3291 vs 2722 contacts). **The rate is the
    /// calibrated thing and the geometry was the wrong thing**, so the
    /// gain carries the correction: 0.40 x 16.5/13.6 = 0.485.
    const GAIN: f32 = 0.485;

    /// **Will he put a leg in it?** — the depth ramp.
    ///
    /// Zero from the halfway line back, rising to one at his own goal
    /// line, squared so the weight is concentrated in the last twenty
    /// metres. This is the whole of the scoping argument for the lunge:
    /// throwing a leg at a pass is last-ditch defending, it belongs in
    /// front of your own goal, and a flat rate across the pitch would
    /// re-tune ordinary build-up play for every side in the game. The
    /// charge-down does not read it — a ball struck into a body hits it
    /// wherever the body is.
    pub(crate) fn danger(ball_x: f32, defending_side: PlayerSide, field_width: f32) -> f32 {
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

    /// The blocker's side of the duel: read the cue, be brave enough to
    /// be in the way, be quick enough to get there, and know how to use
    /// the leg — the same blend the shot block uses, because it is the
    /// same action.
    pub(crate) fn read(player: &MatchPlayer, minute: u32) -> f32 {
        let tech = EffSkillCtx::technical(minute);
        let mental = EffSkillCtx::mental(minute);
        let expl = EffSkillCtx::explosive(minute);
        (effective_skill(player, player.skills.mental.bravery, mental) * 0.24
            + effective_skill(player, player.skills.mental.anticipation, mental) * 0.26
            + effective_skill(player, player.skills.mental.positioning, mental) * 0.22
            + effective_skill(player, player.skills.physical.agility, expl) * 0.14
            + effective_skill(player, player.skills.technical.tackling, tech) * 0.14)
            / 20.0
    }

    /// The striker's side of the duel: getting the ball past the man —
    /// his crossing for a cross, his passing otherwise, in the shape of
    /// [`sc::passing_execution`] so the two strikes are on one scale.
    pub(crate) fn technique(striker: &MatchPlayer, minute: u32, cross: bool) -> f32 {
        if !cross {
            return sc::passing_execution(striker, minute);
        }
        let tech = EffSkillCtx::technical(minute);
        let mental = EffSkillCtx::mental(minute);
        let s = &striker.skills;
        ((effective_skill(striker, s.technical.crossing, tech) * 0.38
            + effective_skill(striker, s.technical.technique, tech) * 0.20
            + effective_skill(striker, s.mental.vision, mental) * 0.16
            + effective_skill(striker, s.mental.decisions, mental) * 0.10
            + effective_skill(striker, s.mental.composure, mental) * 0.08
            + effective_skill(striker, s.mental.concentration, mental) * 0.08)
            / 20.0)
            .clamp(0.0, 1.0)
    }

    /// The chance the contest will roll for the man `blocker` on a ball
    /// about to be struck along `line`, priced from where he stands now.
    /// `record` is the same man's full player record, read for his skills.
    ///
    /// The contest rolls a man on the first tick he is inside the window
    /// — [`Self::LOOKAHEAD`] short of him, or at the strike when he is
    /// nearer than that — and reads the danger where the ball is then, so
    /// the price is taken at the same point.
    pub(crate) fn priced(
        line: &StrikeLine,
        blocker: &MatchPlayerLite,
        record: &MatchPlayer,
        minute: u32,
        field_width: f32,
    ) -> f32 {
        if MatchContext::box_defence_off() || blocker.tactical_positions.is_goalkeeper() {
            return 0.0;
        }
        match BlockGeometry::at_strike(line, blocker.position, field_width) {
            Some(geometry) if geometry.is_live() => {
                Self::chance(&geometry, Self::read(record, minute), line.delivery)
            }
            _ => 0.0,
        }
    }

    /// The chance for one man. Skill enters as a duel
    /// ([`InterceptionDuel`]) for both kinds: an absolute skill term would
    /// walk straight up the pyramid, while the duel resolves to parity at
    /// every level, so the population rate is a property of these
    /// constants and not of the division.
    pub(crate) fn chance(geometry: &BlockGeometry, read: f32, delivery: f32) -> f32 {
        let skill = InterceptionDuel::advantage(read, delivery);
        let reaction = geometry.reaction();
        let charge_down = if geometry.charge_down_open() {
            // Square on the line is the whole body; a metre off it is a
            // toe.
            skill * (1.0 - geometry.perp / Self::BODY) * Self::charge_gain()
        } else {
            0.0
        };
        let lunge = if geometry.lunge_open() {
            // Right on the line is a block; the edge of the corridor is a
            // toe at full stretch.
            let perp_factor = 1.0 - (geometry.perp / Self::CORRIDOR) * 0.65;
            // Close to the ball is a block; the far end of the window is a
            // man who has to get there first.
            let line_factor = 1.0 - (geometry.along / Self::LOOKAHEAD) * 0.45;
            // A driven ball is harder to get a foot to than a rolled one.
            let speed_penalty = 1.0 / (1.0 + geometry.speed * 0.35);
            skill * perp_factor * line_factor * speed_penalty * geometry.danger * Self::GAIN
        } else {
            0.0
        };
        (reaction * charge_down + (1.0 - reaction) * lunge).clamp(0.0, Self::CEILING)
    }

    /// The highest ball the contact accepts once the roll is won: a leg
    /// for a lunge, a standing body for a charge-down.
    pub(crate) fn contact_ceiling(reaction: f32) -> f32 {
        Self::MAX_HEIGHT + (AerialReach::STANDING - Self::MAX_HEIGHT) * reaction
    }
}

impl Ball {
    /// Reach at the moment of contact — the same 2 m the shot block's own
    /// `BLOCK_REACH` calls "a committed lunge or a slide rather than a
    /// standing body".
    const PASS_BLOCK_REACH: f32 = 16.0;
    /// Below this the ball is not really travelling and the loose-ball
    /// machinery owns it. Matches `try_intercept`'s own floor.
    const MIN_PASS_SPEED: f32 = 0.25;
    /// Outside this depth from his own goal there is always somewhere
    /// else to put it. 150u ≈ 18.75 m — the same window
    /// `DeliveryResolver::heads_it_behind` uses, because it is the same
    /// question about the same defender.
    const BEHIND_DEPTH: f32 = 150.0;
    /// …and the share of blocks that go behind when he is right on his
    /// own goal line. Deliberately below the headed clearance's 0.50: a
    /// header is a deliberate act with a chosen direction, while a block
    /// is a ball coming off a leg, so it is less often aimed anywhere at
    /// all — including behind.
    const BEHIND_AT_LINE: f32 = 0.38;

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
        //
        // ⚠ **And "where the body is" means all three axes.** This tested
        // the distance across the grass alone, with the height gate left
        // behind at the roll — so a pass rolled for on the deck was
        // deflected off a man's head five metres later. See
        // [`Ball::block_contact_with`].
        if let Some(commit) = self.pass_blocked_by {
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
            match self.block_contact_with(
                commit.blocker_id,
                players,
                Self::PASS_BLOCK_REACH,
                PassBlock::contact_ceiling(commit.reaction),
            ) {
                BlockContact::Coming => {}
                // Over him or past him — one pass, one roll, and he has
                // had it.
                BlockContact::Missed => self.pass_blocked_by = None,
                BlockContact::AtTheBody => {
                    self.pass_blocked_by = None;
                    self.resolve_pass_block(commit, context, players, events, true);
                }
            }
            return;
        }

        if MatchContext::box_defence_off() {
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
        let danger = PassBlock::danger(self.position.x, defending_side, field_width);

        let minute = sc::minute_from_ticks(self.current_tick_cached);
        let delivery = PassBlock::technique(passer, minute, self.pending_pass_was_cross);

        let mut best_blocker: Option<(u32, u64, f32)> = None;
        let mut best_chance = 0.0f32;

        for (slot, player) in players.iter().enumerate() {
            if player.team_id == passer.team_id {
                continue;
            }
            // One flight, one roll per man — see
            // [`Ball::pass_block_rolled`].
            let bit = 1u64 << slot;
            if self.pass_block_rolled & bit != 0 {
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
            let geometry = BlockGeometry::in_flight(self, player.position, speed, danger);
            if !geometry.is_live() {
                continue;
            }
            let chance = PassBlock::chance(&geometry, PassBlock::read(player, minute), delivery);
            if chance > best_chance {
                best_chance = chance;
                best_blocker = Some((player.id, bit, geometry.reaction()));
            }
        }

        let Some((blocker_id, bit, reaction)) = best_blocker else {
            return;
        };
        // Latched per MAN, so the rate stays a property of the defending
        // rather than of how long the flight happened to be, while the
        // defender downstream still gets the encounter that is his.
        self.pass_block_rolled |= bit;
        let chance = best_chance;
        let fired = context.rng.unit_f32() < chance;
        #[cfg(feature = "match-logs")]
        crate::mid_run_diag::BoxPassDiag::note_block_roll(chance, fired);
        if !fired {
            return;
        }
        let commit = PassBlockCommit {
            blocker_id,
            outcome_roll: context.rng.unit_f32(),
            reaction,
        };
        match self.block_contact_with(
            blocker_id,
            players,
            Self::PASS_BLOCK_REACH,
            PassBlock::contact_ceiling(reaction),
        ) {
            BlockContact::AtTheBody => {
                self.resolve_pass_block(commit, context, players, events, false)
            }
            _ => self.pass_blocked_by = Some(commit),
        }
    }

    /// Turn a won pass-block into a deflection, at the blocker.
    fn resolve_pass_block(
        &mut self,
        commit: PassBlockCommit,
        context: &MatchContext,
        players: &[MatchPlayer],
        events: &mut EventCollection,
        deferred: bool,
    ) {
        let PassBlockCommit {
            blocker_id,
            outcome_roll,
            reaction,
        } = commit;
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
        #[cfg(feature = "match-logs")]
        BlockDiag::note_contact(
            1,
            self.position.z,
            blocker_gap,
            PassBlock::contact_ceiling(reaction),
            deferred,
        );
        #[cfg(not(feature = "match-logs"))]
        let _ = deferred;
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
        #[cfg(feature = "match-logs")]
        if let Some(origin) = self.strike_origin {
            origin.note_block(reaction >= 0.5);
        }
        #[cfg(feature = "match-logs")]
        let census = PassBlockCensus::open(
            self.pending_pass_was_cross,
            self.pending_pass_origin
                .map(|origin| (blocker.position - origin).xy().norm()),
            reaction >= 0.5,
        );
        self.record_touch(blocker_id, blocker_team, tick, controlled);
        #[cfg(feature = "match-logs")]
        if controlled {
            census.close(PassBlockCensus::KEPT);
        } else {
            self.pass_block_census = Some(census);
        }
        self.pass_target_player_id = None;
        self.clear_pending_pass_metadata();
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

        // ── At point blank it glances ────────────────────────────────
        //
        // A man the ball reached before he could react aimed nothing: it
        // came off whatever it hit and kept much of its own line and pace.
        // That is what takes a cross charged down near the byline behind —
        // through the last-touch rule, with no branch of its own.
        if context.rng.unit_f32() < reaction {
            let own = Vector3::new(self.velocity.x / speed, self.velocity.y / speed, 0.0);
            let dir = Self::turned(own, (context.rng.unit_f32() - 0.5) * Self::GLANCE_SPREAD);
            let keep = 0.40 + context.rng.unit_f32() * 0.35;
            self.velocity = dir * speed * keep;
            self.claim_cooldown = self.claim_cooldown.max(4);
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
        // ── …AND SOMETIMES IT GOES BEHIND ────────────────────────────
        //
        // The commonest corner in football is a defender getting
        // something on a ball played across his own six-yard box and
        // putting it out, and the engine had no path for it at all. The
        // corner-source census reads **ordinary play 0.15 a match (4%)**
        // against a real ~25% of a 10.4-corner match, while the whole
        // "defender puts it behind" family is fed only by SHOTS and by
        // AIRBORNE deliveries — `try_block_shot`'s corner branch and
        // `DeliveryResolver::heads_it_behind`. The low ball across the
        // face of goal had no way of ending up behind, which is exactly
        // the note `try_block_shot` carries about an earlier attempt at
        // this: correct football, dead because the situation never
        // arose. It arises now — this contest fires 5.7 times a match
        // INSIDE the penalty area.
        //
        // Same curve as `heads_it_behind`, deliberately: the closer to
        // his own line the defender is, the less choice he has about
        // where it goes, and on the line there is no "away" left. A
        // block at the edge of the area still comes off him up the
        // pitch, which is what the branch below does.
        let depth = (self.position.x - own_goal.x).abs();
        if depth <= Self::BEHIND_DEPTH {
            let urgency = 1.0 - depth / Self::BEHIND_DEPTH;
            if context.rng.unit_f32() < Self::BEHIND_AT_LINE * urgency.powf(1.2) {
                // The same geometry the headed clearance uses, so a
                // blocked ball and a headed one leave the pitch the same
                // way. `record_touch` above already made this HIS touch,
                // which is what makes it a corner rather than a goal
                // kick.
                self.velocity = Ball::hook_behind_velocity(
                    self.position,
                    own_goal,
                    context.field_size.height as f32,
                );
                self.flags.in_flight_state =
                    FlightProtection::for_launch(self.velocity, self.position.z);
                self.claim_cooldown = self.claim_cooldown.max(4);
                return;
            }
        }

        let away = (self.position - own_goal)
            .try_normalize(1.0e-3)
            .unwrap_or_else(|| Vector3::new(-self.velocity.x, -self.velocity.y, 0.0).normalize());
        let back = Vector3::new(-self.velocity.x / speed, -self.velocity.y / speed, 0.0);
        let aim = (back * 0.45 + away * 0.55)
            .try_normalize(1.0e-3)
            .unwrap_or(back);
        // ±60° of spread — a block is not an aimed clearance.
        let dir = Self::turned(aim, (context.rng.unit_f32() - 0.5) * 2.1);
        // Pace kept: 25-60%. A block off the shin dies; one off the toe
        // of a stretching leg runs.
        let keep = 0.25 + context.rng.unit_f32() * 0.35;
        self.velocity = dir * speed * keep;
        // It stays a live loose ball at whatever height it was — nothing
        // here writes the vertical axis, which is `ballistics`' business.
        self.claim_cooldown = self.claim_cooldown.max(4);
    }

    /// How far a glance turns the ball off its own line: ±40°, a ball
    /// coming off a shin or a hip rather than out of a swing.
    const GLANCE_SPREAD: f32 = 1.4;

    /// `dir` turned through `angle` radians across the grass.
    fn turned(dir: Vector3<f32>, angle: f32) -> Vector3<f32> {
        let (s, c) = angle.sin_cos();
        Vector3::new(dir.x * c - dir.y * s, dir.x * s + dir.y * c, 0.0)
    }
}

#[cfg(test)]
mod pass_block_tests {
    use super::*;

    /// A ball crossing the grass at 1.6 u/tick (20 m/s) — a cross.
    fn at(perp: f32, along: f32, ticks_to_him: f32, height: f32, danger: f32) -> BlockGeometry {
        BlockGeometry {
            perp,
            along,
            speed: 1.6,
            ticks_to_him,
            height,
            danger,
        }
    }

    /// The lunge as `try_block_pass` wrote it inline before it became
    /// [`PassBlock::chance`] — a man with a full reaction must be rolled
    /// exactly as he was.
    fn inline_lunge(
        perp: f32,
        along: f32,
        speed: f32,
        read: f32,
        delivery: f32,
        danger: f32,
    ) -> f32 {
        if danger <= 0.0 || along < 0.5 || along > 40.0 || perp > 14.0 {
            return 0.0;
        }
        let skill = InterceptionDuel::advantage(read, delivery);
        let perp_factor = 1.0 - (perp / 14.0) * 0.65;
        let line_factor = 1.0 - (along / 40.0) * 0.45;
        let speed_penalty = 1.0 / (1.0 + speed * 0.35);
        (skill * perp_factor * line_factor * speed_penalty * danger * 0.485).clamp(0.0, 0.65)
    }

    #[test]
    fn a_man_with_time_to_read_it_is_rolled_as_the_lunge_always_was() {
        for (perp, along, read, delivery, danger) in [
            (0.0, 16.0, 0.6, 0.6, 0.66),
            (7.0, 8.0, 0.8, 0.4, 1.0),
            (13.9, 39.0, 0.3, 0.9, 0.2),
            (2.0, 0.6, 0.5, 0.5, 0.05),
            (15.0, 10.0, 0.7, 0.7, 0.8),
            (3.0, 41.0, 0.7, 0.7, 0.8),
            (3.0, 10.0, 0.7, 0.7, 0.0),
        ] {
            let geometry = at(perp, along, PassBlock::REACTION_TICKS, 0.2, danger);
            let shared = PassBlock::chance(&geometry, read, delivery);
            let inline = inline_lunge(perp, along, 1.6, read, delivery, danger);
            assert!(
                (shared - inline).abs() < 1e-6,
                "{shared} vs {inline} at perp {perp} along {along}"
            );
        }
    }

    #[test]
    fn a_ball_struck_into_a_man_a_stride_away_is_charged_down() {
        // Halfway (no danger) and too soon to read it: only his body.
        let square = PassBlock::chance(&at(0.0, 16.0, 10.0, 1.0, 0.0), 0.6, 0.6);
        let off_the_line = PassBlock::chance(&at(6.0, 16.0, 10.0, 1.0, 0.0), 0.6, 0.6);
        assert!(square > 0.0, "a ball struck into a body is not a ball through it");
        assert!(square > off_the_line, "{square} square on vs {off_the_line} a toe away");
    }

    #[test]
    fn the_charge_down_is_the_same_on_halfway_and_in_front_of_goal() {
        let halfway = PassBlock::chance(&at(2.0, 8.0, 0.0, 0.5, 0.0), 0.6, 0.6);
        let at_goal = PassBlock::chance(&at(2.0, 8.0, 0.0, 0.5, 1.0), 0.6, 0.6);
        assert!(halfway > 0.0);
        assert!((halfway - at_goal).abs() < 1e-6, "{halfway} vs {at_goal}");
    }

    #[test]
    fn a_ball_over_him_is_not_blocked() {
        for danger in [0.0, 1.0] {
            let geometry = at(0.0, 16.0, 5.0, AerialReach::STANDING + 0.3, danger);
            assert!(!geometry.is_live());
            assert_eq!(PassBlock::chance(&geometry, 0.6, 0.6), 0.0);
        }
    }

    #[test]
    fn no_lunge_far_from_his_own_goal() {
        let read_in_midfield = at(5.0, 20.0, 40.0, 0.2, 0.0);
        assert!(!read_in_midfield.is_live());
        assert_eq!(PassBlock::chance(&read_in_midfield, 0.6, 0.6), 0.0);
        let read_in_his_box = at(5.0, 20.0, 40.0, 0.2, 0.8);
        assert!(PassBlock::chance(&read_in_his_box, 0.6, 0.6) > 0.0);
    }

    #[test]
    fn the_charge_down_fades_with_time_to_react() {
        let chance = |ticks: f32| PassBlock::chance(&at(1.0, 12.0, ticks, 0.5, 0.0), 0.6, 0.6);
        assert!(chance(0.0) > chance(12.0));
        assert!(chance(12.0) > chance(24.0));
        assert_eq!(chance(PassBlock::REACTION_TICKS), 0.0);
    }

    #[test]
    fn the_rate_does_not_walk_with_the_level() {
        let geometry = at(1.0, 12.0, 8.0, 0.5, 0.6);
        let low = PassBlock::chance(&geometry, 0.3, 0.3);
        for level in [0.45, 0.6, 0.8] {
            let here = PassBlock::chance(&geometry, level, level);
            assert!((here - low).abs() < 1e-6, "{here} vs {low} at {level}");
        }
    }

    #[test]
    fn the_contact_takes_a_leg_for_a_lunge_and_a_body_for_a_charge_down() {
        assert_eq!(PassBlock::contact_ceiling(0.0), PassBlock::MAX_HEIGHT);
        assert_eq!(PassBlock::contact_ceiling(1.0), AerialReach::STANDING);
    }

    /// The danger ramp is the scoping argument, so it has to actually
    /// scope: nothing from the halfway line back, everything at the goal.
    #[test]
    fn the_danger_ramp_is_zero_upfield_and_one_at_the_goal() {
        let w = 840.0;
        assert_eq!(PassBlock::danger(w * 0.5, PlayerSide::Left, w), 0.0);
        assert_eq!(PassBlock::danger(w * 0.9, PlayerSide::Left, w), 0.0);
        assert!((PassBlock::danger(0.0, PlayerSide::Left, w) - 1.0).abs() < 1e-6);
        assert!((PassBlock::danger(w, PlayerSide::Right, w) - 1.0).abs() < 1e-6);
        // …and it is monotone toward the goal being defended.
        let near = PassBlock::danger(w * 0.05, PlayerSide::Left, w);
        let far = PassBlock::danger(w * 0.35, PlayerSide::Left, w);
        assert!(near > far, "{near} vs {far}");
    }

    /// Mirror image for the other side, because a rule that reads
    /// differently on the two halves of the pitch is a bug that only
    /// shows up in one direction of play.
    #[test]
    fn the_ramp_is_symmetric_between_the_sides() {
        let w = 840.0;
        for t in [0.0_f32, 0.1, 0.2, 0.35, 0.49] {
            let left = PassBlock::danger(w * t, PlayerSide::Left, w);
            let right = PassBlock::danger(w * (1.0 - t), PlayerSide::Right, w);
            assert!((left - right).abs() < 1e-6, "{left} vs {right} at {t}");
        }
    }
}
