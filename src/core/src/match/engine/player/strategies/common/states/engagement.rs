use crate::r#match::engine::context::PenaltyArea;
use crate::r#match::player::events::FoulSeverity;
use crate::r#match::player::strategies::players::ops::skill_composites as sc;
use crate::r#match::{
    DefensiveDuty, MatchContext, MatchPlayer, PlayerSide, StateProcessingContext, SteeringBehavior,
};
use nalgebra::Vector3;

/// How far behind a challenging player a team-mate can be and still count
/// as cover (~30 m). Wide, because the question is "will somebody deal
/// with it if the carrier goes past me", not "is he standing next to me".
const COVER_SCAN: f32 = 240.0;

/// Where a player's engagement with the ball carrier starts, where the
/// challenge actually goes in, and where he lets the carrier go.
///
/// # Why this exists
///
/// Every role had its own pair of numbers for the same decision, and in
/// every case the two overlapped. Defenders committed to `Tackling` inside
/// 25u and broke off outside 10u; midfielders committed inside **50u**;
/// forwards inside 30u. A carrier anywhere in the overlap satisfied both
/// the commit and the break-off condition at once, so the player
/// alternated between the two states on consecutive AI ticks. Measured
/// per match: `Defender: Tackling` was entered and left within a single
/// tick **100% of the time**, and the three role pairs together ran ~36k
/// round trips (`dev_match trace`). Across a 30-match batch the `Tackling`
/// states were entered 2.05 MILLION times to produce 14k actual
/// challenges — 142 entries per attempt.
///
/// # The rule
///
/// Three ordered distances, shared by every role, because none of this is
/// role-specific — it is how close two people have to be for one to
/// challenge the other. `COMMIT` is where the player decides to engage;
/// inside `CONTACT` the challenge is made; between them he simply keeps
/// closing, which is what the `Tackling` states' own Pursuit velocity has
/// always done. Only once the carrier is past `DISENGAGE` — outside the
/// distance that triggered the commitment in the first place — does he
/// break off. The ordering `CONTACT < COMMIT < DISENGAGE` is what makes
/// the hand-off one-way instead of a two-cycle.
pub struct TackleEngagement;

impl TackleEngagement {
    /// Contact range — the challenge itself (~1.25 m). Unchanged from the
    /// defenders' original threshold, so how and when a tackle is actually
    /// attempted is untouched.
    pub const CONTACT: f32 = 10.0;
    /// Close enough to commit to engaging (~2 m). A player further out
    /// than this presses the space rather than diving in.
    ///
    /// The roles previously used 25u, 30u and 50u — the last of these
    /// (~6 m) a distance at which nobody can challenge anybody. Tightening
    /// to contact range itself was tried and is worse: it costs goals
    /// (2.03 → 1.40 per match over 30) without touching the tackle count,
    /// which turns out to be insensitive to this distance — successful
    /// tackles held at 92.8/team across 25u, 16u and 10u commits. The
    /// engine's tackle VOLUME lives in the attempt model, not in the
    /// engagement geometry, and is left alone here.
    pub const COMMIT: f32 = 16.0;
    /// The carrier has got away (~3 m) — break off. Strictly outside
    /// `COMMIT`; that gap is what makes the hand-off one-way, and it is
    /// where the containment run lives: a player who has committed keeps
    /// closing through this band instead of being handed back out of the
    /// state on the tick after he entered it.
    pub const DISENGAGE: f32 = 24.0;

    /// Should this player commit to a challenge on a carrier at `distance`?
    ///
    /// Distance is the smallest part of it. The other two conditions are
    /// the ones every `Tackling` state checks on ENTRY and hands the
    /// player straight back out on — so `Pressing` has to check them
    /// before sending him, or the pair is a two-cycle by construction:
    ///
    ///   * the per-player tackle cooldown. A player who has just lunged
    ///     cannot lunge again, so `Tackling` returns him immediately.
    ///     Containing on his feet is also the right football answer.
    ///   * the closest-teammate duel gate. Only the designated engager
    ///     challenges; everyone else covers. `Forward: Pressing <->
    ///     Forward: Tackling` ran ~6,100 round trips a match purely on
    ///     this one — the forward was committed by distance, refused by
    ///     the duel gate, and sent back to be committed again.
    ///
    /// Neither check changes how many tackles are ATTEMPTED — both were
    /// already enforced inside `Tackling` before any attempt is rolled.
    /// They only stop the state being entered to be left again.
    /// …and the duel gate has to accept the TEAM PLAN's answer, not only
    /// the chase election's.
    ///
    /// `is_best_player_to_chase_ball` scores every candidate with a
    /// `position_factor` — Forward 1.2, Midfielder 1.1, **Defender 0.9**.
    /// That is the right bias for a loose ball nobody owns (a forward does
    /// gamble on those), and the wrong one entirely for "who challenges
    /// the man carrying the ball at our box", where the defender is by
    /// definition the man. Measured, it inverted the whole ladder:
    /// **0.47 tackles per defender per match against 3.01 per forward**,
    /// with a real distribution of ~1.6 / ~1.0 the other way up, and
    /// `Defender: Tackling` below 0.25% of all ticks.
    ///
    /// `DefensivePlan` already nominates exactly one engager per side,
    /// by distance and with no positional thumb on the scale, and every
    /// other part of the defensive model treats that nomination as
    /// authoritative. Accepting it here is what lets the nominated man
    /// actually go in.
    pub fn should_commit(ctx: &StateProcessingContext, distance: f32) -> bool {
        distance < Self::COMMIT && ctx.player.can_attempt_tackle() && Self::may_engage_carrier(ctx)
    }

