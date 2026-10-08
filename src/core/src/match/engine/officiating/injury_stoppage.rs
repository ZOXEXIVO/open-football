//! **The referee stops play for an injured player.**
//!
//! A knock is played on — the player gets up by himself. Anything worse
//! stops the match: if the ball is already dead the restart waits for the
//! physio, and if it is live the referee stops play and restarts with a
//! drop ball once the player has been treated. Either way the wait is
//! treatment time, which the referee adds back.

use crate::r#match::engine::ball::ball::RestartHold;
use crate::r#match::engine::flow::context::MATCH_TIME_INCREMENT_MS;
use crate::r#match::engine::player::injury::{InjuryGrade, InjuryRisk};
use crate::r#match::engine::result::DeadTime;
use crate::r#match::{MatchContext, MatchField, PlayerSide};

pub struct InjuryStoppage;

impl InjuryStoppage {
    pub fn call(field: &mut MatchField, context: &mut MatchContext, severity: InjuryGrade) {
        if severity == InjuryGrade::Knock {
            return;
        }
        let now = context.current_tick();
        let hold = RestartHold {
            until_tick: now + InjuryRisk::treatment_ms(severity) / MATCH_TIME_INCREMENT_MS,
            reason: DeadTime::Treatment,
        };
        if let Some(restart) = field.ball.awaiting_restart.as_mut() {
            restart.hold_until(hold.reason, hold.until_tick);
            return;
        }
        // A goal is its own stoppage; the celebration window covers it.
        if field.ball.in_net.is_some() {
            return;
        }
        let Some(taker) = Self::drop_ball_taker(field, context) else {
            return;
        };
        let walk = field
            .get_player(taker)
            .map_or(0.0, |p| (p.position - field.ball.position).magnitude());
        field.ball.stop_for_drop_ball(taker, walk, now, hold);
    }

    /// Law 8: dropped to the defending goalkeeper when play stopped inside
    /// his penalty area, otherwise to the nearest fit player of the side
    /// that last touched the ball.
    fn drop_ball_taker(field: &MatchField, context: &MatchContext) -> Option<u32> {
        let spot = field.ball.position;
        for side in [PlayerSide::Left, PlayerSide::Right] {
            if context
                .penalty_area(side == PlayerSide::Left)
                .contains(&spot)
            {
                let keeper = field.players.iter().find(|p| {
                    p.side == Some(side)
                        && !p.off_pitch
                        && p.tactical_position.current_position.is_goalkeeper()
                });
                if let Some(keeper) = keeper {
                    return Some(keeper.id);
                }
            }
        }
        let team = field
            .ball
            .current_owner
            .and_then(|id| field.get_player(id))
            .map(|p| p.team_id)
            .or(field.ball.last_touch_team_id)?;
        field
            .players
            .iter()
            .filter(|p| p.team_id == team && !p.off_pitch && p.injury.is_none())
            .min_by(|a, b| {
                let da = (a.position - spot).magnitude_squared();
                let db = (b.position - spot).magnitude_squared();
                da.total_cmp(&db)
            })
            .map(|p| p.id)
    }
}
