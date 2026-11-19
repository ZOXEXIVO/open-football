use crate::r#match::StateProcessingContext;
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::player::state::PlayerState;

/// Where a player's shape position is, and whether he is far enough
/// off it to be worth a recovery run.
pub struct ShapeStation;

impl ShapeStation {
    /// Recovery run is complete inside this radius. Same value
    /// `MidfielderReturningState` has always used to hand back to
    /// `Running`, so "arrived" means exactly what it did before.
    pub const HOME: f32 = 80.0;
    /// A midfielder already home is only sent back out once he has
    /// drifted past this. The gap to `HOME` is the commitment that stops
    /// the two states contradicting each other.
    pub const DISPLACED: f32 = 110.0;

    /// Distance from the player's shape position.
    pub fn drift(ctx: &StateProcessingContext) -> f32 {
        (ctx.player.position - ctx.team().my_anchor()).magnitude()
    }

    /// Is there a recovery run left to make?
    ///
    /// Hysteretic on the current state: a player already on a recovery run
    /// keeps going until he is inside `HOME`, while one who is not has to
    /// have drifted past `DISPLACED` before a fresh recovery is worth
    /// starting. Both sides of the hand-off read this, so neither can
    /// contradict the other.
    pub fn should_recover(ctx: &StateProcessingContext) -> bool {
        let returning = matches!(
            ctx.player.state,
            PlayerState::Midfielder(MidfielderState::Returning | MidfielderState::TrackingBack)
                | PlayerState::Forward(ForwardState::Returning)
        );
        let band = if returning {
            Self::HOME
        } else {
            Self::DISPLACED
        };
        Self::drift(ctx) > band
    }
}