    /// Who is allowed to challenge the man on the ball.
    ///
    /// ⚠ THE TWO DOORS WERE OPEN AT ONCE, AND THAT IS WHY THE LADDER
    /// STAYED UPSIDE DOWN.
    ///
    /// This read `is_best_player_to_chase_ball() || is_nominated_presser()`
    /// — the plan's nomination was ADDED to the chase election rather than
    /// preferred over it, so the election's `position_factor` (Forward
    /// 1.2, Midfielder 1.1, **Defender 0.9**) still handed the duel to
    /// whichever forward happened to be in the area. The note above says
    /// exactly why that factor is wrong for this question and then leaves
    /// it live.
    ///
    /// Measured over 120 fixtures with both doors open: **2.74 tackles per
    /// forward per match against a real ~0.8**, with defenders on 1.37
    /// against ~1.6 — a front line winning the ball more often than the
    /// back four, in an engine that already documents that as the defect.
    ///
    /// The election is the right answer for a LOOSE ball — a forward does
    /// gamble on those, and the plan nominates nobody for a ball at rest
    /// by design (see `DutyAssigner::assign`). So it stays, as the
    /// fallback for exactly the case the plan declines to answer: when
    /// nobody has been nominated, whoever can get there first goes.
    /// When somebody HAS been nominated, he is the man, and everybody
    /// else covers.
    /// …AND IN YOUR OWN BOX, TWO GO.
    ///
    /// One engager is right in open play — it is what stopped four
    /// defenders converging on one carrier and rolling four independent
    /// foul chances. It is wrong inside your own penalty area, and the
    /// engine already knew that: `is_box_emergency_for_me` elects the
    /// **two** closest defenders to a carrier in our area, and five
    /// defender states (`Standing`, `Marking`, `Covering`, `Guarding`,
    /// `HoldingLine`) send them to `Tackling` on the strength of it.
    /// Every one of those second men was then refused here on the entry
    /// tick and handed back to `Pressing`, so the emergency was computed
    /// in five places and discarded in one.
    ///
    /// Measured, that is the user's report exactly: with a carrier in
    /// our own area, 20% of all defender-ticks inside `COMMIT` fail on
    /// this gate, and a box carry draws 20 commit decisions a match
    /// between all of them.
    ///
    /// The second man is the plan's `Cover` — the duty is already
    /// defined as "second body, goal-side of the presser — the one who
    /// deals with it when the presser is beaten", it is already
    /// exclusive, and `DutyAssigner` already picks it markers-only and
    /// within `COVER_REACH`. So the licence stays a licence: two named
    /// players, not everybody who is near.
    ///
    /// Gated on [`PenaltyRisk::applies`] — the referee's test, the ball
    /// inside the area we are defending — rather than on the defender's
    /// own position, so the second man is licensed by where the BALL is
    /// and not by his having drifted into his own box.
    pub fn may_engage_carrier(ctx: &StateProcessingContext) -> bool {
        if Self::is_nominated_presser(ctx) {
            return true;
        }
        if PenaltyRisk::applies(ctx) && matches!(ctx.team().my_duty(), DefensiveDuty::Cover) {
            return true;
        }
        ctx.team().defensive_plan().presser().is_none() && ctx.team().is_best_player_to_chase_ball()
    }

    /// True when the team plan has made this player the engager.
    pub fn is_nominated_presser(ctx: &StateProcessingContext) -> bool {
        matches!(ctx.team().my_duty(), DefensiveDuty::Press)
    }
}

/// Where a marker goes to his man, and where he gives him up.
///
/// # Why this exists
///
/// The same defect [`TackleEngagement`] was built to remove, in the
/// marking states, and left in place for a documented reason that has
/// since stopped being true.
///
/// `DefenderRunningState` broke shape to go and mark a man at 150u
/// (18.75 m); `DefenderMarkingState` gave him up at
/// `ideal_marking_distance * 2` — **14-28u, i.e. 1.75-3.5 m**. Every man
/// between the two satisfied both conditions at once, so the pair ran as
/// a two-tick oscillator: measured with `dev_match trace`, ~191,000
/// `Running <-> Marking` loops across three matches, an order of
/// magnitude above anything else in the engine. The midfield had the same
/// break with the numbers closer together (enter at 150u, give up at
/// `MAX_GUARD_RANGE` 100u).
///
/// **The old release figure was a STEERING distance being used as a state
/// boundary.** `ideal_marking_distance` is how close a marker wants to
/// stand — 1.75-3.5 m, correct — and has nothing to say about when he
/// abandons the man. Using it as the exit meant a defender who was doing
/// his job perfectly well from four metres was handed out of the state on
/// the next tick.
///
/// # Why it is safe to close now, when it was not before
///
/// `defensive_shape_ownership` records this pair being closed
/// hysteretically once before and REVERTED: the loop went away but the
/// match got worse on every axis (goals 4.4 → 5.2), because pinning a
/// defender in `Marking` let him follow his man clean out of the line.
/// Two things have changed since:
///
///   * `DefensiveLine::hold_shape` is now applied inside `Marking`'s own
///     steering, so the target he is pinned onto is clamped to his zone.
///     That is exactly the "until `Marking`'s own steering is shape-safe"
///     condition the revert note left open.
///   * `Marking` now takes the man the **team plan** assigned rather than
///     re-picking one locally every tick. The earlier experiment pinned
///     defenders onto a locally-chosen argmax that swapped between
///     candidates on sub-metre movement — pinning made that worse by
///     construction. An assigned man is exclusive, and was chosen because
///     this defender was the nearest free body to him.
///
/// The ordering `ENGAGE < RELEASE` is the whole point, and it is the same
/// invariant `TackleEngagement` documents: a state whose give-up
/// condition overlaps its own entry condition is a two-cycle.
pub struct MarkEngagement;

impl MarkEngagement {
    /// Close enough that a marker breaks shape to go to his man (~19 m).
    /// Unchanged — this is the figure both `Running` states already used.
    pub const ENGAGE: f32 = 150.0;
    /// …and far enough that he has genuinely lost him (~25 m). Strictly
    /// outside `ENGAGE`, so the hand-off is one-way.
    pub const RELEASE: f32 = 200.0;
}

/// **The ball is in the goalkeeper's hands — get out of his area.**
///
/// # Why this exists
///
/// Nothing in the engine moved a player because the opposing keeper had
/// picked the ball up. The pressing states stood down (see
/// `BallOperationsImpl::carrier_id`), which stopped them running AT him,
/// but standing down is not the same as backing off: whoever was in the
/// box when he claimed it simply stayed there for the whole hold.
/// Measured over 12 matches, on the ticks the ball was in a keeper's
/// gloves there were **3.2 opponents inside his penalty area on average,
/// at least one on 97% of those ticks, and one within 5u — 62 cm — of him
/// on 22%**. On screen that is a forward standing over a keeper who is
/// holding the ball, which reads as trying to take it off him whether or
/// not the ownership layer would ever allow it.
///
/// It is also the football. A keeper in possession of the ball with his
/// hands cannot be challenged — Law 12 makes even attempting to kick it
/// while he is releasing it an indirect free kick — so there is nothing
/// for an attacker to win by staying, and every attacking side in the
/// world turns and jogs out to press the distribution instead.
///
/// # Where it is applied
///
/// At the single point every state's movement converges on
/// (`StateProcessor::process_inner`), and DELIBERATELY ahead of
/// `ShapeDiscipline`: the attacking plan's box slots are inside the very
/// area he has to leave, so shaping him would pull him straight back in.
/// It is a velocity override rather than a state, because it must reach
/// the states that stand still as well as the ones that run — a striker
/// idling on the six-yard line is the exact case being fixed.
pub struct KeeperReleaseSpace;

