//! Per-player transfer **funnel trace** — the diagnostic every springboard
//! change is judged with.
//!
//! A move that never happens leaves no evidence anywhere: no request, no
//! shortlist row, no negotiation, no rejection. Asking "why was this player
//! never bought?" of the shipped model meant re-deriving the whole funnel by
//! hand from file:line. This turns that into one line per (buyer, stage) per
//! pass, printed to **stderr** so it survives being read next to a report on
//! stdout.
//!
//! Armed by `OF_TRACE_PLAYER=<player_id>` (read once, like every other `OF_*`
//! knob). Disarmed it costs one cached `OnceLock` read per call site and
//! nothing else: every emitter is written as
//! `if TransferTrace::is(id) { TransferTrace::line(...) }`, so no string is
//! ever formatted for a player nobody is tracing.
//!
//! Stages, in funnel order:
//!   * `pool`      — the player is in the world snapshot a buyer scans
//!   * `discovery` — breakout / standing scores against the admission bar
//!   * `buyer`     — one buying club's gates: reach, dedup, plausibility,
//!                   listed-target verdict, whether a file was opened
//!   * `need`      — the buyer's own squad evaluation (investment appetite)
//!   * `plaus`     — the staged move-plausibility verdict at negotiation time
//!   * `approach`  — the seller's engagement roll and its inputs
//!   * `fee`       — the club-fee round: reservation, ratio, windfall
//!   * `seller`    — seller-side economics (asset class, importance, income)
//!   * `list`      — the SELLER side putting him on the market: which pass
//!                   listed him, for what reason, and whether his signing
//!                   protection was live at the time
//!   * `exit`      — a release, a terminated contract, or a squad removal
//!   * `squad`     — where the club registered him and why: the day-0
//!                   placement verdict, and every weekly promotion the
//!                   rebalance considered (which guard stopped it, or
//!                   which loan intent it withdrew)
//!   * `loan`      — the loan funnel, one line per gate per candidate
//!                   destination: the asset class, the unsolicited-target
//!                   verdict, the guard's reach and both money terms, the
//!                   destination-level floors and the minutes read
//!
//! `list` and `exit` are the seller-side half the funnel used to have no
//! record of at all. A move that never happens leaves no evidence; so does a
//! sale that should never have happened, and the only way to answer "why was
//! this player listed four months after he was bought?" was to re-derive the
//! listing passes by hand.
//!
//! Per memory `feedback_keep_match_debug_data`, this is kept after the
//! campaign that motivated it, not deleted.

use std::env;
use std::sync::OnceLock;

use chrono::NaiveDate;

use crate::{Player, PlayerSquadStatus, PlayerStatusType};

/// Funnel tracer for one player id. Zero-cost when disarmed.
pub struct TransferTrace;

impl TransferTrace {
    /// The traced player id, or `None` when `OF_TRACE_PLAYER` is unset or
    /// unparseable. Read once for the lifetime of the process.
    pub fn target() -> Option<u32> {
        static TARGET: OnceLock<Option<u32>> = OnceLock::new();
        *TARGET.get_or_init(|| {
            env::var("OF_TRACE_PLAYER")
                .ok()
                .and_then(|v| v.trim().parse::<u32>().ok())
        })
    }

    /// True when `player_id` is the traced player. Guard every emitter with
    /// this so the argument formatting never runs on an untraced pass.
    #[inline]
    pub fn is(player_id: u32) -> bool {
        Self::target() == Some(player_id)
    }

    /// Emit one funnel line. `stage` is one of the stage tags in the module
    /// docs; `detail` is free-form `key=value` text so the whole trace can be
    /// grepped per stage and pasted as a table.
    pub fn line(player_id: u32, stage: &str, detail: impl AsRef<str>) {
        eprintln!("[of-trace {player_id}] {stage:<9} | {}", detail.as_ref());
    }

    /// Seller side: a pass has decided to put this player on the market.
    ///
    /// Every listing entry point calls this with the same four facts, which
    /// together answer the question the buy-side trace cannot: WHICH pass
    /// listed him, for what stated reason, whether the signing-patience
    /// clock was still running when it did, and how long he had actually
    /// been at the club. A marquee signing listed in January shows up here
    /// as one line naming the pass responsible.
    pub fn list(player: &Player, date: NaiveDate, pass: &str, reason: &str) {
        if !Self::is(player.id) {
            return;
        }
        Self::line(
            player.id,
            "list",
            format!(
                "pass={pass} reason={reason} protected={} status={:?} days_at_club={} \
                 statuses=[{}]",
                player.signing_protection_active(date),
                player
                    .contract
                    .as_ref()
                    .map(|c| c.squad_status.clone())
                    .unwrap_or(PlayerSquadStatus::NotYetSet),
                player
                    .contract
                    .as_ref()
                    .and_then(|c| c.started)
                    .map(|started| (date - started).num_days())
                    .unwrap_or(-1),
                Self::status_tags(player),
            ),
        );
    }

    /// Seller side: the player is leaving without a transfer — released,
    /// terminated, or swept out of the squad.
    pub fn exit(player: &Player, date: NaiveDate, pass: &str, reason: &str) {
        if !Self::is(player.id) {
            return;
        }
        Self::line(
            player.id,
            "exit",
            format!(
                "pass={pass} reason={reason} protected={} days_at_club={} statuses=[{}]",
                player.signing_protection_active(date),
                player
                    .contract
                    .as_ref()
                    .and_then(|c| c.started)
                    .map(|started| (date - started).num_days())
                    .unwrap_or(-1),
                Self::status_tags(player),
            ),
        );
    }

