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

use crate::club::player::mind::MindSwitch;
use crate::r#match::engine::psychology::{PsychState, Psychology};
use crate::r#match::{MatchPlayer, StateProcessingContext};

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

pub struct KeeperLapse;

impl KeeperLapse {
    /// Chance an ordinary save or pressed release of a median, settled
    /// keeper goes wrong. With `HOWLER_SHARE` it is fitted on
    /// `dev_match league 20 2 12 16` to about one error leading to a goal
    /// per keeper-season (measured 1.05, p90 2); a fumble that dies in
    /// front of him is regathered nineteen times in twenty, so the howler
    /// carries most of that.
    pub(crate) const BASE: f32 = 0.08;
    /// Full step-up nerves (~0.55) put a median keeper at about three
    /// times his settled lapse rate, where the real shaky keeper's three
    /// to six errors leading to a goal a season sit.
    const NERVE_GAIN: f32 = 4.0;
    const LOW_BELIEF_GAIN: f32 = 1.5;
    /// Of his lapses on a save, the share that go in through him rather
    /// than squirm loose: the howler.
    pub(crate) const HOWLER_SHARE: f32 = 0.20;

    /// 0.4..1.6 around a median keeper's 1.0: does he stay switched on,
    /// keep his head, choose well and turn up the same every week.
    fn steadiness(keeper: &MatchPlayer) -> f32 {
        let n = |v: f32| (v / 20.0).clamp(0.0, 1.0);
        let m = &keeper.skills.mental;
        let s = 0.35 * n(m.concentration)
            + 0.25 * n(m.composure)
            + 0.20 * n(m.decisions)
            + 0.20 * n(keeper.attributes.consistency);
        1.6 - 1.2 * s
    }

    /// What his state of mind does to his lapses: 1.0 settled, up with
    /// nerves and with lost belief. 1.0 under `OF_MIND_OFF`.
    pub fn multiplier(state: Option<&PsychState>) -> f32 {
        if !MindSwitch::armed() {
            return 1.0;
        }
        state.map_or(1.0, |s| {
            1.0 + Self::NERVE_GAIN * s.nervousness.clamp(0.0, 1.0)
                + Self::LOW_BELIEF_GAIN * (-s.confidence).clamp(0.0, 1.0)
        })
    }

    /// Chance an ordinary action of his goes wrong. None under
    /// `OF_MIND_OFF`.
    pub fn probability(keeper: &MatchPlayer, state: Option<&PsychState>) -> f32 {
        if !MindSwitch::armed() {
            return 0.0;
        }
        (Self::BASE * Self::steadiness(keeper) * Self::multiplier(state)).clamp(0.0, 0.5)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(KeeperLapse::multiplier(Some(&state(0.0, 0.0))), 1.0);
    }

    #[test]
    fn nerves_and_lost_belief_multiply_lapses() {
        let settled = KeeperLapse::multiplier(Some(&state(0.0, 0.0)));
        let nervous = KeeperLapse::multiplier(Some(&state(0.0, 0.6)));
        let shaken = KeeperLapse::multiplier(Some(&state(-0.5, 0.6)));
        assert!(nervous > settled);
        assert!(shaken > nervous);
    }
}