impl KeeperReleaseSpace {
    /// How far beyond the edge of the area he keeps going, so he is not
    /// hovering on the line waiting to step back in. 16u = 2 m.
    const CLEAR_MARGIN: f32 = 16.0;
    /// Share of `Arrive`'s own output to use. `Arrive` already caps itself
    /// at roughly `max_speed * agility * 0.7` and tapers to nothing over
    /// the last `slowing_distance`, so this lands at a jog and needs no
    /// separate "don't overshoot" term.
    ///
    /// ⚠ If the retreat looks too slow, RAISE THE EFFORT FLOOR, NOT THIS.
    /// The realised speed is `min(this * Arrive, effort * max_speed)` and
    /// the second term is the binding one — pushing this from 0.62 to 1.0
    /// moved the measured box occupancy by 0.05 of a player, because the
    /// request was already above the ceiling. (What was actually wrong at
    /// the time was the side lookup below, and it is worth knowing that a
    /// tuning knob can be turned twice for nothing while a bug is hiding
    /// underneath it.)
    const PACE: f32 = 0.85;
    /// Nobody stands this close to a keeper who is holding the ball. 26u
    /// ≈ 3.25 m — outside his spread, which is the distance the Laws are
    /// really about.
    const PERSONAL_SPACE: f32 = 26.0;
    /// Effort floor for the walk out at the edge of the area, on
    /// `MovementEffort::speed_fraction`'s scale — between `Low` (0.25, a
    /// stroll) and `Moderate` (0.52, a jog into space). He has a couple of
    /// metres left at this point.
    const JOG_EFFORT: f32 = 0.40;
    /// …rising to this deep inside it, and for backing off a keeper you
    /// are standing on top of. A man on the six-yard line has 16 m to
    /// cover and about three and a half seconds of hold to do it in, so a
    /// stroll never gets him out; `Arrive` tapers him back down to a walk
    /// as he reaches the line, which is the shape a real jog out of the
    /// box has.
    const BACK_OFF_EFFORT: f32 = 0.65;

    /// The velocity that takes this player out of the area of a keeper
    /// holding the ball and the effort floor to serve it at, or `None`
    /// when there is nothing to do — which is almost always.
    pub fn retreat(ctx: &StateProcessingContext) -> Option<(Vector3<f32>, f32)> {
        if !ctx.tick_context.ball.held_in_hands {
            return None;
        }
        // Which end the holder defends, read off the LIVE position store
        // rather than `context.players`. That collection is a pre-kickoff
        // snapshot, and `side` on it is an `Option` that is not reliably
        // populated — taking it as `None` resolves `penalty_area(false)`,
        // the RIGHT-hand box, for everybody. Which is exactly what the
        // instrumentation showed: the retreat fired on 3,383 player-ticks
        // a match against the ~7,300 opponent-in-area ticks it should have,
        // i.e. one team's keeper was protected and the other's was not.
        let holder_id = ctx.tick_context.ball.current_owner?;
        let holder_side = ctx
            .tick_context
            .positions
            .players
            .as_slice()
            .iter()
            .find(|e| e.player_id == holder_id)
            .map(|e| e.side)?;
        if Some(holder_side) == ctx.player.side {
            return None;
        }
        let area = ctx.context.penalty_area(holder_side == PlayerSide::Left);
        let me = ctx.player.position;
        if !(area.min.x..=area.max.x).contains(&me.x) || !(area.min.y..=area.max.y).contains(&me.y)
        {
            return None;
        }

        // Out along the goal-to-goal axis, up the pitch. Leaving sideways
        // would put him level with the goal line by the corner flag, which
        // is not where anybody goes — the press re-forms in front of the
        // area, facing the keeper.
        let out_x = if holder_side == PlayerSide::Left {
            area.max.x + Self::CLEAR_MARGIN
        } else {
            area.min.x - Self::CLEAR_MARGIN
        };
        let mut velocity = SteeringBehavior::Arrive {
            target: Vector3::new(out_x, me.y, 0.0),
            slowing_distance: 24.0,
        }
        .calculate(ctx.player)
        .velocity
            * Self::PACE;
        // How deep he is, as a share of the area's depth. The man on the
        // goal line has the furthest to go and the least time to do it.
        let depth = ((out_x - me.x).abs() / (area.max.x - area.min.x).max(1.0)).clamp(0.0, 1.0);
        let mut effort = Self::JOG_EFFORT + (Self::BACK_OFF_EFFORT - Self::JOG_EFFORT) * depth;

        // Crossing the box takes several seconds and a hold lasts three
        // and a half, so the walk out on its own cannot answer the worst
        // version of this: a forward standing ON the keeper, which was 22%
        // of all hand-ticks. That one is two metres of movement, and it is
        // the one an official would actually intervene over — so it gets
        // its own direct term and its own urgency.
        let gap = ctx.player.position - ctx.tick_context.positions.ball.position;
        let dist = gap.magnitude();
        if dist < Self::PERSONAL_SPACE {
            let away = gap
                .try_normalize(1.0e-3)
                .unwrap_or_else(|| Vector3::new(-holder_side.forward_dir_x(), 0.0, 0.0));
            let urgency = 1.0 - dist / Self::PERSONAL_SPACE;
            velocity += away * ctx.player.max_speed_with_condition_cached() * urgency * 0.6;
            effort = Self::BACK_OFF_EFFORT;
        }
        #[cfg(feature = "match-logs")]
        {
            use crate::r#match::engine::ball::ball::ownership::reception_diag as d;
            d::keeper_ball_note(20);
            d::keeper_ball_add(21, (velocity.magnitude() * 1000.0) as u64);
        }
        Some((velocity, effort))
    }
}

/// The rule that keeps a race for a loose ball a RACE.
///
/// # Why this exists
///
/// All four `TakeBall` states add a plain repulsion from every player
/// within 25u — team-mates AND opponents — to their pursuit of the ball,
/// at up to 0.4 of top speed. So an opponent standing between the chaser
/// and the ball produces a force pointing away from that opponent, which
/// is to say **away from the ball**, and the chaser visibly hangs off
/// while a rival who started FURTHER away arrives first. Reported from
/// the viewer exactly that way: "defenders with Take Ball not running to
/// the ball, and the opponent is first even though he was further".
///
/// It applies for the whole approach, too — the states' `separation_factor`
/// is 1.0 at every distance beyond 10u and only ramps down inside that,
/// i.e. only once the chaser is already within claim range.
///
/// Real players do not give way to the man they are racing; they run the
/// same line and fight for it shoulder to shoulder. Separation still has
/// a job here — stopping four players stacking on one point — and that
/// job is entirely lateral, so keeping only the part of the force that
/// does not oppose the chase preserves it while making it impossible for
/// anybody to be pushed off the ball.
pub struct LooseBallChase;

