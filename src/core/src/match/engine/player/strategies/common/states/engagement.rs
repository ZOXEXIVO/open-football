use crate::r#match::StateProcessingContext;
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
    pub fn should_commit(ctx: &StateProcessingContext, distance: f32) -> bool {
        distance < Self::COMMIT
            && ctx.player.can_attempt_tackle()
            && ctx.team().is_best_player_to_chase_ball()
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
    /// a typical decision lands near 0.25-0.30 once the multipliers are
    /// applied — a defender who stands his man up for a second or two
    /// usually does not dive in.
    const BASE: f32 = 0.16;

    /// How often the decision is taken while containing, in ticks.
    ///
    /// ONE ROLL PER MOMENT, not one per tick — the same discipline
    /// `intercept_rolled` / `save_rolled` / `block_rolled` enforce
    /// elsewhere, and for the same reason. Rolling every tick makes the
    /// rate a function of how long the defender happens to stay in range
    /// rather than of the defending: at 100 ticks in contact and any
    /// per-tick probability above ~3%, a challenge becomes a certainty,
    /// so the tackle cooldown (~1 s) silently remained the only real
    /// limiter and the whole decision was decorative. Measured that way
    /// it moved tackles per defender only 11.9 → 10.3 against a real 1.6.
    ///
    /// One second is the natural cadence: it is roughly how long a
    /// carrier holds a shape before his next touch, which is what creates
    /// or denies the moment.
    const DECISION_INTERVAL_TICKS: u64 = 100;

    /// Is this tick one on which the defender re-decides? Entry always
    /// counts, so a defender arriving on a carrier who has already lost
    /// control can challenge immediately.
    pub fn is_decision_tick(ctx: &StateProcessingContext) -> bool {
        ctx.in_state_time % Self::DECISION_INTERVAL_TICKS == 0
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
        let temperament = 0.55 + aggression * 0.90 - decisions * 0.30;

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

        (Self::BASE * temperament * cover_licence * necessity * timing * reach).clamp(0.0, 0.55)
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
    fn cover_exists(ctx: &StateProcessingContext) -> bool {
        let own_goal = ctx.ball().direction_to_own_goal();
        let me = ctx.player.position;
        let my_depth = (own_goal - me).magnitude();
        ctx.players().teammates().all().any(|t| {
            t.id != ctx.player.id
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
