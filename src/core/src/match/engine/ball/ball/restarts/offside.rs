use crate::r#match::PlayerSide;
use crate::r#match::engine::ball::ball::PassOriginRestart;

/// **Who was in an offside position when a team-mate last played the
/// ball.**
///
/// Taken at every pass and shot and kept until somebody deliberately plays
/// the ball again — a deflection, a save or the woodwork does not end it —
/// so whichever of them is first involved is flagged, whether the ball was
/// meant for him or not.
#[derive(Debug, Clone, Copy)]
pub struct OffsideSnapshot {
    pub origin: PassOriginRestart,
    pub passer_id: u32,
    pub passer_side: PlayerSide,
    offside: [u32; Self::MAX_ATTACKERS],
    count: u8,
    pub set_tick: u64,
}

impl OffsideSnapshot {
    const MAX_ATTACKERS: usize = 10;

    /// The snapshot at a kick from `ball_x` by a side attacking from
    /// `passer_side`, against the defending line `line_x`, with the
    /// attackers given as `(id, x)`. `None` when nobody is offside.
    pub fn at_kick(
        origin: PassOriginRestart,
        passer_id: u32,
        passer_side: PlayerSide,
        ball_x: f32,
        line_x: f32,
        halfway_x: f32,
        attackers: impl Iterator<Item = (u32, f32)>,
        tick: u64,
    ) -> Option<Self> {
        let mut snap = Self {
            origin,
            passer_id,
            passer_side,
            offside: [0; Self::MAX_ATTACKERS],
            count: 0,
            set_tick: tick,
        };
        for (id, x) in attackers {
            let in_opponent_half = match passer_side {
                PlayerSide::Left => x > halfway_x,
                PlayerSide::Right => x < halfway_x,
            };
            if id != passer_id
                && in_opponent_half
                && OffsideLine::is_beyond(passer_side, x, ball_x, line_x)
                && (snap.count as usize) < Self::MAX_ATTACKERS
            {
                snap.offside[snap.count as usize] = id;
                snap.count += 1;
            }
        }
        (snap.count > 0).then_some(snap)
    }

    /// Was `player_id` offside when the ball was played?
    pub fn flags(&self, player_id: u32) -> bool {
        self.offside[..self.count as usize].contains(&player_id)
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

#[cfg(test)]
mod offside_snapshot_tests {
    use super::*;

    const HALFWAY: f32 = 420.0;

    fn kick_left(ball_x: f32, line_x: f32, attackers: &[(u32, f32)]) -> Option<OffsideSnapshot> {
        OffsideSnapshot::at_kick(
            PassOriginRestart::OpenPlay,
            1,
            PlayerSide::Left,
            ball_x,
            line_x,
            HALFWAY,
            attackers.iter().copied(),
            0,
        )
    }

    #[test]
    fn left_attacker_beyond_second_last_is_offside() {
        let snap = kick_left(600.0, 680.0, &[(2, 700.0)]).expect("one man offside");
        assert!(snap.flags(2));
    }

    #[test]
    fn left_attacker_behind_ball_not_offside() {
        assert!(kick_left(600.0, 680.0, &[(2, 500.0)]).is_none());
    }

    #[test]
    fn left_attacker_level_with_defender_not_offside() {
        assert!(kick_left(600.0, 680.0, &[(2, 681.0)]).is_none());
    }

    #[test]
    fn every_attacker_beyond_the_line_is_recorded() {
        let snap = kick_left(600.0, 680.0, &[(2, 700.0), (3, 650.0), (4, 720.0)]).unwrap();
        assert!(snap.flags(2));
        assert!(!snap.flags(3));
        assert!(snap.flags(4));
    }

    #[test]
    fn nobody_in_his_own_half_is_offside() {
        assert!(kick_left(200.0, 300.0, &[(2, 400.0)]).is_none());
    }

    #[test]
    fn the_passer_is_never_his_own_offside_man() {
        assert!(kick_left(600.0, 680.0, &[(1, 700.0)]).is_none());
    }

    #[test]
    fn restart_origins_offside_exempt() {
        assert!(PassOriginRestart::GoalKick.is_offside_exempt());
        assert!(PassOriginRestart::Corner.is_offside_exempt());
        assert!(PassOriginRestart::ThrowIn.is_offside_exempt());
        assert!(!PassOriginRestart::OpenPlay.is_offside_exempt());
        assert!(!PassOriginRestart::IndirectFreeKick.is_offside_exempt());
    }
}
