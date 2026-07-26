//! Market-discovery state for a *signed* player who is available to leave
//! his club — transfer-listed (`Lst`), has handed in a request (`Req`),
//! is unhappy (`Unh`), or is loan-listed (`Loa`).
//!
//! This is the signed-side mirror of [`super::free_agent_market`]. The
//! free-agent model answers "why is this clubless player still
//! unsigned?"; this one answers "why has the market not bitten on this
//! available, contracted player?". Both carry a durable per-player
//! block-reason so the long-sit diagnosis survives across ticks, and
//! both feed a softening curve (the seller drops the asking price, the
//! player relaxes his wage demand) as the failed weeks accumulate.
//!
//! The market-exposure *scoring* lives in
//! [`crate::transfers::pipeline::exposure`]; this module owns only the
//! durable state and the reason taxonomy, exactly as `free_agent_market`
//! owns `FreeAgentMarketState` / `FreeAgentBlockReason` while the matcher
//! lives in the country pipeline.

use chrono::{Duration, NaiveDate};

use crate::club::player::player::Player;
use crate::{PlayerSquadStatus, PlayerStatusType};

/// Statuses that advertise a signed player as available to the market.
/// A player carrying any of these is "on the market" for the purposes of
/// the exposure / circulation layer. Synthetic / internal statuses are
/// deliberately excluded — only the player's own public availability
/// flags count, the same principle the plausibility exemption uses.
pub const AVAILABILITY_STATUSES: [PlayerStatusType; 4] = [
    PlayerStatusType::Lst,
    PlayerStatusType::Req,
    PlayerStatusType::Unh,
    PlayerStatusType::Loa,
];

/// Why the market has produced no interest in an available, contracted
/// player. One value per player, refreshed every time the circulation
/// pass scans the plausible buyer field and finds no taker — the
/// diagnosis layer reads it to answer "why is this quality available
/// player still here?". Ordered by `rank`: a richer, closer-to-a-deal
/// blocker outranks a coarse early-gate one when two are recorded on the
/// same scan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AvailabilityBlockReason {
    /// Not enough time on the market yet to draw a conclusion — the
    /// player only just became available. A non-diagnosis sentinel.
    TooEarly,
    /// No club in the player's own country sits in a tier window that
    /// could plausibly want him at all — the shallowest funnel stage.
    NoPlausibleBuyer,
    /// Every plausible buyer is closed off by a country / region
    /// realism block (route policy, prestige region gap).
    CountryRegionBlocked,
    /// The player's reputation sits above the tier window of every club
    /// that has a need — too prestigious for the clubs that would take
    /// him, not prestigious enough to interest the clubs that wouldn't
    /// need him.
    ReputationTooHigh,
    /// Plausible buyers exist but none has a squad need this player would
    /// improve — the agent must circulate wider / wait for an opening.
    NoAffordableSquadNeed,
    /// A club would take him but the asking price (value × seller
    /// multiplier) is beyond what it can fund — the seller should
    /// discount.
    AskingPriceTooHigh,
    /// A club would take him but his wage demand exceeds the wage
    /// headroom of every interested buyer — the player should soften.
    WageTooHigh,
    /// The only interested buyers are a clear sporting step down that
    /// the player (prime-age, important) is unwilling to accept yet.
    PlayerWontStepDown,
}

impl AvailabilityBlockReason {
    /// How far through the discovery funnel the player got before the
    /// market stalled. Higher = closer to an actual deal = more
    /// actionable. Used as the same-scan tiebreak and the merge rule.
    pub fn rank(self) -> u8 {
        match self {
            AvailabilityBlockReason::TooEarly => 0,
            AvailabilityBlockReason::NoPlausibleBuyer => 1,
            AvailabilityBlockReason::CountryRegionBlocked => 2,
            AvailabilityBlockReason::ReputationTooHigh => 3,
            AvailabilityBlockReason::NoAffordableSquadNeed => 4,
            AvailabilityBlockReason::AskingPriceTooHigh => 5,
            AvailabilityBlockReason::WageTooHigh => 6,
            AvailabilityBlockReason::PlayerWontStepDown => 7,
        }
    }

