# core/transfers/scouting Specification

## Purpose
Beyond its `recruitment/` child, this directory's own files run one club's
scouting day: assignment scanning and observation (`assignment.rs`), the
performance-breakout discoverability signal (`breakout.rs`), tunable scouting
config (`config.rs`), the scout-market desk that opens and closes scouting
corridors (`desk.rs`), available-player market-exposure scoring
(`exposure.rs`), what a scout concludes from observable signals
(`judgement.rs`), the year-round breakout watch (`watch.rs`), and the club's
standing watchlist of names (`watchlist.rs`).

## Requirements

### Requirement: A breakout player must clear a discoverability threshold to enter scouting attention
The system SHALL compute a 0-100 breakout score for standout non-star performers from position-weighted output, discounted by league reputation (except for youth squads), and SHALL only treat a player as a breakout once that score clears a fixed threshold.

#### Scenario: Breakout threshold gates discovery
- **WHEN** a player's breakout score is 45.0 or higher
- **THEN** he becomes eligible to enter club watchlists and recommendation flows as a breakout candidate

#### Scenario: A player's own desire to leave can lower his discovery bar
- **WHEN** a player privately wants to leave and the watching league's reputation exceeds his own club's league reputation by 1200 or more
- **THEN** his effective breakout/discovery bar is lowered, down to a floor of 22.0
