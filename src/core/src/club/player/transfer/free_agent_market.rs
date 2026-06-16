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
use chrono::Duration;
use chrono::{Datelike, NaiveDate};

/// Why the matcher most recently passed over a free agent. One value
/// per player, refreshed every time a country's matcher evaluates the
/// candidate and skips them — the diagnosis layer reads it to answer
/// "why is this long-term free agent still unsigned?". Ordered by
/// `rank`: the closer the candidate got to an actual signing, the
/// higher the rank, and the per-tick merge keeps only the
/// highest-ranked reason so a near-miss isn't overwritten by a
/// coarse early-gate rejection from another buyer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FreeAgentBlockReason {
    /// Nationality country could not be resolved — the snapshot
    /// fail-closed fallback (`u16::MAX` reference reputation) blocks
    /// every buyer. Data problem, not a market outcome.
    UnknownNationality,
    /// No club in any processed country holds an open transfer
    /// request matching the player's position group.
    NoMatchingRequest,
    /// Candidate's position group didn't match the request under
    /// evaluation.
    PositionMismatch,
    /// Another signing or staged negotiation already claimed the
    /// player this tick.
    AlreadySignedOrStaged,
    /// Below the buyer tier's minimum acceptable CA.
    BelowMinimumAbility,
    /// Above the buyer tier's ceiling — a star slumming gate.
    AboveMaximumAbility,
    /// Buyer country reputation too far below the player's reference
    /// reputation even with the pressure-widened allowance.
    CountryReputationGap,
    /// Cross-continent step-down blocked: career pressure below the
    /// floor the hard gate requires.
    CrossContinentPressureTooLow,
    /// Player's home region too prestigious for the buyer's region.
    RegionPrestigeGap,
    /// Buying club has no roster room left.
    ClubAtSquadCapacity,
    /// The country's per-day free-agent signing cap was already
    /// consumed before this candidate could be tried.
    PerDaySigningCapReached,
    /// The club's daily approach roll didn't come up — no offer made
    /// today.
    DailyChanceRollFailed,
    /// Offer was made but the wage sat far below the player's
    /// reservation — the dominant cause of the failed acceptance.
    WageReservationMismatch,
    /// Offer was made and declined on the overall acceptance roll.
    AcceptanceRollFailed,
}

impl FreeAgentBlockReason {
    /// How far through the matching funnel the candidate got before
    /// being blocked. Higher = closer to signing = more informative.
    pub fn rank(self) -> u8 {
        match self {
            FreeAgentBlockReason::UnknownNationality => 0,
            FreeAgentBlockReason::NoMatchingRequest => 1,
            FreeAgentBlockReason::PositionMismatch => 2,
            FreeAgentBlockReason::AlreadySignedOrStaged => 3,
            FreeAgentBlockReason::BelowMinimumAbility => 4,
            FreeAgentBlockReason::AboveMaximumAbility => 5,
            FreeAgentBlockReason::CountryReputationGap => 6,
            FreeAgentBlockReason::CrossContinentPressureTooLow => 7,
            FreeAgentBlockReason::RegionPrestigeGap => 8,
            FreeAgentBlockReason::ClubAtSquadCapacity => 9,
            FreeAgentBlockReason::PerDaySigningCapReached => 10,
            FreeAgentBlockReason::DailyChanceRollFailed => 11,
            FreeAgentBlockReason::WageReservationMismatch => 12,
            FreeAgentBlockReason::AcceptanceRollFailed => 13,
        }
    }

