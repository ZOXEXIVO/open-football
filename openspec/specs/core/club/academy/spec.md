# core/club/academy Specification

## Purpose
The academy capability owns the pool of youth prospects a club develops: who is eligible to graduate into the youth-team pathway, how many new players the intake produces each year, how a short-handed youth side gets an emergency top-up — all bounded by capacity rather than quality cut-offs — and how far academy training may take a prospect.

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

### Requirement: Academy development ceilings are sized like first-team ceilings

An academy player's attribute ceilings SHALL be the ceilings first-team development would set for him: from his
potential, his position's development weights, his families' maturity at his fractional age and his match exposure.
They SHALL replace the flat age cap. An attribute already above its ceiling SHALL keep its value and SHALL NOT gain
further until the ceiling rises past it.

#### Scenario: An 18-year-old academy keeper cannot outgrow the first team's ceiling
- **WHEN** a PA 150 keeper of 18 trains in the academy for a season
- **THEN** none of his goalkeeping attributes rises above the first-team ceiling for his age and his match exposure

#### Scenario: A graduate keeps what he built
- **WHEN** an academy player whose attribute already sits above his new ceiling is checked against it
- **THEN** the attribute keeps its value rather than being cut to the ceiling