impl LooseBallChase {
    /// Ball height below which the aim point is the ball itself.
    const GROUND_H: f32 = 1.5;
    /// Ball height above which the aim point is where it will land.
    const AERIAL_H: f32 = 3.0;

    /// Where to run for a loose ball, and the velocity to run there with.
    ///
    /// Returns `(target, velocity)` — the target is also what the caller
    /// measures its separation ramp against.
    ///
    /// # Why the aim point is blended
    ///
    /// This used to be `if ball.z > 2.3 { landing } else { ball_pos }`, with
    /// the steering switching between `Arrive` and `Pursuit` on the same
    /// test. A bouncing ball crosses 2.3 repeatedly, and the two targets can
    /// be tens of units apart in DIFFERENT directions — so the chaser's
    /// velocity **inverted on every crossing**. `dev_match trace` measured
    /// 6.6-7.8 velocity reversals per second with the player never leaving
    /// the state: a chaser visibly shivering next to a loose ball instead of
    /// collecting it. Crossing both the target and the behaviour smoothly
    /// across a height band means there is no height at which either can
    /// jump. (Smoothstep, so the aim point has zero gradient at both ends
    /// and no corner where it starts or finishes moving.)
    ///
    /// `Arrive` brakes into a landing spot; `Pursuit` leads a rolling ball.
    /// Seek alone would chase a moving ball's *current* position and always
    /// lag behind — fatal for a ground pass rolling through the chaser.
    pub fn aim(
        player: &MatchPlayer,
        ball_pos: Vector3<f32>,
        ball_vel: Vector3<f32>,
        landing: Vector3<f32>,
        slowing_distance: f32,
    ) -> (Vector3<f32>, Vector3<f32>) {
        let t = ((ball_pos.z - Self::GROUND_H) / (Self::AERIAL_H - Self::GROUND_H)).clamp(0.0, 1.0);
        let aerial = t * t * (3.0 - 2.0 * t);
        let target = ball_pos + (landing - ball_pos) * aerial;

        let brake = || {
            SteeringBehavior::Arrive {
                target,
                slowing_distance,
            }
            .calculate(player)
            .velocity
        };
        let lead = || {
            SteeringBehavior::Pursuit {
                target,
                target_velocity: ball_vel,
            }
            .calculate(player)
            .velocity
        };

        let velocity = if aerial >= 1.0 {
            brake()
        } else if aerial <= 0.0 {
            lead()
        } else {
            lead() * (1.0 - aerial) + brake() * aerial
        };

        (target, velocity)
    }

    /// Remove the component of `separation` that points against the run to
    /// the ball, leaving the lateral part untouched.
    ///
    /// A component that happens to push the chaser TOWARD the ball is left
    /// alone: it costs nothing, and stripping it as well would be its own
    /// arbitrary rule rather than a consequence of anything physical.
    pub fn keep_non_opposing(separation: Vector3<f32>, to_ball: Vector3<f32>) -> Vector3<f32> {
        let Some(chase_dir) = to_ball.try_normalize(1e-3) else {
            return separation;
        };
        let opposing = separation.dot(&chase_dir);
        if opposing < 0.0 {
            separation - chase_dir * opposing
        } else {
            separation
        }
    }
}

/// Would a foul by this player, right now, be a penalty?
///
/// # Why this is one function
///
/// Three places need this answer and two of them used to compute it
/// differently from the one that matters.
/// [`FoulResolver::award_restart_for_foul`] — the code that actually
/// awards the penalty — tests **the BALL's position** against the
/// fouler's own penalty area. Both restraint models tested **the
/// FOULER's position** instead.
///
/// Those differ constantly, and the gap is not symmetric: a defender
/// standing on the edge of his box challenging for a ball inside it gets
/// no restraint and concedes a penalty. Measured, that was the whole
/// story — restraining tackles on the defender's own position cut tackles
/// 22.4 → 18.3 per team and moved the penalty rate by nothing, and the
/// foul-source census then showed **41 of 43 box fouls were being emitted
/// by midfielders**, who are exactly the players standing at the edge of
/// the area rather than inside it.
///
/// So the restraint asks the referee's question, in the referee's terms.
pub struct PenaltyRisk;

impl PenaltyRisk {
    /// True when a foul by this player would be given as a penalty —
    /// the ball is inside the area his own team is defending. This is the
    /// referee's exact test, mirrored from `award_restart_for_foul`.
    pub fn applies(ctx: &StateProcessingContext) -> bool {
        Self::own_box(ctx).contains(&ctx.tick_context.positions.ball.position)
    }

    /// True when the player should be defending carefully because of
    /// where he is standing, whether or not the ball is with him yet.
    ///
    /// Both halves of this are needed and they are not the same
    /// population. Swapping the models from the standing test to the ball
    /// test alone fixed penalties (1.03 → 0.10) and sent fouls from 14.3
    /// to 23.5 per team, because most defensive engagements inside the
    /// area happen while the ball is still on its way in — a defender
    /// picking up a runner at the back post is in the box, the ball is
    /// not, and the old test was quietly suppressing every one of those.
    ///
    /// A defender is careful in his own box because of the consequences
    /// of being *there*, and doubly so once the ball arrives. Taking the
    /// union keeps the first and lets [`applies`](Self::applies) carry
    /// the second.
    pub fn in_own_box(ctx: &StateProcessingContext) -> bool {
        Self::own_box(ctx).contains(&ctx.player.position) || Self::applies(ctx)
    }

    fn own_box(ctx: &StateProcessingContext) -> PenaltyArea {
        ctx.context
            .penalty_area(ctx.player.side == Some(PlayerSide::Left))
    }
}

/// When a defender in contact range actually commits to a challenge,
/// rather than staying on his feet and containing.
///
/// # Why the volume was wrong
///
/// The tackling state attempted a challenge on every tick it was in
/// contact range and off cooldown. That produced **11.9 tackles per
/// defender per match against a real ~1.6** (84.9 per team against 18),
/// with 441k state entries collapsing to 9.5k attempts and 4.2k
/// successes. `TackleEngagement`'s own notes had already established that
/// the volume does not live in the engagement GEOMETRY — successful
/// tackles held at 92.8/team across 25u, 16u and 10u commit distances —
/// so it has to live here, in whether the attempt is made at all.
///
/// # The football
///
/// A defender does not lunge because he can reach. Most of a duel is
/// spent containing: standing the carrier up, showing him away from goal,
/// waiting. He commits when something gives him the moment — the carrier
/// takes a touch too far, commits his weight, or runs out of pitch — and
/// he commits more readily when there is somebody behind him to clean up
/// if he misses.
///
/// Every term below is continuous, so the tackle rate emerges from the
/// situation and the defender's temperament instead of a threshold.
pub struct TackleDecision;

