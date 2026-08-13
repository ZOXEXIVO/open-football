use crate::PlayerFieldPositionGroup;
use crate::club::player::skills::GoalkeeperSpeedContext;
use crate::r#match::defenders::states::DefenderState;
use crate::r#match::engine::ball::ball::AerialReach;
use crate::r#match::engine::flow::goal::GOAL_HEIGHT;
use crate::r#match::events::EventCollection;
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::player::strategies::players::ops::skill_composites as sc;
use crate::r#match::player::transition::TransitionSource;
use crate::r#match::{
    GameTickContext, MatchContext, MatchPlayer, MovementEffort, StateProcessingResult,
};

use nalgebra::Vector3;
use std::fmt::Display;
use std::fmt::Formatter;
use std::fmt::Result;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PlayerState {
    Injured,
    Goalkeeper(GoalkeeperState),
    Defender(DefenderState),
    Midfielder(MidfielderState),
    Forward(ForwardState),
}

impl PlayerState {
    /// Cheap integer ID for fast dedup — avoids `to_string()` allocation.
    /// Each (outer variant, inner variant) pair maps to a unique u16.
    ///
    /// The id is embedded in replay / position records, so it must stay
    /// stable across releases. The inner `*s as u16` reads the inner
    /// enum's **explicit** discriminant (every state enum pins them), and
    /// `compact_id_snapshot` in this file's tests fails loudly if any
    /// value drifts — so it never silently depends on variant order.
    #[inline]
    pub fn compact_id(&self) -> u16 {
        match self {
            PlayerState::Injured => 0,
            PlayerState::Goalkeeper(s) => 100 + (*s as u16),
            PlayerState::Defender(s) => 200 + (*s as u16),
            PlayerState::Midfielder(s) => 300 + (*s as u16),
            PlayerState::Forward(s) => 400 + (*s as u16),
        }
    }

    /// Every state in the engine's universe, role-then-declaration order.
    /// The single source of truth for the transition-graph audit and the
    /// compact-id stability snapshot. Built from each role's `ALL`
    /// registry, so adding a state in one place flows through here.
    pub fn all() -> Vec<PlayerState> {
        let mut states = Vec::with_capacity(
            1 + GoalkeeperState::ALL.len()
                + DefenderState::ALL.len()
                + MidfielderState::ALL.len()
                + ForwardState::ALL.len(),
        );
        states.push(PlayerState::Injured);
        states.extend(GoalkeeperState::ALL.map(PlayerState::Goalkeeper));
        states.extend(DefenderState::ALL.map(PlayerState::Defender));
        states.extend(MidfielderState::ALL.map(PlayerState::Midfielder));
        states.extend(ForwardState::ALL.map(PlayerState::Forward));
        states
    }

    /// States a player may occupy with no inbound transition: the per-role
    /// kickoff defaults the match *starts* in (via `set_default_state`)
    /// before any transition fires. The transition-graph audit exempts
    /// these from the "every state has an inbound edge" rule.
    pub fn entry_states() -> [PlayerState; 4] {
        [
            PlayerState::Goalkeeper(GoalkeeperState::Standing),
            PlayerState::Defender(DefenderState::Standing),
            PlayerState::Midfielder(MidfielderState::Standing),
            PlayerState::Forward(ForwardState::Standing),
        ]
    }

    /// States reserved / not wired into the in-match state machine.
    ///
    /// Empty: every state in the universe is now reachable. The audit
    /// treats reserved states as both entry (no inbound required) and
    /// terminal (no outbound required); keeping the hook (rather than
    /// deleting it) means a future placeholder state has a documented
    /// home instead of silently tripping the reachability guard.
    pub fn reserved_states() -> [PlayerState; 0] {
        []
    }

