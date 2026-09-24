# core/club/academy Specification

## Purpose
The academy capability owns the pool of youth prospects a club develops: who is eligible to graduate into the youth-team pathway, how many new players the intake produces each year, and how a short-handed youth side gets an emergency top-up — all bounded by capacity rather than quality cut-offs.

## Requirements

### Requirement: Academy pathway graduates on eligibility, not quality
The academy SHALL move players into the youth-team pathway based on an age floor and a readiness ranking, and SHALL NOT apply a quality/ability cut-off that would leave an age-eligible, fit prospect stuck in the academy.

#### Scenario: Ordinary graduation round
- **WHEN** the scheduled graduation pass runs and finds academy players at or above the club's minimum graduation age who are healthy and not exhausted
- **THEN** up to the season's throughput capacity graduate to the youth team, ranked by readiness first, then age, then assessed potential, then current ability, with no minimum ability required to qualify

### Requirement: Emergency call-up lowers the age floor, never raises it
When a youth team cannot field a full side, the academy SHALL allow an emergency call-up at a lower (or equal) age floor than the scheduled pathway, sized to the shortfall rather than the normal seasonal quota.

#### Scenario: Youth team short of players
- **WHEN** the youth squad lacks enough eligible players for a fixture
- **THEN** the academy promotes players down to the emergency age floor (never above the club's own scheduled minimum) until the shortfall is filled

### Requirement: Academy intake is an annual, capacity-bounded event
The academy SHALL generate new youth players once per year, sized by recruitment quality, pathway reputation and club reputation, and SHALL be throttled so the academy roster never exceeds its configured population cap.

#### Scenario: Intake month arrives with academy near its cap
- **WHEN** the annual intake window opens and the academy roster is close to `max_academy_players`
- **THEN** the computed intake count is clamped to the remaining headroom, and if there is no headroom the academy records the year as processed and produces zero players
