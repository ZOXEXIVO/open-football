# core/transfers/view Specification

## Purpose
Provides read-only projections of the world for the market to consult — resolving a player's identity, club, and a uniform market summary of him — without deciding anything itself.

## Requirements

### Requirement: A player-facing market summary surfaces valuation and career-pressure context uniformly
The system SHALL build a single player market summary — combining estimated value, contract status, position-group ranking, career-desire pressures, loan willingness, and seller-plausibility context — from the same construction path whether the caller is scanning the whole player pool or resolving one specific candidate, so both paths describe a given player identically.

The summary SHALL carry the two readings of the player that do not depend on who is asking: his level floor, and the wage he would ask to move. Every club SHALL read the same two figures for him on the same day.

Any player on the world pool SHALL resolve to his summary by his id, from any country. A caller that holds the live club and player records SHALL give the gate the same reading of him as a caller that holds his summary.

#### Scenario: Pool scan and single-candidate lookup agree
- **WHEN** the same player is resolved once through a country-wide pool scan and once through a direct single-player lookup in the same simulated week
- **THEN** both summaries report the same estimated value, position-group rank, and seller-plausibility fields for him

#### Scenario: A foreign player is found by his id
- **WHEN** a club's shortlist names a player registered in another country
- **THEN** his summary resolves, and the gate reads him as it would read a domestic player

#### Scenario: The live path and the summary path agree
- **WHEN** a negotiation reads a reserve-squad player from the live records, and a scan reads the same player from his summary on the same day
- **THEN** both give the gate the same ability, the same rank in his position group and the same expected wage
