# core/league/storages Specification

## Purpose
Owns the league's match-result store: bounded, date-indexed retention and the pre-processing-snapshot-then-sync lifecycle for each stored result.

## Requirements

### Requirement: Match results are retained with a bounded, indexed history
Every processed match result SHALL be stored in the league's match history, indexed by the date it was recorded, with entries older than a configured retention window (three completed seasons by default) evicted without requiring a full scan of the store.

#### Scenario: Query for matches in a date range
- **WHEN** a caller requests matches recorded within a given date range (e.g. to score a week's Player of the Week candidates)
- **THEN** only matches indexed within that range are returned, without needing to scan matches outside it

#### Scenario: Retention window exceeded
- **WHEN** the store is trimmed and some recorded matches fall before the retention cutoff relative to the current date
- **THEN** those matches are removed from both the primary store and the date index

### Requirement: A pre-processing snapshot is later synced with finalized match data
A match result SHALL first be stored as a pre-processing snapshot (before Player of the Match and finalized ratings are computed) and later synced in place, by id, once the finalized values are available, without disturbing the store's date index or creating a duplicate, undated entry.

#### Scenario: Finalized match data arrives after the pre-processing snapshot was stored
- **WHEN** the finalized version of an already-stored match result (same id) is synced into the store
- **THEN** the existing entry is replaced in place using the original recorded date, and no second, date-unindexed entry is created

#### Scenario: Sync attempted for an id never stored
- **WHEN** a sync is attempted for a match id that was never previously pushed into the store
- **THEN** the sync is a no-op and no new entry is created
