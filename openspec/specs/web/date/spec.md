# web/date Specification

## Purpose
Defines the small API that reports the current in-game date and time, used by the UI clock display.

## Requirements

### Requirement: Current in-game date is available via API
The system SHALL expose `GET /api/date`, returning the loaded world's current simulated date formatted for display, plus a machine-readable ISO timestamp.

#### Scenario: World is loaded
- **WHEN** a date request is made while a game world is loaded
- **THEN** the response SHALL include the world's current date formatted as day/month/year, a weekday-and-time string, and an ISO 8601 timestamp, all reflecting the simulated in-game clock

#### Scenario: No world loaded yet
- **WHEN** a date request is made before any game world has been loaded
- **THEN** the response SHALL fall back to the real wall-clock time in the same three fields
