//! Pure projection layer for player career and competition statistics.
//!
//! Legacy storage in [`PlayerStatisticsHistory`] (`items`, `current`,
//! `continental`) and the live per-spell caches on [`Player`]
//! (`statistics`, `friendly_statistics`, `cup_statistics_by_competition`)
//! drive event ordering and save compatibility, but the Overview and
//! History pages are now built from a single read-only projection on top
//! of all of them. The same projection feeds both pages so they cannot
//! disagree, and a single grouping policy decides cup-folding rules.
//!
//! Domain boundary: source records below
//! ([`PlayerStatLedgerEntry`]) are adapted from the existing storage at
//! read time; the projection layer never mutates `Player`, history, or
//! any team/country state — calling it twice with the same input
//! returns identical output.

use super::history::PlayerStatisticsHistory;
use super::ledger::{
    DomesticCupOverride, PlayerCompetitionStatsRow, PlayerHistoryRow, PlayerHistoryRowBreakdown,
    PlayerLiveStatsInput, PlayerStatCompetitionKind, PlayerStatLedgerEntry,
};
use super::types::PlayerStatistics;
use crate::league::Season;
use chrono::NaiveDate;
use std::collections::HashMap;

// Type definitions live in `super::ledger` so storage and projection
// can both depend on them without a module cycle. The projection only
// reads them — it never mutates `Player`, history, or any team/country
// state. Calling it twice with the same input returns identical output.

/// Single source of truth for how a ledger entry maps to a History row /
/// breakdown grouping key. `player_history_rows` and
/// `player_history_breakdowns` both build their map keys via
/// [`Self::from_entry`] so changing the grouping policy here updates
/// both consumers in lock-step.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct RowKey {
    season_year: u16,
    team_slug: String,
    league_slug: String,
}

impl RowKey {
    fn from_entry(entry: &PlayerStatLedgerEntry) -> Self {
        Self {
            season_year: entry.season_start_year,
            team_slug: entry.team_slug.clone(),
            league_slug: entry.league_slug.clone(),
        }
    }
}

/// Pure-projection facade. All methods are read-only and side-effect
/// free: handing the same `PlayerStatisticsHistory` + live inputs in
/// twice yields identical results.
pub struct PlayerStatisticsProjection;

impl PlayerStatisticsProjection {
    /// Build the canonical ledger from existing storage and the live
    /// caches. The ledger is the homogeneous shape every projection
    /// function consumes; building it explicitly makes the
    /// frozen-vs-live splits visible in one place instead of being
    /// inferred from event ordering downstream.
    ///
    /// Every frozen `items` row becomes a League ledger entry. Every
    /// `continental` entry becomes a ContinentalCup entry. Every
    /// `current` entry becomes a League entry for its season — using
    /// the live counter for the still-active spell, the snapshot stats
    /// for departed spells. Live per-competition cup slices become
    /// per-kind entries attributed to the active spell's
    /// `(season, team)`; the friendly bucket adds a Friendly entry.
    pub fn build_ledger(
        history: &PlayerStatisticsHistory,
        live: &PlayerLiveStatsInput<'_>,
        domestic_cup: Option<&DomesticCupOverride>,
        current_date: NaiveDate,
    ) -> Vec<PlayerStatLedgerEntry> {
        let mut ledger: Vec<PlayerStatLedgerEntry> = Vec::new();
        let current_year = Self::current_season_year(history, current_date);

        // ── 1. Past-season frozen records ─────────────────────────
        //
        // Prefer the canonical `season_ledger` when populated: it was
        // written before any storage filters could drop a row, so a
        // quiet year that legacy `items` lost is still present here.
        // Old saves predate the field — fall back to the legacy adapter
        // for them.
        let use_canonical = !history.season_ledger.is_empty();
        if use_canonical {
            for entry in &history.season_ledger {
                // League: load past seasons only — the in-progress
                // year's League stats live in `history.current` + the
                // live counter.
                //
                // Non-League: load regardless of year. Inter-spell
                // drains (mid-season transfer / loan / cancel-loan)
                // tag the row with the current season year; the active
                // spell's own non-League stats are still in the live
                // caches, so no double-count risk.
                let is_past = entry.season_start_year < current_year;
                let is_inter_spell_non_league =
                    entry.competition_kind != PlayerStatCompetitionKind::League;
                if is_past || is_inter_spell_non_league {
                    ledger.push(entry.clone());
                }
            }
            // Save-compat: the `continental` field is older than the
            // canonical ledger and may carry rows the ledger doesn't
            // mirror yet (a save written between the two adds). Surface
            // any (season, team) ContinentalCup entry the ledger
            // doesn't already have to avoid double-counting.
            for cont in &history.continental {
                if cont.season_year >= current_year {
                    continue;
                }
                let already_in_ledger = history.season_ledger.iter().any(|e| {
                    e.season_start_year == cont.season_year
                        && e.team_slug == cont.team_slug
                        && e.competition_kind == PlayerStatCompetitionKind::ContinentalCup
                });
                if already_in_ledger {
                    continue;
                }
                let (seq_id, league_slug, league_name) =
                    league_anchor_for(history, cont.season_year, &cont.team_slug)
                        .unwrap_or((0, String::new(), String::new()));
                ledger.push(PlayerStatLedgerEntry {
                    seq_id,
                    season_start_year: cont.season_year,
                    team_slug: cont.team_slug.clone(),
                    team_name: cont.team_slug.clone(),
                    team_reputation: 0,
                    league_slug,
                    league_name,
                    competition_kind: PlayerStatCompetitionKind::ContinentalCup,
                    competition_slug: String::new(),
                    is_loan: false,
                    transfer_fee: None,
                    statistics: cont.statistics.clone(),
                });
            }
        } else {
            for item in &history.items {
                ledger.push(PlayerStatLedgerEntry {
                    seq_id: item.seq_id,
                    season_start_year: item.season.start_year,
                    team_slug: item.team_slug.clone(),
                    team_name: item.team_name.clone(),
                    team_reputation: item.team_reputation,
                    league_slug: item.league_slug.clone(),
                    league_name: item.league_name.clone(),
                    competition_kind: PlayerStatCompetitionKind::League,
                    competition_slug: item.league_slug.clone(),
                    is_loan: item.is_loan,
                    transfer_fee: item.transfer_fee,
                    statistics: item.statistics.clone(),
                });
            }

            // Frozen continental rows (legacy adapter only).
            for cont in &history.continental {
                let (seq_id, league_slug, league_name) =
                    league_anchor_for(history, cont.season_year, &cont.team_slug)
                        .unwrap_or((0, String::new(), String::new()));
                ledger.push(PlayerStatLedgerEntry {
                    seq_id,
                    season_start_year: cont.season_year,
                    team_slug: cont.team_slug.clone(),
                    team_name: cont.team_slug.clone(),
                    team_reputation: 0,
                    league_slug,
                    league_name,
                    competition_kind: PlayerStatCompetitionKind::ContinentalCup,
                    competition_slug: String::new(),
                    is_loan: false,
                    transfer_fee: None,
                    statistics: cont.statistics.clone(),
                });
            }
        }

        // ── 3. Current-season spells ──────────────────────────────
        //
        // The live League counter (`player.statistics`) is the
        // authoritative tally for the player's *active* spell. The
        // snapshot stored in an active `current` entry is only updated
        // at event boundaries, where it is written from the drained
        // live counter — so for the active spell it is always either
        // empty (freshly seeded) or a stale duplicate of the live
        // counter. Merging snapshot + live would therefore double-count
        // and produce the unstable mid-season row this projection is
        // meant to avoid; the active spell's stats come from
        // `live.league` alone.
        //
        // Prior same-season spells at the same club survive as their own
        // *departed* entries (see `record_intra_club_move`) and keep
        // their stored snapshot, so an intra-club bounce still sums
        // correctly once `player_history_rows` groups by
        // (season, team, league, is_loan) — without the active row ever
        // merging a snapshot.
        //
        // Only the FIRST active entry adopts the live counter; any
        // further active entry (which a malformed or legacy save could
        // carry) falls back to its snapshot so the live counter is never
        // counted twice. Active rows are re-labelled to today's season —
        // the snapshot's `joined_date` season can be stale when the next
        // season-end has been delayed.
        let mut live_applied = false;
        for entry in &history.current {
            let is_active = entry.departed_date.is_none();
            let use_live = is_active && !live_applied;
            let row_season_year = if is_active {
                current_year
            } else {
                let joined_year = Season::from_date(entry.joined_date).start_year;
                joined_year.min(current_year)
            };

            let stats = if use_live {
                live_applied = true;
                live.league.clone()
            } else {
                entry.statistics.clone()
            };

            ledger.push(PlayerStatLedgerEntry {
                seq_id: entry.seq_id,
                season_start_year: row_season_year,
                team_slug: entry.team_slug.clone(),
                team_name: entry.team_name.clone(),
                team_reputation: entry.team_reputation,
                league_slug: entry.league_slug.clone(),
                league_name: entry.league_name.clone(),
                competition_kind: PlayerStatCompetitionKind::League,
                competition_slug: entry.league_slug.clone(),
                is_loan: entry.is_loan,
                transfer_fee: entry.transfer_fee,
                statistics: stats,
            });
        }

        // ── 3b. Live secondary-team league appearances ────────────
        //
        // In-progress-season league games the player made for ANOTHER of
        // his club's teams (a reserve pulled up to the main XI, or a senior
        // fielded for the "2" side). These live on the player's history
        // until the season-end snapshot freezes them, so the projection
        // reads them straight from `current_secondary` and emits one
        // current-season League row per team — the page then shows a line
        // for every team the player turned out for this season instead of
        // folding both teams' games under the active spell. seq_id 0 keeps
        // these below the active home row in the per-season sort.
        for slice in &history.current_secondary {
            if slice.statistics.total_games() == 0 {
                continue;
            }
            ledger.push(PlayerStatLedgerEntry {
                seq_id: 0,
                season_start_year: slice.season_start_year,
                team_slug: slice.team_slug.clone(),
                team_name: slice.team_name.clone(),
                team_reputation: slice.team_reputation,
                league_slug: slice.league_slug.clone(),
                league_name: slice.league_name.clone(),
                competition_kind: PlayerStatCompetitionKind::League,
                competition_slug: slice.league_slug.clone(),
                is_loan: false,
                transfer_fee: None,
                statistics: slice.statistics.clone(),
            });
        }

        // Resolve the active spell's `(team_slug, season_year)` once.
        // Live cup / friendly slices belong to *this* spell only — never
        // to a past row, and never to a departed current-season row.
        // No `is_loan` here: cup / friendly entries don't carry the
        // loan flag because grouping ignores it (a match is a match,
        // regardless of contract type).
        let active_anchor: Option<(String, String, String, u16, u32)> = history
            .current
            .iter()
            .find(|e| e.departed_date.is_none())
            .map(|e| {
                (
                    e.team_slug.clone(),
                    e.league_slug.clone(),
                    e.league_name.clone(),
                    current_year,
                    e.seq_id,
                )
            });

        // ── 4. Live per-competition cup slices ────────────────────
        //
        // The domestic-cup override (if any) wins over the live slice
        // with the same slug: cups must be read from exactly one
        // source per render, and the records-based aggregate is the
        // authoritative one because the live per-spell counter gets
        // drained on intra-club moves.
        let domestic_slug = domestic_cup.map(|d| d.competition_slug.as_str());
        if let Some((team_slug, league_slug, league_name, season_year, active_seq)) =
            active_anchor.as_ref()
        {
            for slice in live.cups {
                if Some(slice.competition_slug) == domestic_slug {
                    continue;
                }
                if slice.statistics.total_games() == 0 {
                    continue;
                }
                ledger.push(PlayerStatLedgerEntry {
                    seq_id: *active_seq,
                    season_start_year: *season_year,
                    team_slug: team_slug.clone(),
                    team_name: team_slug.clone(),
                    team_reputation: 0,
                    league_slug: league_slug.clone(),
                    league_name: league_name.clone(),
                    competition_kind: PlayerStatCompetitionKind::from_cup_slug(
                        slice.competition_slug,
                    ),
                    competition_slug: slice.competition_slug.to_string(),
                    is_loan: false,
                    transfer_fee: None,
                    statistics: slice.statistics.clone(),
                });
            }

            // ── 5. Domestic-cup override (records-sourced) ────────
            if let Some(dc) = domestic_cup {
                if dc.statistics.total_games() > 0 {
                    ledger.push(PlayerStatLedgerEntry {
                        seq_id: *active_seq,
                        season_start_year: *season_year,
                        team_slug: team_slug.clone(),
                        team_name: team_slug.clone(),
                        team_reputation: 0,
                        league_slug: league_slug.clone(),
                        league_name: league_name.clone(),
                        competition_kind: PlayerStatCompetitionKind::DomesticCup,
                        competition_slug: dc.competition_slug.clone(),
                        is_loan: false,
                        transfer_fee: None,
                        statistics: dc.statistics.clone(),
                    });
                }
            }

            // ── 6. Live friendly slice ───────────────────────────
            if live.friendly.total_games() > 0 {
                let friendly_slug = if live.friendly_source_slug.is_empty() {
                    league_slug.clone()
                } else {
                    live.friendly_source_slug.to_string()
                };
                ledger.push(PlayerStatLedgerEntry {
                    seq_id: *active_seq,
                    season_start_year: *season_year,
                    team_slug: team_slug.clone(),
                    team_name: team_slug.clone(),
                    team_reputation: 0,
                    league_slug: league_slug.clone(),
                    league_name: league_name.clone(),
                    competition_kind: PlayerStatCompetitionKind::Friendly,
                    competition_slug: friendly_slug,
                    is_loan: false,
                    transfer_fee: None,
                    statistics: live.friendly.clone(),
                });
            }
        }

        ledger
    }

    /// The season year the player's still-active spell belongs to — the
    /// boundary `build_ledger` uses to split frozen past seasons from the
    /// in-progress one.
    ///
    /// `Season::from_date` hardcodes an August boundary, but the season-end
    /// snapshot freezes a just-ended season under its CALENDAR year
    /// (`date.year() - 1`) on each league's own season-start day. For a
    /// league whose season starts before August the two disagree for the
    /// whole Jan–Jul window after the snapshot: `from_date` still reports
    /// the season that was just frozen as the "current" one. Using it
    /// directly made `build_ledger` treat the freshly-frozen League rows as
    /// in-progress and hide them (expecting the live counter to carry them);
    /// they only reappeared once the calendar crossed the next August
    /// boundary — the user-reported "history stats hidden until the new
    /// season, especially after a loan return" bug (a returned loanee has no
    /// active spell to fall back on, so the loan row vanishes entirely).
    ///
    /// A League row in the canonical ledger (or legacy `items`) is only ever
    /// written by the season-end drain, so the latest such year is a
    /// definitively COMPLETED season and the current season is at least one
    /// past it. Taking the max of that and the calendar boundary recovers the
    /// true current season without threading per-league season-start
    /// configuration into the projection. Non-League ledger rows are ignored
    /// here: inter-spell drains stamp them with the in-progress season year,
    /// so they are not proof a season has ended.
    fn current_season_year(history: &PlayerStatisticsHistory, current_date: NaiveDate) -> u16 {
        let today_year = Season::from_date(current_date).start_year;
        let last_frozen_league_year = history
            .season_ledger
            .iter()
            .filter(|e| e.competition_kind == PlayerStatCompetitionKind::League)
            .map(|e| e.season_start_year)
            .chain(history.items.iter().map(|i| i.season.start_year))
            .max();
        match last_frozen_league_year {
            Some(frozen) => today_year.max(frozen.saturating_add(1)),
            None => today_year,
        }
    }

