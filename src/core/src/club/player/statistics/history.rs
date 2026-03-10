use chrono::NaiveDate;
use crate::league::Season;
use super::types::{PlayerStatistics, TeamInfo};

#[derive(Debug, Clone)]
pub struct PlayerStatisticsHistory {
    pub items: Vec<PlayerStatisticsHistoryItem>,
    next_seq: u32,
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
    /// Monotonic sequence ID for stable ordering within a season
    pub seq_id: u32,
}

impl Default for PlayerStatisticsHistory {
    fn default() -> Self {
        Self::new()
    }
}

impl PlayerStatisticsHistory {
    pub fn new() -> Self {
        PlayerStatisticsHistory { items: Vec::new(), next_seq: 0 }
    }

    fn assign_seq(&mut self, item: &mut PlayerStatisticsHistoryItem) {
        item.seq_id = self.next_seq;
        self.next_seq += 1;
    }

    // ── Events ───────────────────────────────────────────────────

    /// Record a permanent transfer. Saves old club stats and creates new club placeholder.
    pub fn record_transfer(
        &mut self,
        old_stats: PlayerStatistics,
        from: &TeamInfo,
        to: &TeamInfo,
        fee: f64,
        date: NaiveDate,
    ) {
        let season = Season::from_date(date);
        let has_games = old_stats.total_games() > 0;

        // Save selling club stats (skip 0-game entry unless it's the only record)
        if has_games || self.items.is_empty() {
            self.push_or_replace(PlayerStatisticsHistoryItem {
                season: season.clone(),
                team_name: from.name.clone(),
                team_slug: from.slug.clone(),
                team_reputation: from.reputation,
                league_name: from.league_name.clone(),
                league_slug: from.league_slug.clone(),
                is_loan: false,
                transfer_fee: None,
                statistics: old_stats,
                seq_id: 0, // placeholder
            });
        }

        // Buying club placeholder with fee (later time so it sorts after)
        self.push_or_replace(PlayerStatisticsHistoryItem {
            season,
            team_name: to.name.clone(),
            team_slug: to.slug.clone(),
            team_reputation: to.reputation,
            league_name: to.league_name.clone(),
            league_slug: to.league_slug.clone(),
            is_loan: false,
            transfer_fee: Some(fee),
            statistics: PlayerStatistics::default(),
            seq_id: 0, // placeholder
        });
    }

    /// Record a loan move. Saves old club stats and creates loan placeholder.
    pub fn record_loan(
        &mut self,
        old_stats: PlayerStatistics,
        from: &TeamInfo,
        to: &TeamInfo,
        loan_fee: f64,
        date: NaiveDate,
    ) {
        let season = Season::from_date(date);
        let has_games = old_stats.total_games() > 0;

        // Save parent club stats
        if has_games || self.items.is_empty() {
            self.push_or_replace(PlayerStatisticsHistoryItem {
                season: season.clone(),
                team_name: from.name.clone(),
                team_slug: from.slug.clone(),
                team_reputation: from.reputation,
                league_name: from.league_name.clone(),
                league_slug: from.league_slug.clone(),
                is_loan: false,
                transfer_fee: None,
                statistics: old_stats,
                seq_id: 0, // placeholder
            });
        }

        // Loan destination placeholder (later time so it sorts after)
        self.push_or_replace(PlayerStatisticsHistoryItem {
            season,
            team_name: to.name.clone(),
            team_slug: to.slug.clone(),
            team_reputation: to.reputation,
            league_name: to.league_name.clone(),
            league_slug: to.league_slug.clone(),
            is_loan: true,
            transfer_fee: Some(loan_fee),
            statistics: PlayerStatistics::default(),
            seq_id: 0, // placeholder
        });
    }

