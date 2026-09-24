# core/club/player/squad Specification

## Purpose
Owns a player's squad-facing identity — his pathway stage and his set of status flags (listed, requested, unhappy, loan-listed, and the rest) that other modules read rather than mutate directly.

## Requirements

### Requirement: Signed players advertise availability through a fixed status set
A player SHALL carry a fixed, recognised set of availability statuses (transfer-listed, requested, unhappy, loan-listed) as the only statuses that mark him market-available, held as single-instance flags a reader can check without re-deriving availability from other state.

#### Scenario: Player re-affirmed as already listed
- **WHEN** a status the player already carries is added again
- **THEN** the add is a no-op and the original start date of the continuous spell is preserved, so "how long has he been listed" stays accurate
