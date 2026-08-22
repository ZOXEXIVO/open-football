//! The pitch as the engine sees it: how big it is, which half the ball is
//! in, and the tactics each end is playing.

use crate::r#match::field::MatchField;
use crate::Tactics;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum BallSide {
    Left,
    Right,
}

impl From<BallSide> for u8 {
    fn from(side: BallSide) -> Self {
        match side {
            BallSide::Left => 0,
            BallSide::Right => 1,
        }
    }
}

#[derive(Clone)]
pub struct TeamsTactics {
    pub left: Tactics,
    pub right: Tactics,
}

impl TeamsTactics {
    pub fn from_field(field: &MatchField) -> Self {
        TeamsTactics {
            left: field.left_team_tactics.clone(),
            right: field.right_team_tactics.clone(),
        }
    }
}

#[derive(Clone)]
pub struct MatchFieldSize {
    pub width: usize,
    pub height: usize,

    pub half_width: usize,
}

impl MatchFieldSize {
    pub fn new(width: usize, height: usize) -> Self {
        MatchFieldSize {
            width,
            height,
            half_width: width / 2,
        }
    }
}
