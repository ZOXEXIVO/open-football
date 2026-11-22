//! **The keeper's head.** How his confidence and his nerves change what he
//! takes on and how often it goes wrong.
//!
//! One setting, read where each decision is taken rather than inside the
//! memoized `GoalkeeperSkillProfile`: his appetite moves during a match and
//! the profile's memo is keyed on condition and minute alone.
//!
//! A nervous keeper is passive — he claims less, comes off his line less,
//! parries what he could hold and kicks long — and his hesitation and
//! handling fail more often. An over-confident one claims more, sweeps
//! further, holds more and plays out more; his errors are the attempts
//! his skill does not support, priced by the outcome rolls those actions
//! already make. A settled keeper plays exactly as his attributes say.

use crate::r#match::StateProcessingContext;
use crate::r#match::engine::psychology::Psychology;

pub struct KeeperAppetite;

impl KeeperAppetite {
    const CLAIM: f32 = 0.25;
    const RUSH: f32 = 0.30;
    const HOLD: f32 = 0.20;
    const SHORT: f32 = 0.25;
    const LONG: f32 = 0.15;

    /// His appetite now, −1..1 — zero for a settled keeper.
    pub fn of(ctx: &StateProcessingContext) -> f32 {
        ctx.context
            .psychology
            .get(ctx.player.id)
            .map_or(0.0, Psychology::keeper_appetite)
    }

    /// Scale on the ground he claims a loose or dropping ball over.
    pub fn claim_reach(appetite: f32) -> f32 {
        (1.0 + Self::CLAIM * appetite).max(0.5)
    }

    /// Added to his risk appetite for joining a race or sweeping.
    pub fn rush_risk(appetite: f32) -> f32 {
        Self::RUSH * appetite
    }

    /// Scale on how much of a save he tries to hold rather than parry.
    pub fn hold_share(appetite: f32) -> f32 {
        (1.0 + Self::HOLD * appetite).max(0.5)
    }

    /// Added to the short build-up score, and taken off the long kick's.
    pub fn short_ball(appetite: f32) -> (f32, f32) {
        (Self::SHORT * appetite, -Self::LONG * appetite)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::r#match::engine::psychology::PsychState;

    fn state(confidence: f32, nervousness: f32) -> PsychState {
        PsychState {
            confidence,
            nervousness,
            ..PsychState::default()
        }
    }

    #[test]
    fn a_nervous_keeper_claims_less_and_a_confident_one_more() {
        let nervous = Psychology::keeper_appetite(&state(0.0, 0.6));
        let confident = Psychology::keeper_appetite(&state(0.5, 0.0));
        assert!(KeeperAppetite::claim_reach(nervous) < KeeperAppetite::claim_reach(0.0));
        assert!(KeeperAppetite::claim_reach(confident) > KeeperAppetite::claim_reach(0.0));
        assert!(KeeperAppetite::rush_risk(nervous) < 0.0);
        assert!(KeeperAppetite::hold_share(nervous) < 1.0);
        assert!(KeeperAppetite::short_ball(nervous).0 < 0.0);
    }

    #[test]
    fn a_settled_keeper_plays_his_attributes() {
        let settled = Psychology::keeper_appetite(&state(0.0, 0.0));
        assert_eq!(settled, 0.0);
        assert_eq!(KeeperAppetite::claim_reach(settled), 1.0);
        assert_eq!(KeeperAppetite::rush_risk(settled), 0.0);
        assert_eq!(KeeperAppetite::hold_share(settled), 1.0);
    }
}
