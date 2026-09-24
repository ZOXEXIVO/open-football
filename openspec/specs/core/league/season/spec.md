# core/league/season Specification

## Purpose
Owns calendar-year and season-phase resolution: which season and phase any given date belongs to, both globally and per league's own opening/closing windows.

## Requirements

### Requirement: Season phase governs training and rotation posture
Every calendar date within a season SHALL resolve to exactly one season phase (off-season, pre-season, early season, mid-season, winter break, or run-in), each carrying its own condition-recovery multiplier, match-readiness gain, and competitive/rest-window classification.

#### Scenario: Date falls in the winter break window
- **WHEN** the current date is between December 20 and December 31
- **THEN** the resolved phase reports faster condition recovery than a normal mid-season rest day and is classified as a non-competitive rest window

### Requirement: Season identity resolves from an August-start year
A season SHALL be identified by the calendar year in which it starts (August), so that any date from August of one year through July of the next resolves to the same season.

#### Scenario: Date in June
- **WHEN** a date falls in June of year Y
- **THEN** it resolves to the season that started in August of year Y-1, not a season starting in year Y

### Requirement: A league's settings define its own season calendar
A league's start and end windows SHALL define a season calendar. The calendar files any date into exactly one of that league's seasons, identified by the calendar year in which the season opens. A season SHALL turn over at the midpoint of the league's off-season, meaning the gap between the end of one campaign's ending window and the opening day of the next campaign. A season whose ending window closes in a later calendar year than its opening day SHALL be labelled `YYYY/YY` (for example `2026/27`). Any other season SHALL be labelled with its single year (for example `2026`). This calendar is independent of the global August-start season identity, which is unchanged.

#### Scenario: Autumn-spring league mid-campaign
- **WHEN** a league opens on 9 August and closes on 17 May, and a date falls on 14 March 2027
- **THEN** the date SHALL resolve to the season that opened in 2026, labelled `2026/27`

#### Scenario: Calendar-year league
- **WHEN** a league opens on 1 February and closes on 15 December, and a date falls on 20 November 2026
- **THEN** the date SHALL resolve to the season that opened in 2026, labelled `2026`

#### Scenario: Fixture played just before the opening day
- **WHEN** a league opens on 9 August and a fixture of that league is dated 8 August 2026
- **THEN** the fixture SHALL resolve to the season that opened in 2026, not to the one that opened in 2025, because the turnover sits at the midpoint of the break rather than on the opening day

#### Scenario: Playoff after the campaign ends
- **WHEN** a league closes on 1 June and opens again on 15 July, and a playoff fixture is dated 10 June 2027
- **THEN** the fixture SHALL resolve to the season that opened in 2026

#### Scenario: Off-season crossing the new year
- **WHEN** a calendar-year league closes on 1 October and opens on 1 March, and a date falls on 20 December 2026
- **THEN** the date SHALL resolve to the season opening in 2027, because the midpoint of that break falls in mid-December
