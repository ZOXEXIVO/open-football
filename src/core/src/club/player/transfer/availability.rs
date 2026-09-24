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
//! The player brings his own clock to that curve: seasons without
//! first-team football ([`FootballDrought`]) resign him to a step down
//! before the market has declined him once.
//!
//! The market-exposure *scoring* lives in
//! [`crate::transfers::scouting::exposure`]; this module owns only the
//! durable state and the reason taxonomy, exactly as `free_agent_market`
//! owns `FreeAgentMarketState` / `FreeAgentBlockReason` while the matcher
//! lives in the country pipeline.

use chrono::{Duration, NaiveDate};

use crate::club::player::StuckCareerScan;
use crate::club::player::mind::CareerArc;
use crate::club::player::player::Player;
use crate::{Person, PlayerSquadStatus, PlayerStatusType, TeamType};

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
/// place in the plans, scans that found no taker — and, before any of
/// that, the seasons he has already gone without football
/// ([`FootballDrought`]), which lower his sights on the day he is
/// listed rather than half a year later. The plausibility
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
    /// circulation scans from [`AvailabilityMarketState`]; `drought` is
    /// [`FootballDrought::score`], the same ramp already served on the
    /// bench, so whichever clock has run further sets where he starts.
    /// Continuous in every axis — no cliffs.
    pub fn compute(
        days_on_market: i64,
        squad_status: Option<&PlayerSquadStatus>,
        failed_scans: u16,
        drought: f32,
    ) -> f32 {
        let unsold = ((days_on_market as f32 - Self::GRACE_DAYS) / Self::RAMP_DAYS).clamp(0.0, 1.0);
        let time = unsold.max(drought.clamp(0.0, 1.0));
        if time <= 0.0 {
            return 0.0;
        }
        let urgency = Self::role_urgency(squad_status);
        let scans = (failed_scans as f32 / 12.0).clamp(0.0, 1.0);
        // Role and dry scans set how FAST he re-reads the market, not how
        // far he will ever go. The ceiling used to be
        // `0.55 + 0.35·urgency + 0.10·scans`, which for the roles that
        // matter — a listed key man at 0.25 urgency, a first-team regular
        // at 0.35 — topped out around 0.75 no matter how long he went
        // unsold. Everything downstream reads resignation as a 0..1 ramp,
        // so that quietly capped how far a listed player's sights could
        // ever fall: the deepest step-downs stayed shut against a player
        // who had been on the market for two years. A man nobody has bid
        // for in that long has genuinely run out of alternatives; the
        // slower roles simply take the full ramp to get there.
        let pace = (0.55 + 0.35 * urgency + 0.10 * scans).clamp(0.0, 1.0);
        let eased = time.powf(1.0 / pace.max(0.2));
        eased.clamp(0.0, 1.0)
    }
}

/// Seasons without first-team football at this club, as the player
/// himself counts them: the completed ones off the ledger
/// ([`StuckCareerScan`], read through the squad he is registered with,
/// so a B side's minutes do not count), and the one in progress off his
/// live start share. What resigns a man to a step down before anyone
/// has listed him — he has to play somewhere to be worth anything, to
/// the club and to himself.
#[derive(Debug, Clone, Copy)]
pub struct FootballDrought {
    pub stuck_years: u16,
    /// Registered with the first team. Below it the live share is a B
    /// side's football and says nothing about being picked.
    pub in_first_team: bool,
    pub starter_ratio: f32,
    pub appearances_tracked: u8,
    pub age: u8,
}

impl FootballDrought {
    /// Full seasons without football that leave him ready for any club
    /// that will play him.
    const SEASONS_TO_RESIGN: f32 = 2.0;
    /// Start share below which he is not being picked — the line the
    /// transfer-desire pass draws for "breaking through".
    const BENCHED_SHARE: f32 = 0.40;
    /// Matches the start-share EMA needs before it is trusted at all.
    const MIN_TRACKED_APPS: u8 = 6;
    /// Tracked matches that make the season in progress a full one.
    const SEASON_APPS: f32 = 30.0;
    /// The age from which a season out of the first team starts to
    /// count, and the age at which it counts in full — the years a boy
    /// is still meant to be in an age-group side.
    const YOUTH_FROM: f32 = 17.0;
    const YOUTH_TO: f32 = 20.0;

