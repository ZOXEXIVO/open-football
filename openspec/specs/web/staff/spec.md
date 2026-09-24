# web/staff Specification

## Purpose
Describes the web pages available for a member of club staff, identified by staff id: a profile/attributes page
and a personal/psychological profile page, both scoped to the team the staff member currently works for.

## Requirements

### Requirement: Staff profile page
The system SHALL provide, for a staff member identified by id, a profile page with identity, role, contract
terms, coaching/goalkeeping/mental/knowledge/medical attribute ratings, and a list of players they have
previously worked with.

#### Scenario: Requesting a staff member's profile
- **WHEN** a user requests the staff page for a valid staff id
- **THEN** the system SHALL return role, age, birth date, nationality, current team, contract salary/expiry
  (when contracted), and the full set of coaching/goalkeeping/mental/knowledge/medical attribute ratings

#### Scenario: Unknown staff id
- **WHEN** the requested staff id does not resolve to a staff member on any team
- **THEN** the system SHALL return a not-found error

#### Scenario: Known-players list
- **WHEN** the staff member has dossier records of players they previously worked with
- **THEN** the system SHALL return up to 24 of them, warmest-remembered first (ties broken by matches worked
  together), each with a five-step regard label, spell/match counts, whether they currently work together, how
  a past working relationship ended, and any medals/scars on record

### Requirement: Staff personal profile page
The system SHALL provide, for a staff member, a personal/psychological profile page: personality radar (eight
hidden traits), behavioural/coaching-style/license summary, fatigue and job satisfaction, recent performance
metrics, a recent-events feed, and — for scouting roles — a player-monitoring workload.

#### Scenario: Requesting the personal profile
- **WHEN** a user requests the personal page for a staff member
- **THEN** the system SHALL return the personality radar values, coaching style, license tier, fatigue and job
  satisfaction percentages, mental attribute ratings, contract salary/expiry, and up to 8 recent events
  (newest first) each flagged positive or negative

#### Scenario: Scouting-role monitoring workload
- **WHEN** the staff member holds a scouting position
- **THEN** the system SHALL additionally return every player they are currently monitoring, with target club,
  monitoring status, confidence percentage, observation counts, assessed ability/potential, and the latest
  recruitment-meeting vote when one exists

#### Scenario: Non-scouting role
- **WHEN** the staff member does not hold a scouting position
- **THEN** the system SHALL return an empty monitoring list rather than an error
