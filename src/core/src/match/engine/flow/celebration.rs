//! The forty-five to seventy-five seconds between the ball hitting the net
//! and the referee's whistle for the restart.
//!
//! # Why this exists at all
//!
//! The post-goal window was a hole in the simulation. `handle_goal_reset`
//! teleported all twenty-two players back into formation and the ball onto
//! the centre spot on the very tick the goal went in, then the engine loop
//! skipped every tick until [`MatchContext::dead_ball_until_ms`] — no
//! physics, no movement, and, crucially, **no position samples**. A replay
//! therefore showed the last frame before the goal held on screen for a
//! minute: the ball frozen an inch short of the line, twenty-two players
//! standing where they happened to be. The most dramatic thing that happens
//! in a football match was the one thing the recording had nothing to say
//! about.
//!
//! # What it does instead
//!
//! The window is now played out. The ball stays in the net (see
//! [`net`](crate::r#match::engine::ball::ball::net)), the scorer wheels away,
//! his team-mates chase him down, the conceding side trudges back, and
//! somebody goes and fetches the ball out of the goal. Only at the END of the
//! window does the field snap into the kickoff set-up — the same set-up, at
//! the same instant, that play used to resume from.
//!
//! # Why it is not the state machine
//!
//! Deliberately choreographed rather than expressed as player states, for the
//! same reason the corner dead-ball set-up teleports its centre-backs: this is
//! a cutscene, not a contest. No decision here can affect the match, so
//! running the AI through it would spend real CPU on twenty-two players
//! deciding how to press a ball that is in the back of the net — and would
//! risk moving numbers that took a lot of work to calibrate.
//!
//! That constraint is load-bearing and worth stating outright: **nothing in
//! this module may draw from [`MatchContext::rng`], emit an event, run a
//! state machine, or touch a statistic.** The RNG stream is shared with every
//! calibrated roll in the engine, so a single extra draw here would shift
//! every subsequent decision in the match. Variation comes from player ids
//! instead — deterministic, free, and outside the stream.

use crate::PlayerFieldPositionGroup;
use crate::r#match::engine::ball::ball::net::GoalNet;
use crate::r#match::{MatchContext, MatchField, MatchPlayer, PlayerSide};
use nalgebra::Vector3;

/// What one player is doing while the goal is being celebrated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    /// The scorer. Runs away from the goal toward the corner, and gets mobbed.
    Hero,
    /// A team-mate near enough to chase the hero down and pile on.
    Mob,
    /// A team-mate too far away to get there — the goalkeeper, the far-side
    /// full-back. He sets off, arms up, gets a fraction of the way, and gives
    /// it up. Nobody sprints ninety metres to a pile-on that will have broken
    /// up before they arrive.
    DistantJoy,
    /// The conceding player who goes and gets the ball out of the net.
    Retriever,
    /// Everyone else on the conceding side. Head down, walk back.
    Dejected,
}

impl Role {
    /// How long this man is simply NOT MOVING after the ball goes in.
    ///
    /// Nobody's first reaction to a goal is to set off somewhere. A
    /// conceding side stands still — hands on hips, hands on head, staring
    /// at the net or at each other — for several seconds before anybody
    /// walks anywhere, and the man who has just been beaten stands there
    /// longest of all, because he is usually on the floor when it happens
    /// and has to get up first.
    ///
    /// This was missing entirely and it is the whole of the reported bug:
    /// the beaten keeper set off for the ball, or trudged toward his
    /// formation spot, on the very tick the ball crossed the line. There
    /// was no reaction to draw because there was no reaction.
    fn stillness_ms(self, beaten: bool) -> u64 {
        if beaten {
            // On the floor. He rolls over, gets to his knees, and looks at
            // it — and only then does he go anywhere, whether that is back
            // into his goal for the ball or out of it.
            return 4_200;
        }
        match self {
            Role::Dejected => 1_600,
            // The retriever's own pause is `FETCH_AFTER_MS`, which already
            // knows whether his side can afford one. Adding a second one on
            // top would hold up a team that is chasing the game.
            _ => 0,
        }
    }
}

