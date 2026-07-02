//! Pair-keyed chemistry between teammates and team-level tactical
//! familiarity. Pure scoring + an isolated `ChemistryMap` cache so the
//! per-pair lookup is amortised across the match.
//!
//! Effects (caller applies these):
//!  - one-touch pass success +0.04 at chemistry > 0.65
//!  - wall-pass / give-and-go selection +0.08
//!  - defensive handoff success +0.06
//!  - offside line synchronisation +0.04
//!  - duplicate pressing penalty -8..-15%
//!  - poor chemistry: 6..10% chance both target same attacking space

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

/// Multiply-fold hasher for the small integer pair keys below. The
/// std `RandomState`/SipHash showed up in match CPU traces (the pass
/// evaluator probes a pair per candidate per decision); a fixed
/// multiplicative fold is ~5× cheaper and deterministic. Only lookups
/// use the map (no iteration), so hasher choice cannot affect results.
#[derive(Default)]
pub struct PairKeyHasher(u64);

impl Hasher for PairKeyHasher {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        // Generic fallback (derived `Hash` for `(u32, u32)` uses
        // `write_u32`, so this path stays cold).
        for &b in bytes {
            self.0 = (self.0 ^ b as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        }
    }

    #[inline]
    fn write_u32(&mut self, i: u32) {
        self.0 = (self.0 ^ i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    }

    #[inline]
    fn finish(&self) -> u64 {
        // Final avalanche so low-entropy ids spread across buckets.
        let mut x = self.0;
        x ^= x >> 33;
        x = x.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
        x ^= x >> 33;
        x
    }
}

type PairKeyState = BuildHasherDefault<PairKeyHasher>;

/// Player roles that matter for chemistry priors. Lighter than the full
/// PlayerPositionType — chemistry only cares about positional family +
/// lane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    Goalkeeper,
    CenterBack,
    FullBack,
    DefensiveMid,
    CentralMid,
    AttackingMid,
    Winger,
    Striker,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Lane {
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, Copy)]
pub struct ChemistryInputs {
    pub role_a: Role,
    pub lane_a: Lane,
    pub role_b: Role,
    pub lane_b: Lane,
    /// 0..20 each — high teamwork raises chemistry; very low pulls it down.
    pub teamwork_a_0_20: f32,
    pub teamwork_b_0_20: f32,
    /// True if either is flagged as new/just-arrived (low adaptability).
    pub either_is_new: bool,
}

/// Compute the initial chemistry score for a pair of teammates (0..1).
///
/// Approximations from spec:
///   adjacent roles: 0.45 base
///   same-side fullback-winger: +0.15
///   CB pair: +0.12
///   CM pair: +0.10
///   striker-AM: +0.08
///   both high teamwork: +0.08
///   both low teamwork: -0.08
///   either is new (low adaptability): -0.05
pub fn initial_chemistry(inputs: ChemistryInputs) -> f32 {
    let adjacent = is_adjacent_roles(inputs.role_a, inputs.role_b);
    let mut score: f32 = if adjacent { 0.45 } else { 0.30 };

    let same_lane = inputs.lane_a == inputs.lane_b;
    if same_lane && is_fullback_winger(inputs.role_a, inputs.role_b) {
        score += 0.15;
    }
    if is_pair(inputs.role_a, inputs.role_b, Role::CenterBack) {
        score += 0.12;
    }
    if is_pair(inputs.role_a, inputs.role_b, Role::CentralMid) {
        score += 0.10;
    }
    if is_striker_am(inputs.role_a, inputs.role_b) {
        score += 0.08;
    }

    let tw_a = (inputs.teamwork_a_0_20 / 20.0).clamp(0.0, 1.0);
    let tw_b = (inputs.teamwork_b_0_20 / 20.0).clamp(0.0, 1.0);
    if tw_a >= 0.70 && tw_b >= 0.70 {
        score += 0.08;
    }
    if tw_a <= 0.30 && tw_b <= 0.30 {
        score -= 0.08;
    }

    if inputs.either_is_new {
        score -= 0.05;
    }

    score.clamp(0.0, 1.0)
}

fn is_pair(a: Role, b: Role, target: Role) -> bool {
    a == target && b == target
}

fn is_fullback_winger(a: Role, b: Role) -> bool {
    matches!(
        (a, b),
        (Role::FullBack, Role::Winger) | (Role::Winger, Role::FullBack)
    )
}

