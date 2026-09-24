# core/shared/indexes Specification

## Purpose
Resolves world entities by numeric id or URL slug through cached positional shortcuts with brute-force fallback, and aggregates scattered club scouting state into presentation-ready rows.

## Requirements

### Requirement: Entity lookup by id resolves even when position indexes are stale
The system SHALL resolve a continent, country, league, club, team, player, or staff member by its numeric id, preferring a cached positional shortcut when available but always falling back to an id-based or brute-force search of the world data so a lookup never fails solely because a cached position is outdated.

#### Scenario: Cached position still points at the requested entity
- **WHEN** a player is looked up by id and the cached position resolves to a team whose roster still contains that player id
- **THEN** the player is returned directly from that cached position without scanning the rest of the world

#### Scenario: Cached position has gone stale after a roster change
- **WHEN** a player is looked up by id and the cached position no longer resolves to a team containing that player (for example after a transfer moved the player to a different club)
- **THEN** the system falls back to a full scan of every continent, country, club, and team (including free agents) and still returns the player if present

#### Scenario: League lookup covers competitions stored outside the standard league list
- **WHEN** a league id belongs to a domestic cup or a grouped-competition playoff rather than an entry in a country's regular league list
- **THEN** the lookup still finds it by scanning each country's domestic cup and playoff entries after the positional and standard-league fallbacks are exhausted

### Requirement: Mutable entity lookups mirror the read paths
The system SHALL provide mutable-reference lookups for continent, country, league, club, team, and player that use the same positional-fast-path-then-fallback strategy as their read-only counterparts, so callers that need to modify an entity do not have to re-implement the resolution logic.

#### Scenario: Mutable player lookup after a stale index
- **WHEN** a caller requests a mutable reference to a player whose cached position is stale
- **THEN** the system locates the player's current array position via a fresh scan and returns a mutable reference through that position, without requiring a manual index rebuild first

### Requirement: Slug-based lookup resolves countries, leagues, and teams for routing
The system SHALL maintain a slug index mapping each country's, league's, and team's URL-friendly slug to its numeric id, so web-facing code can resolve a route segment to an entity id without scanning the world.

#### Scenario: Team slug resolves to its id
- **WHEN** a team's slug is looked up in the slug index
- **THEN** the team's numeric id is returned if a team with that slug exists, and nothing otherwise

### Requirement: Indexes can be rebuilt fully or incrementally
The system SHALL support rebuilding all entity/location/position/slug indexes from the current world state in one pass, and separately support rebuilding only the player-related indexes, so a caller that knows only players moved (e.g. after a transfer) does not pay the cost of re-indexing every entity type.

#### Scenario: Full refresh after world data changes
- **WHEN** the full index refresh runs
- **THEN** every league, club, team, player, and staff location/position entry and every slug entry is rebuilt from the current continents/countries/clubs/teams structure, and stale positional entries from the previous refresh are discarded first

#### Scenario: Dirty-flag-gated refresh skips unnecessary work
- **WHEN** the world is asked to refresh indexes only if a player actually moved that day
- **THEN** the refresh is skipped entirely if no transfer marked the player index dirty, and runs (and clears the dirty marker) otherwise

### Requirement: Club and continent membership can be derived from a club id alone
The system SHALL let callers resolve a club's country and continent, and list a club's continental-competition fixtures (Champions League, Europa League, Conference League, Copa Libertadores), from just the club's id.

#### Scenario: Continental fixtures filtered to one club
- **WHEN** a club's continental matches are requested
- **THEN** only matches from that club's continent's competitions where the club appears as the home or away side are returned, each tagged with which competition it belongs to

### Requirement: Player and staff monitoring/scouting state is aggregated into presentation-ready rows
The system SHALL assemble scouting and monitoring information scattered across clubs' scouting departments into per-player and per-staff summary rows (status, confidence, observation counts, workload) ready for direct display, so presentation code does not need to traverse club scouting state itself.

#### Scenario: A player being watched by multiple clubs
- **WHEN** monitoring details are requested for a player who is being scouted by more than one club
- **THEN** one row per club is returned, excluding the player's own current club and loan parent, ordered with actively-monitoring clubs first by most recent observation and all other interested clubs after

### Requirement: Person full names produce a single consistent display and slug across nicknames and mononyms
The system SHALL derive a person's displayed first/last name and a URL-safe slug from their stored first name, last name, optional middle name, and optional nickname, using one consistent rule: an effective (non-empty) nickname is shown alone, a person with no last name is treated as a mononym and shown by their first name alone, and everyone else is shown as first plus last name.

#### Scenario: Empty nickname is treated as no nickname
- **WHEN** a person's stored nickname is an empty string
- **THEN** display and slug generation behave as if no nickname were set, falling through to the mononym/standard-name rule

#### Scenario: Slug generation folds diacritics and non-Latin scripts to ASCII
- **WHEN** a slug is generated from a display name containing accented Latin characters or a non-Latin script (e.g. Cyrillic)
- **THEN** the result contains only lowercase ASCII alphanumeric characters and single dashes between segments, with no leading, trailing, or doubled dashes

### Requirement: Currency and city-location values are typed wrappers
The system SHALL represent monetary amounts as a currency-tagged numeric value and a physical location as a reference to a city id, so other modules pass these through a single shared type rather than raw numbers.

#### Scenario: A currency value carries both amount and currency together
- **WHEN** a monetary amount is constructed via the shared currency value type
- **THEN** the amount and its currency travel together as one value wherever it is passed or cloned
