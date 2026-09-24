# core/transfers/view Specification

## Purpose
Provides read-only projections of the world for the market to consult — resolving a player's identity, club, and a uniform market summary of him — without deciding anything itself.

## Requirements

### Requirement: A player-facing market summary surfaces valuation and career-pressure context uniformly
The system SHALL build a single player market summary — combining estimated value, contract status, position-group ranking, career-desire pressures, loan willingness, and seller-plausibility context — from the same construction path whether the caller is scanning the whole player pool or resolving one specific candidate, so both paths describe a given player identically.

#### Scenario: Pool scan and single-candidate lookup agree
- **WHEN** the same player is resolved once through a country-wide pool scan and once through a direct single-player lookup in the same simulated week
- **THEN** both summaries report the same estimated value, position-group rank, and seller-plausibility fields for him