fn is_striker_am(a: Role, b: Role) -> bool {
    matches!(
        (a, b),
        (Role::Striker, Role::AttackingMid) | (Role::AttackingMid, Role::Striker)
    )
}

fn is_adjacent_roles(a: Role, b: Role) -> bool {
    use Role::*;
    let pair = (a, b);
    matches!(
        pair,
        (Goalkeeper, CenterBack)
            | (CenterBack, Goalkeeper)
            | (CenterBack, CenterBack)
            | (CenterBack, FullBack)
            | (FullBack, CenterBack)
            | (CenterBack, DefensiveMid)
            | (DefensiveMid, CenterBack)
            | (FullBack, DefensiveMid)
            | (DefensiveMid, FullBack)
            | (FullBack, Winger)
            | (Winger, FullBack)
            | (DefensiveMid, CentralMid)
            | (CentralMid, DefensiveMid)
            | (CentralMid, CentralMid)
            | (CentralMid, AttackingMid)
            | (AttackingMid, CentralMid)
            | (AttackingMid, Striker)
            | (Striker, AttackingMid)
            | (Winger, Striker)
            | (Striker, Winger)
            | (Winger, AttackingMid)
            | (AttackingMid, Winger)
    )
}

/// Pair-keyed chemistry cache. Key is the (min, max) of the two player
/// IDs so order doesn't matter. Lazily filled by callers.
#[derive(Debug, Clone, Default)]
pub struct ChemistryMap {
    pairs: HashMap<(u32, u32), f32, PairKeyState>,
}

fn pair_key(a: u32, b: u32) -> (u32, u32) {
    if a <= b { (a, b) } else { (b, a) }
}

/// Map a player's tactical position into a chemistry `Role`.
/// Conservative bucketing — anything mid/forward leaning collapses
/// into the broader Striker/Winger families to keep the role pair
/// inputs stable.
pub fn role_from_position(pos: crate::PlayerFieldPositionGroup) -> Role {
    match pos {
        crate::PlayerFieldPositionGroup::Goalkeeper => Role::Goalkeeper,
        crate::PlayerFieldPositionGroup::Defender => Role::CenterBack,
        crate::PlayerFieldPositionGroup::Midfielder => Role::CentralMid,
        crate::PlayerFieldPositionGroup::Forward => Role::Striker,
    }
}

/// Pick the lane from an in-game x/y position. We look at lateral
/// y-coordinate against the pitch height: left third, centre third,
/// right third.
pub fn lane_from_y(y: f32, field_height: f32) -> Lane {
    if y < field_height * 0.33 {
        Lane::Left
    } else if y > field_height * 0.67 {
        Lane::Right
    } else {
        Lane::Center
    }
}

impl ChemistryMap {
    pub fn set(&mut self, a: u32, b: u32, score: f32) {
        if a == b {
            return;
        }
        self.pairs.insert(pair_key(a, b), score.clamp(0.0, 1.0));
    }

    pub fn get(&self, a: u32, b: u32) -> Option<f32> {
        if a == b {
            return Some(1.0);
        }
        self.pairs.get(&pair_key(a, b)).copied()
    }

    /// Get or compute the chemistry. Uses the supplied `compute` closure
    /// to fill the cache on miss.
    pub fn get_or_compute<F: FnOnce() -> f32>(&mut self, a: u32, b: u32, compute: F) -> f32 {
        if a == b {
            return 1.0;
        }
        let key = pair_key(a, b);
        if let Some(&v) = self.pairs.get(&key) {
            return v;
        }
        let v = compute().clamp(0.0, 1.0);
        self.pairs.insert(key, v);
        v
    }

