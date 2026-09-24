# core/match/engine/state Specification

## Purpose
Owns the match's period state machine — the ordered progression from kickoff through full time, extra time and a penalty shootout when required.

## Requirements

### Requirement: Match lifecycle progression
A simulated match SHALL progress through an ordered sequence of periods — pre-match/initial, first half, half time, second half, and (when the fixture requires a winner and normal time ends level) extra time and a penalty shootout — ending in a terminal "match over" state, and SHALL NOT allow the second half to begin before half time has occurred.

#### Scenario: Normal-time fixture with a winner
- **WHEN** a league match completes its second half with the scores not level
- **THEN** the match transitions directly to its end state without extra time or a shootout

#### Scenario: Knockout fixture level after normal time
- **WHEN** a cup or continental knockout match's second half ends with the score tied
- **THEN** the match proceeds into extra time and, if still level afterward, into a penalty shootout to determine the outcome