impl TackleDecision {
    /// Per-DECISION commitment chance for an ordinary contain, before the
    /// situational terms. Containing is the default action in a duel, so
    /// a typical decision lands near 0.15-0.20 once the multipliers are
    /// applied — a defender who stands his man up for a second or two
    /// usually does not dive in.
    ///
    /// ⚠ RE-ANCHORED 0.16 → 0.098, AND NOT BECAUSE THE DECISION CHANGED.
    ///
    /// This is a rate per second OF CONTACT, so its calibration depends
    /// entirely on how much contact there is — and until the chase-speed
    /// inversion was fixed (see `MovementEffort::carrier_ceiling`) there
    /// was very little, because the man on the ball had a higher speed
    /// ceiling than the man chasing him and defenders spent their duels
    /// trailing at three metres rather than standing anybody up.
    ///
    /// Measured across that fix, same 120 fixtures: the carrier's nearest
    /// opponent inside 2 m went from **37% of ticks to 54%**, and mean
    /// engagement distance 3.5 m → 2.6 m. The same 0.16 then produced
    /// 29.5 successful tackles per team per match against a real ~18, and
    /// 22.4 fouls against ~12 — a foul being a failed challenge, the two
    /// move together and both are a function of this number.
    ///
    /// ⚠ RE-ANCHORED AGAIN, 0.098 → 0.073, AND AGAIN NOT BECAUSE THE
    /// DECISION CHANGED — because the DIVISOR did.
    ///
    /// [`Self::is_decision_tick`] now counts time in CONTACT rather than
    /// time in the `Tackling` state, and the interval is the one real
    /// second the doc always claimed rather than two. Both corrections
    /// pull the same way: a defender who arrives on the carrier is asked
    /// on the tick he arrives, and asked again every second he stands him
    /// up. Measured over the change, 120 fixtures at L14: commit
    /// decisions **470 → 632 per match**, and inside our own penalty area
    /// — the case that had almost no decisions at all — **35.6 → 75.0**.
    ///
    /// The same 0.098 over that many more moments gave 24.5 successful
    /// tackles per team against a real ~18 and 20.0 fouls against ~12.
    /// Scaled back by the ratio the volume moved by, so the per-match
    /// rate returns to where it was calibrated and only its DISTRIBUTION
    /// changes — which is the point: challenges now land where the
    /// contact is, and a carrier in our own box draws roughly three times
    /// as many as before.
    const BASE: f32 = 0.062;

    /// Commitment multiplier inside the defender's own penalty area when
    /// there is cover behind him — see [`Self::box_restraint`].
    ///
    /// 0.22 → 0.14 alongside `ContactFoul::BOX_PENALTY_RESTRAINT`, and
    /// for the same reason: this is a per-DECISION rate, and the marking
    /// work put far more defenders into duels inside their own area, so
    /// the same rate produced a different number of penalties per match.
    const BOX_RESTRAINT: f32 = 0.14;
    /// …and when he is the last man, where the challenge has to be made.
    const BOX_RESTRAINT_LAST_MAN: f32 = 0.60;

    /// How often the decision is taken while containing, in AI ticks.
    ///
    /// ONE ROLL PER MOMENT, not one per tick — the same discipline
    /// `intercept_rolled` / `save_rolled` / `block_rolled` enforce
    /// elsewhere, and for the same reason. Rolling every tick makes the
    /// rate a function of how long the defender happens to stay in range
    /// rather than of the defending: at 100 ticks in contact and any
    /// per-tick probability above ~3%, a challenge becomes a certainty,
    /// so the tackle cooldown silently remained the only real limiter and
    /// the whole decision was decorative. Measured that way it moved
    /// tackles per defender only 11.9 → 10.3 against a real 1.6.
    ///
    /// One second is the natural cadence: it is roughly how long a
    /// carrier holds a shape before his next touch, which is what creates
    /// or denies the moment.
    ///
    /// ⚠ 100 → 50, AND THAT IS A UNITS FIX, NOT A RATE CHANGE.
    ///
    /// The engine alternates full AI ticks with movement-only light ones
    /// and only the full ones run the state machine, so **one AI tick is
    /// 20 ms**, not 10 (`MatchPlayer::in_state_time`, `game_tick_light`).
    /// 100 was therefore a TWO-second cadence while the comment above —
    /// and the whole argument for the number — says one. 50 is what the
    /// doc has always described.
    const DECISION_INTERVAL_TICKS: u16 = 50;

    /// Is this tick one on which the defender re-decides?
    ///
    /// ⚠ THE CLOCK IS TIME IN CONTACT, NOT TIME IN THE STATE.
    ///
    /// This read `ctx.in_state_time`, and the comment claimed "entry
    /// always counts, so a defender arriving on a carrier who has already
    /// lost control can challenge immediately". Neither half held.
    /// `Tackling` is entered from up to 25u — every one of the five
    /// box-emergency routes hands over at that range, and so does
    /// `Pressing` — while an attempt is only ever rolled inside
    /// [`TackleEngagement::CONTACT`] (10u), and the state's own distance
    /// guard `return None`s above this call. So the entry roll was spent
    /// three metres away and discarded, and the defender then contained
    /// in silence until the phase came round again.
    ///
    /// Measured: **46% of the players in a `Tackling` state are inside
    /// contact**, and a carrier inside our own area — where a possession
    /// lasts a second or two — drew 20 commit decisions a match between
    /// every defender on the pitch. The commitment model was not
    /// declining those duels; it was never asked about them.
    ///
    /// [`MatchPlayer::contact_ticks`] counts the thing the cadence is
    /// about. It is incremented before the state machine runs, so the
    /// first tick in contact reads 1 and the roll happens on arrival.
    pub fn is_decision_tick(ctx: &StateProcessingContext) -> bool {
        let t = ctx.player.contact_ticks;
        t > 0 && (t - 1) % Self::DECISION_INTERVAL_TICKS == 0
    }

