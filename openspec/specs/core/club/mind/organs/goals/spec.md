# core/club/mind/organs/goals Specification

## Purpose
The goals organ models the intentions a player or a member of staff holds over time — a catalog of wants, each carried with strength, urgency and progress through a status ladder from a silent inclination to a formal demand, rather than being rebuilt from scratch on every weekly review.

## Requirements

### Requirement: Big-stage pull escalates through inclination, mood and formal request
A player's desire to move to a bigger competition SHALL be modeled as one continuous score with three tiers of consequence — a silent inclination the market can act on, a visible recurring mood event with a morale drag, and a formal transfer request — and escalation to a request SHALL require persistence of the itch or a denied move, not a period of unhappiness.

#### Scenario: A good player in a sub-elite league has gone unbought
- **WHEN** a player's big-stage pull has persisted for a season without an incoming bid, or a concrete move for him was blocked
- **THEN** the player's own transfer request is triggered even though he was not previously unhappy

### Requirement: Wants to leave and wants to stay compete in both directions
Every goal that points its holder out of the club (a leave-direction goal) and every goal that keeps the holder there
(a stay-direction goal) SHALL take part in the weekly competition from both sides:

- it SHALL weaken at least one goal pointing the other way that its holder can hold, in proportion to its own
  pressure;
- it SHALL be weakened by at least one goal pointing the other way.

The same holds for a player's wants and a manager's, each against goals of its own kind.

A leave- or stay-direction goal SHALL sit outside the competition only when the catalog declares it as standing
apart. A goal declared apart SHALL weaken nothing and be weakened by nothing. A goal SHALL NOT end up outside the
competition by being left out of it.

For a player, the goals that stand apart are:

- wanting a loan, because a loan changes where the player plays and not who owns them;
- wanting to play for a boyhood club, because that is about another club, not this one;
- learning the local language, finding a mentor and ending a scoring drought, because they are about settling and
  form, not about this club.

Wanting permission to leave if the right offer comes, and wanting the captaincy, take part in the competition like
any other leave or stay goal.

Competition SHALL stay gradual: in one weekly review, each goal weakening another SHALL take less than a tenth of
that goal's strength, however strong it is.

#### Scenario: A decision to stay wears down permission to leave
- **WHEN** a player who wants to stay at the club also comes to want permission to leave if the right offer arrives,
  and neither want is fed again
- **THEN** at each weekly review, permission to leave loses strength in proportion to the stay want's pressure
- **AND** the want to stay loses strength in proportion to the pressure of permission to leave

#### Scenario: The change is gradual
- **WHEN** a player whose only stay want is at full pressure holds a want of permission to leave through one weekly
  review
- **THEN** permission to leave keeps more than nine tenths of its strength

#### Scenario: Wanting the captaincy pushes back on wanting out
- **WHEN** a player holds both a want of the captaincy and a want to move to a bigger club
- **THEN** each weakens the other at the weekly review

#### Scenario: Wanting a loan does not argue against staying
- **WHEN** a player who wants to stay at the club also wants a loan to get games
- **THEN** neither want weakens the other

#### Scenario: No leave or stay goal is outside the competition by omission
- **WHEN** the goal catalog is inspected, for players and for managers
- **THEN** every leave- or stay-direction goal both weakens and is weakened by a goal of its own holder pointing the
  other way, or is declared as standing apart
- **AND** every goal declared apart weakens nothing and is weakened by nothing

### Requirement: A want its holder cannot act on exerts no competition
A goal held blocked, which its holder cannot act on for now (frozen out, just arrived at a club, window shut, and the
other recorded reasons), SHALL keep its strength and SHALL keep shaping the holder's mood. It SHALL NOT weaken any goal
it competes with while the block holds. It SHALL still be weakened by the goals that compete with it. When the block
lifts, it SHALL compete again at its full pressure.

This holds for players and managers alike.

#### Scenario: A frozen-out player's blocked want does not wear down the way out
- **WHEN** a player is frozen out, holding the want to win back a place as blocked, and is pursuing permission to
  leave
- **THEN** the blocked want does not weaken permission to leave at the weekly review
- **AND** permission to leave does weaken the blocked want

#### Scenario: A want carried into a new club argues for nothing until the player settles
- **WHEN** a player who wanted first-team football moves club, the want travels with them held back as just arrived,
  and they form a want to win the new manager's trust
- **THEN** the carried want does not weaken the new want while the hold lasts

#### Scenario: Competition resumes when the block lifts
- **WHEN** the block on a goal is lifted
- **THEN** from the next weekly review it weakens the goals it competes with in proportion to its own pressure, as if
  it had never been blocked
