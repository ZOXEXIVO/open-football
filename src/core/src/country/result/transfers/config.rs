//! Tuning knobs for the country-level transfer subsystem.
//!
//! Constants here used to live as inline literals across `free_agents.rs`
//! and `execution.rs` — match arms scattered through 400+ lines of logic.
//! Centralising them lets balance tuning happen as a config edit, makes
//! the simulator's behaviour auditable from a single place, and gives us
//! a hook for per-difficulty / per-save overrides later.
//!
//! All values are deliberately `pub` so tests can override individual
//! tiers; the simulation reads via `TransferConfig::default()` for now.

/// Daily probability that a free agent of a given calibre is signed by
/// any one club whose unfulfilled transfer request matches their position
/// and ability floor. Independent rolls per club-need pair.
#[derive(Debug, Clone, Copy)]
pub struct FreeAgentProbability {
    /// Inclusive lower bound on `current_ability` for this tier.
    pub ability_floor: u8,
    /// Daily probability percentage at the floor.
    pub min_chance_pct: f32,
    /// Daily probability percentage at the next tier's floor (linear interp).
    pub max_chance_pct: f32,
}

#[derive(Debug, Clone)]
pub struct TransferConfig {
    // ── Free agent signing tiers ──────────────────────────────────
    /// Probability tiers, ordered from elite → low. Each tier covers
    /// `[ability_floor, next_tier.ability_floor)` and interpolates the
    /// daily chance linearly across the band.
    pub free_agent_tiers: Vec<FreeAgentProbability>,

    /// Multiplier applied to the daily chance for older players. Indexed
    /// by `(age_floor, multiplier)`; the largest `age_floor ≤ player_age`
    /// wins. Empty band means no penalty.
    pub free_agent_age_multipliers: Vec<(u8, f32)>,

    /// Boost applied when a young player has clear room to grow.
    pub young_potential_age_max: u8,
    pub young_potential_gap_min: u8,
    pub young_potential_multiplier: f32,

    /// Final clamp on the daily chance percentage after all multipliers.
    pub daily_chance_min_pct: f32,
    pub daily_chance_max_pct: f32,

    // ── Per-tick limits ───────────────────────────────────────────
    /// Hard cap on free-agent signings completed per country per day.
    /// Prevents the matcher from emptying the pool in a single tick when
    /// many clubs all have the same gap.
    pub max_free_agent_signings_per_day: usize,

    /// Slack on the requested `min_ability` filter — clubs accept a free
    /// agent slightly below their nominal target because the price (zero
    /// fee, possibly lower wage) compensates.
    pub free_agent_ability_slack: u8,

    /// Maximum allowed `player.world_reputation - club.world_reputation`
    /// gap for a free-agent signing. Above this, the player is treated as
    /// out-of-reach for the buyer (a Ballon d'Or-tier player won't drop
    /// into a third-division side regardless of country). This is a hard
    /// reject — distinct from the scouting `WageDemands` risk flag, which
    /// is informational and uses a smaller gap.
    pub free_agent_world_rep_gap_max: i16,
}

impl Default for TransferConfig {
    fn default() -> Self {
        TransferConfig {
            free_agent_tiers: vec![
                // Elite: 25% daily flat from ability 160 upwards.
                FreeAgentProbability {
                    ability_floor: 160,
                    min_chance_pct: 25.0,
                    max_chance_pct: 25.0,
                },
                // Good: 5% at 130, scales to 25% just below 160.
                FreeAgentProbability {
                    ability_floor: 130,
                    min_chance_pct: 5.0,
                    max_chance_pct: 25.0,
                },
                // Average: 1.5% at 100, scales to 5% near 130.
                FreeAgentProbability {
                    ability_floor: 100,
                    min_chance_pct: 1.5,
                    max_chance_pct: 5.0,
                },
                // Below average: 0.3% at 60, scales to 1.5% near 100.
                FreeAgentProbability {
                    ability_floor: 60,
                    min_chance_pct: 0.3,
                    max_chance_pct: 1.5,
                },
                // Low quality: 0.1% at 0, scales to 0.3% near 60.
                FreeAgentProbability {
                    ability_floor: 0,
                    min_chance_pct: 0.1,
                    max_chance_pct: 0.3,
                },
            ],
            free_agent_age_multipliers: vec![
                (0, 1.00),
                (30, 0.80),
                (32, 0.50),
                (34, 0.30),
                (36, 0.15),
            ],
            young_potential_age_max: 24,
            young_potential_gap_min: 20,
            young_potential_multiplier: 1.5,
            daily_chance_min_pct: 0.1,
            daily_chance_max_pct: 30.0,
            max_free_agent_signings_per_day: 2,
            free_agent_ability_slack: 5,
            free_agent_world_rep_gap_max: 2500,
        }
    }
}

