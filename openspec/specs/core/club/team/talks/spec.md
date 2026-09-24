# core/club/team/talks Specification

## Purpose
Team talks let the manager deliver a pre-match, half-time or full-time speech in a chosen tone, translating that tone into morale effects on the squad.

## Requirements

### Requirement: Team talks apply personality-weighted morale effects
The club SHALL let its manager deliver a team talk with one of several tones (praise, criticise, encourage, passionate, silent), and the morale effect on each player SHALL depend on both the tone chosen and that player's own mental-strength/personality profile.

#### Scenario: Same criticising team talk given to two different personalities
- **WHEN** a critical team talk is delivered before a match
- **THEN** a mentally resilient player responds well while a nervy player's morale is harmed, from the same talk
