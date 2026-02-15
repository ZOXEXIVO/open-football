#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MatchState {
    Initial,
    FirstHalf,
    HalfTime,
    SecondHalf,
    ExtraTime,
    PenaltyShootout,
    End,
}

impl MatchState {
    pub fn need_swap_squads(&self) -> bool {
        match *self {
            MatchState::HalfTime => true,
            _ => false,
        }
    }
}
