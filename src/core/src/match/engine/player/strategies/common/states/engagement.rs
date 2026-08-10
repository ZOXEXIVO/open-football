use crate::r#match::StateProcessingContext;

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
