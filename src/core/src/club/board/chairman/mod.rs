//! The chairman: the person in the room whose temperament decides how the
//! board's powers are actually used.
//!
//! Two knobs — how much he wants and how long he will wait — plus how
//! personally attached he is to the manager currently in the job. The
//! richer governance model lives in [`super::ownership`]; this is the
//! human in front of it.

/// Ownership personality — a simplified chairman archetype whose traits
/// shape how the board actually exercises its powers. Two knobs, each
/// with meaningful consequences downstream of board.simulate().
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ChairmanAmbition {
    #[default]
    Balanced,
    /// "We want the Champions League." Budget skew +, expectations +.
    Ambitious,
    /// Sugar daddy / oil money. Budget skew ++, expectations ++,
    /// but also trigger-happy when results slip.
    Reckless,
    /// Old-money prudent. Budget skew -, stability prized.
    Conservative,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ChairmanPatience {
    #[default]
    Medium,
    /// Results yesterday. Sacking threshold is one bad run away.
    Low,
    /// Long-term project builder, trusts the process.
    High,
}

#[derive(Debug, Clone, Default)]
pub struct ChairmanProfile {
    pub ambition: ChairmanAmbition,
    pub patience: ChairmanPatience,
    /// 0..100 — how personally loyal the chairman is to the current manager.
    /// Rebuilt on each hire; decays with poor form, lifts with trophies.
    pub manager_loyalty: u8,
}

impl ChairmanProfile {
    pub fn new() -> Self {
        ChairmanProfile {
            ambition: ChairmanAmbition::Balanced,
            patience: ChairmanPatience::Medium,
            manager_loyalty: 50,
        }
    }

    /// Poor-mood-month threshold before patience snaps. Lower = quicker
    /// firing. High-loyalty chairmen buy their guy some extra time.
    pub fn poor_mood_threshold(&self) -> u8 {
        let base = match self.patience {
            ChairmanPatience::Low => 3,
            ChairmanPatience::Medium => 4,
            ChairmanPatience::High => 6,
        };
        // Loyal chairmen tolerate one extra poor month before acting.
        if self.manager_loyalty >= 70 {
            base + 1
        } else if self.manager_loyalty <= 20 {
            base.saturating_sub(1).max(1)
        } else {
            base
        }
    }

    /// Multiplier applied to the baseline transfer budget. Reckless owners
    /// push spend harder; conservative ones throttle it.
    pub fn budget_multiplier(&self) -> f32 {
        match self.ambition {
            ChairmanAmbition::Reckless => 1.4,
            ChairmanAmbition::Ambitious => 1.15,
            ChairmanAmbition::Balanced => 1.0,
            ChairmanAmbition::Conservative => 0.85,
        }
    }
}
