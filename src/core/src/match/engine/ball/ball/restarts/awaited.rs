use nalgebra::Vector3;

/// Origin of the most recent live pass / restart. Read by the offside
/// resolver: only goal kicks, throw-ins, and corners are exempt from
/// offside; free kicks (direct/indirect) and penalties are not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PassOriginRestart {
    OpenPlay,
    GoalKick,
    Corner,
    ThrowIn,
    /// Generic free kick (legacy / offside fallback). Treated like a
    /// direct free kick by the offside resolver.
    FreeKick,
    /// Foul outside the penalty area, severity Normal+: ball can be shot
    /// at goal directly.
    DirectFreeKick,
    /// Offside or technical infringement: cannot be shot directly into
    /// goal — needs a touch from a second player first.
    IndirectFreeKick,
    /// Foul inside defending penalty area: ball at penalty spot.
    Penalty,
}

impl Default for PassOriginRestart {
    fn default() -> Self {
        PassOriginRestart::OpenPlay
    }
}

impl PassOriginRestart {
    /// Set-piece restarts that exempt the receiver from offside.
    pub fn is_offside_exempt(self) -> bool {
        matches!(
            self,
            PassOriginRestart::GoalKick | PassOriginRestart::Corner | PassOriginRestart::ThrowIn
        )
    }

    /// True for any free-kick-style restart (direct/indirect/legacy).
    /// Penalties and corners are NOT free kicks for routine selection.
    pub fn is_free_kick(self) -> bool {
        matches!(
            self,
            PassOriginRestart::FreeKick
                | PassOriginRestart::DirectFreeKick
                | PassOriginRestart::IndirectFreeKick
        )
    }
}