    /// Project the current season's stats into per-competition Overview
    /// rows. Filters the ledger to `Season::from_date(current_date)` and
    /// groups remaining entries by `(competition_kind, competition_slug)`
    /// so the same cup can never appear twice.
    ///
    /// Output order is: League → Friendly → DomesticCup(s) → ContinentalCup(s).
    pub fn player_overview_statistics(
        history: &PlayerStatisticsHistory,
        live: &PlayerLiveStatsInput<'_>,
        domestic_cup: Option<&DomesticCupOverride>,
        current_date: NaiveDate,
    ) -> Vec<PlayerCompetitionStatsRow> {
        let ledger = Self::build_ledger(history, live, domestic_cup, current_date);
        let current_year = Self::current_season_year(history, current_date);

        // Accumulators keyed by (kind, slug) so a stale duplicate (e.g. a
        // legacy live cup slice plus an override that matches it) cannot
        // bleed into two rows.
        let mut league_total = PlayerStatistics::default();
        let mut league_seen = false;
        let mut friendly_total = PlayerStatistics::default();
        let mut friendly_seen = false;
        let mut per_cup: HashMap<(PlayerStatCompetitionKind, String), PlayerCompetitionStatsRow> =
            HashMap::new();
        // Stable order for the per-cup rows in the output.
        let mut cup_order: Vec<(PlayerStatCompetitionKind, String)> = Vec::new();

        for entry in ledger
            .into_iter()
            .filter(|e| e.season_start_year == current_year)
        {
            match entry.competition_kind {
                PlayerStatCompetitionKind::League => {
                    league_total.merge_from(&entry.statistics);
                    league_seen = true;
                }
                PlayerStatCompetitionKind::Friendly => {
                    friendly_total.merge_from(&entry.statistics);
                    friendly_seen = true;
                }
                kind @ (PlayerStatCompetitionKind::DomesticCup
                | PlayerStatCompetitionKind::ContinentalCup) => {
                    let key = (kind, entry.competition_slug.clone());
                    let row = per_cup.entry(key.clone()).or_insert_with(|| {
                        cup_order.push(key.clone());
                        PlayerCompetitionStatsRow {
                            competition_kind: kind,
                            competition_slug: entry.competition_slug.clone(),
                            competition_name: Self::resolve_cup_name(
                                &entry.competition_slug,
                                live,
                                domestic_cup,
                            ),
                            statistics: PlayerStatistics::default(),
                        }
                    });
                    row.statistics.merge_from(&entry.statistics);
                }
            }
        }

        let mut rows: Vec<PlayerCompetitionStatsRow> = Vec::new();
        if league_seen {
            rows.push(PlayerCompetitionStatsRow {
                competition_kind: PlayerStatCompetitionKind::League,
                competition_slug: String::new(),
                competition_name: String::new(),
                statistics: league_total,
            });
        }
        if friendly_seen && friendly_total.total_games() > 0 {
            rows.push(PlayerCompetitionStatsRow {
                competition_kind: PlayerStatCompetitionKind::Friendly,
                competition_slug: String::new(),
                competition_name: String::new(),
                statistics: friendly_total,
            });
        }
        // Domestic cups before continental ones, in stable insertion
        // order within each block.
        for key in cup_order
            .iter()
            .filter(|(k, _)| *k == PlayerStatCompetitionKind::DomesticCup)
        {
            if let Some(row) = per_cup.remove(key) {
                rows.push(row);
            }
        }
        for key in cup_order
            .iter()
            .filter(|(k, _)| *k == PlayerStatCompetitionKind::ContinentalCup)
        {
            if let Some(row) = per_cup.remove(key) {
                rows.push(row);
            }
        }
        rows
    }

    /// Project the ledger into History rows, grouped by
    /// `(season_start_year, team_slug, league_slug)`. League and
    /// ContinentalCup and DomesticCup entries fold into the same row;
    /// Friendly entries are excluded by [`counts_toward_career_history`].
    ///
    /// Grouping deliberately ignores the spell's loan flag — a match
    /// is a match regardless of contract type. The row's `is_loan`
    /// label is derived from the League entries in the group (latest
    /// seq_id wins, so a loan-then-permanent rare case shows the most
    /// recent contract). This is what makes the data flow robust: the
    /// only thing the freeze pipeline has to get right is *which team
    /// the matches belong to*, never the loan flag — that's metadata
    /// the row picks up from its own League slice.
    ///
    /// Output is sorted by season-year descending, then by `seq_id`
    /// descending so the most recent row surfaces first.
    pub fn player_history_rows(
        history: &PlayerStatisticsHistory,
        live: &PlayerLiveStatsInput<'_>,
        current_date: NaiveDate,
    ) -> Vec<PlayerHistoryRow> {
        // History never sources from the records-based domestic cup
        // override. The in-house ledger is the only allowed source for
        // career rows so domestic cups are counted from exactly one
        // source.
        let ledger = Self::build_ledger(history, live, None, current_date);

        // (season, team, league) → row. HashMap is fine for grouping;
        // we re-sort the result vector below for stable rendering.
        let mut rows: HashMap<RowKey, PlayerHistoryRow> = HashMap::new();
        // Stable insertion order for rows that share their sort key
        // (rare, but keeps test output deterministic).
        let mut order: Vec<RowKey> = Vec::new();
        // Highest seq_id of a LEAGUE entry seen per row. League entries
        // own the spell metadata (is_loan, transfer_fee). Cup / friendly
        // slices carry no loan flag of their own, so they must not
        // overwrite the row's loan label with their hardcoded `false`.
        let mut latest_league_seq: HashMap<RowKey, u32> = HashMap::new();

        // The active current-season spell is always shown — even at
        // 0 games — so the renderer can say "this is where the player
        // is right now". The earliest seq_id in the player's career is
        // protected on a first/only season so a manual transfer out
        // before any senior game cannot erase the origin row.
        let active_seq: Option<u32> = history
            .current
            .iter()
            .find(|e| e.departed_date.is_none())
            .map(|e| e.seq_id);
        let initial_seq: Option<u32> = if history.items.is_empty() {
            history.current.iter().map(|e| e.seq_id).min()
        } else {
            None
        };
        let mut protected_seqs: Vec<u32> = Vec::new();
        if let Some(s) = active_seq {
            protected_seqs.push(s);
        }
        if let Some(s) = initial_seq {
            protected_seqs.push(s);
        }

        for entry in ledger {
            if !entry.competition_kind.counts_toward_career_history() {
                continue;
            }
            let key = RowKey::from_entry(&entry);
            let row = rows.entry(key.clone()).or_insert_with(|| {
                order.push(key.clone());
                PlayerHistoryRow {
                    seq_id: entry.seq_id,
                    season: Season::new(entry.season_start_year),
                    team_slug: entry.team_slug.clone(),
                    team_name: entry.team_name.clone(),
                    team_reputation: entry.team_reputation,
                    league_slug: entry.league_slug.clone(),
                    league_name: entry.league_name.clone(),
                    is_loan: false,
                    transfer_fee: None,
                    statistics: PlayerStatistics::default(),
                }
            });
            row.statistics.merge_from(&entry.statistics);
            row.seq_id = row.seq_id.max(entry.seq_id);
            // Only League entries are authoritative for is_loan / fee —
            // latest seq_id wins so loan→permanent in the same season
            // shows the player's final contract type.
            if entry.competition_kind == PlayerStatCompetitionKind::League {
                let is_new_latest = latest_league_seq
                    .get(&key)
                    .is_none_or(|&prev| entry.seq_id >= prev);
                if is_new_latest {
                    latest_league_seq.insert(key.clone(), entry.seq_id);
                    row.is_loan = entry.is_loan;
                    if entry.transfer_fee.is_some() {
                        row.transfer_fee = entry.transfer_fee;
                    }
                } else if row.transfer_fee.is_none() {
                    row.transfer_fee = entry.transfer_fee;
                }
            }
            if row.team_reputation == 0 && entry.team_reputation > 0 {
                row.team_reputation = entry.team_reputation;
            }
            if row.team_name.is_empty() && !entry.team_name.is_empty() {
                row.team_name = entry.team_name;
            }
            if row.league_name.is_empty() && !entry.league_name.is_empty() {
                row.league_name = entry.league_name;
                row.league_slug = entry.league_slug;
            }
        }

        // Drop noise rows: 0-game / no-fee entries that are neither
        // protected nor the sole record of the season. Same shape as
        // the legacy merge step — but here the protection set is
        // visible in one place instead of two duplicated helpers.
        let snapshot: Vec<PlayerHistoryRow> = rows.values().cloned().collect();
        // The player's first/debut season — its owning-club record is kept
        // even when they were loaned out immediately ("where the career
        // began"). Later full-loan seasons don't get that protection.
        let debut_year: Option<u16> = snapshot.iter().map(|r| r.season.start_year).min();
        // The career-origin row: the earliest spell of the debut season.
        // The season-end drain keeps it forever (`is_initial_record` —
        // "the very first career record is always kept, even with 0
        // games — it's the player's starting club") and the projection
        // must agree. The `initial_seq` protection above only covers the
        // pre-freeze window (`items` still empty); without this, a player
        // sold before his first senior appearance loses the origin club
        // from the page at the first season-end freeze, while the
        // transfers page still shows the move.
        let origin_seq: Option<u32> = snapshot
            .iter()
            .filter(|r| Some(r.season.start_year) == debut_year)
            .map(|r| r.seq_id)
            .min();
        rows.retain(|_, row| {
            if protected_seqs.contains(&row.seq_id) {
                return true;
            }
            if Some(row.season.start_year) == debut_year && Some(row.seq_id) == origin_seq {
                return true;
            }
            if row.statistics.total_games() > 0 {
                return true;
            }
            // A real signing fee marks a real event — keep even at 0 apps.
            // Two cases qualify:
            //   * a *paid* fee on any row (loan or permanent), and
            //   * the `Some(0.0)` "free" sentinel on a PERMANENT row — a
            //     free transfer / free signing is a genuine event, and
            //     only the re-seed paths (season-end roll-over) ever write
            //     a permanent row with `transfer_fee = None`, so a present
            //     fee here can't be a phantom seed.
            // A free LOAN's `Some(0.0)` is deliberately NOT short-circuited
            // here: it must fall through to the `is_loan` branch below,
            // which owns phantom-loan detection (a continued-loan re-seed
            // can also carry `Some(0.0)`). Keeping the permanent free
            // signing here, before the `phantom_alongside_other_senior`
            // drop, is what lets its "Free" label survive once the row
            // freezes alongside a played sibling spell from the same season
            // (the reported "free move shows on transfers but not in
            // history" bug).
            let paid_fee = matches!(row.transfer_fee, Some(f) if f > 0.0);
            if paid_fee || (row.transfer_fee.is_some() && !row.is_loan) {
                return true;
            }
            // Every loan spell is a real part of the player's career and
            // must show — even at 0 apps (injury, squad rotation, a loan
            // they were registered for but never featured in). The ONLY
            // loan row dropped is a genuine phantom: a 0-app loan stamped
            // under a season the player demonstrably spent ELSEWHERE,
            // proven by a sibling row that actually PLAYED games that
            // season (e.g. 36 league games at the parent club, with a
            // loan event mis-stamped into the same season window). A
            // sibling that merely *exists* at 0 apps — the owning-club
            // "career home" row — does NOT make the loan redundant; both
            // coexist. The fee is irrelevant here: the re-seed for a
            // continued loan drops it to `None`, so it can't distinguish
            // a real spell from a seed.
            if row.is_loan {
                let player_actually_played_elsewhere = snapshot.iter().any(|other| {
                    other.season.start_year == row.season.start_year
                        && !(other.team_slug == row.team_slug
                            && other.league_slug == row.league_slug)
                        && other.statistics.total_games() > 0
                });
                return !player_actually_played_elsewhere;
            }
            // Non-loan 0-app, no real fee. Drop when a sibling NON-LOAN
            // team in the same season actually played or paid a real
            // fee — that's the intra-club bounce pattern. A loan
            // sibling does not trigger the drop: the parent-club row
            // must coexist with a loan spell as the "career home"
            // marker. A `Some(0.0)` sibling counts as content (a free
            // signing record).
            let phantom_alongside_other_senior = snapshot.iter().any(|other| {
                other.season.start_year == row.season.start_year
                    && !other.is_loan
                    && (other.team_slug != row.team_slug || other.league_slug != row.league_slug)
                    && (other.statistics.total_games() > 0 || other.transfer_fee.is_some())
            });
            if phantom_alongside_other_senior {
                return false;
            }
            // Owning-club 0-app row during a loan-out season. The player
            // spent the season away, so the loan row(s) already represent
            // it; a 0-app parent line is redundant noise — EXCEPT for the
            // player's debut season, whose owning-club record is preserved
            // as the "where the career began" marker (the message the
            // earlier "initial Spartak row collapsed" report was about).
            let loaned_out_this_season = snapshot.iter().any(|other| {
                other.season.start_year == row.season.start_year && other.is_loan
            });
            if loaned_out_this_season {
                return Some(row.season.start_year) == debut_year;
            }
            // 0-app non-loan row with no contesting sibling. Two
            // patterns land here and both must be kept:
            //  * The U18/U21 alias "career home" row — a youth-only
            //    season has no senior callups (0 apps) and no fee, but
            //    the Main-aliased row is the sole record of where the
            //    player spent the year. Without it, every past quiet
            //    youth season vanishes from the history page.
            //  * A `Some(0.0)` "Free" signing record — its fee marks a
            //    real signing event for the season.
            // If there's any sibling row at all for this season — even
            // a 0-app one — the earlier branches already covered the
            // content-bearing variants and this row is a phantom seed.
            let is_sole_record_of_season = !snapshot.iter().any(|other| {
                other.season.start_year == row.season.start_year
                    && (other.team_slug != row.team_slug
                        || other.league_slug != row.league_slug)
            });
            is_sole_record_of_season || row.transfer_fee.is_some()
        });

        let mut result: Vec<PlayerHistoryRow> = order
            .into_iter()
            .filter_map(|key| rows.remove(&key))
            .collect();

        // Continuity gap-fill: every season the player demonstrably
        // existed at a club must surface at least one row, even if the
        // storage layer dropped the seed (a quiet U21 year between two
        // played senior seasons is the classic case). When two non-loan
        // rows for the same team bracket a gap of N missing years, we
        // synthesise a 0-app placeholder for each gap year — but only
        // when no other row of any kind already covers that year, so a
        // loan-out spell or a different-team row in the gap is left
        // alone.
        Self::fill_career_gaps(&mut result);

        result.sort_by(|a, b| {
            b.season
                .start_year
                .cmp(&a.season.start_year)
                // Within a season, loan spells sort ABOVE the parent /
                // home-club row. A home row can otherwise acquire a
                // seq_id HIGHER than a loan that chronologically preceded
                // it: a loan return lands the player on the Main team, and
                // `move_loan_returns_to_reserve` then opens a *fresh*
                // reserve spell (`record_intra_club_move`) with a new
                // seq_id — so the season-long home floats above the loan
                // it contained. Main-home players never get that re-place,
                // so their home keeps its original (pre-loan) seq and the
                // loan already sorts on top; ordering loans first makes the
                // reserve case match, independent of the inflated seq.
                .then(b.is_loan.cmp(&a.is_loan))
                .then(b.seq_id.cmp(&a.seq_id))
        });

        // Played-subs roll into played for all but the latest row, so
        // historical lines render as a single "appearances" column. The
        // most recent row keeps subs as a separate column per the UI.
        if let Some(max_seq) = result.iter().map(|r| r.seq_id).max() {
            for row in &mut result {
                if row.seq_id != max_seq && row.statistics.played_subs > 0 {
                    row.statistics.played += row.statistics.played_subs;
                    row.statistics.played_subs = 0;
                }
            }
        }

        result
    }

