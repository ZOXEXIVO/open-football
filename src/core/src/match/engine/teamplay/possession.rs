//! **Who holds the ball.** One answer per tick, the same for every player on
//! the pitch and for both team plans.
//!
//! Three definitions used to run side by side. `is_control_ball` was computed
//! per player, left out the man asking and turned on the ball's speed and
//! direction; `TeamTacticalState::in_possession` could not change hands on a
//! ball nobody touched; `DefensivePlan::refresh` called every unowned ball
//! both sides' to defend. Measured over 30 matches they flipped 61,560 times
//! a match, team-mates disagreed on 24% of unowned team-ticks, and marks were
//! handed to a side whose own pass was in the air and then rejected by the
//! state they sent the man to: `Defender: Running <-> Marking`, 34,455 flips
//! a match, the largest loop in the engine.

use crate::r#match::MatchField;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TeamPossession {
    holder: Option<u32>,
}

impl TeamPossession {
    /// How much closer, as a ratio of squared distances, the other side has
    /// to be before a loose ball is theirs: 0.7 of the distance.
    const DECISIVELY_CLOSER: f32 = 0.49;

    /// The answer for a field nobody has read before.
    pub fn read(field: &MatchField) -> Self {
        let mut possession = Self::default();
        possession.update(field);
        possession
    }

    /// Bring the answer up to date. Called once per tick, after the ball has
    /// moved and before anybody decides anything.
    pub fn update(&mut self, field: &MatchField) {
        let team_of = |id: u32| field.get_player(id).map(|p| p.team_id);
        let ball = &field.ball;
        let taker = ball
            .restart_in_progress()
            .map(|(_, taker)| taker)
            .or(ball.kickoff_taker);
        self.holder = ball
            .current_owner
            .and_then(team_of)
            .or_else(|| taker.and_then(team_of))
            .or_else(|| ball.pass_target_player_id.and_then(team_of))
            .or_else(|| Self::loose_ball_holder(field, self.holder));
    }

    pub fn holder(&self) -> Option<u32> {
        self.holder
    }

    #[inline]
    pub fn held_by(&self, team_id: u32) -> bool {
        self.holder == Some(team_id)
    }

    /// A ball nobody has and nobody was passing to stays with the side that
    /// last held it until the other side is decisively closer. With no side
    /// to keep it, the nearer one has it.
    fn loose_ball_holder(field: &MatchField, incumbent: Option<u32>) -> Option<u32> {
        let ball = field.ball.position;
        let mut sides: [Option<(u32, f32)>; 2] = [None, None];
        for player in field.players.iter().filter(|p| !p.off_pitch) {
            let distance_sq = (player.position - ball).norm_squared();
            let side = sides
                .iter_mut()
                .find(|side| side.is_none_or(|(team, _)| team == player.team_id));
            if let Some(side) = side {
                let nearest = side.map_or(distance_sq, |(_, best)| best.min(distance_sq));
                *side = Some((player.team_id, nearest));
            }
        }
        let [Some(a), Some(b)] = sides else {
            return sides[0].map(|(team, _)| team).or(incumbent);
        };
        let (near, far) = if a.1 <= b.1 { (a, b) } else { (b, a) };
        match incumbent {
            Some(team) if team != near.0 && near.1 >= far.1 * Self::DECISIVELY_CLOSER => Some(team),
            _ => Some(near.0),
        }
    }
}