/// **A dead ball waiting for the man who has to take it.**
///
/// # Why this exists
///
/// Every restart in this engine used to place the ball and TELEPORT the
/// taker onto it (`pending_set_piece_teleport`). For a corner or a goal
/// kick that is a defensible shortcut — there is a real stoppage of thirty
/// seconds there and the sim has nothing to fill it with. For a throw-in
/// it is not, and it was the reported bug: measured over 60 matches, the
/// taker was teleported on **100% of throw-ins, a mean of 21.5 m**, so a
/// player materialised on the touchline roughly every forty seconds of
/// watched football.
///
/// The ball instead lies where it went out, out of play and untouchable,
/// while the taker runs to it. He is a normal player under normal AI while
/// he does it, so the run costs him the stamina it should and the picture
/// is a man jogging to the line rather than one appearing there.
///
/// The teleport survives as the TIMEOUT (see [`Self::PATIENCE_TICKS`]):
/// a restart that never happens would stall the match, and no visual is
/// worth that.
///
/// # The ball is not on the line when this is armed
///
/// It is armed on the tick the ball CROSSES the line, and the ball goes on
/// travelling — out of the pitch, into the run-off, until the boards stop
/// it ([`RunOff`]). So a restart now has three phases rather than one, and
/// the fields below split along them:
///
/// | phase | [`Self::settled`] | [`Self::carrying`] | where the ball is |
/// |---|---|---|---|
/// | running out | false | false | rolling, wherever the physics has it |
/// | waiting | true | false | at rest in the run-off, pinned |
/// | being carried in | true | true | under the taker's feet |
///
/// [`Self::spot`] follows the ball through all three; [`Self::take_from`]
/// is the point the kick or the throw is legally taken from, captured at
/// the crossing and fixed from then on.
#[derive(Debug, Clone, Copy)]
pub struct AwaitedRestart {
    /// Who is taking it.
    pub taker_id: u32,
    /// Where the ball is and where he has to get to.
    ///
    /// Provisional while [`Self::settled`] is false: the ball is still
    /// rolling out and this is rewritten to follow it every tick. Latched
    /// the moment it stops, which is also when [`Self::patience_ticks`] is
    /// recomputed against the distance the taker actually has to cover.
    pub spot: Vector3<f32>,
    /// Where the restart must be TAKEN from, when that is not where the
    /// ball came to rest — so the taker has to bring it there.
    ///
    /// **This used to be the corner's alone**, because the corner was the
    /// only restart whose ball did not die where it was taken from: it is
    /// taken from the ARC while the ball goes out anywhere along the
    /// byline, a measured mean of 220 u — **27.5 m** — away.
    ///
    /// Now every restart has one, because no restart's ball dies where it
    /// is taken from any more. A ball put out of play crosses the line and
    /// keeps going ([`RunOff`]); the legal spot is the point it crossed at,
    /// and the taker has to go out into the run-off, pick it up, and bring
    /// it back to that point. Law 15 has the throw taken "from the point
    /// where it crossed the touchline" and Law 16 puts the goal kick in the
    /// goal area — neither of them is "wherever it finished rolling".
    ///
    /// Consumed on arrival: the taker picks the ball up, `spot` becomes
    /// this point and [`Self::carrying`] goes up, so the second leg reuses
    /// the same wait, the same nudge and the same backstop as the first.
    pub take_from: Option<Vector3<f32>>,
    /// False while the ball is still running out of play.
    ///
    /// The award happens on the tick the ball crosses the line and not one
    /// tick later, because everything that keeps a dead ball dead —
    /// `RestartHold`, [`DeadBall`], the dispatcher's allow-list — keys off
    /// `awaiting_restart` being set. Defer it and the ball spends its
    /// run-out as a live loose ball outside the pitch with `TakeMe` signals
    /// sending the nearest man of either side at it.
    ///
    /// So the restart is armed first and the ball rolls afterwards, and
    /// this is the flag that says which of the two is happening. While it
    /// is false [`Ball::tick_awaited_restart`] integrates the physics
    /// instead of pinning the ball, and the taker's arrival test is held
    /// off — he must not pick up a ball that is still moving.
    pub settled: bool,
    /// True once he has reached the ball and is carrying it to `spot`.
    ///
    /// While it is set the ball rides on him rather than lying on the
    /// spot — he is walking to the flag with it, which is what a corner
    /// taker does — and [`CornerHold`](crate::r#match::player::strategies::
    /// common::states::CornerHold) steers him there, because everything
    /// that normally moves a player toward a ball reads this one as
    /// already reached.
    pub carrying: bool,
    /// Which restart this is, re-applied when he arrives — the origin
    /// decides offside exemption and how the delivery is scored.
    pub origin: PassOriginRestart,
    /// The tick it was awarded on, for the patience bound.
    pub awarded_tick: u64,
    /// How long THIS restart waits, in engine ticks.
    ///
    /// A throw-in's taker is chosen for being near the ball —
    /// `ThrowIn::pick_thrower` weights distance at half the score — so the
    /// walk is short by construction and one constant covers it. A goal
    /// kick's taker is not chosen at all: it is the goalkeeper, and he is
    /// wherever the shot that went out of play left him, which can be the
    /// far post at the end of a dive. `run_for_ball` will not interrupt a
    /// dive either (it is a committed action), so up to 1.8 s of the wait
    /// can be spent before he takes his first step towards it.
    ///
    /// Measured with the flat 5 s bound: 11.2% of goal kicks timed out with
    /// the keeper still **15.1 m** short, and a timeout is the teleport
    /// this whole mechanism exists to avoid. See [`Self::patience_for`].
    pub patience_ticks: u64,
    /// The tick the taker got to the spot with nothing left to do but
    /// wait for his team-mates. `None` until he arrives.
    ///
    /// # Why a corner needs a leg the other restarts do not
    ///
    /// Every other restart is ready when the taker is: a throw-in needs a
    /// thrower and a ball, a goal kick needs a keeper. A CORNER needs five
    /// runners in the box, and they are 60-80 m away when it is awarded.
    ///
    /// Taking the kick the moment the taker was ready is what kept the
    /// walked corner switched off: measured over 60 matches at level 14,
    /// the attacking box at the delivery read **3.5 against a placed
    /// corner's 5.4** and a real 5-7, and the defending box's worst case
    /// fell from 7 to 2. The taker was ready in a couple of seconds after
    /// a short fetch, which is nowhere near long enough for the shape to
    /// arrive — so the kick was struck into an empty box.
    ///
    /// A real taker stands over the ball and waits, and that is all this
    /// is: the arrival test is satisfied, the ball is on the arc, and the
    /// restart holds until [`Self::CORNER_BOX_TARGET`] attackers are in
    /// the penalty area or [`Self::CORNER_SETUP_CEILING`] expires.
    pub settled_tick: Option<u64>,
}