/// One player's part in the celebration, fixed at the moment of the goal.
///
/// `anchor` is where he was standing when the ball went in, and it is the
/// reason this is a struct rather than a `(id, role)` pair. Two of the roles
/// want to move a FRACTION of the way somewhere — a fifth of the way to the
/// pile-on, half the way back into shape — and computing that fraction from
/// the player's live position re-computes it every tick, so the target
/// retreats exactly as fast as he approaches it and he never arrives. It is
/// a Zeno chase, and it showed up in the recording as the entire conceding
/// eleven trudging in a straight line at a constant 1.1 m/s for as long as
/// the celebration lasted. Anchoring the fraction to a FIXED point is what
/// makes "a fifth of the way" a place rather than a direction.
struct CastMember {
    id: u32,
    role: Role,
    anchor: Vector3<f32>,
}

/// A goal that has been scored and is being celebrated.
///
/// Created the moment the ball crosses the line and consumed by the restart.
/// The cast is decided once, at that instant, so the choreography is stable
/// for the whole window instead of re-deciding who is chasing whom on every
/// tick.
pub struct GoalCelebration {
    /// The side that restarts — i.e. the one that just conceded.
    pub kickoff_side: PlayerSide,
    /// Match clock (ms) at which the field snaps into the kickoff set-up.
    /// The same instant play used to resume at, so the dead time this
    /// consumes is unchanged.
    pub restart_at_ms: u64,
    /// Match clock (ms) the ball crossed the line.
    started_ms: u64,
    /// Where the hero is running and the pile-on forms.
    focus: Vector3<f32>,
    /// Who is doing what, resolved once at the goal. Keyed by player id
    /// rather than by roster index because substitutions and dismissals
    /// reshuffle the roster.
    cast: Vec<CastMember>,
    /// The man fetching the ball, how long he leaves it in the net first,
    /// and whether he has it yet.
    retriever_id: Option<u32>,
    fetch_at_ms: u64,
    collected: bool,
    /// The goalkeeper who has just been beaten.
    ///
    /// Held separately from [`Role`] because being beaten is not a job —
    /// he may or may not also be the man who fetches the ball, and either
    /// way he is the one player on the pitch whose reaction to the goal is
    /// worth drawing. See [`Role::stillness_ms`].
    beaten_id: Option<u32>,
    /// The conceding side is now behind. Decides whether the ball is walked
    /// back or sprinted back.
    chasing: bool,
}

impl GoalCelebration {
    /// The huddle breaks up here and everybody starts walking back, in ms of
    /// match clock.
    const HUDDLE_MS: u64 = 12_000;

    /// How long the ball is left in the net before anybody goes for it.
    ///
    /// Not padding: this IS the shot everyone replays. A beaten keeper picks
    /// himself up off his line before he trudges in for it, and the seconds
    /// he takes are the seconds the ball spends bagged in the netting. A side
    /// that is behind does not have them to spare.
    const FETCH_AFTER_MS: u64 = 5_000;
    const FETCH_AFTER_CHASING_MS: u64 = 1_200;

    /// Movement speeds, in game units per tick. One unit is 12.5 cm and one
    /// tick is 10 ms, so 0.08 u/tick is 1 m/s — these are real speeds, not
    /// tuned ones.
    const SPRINT: f32 = 0.70; // 8.75 m/s — a celebration is a sprint
    const RUN: f32 = 0.48; // 6 m/s
    const JOG: f32 = 0.26; // 3.25 m/s
    /// 0.875 m/s. You have just conceded, and you are not in a hurry.
    ///
    /// ⚠ Was 0.09 — **1.125 m/s**, which is within a fiftieth of the
    /// replay viewer's own walk/run threshold (`Actors::MOVING`, 1.1 m/s).
    /// That threshold is what decides whether a figure is drawn facing the
    /// way he is going or facing the ball, so an entire conceding eleven
    /// walked back oscillating across it and pivoted between the two —
    /// reported as players "spinning round". A speed that sits on a
    /// consumer's threshold is a bug even when the number itself is
    /// defensible.
    const TRUDGE: f32 = 0.07;

