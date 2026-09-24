# web/competition-views Specification

## Purpose
Describes the web pages that show standings, brackets and results for continental, national-team, and domestic
competitions: UEFA-style group+knockout competitions (Champions League, Europa League, Conference League), Copa
Libertadores, national-team competitions (World Cup and continental qualifiers/tournaments), domestic league
standings (with associated awards, transfer and newspaper tabs), domestic cups, and grouped-competition playoffs.

## Requirements

### Requirement: Continental club competition standings page
The system SHALL provide, for each supported continental club competition (Champions League, Europa League,
Conference League, Copa Libertadores), a page showing the current group-stage standings and the knockout bracket
for that competition's active edition.

#### Scenario: Competition has started
- **WHEN** a user requests the Champions League (or Europa League, Conference League, Copa Libertadores) page for
  a language
- **THEN** the system SHALL return the current competition stage, one table per group with club name, played,
  won, drawn, lost, goals for, goals against and points, and any knockout ties with two-leg scores and the
  resolved winner where available

#### Scenario: Competition has not started
- **WHEN** the relevant continent has no groups formed yet and the competition stage is "not started"
- **THEN** the system SHALL return the page with empty groups and knockout ties and a "not started" stage label
  rather than an error

#### Scenario: Unknown club reference
- **WHEN** a group row or knockout tie references a club id that cannot be resolved
- **THEN** the system SHALL render that side using an "unknown" placeholder name and an empty slug rather than
  failing the request

### Requirement: National-team competition standings page
The system SHALL provide a page listing all national-team competitions (global tournaments and continental
qualifiers/tournaments) with their groups and knockout brackets, and a per-competition page reachable by a
slug derived from the competition's name.

#### Scenario: Listing all competitions
- **WHEN** a user requests the national competitions index for a language
- **THEN** the system SHALL return every live competition instance not yet excluded (a completed instance with no
  champion is excluded), each with its confederation label (FIFA for global scope, the continental body otherwise),
  phase, team level, group standings and knockout fixtures

#### Scenario: Selecting one competition by slug
- **WHEN** a user requests a national competition page with a specific competition slug
- **THEN** the system SHALL return only the competition(s) matching that configuration id, and the page title
  SHALL be that competition's name

#### Scenario: Tournament groups supersede qualifying groups
- **WHEN** a competition instance has tournament groups populated
- **THEN** the system SHALL display the tournament groups instead of the qualifying groups

### Requirement: Domestic league standings page
The system SHALL provide, for a given domestic league identified by slug, a page with the league table, the
current/upcoming round's fixtures, continental-reputation cross-links, and top scorers/assisters/rated players
for that league and season.

#### Scenario: Requesting a non-cup league by slug
- **WHEN** a user requests a league page for a slug that resolves to a non-cup league
- **THEN** the system SHALL return the standings table (each row with played/win/draw/lost/goals/points and a
  qualification/promotion/relegation zone marker where applicable), a legend of the zones present, the schedule
  for the most recently started round plus the next round once it is within a day of kickoff, and top-10 lists
  of scorers, assisters and rated players computed from that league's own match records

#### Scenario: Requesting a cup or playoff league by slug
- **WHEN** the slug resolves to a league flagged as a cup
- **THEN** the system SHALL redirect to that competition's playoff bracket page if it is a grouped-competition
  playoff, or to its cup bracket page otherwise, rather than rendering a standings table

#### Scenario: Split-season competition
- **WHEN** the league uses a split season (e.g. Apertura/Clausura)
- **THEN** the system SHALL label the live standings with the current tournament name and additionally provide an
  annual aggregate table across every zone of the competition

### Requirement: Domestic league awards page
The system SHALL provide, for a league, a page of individual and team awards: the latest Player of the Week /
Young Player of the Week, latest Team of the Week / Young Team of the Week, Team of the Year, the current
month's snapshot (player of month, team of month, stat leaders, match count), recent weekly awards, an archive
of past monthly snapshots, and season-level highlights (player/young player of season, top scorer, top assists,
golden glove).

#### Scenario: No awards recorded yet
- **WHEN** a league has not yet produced any weekly, monthly or season awards
- **THEN** the system SHALL return the awards page with each award section empty/`None` rather than erroring

