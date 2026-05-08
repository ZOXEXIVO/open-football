//! Free-agent market decay state. Carries the durable signals that drive
//! the career-pressure model: how long the player has been free, what
//! market they came from, how often clubs have come knocking, how many
//! transfer windows have passed without a deal.
//!
//! Without this, the matcher only sees nationality reputation — a
//! Russian free agent stays "too good for Malta" forever, even after a
//! year of unemployment. The pressure score derived from these fields
//! lowers wage demands, widens acceptable destinations, and (eventually)
//! triggers retirement.

use crate::club::player::calculators::WageCalculator;
use crate::club::player::player::Player;
use crate::{Person, PlayerSquadStatus};
use chrono::{Datelike, NaiveDate};

/// Snapshot of where the player came from and how the market has treated
/// them since. Populated when the player enters the free-agent pool;
/// updated by `on_offer_*` while they sit there; cleared on signing.
#[derive(Debug, Clone)]
pub struct FreeAgentMarketState {
    pub free_since: NaiveDate,

    pub last_club_id: Option<u32>,
    pub last_country_id: Option<u32>,

    /// Reputation (0–10000) of the country whose league the player last
    /// played in. For nationality-only inferences (database free agents
    /// with no club history), seeded from nationality reputation.
    pub last_country_reputation: u16,
    /// Reputation (0–10000) of the league the player last played in.
    /// Inferred at 0.75 × country rep when no club history is known.
    pub last_league_reputation: u16,
    /// Club reputation `world` value (0–10000) of the player's last
    /// club, normalised at the call site to [0,1] via `/ 10_000.0`.
    pub last_club_reputation_score: f32,

    pub last_salary: u32,
    pub last_squad_status: PlayerSquadStatus,

    /// Bounded log of dates when offers landed; used to recompute the
    /// 30-day window without storing a separate stale counter.
    pub recent_offer_dates: Vec<NaiveDate>,
    pub offers_rejected_total: u16,
}

impl FreeAgentMarketState {
    /// Number of offers received in the last 30 days. Computed from
    /// `recent_offer_dates`; the helper prunes stale entries on every
    /// `on_offer_received` so the vector stays small.
    pub fn offers_received_30d(&self, today: NaiveDate) -> u8 {
        let cutoff = today - chrono::Duration::days(30);
        self.recent_offer_dates
            .iter()
            .filter(|d| **d >= cutoff)
            .count()
            .min(255) as u8
    }

    /// Whole transfer windows that have closed since the player went
    /// free. Stateless: derived from a fixed schedule of two annual
    /// closes (Aug 31 summer, Jan 31 winter) so it stays correct after
    /// loads and doesn't drift if `on_window_closed` calls are missed.
    pub fn transfer_windows_missed(&self, today: NaiveDate) -> u8 {
        Self::windows_closed_between(self.free_since, today)
    }

    pub(crate) fn windows_closed_between(from: NaiveDate, to: NaiveDate) -> u8 {
        if to <= from {
            return 0;
        }
        let mut count: u32 = 0;
        let mut year = from.year();
        while year <= to.year() {
            // Both close events sit within the same calendar year:
            // winter on Jan 31, summer on Aug 31. Counting them in
            // adjacent years would skew long sits by 1.
            if let Some(winter) = NaiveDate::from_ymd_opt(year, 1, 31) {
                if winter > from && winter <= to {
                    count += 1;
                }
            }
            if let Some(summer) = NaiveDate::from_ymd_opt(year, 8, 31) {
                if summer > from && summer <= to {
                    count += 1;
                }
            }
            year += 1;
        }
        count.min(255) as u8
    }
}

/// Debug / behaviour-band label for a free agent's position on the
/// decay curve. Maps the continuous `career_pressure` score onto the
/// five qualitative stages from the design model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarketStage {
    Fresh,
    Open,
    Flexible,
    Desperate,
    LastChance,
}

impl MarketStage {
    pub fn from_days_free(days_free: i64) -> Self {
        match days_free {
            i if i < 30 => MarketStage::Fresh,
            i if i < 90 => MarketStage::Open,
            i if i < 180 => MarketStage::Flexible,
            i if i < 365 => MarketStage::Desperate,
            _ => MarketStage::LastChance,
        }
    }
}

/// Inputs for `Player::on_release`. Bundled because every release path
/// needs the same context — the buying-side `TransferCompletion` /
/// `LoanCompletion` precedent gives us the convention.
pub struct ReleaseContext {
    pub date: NaiveDate,
    pub last_club_id: Option<u32>,
    pub last_country_id: Option<u32>,
    pub last_country_reputation: u16,
    pub last_league_reputation: u16,
    pub last_club_reputation_score: f32,
    pub last_salary: u32,
    pub last_squad_status: PlayerSquadStatus,
}

