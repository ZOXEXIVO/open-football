use crate::shared::{Currency, CurrencyValue};
use crate::{Club, Country, Player, PlayerStatusType, PlayerValueCalculator};
use chrono::{Datelike, NaiveDate};
use std::collections::HashMap;

/// How wide the buyer/seller "talks are allowed" window is around the
/// formal registration window. Real markets quietly start negotiating
/// roughly a fortnight before a window opens and finalise terms a few
/// days after a clear deadline closes; the registration record is filed
/// only within the formal window. Returned by
/// [`TransferWindowManager::current_agreement_window_dates`].
const AGREEMENT_PRE_OPEN_DAYS: i64 = 14;
const AGREEMENT_POST_CLOSE_DAYS: i64 = 3;

#[derive(Debug, Clone)]
pub struct TransferWindowManager {
    pub windows: HashMap<u32, TransferWindow>, // Keyed by country_id
}

#[derive(Debug, Clone)]
pub struct TransferWindow {
    pub summer_window: (NaiveDate, NaiveDate),
    pub winter_window: (NaiveDate, NaiveDate),
    pub country_id: u32,
}

impl TransferWindowManager {
    pub fn new() -> Self {
        TransferWindowManager {
            windows: HashMap::new(),
        }
    }

    /// Construct a manager pre-seeded with the right windows for the given
    /// country. Falls back to default European windows for codes the
    /// calendar table doesn't recognise, so unknown countries behave
    /// exactly as before. Cheap to call; no global state.
    pub fn for_country(country: &Country, date: NaiveDate) -> Self {
        let mut mgr = Self::new();
        let window = TransferCalendar::for_country(&country.code, date);
        mgr.add_window(country.id, window.into_window(country.id));
        mgr
    }

    pub fn add_window(&mut self, country_id: u32, window: TransferWindow) {
        self.windows.insert(country_id, window);
    }

    pub fn is_window_open(&self, country_id: u32, date: NaiveDate) -> bool {
        self.current_window_dates(country_id, date).is_some()
    }

    /// Returns the (start, end) dates of the currently open transfer window,
    /// or None if no window is currently open for this country.
    pub fn current_window_dates(
        &self,
        country_id: u32,
        date: NaiveDate,
    ) -> Option<(NaiveDate, NaiveDate)> {
        let (summer, winter) = if let Some(window) = self.windows.get(&country_id) {
            (window.summer_window, window.winter_window)
        } else {
            Self::default_european_windows(date)
        };

        if self.is_date_in_window(date, &summer) {
            Some(summer)
        } else if self.is_date_in_window(date, &winter) {
            Some(winter)
        } else {
            None
        }
    }

    /// True when a club is allowed to *agree* terms with another club
    /// for this country (in the open registration window OR within the
    /// pre/post tolerance band). Real markets quietly negotiate just
    /// outside formal windows; registration still has to land inside
    /// `current_window_dates`. Drives the deferred-registration path:
    /// clubs can agree now and register later when the window opens.
    pub fn is_agreement_window_open(&self, country_id: u32, date: NaiveDate) -> bool {
        self.current_agreement_window_dates(country_id, date).is_some()
    }

    /// Returns the (start, end) bounds of the agreement window — the
    /// formal registration window widened by a few days on either side.
    /// `None` outside that wider band. Used by negotiation entry to allow
    /// pre-window talks; registration uses [`current_window_dates`].
    pub fn current_agreement_window_dates(
        &self,
        country_id: u32,
        date: NaiveDate,
    ) -> Option<(NaiveDate, NaiveDate)> {
        let (summer, winter) = if let Some(window) = self.windows.get(&country_id) {
            (window.summer_window, window.winter_window)
        } else {
            Self::default_european_windows(date)
        };
        let summer_agreement = AgreementBand::expand(summer);
        let winter_agreement = AgreementBand::expand(winter);
        if self.is_date_in_window(date, &summer_agreement) {
            Some(summer_agreement)
        } else if self.is_date_in_window(date, &winter_agreement) {
            Some(winter_agreement)
        } else {
            None
        }
    }

