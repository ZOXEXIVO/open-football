use super::cast::{CastMember, Role};
use crate::PlayerFieldPositionGroup;
use crate::r#match::engine::flow::field::ResetReason;
use crate::r#match::engine::flow::goal;
use crate::r#match::{MatchContext, MatchField, PlayerSide};
use nalgebra::Vector3;

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
    /// Where the ball was lying when he got to it, and the match clock (ms)
    /// he reached it at. The pick-up is played out between the two — see
    /// [`GoalCelebration::PICKUP_MS`].
    pickup_from: Vector3<f32>,
    pickup_at_ms: u64,
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

    /// How long the pick-up itself takes, in ms of match clock.
    ///
    /// It used to take none: the tick he came within `COLLECT_DISTANCE` the
    /// ball was WRITTEN onto his own coordinate at `CARRY_HEIGHT`, so it
    /// jumped up to 75 cm across the grass and a metre into the air inside
    /// one frame, and he set off with it on the same tick. Measured over
    /// 3 000 recorded goal clips: 1 366 of them show it, a median 1.9 s
    /// after the ball crossed the line — which is the second half of the
    /// reported *"instantly flips back into the goalkeeper's hands"*, the
    /// first half being the save teleport in `Ball::try_save_shot`.
    ///
    /// Half a second is a man bending down, picking a ball out of a net and
    /// straightening up with it. He stands still for it, and the ball is
    /// eased rather than written — see [`GoalCelebration::move_ball`].
    const PICKUP_MS: u64 = 500;

    /// Where a carried ball rides: in his hands, at chest height.
    const CARRY_HEIGHT: f32 = 1.05;

    /// How far a team-mate will chase the pile-on: 360u = 45 m, about half
    /// the pitch. Beyond that he raises an arm and jogs a few steps instead —
    /// see [`Role::DistantJoy`]. Without the bound the whole eleven, keeper
    /// aside, sprinted flat out for the corner flag from wherever they stood,
    /// which measured at 70 m each in the eight seconds after a goal.
    const MOB_RADIUS: f32 = 360.0;

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
            pickup_from: Vector3::zeros(),
            pickup_at_ms: 0,
            beaten_id,
            chasing,
        }
    }

    /// One tick of the celebration. `true` while it is still running,
    /// `false` once the restart has been performed and the goal is over.
    pub fn advance(&mut self, field: &mut MatchField, context: &MatchContext) -> bool {
        if context.total_match_time >= self.restart_at_ms {
            // The retriever is standing on the centre spot with the ball —
            // he takes it. See `assign_kickoff`.
            Self::restart(field, self.kickoff_side, self.retriever_id);
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
    pub fn restart(field: &mut MatchField, kickoff_side: PlayerSide, taker: Option<u32>) {
        // Only leave him standing if he is the man who takes it — the
        // retriever is sometimes the beaten goalkeeper, who is not.
        // `can_take_kickoff` is the one test, asked here and again inside
        // `assign_kickoff`, so the two cannot disagree and strand a player
        // off his spot with somebody else on the ball.
        let keep = taker.filter(|id| goal::can_take_kickoff(field, kickoff_side, *id));
        field.reset_players_positions(ResetReason::Restart { keep });
        for player in field.players.iter_mut() {
            // Nobody restarts mid-leap.
            player.height = 0.0;
            player.vertical_speed = 0.0;
        }
        field.ball.reset();
        goal::assign_kickoff(field, kickoff_side, keep);
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
        // …and the man fetching the ball is bending down for it until here.
        let pickup_over = self.pickup_at_ms.saturating_sub(self.started_ms) + Self::PICKUP_MS;

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
                // He is bending down for it. Nobody sets off with a ball on
                // the tick his fingers reach it, and the ball is still on
                // its way into his hands — see `PICKUP_MS`.
                Role::Retriever if self.collected && elapsed < pickup_over => {
                    (player.position, 0.0)
                }
                // The man with the ball keeps going whatever the clock says:
                // the restart needs it back on the centre spot. A side that
                // is behind runs it there.
                //
                // ⚠ **And he stays there, holding it, until the restart.**
                //
                // He should not: `pick_retriever` sends the GOALKEEPER
                // unless the side is chasing, and a keeper cannot take the
                // kickoff — so `GoalCelebration::restart` sends him back to
                // his line, which the player census reports as 0.3 players
                // a restart at a suspiciously tight **400 u mean, 50 m**
                // (the centre spot to a goalkeeper's slot, almost exactly).
                // 33 m/match.
                //
                // Walking him home instead was tried and reverted, and the
                // reason is worth keeping: while `collected` is up,
                // `move_ball` parks the ball **on the carrier's own
                // coordinate every tick** — that is the signal the replay
                // reads a carried ball by. So the instant he walks anywhere
                // the ball goes with him, and releasing it costs a jump of
                // up to a metre on the grass, caught immediately by
                // `the_ball_is_picked_out_of_the_net_rather_than_snapping_
                // into_his_hands`. Trading 33 m of player relocation for a
                // new ball one is not a trade.
                //
                // The fix that would work is a put-down eased like
                // `PICKUP_MS` is (a `putdown_from` / `putdown_at_ms` pair,
                // mirroring the pick-up), after which he is free to walk.
                // Left for whoever picks this up next.
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
                let hold = Vector3::new(carry.x, carry.y, Self::CARRY_HEIGHT);
                // …but he has to PICK IT UP first. Straight to the hold is
                // a teleport of up to `COLLECT_DISTANCE` across the grass
                // and the whole carry height into the air, in one tick —
                // see `PICKUP_MS`. Eased with a smoothstep, so the ball
                // leaves the ground and arrives in his hands with no step
                // change in speed at either end.
                let since = context.total_match_time.saturating_sub(self.pickup_at_ms);
                field.ball.position = if since >= Self::PICKUP_MS {
                    hold
                } else {
                    let t = since as f32 / Self::PICKUP_MS as f32;
                    let eased = t * t * (3.0 - 2.0 * t);
                    self.pickup_from + (hold - self.pickup_from) * eased
                };
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
                // Where it was lying, and when he got to it. The pick-up is
                // played out from here — see `PICKUP_MS`.
                self.pickup_from = field.ball.position;
                self.pickup_at_ms = context.total_match_time;
                // Ownership is what tells the netting to stop simulating it;
                // the restart clears it again.
                field.ball.current_owner = Some(retriever.id);
                field.ball.velocity = Vector3::zeros();
            }
        }
    }
}