    /// True while the player is mid-way through a physical action that a
    /// real footballer cannot abort: a keeper already committed to a dive
    /// or a claim, a defender sliding in, a header already launched, a
    /// shot or cross being struck.
    ///
    /// The engine's out-of-band loose-ball redirects (the universal
    /// `should_force_takeball` override and the ball's `TakeMe` signal)
    /// used to yank players out of exactly these states — the observed
    /// transition graph carried `Goalkeeper: Diving -> Take Ball`,
    /// `Forward: Heading -> Take Ball` and `Defender: Tackling -> Take
    /// Ball` edges. Committed players are skipped by those redirects AND
    /// excluded from the loose-ball chase table
    /// ([`LooseBallChase`](crate::r#match::LooseBallChase)), so the
    /// designation passes to the next-closest teammate instead of
    /// stranding the ball.
    ///
    /// Deliberately narrow: only sub-second actions belong here. Longer
    /// deliberative states (`Passing`, `Distributing`, `Assisting`,
    /// `SwitchingPlay`) stay interruptible so a player can never be
    /// frozen out of a chase for a meaningful stretch of play. Even if
    /// every nearby player were committed, the per-state
    /// `is_best_player_to_chase_ball` path still claims loose balls, so
    /// this can never deadlock possession.
    pub fn is_committed_action(&self) -> bool {
        match self {
            // An injured player is on the floor — they chase nothing.
            PlayerState::Injured => true,
            // Only the strike itself — the dive, the leap, the punch, the
            // claim. `PreparingForSave` is deliberately NOT here: it is a
            // set STANCE the keeper can hold for a long time, not an
            // un-abortable action, and freezing them in it stops them
            // coming off their line for a loose ball. `HoldingBall` is
            // likewise excluded — the ball is owned, so the loose-ball
            // redirects cannot fire anyway.
            PlayerState::Goalkeeper(s) => matches!(
                s,
                GoalkeeperState::Diving
                    | GoalkeeperState::Jumping
                    | GoalkeeperState::Catching
                    | GoalkeeperState::Punching
            ),
            PlayerState::Defender(s) => matches!(
                s,
                DefenderState::Tackling
                    | DefenderState::Heading
                    | DefenderState::Clearing
                    | DefenderState::Shooting
                    | DefenderState::Crossing
                    | DefenderState::AttackingCorner
            ),
            PlayerState::Midfielder(s) => matches!(
                s,
                MidfielderState::Tackling
                    | MidfielderState::Heading
                    | MidfielderState::Shooting
                    | MidfielderState::DistanceShooting
                    | MidfielderState::Crossing
            ),
            PlayerState::Forward(s) => matches!(
                s,
                ForwardState::Shooting
                    | ForwardState::Finishing
                    | ForwardState::Heading
                    | ForwardState::Tackling
                    | ForwardState::Crossing
            ),
        }
    }
}

impl Display for PlayerState {
    fn fmt(&self, f: &mut Formatter) -> Result {
        match self {
            PlayerState::Injured => write!(f, "Injured"),
            PlayerState::Goalkeeper(state) => write!(f, "Goalkeeper: {}", state),
            PlayerState::Defender(state) => write!(f, "Defender: {}", state),
            PlayerState::Midfielder(state) => write!(f, "Midfielder: {}", state),
            PlayerState::Forward(state) => write!(f, "Forward: {}", state),
        }
    }
}

/// How long a player is committed to a decision before he is allowed to
/// simply undo it.
///
/// # The problem this exists for
///
/// The engine re-runs every player's full decision tree on every AI tick
/// — one every 20 ms. Nothing anywhere required a decision to survive
/// longer than that. Measured over a real match (`dev_match trace`), the
/// consequence was that **90% of all state transitions left the state
/// after one tick or less**, players changed state ~9 times a second, and
/// **73% of transitions went straight back to the state just left**.
///
/// The cause is always the same shape: two states answering one question
/// with slightly different predicates, so a player satisfies both at
/// once. `Pressing` commits to a challenge inside 25u while `Tackling`
/// breaks off outside 10u; `Returning` finishes its run at 80u from the
/// mark while `Running` sends it back whenever the ball is 50u away. Each
/// such pair is a two-cycle with no exit, and the engine has a dozen of
/// them. The worst are worth fixing individually — and are fixed — but
/// the defect is structural: nothing was stopping the machine from
/// oscillating in the first place.
///
/// # The constraint
///
/// A footballer cannot reverse a decision faster than they can make one.
/// Choice reaction time in sport is ~150-250 ms; the engine's decision
/// tick is 20 ms, so re-deciding every tick is not a modelling choice,
/// it is physically impossible. This applies that latency — and only to
/// the pathological case: a transition BACK to the state the player just
/// came from, while the current decision is still younger than his own
/// reaction time. Any transition to a genuinely new state is untouched,
/// so nothing here can trap a player anywhere.
///
/// Latency is the player's own: reading the game quickly is what
/// `decisions`, `anticipation` and `concentration` describe, so a sharp
/// midfielder corrects himself in ~120 ms and a poor one takes ~200 ms.
pub struct DecisionCommitment;