    /// Date of the next time a deal *can* be formally registered for the
    /// given country. Returns `Some` when:
    ///   - we're already inside an open registration window (today), or
    ///   - the next summer/winter window opens within the next ~12 months.
    /// `None` only when no window definition resolves (should not happen
    /// for known countries — European defaults always apply).
    pub fn next_registration_open_date(
        &self,
        country_id: u32,
        date: NaiveDate,
    ) -> Option<NaiveDate> {
        // If we're inside a window today, registration is open today.
        if let Some((open, _close)) = self.current_window_dates(country_id, date) {
            if date >= open {
                return Some(date);
            }
            return Some(open);
        }

        // Otherwise look at next year's windows too — winter may have
        // closed but next summer is still pending.
        let mut candidates: Vec<NaiveDate> = Vec::new();
        for year_offset in 0..=1i32 {
            let probe = NaiveDate::from_ymd_opt(date.year() + year_offset, 1, 15).unwrap_or(date);
            let (summer, winter) = if let Some(window) = self.windows.get(&country_id) {
                // Custom windows are static — they refer to a fixed
                // year. Rather than guess year arithmetic on custom
                // calendars, only treat the registered window as a
                // candidate (no rollover). For most uses the registered
                // window will be the relevant one anyway.
                let _ = probe;
                (window.summer_window, window.winter_window)
            } else {
                Self::default_european_windows(probe)
            };
            if summer.0 > date {
                candidates.push(summer.0);
            }
            if winter.0 > date {
                candidates.push(winter.0);
            }
        }
        candidates.sort();
        candidates.into_iter().next()
    }

    fn is_date_in_window(&self, date: NaiveDate, window: &(NaiveDate, NaiveDate)) -> bool {
        date >= window.0 && date <= window.1
    }

    fn default_european_windows(
        date: NaiveDate,
    ) -> ((NaiveDate, NaiveDate), (NaiveDate, NaiveDate)) {
        let year = date.year();
        let summer_start = NaiveDate::from_ymd_opt(year, 6, 1).unwrap_or(date);
        let summer_end = NaiveDate::from_ymd_opt(year, 8, 31).unwrap_or(date);
        let winter_start = NaiveDate::from_ymd_opt(year, 1, 1).unwrap_or(date);
        let winter_end = NaiveDate::from_ymd_opt(year, 1, 31).unwrap_or(date);
        ((summer_start, summer_end), (winter_start, winter_end))
    }
}

impl Default for TransferWindowManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Helper that maps a country code to its real-world transfer calendar.
/// Wrapped in a unit struct (rather than free `fn`s) so the calendar
/// surface reads like an API: `TransferCalendar::for_country(...)`. The
/// table is intentionally conservative — only countries with clearly
/// different windows from the European norm need an entry; the default
/// branch falls through to the European summer/winter pair.
pub struct TransferCalendar;

impl TransferCalendar {
    /// Canonical summer + winter windows for the given country code and
    /// year-anchor. Year is taken from `date` so callers don't have to
    /// pre-compute it (December still uses the *current* year for the
    /// winter window — the manager rolls over via `next_registration_open_date`).
    pub fn for_country(code: &str, date: NaiveDate) -> CountryTransferWindow {
        let year = date.year();
        let normalized = code.trim().to_ascii_lowercase();
        let calendar = Self::lookup(&normalized);
        calendar.windows(year, date)
    }