    /// Stable label for debug output / diagnosis dumps.
    pub fn label(self) -> &'static str {
        match self {
            FreeAgentBlockReason::UnknownNationality => "unknown_nationality",
            FreeAgentBlockReason::NoMatchingRequest => "no_matching_request",
            FreeAgentBlockReason::PositionMismatch => "position_mismatch",
            FreeAgentBlockReason::AlreadySignedOrStaged => "already_signed_or_staged",
            FreeAgentBlockReason::BelowMinimumAbility => "below_minimum_ability",
            FreeAgentBlockReason::AboveMaximumAbility => "above_maximum_ability",
            FreeAgentBlockReason::CountryReputationGap => "country_reputation_gap",
            FreeAgentBlockReason::CrossContinentPressureTooLow => {
                "cross_continent_pressure_too_low"
            }
            FreeAgentBlockReason::RegionPrestigeGap => "region_prestige_gap",
            FreeAgentBlockReason::ClubAtSquadCapacity => "club_at_squad_capacity",
            FreeAgentBlockReason::PerDaySigningCapReached => "per_day_signing_cap_reached",
            FreeAgentBlockReason::DailyChanceRollFailed => "daily_chance_roll_failed",
            FreeAgentBlockReason::WageReservationMismatch => "wage_reservation_mismatch",
            FreeAgentBlockReason::AcceptanceRollFailed => "acceptance_roll_failed",
        }
    }
}

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

    /// Most recent date any club made the player a concrete offer,
    /// retained beyond the pruned 30-day `recent_offer_dates` window so
    /// the pressure model can tell a 90-day offer drought ("no club
    /// wants him") apart from a player who keeps getting and refusing
    /// offers. `None` until the first offer arrives.
    pub last_offer_date: Option<NaiveDate>,

    /// Most recent reason a matcher passed over this player, with the
    /// date it was recorded. Same-day updates keep the highest-ranked
    /// (closest-to-signing) reason; a new day overwrites. Diagnosis
    /// only — no gate reads it.
    pub last_block: Option<(NaiveDate, FreeAgentBlockReason)>,

    /// Consecutive ticks the player cleared every structural gate but
    /// lost the daily approach roll without fielding an offer. Feeds a
    /// pity bonus into the daily signing chance so a structurally
    /// signable player isn't left waiting months on dice alone. Reset
    /// the instant any offer lands. Bounded by the pity helper's cap.
    pub failed_approach_streak: u8,
    /// Day the streak counter was last advanced. A pool player is
    /// evaluated by many countries in one tick, so without this guard
    /// the streak would jump by the number of countries that dice-failed
    /// him in a single day. The guard keeps it to at most one increment
    /// per calendar day and makes the bump order-independent.
    pub last_streak_update: Option<NaiveDate>,
}

impl FreeAgentMarketState {
    /// Number of offers received in the last 30 days. Computed from
    /// `recent_offer_dates`; the helper prunes stale entries on every
    /// `on_offer_received` so the vector stays small.
    pub fn offers_received_30d(&self, today: NaiveDate) -> u8 {
        let cutoff = today - Duration::days(30);
        self.recent_offer_dates
            .iter()
            .filter(|d| **d >= cutoff)
            .count()
            .min(255) as u8
    }

    /// Days since the player last fielded a concrete offer, measured
    /// from `free_since` when no offer has ever landed. This is the "is
    /// anyone actually calling?" signal — it tells a genuine market
    /// drought ("no club wants him") apart from a player who keeps
    /// getting offers and turning them down, which the 30-day window
    /// alone can't because it's pruned.
    pub fn days_since_last_offer(&self, today: NaiveDate) -> i64 {
        let anchor = self.last_offer_date.unwrap_or(self.free_since);
        (today - anchor).num_days().max(0)
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

    /// Short stable label for debug output / UI rendering.
    pub fn label(self) -> &'static str {
        match self {
            MarketStage::Fresh => "Fresh",
            MarketStage::Open => "Open",
            MarketStage::Flexible => "Flexible",
            MarketStage::Desperate => "Desperate",
            MarketStage::LastChance => "Last chance",
        }
    }
}

/// Coarse category of *why* a free agent is still unsigned, derived
/// purely from his own durable market state (no world scan). Stable
/// enough to assert on in tests and to map to a localized string in the
/// web layer; each carries a plain-English default via
/// [`Self::default_message`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FreeAgentStatusCategory {
    /// Data hole — nationality couldn't be resolved.
    DataUnknown,
    /// No club is recruiting his position group anywhere reachable.
    NoPositionNeed,
    /// He keeps fielding offers whose wage sits below his demand.
    WageTooHigh,
    /// His reputation / region expectations are still above the clubs
    /// that would take him — he's holding out for a bigger name.
    ReputationWait,
    /// Clubs have made offers, but none matched his overall terms yet.
    OffersRefused,
    /// Few clubs are looking his way; he may need to drop to a smaller
    /// role / weaker league.
    LowInterest,
    /// He clears the gates and clubs are interested — a deal is close,
    /// the approach roll just hasn't landed yet.
    InterestBuilding,
}

