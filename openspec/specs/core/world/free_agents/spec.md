# core/world/free_agents Specification

## Purpose
Owns the global unattached-player pool: sweeping released and contract-expired players into it, resolving monthly retirement risk for those who linger, and billing a parent club for the residual wages of a player it has loaned out.

## Requirements

### Requirement: Released and contract-expired players are swept into the global free-agent pool
The system SHALL move every roster player whose primary contract has been cleared (and who is not currently on loan and not retired) out of their team and into the global free-agent pool, recording a zero-fee "Free" transfer-history entry with a reason distinguishing an explicit club release from a natural contract expiry, and SHALL clear the player's club-specific transient status flags (listed, loan-listed, wants-transfer, unhappiness tied to the old club) as part of the move.

#### Scenario: Contract simply expires
- **WHEN** a player's contract lapses with no explicit release marker set
- **THEN** the player is swept into the free-agent pool with a transfer-history reason of natural expiry, not club release

#### Scenario: Club releases a player early
- **WHEN** a club explicitly releases a player (surplus, mutual termination, or unresolved salary) before natural expiry
- **THEN** the swept transfer-history entry records the specific release reason, and the player's leftover club-specific statuses are cleared

#### Scenario: Loaned player is not swept
- **WHEN** a player is currently out on loan
- **THEN** the sweep does not move them into the free-agent pool even if their nominal contract field is not directly settled

### Requirement: A swept player's global market visibility lags by one simulated day
A player newly moved into the free-agent pool SHALL be excluded from any market snapshot taken before the sweep runs, and SHALL be included in any snapshot taken after the sweep runs within the same simulated day; cross-country clubs act on him starting from the next day's snapshot.

#### Scenario: Same-day domestic signing is possible, cross-country is not
- **WHEN** a player's contract is cleared and swept into the pool on a given simulated day
- **THEN** his own country's market can act on him that same day, while clubs in other countries first see him in the snapshot built at the start of the following day

### Requirement: Free agents unemployed for a year or more are subject to monthly retirement risk
Once a free agent has been without a club for at least 365 days, the system SHALL evaluate retirement on the first day of each month using a probability that increases with age, decreases with playing quality and observable growth potential, and decreases with reputation; independent of the probability roll, the system SHALL enforce a deterministic maximum number of months a player may remain in the pool before retiring outright, with younger and higher-potential players given more months of leeway.

#### Scenario: Old, low-quality journeyman exceeds the hard bound
- **WHEN** an older, low-ability free agent has gone unemployed well past his cohort's deterministic month limit
- **THEN** he is retired that month without any probability roll being needed

#### Scenario: Young, promising player resists early retirement
- **WHEN** a young free agent with strong reputation has only just crossed the 12-month unemployment threshold
- **THEN** the computed retirement probability for that month is zero or negligible and he remains in the pool

#### Scenario: Freshly-seeded free agent is exempt
- **WHEN** a free agent's recorded time in the pool is under 12 months
- **THEN** the retirement pass does not evaluate them at all that month

### Requirement: A retiring free agent's departure is attributed and recorded against his home country
When a free agent retires from the pool, the system SHALL remove him from the pool, mark him retired with a reason (a planned farewell for sufficiently high-reputation players, otherwise a generic long-free-agency reason), and SHALL add him to his nationality country's retired-players list when that country is loaded in the current world.

#### Scenario: High-reputation player retires
- **WHEN** a free agent with world reputation above the high-profile threshold retires from the pool
- **THEN** his retirement reason is recorded as a planned farewell rather than a generic exit

#### Scenario: Nationality country not loaded
- **WHEN** a retiring free agent's nationality country is not part of the currently loaded world
- **THEN** he is still removed from the free-agent pool, and no retired-player record is created for him

### Requirement: A parent club continues to bear the residual wage cost of a loaned-out player
For every player out on loan, the system SHALL bill the parent club monthly for the positive difference between the player's primary contract salary and the loan salary the borrowing club pays, divided across twelve months, with any non-positive residual treated as zero.

#### Scenario: Loan salary is lower than the primary contract
- **WHEN** a player's loan salary is less than his primary contract salary
- **THEN** the parent club's finances are charged the prorated monthly difference as a wage expense

#### Scenario: Loan salary matches or exceeds the primary contract
- **WHEN** a player's loan salary is equal to or greater than his primary contract salary
- **THEN** no residual wage charge is applied to the parent club for that player