    /// Seed every same-team pair from a player roster. Roles and lanes
    /// are derived from each player's tactical position group + spawn
    /// y-coordinate. Teamwork inputs come from the player's mental
    /// attribute. Called once at kickoff — subsequent live events can
    /// adjust pair scores, but the initial baseline matches the squad
    /// as it took the pitch.
    pub fn seed_from_roster(
        &mut self,
        players: &[(u32, u32, crate::PlayerFieldPositionGroup, f32, f32)],
        field_height: f32,
    ) {
        // Tuple layout: (player_id, team_id, position_group, y, teamwork_0_20).
        for i in 0..players.len() {
            for j in (i + 1)..players.len() {
                let (id_a, team_a, pos_a, y_a, tw_a) = players[i];
                let (id_b, team_b, pos_b, y_b, tw_b) = players[j];
                if team_a != team_b {
                    continue;
                }
                let role_a = role_from_position(pos_a);
                let role_b = role_from_position(pos_b);
                let lane_a = lane_from_y(y_a, field_height);
                let lane_b = lane_from_y(y_b, field_height);
                let score = initial_chemistry(ChemistryInputs {
                    role_a,
                    lane_a,
                    role_b,
                    lane_b,
                    teamwork_a_0_20: tw_a,
                    teamwork_b_0_20: tw_b,
                    either_is_new: false,
                });
                self.set(id_a, id_b, score);
            }
        }
    }
}

/// Tactical familiarity for a team (0..1). Default 0.65 — recent tactic
/// changes / new players drop it; long-stable XI raises it slightly.
#[derive(Debug, Clone, Copy)]
pub struct TacticalFamiliarity {
    pub score: f32,
}

impl Default for TacticalFamiliarity {
    fn default() -> Self {
        TacticalFamiliarity { score: 0.65 }
    }
}

impl TacticalFamiliarity {
    /// Spacing error in field units that the team's formation drifts by
    /// because of unfamiliarity. Spec: `(1 - familiarity) * 8u`.
    pub fn formation_spacing_error_units(&self) -> f32 {
        (1.0 - self.score.clamp(0.0, 1.0)) * 8.0
    }

    /// Press timing error (0..0.08) — higher = press triggers go off
    /// at the wrong time more often.
    pub fn press_timing_error(&self) -> f32 {
        (1.0 - self.score.clamp(0.0, 1.0)) * 0.08
    }

    /// Offside trap risk (0..0.06).
    pub fn offside_trap_risk(&self) -> f32 {
        (1.0 - self.score.clamp(0.0, 1.0)) * 0.06
    }

    /// Build-up patience consistency bonus 0..0.06.
    pub fn build_up_patience_bonus(&self) -> f32 {
        self.score.clamp(0.0, 1.0) * 0.06
    }
}

/// Modifiers a chemistry score applies to specific pair-level events.
#[derive(Debug, Clone, Copy, Default)]
pub struct ChemistryModifiers {
    pub one_touch_pass_bonus: f32,
    pub give_and_go_selection_bonus: f32,
    pub defensive_handoff_bonus: f32,
    pub offside_line_sync_bonus: f32,
    pub duplicate_pressing_penalty: f32,
    pub same_space_attack_chance: f32,
}

