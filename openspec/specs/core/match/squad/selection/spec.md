# core/match/squad/selection Specification

## Purpose
Determines the matchday squad for a fixture — eligibility, the starting XI, the bench, and how the opponent's known threat profile shapes those choices.

## Requirements

### Requirement: Matchday squad selection — eligibility
For a given fixture, the system SHALL determine each player's eligibility to be selected from hard eligibility blocks (injury, competition suspension, cup-tie status, loan-clause restriction, international duty, non-registration for the competition) and soft eligibility limits (e.g. returning from injury) that penalize rather than exclude a player from selection.

#### Scenario: Injured player cannot be selected
- **WHEN** a player is currently injured
- **THEN** that player is hard-blocked from selection for the fixture regardless of other factors

#### Scenario: Player returning from injury can still be selected under pressure
- **WHEN** a player is in a recovery/return-to-fitness period but not hard-blocked
- **THEN** that player remains selectable with a penalty applied to their selection score rather than being excluded outright

#### Scenario: Cup-tied player excluded from a specific competition
- **WHEN** a player is cup-tied to a different club for the competition of the current fixture
- **THEN** that player is excluded from selection for that fixture even if eligible for other competitions

### Requirement: Matchday squad selection — fixture context and starting XI
The system SHALL classify the upcoming fixture's type and the coach's tactical objective (e.g. routine league match, title race, relegation six-pointer, derby, cup round, continental knockout, friendly; protect a lead, chase a game, develop players) from the competition and match-importance context, and SHALL score candidate players for starting selection using this context together with squad depth, fixture congestion, and the responsible coach's selection tendencies (rotation discipline, star favoritism, academy trust, tactical flexibility, medical caution).

#### Scenario: Cup final classified distinctly from an early cup round
- **WHEN** the fixture is the last round of a domestic cup competition
- **THEN** it is classified as a cup final rather than a cup early-round or cup-knockout fixture, and selection weighs it accordingly

#### Scenario: Fixture congestion increases rotation pressure
- **WHEN** the selecting club has multiple competitive fixtures scheduled within a few days of the current one
- **THEN** the squad-state congestion signal used by selection is higher than for a club with no nearby fixtures, favoring rotation

#### Scenario: A cautious coach rotates less aggressively
- **WHEN** the responsible coach's profile reflects high conservatism and rotation discipline
- **THEN** the selection scoring favors sticking with established first-choice players over an otherwise-similar coach with low rotation discipline

### Requirement: Matchday squad selection — opponent-aware threat profile
When the opponent's roster is known, the system SHALL derive a continuous opponent threat profile (pace, aerial, pressing intensity, low-block likelihood, set-piece threat, left/right wide threat, central overload) from that roster's actual player attributes and tactical shape rather than from a fixed or generic opponent assumption.

#### Scenario: Strong aerial opponent raises the aerial-threat signal
- **WHEN** the opponent's best few players in heading and jumping attributes are materially above league-average
- **THEN** the opponent's aerial-threat signal used by selection is elevated accordingly

#### Scenario: No opponent roster available falls back to neutral
- **WHEN** the opponent roster is empty or unavailable at selection time
- **THEN** the opponent threat profile defaults to a neutral, balanced reading rather than to a zero or undefined threat

### Requirement: Matchday squad selection — bench composition
The system SHALL select a bench from the remaining eligible, non-starting players, and SHALL consider tactical balance, rotation candidates, and squad-role scenarios (e.g. covering an injury-depleted position, protecting a young/promising player's minutes) when composing it, rather than filling bench slots purely by descending ability.

#### Scenario: Position coverage on the bench
- **WHEN** the starting XI is thin at a given position group relative to the rest of the squad
- **THEN** bench selection favors including cover for that position group over an otherwise higher-rated player at an already well-covered position