    /// Stable label for debug output / diagnosis dumps.
    pub fn label(self) -> &'static str {
        match self {
            AvailabilityBlockReason::TooEarly => "too_early",
            AvailabilityBlockReason::NoPlausibleBuyer => "no_plausible_buyer",
            AvailabilityBlockReason::CountryRegionBlocked => "country_region_blocked",
            AvailabilityBlockReason::ReputationTooHigh => "reputation_too_high",
            AvailabilityBlockReason::NoAffordableSquadNeed => "no_affordable_squad_need",
            AvailabilityBlockReason::AskingPriceTooHigh => "asking_price_too_high",
            AvailabilityBlockReason::WageTooHigh => "wage_too_high",
            AvailabilityBlockReason::PlayerWontStepDown => "player_wont_step_down",
        }
    }

    /// True for reasons the seller can act on by lowering the asking
    /// price. Drives the price-softening arm of the exposure curve.
    pub fn seller_should_discount(self) -> bool {
        matches!(
            self,
            AvailabilityBlockReason::AskingPriceTooHigh
                | AvailabilityBlockReason::NoAffordableSquadNeed
        )
    }

    /// True for reasons the player can act on by relaxing wage / level
    /// demands. Drives the wage-softening arm of the exposure curve.
    pub fn player_should_soften(self) -> bool {
        matches!(
            self,
            AvailabilityBlockReason::WageTooHigh | AvailabilityBlockReason::PlayerWontStepDown
        )
    }
}

/// Continuous "market resignation" of a contracted player stuck on the
/// permanent-transfer market — the signed-side mirror of the free-agent
/// `career_pressure` curve. 0.0 = fresh listing, full expectations;
/// 1.0 = fully resigned, he will consider any club that offers regular
/// football. It is the player's own reading of the situation: weeks of
/// silence after his club put him up for sale (or he asked to go), no
/// place in the plans, scans that found no taker. The plausibility
/// level gates, the personal-terms resistance, and the scouting realism
/// band all consume it, so a benched player at a big club *gradually*
/// lowers his sights toward clubs where he would actually play instead
/// of sitting listed for a year and walking on a free.
///
/// Pure policy struct (no free functions): [`Player::market_resignation`]
/// is the live read, [`MarketResignation::compute`] the testable core.
pub struct MarketResignation;

impl MarketResignation {
    /// Days on the market before resignation starts building — aligned
    /// with the transfer-broadcast grace, i.e. the moment the player
    /// formally asks the club to arrange a move (the
    /// `AskedClubToArrangeTransfer` beat). Before that he still believes
    /// a peer club will come.
    pub const GRACE_DAYS: f32 = 21.0;
    /// Days (past the grace) to reach full resignation — the same clock
    /// the seller's fee-floor erosion runs on, so the player's sights and
    /// the club's price drop together.
    pub const RAMP_DAYS: f32 = 180.0;

    /// How urgently the player's declared role at his club pushes him to
    /// re-read the market. A player the club openly does not need resigns
    /// fastest; a listed key man clings to his level far longer.
    fn role_urgency(status: Option<&PlayerSquadStatus>) -> f32 {
        match status {
            Some(PlayerSquadStatus::NotNeeded) => 1.0,
            Some(PlayerSquadStatus::MainBackupPlayer) => 0.85,
            Some(PlayerSquadStatus::DecentYoungster) => 0.75,
            Some(PlayerSquadStatus::HotProspectForTheFuture) => 0.65,
            Some(PlayerSquadStatus::FirstTeamSquadRotation) => 0.55,
            Some(PlayerSquadStatus::FirstTeamRegular) => 0.35,
            Some(PlayerSquadStatus::KeyPlayer) => 0.25,
            _ => 0.70,
        }
    }

    /// Pure resignation curve. `days_on_market` is time under an active
    /// availability status; `failed_scans` is the consecutive dry
    /// circulation scans from [`AvailabilityMarketState`]. Continuous in
    /// every axis — no cliffs.
    pub fn compute(
        days_on_market: i64,
        squad_status: Option<&PlayerSquadStatus>,
        failed_scans: u16,
    ) -> f32 {
        let time =
            ((days_on_market as f32 - Self::GRACE_DAYS) / Self::RAMP_DAYS).clamp(0.0, 1.0);
        if time <= 0.0 {
            return 0.0;
        }
        let urgency = Self::role_urgency(squad_status);
        let scans = (failed_scans as f32 / 12.0).clamp(0.0, 1.0);
        (time * (0.55 + 0.35 * urgency + 0.10 * scans)).clamp(0.0, 1.0)
    }
}

