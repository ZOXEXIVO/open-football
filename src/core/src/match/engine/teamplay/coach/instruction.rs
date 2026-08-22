//! **What an instruction means.** [`CoachInstruction`] — the high-level
//! tempo call the coach holds — and [`InstructionCoefficients`], the
//! table that turns it into the decision biases the passing / shooting /
//! movement scorers consume.
//!
//! Pure lookup: nothing here reads the match. [`MatchCoach`](super::match_coach::MatchCoach)
//! decides WHICH instruction is held; this decides what holding it does.

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
            // PushForward / AllOutAttack lifts roughly halved
            // (2026-06 regime neutralization): the scoring-rate-by-
            // game-state instrument measured trailing teams scoring at
            // 2.35 goals/90 vs leading 1.08 (real football: the three
            // states are nearly equal) — the stacked chase lifts
            // (these coefficients + tactical risk lift + late shape
            // changes) made the desperate siege far MORE productive
            // than settled play, which is backwards: real late sieges
            // against a set deep block are mostly huff. The volume
            // push stays visible (line up, tempo up) but no longer
            // outscores normal football.
            CoachInstruction::PushForward => Self {
                risk_appetite: 0.10,
                tempo: 0.10,
                defensive_line_units: 7.0,
                width_units: 3.0,
            },
            CoachInstruction::AllOutAttack => Self {
                risk_appetite: 0.10,
                tempo: 0.10,
                defensive_line_units: 8.0,
                width_units: 5.0,
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