impl Player {
    /// Read-only access to the player's free-agent market state. `None`
    /// when the player is signed; `Some` whenever they sit in the global
    /// free-agent pool (or just got released and the daily sweep hasn't
    /// moved them yet).
    pub fn free_agent_state(&self) -> Option<&FreeAgentMarketState> {
        self.free_agent_state.as_ref()
    }

    /// Stamp the player as just-released and seed their market-state
    /// snapshot. Idempotent only for the *first* release in a sit —
    /// calling it a second time would reset `free_since` and erase the
    /// pressure built up so far. Callers must check `free_agent_state`
    /// is `None` before invoking.
    ///
    /// Distinct from `Player::on_release` (in `statistics::processing`)
    /// which owns the *stats history* side of release. This one owns
    /// the *market state* side; a complete release fires both.
    pub fn enter_free_agent_market(&mut self, ctx: ReleaseContext) {
        self.free_agent_state = Some(FreeAgentMarketState {
            free_since: ctx.date,
            last_club_id: ctx.last_club_id,
            last_country_id: ctx.last_country_id,
            last_country_reputation: ctx.last_country_reputation,
            last_league_reputation: ctx.last_league_reputation,
            last_club_reputation_score: ctx.last_club_reputation_score,
            last_salary: ctx.last_salary,
            last_squad_status: ctx.last_squad_status,
            recent_offer_dates: Vec::new(),
            offers_rejected_total: 0,
        });
    }

    /// Lazy initializer for database-only free agents who never came
    /// through `on_release` (the simulation booted with them already in
    /// the pool, so we have nothing but their nationality and ability
    /// to go on). Idempotent — the state is only seeded when missing.
    ///
    /// `nationality_country_reputation` is the rep value the snapshot
    /// path resolves from `country` / `country_info` — passing it in
    /// keeps this method free of SimulatorData borrows.
    pub fn ensure_free_agent_state(
        &mut self,
        date: NaiveDate,
        nationality_country_reputation: u16,
    ) {
        if self.free_agent_state.is_some() {
            return;
        }
        let nat_rep = nationality_country_reputation;
        let last_league_rep = ((nat_rep as f32) * 0.75) as u16;
        let club_score = (nat_rep as f32 / 10_000.0).clamp(0.0, 1.0) * 0.35;
        let inferred_salary = WageCalculator::expected_annual_wage(
            self,
            self.age(date),
            club_score,
            last_league_rep,
        );
        // Seed `free_since` 30 days in the past so a fresh database
        // free agent isn't treated as "released yesterday". They've
        // been on the market — the engine just hasn't been simulating
        // their sit until now.
        let free_since = date - chrono::Duration::days(30);
        self.free_agent_state = Some(FreeAgentMarketState {
            free_since,
            last_club_id: None,
            last_country_id: Some(self.country_id),
            last_country_reputation: nat_rep,
            last_league_reputation: last_league_rep,
            last_club_reputation_score: club_score,
            last_salary: inferred_salary,
            last_squad_status: PlayerSquadStatus::FirstTeamSquadRotation,
            recent_offer_dates: Vec::new(),
            offers_rejected_total: 0,
        });
    }

    /// Record a fresh offer landing on this player. Prunes the rolling
    /// window so `offers_received_30d` stays accurate without a daily
    /// sweep. No-op if the player isn't a free agent.
    pub fn on_offer_received(&mut self, date: NaiveDate) {
        if let Some(state) = self.free_agent_state.as_mut() {
            let cutoff = date - chrono::Duration::days(30);
            state.recent_offer_dates.retain(|d| *d >= cutoff);
            state.recent_offer_dates.push(date);
        }
    }

    /// The player turned down an offer they received. Bumps the
    /// running rejected counter (one signal that they're being too
    /// picky). No-op if not a free agent.
    pub fn on_offer_rejected(&mut self) {
        if let Some(state) = self.free_agent_state.as_mut() {
            state.offers_rejected_total = state.offers_rejected_total.saturating_add(1);
        }
    }

    /// Drop the market state — the player just signed somewhere. Called
    /// from `complete_transfer` and `complete_free_agent_signing` so
    /// no path that re-clubs the player leaves stale state behind.
    pub fn clear_free_agent_state(&mut self) {
        self.free_agent_state = None;
    }

