//! Manager–board relationship. Replaces the single `manager_loyalty`
//! drift with five trust facets so the board can be happy with results
//! yet uneasy about finances, or back a manager through a bad run because
//! the squad-building and communication are sound. The overall blend is
//! mirrored back into `ChairmanProfile.manager_loyalty` so legacy callers
//! keep working.

use super::scoring::BoardComponentScores;

#[derive(Debug, Clone)]
pub struct ManagerRelationship {
    /// Trust earned through on-pitch results vs expectations.
    pub trust_results: u8,
    /// Trust in the manager's financial discipline (wages, fees).
    pub trust_finances: u8,
    /// Trust in squad construction — age profile, depth, recruitment.
    pub trust_squad_building: u8,
    /// Trust built through promises kept and board-meeting conduct.
    pub trust_communication: u8,
    /// How closely the manager's football matches the board's vision.
    pub style_alignment: u8,
}

impl Default for ManagerRelationship {
    fn default() -> Self {
        // Fresh appointment — cautious optimism across the board.
        ManagerRelationship {
            trust_results: 55,
            trust_finances: 55,
            trust_squad_building: 55,
            trust_communication: 55,
            style_alignment: 55,
        }
    }
}

impl ManagerRelationship {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reset to the new-appointment baseline (called when a manager is
    /// hired / sacked).
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    fn nudge(value: &mut u8, delta: f32) {
        let v = (*value as f32 + delta).clamp(0.0, 100.0);
        *value = v as u8;
    }

    /// Drift each facet towards the latest component scores. Component
    /// scores are roughly -40..40; we map them onto small monthly nudges
    /// so trust moves believably rather than snapping.
    pub fn update_from_scores(&mut self, scores: &BoardComponentScores, style_alignment_drag: i32) {
        Self::nudge(&mut self.trust_results, scores.sporting * 0.10);
        Self::nudge(&mut self.trust_finances, scores.financial * 0.10);
        Self::nudge(&mut self.trust_squad_building, scores.squad_building * 0.10);
        Self::nudge(&mut self.trust_communication, scores.strategy * 0.08);
        // Style alignment: drag is 0 (perfect) .. 2 (clash). Convert to a
        // small signed nudge — good fit slowly builds, a clash erodes.
        let style_delta = 1.5 - style_alignment_drag as f32;
        Self::nudge(&mut self.style_alignment, style_delta);
    }

    /// Apply a direct trust adjustment to communication (promise kept /
    /// broken, meeting outcome).
    pub fn adjust_communication(&mut self, delta: i32) {
        Self::nudge(&mut self.trust_communication, delta as f32);
    }

    /// Blended 0..100 loyalty equivalent. Results weigh heaviest — a board
    /// forgives a lot if the team is winning.
    pub fn overall_trust(&self) -> u8 {
        let blend = self.trust_results as f32 * 0.34
            + self.trust_finances as f32 * 0.18
            + self.trust_squad_building as f32 * 0.18
            + self.trust_communication as f32 * 0.15
            + self.style_alignment as f32 * 0.15;
        blend.clamp(0.0, 100.0) as u8
    }

    /// True when trust is high enough across the board to justify a
    /// contract renewal offer. Needs strong results *and* no facet in
    /// crisis.
    pub fn merits_renewal(&self) -> bool {
        self.overall_trust() >= 68
            && self.trust_results >= 60
            && self.trust_finances >= 45
            && self.trust_communication >= 45
    }

    /// True when the board–manager bond has broken down enough that
    /// dismissal is justified on relationship grounds (independent of the
    /// raw form-based sacking gate).
    pub fn relationship_breakdown(&self) -> bool {
        self.overall_trust() <= 25 || (self.trust_results <= 20 && self.trust_communication <= 30)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scores(s: f32, f: f32, sq: f32, st: f32) -> BoardComponentScores {
        BoardComponentScores {
            sporting: s,
            financial: f,
            squad_building: sq,
            strategy: st,
        }
    }

    #[test]
    fn strong_results_build_trust() {
        let mut rel = ManagerRelationship::new();
        let before = rel.trust_results;
        for _ in 0..6 {
            rel.update_from_scores(&scores(35.0, 10.0, 10.0, 10.0), 0);
        }
        assert!(rel.trust_results > before);
    }

    #[test]
    fn sustained_high_trust_merits_renewal() {
        let mut rel = ManagerRelationship::new();
        for _ in 0..12 {
            rel.update_from_scores(&scores(40.0, 30.0, 30.0, 30.0), 0);
        }
        assert!(rel.merits_renewal());
    }

    #[test]
    fn collapse_triggers_breakdown() {
        let mut rel = ManagerRelationship::new();
        for _ in 0..18 {
            rel.update_from_scores(&scores(-40.0, -30.0, -30.0, -30.0), 2);
        }
        assert!(rel.relationship_breakdown());
    }
}