#### Scenario: Monthly snapshot selection
- **WHEN** the league has one or more archived monthly snapshots
- **THEN** the system SHALL show the most recent snapshot as the "current month" and list the remaining
  snapshots (skipping the most recent) as the monthly archive, newest-first, capped at 8 entries

### Requirement: Domestic league newspaper page
The system SHALL provide, for a league, a monthly "newspaper" page of generated news stories about clubs and
players in that league, with a tab badge counting the number of published editions.

#### Scenario: League has published editions
- **WHEN** a league's newsroom holds one or more issues
- **THEN** the system SHALL return the issues newest-first, each with its own masthead, date, mood, lead/secondary/
  run stories, and each story crediting the specific club it concerns

#### Scenario: No editions yet
- **WHEN** the league's newsroom has no issues
- **THEN** the system SHALL return the page in an empty state with no issues and a zero/absent edition badge

### Requirement: Domestic league transfers page
The system SHALL provide, for a league and an optional season, a page of completed transfers involving that
league's clubs and any pending transfer negotiations involving those clubs.

#### Scenario: Filtering by season
- **WHEN** a user requests the league transfers page with a `season` query parameter
- **THEN** the system SHALL return only completed transfers recorded for that season year, and SHALL offer the
  full range of seasons with recorded transfer history as selectable options; omitting the parameter SHALL
  default to the current season

#### Scenario: Completed transfer row detail
- **WHEN** a completed transfer involving a league club is listed
- **THEN** each row SHALL include the player's position, nationality, age, origin and destination clubs
  (or a "free agent" label when there is no origin/destination club), the fee (or a "free" label when the fee is
  zero), whether it was a loan, and the transfer date, ordered by playing position

### Requirement: Domestic cup bracket page
The system SHALL provide, for a domestic cup identified by slug, a page showing the full knockout bracket
(rounds, ties, byes), the champion once decided, and top scorers/assisters for that cup's matches.

#### Scenario: Requesting a cup slug
- **WHEN** a user requests the cup page for a slug that resolves to a domestic cup (not a league standings page
  and not a grouped playoff)
- **THEN** the system SHALL return every round with its ties (each tie showing goals, penalty-shootout tallies
  when applicable, and the winner) and any byes teams received advancing without playing

#### Scenario: Cup decided
- **WHEN** the final round holds exactly one played tie
- **THEN** the system SHALL report the cup as decided and identify the champion; otherwise it SHALL report the
  furthest round reached as the current stage

#### Scenario: Wrong competition type for the slug
- **WHEN** the slug resolves to a non-cup league or to a grouped-competition playoff
- **THEN** the system SHALL redirect to that competition's own page (league standings, or the playoff bracket)
  instead of rendering the cup bracket

### Requirement: Domestic cup and playoff history page
The system SHALL provide a roll-of-honour page for a domestic cup or grouped-competition playoff, listing past
champions and runners-up per edition (and, for MLS-style formats, past Supporters' Shield winners), most recent
first, along with the count of editions and of distinct winning clubs.

#### Scenario: Cup history
- **WHEN** a user requests the history page for a domestic cup slug
- **THEN** the system SHALL return every past champion/runner-up recorded on that cup, newest edition first

#### Scenario: Playoff history with a shield
- **WHEN** a user requests the history page for a grouped-competition playoff that tracks a Supporters' Shield
- **THEN** the system SHALL additionally return the shield's past winners, separate from the championship rows

### Requirement: Grouped-competition playoff bracket page
The system SHALL provide, for a grouped-competition playoff (e.g. an MLS-style conference/cross-conference
bracket) identified by slug, a page with the bracket organized into sections (one per conference plus a
cross-conference final when the format calls for it, or a single unlabelled section for merged brackets), the
champion once decided, entrant count, round count, and top scorers/assisters for the playoff's matches.

#### Scenario: Per-conference bracket
- **WHEN** the playoff format has more than one conference bracket
- **THEN** the system SHALL return one labelled section per conference (named after that conference's league)
  plus, if a cross-conference final exists, a centered closing section labelled as the final

#### Scenario: Series vs. single-game ties
- **WHEN** a bracket tie is a best-of-N series
- **THEN** each card SHALL show the series win tally per side plus one score chip per game (including
  not-yet-played placeholder chips while the series is undecided); a single-game tie SHALL instead show the
  game score and any penalty-shootout tally directly