    /// Record a loan return. Merges remaining stats into the loan entry
    /// and creates a parent club placeholder for the return season.
    pub fn record_loan_return(
        &mut self,
        remaining_stats: PlayerStatistics,
        borrowing: &TeamInfo,
        date: NaiveDate,
    ) {
        let season = Season::from_date(date);

        if let Some(existing) = self.items.iter_mut().find(|e| {
            e.season.start_year == season.start_year
                && e.team_slug == borrowing.slug
                && e.is_loan
        }) {
            let existing_games = existing.statistics.total_games();
            let new_games = remaining_stats.total_games();
            if existing_games == 0 && new_games > 0 {
                existing.statistics = remaining_stats;
            }
        } else {
            self.push_or_replace(PlayerStatisticsHistoryItem {
                season: season.clone(),
                team_name: borrowing.name.clone(),
                team_slug: borrowing.slug.clone(),
                team_reputation: borrowing.reputation,
                league_name: borrowing.league_name.clone(),
                league_slug: borrowing.league_slug.clone(),
                is_loan: true,
                transfer_fee: None,
                statistics: remaining_stats,
                seq_id: 0, // placeholder
            });
        }

        // Create parent club entry for the return season (player is back at parent)
        let parent_info = self.items.iter()
            .filter(|e| !e.is_loan && e.team_slug != borrowing.slug)
            .last()
            .map(|e| (e.team_name.clone(), e.team_slug.clone(), e.team_reputation,
                       e.league_name.clone(), e.league_slug.clone()));

        if let Some((name, slug, rep, league_name, league_slug)) = parent_info {
            let exists = self.items.iter().any(|e| {
                e.season.start_year == season.start_year && e.team_slug == slug && !e.is_loan
            });
            if !exists {
                self.items.push(PlayerStatisticsHistoryItem {
                    season,
                    team_name: name,
                    team_slug: slug,
                    team_reputation: rep,
                    league_name,
                    league_slug,
                    is_loan: false,
                    transfer_fee: None,
                    statistics: PlayerStatistics::default(),
                    seq_id: 0,
                });
            }
        }
    }

    /// Record a cancel-loan departure. Snapshots stats at borrowing club,
    /// cleans stale entries, and re-creates a parent club placeholder.
    pub fn record_cancel_loan(
        &mut self,
        old_stats: PlayerStatistics,
        borrowing: &TeamInfo,
        parent: &TeamInfo,
        is_loan: bool,
        date: NaiveDate,
    ) {
        let season = Season::from_date(date);

        // Save borrowing club stats if player actually played
        if old_stats.total_games() > 0 {
            if let Some(existing) = self.items.iter_mut().find(|e| {
                e.season.start_year == season.start_year && e.team_slug == borrowing.slug
            }) {
                if existing.statistics.total_games() == 0 {
                    existing.statistics = old_stats;
                } else {
                    existing.statistics.merge_from(&old_stats);
                }
            } else {
                self.push_item(PlayerStatisticsHistoryItem {
                    season: season.clone(),
                    team_name: borrowing.name.clone(),
                    team_slug: borrowing.slug.clone(),
                    team_reputation: borrowing.reputation,
                    league_name: borrowing.league_name.clone(),
                    league_slug: borrowing.league_slug.clone(),
                    is_loan,
                    transfer_fee: None,
                    statistics: old_stats,
                    seq_id: 0,
                });
            }
        }

        // Wipe stale 0-game placeholders for current season (keep entries with transfer_fee)
        self.items.retain(|e| {
            !(e.season.start_year == season.start_year
                && e.statistics.total_games() == 0
                && e.transfer_fee.is_none())
        });

        // Re-create parent club placeholder (after borrowing entry in sort order)
        let exists = self.items.iter().any(|e| {
            e.season.start_year == season.start_year && e.team_slug == parent.slug
        });
        if !exists {
            self.push_item(PlayerStatisticsHistoryItem {
                season: season.clone(),
                team_name: parent.name.clone(),
                team_slug: parent.slug.clone(),
                team_reputation: parent.reputation,
                league_name: parent.league_name.clone(),
                league_slug: parent.league_slug.clone(),
                is_loan: false,
                transfer_fee: None,
                statistics: PlayerStatistics::default(),
                seq_id: 0,
            });
        }
    }

