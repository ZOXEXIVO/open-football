# core/club/board/severance Specification

## Purpose
The severance capability prices what it costs the club to end a manager's contract early, so dismissal is a real financial brake rather than a free action.

## Requirements

### Requirement: Manager dismissal carries a severance cost
Ending a manager's contract early SHALL cost the club a settlement based on the months remaining on the contract and the owner's settlement share, with a floor so no dismissal is free.

#### Scenario: Owner dismisses a manager with years left on his deal
- **WHEN** the board settles a manager's contract that has significant time remaining
- **THEN** the club owes at least three months of his annual salary, scaled up by the months remaining and the owner's settlement share
