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
        matches!(
            self,
            CoachInstruction::SlowDown | CoachInstruction::WasteTime | CoachInstruction::ParkTheBus
        )
    }
}

/// Coefficients applied by an instruction to player decision biases.
/// All deltas are additive and the resulting bias is consumed by the
/// passing / shooting / movement scorers. Centralised so the table can
/// be edited in one place and the scorers stay readable.
#[derive(Debug, Clone, Copy)]
pub struct InstructionCoefficients {
    pub risk_appetite: f32,
    pub tempo: f32,
    pub defensive_line_units: f32,
    pub width_units: f32,
}

impl InstructionCoefficients {
    pub fn for_instruction(i: CoachInstruction) -> Self {
        match i {
            CoachInstruction::Normal => Self {
                risk_appetite: 0.0,
                tempo: 0.0,
                defensive_line_units: 0.0,
                width_units: 0.0,
            },
            CoachInstruction::SlowDown => Self {
                risk_appetite: -0.16,
                tempo: -0.14,
                defensive_line_units: -10.0,
                width_units: -3.0,
            },
            CoachInstruction::PushForward => Self {
                risk_appetite: 0.18,
                tempo: 0.14,
                defensive_line_units: 12.0,
                width_units: 4.0,
            },
            CoachInstruction::AllOutAttack => Self {
                risk_appetite: 0.34,
                tempo: 0.22,
                defensive_line_units: 24.0,
                width_units: 8.0,
            },
            CoachInstruction::WasteTime => Self {
                risk_appetite: -0.30,
                tempo: -0.26,
                defensive_line_units: -20.0,
                width_units: -2.0,
            },
            CoachInstruction::ParkTheBus => Self {
                risk_appetite: -0.24,
                tempo: -0.10,
                defensive_line_units: -35.0,
                width_units: -5.0,
            },
        }
    }
}

/// Rolling team metrics consumed by the smarter coach evaluator. The
/// match loop is responsible for keeping these up to date (sliding
/// window — minute-window data trimmed every tick).
#[derive(Debug, Clone, Copy, Default)]
pub struct RollingTeamMetrics {
    pub xg_for_last_15: f32,
    pub xg_against_last_15: f32,
    pub shots_for_last_15: u16,
    pub deep_entries_for_last_15: u16,
    /// Possession-by-position metric (0.0..1.0): fraction of recent ticks
    /// where the ball was in the opposition half.
    pub field_tilt_last_10: f32,
    pub possession_last_10: f32,
    pub dangerous_turnovers_last_10: u16,
    /// Successful pressures / total pressures in the last 10 minutes.
    pub press_success_rate_last_10: f32,
    /// Rolling average of how many times the opposition played through
    /// our defensive line (per minute, last 10).
    pub avg_defensive_line_breaks: f32,
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
    /// Rolling tactical metrics — populated by the match loop and read
    /// by `evaluate_with_metrics` for smarter instruction switches.
    pub metrics: RollingTeamMetrics,
}

