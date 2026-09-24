# core/continent/economics Specification

## Purpose
Owns the continent's economic zone: the aggregate financial-health indicator, TV rights pool and sponsorship value that other continental systems (regulations, rankings) read from.

## Requirements

### Requirement: Continental economic zone health tracks aggregate club finances
A continent's economic health indicator SHALL be updated from the aggregate income and expenses of every club across its countries, smoothed against its previous value rather than replaced outright, and clamped to the 0.0-1.0 range.

#### Scenario: Aggregate club profit margin is positive
- **WHEN** total continental club income exceeds total expenses for a period
- **THEN** the economic health indicator moves upward but is blended with its prior value rather than jumping directly to the new profit margin

### Requirement: Continental TV rights pool grows with competitive balance
The continental TV rights pool SHALL be rescaled based on a competitive-balance measure derived from continental rankings, and sponsorship value SHALL grow at a fixed periodic rate.

#### Scenario: Rights pool recalculation runs for a continent
- **WHEN** the periodic economic update executes
- **THEN** the TV rights pool value changes by a factor derived from the competitive balance measure and the sponsorship value increases by its fixed growth rate
