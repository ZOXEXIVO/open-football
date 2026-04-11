//! Individual player instructions (FM-style per-slot overrides).
//!
//! A tactic picks a formation and team-wide style; player instructions
//! allow the manager to tweak how a specific player inside that formation
//! should play. Match-engine decision weights can read these overrides to
//! tilt behaviour (e.g. "stay wider" shifts the player's average x target
//! toward the touchline, "shoot less often" dampens shot decisions).

use crate::club::PlayerPositionType;

/// Per-slot individual instructions keyed by the formation slot position.
#[derive(Debug, Clone, Default)]
pub struct IndividualInstructions {
    pub slots: Vec<SlotInstructions>,
}

#[derive(Debug, Clone)]
pub struct SlotInstructions {
    /// Which formation slot these apply to.
    pub slot: PlayerPositionType,
    /// Positional width bias (-1.0 tuck in .. 0.0 normal .. 1.0 hug line).
    pub width: f32,
    /// Depth bias (-1.0 drop deeper .. 0.0 normal .. 1.0 push higher).
    pub depth: f32,
    /// Shoot frequency override (-1.0 shoot less .. 0.0 normal .. 1.0 shoot more).
    pub shoot_frequency: f32,
    /// Risk tolerance on passes (-1.0 safe .. 0.0 normal .. 1.0 risky).
    pub pass_risk: f32,
    /// Tackle aggression (-1.0 stay on feet .. 0.0 normal .. 1.0 dive in).
    pub tackle_aggression: f32,
    /// Does this player close down? (-1.0 hold line .. 0.0 normal .. 1.0 press).
    pub closing_down: f32,
    /// Mark a specific opposing player tightly (opponent player id).
    pub mark_opponent: Option<u32>,
    /// Role override — e.g. for a midfielder to play as a deep-lying playmaker.
    pub role_override: Option<PlayerRole>,
}

impl SlotInstructions {
    pub fn default_for(slot: PlayerPositionType) -> Self {
        Self {
            slot,
            width: 0.0,
            depth: 0.0,
            shoot_frequency: 0.0,
            pass_risk: 0.0,
            tackle_aggression: 0.0,
            closing_down: 0.0,
            mark_opponent: None,
            role_override: None,
        }
    }
}

/// FM-style role names. These are advisory — the match engine translates
/// them into decision weights via the slot instruction values above.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerRole {
    // Defenders
    BallPlayingDefender,
    LimitedDefender,
    NoNonsenseCentreBack,
    Libero,
    FullBack,
    WingBack,
    CompleteWingBack,
    InvertedFullBack,
    // Midfielders
    DeepLyingPlaymaker,
    BoxToBox,
    Anchor,
    BallWinningMidfielder,
    AdvancedPlaymaker,
    Mezzala,
    Regista,
    // Wide
    Winger,
    InvertedWinger,
    WideMidfielder,
    WidePlaymaker,
    // Forwards
    AdvancedForward,
    DeepLyingForward,
    TargetMan,
    Poacher,
    CompleteForward,
    FalseNine,
    TrequartistaNum10,
    // GK
    Sweeper,
    Goalkeeper,
}

impl IndividualInstructions {
    pub fn for_slot(&self, slot: PlayerPositionType) -> Option<&SlotInstructions> {
        self.slots.iter().find(|s| s.slot == slot)
    }

    pub fn upsert(&mut self, instructions: SlotInstructions) {
        if let Some(idx) = self.slots.iter().position(|s| s.slot == instructions.slot) {
            self.slots[idx] = instructions;
        } else {
            self.slots.push(instructions);
        }
    }
}
