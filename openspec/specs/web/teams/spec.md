# web/teams Specification

## Purpose
Describes the web pages available for a single team (squad): the squad roster, academy, finances, player-relations
web, schedule, scouting dashboard, staff roster, statistics, tactics/formation, transfer activity, and the team's
own newspaper — each identified by the team's URL slug.

## Requirements

### Requirement: Team squad page
The system SHALL provide, for a team identified by slug, a page listing every non-retired squad player plus any
players out on loan from this team, with per-player identity, ability, status and season-statistic fields.

#### Scenario: Requesting a team's squad
- **WHEN** a user requests the team page for a valid team slug
- **THEN** the system SHALL return each squad player with name, position, nationality, age, current/potential
  ability (as staff-perceived star ratings), market value, condition, captain/vice-captain flags, and season
  played/goals/rating, ordered by playing position

#### Scenario: Players out on loan
- **WHEN** one or more of the team's contracted players are out on loan at another club
- **THEN** the system SHALL include them in the squad listing flagged as loaned out, reflecting their current
  club's roster data rather than this team's

#### Scenario: Unknown team slug
- **WHEN** the requested team slug does not resolve to a team
- **THEN** the system SHALL return a not-found error

### Requirement: Team academy page
The system SHALL provide, for a Main or U18 team, an academy page summarizing the club's academy: level, tier,
pathway reputation, development identity, recruitment priorities, pipeline counts by phase, and each academy
player's readiness and risk flags.

#### Scenario: Requesting academy for an eligible team type
- **WHEN** a user requests the academy page for a Main or U18 team
- **THEN** the system SHALL return every academy player with position, age, ability ratings, readiness score,
  and injury/condition/jadedness risk flags, sorted by development phase then readiness then potential

#### Scenario: Requesting academy for an ineligible team type
- **WHEN** a user requests the academy page for a team type other than Main or U18 (e.g. Reserve, B, youth
  age-groups other than U18)
- **THEN** the system SHALL return a not-found error

### Requirement: Team finances page
The system SHALL provide, for a team competing under its own brand (Main, B, Second/Reserve-equivalent "own"
teams), a finances page with the club's current balance, budgets, income/expense breakdown, sponsorships and a
monthly history.

#### Scenario: Requesting finances for an own team
- **WHEN** a user requests the finances page for a team whose type is one that competes under its own brand
- **THEN** the system SHALL return the current balance (flagged positive/negative), transfer and wage budgets
  (or a "not set" label when unset), annual wage total across the club's teams, the latest completed month's
  income/expense breakdown by category, active sponsorship contracts, and up to 12 months of balance/income/
  expense history oldest-to-newest for charting

#### Scenario: Requesting finances for a non-own team
- **WHEN** a user requests the finances page for a team type that does not compete under its own brand
- **THEN** the system SHALL return a not-found error

### Requirement: Team player-relations page
The system SHALL provide, for a team, a social graph of player relationships within the dressing-room group the
team shares (Main+Reserve pooled, older youth squads pooled, or a self-contained squad), showing players who
have at least one strong positive or negative relationship.

#### Scenario: Requesting the relations graph
- **WHEN** a user requests the relations page for a team
- **THEN** the system SHALL return one node per player touched by a kept-tier relationship (bond, friendly,
  tension or rivalry), one edge per kept relationship pruned to each player's strongest threads, and summary
  counts per tier

#### Scenario: No qualifying relationships
- **WHEN** no pair of players in the pooled group has a relationship strong enough to be classified into a tier
- **THEN** the system SHALL return an empty node/edge set rather than an error

### Requirement: Team schedule page
The system SHALL provide, for a team, a combined fixture list across league, continental (Champions/Europa/
Conference League, Main squad only), and domestic cup competitions, filterable by season. League and cup fixtures
SHALL be filed under their own competition's season calendar. Continental fixtures SHALL be filed under the season
calendar of the team's own league.

#### Scenario: Requesting the full schedule
- **WHEN** a user requests the schedule page for a team with no season filter
- **THEN** the system SHALL show the fixtures of the season under way in the team's own league when that season has
  fixtures, else of the most recent season that has fixtures. The fixtures SHALL be sorted by date, each with
  opponent, home/away flag, competition name, and the result once played

#### Scenario: Filtering by season
- **WHEN** a user requests the schedule page with a `season` query parameter
- **THEN** the system SHALL return only fixtures filed under that season, and SHALL offer the seasons that have
  fixtures as navigable options

#### Scenario: Filtering by year
- **WHEN** a user requests the schedule page with the retired `year` query parameter and no `season` parameter
- **THEN** the system SHALL ignore the parameter and land as if no season were requested

