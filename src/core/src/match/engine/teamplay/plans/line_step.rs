//! **The offside trap.** A back line that reads a ball about to be played
//! in behind it can step up together and leave the runner offside.
//!
//! The read is the team's: one call, made when the picture is there — an
//! opponent on the ball with room to play it, a flat line outside its own
//! area, and a runner on the shoulder of the last defender. Whether each
//! man goes with it is his own (`LineStep` on the defender side), and the
//! one who reads it late stays where he was and plays the runner on.

use crate::PlayerFieldPositionGroup;
use crate::r#match::engine::ball::ball::OffsideLine;
use crate::r#match::{MatchField, MatchPlayer};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LineStepCall {
    /// When the line read it and was called up.
    pub called_tick: u64,
    /// The runner on its shoulder it means to leave behind.
    pub runner: u32,
}

impl LineStepCall {
    /// 2 m either side of the line: a runner here is on its shoulder.
    const SHOULDER: f32 = 16.0;
    /// 40 m: further from the runner than this the carrier has no ball to
    /// play him in with.
    const PASS_RANGE: f32 = 320.0;
    /// 10 m: a carrier nearer the line than this is already on it, and the
    /// line has to deal with him rather than step past his runner.
    const CARRIER_ROOM: f32 = 80.0;
    /// 16.5 m: a line inside its own area does not step anywhere.
    const OWN_AREA: f32 = 132.0;
    /// 3 m: a back four strung out deeper than this is not a line that
    /// can step as one.
    const FLAT: f32 = 24.0;
    /// How organised a line has to be to try it at all.
    pub const ORGANISED: f32 = 0.55;

    /// How organised a back line is, 0..1: how well its men read the game,
    /// how used the side is to its shape, and the keeper behind it.
    pub fn organisation(anticipation: f32, familiarity: f32, keeper_voice: f32) -> f32 {
        ((anticipation / 20.0).clamp(0.0, 1.0) * 0.5
            + familiarity.clamp(0.0, 1.0) * 0.3
            + keeper_voice.clamp(0.0, 1.0) * 0.2)
            .clamp(0.0, 1.0)
    }

    /// The call for `team_id`'s line now, the tick carried over from
    /// `previous` while it is the same runner.
    pub fn read(
        field: &MatchField,
        team_id: u32,
        organisation: f32,
        previous: Option<Self>,
        tick: u64,
    ) -> Option<Self> {
        if organisation < Self::ORGANISED {
            return None;
        }
        let carrier = field
            .ball
            .current_owner
            .and_then(|id| field.players.iter().find(|p| p.id == id))
            .filter(|p| p.team_id != team_id && !field.ball.held_in_hands)?;
        let attacking = carrier.side?;
        let forward = attacking.forward_dir_x();
        let ours = || {
            field
                .players
                .iter()
                .filter(move |p| p.team_id == team_id && !p.off_pitch)
        };
        let line = OffsideLine::second_last(ours().map(|p| p.position.x), attacking)?;
        let our_goal_x = if forward > 0.0 {
            field.size.width as f32
        } else {
            0.0
        };
        if (our_goal_x - line).abs() < Self::OWN_AREA
            || (line - carrier.position.x) * forward < Self::CARRIER_ROOM
        {
            return None;
        }
        let strung_out = ours()
            .filter(|p| Self::in_back_line(p))
            .any(|p| ((p.position.x - line) * forward).abs() > Self::FLAT);
        if strung_out {
            return None;
        }
        let runner = field
            .players
            .iter()
            .filter(|p| p.team_id != team_id && p.id != carrier.id && !p.off_pitch)
            .filter(|p| {
                let beyond = (p.position.x - line) * forward;
                beyond.abs() <= Self::SHOULDER
                    && (p.position - carrier.position).norm() <= Self::PASS_RANGE
            })
            .max_by(|a, b| (a.position.x * forward).total_cmp(&(b.position.x * forward)))?;
        let called_tick = previous
            .filter(|call| call.runner == runner.id)
            .map_or(tick, |call| call.called_tick);
        Some(Self {
            called_tick,
            runner: runner.id,
        })
    }

    /// The back four: defenders, without the man screening in front of it.
    fn in_back_line(player: &MatchPlayer) -> bool {
        let position = player.tactical_position.current_position;
        position.position_group() == PlayerFieldPositionGroup::Defender
            && !position.is_defensive_midfielder()
    }
}
