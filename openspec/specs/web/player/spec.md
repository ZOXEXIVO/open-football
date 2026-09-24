# web/player Specification

## Purpose
Describes the web pages and mutating actions available for a single player, identified by slug: overview
(attributes/statistics), awards, contract, career-events feed, career history, match log, relations web, personal
profile (personality/happiness/mind), and manual roster-editing actions (release, transfer, loan, contract edit).

## Requirements

### Requirement: Player overview page
The system SHALL provide, for a player identified by slug, an overview page with identity, technical/mental/
physical/goalkeeping skill ratings, contract summary, market value, position map, loan status, and a
per-competition statistics breakdown for the season in progress.

#### Scenario: Active squad player
- **WHEN** a user requests the overview page for a player currently on a team's roster
- **THEN** the system SHALL return contract terms, current/potential ability ratings, market value, skills,
  position coverage, loan direction when on loan, and one statistics row per competition (friendly, cups, league,
  then any national-team competitions played this season), each with played/goals/assists/cards/rating

#### Scenario: Retired or free-agent player
- **WHEN** the resolved player has no current team (retired or unattached)
- **THEN** the system SHALL return the overview with no contract, no team-scoped fields, and a subtitle of
  "retired" or "free agent" as appropriate, still including career statistics rows

#### Scenario: Debug query parameter
- **WHEN** a user requests the page with a `debug` query parameter present
- **THEN** the system SHALL additionally return an internal-state diagnostic block (condition, fitness, form,
  injury, training internals) not shown otherwise

#### Scenario: Slug redirect
- **WHEN** the requested player slug does not match the player's canonical current slug
- **THEN** the system SHALL redirect to the canonical slug

### Requirement: Player awards page
The system SHALL provide, for a player, an awards page summarizing career awards: a lifetime summary (weekly/
monthly/season/global totals), a past-12-months chart, and one block per league/competition the player has won
awards in (most recent first), each broken into weekly/monthly/season/silverware/global award tallies.

#### Scenario: Player with award history
- **WHEN** a user requests the awards page for a player who has won at least one award
- **THEN** the system SHALL return the summary totals, the 12-month chart, and a league block per distinct
  league/competition (plus a global block for Continental/World Player of the Year, only when won), each block
  showing tallies for every award category won there

#### Scenario: Player with no awards
- **WHEN** the player has never won an award
- **THEN** the system SHALL return the page with zeroed summary tiles, no chart, and no league blocks

### Requirement: Player contract page
The system SHALL provide, for a player, a contract page with current club contract detail (salary, dates, squad
status, transfer-listed state, bonuses, clauses) and, when applicable, loan contract detail (parent/borrower,
loan salary, match fee, wage contribution, minimum appearances).

#### Scenario: Contracted player
- **WHEN** the player holds an active club contract
- **THEN** the system SHALL return salary (weekly and annual), contract type, squad status, shirt number, start/
  expiry dates with a human-readable remaining-time label, transfer-listed status, and every bonus and clause on
  the contract with localized labels and formatted values

#### Scenario: Player currently out on loan
- **WHEN** the player has an active loan contract
- **THEN** the system SHALL additionally return the loan's parent and borrowing clubs, loan salary and
  expiration, and optional match-fee/wage-contribution/minimum-appearance terms

#### Scenario: Free agent
- **WHEN** the player has no contract
- **THEN** the system SHALL return the page with no contract detail, no bonuses, and no clauses

### Requirement: Player career-events feed page
The system SHALL provide, for a player, a chronological feed combining three sources: happiness events (things
that happened to him), decision-register entries (roster/contract decisions the club made about him), and mind
journal notes (his own formed wants and convictions) — each event carrying a positive/negative/big classification
and, where available, cause/evidence detail and a follow-up hint.

#### Scenario: Requesting the events feed
- **WHEN** a user requests the events page for a player
- **THEN** the system SHALL return happiness events (routine noise filtered out), every decision-register row
  (unlimited, unlike the happiness feed which is capped), and every mind journal note, merged and sorted
  newest-first

#### Scenario: Legacy Decisions tab link
- **WHEN** a user requests the retired `/decisions` URL for a player
- **THEN** the system SHALL respond with a permanent redirect to the player's events page

### Requirement: Player career history page
The system SHALL provide, for a player, a season-by-season career table (club, loan flag, transfer fee, division,
per-competition breakdown, and season totals) plus career-aggregate totals.