/// Durable record of how the market has treated a signed, available
/// player. Seeded the first time the circulation pass sees the player
/// carrying an availability status; updated each weekly scan; dropped
/// when he is no longer available (status cleared, or he changes club).
#[derive(Debug, Clone)]
pub struct AvailabilityMarketState {
    /// When the player first became available in the current sit. Anchors
    /// the staleness curve. Derived from the earliest active availability
    /// status the first time the state is seeded.
    pub since: NaiveDate,
    /// Bounded log of dates a plausible buyer showed concrete interest
    /// (monitoring, shortlist, recommendation, or live negotiation).
    /// Used to compute the rolling 30-day interest count without a
    /// separate stale counter — same shape as the free-agent model's
    /// `recent_offer_dates`.
    pub recent_interest_dates: Vec<NaiveDate>,
    /// Consecutive circulation scans that found no interest. Resets to 0
    /// the moment any interest is recorded. Feeds the softening curve so
    /// a player nobody has touched in months relaxes faster than one
    /// being actively, if slowly, pursued.
    pub failed_scans: u16,
    /// Most recent diagnosis of why the market stalled, with the date it
    /// was recorded. Diagnosis only — no gate reads it; the exposure /
    /// softening layer and the UI do.
    pub last_block: Option<(NaiveDate, AvailabilityBlockReason)>,
}

impl AvailabilityMarketState {
    /// Concrete approaches in the last 30 days. Computed from
    /// `recent_interest_dates`; the helper prunes stale entries on every
    /// `on_availability_interest` so the vector stays small.
    pub fn recent_interest(&self, today: NaiveDate) -> u8 {
        let cutoff = today - Duration::days(30);
        self.recent_interest_dates
            .iter()
            .filter(|d| **d >= cutoff)
            .count()
            .min(255) as u8
    }

    /// Days the player has sat on the market in the current sit.
    pub fn days_on_market(&self, today: NaiveDate) -> i64 {
        (today - self.since).num_days().max(0)
    }
}

impl Player {
    /// Read-only access to the availability-market state. `None` when the
    /// player is not currently advertised as available (or the
    /// circulation pass hasn't seeded it yet).
    pub fn availability_market_state(&self) -> Option<&AvailabilityMarketState> {
        self.availability_market.as_ref()
    }

    /// True if the player currently carries any market-availability
    /// status. The single predicate the circulation lifecycle keys on.
    pub fn is_market_available(&self) -> bool {
        AVAILABILITY_STATUSES.iter().any(|s| self.statuses.has(*s))
    }

    /// Days since the player first became available (the earliest active
    /// `Lst`/`Req`/`Unh`/`Loa` status), or 0 when not currently available.
    /// Derived straight from the status records, so it is correct on the
    /// very first tick before the durable state has been seeded.
    pub fn days_available(&self, today: NaiveDate) -> i64 {
        if !self.is_market_available() {
            return 0;
        }
        (today - self.earliest_availability_date(today))
            .num_days()
            .max(0)
    }

    /// Earliest start date among the player's active availability
    /// statuses, falling back to `fallback` when none is present (which
    /// only happens when a caller seeds the state defensively).
    fn earliest_availability_date(&self, fallback: NaiveDate) -> NaiveDate {
        self.statuses
            .statuses
            .iter()
            .filter(|s| AVAILABILITY_STATUSES.contains(&s.status))
            .map(|s| s.start_date)
            .min()
            .unwrap_or(fallback)
    }

    /// Lazily seed the market state for an available player. Idempotent —
    /// the state is only created when missing, so the `since` anchor (and
    /// the pressure built on top of it) is never reset by a repeat call.
    pub fn ensure_availability_state(&mut self, date: NaiveDate) {
        if self.availability_market.is_some() {
            return;
        }
        let since = self.earliest_availability_date(date);
        self.availability_market = Some(AvailabilityMarketState {
            since,
            recent_interest_dates: Vec::new(),
            failed_scans: 0,
            last_block: None,
        });
    }

    /// Drop the market state — the player is no longer available (status
    /// cleared) or has left the club. Mirrors `clear_free_agent_state`.
    pub fn clear_availability_state(&mut self) {
        self.availability_market = None;
    }