    /// Close enough to the ball to pick it up (75 cm).
    const COLLECT_DISTANCE: f32 = 6.0;

    /// How far a team-mate will chase the pile-on: 360u = 45 m, about half
    /// the pitch. Beyond that he raises an arm and jogs a few steps instead —
    /// see [`Role::DistantJoy`]. Without the bound the whole eleven, keeper
    /// aside, sprinted flat out for the corner flag from wherever they stood,
    /// which measured at 70 m each in the eight seconds after a goal.
    const MOB_RADIUS: f32 = 360.0;

    /// Nobody walks through a touchline, even in a celebration.
    const TOUCHLINE_MARGIN: f32 = 12.0;

    /// How close to the focus counts as having arrived (1.5 m).
    const ARRIVED: f32 = 12.0;

    /// Decide the cast and where the party is.
    ///
    /// `kickoff_side` is the conceding side; everything else is derived from
    /// the ball, which is still in the net and still knows who put it there.
    pub fn arm(
        field: &MatchField,
        context: &MatchContext,
        kickoff_side: PlayerSide,
        restart_at_ms: u64,
    ) -> Self {
        let conceding_side = kickoff_side;
        let scoring_side = match conceding_side {
            PlayerSide::Left => PlayerSide::Right,
            PlayerSide::Right => PlayerSide::Left,
        };

        // An own goal has a scorer but no hero: nobody mobs the defender who
        // has just put it in his own net, and he is on the OTHER team from
        // the one celebrating. The pile-on then forms at the corner flag on
        // its own, which is what a team does when the ball goes in off
        // somebody else.
        let hero_id = field
            .ball
            .in_net
            .filter(|state| !state.auto_goal)
            .map(|state| state.scorer_id);

        let chasing = Self::conceding_side_is_behind(field, context, conceding_side);
        let retriever_id = Self::pick_retriever(field, conceding_side, chasing);
        let focus = Self::party_spot(field, conceding_side);

        let beaten_id = field
            .players
            .iter()
            .find(|p| {
                p.side == Some(conceding_side)
                    && !p.is_sent_off
                    && p.tactical_position.current_position.position_group()
                        == PlayerFieldPositionGroup::Goalkeeper
            })
            .map(|p| p.id);

        let mut cast = Vec::with_capacity(field.players.len());
        for player in field.players.iter().filter(|p| !p.is_sent_off) {
            let is_keeper = player.tactical_position.current_position.position_group()
                == PlayerFieldPositionGroup::Goalkeeper;
            let role = if Some(player.id) == retriever_id {
                Role::Retriever
            } else if player.side == Some(scoring_side) {
                let far = (player.position - focus).magnitude() > Self::MOB_RADIUS;
                if Some(player.id) == hero_id {
                    Role::Hero
                } else if is_keeper || far {
                    Role::DistantJoy
                } else {
                    Role::Mob
                }
            } else {
                Role::Dejected
            };
            cast.push(CastMember {
                id: player.id,
                role,
                anchor: player.position,
            });
        }

        GoalCelebration {
            kickoff_side,
            restart_at_ms,
            started_ms: context.total_match_time,
            focus,
            cast,
            retriever_id,
            fetch_at_ms: if chasing {
                Self::FETCH_AFTER_CHASING_MS
            } else {
                Self::FETCH_AFTER_MS
            },
            collected: false,
            beaten_id,
            chasing,
        }
    }