impl DecisionCommitment {
    /// Reaction latency in AI ticks (one AI tick = 20 ms of match time).
    /// 6 ticks = 120 ms for an elite reader of the game, 10 ticks =
    /// 200 ms for a poor one — the human choice-reaction band.
    const FASTEST_TICKS: f32 = 6.0;
    const SLOWEST_TICKS: f32 = 10.0;

    /// This player's own latency before he may reverse a decision.
    fn latency_ticks(player: &MatchPlayer) -> u64 {
        let m = &player.skills.mental;
        // Mean of the three attributes that describe how fast a player
        // reads a changing picture, on 1..20.
        let sharpness = ((m.decisions + m.anticipation + m.concentration) / 3.0).clamp(1.0, 20.0);
        let t = (sharpness - 1.0) / 19.0;
        (Self::SLOWEST_TICKS - t * (Self::SLOWEST_TICKS - Self::FASTEST_TICKS)).round() as u64
    }

    /// Should this transition be held back for a tick?
    ///
    /// True only when ALL of the following hold:
    ///   * the destination is exactly the state we came from — a
    ///     reversal, not progress;
    ///   * the current decision is younger than the player's reaction
    ///     latency;
    ///   * the transition carries no event, so nothing has actually been
    ///     executed (a pass, a claim, a foul) that must not be lost;
    ///   * neither end is a committed physical action — a strike, a
    ///     header, a dive resolves the moment it is decided.
    ///
    /// Blocking one leg is enough to break a two-cycle, so exempting the
    /// committed-action end costs nothing.
    ///
    /// Being ON THE BALL is deliberately NOT an exemption. It was one at
    /// first, on the reasoning that the picture changes under a carrier's
    /// feet and must stay reactive — but the exemption is what left the
    /// two largest loops in the engine undamped once the off-ball ones
    /// were fixed: `Forward: Passing <-> Forward: Shooting` (~14,400 round
    /// trips a match) and `Forward: Passing <-> Forward: Running`
    /// (~9,500). It is also redundant. A carrier who actually DOES
    /// something — plays the pass, strikes the ball — emits an event, and
    /// event-carrying results are exempt on the line above; one who
    /// merely changes his mind about what he might do is exactly the case
    /// this guard exists for. A real player cannot re-decide a pass into
    /// a shot and back twice in a tenth of a second either.
    fn holds(
        player: &MatchPlayer,
        target: PlayerState,
        result: &StateProcessingResult,
        _on_ball: bool,
    ) -> bool {
        if player.state == PlayerState::Injured {
            return false;
        }
        if result.events.has_events() {
            return false;
        }
        if target.is_committed_action() || player.state.is_committed_action() {
            return false;
        }
        let Some(previous) = player.previous_state else {
            return false;
        };
        if previous.compact_id() != target.compact_id() {
            return false;
        }
        player.in_state_time < Self::latency_ticks(player)
    }
}

pub struct PlayerMatchState;

