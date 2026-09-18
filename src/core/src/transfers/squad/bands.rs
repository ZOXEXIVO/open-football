//! What a club of a given standing expects of a player.
//!
//! One continuous curve rather than six enum buckets: a club mid-Continental
//! gets a different baseline from a top-of-Continental one, instead of both
//! snapping to the same tier. Everything here is a function of a reputation
//! SCORE (0..1) and a position group — no club, no player, no world.

use crate::{ClubLevelAnchor, PlayerFieldPositionGroup, ReputationLevel};

/// How good a club is for a player of a given level — one continuous
/// reading, shared by the player's own plan and the club's pathway.
///
/// 1.0 is a starter here, 0.0 rotation depth, −0.5 out of his depth, 1.5
/// too good for the place. Because both sides speak it, "drop one level to
/// play" is a band delta rather than a reputation-tier step, and the same
/// sentence means the same thing to a nineteen-year-old at a giant and a
/// thirty-year-old in a second division.
pub struct LevelBand;

impl LevelBand {
    /// Out of his depth at the bottom, plainly too good at the top.
    pub const MIN: f32 = -1.0;
    pub const MAX: f32 = 2.0;

    /// A division, in band terms. The step a loan normally asks a player
    /// to take, and the width every band tolerance is written against.
    pub const ONE_LEVEL: f32 = 0.6;

    /// The band `level` occupies at a club with this anchor.
    pub fn of(level: u8, group: PlayerFieldPositionGroup, anchor: &ClubLevelAnchor) -> f32 {
        let key = anchor.key_floor(group) as f32;
        let rotation = anchor.rotation_floor(group) as f32;
        let span = (key - rotation).max(1.0);
        ((level as f32 - rotation) / span).clamp(Self::MIN, Self::MAX)
    }

    /// The same reading against a club known only by its reputation
    /// `overall_score` (0..1) — the cross-border and summary paths.
    pub fn at_reputation(level: u8, group: PlayerFieldPositionGroup, score: f32) -> f32 {
        Self::of(level, group, &ClubLevelAnchor::for_reputation(score))
    }

    /// How well a band matches what somebody meant him to be playing at,
    /// 1.0 for a match and 0.0 a full level away.
    pub fn fit(band: f32, target: f32) -> f32 {
        1.0 - ((band - target).abs() / Self::ONE_LEVEL).clamp(0.0, 1.0)
    }

    /// How ready a player already is for his parent club's own first
    /// team, 0..1 — measured against the parent's best in his position,
    /// which is the standard he is actually judged by there. An unknown
    /// parent standard reads as fully raw, which is what stands the
    /// destination floors down.
    pub fn readiness_of(ability: u8, parent_best_in_group: u8) -> f32 {
        if parent_best_in_group == 0 {
            return 0.0;
        }
        const RAW_RATIO: f32 = 0.60;
        const READY_RATIO: f32 = 0.90;
        let ratio = ability as f32 / parent_best_in_group as f32;
        ((ratio - RAW_RATIO) / (READY_RATIO - RAW_RATIO)).clamp(0.0, 1.0)
    }
}

/// The ability bands a club recruits in.
pub(crate) struct TierBands;

impl TierBands {
    pub(in crate::transfers) fn rep_level_value(level: &ReputationLevel) -> u8 {
        match level {
            ReputationLevel::Elite => 5,
            ReputationLevel::Continental => 4,
            ReputationLevel::National => 3,
            ReputationLevel::Regional => 2,
            ReputationLevel::Local => 1,
            ReputationLevel::Amateur => 0,
        }
    }