#### Scenario: Autumn-spring campaign in progress
- **WHEN** the in-game date is 24 September 2026 and the team's league runs August to May
- **THEN** the default view SHALL be labelled `2026/27` and SHALL list the whole campaign, from its August fixtures
  through its May fixtures, including continental and cup fixtures dated in 2027

### Requirement: Team scouting dashboard page
The system SHALL provide, for a team, a scouting dashboard summarizing the club's scouting operation: scout
workload, active player monitoring, filed scouting reports, scouting assignments, match-attendance assignments,
recruitment meetings (with decisions and votes), known players, shadow reports, and transfer requests.

#### Scenario: Requesting the scouting dashboard
- **WHEN** a user requests the scouting page for a team
- **THEN** the system SHALL return each dashboard section as recorded for that team's club, with monetary values
  formatted (rendering unset/zero amounts as a dash) and dates formatted for display

### Requirement: Team staff page
The system SHALL provide, for a team, a roster of contracted staff grouped by department (management, coaching,
scouting, medical, directors/other), plus a goalkeeping department panel when the club has one.

#### Scenario: Requesting the staff roster
- **WHEN** a user requests the staff page for a team
- **THEN** the system SHALL return every contracted staff member (excluding those without a position or marked
  as free) grouped into department sections in a fixed department order, each with role, nationality, age,
  contract end date and wage

#### Scenario: No goalkeeping department reviewed yet
- **WHEN** the club has not yet reviewed its goalkeeper room
- **THEN** the system SHALL omit the goalkeeping panel from the page rather than showing an empty one

### Requirement: Team statistics page
The system SHALL provide, for a team, a page of season playing statistics per squad player (appearances, goals,
assists, cards, shooting/passing/tackling figures, average rating).

#### Scenario: Requesting the stats page
- **WHEN** a user requests the stats page for a team
- **THEN** the system SHALL return every squad player's season statistics, ordered by a sample-size-regressed
  rating so a player with very few appearances cannot outrank established performers

### Requirement: Team tactics page
The system SHALL provide, for a team, the formation/lineup last used (or the team's persistent tactical plan
when no usable match history exists) mapped onto pitch positions, plus a short history of recently used
in-match shapes.

#### Scenario: Team has recorded match history
- **WHEN** the team has at least one match recording both a final tactic and a starting eleven
- **THEN** the system SHALL display that match's formation and starting eleven on the pitch, filling any slot
  whose recorded starter has since left the club with the best currently available player for that slot

#### Scenario: No usable match history
- **WHEN** the team has no match recording both a tactic and a starting eleven
- **THEN** the system SHALL fall back to the team's persistent tactical plan and pick the best available player
  per slot

#### Scenario: In-match tactical shifts
- **WHEN** one of the team's recent matches ended with a different shape than it started
- **THEN** the system SHALL flag that match's entry in the recent-shapes list as a shift and report the match
  minute the shape first changed

### Requirement: Team transfers page
The system SHALL provide, for a team and an optional season, the current transfer-list, and completed incoming/
outgoing permanent transfers and loans involving the team's club, with a selectable season range.

#### Scenario: Requesting transfers for a season
- **WHEN** a user requests the team transfers page with a `season` query parameter (or omits it, defaulting to
  the current season)
- **THEN** the system SHALL return, for that season, every permanent transfer and loan in which the team's club
  was the buying or selling side (searched across all countries, since cross-border deals are recorded on the
  buying country), split into incoming/outgoing transfers and incoming/outgoing loans, and offer every season
  with recorded activity for the club as a selectable option

#### Scenario: Players currently transfer-listed
- **WHEN** the team has players on its transfer list
- **THEN** the system SHALL include them with their position and current market value, independent of the
  season filter

### Requirement: Team newspaper page
The system SHALL provide, for a team, a "newspaper" page of generated stories about the club and its players,
shown under the masthead of whichever squad runs the presses for that dressing room (a squad without its own
brand reads its parent's paper), with a tab badge counting published editions.

#### Scenario: Team runs its own presses
- **WHEN** the team's type is one that competes under its own brand
- **THEN** the system SHALL show that team's own editions under its own masthead

#### Scenario: Team has no press of its own
- **WHEN** the team's type has no brand of its own (e.g. Reserve, U19–U23)
- **THEN** the system SHALL show the club's Main-team editions under the Main team's masthead instead, and SHALL
  fall back to the team's own name (with no editions) if the club has no Main team

#### Scenario: No editions printed
- **WHEN** the covering team's newsroom has no issues
- **THEN** the system SHALL return the page in an empty state with a zero edition badge
