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

### Requirement: Staff advice and its intake act only on a player the gate has read
The system SHALL file a staff recommendation only for a player whose move the gate reads as at least "may be privately scouted", and SHALL open a transfer request or a shortlist place from a recommendation only when the gate reads the move as at least "may be shortlisted". This SHALL hold for every source of advice: the scout network, scouts' memories of players seen, expiring contracts, bargains, loans and players seeking game time.

A recommended player who cannot be read, at home or abroad, SHALL be dropped. He SHALL NOT pass because no reading was available.

#### Scenario: A remembered foreign player the club can no longer reach
- **WHEN** a scout's memory names a foreign player whose standing no longer fits the club
- **THEN** no recommendation is filed for him, and no file is opened

#### Scenario: Intake does not open a request for an unreadable player
- **WHEN** a recommendation names a player who no longer resolves to a readable player
- **THEN** intake opens no transfer request and no shortlist place for him

#### Scenario: A recommendation the club could act on
- **WHEN** a recommendation names a player whose move the gate reads as "may be shortlisted"
- **THEN** intake may fold it into an open request or open a new one, within the club's request cap
