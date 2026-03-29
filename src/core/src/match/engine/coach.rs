/// Coach in-match instructions that control team tempo and behavior.
///
/// The coach evaluates score, time, and fatigue every few seconds and issues
/// instructions that all players consult when making decisions.

/// High-level tempo instruction from the coach
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CoachInstruction {
    /// Normal play - balanced attack/defense
    Normal,
    /// Slow tempo - keep possession, pass back, let team rest
    SlowDown,
    /// Push forward - more direct play, take risks
    PushForward,
    /// All-out attack - overload offense, abandon defensive shape
    AllOutAttack,
    /// Time wasting - hold ball in defense, slow everything down
    WasteTime,
    /// Park the bus - deep defensive block, clear ball, counter only
    ParkTheBus,
}

impl Default for CoachInstruction {
    fn default() -> Self {
        CoachInstruction::Normal
    }
}

impl CoachInstruction {
    /// How much this instruction discourages shooting (0.0 = no effect, 1.0 = never shoot)
    pub fn shooting_reluctance(&self) -> f32 {
        match self {
            CoachInstruction::Normal => 0.0,
            CoachInstruction::SlowDown => 0.3,
            CoachInstruction::PushForward => -0.1, // slightly encourages
            CoachInstruction::AllOutAttack => -0.2,
            CoachInstruction::WasteTime => 0.6,
            CoachInstruction::ParkTheBus => 0.4,
        }
    }

    /// How much this instruction encourages passing backward (0.0 = no effect, 1.0 = always back)
    pub fn backward_pass_preference(&self) -> f32 {
        match self {
            CoachInstruction::Normal => 0.0,
            CoachInstruction::SlowDown => 0.4,
            CoachInstruction::PushForward => -0.2,
            CoachInstruction::AllOutAttack => -0.3,
            CoachInstruction::WasteTime => 0.7,
            CoachInstruction::ParkTheBus => 0.5,
        }
    }

    /// Speed multiplier for player movement (1.0 = normal)
    pub fn tempo_multiplier(&self) -> f32 {
        match self {
            CoachInstruction::Normal => 1.0,
            CoachInstruction::SlowDown => 0.82,
            CoachInstruction::PushForward => 1.05,
            CoachInstruction::AllOutAttack => 1.1,
            CoachInstruction::WasteTime => 0.7,
            CoachInstruction::ParkTheBus => 0.85,
        }
    }

    /// Minimum ticks a player should hold ball before passing (encourages slow build-up)
    pub fn min_possession_ticks(&self) -> u32 {
        match self {
            CoachInstruction::Normal => 8,
            CoachInstruction::SlowDown => 25,
            CoachInstruction::PushForward => 5,
            CoachInstruction::AllOutAttack => 3,
            CoachInstruction::WasteTime => 40,
            CoachInstruction::ParkTheBus => 10,
        }
    }

    /// Whether players should prefer keeping possession over attacking
    pub fn prefer_possession(&self) -> bool {
        matches!(self, CoachInstruction::SlowDown | CoachInstruction::WasteTime | CoachInstruction::ParkTheBus)
    }
}

/// Per-team coach state during a match
#[derive(Debug, Clone)]
pub struct MatchCoach {
    pub instruction: CoachInstruction,
    /// Tick when instruction was last updated
    pub last_update_tick: u64,
    /// Team's last shot tick (for team-wide shot cooldown)
    pub last_shot_tick: u64,
}

impl Default for MatchCoach {
    fn default() -> Self {
        MatchCoach {
            instruction: CoachInstruction::Normal,
            last_update_tick: 0,
            last_shot_tick: 0,
        }
    }
}

impl MatchCoach {
    pub fn new() -> Self {
        Self::default()
    }

    /// Evaluate match state and decide what instruction to give.
    /// Called periodically (every ~500 ticks = ~5 seconds).
    pub fn evaluate(
        &mut self,
        score_diff: i8, // positive = leading, negative = losing
        match_progress: f32, // 0.0 = start, 1.0 = end of match
        avg_team_condition: f32, // 0.0-1.0
        current_tick: u64,
    ) {
        self.last_update_tick = current_tick;

        let time_remaining = 1.0 - match_progress;
        let is_late_game = match_progress > 0.75;
        let is_very_late = match_progress > 0.88;
        let is_first_half_end = match_progress > 0.45 && match_progress < 0.55;
        let team_tired = avg_team_condition < 0.45;

        self.instruction = match score_diff {
            // Leading by 3+ goals
            d if d >= 3 => {
                if is_late_game {
                    CoachInstruction::WasteTime
                } else if team_tired {
                    CoachInstruction::SlowDown
                } else {
                    CoachInstruction::SlowDown
                }
            }
            // Leading by 2 goals
            2 => {
                if is_very_late {
                    CoachInstruction::WasteTime
                } else if is_late_game {
                    CoachInstruction::ParkTheBus
                } else if team_tired {
                    CoachInstruction::SlowDown
                } else {
                    CoachInstruction::SlowDown
                }
            }
            // Leading by 1 goal
            1 => {
                if is_very_late {
                    CoachInstruction::WasteTime
                } else if is_late_game {
                    CoachInstruction::ParkTheBus
                } else if is_first_half_end {
                    CoachInstruction::SlowDown
                } else if team_tired {
                    CoachInstruction::SlowDown
                } else {
                    CoachInstruction::Normal
                }
            }
            // Drawing
            0 => {
                if is_very_late {
                    // Late draw - push for winner
                    CoachInstruction::PushForward
                } else if team_tired {
                    CoachInstruction::SlowDown
                } else {
                    CoachInstruction::Normal
                }
            }
            // Losing by 1
            -1 => {
                if is_very_late {
                    CoachInstruction::AllOutAttack
                } else if is_late_game {
                    CoachInstruction::PushForward
                } else {
                    CoachInstruction::Normal
                }
            }
            // Losing by 2
            -2 => {
                if is_very_late {
                    CoachInstruction::AllOutAttack
                } else if is_late_game {
                    CoachInstruction::AllOutAttack
                } else if match_progress > 0.55 {
                    CoachInstruction::PushForward
                } else {
                    CoachInstruction::Normal
                }
            }
            // Losing by 3+
            _ => {
                if is_late_game {
                    CoachInstruction::AllOutAttack
                } else {
                    CoachInstruction::PushForward
                }
            }
        };
    }

    /// Whether the team should allow a shot right now (team-level cooldown)
    pub fn can_shoot(&self, current_tick: u64) -> bool {
        // Minimum 50 ticks (~0.5 seconds) between team shots
        current_tick.saturating_sub(self.last_shot_tick) >= 50
    }

    pub fn record_shot(&mut self, current_tick: u64) {
        self.last_shot_tick = current_tick;
    }
}