    /// Insert 0-app placeholder rows for any year a player demonstrably
    /// spent at the same non-loan team — bracketed by two existing
    /// non-loan rows for that team — but for which no row of any kind
    /// survived storage. Defensive: the storage pipeline already does
    /// gap-fill at season-end, but a missed-snapshot / trivial-stint
    /// drop can still erase a quiet year; the projection patches it so
    /// the rule "every season the player existed at a club shows at
    /// least one row" holds at render time.
    fn fill_career_gaps(rows: &mut Vec<PlayerHistoryRow>) {
        if rows.is_empty() {
            return;
        }
        // Years that already carry SOME row (loan or otherwise) — those
        // are not gaps; the player accounted for that season elsewhere
        // (e.g. a loan-out spell or a different-team row).
        let occupied_years: std::collections::HashSet<u16> =
            rows.iter().map(|r| r.season.start_year).collect();

        // The career span is bounded by the player's actual rows: we
        // only fill *internal* gaps, never before the first season or
        // after the last. A gap year inside the span means the storage
        // pipeline dropped the season's seed (missed snapshot, trivial-
        // stint filter) even though the player demonstrably existed at a
        // club on both sides of it.
        let min_year = rows.iter().map(|r| r.season.start_year).min().unwrap();
        let max_year = rows.iter().map(|r| r.season.start_year).max().unwrap();

        // Non-loan rows are the "career home" anchors a placeholder is
        // attributed to: a synthetic gap row continues the most recent
        // home club (carry-forward), falling back to the earliest home
        // after the gap when the gap precedes the player's first non-loan
        // season. A loan row is never used as an anchor — synthesising a
        // phantom loan would misrepresent the spell — so a career made up
        // entirely of loans gets no fill (there's no home to attribute).
        let mut homes: Vec<&PlayerHistoryRow> = rows.iter().filter(|r| !r.is_loan).collect();
        if homes.is_empty() {
            return;
        }
        homes.sort_by_key(|r| r.season.start_year);

        let mut additions: Vec<PlayerHistoryRow> = Vec::new();
        for year in (min_year.saturating_add(1))..max_year {
            if occupied_years.contains(&year) {
                continue;
            }
            let anchor = homes
                .iter()
                .rev()
                .find(|h| h.season.start_year < year)
                .or_else(|| homes.iter().find(|h| h.season.start_year > year));
            let anchor = match anchor {
                Some(a) => *a,
                None => continue,
            };
            additions.push(PlayerHistoryRow {
                // Synthetic rows take seq_id 0 so the played-subs rollup
                // below never treats them as the latest row — that role
                // belongs to a real seq.
                seq_id: 0,
                season: Season::new(year),
                team_slug: anchor.team_slug.clone(),
                team_name: anchor.team_name.clone(),
                team_reputation: anchor.team_reputation,
                league_slug: anchor.league_slug.clone(),
                league_name: anchor.league_name.clone(),
                is_loan: false,
                transfer_fee: None,
                statistics: PlayerStatistics::default(),
            });
        }
        rows.extend(additions);
    }

    /// Career totals across the rendered History rows. Uses the same
    /// minutes-weighted ledger as [`PlayerStatistics::merge_from`] so
    /// the average rating cell is the weighted blend, not a flat mean.
    pub fn player_history_totals(rows: &[PlayerHistoryRow]) -> PlayerStatistics {
        let mut total = PlayerStatistics::default();
        for row in rows {
            total.merge_from(&row.statistics);
        }
        total
    }

    /// Project the ledger into per-row competition breakdowns for the
    /// History page accordion. The output is keyed identically to
    /// [`Self::player_history_rows`] — `(season_start_year, team_slug,
    /// league_slug)` — so the renderer can pair each main row with its
    /// breakdown.
    ///
    /// Each breakdown aggregates ledger entries by
    /// `(competition_kind, competition_slug)`, so per-cup tournaments
    /// (Champions League vs Europa League, FA Cup vs League Cup)
    /// surface as separate lines. The `competition_slug` is preserved
    /// on the output so the renderer can resolve a human-readable
    /// name from SimulatorData.
    ///
    /// Loan flag is NOT part of the key: cup / friendly entries don't
    /// carry the loan flag (a match is a match), so all match records
    /// for a given `(year, team, league)` fold into one breakdown
    /// regardless of which contract they were played under. The
    /// breakdown's `is_loan` mirrors the row's `is_loan`, derived from
    /// the latest League entry in the group.
    ///
    /// Lines with zero appearances are dropped except for the League
    /// row, which is synthesised at 0 apps when missing so every row
    /// surfaces SOMETHING in the accordion. Kinds are sorted League →
    /// ContinentalCup → DomesticCup → Friendly; within a kind, lines
    /// keep their first-seen insertion order.
    pub fn player_history_breakdowns(
        history: &PlayerStatisticsHistory,
        live: &PlayerLiveStatsInput<'_>,
        current_date: NaiveDate,
    ) -> Vec<PlayerHistoryRowBreakdown> {
        let ledger = Self::build_ledger(history, live, None, current_date);

        let mut breakdowns: HashMap<RowKey, PlayerHistoryRowBreakdown> = HashMap::new();
        let mut order: Vec<RowKey> = Vec::new();
        // Mirror the row's "latest League entry wins" rule for the
        // breakdown's is_loan flag, so the row and its breakdown stay
        // in lock-step on the loan label.
        let mut latest_league_seq: HashMap<RowKey, u32> = HashMap::new();

        for entry in ledger {
            let key = RowKey::from_entry(&entry);
            let breakdown = breakdowns.entry(key.clone()).or_insert_with(|| {
                order.push(key.clone());
                PlayerHistoryRowBreakdown {
                    season_start_year: entry.season_start_year,
                    team_slug: entry.team_slug.clone(),
                    league_slug: entry.league_slug.clone(),
                    is_loan: false,
                    competitions: Vec::new(),
                }
            });
            if entry.competition_kind == PlayerStatCompetitionKind::League {
                let is_new_latest = latest_league_seq
                    .get(&key)
                    .is_none_or(|&prev| entry.seq_id >= prev);
                if is_new_latest {
                    latest_league_seq.insert(key.clone(), entry.seq_id);
                    breakdown.is_loan = entry.is_loan;
                }
            }
            if let Some(row) = breakdown.competitions.iter_mut().find(|r| {
                r.competition_kind == entry.competition_kind
                    && r.competition_slug == entry.competition_slug
            }) {
                row.statistics.merge_from(&entry.statistics);
            } else {
                breakdown.competitions.push(PlayerCompetitionStatsRow {
                    competition_kind: entry.competition_kind,
                    competition_slug: entry.competition_slug.clone(),
                    competition_name: String::new(),
                    statistics: entry.statistics,
                });
            }
        }

        let kind_order = |k: PlayerStatCompetitionKind| match k {
            PlayerStatCompetitionKind::League => 0,
            PlayerStatCompetitionKind::ContinentalCup => 1,
            PlayerStatCompetitionKind::DomesticCup => 2,
            PlayerStatCompetitionKind::Friendly => 3,
        };
        for breakdown in breakdowns.values_mut() {
            breakdown.competitions.retain(|c| {
                c.competition_kind == PlayerStatCompetitionKind::League
                    || c.statistics.total_games() > 0
            });
            if !breakdown
                .competitions
                .iter()
                .any(|c| c.competition_kind == PlayerStatCompetitionKind::League)
            {
                breakdown.competitions.push(PlayerCompetitionStatsRow {
                    competition_kind: PlayerStatCompetitionKind::League,
                    competition_slug: breakdown.league_slug.clone(),
                    competition_name: String::new(),
                    statistics: PlayerStatistics::default(),
                });
            }
            // Stable kind order, preserve insertion order within a kind
            // so two continental cups appear in the order they were
            // first written to the ledger.
            breakdown
                .competitions
                .sort_by_key(|c| kind_order(c.competition_kind));
        }

        order
            .into_iter()
            .filter_map(|key| breakdowns.remove(&key))
            .collect()
    }

    /// Display name for an Overview cup row, given its slug. The
    /// projection prefers a name supplied by the caller (live slice or
    /// domestic-cup override) over slug echoing.
    fn resolve_cup_name(
        slug: &str,
        live: &PlayerLiveStatsInput<'_>,
        domestic_cup: Option<&DomesticCupOverride>,
    ) -> String {
        if let Some(dc) = domestic_cup {
            if dc.competition_slug == slug {
                return dc.competition_name.clone();
            }
        }
        for slice in live.cups {
            if slice.competition_slug == slug {
                return slice.competition_name.clone();
            }
        }
        // Last-resort fallback: a ledger entry from a previous spell
        // whose cup isn't part of the current club's live data and
        // isn't this country's configured domestic cup. Without a
        // matching name source, present the slug in Title Case so the
        // page reads "Copa Paraguay" instead of leaking the raw
        // kebab-case "copa-paraguay" identifier.
        titlecase_slug(slug)
    }
}