    /// Record a manual transfer departure from the web UI.
    /// Snapshots stats at current club, cleans stale entries, creates destination placeholder.
    pub fn record_departure_transfer(
        &mut self,
        old_stats: PlayerStatistics,
        from: &TeamInfo,
        to: &TeamInfo,
        fee: Option<f64>,
        is_loan: bool,
        date: NaiveDate,
    ) {
        let season = Season::from_date(date);

        // Save source club stats (always — shows previous club even with 0 games)
        if let Some(existing) = self.items.iter_mut().find(|e| {
            e.season.start_year == season.start_year && e.team_slug == from.slug
        }) {
            if existing.statistics.total_games() == 0 {
                existing.statistics = old_stats;
            } else {
                existing.statistics.merge_from(&old_stats);
            }
        } else {
            self.push_item(PlayerStatisticsHistoryItem {
                season: season.clone(),
                team_name: from.name.clone(),
                team_slug: from.slug.clone(),
                team_reputation: from.reputation,
                league_name: from.league_name.clone(),
                league_slug: from.league_slug.clone(),
                is_loan,
                transfer_fee: None,
                statistics: old_stats,
                seq_id: 0,
            });
        }

        // Wipe stale 0-game placeholders for current season
        // (keep source club entry, keep entries with transfer_fee)
        self.items.retain(|e| {
            !(e.season.start_year == season.start_year
                && e.statistics.total_games() == 0
                && e.transfer_fee.is_none()
                && e.team_slug != from.slug)
        });

        // Create destination placeholder
        let exists = self.items.iter().any(|e| {
            e.season.start_year == season.start_year && e.team_slug == to.slug
        });
        if !exists {
            self.push_item(PlayerStatisticsHistoryItem {
                season,
                team_name: to.name.clone(),
                team_slug: to.slug.clone(),
                team_reputation: to.reputation,
                league_name: to.league_name.clone(),
                league_slug: to.league_slug.clone(),
                is_loan: false,
                transfer_fee: fee,
                statistics: PlayerStatistics::default(),
                seq_id: 0,
            });
        }
    }

    /// Record a manual loan departure from the web UI.
    /// Snapshots stats, cleans stale entries, creates loan destination placeholder.
    pub fn record_departure_loan(
        &mut self,
        old_stats: PlayerStatistics,
        from: &TeamInfo,
        _parent: &TeamInfo,
        to: &TeamInfo,
        is_loan: bool,
        date: NaiveDate,
    ) {
        let season = Season::from_date(date);
        let has_games = old_stats.total_games() > 0;

        // Save source club stats if player played, or if this is the first record
        if has_games || self.items.is_empty() {
            if let Some(existing) = self.items.iter_mut().find(|e| {
                e.season.start_year == season.start_year && e.team_slug == from.slug
            }) {
                if existing.statistics.total_games() == 0 {
                    existing.statistics = old_stats;
                } else {
                    existing.statistics.merge_from(&old_stats);
                }
            } else {
                self.push_item(PlayerStatisticsHistoryItem {
                    season: season.clone(),
                    team_name: from.name.clone(),
                    team_slug: from.slug.clone(),
                    team_reputation: from.reputation,
                    league_name: from.league_name.clone(),
                    league_slug: from.league_slug.clone(),
                    is_loan: false,
                    transfer_fee: None,
                    statistics: old_stats,
                    seq_id: 0,
                });
            }
        }

        // Wipe stale 0-game placeholders for current season
        // (keep source club entry if just created, keep entries with transfer_fee)
        let keep_from = has_games || self.items.len() <= 1;
        self.items.retain(|e| {
            !(e.season.start_year == season.start_year
                && e.statistics.total_games() == 0
                && e.transfer_fee.is_none()
                && !(keep_from && e.team_slug == from.slug))
        });

        // Parent club entry: if the player played at the parent this season,
        // the entry already survived cleanup above. No need to create a 0-game
        // placeholder — it would be a ghost record with no stats.

        // Loan destination placeholder (player goes here on loan)
        let dest_exists = self.items.iter().any(|e| {
            e.season.start_year == season.start_year && e.team_slug == to.slug
        });
        if !dest_exists {
            self.push_item(PlayerStatisticsHistoryItem {
                season: season.clone(),
                team_name: to.name.clone(),
                team_slug: to.slug.clone(),
                team_reputation: to.reputation,
                league_name: to.league_name.clone(),
                league_slug: to.league_slug.clone(),
                is_loan: true,
                transfer_fee: None,
                statistics: PlayerStatistics::default(),
                seq_id: 0,
            });
        }

        // Clean up oldest season if all its entries are 0-game non-loan placeholders
        // (stale bootstrap entries superseded by newer season-end entries).
        // Loan entries are preserved — they represent real transfer history.
        if let Some(oldest) = self.items.iter().map(|e| e.season.start_year).min() {
            if oldest < season.start_year {
                let all_stale = self.items.iter()
                    .filter(|e| e.season.start_year == oldest)
                    .all(|e| e.statistics.total_games() == 0 && e.transfer_fee.is_none() && !e.is_loan);
                if all_stale {
                    self.items.retain(|e| e.season.start_year != oldest);
                }
            }
        }
    }

