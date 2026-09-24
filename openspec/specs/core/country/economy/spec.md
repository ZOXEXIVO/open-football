# core/country/economy Specification

## Purpose
Owns a country's economic simulation: reputation-derived market factors, their bounded monthly drift, and the yearly transfer-market price-level inflation model.

## Requirements

### Requirement: Country economic factors scale with country reputation

A country's economic factors (TV revenue multiplier, sponsorship market strength, stadium attendance factor) SHALL derive from its reputation on a 0-10000 scale, with top-reputation countries landing near a multiplier of 1.0 and small countries landing near 0.09, except stadium attendance which is floored so it never falls below 0.3.

#### Scenario: A country with reputation near 9500 derives economic factors
- **WHEN** economic factors are derived from a reputation of approximately 9500
- **THEN** the TV revenue multiplier and sponsorship market strength are close to 1.0

#### Scenario: A country with reputation near 3000 derives economic factors
- **WHEN** economic factors are derived from a reputation of approximately 3000
- **THEN** the market multipliers are near 0.09 but stadium attendance factor does not fall below 0.3

### Requirement: Country economic factors drift monthly within bounds

On each month boundary, a country's GDP growth and inflation rate SHALL fluctuate by small bounded random deltas, with GDP growth clamped to -0.05..0.10 and inflation clamped to 0.0..0.10; the TV revenue multiplier SHALL drift by up to plus-or-minus 2% relative to its current value, floored at 0.005.

#### Scenario: A monthly update runs
- **WHEN** the monthly economic update executes for a country
- **THEN** GDP growth and inflation rate each move by a small random amount within their respective clamped ranges

### Requirement: Transfer-market price level re-prices once a year based on spending pressure, not on itself

A country's transfer-market price level SHALL move exactly once per year (January 1st), driven by the ratio of gross permanent-transfer spend over the trailing year to the clubs' estimated annual income, not by the current valuations themselves. The yearly drift is bounded to at most 10% in either direction, and the price level is bounded to the 0.25-4.0 range overall.

#### Scenario: A country's spend-to-income ratio equals the equilibrium reference
- **WHEN** gross transfer spend divided by league income equals the configured equilibrium reference (0.45)
- **THEN** the price level does not move that year

#### Scenario: A country spends far beyond its income for a year
- **WHEN** gross transfer spend vastly exceeds league income
- **THEN** the price level still moves by no more than 10% that year, and never exceeds the 4.0 ceiling

#### Scenario: A country records no measurable league income
- **WHEN** league income for the year is zero or negative
- **THEN** the price level does not move, since a missing denominator must not be read as infinite pressure

#### Scenario: Re-pricing is checked on a non-January-1st date
- **WHEN** the simulation date is not January 1st
- **THEN** the country's price level is not re-priced that day
