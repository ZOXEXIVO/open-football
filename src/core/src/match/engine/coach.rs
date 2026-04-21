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
            CoachInstruction::PushForward => -0.25, // meaningful shooting boost
            CoachInstruction::AllOutAttack => -0.45, // shoot from anything
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
    /// Tick when this team most recently gained possession. Used as a
    /// build-up gate: teams can't shoot within a short window of winning
    /// the ball, which forces an outlet pass / progression instead of
    /// hack-and-counter. Updated by the match loop on possession-change.
    pub last_possession_gain_tick: u64,
    /// Shots fired in the current possession. Reset when we lose the
    /// ball (possession change TO us, FROM us). Real football: one
    /// quality chance per possession. Rebound / tap-in scrambles
    /// (ball leaves owner briefly but team keeps control) don't
    /// count as a new possession — the cap holds until the opposition
    /// touches the ball.
    pub shots_this_possession: u32,
}

impl Default for MatchCoach {
    fn default() -> Self {
        MatchCoach {
            instruction: CoachInstruction::Normal,
            last_update_tick: 0,
            last_shot_tick: 0,
            last_possession_gain_tick: 0,
            shots_this_possession: 0,
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

        let _time_remaining = 1.0 - match_progress;
        let is_late_game = match_progress > 0.75;
        let is_very_late = match_progress > 0.88;
        let is_first_half_end = match_progress > 0.45 && match_progress < 0.55;
        let team_tired = avg_team_condition < 0.45;

        self.instruction = match score_diff {
            // Leading by 5+ goals — shut the game down. Previously even a
            // 0-6 leader stayed on `SlowDown` early in the match, which
            // still lets forwards take shots and convert against an
            // already-collapsing defence. `WasteTime` at any clock time
            // strips the attacking urge and keeps the ball at the back.
            d if d >= 5 => CoachInstruction::WasteTime,
            // Leading by 3-4 goals
            d if d >= 3 => {
                if is_late_game {
                    CoachInstruction::WasteTime
                } else {
                    CoachInstruction::SlowDown
                }
            }
            // Leading by 2 goals
            2 => {
                if is_very_late {
                    CoachInstruction::WasteTime
                } else if is_late_game {
                    CoachInstruction::SlowDown
                } else {
                    CoachInstruction::Normal
                }
            }
            // Leading by 1 goal — don't fully park the bus until the final 10min.
            // Parking too early creates 1-0 lock-ins that equalizers turn into draws.
            1 => {
                if is_very_late {
                    CoachInstruction::ParkTheBus
                } else if is_late_game {
                    CoachInstruction::SlowDown
                } else if is_first_half_end {
                    CoachInstruction::SlowDown
                } else if team_tired {
                    CoachInstruction::SlowDown
                } else {
                    CoachInstruction::Normal
                }
            }
            // Drawing — push for a winner from the 60th minute, all-out in final 10min
            0 => {
                if is_very_late {
                    CoachInstruction::AllOutAttack
                } else if is_late_game {
                    CoachInstruction::PushForward
                } else if team_tired {
                    CoachInstruction::SlowDown
                } else {
                    CoachInstruction::Normal
                }
            }
            // Losing by 1 — start pushing earlier to reduce draw lock-ins
            -1 => {
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
            // Losing by 3-4 — push hard, go all-out late
            d if d >= -4 => {
                if is_late_game {
                    CoachInstruction::AllOutAttack
                } else {
                    CoachInstruction::PushForward
                }
            }
            // Losing by 5+ — the game is gone. `AllOutAttack` from here
            // just kept conceding more because defenders pushed forward
            // into space the leader then counter-attacked through. Accept
            // the damage and hold shape (`PushForward` for chance creation
            // without gutting the back line).
            _ => CoachInstruction::PushForward,
        };
    }

    /// Whether the team should allow a shot right now (team-level cooldown).
    /// 500 ticks = 5 seconds between any shot by this team.
    ///
    /// Math: real football ≈ 13 shots / team / 90min = one shot every
    /// ~4100 ticks. A 500-tick floor caps team shots at 108 per match
    /// and lets the rate fall well below that when opportunities don't
    /// materialise. At 200 ticks the cap was 270, which the simulator
    /// was approaching in desperation-attack matches (one team losing
    /// 0-5 and spam-shooting from anywhere). At 500, a losing team
    /// can still fire ~100 times (plenty) but can't rebound-spam the
    /// same possession four times per second.
    pub fn can_shoot(&self, current_tick: u64) -> bool {
        // Per-team shot cadence — see type docs for the full rationale.
        let shot_spaced = current_tick.saturating_sub(self.last_shot_tick) >= 500;
        // Build-up gate: a team that just won possession can't fire
        // within ~1 second. Real football: even elite counter-attacks
        // need at least one progressive pass before a shot arrives.
        let settled = current_tick.saturating_sub(self.last_possession_gain_tick) >= 100;
        // Possession-phase shot cap: at most ONE shot per possession.
        // This is the single biggest natural-logic lever for shot
        // volume. Real football: a possession produces a chance OR a
        // turnover, not "three chances over seven seconds of box
        // scramble." The counter resets on possession change; rebounds
        // where the team keeps control DON'T count as a new possession,
        // so rebound-spam is naturally capped. Sets the realistic
        // "one good look per attack" rhythm.
        let phase_allows = self.shots_this_possession < 1;
        shot_spaced && settled && phase_allows
    }

    /// Record that this team just won possession. Starts the build-up
    /// gate AND resets the per-possession shot counter.
    pub fn record_possession_gain(&mut self, current_tick: u64) {
        self.last_possession_gain_tick = current_tick;
        self.shots_this_possession = 0;
    }

    pub fn record_shot(&mut self, current_tick: u64) {
        self.last_shot_tick = current_tick;
        self.shots_this_possession += 1;
    }
}