    /// Record season-end snapshot. Saves current stats to history.
    pub fn record_season_end(
        &mut self,
        season: Season,
        old_stats: PlayerStatistics,
        team: &TeamInfo,
        is_loan: bool,
        last_transfer_date: Option<NaiveDate>,
    ) {
        // If player transferred AFTER the season being snapshotted started,
        // stats belong to the transfer's season entry (not the old season).
        if let Some(transfer_date) = last_transfer_date {
            let transfer_season = Season::from_date(transfer_date);
            if transfer_season.start_year > season.start_year {
                if let Some(placeholder) = self.items.iter_mut().find(|e| {
                    e.season.start_year == transfer_season.start_year
                        && e.team_slug == team.slug
                }) {
                    placeholder.statistics = old_stats;
                }
                return;
            }
        }

        // Merge into existing entry for this season + team (if any)
        if let Some(existing) = self.items.iter_mut().find(|e| {
            e.season.start_year == season.start_year && e.team_slug == team.slug
        }) {
            if old_stats.total_games() > 0 {
                if existing.statistics.total_games() == 0 {
                    existing.statistics = old_stats;
                } else {
                    existing.statistics.merge_from(&old_stats);
                }
            }
            return;
        }

        // No entry yet. Skip 0-game phantom if other season entries already exist.
        if old_stats.total_games() == 0 {
            let has_any = self.items.iter()
                .any(|e| e.season.start_year == season.start_year);
            if has_any {
                return;
            }
        }

        self.push_or_replace(PlayerStatisticsHistoryItem {
            season,
            team_name: team.name.clone(),
            team_slug: team.slug.clone(),
            team_reputation: team.reputation,
            league_name: team.league_name.clone(),
            league_slug: team.league_slug.clone(),
            is_loan,
            transfer_fee: None,
            statistics: old_stats,
            seq_id: 0, // placeholder
        });
    }

    // ── View ─────────────────────────────────────────────────────

