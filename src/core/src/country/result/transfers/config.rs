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

    /// How many ranked candidates the request-driven matcher tries per
    /// request per tick before giving up. The legacy behaviour was 1 —
    /// if the single best candidate failed its daily roll or rejected
    /// terms the request was skipped for the day, which let one
    /// unrealistic strong candidate starve every realistic one behind
    /// them.
    pub free_agent_attempts_per_request: usize,

    // ── Long-term market clearing — runs AFTER the emergency and
    //    request-driven passes. Resolves free agents who have sat so
    //    long that waiting for an explicit transfer request is no
    //    longer realistic: they take a modest squad-role deal at a
    //    lower-tier club instead.
    //
    //    Two layers run back to back:
    //      • Soft clearing kicks in early (90 days OR pressure 0.45)
    //        but only matches DOMESTIC / same-region clubs and only
    //        when an opportunistic squad-fit gate passes — the
    //        realistic "a local club takes a punt on a useful free
    //        body" outcome. Tight per-day cap.
    //      • Hard clearing is the long-tail backstop (365 days OR
    //        pressure 0.75) with the broader region / reputation
    //        tolerance, for players the soft layer's locality
    //        restriction never reached.
    /// Career-pressure floor for SOFT (early, domestic-only) clearing.
    pub soft_market_clearing_min_pressure: f32,
    /// Days-free floor for SOFT clearing (either criterion qualifies).
    pub soft_market_clearing_min_days_free: i64,
    /// Per-country per-day cap on soft-clearing signings. Deliberately
    /// tiny (1) so the early domestic layer trickles rather than floods.
    pub soft_market_clearing_max_signings_per_country_per_day: usize,
    /// Career-pressure floor for HARD (long-tail, broad) clearing.
    pub hard_market_clearing_min_pressure: f32,
    /// Days-free floor for HARD clearing (either criterion qualifies).
    pub hard_market_clearing_min_days_free: i64,
    /// Per-country per-day cap on hard-clearing signings, so the
    /// fallback drains the long-tail pool gradually instead of mass-
    /// clearing it the first day someone crosses the threshold.
    pub market_clearing_max_signings_per_country_per_day: usize,

    // ── Emergency squad fill — runs BEFORE the normal request-driven
    //    matcher. Keeps a club below `MIN_FIRST_TEAM_SQUAD` from
    //    waiting weeks for the standard scouting / shortlist pipeline.
    /// Per-country hard cap on emergency signings completed per tick.
    /// Sits on top of `max_free_agent_signings_per_day` rather than
    /// consuming it — emergency clubs aren't competing for the same
    /// slot count as normal day-to-day activity.
    pub emergency_max_signings_per_country_per_day: usize,
    /// Per-club hard cap. A single underfilled club shouldn't be
    /// allowed to sign 25 players in one day; spreading the catch-up
    /// across a few ticks keeps the pool from being drained and
    /// matches how real markets fill emergency gaps.
    pub emergency_max_signings_per_club_per_day: usize,
    /// Above this main-team headcount the emergency pass stops
    /// running — slightly below `MIN_FIRST_TEAM_SQUAD` so the normal
    /// pipeline takes the last few slots through proper scouting.
    pub emergency_squad_size_threshold: usize,

    /// Minimum playable squad size — emergency fill keeps signing
    /// until a club reaches this regardless of the regular per-club
    /// cap. Mirrors the formation requirement: anything below 11 is
    /// "can't field a side" and warrants unconditional catch-up
    /// within the available pool.
    pub emergency_min_playable_size: usize,

    /// When the projected squad is below
    /// [`Self::emergency_min_playable_size`], the per-club cap is
    /// lifted to *at least* this many signings so a 0-player club can
    /// reach 11 in a single tick when candidates exist. Country cap
    /// still applies on top.
    pub emergency_urgent_per_club_cap_floor: usize,
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
            free_agent_attempts_per_request: 3,
            // Soft clearing: early, domestic-only, opportunistic. Either
            // ~3 months free or a 0.45 pressure score opens it; capped to
            // a single signing per country per day so the long tail
            // resolves gradually through realistic local fits.
            soft_market_clearing_min_pressure: 0.45,
            soft_market_clearing_min_days_free: 90,
            soft_market_clearing_max_signings_per_country_per_day: 1,
            // Hard clearing: the long-tail backstop, broad region /
            // reputation tolerance, capped at 2 per country per day.
            hard_market_clearing_min_pressure: 0.75,
            hard_market_clearing_min_days_free: 365,
            market_clearing_max_signings_per_country_per_day: 2,
            emergency_max_signings_per_country_per_day: 20,
            emergency_max_signings_per_club_per_day: 5,
            // 18 keeps a small buffer below the 25 minimum so the
            // normal scouting pipeline can take over once the squad
            // is at least playable. Picking 25 here would have the
            // emergency pass aggressively churn through low-quality
            // free agents right up to the cap and starve the proper
            // recruitment flow of work.
            emergency_squad_size_threshold: 18,
            // The minimum body count to field a side. Below this the
            // urgent path lifts caps so the club can become playable
            // within a single tick when the pool allows it.
            emergency_min_playable_size: 11,
            // Raises per-club cap from the regular 5 to 11 when below
            // 11 players — enough to bridge a 0-player roster to a
            // playable side in one tick. Country cap still applies.
            emergency_urgent_per_club_cap_floor: 11,
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
