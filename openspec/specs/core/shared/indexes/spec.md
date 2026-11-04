# core/shared/indexes Specification

## Purpose
Resolves world entities by numeric id or URL slug through cached positional shortcuts with brute-force fallback, and aggregates scattered club scouting state into presentation-ready rows.

## Requirements

### Requirement: Entity lookup by id resolves even when position indexes are stale
The system SHALL resolve a continent, country, league, club, team, player, or staff member by its numeric id. It SHALL prefer the place the index recorded for that id, and SHALL confirm that the entity found there carries the requested id before returning it. It SHALL always fall back to an id-based or brute-force search of the world data, so that a lookup never fails solely because a recorded place is outdated. A player who sits in the global free-agent pool SHALL be resolved through the pool's own id index, without scanning any club roster first. A player in a country's retired list or in its senior national team's generated squad SHALL be resolved through his recorded place, without scanning any club roster. A league SHALL be resolved through its recorded place whether it is a regular league, a domestic cup or a grouped-competition playoff.

#### Scenario: Cached position still points at the requested entity
- **WHEN** a player is looked up by id and the recorded team still holds that player at the recorded roster slot
- **THEN** the player is returned directly from that slot, without scanning his team's roster or any other part of the world

#### Scenario: Player's roster slot shifted inside the same team
- **WHEN** a player is looked up by id and the recorded team still contains him, but at a different roster slot (for example because a teammate listed before him left)
- **THEN** he is found by searching that one team, without scanning any other team in the world

#### Scenario: Cached position has gone stale after a roster change
- **WHEN** a player is looked up by id and the recorded place no longer resolves to a team containing that player (for example after a transfer moved the player to a different club)
- **THEN** the system falls back to a full scan of every continent, country, club, and team (including free agents) and still returns the player if present

#### Scenario: A pooled free agent is looked up
- **WHEN** a player who sits in the global free-agent pool is looked up by id
- **THEN** he is returned through the pool's id index without any continent, country, club or team being scanned

#### Scenario: A retired or national generated player is looked up
- **WHEN** a player who sits in a country's retired list, or in its senior national team's generated squad, is looked up by id after the indexes were last reconciled
- **THEN** he is returned through his recorded place, without any club roster in the world being scanned, and the index reports no club or team for him

#### Scenario: League lookup covers competitions stored outside the standard league list
- **WHEN** a league id belongs to a domestic cup or a grouped-competition playoff rather than an entry in a country's regular league list
- **THEN** the lookup resolves it through its recorded place, read-only and mutable alike, without scanning any other country's cups or playoffs

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

### Requirement: Indexes are built once and then reconciled against what changed
The system SHALL build the id, location, position and slug indexes from the whole world once, when the world is loaded. After that it SHALL bring them up to date only by reconciling them against what changed, under these rules:
- A roster whose membership has not changed since it was last indexed SHALL NOT have its members walked again. A roster here means any of: a team's players, a team's staff, a country's retired players, or its senior national team's generated squad.
- A roster whose membership changed SHALL have its members' entries derived again.
- A member who left that roster SHALL lose his entry unless another roster now holds him.
- Clubs, teams, leagues, domestic cups and playoffs that the indexes have not seen yet SHALL be added.

After a reconcile, the indexes SHALL be identical to indexes built from scratch from the same world.

#### Scenario: Reconciled indexes equal a fresh build
- **WHEN** a world is simulated for many days with its indexes only ever reconciled
- **THEN** at the end of every day the indexes equal indexes built from scratch from that day's world: every entry, recorded place, roster slot and slug

#### Scenario: Unchanged rosters are not walked
- **WHEN** no roster's membership has changed since the last reconcile
- **THEN** the reconcile visits no player and no staff member, and every entry stays as it was

#### Scenario: A transferred player follows his new roster
- **WHEN** a player leaves one club's team for another club's team and the indexes are then reconciled
- **THEN** his entry names the new continent, country, club and team, and the location reported for him is the new one

#### Scenario: A released player loses his roster location
- **WHEN** a player leaves a team for the global free-agent pool and the indexes are then reconciled
- **THEN** the index no longer reports a club or team for him, and he is still found by id through the pool

#### Scenario: A move inside one club is reconciled like any other
- **WHEN** a player moves between two teams of the same club, or graduates from the academy into a team, without any transfer taking place
- **THEN** the next reconcile records his new team and roster slot exactly as it would after a transfer

#### Scenario: A team added after load becomes resolvable
- **WHEN** a team is added to a club after the world was loaded and the indexes are then reconciled
- **THEN** the team, its slug, its players and its staff all resolve by id through the indexes

### Requirement: Staff locations stay current after staff move
The system SHALL keep each employed staff member's location (continent, country, club and team) and roster place current through the same reconciliation as players. A coach, scout or other staff member who is hired, dismissed or moved after load SHALL be resolvable by id through the index after the next reconcile, without walking every team in the world.

#### Scenario: A coach who changes clubs is found at his new club
- **WHEN** a staff member leaves one club's team, joins another club's team, and the indexes are then reconciled
- **THEN** looking him up by id returns him together with his new team without scanning other teams, and the location reported for him names the new club and team

#### Scenario: A scout hired after load is indexed
- **WHEN** a club hires a scout who was not on any team when the world was loaded, and the indexes are then reconciled
- **THEN** the scout is resolved by id through the index

### Requirement: Country-scoped lookups are reconciled, not rebuilt
The system SHALL keep the per-country lookup that a country uses while processing its own match results between ticks. Before each use it SHALL reconcile that lookup against the country's rosters that changed, instead of rebuilding it from every player in the country. Every player, team and club lookup made through it SHALL answer exactly what a lookup built from scratch at that moment would answer:
- an entity present in the country is found at its current position;
- an entity absent from the country yields nothing.

#### Scenario: Same answers as a lookup built from scratch
- **WHEN** a country processes its match results after some of its rosters changed earlier in the same tick
- **THEN** every player, team and club lookup through the country-scoped lookup returns the same entity, or nothing, as a lookup built from scratch at that point would

#### Scenario: No roster in the country changed
- **WHEN** none of the country's rosters changed since its country-scoped lookup was last reconciled
- **THEN** preparing the lookup for use visits no player

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