/// Convert a kebab-case competition slug into a presentable Title Case
/// display string. Pure string transformation — no locale awareness —
/// since this only runs when every other name source has missed and
/// we'd otherwise leak the raw slug to the page.
fn titlecase_slug(slug: &str) -> String {
    if slug.is_empty() {
        return String::new();
    }
    slug.split('-')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => {
                    let rest: String = chars.collect();
                    format!("{}{}", first.to_uppercase(), rest)
                }
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn league_anchor_for(
    history: &PlayerStatisticsHistory,
    season_year: u16,
    team_slug: &str,
) -> Option<(u32, String, String)> {
    history
        .items
        .iter()
        .filter(|item| item.season.start_year == season_year && item.team_slug == team_slug)
        .max_by_key(|item| item.seq_id)
        .map(|item| {
            (
                item.seq_id,
                item.league_slug.clone(),
                item.league_name.clone(),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::club::player::statistics::history::{
        ContinentalSeasonStats, CurrentSeasonEntry, PlayerStatisticsHistoryItem,
    };
    use crate::club::player::statistics::ledger::LiveCupSlice;
    use crate::club::player::statistics::types::TeamInfo;
    use crate::continent::competitions::CHAMPIONS_LEAGUE_SLUG;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn stats(played: u16, goals: u16) -> PlayerStatistics {
        let mut s = PlayerStatistics::default();
        s.played = played;
        s.goals = goals;
        s
    }

    fn stats_with_subs(played: u16, played_subs: u16, goals: u16) -> PlayerStatistics {
        let mut s = stats(played, goals);
        s.played_subs = played_subs;
        s
    }

    fn frozen(year: u16, slug: &str, played: u16, goals: u16) -> PlayerStatisticsHistoryItem {
        PlayerStatisticsHistoryItem {
            season: Season::new(year),
            team_name: slug.to_string(),
            team_slug: slug.to_string(),
            team_reputation: 5_000,
            league_name: "League".to_string(),
            league_slug: "league".to_string(),
            is_loan: false,
            transfer_fee: None,
            statistics: stats(played, goals),
            seq_id: year as u32,
        }
    }

    fn frozen_in_league(
        year: u16,
        slug: &str,
        league_slug: &str,
        played: u16,
        goals: u16,
        seq_id: u32,
    ) -> PlayerStatisticsHistoryItem {
        PlayerStatisticsHistoryItem {
            season: Season::new(year),
            team_name: slug.to_string(),
            team_slug: slug.to_string(),
            team_reputation: 5_000,
            league_name: league_slug.to_string(),
            league_slug: league_slug.to_string(),
            is_loan: false,
            transfer_fee: None,
            statistics: stats(played, goals),
            seq_id,
        }
    }

    fn current_entry(
        slug: &str,
        joined: NaiveDate,
        departed: Option<NaiveDate>,
    ) -> CurrentSeasonEntry {
        CurrentSeasonEntry {
            team_name: slug.to_string(),
            team_slug: slug.to_string(),
            team_reputation: 5_000,
            league_name: "League".to_string(),
            league_slug: "league".to_string(),
            is_loan: false,
            transfer_fee: None,
            statistics: PlayerStatistics::default(),
            joined_date: joined,
            departed_date: departed,
            seq_id: 100,
        }
    }

    fn empty_live<'a>(empty: &'a PlayerStatistics) -> PlayerLiveStatsInput<'a> {
        PlayerLiveStatsInput {
            league: empty,
            friendly: empty,
            cups: &[],
            friendly_source_slug: "",
        }
    }

    fn _team(slug: &str) -> TeamInfo {
        TeamInfo {
            name: slug.to_string(),
            slug: slug.to_string(),
            reputation: 5_000,
            league_name: "League".to_string(),
            league_slug: "league".to_string(),
        }
    }

    #[test]
    fn overview_filters_to_current_season_only() {
        let hist = PlayerStatisticsHistory::from_items(vec![
            frozen(2024, "juventus", 30, 8),
            frozen(2025, "juventus", 28, 6),
        ]);
        let empty_stats = PlayerStatistics::default();
        let live = empty_live(&empty_stats);
        let rows = PlayerStatisticsProjection::player_overview_statistics(
            &hist,
            &live,
            None,
            d(2027, 1, 15),
        );
        // No current-season ledger entry → no League row. Past
        // frozen rows must never bleed into the Overview.
        assert!(rows.is_empty(), "Overview must filter to current season");
    }

    #[test]
    fn overview_aggregates_current_season_league_and_continental() {
        let mut hist = PlayerStatisticsHistory::new();
        hist.current
            .push(current_entry("juventus", d(2026, 8, 1), None));

        let live_league = stats(20, 5);
        let live_friendly = PlayerStatistics::default();
        let live_continental = stats(7, 3);
        let cups = vec![LiveCupSlice {
            competition_slug: CHAMPIONS_LEAGUE_SLUG,
            competition_name: "Champions League".to_string(),
            statistics: &live_continental,
        }];
        let live = PlayerLiveStatsInput {
            league: &live_league,
            friendly: &live_friendly,
            cups: &cups,
            friendly_source_slug: "",
        };

        let rows = PlayerStatisticsProjection::player_overview_statistics(
            &hist,
            &live,
            None,
            d(2026, 10, 1),
        );

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].competition_kind, PlayerStatCompetitionKind::League);
        assert_eq!(rows[0].statistics.played, 20);
        assert_eq!(rows[0].statistics.goals, 5);
        assert_eq!(
            rows[1].competition_kind,
            PlayerStatCompetitionKind::ContinentalCup
        );
        assert_eq!(rows[1].statistics.played, 7);
        assert_eq!(rows[1].statistics.goals, 3);
    }

    #[test]
    fn overview_domestic_cup_override_replaces_live_slice() {
        let mut hist = PlayerStatisticsHistory::new();
        hist.current
            .push(current_entry("juventus", d(2026, 8, 1), None));

        let live_league = PlayerStatistics::default();
        let live_friendly = PlayerStatistics::default();
        // Live per-spell domestic cup (drained mid-season → unreliable).
        let live_domestic = stats(2, 0);
        let cups = vec![LiveCupSlice {
            competition_slug: "coppa-italia",
            competition_name: "Coppa Italia".to_string(),
            statistics: &live_domestic,
        }];
        let live = PlayerLiveStatsInput {
            league: &live_league,
            friendly: &live_friendly,
            cups: &cups,
            friendly_source_slug: "",
        };
        let override_stats = stats(5, 1);
        let dc = DomesticCupOverride {
            competition_slug: "coppa-italia".to_string(),
            competition_name: "Coppa Italia".to_string(),
            statistics: override_stats,
        };

        let rows = PlayerStatisticsProjection::player_overview_statistics(
            &hist,
            &live,
            Some(&dc),
            d(2026, 10, 1),
        );

        let cup_row = rows
            .iter()
            .find(|r| r.competition_kind == PlayerStatCompetitionKind::DomesticCup)
            .expect("expected a domestic cup row");
        assert_eq!(
            cup_row.statistics.played, 5,
            "override (records source) must beat the live per-spell slice"
        );
        assert_eq!(cup_row.statistics.goals, 1);
        // And it must not also appear under the live slice's slug.
        assert_eq!(
            rows.iter()
                .filter(|r| r.competition_slug == "coppa-italia")
                .count(),
            1
        );
    }

    #[test]
    fn history_groups_seasons_by_team_and_loan_flag() {
        let hist = PlayerStatisticsHistory::from_items(vec![
            frozen(2024, "juventus", 30, 8),
            frozen(2025, "juventus", 28, 6),
        ]);
        let empty_stats = PlayerStatistics::default();
        let live = empty_live(&empty_stats);
        let rows = PlayerStatisticsProjection::player_history_rows(&hist, &live, d(2026, 9, 1));
        // Sorted desc by season — most recent first.
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].season.start_year, 2025);
        assert_eq!(rows[1].season.start_year, 2024);
    }

    #[test]
    fn loan_sorts_above_reserve_home_row_within_season() {
        // User-reported repro (Ruslan Pichienko): a reserve-home player's
        // "Spartak Moscow 2" parent row rendered ABOVE the "Dinamo
        // Vladivostok" loan that chronologically came first in the same
        // 2026/27 season. Root cause: after the Vladivostok loan returned,
        // the player landed on the Main team and
        // `move_loan_returns_to_reserve` opened a FRESH reserve spell, so
        // the season-long home row took a seq_id (30) higher than the loan
        // it contained (20). Pure seq-desc ordering then floats the home
        // above the loan. Main-home players never get re-placed to the
        // reserve, which is why "it works for Main teams".
        let mut hist = PlayerStatisticsHistory::new();
        // Frozen 2026/27: the earlier loan (low seq) + the inflated home row.
        hist.season_ledger.push(PlayerStatLedgerEntry {
            seq_id: 20,
            season_start_year: 2026,
            team_slug: "dinamo-vladivostok".to_string(),
            team_name: "Dinamo Vladivostok".to_string(),
            team_reputation: 100,
            league_slug: "second-division-a-silver".to_string(),
            league_name: "Second Division A Silver".to_string(),
            competition_kind: PlayerStatCompetitionKind::League,
            competition_slug: "second-division-a-silver".to_string(),
            is_loan: true,
            transfer_fee: Some(0.0),
            statistics: PlayerStatistics::default(),
        });
        hist.season_ledger.push(PlayerStatLedgerEntry {
            // Inflated by the post-loan-return reserve re-placement.
            seq_id: 30,
            season_start_year: 2026,
            team_slug: "spartak-moscow-2".to_string(),
            team_name: "Spartak Moscow 2".to_string(),
            team_reputation: 200,
            league_slug: "second-division-b2".to_string(),
            league_name: "Second Division B2".to_string(),
            competition_kind: PlayerStatCompetitionKind::League,
            competition_slug: "second-division-b2".to_string(),
            is_loan: false,
            transfer_fee: None,
            statistics: PlayerStatistics::default(),
        });
        // Active 2027/28 spell on a fresh loan.
        hist.current.push(CurrentSeasonEntry {
            team_name: "Dinamo Vologda".to_string(),
            team_slug: "dinamo-vologda".to_string(),
            team_reputation: 100,
            league_name: "Second Division B2".to_string(),
            league_slug: "second-division-b2".to_string(),
            is_loan: true,
            transfer_fee: Some(0.0),
            statistics: PlayerStatistics::default(),
            joined_date: d(2027, 7, 7),
            departed_date: None,
            seq_id: 40,
        });

        let live_league = stats(3, 0);
        let live_friendly = PlayerStatistics::default();
        let live = PlayerLiveStatsInput {
            league: &live_league,
            friendly: &live_friendly,
            cups: &[],
            friendly_source_slug: "",
        };

        let rows = PlayerStatisticsProjection::player_history_rows(&hist, &live, d(2027, 10, 1));

        let pos = |slug: &str, year: u16| {
            rows.iter()
                .position(|r| r.team_slug == slug && r.season.start_year == year)
        };
        let vladivostok = pos("dinamo-vladivostok", 2026).expect("Vladivostok loan row missing");
        let home = pos("spartak-moscow-2", 2026).expect("Spartak Moscow 2 home row missing");
        assert!(
            vladivostok < home,
            "loan (Dinamo Vladivostok) must sort above the parent home row \
             (Spartak Moscow 2) in the same season; got vladivostok={vladivostok}, home={home}"
        );

        // Sanity: the active current-season spell stays on top overall.
        assert_eq!(
            rows.first().map(|r| r.team_slug.as_str()),
            Some("dinamo-vologda"),
            "active current-season spell stays at the top"
        );
    }

    #[test]
    fn history_folds_continental_into_league_row_exactly_once() {
        let mut hist = PlayerStatisticsHistory::from_items(vec![frozen(2024, "juventus", 30, 8)]);
        hist.continental.push(ContinentalSeasonStats {
            season_year: 2024,
            team_slug: "juventus".to_string(),
            statistics: stats(10, 5),
        });
        let empty_stats = PlayerStatistics::default();
        let live = empty_live(&empty_stats);
        let rows = PlayerStatisticsProjection::player_history_rows(&hist, &live, d(2026, 9, 1));
        let row = rows
            .iter()
            .find(|r| r.season.start_year == 2024 && r.team_slug == "juventus")
            .expect("league row missing");
        assert_eq!(row.statistics.played, 40, "30 league + 10 continental");
        assert_eq!(row.statistics.goals, 13, "8 league + 5 continental");
    }

    #[test]
    fn continental_synthetic_seq_does_not_become_latest_row() {
        let mut old = frozen(2024, "juventus", 20, 3);
        old.statistics = stats_with_subs(20, 4, 3);
        old.seq_id = 1;
        let mut latest = frozen(2025, "juventus", 10, 2);
        latest.seq_id = 2;
        let mut hist = PlayerStatisticsHistory::from_items(vec![old, latest]);
        hist.continental.push(ContinentalSeasonStats {
            season_year: 2024,
            team_slug: "juventus".to_string(),
            statistics: stats(5, 1),
        });

        let empty_stats = PlayerStatistics::default();
        let live = empty_live(&empty_stats);
        let rows = PlayerStatisticsProjection::player_history_rows(&hist, &live, d(2026, 9, 1));
        let old_row = rows
            .iter()
            .find(|r| r.season.start_year == 2024)
            .expect("old row missing");
        assert_eq!(
            old_row.seq_id, 1,
            "continental rows must keep the real season seq"
        );
        assert_eq!(
            old_row.statistics.played, 29,
            "20 starts + 4 subs rolled into played + 5 continental"
        );
        assert_eq!(
            old_row.statistics.played_subs, 0,
            "older rows must not keep subs because a synthetic cup seq won"
        );
    }

    #[test]
    fn history_active_row_uses_live_not_frozen_continental_ledger() {
        let mut hist = PlayerStatisticsHistory::from_items(vec![frozen(2024, "juventus", 30, 8)]);
        // Frozen continental belongs to 2024; the active spell sits in
        // the current season and must use the live cup slice instead.
        hist.continental.push(ContinentalSeasonStats {
            season_year: 2024,
            team_slug: "juventus".to_string(),
            statistics: stats(8, 3),
        });
        hist.current
            .push(current_entry("juventus", d(2025, 8, 1), None));

        let live_league = stats(20, 4);
        let live_friendly = PlayerStatistics::default();
        let live_continental = stats(7, 3);
        let cups = vec![LiveCupSlice {
            competition_slug: CHAMPIONS_LEAGUE_SLUG,
            competition_name: "Champions League".to_string(),
            statistics: &live_continental,
        }];
        let live = PlayerLiveStatsInput {
            league: &live_league,
            friendly: &live_friendly,
            cups: &cups,
            friendly_source_slug: "",
        };

        let rows = PlayerStatisticsProjection::player_history_rows(&hist, &live, d(2025, 10, 1));

        let active = rows
            .iter()
            .find(|r| r.season.start_year == 2025 && r.team_slug == "juventus")
            .expect("active row missing");
        assert_eq!(
            active.statistics.played, 27,
            "20 live league + 7 live cup, NOT plus the 2024 ledger"
        );
        assert_eq!(active.statistics.goals, 7);

        let past = rows
            .iter()
            .find(|r| r.season.start_year == 2024 && r.team_slug == "juventus")
            .expect("past row missing");
        assert_eq!(
            past.statistics.played, 38,
            "30 league + 8 frozen continental on the past row"
        );
    }

    #[test]
    fn active_current_row_uses_live_not_stored_snapshot() {
        // Required regression #1: the active spell's stats come from the
        // live League counter alone. The snapshot stored on the active
        // entry is for the same spell (or a stale duplicate) — merging it
        // with live would double-count. Here snapshot==live==6, and the
        // row must show 6, never 12.
        let mut hist = PlayerStatisticsHistory::new();
        let mut entry = current_entry("juventus", d(2026, 8, 1), None);
        entry.statistics = stats(6, 1);
        hist.current.push(entry);

        let live_league = stats(6, 1);
        let live_friendly = PlayerStatistics::default();
        let live = PlayerLiveStatsInput {
            league: &live_league,
            friendly: &live_friendly,
            cups: &[],
            friendly_source_slug: "",
        };

        let overview = PlayerStatisticsProjection::player_overview_statistics(
            &hist,
            &live,
            None,
            d(2026, 10, 1),
        );
        let league = overview
            .iter()
            .find(|r| r.competition_kind == PlayerStatCompetitionKind::League)
            .expect("league overview row missing");
        assert_eq!(league.statistics.played, 6, "active spell must not double-count");
        assert_eq!(league.statistics.goals, 1);

        let history = PlayerStatisticsProjection::player_history_rows(&hist, &live, d(2026, 10, 1));
        let row = history
            .iter()
            .find(|r| r.season.start_year == 2026 && r.team_slug == "juventus")
            .expect("history row missing");
        assert_eq!(row.statistics.played, 6, "active spell must not double-count");
        assert_eq!(row.statistics.goals, 1);
    }

    #[test]
    fn departed_and_active_same_season_spells_aggregate() {
        // Required regression #2: a departed spell at the same club this
        // season keeps its stored snapshot (4 apps); the active spell
        // contributes the live counter (6 apps). Grouped by
        // (season, team, league, is_loan) the history row shows 10, with
        // the live counter applied exactly once.
        let mut hist = PlayerStatisticsHistory::new();
        // Earlier spell at Juventus, drained and marked departed.
        hist.current.push(CurrentSeasonEntry {
            team_name: "juventus".to_string(),
            team_slug: "juventus".to_string(),
            team_reputation: 5_000,
            league_name: "League".to_string(),
            league_slug: "league".to_string(),
            is_loan: false,
            transfer_fee: None,
            statistics: stats(4, 1),
            joined_date: d(2026, 8, 1),
            departed_date: Some(d(2026, 11, 1)),
            seq_id: 1,
        });
        // Fresh active spell back at Juventus, snapshot empty.
        hist.current.push(CurrentSeasonEntry {
            team_name: "juventus".to_string(),
            team_slug: "juventus".to_string(),
            team_reputation: 5_000,
            league_name: "League".to_string(),
            league_slug: "league".to_string(),
            is_loan: false,
            transfer_fee: None,
            statistics: PlayerStatistics::default(),
            joined_date: d(2027, 1, 1),
            departed_date: None,
            seq_id: 2,
        });

        let live_league = stats(6, 2);
        let live_friendly = PlayerStatistics::default();
        let live = PlayerLiveStatsInput {
            league: &live_league,
            friendly: &live_friendly,
            cups: &[],
            friendly_source_slug: "",
        };

        let rows = PlayerStatisticsProjection::player_history_rows(&hist, &live, d(2027, 2, 1));
        let juve: Vec<_> = rows
            .iter()
            .filter(|r| r.season.start_year == 2026 && r.team_slug == "juventus")
            .collect();
        assert_eq!(juve.len(), 1, "same-season same-club spells must group into one row");
        assert_eq!(juve[0].statistics.played, 10, "4 departed + 6 live");
        assert_eq!(juve[0].statistics.goals, 3);
    }

    #[test]
    fn active_row_season_label_follows_current_date_not_stale_joined() {
        // Required regression #3: the active row is labelled with the
        // season containing current_date even when its `joined_date` is
        // stuck on an earlier season (delayed season-end snapshot). The
        // active spell's live stats must land on today's season row and
        // must not be attributed to the stale `joined_date` season.
        let mut hist = PlayerStatisticsHistory::new();
        hist.current.push(CurrentSeasonEntry {
            team_name: "spartak".to_string(),
            team_slug: "spartak".to_string(),
            team_reputation: 5_000,
            league_name: "League".to_string(),
            league_slug: "league".to_string(),
            is_loan: false,
            transfer_fee: None,
            statistics: PlayerStatistics::default(),
            // Stale: seeded for 2026/27 but never re-seeded since.
            joined_date: d(2026, 8, 1),
            departed_date: None,
            seq_id: 50,
        });

        let live_league = stats(18, 4);
        let live_friendly = PlayerStatistics::default();
        let live = PlayerLiveStatsInput {
            league: &live_league,
            friendly: &live_friendly,
            cups: &[],
            friendly_source_slug: "",
        };

        // Game date is well into 2027/28.
        let rows = PlayerStatisticsProjection::player_history_rows(&hist, &live, d(2028, 3, 1));
        let spartak: Vec<_> = rows.iter().filter(|r| r.team_slug == "spartak").collect();
        // Exactly one spartak row, under today's season, with the live stats.
        assert_eq!(spartak.len(), 1, "stale joined_date must not split the active spell");
        assert_eq!(
            spartak[0].season.start_year, 2027,
            "active row must use the season containing current_date"
        );
        assert_eq!(spartak[0].statistics.played, 18);
        assert_eq!(spartak[0].statistics.goals, 4);
    }

    #[test]
    fn history_shows_just_frozen_season_in_sub_august_window() {
        // Sub-August league: the season-end snapshot froze 2025/26 (a loan
        // spell) on a July regen and re-seeded the new season. But
        // `Season::from_date(July 2026)` still reports 2025/26 as "current",
        // so the just-frozen row must NOT be hidden until the calendar
        // crosses the next August boundary.
        let mut hist = PlayerStatisticsHistory::new();
        hist.season_ledger.push(PlayerStatLedgerEntry {
            seq_id: 1,
            season_start_year: 2025,
            team_slug: "borrowing".to_string(),
            team_name: "Borrowing".to_string(),
            team_reputation: 100,
            league_slug: "league-b".to_string(),
            league_name: "League B".to_string(),
            competition_kind: PlayerStatCompetitionKind::League,
            competition_slug: "league-b".to_string(),
            is_loan: true,
            transfer_fee: Some(0.0),
            statistics: stats(20, 5),
        });
        // Re-seeded active spell for the new season (joined Aug 2026 by the
        // season-end re-seed convention).
        hist.current.push(CurrentSeasonEntry {
            team_name: "Parent".to_string(),
            team_slug: "parent".to_string(),
            team_reputation: 200,
            league_name: "League A".to_string(),
            league_slug: "league-a".to_string(),
            is_loan: false,
            transfer_fee: None,
            statistics: PlayerStatistics::default(),
            joined_date: d(2026, 8, 1),
            departed_date: None,
            seq_id: 2,
        });

        let empty = PlayerStatistics::default();
        let live = empty_live(&empty);
        // Render in the Jan–Jul window: from_date(July 2026) == 2025.
        let rows = PlayerStatisticsProjection::player_history_rows(&hist, &live, d(2026, 7, 1));
        let loan = rows
            .iter()
            .find(|r| r.team_slug == "borrowing" && r.season.start_year == 2025)
            .expect("just-frozen loan row must not be hidden in the sub-August window");
        assert_eq!(loan.statistics.played, 20);
        assert!(loan.is_loan);
    }

    #[test]
    fn history_shows_just_frozen_loan_when_player_already_returned() {
        // Same league shape, but the loan already returned and the only
        // active spell's `joined_date` is itself a Jan–Jul date — so a naive
        // `max(today, joined)` would still lag. The frozen-season inference
        // (last completed League season + 1) must still surface the 2025/26
        // loan row.
        let mut hist = PlayerStatisticsHistory::new();
        hist.season_ledger.push(PlayerStatLedgerEntry {
            seq_id: 1,
            season_start_year: 2025,
            team_slug: "borrowing".to_string(),
            team_name: "Borrowing".to_string(),
            team_reputation: 100,
            league_slug: "league-b".to_string(),
            league_name: "League B".to_string(),
            competition_kind: PlayerStatCompetitionKind::League,
            competition_slug: "league-b".to_string(),
            is_loan: true,
            transfer_fee: Some(0.0),
            statistics: stats(18, 2),
        });
        hist.current.push(CurrentSeasonEntry {
            team_name: "Parent".to_string(),
            team_slug: "parent".to_string(),
            team_reputation: 200,
            league_name: "League A".to_string(),
            league_slug: "league-a".to_string(),
            is_loan: false,
            transfer_fee: None,
            statistics: PlayerStatistics::default(),
            joined_date: d(2026, 6, 15),
            departed_date: None,
            seq_id: 2,
        });

        let empty = PlayerStatistics::default();
        let live = empty_live(&empty);
        let rows = PlayerStatisticsProjection::player_history_rows(&hist, &live, d(2026, 7, 1));
        let loan = rows
            .iter()
            .find(|r| r.team_slug == "borrowing" && r.season.start_year == 2025)
            .expect("returned-loan row must still show in the sub-August window");
        assert_eq!(loan.statistics.played, 18);
    }

    #[test]
    fn history_includes_domestic_cup_but_excludes_friendly() {
        let mut hist = PlayerStatisticsHistory::new();
        hist.current
            .push(current_entry("juventus", d(2026, 8, 1), None));
        let live_league = stats(10, 2);
        let live_friendly = stats(3, 1);
        let live_domestic = stats(4, 1);
        let cups = vec![LiveCupSlice {
            competition_slug: "coppa-italia",
            competition_name: "Coppa Italia".to_string(),
            statistics: &live_domestic,
        }];
        let live = PlayerLiveStatsInput {
            league: &live_league,
            friendly: &live_friendly,
            cups: &cups,
            friendly_source_slug: "",
        };

        let rows = PlayerStatisticsProjection::player_history_rows(&hist, &live, d(2026, 10, 1));
        assert_eq!(rows.len(), 1);
        // League + domestic cup; friendly remains overview-only.
        assert_eq!(rows[0].statistics.played, 14);
        assert_eq!(rows[0].statistics.goals, 3);
    }

    #[test]
    fn history_keeps_same_team_same_season_split_by_league() {
        let hist = PlayerStatisticsHistory::from_items(vec![
            frozen_in_league(2026, "spartak", "premier-league", 12, 0, 1),
            frozen_in_league(2026, "spartak", "first-league", 7, 0, 2),
        ]);
        let empty_stats = PlayerStatistics::default();
        let live = empty_live(&empty_stats);

        let rows = PlayerStatisticsProjection::player_history_rows(&hist, &live, d(2027, 9, 1));
        let spartak_rows: Vec<_> = rows
            .iter()
            .filter(|r| r.season.start_year == 2026 && r.team_slug == "spartak")
            .collect();
        assert_eq!(
            spartak_rows.len(),
            2,
            "same team/season rows from different leagues must not be merged"
        );
        assert!(
            spartak_rows
                .iter()
                .any(|r| r.league_slug == "premier-league")
        );
        assert!(spartak_rows.iter().any(|r| r.league_slug == "first-league"));
    }

    #[test]
    fn totals_equal_sum_of_rendered_rows() {
        let hist = PlayerStatisticsHistory::from_items(vec![
            frozen(2024, "juventus", 30, 8),
            frozen(2025, "juventus", 28, 6),
            frozen(2025, "roma", 0, 0), // would be dropped as phantom
        ]);
        let empty_stats = PlayerStatistics::default();
        let live = empty_live(&empty_stats);
        let rows = PlayerStatisticsProjection::player_history_rows(&hist, &live, d(2026, 9, 1));
        let totals = PlayerStatisticsProjection::player_history_totals(&rows);
        let summed: u16 = rows.iter().map(|r| r.statistics.played).sum();
        let summed_goals: u16 = rows.iter().map(|r| r.statistics.goals).sum();
        assert_eq!(totals.played, summed);
        assert_eq!(totals.goals, summed_goals);
    }

    #[test]
    fn history_breakdown_groups_competitions_per_row() {
        // Active spell at juventus 2026/27 with live league + friendly +
        // continental + domestic cup. Past 2025/26 row at juventus has
        // frozen league + frozen continental ledger entries. The
        // breakdown must surface one row per season, each with at most
        // one line per competition kind, sorted League → Continental
        // → DomesticCup → Friendly, with zero-game lines dropped.
        let team_info = TeamInfo {
            name: "Juventus".to_string(),
            slug: "juventus".to_string(),
            reputation: 5_000,
            league_name: "Serie A".to_string(),
            league_slug: "serie-a".to_string(),
        };
        let mut hist = PlayerStatisticsHistory::new();
        // Past season: League + Continental written to the canonical ledger.
        hist.append_to_ledger(
            2025,
            &team_info,
            PlayerStatCompetitionKind::League,
            false,
            None,
            stats(30, 8),
        );
        hist.append_to_ledger(
            2025,
            &team_info,
            PlayerStatCompetitionKind::ContinentalCup,
            false,
            None,
            stats(10, 5),
        );
        hist.append_to_ledger(
            2025,
            &team_info,
            PlayerStatCompetitionKind::DomesticCup,
            false,
            None,
            stats(4, 1),
        );
        hist.append_to_ledger(
            2025,
            &team_info,
            PlayerStatCompetitionKind::Friendly,
            false,
            None,
            stats(3, 0),
        );
        // Active 2026/27 spell — same club, fresh entry.
        hist.current
            .push(current_entry("juventus", d(2026, 8, 1), None));

        let live_league = stats(12, 2);
        let live_friendly = stats(2, 1);
        let live_continental = stats(5, 3);
        let cups = vec![LiveCupSlice {
            competition_slug: CHAMPIONS_LEAGUE_SLUG,
            competition_name: "Champions League".to_string(),
            statistics: &live_continental,
        }];
        let live = PlayerLiveStatsInput {
            league: &live_league,
            friendly: &live_friendly,
            cups: &cups,
            friendly_source_slug: "",
        };

        let breakdowns = PlayerStatisticsProjection::player_history_breakdowns(
            &hist,
            &live,
            d(2026, 10, 1),
        );

        // Two breakdowns: one per season at juventus.
        let past = breakdowns
            .iter()
            .find(|b| b.season_start_year == 2025 && b.team_slug == "juventus")
            .expect("past breakdown missing");
        let kinds: Vec<PlayerStatCompetitionKind> =
            past.competitions.iter().map(|c| c.competition_kind).collect();
        assert_eq!(
            kinds,
            vec![
                PlayerStatCompetitionKind::League,
                PlayerStatCompetitionKind::ContinentalCup,
                PlayerStatCompetitionKind::DomesticCup,
                PlayerStatCompetitionKind::Friendly,
            ],
            "past row breakdown order"
        );
        assert_eq!(past.competitions[0].statistics.played, 30);
        assert_eq!(past.competitions[1].statistics.played, 10);
        assert_eq!(past.competitions[2].statistics.played, 4);
        assert_eq!(past.competitions[3].statistics.played, 3);

        let active = breakdowns
            .iter()
            .find(|b| b.season_start_year == 2026 && b.team_slug == "juventus")
            .expect("active breakdown missing");
        let active_kinds: Vec<PlayerStatCompetitionKind> = active
            .competitions
            .iter()
            .map(|c| c.competition_kind)
            .collect();
        assert!(
            active_kinds.contains(&PlayerStatCompetitionKind::League),
            "active league line"
        );
        assert!(
            active_kinds.contains(&PlayerStatCompetitionKind::ContinentalCup),
            "active continental line"
        );
        assert!(
            active_kinds.contains(&PlayerStatCompetitionKind::Friendly),
            "active friendly line"
        );
    }

    #[test]
    fn history_breakdown_for_loan_spell_includes_live_friendly_and_cups() {
        // User-reported repro: a player is loaned to a senior team, plays
        // only pre-season friendlies, and the History page shows the
        // loan row (0 league apps) but the Friendly games never surface
        // in the breakdown. The active spell's row key is
        // (year, team, league, is_loan=true) but the live friendly /
        // cup entries used to hardcode `is_loan: false` — so they
        // orphaned into a phantom key the renderer never matched.
        let mut hist = PlayerStatisticsHistory::new();
        hist.current.push(CurrentSeasonEntry {
            team_name: "spartak".to_string(),
            team_slug: "spartak".to_string(),
            team_reputation: 5_000,
            league_name: "Premier League".to_string(),
            league_slug: "premier-league".to_string(),
            is_loan: true,
            transfer_fee: Some(0.0),
            statistics: PlayerStatistics::default(),
            joined_date: d(2026, 7, 1),
            departed_date: None,
            seq_id: 50,
        });

        let live_league = PlayerStatistics::default();
        let live_friendly = stats(2, 0);
        let live_cup = stats(1, 0);
        let cups = vec![LiveCupSlice {
            competition_slug: CHAMPIONS_LEAGUE_SLUG,
            competition_name: "Champions League".to_string(),
            statistics: &live_cup,
        }];
        let live = PlayerLiveStatsInput {
            league: &live_league,
            friendly: &live_friendly,
            cups: &cups,
            friendly_source_slug: "",
        };

        let breakdowns =
            PlayerStatisticsProjection::player_history_breakdowns(&hist, &live, d(2026, 8, 15));
        let loan = breakdowns
            .iter()
            .find(|b| {
                b.season_start_year == 2026
                    && b.team_slug == "spartak"
                    && b.league_slug == "premier-league"
                    && b.is_loan
            })
            .expect("loan-row breakdown must exist under is_loan=true");
        let kinds: Vec<PlayerStatCompetitionKind> =
            loan.competitions.iter().map(|c| c.competition_kind).collect();
        assert!(
            kinds.contains(&PlayerStatCompetitionKind::Friendly),
            "loan-row breakdown must include the live friendly line, got: {:?}",
            kinds
        );
        assert!(
            kinds.contains(&PlayerStatCompetitionKind::ContinentalCup),
            "loan-row breakdown must include the live cup line, got: {:?}",
            kinds
        );
        // No phantom non-loan breakdown should hold the live entries.
        assert!(
            !breakdowns
                .iter()
                .any(|b| b.season_start_year == 2026 && b.team_slug == "spartak" && !b.is_loan),
            "live entries must not orphan into a separate is_loan=false breakdown"
        );
    }

    #[test]
    fn history_breakdown_drops_zero_app_competitions() {
        // A row with only a League entry must not produce a Continental
        // / Domestic / Friendly stub.
        let team_info = TeamInfo {
            name: "Juventus".to_string(),
            slug: "juventus".to_string(),
            reputation: 5_000,
            league_name: "Serie A".to_string(),
            league_slug: "serie-a".to_string(),
        };
        let mut hist = PlayerStatisticsHistory::new();
        hist.append_to_ledger(
            2024,
            &team_info,
            PlayerStatCompetitionKind::League,
            false,
            None,
            stats(28, 6),
        );
        let empty_stats = PlayerStatistics::default();
        let live = empty_live(&empty_stats);

        let breakdowns =
            PlayerStatisticsProjection::player_history_breakdowns(&hist, &live, d(2026, 9, 1));
        let past = breakdowns
            .iter()
            .find(|b| b.season_start_year == 2024)
            .expect("past breakdown missing");
        assert_eq!(past.competitions.len(), 1);
        assert_eq!(
            past.competitions[0].competition_kind,
            PlayerStatCompetitionKind::League
        );
    }

    #[test]
    fn history_breakdown_keeps_loan_cup_friendly_after_cancel_loan() {
        // User-reported repro: a player on loan at Pari plays League +
        // Russia Cup + Premier League U19 friendlies. The user cancels
        // the loan mid-season. After cancel, the Cup and Friendly lines
        // used to disappear from the 2026/27 Pari breakdown — they were
        // frozen with `is_loan=false` (the cup/friendly recorder's
        // hardcoded value) while the row's League entry had
        // `is_loan=true`. The old `(year, team, league, is_loan)` key
        // orphaned the non-League entries into a phantom row.
        //
        // The fix: grouping ignores is_loan. A match is a match; loan
        // status is row metadata, not part of the match record.
        let pari = TeamInfo {
            name: "Pari Nizhniy Novgorod".to_string(),
            slug: "pari".to_string(),
            reputation: 2_000,
            league_name: "Premier League".to_string(),
            league_slug: "russian-premier-league".to_string(),
        };
        let mut hist = PlayerStatisticsHistory::new();
        // Pari LEAGUE entry from the just-departed loan spell — is_loan=true.
        hist.current.push(CurrentSeasonEntry {
            team_name: pari.name.clone(),
            team_slug: pari.slug.clone(),
            team_reputation: pari.reputation,
            league_name: pari.league_name.clone(),
            league_slug: pari.league_slug.clone(),
            is_loan: true,
            transfer_fee: Some(0.0),
            statistics: stats(9, 0),
            joined_date: d(2026, 8, 1),
            departed_date: Some(d(2026, 12, 1)),
            seq_id: 1,
        });
        // Parent club active after cancel.
        hist.current.push(CurrentSeasonEntry {
            team_name: "Spartak Moscow".to_string(),
            team_slug: "spartak".to_string(),
            team_reputation: 5_000,
            league_name: "Premier League".to_string(),
            league_slug: "russian-premier-league".to_string(),
            is_loan: false,
            transfer_fee: None,
            statistics: PlayerStatistics::default(),
            joined_date: d(2026, 12, 1),
            departed_date: None,
            seq_id: 2,
        });
        // Cup + Friendly entries frozen during the cancel-loan drain.
        // The recorder hardcodes is_loan=false; the projection's
        // is_loan-free grouping must surface them under the Pari row
        // anyway.
        hist.record_domestic_cup(2026, &pari, "russia-cup".to_string(), stats(1, 0));
        hist.record_friendly(
            2026,
            &pari,
            "russian-premier-league-u19".to_string(),
            stats(2, 0),
        );

        let live_league = PlayerStatistics::default();
        let live_friendly = PlayerStatistics::default();
        let live = PlayerLiveStatsInput {
            league: &live_league,
            friendly: &live_friendly,
            cups: &[],
            friendly_source_slug: "",
        };

        let rows = PlayerStatisticsProjection::player_history_rows(&hist, &live, d(2026, 12, 15));
        let pari_row = rows
            .iter()
            .find(|r| r.season.start_year == 2026 && r.team_slug == "pari")
            .expect("Pari row missing");
        assert!(pari_row.is_loan, "row label inherits is_loan from League entry");
        assert_eq!(pari_row.statistics.played, 10);

        let breakdowns =
            PlayerStatisticsProjection::player_history_breakdowns(&hist, &live, d(2026, 12, 15));
        let pari_bd = breakdowns
            .iter()
            .find(|b| b.season_start_year == 2026 && b.team_slug == "pari")
            .expect("Pari breakdown missing");
        let kinds: Vec<PlayerStatCompetitionKind> = pari_bd
            .competitions
            .iter()
            .map(|c| c.competition_kind)
            .collect();
        assert!(
            kinds.contains(&PlayerStatCompetitionKind::League),
            "Pari breakdown must include League, got: {:?}",
            kinds
        );
        assert!(
            kinds.contains(&PlayerStatCompetitionKind::DomesticCup),
            "Pari breakdown must include Russia Cup after cancel-loan, got: {:?}",
            kinds
        );
        assert!(
            kinds.contains(&PlayerStatCompetitionKind::Friendly),
            "Pari breakdown must include U19 friendly after cancel-loan, got: {:?}",
            kinds
        );
        assert!(pari_bd.is_loan, "breakdown loan label mirrors the row");
    }

    #[test]
    fn history_breakdown_loads_current_year_non_league_canonical_entries() {
        // User-reported repro: a player transferred mid-season loses
        // their previous club's friendly / cup stats from the History
        // breakdown. on_transfer freezes the source spell via
        // record_friendly_spell / record_continental_spell /
        // record_domestic_cup_spell tagged with the CURRENT season year,
        // then clears the live caches. If the canonical loader skips
        // current-year rows, those frozen entries become invisible —
        // the active spell's live caches are empty (they belong to the
        // new club) and history.current only carries League snapshots.
        let team_a = TeamInfo {
            name: "Club A".to_string(),
            slug: "club-a".to_string(),
            reputation: 5_000,
            league_name: "Premier League".to_string(),
            league_slug: "premier-league".to_string(),
        };
        let team_b = TeamInfo {
            name: "Club B".to_string(),
            slug: "club-b".to_string(),
            reputation: 5_000,
            league_name: "Premier League".to_string(),
            league_slug: "premier-league".to_string(),
        };
        let mut hist = PlayerStatisticsHistory::new();
        // Source-spell League stats survive on a departed current entry.
        hist.current.push(CurrentSeasonEntry {
            team_name: team_a.name.clone(),
            team_slug: team_a.slug.clone(),
            team_reputation: team_a.reputation,
            league_name: team_a.league_name.clone(),
            league_slug: team_a.league_slug.clone(),
            is_loan: false,
            transfer_fee: None,
            statistics: stats(8, 1),
            joined_date: d(2026, 8, 1),
            departed_date: Some(d(2026, 11, 1)),
            seq_id: 1,
        });
        // Destination spell — active.
        hist.current.push(CurrentSeasonEntry {
            team_name: team_b.name.clone(),
            team_slug: team_b.slug.clone(),
            team_reputation: team_b.reputation,
            league_name: team_b.league_name.clone(),
            league_slug: team_b.league_slug.clone(),
            is_loan: false,
            transfer_fee: Some(1_000_000.0),
            statistics: PlayerStatistics::default(),
            joined_date: d(2026, 11, 1),
            departed_date: None,
            seq_id: 2,
        });
        // Source-spell Friendly + Continental frozen mid-season via
        // record_friendly_spell / record_continental_spell with the
        // CURRENT season year (2026).
        hist.record_friendly(
            2026,
            &team_a,
            team_a.league_slug.clone(),
            stats(3, 1),
        );
        hist.record_continental(
            2026,
            &team_a,
            CHAMPIONS_LEAGUE_SLUG.to_string(),
            stats(2, 0),
        );

        // Active spell's live caches reset (fresh at Club B).
        let live_league = PlayerStatistics::default();
        let live_friendly = PlayerStatistics::default();
        let live = PlayerLiveStatsInput {
            league: &live_league,
            friendly: &live_friendly,
            cups: &[],
            friendly_source_slug: "",
        };

        let breakdowns =
            PlayerStatisticsProjection::player_history_breakdowns(&hist, &live, d(2026, 12, 1));

        // Source-spell breakdown must surface both the frozen Friendly
        // and the frozen Continental rows even though they're tagged
        // with the current season year.
        let source = breakdowns
            .iter()
            .find(|b| {
                b.season_start_year == 2026 && b.team_slug == "club-a" && !b.is_loan
            })
            .expect("source-club breakdown missing for current-year transfer");
        let kinds: Vec<PlayerStatCompetitionKind> =
            source.competitions.iter().map(|c| c.competition_kind).collect();
        assert!(
            kinds.contains(&PlayerStatCompetitionKind::Friendly),
            "source-club breakdown must include the frozen Friendly line, got: {:?}",
            kinds
        );
        assert!(
            kinds.contains(&PlayerStatCompetitionKind::ContinentalCup),
            "source-club breakdown must include the frozen Continental line, got: {:?}",
            kinds
        );
    }

    #[test]
    fn history_breakdown_for_freshly_seeded_youth_player_includes_live_friendly() {
        // No past rows at all (fresh-start player at Krasnodar U21).
        // Only the seeded Main-alias current entry + the live friendly bucket.
        // The 2026/27 breakdown must still surface the Friendly line.
        let mut hist = PlayerStatisticsHistory::new();
        hist.current.push(CurrentSeasonEntry {
            team_name: "Krasnodar".to_string(),
            team_slug: "krasnodar".to_string(),
            team_reputation: 5_000,
            league_name: "Premier League".to_string(),
            league_slug: "russian-premier-league".to_string(),
            is_loan: false,
            transfer_fee: None,
            statistics: PlayerStatistics::default(),
            joined_date: d(2026, 7, 1),
            departed_date: None,
            seq_id: 1,
        });

        let live_league = PlayerStatistics::default();
        let live_friendly = stats(1, 0);
        let live = PlayerLiveStatsInput {
            league: &live_league,
            friendly: &live_friendly,
            cups: &[],
            friendly_source_slug: "russian-premier-league-u19",
        };

        let breakdowns =
            PlayerStatisticsProjection::player_history_breakdowns(&hist, &live, d(2026, 8, 15));
        let active = breakdowns
            .iter()
            .find(|b| {
                b.season_start_year == 2026
                    && b.team_slug == "krasnodar"
                    && b.league_slug == "russian-premier-league"
                    && !b.is_loan
            })
            .expect("active Main-aliased breakdown missing for fresh-start player");
        assert!(
            active
                .competitions
                .iter()
                .any(|c| c.competition_kind == PlayerStatCompetitionKind::Friendly
                    && c.statistics.played == 1),
            "fresh-start youth player's breakdown must include the live friendly line"
        );
    }

    #[test]
    fn history_breakdown_for_youth_aliased_player_includes_live_friendly() {
        // User-reported repro: U21 player Sergey Petrov at Krasnodar U21
        // plays one U21-league friendly. Overview shows it. History
        // breakdown row for the Main-aliased current spell drops the
        // Friendly line and shows only the bare Premier League stub.
        //
        // Setup matches what `seed_player_histories` writes for a U21
        // player: the current entry is the Main team alias
        // (team_slug=krasnodar, league_slug=russian-premier-league), with
        // a frozen prior season at the same Main alias. The web layer
        // passes the youth league slug as `friendly_source_slug` so the
        // breakdown can label the Friendly row "Russian Premier League
        // U19".
        let mut hist = PlayerStatisticsHistory::from_items(vec![
            frozen_in_league(2025, "krasnodar", "russian-premier-league", 0, 0, 1),
        ]);
        hist.current.push(CurrentSeasonEntry {
            team_name: "Krasnodar".to_string(),
            team_slug: "krasnodar".to_string(),
            team_reputation: 5_000,
            league_name: "Premier League".to_string(),
            league_slug: "russian-premier-league".to_string(),
            is_loan: false,
            transfer_fee: None,
            statistics: PlayerStatistics::default(),
            joined_date: d(2026, 7, 1),
            departed_date: None,
            seq_id: 50,
        });

        let live_league = PlayerStatistics::default();
        let live_friendly = stats(1, 0);
        let live = PlayerLiveStatsInput {
            league: &live_league,
            friendly: &live_friendly,
            cups: &[],
            friendly_source_slug: "russian-premier-league-u19",
        };

        let breakdowns =
            PlayerStatisticsProjection::player_history_breakdowns(&hist, &live, d(2026, 8, 15));
        let active = breakdowns
            .iter()
            .find(|b| {
                b.season_start_year == 2026
                    && b.team_slug == "krasnodar"
                    && b.league_slug == "russian-premier-league"
                    && !b.is_loan
            })
            .expect("active Main-aliased breakdown missing");
        let kinds: Vec<PlayerStatCompetitionKind> =
            active.competitions.iter().map(|c| c.competition_kind).collect();
        assert!(
            kinds.contains(&PlayerStatCompetitionKind::Friendly),
            "youth-aliased breakdown must include the live friendly line, got: {:?}",
            kinds
        );
        let friendly = active
            .competitions
            .iter()
            .find(|c| c.competition_kind == PlayerStatCompetitionKind::Friendly)
            .unwrap();
        assert_eq!(friendly.statistics.played, 1);
        assert_eq!(
            friendly.competition_slug, "russian-premier-league-u19",
            "friendly entry must keep the youth league slug so the renderer labels it correctly"
        );
    }

    #[test]
    fn history_keeps_every_zero_app_main_alias_season_for_youth_only_career() {
        // User-reported repro: a U21 player (Vladislav Torop) who has
        // never logged a senior appearance shows only the current-season
        // row on the history page — every past 0-app Main alias row
        // gets dropped by the projection's "no contesting sibling"
        // fall-through, which used to require a transfer fee. The
        // storage layer (record_season_end on the non-senior path)
        // writes the row faithfully; the projection must surface it.
        let hist = PlayerStatisticsHistory::from_items(vec![
            frozen(2025, "spartak", 0, 0),
            frozen(2026, "spartak", 0, 0),
            frozen(2027, "spartak", 0, 0),
        ]);
        let empty_stats = PlayerStatistics::default();
        let live = empty_live(&empty_stats);
        let rows = PlayerStatisticsProjection::player_history_rows(&hist, &live, d(2028, 9, 1));
        let years: Vec<u16> = rows
            .iter()
            .filter(|r| r.team_slug == "spartak")
            .map(|r| r.season.start_year)
            .collect();
        assert!(
            years.contains(&2025) && years.contains(&2026) && years.contains(&2027),
            "every past Main alias row must surface for a youth-only career, got: {:?}",
            years
        );
    }

    #[test]
    fn history_fills_quiet_year_between_two_played_seasons_at_same_club() {
        // User-reported repro: a Spartak Moscow player with rows for
        // 2025/26 and 2027/28 was missing the 2026/27 row because the
        // storage layer dropped the quiet middle season (0 senior apps,
        // no fee). Continuity gap-fill must surface it.
        let hist = PlayerStatisticsHistory::from_items(vec![
            frozen(2023, "spartak", 24, 0),
            frozen(2024, "spartak", 18, 0),
            frozen(2025, "spartak", 22, 0),
            // 2026/27 deliberately missing
            frozen(2027, "spartak", 14, 0),
        ]);
        let empty_stats = PlayerStatistics::default();
        let live = empty_live(&empty_stats);
        let rows = PlayerStatisticsProjection::player_history_rows(&hist, &live, d(2028, 9, 1));
        let years: Vec<u16> = rows.iter().map(|r| r.season.start_year).collect();
        assert!(
            years.contains(&2026),
            "missing 2026/27 must be back-filled, got years: {:?}",
            years
        );
        let filled = rows
            .iter()
            .find(|r| r.season.start_year == 2026)
            .expect("filled row must exist");
        assert_eq!(filled.team_slug, "spartak");
        assert_eq!(filled.statistics.played, 0);
        assert!(!filled.is_loan);
    }

    #[test]
    fn history_does_not_fill_gap_year_covered_by_a_loan_row() {
        // Spartak in 2025/26, on loan to Other in 2026/27, back at
        // Spartak in 2027/28. The loan row already accounts for the
        // middle year — no synthetic Spartak row may shadow it.
        let mut a = PlayerStatistics::default();
        a.played = 22;
        let mut c = PlayerStatistics::default();
        c.played = 14;
        let hist = PlayerStatisticsHistory::from_items(vec![
            PlayerStatisticsHistoryItem {
                season: Season::new(2025),
                team_name: "spartak".to_string(),
                team_slug: "spartak".to_string(),
                team_reputation: 5_000,
                league_name: "League".to_string(),
                league_slug: "league".to_string(),
                is_loan: false,
                transfer_fee: None,
                statistics: a,
                seq_id: 1,
            },
            PlayerStatisticsHistoryItem {
                season: Season::new(2026),
                team_name: "other".to_string(),
                team_slug: "other".to_string(),
                team_reputation: 5_000,
                league_name: "League".to_string(),
                league_slug: "league".to_string(),
                is_loan: true,
                transfer_fee: Some(0.0),
                statistics: PlayerStatistics {
                    played: 15,
                    ..Default::default()
                },
                seq_id: 2,
            },
            PlayerStatisticsHistoryItem {
                season: Season::new(2027),
                team_name: "spartak".to_string(),
                team_slug: "spartak".to_string(),
                team_reputation: 5_000,
                league_name: "League".to_string(),
                league_slug: "league".to_string(),
                is_loan: false,
                transfer_fee: None,
                statistics: c,
                seq_id: 3,
            },
        ]);
        let empty_stats = PlayerStatistics::default();
        let live = empty_live(&empty_stats);
        let rows = PlayerStatisticsProjection::player_history_rows(&hist, &live, d(2028, 9, 1));
        // Exactly one row per (year, team, is_loan); the loan row
        // covers 2026 and no synthetic Spartak row was added.
        let spartak_2026 = rows
            .iter()
            .filter(|r| r.season.start_year == 2026 && r.team_slug == "spartak")
            .count();
        assert_eq!(
            spartak_2026, 0,
            "loan-covered year must not receive a parent-club placeholder"
        );
    }

    #[test]
    fn canonical_ledger_survives_when_legacy_items_dropped_the_row() {
        // The point of the canonical ledger: storage filters that strip
        // a row from `history.items` cannot hide it from the projection.
        // We simulate that exact pattern — write a canonical ledger
        // entry for 2026/27 directly, leave `items` empty for that
        // year — and verify the projection still surfaces the row.
        let team_info = TeamInfo {
            name: "Spartak Moscow".to_string(),
            slug: "spartak".to_string(),
            reputation: 5_000,
            league_name: "Premier League".to_string(),
            league_slug: "premier-league".to_string(),
        };
        let mut hist = PlayerStatisticsHistory::from_items(vec![
            frozen(2025, "spartak", 22, 0),
            frozen(2027, "spartak", 14, 0),
        ]);
        // 2026/27 is missing from `items` — the storage filters
        // dropped it. The canonical ledger gets the row written
        // directly (this is what `record_season_end` now does for
        // every closing-team write, no filters in between).
        hist.append_to_ledger(
            2026,
            &team_info,
            PlayerStatCompetitionKind::League,
            false,
            None,
            PlayerStatistics::default(),
        );

        let empty_stats = PlayerStatistics::default();
        let live = empty_live(&empty_stats);
        let rows = PlayerStatisticsProjection::player_history_rows(&hist, &live, d(2028, 9, 1));
        let years: Vec<u16> = rows
            .iter()
            .filter(|r| r.team_slug == "spartak")
            .map(|r| r.season.start_year)
            .collect();
        assert!(
            years.contains(&2026),
            "canonical ledger must surface a row even when legacy `items` dropped it; \
             got years: {:?}",
            years
        );
    }

    #[test]
    fn canonical_ledger_idempotent_merge_on_repeat_writes() {
        // Repeated `record_season_end` for the same (year, team, kind, is_loan)
        // must merge stats, not duplicate the row. Models the multi-league
        // country pattern where snapshot fires twice for the same season.
        let team_info = TeamInfo {
            name: "Spartak Moscow".to_string(),
            slug: "spartak".to_string(),
            reputation: 5_000,
            league_name: "Premier League".to_string(),
            league_slug: "premier-league".to_string(),
        };
        let mut hist = PlayerStatisticsHistory::new();
        let mut s1 = PlayerStatistics::default();
        s1.played = 10;
        let mut s2 = PlayerStatistics::default();
        s2.played = 5;
        hist.append_to_ledger(
            2026,
            &team_info,
            PlayerStatCompetitionKind::League,
            false,
            None,
            s1,
        );
        hist.append_to_ledger(
            2026,
            &team_info,
            PlayerStatCompetitionKind::League,
            false,
            None,
            s2,
        );
        let league_rows: Vec<_> = hist
            .season_ledger
            .iter()
            .filter(|e| {
                e.season_start_year == 2026
                    && e.team_slug == "spartak"
                    && e.competition_kind == PlayerStatCompetitionKind::League
            })
            .collect();
        assert_eq!(league_rows.len(), 1, "repeat writes must merge in place");
        assert_eq!(league_rows[0].statistics.played, 15);
    }

    #[test]
    fn canonical_ledger_keeps_league_and_continental_under_one_row() {
        // ContinentalCup written separately must fold into the season's
        // League row at render time, exactly once.
        let team_info = TeamInfo {
            name: "Juventus".to_string(),
            slug: "juventus".to_string(),
            reputation: 5_000,
            league_name: "Serie A".to_string(),
            league_slug: "serie-a".to_string(),
        };
        let mut hist = PlayerStatisticsHistory::new();
        let mut league = PlayerStatistics::default();
        league.played = 28;
        league.goals = 6;
        hist.append_to_ledger(
            2024,
            &team_info,
            PlayerStatCompetitionKind::League,
            false,
            None,
            league,
        );
        let mut cont = PlayerStatistics::default();
        cont.played = 10;
        cont.goals = 5;
        hist.append_to_ledger(
            2024,
            &team_info,
            PlayerStatCompetitionKind::ContinentalCup,
            false,
            None,
            cont,
        );

        let empty_stats = PlayerStatistics::default();
        let live = empty_live(&empty_stats);
        let rows = PlayerStatisticsProjection::player_history_rows(&hist, &live, d(2026, 9, 1));
        let row = rows
            .iter()
            .find(|r| r.season.start_year == 2024 && r.team_slug == "juventus")
            .expect("league row must exist");
        assert_eq!(row.statistics.played, 38, "28 league + 10 continental");
        assert_eq!(row.statistics.goals, 11, "6 league + 5 continental");
        // Only one row for the (year, team, is_loan) group — no
        // duplicate Continental-only render line.
        assert_eq!(
            rows.iter()
                .filter(|r| r.season.start_year == 2024 && r.team_slug == "juventus")
                .count(),
            1
        );
    }

    #[test]
    fn history_drops_zero_app_phantom_loan_alongside_played_parent_row() {
        // User-reported repro: a player plays 36 league games at
        // Spartak Moscow in 2025/26 then goes on loan to Pari in late
        // May 2026. `Season::from_date` puts that May date in the
        // 2025/26 window, so the loan ledger row gets stamped under
        // 2025/26 even though the player effectively spent the
        // season at Spartak. The projection must drop the phantom
        // loan row because a sibling for the same year has games.
        let spartak = TeamInfo {
            name: "Spartak Moscow".to_string(),
            slug: "spartak".to_string(),
            reputation: 5_000,
            league_name: "Premier League".to_string(),
            league_slug: "russian-premier-league".to_string(),
        };
        let pari = TeamInfo {
            name: "Pari Nizhniy Novgorod".to_string(),
            slug: "pari".to_string(),
            reputation: 2_000,
            league_name: "Premier League".to_string(),
            league_slug: "russian-premier-league".to_string(),
        };
        let mut hist = PlayerStatisticsHistory::new();
        let mut spartak_played = PlayerStatistics::default();
        spartak_played.played = 36;
        hist.append_to_ledger(
            2025,
            &spartak,
            PlayerStatCompetitionKind::League,
            false,
            None,
            spartak_played,
        );
        // Phantom: 0 apps, free-loan sentinel fee, loan row stamped
        // by the late-May loan event.
        hist.append_to_ledger(
            2025,
            &pari,
            PlayerStatCompetitionKind::League,
            true,
            Some(0.0),
            PlayerStatistics::default(),
        );

        let empty_stats = PlayerStatistics::default();
        let live = empty_live(&empty_stats);
        let rows = PlayerStatisticsProjection::player_history_rows(&hist, &live, d(2027, 9, 1));
        let pari_2025_count = rows
            .iter()
            .filter(|r| r.season.start_year == 2025 && r.team_slug == "pari")
            .count();
        assert_eq!(
            pari_2025_count, 0,
            "phantom 0-app loan row alongside a played parent must be dropped"
        );
        let spartak_2025 = rows
            .iter()
            .find(|r| r.season.start_year == 2025 && r.team_slug == "spartak")
            .expect("real parent row must remain");
        assert_eq!(spartak_2025.statistics.played, 36);
    }

    #[test]
    fn history_keeps_loan_row_when_it_is_the_sole_record_of_the_season() {
        // A 0-app loan with NO sibling for the season (e.g. a continuous
        // multi-season loan whose middle year had no parent-club row)
        // must remain — it's the only record of where the player was.
        let loan_to = TeamInfo {
            name: "LoanClub".to_string(),
            slug: "loan-club".to_string(),
            reputation: 1_000,
            league_name: "League".to_string(),
            league_slug: "league".to_string(),
        };
        let mut hist = PlayerStatisticsHistory::new();
        hist.append_to_ledger(
            2025,
            &loan_to,
            PlayerStatCompetitionKind::League,
            true,
            Some(0.0),
            PlayerStatistics::default(),
        );

        let empty_stats = PlayerStatistics::default();
        let live = empty_live(&empty_stats);
        let rows = PlayerStatisticsProjection::player_history_rows(&hist, &live, d(2027, 9, 1));
        let kept = rows
            .iter()
            .any(|r| r.season.start_year == 2025 && r.team_slug == "loan-club" && r.is_loan);
        assert!(kept, "loan row must be kept when it's the only career mark of the season");
    }

    #[test]
    fn history_keeps_empty_loan_row_alongside_owning_club_row() {
        // User rule: every loan spell must show, even at 0 apps. A player
        // loaned out who never featured still gets the loan row; the
        // owning-club "career home" row coexists with it. Neither erases
        // the other — only a sibling that actually PLAYED games would mark
        // the loan a phantom (covered by a separate test).
        let parent = TeamInfo {
            name: "Spartak Moscow".to_string(),
            slug: "spartak-moscow".to_string(),
            reputation: 5_000,
            league_name: "Premier League".to_string(),
            league_slug: "russian-premier-league".to_string(),
        };
        let loan_to = TeamInfo {
            name: "Zenit".to_string(),
            slug: "zenit".to_string(),
            reputation: 5_000,
            league_name: "Premier League".to_string(),
            league_slug: "russian-premier-league".to_string(),
        };
        let mut hist = PlayerStatisticsHistory::new();
        hist.append_to_ledger(
            2026,
            &parent,
            PlayerStatCompetitionKind::League,
            false,
            None,
            PlayerStatistics::default(),
        );
        hist.append_to_ledger(
            2026,
            &loan_to,
            PlayerStatCompetitionKind::League,
            true,
            Some(0.0),
            PlayerStatistics::default(),
        );

        let empty_stats = PlayerStatistics::default();
        let live = empty_live(&empty_stats);
        let rows = PlayerStatisticsProjection::player_history_rows(&hist, &live, d(2027, 9, 1));
        assert!(
            rows.iter()
                .any(|r| r.season.start_year == 2026 && r.team_slug == "zenit" && r.is_loan),
            "empty loan row must be kept; got {:?}",
            rows.iter()
                .map(|r| format!("{}{}", r.team_slug, if r.is_loan { "(loan)" } else { "" }))
                .collect::<Vec<_>>()
        );
        assert!(
            rows.iter().any(|r| r.season.start_year == 2026
                && r.team_slug == "spartak-moscow"
                && !r.is_loan),
            "owning-club row must coexist with the loan"
        );
    }

    #[test]
    fn history_drops_owning_club_row_for_later_full_loan_season_but_keeps_debut() {
        // User report: a player owned by Spartak spends 2026/27 (debut),
        // 2027/28 and 2028/29 on loan. The debut Spartak row stays; the
        // 0-app Spartak rows for the later full-loan seasons are redundant
        // noise and must drop. The loan rows always remain.
        let spartak = TeamInfo {
            name: "Spartak Moscow".to_string(),
            slug: "spartak".to_string(),
            reputation: 5_000,
            league_name: "Premier League".to_string(),
            league_slug: "rpl".to_string(),
        };
        let mut hist = PlayerStatisticsHistory::new();
        let loan_clubs = [("zenit", 2026, 0u16), ("krylya", 2027, 1), ("krylya", 2028, 29)];
        for year in [2026u16, 2027, 2028] {
            // Owning-club 0-app row each season.
            hist.append_to_ledger(
                year,
                &spartak,
                PlayerStatCompetitionKind::League,
                false,
                None,
                PlayerStatistics::default(),
            );
        }
        for (slug, year, games) in loan_clubs {
            let mut s = PlayerStatistics::default();
            s.played = games;
            let club = TeamInfo {
                name: slug.to_string(),
                slug: slug.to_string(),
                reputation: 4_000,
                league_name: "Premier League".to_string(),
                league_slug: "rpl".to_string(),
            };
            hist.append_to_ledger(year, &club, PlayerStatCompetitionKind::League, true, Some(0.0), s);
        }

        let empty_stats = PlayerStatistics::default();
        let live = empty_live(&empty_stats);
        let rows = PlayerStatisticsProjection::player_history_rows(&hist, &live, d(2030, 9, 1));
        let has = |y: u16, slug: &str, loan: bool| {
            rows.iter().any(|r| r.season.start_year == y && r.team_slug == slug && r.is_loan == loan)
        };
        // Debut owning-club row kept; later full-loan owning-club rows dropped.
        assert!(has(2026, "spartak", false), "debut owning-club row must stay");
        assert!(!has(2027, "spartak", false), "later full-loan owning-club row must drop");
        assert!(!has(2028, "spartak", false), "later full-loan owning-club row must drop");
        // All loan rows always present, even the 0-app one.
        assert!(has(2026, "zenit", true));
        assert!(has(2027, "krylya", true));
        assert!(has(2028, "krylya", true));
    }

    #[test]
    fn history_keeps_parent_club_row_during_loan_out_season_after_freeze() {
        // User-reported repro: a player owned by Spartak is loaned to
        // Zenit for their debut season. During the season the table shows
        // both rows; once the season freezes, the Spartak 0-app parent
        // row must NOT collapse — it's the player's owning club ("career
        // home"), and the loan sibling alone shouldn't erase it. The
        // re-seed drops the parent fee to `None`, so the fee gate must
        // not be what decides this.
        let spartak = TeamInfo {
            name: "Spartak Moscow".to_string(),
            slug: "spartak-moscow".to_string(),
            reputation: 5_000,
            league_name: "Premier League".to_string(),
            league_slug: "russian-premier-league".to_string(),
        };
        let zenit = TeamInfo {
            name: "Zenit".to_string(),
            slug: "zenit".to_string(),
            reputation: 5_000,
            league_name: "Premier League".to_string(),
            league_slug: "russian-premier-league".to_string(),
        };
        let mut hist = PlayerStatisticsHistory::new();
        // Parent club, 0 apps, no fee (re-seed dropped it).
        hist.append_to_ledger(
            2026,
            &spartak,
            PlayerStatCompetitionKind::League,
            false,
            None,
            PlayerStatistics::default(),
        );
        // Loan-out spell with real games.
        let mut zenit_played = PlayerStatistics::default();
        zenit_played.played = 20;
        zenit_played.goals = 3;
        hist.append_to_ledger(
            2026,
            &zenit,
            PlayerStatCompetitionKind::League,
            true,
            Some(0.0),
            zenit_played,
        );

        let empty_stats = PlayerStatistics::default();
        let live = empty_live(&empty_stats);
        let rows = PlayerStatisticsProjection::player_history_rows(&hist, &live, d(2027, 9, 1));
        assert!(
            rows.iter().any(|r| r.season.start_year == 2026
                && r.team_slug == "spartak-moscow"
                && !r.is_loan),
            "parent-club row must survive the freeze alongside the loan row; got {:?}",
            rows.iter()
                .map(|r| format!("{}:{}{}", r.season.start_year, r.team_slug, if r.is_loan { "(loan)" } else { "" }))
                .collect::<Vec<_>>()
        );
        assert!(
            rows.iter()
                .any(|r| r.season.start_year == 2026 && r.team_slug == "zenit" && r.is_loan),
            "loan row must remain too"
        );
    }

    #[test]
    fn projection_is_pure_when_called_twice() {
        let mut hist = PlayerStatisticsHistory::from_items(vec![frozen(2024, "juventus", 30, 8)]);
        hist.current
            .push(current_entry("juventus", d(2025, 8, 1), None));
        let live_league = stats(15, 3);
        let live_friendly = PlayerStatistics::default();
        let live = PlayerLiveStatsInput {
            league: &live_league,
            friendly: &live_friendly,
            cups: &[],
            friendly_source_slug: "",
        };

        let frozen_items_before = hist.items.len();
        let current_before = hist.current.len();

        let rows_a = PlayerStatisticsProjection::player_history_rows(&hist, &live, d(2025, 10, 1));
        let rows_b = PlayerStatisticsProjection::player_history_rows(&hist, &live, d(2025, 10, 1));

        assert_eq!(rows_a.len(), rows_b.len());
        for (a, b) in rows_a.iter().zip(rows_b.iter()) {
            assert_eq!(a.season.start_year, b.season.start_year);
            assert_eq!(a.team_slug, b.team_slug);
            assert_eq!(a.statistics.played, b.statistics.played);
            assert_eq!(a.statistics.goals, b.statistics.goals);
        }
        assert_eq!(hist.items.len(), frozen_items_before);
        assert_eq!(hist.current.len(), current_before);
    }

    #[test]
    fn weighted_rating_blends_through_ledger_merge() {
        // Two seasons at 7.0 and 6.0 (full starter weight each) → 6.5.
        let mut a = PlayerStatistics::default();
        for _ in 0..10 {
            a.played += 1;
            a.record_match_rating(7.0, 90, true);
        }
        let mut b = PlayerStatistics::default();
        for _ in 0..10 {
            b.played += 1;
            b.record_match_rating(6.0, 90, true);
        }

        let hist = PlayerStatisticsHistory::from_items(vec![
            PlayerStatisticsHistoryItem {
                season: Season::new(2024),
                team_name: "juventus".to_string(),
                team_slug: "juventus".to_string(),
                team_reputation: 5_000,
                league_name: "League".to_string(),
                league_slug: "league".to_string(),
                is_loan: false,
                transfer_fee: None,
                statistics: a,
                seq_id: 1,
            },
            PlayerStatisticsHistoryItem {
                season: Season::new(2025),
                team_name: "juventus".to_string(),
                team_slug: "juventus".to_string(),
                team_reputation: 5_000,
                league_name: "League".to_string(),
                league_slug: "league".to_string(),
                is_loan: false,
                transfer_fee: None,
                statistics: b,
                seq_id: 2,
            },
        ]);
        let empty_stats = PlayerStatistics::default();
        let live = empty_live(&empty_stats);
        let rows = PlayerStatisticsProjection::player_history_rows(&hist, &live, d(2026, 9, 1));
        let totals = PlayerStatisticsProjection::player_history_totals(&rows);
        let weighted = totals.weighted_average_rating();
        assert!(
            (weighted - 6.5).abs() < 0.01,
            "weighted merge expected ~6.5, got {}",
            weighted
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Projection invariants:
//
//   - non-League entries do NOT use `is_loan` as a grouping key
//   - League entries are the only source of row is_loan / transfer_fee
//   - Friendly never contributes to career totals
//   - Competitive cups contribute exactly once (folded into the row's League)
//   - breakdown order is League → ContinentalCup → DomesticCup → Friendly,
//     stable within a kind
//   - every visible history row has a matching breakdown (no drift)
// ─────────────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod projection_invariants_tests {
    use super::*;
    use crate::club::player::statistics::history::{
        CurrentSeasonEntry, PlayerStatisticsHistory,
    };
    use crate::club::player::statistics::ledger::LiveCupSlice;
    use crate::club::player::statistics::types::TeamInfo;
    use crate::continent::competitions::{
        CHAMPIONS_LEAGUE_SLUG, EUROPA_LEAGUE_SLUG,
    };

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn stats(played: u16, goals: u16) -> PlayerStatistics {
        let mut s = PlayerStatistics::default();
        s.played = played;
        s.goals = goals;
        s
    }

    fn team(slug: &str, league_slug: &str) -> TeamInfo {
        TeamInfo {
            name: slug.to_string(),
            slug: slug.to_string(),
            reputation: 5_000,
            league_name: "L".to_string(),
            league_slug: league_slug.to_string(),
        }
    }

    fn empty_live<'a>(s: &'a PlayerStatistics) -> PlayerLiveStatsInput<'a> {
        PlayerLiveStatsInput {
            league: s,
            friendly: s,
            cups: &[],
            friendly_source_slug: "",
        }
    }

    #[test]
    fn non_league_grouping_ignores_is_loan() {
        // The row's League entry is is_loan=true. The drain-style
        // non-League entries are written with is_loan=false (the
        // recorder's hardcoded value). Both must group into ONE
        // breakdown — grouping ignores is_loan for non-League.
        // Use a PAST season so the League entry is read from the
        // canonical ledger (current-year League comes from history.current).
        let info = team("pari", "rpl");
        let mut hist = PlayerStatisticsHistory::new();
        hist.append_to_ledger(
            2026,
            &info,
            PlayerStatCompetitionKind::League,
            true,
            Some(0.0),
            stats(9, 0),
        );
        hist.record_friendly(2026, &info, "rpl-u19".to_string(), stats(2, 0));
        hist.record_domestic_cup(2026, &info, "russia-cup".to_string(), stats(1, 0));

        let empty = PlayerStatistics::default();
        let live = empty_live(&empty);
        let bds = PlayerStatisticsProjection::player_history_breakdowns(&hist, &live, d(2028, 9, 1));
        let pari: Vec<_> = bds
            .iter()
            .filter(|b| b.season_start_year == 2026 && b.team_slug == "pari")
            .collect();
        assert_eq!(pari.len(), 1, "non-League entries must not orphan into a second breakdown");
        assert!(pari[0].is_loan, "loan label inherited from the League entry");
    }

    #[test]
    fn row_is_loan_and_fee_come_from_league_entries_only() {
        // Two League rows for the same (year, team, league): one loan
        // (older seq), one permanent (newer seq, with a fee). The row
        // adopts the latest LEAGUE entry's (is_loan, transfer_fee).
        // A non-League entry's hardcoded is_loan=false / fee=None must
        // NEVER overwrite this. PAST season so the canonical ledger
        // sources the League entries directly.
        let info = team("juventus", "serie-a");
        let mut hist = PlayerStatisticsHistory::new();
        hist.append_to_ledger(
            2026,
            &info,
            PlayerStatCompetitionKind::League,
            true,
            Some(0.0),
            stats(5, 0),
        );
        hist.append_to_ledger(
            2026,
            &info,
            PlayerStatCompetitionKind::League,
            false,
            Some(5_000_000.0),
            stats(8, 1),
        );
        // Non-League entries written AFTER League (newer seq) must not
        // hijack the row's metadata.
        hist.record_friendly(2026, &info, "serie-a".to_string(), stats(3, 0));
        hist.record_continental(
            2026,
            &info,
            CHAMPIONS_LEAGUE_SLUG.to_string(),
            stats(7, 2),
        );

        let empty = PlayerStatistics::default();
        let live = empty_live(&empty);
        let rows = PlayerStatisticsProjection::player_history_rows(&hist, &live, d(2028, 9, 1));
        let juve = rows
            .iter()
            .find(|r| r.season.start_year == 2026 && r.team_slug == "juventus")
            .expect("row missing");
        assert!(!juve.is_loan, "latest League entry is permanent");
        assert_eq!(juve.transfer_fee, Some(5_000_000.0));
    }

    #[test]
    fn domestic_cup_contributes_to_career_totals_but_friendly_does_not() {
        let info = team("inter", "serie-a");
        let mut hist = PlayerStatisticsHistory::new();
        hist.append_to_ledger(
            2025,
            &info,
            PlayerStatCompetitionKind::League,
            false,
            None,
            stats(30, 8),
        );
        hist.record_friendly(2025, &info, "serie-a".to_string(), stats(5, 2));
        hist.record_domestic_cup(2025, &info, "coppa-italia".to_string(), stats(4, 1));

        let empty = PlayerStatistics::default();
        let live = empty_live(&empty);
        let rows = PlayerStatisticsProjection::player_history_rows(&hist, &live, d(2027, 9, 1));
        let totals = PlayerStatisticsProjection::player_history_totals(&rows);
        assert_eq!(
            totals.played, 34,
            "DomesticCup included; Friendly excluded from totals"
        );
        assert_eq!(totals.goals, 9);
    }

    #[test]
    fn continental_cup_contributes_to_career_totals_exactly_once() {
        let info = team("juventus", "serie-a");
        let mut hist = PlayerStatisticsHistory::new();
        hist.append_to_ledger(
            2025,
            &info,
            PlayerStatCompetitionKind::League,
            false,
            None,
            stats(28, 6),
        );
        // Two continental tournaments in one season.
        hist.record_continental(2025, &info, CHAMPIONS_LEAGUE_SLUG.to_string(), stats(10, 5));
        hist.record_continental(2025, &info, EUROPA_LEAGUE_SLUG.to_string(), stats(4, 1));

        let empty = PlayerStatistics::default();
        let live = empty_live(&empty);
        let rows = PlayerStatisticsProjection::player_history_rows(&hist, &live, d(2027, 9, 1));
        let row = rows
            .iter()
            .find(|r| r.season.start_year == 2025 && r.team_slug == "juventus")
            .unwrap();
        // 28 league + 10 UCL + 4 UEL = 42, each folded ONCE.
        assert_eq!(row.statistics.played, 42);
        assert_eq!(row.statistics.goals, 12);

        // Same row, projecting again: still 42 — pure, idempotent.
        let rows_b =
            PlayerStatisticsProjection::player_history_rows(&hist, &live, d(2027, 9, 1));
        let row_b = rows_b
            .iter()
            .find(|r| r.season.start_year == 2025 && r.team_slug == "juventus")
            .unwrap();
        assert_eq!(row_b.statistics.played, 42);
    }

    #[test]
    fn breakdown_order_is_league_continental_domestic_friendly() {
        let info = team("juventus", "serie-a");
        let mut hist = PlayerStatisticsHistory::new();
        // Insert in REVERSE order of expected output to prove sorting.
        hist.record_friendly(2025, &info, "serie-a".to_string(), stats(3, 0));
        hist.record_domestic_cup(2025, &info, "coppa-italia".to_string(), stats(4, 1));
        hist.record_continental(2025, &info, CHAMPIONS_LEAGUE_SLUG.to_string(), stats(8, 3));
        hist.append_to_ledger(
            2025,
            &info,
            PlayerStatCompetitionKind::League,
            false,
            None,
            stats(28, 6),
        );

        let empty = PlayerStatistics::default();
        let live = empty_live(&empty);
        let bds =
            PlayerStatisticsProjection::player_history_breakdowns(&hist, &live, d(2027, 1, 1));
        let bd = bds
            .iter()
            .find(|b| b.season_start_year == 2025 && b.team_slug == "juventus")
            .unwrap();
        let kinds: Vec<PlayerStatCompetitionKind> =
            bd.competitions.iter().map(|c| c.competition_kind).collect();
        assert_eq!(
            kinds,
            vec![
                PlayerStatCompetitionKind::League,
                PlayerStatCompetitionKind::ContinentalCup,
                PlayerStatCompetitionKind::DomesticCup,
                PlayerStatCompetitionKind::Friendly,
            ],
            "breakdown kind order"
        );
    }

    #[test]
    fn breakdown_within_kind_keeps_first_seen_order() {
        // Two continental tournaments: UCL first, UEL second. Even
        // though sort_by_key is used, it's stable on identical keys —
        // within a kind, the first-seen entry stays first.
        let info = team("juventus", "serie-a");
        let mut hist = PlayerStatisticsHistory::new();
        hist.append_to_ledger(
            2025,
            &info,
            PlayerStatCompetitionKind::League,
            false,
            None,
            stats(28, 6),
        );
        hist.record_continental(2025, &info, CHAMPIONS_LEAGUE_SLUG.to_string(), stats(6, 2));
        hist.record_continental(2025, &info, EUROPA_LEAGUE_SLUG.to_string(), stats(4, 1));

        let empty = PlayerStatistics::default();
        let live = empty_live(&empty);
        let bds =
            PlayerStatisticsProjection::player_history_breakdowns(&hist, &live, d(2027, 1, 1));
        let bd = bds
            .iter()
            .find(|b| b.season_start_year == 2025 && b.team_slug == "juventus")
            .unwrap();
        let cont_slugs: Vec<&str> = bd
            .competitions
            .iter()
            .filter(|c| c.competition_kind == PlayerStatCompetitionKind::ContinentalCup)
            .map(|c| c.competition_slug.as_str())
            .collect();
        assert_eq!(cont_slugs, vec![CHAMPIONS_LEAGUE_SLUG, EUROPA_LEAGUE_SLUG]);
    }

    // ── Rows / breakdowns alignment ───────────────────────────────────

    #[test]
    fn every_visible_row_has_a_matching_breakdown_or_synthetic_fallback() {
        // Mixed scenario stressing every code path: frozen seasons,
        // continental ledger, current departed + active spells, live
        // cups + friendlies, gap year that triggers synthetic fill.
        let juve = team("juventus", "serie-a");
        let mut hist = PlayerStatisticsHistory::new();
        hist.append_to_ledger(
            2024,
            &juve,
            PlayerStatCompetitionKind::League,
            false,
            None,
            stats(30, 8),
        );
        hist.record_continental(2024, &juve, CHAMPIONS_LEAGUE_SLUG.to_string(), stats(10, 5));
        // 2025 frozen.
        hist.append_to_ledger(
            2025,
            &juve,
            PlayerStatCompetitionKind::League,
            false,
            None,
            stats(28, 6),
        );
        // 2026 deliberately missing → gap-fill territory.
        hist.append_to_ledger(
            2027,
            &juve,
            PlayerStatCompetitionKind::League,
            false,
            None,
            stats(20, 4),
        );
        // Current active spell at juventus.
        hist.current.push(CurrentSeasonEntry {
            team_name: "juventus".to_string(),
            team_slug: "juventus".to_string(),
            team_reputation: 5_000,
            league_name: "Serie A".to_string(),
            league_slug: "serie-a".to_string(),
            is_loan: false,
            transfer_fee: None,
            statistics: PlayerStatistics::default(),
            joined_date: d(2028, 8, 1),
            departed_date: None,
            seq_id: 99,
        });

        let live_league = stats(12, 3);
        let live_friendly = stats(2, 0);
        let live_cup = stats(5, 2);
        let cups = vec![LiveCupSlice {
            competition_slug: CHAMPIONS_LEAGUE_SLUG,
            competition_name: "Champions League".to_string(),
            statistics: &live_cup,
        }];
        let live = PlayerLiveStatsInput {
            league: &live_league,
            friendly: &live_friendly,
            cups: &cups,
            friendly_source_slug: "",
        };

        let rows =
            PlayerStatisticsProjection::player_history_rows(&hist, &live, d(2028, 10, 1));
        let bds =
            PlayerStatisticsProjection::player_history_breakdowns(&hist, &live, d(2028, 10, 1));

        // Each visible row must have a corresponding breakdown by
        // (year, team, league), OR — for synthetic gap-fill rows the
        // projection inserts at render time — the web layer falls
        // through to a single-League stub. Both modes are acceptable;
        // what's NOT acceptable is a row keyed differently from its
        // breakdown.
        let bd_keys: std::collections::HashSet<(u16, String, String)> = bds
            .iter()
            .map(|b| (b.season_start_year, b.team_slug.clone(), b.league_slug.clone()))
            .collect();
        for row in &rows {
            let key = (
                row.season.start_year,
                row.team_slug.clone(),
                row.league_slug.clone(),
            );
            let synthetic_gap = row.seq_id == 0 && row.statistics.total_games() == 0;
            assert!(
                bd_keys.contains(&key) || synthetic_gap,
                "row {:?} has no breakdown and is not a synthetic gap-fill",
                key
            );
        }

        // And: every breakdown key must correspond to a visible row OR
        // be excluded ONLY because the row was a phantom-drop (which
        // means the breakdown is unreachable and harmless). To prove
        // the latter case isn't masking a real bug, we assert there is
        // no breakdown that surfaces non-zero career-counting stats
        // without a matching row.
        let row_keys: std::collections::HashSet<(u16, String, String)> = rows
            .iter()
            .map(|r| (r.season.start_year, r.team_slug.clone(), r.league_slug.clone()))
            .collect();
        for bd in &bds {
            let key = (bd.season_start_year, bd.team_slug.clone(), bd.league_slug.clone());
            if row_keys.contains(&key) {
                continue;
            }
            let career_counting: u32 = bd
                .competitions
                .iter()
                .filter(|c| c.competition_kind.counts_toward_career_history())
                .map(|c| {
                    c.statistics.played as u32 + c.statistics.played_subs as u32
                })
                .sum();
            assert_eq!(
                career_counting, 0,
                "orphan breakdown {:?} carries career-counting stats with no visible row",
                key
            );
        }
    }

    #[test]
    fn titlecase_slug_handles_kebab_competition_slugs() {
        // Real cases pulled from generated databases: when a ledger
        // entry references a previous-country cup that doesn't appear
        // in the player's current spell, the projection now Title-
        // Cases the slug rather than leaking "copa-paraguay" verbatim.
        assert_eq!(super::titlecase_slug("copa-paraguay"), "Copa Paraguay");
        assert_eq!(super::titlecase_slug("fa-cup"), "Fa Cup");
        assert_eq!(super::titlecase_slug("dfb-pokal"), "Dfb Pokal");
        assert_eq!(super::titlecase_slug("league"), "League");
        assert_eq!(super::titlecase_slug(""), "");
        // Stray double-dash shouldn't introduce a blank fragment.
        assert_eq!(super::titlecase_slug("copa--del--rey"), "Copa Del Rey");
    }

    #[test]
    fn resolve_cup_name_falls_back_to_titlecase_for_unknown_slug() {
        let live_league = stats(0, 0);
        let live_friendly = stats(0, 0);
        let live = PlayerLiveStatsInput {
            league: &live_league,
            friendly: &live_friendly,
            cups: &[],
            friendly_source_slug: "",
        };
        // Neither the live cup slice nor the domestic-cup override
        // mention `copa-paraguay`, so the resolver must Title-case
        // the slug rather than echoing the raw kebab string.
        let name = PlayerStatisticsProjection::resolve_cup_name("copa-paraguay", &live, None);
        assert_eq!(name, "Copa Paraguay");
    }

    #[test]
    fn origin_club_survives_transfer_out_before_first_senior_game() {
        // Kokarev case: seeded at the origin club at game start, zero
        // senior apps there (youth keeper — only friendlies), sold
        // mid-season, played at the new club, season frozen, page
        // rendered a year later. The origin row is kept by the freeze
        // (`is_initial_record`) and the projection must not re-drop it
        // as a phantom alongside the new club's played row.
        let mut hist = PlayerStatisticsHistory::new();
        hist.seed_initial_team(&team("krylya", "rpl"), d(2026, 7, 1), false);
        hist.record_transfer(
            PlayerStatistics::default(),
            &team("krylya", "rpl"),
            &team("spartak", "rpl"),
            1_600_000.0,
            d(2026, 9, 2),
        );
        hist.record_season_end(
            Season::new(2026),
            stats(2, 0),
            &team("spartak", "rpl"),
            false,
            Some(d(2026, 9, 2)),
        );

        let empty = PlayerStatistics::default();
        let live = empty_live(&empty);
        let rows = PlayerStatisticsProjection::player_history_rows(&hist, &live, d(2027, 12, 12));

        let origin = rows
            .iter()
            .find(|r| r.team_slug == "krylya")
            .expect("origin club row must survive the season-end freeze");
        assert_eq!(origin.season.start_year, 2026);
        assert_eq!(origin.statistics.total_games(), 0);

        let destination = rows
            .iter()
            .find(|r| r.team_slug == "spartak" && r.season.start_year == 2026)
            .expect("destination row missing");
        assert_eq!(destination.statistics.total_games(), 2);
        assert_eq!(destination.transfer_fee, Some(1_600_000.0));
    }

    #[test]
    fn origin_protection_does_not_keep_later_season_phantoms() {
        // The origin carve-out applies to the debut season only: a
        // 0-app no-fee spell in a LATER season, alongside a sibling
        // that actually played, is still the intra-club bounce phantom
        // the filter exists for.
        let mut hist = PlayerStatisticsHistory::new();
        hist.seed_initial_team(&team("krylya", "rpl"), d(2025, 7, 1), false);
        hist.record_season_end(
            Season::new(2025),
            stats(12, 0),
            &team("krylya", "rpl"),
            false,
            None,
        );
        // 2026/27: a phantom 0-app re-registration at a second club in
        // the same season the player demonstrably spent at Krylya.
        hist.append_to_ledger(
            2026,
            &team("krylya", "rpl"),
            PlayerStatCompetitionKind::League,
            false,
            None,
            stats(20, 1),
        );
        hist.append_to_ledger(
            2026,
            &team("dynamo", "rpl"),
            PlayerStatCompetitionKind::League,
            false,
            None,
            stats(0, 0),
        );

        let empty = PlayerStatistics::default();
        let live = empty_live(&empty);
        let rows = PlayerStatisticsProjection::player_history_rows(&hist, &live, d(2027, 12, 12));

        assert!(
            !rows
                .iter()
                .any(|r| r.team_slug == "dynamo" && r.season.start_year == 2026),
            "later-season 0-app phantom must still be dropped"
        );
        assert!(
            rows.iter()
                .any(|r| r.team_slug == "krylya" && r.season.start_year == 2026),
        );
    }
}
