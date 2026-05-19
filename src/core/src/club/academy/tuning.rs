//! Tuning constants & scale helpers for the academy system.
//!
//! Everything here is wrapped in a struct so callers go through a named
//! type rather than a soup of free helpers. The struct layout is:
//!
//!   * `AcademyTuning` — knobs (intake month, target sizes, PA thresholds).
//!   * `AcademyTier`   — 1..10 short-scale tier derived from a 1..20
//!                       facility rating. Owns every formula that
//!                       needs "how strong is this academy" on the
//!                       short scale (norm, readiness threshold,
//!                       sessions-per-phase, …).

/// Half-open size bands the academy aims to keep itself inside, indexed
/// by 1..10 pathway tier (index 0 unused). The pathway-review tick
/// uses these as `players_count_range`, and the intake/backfill paths
/// keep population within them.
pub const TARGET_ACADEMY_SIZE_BY_TIER: [(u8, u8); 11] = [
    (0, 0),   // index 0 unused — tier is 1-based
    (22, 34), // tier 1
    (24, 36), // tier 2
    (26, 38), // tier 3
    (28, 42), // tier 4
    (30, 44), // tier 5
    (32, 46), // tier 6
    (34, 50), // tier 7
    (36, 54), // tier 8
    (38, 58), // tier 9
    (40, 62), // tier 10
];

/// One source of truth for academy-wide knobs. Cloned into the
/// `ClubAcademy` at construction so per-club overrides remain possible
/// later without touching the rest of the code.
#[derive(Debug, Clone)]
pub struct AcademyTuning {
    /// Month (1..12) the annual intake fires.
    pub intake_month: u32,
    /// Smallest candidate pool the recruiter is willing to evaluate.
    pub min_pool_size: usize,
    /// Hard upper bound on a single year's intake.
    pub max_intake: usize,
    /// Soft cap on total academy population.
    pub max_academy_players: usize,
    /// Tier → (min, max) target academy population.
    pub target_academy_players_by_tier: [(u8, u8); 11],
    /// PA threshold above which a prospect counts as "elite".
    pub elite_pa_threshold: u8,
    /// PA threshold for "world-class" prospects.
    pub world_class_pa_threshold: u8,
}

impl Default for AcademyTuning {
    fn default() -> Self {
        AcademyTuning {
            intake_month: 7,
            min_pool_size: 12,
            max_intake: 12,
            max_academy_players: 64,
            target_academy_players_by_tier: TARGET_ACADEMY_SIZE_BY_TIER,
            elite_pa_threshold: 160,
            world_class_pa_threshold: 180,
        }
    }
}

impl AcademyTuning {
    /// Per-tier (min, max) target. Tier is clamped to 1..10.
    pub fn target_size(&self, tier: AcademyTier) -> (u8, u8) {
        let idx = tier.0 as usize;
        self.target_academy_players_by_tier[idx]
    }
}

/// 1..10 short-scale pathway tier derived from a 1..20 facility rating.
/// All academy formulas that previously took the raw 1..20 level go
/// through this newtype so a one-place change keeps the storage and
/// math in sync.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AcademyTier(pub u8);

impl AcademyTier {
    /// Collapse the 1..20 facility-rating scale into the 1..10 tier.
    pub fn from_level(level: u8) -> Self {
        let lvl = level.clamp(1, 20) as u16;
        AcademyTier((((lvl + 1) / 2) as u8).clamp(1, 10))
    }

    /// Raw tier number, 1..10.
    pub fn value(self) -> u8 {
        self.0
    }

    /// Normalised tier in 0.1..1.0 — what most academy formulas
    /// multiply against.
    pub fn norm(self) -> f32 {
        self.0 as f32 / 10.0
    }

    /// 0..100 readiness threshold required to graduate at this tier.
    /// Higher-tier pathways set a higher bar because the U18 they feed
    /// into is already strong.
    pub fn readiness_threshold(self) -> i16 {
        match self.0 {
            1..=3 => 58,
            4..=6 => 64,
            7..=8 => 70,
            _ => 75,
        }
    }

    /// Weekly training sessions for a given phase / tier combination.
    ///
    /// `phase`: 0 = Foundation (ages 8-11), 1 = Development (12-14),
    /// 2 = Professional (15-17).
    pub fn sessions_for_phase(self, phase: u8) -> u8 {
        let bracket = if self.0 <= 3 {
            0
        } else if self.0 <= 6 {
            1
        } else if self.0 <= 9 {
            2
        } else {
            3
        };
        match phase {
            0 => [2, 3, 3, 4][bracket],
            1 => [3, 4, 5, 5][bracket],
            _ => [3, 4, 5, 6][bracket],
        }
    }
}