    /// Probability this defender commits to a challenge at this decision.
    ///
    /// `distance` is defender-to-carrier; closer is a better moment
    /// because the angle to the ball is better.
    pub fn commit_probability(ctx: &StateProcessingContext, distance: f32) -> f32 {
        let skills = &ctx.player.skills;
        let aggression = (skills.mental.aggression / 20.0).clamp(0.0, 1.0);
        let decisions = (skills.mental.decisions / 20.0).clamp(0.0, 1.0);
        let anticipation = (skills.mental.anticipation / 20.0).clamp(0.0, 1.0);

        // Temperament. An aggressive defender dives in; a good
        // decision-maker picks his moment, which means fewer challenges
        // but better ones. They pull in opposite directions on purpose.
        //
        // …and how good a tackler he actually IS decides whether the
        // moment looks like a moment at all. This model read no technical
        // attribute whatever, so a striker with `tackling` 5 weighed a
        // challenge exactly as a centre-half with 17 does — which is why
        // the front line kept out-tackling the back four however the duel
        // gate was arranged: **2.51 tackles per forward per match against
        // a real ~0.8, with defenders on 1.23 against ~1.6**, measured
        // over 120 fixtures with the plan already winning the gate.
        //
        // A striker presses to cut the pass and force the error; he
        // rarely goes to ground, because he is not good at it and knows
        // it. A centre-half's whole trade is the challenge. That is one
        // continuous attribute, not a role switch, and it costs the
        // calibration nothing: the term is centred on the MEASURED
        // population mean (`dev_match audit_levels`: outfield `tackling`
        // 14.03 at squad level 14, the harness's calibration level), so
        // the average player's commitment rate is unchanged and only the
        // spread is new. Centring it on a guessed 12/20 instead moved the
        // whole population — total tackles 18.0 → 19.8 per team — which
        // is a calibration change wearing a modelling change's clothes.
        //
        // `tackle_profile` already prices whether he WINS it. This prices
        // whether he goes — the two are different questions and the
        // second one had no answer.
        let tackling = (skills.technical.tackling / 20.0).clamp(0.0, 1.0);
        let temperament =
            (0.55 + aggression * 0.90 - decisions * 0.30 + (tackling - 0.70) * 1.10).max(0.12);

        // Cover behind me. This is the single biggest real-world licence
        // to commit: with a spare man you can afford to miss, without one
        // a failed tackle is a clear run at goal. The plan knows who the
        // cover is, so this is a fact rather than a guess.
        let cover_licence = if Self::cover_exists(ctx) { 1.55 } else { 0.75 };

        // Necessity. Near our own goal the cost of NOT engaging rises
        // faster than the cost of missing — this is where last-ditch
        // challenges come from.
        let own_goal = ctx.ball().direction_to_own_goal();
        let ball_to_goal = (ctx.tick_context.positions.ball.position - own_goal).magnitude();
        // 1.0 on the goal line, 0 at ~30 m out.
        let danger = (1.0 - ball_to_goal / 240.0).clamp(0.0, 1.0);
        let necessity = 1.0 + danger * 1.9;

        // The moment. A carrier moving quickly across or past the
        // defender has committed his weight and can be challenged; one
        // standing still shielding cannot. Anticipation is how well the
        // defender reads that.
        let carrier_speed = ctx
            .players()
            .opponents()
            .with_ball()
            .next()
            .map(|c| c.velocity(ctx).norm())
            .unwrap_or(0.0);
        let exposure = (carrier_speed / 0.45).clamp(0.0, 1.0);
        let timing = 1.0 + exposure * anticipation * 1.1;

        // Angle. At the outer edge of contact range the ball is a lunge
        // away; at his feet it is a clean block tackle.
        let proximity = (1.0 - distance / TackleEngagement::CONTACT.max(1.0)).clamp(0.0, 1.0);
        let reach = 0.65 + proximity * 0.70;

        let p = (Self::BASE
            * temperament
            * cover_licence
            * necessity
            * timing
            * reach
            * Self::urgency(ctx)
            * Self::box_restraint(ctx))
        .clamp(0.0, 0.55);
        #[cfg(feature = "match-logs")]
        crate::mid_run_diag::DuelDiag::note_decision(p, PenaltyRisk::in_own_box(ctx));
        p
    }

    /// How much the state of the match makes this defender go and get it.
    ///
    /// Nothing in the model above knows the score. A side a goal down with
    /// ten minutes left presses and challenges far more than the same side
    /// at 0-0, and a side protecting a lead drops off and stays on its
    /// feet — this is one of the most visible things in real football and
    /// the engine had no channel for it at all.
    ///
    /// The size of the swing is the player's, through
    /// [`skill_composites::resilience`]: `determination` heads that blend,
    /// and before this the attribute reached exactly one thing in the
    /// whole engine — a secondary mitigation inside the fatigue model's
    /// late-game mental penalty. A 19-determination player and a
    /// 5-determination player behaved identically at 0-1 down in the 85th
    /// minute, which is the one moment the attribute is *about*.
    ///
    /// Gated on [`MatchContext::behavioral_score_visible`], like every
    /// other score-reactive read in the engine — the score-reaction
    /// regime is deliberately bounded to the closing half-hour to cap its
    /// draw-correlation budget, and `OF_SCORE_BLIND` has to switch ALL of
    /// it off or the A/B control means nothing. Within that window
    /// `pressure` ramps continuously to the whistle, so there is no tick
    /// where a match visibly changes character.
    fn urgency(ctx: &StateProcessingContext) -> f32 {
        if !ctx.context.behavioral_score_visible() {
            return 1.0;
        }
        let minute = sc::minute_from_ms(ctx.context.total_match_time);
        let from = MatchContext::SCORE_REACTION_FROM_MINUTE as f32;
        // Scaled with the rest of the regime — see
        // `MatchContext::SCORE_REACTION_GAIN`.
        let pressure = ((minute as f32 - from) / (90.0 - from)).clamp(0.0, 1.0)
            * MatchContext::score_reaction_gain();
        let home = ctx.player.team_id == ctx.context.field_home_team_id;
        let (mine, theirs) = if home {
            (
                ctx.context.score.home_team.get() as i32,
                ctx.context.score.away_team.get() as i32,
            )
        } else {
            (
                ctx.context.score.away_team.get() as i32,
                ctx.context.score.home_team.get() as i32,
            )
        };
        let deficit = theirs - mine;
        if deficit == 0 {
            return 1.0;
        }
        // Two goals down is as urgent as it gets — three down is a
        // different kind of resignation, not more chasing.
        let magnitude = (deficit.abs().min(2) as f32) / 2.0;
        let drive = sc::resilience(ctx.player, minute);
        if deficit > 0 {
            // Behind: chase. A driven player goes after it much harder
            // than a passenger.
            1.0 + pressure * magnitude * (0.10 + drive * 0.35)
        } else {
            // Ahead: see it out. Determination reads as game management
            // here, not as diving in — the driven player is the one who
            // stays on his feet and holds the shape.
            1.0 - pressure * magnitude * (0.08 + drive * 0.22)
        }
    }

