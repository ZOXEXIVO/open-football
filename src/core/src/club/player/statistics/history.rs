use super::types::{PlayerStatistics, TeamInfo};
use crate::league::Season;
use chrono::NaiveDate;

const ZERO_APP_TRIVIAL_SEASON_SHARE: f64 = 0.35;

/// Stable enum kept for legacy callers / tests that constructed a
/// `CurrentSeasonEntry` directly. The spell model now classifies events
/// through `CareerEventKind`; this enum survives only to satisfy the
/// derived compatibility view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    InitialSeed,
    SeasonSeed,
    TransferIn,
    LoanIn,
    LoanReturn,
    FreeAgentSigning,
    SourceSnapshot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CareerSpellKind {
    Permanent,
    Loan,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CareerEventKind {
    InitialSeed,
    SeasonSeed,
    PermanentTransfer,
    ManualTransfer,
    LoanStart,
    LoanReturn,
    ReLoan,
    Release,
    FreeAgentSigning,
    SeasonSnapshot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootKind {
    None,
    CareerRoot,
}

/// Why a derived display row survives the visibility filter. One value
/// per surviving row; rows that map to `None` are dropped. New exceptions
/// belong here, not in scenario-specific sort closures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreserveReason {
    HasOfficialApps,
    ExplicitFee,
    ActiveSpell,
    FirstCareerClub,
    LongZeroAppSpell,
    OriginalClubAnchor,
}

struct ActiveLoanCtx {
    season_start_year: u16,
    parent_slug: String,
}

/// The source-of-truth record for a single career spell at one club, in
/// one season, of one kind (Permanent or Loan). Career history is a `Vec`
/// of these; everything else (`items`, `current`, `view_items`) is derived.
#[derive(Debug, Clone)]
pub struct PlayerCareerSpell {
    pub spell_id: u64,
    /// Monotonically-increasing per career — the most recent move always
    /// has the highest `movement_order`. Drives display ordering inside
    /// a season (newest action first) and breaks ties between same-season
    /// spells in `view_items`.
    pub movement_order: u64,

    pub season_start_year: u16,

    pub team_name: String,
    pub team_slug: String,
    pub team_reputation: u16,
    pub league_name: String,
    pub league_slug: String,

    pub kind: CareerSpellKind,
    /// For loan spells, the slug of the parent club that still owns the
    /// permanent contract. Pure metadata — never spawns a display row.
    pub parent_team_slug: Option<String>,

    pub joined_date: NaiveDate,
    pub departed_date: Option<NaiveDate>,

    pub opened_by: CareerEventKind,
    pub closed_by: Option<CareerEventKind>,

    pub transfer_fee: Option<f64>,
    pub statistics: PlayerStatistics,

    pub root_kind: RootKind,
}

#[derive(Debug, Clone)]
pub struct PlayerStatisticsHistory {
    /// The single source of truth — every event mutates this list.
    pub spells: Vec<PlayerCareerSpell>,

    /// Compatibility cache: closed spells projected to one item per
    /// `(year, slug, kind)` group. External readers (calculator, callup,
    /// templates) iterate this. Rebuilt on every mutation.
    pub items: Vec<PlayerStatisticsHistoryItem>,

    /// Compatibility cache: open spells projected to one entry. Tests
    /// historically asserted on `current[0]`; the field stays so they
    /// keep compiling. Rebuilt on every mutation.
    pub current: Vec<CurrentSeasonEntry>,

    next_spell_id: u64,
    next_movement_order: u64,
}

#[derive(Debug, Clone)]
pub struct PlayerStatisticsHistoryItem {
    pub season: Season,
    pub team_name: String,
    pub team_slug: String,
    pub team_reputation: u16,
    pub league_name: String,
    pub league_slug: String,
    pub is_loan: bool,
    pub transfer_fee: Option<f64>,
    pub statistics: PlayerStatistics,
    pub seq_id: u32,
}

#[derive(Debug, Clone)]
pub struct CurrentSeasonEntry {
    pub team_name: String,
    pub team_slug: String,
    pub team_reputation: u16,
    pub league_name: String,
    pub league_slug: String,
    pub is_loan: bool,
    pub transfer_fee: Option<f64>,
    pub statistics: PlayerStatistics,
    pub joined_date: NaiveDate,
    pub departed_date: Option<NaiveDate>,
    pub seq_id: u32,
    pub season_start_year: u16,
    pub kind: EntryKind,
    pub is_career_root: bool,
    pub parent_team_slug: Option<String>,
}

impl Default for PlayerStatisticsHistory {
    fn default() -> Self {
        Self::new()
    }
}

impl PlayerStatisticsHistory {
    pub fn new() -> Self {
        PlayerStatisticsHistory {
            spells: Vec::new(),
            items: Vec::new(),
            current: Vec::new(),
            next_spell_id: 0,
            next_movement_order: 0,
        }
    }

    /// Reconstruct a history from a flat list of frozen items. Used by the
    /// database loader and a few tests. Each item becomes one closed spell;
    /// the chronologically-earliest spell is marked `CareerRoot`. Old saves
    /// can carry impossible `(year, parent-club, Permanent, 0 apps, no fee)`
    /// rows alongside the same-year loan row — clean those up post-import
    /// so the projection doesn't render them.
    pub fn from_items(items: Vec<PlayerStatisticsHistoryItem>) -> Self {
        let mut h = Self::new();
        if items.is_empty() {
            return h;
        }

        let mut sorted = items;
        sorted.sort_by(|a, b| {
            a.season
                .start_year
                .cmp(&b.season.start_year)
                .then(a.seq_id.cmp(&b.seq_id))
        });

        for (idx, item) in sorted.into_iter().enumerate() {
            let spell_id = h.alloc_spell_id();
            let movement_order = h.alloc_movement_order();
            let season = Season::new(item.season.start_year);
            h.spells.push(PlayerCareerSpell {
                spell_id,
                movement_order,
                season_start_year: item.season.start_year,
                team_name: item.team_name,
                team_slug: item.team_slug,
                team_reputation: item.team_reputation,
                league_name: item.league_name,
                league_slug: item.league_slug,
                kind: if item.is_loan {
                    CareerSpellKind::Loan
                } else {
                    CareerSpellKind::Permanent
                },
                parent_team_slug: None,
                joined_date: season.start_date(),
                departed_date: Some(season.end_date()),
                opened_by: CareerEventKind::SeasonSeed,
                closed_by: Some(CareerEventKind::SeasonSnapshot),
                transfer_fee: item.transfer_fee,
                statistics: item.statistics,
                root_kind: if idx == 0 {
                    RootKind::CareerRoot
                } else {
                    RootKind::None
                },
            });
        }
        h.normalize_legacy_spells();
        h.refresh_compat();
        h
    }

    /// Drop impossible "parent placeholder" rows imported from legacy
    /// saves — `(year, slug, Permanent, 0 apps, no fee, non-root, brief
    /// stint)` spells that exist in the same season as a Loan spell. The
    /// pre-spell model used to seed these as a side-effect; the spell
    /// model never produces them. Keeps cleanly-saved data untouched.
    fn normalize_legacy_spells(&mut self) {
        let to_remove: Vec<u64> = self
            .spells
            .iter()
            .filter(|s| {
                if !matches!(s.kind, CareerSpellKind::Permanent) {
                    return false;
                }
                if s.statistics.total_games() > 0 {
                    return false;
                }
                if s.transfer_fee.is_some() {
                    return false;
                }
                if matches!(s.root_kind, RootKind::CareerRoot) {
                    return false;
                }
                // Only auto-seeded rows are removable. Real transfer /
                // loan-return events stay.
                if !matches!(
                    s.opened_by,
                    CareerEventKind::SeasonSeed | CareerEventKind::InitialSeed
                ) {
                    return false;
                }
                self.spells.iter().any(|loan| {
                    loan.season_start_year == s.season_start_year
                        && matches!(loan.kind, CareerSpellKind::Loan)
                        && self.parent_slug_for_loan_spell(loan).as_deref()
                            == Some(s.team_slug.as_str())
                })
            })
            .map(|s| s.spell_id)
            .collect();
        if to_remove.is_empty() {
            return;
        }
        self.spells.retain(|s| !to_remove.contains(&s.spell_id));
    }

    pub fn is_empty(&self) -> bool {
        self.spells.is_empty()
    }

    pub fn needs_current_season_seed(&self) -> bool {
        self.active_spell().is_none()
    }

    fn alloc_spell_id(&mut self) -> u64 {
        let id = self.next_spell_id;
        self.next_spell_id += 1;
        id
    }

    fn alloc_movement_order(&mut self) -> u64 {
        let mo = self.next_movement_order;
        self.next_movement_order += 1;
        mo
    }

    fn season_year_for(date: NaiveDate) -> u16 {
        Season::from_date(date).start_year
    }

    /// Sole open spell, or `None` if the player has no live attachment
    /// (free agent, retired, or just snapshot-closed). The model is
    /// invariant: at most one spell is open at any time.
    pub fn active_spell(&self) -> Option<&PlayerCareerSpell> {
        self.spells.iter().find(|s| s.departed_date.is_none())
    }

    pub fn active_spell_mut(&mut self) -> Option<&mut PlayerCareerSpell> {
        self.spells.iter_mut().find(|s| s.departed_date.is_none())
    }

    pub fn active_team_slug(&self) -> Option<&str> {
        self.active_spell().map(|s| s.team_slug.as_str())
    }

    fn kind_for(is_loan: bool) -> CareerSpellKind {
        if is_loan {
            CareerSpellKind::Loan
        } else {
            CareerSpellKind::Permanent
        }
    }

    fn open_spell(
        &mut self,
        team: &TeamInfo,
        season_start_year: u16,
        kind: CareerSpellKind,
        parent_team_slug: Option<String>,
        joined_date: NaiveDate,
        transfer_fee: Option<f64>,
        opened_by: CareerEventKind,
        root_kind: RootKind,
    ) -> u64 {
        let spell_id = self.alloc_spell_id();
        let movement_order = self.alloc_movement_order();
        self.spells.push(PlayerCareerSpell {
            spell_id,
            movement_order,
            season_start_year,
            team_name: team.name.clone(),
            team_slug: team.slug.clone(),
            team_reputation: team.reputation,
            league_name: team.league_name.clone(),
            league_slug: team.league_slug.clone(),
            kind,
            parent_team_slug,
            joined_date,
            departed_date: None,
            opened_by,
            closed_by: None,
            transfer_fee,
            statistics: PlayerStatistics::default(),
            root_kind,
        });
        spell_id
    }

    fn close_spell(
        &mut self,
        spell_id: u64,
        date: NaiveDate,
        closed_by: CareerEventKind,
        stats: PlayerStatistics,
    ) {
        if let Some(s) = self.spells.iter_mut().find(|s| s.spell_id == spell_id) {
            Self::merge_stats_into(&mut s.statistics, stats);
            // `departed_date` must always be ≥ `joined_date`. Snapshot
            // closes happening after a stale joined_date (e.g. a parent
            // spell reopened in the summer break and closed at the next
            // season-end) would otherwise produce negative spell durations.
            let bounded = date.max(s.joined_date);
            s.departed_date = Some(bounded);
            s.closed_by = Some(closed_by);
        }
    }

    fn close_active_with(
        &mut self,
        date: NaiveDate,
        closed_by: CareerEventKind,
        stats: PlayerStatistics,
    ) -> Option<u64> {
        let id = self.active_spell().map(|s| s.spell_id);
        if let Some(id) = id {
            self.close_spell(id, date, closed_by, stats);
        }
        id
    }

    fn merge_stats_into(target: &mut PlayerStatistics, incoming: PlayerStatistics) {
        if incoming.total_games() == 0 {
            return;
        }
        if target.total_games() == 0 {
            *target = incoming;
        } else {
            target.merge_from(&incoming);
        }
    }

    /// Same as `merge_stats_into` but always merges (used for explicit
    /// per-season folds where the incoming stats might be 0 by intent).
    fn merge_stats_force(target: &mut PlayerStatistics, incoming: &PlayerStatistics) {
        if incoming.total_games() == 0 {
            return;
        }
        if target.total_games() == 0 {
            *target = incoming.clone();
        } else {
            target.merge_from(incoming);
        }
    }

    fn drop_phantom_forward_spell(&mut self, spell_id: u64) {
        self.spells.retain(|s| s.spell_id != spell_id);
    }

    /// Find the most-recently-opened spell matching `(year, slug, kind)`.
    fn find_spell_for(
        &mut self,
        year: u16,
        slug: &str,
        kind: CareerSpellKind,
    ) -> Option<&mut PlayerCareerSpell> {
        self.spells
            .iter_mut()
            .rev()
            .find(|s| s.season_start_year == year && s.team_slug == slug && s.kind == kind)
    }

    fn has_spell_for(&self, year: u16, slug: &str, kind: CareerSpellKind) -> bool {
        self.spells
            .iter()
            .any(|s| s.season_start_year == year && s.team_slug == slug && s.kind == kind)
    }

    // ── Mid-season events ─────────────────────────────────

    pub fn record_transfer(
        &mut self,
        old_stats: PlayerStatistics,
        from: &TeamInfo,
        to: &TeamInfo,
        fee: f64,
        date: NaiveDate,
    ) {
        let year = Self::season_year_for(date);
        // Close whatever spell the player currently lives in. For a
        // permanent transfer that's the parent perm spell; the active is
        // accurate even when the source club passed in is stale.
        self.close_active_with(date, CareerEventKind::PermanentTransfer, old_stats);
        // Make sure a row for the source club + season exists when the
        // active spell didn't match (rare — generated/fresh players whose
        // initial seed wasn't aligned with the transferring `from`).
        self.ensure_source_spell_exists(from, year);
        self.open_spell(
            to,
            year,
            CareerSpellKind::Permanent,
            None,
            date,
            Some(fee),
            CareerEventKind::PermanentTransfer,
            RootKind::None,
        );
        self.refresh_compat();
    }

    pub fn record_loan(
        &mut self,
        old_stats: PlayerStatistics,
        from: &TeamInfo,
        to: &TeamInfo,
        loan_fee: f64,
        date: NaiveDate,
    ) {
        let year = Self::season_year_for(date);
        self.close_active_with(date, CareerEventKind::LoanStart, old_stats);
        self.ensure_source_spell_exists(from, year);
        self.open_spell(
            to,
            year,
            CareerSpellKind::Loan,
            Some(from.slug.clone()),
            date,
            Some(loan_fee),
            CareerEventKind::LoanStart,
            RootKind::None,
        );
        self.refresh_compat();
    }

    pub fn record_loan_return(
        &mut self,
        remaining_stats: PlayerStatistics,
        borrowing: &TeamInfo,
        parent: &TeamInfo,
        date: NaiveDate,
    ) {
        let stats_year = Self::season_year_for(date);
        let parent_year = Self::season_year_for_parent_event(date);

        // Where do the remaining stats and the close-event belong?
        let active_id = self.active_spell().map(|s| s.spell_id);
        let active_meta = self
            .active_spell()
            .map(|s| (s.season_start_year, s.team_slug.clone(), s.kind));

        match (active_id, active_meta) {
            (Some(id), Some((year, slug, kind))) if slug == borrowing.slug && kind == CareerSpellKind::Loan => {
                if year == stats_year {
                    // Same-season loan still open: simple close.
                    self.close_spell(id, date, CareerEventKind::LoanReturn, remaining_stats);
                } else {
                    // Forward-looking continuation seed — drop it and fold
                    // the remaining stats into the matching prior loan
                    // spell at `stats_year`. Materialise one if missing
                    // (cross-country: borrowing snapshot may have closed
                    // it; if no games were ever played there we still need
                    // a real spell to attach the close event to).
                    let phantom_no_history = self
                        .spells
                        .iter()
                        .find(|s| s.spell_id == id)
                        .map(|s| s.statistics.total_games() == 0 && s.transfer_fee.is_none())
                        .unwrap_or(false);
                    if phantom_no_history {
                        self.drop_phantom_forward_spell(id);
                    } else {
                        self.close_spell(id, date, CareerEventKind::LoanReturn, PlayerStatistics::default());
                    }
                    let needs_materialise = !self.has_spell_for(
                        stats_year,
                        &borrowing.slug,
                        CareerSpellKind::Loan,
                    );
                    if needs_materialise {
                        let season = Season::new(stats_year);
                        let new_id = self.open_spell(
                            borrowing,
                            stats_year,
                            CareerSpellKind::Loan,
                            Some(parent.slug.clone()),
                            season.start_date(),
                            None,
                            CareerEventKind::SeasonSeed,
                            RootKind::None,
                        );
                        self.close_spell(new_id, date, CareerEventKind::LoanReturn, remaining_stats);
                    } else if let Some(prior) =
                        self.find_spell_for(stats_year, &borrowing.slug, CareerSpellKind::Loan)
                    {
                        Self::merge_stats_force(&mut prior.statistics, &remaining_stats);
                        if prior.parent_team_slug.is_none() {
                            prior.parent_team_slug = Some(parent.slug.clone());
                        }
                    }
                }
            }
            _ => {
                // No matching active loan spell — uncommon. If remaining
                // stats exist, fold them into the closest prior borrowing
                // loan spell so games are not lost.
                if remaining_stats.total_games() > 0 {
                    if let Some(prior) =
                        self.find_spell_for(stats_year, &borrowing.slug, CareerSpellKind::Loan)
                    {
                        Self::merge_stats_force(&mut prior.statistics, &remaining_stats);
                    }
                }
            }
        }

        // Always open a fresh parent permanent spell on return. This
        // gives the parent row a higher `movement_order` than the just-
        // closed loan, so the projection sorts it on top. Use
        // `parent_year` so a return landing in summer break (June/July)
        // opens the spell at the upcoming season.
        self.open_spell(
            parent,
            parent_year,
            CareerSpellKind::Permanent,
            None,
            date,
            None,
            CareerEventKind::LoanReturn,
            RootKind::None,
        );
        self.refresh_compat();
    }

    pub fn record_cancel_loan(
        &mut self,
        old_stats: PlayerStatistics,
        borrowing: &TeamInfo,
        parent: &TeamInfo,
        _is_loan: bool,
        date: NaiveDate,
    ) {
        self.record_loan_return(old_stats, borrowing, parent, date);
    }

    pub fn record_release(
        &mut self,
        last_stats: PlayerStatistics,
        _from: &TeamInfo,
        date: NaiveDate,
    ) {
        self.close_active_with(date, CareerEventKind::Release, last_stats);
        self.refresh_compat();
    }

    pub fn record_free_agent_signing(
        &mut self,
        last_stats: PlayerStatistics,
        to: &TeamInfo,
        date: NaiveDate,
    ) {
        // Any still-open spell becomes the source of the player's last
        // game tally before joining the new club. Close it so the games
        // stay attributed to that club, not the new free signing row.
        self.close_active_with(date, CareerEventKind::Release, last_stats);

        let year = Self::season_year_for(date);
        let root = if self.spells.is_empty() {
            RootKind::CareerRoot
        } else {
            RootKind::None
        };
        self.open_spell(
            to,
            year,
            CareerSpellKind::Permanent,
            None,
            date,
            Some(0.0),
            CareerEventKind::FreeAgentSigning,
            root,
        );
        self.refresh_compat();
    }

    pub fn record_departure_transfer(
        &mut self,
        old_stats: PlayerStatistics,
        from: &TeamInfo,
        to: &TeamInfo,
        fee: Option<f64>,
        _is_loan: bool,
        date: NaiveDate,
    ) {
        let year = Self::season_year_for(date);
        self.close_active_with(date, CareerEventKind::ManualTransfer, old_stats);
        self.ensure_source_spell_exists(from, year);
        self.open_spell(
            to,
            year,
            CareerSpellKind::Permanent,
            None,
            date,
            fee,
            CareerEventKind::ManualTransfer,
            RootKind::None,
        );
        self.refresh_compat();
    }

    pub fn record_departure_loan(
        &mut self,
        old_stats: PlayerStatistics,
        from: &TeamInfo,
        parent: &TeamInfo,
        to: &TeamInfo,
        is_loan: bool,
        date: NaiveDate,
    ) {
        let year = Self::season_year_for(date);
        let opened_by = if is_loan {
            CareerEventKind::ReLoan
        } else {
            CareerEventKind::LoanStart
        };
        self.close_active_with(date, opened_by, old_stats);
        self.ensure_source_spell_exists(from, year);
        self.open_spell(
            to,
            year,
            CareerSpellKind::Loan,
            Some(parent.slug.clone()),
            date,
            Some(0.0),
            opened_by,
            RootKind::None,
        );
        self.refresh_compat();
    }

    /// When a transfer/loan event names a `from` club for which no spell
    /// exists yet (player is fresh from generation, or the seed never
    /// matched), insert a closed-immediately stub so the source club is
    /// still represented. Cheap and idempotent.
    fn ensure_source_spell_exists(&mut self, from: &TeamInfo, year: u16) {
        if from.slug.is_empty() {
            return;
        }
        if self.has_spell_for(year, &from.slug, CareerSpellKind::Permanent)
            || self.has_spell_for(year, &from.slug, CareerSpellKind::Loan)
        {
            return;
        }
        // No-op: the spell model attributes stats to whatever spell was
        // active. If `from` didn't exist as a spell, those games belonged
        // to whatever the active spell actually was — fabricating a
        // synthetic row would invent history.
    }

    // ── Season end ───────────────────────────────────────

    pub fn record_season_end(
        &mut self,
        season: Season,
        current_stats: PlayerStatistics,
        team: &TeamInfo,
        is_loan: bool,
        last_transfer_date: Option<NaiveDate>,
    ) {
        let target_year = season.start_year;
        let snapshot_close_date = season.end_date();

        // If an active spell already covers this (year, team), its kind
        // is the source of truth — overrides the `is_loan` flag from the
        // caller. The flag can lag the model when the contract_loan field
        // wasn't set by the caller before snapshot, but the underlying
        // spell knows what it is.
        let kind = match self.active_spell() {
            Some(s) if s.team_slug == team.slug && s.season_start_year == target_year => s.kind,
            _ => Self::kind_for(is_loan),
        };

        // If the active spell is a loan at a different club, this is a
        // parent-side snapshot for a player on loan elsewhere — no-op,
        // unconditionally. Parent ownership is metadata on the loan
        // spell, not active state. The borrowing club's snapshot is the
        // sole authority on the loanee's season stats; folding live loan
        // stats onto a parent spell (whether one already exists from a
        // pre-loan stint or not) would migrate apps off the loan club.
        let active_meta = self
            .active_spell()
            .map(|s| (s.team_slug.clone(), s.kind, s.season_start_year, s.spell_id));

        if let Some((active_slug, active_kind, _, _)) = active_meta.clone() {
            if active_kind == CareerSpellKind::Loan && active_slug != team.slug {
                self.refresh_compat();
                return;
            }
        }

        // 1. Land the stats on the right spell.
        match active_meta {
            Some((slug, k, year, id))
                if slug == team.slug && k == kind && year == target_year =>
            {
                // Standard case: snapshot+close the matching active spell.
                self.close_spell(
                    id,
                    snapshot_close_date,
                    CareerEventKind::SeasonSnapshot,
                    current_stats.clone(),
                );
            }
            Some((slug, k, year, id))
                if slug == team.slug && k == kind && year < target_year =>
            {
                // Active spell is stale (prior season). Close it at its
                // own season end, then create a fresh target-year spell
                // and snapshot into it.
                let prev_season = Season::new(year);
                self.close_spell(
                    id,
                    prev_season.end_date(),
                    CareerEventKind::SeasonSnapshot,
                    PlayerStatistics::default(),
                );
                self.snapshot_into_or_create(
                    team,
                    target_year,
                    kind,
                    last_transfer_date,
                    snapshot_close_date,
                    current_stats.clone(),
                );
            }
            Some((slug, k, year, _))
                if slug == team.slug && k == kind && year > target_year =>
            {
                // Active is forward-looking (e.g. a continuation seed
                // already opened for next year, or the simulator seeded
                // the player at the current calendar season while the
                // snapshot is recording the prior one). Materialise or
                // fold into the target-year spell without disturbing
                // the forward continuation.
                let _ = (slug, k, year);
                self.snapshot_into_or_create(
                    team,
                    target_year,
                    kind,
                    last_transfer_date,
                    snapshot_close_date,
                    current_stats.clone(),
                );
            }
            _ => {
                // No matching active spell — could be the very first
                // snapshot for this team or a duplicate-snapshot for a
                // year whose active spell was already closed/diverged.
                self.snapshot_into_or_create(
                    team,
                    target_year,
                    kind,
                    last_transfer_date,
                    snapshot_close_date,
                    current_stats.clone(),
                );
            }
        }

        // 2. Open a continuation for next season at the same team/kind,
        //    if the player is still here (no other spell already covers
        //    the next year). Carry the parent slug forward for loans so
        //    re-snapshots see the parent without re-deriving it. Gate on
        //    `kind` (already corrected from the active spell), not on
        //    the caller's `is_loan` — that flag can lag the model when
        //    `contract_loan` hasn't been set yet by the caller, and
        //    losing the parent slug here disables the projection-side
        //    suppression that hides the parent's same-season placeholder.
        let next_year = target_year + 1;
        if !self.has_spell_for(next_year, &team.slug, kind) && self.active_spell().is_none() {
            let parent_slug = if matches!(kind, CareerSpellKind::Loan) {
                self.spells
                    .iter()
                    .rev()
                    .find(|s| {
                        s.season_start_year == target_year
                            && s.team_slug == team.slug
                            && s.kind == CareerSpellKind::Loan
                    })
                    .and_then(|s| s.parent_team_slug.clone())
            } else {
                None
            };
            self.open_spell(
                team,
                next_year,
                kind,
                parent_slug,
                Season::new(next_year).start_date(),
                None,
                CareerEventKind::SeasonSeed,
                RootKind::None,
            );
        }

        self.refresh_compat();
    }

    /// Open a new (year, team, kind) spell, snapshot stats into it, then
    /// close it. Used when the snapshot can't piggy-back on an already-
    /// open spell.
    fn snapshot_into_or_create(
        &mut self,
        team: &TeamInfo,
        target_year: u16,
        kind: CareerSpellKind,
        last_transfer_date: Option<NaiveDate>,
        snapshot_close_date: NaiveDate,
        current_stats: PlayerStatistics,
    ) {
        // If a spell already exists for (target_year, team, kind), fold
        // into it (handles multi-league duplicate snapshots).
        if self.has_spell_for(target_year, &team.slug, kind) {
            self.fold_into_closed(
                target_year,
                &team.slug,
                kind,
                &current_stats,
                snapshot_close_date,
            );
            return;
        }
        let join_date = last_transfer_date
            .filter(|d| Self::season_year_for(*d) == target_year)
            .unwrap_or_else(|| Season::new(target_year).start_date());
        // Gate parent-slug carry on the resolved `kind`, not the caller's
        // `is_loan` flag — same reason as the continuation block in
        // `record_season_end`.
        let parent_slug = if matches!(kind, CareerSpellKind::Loan) {
            self.spells
                .iter()
                .rev()
                .find(|s| s.kind == CareerSpellKind::Loan && s.team_slug == team.slug)
                .and_then(|s| s.parent_team_slug.clone())
        } else {
            None
        };
        let spell_id = self.open_spell(
            team,
            target_year,
            kind,
            parent_slug,
            join_date,
            None,
            CareerEventKind::SeasonSeed,
            RootKind::None,
        );
        self.close_spell(
            spell_id,
            snapshot_close_date,
            CareerEventKind::SeasonSnapshot,
            current_stats,
        );
    }

    fn fold_into_closed(
        &mut self,
        year: u16,
        slug: &str,
        kind: CareerSpellKind,
        current_stats: &PlayerStatistics,
        snapshot_close_date: NaiveDate,
    ) {
        if let Some(prior) = self.find_spell_for(year, slug, kind) {
            Self::merge_stats_force(&mut prior.statistics, current_stats);
            // Keep the latest snapshot date so long-stint and ordering
            // reads see the most recent close.
            if let Some(d) = prior.departed_date {
                if snapshot_close_date > d {
                    prior.departed_date = Some(snapshot_close_date);
                }
            } else {
                prior.departed_date = Some(snapshot_close_date);
            }
            if prior.closed_by.is_none() {
                prior.closed_by = Some(CareerEventKind::SeasonSnapshot);
            }
        }
    }

    /// Compute the season year a parent-side event (loan return, free
    /// signing) belongs to. Summer-break dates (June/July) belong to the
    /// upcoming season — not the just-finished one — so a parent spell
    /// opened on `2026-06-01` lives in 2026/27, not 2025/26.
    fn season_year_for_parent_event(date: NaiveDate) -> u16 {
        let year = Self::season_year_for(date);
        let season_end = Season::new(year).end_date();
        if date > season_end { year + 1 } else { year }
    }

    // ── Initial seeding ───────────────────────────────────

    pub fn seed_initial_team(&mut self, team: &TeamInfo, date: NaiveDate, is_loan: bool) {
        if self.active_spell().is_some() {
            return;
        }
        let year = Self::season_year_for(date);
        let kind = Self::kind_for(is_loan);
        if self.has_spell_for(year, &team.slug, kind) {
            return;
        }
        let root = if self.spells.is_empty() {
            RootKind::CareerRoot
        } else {
            RootKind::None
        };
        self.open_spell(
            team,
            year,
            kind,
            None,
            date,
            None,
            CareerEventKind::InitialSeed,
            root,
        );
        self.refresh_compat();
    }

    // ── Projection / view ────────────────────────────────

    /// Project spells to a deterministic display list.
    pub fn view_items(
        &self,
        live_stats: Option<&PlayerStatistics>,
    ) -> Vec<PlayerStatisticsHistoryItem> {
        use std::collections::BTreeMap;

        let active_id = self.active_spell().map(|s| s.spell_id);
        let chronological_root = self.chronological_root_key();
        let active_loan_ctx: Option<ActiveLoanCtx> = self.active_spell().and_then(|s| {
            if matches!(s.kind, CareerSpellKind::Loan) {
                self.inferred_active_loan_parent_slug(s)
                    .map(|parent_slug| ActiveLoanCtx {
                        season_start_year: s.season_start_year,
                        parent_slug,
                    })
            } else {
                None
            }
        });

        // Group spells by (year, slug, kind).
        type Key = (u16, String, CareerSpellKind);
        let mut groups: BTreeMap<Key, Vec<&PlayerCareerSpell>> = BTreeMap::new();
        for s in &self.spells {
            groups
                .entry((s.season_start_year, s.team_slug.clone(), s.kind))
                .or_default()
                .push(s);
        }

        let mut result: Vec<PlayerStatisticsHistoryItem> = Vec::with_capacity(groups.len());
        for ((year, slug, kind), spells) in groups {
            // Aggregate stats — overlay live_stats on the active spell.
            let mut stats = PlayerStatistics::default();
            for s in &spells {
                let mut s_stats = s.statistics.clone();
                if Some(s.spell_id) == active_id {
                    if let Some(live) = live_stats {
                        s_stats = live.clone();
                    }
                }
                Self::merge_stats_force(&mut stats, &s_stats);
            }

            let display_order = spells.iter().map(|s| s.movement_order).max().unwrap();
            let primary = spells
                .iter()
                .max_by_key(|s| s.movement_order)
                .copied()
                .unwrap();

            // Fee: any Some(_) wins, including Some(0.0); pick the most
            // recent fee-bearing spell so a re-transfer overrides a stale
            // initial entry.
            let transfer_fee = spells
                .iter()
                .filter(|s| s.transfer_fee.is_some())
                .max_by_key(|s| s.movement_order)
                .and_then(|s| s.transfer_fee);

            // Defensive: while a loan is active, the parent club may
            // never display a phantom permanent row in the same season —
            // even if a stale spell slipped into `spells` from an old
            // save or a buggy mutator. Apps / fee / root-season
            // exceptions still let legitimate parent rows through (e.g.
            // the pre-loan stint at the parent in the root season).
            if Self::is_phantom_parent_during_active_loan(
                year,
                &slug,
                kind,
                active_loan_ctx.as_ref(),
                &stats,
                transfer_fee,
                &spells,
                chronological_root,
            ) {
                continue;
            }

            // Broader rule: a 0-app, no-fee parent placeholder is
            // suppressed whenever ANY same-season loan (active OR
            // closed) names this club as its parent. Runs before
            // `group_visibility` so `LongZeroAppSpell` can't keep a
            // fictional parent row alive.
            if self.is_phantom_parent_for_any_same_season_loan(
                year,
                &slug,
                kind,
                &stats,
                transfer_fee,
                &spells,
                chronological_root,
            ) {
                continue;
            }

            // Visibility: the group survives if any reason holds.
            if Self::group_visibility(&spells, &stats, transfer_fee, active_id, chronological_root)
                .is_none()
            {
                continue;
            }

            result.push(PlayerStatisticsHistoryItem {
                season: Season::new(year),
                team_name: primary.team_name.clone(),
                team_slug: slug,
                team_reputation: primary.team_reputation,
                league_name: primary.league_name.clone(),
                league_slug: primary.league_slug.clone(),
                is_loan: matches!(kind, CareerSpellKind::Loan),
                transfer_fee,
                statistics: stats,
                seq_id: display_order as u32,
            });
        }

        result.retain(|i| !i.team_slug.is_empty());

        // Original-parent anchors. Parent ownership is not a recurring
        // display row; show it once, in the parent's first loan season,
        // when no real Permanent row already covers that pair. Synthetic —
        // never lands in `spells`, never opens a spell, never carries live
        // stats. Runs after the suppression-driven projection so a real
        // parent spell (root, loan return, with apps, with fee) always
        // wins over the synthesized anchor.
        let anchors = self.synthesize_original_club_anchors(&result);
        result.extend(anchors);

        // Sort: season DESC, display_order DESC, slug ASC.
        result.sort_by(|a, b| {
            b.season
                .start_year
                .cmp(&a.season.start_year)
                .then(b.seq_id.cmp(&a.seq_id))
                .then(a.team_slug.cmp(&b.team_slug))
        });

        // played_subs collapses into played on every row except the
        // newest (the active spell), matching how FM-style tables read.
        if let Some(max_seq) = result.iter().map(|i| i.seq_id).max() {
            for item in &mut result {
                if item.seq_id != max_seq && item.statistics.played_subs > 0 {
                    item.statistics.played += item.statistics.played_subs;
                    item.statistics.played_subs = 0;
                }
            }
        }

        result
    }

    /// Resolve the parent club slug for any loan spell — active or
    /// closed. Trusts `parent_team_slug` first, then falls back to the
    /// most-recently-opened Permanent spell that predates the loan.
    /// Used by both the projection suppressor and the legacy normalizer.
    fn parent_slug_for_loan_spell(&self, loan: &PlayerCareerSpell) -> Option<String> {
        if let Some(parent) = loan.parent_team_slug.clone() {
            return Some(parent);
        }
        self.spells
            .iter()
            .filter(|s| {
                matches!(s.kind, CareerSpellKind::Permanent)
                    && s.movement_order < loan.movement_order
            })
            .max_by_key(|s| s.movement_order)
            .map(|s| s.team_slug.clone())
    }

    fn inferred_active_loan_parent_slug(
        &self,
        active: &PlayerCareerSpell,
    ) -> Option<String> {
        self.parent_slug_for_loan_spell(active)
    }

    /// One synthetic display row per parent slug, anchored at that
    /// parent's chronologically-earliest loan. Suppressed when the
    /// projection already shows a real Permanent row for the same
    /// (year, parent) — pre-loan stint, loan return, or any other real
    /// signing keeps its rightful row, the anchor only fills the gap.
    /// Rows produced here are display-only and have no backing spell.
    fn synthesize_original_club_anchors(
        &self,
        items: &[PlayerStatisticsHistoryItem],
    ) -> Vec<PlayerStatisticsHistoryItem> {
        use std::collections::HashMap;

        let mut earliest: HashMap<String, &PlayerCareerSpell> = HashMap::new();
        for s in &self.spells {
            if !matches!(s.kind, CareerSpellKind::Loan) {
                continue;
            }
            let parent = match self.parent_slug_for_loan_spell(s) {
                Some(p) if !p.is_empty() && p != s.team_slug => p,
                _ => continue,
            };
            let s_key = (s.season_start_year, s.joined_date, s.movement_order);
            match earliest.get(&parent) {
                Some(prev) => {
                    let prev_key =
                        (prev.season_start_year, prev.joined_date, prev.movement_order);
                    if s_key < prev_key {
                        earliest.insert(parent, s);
                    }
                }
                None => {
                    earliest.insert(parent, s);
                }
            }
        }

        let mut anchors = Vec::new();
        for (parent_slug, loan) in earliest {
            let year = loan.season_start_year;
            if items.iter().any(|i| {
                i.season.start_year == year && i.team_slug == parent_slug && !i.is_loan
            }) {
                continue;
            }

            let meta = self
                .spells
                .iter()
                .filter(|s| s.team_slug == parent_slug)
                .max_by_key(|s| s.movement_order);
            let (team_name, team_reputation, league_name, league_slug) = match meta {
                Some(s) => (
                    s.team_name.clone(),
                    s.team_reputation,
                    s.league_name.clone(),
                    s.league_slug.clone(),
                ),
                None => (parent_slug.clone(), 0, String::new(), String::new()),
            };

            // Slot the anchor just below the loan row in the same season,
            // so the seq_id-DESC sort places loan first, anchor second.
            let anchor_seq_id = loan.movement_order.saturating_sub(1) as u32;

            anchors.push(PlayerStatisticsHistoryItem {
                season: Season::new(year),
                team_name,
                team_slug: parent_slug,
                team_reputation,
                league_name,
                league_slug,
                is_loan: false,
                transfer_fee: None,
                statistics: PlayerStatistics::default(),
                seq_id: anchor_seq_id,
            });
        }
        anchors
    }

    /// Identify the chronologically-earliest spell as `(season_start_year,
    /// spell_id)`. Used to gate `RootKind::CareerRoot` preservation —
    /// insertion order alone is unreliable because earlier seasons can be
    /// backfilled after a later root-marked spell already exists. A
    /// `CareerRoot` flag only protects a row when that row is also the
    /// chronological root.
    fn chronological_root_key(&self) -> Option<(u16, u64)> {
        self.spells
            .iter()
            .min_by_key(|s| (s.season_start_year, s.joined_date, s.movement_order))
            .map(|s| (s.season_start_year, s.spell_id))
    }

    fn has_effective_root(
        spells: &[&PlayerCareerSpell],
        chronological_root: Option<(u16, u64)>,
    ) -> bool {
        spells.iter().any(|s| {
            matches!(s.root_kind, RootKind::CareerRoot)
                && chronological_root == Some((s.season_start_year, s.spell_id))
        })
    }

    /// Broader projection-side suppression. A 0-app, no-fee, non-root,
    /// non-loan-return Permanent group is hidden whenever ANY loan spell
    /// in the same season identifies this club as its parent. Doesn't
    /// require the loan to still be active — covers the case where the
    /// active spell is the parent's continuation rather than the loan,
    /// or where the loan was just closed and a stale placeholder
    /// survived. Runs BEFORE `group_visibility` so `LongZeroAppSpell`
    /// can't keep a fictional parent row.
    fn is_phantom_parent_for_any_same_season_loan(
        &self,
        group_year: u16,
        group_slug: &str,
        group_kind: CareerSpellKind,
        stats: &PlayerStatistics,
        fee: Option<f64>,
        spells: &[&PlayerCareerSpell],
        chronological_root: Option<(u16, u64)>,
    ) -> bool {
        if group_kind != CareerSpellKind::Permanent {
            return false;
        }
        if stats.total_games() > 0 {
            return false;
        }
        // Any explicit `transfer_fee` (including `Some(0.0)`) marks a
        // real signing event — paid transfer, free transfer, or free-
        // agent contract. Auto-seeded placeholders carry `None`.
        if fee.is_some() {
            return false;
        }
        // CareerRoot protects the root's own season unconditionally —
        // even when the player was loaned out the same day they were
        // seeded ("root team replaced by loan team"). The root row is
        // the user's career anchor at this club; the same-season loan
        // is the contextual replacement, not a reason to hide the
        // anchor. Stale CareerRoot flags on a later season (an earlier
        // spell was backfilled afterwards) do NOT count — only the
        // chronological root deserves protection.
        if Self::has_effective_root(spells, chronological_root) {
            return false;
        }
        // Only auto-seeded rows are eligible for suppression. Manual /
        // pipeline / loan-return events leave a richer `opened_by` and
        // must never be hidden — the user did something real.
        if !spells.iter().all(|s| {
            matches!(
                s.opened_by,
                CareerEventKind::SeasonSeed | CareerEventKind::InitialSeed
            )
        }) {
            return false;
        }
        self.spells.iter().any(|loan| {
            loan.season_start_year == group_year
                && matches!(loan.kind, CareerSpellKind::Loan)
                && self.parent_slug_for_loan_spell(loan).as_deref() == Some(group_slug)
        })
    }

    /// Diagnostic dump of every spell, for logging in real-save
    /// debugging without leaning on test-only `describe_spells`.
    pub fn debug_spell_dump(&self) -> String {
        self.spells
            .iter()
            .map(|s| {
                format!(
                    "id={} mo={} season={} club={} kind={:?} parent={:?} joined={} departed={:?} apps={} subs={} fee={:?} root={:?} opened={:?} closed={:?}",
                    s.spell_id,
                    s.movement_order,
                    s.season_start_year,
                    s.team_slug,
                    s.kind,
                    s.parent_team_slug,
                    s.joined_date,
                    s.departed_date,
                    s.statistics.played,
                    s.statistics.played_subs,
                    s.transfer_fee,
                    s.root_kind,
                    s.opened_by,
                    s.closed_by,
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Hide phantom 0-app, no-fee, non-root, brief-stint Permanent groups
    /// that sit on the active loan's parent club in the same season.
    /// Defensive: catches stale spells the model shouldn't have produced
    /// (old saves, future-pipeline bugs) before they reach the UI. The
    /// real fix lives at creation time; this is the second line.
    fn is_phantom_parent_during_active_loan(
        group_year: u16,
        group_slug: &str,
        group_kind: CareerSpellKind,
        active_loan_ctx: Option<&ActiveLoanCtx>,
        stats: &PlayerStatistics,
        fee: Option<f64>,
        spells: &[&PlayerCareerSpell],
        chronological_root: Option<(u16, u64)>,
    ) -> bool {
        let ctx = match active_loan_ctx {
            Some(c) => c,
            None => return false,
        };
        if group_kind != CareerSpellKind::Permanent {
            return false;
        }
        if ctx.season_start_year != group_year {
            return false;
        }
        if ctx.parent_slug != group_slug {
            return false;
        }
        if stats.total_games() > 0 {
            return false;
        }
        if fee.is_some() {
            return false;
        }
        // Career-root protects the root's own season — but only when
        // this group really is the chronological root. A later
        // root-marked parent placeholder, with an earlier spell
        // backfilled afterwards, is not anchoring anything.
        if Self::has_effective_root(spells, chronological_root) {
            return false;
        }
        // Long-stint exception from `group_visibility` does NOT apply
        // here — the player is on loan elsewhere for that whole stint,
        // so the parent's apparent presence is fictional by construction.
        true
    }

    fn group_visibility(
        spells: &[&PlayerCareerSpell],
        aggregated: &PlayerStatistics,
        fee: Option<f64>,
        active_id: Option<u64>,
        chronological_root: Option<(u16, u64)>,
    ) -> Option<PreserveReason> {
        if aggregated.total_games() > 0 {
            return Some(PreserveReason::HasOfficialApps);
        }
        if fee.is_some() {
            return Some(PreserveReason::ExplicitFee);
        }
        if let Some(active_id) = active_id {
            if spells.iter().any(|s| s.spell_id == active_id) {
                return Some(PreserveReason::ActiveSpell);
            }
        }
        // Career-root preservation only applies in the root spell's own
        // season AND only when this group is the chronological root.
        // Insertion order alone is unreliable: an earlier spell can be
        // backfilled after a later one was already marked CareerRoot,
        // leaving a stale flag we must not honour.
        if Self::has_effective_root(spells, chronological_root) {
            return Some(PreserveReason::FirstCareerClub);
        }
        if Self::any_long_stint(spells) {
            return Some(PreserveReason::LongZeroAppSpell);
        }
        None
    }

    fn any_long_stint(spells: &[&PlayerCareerSpell]) -> bool {
        for s in spells {
            let season = Season::new(s.season_start_year);
            let season_start = season.start_date();
            let season_end = season.end_date();
            let join = s.joined_date.max(season_start);
            let end = s.departed_date.unwrap_or(season_end).min(season_end).max(join);
            let days = (end - join).num_days().max(0) as f64;
            let span = (season_end - season_start).num_days().max(1) as f64;
            if days / span >= ZERO_APP_TRIVIAL_SEASON_SHARE {
                return true;
            }
        }
        false
    }

    pub fn career_totals(items: &[PlayerStatisticsHistoryItem]) -> PlayerStatistics {
        let mut totals = PlayerStatistics::default();
        for item in items {
            totals.merge_from(&item.statistics);
        }
        totals
    }

    pub fn current_club_career_apps(&self, live_played: u16, live_played_subs: u16) -> u32 {
        let slug = match self.active_team_slug() {
            Some(s) => s.to_string(),
            None => return live_played as u32 + live_played_subs as u32,
        };
        let mut total: u32 = 0;
        for item in &self.items {
            if item.team_slug == slug {
                total = total
                    .saturating_add(item.statistics.played as u32)
                    .saturating_add(item.statistics.played_subs as u32);
            }
        }
        total = total
            .saturating_add(live_played as u32)
            .saturating_add(live_played_subs as u32);
        total
    }

    // ── Compatibility cache rebuild ──────────────────────

    /// Rebuild `items` and `current` from `spells`. Cheap (per-player
    /// histories are tiny) and called after every mutator so external
    /// readers always see a coherent projection. Runs the same legacy
    /// normalizer on every cycle so any mutator path that produced a
    /// loan-implied parent placeholder is cleaned at the source rather
    /// than only hidden by the projection.
    fn refresh_compat(&mut self) {
        self.normalize_legacy_spells();
        // `items` mirrors `view_items(None)` for closed-spell-only data.
        // We project the full visibility-filtered view, then drop any row
        // that is purely the active continuation (open spell).
        let projected = self.view_items(None);
        let active_id = self.active_spell().map(|s| s.spell_id);
        let active_year_slug_kind: Option<(u16, String, CareerSpellKind)> = active_id
            .and_then(|id| self.spells.iter().find(|s| s.spell_id == id))
            .map(|s| (s.season_start_year, s.team_slug.clone(), s.kind));

        self.items = projected
            .into_iter()
            .filter(|i| {
                // Drop the "purely-active" row: only present if the only
                // spell behind it is the active continuation. Every other
                // visible row (including ones grouping closed + active in
                // the same key) stays.
                if let Some((y, ref slug, k)) = active_year_slug_kind {
                    let same_key = i.season.start_year == y
                        && i.team_slug == *slug
                        && (matches!(k, CareerSpellKind::Loan) == i.is_loan);
                    if same_key {
                        // Are there any closed spells in this group?
                        let has_closed = self.spells.iter().any(|s| {
                            s.season_start_year == i.season.start_year
                                && s.team_slug == i.team_slug
                                && (matches!(s.kind, CareerSpellKind::Loan) == i.is_loan)
                                && s.departed_date.is_some()
                        });
                        return has_closed;
                    }
                }
                true
            })
            .collect();

        // `current` mirrors the open spells (max one in the new model,
        // but kept as a Vec to match the legacy API). Fields are filled
        // from the spell so legacy readers (`current[0].team_slug`) work.
        self.current = self
            .spells
            .iter()
            .filter(|s| s.departed_date.is_none())
            .map(|s| CurrentSeasonEntry {
                team_name: s.team_name.clone(),
                team_slug: s.team_slug.clone(),
                team_reputation: s.team_reputation,
                league_name: s.league_name.clone(),
                league_slug: s.league_slug.clone(),
                is_loan: matches!(s.kind, CareerSpellKind::Loan),
                transfer_fee: s.transfer_fee,
                statistics: s.statistics.clone(),
                joined_date: s.joined_date,
                departed_date: s.departed_date,
                seq_id: s.movement_order as u32,
                season_start_year: s.season_start_year,
                kind: match s.opened_by {
                    CareerEventKind::InitialSeed => EntryKind::InitialSeed,
                    CareerEventKind::SeasonSeed => EntryKind::SeasonSeed,
                    CareerEventKind::PermanentTransfer | CareerEventKind::ManualTransfer => {
                        EntryKind::TransferIn
                    }
                    CareerEventKind::LoanStart | CareerEventKind::ReLoan => EntryKind::LoanIn,
                    CareerEventKind::LoanReturn => EntryKind::LoanReturn,
                    CareerEventKind::FreeAgentSigning => EntryKind::FreeAgentSigning,
                    CareerEventKind::Release | CareerEventKind::SeasonSnapshot => {
                        EntryKind::SourceSnapshot
                    }
                },
                is_career_root: matches!(s.root_kind, RootKind::CareerRoot),
                parent_team_slug: s.parent_team_slug.clone(),
            })
            .collect();

        self.debug_assert_spell_invariants();
    }

    /// Cheap consistency check on the spell list, only enforced in
    /// debug / test builds. Two invariants matter: at most one spell is
    /// open at any time, and an active loan cannot coexist with an
    /// open, 0-app, no-fee permanent spell at the loan's parent club in
    /// the same season — that combination is the visible bug we want
    /// to catch at mutation time, not just at projection time.
    fn debug_assert_spell_invariants(&self) {
        debug_assert!(
            self.spells.iter().filter(|s| s.departed_date.is_none()).count() <= 1,
            "more than one active spell"
        );
        if let Some(active) = self.active_spell() {
            if matches!(active.kind, CareerSpellKind::Loan) {
                if let Some(parent) = active.parent_team_slug.as_deref() {
                    let conflict = self.spells.iter().any(|s| {
                        s.spell_id != active.spell_id
                            && s.season_start_year == active.season_start_year
                            && s.team_slug == parent
                            && matches!(s.kind, CareerSpellKind::Permanent)
                            && s.departed_date.is_none()
                            && s.statistics.total_games() == 0
                            && s.transfer_fee.is_none()
                    });
                    debug_assert!(!conflict, "active loan has phantom open parent spell");
                }
            }
        }
    }
}

#[cfg(test)]
mod club_career_apps_tests {
    use super::*;
    use crate::league::Season;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn frozen(
        season_start: u16,
        slug: &str,
        played: u16,
        played_subs: u16,
    ) -> PlayerStatisticsHistoryItem {
        let mut stats = PlayerStatistics::default();
        stats.played = played;
        stats.played_subs = played_subs;
        PlayerStatisticsHistoryItem {
            season: Season::new(season_start),
            team_name: slug.to_string(),
            team_slug: slug.to_string(),
            team_reputation: 5_000,
            league_name: String::new(),
            league_slug: String::new(),
            is_loan: false,
            transfer_fee: None,
            statistics: stats,
            seq_id: season_start as u32,
        }
    }

    fn make_team(slug: &str) -> TeamInfo {
        TeamInfo {
            name: slug.to_string(),
            slug: slug.to_string(),
            reputation: 5_000,
            league_name: String::new(),
            league_slug: String::new(),
        }
    }

    #[test]
    fn club_career_apps_sums_history_at_current_club_plus_live() {
        let mut hist = PlayerStatisticsHistory::from_items(vec![
            frozen(2023, "juventus", 30, 5),
            frozen(2024, "juventus", 40, 5),
        ]);
        hist.seed_initial_team(&make_team("juventus"), d(2025, 8, 1), false);
        let apps = hist.current_club_career_apps(20, 0);
        assert_eq!(apps, 35 + 45 + 20);
    }

    #[test]
    fn club_career_apps_excludes_other_clubs() {
        let mut hist = PlayerStatisticsHistory::from_items(vec![
            frozen(2022, "torino", 60, 0),
            frozen(2023, "juventus", 25, 5),
        ]);
        hist.seed_initial_team(&make_team("juventus"), d(2025, 8, 1), false);
        let apps = hist.current_club_career_apps(10, 0);
        assert_eq!(apps, 30 + 10);
    }

    #[test]
    fn club_career_apps_falls_back_to_live_only_with_no_active_spell() {
        let hist = PlayerStatisticsHistory::new();
        let apps = hist.current_club_career_apps(5, 2);
        assert_eq!(apps, 7);
    }
}

#[cfg(test)]
mod active_loan_parent_suppression_tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn make_team(name: &str, slug: &str) -> TeamInfo {
        TeamInfo {
            name: name.to_string(),
            slug: slug.to_string(),
            reputation: 5_000,
            league_name: "Premier League".to_string(),
            league_slug: "premier-league".to_string(),
        }
    }

    fn describe_spells(history: &PlayerStatisticsHistory) -> String {
        history
            .spells
            .iter()
            .map(|s| {
                format!(
                    "id={} mo={} {} {} kind={:?} parent={:?} joined={} departed={:?} apps={} fee={:?} root={:?} opened={:?} closed={:?}",
                    s.spell_id,
                    s.movement_order,
                    s.season_start_year,
                    s.team_slug,
                    s.kind,
                    s.parent_team_slug,
                    s.joined_date,
                    s.departed_date,
                    s.statistics.total_games(),
                    s.transfer_fee,
                    s.root_kind,
                    s.opened_by,
                    s.closed_by,
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Inject a stale parent permanent spell directly onto `spells` to
    /// simulate either an old-save migration artefact or a future bug
    /// that bypasses the mutator guards. The projection must still
    /// suppress the row.
    fn inject_phantom_parent(
        history: &mut PlayerStatisticsHistory,
        team: &TeamInfo,
        season_start_year: u16,
    ) {
        let spell_id = history.alloc_spell_id();
        let movement_order = history.alloc_movement_order();
        let season = Season::new(season_start_year);
        history.spells.push(PlayerCareerSpell {
            spell_id,
            movement_order,
            season_start_year,
            team_name: team.name.clone(),
            team_slug: team.slug.clone(),
            team_reputation: team.reputation,
            league_name: team.league_name.clone(),
            league_slug: team.league_slug.clone(),
            kind: CareerSpellKind::Permanent,
            parent_team_slug: None,
            joined_date: season.start_date(),
            departed_date: Some(season.end_date()),
            opened_by: CareerEventKind::SeasonSeed,
            closed_by: Some(CareerEventKind::SeasonSnapshot),
            transfer_fee: None,
            statistics: PlayerStatistics::default(),
            root_kind: RootKind::None,
        });
    }

    /// Mid-season-2026/27, active loan to Dinamo, parent Spartak.
    /// A 0-app, no-fee Spartak Permanent 2026 spell injected from any
    /// upstream path (legacy save, future bug) must not surface in
    /// `view_items`.
    #[test]
    fn parent_permanent_during_active_loan_is_hidden_in_view() {
        let spartak = make_team("Spartak Moscow", "spartak-moscow");
        let dinamo = make_team("Dinamo Moscow", "dinamo-moscow");
        let mut history = PlayerStatisticsHistory::new();

        // Pre-loan stint at Spartak (root, will keep its 2025 row).
        history.seed_initial_team(&spartak, d(2025, 8, 1), false);
        // Loan starts mid-pre-season; first season at Dinamo records 24 apps.
        history.record_loan(
            PlayerStatistics::default(),
            &spartak,
            &dinamo,
            0.0,
            d(2025, 8, 15),
        );
        let mut s1 = PlayerStatistics::default();
        s1.played = 24;
        history.record_season_end(
            Season::new(2025),
            s1,
            &dinamo,
            true,
            Some(d(2025, 8, 15)),
        );

        // Inject the wrongly-attached Spartak 2026 permanent (the case
        // the user observed in the UI). Active loan = Dinamo 2026.
        inject_phantom_parent(&mut history, &spartak, 2026);
        history.refresh_compat();

        let view = history.view_items(None);
        let spartak_2026 = view
            .iter()
            .find(|e| e.season.start_year == 2026 && e.team_slug == "spartak-moscow");
        assert!(
            spartak_2026.is_none(),
            "Spartak 2026/27 must not display while loan to Dinamo is active.\n{}",
            describe_spells(&history)
        );
        let dinamo_2026 = view
            .iter()
            .find(|e| e.season.start_year == 2026 && e.team_slug == "dinamo-moscow");
        assert!(
            dinamo_2026.is_some(),
            "Dinamo 2026/27 (active loan) must remain visible.\n{}",
            describe_spells(&history)
        );
    }

    /// After a real loan return, the parent row IS expected to display
    /// in the same season — the suppression must not mask it.
    #[test]
    fn parent_permanent_after_loan_return_remains_visible() {
        let spartak = make_team("Spartak Moscow", "spartak-moscow");
        let dinamo = make_team("Dinamo Moscow", "dinamo-moscow");
        let mut history = PlayerStatisticsHistory::new();

        history.seed_initial_team(&spartak, d(2025, 8, 1), false);
        history.record_loan(
            PlayerStatistics::default(),
            &spartak,
            &dinamo,
            0.0,
            d(2025, 8, 15),
        );
        let mut s1 = PlayerStatistics::default();
        s1.played = 24;
        history.record_season_end(Season::new(2025), s1, &dinamo, true, Some(d(2025, 8, 15)));

        // Loan returns mid-2026/27.
        history.record_loan_return(PlayerStatistics::default(), &dinamo, &spartak, d(2026, 12, 15));

        let view = history.view_items(None);
        let spartak_2026 = view
            .iter()
            .find(|e| e.season.start_year == 2026 && e.team_slug == "spartak-moscow");
        assert!(
            spartak_2026.is_some(),
            "After loan return, parent row must surface for the active season.\n{}",
            describe_spells(&history)
        );
    }

    /// The exact bug reported from real save state: caller's `is_loan`
    /// flag is stale (`false`) at season-end while the spell model has
    /// the player on loan. Continuation must still come out as a loan
    /// spell carrying the parent slug, so the projection-side
    /// suppression can hide the same-season parent placeholder.
    #[test]
    fn two_season_loan_carries_parent_slug_when_is_loan_flag_lags_false() {
        let spartak = make_team("Spartak Moscow", "spartak-moscow");
        let dinamo = make_team("Dinamo Moscow", "dinamo-moscow");
        let mut history = PlayerStatisticsHistory::new();

        history.seed_initial_team(&spartak, d(2025, 8, 1), false);
        history.record_loan(
            PlayerStatistics::default(),
            &spartak,
            &dinamo,
            0.0,
            d(2025, 8, 15),
        );

        // Caller passes is_loan=false even though the active spell is a
        // loan — simulates the real flow where contract_loan flag was
        // stale at the moment of season-end snapshot.
        let mut s1 = PlayerStatistics::default();
        s1.played = 28;
        history.record_season_end(
            Season::new(2025),
            s1,
            &dinamo,
            false,
            Some(d(2025, 8, 15)),
        );

        let active = history
            .active_spell()
            .expect("active spell missing after season end");
        assert_eq!(active.team_slug, "dinamo-moscow");
        assert!(matches!(active.kind, CareerSpellKind::Loan));
        assert_eq!(
            active.parent_team_slug.as_deref(),
            Some("spartak-moscow"),
            "loan continuation must carry parent slug even when is_loan flag is stale.\n{}",
            describe_spells(&history)
        );

        // Inject the observed phantom parent row directly to mirror the
        // real-save state described in the bug report.
        let phantom_id = history.alloc_spell_id();
        let phantom_order = history.alloc_movement_order();
        history.spells.push(PlayerCareerSpell {
            spell_id: phantom_id,
            movement_order: phantom_order,
            season_start_year: 2026,
            team_name: "Spartak Moscow".to_string(),
            team_slug: "spartak-moscow".to_string(),
            team_reputation: 100,
            league_name: "Premier League".to_string(),
            league_slug: "premier-league".to_string(),
            kind: CareerSpellKind::Permanent,
            parent_team_slug: None,
            joined_date: d(2026, 8, 1),
            departed_date: Some(d(2027, 5, 31)),
            opened_by: CareerEventKind::SeasonSeed,
            closed_by: Some(CareerEventKind::SeasonSnapshot),
            transfer_fee: None,
            statistics: PlayerStatistics::default(),
            root_kind: RootKind::None,
        });

        let mut live = PlayerStatistics::default();
        live.played = 3;
        let view = history.view_items(Some(&live));
        assert!(
            !view.iter().any(|e| e.season.start_year == 2026 && e.team_slug == "spartak-moscow"),
            "Spartak 2026/27 phantom must be hidden during ongoing Dinamo loan.\n{}",
            describe_spells(&history)
        );
        let dinamo_2026 = view
            .iter()
            .find(|e| e.season.start_year == 2026 && e.team_slug == "dinamo-moscow")
            .expect("Dinamo 2026/27 loan row missing");
        assert!(dinamo_2026.is_loan);
        assert_eq!(dinamo_2026.statistics.played, 3);
    }

    /// Active loan spell with `parent_team_slug = None` (corrupted save
    /// or pre-fix bug). The view must still infer the parent from the
    /// most-recent prior Permanent spell and suppress its placeholder.
    #[test]
    fn view_infers_parent_when_active_loan_is_missing_parent_slug() {
        let spartak = make_team("Spartak Moscow", "spartak-moscow");
        let dinamo = make_team("Dinamo Moscow", "dinamo-moscow");
        let mut history = PlayerStatisticsHistory::new();

        history.seed_initial_team(&spartak, d(2025, 8, 1), false);
        history.record_loan(
            PlayerStatistics::default(),
            &spartak,
            &dinamo,
            0.0,
            d(2025, 8, 15),
        );
        let mut s1 = PlayerStatistics::default();
        s1.played = 24;
        history.record_season_end(Season::new(2025), s1, &dinamo, true, Some(d(2025, 8, 15)));

        // Strip parent_team_slug from the active loan continuation to
        // simulate a corrupted save state.
        if let Some(active) = history.active_spell_mut() {
            active.parent_team_slug = None;
        }

        // Phantom Spartak 2026 placeholder.
        let phantom_id = history.alloc_spell_id();
        let phantom_order = history.alloc_movement_order();
        history.spells.push(PlayerCareerSpell {
            spell_id: phantom_id,
            movement_order: phantom_order,
            season_start_year: 2026,
            team_name: "Spartak Moscow".to_string(),
            team_slug: "spartak-moscow".to_string(),
            team_reputation: 100,
            league_name: "Premier League".to_string(),
            league_slug: "premier-league".to_string(),
            kind: CareerSpellKind::Permanent,
            parent_team_slug: None,
            joined_date: d(2026, 8, 1),
            departed_date: Some(d(2027, 5, 31)),
            opened_by: CareerEventKind::SeasonSeed,
            closed_by: Some(CareerEventKind::SeasonSnapshot),
            transfer_fee: None,
            statistics: PlayerStatistics::default(),
            root_kind: RootKind::None,
        });

        let view = history.view_items(None);
        assert!(
            !view.iter().any(|e| e.season.start_year == 2026 && e.team_slug == "spartak-moscow"),
            "Inferred parent must drive suppression even when parent_team_slug is None.\n{}",
            describe_spells(&history)
        );
    }

    /// Migration path: a legacy save with a 0-app, no-fee Spartak
    /// Permanent 2026 row sitting next to a Dinamo Loan 2026 row gets
    /// imported. `from_items` cleans the parent placeholder so the
    /// projection never sees it.
    #[test]
    fn from_items_drops_legacy_parent_placeholder() {
        let spartak_2025 = PlayerStatisticsHistoryItem {
            season: Season::new(2025),
            team_name: "Spartak Moscow".to_string(),
            team_slug: "spartak-moscow".to_string(),
            team_reputation: 5_000,
            league_name: String::new(),
            league_slug: String::new(),
            is_loan: false,
            transfer_fee: None,
            statistics: PlayerStatistics::default(),
            seq_id: 0,
        };
        let mut dinamo_stats = PlayerStatistics::default();
        dinamo_stats.played = 24;
        let dinamo_2025 = PlayerStatisticsHistoryItem {
            season: Season::new(2025),
            team_name: "Dinamo Moscow".to_string(),
            team_slug: "dinamo-moscow".to_string(),
            team_reputation: 5_000,
            league_name: String::new(),
            league_slug: String::new(),
            is_loan: true,
            transfer_fee: None,
            statistics: dinamo_stats,
            seq_id: 1,
        };
        let spartak_2026 = PlayerStatisticsHistoryItem {
            season: Season::new(2026),
            team_name: "Spartak Moscow".to_string(),
            team_slug: "spartak-moscow".to_string(),
            team_reputation: 5_000,
            league_name: String::new(),
            league_slug: String::new(),
            is_loan: false,
            transfer_fee: None,
            statistics: PlayerStatistics::default(),
            seq_id: 2,
        };
        let mut dinamo_2026_stats = PlayerStatistics::default();
        dinamo_2026_stats.played = 5;
        let dinamo_2026 = PlayerStatisticsHistoryItem {
            season: Season::new(2026),
            team_name: "Dinamo Moscow".to_string(),
            team_slug: "dinamo-moscow".to_string(),
            team_reputation: 5_000,
            league_name: String::new(),
            league_slug: String::new(),
            is_loan: true,
            transfer_fee: None,
            statistics: dinamo_2026_stats,
            seq_id: 3,
        };

        let history = PlayerStatisticsHistory::from_items(vec![
            spartak_2025,
            dinamo_2025,
            spartak_2026,
            dinamo_2026,
        ]);

        let phantom = history
            .spells
            .iter()
            .find(|s| s.season_start_year == 2026 && s.team_slug == "spartak-moscow");
        assert!(
            phantom.is_none(),
            "Legacy parent placeholder for 2026 should be normalized away.\n{}",
            describe_spells(&history)
        );
        // The 2025 root row stays — it has its own season's loan
        // alongside, but the rule keeps roots and rows with apps/fee.
        assert!(
            history
                .spells
                .iter()
                .any(|s| s.season_start_year == 2025
                    && s.team_slug == "spartak-moscow"
                    && matches!(s.root_kind, RootKind::CareerRoot)),
            "Career-root 2025 row must survive normalization."
        );
    }

    /// The phantom must be hidden even when the matching loan spell
    /// isn't currently active — covers the case where the 2026 Dinamo
    /// loan was already closed (mid-window snapshot, simulated tick
    /// timing) and only the parent placeholder remained visible.
    #[test]
    fn same_season_loan_suppresses_zero_app_parent_even_when_loan_not_active() {
        let mut history = PlayerStatisticsHistory::new();
        let spartak = make_team("Spartak Moscow", "spartak-moscow");
        let dinamo = make_team("Dinamo Moscow", "dinamo-moscow");

        history.seed_initial_team(&spartak, d(2025, 8, 1), false);
        history.record_loan(
            PlayerStatistics::default(),
            &spartak,
            &dinamo,
            0.0,
            d(2025, 8, 15),
        );
        let mut s1 = PlayerStatistics::default();
        s1.played = 26;
        history.record_season_end(Season::new(2025), s1, &dinamo, true, Some(d(2025, 8, 15)));

        // Inject the observed phantom 2026 Spartak placeholder.
        let phantom_id = history.alloc_spell_id();
        let phantom_order = history.alloc_movement_order();
        history.spells.push(PlayerCareerSpell {
            spell_id: phantom_id,
            movement_order: phantom_order,
            season_start_year: 2026,
            team_name: "Spartak Moscow".to_string(),
            team_slug: "spartak-moscow".to_string(),
            team_reputation: 100,
            league_name: "Premier League".to_string(),
            league_slug: "premier-league".to_string(),
            kind: CareerSpellKind::Permanent,
            parent_team_slug: None,
            joined_date: d(2026, 8, 1),
            departed_date: Some(d(2027, 5, 31)),
            opened_by: CareerEventKind::SeasonSeed,
            closed_by: Some(CareerEventKind::SeasonSnapshot),
            transfer_fee: None,
            statistics: PlayerStatistics::default(),
            root_kind: RootKind::None,
        });

        // Force-close the 2026 Dinamo loan continuation so no spell is
        // active. Suppression must still fire from the same-season loan
        // relationship alone.
        let mut closed_played = PlayerStatistics::default();
        closed_played.played = 10;
        for s in &mut history.spells {
            if s.season_start_year == 2026 && s.team_slug == "dinamo-moscow" {
                s.departed_date = Some(d(2027, 5, 31));
                s.statistics = closed_played.clone();
            }
        }

        let view = history.view_items(None);
        assert!(
            !view.iter().any(|e| e.season.start_year == 2026 && e.team_slug == "spartak-moscow"),
            "Same-season loan must suppress phantom parent even when no loan is active.\n{}",
            history.debug_spell_dump()
        );
        assert!(
            view.iter().any(|e| {
                e.season.start_year == 2026 && e.team_slug == "dinamo-moscow" && e.is_loan
            }),
            "Dinamo 2026 closed-loan row must remain visible.\n{}",
            history.debug_spell_dump()
        );
    }

    /// `LongZeroAppSpell` would normally let a full-season Permanent
    /// row pass `group_visibility`. The broader suppressor must run
    /// first so a loan-implied placeholder is dropped regardless of
    /// stint length.
    #[test]
    fn long_zero_app_parent_placeholder_is_suppressed_by_same_season_loan() {
        let mut history = PlayerStatisticsHistory::new();
        let spartak = make_team("Spartak Moscow", "spartak-moscow");
        let dinamo = make_team("Dinamo Moscow", "dinamo-moscow");

        history.seed_initial_team(&spartak, d(2025, 8, 1), false);
        history.record_loan(
            PlayerStatistics::default(),
            &spartak,
            &dinamo,
            0.0,
            d(2025, 8, 15),
        );
        let mut s1 = PlayerStatistics::default();
        s1.played = 26;
        history.record_season_end(Season::new(2025), s1, &dinamo, true, Some(d(2025, 8, 15)));

        // Full-season parent placeholder — long-stint test would mark it
        // as visible.
        let phantom_id = history.alloc_spell_id();
        let phantom_order = history.alloc_movement_order();
        history.spells.push(PlayerCareerSpell {
            spell_id: phantom_id,
            movement_order: phantom_order,
            season_start_year: 2026,
            team_name: "Spartak Moscow".to_string(),
            team_slug: "spartak-moscow".to_string(),
            team_reputation: 100,
            league_name: "Premier League".to_string(),
            league_slug: "premier-league".to_string(),
            kind: CareerSpellKind::Permanent,
            parent_team_slug: None,
            joined_date: d(2026, 8, 1),
            departed_date: Some(d(2027, 5, 31)),
            opened_by: CareerEventKind::SeasonSeed,
            closed_by: Some(CareerEventKind::SeasonSnapshot),
            transfer_fee: None,
            statistics: PlayerStatistics::default(),
            root_kind: RootKind::None,
        });

        let view = history.view_items(None);
        assert!(
            !view.iter().any(|e| e.season.start_year == 2026 && e.team_slug == "spartak-moscow"),
            "Long-stint parent placeholder must not survive same-season-loan suppression.\n{}",
            history.debug_spell_dump()
        );
    }

    /// Loan with no `parent_team_slug` (corrupted save). The
    /// `parent_slug_for_loan_spell` fallback infers parent from the
    /// most-recent prior Permanent and the suppressor still fires.
    #[test]
    fn same_season_loan_suppression_infers_parent_when_parent_slug_missing() {
        let mut history = PlayerStatisticsHistory::new();
        let spartak = make_team("Spartak Moscow", "spartak-moscow");
        let dinamo = make_team("Dinamo Moscow", "dinamo-moscow");

        history.seed_initial_team(&spartak, d(2025, 8, 1), false);
        history.record_loan(
            PlayerStatistics::default(),
            &spartak,
            &dinamo,
            0.0,
            d(2025, 8, 15),
        );
        let mut s1 = PlayerStatistics::default();
        s1.played = 26;
        history.record_season_end(Season::new(2025), s1, &dinamo, true, Some(d(2025, 8, 15)));

        // Strip parent_team_slug from EVERY loan spell.
        for s in &mut history.spells {
            if matches!(s.kind, CareerSpellKind::Loan) {
                s.parent_team_slug = None;
            }
        }

        let phantom_id = history.alloc_spell_id();
        let phantom_order = history.alloc_movement_order();
        history.spells.push(PlayerCareerSpell {
            spell_id: phantom_id,
            movement_order: phantom_order,
            season_start_year: 2026,
            team_name: "Spartak Moscow".to_string(),
            team_slug: "spartak-moscow".to_string(),
            team_reputation: 100,
            league_name: "Premier League".to_string(),
            league_slug: "premier-league".to_string(),
            kind: CareerSpellKind::Permanent,
            parent_team_slug: None,
            joined_date: d(2026, 8, 1),
            departed_date: Some(d(2027, 5, 31)),
            opened_by: CareerEventKind::SeasonSeed,
            closed_by: Some(CareerEventKind::SeasonSnapshot),
            transfer_fee: None,
            statistics: PlayerStatistics::default(),
            root_kind: RootKind::None,
        });

        let view = history.view_items(None);
        assert!(
            !view.iter().any(|e| e.season.start_year == 2026 && e.team_slug == "spartak-moscow"),
            "Inferred parent from prior permanent must drive suppression.\n{}",
            history.debug_spell_dump()
        );
    }

    /// A real loan return materialises a parent permanent spell whose
    /// `opened_by` is `LoanReturn`. That row is legitimate and must NOT
    /// be hidden — even with 0 apps and no fee.
    #[test]
    fn parent_after_real_loan_return_is_not_suppressed() {
        let mut history = PlayerStatisticsHistory::new();
        let spartak = make_team("Spartak Moscow", "spartak-moscow");
        let dinamo = make_team("Dinamo Moscow", "dinamo-moscow");

        history.seed_initial_team(&spartak, d(2025, 8, 1), false);
        history.record_loan(
            PlayerStatistics::default(),
            &spartak,
            &dinamo,
            0.0,
            d(2025, 8, 15),
        );
        let mut s1 = PlayerStatistics::default();
        s1.played = 26;
        history.record_season_end(Season::new(2025), s1, &dinamo, true, Some(d(2025, 8, 15)));

        // Loan returns mid-2026/27.
        history.record_loan_return(
            PlayerStatistics::default(),
            &dinamo,
            &spartak,
            d(2026, 12, 15),
        );

        let view = history.view_items(None);
        let spartak_2026 = view
            .iter()
            .find(|e| e.season.start_year == 2026 && e.team_slug == "spartak-moscow");
        assert!(
            spartak_2026.is_some(),
            "Parent row opened by LoanReturn must remain visible.\n{}",
            history.debug_spell_dump()
        );
    }

    /// Real-save reproduction: the player is seeded at Spartak 2026 (root,
    /// `InitialSeed`), instantly loaned to Dinamo, and then the prior
    /// 2025/26 Dinamo loan season is materialised by a season-end backfill
    /// AFTER the root flag was already attached. The 2025 Dinamo spell is
    /// now chronologically earlier, so the stale `CareerRoot` flag on
    /// Spartak 2026 must no longer protect it from the same-season-loan
    /// suppressor.
    #[test]
    fn backfilled_prior_loan_invalidates_later_root_parent_placeholder() {
        let mut history = PlayerStatisticsHistory::new();
        let spartak = make_team("Spartak Moscow", "spartak-moscow");
        let dinamo = make_team("Dinamo Moscow", "dinamo-moscow");

        // Reproduce the real save's insertion order: current 2026 parent
        // is seeded and immediately loaned out same day.
        history.seed_initial_team(&spartak, d(2026, 8, 1), false);
        history.record_departure_loan(
            PlayerStatistics::default(),
            &spartak,
            &spartak,
            &dinamo,
            false,
            d(2026, 8, 1),
        );

        // THEN the older 2025 Dinamo loan season is materialised /
        // backfilled. After this point the 2025 Dinamo spell is the
        // chronologically-earliest entry in the history.
        let mut s2025 = PlayerStatistics::default();
        s2025.played = 26;
        history.record_season_end(
            Season::new(2025),
            s2025,
            &dinamo,
            true,
            Some(d(2025, 8, 1)),
        );

        let mut live = PlayerStatistics::default();
        live.played = 10;
        let view = history.view_items(Some(&live));

        assert!(
            !view.iter().any(|e| e.season.start_year == 2026 && e.team_slug == "spartak-moscow"),
            "Spartak 2026 root placeholder must be hidden because a 2025 spell exists before it.\n{}",
            history.debug_spell_dump()
        );

        let dinamo_2026 = view
            .iter()
            .find(|e| e.season.start_year == 2026 && e.team_slug == "dinamo-moscow")
            .expect("Dinamo 2026 loan row missing");
        assert!(dinamo_2026.is_loan);
        assert_eq!(dinamo_2026.statistics.played, 10);

        let dinamo_2025 = view
            .iter()
            .find(|e| e.season.start_year == 2025 && e.team_slug == "dinamo-moscow")
            .expect("Dinamo 2025 loan row missing");
        assert!(dinamo_2025.is_loan);
        assert_eq!(dinamo_2025.statistics.played, 26);
    }

    /// Direct unit-level coverage: `CareerRoot` preservation must require
    /// chronological-root identity, not just the flag. Construct a
    /// 2026/Spartak `CareerRoot` spell and a backfilled earlier 2025
    /// Dinamo spell, then assert the projection drops the Spartak row.
    #[test]
    fn career_root_preserve_requires_chronological_root() {
        let mut history = PlayerStatisticsHistory::new();
        let spartak = make_team("Spartak Moscow", "spartak-moscow");
        let dinamo = make_team("Dinamo Moscow", "dinamo-moscow");

        // 2026 Spartak root spell, opened and immediately closed (mirrors
        // the seed-then-loan pattern that produced the bug). Ends up with
        // `RootKind::CareerRoot` because it was the first spell created.
        history.seed_initial_team(&spartak, d(2026, 8, 1), false);
        history.record_departure_loan(
            PlayerStatistics::default(),
            &spartak,
            &spartak,
            &dinamo,
            false,
            d(2026, 8, 1),
        );
        // Backfill an earlier 2025 Dinamo loan season so the chronological
        // root shifts away from Spartak 2026.
        let mut s2025 = PlayerStatistics::default();
        s2025.played = 12;
        history.record_season_end(
            Season::new(2025),
            s2025,
            &dinamo,
            true,
            Some(d(2025, 8, 1)),
        );

        // Sanity: the Spartak 2026 spell still carries the legacy root
        // flag — the bug is precisely that this flag survives even after
        // a chronologically-earlier spell is added.
        assert!(
            history
                .spells
                .iter()
                .any(|s| s.season_start_year == 2026
                    && s.team_slug == "spartak-moscow"
                    && matches!(s.root_kind, RootKind::CareerRoot)),
            "test precondition: Spartak 2026 must still hold the stale CareerRoot flag.\n{}",
            history.debug_spell_dump()
        );

        let view = history.view_items(None);
        assert!(
            !view.iter().any(|e| e.season.start_year == 2026 && e.team_slug == "spartak-moscow"),
            "Stale CareerRoot on a later season must not preserve its row.\n{}",
            history.debug_spell_dump()
        );
    }

    /// Real-bug reproduction: Spartak owns the player, immediate loan to
    /// Dinamo at the start of 2026/27, and the prior 2025/26 Dinamo loan
    /// season is materialised by a backfill. Spartak must appear once as
    /// the parent anchor in the first loan season (2025), not as a
    /// 2026/27 phantom and not be missing entirely.
    #[test]
    fn two_season_loan_shows_original_parent_anchor_in_first_loan_season_only() {
        let mut history = PlayerStatisticsHistory::new();
        let spartak = make_team("Spartak Moscow", "spartak-moscow");
        let dinamo = make_team("Dinamo Moscow", "dinamo-moscow");

        history.seed_initial_team(&spartak, d(2026, 8, 1), false);
        history.record_departure_loan(
            PlayerStatistics::default(),
            &spartak,
            &spartak,
            &dinamo,
            false,
            d(2026, 8, 1),
        );

        let mut s2025 = PlayerStatistics::default();
        s2025.played = 26;
        history.record_season_end(
            Season::new(2025),
            s2025,
            &dinamo,
            true,
            Some(d(2025, 8, 1)),
        );

        let mut live = PlayerStatistics::default();
        live.played = 10;
        let view = history.view_items(Some(&live));

        let rows: Vec<(u16, &str, bool, u16)> = view
            .iter()
            .map(|e| {
                (
                    e.season.start_year,
                    e.team_slug.as_str(),
                    e.is_loan,
                    e.statistics.played,
                )
            })
            .collect();

        assert_eq!(
            rows,
            vec![
                (2026, "dinamo-moscow", true, 10),
                (2025, "dinamo-moscow", true, 26),
                (2025, "spartak-moscow", false, 0),
            ],
            "Parent anchor must show in 2025 only, never as a 2026 phantom.\n{}",
            history.debug_spell_dump()
        );
    }

    /// Clean immediate loan with a parent stint: Spartak 2025 stub already
    /// exists as the chronological root, so the anchor reuses the real row
    /// rather than synthesizing a duplicate. Order: loan above parent.
    #[test]
    fn immediate_loan_from_original_club_shows_anchor_once() {
        let mut history = PlayerStatisticsHistory::new();
        let spartak = make_team("Spartak Moscow", "spartak-moscow");
        let dinamo = make_team("Dinamo Moscow", "dinamo-moscow");

        history.seed_initial_team(&spartak, d(2025, 8, 1), false);
        history.record_loan(
            PlayerStatistics::default(),
            &spartak,
            &dinamo,
            0.0,
            d(2025, 8, 15),
        );
        let mut s1 = PlayerStatistics::default();
        s1.played = 24;
        history.record_season_end(Season::new(2025), s1, &dinamo, true, Some(d(2025, 8, 15)));

        let view = history.view_items(None);
        let rows_2025: Vec<(&str, bool)> = view
            .iter()
            .filter(|e| e.season.start_year == 2025)
            .map(|e| (e.team_slug.as_str(), e.is_loan))
            .collect();

        assert_eq!(
            rows_2025,
            vec![("dinamo-moscow", true), ("spartak-moscow", false)],
            "First-loan-season ordering: loan first, parent anchor second.\n{}",
            describe_spells(&history)
        );

        let spartak_rows = view
            .iter()
            .filter(|e| e.team_slug == "spartak-moscow")
            .count();
        assert_eq!(
            spartak_rows, 1,
            "Original parent anchor must appear exactly once.\n{}",
            describe_spells(&history)
        );
    }

    /// Two-season loan: parent anchor must NOT repeat in continuation
    /// seasons after the first loan year.
    #[test]
    fn original_parent_anchor_does_not_repeat_in_second_loan_season() {
        let mut history = PlayerStatisticsHistory::new();
        let spartak = make_team("Spartak Moscow", "spartak-moscow");
        let dinamo = make_team("Dinamo Moscow", "dinamo-moscow");

        history.seed_initial_team(&spartak, d(2025, 8, 1), false);
        history.record_loan(
            PlayerStatistics::default(),
            &spartak,
            &dinamo,
            0.0,
            d(2025, 8, 15),
        );
        let mut s1 = PlayerStatistics::default();
        s1.played = 24;
        history.record_season_end(Season::new(2025), s1, &dinamo, true, Some(d(2025, 8, 15)));

        let view = history.view_items(None);

        assert!(
            view.iter().any(|e| e.season.start_year == 2025
                && e.team_slug == "spartak-moscow"
                && !e.is_loan),
            "Spartak must appear in first loan season (2025).\n{}",
            describe_spells(&history)
        );
        assert!(
            !view
                .iter()
                .any(|e| e.season.start_year == 2026 && e.team_slug == "spartak-moscow"),
            "Spartak must not appear in 2026 continuation season.\n{}",
            describe_spells(&history)
        );
    }

    /// Player plays at parent before the loan; parent row already has apps
    /// and survives `group_visibility` on its own — anchor must not
    /// duplicate it.
    #[test]
    fn parent_spell_with_apps_survives_as_real_row_not_synthetic_anchor() {
        let mut history = PlayerStatisticsHistory::new();
        let spartak = make_team("Spartak Moscow", "spartak-moscow");
        let dinamo = make_team("Dinamo Moscow", "dinamo-moscow");

        history.seed_initial_team(&spartak, d(2025, 8, 1), false);
        let mut at_spartak = PlayerStatistics::default();
        at_spartak.played = 3;
        history.record_loan(at_spartak, &spartak, &dinamo, 0.0, d(2025, 9, 15));
        let mut s1 = PlayerStatistics::default();
        s1.played = 22;
        history.record_season_end(Season::new(2025), s1, &dinamo, true, Some(d(2025, 9, 15)));

        let view = history.view_items(None);
        let spartak_2025: Vec<&PlayerStatisticsHistoryItem> = view
            .iter()
            .filter(|e| e.season.start_year == 2025 && e.team_slug == "spartak-moscow")
            .collect();
        assert_eq!(
            spartak_2025.len(),
            1,
            "Real parent stint must produce exactly one Spartak 2025 row, no synthetic duplicate.\n{}",
            describe_spells(&history)
        );
        assert_eq!(
            spartak_2025[0].statistics.played, 3,
            "Real apps at parent must be preserved, not zeroed by anchor.\n{}",
            describe_spells(&history)
        );
    }

    /// Multi-parent: after a permanent move to a new owner who then
    /// loans the player out, the new owner gets its own anchor at its
    /// loan's first season (no real row in that season).
    #[test]
    fn anchor_synthesized_per_parent_for_multi_parent_careers() {
        let mut history = PlayerStatisticsHistory::new();
        let bayern = make_team("Bayern", "bayern");
        let hertha = make_team("Hertha", "hertha");

        // Bayern signs the player late in pre-season then loans them
        // straight to Hertha — common transfer-window flow. Bayern has
        // no Permanent row in the loan's season, so the anchor must
        // synthesize one.
        history.seed_initial_team(&bayern, d(2026, 9, 1), false);
        history.record_loan(
            PlayerStatistics::default(),
            &bayern,
            &hertha,
            0.0,
            d(2026, 9, 5),
        );

        let view = history.view_items(None);

        let bayern_2026 = view
            .iter()
            .find(|e| e.season.start_year == 2026 && e.team_slug == "bayern" && !e.is_loan)
            .unwrap_or_else(|| {
                panic!(
                    "Bayern 2026 anchor missing.\n{}",
                    describe_spells(&history)
                )
            });
        assert_eq!(bayern_2026.statistics.played, 0);
        assert!(
            view.iter().any(|e| e.season.start_year == 2026
                && e.team_slug == "hertha"
                && e.is_loan),
            "Hertha 2026 loan must be visible.\n{}",
            describe_spells(&history)
        );
        let bayern_rows = view.iter().filter(|e| e.team_slug == "bayern").count();
        assert_eq!(
            bayern_rows, 1,
            "Bayern must appear exactly once as the original-parent anchor.\n{}",
            describe_spells(&history)
        );
    }
}
