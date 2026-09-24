# core/continent/result Specification

## Purpose
Owns the continent's periodic tick orchestration: gating competition draws to their calendar dates, collecting each draw's qualified clubs with one-competition-per-club enforcement, and applying the cooldown-gated year-end award outcome.

## Requirements

### Requirement: Continental competition draws occur on fixed calendar dates
Continental competition draws SHALL be conducted on specific, continent-scoped calendar days each year: Champions League on August 15, Copa Libertadores on August 16, Europa League on August 20, and Conference League on August 25; knockout draws occur in mid-December.

#### Scenario: It is August 15 in a European continent
- **WHEN** the simulation date is August 15 and the continent is Europe
- **THEN** the Champions League draw is conducted using clubs qualified for that tier

#### Scenario: It is August 16 in a non-South-American continent
- **WHEN** the simulation date is August 16 and the continent is not South America
- **THEN** no Copa Libertadores draw is conducted

### Requirement: A club enters one continental club competition a season
When collecting the clubs qualified for a competition's draw, a club already drawn into another of the season's competitions SHALL be passed over and its qualification band place SHALL go to the next-placed club of the same league that is not yet drawn, so each competition still receives its full allocation.

#### Scenario: A club moves up the table between two draws
- **WHEN** a club sits in its league's Europa League places at the Europa League draw and in its Conference League places five days later at the Conference League draw
- **THEN** it plays only in the Europa League, and the Conference League place goes to the next club down the table that has not been drawn

### Requirement: Continental Player of the Year nomination and award are cooldown-gated
Year-end continental player awards SHALL nominate the top three ranked players and crown one winner, applying a nomination happiness event to each of the top three and an award happiness event plus reputation impact to the winner, both gated by a cooldown so repeated recomputation does not double-fire.

#### Scenario: The year-end award computation runs twice in quick succession
- **WHEN** the continental award outcome is applied a second time within the cooldown window
- **THEN** the nomination and award happiness events do not fire again for the same players
