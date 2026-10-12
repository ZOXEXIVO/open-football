//! **Lapses.** How often an action goes wrong for reasons skill does not
//! cover: a player switching off, snatching at it, or freezing.
//!
//! One lane for every player and every kind of action. A family whose
//! action already fails on an attribute-driven roll has that roll scaled
//! by the player's state of mind; a family with no such roll draws a lapse
//! of its own. The state-of-mind factor is the same for everyone, so a
//! keeper and an outfielder in the same frame of mind see the same rise in
//! the same kind of action. Each kind feels it in proportion to how much
//! of its failure is a lapse rather than a matter of skill.

use crate::club::player::mind::MindSwitch;
use crate::r#match::MatchPlayer;
use crate::r#match::engine::psychology::PsychState;

/// The kinds of action a lapse can spoil.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LapseFamily {
    /// A granted save that goes in through him or squirms loose. Drawn.
    Save,
    /// A claim that fails. Scaled.
    Claim,
    /// A pass snatched under the press. Drawn.
    Pass,
    /// A first touch that gets away. Scaled.
    Touch,
    /// A clearance miscued. Scaled on the foot, drawn on the head.
    Clearance,
    /// A challenge mistimed into a foul. Scaled.
    Challenge,
    /// A shot snatched wide or over. Scaled.
    Finish,
    /// A marker late to the run of his man. Drawn.
    Mark,
    /// How much of the press reaches a pass. Scales an execution error
    /// rather than a chance.
    Press,
}

impl LapseFamily {
    pub const COUNT: usize = 9;
    pub const ALL: [LapseFamily; Self::COUNT] = [
        LapseFamily::Save,
        LapseFamily::Claim,
        LapseFamily::Pass,
        LapseFamily::Touch,
        LapseFamily::Clearance,
        LapseFamily::Challenge,
        LapseFamily::Finish,
        LapseFamily::Mark,
        LapseFamily::Press,
    ];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn label(self) -> &'static str {
        match self {
            LapseFamily::Save => "save",
            LapseFamily::Claim => "claim",
            LapseFamily::Pass => "pass",
            LapseFamily::Touch => "touch",
            LapseFamily::Clearance => "clearance",
            LapseFamily::Challenge => "challenge",
            LapseFamily::Finish => "finish",
            LapseFamily::Mark => "mark",
            LapseFamily::Press => "press",
        }
    }

    /// Share of this kind of action's failures that are lapses — switching
    /// off, snatching, freezing — rather than a matter of skill: how much
    /// of his state of mind it feels. A shot that misses or a challenge
    /// that fouls is mostly the skill and the situation; a fluffed touch or
    /// a miscued clearance is mostly the head.
    fn exposure(self) -> f32 {
        match self {
            LapseFamily::Press => 0.35,
            LapseFamily::Challenge => 0.25,
            LapseFamily::Finish => 0.20,
            LapseFamily::Save
            | LapseFamily::Claim
            | LapseFamily::Pass
            | LapseFamily::Touch
            | LapseFamily::Clearance
            | LapseFamily::Mark => 1.0,
        }
    }

    /// Settled chance a median player's action of a drawn family goes
    /// wrong, before the situation (how hard he is pressed) is applied.
    fn base(self) -> f32 {
        match self {
            // With `HOWLER_SHARE`, fitted on `dev_match league 20 2 12 16`
            // to about one error leading to a goal per keeper-season.
            LapseFamily::Save => 0.08,
            LapseFamily::Pass => 0.06,
            LapseFamily::Clearance => 0.30,
            LapseFamily::Claim
            | LapseFamily::Touch
            | LapseFamily::Challenge
            | LapseFamily::Finish
            | LapseFamily::Mark
            | LapseFamily::Press => 0.0,
        }
    }
}

pub struct ActionLapse;

impl ActionLapse {
    /// Full step-up nerves (~0.55) put a median player at about three
    /// times his settled lapse rate, where the real shaky keeper's three
    /// to six errors leading to a goal a season sit.
    const NERVE_GAIN: f32 = 4.0;
    const LOW_BELIEF_GAIN: f32 = 1.5;
    /// Of a keeper's lapses on a save, the share that go in through him
    /// rather than squirm loose: the howler.
    pub(crate) const HOWLER_SHARE: f32 = 0.20;

    /// 0.4..1.6 around a median player's 1.0: does he stay switched on,
    /// keep his head, choose well and turn up the same every week.
    pub fn steadiness(player: &MatchPlayer) -> f32 {
        let n = |v: f32| (v / 20.0).clamp(0.0, 1.0);
        let m = &player.skills.mental;
        let s = 0.35 * n(m.concentration)
            + 0.25 * n(m.composure)
            + 0.20 * n(m.decisions)
            + 0.20 * n(player.attributes.consistency);
        1.6 - 1.2 * s
    }

    /// What his state of mind does to his failures: 1.0 settled, up with
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