    /// Returns history items grouped by (season, team, is_loan), merged and sorted
    /// for display. Most recent season first, then by created_at within same season.
    pub fn view_items(&self) -> Vec<PlayerStatisticsHistoryItem> {
        struct Group {
            key_idx: usize,
            stats: PlayerStatistics,
            rating_sum: f32,
            rating_count: u16,
        }

        let mut groups: Vec<Group> = Vec::new();
        let mut keys: Vec<PlayerStatisticsHistoryItem> = Vec::new();

        for item in &self.items {
            let games = item.statistics.total_games();

            let group_idx = keys.iter().position(|k| {
                k.season.start_year == item.season.start_year
                    && k.team_slug == item.team_slug
                    && k.is_loan == item.is_loan
            });

            if let Some(idx) = group_idx {
                let group = &mut groups[idx];
                let key = &mut keys[group.key_idx];

                group.stats.played += item.statistics.played;
                group.stats.played_subs += item.statistics.played_subs;
                group.stats.goals += item.statistics.goals;
                group.stats.assists += item.statistics.assists;
                group.stats.player_of_the_match += item.statistics.player_of_the_match;
                group.stats.conceded += item.statistics.conceded;
                group.stats.clean_sheets += item.statistics.clean_sheets;
                group.rating_sum += item.statistics.average_rating * games as f32;
                group.rating_count += games;

                if item.transfer_fee.is_some() && key.transfer_fee.is_none() {
                    key.transfer_fee = item.transfer_fee;
                }
                if item.seq_id > key.seq_id {
                    if !item.league_name.is_empty() {
                        key.league_name = item.league_name.clone();
                        key.league_slug = item.league_slug.clone();
                    }
                    if games > 0 {
                        key.seq_id = item.seq_id;
                    }
                }
            } else {
                let key_idx = keys.len();
                keys.push(PlayerStatisticsHistoryItem {
                    season: item.season.clone(),
                    team_name: item.team_name.clone(),
                    team_slug: item.team_slug.clone(),
                    team_reputation: item.team_reputation,
                    league_name: item.league_name.clone(),
                    league_slug: item.league_slug.clone(),
                    is_loan: item.is_loan,
                    transfer_fee: item.transfer_fee,
                    statistics: PlayerStatistics::default(), // filled below
                    seq_id: item.seq_id,
                });
                groups.push(Group {
                    key_idx,
                    stats: PlayerStatistics {
                        played: item.statistics.played,
                        played_subs: item.statistics.played_subs,
                        goals: item.statistics.goals,
                        assists: item.statistics.assists,
                        penalties: item.statistics.penalties,
                        player_of_the_match: item.statistics.player_of_the_match,
                        yellow_cards: item.statistics.yellow_cards,
                        red_cards: item.statistics.red_cards,
                        shots_on_target: item.statistics.shots_on_target,
                        tackling: item.statistics.tackling,
                        passes: item.statistics.passes,
                        average_rating: 0.0, // computed from sum/count
                        conceded: item.statistics.conceded,
                        clean_sheets: item.statistics.clean_sheets,
                    },
                    rating_sum: item.statistics.average_rating * games as f32,
                    rating_count: games,
                });
            }
        }

        // Finalize: set computed stats into keys
        for group in &groups {
            let key = &mut keys[group.key_idx];
            key.statistics = PlayerStatistics {
                played: group.stats.played,
                played_subs: group.stats.played_subs,
                goals: group.stats.goals,
                assists: group.stats.assists,
                penalties: group.stats.penalties,
                player_of_the_match: group.stats.player_of_the_match,
                yellow_cards: group.stats.yellow_cards,
                red_cards: group.stats.red_cards,
                shots_on_target: group.stats.shots_on_target,
                tackling: group.stats.tackling,
                passes: group.stats.passes,
                average_rating: if group.rating_count > 0 {
                    group.rating_sum / group.rating_count as f32
                } else {
                    0.0
                },
                conceded: group.stats.conceded,
                clean_sheets: group.stats.clean_sheets,
            };
        }

        // Fill in missing seasons so every year has at least 1 entry.
        // Use the last known non-loan team for gaps.
        if !keys.is_empty() {
            // Sort chronologically first to find gaps
            keys.sort_by(|a, b| {
                a.season.start_year.cmp(&b.season.start_year)
                    .then(a.seq_id.cmp(&b.seq_id))
            });

            let min_year = keys.first().unwrap().season.start_year;
            let max_year = keys.last().unwrap().season.start_year;

            let mut fill: Vec<PlayerStatisticsHistoryItem> = Vec::new();

            for year in min_year..=max_year {
                let has_entry = keys.iter().any(|k| k.season.start_year == year);
                if has_entry {
                    continue;
                }

                // Find the last non-loan team before this gap
                let parent = keys.iter()
                    .rev()
                    .find(|k| k.season.start_year < year && !k.is_loan);

                // Fall back to last team of any kind before the gap
                let template = parent
                    .or_else(|| keys.iter().rev().find(|k| k.season.start_year < year));

                if let Some(tmpl) = template {
                    let season = Season::new(year);
                    fill.push(PlayerStatisticsHistoryItem {
                        season: season.clone(),
                        team_name: tmpl.team_name.clone(),
                        team_slug: tmpl.team_slug.clone(),
                        team_reputation: tmpl.team_reputation,
                        league_name: tmpl.league_name.clone(),
                        league_slug: tmpl.league_slug.clone(),
                        is_loan: false,
                        transfer_fee: None,
                        statistics: PlayerStatistics::default(),
                        seq_id: 0,
                    });
                }
            }

            keys.extend(fill);
        }

        // Filter out 0-game parent placeholders when the same season has a loan entry.
        // These are artefacts from departure_loan / loan_return bookkeeping.
        let remove_indices: Vec<usize> = (0..keys.len()).filter(|&i| {
            let entry = &keys[i];
            if entry.statistics.total_games() > 0 || entry.is_loan || entry.transfer_fee.is_some() {
                return false;
            }
            keys.iter().any(|other| {
                other.season.start_year == entry.season.start_year && other.is_loan
            })
        }).collect();
        for &i in remove_indices.iter().rev() {
            keys.remove(i);
        }

        // Sort: most recent season first, then seq_id desc within same season
        keys.sort_by(|a, b| {
            b.season.start_year.cmp(&a.season.start_year)
                .then(b.seq_id.cmp(&a.seq_id))
        });

        keys
    }