pub fn chemistry_modifiers(chem: f32) -> ChemistryModifiers {
    let chem = chem.clamp(0.0, 1.0);
    let mut m = ChemistryModifiers::default();
    if chem > 0.65 {
        m.one_touch_pass_bonus = 0.04;
        m.give_and_go_selection_bonus = 0.08;
        m.defensive_handoff_bonus = 0.06;
        m.offside_line_sync_bonus = 0.04;
        // Duplicate pressing reduced by 8-15% scaled by how far above 0.65.
        let above = (chem - 0.65) / 0.35;
        m.duplicate_pressing_penalty = -(0.08 + above * 0.07).min(0.15);
    } else if chem < 0.35 {
        // Poor chemistry — increased same-space attack chance.
        let below = (0.35 - chem) / 0.35;
        m.same_space_attack_chance = (0.06 + below * 0.04).min(0.10);
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs(role_a: Role, lane_a: Lane, role_b: Role, lane_b: Lane) -> ChemistryInputs {
        ChemistryInputs {
            role_a,
            lane_a,
            role_b,
            lane_b,
            teamwork_a_0_20: 12.0,
            teamwork_b_0_20: 12.0,
            either_is_new: false,
        }
    }

    #[test]
    fn cb_pair_higher_chemistry_than_arbitrary() {
        let cb_pair = initial_chemistry(inputs(
            Role::CenterBack,
            Lane::Center,
            Role::CenterBack,
            Lane::Center,
        ));
        let gk_striker = initial_chemistry(inputs(
            Role::Goalkeeper,
            Lane::Center,
            Role::Striker,
            Lane::Center,
        ));
        assert!(cb_pair > gk_striker);
    }

    #[test]
    fn fullback_winger_same_side_high() {
        let same_side =
            initial_chemistry(inputs(Role::FullBack, Lane::Left, Role::Winger, Lane::Left));
        let opposite = initial_chemistry(inputs(
            Role::FullBack,
            Lane::Left,
            Role::Winger,
            Lane::Right,
        ));
        assert!(same_side > opposite);
    }

    #[test]
    fn striker_am_pair_gets_bonus() {
        let pair = initial_chemistry(inputs(
            Role::AttackingMid,
            Lane::Center,
            Role::Striker,
            Lane::Center,
        ));
        let other = initial_chemistry(inputs(
            Role::DefensiveMid,
            Lane::Center,
            Role::Striker,
            Lane::Center,
        ));
        assert!(pair > other);
    }

    #[test]
    fn high_teamwork_lifts_chemistry() {
        let mut hi = inputs(Role::CenterBack, Lane::Center, Role::FullBack, Lane::Left);
        hi.teamwork_a_0_20 = 18.0;
        hi.teamwork_b_0_20 = 18.0;
        let mut lo = hi;
        lo.teamwork_a_0_20 = 4.0;
        lo.teamwork_b_0_20 = 4.0;
        assert!(initial_chemistry(hi) > initial_chemistry(lo));
    }

    #[test]
    fn newcomer_penalty() {
        let mut x = inputs(
            Role::CentralMid,
            Lane::Center,
            Role::CentralMid,
            Lane::Center,
        );
        let baseline = initial_chemistry(x);
        x.either_is_new = true;
        assert!(initial_chemistry(x) < baseline);
    }

    #[test]
    fn chemistry_score_is_unit_clamped() {
        // High-bonus configuration shouldn't break 1.0.
        let mut x = inputs(Role::FullBack, Lane::Left, Role::Winger, Lane::Left);
        x.teamwork_a_0_20 = 20.0;
        x.teamwork_b_0_20 = 20.0;
        let s = initial_chemistry(x);
        assert!((0.0..=1.0).contains(&s));
    }

    #[test]
    fn chemistry_map_is_order_independent() {
        let mut m = ChemistryMap::default();
        m.set(7, 12, 0.8);
        assert_eq!(m.get(7, 12), Some(0.8));
        assert_eq!(m.get(12, 7), Some(0.8));
    }

    #[test]
    fn chemistry_map_self_is_one() {
        let m = ChemistryMap::default();
        assert_eq!(m.get(5, 5), Some(1.0));
    }

    #[test]
    fn chemistry_map_get_or_compute_caches() {
        let mut m = ChemistryMap::default();
        let mut calls = 0;
        let v = m.get_or_compute(1, 2, || {
            calls += 1;
            0.55
        });
        assert_eq!(v, 0.55);
        let v2 = m.get_or_compute(2, 1, || {
            calls += 1;
            0.99
        });
        // Cached — closure not called again.
        assert_eq!(v2, 0.55);
        assert_eq!(calls, 1);
    }

    #[test]
    fn chemistry_modifiers_high_helps_one_touch() {
        let m = chemistry_modifiers(0.85);
        assert!(m.one_touch_pass_bonus > 0.0);
        assert!(m.give_and_go_selection_bonus > 0.0);
        assert!(m.duplicate_pressing_penalty < 0.0);
    }

    #[test]
    fn chemistry_modifiers_low_increases_same_space() {
        let m = chemistry_modifiers(0.20);
        assert!(m.same_space_attack_chance > 0.0);
        assert_eq!(m.one_touch_pass_bonus, 0.0);
    }

    #[test]
    fn chemistry_modifiers_neutral_zone_is_neutral() {
        let m = chemistry_modifiers(0.50);
        assert_eq!(m.one_touch_pass_bonus, 0.0);
        assert_eq!(m.same_space_attack_chance, 0.0);
    }

    #[test]
    fn tactical_familiarity_drives_press_and_offside() {
        let strong = TacticalFamiliarity { score: 0.95 };
        let weak = TacticalFamiliarity { score: 0.30 };
        assert!(strong.formation_spacing_error_units() < weak.formation_spacing_error_units());
        assert!(strong.press_timing_error() < weak.press_timing_error());
        assert!(strong.offside_trap_risk() < weak.offside_trap_risk());
        assert!(strong.build_up_patience_bonus() > weak.build_up_patience_bonus());
    }
}
