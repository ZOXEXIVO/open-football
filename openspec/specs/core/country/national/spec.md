# core/country/national Specification

## Purpose
Owns the national-team pipeline: candidate scouting and call-up policy per team level, and match-day squad rotation for fixtures.

## Requirements

### Requirement: National-team call-up policy differs between senior and U21 levels

Senior national-team selection SHALL scout only each club's main team with no age cap and a minimum of 16 real players before generating synthetic depth; U21 selection SHALL scout the full club pyramid (main through U18), cap eligibility at age 21, target a 23-man squad, and require only 14 real players before generating synthetic depth.

#### Scenario: A U21 candidate pool is being assembled
- **WHEN** U21 selection scouts a club's roster
- **THEN** players are drawn from Main, Reserve, B, Second, U23, U21, U20, U19, and U18 teams, and any candidate older than 21 is excluded

### Requirement: National-team match-day rotation favors freshness and, in low-stakes fixtures, experimentation

Starting-XI selection for a national-team fixture SHALL adjust each candidate's positional merit score by a freshness term (non-positive, larger swing for outfield players than goalkeepers, dampened in knockout fixtures) and an experimentation term that opens room for uncapped players in low-stakes (friendly/group-stage) fixtures; knockout fixtures SHALL field the strongest-fit XI with minimal rotation.

#### Scenario: A goalkeeper has just played and is not fully fresh
- **WHEN** the freshness delta is computed for a goalkeeper versus an outfield player with identical recent-match history
- **THEN** the goalkeeper's freshness penalty is smaller (dampened by a 0.4 factor) than the outfield player's

#### Scenario: A knockout fixture is being selected
- **WHEN** match-day importance is `Peak` (knockout)
- **THEN** the freshness swing is reduced relative to competitive or friendly fixtures, and experimentation for uncapped players is minimized