    // ── Internal ─────────────────────────────────────────────────

    /// Push with auto-assigned seq_id.
    fn push_item(&mut self, mut item: PlayerStatisticsHistoryItem) {
        self.assign_seq(&mut item);
        self.items.push(item);
    }

    /// Push a new history entry, deduplicating against existing entries for the
    /// same season + team to preserve correct seq_id ordering.
    fn push_or_replace(&mut self, mut item: PlayerStatisticsHistoryItem) {
        self.assign_seq(&mut item);
        let new_games = item.statistics.played + item.statistics.played_subs;

        // Find existing entry for same season + team with 0 games
        let zero_games_idx = self.items.iter().position(|existing| {
            existing.season.start_year == item.season.start_year
                && existing.team_slug == item.team_slug
                && (existing.statistics.played + existing.statistics.played_subs) == 0
        });

        if let Some(idx) = zero_games_idx {
            // Replace 0-game placeholder only if new entry has actual games;
            // preserve original seq_id, transfer info, and loan flag
            if new_games > 0 {
                let original_seq_id = self.items[idx].seq_id;
                let transfer_fee = self.items[idx].transfer_fee;
                let original_is_loan = self.items[idx].is_loan;
                self.items[idx] = item;
                self.items[idx].seq_id = original_seq_id;
                self.items[idx].transfer_fee = transfer_fee;
                self.items[idx].is_loan = original_is_loan;
            }
        } else if new_games == 0 {
            // New 0-game entry: only push if no entry exists at all for this season + team
            let any_existing = self.items.iter().any(|existing| {
                existing.season.start_year == item.season.start_year
                    && existing.team_slug == item.team_slug
            });
            if !any_existing {
                self.items.push(item);
            }
        } else {
            self.items.push(item);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn make_team(name: &str, slug: &str) -> TeamInfo {
        TeamInfo {
            name: name.to_string(),
            slug: slug.to_string(),
            reputation: 100,
            league_name: "Serie A".to_string(),
            league_slug: "serie-a".to_string(),
        }
    }

    fn make_stats(played: u16, rating: f32) -> PlayerStatistics {
        let mut s = PlayerStatistics::default();
        s.played = played;
        s.average_rating = rating;
        s
    }

    // ---------------------------------------------------------------
    // Track A: Loan with 0 games at original team
    // ---------------------------------------------------------------

    /// After loan to Inter with 0 games at Juventus:
    /// view = [2025/2026 Inter Loan 0g]
    #[test]
    fn zero_games_after_loan() {
        let mut h = PlayerStatisticsHistory::new();
        let juve = make_team("Juventus", "juventus");
        let inter = make_team("Inter", "inter");

        h.record_departure_loan(
            make_stats(0, 0.0),
            &juve, &juve, &inter, false,
            make_date(2026, 1, 15),
        );

        let view = h.view_items();
        assert_eq!(view.len(), 1);
        assert_eq!(view[0].season.start_year, 2025);
        assert_eq!(view[0].team_slug, "inter");
        assert!(view[0].is_loan);
        assert_eq!(view[0].statistics.played, 0);
    }

    /// After season 2025/2026 ends + loan return + next season:
    /// view = [2026/2027 Juventus 0g, 2025/2026 Inter Loan 0g]
    #[test]
    fn zero_games_after_season_end() {
        let mut h = PlayerStatisticsHistory::new();
        let juve = make_team("Juventus", "juventus");
        let inter = make_team("Inter", "inter");

        // Loan to Inter (2025/2026)
        h.record_departure_loan(
            make_stats(0, 0.0),
            &juve, &juve, &inter, false,
            make_date(2026, 1, 15),
        );
        // Season end at Inter
        h.record_season_end(
            Season::new(2025), make_stats(0, 0.0), &inter, true,
            Some(make_date(2026, 1, 15)),
        );
        // Loan return
        h.record_loan_return(
            PlayerStatistics::default(), &inter,
            make_date(2026, 5, 28),
        );
        // Next season ends at Juventus (0 games)
        h.record_season_end(
            Season::new(2026), make_stats(0, 0.0), &juve, false, None,
        );

        let view = h.view_items();
        assert_eq!(view.len(), 2);
        assert_eq!(view[0].season.start_year, 2026);
        assert_eq!(view[0].team_slug, "juventus");
        assert_eq!(view[0].statistics.played, 0);
        assert_eq!(view[1].season.start_year, 2025);
        assert_eq!(view[1].team_slug, "inter");
        assert!(view[1].is_loan);
    }

    /// After re-loan to Inter (0 games at Juventus):
    /// view = [2026/2027 Inter Loan 0g, 2025/2026 Inter Loan 0g]
    #[test]
    fn zero_games_after_reloan() {
        let mut h = PlayerStatisticsHistory::new();
        let juve = make_team("Juventus", "juventus");
        let inter = make_team("Inter", "inter");

        // Loan to Inter (2025/2026)
        h.record_departure_loan(
            make_stats(0, 0.0),
            &juve, &juve, &inter, false,
            make_date(2026, 1, 15),
        );
        // Season end at Inter
        h.record_season_end(
            Season::new(2025), make_stats(0, 0.0), &inter, true,
            Some(make_date(2026, 1, 15)),
        );
        // Loan return
        h.record_loan_return(
            PlayerStatistics::default(), &inter,
            make_date(2026, 5, 28),
        );
        // Next season ends at Juventus (0 games)
        h.record_season_end(
            Season::new(2026), make_stats(0, 0.0), &juve, false, None,
        );
        // Re-loan to Inter (0 games at Juventus)
        h.record_departure_loan(
            make_stats(0, 0.0),
            &juve, &juve, &inter, false,
            make_date(2026, 9, 1),
        );

        let view = h.view_items();
        assert_eq!(view.len(), 2);
        assert_eq!(view[0].season.start_year, 2026);
        assert_eq!(view[0].team_slug, "inter");
        assert!(view[0].is_loan);
        assert_eq!(view[1].season.start_year, 2025);
        assert_eq!(view[1].team_slug, "inter");
        assert!(view[1].is_loan);
    }

    // ---------------------------------------------------------------
    // Track B: Loan with non-0 games at original team
    // ---------------------------------------------------------------

    /// After loan to Inter with 1 game at Juventus:
    /// view = [2025/2026 Inter Loan 0g, 2025/2026 Juventus 1g]
    #[test]
    fn with_games_after_loan() {
        let mut h = PlayerStatisticsHistory::new();
        let juve = make_team("Juventus", "juventus");
        let inter = make_team("Inter", "inter");

        h.record_departure_loan(
            make_stats(1, 7.0),
            &juve, &juve, &inter, false,
            make_date(2026, 1, 15),
        );

        let view = h.view_items();
        assert_eq!(view.len(), 2);
        assert_eq!(view[0].season.start_year, 2025);
        assert_eq!(view[0].team_slug, "inter");
        assert!(view[0].is_loan);
        assert_eq!(view[0].statistics.played, 0);
        assert_eq!(view[1].season.start_year, 2025);
        assert_eq!(view[1].team_slug, "juventus");
        assert!(!view[1].is_loan);
        assert_eq!(view[1].statistics.played, 1);
    }

    /// After season 2025/2026 ends + loan return + next season:
    /// view = [2026/2027 Juventus 0g, 2025/2026 Inter Loan 0g, 2025/2026 Juventus 1g]
    #[test]
    fn with_games_after_season_end() {
        let mut h = PlayerStatisticsHistory::new();
        let juve = make_team("Juventus", "juventus");
        let inter = make_team("Inter", "inter");

        // Loan to Inter with 1 game at Juve (2025/2026)
        h.record_departure_loan(
            make_stats(1, 7.0),
            &juve, &juve, &inter, false,
            make_date(2026, 1, 15),
        );
        // Season end at Inter
        h.record_season_end(
            Season::new(2025), make_stats(0, 0.0), &inter, true,
            Some(make_date(2026, 1, 15)),
        );
        // Loan return
        h.record_loan_return(
            PlayerStatistics::default(), &inter,
            make_date(2026, 5, 28),
        );
        // Next season ends at Juventus (0 games)
        h.record_season_end(
            Season::new(2026), make_stats(0, 0.0), &juve, false, None,
        );

        let view = h.view_items();
        assert_eq!(view.len(), 3);
        assert_eq!(view[0].season.start_year, 2026);
        assert_eq!(view[0].team_slug, "juventus");
        assert_eq!(view[0].statistics.played, 0);
        assert_eq!(view[1].season.start_year, 2025);
        assert_eq!(view[1].team_slug, "inter");
        assert!(view[1].is_loan);
        assert_eq!(view[2].season.start_year, 2025);
        assert_eq!(view[2].team_slug, "juventus");
        assert_eq!(view[2].statistics.played, 1);
    }

    /// After re-loan to Inter (1 game at Juventus before re-loan):
    /// view = [2026/2027 Inter Loan 0g, 2026/2027 Juventus 1g,
    ///         2025/2026 Inter Loan 0g, 2025/2026 Juventus 1g]
    #[test]
    fn with_games_after_reloan() {
        let mut h = PlayerStatisticsHistory::new();
        let juve = make_team("Juventus", "juventus");
        let inter = make_team("Inter", "inter");

        // Loan to Inter with 1 game at Juve (2025/2026)
        h.record_departure_loan(
            make_stats(1, 7.0),
            &juve, &juve, &inter, false,
            make_date(2026, 1, 15),
        );
        // Season end at Inter
        h.record_season_end(
            Season::new(2025), make_stats(0, 0.0), &inter, true,
            Some(make_date(2026, 1, 15)),
        );
        // Loan return
        h.record_loan_return(
            PlayerStatistics::default(), &inter,
            make_date(2026, 5, 28),
        );
        // Next season ends at Juventus (0 games)
        h.record_season_end(
            Season::new(2026), make_stats(0, 0.0), &juve, false, None,
        );
        // Re-loan to Inter (1 game at Juventus before re-loan)
        h.record_departure_loan(
            make_stats(1, 7.0),
            &juve, &juve, &inter, false,
            make_date(2026, 9, 1),
        );

        let view = h.view_items();
        assert_eq!(view.len(), 4);
        assert_eq!(view[0].season.start_year, 2026);
        assert_eq!(view[0].team_slug, "inter");
        assert!(view[0].is_loan);
        assert_eq!(view[0].statistics.played, 0);
        assert_eq!(view[1].season.start_year, 2026);
        assert_eq!(view[1].team_slug, "juventus");
        assert_eq!(view[1].statistics.played, 1);
        assert_eq!(view[2].season.start_year, 2025);
        assert_eq!(view[2].team_slug, "inter");
        assert!(view[2].is_loan);
        assert_eq!(view[3].season.start_year, 2025);
        assert_eq!(view[3].team_slug, "juventus");
        assert_eq!(view[3].statistics.played, 1);
    }
}