    pub fn read(player: &Player, today: NaiveDate) -> Self {
        let squad_tier = player
            .squad_standing_view
            .map_or(TeamType::Main, |view| view.squad_tier);
        FootballDrought {
            stuck_years: StuckCareerScan::of_in_squad(player, today, squad_tier)
                .map_or(0, |scan| scan.stuck_years),
            in_first_team: matches!(squad_tier, TeamType::Main),
            starter_ratio: player.happiness.starter_ratio,
            appearances_tracked: player.happiness.appearances_tracked,
            age: player.age(today),
        }
    }

    /// 0..1. The season in progress is weighted by how much of it he has
    /// sat through, so six quiet weeks at a new club say nothing; the
    /// completed seasons count only while he is still not being picked —
    /// a man who has claimed the shirt this year has nothing left to be
    /// resigned about.
    pub fn score(&self) -> f32 {
        if self.appearances_tracked < Self::MIN_TRACKED_APPS {
            return 0.0;
        }
        let sat_through = (self.appearances_tracked as f32 / Self::SEASON_APPS).clamp(0.0, 1.0);
        let not_picked = if self.in_first_team {
            (1.0 - self.starter_ratio / Self::BENCHED_SHARE).clamp(0.0, 1.0)
        } else {
            1.0
        };
        let grown = ((self.age as f32 - Self::YOUTH_FROM) / (Self::YOUTH_TO - Self::YOUTH_FROM))
            .clamp(0.0, 1.0);
        let seasons = not_picked * sat_through * (self.stuck_years as f32 + 1.0);
        (grown * seasons / Self::SEASONS_TO_RESIGN).clamp(0.0, 1.0)
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
    /// while he is genuinely on the permanent market — listed (`Lst`) or
    /// formally requesting out (`Req`) — or while his own football has
    /// dried up ([`FootballDrought`]); a merely unhappy or loan-listed
    /// player who is still being picked keeps his full expectations for
    /// a permanent move. The days anchor is his earliest active
    /// availability status, so a long-unsettled player who is then
    /// listed resigns from where his sit actually began, not from the
    /// paperwork date.
    pub fn market_resignation(&self, today: NaiveDate) -> f32 {
        let listed = self.statuses.has(PlayerStatusType::Lst);
        let requested = self.statuses.has(PlayerStatusType::Req);
        let drought = self.football_drought(today);
        // A man who has DECIDED to drop a level, or to go home and
        // finish there, has already lowered his sights — that is what
        // the arc is. Without this a `Loa`-only player accrued no
        // resignation at all, so the one population whose whole plan is
        // a step down was the one the step-down model could not see.
        let plan_resignation = match self.mind.career.plan.map(|plan| plan.arc) {
            Some(CareerArc::StepDownToPlay) | Some(CareerArc::FinishAtHome) => {
                self.mind.career.plan.map(|p| p.strength).unwrap_or(0.0)
            }
            _ => 0.0,
        };
        if !listed && !requested && drought <= 0.0 && plan_resignation <= 0.0 {
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
            drought.max(plan_resignation),
        )
    }

    /// [`FootballDrought::score`] for this player today.
    pub fn football_drought(&self, today: NaiveDate) -> f32 {
        FootballDrought::read(self, today).score()
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
        PlayerSkills, PlayerStatCompetitionKind, PlayerStatLedgerEntry, PlayerStatistics,
    };
    use chrono::Datelike;

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
            Self::player_aged(today, 26)
        }

