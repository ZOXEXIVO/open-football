//! Set piece designation and execution helpers.
//!
//! A team chooses its corner, free-kick and penalty takers from the
//! available squad, biased by Technique / Crossing / Finishing / Long Shots
//! / Penalty Taking. The match engine can consult `SetPieceSetup` when a
//! set-piece event fires.

use crate::club::player::Player;
use crate::club::PlayerPositionType;

#[derive(Debug, Clone, Default)]
pub struct SetPieceSetup {
    pub corner_taker: Option<u32>,
    pub left_corner_taker: Option<u32>,
    pub right_corner_taker: Option<u32>,
    pub free_kick_taker: Option<u32>,
    pub long_free_kick_taker: Option<u32>,
    pub penalty_taker: Option<u32>,
    /// Designated penalty order for shootouts — up to 11 takers.
    pub penalty_order: Vec<u32>,
    /// Corner/FK routine preference.
    pub corner_routine: CornerRoutine,
    pub defensive_set_piece: DefensiveSetPiece,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CornerRoutine {
    /// Standard delivery into the box, tallest players attack first ball.
    #[default]
    Mixed,
    /// In-swinging delivery toward near post.
    NearPost,
    /// Out-swinging delivery to the edge of the box.
    FarPost,
    /// Short corner routine with midfielder drop.
    Short,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DefensiveSetPiece {
    /// Everyone marks zones near the box.
    #[default]
    Zonal,
    /// Man-mark the opposition aerial threats.
    ManToMan,
    /// Hybrid — zonal front posts, man-mark key threats.
    Mixed,
}

impl SetPieceSetup {
    /// Compute an ideal set piece setup from the 11 starters.
    /// Skips goalkeepers (unless nobody else is available).
    pub fn choose(starters: &[&Player]) -> Self {
        let outfield: Vec<&&Player> = starters
            .iter()
            .filter(|p| !matches!(p.position(), PlayerPositionType::Goalkeeper))
            .collect();

        let pool: &[&&Player] = if outfield.is_empty() {
            // shouldn't happen in practice; degrade gracefully
            return SetPieceSetup::default();
        } else {
            &outfield
        };

        let score = |score_fn: fn(&Player) -> f32| -> Option<u32> {
            pool.iter()
                .max_by(|a, b| {
                    score_fn(a).partial_cmp(&score_fn(b)).unwrap_or(std::cmp::Ordering::Equal)
                })
                .map(|p| p.id)
        };

        let corner_taker = score(|p| {
            let t = &p.skills.technical;
            t.crossing * 0.6 + t.technique * 0.3 + t.corners * 0.1
        });

        let penalty_taker = score(|p| {
            let t = &p.skills.technical;
            let m = &p.skills.mental;
            t.penalty_taking * 0.5 + t.finishing * 0.2 + t.technique * 0.15
                + (20.0 - m.pressure_handling(&p.attributes)) * 0.15
        });

        let free_kick_taker = score(|p| {
            let t = &p.skills.technical;
            t.free_kicks * 0.5 + t.technique * 0.25 + t.crossing * 0.15 + t.long_shots * 0.1
        });

        let long_free_kick_taker = score(|p| {
            let t = &p.skills.technical;
            t.long_shots * 0.5 + t.free_kicks * 0.3 + t.technique * 0.2
        });

        // Shootout order: best finisher first, then penalty specialists,
        // then other technical players. Up to 11 takers.
        let mut ordered: Vec<(&&Player, f32)> = pool
            .iter()
            .map(|p| {
                let t = &p.skills.technical;
                let m = &p.skills.mental;
                let score = t.penalty_taking * 0.45
                    + t.finishing * 0.25
                    + t.technique * 0.15
                    + m.composure * 0.15;
                (*p, score)
            })
            .collect();
        ordered.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        let penalty_order: Vec<u32> = ordered.iter().take(11).map(|(p, _)| p.id).collect();

        SetPieceSetup {
            corner_taker,
            left_corner_taker: corner_taker,
            right_corner_taker: corner_taker,
            free_kick_taker,
            long_free_kick_taker,
            penalty_taker,
            penalty_order,
            corner_routine: CornerRoutine::Mixed,
            defensive_set_piece: DefensiveSetPiece::Zonal,
        }
    }
}

// Placeholder mental helper — pressure_handling doesn't exist, so we inline
// a small compatibility shim using existing composure + pressure personality.
trait MentalPressureHelper {
    fn pressure_handling(&self, personality: &crate::PersonAttributes) -> f32;
}

impl MentalPressureHelper for crate::club::player::skills::Mental {
    fn pressure_handling(&self, personality: &crate::PersonAttributes) -> f32 {
        // Low number = player handles pressure well (used as cost in ranking).
        let composure_bonus = self.composure * 0.7;
        let personality_bonus = personality.pressure * 0.3;
        (20.0 - composure_bonus - personality_bonus).clamp(0.0, 20.0)
    }
}