impl FreeAgentStatusCategory {
    /// Map the most recent funnel block reason onto a category. `None`
    /// (no matcher has touched him yet) falls back to `LowInterest`.
    pub fn from_block_reason(reason: Option<FreeAgentBlockReason>) -> Self {
        match reason {
            Some(FreeAgentBlockReason::UnknownNationality) => Self::DataUnknown,
            Some(FreeAgentBlockReason::NoMatchingRequest)
            | Some(FreeAgentBlockReason::PositionMismatch) => Self::NoPositionNeed,
            Some(FreeAgentBlockReason::WageReservationMismatch) => Self::WageTooHigh,
            Some(FreeAgentBlockReason::CountryReputationGap)
            | Some(FreeAgentBlockReason::RegionPrestigeGap)
            | Some(FreeAgentBlockReason::CrossContinentPressureTooLow)
            | Some(FreeAgentBlockReason::AboveMaximumAbility) => Self::ReputationWait,
            Some(FreeAgentBlockReason::AcceptanceRollFailed) => Self::OffersRefused,
            Some(FreeAgentBlockReason::BelowMinimumAbility)
            | Some(FreeAgentBlockReason::ClubAtSquadCapacity)
            | Some(FreeAgentBlockReason::PerDaySigningCapReached) => Self::LowInterest,
            Some(FreeAgentBlockReason::DailyChanceRollFailed) => Self::InterestBuilding,
            Some(FreeAgentBlockReason::AlreadySignedOrStaged) => Self::InterestBuilding,
            None => Self::LowInterest,
        }
    }

    /// Plain-English one-liner. The web layer may swap these for
    /// localized strings keyed off the category; they live here so the
    /// core has a usable default and tests have something to read.
    pub fn default_message(self) -> &'static str {
        match self {
            Self::DataUnknown => "His registration data is incomplete, so clubs can't approach him.",
            Self::NoPositionNeed => "No clubs currently need his position.",
            Self::WageTooHigh => {
                "His wage demands remain above what interested clubs can offer."
            }
            Self::ReputationWait => {
                "He is waiting for a club closer to his previous reputation level."
            }
            Self::OffersRefused => {
                "Several clubs considered him, but no offer matched his expectations."
            }
            Self::LowInterest => "Market interest is low; he may need to accept a smaller role.",
            Self::InterestBuilding => "Clubs are showing interest; a move should come soon.",
        }
    }
}