    /// Linear-interpolated lookup of base baseline CA from a continuous
    /// reputation score. Anchors are calibrated so that the midpoint of
    /// each enum tier reproduces the bucketed baseline the rest of the
    /// pipeline expects. Score is `Reputation::overall_score()` (0..1).
    fn baseline_anchor_curve(score: f32) -> f32 {
        const ANCHORS: [(f32, f32); 7] = [
            (0.000, 50.0),
            (0.075, 55.0),
            (0.225, 70.0),
            (0.400, 88.0),
            (0.575, 110.0),
            (0.725, 130.0),
            (0.900, 145.0),
        ];
        let s = score.clamp(0.0, 1.0);
        // Above the top anchor we keep climbing — top-of-Elite (e.g. a
        // generational Real Madrid side) demands more than mid-Elite.
        if s >= ANCHORS[ANCHORS.len() - 1].0 {
            let (s_top, b_top) = ANCHORS[ANCHORS.len() - 1];
            let extrapolation = (s - s_top) * (162.0 - b_top) / (1.0 - s_top).max(1e-6);
            return b_top + extrapolation;
        }
        for window in ANCHORS.windows(2) {
            let (s0, b0) = window[0];
            let (s1, b1) = window[1];
            if s >= s0 && s <= s1 {
                let t = (s - s0) / (s1 - s0).max(1e-6);
                return b0 + (b1 - b0) * t;
            }
        }
        ANCHORS[0].1
    }

    /// Linear-interpolated headroom (max CA above baseline a club can
    /// realistically pursue). Anchored at the same enum-tier midpoints
    /// as [`baseline_anchor_curve`].
    fn headroom_anchor_curve(score: f32) -> f32 {
        const ANCHORS: [(f32, f32); 7] = [
            (0.000, 6.0),
            (0.075, 8.0),
            (0.225, 10.0),
            (0.400, 14.0),
            (0.575, 22.0),
            (0.725, 35.0),
            (0.900, 55.0),
        ];
        let s = score.clamp(0.0, 1.0);
        if s >= ANCHORS[ANCHORS.len() - 1].0 {
            let (s_top, h_top) = ANCHORS[ANCHORS.len() - 1];
            let extrapolation = (s - s_top) * (65.0 - h_top) / (1.0 - s_top).max(1e-6);
            return h_top + extrapolation;
        }
        for window in ANCHORS.windows(2) {
            let (s0, h0) = window[0];
            let (s1, h1) = window[1];
            if s >= s0 && s <= s1 {
                let t = (s - s0) / (s1 - s0).max(1e-6);
                return h0 + (h1 - h0) * t;
            }
        }
        ANCHORS[0].1
    }

    /// Per-group offset applied on top of the base baseline. Goalkeepers
    /// naturally score lower on the unified CA scale (fewer outfield-
    /// style attributes feed the rating); forwards a touch higher. Kept
    /// here as the single source of truth — neither evaluation nor
    /// recommendations carries its own per-group adjustment.
    fn group_baseline_offset(group: PlayerFieldPositionGroup) -> i16 {
        match group {
            PlayerFieldPositionGroup::Goalkeeper => -8,
            PlayerFieldPositionGroup::Defender => -3,
            PlayerFieldPositionGroup::Midfielder => 0,
            PlayerFieldPositionGroup::Forward => 2,
        }
    }

    /// Expected current-ability of an at-tier starter for a club whose
    /// reputation `overall_score` is `score` (0..1). The continuous
    /// version of [`tier_starter_ca`] — a club mid-Continental gets a
    /// different baseline from a top-of-Continental club, instead of
    /// snapping to the same enum bucket. Position offsets are applied
    /// uniformly via [`group_baseline_offset`].
    pub(crate) fn tier_starter_ca_score(score: f32, group: PlayerFieldPositionGroup) -> u8 {
        let base = Self::baseline_anchor_curve(score);
        let offset = Self::group_baseline_offset(group);
        (base.round() as i16 + offset).clamp(20, 200) as u8
    }

    /// Continuous-score counterpart of [`tier_target_ceiling`].
    pub(crate) fn tier_target_ceiling_score(score: f32, group: PlayerFieldPositionGroup) -> u8 {
        let baseline = TierBands::tier_starter_ca_score(score, group);
        let headroom = Self::headroom_anchor_curve(score).round() as i16;
        (baseline as i16 + headroom).clamp(20, 200) as u8
    }

    /// Continuous-score counterpart of [`tier_quality_tolerance`]. Top
    /// clubs upgrade aggressively (small tolerance); small clubs
    /// patient. Linear in 1 - score so that going up the reputation
    /// ladder reduces tolerance smoothly, no enum cliff.
    pub(crate) fn tier_quality_tolerance_score(score: f32) -> i16 {
        let s = score.clamp(0.0, 1.0);
        let raw = 4.0 + (1.0 - s) * 11.0; // 4 at top, 15 at the bottom
        raw.round() as i16
    }
}
