# web/countries Specification

## Purpose
Describes the web pages scoped to a country: the world's list of countries, a country's competitions overview,
its national teams (senior and U21) squad/schedule/staff, and its free-agent pool.

## Requirements

### Requirement: World countries list page
The system SHALL provide a landing page listing every continent and, within it, every country that has at least
one competition, plus world-level summary counts and machine/AI status badges.

#### Scenario: Requesting the country list
- **WHEN** a user requests the countries list page for a language
- **THEN** the system SHALL return every continent with its countries that have at least one league, the total
  number of countries/clubs/players in the loaded world, and localized continent names (falling back to the raw
  name when no translation exists)

#### Scenario: Distributed worker and AI status
- **WHEN** the deployment has distributed match workers configured and/or an AI report contract saved
- **THEN** the page SHALL report the worker count and how many are ready, and whether AI reporting is enabled,
  pre-filling the AI settings with the saved contract or built-in defaults

### Requirement: Country competitions overview page
The system SHALL provide, for a country identified by slug, a page listing its non-friendly leagues grouped into
sections: a headed section per grouped competition (e.g. zoned/conference formats) including its playoff links,
and an unheaded section for consecutive ungrouped divisions, ordered by competition tier.

#### Scenario: Requesting a country's leagues overview
- **WHEN** a user requests the country page for a valid country slug
- **THEN** the system SHALL return the country's leagues grouped and ordered as described, with each grouped
  section's playoff links attached

#### Scenario: Unknown country slug
- **WHEN** the country slug does not resolve
- **THEN** the system SHALL return a not-found error

### Requirement: National team squad page
The system SHALL provide, for a country, a squad page for its senior national team (default) and a separate one
for its U21 team, each listing called-up players plus synthetic depth players when the real pool doesn't fill
every slot, with level-appropriate caps/goals columns.

#### Scenario: Requesting the senior squad
- **WHEN** a user requests a country's default squad page
- **THEN** the system SHALL return real call-ups (with club, ability ratings, condition, international caps/
  goals and call-up reason) and any synthetic depth players needed to fill the squad, ordered by position

#### Scenario: Requesting the U21 squad
- **WHEN** a user requests the U21 variant of the squad page
- **THEN** the system SHALL return the same shape of data sourced from the U21 team and ledger, with U21-specific
  caps/goals labels

#### Scenario: Synthetic depth player
- **WHEN** a squad slot has no real call-up available
- **THEN** the system SHALL render a synthetic player with no club affiliation and a "synthetic depth" call-up
  reason, rather than leaving the slot empty

### Requirement: National team schedule page
The system SHALL provide, for a country, a fixture list for its senior national team (default) and a separate
one for its U21 team, each fixture showing opponent, home/away, competition and result once played.

#### Scenario: Requesting the senior schedule
- **WHEN** a user requests a country's national-team schedule page
- **THEN** the system SHALL return every scheduled fixture for the senior team with opponent name/slug resolved
  and results included where available

#### Scenario: Requesting the U21 schedule
- **WHEN** a user requests the U21 schedule page
- **THEN** the system SHALL return the equivalent fixture list sourced from the U21 team

### Requirement: National team staff page
The system SHALL provide, for a country, a staff listing for its senior national team (default) and a separate
one for its U21 team, with each member's role, nationality and age.

#### Scenario: Requesting national team staff
- **WHEN** a user requests a country's (or its U21's) national-team staff page
- **THEN** the system SHALL return every staff member attached to that team with their role label, nationality
  and age

### Requirement: Country free agents page
The system SHALL provide, for a country, a list of unattached players holding that nationality, each with
ability ratings and a plain-language explanation of why they remain unsigned.

#### Scenario: Requesting the free agents list
- **WHEN** a user requests the free agents page for a country
- **THEN** the system SHALL return every free agent whose nationality is that country, with position, age,
  current/potential ability and a market-status explanation, ordered by position then descending current ability
