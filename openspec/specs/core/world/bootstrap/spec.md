# core/world/bootstrap Specification

## Purpose
Brings a freshly-loaded world up to a runnable, internally consistent state — league tables, career histories, id sequences, nationality passports — and keeps that derived state caught up as the world changes, including the world-level national-team call-up cycle that depends on it.

## Requirements

### Requirement: Initial world construction seeds all derived state
When a simulation world is constructed from raw continent/country/club data, the system SHALL populate every piece of derived state the daily tick depends on before the first tick runs: league tables, player career-history entries, player nationality continent/region tags, the transfer-geography map, and each club's market and loan-placement ledgers.

#### Scenario: World is built from continents
- **WHEN** a world is constructed from a set of continents, countries, and clubs
- **THEN** every league with participating teams has an initialised table, every player has a seeded career-history entry for their current team, every player has a nationality continent and region assigned where the nationality's country is known, and every club has market/loan ledgers bootstrapped from the players and loanees it was shipped with

### Requirement: Club career-history identity aliases youth and reserve squads to the main team
When seeding or catching up a player's career-history "team" entry, the system SHALL record senior competing squads (main team, and reserve squads that compete in their own real division) under their own identity, and SHALL record non-competing developmental squads (youth age groups, a reserve squad with no identity of its own) under the parent club's main-team identity.

#### Scenario: Youth player never leaves the academy
- **WHEN** a player has only ever played for a club's youth squad
- **THEN** his career-history entry for that spell shows the parent club's main-team name, league, and reputation rather than the youth squad's own identity

#### Scenario: Reserve team with its own league
- **WHEN** a team type competes in its own real division (e.g. a "B" or "Second" team)
- **THEN** its players' career-history entries use that team's own name and league, not the main team's

### Requirement: Catch-up seeding is idempotent and incremental
Career-history seeding SHALL be safe to re-run on every tick: it SHALL only touch players whose current-season history entry is still missing, and SHALL skip clubs and teams where no player needs seeding.

#### Scenario: Steady-state tick with nothing new
- **WHEN** the incremental seeding pass runs on a tick where every player already has a current-season history entry
- **THEN** no player, team, or club history data is modified

#### Scenario: Mid-season academy intake
- **WHEN** a new player is generated (youth intake, regeneration, or a newly created club) after world construction
- **THEN** the next incremental seeding pass gives that player a career-history entry within one simulated tick

### Requirement: Player and staff id sequences never collide with shipped data
After world construction (and after any future load path), the system SHALL advance its procedural id generators past the highest player id and highest staff id present anywhere in the loaded world (active rosters, retired players, generated national squads, and the free-agent pool for players; club rosters and the unemployed pool for staff).

#### Scenario: Academy generates a new player after load
- **WHEN** the world has been loaded and id sequences seeded
- **THEN** any newly generated player or staff member receives an id strictly greater than every id already present in the loaded world

### Requirement: Nationality passport data is re-stamped at each transfer-window boundary
The system SHALL fill in a player's nationality continent and region only when at least one of the two is still unset, leaving an already-stamped player untouched, and SHALL re-run this pass for every roster, retired player, generated national-team squad member, and free agent on the first day of January and July.

#### Scenario: Player generated mid-season
- **WHEN** a player is created after world construction with no nationality continent or region set
- **THEN** the next January 1st or July 1st reseed pass assigns both fields from the player's home country, provided that country is registered

#### Scenario: Already-stamped player is left alone
- **WHEN** a player already has both nationality continent and region set
- **THEN** a reseed pass does not modify either field

### Requirement: World-level national-team call-ups draw candidates from every country regardless of club location
At the start of each international break or tournament window, the system SHALL build a global pool of eligible senior candidates spanning every country's clubs (not limited to same-continent clubs), select each country's senior squad from that pool, then separately select each country's U21 squad from a candidate pool that excludes any player already selected to a senior squad in the same window.

#### Scenario: Player at a foreign club is still call-up eligible
- **WHEN** a player's nationality country calls up its squad for an international break
- **THEN** a player who plays his club football on a different continent from his nationality is eligible for selection

#### Scenario: U21 and senior selections never overlap
- **WHEN** both senior and U21 squads are selected for the same call-up window
- **THEN** no player appears in both a country's senior squad and its U21 squad for that window

### Requirement: International windows follow the season week grid
Each season SHALL have exactly four international windows. Seasons are identified by their August start year. The windows sit at season weeks 0, 5, 10 and 29, which are the September, October, November and March windows. Season week 0 begins on the first Monday on or after 29 August of the season's start year, and week `n` begins `7n` days later.

Each window SHALL open on its week's Monday and close on the Tuesday eight days later. Each window therefore holds exactly one Friday, one Saturday and one Sunday. Each window SHALL offer three national-team matchdays: its Thursday, its Sunday and its closing Tuesday.

Every part of the simulation that asks whether a date is inside an international window SHALL get the same answer: call-ups, national-team fixtures and club scheduling. The June–July tournament window is separate from these four and is unchanged.

#### Scenario: The grid reproduces FIFA's real autumn windows
- **WHEN** the September, October and November windows are computed for the seasons starting in 2023, 2024 and 2025
- **THEN** they open on 4 September, 9 October and 13 November 2023; on 2 September, 7 October and 11 November 2024; and on 1 September, 6 October and 10 November 2025

#### Scenario: The 2027-28 windows
- **WHEN** the windows of the season starting in 2027 are computed
- **THEN** they run 30 August–7 September 2027, 4–12 October 2027, 8–16 November 2027 and 20–28 March 2028

#### Scenario: Window shape across many seasons
- **WHEN** the windows of every season from 2026 to 2060 are computed
- **THEN** each season has four windows in date order, each opens on a Monday and closes on the Tuesday eight days later, and each contains exactly one Friday, one Saturday and one Sunday

### Requirement: Called-up players are away for the whole window
National-team call-ups for an international window SHALL happen on the window's opening day. A called-up player SHALL be on international duty from the opening day through the closing day inclusive, and SHALL be available to his club again from the day after the window closes.

#### Scenario: A player is called up for the October window
- **WHEN** a player is called up on the opening Monday of an international window
- **THEN** he holds international duty on every day up to and including the closing Tuesday, and is available for club selection on the Wednesday after it

#### Scenario: No call-up outside a window's opening day
- **WHEN** the simulation ticks any day that is not the opening day of an international window or of the tournament window
- **THEN** no international-window call-up is made