    /// How much a defender holds back from a challenge because he is
    /// inside his own penalty area.
    ///
    /// `necessity` above is right that the cost of NOT engaging rises
    /// toward your own goal — but it peaks at the EDGE of the box, not
    /// inside it. Inside, the cost of the challenge itself jumps
    /// discontinuously: a missed tackle is a penalty and usually a card,
    /// so the professional response is to stay on your feet, show him
    /// wide, and block the shot. Real defenders visibly stop diving in
    /// there, which is why penalties are ~1% of all fouls.
    ///
    /// [`ContactFoul::probability`] has always modelled this — its own
    /// box restraint is 0.004, and its comment says "same restraint the
    /// tackle model applies". **The tackle model did not apply it.** The
    /// shirt-pull path was restrained and the sliding-tackle path was
    /// *encouraged*, by `necessity`, in exactly the same square of grass.
    ///
    /// It stayed invisible while the pitch was sparse. Compacting the
    /// team shape put far more bodies in and around the box, `necessity`
    /// multiplied every one of those engagements by up to 2.9, and
    /// penalties went from 0.27 a match to 1.0 — against a real 0.25-0.30.
    ///
    /// The last-ditch challenge survives: when the carrier has beaten
    /// everyone and this defender is the last man, the restraint lifts,
    /// because at that point conceding a penalty really is better than
    /// conceding a goal. That is the case the dramatic box challenge is
    /// FOR, and it is rare — which is why the rate it produces is rare too.
    fn box_restraint(ctx: &StateProcessingContext) -> f32 {
        if !PenaltyRisk::in_own_box(ctx) {
            return 1.0;
        }
        if Self::cover_exists(ctx) {
            // Somebody is behind me — there is no excuse at all.
            Self::BOX_RESTRAINT
        } else {
            // Last man. Still more careful than in open play, but this is
            // the challenge that has to be made.
            Self::BOX_RESTRAINT_LAST_MAN
        }
    }

    /// Is there somebody behind me if I miss?
    ///
    /// Asked geometrically rather than from the plan. The plan's `Cover`
    /// duty only exists for the back line, so reading it directly told
    /// every midfielder in the game that he had no cover — and midfield
    /// is the one part of the pitch that always does, because the whole
    /// defence is behind it. That suppressed midfield challenges to 628
    /// attempts against the defenders' 1,172, and dragged successful
    /// tackles to 12.9 per team against a real ~18.
    ///
    /// "A team-mate goal-side of me and near enough to deal with it" is
    /// the question a player actually asks, it works for every role, and
    /// it does not depend on which unit the plan happens to cover.
    ///
    /// ⚠ THE GOALKEEPER IS NOT COVER, AND COUNTING HIM MADE
    /// [`Self::BOX_RESTRAINT_LAST_MAN`] DEAD CODE.
    ///
    /// The scan walked the whole roster, and the keeper is goal-side of
    /// every outfielder who is not standing on his own goal line and
    /// well inside `COVER_SCAN` of anybody defending his own box. So
    /// inside our own penalty area — the one place the last-man branch
    /// exists for — `cover_exists` was unconditionally true: every box
    /// challenge took `BOX_RESTRAINT` (0.14) and `cover_licence` 1.55,
    /// and the 0.60 arm written for "the carrier has beaten everyone and
    /// I am the last man" could not be reached.
    ///
    /// He is also the wrong answer to the question. "Is there somebody
    /// behind me if I miss" asks who deals with the carrier once he is
    /// past me; the keeper is the thing being protected, and a defender
    /// who misses in front of him has produced a one-on-one, which is
    /// the opposite of cover. Excluding him is what makes a genuine last
    /// man behave like one: `box_restraint` 0.14 → 0.60 and
    /// `cover_licence` 1.55 → 0.75, a net ×2.1 on exactly the duels
    /// where a real defender does commit.
    fn cover_exists(ctx: &StateProcessingContext) -> bool {
        let own_goal = ctx.ball().direction_to_own_goal();
        let me = ctx.player.position;
        let my_depth = (own_goal - me).magnitude();
        ctx.players().teammates().all().any(|t| {
            t.id != ctx.player.id
                && !t.tactical_positions.is_goalkeeper()
                && (own_goal - t.position).magnitude() < my_depth
                && (t.position - me).magnitude() < COVER_SCAN
        })
    }

    /// Where a containing defender stands: goal-side of the carrier, a
    /// stride off him. This is the jockey — he is between his man and the
    /// goal, close enough to challenge the moment the touch is loose, and
    /// NOT running through him.
    pub fn contain_position(ctx: &StateProcessingContext, carrier: Vector3<f32>) -> Vector3<f32> {
        let own_goal = ctx.ball().direction_to_own_goal();
        let to_goal = (own_goal - carrier)
            .try_normalize(0.01)
            .unwrap_or_else(|| Vector3::new(1.0, 0.0, 0.0));
        carrier + to_goal * TackleEngagement::CONTACT * 0.8
    }
}

/// Fouls that are not tackle attempts — the shirt pull on a runner, the
/// block across his path, the hold at the shoulder.
///
/// # Why this exists
///
/// Tackling was the engine's **only** source of fouls, and the numbers
/// say that cannot reach football. Traced end to end: ~25 failed
/// challenges per team, ~45% of which roll a foul, ~55% of which the
/// referee whistles, gives ~6 fouls per team against a real ~12. Every
/// rate in that chain is defensible on its own; the chain is simply too
/// short. Free kicks starved with it (3.3 per match against 20-24) and
/// so did the card pipeline (2.4 yellows against 3.5-4.5).
///
/// Real defending fouls constantly without attempting a tackle. A player
/// who has been beaten grabs a shirt; one tracking a runner leans across
/// him; one marking at a set piece holds. None of those are challenges
/// for the ball, and none of them existed here.
///
/// Rolled on the same one-decision-per-second cadence as
/// [`TackleDecision`], for the same reason: a per-tick roll makes the
/// rate a function of how long two players happen to stand near each
/// other rather than of the defending.
pub struct ContactFoul;

