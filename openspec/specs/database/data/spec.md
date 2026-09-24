# database/data Specification

## Purpose
Defines the data contract of the embedded seed database shipped with the application: which football-world entities are guaranteed present, how they relate, and what a consumer of the loaded data may assume without validation.

## Requirements

### Requirement: Single versioned embedded database
The application SHALL ship exactly one compiled database document, embedded at compile time, carrying an explicit version marker. The application SHALL refuse to start (fail fast) if the embedded document's version does not match the version the running binary expects.

#### Scenario: Version mismatch is fatal, not degraded
- **WHEN** the embedded database document's version field does not equal the supported version the binary was built against
- **THEN** the application SHALL panic during startup rather than silently loading a partial or reinterpreted dataset

### Requirement: Geographic hierarchy is fully populated
The database SHALL carry continents, countries, and national competitions as top-level entities, with every country assigned to exactly one continent.

#### Scenario: Every country resolves to a continent
- **WHEN** any country record is read from the database
- **THEN** its continent id SHALL identify one of the continent records also present in the database

### Requirement: Countries carry regulatory, economic and demographic settings
Every country record SHALL carry a reputation score, pricing settings, and a skin-color population distribution (white/black/metis percentages) usable for deterministic player-appearance generation, with a documented default distribution when a country omits it.

#### Scenario: A country without an authored skin distribution still generates consistent appearances
- **WHEN** a country record omits its population distribution
- **THEN** the loaded record SHALL fall back to a fixed default distribution (50% white / 20% black / 30% metis) rather than leaving the field absent

### Requirement: Named domestic cups and transfer-market cards are optional per country
The database MAY carry a named domestic cup (e.g. "FA Cup", "Copa del Rey") and a transfer-market profile (import/export corridors, diaspora shares, foreign-player share) for a country. Absence of either MUST be representable and MUST NOT be treated as an error by any consumer.

#### Scenario: A country with no authored cup still has a competition
- **WHEN** a country has no matching entry in the named domestic cups table
- **THEN** the data contract guarantees this is a valid, expected state (the runtime is expected to derive a fallback name rather than treat it as missing data)

#### Scenario: A country's transfer card weights are directionally asymmetric
- **WHEN** two countries both ship transfer cards and have a real-world trade imbalance (e.g. one exports far more players to the other than the reverse)
- **THEN** the shipped corridor weights for that pair SHALL reflect that asymmetry (the export weight from the larger source SHALL exceed the reverse export weight)

### Requirement: Leagues are scoped to a country and carry season/promotion structure
Every league record SHALL declare its enclosing country, a tier, a season start/end window, and promotion/relegation spot counts. A league MAY be marked disabled, in which case it and its clubs are excluded from the playable dataset.

#### Scenario: Disabled leagues do not leak clubs into the loaded world
- **WHEN** a league is marked not enabled
- **THEN** neither that league nor any club whose primary (Main) team points at it SHALL appear in the loaded leagues/clubs collections

### Requirement: Clubs carry a full squad/team structure and are geographically anchored
Every club record SHALL declare its country, one or more teams (each with a team type such as Main/B/Reserve/U18-U23), location, finances, colors, and reputation per team. Satellite reserve-team directories SHALL be folded into their parent club as an additional team rather than appearing as a separate standalone club.

#### Scenario: A satellite second team is not a separate club
- **WHEN** the source data models a club's reserve/B team as its own directory with a `parent_club` reference
- **THEN** the loaded dataset SHALL NOT contain that reserve team as a standalone club entry; its Main team SHALL instead appear as an additional team entry on the referenced parent club

### Requirement: Unmodelled career-history clubs still carry a display name
For clubs that appear only in a player's prior career history (not modelled as playable clubs), the database SHALL ship an id-to-name lookup so career history can render a name without resolving to a full club record. This lookup MUST NOT contain any id also present among the fully modelled clubs or their sub-teams.

#### Scenario: A historic club not in this database still displays by name
- **WHEN** a player's career history references a club id that is not among the loaded club or team ids
- **THEN** a name for that id SHALL be resolvable via the history-club name table

### Requirement: Player records carry identity, ability, contract, and optional career history
Every player record SHALL carry a unique id, name, birth date, nationality, one or more scored positions, a current-ability value, and a potential-ability value. Contract, loan, reputation, and prior-season history fields are optional and MAY be absent for a minimal record.

#### Scenario: A minimal player record still hydrates
- **WHEN** a player record supplies only the required identity/ability/position fields and omits contract, loan, reputation, attrs, and history
- **THEN** the record SHALL still be a valid, loadable player entry

### Requirement: Potential-ability may be encoded as a scouted band
A player's potential-ability field SHALL accept either a positive authoritative value (1..=200) or a negative value encoding a Football-Manager-style scouting band, which resolves to a bounded random potential-ability range at hydration time rather than a fixed number.

#### Scenario: A negative potential-ability value denotes an uncertain band, not a literal ability score
- **WHEN** a player record's potential_ability field is negative
- **THEN** the value SHALL be interpreted as a scouting band selector (whole bands -1..=-10 stepping the resolvable range by 20 points; half bands encoded ×10) rather than as a literal potential score

### Requirement: A player belongs to at most one club, at the club currently fielding him
A player with an active loan SHALL be associated with the borrowing club for squad-placement purposes, while his contractual parent club is retained separately. A player with no club and no loan is a free agent and is not attached to any club's squad.

#### Scenario: A loaned player appears in the borrower's squad, not the parent's
- **WHEN** a player record carries both a parent club id and a loan naming a different borrowing club
- **THEN** the player SHALL be discoverable under the borrowing club's squad and SHALL NOT appear in the parent club's squad listing

### Requirement: Name pools exist per country for generated identities
The database SHALL ship first-name, last-name, and (optionally) nickname pools per country, usable to generate culturally-plausible names for procedurally created people.

#### Scenario: A country with no authored name pool still allows generation
- **WHEN** a country has no matching entry in the names-by-country table
- **THEN** the consuming generator SHALL receive an empty (not missing/erroring) name pool for that country

### Requirement: Senior national competitions do not overlap their qualifying campaigns
The shipped national competition configurations SHALL schedule the senior continental championships so their qualifying campaigns alternate with the World Cup's instead of starting in the same years for the same continent. The UEFA European Championship SHALL run on a four-year cycle whose qualifying years fall between the World Cup's.

#### Scenario: Qualifying years for Europe
- **WHEN** the shipped World Cup and European Championship configurations are asked which years start a qualifying cycle
- **THEN** no year between 2026 and 2040 starts both, and the European Championship starts in 2026, 2030, 2034 and 2038
