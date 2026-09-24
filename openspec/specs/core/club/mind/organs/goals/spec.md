# core/club/mind/organs/goals Specification

## Purpose
The goals organ models the intentions a player or a member of staff holds over time — a catalog of wants, each carried with strength, urgency and progress through a status ladder from a silent inclination to a formal demand, rather than being rebuilt from scratch on every weekly review.

## Requirements

### Requirement: Big-stage pull escalates through inclination, mood and formal request
A player's desire to move to a bigger competition SHALL be modeled as one continuous score with three tiers of consequence — a silent inclination the market can act on, a visible recurring mood event with a morale drag, and a formal transfer request — and escalation to a request SHALL require persistence of the itch or a denied move, not a period of unhappiness.

#### Scenario: A good player in a sub-elite league has gone unbought
- **WHEN** a player's big-stage pull has persisted for a season without an incoming bid, or a concrete move for him was blocked
- **THEN** the player's own transfer request is triggered even though he was not previously unhappy