impl PlayerMatchState {
    pub fn process(
        player: &mut MatchPlayer,
        context: &MatchContext,
        tick_context: &GameTickContext,
    ) -> EventCollection {
        let player_position_group = player.tactical_position.current_position.position_group();

        let state_change_result =
            player_position_group.process(player.in_state_time, player, context, tick_context);

        if state_change_result.start_tackle_cooldown {
            player.start_tackle_cooldown();
        }

        // Decision commitment: a transition that merely undoes the last
        // one is held until the player has had time to actually re-read
        // the picture (see `DecisionCommitment`). Resolved here, before
        // anything else consumes the result, so the shot-reason bookkeeping
        // and the transition itself see one consistent decision.
        let on_ball = tick_context.ball.current_owner == Some(player.id);
        let new_state = state_change_result
            .state
            // A handler returning its OWN state is not a transition, it is
            // a handler that meant `None`. Treating it as one reset
            // `in_state_time` to zero, so every `in_state_time > N` guard
            // inside that state became unreachable for as long as the
            // condition producing the self-return held — the state could
            // never time out of itself. Observed on `Midfielder: Running`
            // and `Forward: Running`, both of which carry timeout-driven
            // exits. Dropping it here fixes every such handler at once and
            // leaves the timer running, which is what a no-op tick means.
            .filter(|target| target.compact_id() != player.state.compact_id())
            .filter(|target| {
                !DecisionCommitment::holds(player, *target, &state_change_result, on_ball)
            });

        // Stash the shot reason on the player. The Shooting state will
        // consume and clear this when it composes the Shoot event.
        // A reason-less transition to any non-shot state invalidates a
        // stale flag: every Shooting-state abort (lost ball, cooldown,
        // range) used to leave the reason armed, so the NEXT entry into
        // Shooting — from any path — skipped the unified shot helper
        // entirely and fired ungated. A blocked shot must not convert
        // into a guaranteed free shot later.
        if let Some(reason) = state_change_result.shot_reason {
            player.pending_shot_reason = Some(reason);
        } else if let Some(new_state) = new_state {
            if !Self::is_shot_emitting_state(new_state) {
                // A STRUCK shot leaves the same way a lost one does, so the
                // event has to be what separates them.
                //
                // The shooting states return `Running` + a `Shoot` event and
                // no `shot_reason` (the reason was consumed to build the
                // event), which lands here — so every armed shot that
                // actually FIRED was being counted as destroyed. The harness
                // read 632 "queued shots lost" against 985 shots taken and
                // the number was almost entirely successful strikes.
                //
                // A player who struck the ball emits an event; one who was
                // pulled out of the strike — lost possession, cooldown, a
                // handler yanking him elsewhere — emits none. Anything
                // event-carrying is therefore a resolution, not a loss.
                // NB an armed player who leaves on a DIFFERENT event (laying
                // it off instead) is not counted either; that is a decision
                // reversal rather than a dropped strike and belongs to the
                // pass-deferral counters.
                #[cfg(feature = "match-logs")]
                if player.pending_shot_reason.is_some()
                    && !state_change_result.events.has_events()
                {
                    let goal_x = match player.side {
                        Some(crate::r#match::PlayerSide::Left) => context.field_size.width as f32,
                        _ => 0.0,
                    };
                    let dx = player.position.x - goal_x;
                    let dy = player.position.y - context.field_size.height as f32 / 2.0;
                    let d = (dx * dx + dy * dy).sqrt();
                    crate::r#match::player::strategies::players::ops::forward_shot_decision::time_band_diag::QUEUED_SHOT_LOST
                        [crate::r#match::player::strategies::players::ops::forward_shot_decision::time_band_diag::band_for_distance(d)]
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
                player.pending_shot_reason = None;
            }
        }

        if let Some(state) = new_state {
            Self::change_state(player, state);
            // A player who has just committed to leaving the ground does so
            // now, on the tick he decided. The rise lives outside `velocity`
            // and outside everything below — see `MatchPlayer::vertical_speed`
            // for why a metric axis cannot travel inside a vector the
            // horizontal speed limits are going to normalise.
            let apex = Self::leap_apex(player, state, tick_context.positions.ball.position.z);
            #[cfg(feature = "match-logs")]
            if matches!(
                state,
                PlayerState::Defender(DefenderState::Heading)
                    | PlayerState::Midfielder(MidfielderState::Heading)
                    | PlayerState::Forward(ForwardState::Heading)
            ) {
                crate::r#match::engine::ball::ball::flight_diag::FlightDiag::note_header(
                    apex.is_some(),
                );
            }
            if let Some(apex) = apex {
                player.leap(apex);
            }
        } else {
            player.in_state_time += 1;
        }

        if let Some(velocity) = state_change_result.velocity {
            let mut max_speed = if player_position_group == PlayerFieldPositionGroup::Goalkeeper {
                let speed_context = match player.state {
                    PlayerState::Goalkeeper(gk) => Self::goalkeeper_speed_context(gk),
                    // An outfielder standing in for a sent-off keeper keeps
                    // their own speed model — they are not a goalkeeper.
                    _ => GoalkeeperSpeedContext::Positioning,
                };
                player
                    .skills
                    .goalkeeper_max_speed(player.player_attributes.condition, speed_context)
            } else {
                player.max_speed_with_condition_cached()
            };
            // Sprint capability BEFORE the carry / effort multipliers —
            // the acceleration bound below is a property of the athlete,
            // not of how hard they happen to be working this tick.
            let sprint_capability = max_speed;

            // Ball-carrier speed multiplier. Real football: carrying
            // the ball costs ~15-25% of top sprint for an average
            // player — they keep the ball in stride, look up, protect
            // it. Elite carriers (Mbappé/Messi) lose almost nothing.
            //
            // Routes through `movement_speed_with_ball` so dribbling +
            // technique + pace + acceleration + agility + balance all
            // contribute, and so fatigue/late-game effects propagate
            // through `effective_skill`. Mapping per spec:
            //
            //   carry_mult = 0.78 + composite * 0.42
            //
            // Composite floor 0.05 → 0.80 (worst carrier under fatigue);
            // composite 1.00 → 1.20 (elite carrier — no realistic
            // penalty). Capped to existing `[0.75, 1.00]` band so the
            // model stays a CARRY COST: an elite carrier matches their
            // off-ball speed but doesn't go faster than it.
            if tick_context.ball.current_owner == Some(player.id)
                && player_position_group != PlayerFieldPositionGroup::Goalkeeper
            {
                let minute = sc::minute_from_ms(context.total_match_time);
                let composite = sc::movement_speed_with_ball(player, minute);
                let raw = 0.78 + composite * 0.42;
                max_speed *= raw.clamp(0.75, 1.00);
            }

            // Off-ball effort: a player jogs to reposition and only
            // sprints to press / chase / break in behind. Scale the speed
            // cap by the effort the state itself declared this tick (the
            // same `ActivityIntensity` the fatigue model reads), so
            // off-ball movement stops pinning to a full sprint — the
            // condition diagnostic had ~77% of outfield ticks in the top
            // band. This caps the top speed only: a velocity already below
            // the ceiling (a decelerating `Arrive`, a slow walk vector) is
            // untouched. On-ball carriers keep the carry model applied
            // above; goalkeepers keep their context-managed speed.
            if player_position_group != PlayerFieldPositionGroup::Goalkeeper
                && tick_context.ball.current_owner != Some(player.id)
            {
                max_speed *= MovementEffort::speed_fraction(
                    player.last_activity_intensity,
                    player.player_attributes.condition_percentage(),
                );
            }

            // NaN/Inf guard: state velocity functions compose many
            // divisions and normalizations, and any zero-magnitude vector
            // put through `.normalize()` anywhere upstream produces a
            // NaN that propagates into player.velocity → player.position
            // → the recording → the viewer renders nothing. Catch it
            // here at the single integration point so no state has to
            // remember to self-sanitize. Non-finite → zero this tick.
            let finite = velocity.x.is_finite() && velocity.y.is_finite() && velocity.z.is_finite();
            let velocity = if finite { velocity } else { Vector3::zeros() };

            // ACCELERATION LIMIT — a player is a body with momentum, so
            // this tick's velocity is reachable from last tick's only
            // within the limits of what they can push against the ground.
            //
            // The steering behaviours already know this: `Seek`, `Arrive`
            // and `Pursuit` each cap their steering vector at roughly
            // `max_speed * agility * 0.7` before returning. The bound was
            // simply being thrown away afterwards. Almost every caller
            // composes the result — `Arrive{..}.velocity +
            // separation_velocity()` — and the added term is not part of
            // any behaviour's limit, while a handful of states
            // (`RunningInBehind` and friends) skip the behaviours entirely
            // and assign `direction * speed` outright. Either way the
            // final velocity could invert between two consecutive ticks.
            //
            // That is what the twitch actually is. With no momentum, two
            // competing pulls — a state steering a defender back to his
            // mark and personal-space steering him off it — do not settle
            // at a balance point, they alternate: full velocity one way,
            // full velocity back, at 50 Hz. Measured, **88% of all
            // velocity reversals happened with no state change under
            // them** (`dev_match trace`), i.e. inside a single state's own
            // steering, which is exactly this signature.
            //
            // Enforcing the bound HERE, at the one place every state's
            // velocity passes through, means no state can bypass it and
            // no caller can compose its way around it. States that
            // already respected it (a plain `Arrive`, a plain `Pursuit`)
            // are unaffected — their velocity change was inside the limit
            // to begin with.
            let agility_normalized = 0.8 + (player.skills.physical.agility - 1.0) / 19.0;
            let max_accel = sprint_capability * agility_normalized * 0.7;
            let delta = velocity - player.velocity;
            let delta_sq = delta.norm_squared();
            let velocity = if delta_sq > max_accel * max_accel && delta_sq > 0.0 {
                player.velocity + delta * (max_accel / delta_sq.sqrt())
            } else {
                velocity
            };

            let velocity_sq = velocity.norm_squared();
            let max_speed_sq = max_speed * max_speed;

            if velocity_sq > max_speed_sq && velocity_sq > 0.0 {
                let velocity_magnitude = velocity_sq.sqrt();
                player.velocity = velocity * (max_speed / velocity_magnitude);
            } else {
                player.velocity = velocity;
            }
        }

        state_change_result.events
    }

    /// How high a player takes himself, in metres, on entering `state` —
    /// `None` for every state that keeps both feet on the grass.
    ///
    /// Scaled by `jumping`, which is the attribute that means exactly this.
    /// The numbers are the rise of the player's HIPS, so they can be checked
    /// against a human being: a poor leaper gets about a third of a metre off
    /// the ground and a good one three quarters, which puts a keeper's gloves
    /// comfortably over a 2.44 m crossbar.
    ///
    /// # Outfielders used to be in the `None` arm, all of them
    ///
    /// Only the two goalkeeper states below ever left the ground. Every
    /// header in the engine — a centre-back attacking a corner, a striker
    /// meeting a cross, a midfielder winning a second ball — was played by
    /// a man standing perfectly still while the ball passed through him,
    /// because `Heading` resolved as a skill roll and nothing in it ever
    /// touched the vertical axis. The state machine already had the
    /// mechanism; the heading states simply were not listed.
    ///
    /// A header is a TIMED jump, not a maximal one, so the apex asked for
    /// here is the one that brings the player's head to the ball
    /// ([`AerialReach::leap_for`]) rather than the highest he could
    /// manage. A ball he can already reach standing gets no jump at all,
    /// which is correct and is why the helper returns zero rather than a
    /// token hop.
    fn leap_apex(player: &MatchPlayer, state: PlayerState, ball_height: f32) -> Option<f32> {
        let spring = ((player.skills.physical.jumping - 1.0) / 19.0).clamp(0.0, 1.0);
        match state {
            // Every outfield aerial challenge. The apex is measured from
            // the ball, so a low header is a low jump and a ball at the
            // very top of his range is a full one.
            PlayerState::Defender(DefenderState::Heading)
            | PlayerState::Midfielder(MidfielderState::Heading)
            | PlayerState::Forward(ForwardState::Heading) => {
                let apex =
                    AerialReach::header_leap_for(ball_height, player.skills.physical.jumping);
                (apex > 0.0).then_some(apex)
            }
            // A standing leap at a cross or a corner — the one moment a
            // keeper is going straight up.
            PlayerState::Goalkeeper(GoalkeeperState::Jumping) => Some(0.34 + spring * 0.41),
            // A dive is leaving your feet, and that is as true of a save
            // along the floor as of one into the top corner — what the height
            // of the ball changes is how far he still has to climb, not
            // whether he takes off at all. Hence a floor and then a term
            // measured against the crossbar, the highest a shot can be and
            // still be worth diving for.
            //
            // The floor is also load-bearing downstream. `PreparingForSave`
            // shares this state's speed band and reaches the same 12 m/s
            // shuffling along the line, so nothing in a position track can
            // tell the two apart — measured, a speed-and-direction test got
            // 2 of 10 real dives and fired constantly on the shuffle. Leaving
            // the ground is the one thing only a dive does, so it has to
            // actually register.
            PlayerState::Goalkeeper(GoalkeeperState::Diving) => {
                let high = (ball_height / GOAL_HEIGHT).clamp(0.0, 1.0);
                Some(0.16 + spring * 0.14 + high * (0.10 + spring * 0.24))
            }
            _ => None,
        }
    }

    /// Movement band for each goalkeeper state.
    ///
    /// EXHAUSTIVE on purpose — the previous `_ => Casual` catch-all quietly
    /// put `TakeBall` in the idle band, so a keeper who committed to
    /// chasing a loose ball capped at 0.65× base speed: SLOWER than the
    /// 0.75-1.0× `Positioning` band they get while simply standing on
    /// their line. `Punching`, likewise, declared `VeryHigh` exertion
    /// while being speed-capped as if idle. Listing every variant means a
    /// newly added GK state fails the build instead of silently
    /// inheriting the idle cap.
    fn goalkeeper_speed_context(state: GoalkeeperState) -> GoalkeeperSpeedContext {
        match state {
            // Reflex actions — the dive, the punch, the jump, the set.
            GoalkeeperState::Diving
            | GoalkeeperState::PreparingForSave
            | GoalkeeperState::Jumping
            | GoalkeeperState::Punching => GoalkeeperSpeedContext::Explosive,
            // Actively covering ground: rushing off the line, closing on a
            // claim, sprinting to a loose ball.
            GoalkeeperState::Catching | GoalkeeperState::ComingOut | GoalkeeperState::TakeBall => {
                GoalkeeperSpeedContext::Active
            }
            // Reading the game and adjusting the angle.
            GoalkeeperState::Standing
            | GoalkeeperState::ReturningToGoal
            | GoalkeeperState::PickingUpBall => GoalkeeperSpeedContext::Positioning,
            // Ball under control or in hand — minimal movement.
            GoalkeeperState::Walking
            | GoalkeeperState::HoldingBall
            | GoalkeeperState::Distributing
            | GoalkeeperState::Throwing
            | GoalkeeperState::Kicking
            | GoalkeeperState::Passing
            | GoalkeeperState::Clearing => GoalkeeperSpeedContext::Casual,
        }
    }

    fn change_state(player: &mut MatchPlayer, state: PlayerState) {
        // Normal state-machine hand-off: a state handler returned a new
        // state via `StateChangeResult`. Routed through the single
        // transition API so the timer reset and graph audit are uniform.
        player.transition_to(state, TransitionSource::Handler);
    }

    /// States that legitimately carry a `pending_shot_reason` into a
    /// Shoot event. A reason-less transition to any OTHER state clears
    /// the flag (see `process`) so an aborted shot can't arm a free,
    /// helper-bypassing shot on a later Shooting entry.
    fn is_shot_emitting_state(state: PlayerState) -> bool {
        matches!(
            state,
            PlayerState::Forward(
                ForwardState::Shooting | ForwardState::Heading | ForwardState::Finishing
            ) | PlayerState::Midfielder(
                MidfielderState::Shooting
                    | MidfielderState::DistanceShooting
                    | MidfielderState::Heading
            ) | PlayerState::Defender(DefenderState::Shooting | DefenderState::Heading)
        )
    }
}

#[cfg(test)]
mod state_id_tests {
    use super::PlayerState;
    use crate::r#match::defenders::states::DefenderState;
    use crate::r#match::forwarders::states::ForwardState;
    use crate::r#match::goalkeepers::states::state::GoalkeeperState;
    use crate::r#match::midfielders::states::MidfielderState;

    #[test]
    fn role_discriminants_are_strictly_increasing_and_unique() {
        // `ALL` must list every variant in ascending discriminant order.
        // Reorder a variant and its discriminant no longer climbs, failing
        // here before any replay could be misnumbered.
        //
        // Equality with the slot index is NOT required: retiring a state
        // leaves a permanent hole in the id space (the GK band has holes
        // at 1, 15, 16, 20) precisely so the surviving ids never shift
        // under existing recordings.
        fn assert_ascending<S: Copy + Into<u16>>(all: &[S], role: &str) {
            let mut prev: Option<u16> = None;
            for (i, s) in all.iter().enumerate() {
                let id: u16 = (*s).into();
                if let Some(p) = prev {
                    assert!(p < id, "{role}::ALL[{i}] discriminant {id} must exceed {p}");
                }
                prev = Some(id);
            }
        }
        // `as u16` per role — the enums don't share a trait, so map first.
        let gk: Vec<u16> = GoalkeeperState::ALL.iter().map(|s| *s as u16).collect();
        let df: Vec<u16> = DefenderState::ALL.iter().map(|s| *s as u16).collect();
        let mf: Vec<u16> = MidfielderState::ALL.iter().map(|s| *s as u16).collect();
        let fw: Vec<u16> = ForwardState::ALL.iter().map(|s| *s as u16).collect();
        assert_ascending(&gk, "GoalkeeperState");
        assert_ascending(&df, "DefenderState");
        assert_ascending(&mf, "MidfielderState");
        assert_ascending(&fw, "ForwardState");

        // Every band must stay inside the recorder's dense window
        // (`BAND_WIDTH = 25` in `transition::recorder`) or the seen-edge
        // bitmap silently falls back to the Mutex path.
        for (role, ids) in [
            ("GoalkeeperState", &gk),
            ("DefenderState", &df),
            ("MidfielderState", &mf),
            ("ForwardState", &fw),
        ] {
            let max = ids.iter().copied().max().unwrap();
            assert!(max < 25, "{role} discriminant {max} outgrew the id band");
        }
    }

    #[test]
    fn compact_id_snapshot() {
        // Pin the entire id space. If a state is added, removed, reordered
        // or renumbered, this fails — the signal to bump the replay format
        // intentionally rather than by accident.
        //
        // The GK band deliberately has holes (101 / 115 / 116 / 120) where
        // `Resting`, `Tackling`, `Shooting` and `Running` were retired.
        // Ids are never reused, so recordings made before the removal
        // still decode every surviving state to the same name.
        let all = PlayerState::all();
        assert_eq!(all.len(), 1 + 17 + 21 + 20 + 19, "state count changed");
        assert_eq!(GoalkeeperState::ALL.len(), 17);
        assert_eq!(DefenderState::ALL.len(), 21);
        assert_eq!(MidfielderState::ALL.len(), 20);
        assert_eq!(ForwardState::ALL.len(), 19);

        let mut ids: Vec<u16> = all.iter().map(|s| s.compact_id()).collect();
        let unique: std::collections::BTreeSet<u16> = ids.iter().copied().collect();
        assert_eq!(unique.len(), ids.len(), "compact_ids must be unique");

        ids.sort_unstable();
        let mut expected: Vec<u16> = vec![0]; // Injured
        // 17 GK — 101/115/116/120 retired, ids never reused.
        expected.extend([
            100, 102, 103, 104, 105, 106, 107, 108, 109, 110, 111, 112, 113, 114, 117, 118, 119,
        ]);
        expected.extend(200..=220u16); // 21 DEF (220 = Crossing)
        expected.extend(300..=319u16); // 20 MID (319 = Heading)
        expected.extend(400..=418u16); // 19 FWD
        assert_eq!(ids, expected, "compact_id space drifted");

        // Anchor a few named states so an intra-band reorder is caught
        // even though the band-set check above would not notice it.
        assert_eq!(PlayerState::Injured.compact_id(), 0);
        assert_eq!(
            PlayerState::Goalkeeper(GoalkeeperState::Standing).compact_id(),
            100
        );
        // Survivors kept their pre-removal ids across the GK holes.
        assert_eq!(
            PlayerState::Goalkeeper(GoalkeeperState::PreparingForSave).compact_id(),
            117
        );
        assert_eq!(
            PlayerState::Goalkeeper(GoalkeeperState::TakeBall).compact_id(),
            119
        );
        assert_eq!(
            PlayerState::Defender(DefenderState::AttackingCorner).compact_id(),
            219
        );
        assert_eq!(
            PlayerState::Defender(DefenderState::Crossing).compact_id(),
            220
        );
        assert_eq!(
            PlayerState::Midfielder(MidfielderState::Guarding).compact_id(),
            318
        );
        assert_eq!(
            PlayerState::Midfielder(MidfielderState::Heading).compact_id(),
            319
        );
        assert_eq!(
            PlayerState::Forward(ForwardState::CrossReceiving).compact_id(),
            411
        );
    }

    #[test]
    fn no_state_is_reserved_any_more() {
        // Injured used to be reserved — implemented but with no inbound
        // transition. It is now wired to the in-match injury roll, so the
        // reserved set is empty and the reachability guard covers every
        // state in the universe.
        assert!(PlayerState::reserved_states().is_empty());
        assert!(!PlayerState::entry_states().contains(&PlayerState::Injured));
        assert_eq!(PlayerState::Injured.compact_id(), 0);
    }

    #[test]
    fn committed_actions_are_short_physical_ones() {
        use crate::r#match::midfielders::states::MidfielderState;

        // A keeper already committed to a dive, and a header already
        // launched, must be shielded from the loose-ball redirects.
        assert!(PlayerState::Goalkeeper(GoalkeeperState::Diving).is_committed_action());
        assert!(PlayerState::Forward(ForwardState::Heading).is_committed_action());
        assert!(PlayerState::Defender(DefenderState::Tackling).is_committed_action());
        assert!(PlayerState::Midfielder(MidfielderState::Heading).is_committed_action());
        assert!(PlayerState::Injured.is_committed_action());

        // Deliberative / positional states stay interruptible so nobody is
        // frozen out of a chase for a meaningful stretch of play.
        assert!(!PlayerState::Forward(ForwardState::Passing).is_committed_action());
        assert!(!PlayerState::Midfielder(MidfielderState::SwitchingPlay).is_committed_action());
        assert!(!PlayerState::Defender(DefenderState::HoldingLine).is_committed_action());
        assert!(!PlayerState::Goalkeeper(GoalkeeperState::Standing).is_committed_action());
        assert!(!PlayerState::Goalkeeper(GoalkeeperState::TakeBall).is_committed_action());
    }
}
