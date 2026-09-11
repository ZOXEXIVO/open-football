use chrono::{Datelike, NaiveDate, Weekday};
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
    /// calendar table does not recognise, so unknown countries behave exactly
    /// as before. Cheap to call; no global state.
    ///
    /// Takes the id and the code rather than the `Country`: the calendar is a
    /// property of the code, and this file is the market calendar MODEL — it
    /// decides from numbers, and reading a world object is what its callers
    /// do for it.
    pub fn for_country(country_id: u32, code: &str, date: NaiveDate) -> Self {
        let mut mgr = Self::new();
        let window = TransferCalendar::for_country(code, date);
        mgr.add_window(country_id, window.into_window(country_id));
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
        self.current_agreement_window_dates(country_id, date)
            .is_some()
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

/// When a country market is live, and how often the pipeline should look at
/// it.
///
/// Every predicate answers from the country CODE. The calendar is a property
/// of the code — Europe runs a summer and a January window, MLS-style leagues
/// run theirs in February and July, Latam spans New Year — and nothing here
/// needs the club list hanging off a `Country`. That is what keeps the
/// cadence in the model layer instead of in the pipeline, where a hard-coded
/// June/January pair starved every non-European calendar for years.
pub struct MarketCadence;

impl MarketCadence {
    /// European-calendar fallback for the mid-season window bias. Kept
    /// only for the pure decision fns that carry no country context
    /// (`resolve_initial_approach`); every country-holding caller uses
    /// [`Self::is_mid_season_window_for`] instead.
    pub(in crate::transfers) fn is_january_window(date: NaiveDate) -> bool {
        date.month() == 1
    }

    /// The date falls inside the COUNTRY's shorter (mid-season
    /// reinforcement) window — January in Europe, July for MLS-style
    /// calendars, June for Latam. Drives the "prefer loans, quick
    /// fixes" mid-season bias, which the raw month==1 check applied to
    /// the wrong month everywhere outside Europe.
    pub(in crate::transfers) fn is_mid_season_window_for(code: &str, date: NaiveDate) -> bool {
        let w = TransferCalendar::for_country(code, date);
        let (s_start, s_end) = w.summer_window;
        let (w_start, w_end) = w.winter_window;
        let summer_len = (s_end - s_start).num_days();
        let winter_len = (w_end - w_start).num_days();
        let in_summer = date >= s_start && date <= s_end;
        let in_winter = date >= w_start && date <= w_end;
        if summer_len >= winter_len {
            in_winter
        } else {
            in_summer
        }
    }

    /// Full plan reset at the opening of each of the COUNTRY's transfer
    /// windows — the opening day and the day before it, mirroring the
    /// old May 31 + June 1 double for the European calendar. The
    /// hard-coded European triple reset Latam plans MID-window (their
    /// Dec–Jan window spans New Year, so Jan 1 wiped requests,
    /// shortlists, and the spent/reserved ledger halfway through their
    /// market) and never reset MLS-style calendars at their real
    /// openings at all.
    pub(in crate::transfers) fn is_window_start_for(code: &str, date: NaiveDate) -> bool {
        let tomorrow = date
            .checked_add_signed(chrono::Duration::days(1))
            .unwrap_or(date);
        // Anchor the calendar on both dates: a window opening Jan 1
        // belongs to next year's anchor when today is Dec 31.
        let today_cal = TransferCalendar::for_country(code, date);
        let tomorrow_cal = TransferCalendar::for_country(code, tomorrow);
        [
            today_cal.summer_window.0,
            today_cal.winter_window.0,
            tomorrow_cal.summer_window.0,
            tomorrow_cal.winter_window.0,
        ]
        .into_iter()
        .any(|start| date == start || tomorrow == start)
    }

    /// Re-evaluate during the COUNTRY's transfer windows.
    /// Daily during the first week of each window for fast pipeline
    /// startup, then weekly (Monday) for the rest of the window. The
    /// old hard-coded June/January cadence starved every non-European
    /// calendar: an MLS-style Feb–Apr window got no evaluation ticks
    /// (and no staff recommendations) for its entire duration.
    pub(in crate::transfers) fn should_evaluate_for(code: &str, date: NaiveDate) -> bool {
        let w = TransferCalendar::for_country(code, date);
        for (start, end) in [w.summer_window, w.winter_window] {
            if date >= start && date <= end {
                let days_in = (date - start).num_days();
                return days_in < 7 || date.weekday() == Weekday::Mon;
            }
        }
        false
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
                summer_window: (
                    mk(year, 12, 1),
                    mk(year.saturating_add(1).min(year + 1), 1, 31),
                ),
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
