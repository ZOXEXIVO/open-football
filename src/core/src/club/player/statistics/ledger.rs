//! Canonical ledger types for player career and competition statistics.
//!
//! The ledger is the homogeneous shape every projection consumes —
//! independent of where the underlying data physically lives. Storage
//! writes append ledger entries directly (idempotent merge on the
//! `(season, team, kind, is_loan)` key); the projection reads them
//! into [`PlayerHistoryRow`] / [`PlayerCompetitionStatsRow`].
//!
//! Types live in their own module so [`super::history`] can hold a
//! `Vec<PlayerStatLedgerEntry>` without dragging in the projection
//! impl and creating a module cycle.

use super::types::PlayerStatistics;
use crate::continent::competitions::{
    CHAMPIONS_LEAGUE_SLUG, CONFERENCE_LEAGUE_SLUG, COPA_LIBERTADORES_SLUG, EUROPA_LEAGUE_SLUG,
};
use crate::league::Season;

/// Discriminates the four "kinds" of competitive context a stat slice
/// can belong to. Every renderable row is labelled with exactly one
/// kind, independent of which database object the underlying records
/// came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlayerStatCompetitionKind {
    /// Senior league appearances (Serie A, Premier League, …). The
    /// History page rolls continental-cup apps into these rows.
    League,
    /// Domestic cup appearances (FA Cup, Coppa Italia, …). Overview
    /// shows them as a separate row; History does not fold them in.
    DomesticCup,
    /// Continental club-competition appearances (UCL / UEL / UECL /
    /// Copa Libertadores). Overview shows them per cup; History folds
    /// them into the season's League row exactly once.
    ContinentalCup,
    /// Pre-season / friendly / youth-league appearances. Never counts
    /// toward career history; renders only as a separate Overview row.
    Friendly,
}

impl PlayerStatCompetitionKind {
    /// Map a cup competition slug to its continental-vs-domestic kind.
    /// The four continental slugs are the only authoritative continental
    /// identifiers; anything else is treated as a domestic cup.
    pub fn from_cup_slug(slug: &str) -> Self {
        if matches!(
            slug,
            CHAMPIONS_LEAGUE_SLUG
                | EUROPA_LEAGUE_SLUG
                | CONFERENCE_LEAGUE_SLUG
                | COPA_LIBERTADORES_SLUG
        ) {
            Self::ContinentalCup
        } else {
            Self::DomesticCup
        }
    }

    /// True when entries of this kind contribute to career history rows.
    /// League stints are the spine of a player's career; competitive cups
    /// fold into the season's League row. Friendlies stay overview-only.
    pub fn counts_toward_career_history(self) -> bool {
        !matches!(self, Self::Friendly)
    }
}

/// Immutable source record for a single stat slice. Storage appends
/// these (with merge on collision); the projection groups them into
/// render rows.
#[derive(Debug, Clone)]
pub struct PlayerStatLedgerEntry {
    /// Deterministic ordering token. Preserved purely so renderers can
    /// resolve ties between rows with the same `(season, team, kind)`;
    /// correctness must not depend on it.
    pub seq_id: u32,
    pub season_start_year: u16,
    pub team_slug: String,
    pub team_name: String,
    pub team_reputation: u16,
    pub league_slug: String,
    pub league_name: String,
    pub competition_kind: PlayerStatCompetitionKind,
    /// Stable competition identifier — empty for the aggregated League
    /// slice (no single league slug), populated for per-cup slices
    /// (`"copa-libertadores"`, `"fa-cup"`, …).
    pub competition_slug: String,
    pub is_loan: bool,
    pub transfer_fee: Option<f64>,
    pub statistics: PlayerStatistics,
}

/// One competition row on the player Overview, after the projection has
/// grouped per-competition slices for the current season.
#[derive(Debug, Clone)]
pub struct PlayerCompetitionStatsRow {
    pub competition_kind: PlayerStatCompetitionKind,
    /// Stable slug identifying the competition. Empty for the aggregated
    /// League / Friendly rows where no single slug applies.
    pub competition_slug: String,
    /// Display name resolved at construction time; the renderer takes
    /// it as-is.
    pub competition_name: String,
    pub statistics: PlayerStatistics,
}

/// One row on the player History, after the projection has grouped
/// ledger entries by `(season_start_year, team_slug, is_loan)`.
#[derive(Debug, Clone)]
pub struct PlayerHistoryRow {
    pub seq_id: u32,
    pub season: Season,
    pub team_slug: String,
    pub team_name: String,
    pub team_reputation: u16,
    pub league_slug: String,
    pub league_name: String,
    pub is_loan: bool,
    pub transfer_fee: Option<f64>,
    pub statistics: PlayerStatistics,
}

/// Inputs the projection needs from the live `Player` state. The live
/// caches are per-spell and get drained on transfer / loan / season
/// boundaries; the projection treats them as compatibility caches that
/// describe the still-active spell, not as the source of truth for
/// completed seasons.
#[derive(Debug, Clone)]
pub struct PlayerLiveStatsInput<'a> {
    /// Live per-spell league counter (`Player::statistics`). Surfaces
    /// as the active current-season League row.
    pub league: &'a PlayerStatistics,
    /// Live per-spell friendly counter (`Player::friendly_statistics`).
    pub friendly: &'a PlayerStatistics,
    /// Per-competition cup slices the engine has booked this spell
    /// (`Player::cup_statistics_by_competition`).
    pub cups: &'a [LiveCupSlice<'a>],
    /// Optional override for the live Friendly entry's competition
    /// slug. Empty (`""`) means the projection falls back to the
    /// active spell's league_slug. Set this to the player's actual
    /// (non-aliased) team's league_slug — e.g. `"russian-premier-league-u19"`
    /// for a U21 player — so the History tooltip can label the Friendly
    /// row with the youth league name rather than the senior team's
    /// league. Senior callers leave it empty; the row then renders as
    /// the generic "Friendly" label.
    pub friendly_source_slug: &'a str,
}

/// One cup-competition slice from the live per-spell breakdown, paired
/// with the display name resolved by the caller (web layer reads from
/// `SimulatorData` / i18n). The projection itself stays free of
/// localisation / world lookups.
#[derive(Debug, Clone)]
pub struct LiveCupSlice<'a> {
    pub competition_slug: &'a str,
    pub competition_name: String,
    pub statistics: &'a PlayerStatistics,
}

/// Per-competition breakdown for a single History row, used by the
/// hover-reveal tooltip on the player History page. One line per
/// `(competition_kind, competition_slug)` — Champions League and
/// Europa League stay distinct, FA Cup and League Cup stay distinct.
/// Lines with zero appearances are omitted by the projection except
/// for the row's own League line, which is always shown.
#[derive(Debug, Clone)]
pub struct PlayerHistoryRowBreakdown {
    pub season_start_year: u16,
    pub team_slug: String,
    pub league_slug: String,
    pub is_loan: bool,
    pub competitions: Vec<PlayerCompetitionStatsRow>,
}

/// External-source row for the domestic cup. The web layer can pass
/// stats aggregated from the cup's stored match records here; the
/// projection treats this as the authoritative domestic-cup source and
/// drops any same-slug entries from the live per-spell breakdown to
/// keep cups read from exactly one source per render.
#[derive(Debug, Clone)]
pub struct DomesticCupOverride {
    pub competition_slug: String,
    pub competition_name: String,
    pub statistics: PlayerStatistics,
}
