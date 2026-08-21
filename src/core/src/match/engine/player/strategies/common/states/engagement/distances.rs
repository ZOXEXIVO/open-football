use super::duel::PenaltyRisk;
use crate::r#match::{DefensiveDuty, StateProcessingContext};

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