    /// Record that a plausible buyer showed concrete interest today.
    /// Prunes the rolling 30-day window, resets the failed-scan streak,
    /// and clears any stale "no interest" diagnosis — the market is
    /// moving again. Seeds the state if missing.
    pub fn on_availability_interest(&mut self, date: NaiveDate) {
        self.ensure_availability_state(date);
        if let Some(state) = self.availability_market.as_mut() {
            let cutoff = date - Duration::days(30);
            state.recent_interest_dates.retain(|d| *d >= cutoff);
            state.recent_interest_dates.push(date);
            state.failed_scans = 0;
            state.last_block = None;
        }
    }

    /// Live read of the player's [`MarketResignation`] curve. Non-zero
    /// only while he is genuinely on the permanent market — listed (`Lst`)
    /// or formally requesting out (`Req`); a merely unhappy or loan-listed
    /// player keeps his full expectations for a permanent move. The days
    /// anchor is his earliest active availability status, so a long-
    /// unsettled player who is then listed resigns from where his sit
    /// actually began, not from the paperwork date.
    pub fn market_resignation(&self, today: NaiveDate) -> f32 {
        let listed = self.statuses.has(PlayerStatusType::Lst);
        let requested = self.statuses.has(PlayerStatusType::Req);
        if !listed && !requested {
            return 0.0;
        }
        let failed_scans = self
            .availability_market_state()
            .map(|s| s.failed_scans)
            .unwrap_or(0);
        MarketResignation::compute(
            self.days_available(today),
            self.contract.as_ref().map(|c| &c.squad_status),
            failed_scans,
        )
    }