impl Default for MatchCoach {
    fn default() -> Self {
        MatchCoach {
            instruction: CoachInstruction::Normal,
            last_update_tick: 0,
            last_shot_tick: 0,
            last_possession_gain_tick: 0,
            shots_this_possession: 0,
            metrics: RollingTeamMetrics::default(),
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
        score_diff: i8,          // positive = leading, negative = losing
        match_progress: f32,     // 0.0 = start, 1.0 = end of match
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
        // Possession-phase shot cap: at most TWO shots per possession.
        // Real football: a possession typically produces ONE chance,
        // but rebounds (saved/blocked → ball comes back to attackers)
        // are a real and common path to goals — Klopp-era Liverpool
        // and most pressing teams convert plenty from rebounds.
        // Cap of 1 forbade ALL rebound shots, including legitimate ones
        // where the GK parries to a striker's feet. Cap of 2 paired
        // with the 5s team-shot cooldown still rules out box-scramble
        // spam (4 shots in 2s) but unlocks the realistic "shoot →
        // parry → tap-in" pattern.
        let phase_allows = self.shots_this_possession < 2;
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

    /// Returns the active instruction's tactical coefficients (risk,
    /// tempo, defensive-line, width). Consumers use these to bias
    /// scoring decisions without needing to match on `CoachInstruction`
    /// at every call site.
    pub fn coefficients(&self) -> InstructionCoefficients {
        InstructionCoefficients::for_instruction(self.instruction)
    }

    /// xG/territory-aware variant of `evaluate`. Falls back to the
    /// classic score/time/condition logic and then upgrades or
    /// downgrades the choice based on rolling metrics. Real football:
    /// a 0-0 team dominating xG shouldn't go AllOutAttack; a leading
    /// team being tilted should drop deeper rather than just slow down.
    pub fn evaluate_with_metrics(
        &mut self,
        score_diff: i8,
        match_progress: f32,
        avg_team_condition: f32,
        current_tick: u64,
        metrics: RollingTeamMetrics,
    ) {
        self.evaluate(score_diff, match_progress, avg_team_condition, current_tick);
        self.metrics = metrics;

        let xg_diff_15 = metrics.xg_for_last_15 - metrics.xg_against_last_15;
        let is_late = match_progress > 0.66;
        let is_very_late = match_progress > 0.83;

        // Drawing but dominating xG → don't blow the shape. Stay on
        // PushForward (or Normal) instead of AllOutAttack.
        if score_diff == 0 && is_late && xg_diff_15 >= 0.7 {
            if matches!(self.instruction, CoachInstruction::AllOutAttack) {
                self.instruction = CoachInstruction::PushForward;
            }
        }

        // Drawing late and getting outxG'd badly → push harder than the
        // base evaluator decided.
        if score_diff == 0 && is_very_late && xg_diff_15 <= -0.5 {
            self.instruction = CoachInstruction::AllOutAttack;
        }

        // Leading by 1 late but conceding heavy xG → switch from
        // WasteTime/SlowDown to a compact mid/low block (we approximate
        // "compact mid block" with ParkTheBus's posture but only after
        // 75').
        if score_diff == 1
            && match_progress > 0.83
            && metrics.xg_against_last_15 > 0.6
            && matches!(
                self.instruction,
                CoachInstruction::WasteTime | CoachInstruction::SlowDown
            )
        {
            self.instruction = CoachInstruction::ParkTheBus;
        }

        // Leading by 2+ but heavily field-tilted by opponent → don't be
        // passive; hold ball with SlowDown (allow safer outlet passes)
        // instead of WasteTime.
        if score_diff >= 2
            && metrics.field_tilt_last_10 > 0.65
            && matches!(self.instruction, CoachInstruction::WasteTime)
        {
            self.instruction = CoachInstruction::SlowDown;
        }

        // Failing press → drop the line. Captured here as switching
        // from PushForward / AllOutAttack to Normal when the pressing
        // isn't producing turnovers and the team is tired.
        if metrics.press_success_rate_last_10 < 0.35
            && avg_team_condition < 0.55
            && matches!(
                self.instruction,
                CoachInstruction::AllOutAttack | CoachInstruction::PushForward
            )
        {
            self.instruction = CoachInstruction::Normal;
        }
    }
}

/// Substitution candidate scoring (Section 6). Lives here so the coach
/// state is the single source of truth for "what does this team want
/// right now". The substitutions module reads `tactical_need_for` to
/// pick which position group to bring on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TacticalNeed {
    /// Trailing late — bring on attackers for goals.
    Chasing,
    /// Leading and absorbing — bring on defenders / DM.
    ProtectingLead,
    /// Outpassed in midfield — bring on a CM/DM with passing/vision.
    LosingMidfield,
    /// Being pressed off the ball — composure / first touch / passing.
    BeingPressed,
    /// Need crosses / wing service.
    NeedingCrosses,
    /// No urgent need — fatigue rotation only.
    Fatigue,
}

impl TacticalNeed {
    /// Decide the most pressing tactical need for a team given match
    /// state and rolling metrics. Order matters — the first match wins.
    pub fn from_state(
        score_diff: i8,
        match_progress: f32,
        avg_team_condition: f32,
        metrics: RollingTeamMetrics,
    ) -> Self {
        let late = match_progress > 0.66;
        if late && score_diff < 0 {
            return TacticalNeed::Chasing;
        }
        if late && score_diff > 0 && metrics.field_tilt_last_10 > 0.55 {
            return TacticalNeed::ProtectingLead;
        }
        if metrics.possession_last_10 < 0.42 && metrics.dangerous_turnovers_last_10 >= 3 {
            return TacticalNeed::LosingMidfield;
        }
        if metrics.dangerous_turnovers_last_10 >= 4 || avg_team_condition < 0.40 {
            return TacticalNeed::BeingPressed;
        }
        if score_diff <= 0 && match_progress > 0.55 && metrics.shots_for_last_15 < 2 {
            return TacticalNeed::NeedingCrosses;
        }
        TacticalNeed::Fatigue
    }
}