#### Scenario: Requesting the history page
- **WHEN** a user requests the history page for a player
- **THEN** the system SHALL return one row per season played, each attributing the correct division for that
  historical season (not the club's current division), a competition breakdown (league/cups/friendly) per row,
  and aggregate career totals across every season

#### Scenario: Retired or free-agent player
- **WHEN** the player has no current team
- **THEN** the system SHALL still return the full season history and career totals, with team-scoped page chrome
  omitted

### Requirement: Player match log page
The system SHALL provide, for a player and an optional season filter, a chronological list of every match they
appeared in across their career (not merely their current team's schedule), with opponent, competition, home/away
and result. Each match SHALL be filed under a season as follows:
- a league, youth sub-league, domestic cup or playoff match under its own competition's season calendar;
- a continental tie under the season calendar of the domestic league of the club the player represented in it;
- an international under the same season as the player's nearest preceding club match, else the nearest following
  one, else the season calendar of their national team's country's top division.

#### Scenario: Requesting the full match log
- **WHEN** a user requests the matches page for a player with no season filter
- **THEN** the system SHALL gather every match on record for that player from match records rather than the
  current team's fixture list, including youth football, pre-transfer matches and matches from a spell between
  clubs, and SHALL show the most recent season that has matches

#### Scenario: Filtering by season
- **WHEN** a user requests the matches page with a `season` query parameter
- **THEN** the system SHALL return only the matches filed under that season and offer the seasons with recorded
  matches as navigable options

#### Scenario: Filtering by year
- **WHEN** a user requests the matches page with the retired `year` query parameter and no `season` parameter
- **THEN** the system SHALL ignore the parameter and show the most recent season that has matches

#### Scenario: Autumn-spring campaign stays together
- **WHEN** a player's league runs August to May and they played in it in October 2026 and March 2027
- **THEN** both matches SHALL appear under the same `2026/27` stop

#### Scenario: Calendar-year league
- **WHEN** a player's league runs February to December and they played in it in March 2026 and November 2026
- **THEN** both matches SHALL appear under a stop labelled `2026`

#### Scenario: International during the club season
- **WHEN** a player whose club league runs August to May earns a cap in March 2027, after a club match in that
  league in February 2027
- **THEN** the cap SHALL appear under the `2026/27` stop alongside that club football

#### Scenario: Continental tie of a calendar-year club
- **WHEN** a player's club plays in a calendar-year domestic league and they appear in a continental tie in March
  2027
- **THEN** the tie SHALL appear under the `2027` stop, alongside that club's 2027 league campaign

### Requirement: Player relations page
The system SHALL provide, for a player, an ego-centered social graph showing that player plus every teammate in
their shared dressing-room pool with whom they have a bond/friendship/tension/rivalry-tier relationship.

#### Scenario: Player with a squad
- **WHEN** a user requests the relations page for a player currently on a team
- **THEN** the system SHALL return the player as the graph's root node plus each directly related teammate, with
  tier counts

#### Scenario: Player with no squad
- **WHEN** the player is a free agent or retired
- **THEN** the system SHALL return an empty graph rather than an error

### Requirement: Player personal profile page
The system SHALL provide, for a player, a personal/psychological profile: personality radar (eight hidden
traits), morale reading with a positive/negative happiness-factor ledger, current concerns, manager relationship,
favourite clubs, biographical info (age, languages, condition, contract), reputation ladder (current/home/world),
career plan, active wants, and remembered-club sentiment.

#### Scenario: Requesting the personal profile
- **WHEN** a user requests the personal page for a player
- **THEN** the system SHALL return the personality radar values, a morale score with a named band and a summary
  sentence describing which way the happiness ledger leans, the ledger split into "weighing on him" and "lifting
  him" factors (each sorted strongest first, neutral factors omitted), current concerns, manager bond (when a
  relationship exists), and the reputation ladder across current/home/world reputation

#### Scenario: Player with an active career plan
- **WHEN** the player's mind holds a formed career plan
- **THEN** the system SHALL return the plan's arc, stage, whether it has been voiced, an optional deadline, the
  lowest level he will accept, and the board's mandate purpose when the club bought him for one

#### Scenario: Player with no memories of the current club
- **WHEN** the player has no recorded memory of his current club (e.g. just signed, or currently a free agent)
- **THEN** the system SHALL omit the "what he remembers" block rather than rendering it empty

### Requirement: Manual roster-editing actions
The system SHALL provide mutating actions, invoked against a specific player id, for an editor to directly alter
squad and market state: release to free agency, clear unhappy/injury flags, toggle forced match selection, cancel
an active loan, view/edit contract terms, execute a manual transfer, and execute a manual loan.

#### Scenario: Manual release to free agency
- **WHEN** an editor triggers the release action for a rostered player
- **THEN** the system SHALL move the player to the global free-agent pool, clear his contract, record a transfer-
  history entry crediting the release, seed his free-agent market state, and clean up any stale listings,
  transfer-list entries, scouting interest and live negotiations referencing him

#### Scenario: Manual transfer
- **WHEN** an editor submits a transfer action with a destination club id and an optional fee
- **THEN** the system SHALL move the player to that club's roster (from his current club or the free-agent pool),
  install a new contract, update his squad status for the new depth chart, and record the deal in transfer
  history

#### Scenario: Manual loan
- **WHEN** an editor submits a loan action with a destination club id and a season count
- **THEN** the system SHALL move the player to the destination roster under a loan contract that references the
  original parent club, set the loan expiration from the parent league's season end (or a default), extend the
  parent contract to cover the loan if needed, and record the loan in transfer history

#### Scenario: Editing contract terms
- **WHEN** an editor submits a new salary and/or expiration date (and, if the player is on loan, loan terms) that
  are not already expired
- **THEN** the system SHALL update the contract (and loan, if present, capped to not outlive the parent contract)
  and reject the request if any submitted date is not in the future
