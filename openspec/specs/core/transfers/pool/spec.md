# core/transfers/pool Specification

## Purpose
Collects every country's free-agent market touches for one simulated tick into a single ledger and applies them in one batched pass, rather than mutating the pool country by country.

## Requirements

### Requirement: The transfer pool batches free-agent market touches into one serial pass
The system SHALL accumulate offered players, rejected players, and per-player block reasons from the free-agent market into a single per-tick ledger and SHALL apply that ledger in one batched pass across all countries rather than processing country pools individually.

#### Scenario: Free-agent offers from multiple countries are applied together
- **WHEN** several countries generate free-agent offers and rejections within the same simulated tick
- **THEN** all of them are collected into the shared pool ledger and applied together in a single batched pass