    /// How far `family` feels his state of mind: 1.0 settled, the full
    /// multiplier for a family whose failures are all lapses.
    pub fn factor(family: LapseFamily, state: Option<&PsychState>) -> f32 {
        1.0 + family.exposure() * (Self::multiplier(state) - 1.0)
    }

    /// An existing failure chance of `family`, scaled by his state of mind
    /// as far as the family feels it. The factor is put on the odds, so a
    /// rare failure rises in proportion and a common one bends toward
    /// certainty rather than through it.
    pub fn scaled(family: LapseFamily, chance: f32, state: Option<&PsychState>) -> f32 {
        let chance = chance.clamp(0.0, 1.0);
        let factor = Self::factor(family, state);
        chance * factor / (1.0 + chance * (factor - 1.0))
    }

    /// Chance a drawn family's action of his lapses, before the situation
    /// is applied. None under `OF_MIND_OFF`.
    pub fn probability(family: LapseFamily, player: &MatchPlayer, state: Option<&PsychState>) -> f32 {
        Self::drawn(family.base() * Self::steadiness(player), state)
    }

    /// A drawn lapse whose settled chance an attribute model already gives
    /// (`settled` stands in for base × steadiness). None under
    /// `OF_MIND_OFF`.
    pub fn drawn(settled: f32, state: Option<&PsychState>) -> f32 {
        if !MindSwitch::armed() {
            return 0.0;
        }
        (settled * Self::multiplier(state)).clamp(0.0, 0.5)
    }
}

/// Lapse diagnostic counters — opportunities and failures per family,
/// giveaways stamped and charged by the family that caused them, and the
/// minute errors leading to a shot are charged. `match-logs` only; the dev
/// harness prints them.
#[cfg(feature = "match-logs")]
pub mod lapse_diag {
    use super::LapseFamily;
    use std::sync::atomic::{AtomicU64, Ordering};

    pub const PHASES: usize = 7;

    pub static OPPORTUNITIES: [AtomicU64; LapseFamily::COUNT] =
        [const { AtomicU64::new(0) }; LapseFamily::COUNT];
    pub static FAILURES: [AtomicU64; LapseFamily::COUNT] =
        [const { AtomicU64::new(0) }; LapseFamily::COUNT];
    pub static STAMPED: [AtomicU64; LapseFamily::COUNT] =
        [const { AtomicU64::new(0) }; LapseFamily::COUNT];
    pub static CHARGED: [AtomicU64; LapseFamily::COUNT] =
        [const { AtomicU64::new(0) }; LapseFamily::COUNT];
    /// Errors leading to a shot by 15-minute phase; the last bucket is
    /// anything after the 90th minute.
    pub static ERRORS_BY_PHASE: [AtomicU64; PHASES] = [const { AtomicU64::new(0) }; PHASES];