impl TransferConfig {
    /// Resolve the daily signing chance for a free agent of `ability`. Returns
    /// a percentage in `[0, 100]` before age / potential modifiers.
    pub fn free_agent_base_chance(&self, ability: u8) -> f32 {
        // Tiers are stored elite-first. Find the highest tier whose floor
        // the player meets, then linearly interpolate within that band
        // toward the next tier's floor (which sets the band's upper edge).
        let mut chosen_idx = self.free_agent_tiers.len().saturating_sub(1);
        for (i, tier) in self.free_agent_tiers.iter().enumerate() {
            if ability >= tier.ability_floor {
                chosen_idx = i;
                break;
            }
        }
        let tier = &self.free_agent_tiers[chosen_idx];
        // The next tier *up* (smaller index) caps this band; if we're at
        // the elite tier, both ends collapse to the elite chance.
        let band_top = chosen_idx
            .checked_sub(1)
            .map(|i| self.free_agent_tiers[i].ability_floor)
            .unwrap_or(tier.ability_floor);
        if band_top <= tier.ability_floor {
            return tier.min_chance_pct;
        }
        let band_size = (band_top - tier.ability_floor) as f32;
        let pos = (ability.saturating_sub(tier.ability_floor)) as f32;
        let fraction = (pos / band_size).clamp(0.0, 1.0);
        tier.min_chance_pct + (tier.max_chance_pct - tier.min_chance_pct) * fraction
    }

    /// Age multiplier for free-agent signing chance. Picks the largest
    /// `age_floor ≤ age` from the configured table; falls back to 1.0
    /// when the table is empty or the age sits below every floor.
    pub fn free_agent_age_multiplier(&self, age: u8) -> f32 {
        let mut multiplier = 1.0;
        for &(floor, m) in &self.free_agent_age_multipliers {
            if age >= floor {
                multiplier = m;
            }
        }
        multiplier
    }

    /// Combined daily chance after age and young-potential adjustments.
    /// Returns a percentage clamped to `[daily_chance_min_pct, daily_chance_max_pct]`.
    pub fn daily_signing_chance(&self, ability: u8, potential: u8, age: u8) -> f32 {
        let base = self.free_agent_base_chance(ability);
        let age_factor = self.free_agent_age_multiplier(age);
        let potential_boost = if age < self.young_potential_age_max
            && potential > ability + self.young_potential_gap_min
        {
            self.young_potential_multiplier
        } else {
            1.0
        };
        (base * age_factor * potential_boost)
            .clamp(self.daily_chance_min_pct, self.daily_chance_max_pct)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elite_player_lands_at_top_of_band() {
        let cfg = TransferConfig::default();
        assert!((cfg.free_agent_base_chance(180) - 25.0).abs() < f32::EPSILON);
        assert!((cfg.free_agent_base_chance(160) - 25.0).abs() < f32::EPSILON);
    }

    #[test]
    fn good_band_interpolates_linearly() {
        let cfg = TransferConfig::default();
        // ability=130 → 5.0 (band floor)
        // ability=160 → 25.0 (band top — lives in elite tier)
        // ability=145 → ~15.0 (midway)
        let mid = cfg.free_agent_base_chance(145);
        assert!((mid - 15.0).abs() < 0.01, "expected ~15.0, got {}", mid);
    }

    #[test]
    fn aged_player_chance_drops() {
        let cfg = TransferConfig::default();
        let young = cfg.daily_signing_chance(140, 145, 25);
        let old = cfg.daily_signing_chance(140, 145, 35);
        assert!(old < young, "old={old}, young={young}");
    }

    #[test]
    fn young_high_potential_gets_boost() {
        let cfg = TransferConfig::default();
        let plain = cfg.daily_signing_chance(80, 85, 22);
        let prospect = cfg.daily_signing_chance(80, 130, 22);
        assert!(prospect > plain * 1.4, "plain={plain}, prospect={prospect}");
    }

    #[test]
    fn chance_is_clamped_to_max() {
        let cfg = TransferConfig::default();
        // Inputs that would otherwise multiply past 30%.
        let chance = cfg.daily_signing_chance(180, 200, 22);
        assert!(chance <= cfg.daily_chance_max_pct + f32::EPSILON);
    }
}