    /// One tick of the celebration. `true` while it is still running,
    /// `false` once the restart has been performed and the goal is over.
    pub fn advance(&mut self, field: &mut MatchField, context: &MatchContext) -> bool {
        if context.total_match_time >= self.restart_at_ms {
            Self::restart(field, self.kickoff_side);
            return false;
        }

        let elapsed = context.total_match_time.saturating_sub(self.started_ms);
        self.move_cast(field, elapsed, context.current_tick());
        self.move_ball(field, context, elapsed);
        true
    }

    /// Snap into the kickoff set-up. This is the moment play used to resume
    /// at, and it leaves exactly the state it always did: everybody on their
    /// formation spot, the ball on the centre spot, the conceding side
    /// standing over it.
    pub fn restart(field: &mut MatchField, kickoff_side: PlayerSide) {
        field.reset_players_positions();
        for player in field.players.iter_mut() {
            // Nobody restarts mid-leap.
            player.height = 0.0;
            player.vertical_speed = 0.0;
        }
        field.ball.reset();
        super::goal::assign_kickoff(field, kickoff_side);
    }

    /// Where the celebration happens: infield of the corner flag nearest to
    /// where the ball went in, on the side of the pitch it crossed on.
    ///
    /// That is where a scorer actually goes — toward the corner and the
    /// crowd behind it, never back toward the halfway line.
    fn party_spot(field: &MatchField, conceding_side: PlayerSide) -> Vector3<f32> {
        let field_width = field.size.width as f32;
        let field_height = field.size.height as f32;
        // The goal that was scored in is the conceding side's own.
        let (goal_line_x, infield) = match conceding_side {
            PlayerSide::Left => (0.0, 1.0),
            PlayerSide::Right => (field_width, -1.0),
        };
        let centre_y = field_height * 0.5;
        // Which flank the ball crossed on — a goal into the far corner is
        // celebrated in that corner.
        let flank = if field.ball.position.y >= centre_y {
            1.0
        } else {
            -1.0
        };
        Vector3::new(
            goal_line_x + infield * 96.0, // 12 m infield of the goal line
            centre_y + flank * (field_height * 0.5 - 44.0),
            0.0,
        )
    }

