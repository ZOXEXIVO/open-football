//! Casting: who is doing what while the goal is being celebrated, fixed
//! once at the moment the ball crosses the line.

use nalgebra::Vector3;

/// What one player is doing while the goal is being celebrated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::r#match::engine::flow) enum Role {
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
    pub(in crate::r#match::engine::flow) fn stillness_ms(self, beaten: bool) -> u64 {
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
pub(in crate::r#match::engine::flow) struct CastMember {
    pub(in crate::r#match::engine::flow) id: u32,
    pub(in crate::r#match::engine::flow) role: Role,
    pub(in crate::r#match::engine::flow) anchor: Vector3<f32>,
}
