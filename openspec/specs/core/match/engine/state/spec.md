# core/match/engine/state Specification

## Purpose
Owns the match's period state machine — the ordered progression from kickoff through full time, extra time and a penalty shootout when required.

## Requirements

### Requirement: Match lifecycle progression
A simulated match SHALL progress through an ordered sequence of periods — pre-match/initial, first half, half time, second half, and (when the fixture requires a winner and normal time ends level) a first extra-time period, an extra-time interval, a second extra-time period and a penalty shootout — ending in a terminal "match over" state, and SHALL NOT allow the second half to begin before half time has occurred. Teams SHALL change ends at half time and at the extra-time interval, the side that did not kick off the preceding period SHALL kick off the next, and a timed period SHALL NOT end while a penalty awarded in that period is untaken.

#### Scenario: Normal-time fixture with a winner
- **WHEN** a league match completes its second half with the scores not level
- **THEN** the match transitions directly to its end state without extra time or a shootout

#### Scenario: Knockout fixture level after normal time
- **WHEN** a cup or continental knockout match's second half ends with the score tied
- **THEN** the match proceeds into two extra-time periods separated by an interval, with a change of ends at that interval, and, if still level afterward, into a penalty shootout to determine the outcome

#### Scenario: Penalty awarded as time expires
- **WHEN** a penalty is awarded and the period's clock, including stoppage time, reaches its end before the kick is taken
- **THEN** the period continues until the penalty kick is resolved and ends immediately afterwards
