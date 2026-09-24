# core/transfers/squad Specification

## Purpose
Turns a club's roster into what it needs and what it would part with: the recruitment brief built against an objective, the continuous sell list a squad review produces, and the bounded emergency fill when a position group falls critically short.

## Requirements

### Requirement: A recruitment brief targets an objective, not a raw deficit
The system SHALL build a club's transfer needs as a recruitment brief comparing each shirt's target ability (set by the board's ambition and divisional baseline) against the current incumbent, rather than only flagging outright squad holes, and SHALL classify each resulting slot into a tier (transformative, upgrade, or cover) with a distinct budget share and minimum required improvement.

#### Scenario: A transformative slot requires a large improvement and dominates the budget
- **WHEN** a brief slot is classified as tier A (transformative)
- **THEN** it must represent at least an 8-point ability gain, draws up to 60% of the transfer budget, and at most one such slot is briefed per window

#### Scenario: A cover slot has no minimum gain requirement
- **WHEN** a brief slot is classified as tier C (cover)
- **THEN** it carries no minimum ability-gain requirement and is preferentially filled by a loan rather than a permanent signing

### Requirement: Squad review classifies players onto a continuous sell list with a readiness score
The system SHALL score every contracted player against six additive sell motives (peak value, expiring contract, wage relief, cash need, plan surplus, and the player pushing to leave, plus a pathway-driven "mature sale" motive) and SHALL only market a player to outside buyers once his combined score clears a minimum readiness bar, capped at a maximum number of marketed players per club.

#### Scenario: A player below the readiness bar stays privately held
- **WHEN** a player's combined sell-motive score is below 0.35
- **THEN** he is not marketed to buyers even if one or more individual sell motives apply to him

#### Scenario: Marketed roster is capped
- **WHEN** more than 6 players at a club would otherwise clear the readiness bar in the same pass
- **THEN** only the top 6 by score are marketed, sorted by combined readiness

### Requirement: Emergency squad fill applies looser but still bounded eligibility gates
The system SHALL trigger an emergency signing search when a position group falls below its minimum required size (goalkeeper 2, defender 7, midfielder 7, forward 4) or the total senior squad falls below 11 players, and SHALL score emergency candidates against reputation-gap and ability-band criteria that are looser than ordinary recruitment but still enforce a minimum acceptable score.

#### Scenario: Goalkeeper shortage triggers emergency fill
- **WHEN** a club's senior squad has fewer than 2 goalkeepers
- **THEN** an emergency signing slot is opened for the goalkeeper position

#### Scenario: Emergency candidates below the minimum score are rejected
- **WHEN** a candidate's emergency-fill suitability score is below 45.0
- **THEN** he is not selected to fill the emergency slot even if no better candidate is available that pass