        fn player_aged(today: NaiveDate, age: i32) -> Player {
            let attrs = PlayerAttributes {
                current_ability: 130,
                potential_ability: 140,
                ..Default::default()
            };
            let birth = NaiveDate::from_ymd_opt(today.year() - age, 1, 1).unwrap();
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

        fn league_season(year: u16, starts: u16) -> PlayerStatLedgerEntry {
            PlayerStatLedgerEntry {
                seq_id: 0,
                season_start_year: year,
                team_slug: "t".into(),
                team_name: "T".into(),
                team_reputation: 6_000,
                league_slug: "l".into(),
                league_name: "L".into(),
                competition_kind: PlayerStatCompetitionKind::League,
                competition_slug: "l".into(),
                is_loan: false,
                transfer_fee: None,
                coverage_days: None,
                spell_end: None,
                statistics: PlayerStatistics {
                    played: starts,
                    ..Default::default()
                },
            }
        }

        /// A homegrown man who has sat through this season, with `stuck`
        /// completed seasons of the same behind him and a full one before
        /// those.
        fn benched(today: NaiveDate, age: i32, stuck: u16) -> Player {
            let mut p = Self::player_aged(today, age);
            p.happiness.starter_ratio = 0.0;
            p.happiness.appearances_tracked = 40;
            let last = today.year() as u16 - 1;
            for year in (last + 1 - stuck)..=last {
                p.statistics_history
                    .season_ledger
                    .push(Self::league_season(year, 1));
            }
            p.statistics_history
                .season_ledger
                .push(Self::league_season(last - stuck, 25));
            p
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
        assert!(
            mid > 0.0 && mid < 1.0,
            "mid-sit resignation in (0,1): {mid}"
        );
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
        let unwanted = MarketResignation::compute(200, Some(&PlayerSquadStatus::NotNeeded), 0, 0.0);
        let key_man = MarketResignation::compute(200, Some(&PlayerSquadStatus::KeyPlayer), 0, 0.0);
        assert!(unwanted > key_man);
        let scanned = MarketResignation::compute(200, Some(&PlayerSquadStatus::NotNeeded), 12, 0.0);
        assert!(scanned > unwanted);
        // Fully saturated case stays bounded.
        assert!(
            MarketResignation::compute(2000, Some(&PlayerSquadStatus::NotNeeded), 30, 0.0) <= 1.0
        );
    }

    /// The force behind a sale: a man who has not played does not need a
    /// listing, or months unsold, before he will drop a level to play.
    #[test]
    fn seasons_without_football_lower_his_sights_before_any_listing() {
        let today = AvailabilityFixtures::d(2026, 6, 15);
        let stuck = AvailabilityFixtures::benched(today, 26, 2);
        assert!(!stuck.is_market_available());
        let resigned = stuck.market_resignation(today);
        assert!(resigned >= 0.99, "two seasons on the bench: {resigned}");

        // The same seasons behind a man now starting every week resign
        // him to nothing: he has claimed the shirt.
        let mut claimed = AvailabilityFixtures::benched(today, 26, 2);
        claimed.happiness.starter_ratio = 0.6;
        assert_eq!(claimed.market_resignation(today), 0.0);

        // His first season on the bench takes him half the way.
        let first = AvailabilityFixtures::benched(today, 26, 0);
        let mid = first.market_resignation(today);
        assert!(mid > 0.3 && mid < 0.6, "{mid}");
    }

    /// A boy in an age-group side is where he belongs; the same season
    /// starts to count as he outgrows it.
    #[test]
    fn a_youth_season_counts_only_once_he_has_outgrown_the_age_group() {
        let today = AvailabilityFixtures::d(2026, 6, 15);
        let boy = AvailabilityFixtures::benched(today, 17, 0);
        assert_eq!(boy.market_resignation(today), 0.0);
        let older = AvailabilityFixtures::benched(today, 19, 0);
        let grown = AvailabilityFixtures::benched(today, 21, 0);
        assert!(older.market_resignation(today) > 0.0);
        assert!(grown.market_resignation(today) > older.market_resignation(today));
    }

    #[test]
    fn drought_needs_a_season_sat_through_by_a_man_not_being_picked() {
        let base = FootballDrought {
            stuck_years: 0,
            in_first_team: true,
            starter_ratio: 0.0,
            appearances_tracked: 40,
            age: 25,
        };
        assert_eq!(
            FootballDrought {
                appearances_tracked: 3,
                ..base
            }
            .score(),
            0.0,
            "six quiet weeks say nothing"
        );
        assert!(
            FootballDrought {
                appearances_tracked: 12,
                ..base
            }
            .score()
                < base.score()
        );
        assert!((base.score() - 0.5).abs() < 1e-6, "{}", base.score());
        assert_eq!(
            FootballDrought {
                stuck_years: 1,
                ..base
            }
            .score(),
            1.0
        );
        assert_eq!(
            FootballDrought {
                starter_ratio: 0.4,
                ..base
            }
            .score(),
            0.0,
            "a man being picked has no drought"
        );
        assert_eq!(
            FootballDrought {
                starter_ratio: 1.0,
                in_first_team: false,
                ..base
            }
            .score(),
            0.5,
            "a B side's starts are not first-team football"
        );
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
