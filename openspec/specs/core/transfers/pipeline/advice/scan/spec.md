# core/transfers/pipeline/advice/scan Specification

## Purpose
Generates and intakes staff transfer advice each week: what scouts and staff surface as recommendations, which of those recommendations are still fresh enough to act on, how many open requests a club may carry at once, and the weekly diagnosis of why an available player has not moved.

## Requirements

### Requirement: Recommendation intake only considers recent staff advice
The system SHALL only fold a staff transfer recommendation into an open or new transfer request if it was filed within the last 7 days.

#### Scenario: Stale recommendation is ignored
- **WHEN** a scout recommendation was filed 10 days ago and has not yet been acted on
- **THEN** the weekly recommendation-intake pass does not consider it for opening or extending a request

### Requirement: A club's active transfer request count is capped
The system SHALL cap the number of simultaneously active (non-terminal) transfer requests a club may hold, and SHALL refuse to open a new recommendation-driven request once that cap is reached.

#### Scenario: Eighth request blocked
- **WHEN** a club already has 8 active transfer requests
- **THEN** a new recommendation-driven request is not opened until an existing one resolves

### Requirement: Weekly market circulation diagnoses why each publicly available player has not moved
The system SHALL run a weekly circulation pass over every publicly available player, tagging each with a status (already-interested, too-early-to-buy, blocked-with-reason, or open) based on listing age and a scan of plausible buyers, subject to a per-country scan budget.

#### Scenario: A very recent listing reads as too early
- **WHEN** a player listing is under 14 days old
- **THEN** the circulation pass tags him "too early" rather than running the full buyer scan

#### Scenario: Scan budget exhaustion keeps the previous diagnosis
- **WHEN** a country's circulation pass has already performed 200 full buyer scans in the current weekly pass
- **THEN** further players keep their previous diagnosis rather than receiving a fresh scan that week

### Requirement: A breakout player must clear a discoverability threshold to enter scouting attention
The system SHALL compute a 0-100 breakout score for standout non-star performers from position-weighted output, discounted by league reputation (except for youth squads), and SHALL only treat a player as a breakout once that score clears a fixed threshold.

#### Scenario: Breakout threshold gates discovery
- **WHEN** a player's breakout score is 45.0 or higher
- **THEN** he becomes eligible to enter club watchlists and recommendation flows as a breakout candidate

#### Scenario: A player's own desire to leave can lower his discovery bar
- **WHEN** a player privately wants to leave and the watching league's reputation exceeds his own club's league reputation by 1200 or more
- **THEN** his effective breakout/discovery bar is lowered, down to a floor of 22.0