impl ContactFoul {
    /// How often the decision is taken while engaged, in AI ticks — so
    /// **two real seconds**, not one. One AI tick is 20 ms: the engine
    /// alternates full ticks with movement-only light ones and only the
    /// full ones advance `in_state_time` (see `game_tick_light`).
    ///
    /// Left at 100 deliberately. Unlike [`TackleDecision`]'s clock this
    /// one is anchored to the right thing already — dwell in the
    /// engagement is exactly what a shirt-pull is a function of — so the
    /// only defect was the doc, and `BASE` below is fitted against this
    /// cadence and a foul rate that is already 16.3 per team against a
    /// real ~12. Halving the interval here doubles contact fouls and
    /// nothing else; it is a calibration change wearing a units fix's
    /// clothes.
    const DECISION_INTERVAL_TICKS: u64 = 100;
    /// Close enough for contact (~2.5 m).
    const CONTACT_RANGE: f32 = 20.0;
    /// Per-decision chance for an ordinary engagement.
    ///
    /// Re-fitted (0.034 → 0.017) after the box restraint landed on
    /// [`TackleDecision`]. The two foul models are coupled through DWELL:
    /// a defender who declines the challenge stays engaged, and this model
    /// rolls once a second *for as long as he is engaged*. Restraining
    /// tackles in and around the box therefore lengthened engagements and
    /// **tripled** defender contact fouls (435 → 1062 emitted per 20
    /// matches), taking the whistled rate from 14.3 to 23.6 per team
    /// against a real ~12. The base was fitted when engagements were
    /// shorter; this is the same rate expressed over the new dwell.
    const BASE: f32 = 0.021;

    /// Restraint when a foul here would be a PENALTY (the ball is in our
    /// own area).
    ///
    /// Fitted to the real penalty rate rather than asserted. The previous
    /// 0.004 was written as "nobody grabs a shirt inside his own area",
    /// which is an exaggeration of a real tendency: the shirt-pull at a
    /// corner and the clumsy challenge on a crosser are precisely where
    /// real penalties come from, and at 0.004 the engine emitted **four
    /// box contacts in twenty matches** and awarded 0.08 penalties a match
    /// against a real 0.25-0.30.
    ///
    /// 0.30 said a defender is about three times less likely to foul with
    /// the ball in his own box than in open play — the real tendency, at
    /// a strength that produced the real rate **for the engagement volume
    /// of the day**.
    ///
    /// Re-fitted 0.30 → 0.11 when the marking work landed. This model
    /// rolls once a second *per engagement*, so its output scales with
    /// how many defenders are engaged, and putting the back line on its
    /// men took box contacts from 95 to 154 per 200 matches (`FOUL SOURCE
    /// CENSUS`, `Defender: Standing` 42 → 114) with penalties following
    /// 0.35 → 0.55 a match against a real 0.25-0.30. The rate per
    /// engagement was never the thing that was calibrated; the rate per
    /// MATCH was, and the divisor moved.
    ///
    /// Fitted in two steps because the first (0.17) recovered only a
    /// third of it — 0.55 → 0.45 — which is the measurement that says the
    /// box contacts really had roughly tripled rather than risen 60%.
    const BOX_PENALTY_RESTRAINT: f32 = 0.11;
    /// Restraint when the player is in his own area but the ball is not
    /// yet — marking a runner at the back post, tracking into the box.
    /// Careful, but not the catastrophic case.
    const BOX_POSITION_RESTRAINT: f32 = 0.10;

    /// Is this tick one on which a contact foul is considered?
    pub fn is_decision_tick(ctx: &StateProcessingContext) -> bool {
        ctx.in_state_time > 0 && ctx.in_state_time % Self::DECISION_INTERVAL_TICKS == 0
    }

    /// Probability this engagement becomes a foul now.
    ///
    /// `gap` is the distance to the man being engaged. `losing_him` is
    /// true when he is pulling away — which is when a beaten defender
    /// actually grabs, and the single biggest driver of this kind of
    /// foul.
    pub fn probability(ctx: &StateProcessingContext, gap: f32, losing_him: bool) -> f32 {
        if gap > Self::CONTACT_RANGE {
            return 0.0;
        }
        let skills = &ctx.player.skills;
        let aggression = (skills.mental.aggression / 20.0).clamp(0.0, 1.0);
        let discipline =
            ((skills.mental.composure + skills.mental.concentration) / 40.0).clamp(0.0, 1.0);

        // Temperament — the same pull the tackle model uses.
        let temperament = (0.55 + aggression * 0.95 - discipline * 0.35).clamp(0.2, 1.6);
        // Being beaten is what turns contact into a foul.
        let desperation = if losing_him { 2.1 } else { 1.0 };
        // Touch-tight contact fouls more than a covering position.
        let proximity = (1.0 - gap / Self::CONTACT_RANGE).clamp(0.0, 1.0);
        let closeness = 0.55 + proximity * 0.90;

        // Nobody grabs a shirt when the downside is a penalty, and real
        // defenders visibly stop doing it. Same test the tackle model
        // uses — see `PenaltyRisk`.
        // Nobody grabs a shirt when the downside is a penalty. Graded,
        // because "would this be a penalty" and "am I standing in the
        // box" are different questions with different answers — see
        // `PenaltyRisk`.
        let box_restraint = if PenaltyRisk::applies(ctx) {
            Self::BOX_PENALTY_RESTRAINT
        } else if PenaltyRisk::in_own_box(ctx) {
            Self::BOX_POSITION_RESTRAINT
        } else {
            1.0
        };

        (Self::BASE * temperament * desperation * closeness * box_restraint).clamp(0.0, 0.30)
    }

    /// Severity of a non-tackle foul. These are overwhelmingly cynical
    /// rather than dangerous — a shirt pull is a yellow at worst — so the
    /// reckless tail is thin and there is no violent one.
    ///
    /// The `losing_him` tail was trimmed `0.10 + aggr·0.22` → `0.06 +
    /// aggr·0.14` alongside [`Self::BOX_PENALTY_RESTRAINT`], and for the
    /// same reason. `losing_him` is true whenever the man is pulling away
    /// from his marker, so its frequency rose with the number of live
    /// marking duels — red cards went **0.24 → 0.46 a match against a
    /// real 0.15-0.20** while total fouls rose only 21%, which is the
    /// signature of a severity tail being sampled more often rather than
    /// of more fouls.
    pub fn severity(ctx: &StateProcessingContext, losing_him: bool) -> FoulSeverity {
        let aggression = (ctx.player.skills.mental.aggression / 20.0).clamp(0.0, 1.0);
        // Stopping a man who has gone past you is the professional foul.
        let reckless_p = if losing_him {
            0.06 + aggression * 0.14
        } else {
            0.04
        };
        if ctx.context.rng.unit_f32() < reckless_p {
            FoulSeverity::Reckless
        } else {
            FoulSeverity::Normal
        }
    }
}
