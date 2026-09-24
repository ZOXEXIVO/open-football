# core/club/board/targets Specification

## Purpose
The targets capability derives each season's board mandate — transfer budget, wage budget and owner subsidy — from continuous financial and ambition signals, and tracks in-season adjustments to that mandate separately from its base values.

## Requirements

### Requirement: Board mandate flows from continuous financial and ambition signals
The board SHALL derive each season's transfer budget, wage budget and owner subsidy from free cash, ambition, regulatory standing and owner backing, rather than from a small set of discrete club-size tiers, and SHALL track in-season adjustments separately from the base mandate.

#### Scenario: Board cuts the budget mid-season
- **WHEN** the board reduces the transfer mandate during a financial crisis
- **THEN** the cut is recorded as a mandate adjustment distinct from the base `transfer_budget`, so a later monthly recompute of the base budget does not silently undo the cut