    /// Career pressure score in [0,1] — the master signal that drives
    /// every gate in the decay model. Higher means more willing to
    /// accept low offers, drop-tier moves, and (eventually) retire.
    /// Returns 0 when the player has no market state (i.e. is signed).
    pub fn career_pressure(&self, today: NaiveDate) -> f32 {
        let Some(state) = self.free_agent_state.as_ref() else {
            return 0.0;
        };

        let age = self.age(today);
        let ca = self.player_attributes.current_ability;

        let days_free = (today - state.free_since).num_days().max(0);
        let months_free = days_free as f32 / 30.0;
        let windows_missed = state.transfer_windows_missed(today) as f32;
        let offers_rejected = state.offers_rejected_total as f32;
        let offers_30d = state.offers_received_30d(today);

        let age_pressure = if age < 22 {
            -0.10
        } else if age < 28 {
            0.00
        } else if age < 32 {
            0.08
        } else if age < 35 {
            0.18
        } else {
            0.30
        };

        let quality_pressure = if ca >= 140 {
            -0.15
        } else if ca >= 110 {
            -0.05
        } else if ca >= 80 {
            0.05
        } else {
            0.15
        };

        let interest_pressure = match offers_30d {
            0 => 0.10,
            1..=2 => -0.05,
            _ => -0.15,
        };

        let raw = 0.025 * months_free
            + 0.18 * windows_missed
            + 0.04 * offers_rejected
            + age_pressure
            + quality_pressure
            + interest_pressure;
        raw.clamp(0.0, 1.0)
    }

    /// Qualitative band the player sits in on the decay curve. Used for
    /// debug labels and as an optional brake on extreme moves
    /// (semi-pro/amateur destinations are LastChance-only).
    pub fn market_stage(&self, today: NaiveDate) -> Option<MarketStage> {
        let state = self.free_agent_state.as_ref()?;
        let days = (today - state.free_since).num_days().max(0);
        Some(MarketStage::from_days_free(days))
    }