    fn lookup(code: &str) -> KnownCalendar {
        match code {
            // Northern hemisphere European-style: summer + winter break.
            // Covered by the default branch.
            // ── Americas (MLS / NWSL pattern) ─────────────────────
            // Two primary windows around the MLS season: secondary
            // window mid-summer and the primary one in winter.
            "us" | "usa" | "ca" | "can" => KnownCalendar::MlsStyle,

            // ── Southern-hemisphere "Apertura/Clausura" — calendar
            // year season, so windows centre on dec-feb and mid-year.
            "ar" | "arg" => KnownCalendar::SouthernHemisphereLatam,
            "br" | "bra" => KnownCalendar::SouthernHemisphereLatam,
            "uy" | "ury" => KnownCalendar::SouthernHemisphereLatam,
            "cl" | "chl" => KnownCalendar::SouthernHemisphereLatam,
            "py" | "pry" => KnownCalendar::SouthernHemisphereLatam,
            "co" | "col" => KnownCalendar::SouthernHemisphereLatam,
            "pe" | "per" => KnownCalendar::SouthernHemisphereLatam,
            "ec" | "ecu" => KnownCalendar::SouthernHemisphereLatam,
            "bo" | "bol" => KnownCalendar::SouthernHemisphereLatam,
            "ve" | "ven" => KnownCalendar::SouthernHemisphereLatam,

            // ── Asia: J-League / K-League calendar year ──────────
            "jp" | "jpn" => KnownCalendar::AsianCalendarYear,
            "kr" | "kor" => KnownCalendar::AsianCalendarYear,
            "cn" | "chn" => KnownCalendar::AsianCalendarYear,

            // ── Nordics: short summer season inside European year
            "no" | "nor" => KnownCalendar::NordicSummerSeason,
            "se" | "swe" => KnownCalendar::NordicSummerSeason,
            "fi" | "fin" => KnownCalendar::NordicSummerSeason,
            "is" | "isl" => KnownCalendar::NordicSummerSeason,
            "ie" | "irl" => KnownCalendar::NordicSummerSeason,

            // ── Russia / former-Soviet: late-summer + winter ─────
            // Russia historically used a "league year" matching
            // European calendar; the winter window opens slightly
            // later because of the long winter break.
            "ru" | "rus" => KnownCalendar::EasternEuropeLateWinter,
            "ua" | "ukr" => KnownCalendar::EasternEuropeLateWinter,
            "by" | "blr" => KnownCalendar::EasternEuropeLateWinter,
            "kz" | "kaz" => KnownCalendar::EasternEuropeLateWinter,

            // ── Default European ─────────────────────────────────
            _ => KnownCalendar::DefaultEuropean,
        }
    }
}

/// A single year's window pair for a specific country. Returned by
/// `TransferCalendar::for_country` — convertible to the storage shape
/// `TransferWindow` once a `country_id` is known.
#[derive(Debug, Clone, Copy)]
pub struct CountryTransferWindow {
    pub summer_window: (NaiveDate, NaiveDate),
    pub winter_window: (NaiveDate, NaiveDate),
}