    pub fn note(family: LapseFamily, failed: bool) {
        OPPORTUNITIES[family.index()].fetch_add(1, Ordering::Relaxed);
        if failed {
            FAILURES[family.index()].fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn note_stamp(family: LapseFamily) {
        STAMPED[family.index()].fetch_add(1, Ordering::Relaxed);
    }

    pub fn note_charge(family: LapseFamily, minute: u32) {
        CHARGED[family.index()].fetch_add(1, Ordering::Relaxed);
        ERRORS_BY_PHASE[((minute / 15) as usize).min(PHASES - 1)].fetch_add(1, Ordering::Relaxed);
    }

    pub fn reset() {
        for set in [&OPPORTUNITIES, &FAILURES, &STAMPED, &CHARGED] {
            for c in set.iter() {
                c.store(0, Ordering::Relaxed);
            }
        }
        for c in ERRORS_BY_PHASE.iter() {
            c.store(0, Ordering::Relaxed);
        }
    }

    /// `(opportunities, failures, stamped, charged)` for one family.
    pub fn snapshot(family: LapseFamily) -> (u64, u64, u64, u64) {
        let i = family.index();
        (
            OPPORTUNITIES[i].load(Ordering::Relaxed),
            FAILURES[i].load(Ordering::Relaxed),
            STAMPED[i].load(Ordering::Relaxed),
            CHARGED[i].load(Ordering::Relaxed),
        )
    }

    pub fn errors_by_phase() -> [u64; PHASES] {
        std::array::from_fn(|i| ERRORS_BY_PHASE[i].load(Ordering::Relaxed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::club::player::mind::KickoffMind;
    use crate::{PersonAttributes, PlayerAttributes, PlayerPositionType, PlayerSkills};
    use chrono::NaiveDate;

    fn state(confidence: f32, nervousness: f32) -> PsychState {
        PsychState {
            confidence,
            nervousness,
            ..PsychState::default()
        }
    }

    fn player(position: PlayerPositionType, mental: f32) -> MatchPlayer {
        let attributes = PersonAttributes {
            consistency: mental,
            ..PersonAttributes::default()
        };
        let mut skills = PlayerSkills::default();
        skills.mental.concentration = mental;
        skills.mental.composure = mental;
        skills.mental.decisions = mental;
        MatchPlayer::from_inputs(
            1,
            1,
            [0.0; 3],
            [0.0; 3],
            attributes,
            PlayerAttributes::default(),
            skills,
            position,
            None,
            Vec::new(),
            NaiveDate::from_ymd_opt(2000, 1, 1).unwrap(),
            false,
            10_000,
            0.0,
            1.0,
            1.0,
            KickoffMind::neutral(),
            false,
        )
    }

    #[test]
    fn a_settled_head_leaves_failures_alone() {
        assert_eq!(ActionLapse::multiplier(Some(&state(0.0, 0.0))), 1.0);
        assert_eq!(ActionLapse::multiplier(None), 1.0);
        assert_eq!(ActionLapse::scaled(LapseFamily::Touch, 0.04, Some(&state(0.0, 0.0))), 0.04);
    }

    #[test]
    fn nerves_and_lost_belief_multiply_failures() {
        let settled = ActionLapse::multiplier(Some(&state(0.0, 0.0)));
        let nervous = ActionLapse::multiplier(Some(&state(0.0, 0.6)));
        let shaken = ActionLapse::multiplier(Some(&state(-0.5, 0.6)));
        assert!(nervous > settled);
        assert!(shaken > nervous);
    }

    #[test]
    fn keeper_and_outfielder_share_one_factor() {
        let head = state(-0.2, 0.4);
        let keeper = player(PlayerPositionType::Goalkeeper, 10.0);
        let back = player(PlayerPositionType::DefenderCenter, 10.0);
        let rise = |p: &MatchPlayer, f: LapseFamily| {
            ActionLapse::probability(f, p, Some(&head)) / ActionLapse::probability(f, p, None)
        };
        assert!((rise(&keeper, LapseFamily::Save) - rise(&back, LapseFamily::Pass)).abs() < 1e-5);
    }

    #[test]
    fn a_steadier_player_lapses_less() {
        let head = state(0.0, 0.3);
        let steady = player(PlayerPositionType::MidfielderCenter, 18.0);
        let flaky = player(PlayerPositionType::MidfielderCenter, 4.0);
        assert!(
            ActionLapse::probability(LapseFamily::Pass, &steady, Some(&head))
                < ActionLapse::probability(LapseFamily::Pass, &flaky, Some(&head))
        );
    }

    /// Other tests add to the same process-wide counters, so this reads
    /// deltas and never resets.
    #[cfg(feature = "match-logs")]
    #[test]
    fn the_lapse_counters_count() {
        for family in LapseFamily::ALL {
            let before = lapse_diag::snapshot(family);
            lapse_diag::note(family, true);
            lapse_diag::note(family, false);
            lapse_diag::note_stamp(family);
            lapse_diag::note_charge(family, 95);
            let after = lapse_diag::snapshot(family);
            assert!(after.0 >= before.0 + 2, "{}", family.label());
            assert!(after.1 > before.1, "{}", family.label());
            assert!(after.2 > before.2, "{}", family.label());
            assert!(after.3 > before.3, "{}", family.label());
        }
        assert!(lapse_diag::errors_by_phase()[lapse_diag::PHASES - 1] >= LapseFamily::COUNT as u64);
    }

    #[test]
    fn a_rare_failure_rises_in_proportion_and_a_common_one_bends() {
        let nervous = state(0.0, 0.5);
        let rare = ActionLapse::scaled(LapseFamily::Touch, 0.01, Some(&nervous));
        assert!((rare / 0.01 - ActionLapse::multiplier(Some(&nervous))).abs() < 0.1);
        let common = ActionLapse::scaled(LapseFamily::Touch, 0.6, Some(&nervous));
        assert!(common > 0.6 && common < 1.0);
    }

    #[test]
    fn a_settled_head_leaves_every_family_at_one() {
        let nervous = state(0.0, 0.5);
        for family in LapseFamily::ALL {
            assert_eq!(ActionLapse::factor(family, Some(&PsychState::default())), 1.0);
        }
        assert_eq!(
            ActionLapse::factor(LapseFamily::Pass, Some(&nervous)),
            ActionLapse::multiplier(Some(&nervous))
        );
        assert!(
            ActionLapse::factor(LapseFamily::Finish, Some(&nervous))
                < ActionLapse::factor(LapseFamily::Pass, Some(&nervous))
        );
    }

    #[test]
    fn a_skill_heavy_family_feels_less_of_his_head() {
        let nervous = state(0.0, 0.5);
        let finish = ActionLapse::scaled(LapseFamily::Finish, 0.3, Some(&nervous));
        let touch = ActionLapse::scaled(LapseFamily::Touch, 0.3, Some(&nervous));
        assert!(finish > 0.3 && finish < touch);
    }

    #[test]
    fn scaled_families_draw_nothing_of_their_own() {
        let p = player(PlayerPositionType::Striker, 10.0);
        for f in [LapseFamily::Claim, LapseFamily::Touch, LapseFamily::Challenge, LapseFamily::Finish] {
            assert_eq!(ActionLapse::probability(f, &p, Some(&state(0.0, 0.8))), 0.0);
        }
    }
}
