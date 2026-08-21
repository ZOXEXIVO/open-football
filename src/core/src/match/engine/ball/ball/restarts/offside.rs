use crate::r#match::PlayerSide;
use crate::r#match::engine::ball::ball::PassOriginRestart;

/// Snapshot of the offside-relevant geometry at the moment a pass is
/// kicked. Stored on the ball for the duration of an in-flight pass so
/// the offside check can fire on receiver involvement (touch / claim /
/// active challenge) instead of at pass start.
#[derive(Debug, Clone, Copy)]
pub struct OffsideSnapshot {
    pub origin: PassOriginRestart,
    pub passer_id: u32,
    pub passer_side: PlayerSide,
    pub receiver_id: u32,
    pub ball_x_at_kick: f32,
    pub second_last_defender_x: f32,
    pub receiver_x_at_kick: f32,
    pub receiver_y_at_kick: f32,
    pub set_tick: u64,
}

impl OffsideSnapshot {
    /// Decide whether the snapshot represents an offside position.
    pub fn is_offside(&self) -> bool {
        OffsideLine::is_beyond(
            self.passer_side,
            self.receiver_x_at_kick,
            self.ball_x_at_kick,
            self.second_last_defender_x,
        )
    }
}

/// **The offside line, and the one rule for being beyond it.**
///
/// # Why it is shared
///
/// The referee had this rule and nobody else did. `build_offside_snapshot`
/// worked the line out at the moment of the pass and flagged the receiver
/// afterwards, while the pass evaluator — which chooses that receiver —
/// had no offside term at all: measured over 60 matches, **25.4 offsides a
/// match against a real 4-6**, because a passer would cheerfully play a
/// ball to a man standing two metres beyond the last defender.
///
/// Real football's offside rate is low not because the flag is rare but
/// because nobody deliberately plays one. That only holds if the passer
/// reads the SAME line the referee does — a passer avoiding a line one
/// unit away from the official one would still concede them, and would
/// look like it was avoiding nothing.
pub struct OffsideLine;

impl OffsideLine {
    /// Absorbs foot-vs-shoulder ambiguity, in game units.
    pub const TOLERANCE: f32 = 1.5;

    /// The second-last opponent's `x` — the line itself — for a side
    /// attacking in `attacking`'s direction.
    ///
    /// One pass and no allocation, because the pass evaluator asks this
    /// on every tick a player is on the ball. `None` when fewer than two
    /// opponents are on the pitch, where there is no line to speak of.
    pub fn second_last(xs: impl Iterator<Item = f32>, attacking: PlayerSide) -> Option<f32> {
        // "Deepest" means nearest the goal being attacked, so the two are
        // tracked in the direction that side plays.
        let (mut deepest, mut second) = (None::<f32>, None::<f32>);
        let beyond = |a: f32, b: f32| match attacking {
            PlayerSide::Left => a > b,
            PlayerSide::Right => a < b,
        };
        for x in xs {
            if deepest.is_none_or(|d| beyond(x, d)) {
                second = deepest;
                deepest = Some(x);
            } else if second.is_none_or(|s| beyond(x, s)) {
                second = Some(x);
            }
        }
        second
    }

    /// Is a receiver at `receiver_x` in an offside position — beyond both
    /// the ball and the line?
    pub fn is_beyond(attacking: PlayerSide, receiver_x: f32, ball_x: f32, line_x: f32) -> bool {
        match attacking {
            PlayerSide::Left => {
                receiver_x > ball_x + Self::TOLERANCE && receiver_x > line_x + Self::TOLERANCE
            }
            PlayerSide::Right => {
                receiver_x < ball_x - Self::TOLERANCE && receiver_x < line_x - Self::TOLERANCE
            }
        }
    }
}

#[allow(dead_code, unused_imports)]
mod offside_snapshot_tests {
    use super::*;

    fn snap_left(receiver_x: f32, ball_x: f32, second_last: f32) -> OffsideSnapshot {
        OffsideSnapshot {
            origin: PassOriginRestart::OpenPlay,
            passer_id: 1,
            passer_side: PlayerSide::Left,
            receiver_id: 2,
            ball_x_at_kick: ball_x,
            second_last_defender_x: second_last,
            receiver_x_at_kick: receiver_x,
            receiver_y_at_kick: 200.0,
            set_tick: 0,
        }
    }

    #[test]
    fn left_attacker_beyond_second_last_is_offside() {
        // Receiver ahead of ball AND past the second-last defender.
        let snap = snap_left(700.0, 600.0, 680.0);
        assert!(snap.is_offside());
    }

    #[test]
    fn left_attacker_behind_ball_not_offside() {
        // Receiver is behind the ball — offside cannot occur.
        let snap = snap_left(500.0, 600.0, 680.0);
        assert!(!snap.is_offside());
    }

    #[test]
    fn left_attacker_level_with_defender_not_offside() {
        // Within tolerance — onside.
        let snap = snap_left(681.0, 600.0, 680.0);
        assert!(!snap.is_offside());
    }

    #[test]
    fn restart_origins_offside_exempt() {
        assert!(PassOriginRestart::GoalKick.is_offside_exempt());
        assert!(PassOriginRestart::Corner.is_offside_exempt());
        assert!(PassOriginRestart::ThrowIn.is_offside_exempt());
        assert!(!PassOriginRestart::OpenPlay.is_offside_exempt());
        assert!(!PassOriginRestart::FreeKick.is_offside_exempt());
    }
}