impl CountryTransferWindow {
    pub fn into_window(self, country_id: u32) -> TransferWindow {
        TransferWindow {
            summer_window: self.summer_window,
            winter_window: self.winter_window,
            country_id,
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum KnownCalendar {
    DefaultEuropean,
    MlsStyle,
    SouthernHemisphereLatam,
    AsianCalendarYear,
    NordicSummerSeason,
    EasternEuropeLateWinter,
}

impl KnownCalendar {
    /// Concrete windows for the given anchor year. `current_date` is
    /// only used as a safe fallback if the YMD construction fails — the
    /// hardcoded month/day pairs always validate.
    fn windows(self, year: i32, current_date: NaiveDate) -> CountryTransferWindow {
        let mk = |year: i32, month: u32, day: u32| {
            NaiveDate::from_ymd_opt(year, month, day).unwrap_or(current_date)
        };
        match self {
            KnownCalendar::DefaultEuropean => CountryTransferWindow {
                summer_window: (mk(year, 6, 1), mk(year, 8, 31)),
                winter_window: (mk(year, 1, 1), mk(year, 1, 31)),
            },
            // Two primary windows: a secondary mid-summer window
            // (clubs reinforce ahead of the playoffs run) and a
            // primary winter window (the off-season).
            KnownCalendar::MlsStyle => CountryTransferWindow {
                summer_window: (mk(year, 7, 8), mk(year, 8, 4)),
                winter_window: (mk(year, 2, 14), mk(year, 4, 23)),
            },
            // Apertura/Clausura: split season inside the calendar
            // year. Two windows cover the off-season (December/
            // January) and the mid-year break (June/July).
            KnownCalendar::SouthernHemisphereLatam => CountryTransferWindow {
                summer_window: (mk(year, 12, 1), mk(year.saturating_add(1).min(year + 1), 1, 31)),
                winter_window: (mk(year, 6, 1), mk(year, 7, 20)),
            },
            // Calendar-year season: winter break is the primary
            // window (Jan–Feb), summer window for mid-season moves.
            KnownCalendar::AsianCalendarYear => CountryTransferWindow {
                summer_window: (mk(year, 7, 22), mk(year, 8, 19)),
                winter_window: (mk(year, 1, 5), mk(year, 2, 22)),
            },
            // Short summer season (April–October). Big window
            // ahead of season, smaller mid-season window.
            KnownCalendar::NordicSummerSeason => CountryTransferWindow {
                summer_window: (mk(year, 2, 1), mk(year, 4, 1)),
                winter_window: (mk(year, 7, 16), mk(year, 8, 16)),
            },
            // Eastern Europe: long winter break pushes the second
            // window into February, broader summer window.
            KnownCalendar::EasternEuropeLateWinter => CountryTransferWindow {
                summer_window: (mk(year, 6, 14), mk(year, 9, 7)),
                winter_window: (mk(year, 1, 25), mk(year, 2, 23)),
            },
        }
    }
}

/// Expansion of a registration window into the broader agreement band —
/// real markets quietly start talking ~2 weeks before a window opens and
/// wrap up a few days after a deadline closes. The numbers live on this
/// struct so `current_agreement_window_dates` stays an obvious one-liner.
struct AgreementBand;

impl AgreementBand {
    fn expand(window: (NaiveDate, NaiveDate)) -> (NaiveDate, NaiveDate) {
        let (open, close) = window;
        let pre = open
            .checked_sub_signed(chrono::Duration::days(AGREEMENT_PRE_OPEN_DAYS))
            .unwrap_or(open);
        let post = close
            .checked_add_signed(chrono::Duration::days(AGREEMENT_POST_CLOSE_DAYS))
            .unwrap_or(close);
        (pre, post)
    }
}

#[cfg(test)]
mod transfer_window_tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn custom_country_window_overrides_default_dates() {
        let mut manager = TransferWindowManager::new();
        manager.add_window(
            99,
            TransferWindow {
                summer_window: (d(2026, 2, 1), d(2026, 3, 15)),
                winter_window: (d(2026, 7, 1), d(2026, 7, 31)),
                country_id: 99,
            },
        );

        assert!(manager.is_window_open(99, d(2026, 2, 20)));
        assert!(!manager.is_window_open(99, d(2026, 6, 20)));
    }

    #[test]
    fn european_calendar_uses_summer_jun_aug() {
        let cal = TransferCalendar::for_country("en", d(2026, 1, 1));
        assert_eq!(cal.summer_window.0, d(2026, 6, 1));
        assert_eq!(cal.summer_window.1, d(2026, 8, 31));
    }

    #[test]
    fn mls_calendar_has_us_primary_winter_window() {
        let cal = TransferCalendar::for_country("us", d(2026, 1, 1));
        // Primary off-season window for the MLS calendar lives in
        // Feb–Apr, not Jun–Aug. The June probe should NOT register
        // the European summer window.
        assert!(cal.winter_window.0.month() >= 2 && cal.winter_window.0.month() <= 4);
        assert!(cal.summer_window.0.month() >= 6 && cal.summer_window.0.month() <= 8);
    }

    #[test]
    fn argentinian_calendar_uses_december_off_season() {
        let cal = TransferCalendar::for_country("ar", d(2026, 1, 1));
        // Apertura/Clausura: primary window straddles year-end.
        assert_eq!(cal.summer_window.0.month(), 12);
    }

    #[test]
    fn russian_calendar_pushes_winter_window_into_february() {
        let cal = TransferCalendar::for_country("ru", d(2026, 1, 1));
        // Long winter break: window opens mid-January, runs into Feb.
        assert!(cal.winter_window.1.month() == 2);
    }

    #[test]
    fn for_country_seeds_window_picks_country_specific_band() {
        // `TransferCalendar::for_country("ru", ...)` is the engine of
        // the `TransferWindowManager::for_country` factory. Verifying it
        // here avoids building a full `Country` (its `NationalTeam`
        // initialisation is heavy and not relevant to the window logic
        // under test). The factory itself is a one-line wrapper.
        let cal = TransferCalendar::for_country("ru", d(2026, 7, 1));
        // Russian summer window contains July; European default
        // (Jun–Aug) would also pass — confirm the seeded one is the
        // wider Russian band that runs into September.
        assert_eq!(cal.summer_window.1.month(), 9);
    }

    #[test]
    fn agreement_window_widens_around_registration_window() {
        // Default European: summer window 1-Jun – 31-Aug. The
        // agreement band should open ~14 days earlier and close ~3
        // days later — both unreachable by the formal window check.
        let mgr = TransferWindowManager::new();
        assert!(!mgr.is_window_open(0, d(2026, 5, 20)));
        assert!(mgr.is_agreement_window_open(0, d(2026, 5, 20)));
        assert!(mgr.is_agreement_window_open(0, d(2026, 9, 2)));
        assert!(!mgr.is_window_open(0, d(2026, 9, 2)));
    }

    #[test]
    fn agreement_window_closed_well_outside_registration_window() {
        // March, well outside any agreement band.
        let mgr = TransferWindowManager::new();
        assert!(!mgr.is_agreement_window_open(0, d(2026, 3, 1)));
        assert!(!mgr.is_window_open(0, d(2026, 3, 1)));
    }

    #[test]
    fn next_registration_open_date_returns_today_when_inside_window() {
        let mgr = TransferWindowManager::new();
        // July is inside the European summer window.
        let inside = d(2026, 7, 15);
        let next = mgr.next_registration_open_date(0, inside).unwrap();
        assert_eq!(next, inside);
    }

    #[test]
    fn next_registration_open_date_skips_to_next_window() {
        let mgr = TransferWindowManager::new();
        // March is outside both default European windows; the next
        // window opens 1-Jun.
        let outside = d(2026, 3, 1);
        let next = mgr.next_registration_open_date(0, outside).unwrap();
        assert_eq!(next, d(2026, 6, 1));
    }

    #[test]
    fn next_registration_open_date_rolls_over_year_after_summer_close() {
        let mgr = TransferWindowManager::new();
        // September is outside both 2026 windows; next opens Jan 2027.
        let outside = d(2026, 9, 15);
        let next = mgr.next_registration_open_date(0, outside).unwrap();
        assert_eq!(next, d(2027, 1, 1));
    }
}

/// Transfer-market-specific player valuation.
/// Wraps `PlayerValueCalculator` with market conditions (selling pressure, squad role).
pub struct PlayerValuationCalculator;

impl PlayerValuationCalculator {
    pub fn calculate_value(
        player: &Player,
        date: NaiveDate,
        league_reputation: u16,
        club_reputation: u16,
    ) -> CurrencyValue {
        Self::calculate_value_with_price_level(
            player,
            date,
            1.0,
            league_reputation,
            club_reputation,
        )
    }

    pub fn calculate_value_with_price_level(
        player: &Player,
        date: NaiveDate,
        price_level: f32,
        league_reputation: u16,
        club_reputation: u16,
    ) -> CurrencyValue {
        let base_value = PlayerValueCalculator::calculate(
            player,
            date,
            price_level,
            league_reputation,
            club_reputation,
        );

        // Transfer-listed players face market discount (buyer leverage)
        let mut market_value = base_value;

        if player.statuses.get().contains(&PlayerStatusType::Lst) {
            market_value *= 0.9;
        }

        // Players wanting to leave lose negotiating power
        if player.statuses.get().contains(&PlayerStatusType::Req) {
            market_value *= 0.85;
        }

        CurrencyValue {
            amount: market_value,
            currency: Currency::Usd,
        }
    }

    /// Resolve (league_reputation, club_market_score) for a club within
    /// its country. Single source of truth for seller-side market
    /// context — avoids each call site re-implementing the same league
    /// lookup or, worse, passing 0/0 and flattening price levels across
    /// every league. Returns (0, 0) only when the club has no main team
    /// or its league isn't registered.
    pub fn seller_context(country: &Country, club: &Club) -> (u16, u16) {
        let main = club.teams.main();
        let club_rep = main.map(|t| t.reputation.market_value_score()).unwrap_or(0);
        let league_rep = main
            .and_then(|t| t.league_id)
            .and_then(|lid| country.leagues.leagues.iter().find(|l| l.id == lid))
            .map(|l| l.reputation)
            .unwrap_or(0);
        (league_rep, club_rep)
    }

    /// Variant for callers that don't carry a `Country` reference (board
    /// audits, AI transfer-listing AI). League reputation is approximated
    /// from the club's blended score since the two correlate strongly
    /// (top-rep clubs play in top-rep leagues), keeping market values
    /// roughly correct without forcing every caller to plumb the country
    /// down.
    pub fn seller_context_from_club(club: &Club) -> (u16, u16) {
        let club_rep = club
            .teams
            .main()
            .map(|t| t.reputation.market_value_score())
            .unwrap_or(0);
        (club_rep, club_rep)
    }
}