impl AwaitedRestart {
    /// How far inside the line a restart is taken from, in game units.
    /// 6 u = 75 cm.
    ///
    /// It used to be 2 u, justified as "nothing a viewer could see" —
    /// which was the right test when the BALL was written onto this point
    /// on the tick it went out. The ball is not written anywhere any more
    /// ([`RunOff`]); this is where the taker brings it BACK to, and the
    /// binding constraint is a different one.
    ///
    /// ⚠ **It has to clear [`SteeringBehavior::Arrive`]'s 3 u deadzone.**
    /// The carrier is steered at this point and stops braking 3 u short of
    /// it, in whatever direction he approached from — which for a man
    /// walking in out of the run-off is from OUTSIDE. At 2 u he came to
    /// rest around a unit the wrong side of the line, with the ball at his
    /// feet, and then: the arrival gate below refuses a restart taken off
    /// the pitch, `Arrive` has already stopped pushing him, and the pair
    /// deadlock until the patience bound teleports the ball. At 6 u he
    /// stops between 3 and 9 u inside and the ball is comfortably in play.
    ///
    /// Still legal on both counts. Law 16 puts a goal kick anywhere in the
    /// goal area, which runs 44 u deep; Law 15's throw-in is taken at the
    /// point the ball crossed, and 75 cm of it is inside the tolerance
    /// every referee gives.
    ///
    /// [`SteeringBehavior::Arrive`]: crate::r#match::SteeringBehavior::Arrive
    pub const SPOT_INSET: f32 = 6.0;

    /// Attackers in the penalty area a corner taker waits for before he
    /// puts his foot through it. Excludes him and the keeper.
    ///
    /// Real deliveries go in with 5-7 attacking bodies in the box, and the
    /// placed corner this replaces measured 5.4. Four is deliberately
    /// under that: it is the number the taker *waits* for, and the last
    /// runner or two arrive during the flight, which is also what happens
    /// on a real corner. Asking for the full five made the ceiling do the
    /// work instead of the condition.
    pub const CORNER_BOX_TARGET: usize = 4;

    /// Longest a corner may be held on the arc waiting for the box, in
    /// engine ticks. 6 s.
    ///
    /// Real corners take 20-30 s from award to delivery and this engine
    /// has no stoppage clock, so this is not a realism bound — it is a
    /// backstop, and it does not bind: measured over 60 matches at level
    /// 14 the box fills in a **mean of 1.19 s and 0% of corners reach
    /// this ceiling**.
    ///
    /// ⚠ It was not always so, and the difference is diagnostic. While
    /// `is_team_attacking_corner` answered false during the set-up (see
    /// its docs) half of all corners hit the ceiling and raising it from
    /// 6 s to 10 s to 20 s moved the box occupancy by 0.1 — which is what
    /// said the constraint was never time. A ceiling that starts binding
    /// again means somebody has stopped arriving, not that it is too
    /// tight; read `set-up wait` in the corner census before touching it.
    pub const CORNER_SETUP_CEILING: u64 = 600;

    /// How far inside the line the carrier has to be before the restart is
    /// handed to him, in game units. 2 u = 25 cm.
    ///
    /// [`Self::REACH`] is 1.5 m and measured from the spot, so a man
    /// walking in from the run-off satisfies it while still standing on
    /// the line — and the ball is at his feet, so the throw or the kick is
    /// then taken from `x = 0.0`, which every out-of-play resolver reads
    /// as out. Measured: he stopped at 0.1 u and the ball at 0.0.
    ///
    /// Chosen against [`Self::SPOT_INSET`] and `Arrive`'s deadzone, and it
    /// has to stay under both: he is steered to within 3 u of a point 6 u
    /// inside, so anything up to 3 u is reachable and anything above it
    /// deadlocks.
    pub const IN_PLAY_CLEARANCE: f32 = 2.0;

