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
    DomesticCupOverride, PlayerCompetitionStatsRow, PlayerHistoryRow, PlayerLiveStatsInput,
    PlayerStatCompetitionKind, PlayerStatLedgerEntry,
};
use super::types::PlayerStatistics;
use crate::league::Season;
use chrono::NaiveDate;
use std::collections::HashMap;

// Type definitions live in `super::ledger` so storage and projection
// can both depend on them without a module cycle. The projection only
// reads them — it never mutates `Player`, history, or any team/country
// state. Calling it twice with the same input returns identical output.

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
        let today_year = Season::from_date(current_date).start_year;

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
                // Past seasons only — the in-progress year's spell
                // metadata still lives in `current` and the live caches,
                // and the canonical ledger doesn't get written until
                // season-end finalisation.
                if entry.season_start_year < today_year {
                    ledger.push(entry.clone());
                }
            }
            // Save-compat: the `continental` field is older than the
            // canonical ledger and may carry rows the ledger doesn't
            // mirror yet (a save written between the two adds). Surface
            // any (season, team) ContinentalCup entry the ledger
            // doesn't already have to avoid double-counting.
            for cont in &history.continental {
                if cont.season_year >= today_year {
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
                today_year
            } else {
                let joined_year = Season::from_date(entry.joined_date).start_year;
                joined_year.min(today_year)
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

        // Resolve the active spell's `(team_slug, season_year)` once.
        // Live cup / friendly slices belong to *this* spell only — never
        // to a past row, and never to a departed current-season row.
        let active_anchor: Option<(String, String, String, u16, u32)> = history
            .current
            .iter()
            .find(|e| e.departed_date.is_none())
            .map(|e| {
                (
                    e.team_slug.clone(),
                    e.league_slug.clone(),
                    e.league_name.clone(),
                    today_year,
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
                ledger.push(PlayerStatLedgerEntry {
                    seq_id: *active_seq,
                    season_start_year: *season_year,
                    team_slug: team_slug.clone(),
                    team_name: team_slug.clone(),
                    team_reputation: 0,
                    league_slug: league_slug.clone(),
                    league_name: league_name.clone(),
                    competition_kind: PlayerStatCompetitionKind::Friendly,
                    competition_slug: String::new(),
                    is_loan: false,
                    transfer_fee: None,
                    statistics: live.friendly.clone(),
                });
            }
        }

        ledger
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
        let today_year = Season::from_date(current_date).start_year;

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
            .filter(|e| e.season_start_year == today_year)
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
    /// `(season_start_year, team_slug, league_slug, is_loan)`. League and
    /// ContinentalCup entries fold into the same row; DomesticCup and
    /// Friendly entries are excluded by [`counts_toward_career_history`].
    ///
    /// Output is sorted by season-year descending, then by `seq_id`
    /// descending so the most recent row surfaces first. Sort is a
    /// presentational choice; correctness does not depend on it.
    pub fn player_history_rows(
        history: &PlayerStatisticsHistory,
        live: &PlayerLiveStatsInput<'_>,
        current_date: NaiveDate,
    ) -> Vec<PlayerHistoryRow> {
        // History never sources from the records-based domestic cup —
        // the in-house ledger is the only allowed source for career
        // rows, and per the centralised policy only continental cups
        // fold in.
        let ledger = Self::build_ledger(history, live, None, current_date);

        // (season, team, league, is_loan) → row. HashMap is fine for grouping;
        // we re-sort the result vector below for stable rendering.
        type Key = (u16, String, String, bool);
        let mut rows: HashMap<Key, PlayerHistoryRow> = HashMap::new();
        // Stable insertion order for rows that share their sort key
        // (rare, but keeps test output deterministic).
        let mut order: Vec<Key> = Vec::new();

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
            let key: Key = (
                entry.season_start_year,
                entry.team_slug.clone(),
                entry.league_slug.clone(),
                entry.is_loan,
            );
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
                    is_loan: entry.is_loan,
                    transfer_fee: entry.transfer_fee,
                    statistics: PlayerStatistics::default(),
                }
            });
            row.statistics.merge_from(&entry.statistics);
            // Highest seq_id wins for tie-breaking sort below; merge
            // empty fields so a continental-only stub doesn't blank
            // out the League row's team/league metadata.
            row.seq_id = row.seq_id.max(entry.seq_id);
            if row.transfer_fee.is_none() {
                row.transfer_fee = entry.transfer_fee;
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
        rows.retain(|_, row| {
            if protected_seqs.contains(&row.seq_id) {
                return true;
            }
            if row.statistics.total_games() > 0 {
                return true;
            }
            // A *paid* transfer fee marks a real signing event — keep
            // even at 0 apps. `Some(0.0)` is the "free" sentinel used
            // for both free transfers and free loans; on its own it
            // does not prove the player actually spent the season at
            // the club. The proxy for "stayed long enough to matter"
            // is "another row for this season has played games" —
            // when one does, this 0-app row is a phantom event seed
            // (typically a transfer/loan stamped in the prior
            // calendar season's window).
            if matches!(row.transfer_fee, Some(f) if f > 0.0) {
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
                        && !(other.team_slug == row.team_slug && other.is_loan == row.is_loan)
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
            // 0-app non-loan row with `Some(0.0)` and no contesting
            // sibling — a "Free" signing record stays as the sole
            // mark of that season.
            row.transfer_fee.is_some()
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
        slug.to_string()
    }
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
    fn history_excludes_domestic_cup_and_friendly() {
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
        };

        let rows = PlayerStatisticsProjection::player_history_rows(&hist, &live, d(2026, 10, 1));
        assert_eq!(rows.len(), 1);
        // 10 league only — neither friendly nor domestic cup folded.
        assert_eq!(rows[0].statistics.played, 10);
        assert_eq!(rows[0].statistics.goals, 2);
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
