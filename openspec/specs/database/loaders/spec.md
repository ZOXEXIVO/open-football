# database/loaders Specification

## Purpose
Defines how the embedded seed database is deserialized into in-memory domain entities, including id resolution, filtering of disabled content, structural folding of satellite clubs, and deduplication of duplicate player records.

## Requirements

### Requirement: The compiled database is parsed exactly once per process
Loading the embedded database SHALL decompress and parse it on first access only; every subsequent load request within the same process SHALL reuse the already-parsed result rather than re-parsing.

#### Scenario: Repeated load calls do not re-parse
- **WHEN** the database is loaded multiple times during a single process's lifetime (e.g. by multiple independent loaders)
- **THEN** the underlying decompress-and-parse work SHALL occur only once, with all callers sharing the same parsed result

### Requirement: Country codes resolve to numeric ids consistently and case-insensitively
Every entity that carries a raw country code (clubs, leagues, name pools) SHALL have that code resolved to the corresponding country's numeric id at load time. An unknown or empty code SHALL resolve to a defined sentinel (id 0) rather than failing the load.

#### Scenario: An unrecognized country code does not abort loading
- **WHEN** an entity's country code does not match any loaded country
- **THEN** its resolved country id SHALL be 0, and loading SHALL continue for the rest of the dataset

#### Scenario: Country code lookups by id are always normalized to lowercase
- **WHEN** a country's code is looked up by its numeric id
- **THEN** the returned code SHALL be lowercase regardless of how it was cased in the source data

### Requirement: Disabled leagues and their clubs are excluded from the loaded tree
A league marked not enabled SHALL be excluded from the loaded league collection. A club whose primary team belongs to a disabled (or otherwise absent) league SHALL be excluded from the loaded club collection.

#### Scenario: A club without a live top-level league disappears from the loaded set
- **WHEN** a club's Main team references a league id that is not among the enabled leagues
- **THEN** that club SHALL NOT appear in the loaded clubs collection

### Requirement: Satellite reserve-team clubs are folded into their parent club, not loaded standalone
A club record flagged as belonging to a parent club (a satellite directory such as a "B team" or numbered reserve side) SHALL NOT appear as its own entry in the loaded clubs collection. Its single team SHALL instead be attached to the referenced parent club as an additional team entry, carrying the enclosing league's id.

#### Scenario: A satellite club's id is unreachable as a standalone club after loading
- **WHEN** the source data models a reserve team as a separate club directory with a parent-club reference
- **THEN** looking up that reserve team's id in the loaded clubs collection SHALL fail, and it SHALL instead be found as a team entry inside its parent club

#### Scenario: The folded-in team keeps its original id and carries the enclosing competition's id
- **WHEN** a satellite team is folded into its parent
- **THEN** the resulting team entry SHALL retain the satellite's own id and name, and SHALL be stamped with the league id of the competition it was found in

### Requirement: Named domestic cups and transfer cards resolve onto countries case-insensitively and independently
A country's named domestic cup SHALL be resolved by folding the cup table's country-slug field with the same URL-safe fold applied to the country's own slug, and matching the two. A country's transfer-market card SHALL be resolved by matching the country's trimmed, lowercased code against the transfer table's code field. Neither resolution depends on the other, and either MAY be absent for a given country without affecting the other.

#### Scenario: A country with a cup but no transfer card (or vice versa) loads correctly
- **WHEN** a country's slug matches an entry in the domestic-cup table but its code has no match in the transfer-card table
- **THEN** the country SHALL load with its domestic cup populated and its transfer card left absent

#### Scenario: A cup keyed by display text follows its country through the fold
- **WHEN** the cup table names its country "czech republic"
- **THEN** the cup SHALL resolve onto the country loaded with slug `czech-republic`

### Requirement: Club identity, team, and league lookups by id are O(1) after first use
Once a database entity collection is loaded, resolving a club, sub-team, or league by its numeric id SHALL not require a linear scan of the full collection on repeated lookups.

#### Scenario: Repeated id lookups do not re-scan the full collection
- **WHEN** many club-by-id or league-by-id lookups are performed after the dataset is loaded
- **THEN** each lookup after the first SHALL resolve via a precomputed index rather than scanning every club or league record

### Requirement: A duplicate player record (same id, differing loan presence) collapses to exactly one, favoring the loan-bearing copy
When two or more player records share the same id, only one SHALL survive in the loaded player index. If any of the duplicate records carries loan information and another does not, the surviving record SHALL be the one that carries the loan information.

#### Scenario: A loanee exported twice (once at the parent club with a loan block, once at the borrowing club without one) collapses to a single record retaining the loan
- **WHEN** the source data contains two records with the same player id — one naming the parent club with loan details, one naming the borrowing club with no loan block
- **THEN** the loaded player index SHALL contain exactly one record for that id, indexed under the borrowing club, and that record SHALL carry the loan information (so the player's contractual parent is still discoverable)

### Requirement: Players are indexed by the club that physically fields them, not their contractual owner
The loaded player index SHALL group players by the club where they physically play: an on-loan player is indexed under the borrowing club, not the contractual parent club, and the parent club's squad listing SHALL NOT include him.

#### Scenario: A loaned player's parent club does not list him in its squad
- **WHEN** a player record carries a parent club id and an active loan to a different club
- **THEN** querying the parent club's squad SHALL NOT return that player, while querying the borrowing club's squad SHALL

### Requirement: Free agents are indexed separately from any club squad
A player record with no parent club and no loan SHALL be routed into a dedicated free-agent list and SHALL NOT occupy any synthetic or zero-value club bucket.

#### Scenario: A clubless player never appears under a club id of zero
- **WHEN** a player record has no club id and no loan
- **THEN** that player SHALL be retrievable from the free-agent list and SHALL NOT be retrievable via any club-squad lookup, including a lookup for club id 0

### Requirement: The highest supplied player id is discoverable for id-sequence seeding
The loaded player index SHALL expose the maximum player id present across both clubbed and free-agent records, so that any downstream id generator can seed past it and avoid collisions.

#### Scenario: The maximum id spans both clubbed and free-agent players
- **WHEN** the loaded dataset's highest player id belongs to a free agent rather than a clubbed player
- **THEN** the exposed maximum id SHALL still reflect that free agent's id

### Requirement: Country slugs are folded to a URL-safe form at load
Each loaded country's slug SHALL be its source slug transliterated to ASCII and lowercased, with every run of other characters collapsed to a single hyphen and no leading or trailing hyphen. Every resulting slug SHALL be non-empty and contain only `a-z`, `0-9` and `-`. Folding SHALL NOT make two distinct source slugs equal.

#### Scenario: Display-text slugs become URL-safe
- **WHEN** the source data gives the slugs "south africa" and "côte d'ivoire"
- **THEN** the loaded countries carry `south-africa` and `cote-d-ivoire`

#### Scenario: Folding keeps countries distinct
- **WHEN** all countries are loaded
- **THEN** the number of distinct folded slugs equals the number of distinct source slugs

### Requirement: A league group without a declared level loads at level 0
A league group record that omits its level SHALL load as level 0, a regional group. A declared level SHALL be carried unchanged into the runtime league.

#### Scenario: Legacy group record
- **WHEN** a league's group record has no level field
- **THEN** the loaded group's level SHALL be 0

#### Scenario: Ranked group record
- **WHEN** a league's group record declares level 1
- **THEN** the runtime league's group carries level 1