    /// Record that a circulation scan found no plausible taker, stamping
    /// the dominant block reason. Bumps the failed-scan streak so the
    /// softening curve opens up over repeated dry weeks. Seeds the state
    /// if missing.
    pub fn on_availability_blocked(&mut self, date: NaiveDate, reason: AvailabilityBlockReason) {
        self.ensure_availability_state(date);
        if let Some(state) = self.availability_market.as_mut() {
            state.failed_scans = state.failed_scans.saturating_add(1);
            state.last_block = Some((date, reason));
        }
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

    /// Fixtures for the availability-market state tests. Wrapped in a unit
    /// struct per project convention (no free functions).
    struct AvailabilityFixtures;

    impl AvailabilityFixtures {
        fn d(y: i32, m: u32, day: u32) -> NaiveDate {
            NaiveDate::from_ymd_opt(y, m, day).unwrap()
        }

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

        fn player(today: NaiveDate) -> Player {
            let mut attrs = PlayerAttributes::default();
            attrs.current_ability = 130;
            attrs.potential_ability = 140;
            let birth = today.checked_sub_signed(Duration::days(26 * 365)).unwrap();
            PlayerBuilder::new()
                .id(1)
                .full_name(FullName::new("Test".to_string(), "Player".to_string()))
                .birth_date(birth)
                .country_id(1)
                .attributes(Self::person())
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
    }

    #[test]
    fn ensure_state_anchors_since_to_earliest_status() {
        let today = AvailabilityFixtures::d(2026, 6, 15);
        let mut p = AvailabilityFixtures::player(today);
        // Listed 40 days ago, requested 10 days ago — `since` must anchor
        // to the earlier of the two.
        p.statuses
            .add(today - Duration::days(40), PlayerStatusType::Lst);
        p.statuses
            .add(today - Duration::days(10), PlayerStatusType::Req);
        p.ensure_availability_state(today);
        let state = p.availability_market_state().unwrap();
        assert_eq!(state.since, today - Duration::days(40));
        assert_eq!(state.days_on_market(today), 40);
    }

    #[test]
    fn ensure_state_is_idempotent() {
        let today = AvailabilityFixtures::d(2026, 6, 15);
        let mut p = AvailabilityFixtures::player(today);
        p.statuses
            .add(today - Duration::days(40), PlayerStatusType::Lst);
        p.ensure_availability_state(today);
        let first = p.availability_market_state().unwrap().since;
        // Even if a new status is added later, the original anchor sticks.
        p.statuses.add(today, PlayerStatusType::Req);
        p.ensure_availability_state(today);
        let second = p.availability_market_state().unwrap().since;
        assert_eq!(first, second, "since must not be reset by a repeat call");
    }

    #[test]
    fn interest_resets_failed_streak_and_clears_block() {
        let today = AvailabilityFixtures::d(2026, 6, 15);
        let mut p = AvailabilityFixtures::player(today);
        p.statuses
            .add(today - Duration::days(60), PlayerStatusType::Req);
        // Two dry scans build a failed streak and a recorded block.
        p.on_availability_blocked(
            today - Duration::days(14),
            AvailabilityBlockReason::WageTooHigh,
        );
        p.on_availability_blocked(
            today - Duration::days(7),
            AvailabilityBlockReason::WageTooHigh,
        );
        assert_eq!(p.availability_market_state().unwrap().failed_scans, 2);
        assert!(p.availability_market_state().unwrap().last_block.is_some());
        // A club finally shows interest — the streak resets, diagnosis clears.
        p.on_availability_interest(today);
        let state = p.availability_market_state().unwrap();
        assert_eq!(state.failed_scans, 0);
        assert!(state.last_block.is_none());
        assert_eq!(state.recent_interest(today), 1);
    }

    #[test]
    fn blocked_records_reason_and_bumps_streak() {
        let today = AvailabilityFixtures::d(2026, 6, 15);
        let mut p = AvailabilityFixtures::player(today);
        p.statuses.add(today, PlayerStatusType::Lst);
        p.on_availability_blocked(today, AvailabilityBlockReason::AskingPriceTooHigh);
        let state = p.availability_market_state().unwrap();
        assert_eq!(state.failed_scans, 1);
        assert_eq!(
            state.last_block.map(|(_, r)| r),
            Some(AvailabilityBlockReason::AskingPriceTooHigh)
        );
    }

    #[test]
    fn is_market_available_tracks_statuses() {
        let today = AvailabilityFixtures::d(2026, 6, 15);
        let mut p = AvailabilityFixtures::player(today);
        assert!(!p.is_market_available());
        p.statuses.add(today, PlayerStatusType::Unh);
        assert!(p.is_market_available());
        p.statuses.remove(PlayerStatusType::Unh);
        assert!(!p.is_market_available());
    }

    #[test]
    fn resignation_zero_without_market_status_and_within_grace() {
        let today = AvailabilityFixtures::d(2026, 6, 15);
        let mut p = AvailabilityFixtures::player(today);
        // No availability status at all → no resignation.
        assert_eq!(p.market_resignation(today), 0.0);
        // Unhappy alone is not a permanent-market signal.
        p.statuses
            .add(today - Duration::days(200), PlayerStatusType::Unh);
        assert_eq!(p.market_resignation(today), 0.0);
        // Freshly listed (inside the grace) → still zero.
        p.statuses.remove(PlayerStatusType::Unh);
        p.statuses
            .add(today - Duration::days(10), PlayerStatusType::Lst);
        assert_eq!(p.market_resignation(today), 0.0);
    }

    #[test]
    fn resignation_builds_continuously_with_time_listed() {
        let today = AvailabilityFixtures::d(2026, 6, 15);
        let mut p = AvailabilityFixtures::player(today);
        p.statuses
            .add(today - Duration::days(80), PlayerStatusType::Lst);
        let mid = p.market_resignation(today);
        assert!(mid > 0.0 && mid < 1.0, "mid-sit resignation in (0,1): {mid}");
        // Same player, much longer sit → strictly more resigned.
        let mut long = AvailabilityFixtures::player(today);
        long.statuses
            .add(today - Duration::days(250), PlayerStatusType::Lst);
        assert!(long.market_resignation(today) > mid);
    }

    #[test]
    fn resignation_scales_with_role_urgency_and_dry_scans() {
        // Pure-core check: an unwanted player resigns faster than a listed
        // key man on the same clock, and dry scans accelerate both.
        let unwanted =
            MarketResignation::compute(200, Some(&PlayerSquadStatus::NotNeeded), 0);
        let key_man =
            MarketResignation::compute(200, Some(&PlayerSquadStatus::KeyPlayer), 0);
        assert!(unwanted > key_man);
        let scanned =
            MarketResignation::compute(200, Some(&PlayerSquadStatus::NotNeeded), 12);
        assert!(scanned > unwanted);
        // Fully saturated case stays bounded.
        assert!(MarketResignation::compute(2000, Some(&PlayerSquadStatus::NotNeeded), 30) <= 1.0);
    }

    #[test]
    fn block_reason_rank_orders_by_funnel_depth() {
        assert!(
            AvailabilityBlockReason::WageTooHigh.rank()
                > AvailabilityBlockReason::NoPlausibleBuyer.rank()
        );
        assert!(
            AvailabilityBlockReason::AskingPriceTooHigh.rank()
                > AvailabilityBlockReason::ReputationTooHigh.rank()
        );
    }
}