/// State-only snapshot of a free agent's market situation. Cheap to
/// build (no world scan) so list views — the country free-agents page —
/// can show one per row. The auditor's [`FreeAgentMarketDiagnosis`]
/// remains the richer, world-scanning answer for a single-player view.
#[derive(Debug, Clone)]
pub struct FreeAgentStatusExplanation {
    pub days_free: i64,
    pub market_stage: MarketStage,
    pub offers_received_30d: u8,
    pub offers_rejected_total: u16,
    pub last_block_reason: Option<FreeAgentBlockReason>,
    pub category: FreeAgentStatusCategory,
    pub message: String,
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
            last_offer_date: None,
            last_block: None,
            failed_approach_streak: 0,
            last_streak_update: None,
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
        let inferred_salary =
            WageCalculator::expected_annual_wage(self, self.age(date), club_score, last_league_rep);
        // Seed `free_since` 30 days in the past so a fresh database
        // free agent isn't treated as "released yesterday". They've
        // been on the market — the engine just hasn't been simulating
        // their sit until now.
        let free_since = date - Duration::days(30);
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
            last_offer_date: None,
            last_block: None,
            failed_approach_streak: 0,
            last_streak_update: None,
        });
    }

    /// Record a fresh offer landing on this player. Prunes the rolling
    /// window so `offers_received_30d` stays accurate without a daily
    /// sweep. Also clears the RNG pity streak — once a concrete offer
    /// lands the player is demonstrably not dice-starved. No-op if the
    /// player isn't a free agent.
    pub fn on_offer_received(&mut self, date: NaiveDate) {
        if let Some(state) = self.free_agent_state.as_mut() {
            let cutoff = date - Duration::days(30);
            state.recent_offer_dates.retain(|d| *d >= cutoff);
            state.recent_offer_dates.push(date);
            state.last_offer_date = Some(date);
            state.failed_approach_streak = 0;
            // Claim today's streak slot so a later same-day dice-fail
            // block (from another country) can't re-grow the streak the
            // offer just cleared.
            state.last_streak_update = Some(date);
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

    /// Record why a matcher skipped this player today. Same-day calls
    /// keep the highest-ranked reason (the one closest to a signing);
    /// a later date replaces the stale entry outright. No-op if the
    /// player isn't a free agent.
    ///
    /// Doubles as the RNG-pity bookkeeping: a pure dice-starvation block
    /// (`DailyChanceRollFailed` — the player cleared every structural
    /// gate at some club but lost the approach roll) advances the
    /// failed-approach streak at most once per calendar day. Any day an
    /// offer actually lands resets the streak via `on_offer_received`,
    /// which also claims the day's streak slot so this can't undo it.
    pub fn on_market_blocked(&mut self, date: NaiveDate, reason: FreeAgentBlockReason) {
        if let Some(state) = self.free_agent_state.as_mut() {
            if reason == FreeAgentBlockReason::DailyChanceRollFailed
                && state.last_streak_update != Some(date)
            {
                state.failed_approach_streak = state.failed_approach_streak.saturating_add(1);
                state.last_streak_update = Some(date);
            }
            match state.last_block {
                Some((existing_date, existing))
                    if existing_date == date && existing.rank() >= reason.rank() => {}
                _ => state.last_block = Some((date, reason)),
            }
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

        // Offer drought: a player no club has approached in months is in
        // a different situation from one fielding (and refusing) offers.
        // The 30-day `interest_pressure` above can't see past its window;
        // this adds a step once the silence stretches to 90 / 180 days,
        // so "nobody wants him" players lower their expectations faster —
        // exactly the "no offers → drop demands sooner" behaviour the
        // design model asks for.
        let drought_days = state.days_since_last_offer(today);
        let drought_pressure = if drought_days >= 180 {
            0.15
        } else if drought_days >= 90 {
            0.08
        } else {
            0.0
        };

        // 0.03/month + 0.18/window: a journeyman with no offers sits
        // around 0.55-0.60 by month six and 0.90+ by month twelve, so
        // the rep/region step-down gates open on the timeline the
        // market-clearing design expects (meaningful by 6 months,
        // major step-downs accepted by 12).
        let raw = 0.03 * months_free
            + 0.18 * windows_missed
            + 0.04 * offers_rejected
            + age_pressure
            + quality_pressure
            + interest_pressure
            + drought_pressure;
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

    /// State-only explanation of why the player is still a free agent,
    /// for list views that can't afford the auditor's world scan. `None`
    /// when the player isn't a free agent. The category is driven by the
    /// most recent matcher block reason, refined by the offer history so
    /// the message reads sensibly even before any matcher has recorded a
    /// reason (a player getting and refusing offers reads as
    /// "OffersRefused" even if the stored reason is coarse).
    pub fn market_explanation(&self, today: NaiveDate) -> Option<FreeAgentStatusExplanation> {
        let state = self.free_agent_state.as_ref()?;
        let days_free = (today - state.free_since).num_days().max(0);
        let market_stage = MarketStage::from_days_free(days_free);
        let offers_received_30d = state.offers_received_30d(today);
        let last_block_reason = state.last_block.map(|(_, r)| r);

        // Start from the stored funnel reason, then let strong offer-
        // history signals override a coarse / stale stored reason: a
        // player who has rejected multiple offers is "too picky", not
        // "nobody wants him", regardless of which gate happened to be
        // recorded last.
        let mut category = FreeAgentStatusCategory::from_block_reason(last_block_reason);
        if state.offers_rejected_total >= 2
            && matches!(
                category,
                FreeAgentStatusCategory::LowInterest | FreeAgentStatusCategory::NoPositionNeed
            )
        {
            category = FreeAgentStatusCategory::OffersRefused;
        }

        let message = category.default_message().to_string();
        Some(FreeAgentStatusExplanation {
            days_free,
            market_stage,
            offers_received_30d,
            offers_rejected_total: state.offers_rejected_total,
            last_block_reason,
            category,
            message,
        })
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
        assert!(
            pressure > 0.85,
            "expected near-cap pressure, got {pressure}"
        );
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
    fn on_market_blocked_keeps_highest_rank_same_day_and_resets_next_day() {
        let today = NaiveDate::from_ymd_opt(2026, 6, 13).unwrap();
        let mut p = make_player(80, 29, today);
        p.ensure_free_agent_state(today, 4000);

        // Same day: a funnel-deep reason must not be overwritten by a
        // coarse early-gate one.
        p.on_market_blocked(today, FreeAgentBlockReason::AcceptanceRollFailed);
        p.on_market_blocked(today, FreeAgentBlockReason::CountryReputationGap);
        assert_eq!(
            p.free_agent_state().unwrap().last_block,
            Some((today, FreeAgentBlockReason::AcceptanceRollFailed))
        );

        // New day: stale entry is replaced outright.
        let tomorrow = today + chrono::Duration::days(1);
        p.on_market_blocked(tomorrow, FreeAgentBlockReason::CountryReputationGap);
        assert_eq!(
            p.free_agent_state().unwrap().last_block,
            Some((tomorrow, FreeAgentBlockReason::CountryReputationGap))
        );
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

    /// Put `p` into the free-agent market `released` days before `today`
    /// at the given previous salary / reputation. Free helper matching
    /// the module's existing `make_player` / `person` style.
    fn release_into_market(p: &mut Player, release: NaiveDate, last_salary: u32, rep: u16) {
        p.enter_free_agent_market(ReleaseContext {
            date: release,
            last_club_id: Some(10),
            last_country_id: Some(1),
            last_country_reputation: rep,
            last_league_reputation: rep,
            last_club_reputation_score: (rep as f32 / 10_000.0).clamp(0.0, 1.0),
            last_salary,
            last_squad_status: PlayerSquadStatus::FirstTeamRegular,
        });
    }

    #[test]
    fn fresh_high_rep_pressure_climbs_from_fresh_to_desperate() {
        // The #2 / #3 timeline at the state level: a strong, well-known
        // player just released sits at low pressure (tight gates → he
        // rejects big step-downs); months later the same player's
        // pressure has climbed sharply (wide gates → he accepts a wider
        // role / reputation range).
        let release = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let mut p = make_player(140, 28, release);
        release_into_market(&mut p, release, 2_000_000, 7000);
        let fresh = p.career_pressure(release + chrono::Duration::days(5));
        let desperate = p.career_pressure(release + chrono::Duration::days(200));
        assert!(fresh < 0.25, "fresh big-name pressure too high: {fresh}");
        assert!(
            desperate > fresh + 0.30,
            "pressure must climb materially over months (fresh={fresh}, desperate={desperate})"
        );
    }

    #[test]
    fn repeated_rejections_increase_pressure() {
        // #4: a player who keeps turning offers down accrues pressure
        // (which in turn decays his reservation wage), versus an
        // otherwise identical player who hasn't rejected anything.
        let release = NaiveDate::from_ymd_opt(2026, 4, 1).unwrap();
        let today = release + chrono::Duration::days(60);
        let mut patient = make_player(85, 28, today);
        release_into_market(&mut patient, release, 300_000, 4000);
        let mut picky = make_player(85, 28, today);
        release_into_market(&mut picky, release, 300_000, 4000);
        for _ in 0..4 {
            picky.on_offer_rejected();
        }
        assert!(
            picky.career_pressure(today) > patient.career_pressure(today),
            "repeated rejections must raise pressure (patient={}, picky={})",
            patient.career_pressure(today),
            picky.career_pressure(today)
        );
    }

    #[test]
    fn offer_drought_raises_pressure() {
        // #6: a true offer drought ("no club wants him") adds pressure
        // beyond what a player gets when clubs at least keep calling.
        // Both have 0 offers in the last 30 days; only the drought tier
        // (days since last offer) differs, isolating that signal.
        let release = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let today = release + chrono::Duration::days(200);

        let mut drought = make_player(85, 27, today);
        release_into_market(&mut drought, release, 200_000, 4000);
        // Never any offer → drought ≈ 200 days (≥180 tier).

        let mut some_history = make_player(85, 27, today);
        release_into_market(&mut some_history, release, 200_000, 4000);
        // Last offer 100 days ago → drought ≈ 100 days (≥90 tier), and
        // it's outside the 30-day window so offers_30d is still 0.
        some_history.on_offer_received(release + chrono::Duration::days(100));

        assert!(
            drought.career_pressure(today) > some_history.career_pressure(today),
            "a longer offer drought must mean more pressure (drought={}, some={})",
            drought.career_pressure(today),
            some_history.career_pressure(today)
        );
    }

    #[test]
    fn failed_approach_streak_increments_once_per_day_and_resets_on_offer() {
        // #5: pure dice-starvation grows the streak at most once per day
        // and any landed offer resets it (so it can't re-grow the same
        // day after the reset).
        let today = NaiveDate::from_ymd_opt(2026, 6, 13).unwrap();
        let mut p = make_player(80, 29, today);
        p.ensure_free_agent_state(today, 4000);

        // Two dice-fail blocks the same day → +1 only.
        p.on_market_blocked(today, FreeAgentBlockReason::DailyChanceRollFailed);
        p.on_market_blocked(today, FreeAgentBlockReason::DailyChanceRollFailed);
        assert_eq!(p.free_agent_state().unwrap().failed_approach_streak, 1);

        // A structural (non-dice) block must not advance the streak.
        let d2 = today + chrono::Duration::days(1);
        p.on_market_blocked(d2, FreeAgentBlockReason::RegionPrestigeGap);
        assert_eq!(p.free_agent_state().unwrap().failed_approach_streak, 1);

        // Next-day dice-fail → +1.
        let d3 = today + chrono::Duration::days(2);
        p.on_market_blocked(d3, FreeAgentBlockReason::DailyChanceRollFailed);
        assert_eq!(p.free_agent_state().unwrap().failed_approach_streak, 2);

        // An offer resets it, and a same-day dice-fail can't re-grow it.
        p.on_offer_received(d3);
        assert_eq!(p.free_agent_state().unwrap().failed_approach_streak, 0);
        p.on_market_blocked(d3, FreeAgentBlockReason::DailyChanceRollFailed);
        assert_eq!(p.free_agent_state().unwrap().failed_approach_streak, 0);
    }

    #[test]
    fn market_explanation_reports_offers_refused_when_rejections_high() {
        // #4 / #8 (state path): a player who has refused multiple offers
        // reads as "offers refused", not a coarse "low interest", even
        // when the stored block reason is absent / generic.
        let release = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
        let today = release + chrono::Duration::days(120);
        let mut p = make_player(90, 28, today);
        release_into_market(&mut p, release, 500_000, 5000);
        p.on_offer_rejected();
        p.on_offer_rejected();

        let exp = p
            .market_explanation(today)
            .expect("a pooled player must have an explanation");
        assert_eq!(exp.category, FreeAgentStatusCategory::OffersRefused);
        assert_eq!(exp.market_stage, MarketStage::Flexible);
        assert!(!exp.message.is_empty());
    }

    #[test]
    fn market_explanation_none_when_signed() {
        let today = NaiveDate::from_ymd_opt(2026, 6, 13).unwrap();
        let p = make_player(80, 27, today);
        assert!(
            p.market_explanation(today).is_none(),
            "a signed player has no free-agent explanation"
        );
    }
}
