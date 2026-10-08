#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MatchState {
    Initial,
    FirstHalf,
    HalfTime,
    SecondHalf,
    ExtraTimeFirst,
    ExtraTimeInterval,
    ExtraTimeSecond,
    PenaltyShootout,
    End,
}

impl MatchState {
    /// The break after which the teams change ends.
    pub fn need_swap_squads(&self) -> bool {
        self.is_interval()
    }

    /// A break in which no football is played, and a change is free.
    pub fn is_interval(&self) -> bool {
        matches!(self, MatchState::HalfTime | MatchState::ExtraTimeInterval)
    }

    /// A period the clock runs in.
    pub fn is_timed(&self) -> bool {
        matches!(
            self,
            MatchState::FirstHalf
                | MatchState::SecondHalf
                | MatchState::ExtraTimeFirst
                | MatchState::ExtraTimeSecond
        )
    }

    pub fn is_extra_time(&self) -> bool {
        matches!(self, MatchState::ExtraTimeFirst | MatchState::ExtraTimeSecond)
    }
}
