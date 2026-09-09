//! Country-pair transfer route policy. Centralises the real-world
//! political / sporting frictions the simulation honours so the rest
//! of the transfer machinery can defer to one source of truth. Today
//! the only entries are:
//!
//!   * Russia ↔ Ukraine moves (loan or permanent) halted from
//!     2022-02-24 onwards. The simulation treats this as a closed
//!     route — neither AI nor stale-negotiation paths may complete
//!     such a transfer past the cutoff.
//!   * Russian clubs barred from continental (UEFA) competition from
//!     2022-02-28 onwards, the date FIFA and UEFA announced their
//!     joint suspension of Russian clubs and national teams. A
//!     Russian club cannot satisfy a `WantsEuropeanCompetition`
//!     desire even at elite reputation.
//!
//! Lookups are tiny and pure. Country codes are 2-letter ISO
//! lowercase (`ru`, `ua`) — matching `Country::code` produced by the
//! database layer. Callers always pass the current sim date so the
//! policy is automatically inert in pre-2022 saves (no special-casing
//! historical leagues at the call site).
//!
//! Designed to grow: future political / federation suspensions can
//! add their own date-windowed predicate without rewiring callers.

use chrono::NaiveDate;

pub struct TransferRoutePolicy;

impl TransferRoutePolicy {
    /// Real-world cut-off after which Russia ↔ Ukraine moves stopped
    /// happening in either direction. The simulation models this as a
    /// closed route — federation transfers between the two countries
    /// don't get registered after this date, so the simulator refuses
    /// to complete the move even if an AI or stale-negotiation path
    /// somehow staged one.
    pub fn ru_ua_block_from() -> NaiveDate {
        NaiveDate::from_ymd_opt(2022, 2, 24).expect("valid date literal")
    }

    /// Real-world cut-off after which Russian clubs and national teams
    /// were barred from continental and world football competitions.
    /// FIFA and UEFA announced the joint suspension on 2022-02-28,
    /// four days after the route closure between RU and UA above.
    pub fn uefa_russia_suspension_from() -> NaiveDate {
        NaiveDate::from_ymd_opt(2022, 2, 28).expect("valid date literal")
    }

    /// True when a permanent or loan move between these two country
    /// codes is blocked outright by real-world policy on the given
    /// date. Symmetric — direction doesn't matter, the route is shut
    /// either way.
    pub fn is_blocked(from_country_code: &str, to_country_code: &str, date: NaiveDate) -> bool {
        if date < Self::ru_ua_block_from() {
            return false;
        }
        let pair = (
            from_country_code.to_ascii_lowercase(),
            to_country_code.to_ascii_lowercase(),
        );
        matches!(
            (pair.0.as_str(), pair.1.as_str()),
            ("ru", "ua") | ("ua", "ru")
        )
    }

    /// True when clubs from this country are barred from UEFA
    /// competition on the given date. Consumed by the European-
    /// ambition path: a Russian elite club's player still gets the
    /// `WantsEuropeanCompetition` mood because the club cannot
    /// actually provide it, regardless of reputation.
    pub fn is_uefa_suspended(country_code: &str, date: NaiveDate) -> bool {
        country_code.eq_ignore_ascii_case("ru") && date >= Self::uefa_russia_suspension_from()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn ru_ua_route_blocked_after_cutoff() {
        let after = d(2026, 3, 1);
        assert!(TransferRoutePolicy::is_blocked("ru", "ua", after));
        assert!(TransferRoutePolicy::is_blocked("ua", "ru", after));
        // Case-insensitive, direction-symmetric.
        assert!(TransferRoutePolicy::is_blocked("RU", "UA", after));
    }

    #[test]
    fn ru_ua_route_open_before_cutoff() {
        let before = d(2021, 6, 1);
        assert!(!TransferRoutePolicy::is_blocked("ru", "ua", before));
        assert!(!TransferRoutePolicy::is_blocked("ua", "ru", before));
    }

    #[test]
    fn other_routes_never_blocked() {
        let after = d(2026, 3, 1);
        assert!(!TransferRoutePolicy::is_blocked("es", "it", after));
        assert!(!TransferRoutePolicy::is_blocked("ru", "by", after));
        assert!(!TransferRoutePolicy::is_blocked("ua", "pl", after));
    }

    #[test]
    fn russia_uefa_suspended_after_cutoff() {
        let after = d(2026, 5, 31);
        assert!(TransferRoutePolicy::is_uefa_suspended("ru", after));
        assert!(TransferRoutePolicy::is_uefa_suspended("RU", after));
        assert!(!TransferRoutePolicy::is_uefa_suspended("ua", after));
        assert!(!TransferRoutePolicy::is_uefa_suspended("es", after));
    }

    #[test]
    fn russia_uefa_not_suspended_before_cutoff() {
        let before = d(2021, 12, 1);
        assert!(!TransferRoutePolicy::is_uefa_suspended("ru", before));
    }
}
