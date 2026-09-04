//! The split-step: the hop a goalkeeper makes as the shot is struck.
//!
//! # Why this exists
//!
//! Watch any goalkeeper face a shot and the first thing he does is leave
//! the ground — a small two-footed hop, timed so that he lands as the
//! ball leaves the foot, with his weight spread and his knees loaded.
//! The point is not the height; it is the landing. A body that has just
//! come down can push off in any direction at once, where a body standing
//! flat has to unweight one leg first. Keepers are coached into it from
//! the age of eight, and it is the single most repeated movement of
//! their match: once for every shot, cross and header that comes their
//! way, twenty-odd times a game.
//!
//! The engine had no such thing. `PreparingForSave` set him and held him
//! still, `KeeperShotReaction` kept him motionless for the reaction, and
//! then he either shuffled or dived. Measured on a recorded match his
//! height stayed at exactly 0.00 m on every shot he saved on his feet —
//! which is 84% of the balls that reach him at pace — so from the stands
//! a shot arrived at a statue that then moved. The reaction was modelled;
//! the thing that makes a reaction visible was not.
//!
//! # The model
//!
//! On the tick he first sees the strike he hops. The rise is a few
//! centimetres and the hang about a fifth of a second — [`Self::APEX_HEAVY`]
//! to [`Self::APEX_SPRING`] by his agility — so he is coming down as his
//! reaction time runs out, which is what the real timing is: a keeper
//! lands on the strike and goes from the landing.
//!
//! ⚠ **It changes nothing about the save.** The hop lives on the same
//! vertical axis as the leap ([`MatchPlayer::hop`]) but is not a leap: a
//! dive may push off from it ([`MatchPlayer::leap`] accepts a keeper on a
//! hop), the physics reads a hopping keeper as on his feet
//! ([`MatchPlayer::is_launched`]), and the horizontal shuffle the reaction
//! model allows is untouched. Every calibrated number — reaction, shuffle,
//! reach, the roll — is where it was; the recording simply now carries a
//! keeper who visibly reacts.
//!
//! Real timing is a fraction earlier still — the hop is cued off the
//! striker's plant foot, not off the ball — but this engine strikes on the
//! tick the decision is made (there is no wind-up on a shot), so the
//! strike is the earliest signal there is.

use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::player::state::PlayerState;
use crate::r#match::{GameTickContext, MatchPlayer};

pub struct KeeperSplitStep;

impl KeeperSplitStep {
    /// Apex of the hop, in metres, for a heavy-footed keeper and for a
    /// springy one. A split-step is two to six centimetres in real life;
    /// the floor is set where the recorder still sees it — samples are
    /// quantised to a centimetre and the viewer's own feet-off-the-ground
    /// bar is two — so a hop that happens is a hop that is drawn.
    const APEX_HEAVY: f32 = 0.04;
    const APEX_SPRING: f32 = 0.06;

    /// He hops when he first sees the strike: elapsed flight, in engine
    /// ticks (10 ms), under this. The AI runs every second tick, so a
    /// keeper sees a shot 0-2 ticks old and this fires exactly once — by
    /// the time he lands the flight is fifteen ticks old and the window
    /// has closed behind him.
    const STRIKE_WINDOW: f32 = 6.0;

    /// A ball that will reach him before he could land is not worth
    /// leaving the ground for — the hang of the hop itself, in engine
    /// ticks, plus a margin. A point-blank shot from three metres finds
    /// him flat-footed, which is also what happens to a real keeper.
    const MIN_FLIGHT_LEFT: f32 = 24.0;

    /// The states in which a keeper is on his feet facing the play and
    /// can hop. Everything else — the dive, the leap, the punch, a ball
    /// in his hands, a release — is a body already committed to
    /// something.
    fn on_his_feet(state: PlayerState) -> bool {
        matches!(
            state,
            PlayerState::Goalkeeper(
                GoalkeeperState::Standing
                    | GoalkeeperState::Walking
                    | GoalkeeperState::PreparingForSave
                    | GoalkeeperState::Catching
                    | GoalkeeperState::ReturningToGoal
                    | GoalkeeperState::ComingOut
                    | GoalkeeperState::TakeBall
            )
        )
    }

    /// False when `OF_KEEPER_HOP=off` — the A/B control. The hop is meant
    /// to be calibration-neutral by construction (nothing that adjudicates
    /// a save reads it), and the switch is how that claim is checked on
    /// one binary rather than believed.
    pub fn armed() -> bool {
        use std::sync::OnceLock;
        static ON: OnceLock<bool> = OnceLock::new();
        *ON.get_or_init(|| std::env::var("OF_KEEPER_HOP").as_deref() != Ok("off"))
    }

    /// The apex to hop to, if this is the tick to hop on; `None` on every
    /// other tick of the match.
    pub fn apex(player: &MatchPlayer, tick_context: &GameTickContext) -> Option<f32> {
        if !Self::armed() {
            return None;
        }
        if !Self::on_his_feet(player.state) || player.is_airborne() || player.vertical_speed != 0.0
        {
            return None;
        }
        let ball = &tick_context.ball;
        if ball.is_owned || ball.held_in_hands {
            return None;
        }
        let target = ball
            .cached_shot_target
            .as_ref()
            .filter(|t| Some(t.defending_side) == player.side)?;

        let flight = &tick_context.positions.ball;
        let speed = flight.velocity.norm();
        if speed < 1e-3 {
            return None;
        }
        // Same clock `KeeperShotReaction::since_strike` reads: the ground
        // already flown, over the pace it is flying at.
        let since_strike = (flight.position - target.struck_from).magnitude() / speed;
        if since_strike >= Self::STRIKE_WINDOW {
            return None;
        }
        // …and how much of the flight is left to his depth. Along the
        // goal axis when the shot has one; a ball squared across the face
        // of goal is measured on its whole path instead, because dividing
        // by a vanishing `x` component hands out windows of seconds.
        let flight_left = if flight.velocity.x.abs() > 0.05 {
            ((player.position.x - flight.position.x) / flight.velocity.x).max(0.0)
        } else {
            (player.position - flight.position).magnitude() / speed
        };
        if flight_left < Self::MIN_FLIGHT_LEFT {
            return None;
        }

        let spring = ((player.skills.physical.agility - 1.0) / 19.0).clamp(0.0, 1.0);
        Some(Self::APEX_HEAVY + (Self::APEX_SPRING - Self::APEX_HEAVY) * spring)
    }
}
