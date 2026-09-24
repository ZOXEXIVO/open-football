# core/league/core Specification

## Purpose
Owns the `League` struct's own identity, format settings, financial scale, and the top-level simulate/build/process entry points that dispatch to every other league capability, plus the `LeagueCollection` that drives a whole country's or continent's leagues together.

## Requirements

### Requirement: League core identity and settings
A league SHALL carry a stable identity (id, name, slug, country, tier, reputation) and format settings (season start/end windows, promotion/relegation spot counts, optional group/split-season configuration) that determine how its schedule and standings behave for the whole season.

#### Scenario: Split-season league configuration
- **WHEN** a league is configured with `split_season = true` (e.g. an Argentine-style Apertura/Clausura competition)
- **THEN** the league treats each half of the season as its own round-robin tournament with its own table, freezes the first tournament's final standings when the second begins, and computes relegation from the annual aggregate of both halves

### Requirement: League financial scale derives from reputation
A league's prize pool and TV deal total SHALL scale from its own reputation, its tier (top flight vs. lower tiers), and its country's reputation, rather than being configured directly per league.

#### Scenario: Lower-tier league in a weaker football nation
- **WHEN** a league's reputation, tier, and country reputation are all below the values a top continental league would have
- **THEN** the computed prize pool and TV deal total are proportionally smaller than a first-tier league in a high-reputation country, following a multiplicative scale of reputation, tier weight, and squared country-market factor

### Requirement: Matchday build/process split
Simulating a matchday SHALL be split into a build phase (selects fixtures, mutates schedule/table state up to kickoff, and produces match objects to be played by the engine) and a process phase (consumes the played results and applies them back to the league's dynamics, table, and statistics). At most one of a "pending" batch of matches or an "immediate" non-matchday result SHALL be produced per build call.

#### Scenario: Non-matchday date reached
- **WHEN** a league is simulated on a date with no fixtures scheduled
- **THEN** the build step performs any non-matchday processing (season end, winter break checks) and returns an immediate result rather than a pending batch of matches

#### Scenario: Matchday date reached
- **WHEN** a league is simulated on a date with fixtures scheduled
- **THEN** the build step returns a pending batch of match objects for the engine to play, and league table/dynamics state is not finalized until the corresponding process step receives the played results

### Requirement: League collection simulates all member leagues together
A collection of leagues SHALL be simulated as a group for a given simulation date, producing one result per league, so callers can drive an entire country's or continent's league system from a single call.

#### Scenario: Simulation tick spans multiple leagues in a country
- **WHEN** a league collection containing several divisions is simulated for the current date
- **THEN** each league in the collection is advanced independently and the collection returns one result entry per league
