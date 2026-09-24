# core/league/rules Specification

## Purpose
Owns the league's disciplinary and financial fair play regulations: suspension bookkeeping from match events and the escalating sanction lifecycle for clubs breaching financial thresholds.

## Requirements

### Requirement: Disciplinary tracking from match events
The league SHALL track, per player, a running yellow-card accumulation and any active suspension in matches remaining, derived from finished match results: direct red cards (including second-yellow reds) trigger an immediate one-match ban, and crossing the league's configured yellow-card threshold triggers a one-match ban and rolls the accumulation counter forward.

#### Scenario: Player receives a yellow card that crosses the threshold
- **WHEN** a player's yellow-card accumulation reaches the league's configured ban threshold as a result of a finished match
- **THEN** a one-match suspension is issued for that player and the accumulation counter is rolled past the threshold rather than left sitting exactly on it

#### Scenario: Player sent off directly
- **WHEN** a player receives a direct (or second-yellow) red card in a finished match
- **THEN** a one-match ban is issued regardless of that player's existing yellow-card tally

### Requirement: Financial fair play case lifecycle
The league SHALL open a financial fair play case against a club when its rolling financial deficit exceeds a configured warning threshold and no case is already open or in post-hearing cooldown for that club; the case SHALL carry an escalating sanction (warning, fine, points deduction, or transfer ban) determined by how far the deficit exceeds the configured bands, and SHALL resolve automatically once its hearing date arrives.

#### Scenario: Club deficit just above the warning band
- **WHEN** a club's rolling deficit exceeds the warning threshold but stays below the fine threshold
- **THEN** a case is opened with a warning-level sanction and a hearing date offset by the configured number of days from today

#### Scenario: Club deficit far above the points-deduction band
- **WHEN** a club's rolling deficit substantially exceeds the points-deduction threshold
- **THEN** the escalated sanction is a points deduction whose magnitude increases with the size of the overshoot, capped at a maximum deduction

#### Scenario: Case reaches its hearing date
- **WHEN** an open financial fair play case's hearing date is reached or passed
- **THEN** its sanction is applied exactly once (a points-deduction sanction updates the league table; other sanction kinds are applied by the caller) and the case moves from pending to resolved history

#### Scenario: Club already under cooldown
- **WHEN** a club's most recently concluded financial fair play case is still inside its configured cooldown window
- **THEN** a new case is not opened even if the club's deficit currently exceeds the warning threshold
