# core/transfers/scouting/recruitment Specification

## Purpose
Runs the recruitment department's scouting effort: which of a club's open requests get a scout first, and how accurately a scout can judge a player given his skill and the market he is working in.

## Requirements

### Requirement: Scout assignment prioritizes a club's own open requests before general discovery
The system SHALL assign scouts to a club's open transfer requests first, ordered by request priority (critical before important before optional), before allocating scouts to opportunistic or general-discovery watching.

#### Scenario: Critical request gets first scout
- **WHEN** a club has one critical and one optional open transfer request and only one scout available to assign this pass
- **THEN** the scout is assigned to the critical request

### Requirement: Scout observation accuracy improves with skill and degrades in unfamiliar foreign markets
The system SHALL shrink a scout's observation error as judging skill and observation count increase, and SHALL widen that error for assessments made in an unfamiliar foreign market.

#### Scenario: Foreign-market assessment is less accurate
- **WHEN** a scout assesses a player in a market the club has little coverage of
- **THEN** the observation error is widened relative to an assessment of a similar player in a well-covered market, though never below a fixed accuracy floor
