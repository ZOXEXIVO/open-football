# core/club/team/squad Specification

## Purpose
Beyond its `contract_renewal` child, this directory's own files decide who is
protected from automatic disposal, who may be demoted out of the Main squad,
and how a fixture's match-squad, penalty taker and free-kick taker are
assembled for a `Team`.

## Requirements

### Requirement: Squad-asset protection classifies players from observable evidence, not hidden ability
Every automatic squad-disposal path (loan-out selection, free-transfer release, transfer-listing) SHALL classify a player's standing from evidence a coach can actually observe (assessed level, match results, training performance, reputation, position scarcity, prior minutes) and SHALL NOT read the player's hidden current-ability value directly; an unclassifiable player SHALL be treated as protected, not surplus.

#### Scenario: Early-season player with a thin match sample
- **WHEN** the season has not produced enough matches to assess a player's results-based evidence
- **THEN** the classifier falls back to his visible skill and reputation rather than treating the small sample as evidence of being surplus, and he is not routed toward loan-out or release on that basis

### Requirement: Main-squad demotion requires a genuine football or administrative reason
A contracted player already carrying a want-away status (listed/requested/unhappy) SHALL remain match-selectable from the main squad unless the club has a specific justification to demote him — he has become surplus, has a serious discipline or absence issue, or credible positional cover exists that meets minimum condition, ability-gap and age thresholds.

#### Scenario: Unhappy but still-useful player is considered for demotion
- **WHEN** the administrative sweep evaluates demoting a listed player from the main squad
- **THEN** the demotion is blocked unless a specific qualifying reason is found, even though the player carries a want-away status