    /// Close enough to pick the ball up, in game units. 12u = 1.5 m.
    ///
    /// Deliberately generous: the taker is steered by the ordinary chase
    /// behaviour, which slows and settles rather than landing on a point,
    /// and a tolerance tighter than his own settling distance leaves him
    /// jogging beside the ball until the patience bound fires. At 8u a
    /// quarter of all throw-ins still timed out and were teleported.
    pub const REACH: f32 = 12.0;

    /// How long the ball is allowed to wait, in engine ticks. 500 = 5 s —
    /// longer than the 21.5 m the taker used to be teleported takes to
    /// run, and short enough that a taker who gets stuck (blocked, or
    /// pulled into another state) cannot hold the match up.
    pub const PATIENCE_TICKS: u64 = 500;

    /// The wait a `walk` of this length actually needs, in engine ticks.
    ///
    /// [`Self::PATIENCE_TICKS`] as a floor, plus the ground at 1.6 m/s —
    /// half the 3.6 m/s the census measures a fetch at, so the bound is
    /// never the thing that decides the outcome for a taker who is
    /// genuinely on his way. Capped at 12 s, which is shorter than a real
    /// goal kick takes and long enough that nothing can stall behind it.
    pub fn patience_for(walk: f32) -> u64 {
        Self::patience_within(walk, Self::CEILING)
    }

    /// Longest an ordinary restart may wait. 12 s — shorter than a real
    /// goal kick takes and long enough that nothing can stall behind it.
    ///
    /// `pub` for the tests, which have to run past it: the wait is no
    /// longer a constant they can predict, because the ball spends the
    /// first fraction of a second running out of play and the bound is
    /// re-derived when it stops. See [`RunOff`].
    pub const CEILING: u64 = 1200;

    /// …and a CORNER's, which is a different job.
    ///
    /// Every other restart is taken by a man chosen for being near the
    /// ball. A corner's taker is chosen for `corners` and `crossing` and
    /// can be anywhere on the pitch — measured, a mean 28.8 m from where
    /// the ball went out, and the tail runs past forty — and then he has
    /// the ball to carry to the flag on top of that.
    ///
    /// ⚠ At the ordinary 12 s ceiling **45% of corners timed out and took
    /// the backstop teleport**, which is the artefact the walk exists to
    /// remove. The census that says so is the one that matters: the takers
    /// who timed out were in `TakeBall` — *coming* — with a mean **34.7 m
    /// still to go**, not standing still the way the goal kick's were
    /// ([[goal-kick-restart-teleport]]'s histogram). A man who is on his
    /// way and runs out of clock needs more clock.
    ///
    /// 30 s is what a real corner takes, and the wait is not dead time in
    /// the sense that matters — it is the stoppage both sides spend
    /// walking into the shape, which is the whole reason `CornerShape`
    /// exists.
    const CORNER_CEILING: u64 = 3000;

    /// The wait for a corner leg — the fetch or the carry.
    pub fn corner_patience_for(walk: f32) -> u64 {
        Self::patience_within(walk, Self::CORNER_CEILING)
    }

    fn patience_within(walk: f32, ceiling: u64) -> u64 {
        /// Engine ticks per game unit at 1.6 m/s.
        const TICKS_PER_UNIT: f32 = 8.0;
        // Clamped BEFORE the cast. `f32 as u64` saturates rather than
        // wrapping, so a non-finite or absurd walk produced `u64::MAX` and
        // the addition below overflowed — a panic reachable from any caller
        // that hands this a garbage distance.
        let ground = (walk.max(0.0) * TICKS_PER_UNIT).min(ceiling as f32) as u64;
        (Self::PATIENCE_TICKS + ground).min(ceiling)
    }
}