    /// Reference-reputation anchor for the buyer's prestige gate. Reads
    /// the player's last-known market and nationality and merges them so
    /// callers don't have to re-derive the formula. `nationality_rep` is
    /// passed in because it lives outside `Player` (resolved from
    /// `Country` / `country_info` at the matcher's call site).
    pub fn reference_reputation(&self, nationality_rep: u16) -> u16 {
        let from_state = self.free_agent_state.as_ref().map(|s| {
            (s.last_country_reputation as f32) * 0.4 + (s.last_league_reputation as f32) * 0.6
        });
        let from_nationality = (nationality_rep as f32) * 0.6;
        let max = from_state
            .map(|v| v.max(from_nationality))
            .unwrap_or(from_nationality);
        max.round().clamp(0.0, u16::MAX as f32) as u16
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::club::player::builder::PlayerBuilder;
    use crate::shared::fullname::FullName;
    use crate::{
        PersonAttributes, PlayerAttributes, PlayerPosition, PlayerPositionType, PlayerPositions,
        PlayerSkills,
    };

    fn person() -> PersonAttributes {
        PersonAttributes {
            adaptability: 10.0,
            ambition: 10.0,
            controversy: 10.0,
            loyalty: 10.0,
            pressure: 10.0,
            professionalism: 10.0,
            sportsmanship: 10.0,
            temperament: 10.0,
            consistency: 10.0,
            important_matches: 10.0,
            dirtiness: 10.0,
        }
    }

    fn make_player(ca: u8, age: u8, today: NaiveDate) -> Player {
        let mut attrs = PlayerAttributes::default();
        attrs.current_ability = ca;
        attrs.potential_ability = ca;
        attrs.current_reputation = (ca as i16) * 30;
        let birth = today
            .checked_sub_signed(chrono::Duration::days(age as i64 * 365))
            .unwrap();
        PlayerBuilder::new()
            .id(1)
            .full_name(FullName::new("Test".to_string(), "Player".to_string()))
            .birth_date(birth)
            .country_id(1)
            .attributes(person())
            .skills(PlayerSkills::default())
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: PlayerPositionType::MidfielderCenter,
                    level: 20,
                }],
            })
            .player_attributes(attrs)
            .build()
            .unwrap()
    }

    #[test]
    fn fresh_release_yields_low_pressure() {
        let today = NaiveDate::from_ymd_opt(2026, 5, 8).unwrap();
        let mut p = make_player(120, 26, today);
        p.enter_free_agent_market(ReleaseContext {
            date: today,
            last_club_id: Some(10),
            last_country_id: Some(1),
            last_country_reputation: 6000,
            last_league_reputation: 7000,
            last_club_reputation_score: 0.6,
            last_salary: 500_000,
            last_squad_status: PlayerSquadStatus::FirstTeamRegular,
        });
        let pressure = p.career_pressure(today);
        assert!(pressure < 0.20, "fresh pressure too high: {pressure}");
    }

    #[test]
    fn old_low_quality_long_unemployed_player_caps_near_one() {
        let release = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
        let today = NaiveDate::from_ymd_opt(2026, 5, 8).unwrap();
        let mut p = make_player(50, 36, today);
        p.enter_free_agent_market(ReleaseContext {
            date: release,
            last_club_id: Some(10),
            last_country_id: Some(1),
            last_country_reputation: 1500,
            last_league_reputation: 1200,
            last_club_reputation_score: 0.2,
            last_salary: 30_000,
            last_squad_status: PlayerSquadStatus::MainBackupPlayer,
        });
        let pressure = p.career_pressure(today);
        assert!(pressure > 0.85, "expected near-cap pressure, got {pressure}");
    }

    #[test]
    fn windows_closed_between_counts_summer_and_winter_closes() {
        let from = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let to = NaiveDate::from_ymd_opt(2027, 2, 28).unwrap();
        // 2026-01-31 (winter), 2026-08-31 (summer), 2027-01-31 (winter) => 3
        assert_eq!(FreeAgentMarketState::windows_closed_between(from, to), 3);
    }

    #[test]
    fn offers_in_last_30_days_prunes_old_entries() {
        let today = NaiveDate::from_ymd_opt(2026, 5, 8).unwrap();
        let mut p = make_player(100, 25, today);
        p.enter_free_agent_market(ReleaseContext {
            date: today - chrono::Duration::days(120),
            last_club_id: Some(10),
            last_country_id: Some(1),
            last_country_reputation: 4000,
            last_league_reputation: 4000,
            last_club_reputation_score: 0.4,
            last_salary: 200_000,
            last_squad_status: PlayerSquadStatus::FirstTeamRegular,
        });
        // Offer 60 days ago: outside the 30d window.
        p.on_offer_received(today - chrono::Duration::days(60));
        // Offer today: inside.
        p.on_offer_received(today);
        let state = p.free_agent_state().unwrap();
        assert_eq!(state.offers_received_30d(today), 1);
    }

    #[test]
    fn ensure_state_is_idempotent() {
        let today = NaiveDate::from_ymd_opt(2026, 5, 8).unwrap();
        let mut p = make_player(100, 27, today);
        p.ensure_free_agent_state(today, 5000);
        let first = p.free_agent_state().unwrap().free_since;
        p.ensure_free_agent_state(today, 9999);
        let second = p.free_agent_state().unwrap().free_since;
        assert_eq!(first, second, "ensure_free_agent_state must be idempotent");
    }

    #[test]
    fn market_stage_thresholds() {
        assert_eq!(MarketStage::from_days_free(0), MarketStage::Fresh);
        assert_eq!(MarketStage::from_days_free(29), MarketStage::Fresh);
        assert_eq!(MarketStage::from_days_free(30), MarketStage::Open);
        assert_eq!(MarketStage::from_days_free(89), MarketStage::Open);
        assert_eq!(MarketStage::from_days_free(90), MarketStage::Flexible);
        assert_eq!(MarketStage::from_days_free(179), MarketStage::Flexible);
        assert_eq!(MarketStage::from_days_free(180), MarketStage::Desperate);
        assert_eq!(MarketStage::from_days_free(364), MarketStage::Desperate);
        assert_eq!(MarketStage::from_days_free(365), MarketStage::LastChance);
    }

    #[test]
    fn reference_reputation_takes_max_of_state_and_nationality() {
        let today = NaiveDate::from_ymd_opt(2026, 5, 8).unwrap();
        let mut p = make_player(100, 27, today);
        p.enter_free_agent_market(ReleaseContext {
            date: today,
            last_club_id: Some(10),
            last_country_id: Some(1),
            last_country_reputation: 5000,
            last_league_reputation: 6000,
            last_club_reputation_score: 0.5,
            last_salary: 200_000,
            last_squad_status: PlayerSquadStatus::FirstTeamRegular,
        });
        // last_country=5000 * 0.4 + last_league=6000 * 0.6 = 2000 + 3600 = 5600
        // nationality 8000 * 0.6 = 4800 — 5600 wins.
        assert_eq!(p.reference_reputation(8000), 5600);
        // Nationality dominates if both lasts are weaker.
        assert!(p.reference_reputation(20_000) > 5600);
    }
}