    /// Who goes to get the ball.
    ///
    /// A keeper who has just picked it out of his own net normally walks it
    /// back; a team that is behind sends whoever is nearest, and he sprints,
    /// because the clock is the thing they are short of. Both are so
    /// characteristic of the situation that the choice is worth making — and
    /// it is free, because nothing decided here can be played.
    fn pick_retriever(
        field: &MatchField,
        conceding_side: PlayerSide,
        chasing: bool,
    ) -> Option<u32> {
        // No ball in the net means no scorer was credited and the ball has
        // already gone back to the centre spot — nothing to fetch.
        field.ball.in_net?;

        let ball = field.ball.position;
        field
            .players
            .iter()
            .filter(|p| p.side == Some(conceding_side) && !p.is_sent_off)
            .filter(|p| {
                let is_keeper = p.tactical_position.current_position.position_group()
                    == PlayerFieldPositionGroup::Goalkeeper;
                if chasing { !is_keeper } else { is_keeper }
            })
            .min_by(|a, b| {
                let da = (a.position - ball).norm_squared();
                let db = (b.position - ball).norm_squared();
                da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|p| p.id)
    }

    /// Is the side that just conceded now losing?
    ///
    /// Reads the real scoreline rather than
    /// [`MatchContext::behavioral_score_visible`], and is the one place in
    /// the engine allowed to: this decides who jogs and who sprints during
    /// dead time, and nothing decided here reaches a ball, a duel or a shot.
    /// The score-blindness discipline exists to stop the scoreline steering
    /// PLAY.
    fn conceding_side_is_behind(
        field: &MatchField,
        context: &MatchContext,
        conceding_side: PlayerSide,
    ) -> bool {
        let Some(conceding_team) = field
            .players
            .iter()
            .find(|p| p.side == Some(conceding_side))
            .map(|p| p.team_id)
        else {
            return false;
        };
        let home = context.score.home_team.get();
        let away = context.score.away_team.get();
        if conceding_team == context.score.home_team.team_id {
            home < away
        } else {
            away < home
        }
    }

    fn move_cast(&self, field: &mut MatchField, elapsed: u64, tick: u64) {
        let field_width = field.size.width as f32;
        let field_height = field.size.height as f32;
        let ball = field.ball.position;
        let centre = Vector3::new(field_width * 0.5, field_height * 0.5, 0.0);
        // Once the huddle breaks, everybody is heading for the same place
        // they would have been teleported to at the whistle.
        let walking_back = elapsed >= Self::HUDDLE_MS;

        // Players outside, cast inside: the roster is the thing being
        // mutated, and it keeps the whole pass linear in the roster rather
        // than doing a `find` over it per cast member.
        for (index, player) in field.players.iter_mut().enumerate() {
            let Some(part) = self.cast.iter().find(|member| member.id == player.id) else {
                continue;
            };
            let (role, anchor) = (&part.role, part.anchor);
            // The offsets below give the pile-on and the walk-back a shape,
            // so twenty-two players don't converge on one point and stack
            // into a single sprite. Derived from the player id, NOT from the
            // RNG — see the module note on the shared stream.
            let spread = Self::spread(player.id, index);

            // Nobody's first reaction to a goal is to walk somewhere, and
            // the beaten keeper's is to stay exactly where the ball left
            // him. Ahead of the role match because it is true whatever his
            // job in the restart turns out to be — see
            // [`Role::stillness_ms`].
            let beaten = Some(player.id) == self.beaten_id;
            if elapsed < role.stillness_ms(beaten) {
                Self::steer(player, player.position, 0.0, field_width, field_height);
                continue;
            }

            let (target, speed) = match role {
                // The man with the ball keeps going whatever the clock says:
                // the restart needs it back on the centre spot. A side that
                // is behind runs it there.
                Role::Retriever if self.collected => {
                    (centre, if self.chasing { Self::RUN } else { Self::JOG })
                }
                // Leave it in the net a moment first — see `FETCH_AFTER_MS`.
                Role::Retriever if elapsed < self.fetch_at_ms => (player.position, 0.0),
                Role::Retriever => (
                    ball,
                    if self.chasing {
                        Self::SPRINT
                    } else {
                        Self::JOG
                    },
                ),
                _ if walking_back => (player.start_position, Self::JOG),
                Role::Hero => {
                    if (player.position - self.focus).norm() < Self::ARRIVED {
                        // He has run out of pitch. This is the bit where he
                        // slides, or points at somebody, or jumps.
                        if player.height <= 0.0 && (tick + player.id as u64) % 190 == 0 {
                            player.leap(0.45);
                        }
                        (player.position, 0.0)
                    } else {
                        (self.focus, Self::SPRINT)
                    }
                }
                Role::Mob => (self.focus + spread * 14.0, Self::SPRINT),
                Role::DistantJoy => {
                    // He sets off for the pile-on, covers a fifth of the
                    // ground, and settles for celebrating where he is. The
                    // fraction is of HIS OWN distance, so the keeper gets
                    // twenty metres up the pitch and the far full-back gets
                    // ten — which is what each of them actually does.
                    let toward = anchor + (self.focus - anchor) * 0.2;
                    (
                        Vector3::new(toward.x, toward.y + spread.y * 8.0, 0.0),
                        Self::RUN,
                    )
                }
                Role::Dejected => {
                    // Back toward their own shape, heads down, taking their
                    // time about it.
                    let drift = anchor + (player.start_position - anchor) * 0.5;
                    (
                        Vector3::new(drift.x, drift.y + spread.y * 6.0, 0.0),
                        Self::TRUDGE,
                    )
                }
            };

            Self::steer(player, target, speed, field_width, field_height);
        }
    }

    /// Advance the ball: netting physics while it is loose in the goal,
    /// carried once somebody has picked it out.
    fn move_ball(&mut self, field: &mut MatchField, context: &MatchContext, elapsed: u64) {
        if field.ball.in_net.is_none() {
            return;
        }

        if self.collected {
            if let Some(carrier) = self
                .retriever_id
                .and_then(|id| field.players.iter().find(|p| p.id == id))
            {
                // In his hands, at chest height — parked on his own
                // coordinate, which is also the signal the replay reads it
                // by. There is no body model here to hang it off, and the
                // viewer has one: a ball sitting still on a man's own
                // centreline at chest height is in his hands and cannot be
                // anything else, so it draws the cradle and puts the ball
                // where the cradle puts it (`Actors::CARRIED_REACH`). Move
                // this off his centreline and the man carrying it goes back
                // to trudging along with his hands on his head.
                let carry = carrier.position;
                field.ball.position = Vector3::new(carry.x, carry.y, 1.05);
                field.ball.velocity = Vector3::zeros();
            }
            return;
        }

        field.ball.tick_net(&context.goal_positions);

        // Nobody picks it up before they have set off for it — a keeper
        // already standing in his own net would otherwise collect it on the
        // tick it arrived, and the netting shot would last one frame.
        if elapsed < self.fetch_at_ms {
            return;
        }

        if let Some(retriever) = self
            .retriever_id
            .and_then(|id| field.players.iter().find(|p| p.id == id))
        {
            let reach = retriever.position - field.ball.position;
            if (reach.x * reach.x + reach.y * reach.y).sqrt() <= Self::COLLECT_DISTANCE {
                self.collected = true;
                // Ownership is what tells the netting to stop simulating it;
                // the restart clears it again.
                field.ball.current_owner = Some(retriever.id);
                field.ball.velocity = Vector3::zeros();
            }
        }
    }

    /// Move `player` toward `target` at `speed` units per tick, and let
    /// whatever leap he is in the middle of run its course.
    fn steer(
        player: &mut MatchPlayer,
        target: Vector3<f32>,
        speed: f32,
        field_width: f32,
        field_height: f32,
    ) {
        let (height, rise) = MatchPlayer::fall(player.height, player.vertical_speed);
        player.height = height;
        player.vertical_speed = rise;

        let dx = target.x - player.position.x;
        let dy = target.y - player.position.y;
        let distance = (dx * dx + dy * dy).sqrt();
        if speed <= 0.0 || distance < 0.05 {
            player.velocity = Vector3::zeros();
            return;
        }

        let step = speed.min(distance);
        let (ux, uy) = (dx / distance, dy / distance);
        player.velocity = Vector3::new(ux * step, uy * step, 0.0);
        player.position.x += ux * step;
        player.position.y += uy * step;

        // A celebration can take a player behind the goal line — that is
        // where the corner flag is, and it is where the ball is — but not
        // off the pitch entirely. The bound is the back of the BAGGED net
        // rather than the net's nominal depth: a ball driven in hard sits
        // inside the mesh's give, and a keeper who can only reach the back
        // bar can never quite get to it.
        let reach_behind = GoalNet::DEPTH + GoalNet::GIVE_BACK;
        player.position.x = player
            .position
            .x
            .clamp(-reach_behind, field_width + reach_behind);
        player.position.y = player.position.y.clamp(
            Self::TOUCHLINE_MARGIN,
            field_height - Self::TOUCHLINE_MARGIN,
        );
    }

    /// A stable per-player offset direction, so the pile-on has a shape.
    ///
    /// A hash of the id rather than a draw from the match RNG: the stream is
    /// shared with every calibrated roll in the engine and a celebration must
    /// not consume from it.
    fn spread(player_id: u32, index: usize) -> Vector3<f32> {
        let mixed = player_id
            .wrapping_mul(2_654_435_761)
            .wrapping_add(index as u32);
        let angle = (mixed % 3600) as f32 * (std::f32::consts::TAU / 3600.0);
        let radius = 0.5 + ((mixed >> 12) % 100) as f32 / 100.0;
        Vector3::new(angle.cos() * radius, angle.sin() * radius, 0.0)
    }
}
