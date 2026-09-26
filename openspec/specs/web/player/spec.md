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

### Requirement: Player events severity and mood filter
The player's events page SHALL let the reader filter the feed by severity and by mood.

Every card SHALL belong to exactly one severity bucket: Minor, Moderate, Serious or Major when the card carries that
severity, and Unrated when it carries none. Unrated covers decision-register rows, mind-journal notes, and happiness
events with no attached context. Every card SHALL also belong to exactly one mood bucket: Positive, Negative or
Neutral, matching the card's existing positive/negative/neutral classification.

The page SHALL show one compact filter toolbar above the feed, holding two visibly separate groups. Each group SHALL
have its own localised label: a Severity group (Minor, Moderate, Serious, Major, Unrated) and a Mood group (Positive,
Negative, Neutral). Each toggle SHALL be a small chip showing a dot in its bucket's accent colour and its localised
label, with no card count. Every chip SHALL be on when the page loads, and a switched-off chip SHALL look switched
off. Assistive technology SHALL announce each group by its visible label. A card SHALL be visible only when both its
severity chip and its mood chip are on.

Filtering SHALL happen in the browser without a page reload or a new request. The filter state SHALL NOT be written
to the URL and SHALL NOT persist across page loads. Adding the filter SHALL NOT change which cards the feed contains,
their order, or how each card is rendered.

#### Scenario: All tiles on at page load
- **WHEN** a user opens the events page for a player with a non-empty feed
- **THEN** the page SHALL show a Severity group with five chips and a Mood group with three chips, each chip on,
  with a label and no count
- **AND** every card in the feed SHALL be visible

#### Scenario: The two groups read as separate filters
- **WHEN** the filter toolbar is shown
- **THEN** each group SHALL be preceded by its own label ("Severity", "Mood" in English), and the two groups SHALL
  be visually divided from each other
- **AND** each group SHALL be exposed to assistive technology as a group named by that label

#### Scenario: Compact on desktop
- **WHEN** the events page is viewed at desktop width (1280px) in English
- **THEN** both groups SHALL sit on a single toolbar line no taller than one chip plus its padding

#### Scenario: Buckets with no cards still show a tile
- **WHEN** a player's feed contains no Major cards
- **THEN** the Major chip SHALL still appear, and toggling it SHALL leave the visible cards unchanged

#### Scenario: Unchecking a severity tile hides its cards
- **WHEN** the user switches off the Minor chip
- **THEN** every Minor card SHALL be hidden, every other card SHALL stay visible, and the Minor chip SHALL look
  switched off
- **AND** switching the Minor chip back on SHALL show those cards again in their original positions

#### Scenario: Unrated tile covers every card without a severity
- **WHEN** the user switches off only the Unrated chip
- **THEN** every decision-register row, every mind-journal note and every happiness event without a severity
  SHALL be hidden
- **AND** every card with a Minor, Moderate, Serious or Major pill SHALL stay visible

#### Scenario: Unchecking a mood tile hides its cards
- **WHEN** the user switches off only the Neutral chip
- **THEN** every neutral card (including every decision-register row) SHALL be hidden, and every positive and
  negative card SHALL stay visible

#### Scenario: Severity and mood combine
- **WHEN** the user leaves only the Serious and Major severity chips and only the Negative mood chip on
- **THEN** exactly the cards that are both Serious-or-Major and negative SHALL be visible

#### Scenario: Visible count follows the filter
- **WHEN** the user changes any chip
- **THEN** the panel's card count SHALL show the number of cards now visible

#### Scenario: Every card filtered out
- **WHEN** the chips that are on leave no visible card
- **THEN** the panel SHALL show a localised line saying no events match the selected filters, distinct from the
  "no events recorded yet" empty state
- **AND** switching on a chip that brings any card back SHALL remove that line

#### Scenario: Empty feed shows no filter
- **WHEN** a player has no events at all
- **THEN** the page SHALL show the existing "no events recorded yet" empty state and SHALL NOT show the filter
  toolbar

#### Scenario: Keyboard operation
- **WHEN** a keyboard user tabs to a chip and presses Space
- **THEN** the chip SHALL toggle exactly as a pointer click does, with a visible focus indicator

#### Scenario: Localised labels in every locale
- **WHEN** the events page is requested in any supported locale (`en`, `de`, `es`, `fr`, `ja`, `pt`, `ru`, `tr`,
  `zh`)
- **THEN** both group labels, every chip label and the filtered-empty line SHALL render in that locale's own copy,
  never as a raw i18n key

#### Scenario: Phone-width layout
- **WHEN** the events page is viewed at phone width (640px or narrower)
- **THEN** each group SHALL sit on its own line with its label in front, its chips SHALL wrap within the panel, and
  the page SHALL NOT scroll horizontally

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
career plan, active wants, and remembered-club sentiment. A happiness factor SHALL be listed among current concerns
only when the ledger itself rates it a major concern. The club's development pathway SHALL describe a planned loan as a
plan while the player is still at his club, and as a loan only while he is actually away.

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

#### Scenario: A minor factor stays off the concerns headline
- **WHEN** the player's playing-time factor is weighing on him but sits above the major-concern band
- **THEN** it appears in the "weighing on him" ledger as a concern and is not listed among current concerns

#### Scenario: A planned loan reads as a plan
- **WHEN** the club's pathway has marked the player for a loan and he is still at the club
- **THEN** the club's view of him reads "To be loaned out"

#### Scenario: A player away on loan reads as on loan
- **WHEN** the club's pathway has him on a loan and he is currently away at the borrowing club
- **THEN** the club's view of him reads "Out on loan"

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

### Requirement: The player transfers page explains a listing by its own reason
The player's transfers page SHALL show, beside a transfer or loan listing status, the reason recorded with the club
decision that listed him. A later, unrelated decision SHALL NOT be shown as the listing's reason. When the player
carries no listing, the page SHALL show no listing reason.

#### Scenario: A pathway change after the listing does not replace its reason
- **WHEN** the board loan-lists a player because he needs competitive matches, and a development-pathway change is
  recorded after that listing
- **THEN** the transfers page gives the loan listing's reason as "needs competitive matches", not the pathway stage

#### Scenario: An unlisted player shows no listing reason
- **WHEN** a player carries no transfer or loan listing
- **THEN** the transfers page shows no listing reason, whatever his latest recorded decision is
