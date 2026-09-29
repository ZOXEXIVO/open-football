# core/club/team/training Specification

## Purpose
Defines when a club's squad trains and how the weekly training plan reads the fixture calendar: no training on match
days, and lighter plans only in genuinely congested weeks.

## Requirements

### Requirement: No training session runs on a match day

A club's plan for a date SHALL contain no sessions when that date is its next fixture. It SHALL also contain no
sessions when that date is its previous fixture, because the league plays before the clubs train.

#### Scenario: The day's match is already played
- **WHEN** a club plans its day after today's match has been filed as its previous fixture
- **THEN** today's plan has no sessions

### Requirement: Congestion is two competitive fixtures in the week around today

A week SHALL count as congested when the club has two or more competitive fixtures, played or upcoming, within three
days either side of today. When the league-side fixture window has not been written yet, the fallback count SHALL be
matches in the last seven days.

#### Scenario: One match a week
- **WHEN** a club plays only on Saturdays
- **THEN** no day of the season is planned as a double-match week

#### Scenario: A midweek fixture
- **WHEN** a club plays Saturday, Wednesday and Saturday
- **THEN** the days around the midweek fixture are planned as a double-match week
