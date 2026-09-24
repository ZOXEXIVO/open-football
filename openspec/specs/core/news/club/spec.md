# core/news/club Specification

## Purpose
Compiles each club's weekly per-side newspaper editions from the world's recent match, transfer, loan, and boardroom activity into printed issues.

## Requirements

### Requirement: Weekly club press run
The system SHALL produce one newspaper edition per week for every side (first team, reserve/"2" side, or B team) that competes under its own brand, covering the seven days just elapsed.

#### Scenario: Multiple sides at one club
- **WHEN** a club fields a first team, a reserve team in a real division, and a B team
- **THEN** up to three separate editions are compiled that week, one per side, each reporting only the football that side played

#### Scenario: Dormant side produces no edition
- **WHEN** a side has no fixtures and no stories to report for the week
- **THEN** no newspaper issue is published for that side, so the shelf is not filled with blank editions

### Requirement: Monthly league press run
The system SHALL produce one newspaper edition per eligible league on the first day of each month, reviewing the month that has just ended.

#### Scenario: Cup and friendly competitions excluded
- **WHEN** a competition is flagged as a cup or as friendly
- **THEN** no league edition is compiled for it

#### Scenario: Year boundary
- **WHEN** the press run executes on 1 January
- **THEN** the reviewed period is the previous December

#### Scenario: Month already closed is never reopened
- **WHEN** a month has already been closed for a league's newsroom
- **THEN** that month is not recompiled or re-walked on a later run

#### Scenario: Quiet month produces no issue
- **WHEN** a division neither had a frozen monthly awards snapshot for the reviewed month nor any transfer rumours filed
- **THEN** no issue is published for that month, though the month is still marked as covered

### Requirement: League edition content and ordering
A published league edition SHALL lead with the month's top scoring chart, ranked above transfer rumours, and SHALL carry no per-club mood or results list.

#### Scenario: Top scorer leads the page
- **WHEN** a league edition is compiled with both a scoring chart and transfer rumour stories
- **THEN** the leading story is the month's top scorer, ahead of every rumour

#### Scenario: League edition mood is level
- **WHEN** a league edition is published
- **THEN** its mood is recorded as steady, since a division has no partisan form to swing on

### Requirement: Story attribution is settled at press time
A story naming a player SHALL be credited to the club he played for at the time the edition went to press, not re-resolved when the archive is later read.

#### Scenario: Player sold after the reporting period
- **WHEN** a player was on a division's team sheet during the reviewed period but has since moved to a different club before the edition is compiled
- **THEN** the story keeps the credit recorded at press time rather than reflecting his new club

#### Scenario: Subject never seen on a division roster
- **WHEN** a story's subject was never found on any team sheet in the division during the walk
- **THEN** the story carries no team credit and the page falls back to displaying his current club

### Requirement: Match-derived facts feed the weekly report
The system SHALL derive hat-tricks, red cards, man-of-the-match, keeper performance, outfield ratings, comeback/drama narratives, continental results, cup-tie outcomes, and playoff series outcomes from completed matches within the reporting week, restricted to non-friendly fixtures.

#### Scenario: Friendly matches excluded
- **WHEN** a completed match is flagged as friendly
- **THEN** it contributes no facts to any weekly desk

#### Scenario: Hat-trick detection
- **WHEN** a player scores three or more non-own goals in a single match and can be attributed to one of the two recorded sides
- **THEN** that match is recorded as a hat-trick for that player and side

#### Scenario: Unattributable event is dropped
- **WHEN** a goal, card, or stat line names a player who cannot be matched to either recorded squad
- **THEN** no story or fact is recorded for that event

#### Scenario: Playoff series decided this week
- **WHEN** a playoff series' most recent game fell inside the reporting week and produced a winner
- **THEN** both sides receive a playoff-tie fact indicating advancement or elimination, and whether the series decided a place in the final

### Requirement: Transfer business reported per club
The system SHALL report a club's completed transfer arrivals and departures, and its live pursuits (bids lodged, fees agreed, medicals booked, and rejections), bucketed to the buying or selling club as appropriate.

#### Scenario: Arrival credited to the receiving squad's own paper
- **WHEN** a player completes a transfer into a specific squad that has its own newspaper
- **THEN** the arrival story appears on that squad's edition rather than only the club's flagship page

#### Scenario: Departure always on the flagship page
- **WHEN** a player leaves the club via transfer
- **THEN** the departure is reported on the club's page of record regardless of which squad he left from

#### Scenario: Stale rejection is not news
- **WHEN** a negotiation was rejected more than the configured freshness window before the current reporting week
- **THEN** it is not reported as a pursuit outcome

#### Scenario: Unresolved timeout is not reported
- **WHEN** a negotiation expired without either side accepting or explicitly rejecting it
- **THEN** no market story is filed for it

### Requirement: Manager pursuits reported to both clubs
The system SHALL report an in-flight managerial approach to both the requesting club and the source club, distinguishing which side initiated it.

#### Scenario: Rejected approach expires
- **WHEN** a managerial approach has been marked rejected
- **THEN** it is no longer reported to either club

### Requirement: Loan activity reported to the parent club
The system SHALL report a loaned-out player's status to the club that still owns his contract, since that club cannot discover the loan by walking its own rosters.

#### Scenario: Loanee's parent club receives the report
- **WHEN** a player is out on loan at another club
- **THEN** his parent club's weekly edition can include a loan-watch entry for him, distinguishing prospect-age loanees from others

### Requirement: Edition mood reflects recent form and board pressure
Only a club's flagship (page-of-record) edition SHALL factor board confidence and pressure into its published mood; non-flagship editions ignore boardroom pressure.

#### Scenario: Manager on final warning
- **WHEN** the flagship team's manager is on a final warning from the board
- **THEN** the computed pressure used for that edition's mood is at its maximum

#### Scenario: Reserve-side edition ignores board pressure
- **WHEN** a non-flagship side's edition mood is computed
- **THEN** board confidence and pressure contribute nothing to it, since the board does not judge that side's manager on the first team's results

### Requirement: Boardroom stories confined to the flagship edition
Boardroom-desk stories SHALL only be filed into a club's page-of-record edition, never into a secondary side's edition.

#### Scenario: Reserve side never reports a sacking
- **WHEN** the club's board takes an action such as dismissing the head coach
- **THEN** that story is filed only into the flagship edition, not into any reserve or youth side's paper