    fn status_tags(player: &Player) -> String {
        [
            (PlayerStatusType::Lst, "Lst"),
            (PlayerStatusType::Loa, "Loa"),
            (PlayerStatusType::Req, "Req"),
            (PlayerStatusType::Unh, "Unh"),
            (PlayerStatusType::Frt, "Frt"),
        ]
        .into_iter()
        .filter(|(status, _)| player.statuses.has(*status))
        .map(|(_, tag)| tag)
        .collect::<Vec<_>>()
        .join(",")
    }
}

/// Market arms one census run can switch off, so ONE binary produces the
/// A/B a volume guard needs.
///
/// Every band in the design's Part VI is written as "± 10 % of HEAD", and
/// there was no HEAD: the pre-design commit carries a different harness,
/// and the pre-polish run had every cash-positive club reading as
/// state-backed. Comparing two builds compares two worlds; comparing two
/// runs of one build with an arm disarmed compares one.
///
/// Each is read once, like [`TransferTrace::target`], and each fails
/// CLOSED — an unset or unparseable variable leaves the arm on, which is
/// production. Set to `1` to disarm:
///
/// * `OF_HOME_REACH_OFF` — [`super::loan_home::HomeLoanGates::reach_ok`]
///   answers with the scout's own map alone, so no club sees a compatriot
///   it could not otherwise scout.
/// * `OF_COMPATRIOT_SWEEP_OFF` — an Elite / Continental club never runs
///   the once-per-window posted-compatriot sweep.
/// * `OF_OWNER_MONEY_OFF` — [`crate::club::board::ownership::ClubBenefactor::subsidy_per_year`]
///   returns 0, which zeroes the wage subsidy, the tier envelopes and the
///   owner's fee headroom together.
/// * `OF_GEOGRAPHY_OFF` — every consumer of the country cards behaves as it
///   did before the transfer geography existed. This is the baseline arm the
///   geography campaign is measured against: comparing the geography build
///   against a pre-geography COMMIT compares two worlds, because the tree
///   also carries match-engine work, and the match engine is what the
///   real-engine harness spends its time in.
pub struct MarketSwitches;

impl MarketSwitches {
    /// True when the named variable is set to something that parses as
    /// "on". Read once per variable for the lifetime of the process.
    fn disarmed(value: Option<&String>) -> bool {
        value
            .map(|v| matches!(v.trim(), "1" | "true" | "TRUE" | "on" | "yes"))
            .unwrap_or(false)
    }

    fn read(name: &str) -> bool {
        Self::disarmed(env::var(name).ok().as_ref())
    }

    /// The compatriot REACH arm — a home club seeing a posted export its
    /// scouts never covered.
    pub fn home_reach_off() -> bool {
        static OFF: OnceLock<bool> = OnceLock::new();
        *OFF.get_or_init(|| Self::read("OF_HOME_REACH_OFF"))
    }

    /// The Elite / Continental posted-compatriot sweep.
    pub fn compatriot_sweep_off() -> bool {
        static OFF: OnceLock<bool> = OnceLock::new();
        *OFF.get_or_init(|| Self::read("OF_COMPATRIOT_SWEEP_OFF"))
    }

    /// Every cheque an owner writes: wage subsidy, tier envelopes and fee
    /// headroom all size off one call.
    pub fn owner_money_off() -> bool {
        static OFF: OnceLock<bool> = OnceLock::new();
        *OFF.get_or_init(|| Self::read("OF_OWNER_MONEY_OFF"))
    }

    /// The whole transfer GEOGRAPHY: corridor affinity on the buy side, the
    /// free-agent visibility layer, the scouting market-reach prefilter and
    /// the loan market's source-country weighting.
    ///
    /// Every one of those four already has an `is_empty()` branch that
    /// behaves pre-geography for a world with no cards loaded, so the arm is
    /// one `||` at each site rather than a second code path to maintain.
    pub fn geography_off() -> bool {
        static OFF: OnceLock<bool> = OnceLock::new();
        *OFF.get_or_init(|| Self::read("OF_GEOGRAPHY_OFF"))
    }

    /// The loan asset guard and every gate it re-shapes: the
    /// destination pricing ([`super::loan_guard::LoanAssetGuard`]), the
    /// readiness-continuous destination floors, the overqualified-minutes
    /// bound, the renown band on a loan, the wage carry, the seller's
    /// refusal delta and the broadcast cascade floor.
    ///
    /// The A/B arm for the loan-asset campaign. Only the *gates* are
    /// disarmed: squad placement at load, the promotion rules and the
    /// label / asset-class fixes are data-identity work and stay on in
    /// both arms, because comparing a world where a first-teamer is
    /// registered in the U20 against one where he is not compares two
    /// worlds rather than two policies (memory `elite_market_freeze_2026_09`).
    pub fn loan_guard_off() -> bool {
        static OFF: OnceLock<bool> = OnceLock::new();
        *OFF.get_or_init(|| Self::read("OF_LOAN_GUARD_OFF"))
    }
}
